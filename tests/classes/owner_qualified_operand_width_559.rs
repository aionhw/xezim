use std::path::Path;
use std::process::Command;

/// Pure-SystemVerilog self-test: a class-STATIC operand of a context-determined
/// comparison must size from its DECLARATION, not from its own self-determined
/// width (SS11.6.1/11.8.2). Elaboration flattens `E::ta` into a single
/// identifier whose path is `["E","ta"]` rather than a `MemberAccess` node, so
/// every probe in the `Ident` arm of `lrm_self_width` -- signal table, packed
/// field, live receiver -- misses it, and `s === E::ta + E::tb` compared the
/// 9-bit 300 against an 8-bit 44. The fix reaches `class_prop_width_impl` with
/// no handle, so it stays declaration-only and cannot run a method (13.4).
///
/// Checks 1-2 are the bug; checks 3-6 are controls that already pass on main,
/// so the MWE cannot be satisfied by simply widening everything -- an 8-bit
/// comparison context must still wrap to 44 in module scope and inside a
/// method, and a package-qualified pair and a handle-rooted instance member must
/// stay untouched.
///
/// This branch is deliberately independent of the other issue-528 fixes and its
/// MWE asserts only what this change turns green. Two neighbouring shapes -- the
/// same comparison from inside a method, and the same sum as a wider call
/// actual -- are closed by different fixes and are covered there instead.
///
/// Verified byte-for-byte against reference simulators.
#[test]
fn owner_qualified_operand_width() {
    let test_file = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/classes/owner_qualified_operand_width_559.sv");
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
        !combined.contains("TEST_FAIL"),
        "owner_qualified_operand_width failed:\n{combined}"
    );
    assert!(
        combined.contains("TEST_PASS"),
        "owner_qualified_operand_width: no TEST_PASS.\nOutput:\n{combined}"
    );
}