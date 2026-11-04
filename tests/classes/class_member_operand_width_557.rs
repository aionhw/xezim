use std::path::Path;
use std::process::Command;

/// Pure-SystemVerilog self-test: the operands of a context-determined comparison inside a class method must widen to the comparison context instead of truncating at their self-determined width when the operand is a class member, a handle select, a packed-struct field, an array element, a static or package variable, or a virtual-interface member (SS11.6.1/11.8.2).
/// Verified byte-for-byte against reference simulators.
#[test]
fn class_member_operand_width() {
    let test_file =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/classes/class_member_operand_width_557.sv");
    assert!(
        test_file.exists(),
        "Test file not found: {}",
        test_file.display()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--simulate")
        .arg("-s")
        .arg("top")
        .arg(test_file.to_str().unwrap())
        .output()
        .expect("Failed to execute xezim");
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !combined.contains("Parse errors"),
        "Parse error:\n{combined}"
    );
    assert!(
        !combined.contains("TAG_FAIL"),
        "class_member_operand_width failed:\n{combined}"
    );
    assert!(
        combined.contains("TAG_PASS"),
        "class_member_operand_width: no TAG_PASS.\nOutput:\n{combined}"
    );
}