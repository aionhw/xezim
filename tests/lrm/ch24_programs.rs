//! IEEE 1800-2023 clause 24: programs.

use crate::harness::{Order, check};

// §3.4, §24.3, §26.2, §26.3, §26.6, §27.3, ...: programs, packages and generate constructs
#[test]
fn c24_26_27_programs_packages_generate() {
    // Known gap (§26.3): `26.3b` reference `2 2`, xezim `51 1`
    // Known gap (§24.3): `24.3b`: xezim prints extra `po=5`
    check(
        "c24_26_27",
        include_str!("sv/c24_26_27.sv"),
        "c2427",
        Order::Exact,
        &[
            "T|26.3a|5 GREEN",
            "T|26.3c|50 7",
            "T|26.3d|12",
            "T|26.6|3",
            "T|26.2|99 3 1001",
            "T|26.2b|99",
            "T|27.4a|010 20",
            "T|27.4b|1",
            "T|27.5a|2 3",
            "T|27.5b|300 100",
            "T|27.4c|2 3",
            "T|27.6|5",
            "T|27.3|1",
            "T|24.3a|program runs in reactive t=5",
            "T|24.7|program final",
        ],
        &["T|26.3b|", "T|24.3b|"],
    );
}

// §24.3: program driving an interface modport
#[test]
fn c24_3_program_interface_drive() {
    // Known gap (§24.3): `24.3nest`: xezim misses `d=33`
    check(
        "r_progif",
        include_str!("sv/r_progif.sv"),
        "r_progif",
        Order::Exact,
        &["T|24.3if|d=5a"],
        &["T|24.3nest|"],
    );
}

// §24.3: program reactive region, clocking drives from programs
#[test]
fn c24_3_program_scheduling() {
    // Known gap (§24.3): `24.3a` reference `t=5 q=1 cnt=1`, xezim `t=5 q=0 cnt=1`
    // Known gap (§24.3): `24.3b` reference `t=15 samp=2 seen_by_ff=7`, xezim `t=15 samp=1 seen_by_ff=7`
    // Known gap (§24.3): `24.3c` reference `t=25 cb.d=10 d=20`, xezim `t=15 cb.d=0 d=10`
    check(
        "p24a",
        include_str!("sv/p24a.sv"),
        "t24a",
        Order::Exact,
        &["T|24.3d|pv=6"],
        &["T|24.3a|", "T|24.3b|", "T|24.3c|"],
    );
}

// §24.7: implicit $finish, $exit, threads of an ending program
#[test]
fn c24_7_program_control() {
    // Known gap (§24.7): `24.7a`: xezim misses 2 reference lines
    // Known gap (§24.7): `24.7b`: xezim misses `p1 done t=10`
    // Known gap (§24.7): `24.7g` reference `end t=10`, xezim `end t=4`
    check(
        "p24c",
        include_str!("sv/p24c.sv"),
        "t24c",
        Order::Exact,
        &["T|24.7a|p1 child t=3", "T|24.7c|p2 exit t=4"],
        &[
            "T|24.7a|p1 child t=6",
            "T|24.7a|p1 child t=9",
            "T|24.7b|",
            "T|24.7g|",
        ],
    );
}

// §24.7: Program control: simulation ends when all programs' initial blocks finish
#[test]
fn c24_7_program_implicit_finish() {
    // §24.3, §24.7: the run ends when both programs have, at t=35, not at
    // the module's #1000; the final procedures run in an arbitrary order
    // (§9.2.3).
    check(
        "24.7_program_implicit_finish",
        include_str!("sv/24.7_program_implicit_finish.sv"),
        "c24x",
        Order::Multiset,
        &[
            "T|24.7a|p1 done t=15",
            "T|24.7b|p2 done t=35",
            "T|24.7c|p2 final t=35",
            "T|24.7e|module final t=35",
        ],
        &[],
    );
}

// §24: module vs program processes at the same edge
#[test]
fn c24_program_region_order() {
    // §24.3: a program process resumes in the Reactive region, after the
    // module's inactive and NBA work of the slot.
    check(
        "p24b",
        include_str!("sv/p24b.sv"),
        "t24b",
        Order::Exact,
        &[
            "T|24.ord|t=5 module active",
            "T|24.ord|t=5 module inactive",
            "T|24.ord|t=5 module after nba q=1",
            "T|24.ord|t=5 program",
            "T|24.ord|t=15 module active",
            "T|24.ord|t=15 module inactive",
            "T|24.ord|t=15 module after nba q=0",
            "T|24.ord|t=15 program",
            "T|24.ord|t=25 module active",
            "T|24.ord|t=25 module inactive",
            "T|24.ord|t=25 module after nba q=1",
            "T|24.ord|t=25 program",
        ],
        &["T|24.7f|"],
    );
    // §24.7: the run ends when the program does, at t=25; the final
    // procedures run in an arbitrary order (§9.2.3).
    check(
        "p24b",
        include_str!("sv/p24b.sv"),
        "t24b",
        Order::Multiset,
        &["T|24.7f|program final", "T|24.7f|module final t=25"],
        &["T|24.ord|"],
    );
}
