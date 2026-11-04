//! IEEE 1800-2023 §7.4.5 / §7.5 / §18.4 / §21.2.1.7: a class member with a
//! dynamic or queue outer dimension over unpacked rows (`T a[][N]`,
//! `T a[][N][M]`, `T a[$][N]`, `T a[][]`). The member was a dynamic array
//! of scalars: `o.raw[0][1] = 9` wrote nothing, `d[1] = new[3]` left one
//! scalar, `%p` printed scalars, the copy constructor dropped the rows, and
//! a constraint on a row element could not be solved (#576 section B).

use std::path::Path;
use std::process::Command;

#[test]
fn dynamic_members_over_unpacked_rows() {
    let test_file =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/classes/nested_dyn_members.sv");
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
