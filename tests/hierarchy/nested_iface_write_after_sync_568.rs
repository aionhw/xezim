//! IEEE 1800-2023 §10.4.1 / §23.6 / §25.9: a blocking write to the whole of a
//! variable in an interface instance nested two levels down, by hierarchical
//! name or through a virtual interface, updates the variable before the next
//! statement — also after a `$monitor` or a `$readmemh` has filled the
//! name-keyed signal copy. The dotted-name fallback wrote that copy instead
//! of the signal table, and the next sync discarded it (#568, reported by
//! git-hevangel).

use std::path::Path;
use std::process::Command;

fn run(design: &str, extra: &[&str], expect_pass: usize) {
    let test_file = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/hierarchy/{design}", ));
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_xezim"));
    // The design writes and reads `repro_mem.hex` in the working
    // directory: run in a scratch dir so no file is left in the tree.
    let scratch = std::env::temp_dir().join(format!("xezim_568_{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    cmd.current_dir(&scratch)
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
fn nested_iface_write_after_sync_568_0() {
    run("nested_iface_write_after_sync_568.sv", &[], 12);
}

#[test]
fn nested_iface_write_after_sync_568_1() {
    run("nested_iface_write_after_sync_568.sv", &["+define+READMEM"], 12);
}

#[test]
fn nested_iface_write_after_sync_568_2() {
    run("nested_iface_write_after_sync_568.sv", &["+define+NONE"], 12);
}
