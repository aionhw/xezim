//! SystemC designs simulated end to end: each `designs/<name>.cpp` runs
//! through `xezim` and its output must equal `designs/<name>.golden`, which
//! is the output of the same program built against the SystemC 2.3.4
//! reference kernel (its banner removed). A mismatch is a scheduling or
//! lowering difference from the reference implementation.

use std::path::PathBuf;
use std::process::Command;

fn designs() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/systemc/designs")
}

/// Run `xezim` on `designs/<name>.cpp` (from that directory, so reports
/// name the file as the reference did) and compare with the golden output.
fn check(name: &str) {
    let dir = designs();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(&dir)
        .args(["--no-cache", &format!("{}.cpp", name)])
        .output()
        .expect("run xezim");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // Drop xezim's own banner and summary lines.
    let actual: Vec<&str> = stdout
        .lines()
        .filter(|l| {
            !(l.starts_with("=== xezim")
                || l.starts_with("git ")
                || l.starts_with("Simulation finished at time")
                || l.starts_with("[xezim]"))
        })
        .collect();
    let mut actual = actual.join("\n");
    actual = actual.trim_start_matches('\n').to_string();
    let golden = std::fs::read_to_string(dir.join(format!("{}.golden", name))).expect("golden");
    assert_eq!(
        actual.trim_end(),
        golden.trim_end(),
        "{}.cpp differs from the SystemC 2.3.4 reference\nstderr:\n{}",
        name,
        stderr
    );
}

/// §5.2.9 SC_METHOD with edge sensitivity and `posedge()` (§6.8.4).
#[test]
fn dff() {
    check("dff");
}

/// §5.2.9 SC_CTHREAD with `reset_signal_is` (§5.2.13); sc_uint wrap-around.
#[test]
fn counter() {
    check("counter");
}

/// §4.1 hierarchy: child pointer + port binding; §7.2.7 concatenation, part
/// and bit selects.
#[test]
fn hier() {
    check("hier");
}

/// §6.23 sc_fifo between two SC_THREADs; `sc_start()` until starvation.
#[test]
fn fifo() {
    check("fifo");
}

/// §11 TLM-2.0 loosely-timed b_transport through simple sockets (§16.1),
/// data pointer into a target memory array.
#[test]
fn tlm() {
    check("tlm");
}

/// §5.10 sc_event immediate/timed/delta notification, OR-lists, timeouts.
#[test]
fn events() {
    check("events");
}

/// §8.3 SC_REPORT_INFO/WARNING/ERROR formats; §4.5.3 sc_stop from a process.
#[test]
fn report() {
    check("report");
}

/// §7 data types (sc_bv/sc_lv/sc_int printing, hex), sc_time arithmetic,
/// switch fall-through, value-returning members, printf.
#[test]
fn types() {
    check("types");
}

/// §4.2.1 delta cycles: signal → method → signal chains; SC_THREAD static
/// sensitivity with `wait()`.
#[test]
fn deltas() {
    check("deltas");
}

/// Template modules at two widths, sc_vector ports (§8.5) bound to an
/// sc_vector of signals, element writes from sc_main.
#[test]
fn template() {
    check("template");
}

/// SC_CTHREAD with `async_reset_signal_is` (§5.2.13) and `wait(n)`.
#[test]
fn async_reset() {
    check("async");
}

/// TLM-2.0 LT initiator/memory with `memcpy` through generic payload data
/// pointers (§14.13) and response checks.
#[test]
fn tlm_lt() {
    check("tlm_lt");
}
