use std::path::Path;
use std::process::Command;

/// Pure-SystemVerilog self-test for process semantics reached through a
/// CLASS MEMBER handle under concurrency (IEEE 1800-2023 §5.7.1/§15.5).
///
/// Two aspects are covered:
///
/// 1. `obj.watcher_proc.kill()` — a method call on a flattened 3-segment
///    receiver (`Ident([obj, watcher_proc, kill])`) where the walked-to
///    receiver is an opaque PROCESS token (≥ PROCESS_HANDLE_BASE, beyond
///    the object heap). The call must dispatch to the process-method
///    interceptor (kill/await/suspend/resume); previously the heap-object
///    guard rejected the token and the call silently no-opped, so a killed
///    watcher kept running and incremented its counter.
///
/// 2. A task FORMAL of class type (`task automatic worker(Obj obj)`) must
///    keep its class-type metadata while another invocation of the same
///    task is still suspended at `#20`. The first worker's return used to
///    strip `obj` from the shared class-type maps, so the second worker's
///    `obj.data = 42` no-opped (`receiver_may_be_handle` false) — the
///    write was lost even though nothing killed it.
///
/// Reference-verified semantics: the awaited worker's counter increments
/// (trig1 == 1); the killed watcher's counter must NOT (trig2 == 0); BOTH
/// workers complete their data writes (data1 == data2 == 42).
#[test]
fn process_kill_via_member_handle() {
    let test_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let test_file = test_dir.join("classes/process_kill_via_member_handle.sv");
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
        "Parse error in process_kill_via_member_handle.sv:\n{combined}"
    );
    assert!(
        !combined.contains("Simulation error"),
        "Simulation error in process_kill_via_member_handle.sv:\n{combined}"
    );
    assert!(
        combined.contains("TAG_PASS"),
        "Test did not pass (killed watcher still ran, or concurrent task formal lost its class type).\nOutput:\n{combined}"
    );
}
