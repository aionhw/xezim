use std::path::Path;
use std::process::Command;

/// Pure-SystemVerilog self-test: whole-collection equality must not depend
/// on the construction path of the operands (IEEE 1800-2017 §7.2, §11.4.5).
///
/// push_back'd narrow elements (byte/shortint) used to live in 32-bit
/// sign-extended slots while assignment-pattern elements lived at the
/// declared width; the collection `==` compared raw slots and reported
/// inequality (or x, for associative arrays) even though every element
/// compared equal. Reference simulators compare all these pairs equal.
#[test]
fn queue_eq_construction_path() {
    let test_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let test_file = test_dir.join("classes/queue_eq_construction_path.sv");
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
        "Parse error in queue_eq_construction_path.sv:\n{combined}"
    );
    assert!(
        !combined.contains("Simulation error"),
        "Simulation error in queue_eq_construction_path.sv:\n{combined}"
    );
    assert!(
        combined.contains("TAG_PASS"),
        "Collection equality depended on the construction path.\nOutput:\n{combined}"
    );
}