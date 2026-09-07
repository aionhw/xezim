use std::path::Path;
use std::process::Command;

/// IEEE 1800-2023 §6.21 / §13.3.1: a `static` subroutine local is ONE persistent
/// storage cell across all invocations.
///
/// In a phase-jump / re-entry pattern, a task suspends (e.g. `#10`), concurrent
/// processes execute and suspend inside subroutines, and the task mutates its
/// static local (`first = 0`) before jumping back to restart the schedule.
///
/// Previously, `static_local_syncs` was not preserved across process suspension
/// in `ProcessContext`. When a background process suspended inside a subroutine,
/// its un-popped sync frame polluted `Simulator.static_local_syncs`. When the
/// main task resumed after `#10`, `static_local_key_for` consulted the wrong
/// frame, so the write `first = 0` was dropped from `static_local_vars`.
/// Consequently, on re-entry the task re-read `first == 1` and spun forever.
///
/// With `static_local_syncs` tracked per-process in `ProcessContext`, the
/// mutation latches cleanly through the jump, `main_phase` runs a second time,
/// observes `first == 0`, and completes in 2 cycles.
///
/// Validated byte-for-byte against the reference simulator:
///   TAG_FIRST at 10 first=1
///   TAG_SECOND at 21 first=0
///   TAG_PASS latched across jump (run_count=2, cycles=2)
#[test]
fn phase_jump_static_latch() {
    let test_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scheduling");
    let test_file = test_dir.join("phase_jump_static_latch.sv");
    assert!(test_file.exists(), "Test file not found: {}", test_file.display());

    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--simulate")
        .arg("-s")
        .arg("top")
        .arg(test_file.to_str().unwrap())
        .output()
        .expect("Failed to execute xezim");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");

    assert!(
        combined.contains("TAG_PASS latched across jump"),
        "static local did not latch across jump (infinite loop or stale value).\nOutput:\n{combined}"
    );
    assert!(
        !combined.contains("TAG_FAIL"),
        "Test reported failure.\nOutput:\n{combined}"
    );
}
