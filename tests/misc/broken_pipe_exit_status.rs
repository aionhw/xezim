//! A reader that closes the pipe early (`xezim run.sv | head -1`) must not
//! abort the run. The CLI's own lines (the `Simulation finished` line, the
//! stats footer) went through `println!`, which panics when the write fails,
//! and `panic = "abort"` turned that into SIGABRT: exit 134 instead of the
//! run's own status, so a `$fatal` or `--error-exit` failure read as a crash
//! and a passing run as a failure. The run now ends with its usual status
//! (§20.10 for `$fatal`) and still writes its waveform. Process handling
//! only: no simulation output is compared.
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

fn xezim() -> String {
    // Cargo names the binary it built for the integration tests.
    env!("CARGO_BIN_EXE_xezim").to_string()
}

/// Enough output after the first line to fill the pipe, so the run is still
/// writing when the reader goes away.
const LINES: &str = "for (int i = 0; i < 50000; i++) $display(\"line %0d\", i);";

/// The standard pipe API provides portable, non-inheritable handles.
fn pipe() -> (std::io::PipeReader, std::io::PipeWriter) {
    std::io::pipe().expect("pipe")
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
    run_closing_early_with_stack(tag, src, args, merge, None)
}

fn run_closing_early_with_stack(
    tag: &str,
    src: &str,
    args: &[&str],
    merge: bool,
    stack_mb: Option<&str>,
) -> Run {
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
    let mut command = Command::new(xezim());
    if let Some(stack_mb) = stack_mb {
        command.env("XEZIM_STACK_MB", stack_mb);
    }
    let mut child = command
        .current_dir(&dir)
        .args(args)
        .arg(&path)
        .stdout(Stdio::from(write_end))
        .stderr(stderr)
        .spawn()
        .expect("run xezim");
    let mut reader = BufReader::new(read_end);
    let mut first = String::new();
    reader.read_line(&mut first).unwrap();
    drop(reader);
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait") {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("run did not terminate after its output reader closed");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let stderr = std::fs::read_to_string(&err_log).unwrap_or_default();
    Run {
        status,
        stderr,
        dir,
    }
}

fn assert_status(run: &Run, code: i32) {
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

/// §20.16: a runtime diagnostic still exits with failure when stderr is closed.
#[test]
fn missing_sdf_keeps_its_failing_status_on_a_closed_pipe() {
    let body = format!("{LINES}\n#1 $sdf_annotate(\"absent_input.sdf\");\n$finish;");
    // Direct execution also exposes a diagnostic panic in debug builds:
    // a worker's caught panic would otherwise resemble the expected exit 1.
    for stack in [None, Some("0")] {
        let run =
            run_closing_early_with_stack("missing_sdf", &initial_only(&body), &[], true, stack);
        assert_status(&run, 1);
    }
}

/// §20.16: parsing an invalid SDF reaches the library's diagnostic writer too.
#[test]
fn invalid_sdf_keeps_its_failing_status_on_a_closed_pipe() {
    let body = format!(
        "integer fd;\n{LINES}\n\
         fd = $fopen(\"bad.sdf\", \"w\");\n\
         $fdisplay(fd, \"not-an-sdf-file\"); $fclose(fd);\n\
         #1 $sdf_annotate(\"bad.sdf\");\n$finish;"
    );
    for stack in [None, Some("0")] {
        let run =
            run_closing_early_with_stack("invalid_sdf", &initial_only(&body), &[], true, stack);
        assert_status(&run, 1);
    }
}

/// §8.4: a null-receiver diagnostic must retain its failing status.
#[test]
fn null_receiver_keeps_its_failing_status_on_a_closed_pipe() {
    let src = format!(
        "class handler;\nfunction void act(); endfunction\nendclass\n\
         module t;\nhandler h;\ninitial begin\n{LINES}\n#1 h.act();\n$finish;\nend\nendmodule"
    );
    for stack in [None, Some("0")] {
        let run = run_closing_early_with_stack("null_receiver", &src, &[], true, stack);
        assert_status(&run, 1);
    }
}

/// The environment listing lives in the library and uses the same stdout policy.
#[test]
fn environment_listing_tolerates_a_closed_reader() {
    let run = run_closing_early(
        "env_listing",
        &initial_only("$finish;"),
        &["--show-env-avail"],
        false,
    );
    assert_status(&run, 0);
}
