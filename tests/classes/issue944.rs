//! Issue #944: a derived class `D` that shadows a base class `B`'s instance
//! property `X` with a *non-instance* member (`localparam` / `parameter` /
//! genuine `static` property) was diverting writes done in `B`'s lexical
//! scope -- `this.X = v` inside a base method, writes through any handle
//! whose static type is `B`, collection-element / function-result / static-
//! holder chains, and `randomize()` of a shadowed `rand` property -- into the
//! shadowing per-class static slot, leaving `B`'s instance cell un-written.
//!
//! Per LRM §8.14/§8.23 a `this.X` write in `B` resolves in `B`'s lexical
//! scope and always hits `B`'s instance property regardless of what a derived
//! class declares. The committed `issue944.sv` is a comprehensive, self-
//! checking suite (groups G0..G43, G23 omitted: a function-call result is not a
//! `variable_lvalue`, A.8.5) covering every write arm plus the controls; it
//! prints `TEST_PASS` only when all checks hold; the reference simulator
//! also prints `TEST_PASS` for it.

use xezim::simulate;

#[test]
fn issue944_shadow_diversion_does_not_steal_instance_writes() {
    let sim = simulate(include_str!("issue944.sv"), 1000).expect("simulate failed");
    let failures = sim
        .get_signal("tb_top.failures")
        .or_else(|| sim.get_signal("failures"))
        .unwrap_or_else(|| panic!("signal not found: failures (did the suite run?)"))
        .to_u64()
        .unwrap_or_else(|| panic!("failures not u64-able"));
    assert_eq!(
        failures, 0,
        "issue944 sweep: every G0..G43 check (G23 omitted) must hold (see FAIL lines above)"
    );
    assert!(
        sim.output.iter().any(|o| o.message.trim() == "TEST_PASS"),
        "SVTEST_PASSFAIL must print TEST_PASS"
    );
}
