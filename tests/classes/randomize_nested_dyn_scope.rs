//! IEEE 1800-2023 §18.12 / §7.4.5: `std::randomize` of a variable with a
//! dynamic or queue outer dimension over unpacked rows, at module scope and
//! as automatic task locals. The scope solve split `raw[][2]` into rows over
//! the 64-slot buffer, so `raw.size() == 2` read 64 and failed, and took
//! `int d[][]` for a dynamic array of scalars. Fixed rows now follow the
//! current size, dynamic rows (rows already sized, or a constraint calling
//! `size()` on one) join as arrays of their own, and the outer size, then
//! each row's size, are solved before the elements.

use std::path::Path;
use std::process::Command;

#[test]
fn std_randomize_solves_nested_dynamic_arrays() {
    let test_file =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/classes/randomize_nested_dyn_scope.sv");
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
    assert!(combined.contains("TEST_PASS"), "no TEST_PASS:\n{combined}");
}
