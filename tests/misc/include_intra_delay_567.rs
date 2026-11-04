//! IEEE 1800-2023 §9.4.5 / §22.4 / §22.5.1: an intra-assignment delay written
//! in an `include`d file, or in a macro defined in one, is a timing control
//! like one in the compiled file. The pre-parse rewrite that canonicalizes
//! `lhs = #d rhs` ran over the command-line sources only, so delays spliced
//! in by the preprocessor were dropped: the `always` was rejected as having
//! no timing control, and a macro's delay let the assignment land at once
//! (#567, reported by git-hevangel).

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
fn include_intra_delay_567_0() {
    run("include_intra_delay_567/repro.sv", &["+incdir+tests/misc/include_intra_delay_567/inc"], 4);
}

#[test]
fn include_intra_delay_567_1() {
    run("include_intra_delay_567/repro_macro.sv", &["+incdir+tests/misc/include_intra_delay_567/inc"], 4);
}

/// The `--compile` path rewrites the preprocessed texts the same way, so a
/// compiled artifact of the `include`d design simulates like the source.
#[test]
fn include_intra_delay_567_compiled() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let scratch = std::env::temp_dir().join(format!("xezim_567_{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    let artifact = scratch.join("repro.xez");
    let compile = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(root)
        .args(["--compile", "--no-cache", "-s", "t", "-o"])
        .arg(&artifact)
        .arg("+incdir+tests/misc/include_intra_delay_567/inc")
        .arg(root.join("tests/misc/include_intra_delay_567/repro.sv"))
        .output()
        .expect("Failed to execute xezim");
    let compile_out = format!(
        "{}\n{}",
        String::from_utf8_lossy(&compile.stdout),
        String::from_utf8_lossy(&compile.stderr)
    );
    assert!(compile.status.success(), "compile failed:\n{compile_out}");
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(root)
        .arg("--simulate")
        .arg(&artifact)
        .output()
        .expect("Failed to execute xezim");
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let pass = combined.lines().filter(|l| l.starts_with("PASS")).count();
    let fail = combined.lines().filter(|l| l.starts_with("FAIL")).count();
    assert!(fail == 0, "compiled run:\n{combined}");
    assert!(pass >= 4, "compiled run: {pass} PASS lines, expected 4:\n{combined}");
}
