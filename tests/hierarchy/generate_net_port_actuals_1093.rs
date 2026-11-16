//! IEEE 1800-2023 regression and main-passing guards for upstream #1093.

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
        "xezim-1093-{}-{unique}-{}",
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

/// §23.3.3: matching user nettypes "shall be merged into a single simulated net"; §6.6.7 averages (2+0)/2=1; §6.6.1 conflicts give x; §§23.6, 27.4 permit constant generate paths.
#[test]
fn hierarchical_generate_actual_merges_port_drivers() {
    check(
        include_str!(
            "generate_net_port_actuals_1093/hierarchical_generate_actual_merges_port_drivers.sv"
        ),
        &["PASS nettype 1,1", "PASS wire 10x0,10x0"],
        false,
    );
}

/// §23.8: "Look in the current scope" before enclosing scopes; §23.6 gives distinct hierarchical objects; top has only driver 1010 (§10.3.2).
#[test]
fn keeps_child_generate_scope_independent() {
    check(
        include_str!("generate_net_port_actuals_1093/keeps_child_generate_scope_independent.sv"),
        &["T|outer 1010"],
        false,
    );
}

/// §23.3.1: "giving priority to the local scope"; §23.8 hierarchical search is independent of declaration order; §23.3.3.1 input connection copies local 0.
#[test]
fn keeps_late_child_and_variable_shadowing() {
    check(
        include_str!("generate_net_port_actuals_1093/keeps_late_child_and_variable_shadowing.sv"),
        &["T|shadow 1 0 0 0 0"],
        false,
    );
}

/// §7.4.2: each unpacked element has its declared type; §23.3.3.3 permits inout net elements, carrying all four zero bits. §6.5: "a continuous assignment is implied" at variable port connections.
#[test]
fn keeps_unpacked_element_width_and_diagnostics() {
    check(
        include_str!(
            "generate_net_port_actuals_1093/keeps_unpacked_element_width_and_diagnostics.sv"
        ),
        &["T|elements 0000 0000"],
        true,
    );
}

/// §23.3.3.5: "each instance shall get a part-select of the port connection"; four one-bit input ports receive bits 3,2,1,0 of 0000.
#[test]
fn keeps_distributed_instance_array_diagnostics() {
    check(
        include_str!(
            "generate_net_port_actuals_1093/keeps_distributed_instance_array_diagnostics.sv"
        ),
        &["T|array 0000"],
        true,
    );
}

/// §23.3.3.4: interface port connects to "an interface instance or a higher level interface port"; §§25.3,23.8 give read 1010 and independent locals 0110. Main-wrong driven interface fields are omitted.
#[test]
fn keeps_interface_port_binding_with_local_shadow() {
    check(
        include_str!(
            "generate_net_port_actuals_1093/keeps_interface_port_binding_with_local_shadow.sv"
        ),
        &["T|iface 1010 1010 0110 0110"],
        false,
    );
}

/// §23.3.3.4 permits a "higher level interface port"; §23.6 gives the local children independent identities, each driven only by 0110.
#[test]
fn keeps_forwarded_interface_local_child_independent() {
    check(
        include_str!(
            "generate_net_port_actuals_1093/keeps_forwarded_interface_local_child_independent.sv"
        ),
        &["T|forward 0110 0110"],
        false,
    );
}

/// §23.3.3: when only one connection has a user nettype, matching data types use "a continuous assignment from source to sink"; §§6.12,10.3.2 copy 3.0 without merging.
#[test]
fn keeps_one_sided_real_nettype_connection() {
    check(
        include_str!("generate_net_port_actuals_1093/keeps_one_sided_real_nettype_connection.sv"),
        &["T|real 3.00 3.00 3.00 3.00"],
        false,
    );
}

/// §23.3.3.2: variable input connection is an implied continuous assignment; §10.6.2 force overrides only that variable, leaving source 1010. The report names plain module-scope actuals as main-wrong; those are omitted. §6.5: "a continuous assignment is implied" at variable port connections.
#[test]
fn keeps_input_variable_force_direction() {
    check(
        include_str!("generate_net_port_actuals_1093/keeps_input_variable_force_direction.sv"),
        &["T|forced 1010 0101 1010 0101", "T|source 1010 1010"],
        false,
    );
}
