//! Automated runner for the SV-2023 compliance testsuite.
//!
//! The suite lives at `../sv2023_compliance_testsuite/`. Each test is a
//! standalone SV file that prints `SVTEST_PASS` on success and
//! `SVTEST_FAIL` on failure.

use std::path::{Path, PathBuf};
use std::process::Command;

fn testsuite_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xezim/Cargo.toml has a parent directory")
        .join("sv2023_compliance_testsuite")
}

fn run_sv2023_positive(category: &str, filename: &str) {
    let root = testsuite_root();
    let test_file = root.join("tests").join(category).join(filename);
    let common = root.join("tests").join("common");
    // The SV-2023 compliance suite is an OPTIONAL sibling checkout, not part of
    // the xezim repo. When it is absent, skip gracefully (treat as a no-op pass)
    // rather than failing the whole `cargo test` run.
    if !test_file.exists() {
        eprintln!(
            "[skip] SV-2023 testsuite not present ({} missing); \
             clone ../sv2023_compliance_testsuite/ to run {}.",
            test_file.display(),
            filename
        );
        return;
    }

    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--sv2023")
        .arg("-I")
        .arg(common.to_str().unwrap())
        .arg(test_file.to_str().unwrap())
        .output()
        .expect("Failed to execute xezim");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "{} exited with {:?}. Output:\n{}{}",
        filename,
        output.status.code(),
        stdout,
        stderr
    );
    assert!(
        !stdout.contains("Parse errors"),
        "Parse error in {}:\n{}",
        filename,
        stdout
    );
    assert!(
        !stderr.contains("Simulation error"),
        "Simulation error in {}:\n{}{}",
        filename,
        stdout,
        stderr
    );
    assert!(
        stdout.contains("SVTEST_PASS") && !stdout.contains("SVTEST_FAIL"),
        "{} did not pass. Output:\n{}{}",
        filename,
        stdout,
        stderr
    );
}

fn run_sv2023_source(name: &str, source: &str) {
    // One directory per call: the tests run in parallel in one process, and
    // one test's cleanup must not delete another's source (#594).
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "xezim_sv2023_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("create temporary directory");
    let path = dir.join(name);
    std::fs::write(&path, source).expect("write temporary source");
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--sv2023")
        .arg("--no-cache")
        .arg(&path)
        .output()
        .expect("execute test source");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        output.status.success()
            && stdout.contains("SVTEST_PASS")
            && !stdout.contains("SVTEST_FAIL"),
        "{name} failed:\n{stdout}{stderr}"
    );
}

#[test]
fn sv2023_t100_triple_quoted_string() {
    run_sv2023_positive("10_sv2023", "t100_triple_quoted_string.sv");
}

#[test]
fn sv2023_t101_ref_static_nba_formal() {
    run_sv2023_positive("10_sv2023", "t101_ref_static_nba_formal.sv");
}

#[test]
fn sv2023_t102_preprocessor_ifdef_logical_ops() {
    run_sv2023_positive("10_sv2023", "t102_preprocessor_ifdef_logical_ops.sv");
}

#[test]
fn sv2023_t103_array_map_method() {
    run_sv2023_positive("10_sv2023", "t103_array_map_method.sv");
}

#[test]
fn sv2023_t104_array_method_index_argument() {
    run_sv2023_positive("10_sv2023", "t104_array_method_index_argument.sv");
}

#[test]
fn sv2023_t105_type_this_parameterized_class() {
    run_sv2023_positive("10_sv2023", "t105_type_this_parameterized_class.sv");
}

#[test]
fn sv2023_t106_soft_packed_union() {
    let external = testsuite_root()
        .join("tests/10_sv2023")
        .join("t106_soft_packed_union.sv");
    if external.exists() {
        run_sv2023_positive("10_sv2023", "t106_soft_packed_union.sv");
    } else {
        run_sv2023_source(
            "soft_packed_union.sv",
            r#"module top;
  typedef union soft packed {
    bit [7:0] byte_value;
    bit [15:0] half_value;
  } soft_u_t;
  soft_u_t value;
  initial begin
    if ($bits(value) != 16) begin $display("SVTEST_FAIL width"); $finish; end
    value.byte_value = 8'h5a;
    if (value.byte_value !== 8'h5a) begin $display("SVTEST_FAIL byte"); $finish; end
    value.half_value = 16'h1234;
    if (value.half_value !== 16'h1234) begin $display("SVTEST_FAIL half"); $finish; end
    $display("SVTEST_PASS");
    $finish;
  end
endmodule
"#,
        );
    }
}

#[test]
fn sv2023_t107_class_final_specifier_compile() {
    run_sv2023_positive("10_sv2023", "t107_class_final_specifier_compile.sv");
}

#[test]
fn sv2023_t108_method_initial_extends_final_specifiers() {
    run_sv2023_positive(
        "10_sv2023",
        "t108_method_initial_extends_final_specifiers.sv",
    );
}

#[test]
fn sv2023_t109_rand_real_constraint() {
    run_sv2023_positive("10_sv2023", "t109_rand_real_constraint.sv");
}

#[test]
fn sv2023_t110_timeunit_timeprecision_system_functions() {
    run_sv2023_positive(
        "10_sv2023",
        "t110_timeunit_timeprecision_system_functions.sv",
    );
}

#[test]
fn sv2023_t111_inside_tolerance_range() {
    run_sv2023_positive("10_sv2023", "t111_inside_tolerance_range.sv");
}

#[test]
fn sv2023_t112_parameter_associative_array() {
    run_sv2023_positive("10_sv2023", "t112_parameter_associative_array.sv");
}
