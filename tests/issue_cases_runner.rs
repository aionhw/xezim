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
    let msgs = outputs(include_str!("issue_cases/dyn.arr.of.mbox.sv"), 100_000);
    let data_lines = msgs.iter().filter(|m| m.starts_with("Data from")).count();
    assert_eq!(data_lines, 80, "all 5x16 mailbox entries must round-trip");
    assert!(
        !msgs.iter().any(|m| m.contains("[x]")),
        "no unbound foreach index may appear"
    );
}

#[test]
fn issue_17_mailbox_in_interface() {
    let msgs = outputs(include_str!("issue_cases/mbox_in_interface.sv"), 2_000_000);
    let received = msgs.iter().filter(|m| m.contains("Received")).count();
    // 1000 post-reset cycles: sender0 every 3, sender1 every 5.
    assert_eq!(received, 533, "1000/3 + 1000/5 sender puts must arrive");
}

#[test]
fn issue_22_final_blocks() {
    let msgs = outputs(include_str!("issue_cases/final.blocks.test.case.sv"), 100_000);
    assert!(msgs.iter().any(|m| m.contains("TEST PASSED")), "{:?}", msgs);
    let finals = msgs.iter().filter(|m| m.contains("inal block")).count();
    assert!(finals >= 4, "all four final blocks must run: {:?}", msgs);
}

#[test]
fn issue_23_string_methods() {
    let msgs = outputs(include_str!("issue_cases/string.compliance.tests.sv"), 100_000);
    assert!(msgs.iter().any(|m| m.contains("TEST PASSED")), "{:?}", msgs);
}

#[test]
fn issue_24_swrite_sformat() {
    let msgs = outputs(include_str!("issue_cases/data.to.string.fmt.sv"), 100_000);
    assert!(msgs.iter().any(|m| m.contains("TEST PASSED")), "{:?}", msgs);
}

#[test]
fn issue_25_format_specifiers() {
    let msgs = outputs(include_str!("issue_cases/fmt.specifiers.sv"), 100_000);
    let pass = msgs.iter().filter(|m| m.starts_with("[PASS]")).count();
    let fail = msgs.iter().filter(|m| m.starts_with("[ERROR]")).count();
    assert_eq!(pass, 42, "passing-check count changed: {:?}", msgs);
    assert_eq!(fail, 0, "all format specifier checks should pass");
}

#[test]
fn orphan_fork_wait_deadlock() {
    let msgs = outputs(include_str!("fork_wait_deadlock.sv"), 100_000);
    assert!(
        msgs.iter().any(|m| m.contains("PASS: fork-local variable sharing works")),
        "{:?}",
        msgs
    );
}

#[test]
fn orphan_force_release_compliance_ratchet() {
    let msgs = outputs(include_str!("dpi/force_release_compliance.sv"), 100_000);
    let fails = msgs.iter().filter(|m| m.starts_with("FAIL")).count();
    assert_eq!(
        fails, 0,
        "force/release known-gap count changed — new regression or a fixed \
         gap (lower the count): {:?}",
        msgs.iter().filter(|m| m.starts_with("FAIL")).collect::<Vec<_>>()
    );
}

#[test]
fn issue_21_timescale_handling() {
    // §3.14.3 precision quantization + per-module directive scales.
    let msgs = outputs(include_str!("issue_cases/timescale.handling.sv"), 1_000_000);
    assert!(msgs.iter().any(|m| m.contains("TEST PASSED")), "{:?}", msgs);
}

#[test]
fn issue_18_type_parameters() {
    // §6.20.3 type params: structs, arrays, class handles.
    let msgs = outputs(include_str!("issue_cases/type-parameter-compliance.sv"), 100_000);
    assert!(msgs.iter().any(|m| m.contains("TEST PASSED")), "{:?}", msgs);
}

#[test]
fn issue_28_constraint_foreach() {
    // §18.5.7 foreach constraint bodies beyond `inside`.
    let msgs = outputs(include_str!("issue_cases/constraint.foreach.sv"), 100_000);
    assert!(msgs.iter().any(|m| m.contains("TEST_PASS")), "{:?}", msgs);
}

#[test]
fn issue_29_constraint_typecast() {
    // §18.3/§6.24.1/§11.6.1 casts inside constraint expressions.
    let msgs = outputs(include_str!("issue_cases/constraint.typecast.sv"), 100_000);
    assert!(msgs.iter().any(|m| m.contains("TEST_PASS")), "{:?}", msgs);
}

#[test]
fn issue_26_static_init_sysfuncs() {
    // §6.21/§10.5 sim-time syscall inits + §20.6 type operands
    // ($size(logic [7:0])). The test reads plusargs, so it needs the
    // args-aware entry point.
    let src = include_str!("issue_cases/static.init.sysfuncs.sv").to_string();
    let plusargs = vec!["TEST_MODE".to_string(), "SEED_VAL=42".to_string()];
    let sim = xezim::simulate_multi(
        &[src], 100_000, None, &[], &[], None, false, None, None, &[],
        &plusargs, 1, None, &[], 0, u64::MAX, None, &[], None, None, None,
        None, false, None,
    )
    .expect("simulate failed");
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(msgs.iter().any(|m| m.contains("TEST_PASS")), "{:?}", msgs);
}
