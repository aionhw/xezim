use std::path::Path;
use std::process::Command;

/// Pure-SystemVerilog self-test: a method call whose receiver is an index
/// select of a local collection (`regs[ridx].m()` — the register-kit
/// hw-reset-sequence shape) must run through the suspend-aware splice path
/// (IEEE 1800-2017 §13.3).
///
/// Stage-1b splicing skipped calls whose receiver was not a plain
/// name/member chain; the synchronous fallback then evaluated the callee's
/// level-sensitive `wait` non-blockingly and every element completed at
/// t=0 — the failure behind the sequencer "concurrent calls to
/// get_next_item()" / mirrored-value-mismatch regression in the UVM
/// register kit. With the fix, both elements park until the gate opens at
/// t=5 (verified byte-for-byte against reference simulators).
#[test]
fn queue_elem_receiver_splice() {
    let test_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let test_file = test_dir.join("classes/queue_elem_receiver_splice.sv");
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
        "Parse error in queue_elem_receiver_splice.sv:\n{combined}"
    );
    assert!(
        !combined.contains("Simulation error"),
        "Simulation error in queue_elem_receiver_splice.sv:\n{combined}"
    );
    assert!(
        combined.contains("TAG_PASS"),
        "Method call through a queue-element receiver ran synchronously.\nOutput:\n{combined}"
    );
}
