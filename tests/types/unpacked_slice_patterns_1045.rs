//! §7.4.5 / §10.9.1: slices receive assignment patterns element by
//! element, including instance arrays, directions and typed items (#1045).

use std::process::Command;

fn run(compiled: bool) {
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "--no-cache", "-s", "top"])
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/types/unpacked_slice_patterns_1045.sv"
        ))
        .env("XEZIM_COMPILE_METHODS", if compiled { "1" } else { "0" })
        .env("XEZIM_METHOD_TIER", "0")
        .env_remove("XEZIM_METHOD_CACHE")
        .output()
        .expect("run slice-pattern regression");
    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{transcript}");
    assert!(!transcript.contains("FAIL"), "{transcript}");
    assert!(
        transcript.contains("PASS unpacked slice patterns"),
        "{transcript}"
    );
}

#[test]
fn interpreted_slice_patterns() {
    run(false);
}

#[test]
fn compiled_slice_patterns() {
    run(true);
}
