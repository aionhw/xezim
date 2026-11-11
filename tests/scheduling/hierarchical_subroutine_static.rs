use std::path::Path;
use std::process::Command;

/// IEEE 1800-2023 §6.21, §13.3 and §23.6: hierarchical references reach
/// declaration-owned static storage, including before the first call.
#[test]
fn hierarchical_subroutine_static() {
    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scheduling/hierarchical_subroutine_static.sv");
    for tier in ["0", "1"] {
        let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
            .args(["--simulate", "-s", "top", "--no-cache"])
            .arg(&file)
            .env("XEZIM_METHOD_TIER", tier)
            .output()
            .expect("execute xezim");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("PASS hierarchical subroutine statics"),
            "tier {tier}: {stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
