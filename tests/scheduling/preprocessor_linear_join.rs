//! IEEE 1800-2017 §22.5.1: macro actual arguments may span many lines.
//! Joining those lines must not repeatedly rescan the accumulated prefix.
use std::{
    fs,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[test]
fn large_multiline_macro_argument_preprocesses_within_budget() {
    let dir = std::env::temp_dir().join(format!("xezim_pp_linear_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let input = dir.join("large.sv");
    let output = dir.join("expanded.sv");
    let lines = 250_000;
    fs::write(
        &input,
        format!(
            "`define ID(x) x\nmodule top;\nlocalparam int N = `ID(\n{}  0);\nendmodule\n",
            "  1 +\n".repeat(lines)
        ),
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--preprocess"])
        .arg(&input)
        .stdout(Stdio::from(fs::File::create(&output).unwrap()))
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            let _ = fs::remove_dir_all(&dir);
            panic!("multiline macro preprocessing exceeded 30 seconds");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let expanded = fs::read_to_string(&output).unwrap();
    fs::remove_dir_all(&dir).unwrap();
    assert!(status.success());
    assert_eq!(expanded.matches("1 +").count(), lines);
    assert!(expanded.contains("0;"));
    assert!(expanded.contains("endmodule"));
    assert!(!expanded.contains("`ID"));
}

/// §22.5.1/§22.6: a literal backtick must not make preprocessing recursively
/// rejoin the conditional branches of an ordinary multiline call.
#[test]
fn quoted_backtick_with_conditional_argument_terminates() {
    let dir = std::env::temp_dir().join(format!("xezim_pp_quoted_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let input = dir.join("quoted.sv");
    let output = dir.join("expanded.sv");
    fs::write(&input, "`define KEEP wrong\nmodule top;\ninitial $display(\"literal `KEEP\",\n`ifdef PICK\n1\n`else\n2\n`endif\n);\nendmodule\n").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--preprocess")
        .arg(&input)
        .env("XEZIM_STACK_MB", "64")
        .stdout(Stdio::from(fs::File::create(&output).unwrap()))
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            let _ = fs::remove_dir_all(&dir);
            panic!("quoted-backtick preprocessing did not terminate");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let expanded = fs::read_to_string(&output).unwrap();
    fs::remove_dir_all(&dir).unwrap();
    assert!(status.success());
    let normalized = expanded.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        normalized.contains("$display(\"literal `KEEP\", 2 );"),
        "{normalized}"
    );
    assert!(!expanded.contains("`ifdef"));
    assert!(!expanded.contains("wrong"));
}
