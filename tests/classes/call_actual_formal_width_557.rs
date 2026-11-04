use std::path::Path;
use std::process::Command;

/// Pure-SystemVerilog self-test for the INTERPRETER half of xezim issue 528: a
/// call actual is context-determined by the callee's declared formal, so
/// `wide9(a + b)` with 8-bit `a`/`b` and a 9-bit `v` must add at 9 bits (300)
/// rather than wrapping to the operands' own 8-bit width (44).  Covers bare,
/// `this`-bounded and receiver-form calls, a task, a default actual, a `ref`
/// formal, a plain module-scope function, and the harder case where the
/// operands are class members reached through a HANDLE FORMAL, plus two
/// controls that must NOT widen.  Every callee is called a handful of times, so
/// it stays below the 1000-call tier that hands the body to the bytecode
/// executor -- the compiled half is `bytecode_wider_formal_width`.
#[test]
fn call_actual_formal_width() {
    let test_file =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/classes/call_actual_formal_width_557.sv");
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
        !combined.contains("TAG_FAIL"),
        "call_actual_formal_width failed:\n{combined}"
    );
    assert!(
        combined.contains("TAG_PASS"),
        "call_actual_formal_width: no TAG_PASS.\nOutput:\n{combined}"
    );
}