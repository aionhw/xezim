//! A reader that closes the pipe early (`xezim run.sv | head -1`) must not
//! abort the run. The CLI's own lines (the `Simulation finished` line, the
//! stats footer) went through `println!`, which panics when the write fails,
//! and `panic = "abort"` turned that into SIGABRT: exit 134 instead of the
//! run's own status, so a `$fatal` or `--error-exit` failure read as a crash
//! and a passing run as a failure. The run now ends with its usual status
//! (§20.10 for `$fatal`) and still writes its waveform. Process handling
//! only: no simulation output is compared.
use std::io::{BufRead, BufReader};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};

fn xezim() -> String {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.join("xezim").to_string_lossy().into_owned()
}

/// Enough output after the first line to fill the pipe, so the run is still
/// writing when the reader goes away.
const LINES: &str = "for (int i = 0; i < 50000; i++) $display(\"line %0d\", i);";

/// Close-on-exec, so a run spawned by a parallel test does not inherit this
/// pipe's read end and keep it open after the reader here goes away.
fn pipe() -> (OwnedFd, OwnedFd) {
    let mut fds = [0; 2];
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    assert_eq!(rc, 0, "pipe2");
    unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) }
}

struct Run {
    status: ExitStatus,
    stderr: String,
    dir: PathBuf,
}

/// Module `t` running `body` in one `initial` block.
fn initial_only(body: &str) -> String {
    format!("module t;\n  initial begin\n{body}\n  end\nendmodule\n")
}

/// Run `src` with `args`, read one line of stdout and close the pipe. With
/// `merge`, stderr shares the pipe (`2>&1 | head -1`).
fn run_closing_early(tag: &str, src: &str, args: &[&str], merge: bool) -> Run {
    let dir = std::env::temp_dir().join(format!("broken_pipe_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.sv");
    std::fs::write(&path, src).unwrap();
    let (read_end, write_end) = pipe();
    let err_log = dir.join("stderr.txt");
    let stderr = match merge {
        true => Stdio::from(write_end.try_clone().unwrap()),
        false => Stdio::from(std::fs::File::create(&err_log).unwrap()),
    };
    let mut child = Command::new(xezim())
        .current_dir(&dir)
        .args(args)
        .arg(&path)
        .stdout(Stdio::from(write_end))
        .stderr(stderr)
        .spawn()
        .expect("run xezim");
    let mut reader = BufReader::new(std::fs::File::from(read_end));
    let mut first = String::new();
    reader.read_line(&mut first).unwrap();
    drop(reader);
    let status = child.wait().expect("wait");
    let stderr = std::fs::read_to_string(&err_log).unwrap_or_default();
    Run {
        status,
        stderr,
        dir,
    }
}

fn assert_status(run: &Run, code: i32) {
    assert_eq!(
        run.status.signal(),
        None,
        "killed by a signal: {:?}; stderr: {}",
        run.status,
        run.stderr
    );
    assert_eq!(run.status.code(), Some(code), "stderr: {}", run.stderr);
    assert!(!run.stderr.contains("panicked"), "stderr: {}", run.stderr);
}

#[test]
fn a_finished_run_exits_zero_after_the_reader_closes() {
    let run = run_closing_early(
        "finish",
        &initial_only(&format!("{LINES}\n$finish;")),
        &[],
        false,
    );
    assert_status(&run, 0);
}

#[test]
fn a_run_that_ends_without_finish_exits_zero() {
    let run = run_closing_early("drain", &initial_only(LINES), &[], false);
    assert_status(&run, 0);
}

#[test]
fn fatal_keeps_its_failing_status() {
    let body = format!("{LINES}\n$fatal(1, \"boom\");");
    let run = run_closing_early("fatal", &initial_only(&body), &[], false);
    assert_status(&run, 1);
}

#[test]
fn error_exit_keeps_its_failing_status() {
    let body = format!("{LINES}\n$error(\"bad\");\n$finish;");
    let run = run_closing_early("error_exit", &initial_only(&body), &["--error-exit"], false);
    assert_status(&run, 1);
}

#[test]
fn stderr_on_the_same_closed_pipe_does_not_abort() {
    let body = format!("{LINES}\nfor (int i = 0; i < 200; i++) $warning(\"w%0d\", i);\n$finish;");
    let run = run_closing_early("merged", &initial_only(&body), &["--report-stats"], true);
    assert_status(&run, 0);
}

#[test]
fn the_stats_footer_to_a_closed_pipe_does_not_abort() {
    let body = format!("{LINES}\n$finish;");
    let run = run_closing_early("stats", &initial_only(&body), &["--report-stats"], false);
    assert_status(&run, 0);
}

#[test]
fn the_waveform_is_complete_after_the_reader_closes() {
    let src = "module t;\n\
               logic [15:0] c = 0;\n\
               initial begin\n\
                 $dumpfile(\"w.vcd\"); $dumpvars(0, t);\n\
                 for (int i = 1; i <= 20000; i++) begin #1 c = i; $display(\"line %0d\", i); end\n\
                 $finish;\n\
               end\n\
               endmodule\n";
    let run = run_closing_early("wave", src, &["--wave"], false);
    assert_status(&run, 0);
    let vcd = std::fs::read_to_string(run.dir.join("w.vcd")).expect("VCD written");
    assert!(vcd.contains("$enddefinitions"), "VCD header");
    assert!(
        vcd.lines().any(|l| l == "#20000"),
        "VCD reaches the last time step"
    );
    assert!(
        vcd.lines().any(|l| l.starts_with("b100111000100000 ")),
        "VCD holds the last value of c (20000)"
    );
}
