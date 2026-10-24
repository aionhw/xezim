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
    assert!(!out.status.success(), "{}.cpp was accepted:\n{}", name, text);
    assert!(
        text.contains(expected),
        "{}.cpp: expected `{}` in:\n{}",
        name,
        expected,
        text
    );
}

/// §6.9: an sc_buffer's per-write events cannot pass through a port.
#[test]
fn buffer_bound_to_port() {
    rejects(
        "buffer_port",
        "buffer_port.cpp:9:10: error: SystemC: binding an sc_buffer to a port is not supported in simulation",
    );
}

/// §5.5 dynamic processes.
#[test]
fn sc_spawn() {
    rejects(
        "spawn",
        "spawn.cpp:5:9: error: SystemC: dynamic processes (sc_spawn) are not supported in simulation",
    );
}

#[test]
fn exceptions() {
    rejects(
        "exception",
        "exception.cpp:4:9: error: SystemC: exceptions are not supported in simulation",
    );
}

/// §5.12 sc_port with a user interface (hierarchical channels).
#[test]
fn custom_port() {
    rejects(
        "custom_port",
        "custom_port.cpp:7:21: error: SystemC: sc_port/sc_export/analysis ports are not supported in simulation yet",
    );
}

/// §5.10.7 `wait(e1 & e2)`.
#[test]
fn event_and_list() {
    rejects(
        "and_list",
        "and_list.cpp:5:14: error: SystemC: waiting on an event AND-list is not supported",
    );
}
