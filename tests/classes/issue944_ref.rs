//! Reference-validatable subset of issue #944 (see `issue944.sv` for the full
//! G0..G43 sweep). This standalone module contains every shadow-diversion case
//! the G23 arm excluded -- the free-function-result lvalue `mk_base().X = v`
//! (`issue944.sv` G23) crashes the reference simulator, so it is cut here.
//!
//! The self-checking `issue944_ref.sv` prints `TEST_PASS` only when every base
//! instance-property write lands in the base's cell despite a derived-class
//! `localparam`/`parameter`/`static` shadow. The output is validated
//! byte-for-byte against the reference simulator: both reach `TEST_PASS` with
//! identical FAIL/@ lines on every supported variant.

use xezim::simulate;

#[test]
fn issue944_shadow_diversion_reference_subset() {
    let sim = simulate(include_str!("issue944_ref.sv"), 1000).expect("simulate failed");
    let failures = sim
        .get_signal("top.failures")
        .or_else(|| sim.get_signal("failures"))
        .unwrap_or_else(|| panic!("signal not found: failures (did the suite run?)"))
        .to_u64()
        .unwrap_or_else(|| panic!("failures not u64-able"));
    assert_eq!(
        failures, 0,
        "issue944 reference subset: every check must hold (see FAIL lines above)"
    );
    assert!(
        sim.output.iter().any(|o| o.message.trim() == "TEST_PASS"),
        "SVTEST_PASSFAIL must print TEST_PASS"
    );
}