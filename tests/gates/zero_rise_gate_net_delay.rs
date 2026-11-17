//! IEEE 1800-2023 §28.16/§10.3.3: even a zero-rise primitive has its
//! own delay stage when its fall delay is nonzero. A fall at t=21 reaches
//! the gate output at 26 and the #(2,3) net at 29. All expectations follow
//! those transition times; no sample coincides with an update (§4.7).

fn check(src: &str, expected: &[&str]) {
    let sim = xezim::simulate(src, 1_000_000).expect("simulate failed");
    let got: Vec<_> = sim
        .output
        .iter()
        .map(|o| o.message.trim())
        .filter(|s| s.starts_with("T|"))
        .collect();
    assert_eq!(got, expected);
}

#[test]
fn logic_primitive_falls_keep_net_delay_and_tristate_guards_stay_unchanged() {
    check(
        include_str!("zero_rise_gate_net_delay.sv"),
        &[
            "T|t=24.5 gates=111111111111111111",
            "T|t=26.5 gates=11111111",
            "T|t=28.5 gates=11111111",
            "T|t=29.5 gates=000000000000000000",
            "T|t=41.5 gates=000000000000000000",
            "T|t=42.5 gates=000000000000000000",
            "T|t=43.5 gates=111111111111111111",
            "T|t=62.5 gates=111111111111111111",
            "T|t=63.5 gates=11111111zzzzzzzzzz",
        ],
    );
}

#[test]
fn adjacent_gate_assign_net_port_and_array_delays_keep_their_stages() {
    check(
        include_str!("zero_rise_gate_net_delay_guards.sv"),
        &[
            "T|early=0110111111111",
            "T|gate_fell=0010111011111",
            "T|implicit_drivers=00",
            "T|generated_name=1",
            "T|single_fell=01111111",
            "T|net_fell=0000000000000",
            "T|rise_pending=0000000100000",
            "T|rise=1000111111110",
            "T|settled=1111111111111",
            "T|off_pending=1",
            "T|off_pending=1",
            "T|off=z",
        ],
    );
}

#[test]
fn staged_buffer_keeps_z_to_x_and_main_correct_case_comparisons() {
    check(
        include_str!("zero_rise_gate_net_delay_unknown.sv"),
        &[
            "T|truth=x is_z=0 conditional_is_z=0 mos_is_x=0",
            "T|settled=0",
        ],
    );
}
