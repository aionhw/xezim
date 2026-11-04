use std::path::Path;
use std::process::Command;

/// Pure-SystemVerilog self-test: a bytecode-compiled class method must give each call ACTUAL the CONTEXT width of the callee's declared formal, not the actual's self-determined width, so `wide9(a + b)` against `function bit wide9(logic [8:0] v)` evaluates `a + b` at 9 bits (300) instead of truncating to 8 (44) (§11.6.1/§11.8.2).
///
/// The method is called 1500x to clear the 1000-call compile threshold, and
/// the run is executed under `XEZIM_FALLBACK_SITES=1` so the test can PROVE
/// the body really compiled.  Without that guard a body the compiler declines
/// silently falls back to the AST interpreter and the check passes for the
/// wrong reason -- exactly how the "hot compiled" section of the original
/// issue-528 MWE reported green while `E.hotrun` never compiled at all.
/// Verified byte-for-byte against reference simulators.
#[test]
fn bytecode_wider_formal_width() {
    let test_file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/classes/bytecode_wider_formal_width_557.sv");
    assert!(
        test_file.exists(),
        "Test file not found: {}",
        test_file.display()
    );
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .env("XEZIM_FALLBACK_SITES", "1")
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
    // Guard: the probed bodies must genuinely reach the bytecode compiler.
    for scope in ["E.probe", "E.probe2"] {
        assert!(
            combined.contains(&format!("runs-compiled scope={scope}")),
            "bytecode_wider_formal_width: {scope} did not report runs-compiled, \
             so the hot check would be measuring the AST interpreter:\n{combined}"
        );
        assert!(
            !combined.contains(&format!("compile-fail scope={scope}"))
                && !combined.contains(&format!("plan-decline reason={scope}")),
            "bytecode_wider_formal_width: {scope} declined to compile:\n{combined}"
        );
    }
    assert!(
        !combined.contains("TAG_FAIL"),
        "bytecode_wider_formal_width failed:\n{combined}"
    );
    assert!(
        combined.contains("TAG_PASS"),
        "bytecode_wider_formal_width: no TAG_PASS.\nOutput:\n{combined}"
    );
}
