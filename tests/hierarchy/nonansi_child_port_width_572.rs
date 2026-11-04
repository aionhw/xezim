//! IEEE 1800-2023 §23.2.2.1 / §23.3.3: an input port also passed to a
//! sub-instance declared with a non-ANSI header keeps its declared width
//! and packed-struct layout. The body-driven scan read a child's port
//! directions from an ANSI header only, so every actual of a non-ANSI
//! child counted as driven and the port lost the shape rules of its own
//! connection (#572, reported by git-hevangel).

use std::path::Path;
use std::process::Command;

fn run(design: &str, extra: &[&str], expect_pass: usize) {
    let test_file = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/hierarchy/{design}", ));
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    cmd.current_dir(env!("CARGO_MANIFEST_DIR"))
        .arg("--simulate")
        .arg("-s")
        .arg("t")
        .arg("--no-cache")
        .args(extra)
        .arg(test_file.to_str().unwrap());
    let output = cmd.output().expect("Failed to execute xezim");
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let pass = combined.lines().filter(|l| l.starts_with("PASS")).count();
    let fail = combined.lines().filter(|l| l.starts_with("FAIL")).count();
    assert!(fail == 0, "{design} {extra:?}:\n{combined}");
    assert!(pass >= expect_pass, "{design} {extra:?}: {pass} PASS lines, expected {expect_pass}:\n{combined}");
}

#[test]
fn nonansi_child_port_width_572() {
    run("nonansi_child_port_width_572.sv", &[], 8);
}
