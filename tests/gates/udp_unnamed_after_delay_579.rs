//! IEEE 1800-2017 §29.3 / A.2.2.3: a UDP instance may omit its name after a
//! bare delay (`my_mux #0 (z, a, b, s);`, also from a macro) or a drive
//! strength, in a list, with a fractional delay, and the delay reaches it
//! as it does a named instance (#579, reported by git-hevangel). The bare
//! delay was parsed as an expression that took the terminal list as a call
//! on the literal, and the strength was taken for the terminal list. A
//! nameless MODULE instance after a delay is still rejected (§23.3.2).

use std::path::Path;
use std::process::Command;

fn run(file: &str) -> (bool, String) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--simulate")
        .arg("-s")
        .arg("t")
        .arg("--no-cache")
        .arg(path.to_str().unwrap())
        .output()
        .expect("Failed to execute xezim");
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), text)
}

#[test]
fn unnamed_udp_after_delay_or_strength() {
    let (ok, text) = run("tests/gates/udp_unnamed_after_delay_579.sv");
    assert!(ok, "{text}");
    assert!(!text.contains("FAIL"), "{text}");
    assert!(text.contains("ALL PASS"), "{text}");
}

#[test]
fn unnamed_module_instance_after_delay_is_rejected() {
    let (ok, text) = run("tests/gates/udp_unnamed_after_delay_579_guard.sv");
    assert!(!ok, "must not compile:\n{text}");
    assert!(text.contains("§23.3.2"), "{text}");
    assert!(!text.contains("FAIL:"), "{text}");
}
