//! IEEE 1800-2023 §11.4.13: a tolerance range `[A +/- B]` or `[A +%- B]`
//! converts the tolerance (`B`, or `A*B/100.0`) to the type of `A`,
//! truncating a real tolerance for an integral `A`, and orders the two
//! bounds. It is a `value_range`, so covergroup bins (§19.5.1) and
//! distribution items (§18.5.3) take it too (#1485). The reference simulator
//! predates 1800-2023 and rejects the syntax; the expected values follow the
//! clause.

use std::path::Path;
use std::process::Command;

fn run(fixture: &str) -> Vec<String> {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/types")
        .join(fixture);
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--sv2023", "--no-cache"])
        .arg(source)
        .output()
        .expect("run xezim");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{fixture}: {stdout}\n{stderr}");
    stdout
        .lines()
        .filter_map(|l| l.strip_prefix("T|"))
        .map(str::to_string)
        .collect()
}

#[test]
fn inside_converts_and_orders_the_tolerance() {
    assert_eq!(
        run("tolerance_range_inside.sv"),
        [
            // 25% of 7 truncates from 1.75 to 1: [6:8].
            "rel 0 1 1 0",
            // 1.75 truncates to 1 for the int centre, for real probes too.
            "abs 0 1 1 0",
            // A negative tolerance gives the same ordered [9:11].
            "neg 1 1 1 1",
            // -1 converted to `int unsigned` still orders to [9:11].
            "uns 1 1 0",
            // A real centre keeps a real tolerance: [8.375:8.625], [8.415:8.585].
            "real 1 0 1 0",
            "int-real 1 0",
            "case hit",
            "case miss",
            // A continuous `inside` follows a change of the centre.
            "cont 1",
            "cont 0",
            "constraint bad=0",
            "constraint 47:1 53:1 46:0",
        ]
    );
}

#[test]
fn bins_take_absolute_and_relative_tolerances() {
    // [3:7] and [90:110]; real bins [8.375:8.625] and [99.0:101.0]. Each bin
    // hit adds half the instance coverage (§19.11).
    assert_eq!(
        run("tolerance_range_bins.sv"),
        ["coverage 0 50 100", "real coverage 0 50 100"]
    );
}

#[test]
fn dist_items_take_absolute_and_relative_tolerances() {
    // Each item is [9:11] with either weight form; real items with `:/` are
    // accepted.
    assert_eq!(
        run("tolerance_range_dist.sv"),
        ["each 1 1 0", "total 1 1 0", "rel 1 1 0", "reltotal 1 1 0"]
    );
}

#[test]
fn real_constraint_ranges_randomize_fractional_values() {
    assert_eq!(run("tolerance_range_real_randomize.sv"), ["near=2 far=6"]);
}
