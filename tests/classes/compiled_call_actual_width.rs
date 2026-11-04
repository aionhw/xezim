//! IEEE 1800-2023 §13.5.1 / §11.6.1: on the bytecode-compiled tier the
//! `super.m(...)` and free-function call lowerings give each actual the
//! context width of the callee's declared formal, as the bare in-class call
//! does (#555 follow-up to #558). The probe runs 1500x to clear the compile
//! threshold under `XEZIM_FALLBACK_SITES=1`, which proves the body compiled.

use std::path::Path;
use std::process::Command;

#[test]
fn compiled_super_and_free_function_actuals_take_the_formal_width() {
    let test_file =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/classes/compiled_call_actual_width.sv");
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
        combined.contains("runs-compiled scope=E.probe"),
        "E.probe did not report runs-compiled:\n{combined}"
    );
    assert!(
        !combined.contains("compile-fail scope=E.probe")
            && !combined.contains("plan-decline reason=E.probe"),
        "E.probe declined to compile:\n{combined}"
    );
    assert!(!combined.contains("TAG_FAIL"), "{combined}");
    assert!(combined.contains("TAG_PASS"), "no TAG_PASS:\n{combined}");
}
