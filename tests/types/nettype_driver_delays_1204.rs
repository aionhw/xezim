//! Issue #1204: each continuous driver of a user-defined-nettype net keeps
//! its own delay (§10.3.3) in front of the resolution function, and the
//! net's declaration initializer is itself a driver. A delayed driver's
//! contribution holds at the nettype's 0.0 for its first delay period while
//! an undelayed driver contributes at once; the resolver re-runs whenever
//! any single driver's contribution changes. A resolver keyed on driver
//! count must see the declaration initializer (§6.6.7, §10.3.1). Expected
//! values were checked against the reference simulator.
#[test]
fn issue_1204_nettype_driver_delays() {
    let sim = xezim::simulate(include_str!("nettype_driver_delays_1204.sv"), 100_000)
        .expect("simulate nettype driver delays");
    let msgs: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert!(
        msgs.iter().any(|m| m == "PASS"),
        "nettype per-driver delays must all pass: {msgs:#?}"
    );
    assert!(!msgs.iter().any(|m| m.starts_with("FAIL")), "{msgs:#?}");
}
