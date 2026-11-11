//! IEEE 1800-2023 §28.16, §10.3.2/§10.3.3: primitive and continuous
//! assignment delays belong to each driver, before §28.12 resolution.
//! §6.6.3 covers wired logic and §6.7.1 the separate net delay.
//! Procedural stimulus starts after time zero (§4.7/§6.8).
//! Expected samples follow the specified rise/fall/turn-off times and the
//! equal-strength wire rule (opposite values resolve to x).

#[test]
fn primitive_and_assign_drivers_keep_their_transition_delays() {
    let sim = xezim::simulate(include_str!("driver_transition_delays.sv"), 1_000)
        .expect("simulate failed");
    let got: Vec<_> = sim.output.iter().map(|o| o.message.trim()).collect();
    for expected in [
        "early pair=1",
        "fall mixed=x reversed=x pair=x gate=1 assign=0 equal=x and=0 or=1",
        "net fall=x",
        "fallen mixed=0 pair=0 gate=0 net=0",
        "rise mixed=1 pair=x gate=1 assign=1 net=0",
        "risen mixed=1 pair=1 net=1",
        "assign on=x",
        "assign off pending=x",
        "off settled=0",
    ] {
        assert!(got.contains(&expected), "missing {expected:?}: {got:#?}");
    }
}

// §28.6/§28.7/§28.9 and §28.12: retain delayed falls and resolved
// values with a z driver, an opposing driver, and unknown controls.
#[test]
fn tristate_and_mos_drivers_retain_their_existing_delays() {
    for src in [
        include_str!("tristate_driver_two_delays.sv"),
        include_str!("tristate_mos_driver_delays.sv"),
        include_str!("tristate_driver_controls.sv"),
        include_str!("tristate_child_driver_delays.sv"),
    ] {
        let sim = xezim::simulate(src, 1_000).expect("simulate failed");
        let got: Vec<_> = sim.output.iter().map(|o| o.message.trim()).collect();
        assert!(got.iter().any(|s| s.starts_with("PASS")), "{got:#?}");
        assert!(!got.iter().any(|s| s.starts_with("FAIL")), "{got:#?}");
    }
}
