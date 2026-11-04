//! IEEE 1800-2023 §3.14.2: `-override_timescale` gives every scope one time
//! unit, so a bare `#868` in a package or compilation-unit task or class
//! method advances 868 units like one in a module. The forced-timescale
//! pass converted those delays to ticks and the delay-only pass then scaled
//! them a second time (#571, reported by git-hevangel).

use std::path::Path;
use std::process::Command;

fn run(design: &str, extra: &[&str], expect_pass: usize) {
    let test_file = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/misc/{design}", ));
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
fn override_timescale_package_delay_571_0() {
    run("override_timescale_package_delay_571.sv", &[], 6);
}

#[test]
fn override_timescale_package_delay_571_1() {
    run("override_timescale_package_delay_571.sv", &["-override_timescale=10ns/1ns"], 6);
}

#[test]
fn override_timescale_package_delay_571_2() {
    run("override_timescale_package_delay_571.sv", &["-override_timescale=1ns/1ps"], 6);
}
