use std::path::Path;
use std::process::Command;

/// §6.21/§13.3.1: a `static` subroutine local is ONE live storage cell shared
/// across all simultaneous activations — including re-entrant/recursive ones.
///
/// Regression: xezim previously kept a per-recursion-frame copy of the static,
/// restored at entry and written back on return, so a recursion window's
/// mutation was invisible to the caller frames and the static was effectively
/// re-initialised to its declaration value on every re-entry. That reset UVM's
/// `static bit first` in `main_phase` on every `phase.jump(reset)` re-entry
/// (the phase-start/end-sequence case), livelocking the phase scheduler.
///
/// This recursive method accumulates into a shared static; the outermost
/// activation must read 4 after four increments. Matched byte-for-byte against
/// the reference simulator:
///   TAG_INNER4 depth=4 counter=1 saved=1
///   TAG_OUTER4 depth=4 counter=4 saved=1
///   TAG_PASS counter=4
#[test]
fn static_recursion_shared_cell() {
    let test_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scheduling");
    let test_file = test_dir.join("static_recursion_shared_cell.sv");
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
        combined.contains("TAG_PASS counter=4"),
        "static local was not shared across recursion (got a fresh frame per re-entry).\nOutput:\n{combined}"
    );
    assert!(
        !combined.contains("TAG_FAIL"),
        "Test reported failure.\nOutput:\n{combined}"
    );
}