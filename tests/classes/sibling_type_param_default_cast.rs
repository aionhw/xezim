use std::path::Path;
use std::process::Command;

/// Pure-SystemVerilog self-test for SIBLING type-parameter defaults and
/// $cast destination resolution through class members.
///
/// IEEE 1800-2023 §8.25.4(ii): a type-parameter default may reference a
/// SIBLING parameter (`class sqr_t#(type REQ=int, type RSP=REQ)`).
/// Instantiating `sqr_t#(item_a)` must bind RSP to the CONCRETE sibling
/// argument (`item_a`), not the literal name "REQ" — a $cast into a member
/// of declared type `sqr_t#(item_a)` compares the instance's type bindings,
/// and a literal "REQ" binding made every such cast fail (the UVM
/// `uvm_declare_p_sequencer` cast in `m_set_p_sequencer`).
///
/// Also covers $cast destination resolution for member paths: a member
/// whose declared type is a TYPE PARAMETER (`RSP r0;`) resolves through
/// the owning instance's bindings, and deep flattened paths
/// (`sq.p_sequencer.r0`) walk to the owning object — previously both
/// resolved to nothing, leaving the cast permissive (§8.16/§8.22).
#[test]
fn sibling_type_param_default_cast() {
    let test_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let test_file = test_dir.join("classes/sibling_type_param_default_cast.sv");
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
        "Parse error in sibling_type_param_default_cast.sv:\n{combined}"
    );
    assert!(
        !combined.contains("Simulation error"),
        "Simulation error in sibling_type_param_default_cast.sv:\n{combined}"
    );
    assert!(
        combined.contains("TAG_PASS"),
        "Test did not pass (sibling default unresolved, or member-path $cast dest unresolved).\nOutput:\n{combined}"
    );
}
