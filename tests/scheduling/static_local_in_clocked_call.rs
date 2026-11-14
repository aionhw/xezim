use std::path::Path;
use std::process::Command;

/// IEEE 1800-2023 §6.21 / §13.3.1: a variable of a subroutine with the
/// default (static) lifetime, and a `static` variable of an automatic one,
/// keeps its value between calls. A void function or task called from a
/// clocked block was inlined into the block with fresh registers for its
/// variables, so each call started from the default value again.
#[test]
fn static_local_in_clocked_call() {
    check(
        "static_local_in_clocked_call.sv",
        "T|n=4 t=8 a=91 f=12 g=8 w=24 blk=4",
    );
}

/// §6.21: static storage is shared across call sites; automatic and
/// write-first locals retain their ordinary initialization behavior.
#[test]
fn static_call_sites_and_automatic_controls() {
    check(
        "static_local_call_controls.sv",
        "T|shared=4 nested=8 branch=16 written=20 automatic=1 x=2 y=2",
    );
}

fn check(name: &str, expected: &str) {
    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scheduling")
        .join(name);
    for tier in ["1", "0"] {
        let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
            .args(["--simulate", "-s", "top", "--no-cache"])
            .arg(&file)
            .env("XEZIM_METHOD_TIER", tier)
            .output()
            .expect("Failed to execute xezim");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains(expected),
            "method tier {tier}: {stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
