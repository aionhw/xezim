//! CLI transcript routing and VCD compatibility; §21.2/§21.3 output streams.
#![cfg(unix)]
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("xezim_cli347_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}
fn run(path: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--no-cache", "top.sv"])
        .args(args)
        .current_dir(path)
        .output()
        .unwrap()
}
const FATAL: &str = r#"module top;
 initial begin
  $display("M1_display");
  $fdisplay(32'h8000_0001,"M2_fd_stdout");
  $fdisplay(1,"M3_mcd1");
  $fdisplay(32'h8000_0002,"M4_fd_stderr");
  $error("M5_error");
  $write("M6_write_no_newline");
  #1 $fatal(1,"M7_fatal");
 end
endmodule"#;

#[test]
fn tee_keeps_fatal_transcript_and_separate_console_streams() {
    let path = scratch("fatal");
    std::fs::write(path.join("top.sv"), FATAL).unwrap();
    let output = run(&path, &["--tee", "run.log"]);
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let log = std::fs::read_to_string(path.join("run.log")).unwrap();
    for marker in [
        "M1_display",
        "M2_fd_stdout",
        "M3_mcd1",
        "M6_write_no_newline",
    ] {
        assert!(stdout.contains(marker), "{stdout}");
        assert!(!stderr.contains(marker), "{stderr}");
    }
    assert!(stderr.contains("M4_fd_stderr"), "{stderr}");
    assert!(!stdout.contains("M4_fd_stderr"), "{stdout}");
    for marker in [
        "M1_display",
        "M2_fd_stdout",
        "M3_mcd1",
        "M4_fd_stderr",
        "M5_error",
        "M6_write_no_newline",
        "M7_fatal",
    ] {
        assert!(log.contains(marker), "{log}");
    }
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn file_only_log_remains_available() {
    let path = scratch("file_only");
    std::fs::write(path.join("top.sv"), FATAL).unwrap();
    let output = run(&path, &["-l", "run.log"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
    assert!(
        std::fs::read_to_string(path.join("run.log"))
            .unwrap()
            .contains("M7_fatal")
    );
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn vcd_compatibility_consumes_optional_dumpon() {
    for (tag, args) in [("vcd", vec!["-vcd"]), ("vcd_on", vec!["-vcd", "dumpon"])] {
        let path = scratch(tag);
        std::fs::write(
            path.join("top.sv"),
            r#"module top;
logic clk=0;
initial begin $dumpfile("wave.vcd"); $dumpvars(0,top); #1 clk=1; #1 $finish; end
endmodule"#,
        )
        .unwrap();
        let output = run(&path, &args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!String::from_utf8_lossy(&output.stderr).contains("unknown flag"));
        let wave = std::fs::read_to_string(path.join("wave.vcd")).unwrap();
        assert!(wave.contains("$enddefinitions"), "{wave}");
        std::fs::remove_dir_all(path).unwrap();
    }
}

fn c_output(mode: &str) {
    let path = scratch(mode);
    std::fs::write(
        path.join("top.sv"),
        "module top; initial begin #2 $finish; end endmodule",
    )
    .unwrap();
    std::fs::write(
        path.join("output.c"),
        r#"
#include <stdio.h>
#include <stdlib.h>
#include <signal.h>
#include <sys/resource.h>
#ifdef __linux__
#include <sys/prctl.h>
#endif
#include "vpi_user.h"
static char buffer[4096];
static void final_output(void) { fputs("C_exit_stdout\n",stdout); fputs("C_exit_stderr\n",stderr); }
static PLI_INT32 callback(p_cb_data ignored) {
 (void)ignored;
 const char *mode=getenv("OUTPUT_MODE");
 if(mode && mode[0]=='b') {
  setvbuf(stdout,buffer,_IOFBF,sizeof buffer);
  fputs("C_buffered_stdout\n",stdout);
  atexit(final_output);
  vpi_control(vpiFinish,0);
 } else {
  fputs("C_signal_stdout\n",stdout); fputs("C_signal_stderr\n",stderr);
  struct rlimit limit={0,0}; setrlimit(RLIMIT_CORE,&limit);
#ifdef __linux__
  prctl(PR_SET_DUMPABLE,0);
#endif
  if(mode && mode[0]=='a') abort();
  else if(mode && mode[0]=='g') kill(0,SIGTERM);
  else raise(SIGTERM);
 }
 return 0;
}
static void boot(void) {
 s_cb_data cb={0}; cb.reason=cbReadWriteSynch; cb.cb_rtn=callback;
 vpi_register_cb(&cb);
}
void (*vlog_startup_routines[])(void)={boot,0};
"#,
    )
    .unwrap();
    let output = Command::new(std::env::var_os("CC").unwrap_or_else(|| "clang".into()))
        .args(["-shared", "-fPIC", "-Wall", "-Wextra", "-I"])
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("include"))
        .args(["output.c", "-o", "output.so"])
        .current_dir(&path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let command = |tee: bool| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_xezim"));
        command
            .args(["--no-cache", "top.sv", "--vpi-lib", "./output.so"])
            .env("OUTPUT_MODE", mode)
            .current_dir(&path);
        if tee {
            command.arg("--tee=run.log");
        }
        if mode == "group" {
            use std::os::unix::process::CommandExt;
            unsafe {
                command.pre_exec(|| {
                    if libc::setsid() < 0 {
                        Err(std::io::Error::last_os_error())
                    } else {
                        Ok(())
                    }
                });
            }
        }
        command
    };
    let baseline = command(false).output().unwrap();
    let output = command(true).output().unwrap();
    assert_eq!(
        output.status, baseline.status,
        "--tee must preserve the simulator's exit status"
    );
    let log = std::fs::read_to_string(path.join("run.log")).unwrap();
    if mode == "buffered" {
        assert!(output.status.success(), "{log}");
        for marker in ["C_buffered_stdout", "C_exit_stdout", "C_exit_stderr"] {
            assert!(log.contains(marker), "{log}");
        }
    } else {
        if mode == "abort" {
            assert!(!output.status.success());
        }
        assert!(
            log.contains("C_signal_stdout") && log.contains("C_signal_stderr"),
            "{log}"
        );
    }
    std::fs::remove_dir_all(path).unwrap();
}
#[test]
fn tee_drains_buffered_c_output_and_exit_handlers() {
    c_output("buffered");
}
#[test]
fn tee_keeps_c_output_before_abort() {
    c_output("abort");
}
#[test]
fn tee_keeps_c_output_before_termination() {
    c_output("terminate");
}

#[test]
fn tee_drains_output_after_group_termination() {
    c_output("group");
}
