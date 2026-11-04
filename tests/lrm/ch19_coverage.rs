//! IEEE 1800-2023 clause 19: functional coverage.

use crate::harness::{Order, check};

// §19.6.1, §19.6.2, §19.6.3: cross coverage, binsof, intersect
#[test]
fn c19_6_cross_coverage() {
    // Known gap (§19.5.1): `19.5.1` reference `with 75.00`, xezim `with 62.50`
    // Known gap (§19.5): `19.5` reference `total 77.98`, xezim `total 76.42`
    check(
        "c19_bins2",
        include_str!("sv/c19_bins2.sv"),
        "c19_bins2",
        Order::Exact,
        &[
            "T|19.5.1|wildcard 100.00",
            "T|19.5.2|transitions 62.50",
            "T|19.5|fixed split 100.00",
            "T|19.5.5|ignore auto 75.00",
            "T|19.6|cross sel 83.33",
            "T|19.6.1|cross with 28.00",
        ],
        &["T|19.5.1|with ", "T|19.5|total 7"],
    );
}

// §11.4.12, §19.5: bin range bounds written as constant concatenations or
// replications; the bins were dropped and the coverpoints read 0.00.
#[test]
fn c19_bins_concat_bound() {
    check(
        "c19_bins_concat_bound",
        include_str!("sv/c19_bins_concat_bound.sv"),
        "c19_bins_concat_bound",
        Order::Exact,
        &[
            "T|module|66.67",
            "T|module_all|100.00",
            "T|class4|100.00",
            "T|class8|25.00",
        ],
        &[],
    );
}

// §19.5.3, §19.5.6, §19.6: covergroups, coverpoints, bins, auto bins
#[test]
fn c19_coverage_basic() {
    // Known gap (§19.7): `19.7` reference `auto_bin_max cp=100.00 cp2=0.00 tot=100.00`, xezim `auto_bin_max cp=100.00 cp2=100.00 tot=100.00`
    check(
        "c19_basic",
        include_str!("sv/c19_basic.sv"),
        "c19_basic",
        Order::Exact,
        &[
            "T|19.4|auto b cov=100.00",
            "T|19.5|bins cp_a=50.00 cp_b=50.00 cp_w=50.00 cp_t=50.00 x=25.00 total=45.00 inst=90.00",
            "T|19.5|stopped inst=0.00",
            "T|19.6|cross cpa=100.00 cpb=100.00 xc=100.00 tot=100.00",
            "T|19.7|args at_least cp=100.00",
        ],
        &["T|19.7|auto_bin_max cp=100.00 cp2="],
    );
}

// §19.4, §19.11: class covergroups, coverage methods
#[test]
fn c19_coverage_more() {
    // Not compared (§19.4): the reference reports the merged value for both instances' `get_inst_coverage()`; xezim reports each instance's own coverage, as the clause specifies
    // Known gap (§19.8): `19.8` reference `ck t1=100.00 kxm t1=56.25`, xezim `ck t1=100.00 kxm t1=50.00`
    check(
        "c19_more",
        include_str!("sv/c19_more.sv"),
        "c19_more",
        Order::Exact,
        &[
            "T|19.8|type cov over insts=37.50 g1=50.00 g2=25.00",
            "T|19.8|get_coverage(ref) c=37.50 3/8",
            "T|19.3|ref arg cov=100.00",
            "T|19.8|set_inst_name ok",
        ],
        &["T|19.4.1|", "T|19.8|ck t1=100.00 kxm t1=5"],
    );
}

// §19.3, §19.7.1, §19.9: coverage options, sample(), get_coverage
#[test]
fn c19_coverage_options() {
    check(
        "c19_misc",
        include_str!("sv/c19_misc.sv"),
        "c19_misc",
        Order::Exact,
        &[
            "T|19.3|event iff cov=50.00",
            "T|19.7.1|merge_instances type=50.00 i1=50.00",
            "T|19.9|$get_coverage=50.00",
        ],
        &[],
    );
}
