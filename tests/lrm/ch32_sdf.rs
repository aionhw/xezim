//! IEEE 1800-2023 clause 32: backannotation using SDF.

use crate::harness::{Order, check_files};

// §32: SDF annotation: $sdf_annotate IOPATH rise/fall (min:typ:max)
#[test]
fn c32_sdf_iopath_rise_fall() {
    check_files(
        "32_sdf_iopath_rise_fall",
        include_str!("sv/32_sdf_iopath_rise_fall.sv"),
        "c32_sdf",
        Order::Exact,
        &["T|32|t=8000 y=0", "T|32|t=25000 y=1", "T|32|t=48000 y=0"],
        &[],
        &[],
        &["32_sdf_iopath_rise_fall.sdf"],
    );
}
