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
    // 40 of 42 checks pass; the two %g exponent-style diffs (1.23e-5 vs
    // 1.23e-05) were explicitly accepted by the reporter. Ratchet on the
    // exact counts.
    let msgs = outputs(include_str!("issue_cases/fmt.specifiers.sv"), 100_000);
    let pass = msgs.iter().filter(|m| m.starts_with("[PASS]")).count();
    let fail = msgs.iter().filter(|m| m.starts_with("[ERROR]")).count();
    assert_eq!(pass, 40, "passing-check count changed: {:?}", msgs);
    assert_eq!(fail, 2, "only the two accepted %g style diffs may fail");
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
