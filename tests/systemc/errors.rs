//! SystemC constructs xezim cannot simulate are rejected with an error at
//! their C++ source location, never simulated approximately.

use std::path::PathBuf;
use std::process::Command;

/// Run `xezim` on `errors/<name>.cpp`; it must fail with `expected` (a
/// `file:line:col: error: SystemC: …` line).
fn rejects(name: &str, expected: &str) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/systemc/errors");
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(&dir)
        .args(["--no-cache", &format!("{}.cpp", name)])
        .output()
        .expect("run xezim");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !out.status.success(),
        "{}.cpp was accepted:\n{}",
        name,
        text
    );
    assert!(
        text.contains(expected),
        "{}.cpp: expected `{}` in:\n{}",
        name,
        expected,
        text
    );
}

/// §5.5 a thread spawned from a lambda (sc_bind of a function works; see
/// the `spawn` design).
#[test]
fn spawn_lambda() {
    rejects(
        "spawn",
        "spawn.cpp:5:18: error: SystemC: sc_spawn of a lambda is not supported in simulation",
    );
}

/// A module constructor argument computed at run time: constructor
/// parameters are SystemVerilog parameters, fixed before simulation.
#[test]
fn runtime_constructor_argument() {
    rejects(
        "runtime_ctor_arg",
        "runtime_ctor_arg.cpp:10:14: error: SystemC: a module constructor argument must be known before simulation ('size' is computed at run time)",
    );
}

#[test]
fn exceptions() {
    rejects(
        "exception",
        "exception.cpp:4:9: error: SystemC: exceptions are not supported in simulation",
    );
}

/// §5.12.7 a port left unbound is error E109 at the end of elaboration, in
/// the reference kernel's words (the run fails).
#[test]
fn unbound_port() {
    rejects(
        "custom_port",
        "Error: (E109) complete binding failed: port not bound: port 'x.port_0' (sc_port)",
    );
}

/// A destructor with statements: C++ runs it at the end of the object's
/// lifetime, which simulation does not model.
#[test]
fn destructor_with_statements() {
    rejects(
        "destructor",
        "destructor.cpp:5:5: error: SystemC: the destructor of 'tracker' has statements; destructors are not run in simulation",
    );
}
