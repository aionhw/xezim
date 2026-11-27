//! IEEE 1800-2023 §19.5.1, §19.5.5, §19.7.1 and §19.11.1.

use xezim::simulate;

fn check(point: &str, group_options: &str, axis: &str, samples: &[(f64, usize)], total: usize) {
    let mut stimulus = String::from("report();\n");
    for (v, _) in samples {
        stimulus.push_str(&format!("x = {v:?}; #1; c.sample(); report();\n"));
    }
    let source = format!(
        r#"`timescale 1ns/1ps
module top;
  localparam real LO = 0.0, HI = 3.3, W = 0.1, V = 1.8;
  localparam int N = 10;
  localparam real MAX_WIDTH = 1.7976931348623157e308;
  real x;
  bit enabled;
  covergroup cg;
    option.per_instance = 1;
    {group_options}
    {point}
  endgroup
  cg c;
  task automatic report();
    int h, n;
    real pct;
    pct = c.{axis}.get_inst_coverage(h, n);
    $display("coverage %0d/%0d %0.2f %0.2f", h, n, pct, c.{axis}.get_coverage());
  endtask
  initial begin
    c = new;
    enabled = 1;
    {stimulus}
  end
endmodule"#
    );
    let sim = simulate(&source, 100_000).expect("simulate covergroup");
    assert_eq!(sim.error_count, 0, "{source}");
    let actual: Vec<_> = sim
        .output
        .iter()
        .map(|l| l.message.trim())
        .filter(|l| l.starts_with("coverage "))
        .collect();
    let expected: Vec<_> = std::iter::once(0)
        .chain(samples.iter().map(|&(_, hits)| hits))
        .map(|hits| {
            let pct = 100.0 * hits as f64 / total as f64;
            format!("coverage {hits}/{total} {pct:.2} {pct:.2}")
        })
        .collect();
    assert_eq!(actual, expected, "{source}");
}

const HALF: &str = "type_option.real_interval = 0.5;";

/// §19.5.1: both array forms distribute whole real intervals.
#[test]
fn open_and_fixed_arrays_count_both_intervals() {
    for size in ["", "2"] {
        check(
            &format!("cp: coverpoint x {{ bins b[{size}] = {{[1.0:2.0]}}; }}"),
            HALF,
            "cp",
            &[(1.25, 1), (1.75, 2)],
            2,
        );
    }
}

/// §19.5.1, §19.7.1: the point width overrides the group width and the final bin is closed.
#[test]
fn partial_last_interval_includes_the_upper_bound() {
    check(
        "cp: coverpoint x { type_option.real_interval = 0.5; bins b[] = {[-0.75:0.4]}; }",
        "type_option.real_interval = 1.0;",
        "cp",
        &[(-0.25, 1), (0.4, 2), (0.25, 2), (0.4001, 2), (-0.75, 3)],
        3,
    );
}

/// §19.5.1: the exact high endpoint belongs to the final decimal interval.
#[test]
fn decimal_width_and_closed_upper_bound() {
    check(
        "cp: coverpoint x { type_option.real_interval = 0.01; bins b[] = {[0.75:0.85]}; }",
        "",
        "cp",
        &[(0.85, 1), (0.84, 1), (0.75, 2)],
        10,
    );
}

/// §19.5.1: identical open intervals merge, while distinct overlapping bins each count.
#[test]
fn open_array_merges_only_identical_intervals() {
    check(
        "cp: coverpoint x { bins b[] = {[1.0:3.0], [1.0:2.0], [1.0:3.0], 1.0}; }",
        "",
        "cp",
        &[(1.0, 3), (1.5, 3), (2.0, 4), (3.0, 4)],
        4,
    );
}

/// §19.5.1, §19.5.5, §19.11.1: distribute duplicate items before excluding empty slots.
#[test]
fn distribute_ordered_items_before_removing_empty_bins() {
    check(
        "cp: coverpoint x { option.at_least = 2; bins b[2] = {3.0, [1.0:2.0], 3.0, 7.0}; }",
        HALF,
        "cp",
        &[(3.0, 0), (7.0, 1), (1.0, 2)],
        2,
    );
    check(
        "cp: coverpoint x { bins b[5] = {[$:0.75], 1.0, [1.5:$]};
                             ignore_bins skip = {1.0}; }",
        HALF,
        "cp",
        &[(1.0, 0), (0.75, 1), (2.0, 2)],
        2,
    );
}

/// §19.6, §19.7.1: the cross uses the bins formed with the inherited width.
#[test]
fn cross_uses_the_inherited_intervals() {
    check(
        "cp: coverpoint x { bins span[] = {[1.0:2.0]}; }
           side: coverpoint enabled { bins on_bin = {1}; }
           pair: cross cp, side;",
        HALF,
        "pair",
        &[(1.5, 1), (2.0, 1), (1.0, 2)],
        2,
    );
}

/// §19.5.1: scalar real bins and integral bin arrays retain their specified counts.
#[test]
fn scalar_real_and_integral_array_controls() {
    let samples = [(1.25, 1), (1.75, 2)];
    check(
        "cp: coverpoint x { bins span = {[1.0:2.0]}; }",
        HALF,
        "cp",
        &[(1.25, 1), (1.75, 1)],
        1,
    );
    check(
        "cp: coverpoint x { bins lo = {[1.0:1.49]}; bins hi = {[1.5:2.0]}; }",
        HALF,
        "cp",
        &samples,
        2,
    );
    check(
        "cp: coverpoint int'(x) { bins span[] = {[1:2]}; }",
        HALF,
        "cp",
        &samples,
        2,
    );
}

/// §19.5.1: sampling a decimal boundary hits the interval starting there.
#[test]
fn decimal_inner_boundaries() {
    let samples = (0..=10)
        .map(|i| (f64::from(i) / 10.0, (i as usize + 1).min(10)))
        .collect::<Vec<_>>();
    check(
        "cp: coverpoint x { bins b[] = {[0.0:1.0]}; }",
        "type_option.real_interval = 0.1;",
        "cp",
        &samples,
        10,
    );
}

/// §19.5.1: evenly divisible ranges have no remainder item in either distribution.
#[test]
fn decimal_ranges_divide_evenly() {
    check(
        "cp: coverpoint x { bins b[] = {[0.0:0.9]}; }",
        "type_option.real_interval = 0.3;",
        "cp",
        &[(0.1, 1), (0.4, 2), (0.7, 3), (0.9, 3)],
        3,
    );
    check(
        "cp: coverpoint x { bins b[2] = {[0.0:0.9]}; }",
        "type_option.real_interval = 0.3;",
        "cp",
        &[(0.4, 1), (0.1, 2)],
        2,
    );
    check(
        "cp: coverpoint x { bins b[] = {[-1.8:0.2]}; }",
        "",
        "cp",
        &[(-1.8, 1), (-0.8, 2), (0.2, 2)],
        2,
    );
}

/// §19.5.1, §11.4.13: an empty range contributes no items to the array.
#[test]
fn reversed_range_beside_real_intervals() {
    check(
        "cp: coverpoint x { bins b[] = {[1.0:2.0], [5.0:4.0]}; }",
        HALF,
        "cp",
        &[(1.25, 1), (1.75, 2)],
        2,
    );
}

/// §19.5.1, §19.6.1: an intersect at a half-open boundary selects only the upper bin.
#[test]
fn real_intersect_selects_cross_products() {
    for bins in [
        "bins b[] = {[1.0:2.0]};",
        "bins b[2] = {[1.0:2.0]};",
        "bins lo = {[1.0:1.49]}; bins hi = {[1.5:2.0]};",
    ] {
        check(
            &format!(
                "cp: coverpoint x {{ {bins} }}
                 side: coverpoint (x == 1.125) {{ bins on_bin = {{1}}; bins off_bin = {{0}}; }}
                 pair: cross cp, side {{ ignore_bins sel = binsof(cp) intersect {{[1.5:2.0]}}; }}"
            ),
            HALF,
            "pair",
            &[(1.25, 1), (1.125, 2), (1.5, 2)],
            2,
        );
    }
}

/// §19.3, §19.5.1: each instance's bounds determine its interval names and counters.
#[test]
fn constructor_bounds_determine_interval_names() {
    let sim = simulate(
        r#"`timescale 1ns/1ps
module top;
  real x;
  covergroup cg(real hi);
    option.per_instance = 1;
    type_option.real_interval = 0.1;
    cp: coverpoint x { bins b[] = {[0.0:hi]}; }
  endgroup
  cg short_range, long_range;
  int hs, ns, hl, nl;
  real pct;
  initial begin
    short_range = new(0.3);
    long_range = new(0.4);
    x = 0.3;
    #1;
    short_range.sample();
    long_range.sample();
    pct = short_range.cp.get_inst_coverage(hs, ns);
    pct = long_range.cp.get_inst_coverage(hl, nl);
    $display("coverage %0d/%0d %0d/%0d", hs, ns, hl, nl);
  end
endmodule"#,
        100_000,
    )
    .expect("simulate two covergroup instances");
    assert_eq!(sim.error_count, 0);
    assert!(
        sim.output
            .iter()
            .any(|line| line.message.trim() == "coverage 1/3 1/4")
    );
    assert_eq!(sim.coverpoint_bin_hits("cg", "cp.b[0.2:0.3]"), 1);
    assert_eq!(sim.coverpoint_bin_hits("cg", "cp.b[0.3:0.4]"), 1);
}

/// §19.5.1: a rounded quotient partitions the range without an extra final item.
#[test]
fn computed_width_divides_open_and_fixed_arrays() {
    check(
        "cp: coverpoint x { bins b[] = {[0.0:1.0]}; }",
        "type_option.real_interval = 1.0/3.0;",
        "cp",
        &[(0.1, 1), (0.5, 2), (0.9, 3), (1.0, 3)],
        3,
    );
    check(
        "cp: coverpoint x { bins b[2] = {[0.0:2.0]}; }",
        "type_option.real_interval = 2.0/3.0;",
        "cp",
        &[(1.0, 1), (0.5, 2)],
        2,
    );
    check(
        "cp: coverpoint x { bins b[] = {[LO:HI]}; }",
        "type_option.real_interval = (HI-LO)/N;",
        "cp",
        &[(0.1, 1), (3.3, 2)],
        10,
    );
}

/// §19.5.1: rounding a computed bound must preserve the whole-interval distribution.
#[test]
fn computed_bounds_divide_open_and_fixed_arrays() {
    check(
        "cp: coverpoint x { bins b[] = {[LO:LO+3*W]}; }",
        "type_option.real_interval = W;",
        "cp",
        &[(0.05, 1), (0.15, 2), (0.25, 3), (0.3, 3)],
        3,
    );
    check(
        "cp: coverpoint x { bins b[2] = {[LO:LO+3*W]}; }",
        "type_option.real_interval = W;",
        "cp",
        &[(0.15, 1), (0.05, 2)],
        2,
    );
    check(
        "cp: coverpoint x { bins b[] = {[1.0-6*W:1.0]}; }",
        "type_option.real_interval = W;",
        "cp",
        &[(0.45, 1), (0.95, 2), (1.0, 2)],
        6,
    );
    check(
        "cp: coverpoint x { bins b[] = {[V*0.9:V*1.1]}; }",
        "type_option.real_interval = V*0.05;",
        "cp",
        &[(1.63, 1), (1.75, 2), (1.85, 3), (1.97, 4)],
        4,
    );
}

/// §19.5.1: a range with a genuine remainder has a closed final partition.
#[test]
fn partial_interval_keeps_its_own_bin() {
    check(
        "cp: coverpoint x { bins b[] = {[0.0:1.0000000000000002]}; }",
        "",
        "cp",
        &[(0.5, 1), (1.0, 2), (1.0000000000000002, 2)],
        2,
    );
    check(
        "cp: coverpoint x { bins b[] = {[0.0:0.31]}; }",
        "type_option.real_interval = 0.1;",
        "cp",
        &[(0.25, 1), (0.3, 2), (0.31, 2)],
        4,
    );
    check(
        "cp: coverpoint x { bins b[] = {[-MAX_WIDTH:MAX_WIDTH/2.0]}; }",
        "type_option.real_interval = MAX_WIDTH;",
        "cp",
        &[(-1.0, 1), (0.0, 2)],
        2,
    );
}
