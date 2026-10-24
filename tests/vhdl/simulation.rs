//! VHDL designs simulated end to end: each `designs/<name>.vhd` runs through
//! `xezim` and its output and exit status must equal `designs/<name>.golden`
//! and `designs/<name>.exit`, produced by GHDL 4.1 (`designs/regen.sh`). A
//! difference is a semantic or formatting difference from GHDL.

use std::path::PathBuf;
use std::process::Command;

fn designs() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/vhdl/designs")
}

fn check(name: &str) {
    let dir = designs();
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(&dir)
        .args(["--no-cache", &format!("{}.vhd", name)])
        .output()
        .expect("run xezim");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let actual: Vec<&str> = stdout
        .lines()
        .filter(|l| {
            !(l.starts_with("=== xezim")
                || l.starts_with("git ")
                || l.starts_with("Simulation finished at time")
                || l.starts_with("[xezim]"))
        })
        .collect();
    let golden = std::fs::read_to_string(dir.join(format!("{}.golden", name))).expect("golden");
    let exit: i32 = std::fs::read_to_string(dir.join(format!("{}.exit", name)))
        .expect("exit status")
        .trim()
        .parse()
        .expect("exit status number");
    assert_eq!(
        actual.join("\n").trim_end(),
        golden.trim_end(),
        "{}.vhd differs from GHDL\nstderr:\n{}",
        name,
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(exit), "{}.vhd exit status", name);
}

/// §10.3 report and §10.4 assertion messages at every severity, GHDL's
/// `@<time>` format, `'image` of integer, real, time, boolean, character
/// and bit (§16.2.2); severity FAILURE stops the run with exit status 1.
#[test]
fn report_formats() {
    check("report_formats");
}

/// The `@<time>` of messages: the largest unit up to ms dividing it.
#[test]
fn time_format() {
    check("time_format");
}

/// Simulation time up to an hour; INTEGER and TIME bounds (§5.2.3, §5.2.4).
#[test]
fn time_units() {
    check("time_units");
}

/// Assertion of severity FAILURE (§10.4).
#[test]
fn assert_failure() {
    check("assert_failure");
}

/// A value outside its subtype: bound check failure (§14.7.3.1).
#[test]
fn bound_check() {
    check("bound_check");
}

/// An array index outside the index range (§8.4).
#[test]
fn index_check() {
    check("index_check");
}

/// INTEGER overflow (§5.2.3.1).
#[test]
fn overflow() {
    check("overflow");
}

/// `std.env.stop` and `std.env.finish` (§16.5).
#[test]
fn env_stop() {
    check("env_stop");
}

#[test]
fn env_finish() {
    check("env_finish");
}

/// A clock from a conditional concurrent assignment with `after`, and a
/// process sensitive to it (§11.6, §10.5.2).
#[test]
fn clock_report() {
    check("clock_report");
}

/// Delta cycles of a chain of concurrent assignments (§14.7.5).
#[test]
fn deltas() {
    check("deltas");
}

/// `wait on`, `wait until`, `wait for` and their combinations (§10.2).
#[test]
fn waits() {
    check("waits");
}

/// Inertial and transport delay, `reject`, waveforms (§10.5.2.2).
#[test]
fn delays() {
    check("delays");
}

/// Signal attributes 'event, 'last_value, 'last_event, 'active, and type
/// attributes (§16.2).
#[test]
fn attributes() {
    check("attrs");
}

/// Overloaded functions, default and named parameters, inout parameters,
/// recursion, a package with a deferred constant (§4).
#[test]
fn subprograms() {
    check("subprograms");
}

/// if, case with ranges, for loops both ways, while, labelled next/exit
/// across nested loops, null ranges (§10.8–§10.12).
#[test]
fn control() {
    check("control");
}

/// Arrays: aggregates, slices, concatenation, equality and ordering,
/// `'range` loops, bit_vector logic (§5.3.2, §9.2, §9.3.3).
#[test]
fn arrays() {
    check("arrays");
}

/// Records: aggregates, element selection, a record signal (§5.3.3).
#[test]
fn records() {
    check("records");
}

/// Subtype range check in a loop (§14.7.3.1).
#[test]
fn range_checks() {
    check("checks");
}

/// IEEE.STD_LOGIC_1164 compiled from its source: the logical operators
/// through their two-dimensional tables, conversions, TO_STRING and
/// TO_HSTRING, RESOLVED, the matching operators `?=` (§9.2.3).
#[test]
fn std_logic() {
    check("std_logic");
}

/// An entity instance with ports collapsed onto the actual signals
/// (§14.7.3.4), IEEE `rising_edge` through a signal parameter, drivers of
/// single elements and slices from several processes (§14.7.2), and a
/// user-defined resolution function (§4.6).
#[test]
fn hierarchy() {
    check("hierarchy");
}

/// Generic values per instance: positional and named generic maps,
/// defaults, generic-dependent subtypes, `open` and literal port actuals
/// (§6.5.6, §14.4.2.2).
#[test]
fn generics() {
    check("generics");
}

/// STD.TEXTIO WRITE of the standard types with justification, field width,
/// DIGITS and UNIT, WRITELINE to OUTPUT, SWRITE, JUSTIFY; LINE as an access
/// type: allocators, `.all`, indexing through it, DEALLOCATE (§5.4, §16.4).
#[test]
fn textio_write() {
    check("textio_write");
}

/// Milestone M2: a clocked IEEE.NUMERIC_STD counter instantiated by a
/// testbench with a generic, a resolved tri-state bus of std_logic
/// vectors driven from two instances and a pull, TEXTIO logging and
/// numeric_std arithmetic, ending with std.env.finish.
#[test]
fn counter_tb() {
    check("counter_tb");
}

/// IEEE.NUMERIC_STD division, MOD and REM of UNSIGNED and SIGNED (the
/// DIVMOD procedure's unconstrained `out` parameters take the actuals'
/// bounds, §4.2.2.2).
#[test]
fn numeric_div() {
    check("numeric_div");
}

/// For-, if-/elsif-/else- and case-generate statements (§11.8): one block
/// per iteration with its own signals, processes and instances, elaborated
/// depth-first in iteration order (§14.5.3).
#[test]
fn generates() {
    check("generates");
}

/// A block statement with generics, generic map, ports and port map
/// (§11.2, §14.5.2) holding a for-generate with declarations and a nested
/// if-generate; signals and subprograms of the enclosing scopes.
#[test]
fn blocks() {
    check("blocks");
}

/// Component declarations and instantiation (§6.8, §11.7.2): default
/// binding to the entity of the same name, configuration specifications by
/// label and `others` with generic and port maps naming the local generics
/// (§7.3), an unbound component, and instantiation of a configuration.
#[test]
fn components() {
    check("components");
}

/// A configuration declaration as the top (§3.4): block configurations of
/// an architecture, of generate iterations by index and range and of an
/// if-generate, component configurations with nested block configurations.
#[test]
fn config_decl() {
    check("config_decl");
}

/// Port associations (§6.5.7.1): conversion functions and type conversions
/// of actuals, formal conversions of out ports, partial association of
/// formals in both directions, a resolved inout bus and a buffer port; no
/// delta is added (§14.7.3.4).
#[test]
fn port_maps() {
    check("port_maps");
}

/// A non-static actual expression is assigned to an implicit signal by an
/// implicit process, one delta behind (§6.5.6.3), unlike a signal name or
/// a conversion function.
#[test]
fn port_exprs() {
    check("port_exprs");
}

/// A generic-width ripple-carry adder of full-adder components in a
/// for-generate with if-generate alternatives, bound by a configuration to
/// two architectures and checked against IEEE.NUMERIC_STD addition.
#[test]
fn ripple_adder() {
    check("ripple_adder");
}

/// Projected waveforms (§10.5.2.2): inertial rejection of several pending
/// transactions keeping the run of the new value, `reject 0 ns`, transport
/// deleting later transactions, and zero-delay elements.
#[test]
fn waveforms() {
    check("waveforms");
}

/// Implicit signals S'STABLE(T), S'QUIET(T), S'DELAYED(T), S'TRANSACTION
/// of signals and elements (§16.2.4) as sensitivities, in a wait condition
/// and in concurrent assignments; 'LAST_ACTIVE.
#[test]
fn implicit_signals() {
    check("implicit_signals");
}

/// Implicit signals are updated in the cycle of their prefix's event or
/// activity (§14.7.5.3 c); 'DELAYED with no time is one delta later.
#[test]
fn implicit_deltas() {
    check("implicit_deltas");
}

/// Postponed processes, a postponed assertion and a postponed procedure
/// call run at the end of a time step with its final values, and last at
/// initialization (§11.3, §14.7.5.2, §14.7.5.3).
#[test]
fn postponed_procs() {
    check("postponed_procs");
}

/// A postponed process causing a delta cycle is an error (§11.3).
#[test]
fn postponed_delta() {
    check("postponed_delta");
}

/// Guarded blocks and assignments (§11.2, §11.6) with nested guards,
/// guarded signals of kinds bus and register (§6.4.2.3), disconnection
/// specifications (§7.4) and drivers turned off by null transactions
/// (§14.7.3.2).
#[test]
fn guarded_blocks() {
    check("guarded_blocks");
}

/// Procedures that wait and assign signals through signal parameters of
/// modes in, out and inout and elements of signals (§4.2.2.3, §10.2), in
/// a package and in a process.
#[test]
fn procedure_waits() {
    check("procedure_waits");
}

/// 'DRIVING and 'DRIVING_VALUE of a signal and an out port (§16.2.4).
#[test]
fn driving_values() {
    check("driving_values");
}
