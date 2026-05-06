//! Automated test runner for the bundled SystemVerilog compliance suite.
//! Positive tests are split between `tests/` and `tests_advanced/`.
//! Negative tests in `tests_negative/` are expected to fail in parse-only mode.

use std::path::{Path, PathBuf};
use std::process::Command;

fn compliance_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("sv_compliance")
}

fn run_positive_compliance_test(subdir: &str, filename: &str) {
    let test_file = compliance_root().join(subdir).join(filename);
    assert!(
        test_file.exists(),
        "Test file not found: {}",
        test_file.display()
    );

    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg(test_file.to_str().unwrap())
        .output()
        .expect("Failed to execute xezim");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "Positive test {} exited with {:?}. Output:\n{}{}",
        filename,
        output.status.code(),
        stdout,
        stderr
    );
    assert!(
        !stdout.contains("Parse errors"),
        "Parse error in {}: {}",
        filename,
        stdout
    );
    assert!(
        !stderr.contains("Simulation error"),
        "Simulation error in {}: {}{}",
        filename,
        stdout,
        stderr
    );
    assert!(
        stdout.contains("TEST_PASS") && !stdout.contains("TEST_FAIL"),
        "Test {} did not pass. Output:\n{}{}",
        filename,
        stdout,
        stderr
    );
}

fn run_negative_compliance_test(filename: &str) {
    let test_file = compliance_root().join("tests_negative").join(filename);
    assert!(
        test_file.exists(),
        "Test file not found: {}",
        test_file.display()
    );

    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--no-sim")
        .arg(test_file.to_str().unwrap())
        .output()
        .expect("Failed to execute xezim");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(
        !output.status.success(),
        "Negative test {} unexpectedly succeeded. Output:\n{}",
        filename,
        combined
    );
    assert!(
        combined.contains(": error:")
            || combined.contains("Parse errors")
            || combined.contains("Simulation error"),
        "Negative test {} failed without an error diagnostic. Output:\n{}",
        filename,
        combined
    );
}

#[test]
fn test_sv_01_lexical_identifiers() {
    run_positive_compliance_test("tests", "01_lexical_identifiers.sv");
}
#[test]
fn test_sv_02_preprocessor() {
    run_positive_compliance_test("tests", "02_preprocessor.sv");
}
#[test]
fn test_sv_03_literal_values() {
    run_positive_compliance_test("tests", "03_literal_values.sv");
}
#[test]
fn test_sv_04_data_types() {
    run_positive_compliance_test("tests", "04_data_types.sv");
}
#[test]
fn test_sv_05_aggregate_types() {
    run_positive_compliance_test("tests", "05_aggregate_types.sv");
}
#[test]
fn test_sv_06_arrays() {
    run_positive_compliance_test("tests", "06_arrays.sv");
}
#[test]
fn test_sv_07_operators_expressions() {
    run_positive_compliance_test("tests", "07_operators_expressions.sv");
}
#[test]
fn test_sv_08_assignments() {
    run_positive_compliance_test("tests", "08_assignments.sv");
}
#[test]
fn test_sv_09_control_flow() {
    run_positive_compliance_test("tests", "09_control_flow.sv");
}
#[test]
fn test_sv_10_tasks_functions() {
    run_positive_compliance_test("tests", "10_tasks_functions.sv");
}
#[test]
fn test_sv_11_modules_ports_params() {
    run_positive_compliance_test("tests", "11_modules_ports_params.sv");
}
#[test]
fn test_sv_12_generate() {
    run_positive_compliance_test("tests", "12_generate.sv");
}
#[test]
fn test_sv_13_packages_imports() {
    run_positive_compliance_test("tests", "13_packages_imports.sv");
}
#[test]
fn test_sv_14_interfaces_modports() {
    run_positive_compliance_test("tests", "14_interfaces_modports.sv");
}
#[test]
fn test_sv_15_processes_events() {
    run_positive_compliance_test("tests", "15_processes_events.sv");
}
#[test]
fn test_sv_16_classes_oop() {
    run_positive_compliance_test("tests", "16_classes_oop.sv");
}
#[test]
fn test_sv_17_randomization_constraints() {
    run_positive_compliance_test("tests", "17_randomization_constraints.sv");
}
#[test]
fn test_sv_18_assertions_basic() {
    run_positive_compliance_test("tests", "18_assertions_basic.sv");
}
#[test]
fn test_sv_19_covergroups_basic() {
    run_positive_compliance_test("tests", "19_covergroups_basic.sv");
}
#[test]
fn test_sv_20_clocking_blocks() {
    run_positive_compliance_test("tests", "20_clocking_blocks.sv");
}
#[test]
fn test_sv_21_strings_typedef_casts() {
    run_positive_compliance_test("tests_advanced", "21_strings_typedef_casts.sv");
}
#[test]
fn test_sv_22_array_methods() {
    run_positive_compliance_test("tests_advanced", "22_array_methods.sv");
}
#[test]
fn test_sv_23_events_mailboxes_semaphores() {
    run_positive_compliance_test("tests_advanced", "23_events_mailboxes_semaphores.sv");
}
#[test]
fn test_sv_24_fork_join_wait() {
    run_positive_compliance_test("tests_advanced", "24_fork_join_wait.sv");
}
#[test]
fn test_sv_25_checker_blocks() {
    run_positive_compliance_test("tests_advanced", "25_checker_blocks.sv");
}
#[test]
fn test_sv_26_specify_blocks() {
    run_positive_compliance_test("tests_advanced", "26_specify_blocks.sv");
}
#[test]
fn test_sv_27_constraints_advanced() {
    run_positive_compliance_test("tests_advanced", "27_constraints_advanced.sv");
}
#[test]
fn test_sv_28_sva_sequences_advanced() {
    run_positive_compliance_test("tests_advanced", "28_sva_sequences_advanced.sv");
}
#[test]
fn test_sv_29_coverage_cross_bins() {
    run_positive_compliance_test("tests_advanced", "29_coverage_cross_bins.sv");
}
#[test]
fn test_sv_30_let_construct() {
    run_positive_compliance_test("tests_advanced", "30_let_construct.sv");
}
#[test]
fn test_sv_31_user_defined_nettypes() {
    run_positive_compliance_test("tests_advanced", "31_user_defined_nettypes.sv");
}

#[test]
fn test_sv_neg01_duplicate_declaration() {
    run_negative_compliance_test("neg01_duplicate_declaration.sv");
}
#[test]
fn test_sv_neg02_undeclared_identifier() {
    run_negative_compliance_test("neg02_undeclared_identifier.sv");
}
#[test]
fn test_sv_neg03_const_write() {
    run_negative_compliance_test("neg03_const_write.sv");
}
#[test]
fn test_sv_neg04_nonconstant_generate_if() {
    run_negative_compliance_test("neg04_nonconstant_generate_if.sv");
}
#[test]
fn test_sv_neg05_bad_package_import() {
    run_negative_compliance_test("neg05_bad_package_import.sv");
}
#[test]
fn test_sv_neg06_bad_modport_drive() {
    run_negative_compliance_test("neg06_bad_modport_drive.sv");
}
#[test]
fn test_sv_neg07_bad_clocking_direction() {
    run_negative_compliance_test("neg07_bad_clocking_direction.sv");
}
#[test]
fn test_sv_neg08_bad_constraint_reference() {
    run_negative_compliance_test("neg08_bad_constraint_reference.sv");
}
