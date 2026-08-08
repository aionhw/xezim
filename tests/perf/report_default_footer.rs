//! Test for always-on default footer (no flag required)
//!
//! Verifies that both compile-only and simulation runs print footer blocks.

use std::process::Command;

#[test]
fn default_footer_compile_only() {
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--compile", "tests/fixtures/tiny.sv"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let combined = format!("{}{}", stdout, stderr);

    // Should print "Compilation Performance Summary" block with dashed separators
    assert!(
        combined.contains("Compilation Performance Summary"),
        "expected compilation footer block in compile-only run:\n{}",
        combined
    );
    assert!(
        combined.contains("--------------------------------------------------------"),
        "expected dashed separator:\n{}",
        combined
    );
    // Should contain machine name / hostname
    assert!(
        combined.contains("Machine name") || combined.contains("host") || combined.contains("hostname"),
        "expected machine/host info in footer:\n{}",
        combined
    );
}

#[test]
fn default_footer_simulation() {
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["tests/fixtures/tiny.sv"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let combined = format!("{}{}", stdout, stderr);

    // Should print BOTH compilation and simulation blocks
    assert!(
        combined.contains("Compilation Performance Summary"),
        "expected compilation footer in simulation run:\n{}",
        combined
    );
    assert!(
        combined.contains("Simulation Performance Summary"),
        "expected simulation footer in simulation run:\n{}",
        combined
    );
    assert!(
        combined.contains("--------------------------------------------------------"),
        "expected dashed separator:\n{}",
        combined
    );
    // Should have "Simulation finished at <time> ns (<time> ms)" line
    assert!(
        combined.contains("Simulation finished at") && combined.contains("ns"),
        "expected 'Simulation finished at' line:\n{}",
        combined
    );
}