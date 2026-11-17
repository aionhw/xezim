//! IEEE 1800-2023 regression and main-passing guards for upstream #1098.

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
        "xezim-1098-{}-{unique}-{}",
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

/// §23.3.3.7.1: "Any port connection with an interconnect net shall merge" the nets; §§6.6.7, 6.6.8 give the element nettype N and (2+0)/2=1.
#[test]
fn interconnect_element_resolves_all_port_drivers() {
    check(
        include_str!(
            "interconnect_array_drivers_1098/interconnect_element_resolves_all_port_drivers.sv"
        ),
        &["PASS expected=1 actual=1.000000"],
        false,
    );
}

/// §10.3.2 evaluates RHS and drives only LHS; §23.3.3.7.1 merges only "port connection" nets; §6.6.7 resolves each upstream network from its sole driver 2.
#[test]
fn keeps_continuous_buffers_directional() {
    check(
        include_str!("interconnect_array_drivers_1098/keeps_continuous_buffers_directional.sv"),
        &["T|upstream 2 2 2 2 2 2"],
        false,
    );
}

/// §23.10.1 permits hierarchical defparam override P=1; §6.6.7 makes it "an error" for an unresolved nettype to have multiple drivers; there is one driver per element: a[1] drives 2 and a[0] drives 6.
#[test]
fn keeps_legal_defparam_index_acceptance() {
    check(
        include_str!("interconnect_array_drivers_1098/keeps_legal_defparam_index_acceptance.sv"),
        &["T|ports 2 6"],
        false,
    );
}

/// §23.3.3.7.1: interconnect port connections "shall merge"; §§6.6.7,6.6.8 resolve drivers 2 and 0 to average 1.
#[test]
fn keeps_scalar_interconnect_resolution() {
    check(
        include_str!("interconnect_array_drivers_1098/keeps_scalar_interconnect_resolution.sv"),
        &["T|scalar 1 1 1"],
        false,
    );
}

/// §6.6.7 resolver receives the driver values; §23.3.3.7.1 merges ports on one element; average of 32 copies of 2 is 2. §23.3.3.7.1: "Any port connection with an interconnect net shall merge" the nets.
#[test]
fn keeps_high_fan_in_equal_drivers() {
    check(
        include_str!("interconnect_array_drivers_1098/keeps_high_fan_in_equal_drivers.sv"),
        &["T|fan-in 2 2"],
        false,
    );
}

/// §5.6.1: escaped identifiers contain printable non-whitespace characters; §7.4.2 array elements are separate objects. Each scalar has one driver, 7 or 9 (§6.6.7). §23.3.3.7.1: "Any port connection with an interconnect net shall merge" the nets.
#[test]
fn keeps_escaped_scalar_names_separate() {
    check(
        include_str!("interconnect_array_drivers_1098/keeps_escaped_scalar_names_separate.sv"),
        &["T|escaped 7 9"],
        false,
    );
}

/// §20.9 $isunknown of x is 1; §6.6.7 resolver maximum of drivers 6 and 2 is 6; §23.3.3.7.1 merges the ports of element 1. §23.3.3.7.1: "Any port connection with an interconnect net shall merge" the nets.
#[test]
fn keeps_deferred_selector_maximum() {
    check(
        include_str!("interconnect_array_drivers_1098/keeps_deferred_selector_maximum.sv"),
        &["T|max 6 6 6"],
        false,
    );
}
