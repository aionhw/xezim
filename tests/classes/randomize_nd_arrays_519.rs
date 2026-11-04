//! IEEE 1800-2023 §18.4 / §18.12: multi-dimensional unpacked arrays are
//! randomize targets, in `std::randomize(t, a)` (static and automatic
//! locals, 2-D and 3-D, with a module-scope scalar) and as `rand` class
//! members with inline constraints. Each array joins the joint solve as
//! one row per prefix of its leading dimensions; `foreach` walks the
//! declared bounds and `a[i][j]` names the row's element. The class solve
//! no longer bars such members, a fixed multi-dimensional local keeps its
//! full shape at declaration, a nested `foreach` body is checked with its
//! loop variables bound, and a class solve no longer mints a bare element
//! copy that a later object's member write would take (#519, reported by
//! Ganesh-TNL; 61 checks, all cells kept in order).

use std::path::Path;
use std::process::Command;

#[test]
fn multi_dimensional_arrays_randomize_in_scope_and_class_solves() {
    let test_file =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/classes/randomize_nd_arrays_519.sv");
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .arg("--simulate")
        .arg("-s")
        .arg("tb")
        .arg("--no-cache")
        .arg(test_file.to_str().unwrap())
        .output()
        .expect("Failed to execute xezim");
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!combined.contains("CHECK_FAIL"), "{combined}");
    assert!(!combined.contains("CHECKS_MISSING"), "{combined}");
    assert!(combined.contains("TEST_PASS"), "no TEST_PASS:\n{combined}");
}
