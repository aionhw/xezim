//! IEEE 1800-2023 clause 10: assignment statements.
//!
//! Expectations updated for #566: `%p` prints vector members as `'h` hex,
//! a packed union's first member, `%0p` as `%p`, and no space before an
//! associative array's closing brace.

use crate::harness::{Order, check};

// §10.6.2: force/release on a bit-select or part-select of a net
#[test]
fn c10_6_2_force_net_bitselect() {
    check(
        "10.6.2_force_net_bitselect",
        include_str!("sv/10.6.2_force_net_bitselect.sv"),
        "rfb",
        Order::Exact,
        &["T|r1|w=00110001", "T|r2|w=00110000", "T|r3|w=11110000"],
        &[],
    );
}

// §10.6.2: force net bitselect same step
#[test]
fn c10_6_2_force_net_bitselect_same_step() {
    check(
        "10.6.2_force_net_bitselect_same_step",
        include_str!("sv/10.6.2_force_net_bitselect_same_step.sv"),
        "ffb2",
        Order::Exact,
        &["T|10.6.2|w=11110001"],
        &[],
    );
}

// §10.3.3, §10.4.1, §10.4.2, §10.4, §10.6.1, §10.8, ...: continuous assigns, net delays, NBA ordering, assign/deassign, force/release, patterns
#[test]
fn c10_assignments() {
    // Known gap (§10.3.3): `10.3.3h` reference `tz=0`, xezim `tz=z`
    // Known gap (§10.3.4): `10.3.4b` reference `sw2=1 Pu1 sw3=z HiZ`, xezim `sw2=x WeX sw3=1 Hi1`
    check(
        "c10_assign",
        include_str!("sv/c10_assign.sv"),
        "c10",
        Order::Exact,
        &[
            "T|10.3.3a|wd=x wn=x",
            "T|10.11|al1=6",
            "T|10.3.3b|wd=5 wn=5",
            "T|10.3.3c|wd=5",
            "T|10.3.3d|pd=0 (pulse filtered)",
            "T|10.3.3e|rf=1",
            "T|10.3.3f|rf=1",
            "T|10.3.3g|rf=0",
            "T|10.3.3i|tz=z",
            "T|10.3.4a|sw=x WeX",
            "T|10.4.2a|1 2",
            "T|10.4.2b|2 1",
            "T|10.4.2c|'{7, 0, 9, 0}",
            "T|10.6.1a|pv=c",
            "T|10.6.1b|pv=c",
            "T|10.6.1c|pv=2",
            "T|10.6.2a|fv=ff",
            "T|10.6.2b|fv=ff",
            "T|10.6.2c|fnet=9",
            "T|10.6.2d|fnet=9",
            "T|10.6.2e|fnet=2",
            "T|10.6.2f|fwb=00110001",
            "T|10.6.2g|fwb=00110000",
            "T|10.6.2h|fwb=11110000",
            "T|10.6.2l|fv=03",
            "T|10.6.2m|fv=04",
            "T|10.9.1a|'{5, 5, 5, 5}",
            "T|10.9.1b|'{0, 10, 0, 30}",
            "T|10.9.1c|'{8, 8, 8, 8}",
            "T|10.9.1d|'{1, 2, 3, 4}",
            "T|10.9.1e|10111111",
            "T|10.9.1f|'{'hf, 'hf}",
            "T|10.9.2|'{x:1, y:'{2, 3}}",
            "T|10.9.2b|'{x:4, y:'{4, 4}}",
            "T|10.9.1g|'{'{7, 7}, '{7, 7}}",
            "T|10.10a|'{1, 2, 3, 4, 5}",
            "T|10.10b|10",
            "T|10.10c|'{1, 1, 2}",
            "T|10.10d|0",
            "T|10.8a|11111110",
            "T|10.8b|b",
            "T|10.8c|11111000",
            "T|10.4.cmp|7",
            "T|10.4.cmp2|-4",
            "T|10.4.cmp3|10",
        ],
        &["T|10.3.3h|", "T|10.3.4b|"],
    );
}

// §10.3.1, §10.3.2, §10.6.2: net declaration assignments, strengths on continuous assigns, force/release
#[test]
fn c10_assignments_more() {
    // Known gap (§10.3.4): `10.3.4c` reference `w1=1 St1 w2=x WeX w3=0 Pu0`, xezim `w1=1 Pu1 w2=x WeX w3=0 Pu0`
    check(
        "c10_more",
        include_str!("sv/c10_more.sv"),
        "c10x",
        Order::Exact,
        &[
            "T|10.3.2|cv=4",
            "T|10.3.4d|bus=zz",
            "T|10.3.4e|bus=11",
            "T|10.3.4f|bus=00xx00xx",
            "T|10.3.4g|bus=22",
            "T|10.3.1a|wdl=x",
            "T|10.3.1b|wdl=1",
            "T|10.3.2b|cv=8 psn=17 pss=17 split=7a",
            "T|10.6.2n|fw=f",
            "T|10.6.2o|fw=1",
            "T|10.6.2p|cv=0",
            "T|10.6.2q|cv=2",
            "T|10.6.2r|55",
            "T|10.6.2s|55",
        ],
        &["T|10.3.4c|"],
    );
}
