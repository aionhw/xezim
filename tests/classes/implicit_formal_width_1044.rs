//! §13.5.1 / §11.8.2: implicit integral method formals retain their
//! declared width for narrow, wide, signed and expression actuals (#1044).

use std::process::Command;

fn run(compiled: bool) {
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--simulate", "--no-cache", "-s", "top"])
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/classes/implicit_formal_width_1044.sv"
        ))
        .env("XEZIM_COMPILE_METHODS", if compiled { "1" } else { "0" })
        .env("XEZIM_METHOD_TIER", "0")
        .env_remove("XEZIM_METHOD_CACHE")
        .output()
        .expect("run implicit-formal regression");
    let transcript = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{transcript}");
    assert!(!transcript.contains("FAIL"), "{transcript}");
    assert!(
        transcript.contains("PASS implicit formal widths"),
        "{transcript}"
    );
}

#[test]
fn interpreted_implicit_formal_widths() {
    run(false);
}

#[test]
fn compiled_implicit_formal_widths() {
    run(true);
}
