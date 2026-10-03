use std::path::Path;
use std::process::Command;

/// Pure-SystemVerilog self-test for late-region same-slot event delivery.
///
/// A process resumed from an NBA-region wait (the `nba <= v; @(nba)` idiom
/// of `uvm_wait_for_nba_region`: package-level task, static task-locals) is
/// resumed by the tick's late-region drains — after the tick's main edge
/// pass. When that resumed process fires a named event, the `@(e)` waiter
/// must still wake in the SAME time slot (IEEE 1800-2017 4.4/4.7: the
/// Re-NBA/reactive regions are part of the same slot). A bare event flip
/// schedules nothing by itself, so without a post-drain edge/waiter re-pass
/// the waiter only wakes in the NEXT slot's edge pass — one full time unit
/// late, which is how a sequencer grant handshake made a register block
/// backdoor mirror read X (its DUT write landed after the model check).
/// Reference-verified: the waiter observes t == 1.
#[test]
fn nba_region_event_same_slot() {
    let test_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let test_file = test_dir.join("classes/nba_region_event_same_slot.sv");
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
        "Parse error in nba_region_event_same_slot.sv:\n{combined}"
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
