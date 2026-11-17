//! Issue #1239: a chained `always @(n.f or n.v)` nettype relay must
//! re-evaluate when its input settles during the final cascade settle, not
//! only on a later edge check. Two successive relays each `assign` their
//! net (with a resolver) from a variable that a blocking-write always block
//! drives; the downstream relay's input changes after the cascade's last
//! edge check, so the relay must fire again in the same time step (§9.4.2).
//! Covers a 3-level chain and a second update. Expected values were checked
//! against the reference simulator.
#[test]
fn issue_1239_nettype_relay_chain() {
    let sim = xezim::simulate(include_str!("nettype_relay_chain_1239.sv"), 100_000)
        .expect("simulate nettype relay chain");
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        msgs.iter().any(|m| m == "PASS"),
        "chained nettype relay must re-evaluate in the same time step: {msgs:#?}"
    );
    assert!(!msgs.iter().any(|m| m.starts_with("FAIL")), "{msgs:#?}");
}
