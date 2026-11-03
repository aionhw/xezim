//! IEEE 1800-2023 clause 14: clocking blocks.

use crate::harness::{Order, check};

// §14.10: clocking block events
#[test]
fn c14_10_clocking_block_events() {
    check(
        "c14_more",
        include_str!("sv/c14_more.sv"),
        "c14x",
        Order::Exact,
        &[
            "T|14.9a|t=15 req=11 gnt_sampled=01",
            "T|14.9b|t=27 bidir=5a cb.bidir=5a",
            "T|14.16.2|tgt=02",
            "T|14.10|cb3.ev_sig changed t=65 val=3",
        ],
        &[],
    );
}

// §14.14, §6.24.2: global clocking, $cast on enums
#[test]
fn c14_14_global_clocking_and_6_24_2_cast() {
    check(
        "c_left",
        include_str!("sv/c_left.sv"),
        "cleft",
        Order::Exact,
        &["T|20.11|fails=1", "T|6.24.2d|e=B", "T|14.14|gcnt=3"],
        &[],
    );
}

// §14.16.2: Driving an inout clockvar that is a net inside an interface
#[test]
fn c14_16_clocking_inout_iface_net() {
    check(
        "14.16_clocking_inout_iface_net",
        include_str!("sv/14.16_clocking_inout_iface_net.sv"),
        "rvd",
        Order::Exact,
        &["T|r1|t=8 req=11 bidir=5a", "T|r2|t=18 bidir=77"],
        &[],
    );
}

// §14.3, §14.5: clockvars bound to expressions in interface instances
#[test]
fn c14_5_clocking_expr_iface() {
    check(
        "14.5_clocking_expr_iface",
        include_str!("sv/14.5_clocking_expr_iface.sv"),
        "top",
        Order::Exact,
        &["T|q|u1=1 u2=0", "T|d|u1=1 u2=0"],
        &[],
    );
}

// §14.4: Output skew (default output #2)
#[test]
fn c14_16_clocking_output_skew() {
    check(
        "14.16_clocking_output_skew",
        include_str!("sv/14.16_clocking_output_skew.sv"),
        "rcb",
        Order::Exact,
        &[
            "T|r1|t=5000 cb.d=0 cb.dl=0 d=1",
            "T|r1|t=15000 cb.d=1 cb.dl=1 d=2",
            "T|r1|t=25000 cb.d=2 cb.dl=2 d=3",
            "T|r1|t=35000 cb.d=3 cb.dl=3 d=4",
            "T|r2|t=45000 cb.d=3 d=4",
            "T|r2|t=55000 cb.d=4 d=5",
            "T|r3|t=56000 dout=xx",
            "T|r4|t=58000 dout=ab",
        ],
        &[],
    );
}

// §14.5: a clockvar bound to a part-select samples and drives that slice
#[test]
fn c14_5_clockvar_slice_binding() {
    check(
        "14.5_clockvar_slice_binding",
        include_str!("sv/14.5_clockvar_slice_binding.sv"),
        "c145s",
        Order::Exact,
        &[
            "T|14.5|in a=a hi=5",
            "T|14.5|out o_drv=09",
            "T|14.5|out2 o_drv=c9",
            "T|14.5|stale a=a",
            "T|14.5|fresh a=3 hi=c",
        ],
        &[],
    );
}

// §14.4, §14.16: the output skew of a renamed clockvar applies to drives
// issued between clocking events
#[test]
fn c14_4_renamed_clockvar_output_skew() {
    check(
        "14.4_renamed_clockvar_output_skew",
        include_str!("sv/14.4_renamed_clockvar_output_skew.sv"),
        "c144r",
        Order::Exact,
        &["T|14.4|q t=17", "T|14.4|r t=28"],
        &[],
    );
}

// §14.3: clocking block inout signals
#[test]
fn c14_3_clocking_inout_signals() {
    check(
        "r_cbinout",
        include_str!("sv/r_cbinout.sv"),
        "rci",
        Order::Exact,
        &[
            "T|r1|t=5 cb.gnt=0 gnt=1",
            "T|r2|t=8 bidir=5a",
            "T|r3|t=15 cb.bidir=5a",
        ],
        &[],
    );
}

// §14.4, §14.13: Input and output skews: input sampling (#1step, #N)
#[test]
fn c14_4_clocking_input_sampling() {
    check(
        "14.4_clocking_input_sampling",
        include_str!("sv/14.4_clocking_input_sampling.sv"),
        "rcb",
        Order::Exact,
        &[
            "T|r1|t=5000 cb.d=0 cb.dl=0 d=1",
            "T|r1|t=15000 cb.d=1 cb.dl=1 d=2",
            "T|r1|t=25000 cb.d=2 cb.dl=2 d=3",
            "T|r1|t=35000 cb.d=3 cb.dl=3 d=4",
            "T|r2|t=45000 cb.d=3 d=4",
            "T|r2|t=55000 cb.d=4 d=5",
            "T|r3|t=56000 dout=xx",
            "T|r4|t=58000 dout=ab",
        ],
        &[],
    );
}

// §14.3, §14.4, §14.11, §14.12, §14.16: clocking blocks: skews, cycle delays, default clocking, ## drives
#[test]
fn c14_clocking_blocks() {
    check(
        "c14_clk",
        include_str!("sv/c14_clk.sv"),
        "c14",
        Order::Exact,
        &[
            "T|14.13a|t=5000 cb.d=0 d=1",
            "T|14.13b|t=15000 cb.d=1 d=2 cb.dl=1",
            "T|14.16a|t=15000 dout=xx",
            "T|14.16b|t=16000 dout=xx",
            "T|14.16c|t=18000 dout=ab",
            "T|14.11a|t=35000",
            "T|14.11b|t=35000",
            "T|14.16d|t=35000 dout=ab",
            "T|14.16e|t=45000 dout=ab",
            "T|14.16f|t=58000 dout=cd",
            "T|14.4|t=60000 d0=6 d=6",
            "T|14.12|t=65000 dcb.d=6",
            "T|14.13c|t=95000 n=3",
            "T|14.16g|t=108000 dout=11",
        ],
        &[],
    );
}
