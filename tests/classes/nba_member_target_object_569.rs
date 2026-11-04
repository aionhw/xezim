//! IEEE 1800-2023 §10.4.2 / §4.9.4: a nonblocking assignment to a member or
//! element of a class object updates the object named when the statement
//! executed. The slow NBA queue kept the root as a NAME and resolved it in
//! the NBA region, where a task formal or local no longer exists, so every
//! write landed in whichever task's frame the event loop still held (#569,
//! reported by git-hevangel).

use std::path::Path;
use std::process::Command;

fn run(design: &str, extra: &[&str], expect_pass: usize) {
    let test_file = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/classes/{design}", ));
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
fn nba_member_target_object_569() {
    run("nba_member_target_object_569.sv", &[], 6);
}
