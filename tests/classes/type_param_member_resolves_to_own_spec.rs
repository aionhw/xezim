use std::path::Path;
use std::process::Command;

/// Pure-SystemVerilog self-test for type-parameter MEMBER resolution under
/// concurrency.
///
/// A method-driven member whose declared type is a class type parameter
/// (e.g. `REQ req;` in `uvm_sequence #(trans)`) must resolve against the
/// owning INSTANCE's own concrete `extends #(...)` specialization, not the
/// ambient `current_spec`. `current_spec` is a single mutable field that a
/// concurrent fork leaves set to whichever method ran last; a parameterized
/// component's blocking `drive()` leaks `component #(item_b)` into it. If
/// that leaked spec outranks the instance's own class chain, a `sub_a
/// extends seq_param #(item_a)` member `req` wrongly resolves to `item_b`
/// and a static accessor on it reports the wrong name — the same failure
/// as a top sequence emitting a bot item (UVM `SQRSNDREQCAST`).
#[test]
fn type_param_member_resolves_to_own_spec() {
    let test_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let test_file = test_dir.join("classes/type_param_member_resolves_to_own_spec.sv");
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
        "Parse error in type_param_member_resolves_to_own_spec.sv:\n{combined}"
    );
    assert!(
        !combined.contains("Simulation error"),
        "Simulation error in type_param_member_resolves_to_own_spec.sv:\n{combined}"
    );
    assert!(
        combined.contains("TAG_PASS"),
        "Test did not pass (member resolved to leaked ambient spec).\nOutput:\n{combined}"
    );
}