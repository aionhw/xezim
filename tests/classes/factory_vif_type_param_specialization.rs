use std::path::Path;
use std::process::Command;

/// Pure-SystemVerilog self-test for parameterized factory + virtual-interface
/// type parameters through nested class scopes.
///
/// Covers the shapes a register-block UVC uses (all in one flow):
///   * a STATIC method of a nested `reg_#(env#(T))` typedef (static
///     methods keyed by the FULL specialization, including nesting through
///     sibling parameterized classes),
///   * `env#(virtual I)` — an interface type as class type parameter —
///     materializing a distinct specialization per interface type (and the
///     `#(virtual I)` argument form itself),
///   * a `T vif;` member (interface instance storage inside the class
///     clone), assigned via a nested-handle LHS (`e.d.vif = top.pif`) and
///     compared with `==` (`e.d.vif == top.pif`),
///   * a task call through a nested instance handle (`e.d.run()`) that
///     fires the interface's event (`vif.fire(); -> e`) — an `@(pif.e)`
///     waiter must wake.
///
/// Reference-verified byte-for-byte.
#[test]
fn factory_vif_type_param_specialization() {
    let test_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let test_file = test_dir.join("classes/factory_vif_type_param_specialization.sv");
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
    let combined = format!("{stdout}\n{stderr}");

    assert!(
        !combined.contains("Parse errors"),
        "Parse error in factory_vif_type_param_specialization.sv:\n{combined}"
    );
    assert!(
        combined.contains("TAG_PASS"),
        "Expected TAG_PASS in output:\n{combined}"
    );
    assert!(
        !combined.contains("TAG_FAIL"),
        "Unexpected TAG_FAIL in output:\n{combined}"
    );
}
