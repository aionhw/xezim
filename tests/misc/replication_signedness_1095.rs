//! IEEE 1800-2023 regression and main-passing guards for upstream #1095.

use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct RunDir(std::path::PathBuf);
impl Drop for RunDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn check(source: &str, expected: &[&str], no_warnings: bool) {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = RunDir(std::env::temp_dir().join(format!(
        "xezim-1095-{}-{unique}-{}",
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
        .args(["--no-cache", "test.sv"])
        .output()
        .expect("run xezim");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    if no_warnings {
        assert!(!stderr.to_ascii_lowercase().contains("warning"), "{stderr}");
    }
    let lines: Vec<_> = stdout
        .lines()
        .filter(|line| {
            line.starts_with("T|") || line.starts_with("PASS") || line.starts_with("FAIL")
        })
        .collect();
    // Several independent initial blocks may print at the same time. Compare
    // the multiset so the test does not impose an Active-region ordering (§4.7).
    let mut actual = lines;
    let mut expected = expected.to_vec();
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected, "{stdout}\n{stderr}");
}

/// §11.8.1: "Concatenation results are unsigned, regardless of the operands."
/// `{1{x}}` zero-extends to a 64-bit target; the operand `x` keeps -5.
#[test]
fn replication_unsigned_regardless_of_operands() {
    check(
        include_str!(
            "replication_signedness_1095/replication_unsigned_regardless_of_operands.sv"
        ),
        &[
            "PASS plain y=4294967291",
            "PASS keyed k=-5,4294967291",
            "PASS default d=4294967291,4294967291",
        ],
        false,
    );
}

/// §23.3.3.8: "The sign attribute shall not cross hierarchy." An equal-width
/// unsigned replication actual must not strip the sign of a signed input formal.
#[test]
fn keeps_signed_input_port_from_unsigned_replication() {
    check(
        include_str!(
            "replication_signedness_1095/keeps_signed_input_port_from_unsigned_replication.sv"
        ),
        &["PASS port q=-5"],
        false,
    );
}

/// §23.3.3.8 (whole-net cousin, both directions): the implied connection copy
/// from a WHOLE-NET actual of the opposite sign must re-interpret the bits
/// through the formal's own declared type. Collapsing the copy onto the actual
/// reads the actual's bits with the actual's sign (signed formal -> unsigned
/// actual zero-extends 0xffff_fffb to 4294967291 instead of -5).
#[test]
fn keeps_signed_input_port_from_unsigned_whole_net() {
    check(
        include_str!(
            "replication_signedness_1095/keeps_signed_input_port_from_unsigned_whole_net.sv"
        ),
        &["PASS signed-formal q=-5"],
        false,
    );
}

/// §23.3.3.8 (mirror whole-net direction): an UNSIGNED `input [31:0] p` bound
/// to a SIGNED `int s` actual must read as the unsigned value inside the child;
/// re-pointing the formal onto the signed actual would sign-extend -5 to -5
/// instead of 4294967291.
#[test]
fn keeps_unsigned_input_port_from_signed_whole_net() {
    check(
        include_str!(
            "replication_signedness_1095/keeps_unsigned_input_port_from_signed_whole_net.sv"
        ),
        &["PASS unsigned-formal q=4294967291"],
        false,
    );
}