//! IEEE 1800-2023 regression and main-passing guards for upstream #1092.

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
        "xezim-1092-{}-{unique}-{}",
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

/// §10.11: "The effects are cumulative"; §6.8 initializes logic to x; §6.6.3 resolves wor to 1 if any driver is 1.
#[test]
fn alias_names_share_one_physical_net() {
    check(
        include_str!("net_alias_groups_1092/alias_names_share_one_physical_net.sv"),
        &[
            "PASS copy r,s=xx",
            "PASS wor a,b=11",
            "PASS cumulative c,d,e=111",
            "PASS generate g2=1",
            "PASS child c2=1",
        ],
        false,
    );
}

/// §10.11: "same physical net"; §6.6.3 wor is 1 when any driver is 1; §6.6.1 z yields to 1.
#[test]
fn keeps_distributed_wor_and_z_driver() {
    check(
        include_str!("net_alias_groups_1092/keeps_distributed_wor_and_z_driver.sv"),
        &["T|protected 1 1"],
        false,
    );
}

/// §§10.11, 6.6.3 require each protected wor bit 1; §23.6 permits hierarchical writes; §14.16: "a driver on that net shall be created" for a clocking output.
#[test]
fn keeps_rooted_upward_member_and_clocking_drivers() {
    check(
        include_str!("net_alias_groups_1092/keeps_rooted_upward_member_and_clocking_drivers.sv"),
        &["T|protected 1 1 1 1 1"],
        false,
    );
}

/// §§10.11, 6.6.3 require 1; §23.3.3: inout assignment is a "non-strength-reducing transistor connection"; §§23.6,25.5 permit these paths.
#[test]
fn keeps_hierarchical_port_chain_and_modport_drivers() {
    check(
        include_str!("net_alias_groups_1092/keeps_hierarchical_port_chain_and_modport_drivers.sv"),
        &["T|protected 1 1 1"],
        false,
    );
}

/// §10.11: "The continuous assign statement is a unidirectional assignment"; §§10.3.2,6.6.3 leave the driven source aliases at 1; §14.16 supplies the clocking driver.
#[test]
fn keeps_downstream_copies_directional() {
    check(
        include_str!("net_alias_groups_1092/keeps_downstream_copies_directional.sv"),
        &["T|one 111", "T|zero 111", "T|again 111"],
        false,
    );
}

/// §23.3.3.7 Table 23-1 selects the internal tri0/tri1 kind; §6.6.5: tri1 has "a continuous 1 value of pull strength driving it" (dual tri0); §10.11 shares it.
#[test]
fn keeps_implicit_pull_ports() {
    check(
        include_str!("net_alias_groups_1092/keeps_implicit_pull_ports.sv"),
        &["T|pull 111 000"],
        false,
    );
}

/// §23.3.1 gives "priority to the local scope"; §10.11: "the scope of an alias is limited"; child 0 is independent of top 11.
#[test]
fn keeps_local_scopes_named_like_top() {
    check(
        include_str!("net_alias_groups_1092/keeps_local_scopes_named_like_top.sv"),
        &["T|scope 11 0"],
        false,
    );
}

/// §§23.3.1,23.6,27.3 give the local block its own declaration; §10.11 limits the alias to top a,b. §10.11: "same physical net".
#[test]
fn keeps_generate_scope_named_like_top() {
    check(
        include_str!("net_alias_groups_1092/keeps_generate_scope_named_like_top.sv"),
        &["T|scope 11 0"],
        false,
    );
}

/// §10.3.4: unspecified strength "shall default to (strong1, strong0)"; §6.6.6 supply strength; §21.2.1.4 defines scalar %v.
#[test]
fn keeps_ordinary_copy_strength() {
    check(
        include_str!("net_alias_groups_1092/keeps_ordinary_copy_strength.sv"),
        &["T|strength We1 St1 Su1 St1 Su0 St0"],
        false,
    );
}

/// §10.6.2: force "shall override all drivers of the net"; §21.2.1.4: "The third character indicates the current logic value of the scalar"; §10.11 shares force/release.
#[test]
fn keeps_forced_alias_strength_value() {
    check(
        include_str!("net_alias_groups_1092/keeps_forced_alias_strength_value.sv"),
        &["T|forced 1 1", "T|released 0"],
        false,
    );
}

/// §28.3.2 assigns output drive strength; §28.6 enables bufif1; §28.12.1: "the stronger signal shall dominate all the weaker drivers"; §21.2.1.4 scalar %v.
#[test]
fn keeps_explicit_wired_primitive_strengths() {
    check(
        include_str!("net_alias_groups_1092/keeps_explicit_wired_primitive_strengths.sv"),
        &["T|gates 00 Pu0 Pu0"],
        false,
    );
}

/// §§28.3.2,28.10,28.12.4: equal pull 0/1 on wor resolve to 1; §10.11 gives both names the same bit; §23.3.3.3 permits the selected actual. §10.11: "same physical net".
#[test]
fn keeps_selected_child_primitive_strengths() {
    check(
        include_str!("net_alias_groups_1092/keeps_selected_child_primitive_strengths.sv"),
        &["T|enabled 11"],
        false,
    );
}

/// §10.11 cumulative identity; §6.6.6 supply0/1; §6.6.4: trireg "shall store a value" after its driver floats.
#[test]
fn keeps_forward_chain_supply_and_driven_charge() {
    check(
        include_str!("net_alias_groups_1092/keeps_forward_chain_supply_and_driven_charge.sv"),
        &["T|stable 111 00 11 1"],
        false,
    );
}
