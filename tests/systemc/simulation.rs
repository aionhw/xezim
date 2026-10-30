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

/// Run `xezim` on `designs/<name>.cpp`, or on every `.cpp` of the
/// directory `designs/<name>/` (from that directory, so reports name the
/// file as the reference did), and compare with `designs/<name>.golden`.
/// Returns the exit status (the reference kernel's: 1 after an error).
fn check(name: &str) -> Option<i32> {
    let dir = designs();
    let (run_dir, files) = if dir.join(name).is_dir() {
        let mut cpps: Vec<String> = std::fs::read_dir(dir.join(name))
            .expect("design directory")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .filter(|f| f.ends_with(".cpp"))
            .collect();
        cpps.sort();
        (dir.join(name), cpps)
    } else {
        (dir.clone(), vec![format!("{}.cpp", name)])
    };
    let out = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .current_dir(&run_dir)
        .arg("--no-cache")
        .args(&files)
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
    out.status.code()
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

/// §6.7 sc_clock duty cycle, start time and `posedge_first = false`;
/// negedge sensitivity (§5.4.3); sc_time printing (§5.11.10).
#[test]
fn clocks() {
    check("clocks");
}

/// §6.9 sc_buffer notifies on every write; sc_signal (§6.4) only on a
/// value change.
#[test]
fn buffer() {
    check("buffer");
}

/// SC_HAS_PROCESS with constructor parameters (§5.2.8), an out-of-class
/// constructor, member child modules built in the initializer list,
/// positional binding (§4.1.3) and sc_inout (§6.10).
#[test]
fn ctor_params() {
    check("ctor");
}

/// §4.5.7 `sc_start()` without a limit ended by `sc_stop()`; statements
/// after it; timed and SC_ZERO_TIME notification (§5.10.8).
#[test]
fn stop() {
    check("stop");
}

/// C++ in process bodies: free functions, reference parameters, static
/// locals, `<cmath>`, break/continue in nested loops.
#[test]
fn functions() {
    check("funcs");
}

/// §7.9 four-valued sc_logic/sc_lv with X and Z, bit-string literals
/// (§7.9.8).
#[test]
fn logic() {
    check("logic");
}

/// §7.6 sc_uint/sc_int/sc_biguint: signed arithmetic and arithmetic right
/// shift, range assignment, reductions, wrap-around.
#[test]
fn slices() {
    check("slices");
}

/// §4.5.7 sc_start(t) in steps: events at the end time run in the next
/// call; sc_main writes apply at the next sc_start.
#[test]
fn start_boundary() {
    check("boundary");
}

/// TLM-2.0 transport_dbg (§11.3), word and byte views of target memory,
/// an array as a data pointer, address error responses (§14.17).
#[test]
fn tlm_dbg() {
    check("tlm_dbg");
}

/// §6.23 sc_fifo of depth 2: blocking write and read, num_available,
/// nb_read on an empty fifo; sc_start() ends when no events are left.
#[test]
fn fifo_depth() {
    check("fifo2");
}

/// Moore FSM with an enum state: SC_METHOD next-state logic, SC_CTHREAD
/// state register with `reset_signal_is` (§5.2.13), `negedge_event()`.
#[test]
fn fsm() {
    check("fsm");
}

/// TLM-2.0 LT interconnect: a router's target socket forwards through two
/// initiator sockets by address (§11.1); delays accumulate.
#[test]
fn tlm_router() {
    check("router");
}

/// Several translation units sharing a header: a module declared in
/// `alu.h`, its process defined in `alu.cpp`.
#[test]
fn multi_file() {
    check("multi");
}

/// §6.13–§6.18 resolved signals and ports: each process writing one is a
/// driver (through ports of two modules and directly), bits resolve as
/// sc_logic does (0/1 conflict → X, Z yields), X before any write;
/// `posedge_event()` of a resolved signal; sc_in_resolved readers.
#[test]
fn resolved() {
    check("resolved");
}

/// §6.9 an sc_buffer bound to ports: a reader's port sees every write, and
/// so does a write of the same value through a writer's port (§6.4: a
/// port's event is its channel's). Supersedes the error test that rejected
/// binding a buffer to a port.
#[test]
fn buffer_port() {
    check("buffer_port");
}

/// §6.4.4 SC_MANY_WRITERS lets two processes write one signal; §5.17 object
/// names — given ones, generated `signal_0`/`port_0`/`fifo_0` per kind and
/// parent, sc_main's top-level ones, `basename()`.
#[test]
fn writers_and_names() {
    check("writers");
}

/// §6.4.4 SC_ONE_WRITER (the default): a second process writing a signal is
/// error E115 at that write, in the reference kernel's words, and the run
/// ends with exit status 1.
#[test]
fn one_writer() {
    assert_eq!(check("e115"), Some(1));
}

/// §6.4.2/§6.23 a user struct as signal, port and FIFO payload: copies on
/// write (later changes of the writer's object do not show), a write equal
/// by `operator==` notifies nothing, `operator<<` prints it, a by-value
/// local is a copy; member functions and constructors.
#[test]
fn class_payload() {
    check("payload");
}

/// C++ classes in process bodies: initializer lists, a base-class
/// constructor, virtual functions through base pointers, objects copied by
/// value (assignment, by-value parameters) versus shared through pointers
/// and references, a static member, `new` and `delete`.
#[test]
fn class_objects() {
    check("objects");
}

/// C++ templates: a module template with a type parameter at two types
/// (one SV module per instantiation), class templates with type and value
/// parameters (defaults, as another template's argument and as a base
/// class, the injected class name); arrays of signals.
#[test]
fn templates() {
    check("templates");
}

/// §5.12–§5.15 interfaces, ports and exports: a hierarchical FIFO channel
/// implementing two interfaces, bound through a parent's port and through
/// an export of an export; a multiport (`size()`, `p[i]->`), static
/// sensitivity to a port's default event, a call through an export from
/// sc_main. The FIFO's last batch notifies an even number of times.
#[test]
fn channels() {
    check("channels");
}

/// §5.15 a primitive channel: `request_update()` and one `update()` per
/// delta cycle, reads seeing the new value only after it.
#[test]
fn prim_channel() {
    check("primchan");
}

/// §6.27–§6.29 sc_mutex (owner-only unlock, trylock), sc_semaphore
/// (trywait, post, wait) and sc_event_queue (several notifications,
/// cancel_all).
#[test]
fn kernel_objects() {
    check("kernelobj");
}

/// §6.23 sc_fifo: written values become readable, and freed space
/// writable, only after the update phase; nb_read/nb_write/num_free.
#[test]
fn fifo_update_phase() {
    check("fifoupd");
}

/// C++ destruction: sc_main's modules when it returns (last first), member
/// modules after their parent's destructor body.
#[test]
fn destructors() {
    check("dtors");
}

/// §4.2.1.3 `wait(SC_ZERO_TIME)` and `notify(SC_ZERO_TIME)` resume in the
/// next delta cycle, after the update phase (signal writes visible).
#[test]
fn delta_wait() {
    check("deltawait");
}

/// §5.10.7 event lists: `wait(e1 & e2)` (each notified, in any order),
/// `wait(e1 | e2)`, and both with a timeout. Supersedes the error test
/// `event_and_list`.
#[test]
fn event_lists() {
    check("andlist");
}

/// §5.2.17 next_trigger(): a method's next activation waits for a time, an
/// event, an AND/OR list, a timeout or a delta cycle, once; then its static
/// sensitivity again.
#[test]
fn next_trigger() {
    check("nexttrig");
}

/// §5.5–§5.6 sc_spawn of member and free functions with sc_bind (bound
/// arguments copied, sc_ref/sc_cref shared), a result through `&r`,
/// SC_FORK/SC_JOIN, process handles (`terminated`, `terminated_event`),
/// function-local events passed by reference.
#[test]
fn spawn() {
    check("spawn");
}

/// §4.4 callbacks in the kernel's hierarchy order; end_of_simulation after
/// sc_stop.
#[test]
fn callbacks() {
    check("callbacks");
}

/// `sc_assert` (fatal error F4, in a process) and `assert`: exit 134.
#[test]
fn asserts() {
    assert_eq!(check("asserts"), Some(134));
}
