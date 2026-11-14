//! An interrupted or killed run must leave a readable FST that reaches the
//! last simulated time.
//!
//! FST value changes live in an in-memory block until it is flushed, and the
//! reader takes a file without trailer up to its last flushed block. Before
//! these fixes:
//!   * nothing was flushed below 64 MB, so `kill -9` (or an OOM kill) of a
//!     short run left a header with no data;
//!   * the Ctrl-C flag was polled at time-slot boundaries only, so a long loop
//!     inside one slot never saw it, and the 5 s backstop killed the run with
//!     the dump unwritten;
//!   * a blocked DPI call could not see it either.
//!
//! Now the writer also flushes on a wall-clock interval
//! (`XEZIM_FST_FLUSH_SECS`), loops poll the flag on their back-edges and stop
//! the run through the normal exit, and an interrupt the simulation thread
//! does not take within 0.5 s makes the FST guard thread close the dump at the
//! current slot time and flush it before the backstop ends the process.
//!
//! Each test drives the release binary and waits for an output line before it
//! signals, so machine load moves only the times, not the outcome.
//!
//! A kill can also land while a block is being written. fst-writer writes a
//! block in 8 KB pieces and then patches its length, so a block cut short used
//! to make the whole file unreadable (fst-reader asserted `compressed_length <=
//! section_length`). xezim now moves each finished block into the dump with a
//! two-step commit (`fst_sink`), and `XEZIM_FST_KILL_AT` kills the run at each
//! step to check what it leaves.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use fst_reader::{FstFilter, FstHierarchyEntry, FstReader, FstSignalValue};

/// What a decoded dump says: its end time and the timeline of one signal.
struct Decoded {
    end_time: u64,
    /// (time, value) of every change of the probed signal.
    probe: Vec<(u64, String)>,
}

fn decode(path: &Path, probe_leaf: &str) -> Decoded {
    let file =
        std::fs::File::open(path).unwrap_or_else(|e| panic!("no FST at {}: {}", path.display(), e));
    let mut reader = FstReader::open_and_read_time_table(std::io::BufReader::new(file))
        .unwrap_or_else(|e| panic!("FST at {} does not decode: {:?}", path.display(), e));
    let end_time = reader.get_header().end_time;
    let mut handle = None;
    reader
        .read_hierarchy(|entry| {
            if let FstHierarchyEntry::Var {
                name, handle: h, ..
            } = entry
            {
                let leaf = name.split_whitespace().next().unwrap_or(&name).to_string();
                if leaf == probe_leaf && handle.is_none() {
                    handle = Some(h.get_index());
                }
            }
        })
        .expect("FST hierarchy does not decode");
    let want = handle.unwrap_or_else(|| panic!("no var `{probe_leaf}` in the FST"));
    let mut probe = Vec::new();
    reader
        .read_signals(&FstFilter::all(), |time, h, value| {
            if h.get_index() == want {
                let v = match value {
                    FstSignalValue::String(b) => String::from_utf8_lossy(b).to_string(),
                    FstSignalValue::Real(f) => format!("r{f}"),
                };
                probe.push((time, v));
            }
            Ok::<(), ()>(())
        })
        .expect("FST value changes do not decode");
    Decoded { end_time, probe }
}

/// Scratch files for one test, removed on drop.
struct Scratch {
    paths: Vec<PathBuf>,
}

impl Scratch {
    fn new() -> Self {
        Scratch { paths: Vec::new() }
    }
    fn path(&mut self, tag: &str, ext: &str) -> PathBuf {
        // Tests run on parallel threads of one process: a process-wide
        // counter keeps their files apart.
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!(
            "xezim_fsttail_{}_{}_{}.{}",
            tag,
            std::process::id(),
            n,
            ext
        ));
        let _ = std::fs::remove_file(&p);
        self.paths.push(p.clone());
        p
    }
    fn write(&mut self, tag: &str, ext: &str, text: &str) -> PathBuf {
        let p = self.path(tag, ext);
        std::fs::File::create(&p)
            .and_then(|mut f| f.write_all(text.as_bytes()))
            .unwrap();
        p
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        for p in &self.paths {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// A running xezim whose stdout and stderr lines arrive on one channel.
struct Run {
    child: Child,
    lines: Receiver<String>,
    seen: Vec<String>,
}

impl Run {
    fn start(args: &[&str], env: &[(&str, &str)]) -> Run {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().expect("spawn xezim");
        let (tx, rx) = mpsc::channel();
        let out = child.stdout.take().unwrap();
        let err = child.stderr.take().unwrap();
        let tx2 = tx.clone();
        std::thread::spawn(move || {
            for l in BufReader::new(out).lines().map_while(Result::ok) {
                let _ = tx.send(l);
            }
        });
        std::thread::spawn(move || {
            for l in BufReader::new(err).lines().map_while(Result::ok) {
                let _ = tx2.send(l);
            }
        });
        Run {
            child,
            lines: rx,
            seen: Vec::new(),
        }
    }

    /// Wait for a line containing `needle`.
    fn wait_for(&mut self, needle: &str, limit: Duration) {
        let deadline = Instant::now() + limit;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match self.lines.recv_timeout(left) {
                Ok(l) => {
                    let hit = l.contains(needle);
                    self.seen.push(l);
                    if hit {
                        return;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = self.child.kill();
        panic!(
            "no line containing {needle:?} within {limit:?}; output so far:\n{}",
            self.seen.join("\n")
        );
    }

    fn signal(&self, sig: libc::c_int) {
        unsafe {
            libc::kill(self.child.id() as libc::pid_t, sig);
        }
    }

    /// Wait for the process to end (killing it after `limit`), then collect
    /// the rest of its output.
    fn finish(mut self, limit: Duration) -> (ExitStatus, String) {
        let deadline = Instant::now() + limit;
        let status = loop {
            if let Some(s) = self.child.try_wait().expect("wait") {
                break s;
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                panic!(
                    "xezim still running {limit:?} after the signal; output:\n{}",
                    self.seen.join("\n")
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        // The reader threads end at EOF; give them a moment to drain.
        while let Ok(l) = self.lines.recv_timeout(Duration::from_millis(500)) {
            self.seen.push(l);
        }
        (status, self.seen.join("\n"))
    }
}

/// `[xezim] interrupted at time N — ...` → N.
fn interrupt_time(out: &str) -> u64 {
    let line = out
        .lines()
        .find(|l| l.contains("[xezim] interrupted at time "))
        .unwrap_or_else(|| panic!("no interrupt message in:\n{out}"));
    line.split("interrupted at time ")
        .nth(1)
        .and_then(|r| r.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("cannot parse the time in {line:?}"))
}

/// A clocked counter (period 10) and a marker line once the run is going.
/// `@BODY@` adds the case-specific process.
const DESIGN: &str = r#"
module top;
  reg clk = 0;
  integer cnt = 0;
  always #5 clk = ~clk;
  always @(posedge clk) cnt <= cnt + 1;
  @BODY@
endmodule
"#;

fn design(body: &str) -> String {
    DESIGN.replace("@BODY@", body)
}

/// The counter's last change is at most one clock period before `t`.
fn assert_counter_reaches(d: &Decoded, t: u64, what: &str) {
    let last = d.probe.last().map(|(time, _)| *time).unwrap_or(0);
    assert!(
        last <= t && last + 10 >= t,
        "{what}: the counter's last change is at {last}, expected within one period of {t}"
    );
}

#[test]
fn sigint_fst_ends_at_the_interrupt_time() {
    let mut s = Scratch::new();
    let sv = s.write(
        "plain",
        "sv",
        &design(r#"initial forever begin #20000; $display("TICK %0t", $time); end"#),
    );
    let fst = s.path("plain", "fst");
    let mut run = Run::start(
        &[
            "--no-cache",
            "-s",
            "top",
            "--fst",
            fst.to_str().unwrap(),
            sv.to_str().unwrap(),
        ],
        &[],
    );
    run.wait_for("TICK", Duration::from_secs(10));
    run.signal(libc::SIGINT);
    let (status, out) = run.finish(Duration::from_secs(10));
    assert!(
        status.success(),
        "interrupted run should exit cleanly:\n{out}"
    );
    let t = interrupt_time(&out);
    let d = decode(&fst, "cnt");
    assert_eq!(d.end_time, t, "the FST should end at the interrupt time");
    assert_counter_reaches(&d, t, "interrupted run");
}

#[test]
fn sigint_stops_a_long_loop_inside_one_time_slot() {
    let mut s = Scratch::new();
    // 4e9 iterations at t=1000: the slot never ends on its own.
    let sv = s.write(
        "loop",
        "sv",
        &design(
            r#"longint spin = 0;
  initial begin
    #1000;
    $display("LOOPING");
    $fflush;
    for (longint i = 0; i < 64'd4000000000; i++) spin += i;
    $display("done %0d", spin);
  end"#,
        ),
    );
    let fst = s.path("loop", "fst");
    let mut run = Run::start(
        &[
            "--no-cache",
            "-s",
            "top",
            "--fst",
            fst.to_str().unwrap(),
            sv.to_str().unwrap(),
        ],
        &[],
    );
    run.wait_for("LOOPING", Duration::from_secs(10));
    run.signal(libc::SIGINT);
    let (status, out) = run.finish(Duration::from_secs(10));
    assert!(
        status.success(),
        "the loop should stop on the interrupt and the run exit cleanly:\n{out}"
    );
    assert_eq!(interrupt_time(&out), 1000, "{out}");
    assert!(!out.contains("done "), "the loop ran to completion:\n{out}");
    let d = decode(&fst, "cnt");
    assert_eq!(d.end_time, 1000, "the FST should end at the loop's time");
    assert_counter_reaches(&d, 1000, "long loop");
}

#[test]
fn sigint_during_a_blocked_dpi_call_flushes_the_fst() {
    let mut s = Scratch::new();
    let c = s.write(
        "dpi",
        "c",
        "#include <stdio.h>\n#include <unistd.h>\n\
         void c_block(void) { fprintf(stderr, \"BLOCKING\\n\"); fflush(stderr); sleep(30); }\n",
    );
    let so = s.path("dpi", "so");
    let ok = Command::new(std::env::var_os("CC").unwrap_or_else(|| "clang".into()))
        .args(["-shared", "-fPIC"])
        .arg(&c)
        .arg("-o")
        .arg(&so)
        .status()
        .expect("launch cc");
    assert!(ok.success(), "cc failed");
    let sv = s.write(
        "dpi",
        "sv",
        &design(
            r#"import "DPI-C" function void c_block();
  initial begin #1000; c_block(); end"#,
        ),
    );
    let fst = s.path("dpi", "fst");
    let mut run = Run::start(
        &[
            "--no-cache",
            "-s",
            "top",
            "--dpi-lib",
            so.to_str().unwrap(),
            "--fst",
            fst.to_str().unwrap(),
            sv.to_str().unwrap(),
        ],
        &[],
    );
    run.wait_for("BLOCKING", Duration::from_secs(10));
    let t0 = Instant::now();
    run.signal(libc::SIGINT);
    // Nothing can take the interrupt inside the C call: the backstop ends the
    // process after the grace period, once the tail is written.
    let (status, out) = run.finish(Duration::from_secs(12));
    assert!(
        !status.success(),
        "the blocked run can only end by the backstop:\n{out}"
    );
    assert!(
        t0.elapsed() >= Duration::from_secs(2),
        "the process ended before the grace period:\n{out}"
    );
    assert!(
        out.contains("FST dump flushed up to time 1000"),
        "no tail flush reported:\n{out}"
    );
    let d = decode(&fst, "cnt");
    assert_eq!(d.end_time, 1000, "the FST should end at the stuck time");
    assert_counter_reaches(&d, 1000, "blocked DPI call");
}

#[test]
fn kill_9_leaves_a_readable_fst_with_data() {
    let mut s = Scratch::new();
    let sv = s.write(
        "kill",
        "sv",
        &design(r#"initial forever begin #20000; $display("TICK %0t", $time); end"#),
    );
    let fst = s.path("kill", "fst");
    let mut run = Run::start(
        &[
            "--no-cache",
            "-s",
            "top",
            "--fst",
            fst.to_str().unwrap(),
            sv.to_str().unwrap(),
        ],
        &[
            ("XEZIM_FST_FLUSH_SECS", "0.3"),
            ("XEZIM_FST_COMMIT_LOG", "1"),
        ],
    );
    run.wait_for("TICK", Duration::from_secs(10));
    // The header, hierarchy and geometry are on disk from the start; wait for
    // the first time-based block flush to grow the file past them.
    let base = std::fs::metadata(&fst).map(|m| m.len()).unwrap_or(0);
    let deadline = Instant::now() + Duration::from_secs(10);
    while std::fs::metadata(&fst).map(|m| m.len()).unwrap_or(0) <= base {
        assert!(
            Instant::now() < deadline,
            "no value-change block flushed within 10 s"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    // Every block reported committed before the kill must be in the file.
    let committed = last_commit(&run.seen.join("\n"));
    run.signal(libc::SIGKILL);
    let (status, _) = run.finish(Duration::from_secs(10));
    assert!(!status.success());
    let d = decode(&fst, "cnt");
    assert!(d.end_time > 0, "the killed run's FST has no time range");
    assert!(
        d.end_time >= committed.unwrap_or(0),
        "the FST ends at {} but a block up to {:?} was committed",
        d.end_time,
        committed
    );
    assert!(
        d.probe.len() > 10,
        "the killed run's FST holds only {} counter changes",
        d.probe.len()
    );
    assert_counter_reaches(&d, d.end_time, "killed run");
}

/// `[xezim] fst-commit N` (with `XEZIM_FST_COMMIT_LOG`) → the last N.
fn last_commit(out: &str) -> Option<u64> {
    out.lines()
        .filter_map(|l| l.split("[xezim] fst-commit ").nth(1))
        .filter_map(|r| r.trim().parse().ok())
        .last()
}

/// Kill the run with `XEZIM_FST_KILL_AT=<point>` and check the file: it
/// decodes, ends at the last committed block (`want_last`), or at the
/// `$finish` time once the final header is in (`finished`).
fn kill_at(point: &str, finished: bool) {
    let mut s = Scratch::new();
    // A short run with blocks every 50 ms of wall time: at least two blocks
    // (the first is written as soon as it has data, the last at `$finish`).
    let sv = s.write(
        "killat",
        "sv",
        &design(r#"initial begin #200000; $display("FINISHING"); $finish; end"#),
    );
    let fst = s.path("killat", "fst");
    let run = Run::start(
        &[
            "--no-cache",
            "-s",
            "top",
            "--fst",
            fst.to_str().unwrap(),
            sv.to_str().unwrap(),
        ],
        &[
            ("XEZIM_FST_FLUSH_SECS", "0.05"),
            ("XEZIM_FST_COMMIT_LOG", "1"),
            ("XEZIM_FST_KILL_AT", point),
        ],
    );
    let (status, out) = run.finish(Duration::from_secs(60));
    assert_eq!(
        std::os::unix::process::ExitStatusExt::signal(&status),
        Some(libc::SIGKILL),
        "{point}: the run was not killed at the hook:\n{out}"
    );
    let d = decode(&fst, "cnt");
    match last_commit(&out) {
        None => {
            assert_eq!(point, "initial", "{point}: no block committed:\n{out}");
            assert!(d.probe.is_empty(), "initial: no block, but value changes");
        }
        Some(t) => {
            let want = if finished { 200000 } else { t };
            assert_eq!(d.end_time, want, "{point}: the FST end time\n{out}");
            assert_counter_reaches(&d, want, point);
        }
    }
}

#[test]
fn kill_before_the_first_block_leaves_a_readable_fst() {
    kill_at("initial", false);
}

#[test]
fn kill_in_a_torn_block_keeps_the_blocks_before_it() {
    kill_at("torn:2", false);
}

#[test]
fn kill_before_a_block_commit_keeps_the_blocks_before_it() {
    kill_at("body:2", false);
}

#[test]
fn kill_after_a_block_commit_keeps_that_block() {
    kill_at("committed:2", false);
}

#[test]
fn kill_before_the_final_header_keeps_the_last_block() {
    kill_at("last", false);
}

#[test]
fn kill_before_the_terminator_is_dropped_keeps_the_whole_run() {
    kill_at("header", true);
}

/// The finished file is exactly what fst-writer would write: no terminator,
/// and the header carries the end time.
#[test]
fn finished_fst_has_no_terminator() {
    let mut s = Scratch::new();
    let sv = s.write(
        "done",
        "sv",
        &design(r#"initial begin #200000; $finish; end"#),
    );
    let fst = s.path("done", "fst");
    let run = Run::start(
        &[
            "--no-cache",
            "-s",
            "top",
            "--fst",
            fst.to_str().unwrap(),
            sv.to_str().unwrap(),
        ],
        &[("XEZIM_FST_FLUSH_SECS", "0.01")],
    );
    let (status, out) = run.finish(Duration::from_secs(60));
    assert!(status.success(), "{out}");
    let bytes = std::fs::read(&fst).unwrap();
    // Walk the block chain: it must end exactly at the end of the file.
    let mut off = 0usize;
    let mut last_type = 0u8;
    while off < bytes.len() {
        assert!(off + 9 <= bytes.len(), "a block head is cut at {off}");
        last_type = bytes[off];
        let len = u64::from_be_bytes(bytes[off + 1..off + 9].try_into().unwrap()) as usize;
        assert!(len >= 8, "block at {off} has length {len}");
        off += 1 + len;
    }
    assert_eq!(off, bytes.len(), "the block chain overruns the file");
    assert_ne!(
        last_type, 255,
        "the finished FST still ends in a SKIP block"
    );
    let end = u64::from_be_bytes(bytes[17..25].try_into().unwrap());
    assert_eq!(end, 200000, "header end time");
    let d = decode(&fst, "cnt");
    assert_eq!(d.end_time, 200000);
    assert_counter_reaches(&d, 200000, "finished run");
}
