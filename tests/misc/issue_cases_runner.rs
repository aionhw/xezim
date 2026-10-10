//! The reporters' own test cases from GitHub issues #17/#22/#23/#24/#25,
//! run verbatim as regressions (they were previously verified by hand only).
//! Plus two formerly-ORPHANED .sv testcases under tests/ that no cargo suite
//! ever executed: fork_wait_deadlock.sv (passing) and
//! dpi/force_release_compliance.sv (ratcheted — see its test).

use xezim::simulate;

fn outputs(src: &str, max_time: u64) -> Vec<String> {
    let sim = simulate(src, max_time).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

#[test]
fn issue_17_dynamic_array_of_mailboxes() {
    let msgs = outputs(include_str!("../issue_cases/dyn.arr.of.mbox.sv"), 100_000);
    let data_lines = msgs.iter().filter(|m| m.starts_with("Data from")).count();
    assert_eq!(data_lines, 80, "all 5x16 mailbox entries must round-trip");
    assert!(
        !msgs.iter().any(|m| m.contains("[x]")),
        "no unbound foreach index may appear"
    );
}

#[test]
fn issue_17_mailbox_in_interface() {
    let msgs = outputs(
        include_str!("../issue_cases/mbox_in_interface.sv"),
        2_000_000,
    );
    let received = msgs.iter().filter(|m| m.contains("Received")).count();
    // 1000 post-reset cycles: sender0 every 3, sender1 every 5.
    assert_eq!(received, 533, "1000/3 + 1000/5 sender puts must arrive");
}

#[test]
fn issue_22_final_blocks() {
    let msgs = outputs(
        include_str!("../issue_cases/final.blocks.test.case.sv"),
        100_000,
    );
    assert!(msgs.iter().any(|m| m.contains("TEST PASSED")), "{:?}", msgs);
    let finals = msgs.iter().filter(|m| m.contains("inal block")).count();
    assert!(finals >= 4, "all four final blocks must run: {:?}", msgs);
}

#[test]
fn issue_23_string_methods() {
    let msgs = outputs(
        include_str!("../issue_cases/string.compliance.tests.sv"),
        100_000,
    );
    assert!(msgs.iter().any(|m| m.contains("TEST PASSED")), "{:?}", msgs);
}

#[test]
fn issue_24_swrite_sformat() {
    let msgs = outputs(
        include_str!("../issue_cases/data.to.string.fmt.sv"),
        100_000,
    );
    assert!(msgs.iter().any(|m| m.contains("TEST PASSED")), "{:?}", msgs);
}

#[test]
fn issue_25_format_specifiers() {
    let msgs = outputs(include_str!("../issue_cases/fmt.specifiers.sv"), 100_000);
    let pass = msgs.iter().filter(|m| m.starts_with("[PASS]")).count();
    let fail = msgs.iter().filter(|m| m.starts_with("[ERROR]")).count();
    assert_eq!(pass, 42, "passing-check count changed: {:?}", msgs);
    assert_eq!(fail, 0, "all format specifier checks should pass");
}

#[test]
fn orphan_fork_wait_deadlock() {
    let msgs = outputs(include_str!("../fork_wait_deadlock.sv"), 100_000);
    assert!(
        msgs.iter()
            .any(|m| m.contains("PASS: fork-local variable sharing works")),
        "{:?}",
        msgs
    );
}

#[test]
fn fork_child_blocking_live_share() {
    // The UVM sequencer watchdog (`m_safe_select_item`) writes a task-local
    // automatic from a `fork … join_none` child that then blocks forever on
    // `await()`, while the parent parks on `wait(select_process != null)`. A
    // merge-on-child-completion model can never deliver the value (the child
    // never completes) — the write must be LIVE-shared with the suspended
    // parent. This regression compiles that exact child-blocks-forever idiom.
    let msgs = outputs(include_str!("../fork_child_blocking_share.sv"), 100_000);
    assert!(
        msgs.iter()
            .any(|m| m.contains("PASS: fork-child live-shared write wakes a parked wait")),
        "{:?}",
        msgs
    );
    assert!(!msgs.iter().any(|m| m.starts_with("FAIL")), "{:?}", msgs);
}

#[test]
fn fork_child_merge_wake() {
    // A fork...join_none child whose first action is a #0 deferral runs after
    // the parent parks, so its write to the parent's task-local automatic is
    // delivered back to the suspended parent through the child->parent frame
    // merge (the path Fix #2's wake-promotion touches) — a deferred child
    // write must still re-check and wake the parked level-sensitive `wait`.
    let msgs = outputs(include_str!("../fork_child_merge_wake.sv"), 100_000);
    assert!(
        msgs.iter().any(|m| m.contains("PASS result=1")),
        "{:?}",
        msgs
    );
    assert!(!msgs.iter().any(|m| m.starts_with("FAIL")), "{:?}", msgs);
}

#[test]
fn orphan_force_release_compliance_ratchet() {
    let msgs = outputs(include_str!("../dpi/force_release_compliance.sv"), 100_000);
    let fails = msgs.iter().filter(|m| m.starts_with("FAIL")).count();
    assert_eq!(
        fails,
        0,
        "force/release known-gap count changed — new regression or a fixed \
         gap (lower the count): {:?}",
        msgs.iter()
            .filter(|m| m.starts_with("FAIL"))
            .collect::<Vec<_>>()
    );
}

#[test]
fn issue_21_timescale_handling() {
    // §3.14.3 precision quantization + per-module directive scales.
    let msgs = outputs(
        include_str!("../issue_cases/timescale.handling.sv"),
        1_000_000,
    );
    assert!(msgs.iter().any(|m| m.contains("TEST PASSED")), "{:?}", msgs);
}

#[test]
fn issue_18_type_parameters() {
    // §6.20.3 type params: structs, arrays, class handles.
    let msgs = outputs(
        include_str!("../issue_cases/type-parameter-compliance.sv"),
        100_000,
    );
    assert!(msgs.iter().any(|m| m.contains("TEST PASSED")), "{:?}", msgs);
}

#[test]
fn issue_28_constraint_foreach() {
    // §18.5.7 foreach constraint bodies beyond `inside`.
    let msgs = outputs(
        include_str!("../issue_cases/constraint.foreach.sv"),
        100_000,
    );
    assert!(msgs.iter().any(|m| m.contains("TEST_PASS")), "{:?}", msgs);
}

#[test]
fn issue_29_constraint_typecast() {
    // §18.3/§6.24.1/§11.6.1 casts inside constraint expressions.
    let msgs = outputs(
        include_str!("../issue_cases/constraint.typecast.sv"),
        100_000,
    );
    assert!(msgs.iter().any(|m| m.contains("TEST_PASS")), "{:?}", msgs);
}

#[test]
fn issue_26_static_init_sysfuncs() {
    // §6.21/§10.5 sim-time syscall inits + §20.6 type operands
    // ($size(logic [7:0])). The test reads plusargs, so it needs the
    // args-aware entry point.
    let src = include_str!("../issue_cases/static.init.sysfuncs.sv").to_string();
    let plusargs = vec!["TEST_MODE".to_string(), "SEED_VAL=42".to_string()];
    let sim = xezim::simulate_multi(
        &[src],
        100_000,
        None,
        &[],
        &[],
        None,
        false,
        None,
        None,
        &[],
        &plusargs,
        None,
        &[],
        0,
        u64::MAX,
        None,
        &[],
        None,
        None,
        None,
        None,
        false,
    )
    .expect("simulate failed");
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(msgs.iter().any(|m| m.contains("TEST_PASS")), "{:?}", msgs);
}

/// Issue #233: the reporter's parenless-call matrix (enum methods on locals,
/// foreach keys and class members; free, package and interface functions;
/// string and queue built-ins), run verbatim.
#[test]
fn issue_233_parenless_calls() {
    let msgs = outputs(include_str!("../issue_cases/parenless.calls.sv"), 100_000);
    assert!(!msgs.iter().any(|m| m.starts_with("FAIL")), "{:#?}", msgs);
    assert!(msgs.iter().any(|m| m == "TEST_PASS"), "{:#?}", msgs);
}

/// Issue #246: the reporter's randomize()-over-struct-fields MWE, run
/// verbatim — member dist/inside/relational/equality targets, part-selects,
/// nested members, unpacked members, `unique` on a 2-D array, the handle vs
/// method storage views, and the nested rand-handle solve that used to spin
/// for minutes (it must finish well inside the run).
#[test]
fn issue_246_struct_rand_fields() {
    let start = std::time::Instant::now();
    let msgs = outputs(
        include_str!("../issue_cases/struct.rand.fields.sv"),
        100_000,
    );
    assert!(!msgs.iter().any(|m| m.starts_with("FAIL")), "{:#?}", msgs);
    assert!(msgs.iter().any(|m| m == "TEST_PASS"), "{:#?}", msgs);
    assert!(
        start.elapsed() < std::time::Duration::from_secs(60),
        "took {:?}",
        start.elapsed()
    );
}

/// The randomize() width-overflow family and dist obedience MWE, run
/// verbatim: `==` / `!=` forcing a value wider than its rand variable, a
/// >64-bit rand struct with member-sum constraints, and the dist-frequency
/// and width-boundary guards around them. `tests/classes/rand_width_overflow.rs`
/// restates each section as its own test.
#[test]
fn rand_width_overflow_and_dist_mwe() {
    let msgs = outputs(include_str!("../issue_cases/rand.width.dist.sv"), 100_000);
    assert!(!msgs.iter().any(|m| m.starts_with("FAIL")), "{:#?}", msgs);
    assert!(msgs.iter().any(|m| m == "TEST_PASS"), "{:#?}", msgs);
}

/// Issue #262: $fscanf %c must match exactly one character (§21.3.4.3) —
/// no whitespace skipping for %c or literal-only formats, newlines kept
/// across repeated %c, all 256 byte values round-tripping through
/// $fwrite/$fscanf %c, task-form $fscanf converting, and C-stdio pushback
/// semantics ($ftell subtracts, $fread drains, $fseek/$rewind discard).
#[test]
fn issue_262_fscanf_percent_c() {
    let msgs = outputs(include_str!("../issue_cases/fscanf_262.sv"), 100_000);
    assert!(
        !msgs.iter().any(|m| m.starts_with("FAIL")),
        "{:#?}",
        msgs
    );
    assert!(msgs.iter().any(|m| m == "TEST_PASS"), "{:#?}", msgs);
}