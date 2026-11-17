//! IEEE 1800-2023 §6.20.2 regression and main-passing guards for upstream
//! #1143: an UNTYPED block-scope `localparam`/`parameter` is self-determined
//! from its initializer, both in the constant-function evaluator and the
//! runtime (hoisted) path.

use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct RunDir(std::path::PathBuf);
impl Drop for RunDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn check_sim(source: &str, expected: &[&str]) {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = RunDir(std::env::temp_dir().join(format!(
        "xezim-1143-{}-{unique}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    std::fs::create_dir(&dir.0).unwrap();
    std::fs::write(dir.0.join("test.sv"), source).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_xezim"));
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("XEZIM_") {
            command.env_remove(key);
        }
    }
    let output = command
        .current_dir(&dir.0)
        .args(["--simulate", "-s", "top", "test.sv"])
        .output()
        .expect("run xezim");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    let lines: Vec<_> = stdout
        .lines()
        .filter(|line| line.starts_with("PASS") || line.starts_with("FAIL"))
        .collect();
    let mut actual = lines;
    let mut expected = expected.to_vec();
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected, "{stdout}\n{stderr}");
}

/// §6.20.2 const-function path: `localparam P = f()` where f's body holds an
/// untyped `localparam Q = 4'd0-4'd1` — self-determined 4-bit 15.
#[test]
fn const_fn_untyped_localparam() {
    check_sim(
        include_str!("localparam_self_determined_1143/const_fn_untyped_localparam.sv"),
        &["PASS const P=15"],
    );
}

/// §6.20.2 runtime/hoisted path: a subroutine-local untyped localparam keeps
/// its self-determined width after hoisting to a static cell.
#[test]
fn runtime_untyped_localparam() {
    check_sim(
        include_str!("localparam_self_determined_1143/runtime_untyped_localparam.sv"),
        &["PASS sub X=15", "PASS big Y=24"],
    );
}

/// A TYPED `localparam int` is NOT self-determined — it stays 32-bit signed.
#[test]
fn typed_remains_signed() {
    check_sim(
        include_str!("localparam_self_determined_1143/typed_remains_signed.sv"),
        &["PASS typed P=-1"],
    );
}

/// §6.20.2: block-scope untyped `localparam`s with explicit signing and mixed
/// declarators (`A = 1.5, B = 2`) resolve per declarator across const-fn,
/// compiled runtime, and interpreter paths, while `static` variables keep their
/// declared type.
#[test]
fn mixed_signed_real_param_decl() {
    check_sim(
        include_str!("localparam_self_determined_1143/mixed_signed_real_param_decl.sv"),
        &["PASS blk C=47", "PASS ret=48", "PASS rt Q=15 S=-1 A=1.5 B=2"],
    );
}