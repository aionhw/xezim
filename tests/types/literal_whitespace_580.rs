//! IEEE 1800-2017 §5.7.1 / §5.3: white space, newlines included, may
//! separate the size of a based literal from its base and the base from its
//! value (#580, reported by git-hevangel). A space before the apostrophe
//! dropped the size, so `2 'd1` was 32 bits wide and `3 'd 9` read 9, with no
//! message; a newline there was a parse error.

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
fn space_before_the_base_keeps_the_size() {
    let (ok, text) = run("tests/types/literal_space_before_base_580.sv");
    assert!(ok, "{text}");
    assert!(!text.contains("FAIL"), "{text}");
    assert!(text.contains("ALL PASS"), "{text}");
}

#[test]
fn newline_inside_a_based_literal() {
    let (ok, text) = run("tests/types/literal_newline_in_based_580.sv");
    assert!(ok, "{text}");
    assert!(!text.contains("FAIL"), "{text}");
    assert!(text.contains("ALL PASS"), "{text}");
}
