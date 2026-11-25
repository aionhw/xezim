//! IEEE 1800-2023 clause 20: utility system tasks and functions.

use crate::harness::{Order, check};

// §20.10: severity system tasks and final blocks after $fatal
#[test]
fn c20_10_severity_tasks() {
    // Not compared: $system output goes to the shell, not the simulator output.
    check(
        "c20_sev",
        include_str!("sv/c20_sev.sv"),
        "c20s",
        Order::Exact,
        &[
            "T|20.10a|info 1",
            "T|20.10b|warn w",
            "T|20.10c|err",
            "T|20.10d|after error",
            "T|20.10e|fatal msg 5",
            "T|20.2d|final after fatal t=5",
        ],
        &["T|20.17|"],
    );
}

// §20.12: Assertion control: $assertoff/$asserton/$assertkill/$assertpassoff/$assertfailoff/$assertcontrol
#[test]
fn c20_12_assertion_control() {
    check(
        "20.12_assertion_control",
        include_str!("sv/20.12_assertion_control.sv"),
        "t20_12",
        Order::Exact,
        &[
            "T|a|base fails=3",
            "T|b|after $assertoff fails=3 (unchanged expected)",
            "T|c|after $asserton fails=5",
            "T|d|after $assertcontrol(off) fails=5",
        ],
        &[],
    );
}

// §20.12: assertion control: $assertoff/$asserton/$assertpassoff/$assertfailoff
#[test]
fn c20_12_assertion_control_tasks() {
    // Not compared: after `$assertpasson(0, ap)` the reference runs the PASS
    // action of every later failure of `ap` (with `a` held at 0 it counts
    // passes); xezim runs the fail action, as §20.12 specifies.
    check(
        "c16_ctl",
        include_str!("sv/c16_ctl.sv"),
        "c16_ctl",
        Order::Exact,
        &[
            "T|20.12|base passes=0 fails=3",
            "T|20.12|after off passes=0 fails=3",
            "T|20.12|after on passes=0 fails=5",
            "T|20.12|passoff passes=0 fails=5",
            "T|16|ap2 F t=115",
            "T|20.12|passon passes=2 fails=5",
            "T|20.12|after kill",
        ],
        &[
            "T|20.12|failoff passes=",
            "T|20.12|failon passes=",
            "T|20.12|ctl off passes=",
            "T|20.12|ctl on passes=",
            "T|20.12|global off passes=",
        ],
    );
}

// §20.17: $system
#[test]
fn c20_17_system_output_order() {
    // Not compared: $system output goes to the shell, not the simulator output.
    check(
        "20.17_system_output_order",
        include_str!("sv/20.17_system_output_order.sv"),
        "fsys",
        Order::Exact,
        &["T|20.17|before", "T|20.17|after"],
        &["T|20.17|from_system"],
    );
}

// §20.7: Array query functions: out-of-range dimension argument and associative arrays
#[test]
fn c20_7_array_query_packed_dims() {
    // Known gap (§20.7): `20.7c` reference `left3=7 size3=8 size4=4 inc2=1 size5=x`, xezim `left3=7 size3=8 size4=4 inc2=1 size5=32`
    // Known gap (§20.7): `20.7d` reference `1`, xezim `32`
    check(
        "20.7_array_query_packed_dims",
        include_str!("sv/20.7_array_query_packed_dims.sv"),
        "faq",
        Order::Exact,
        &["T|20.7a|dims=4 udims=2"],
        &["T|20.7c|", "T|20.7d|"],
    );
}

// §20.2, §20.3, §20.4.2, §20.5, §20.6.2, §20.6.3, ...: simulation time, conversion, data query, array query, math functions
#[test]
fn c20_utility_system_functions() {
    // Not compared: $typename text and the sign of a printed NaN (implementation-defined).
    // Not compared (§20.6.2): `$bits(real)`: the reference returns 0; xezim returns 64, as the clause specifies
    // Known gap (§20.6.2): `20.6.2b` reference `96`, xezim `64`
    // Known gap (§20.7): `20.7c` reference `7 0 8 1`, xezim `7 0 8 -1`
    // Known gap (§20.7): `20.7d` reference `4 3 2 1`, xezim `4 3 2 32`
    // Known gap (§20.7): `20.7f` reference `x`, xezim `32`
    // Known gap (§20.15.1): `20.15.1a` reference `-2147138048 230383387 -2147138048 same=1`, xezim `1351845 336141829 1351845 same=1`
    // Known gap (§20.15.2): `20.15.2a` reference `0 11 475628535`, xezim `93 39 67634689`
    // Known gap (§20.15.2): `20.15.2b` reference `62 14`, xezim `80 1`
    // Known gap (§20.15.2): `20.15.2c` reference `0 2 0`, xezim `0 4 -1`
    // Known gap (§20.15.2): `20.15.2d` reference `47`, xezim `50`
    // Known gap (§20.15.1): `20.15.1b` reference `-8`, xezim `7`
    check(
        "c20_util",
        include_str!("sv/c20_util.sv"),
        "c20",
        Order::Exact,
        &[
            "T|20.3sub|300 3 3.330000",
            "T|20.3a|100000 1 1 1.235000",
            "T|20.3b|              100000|",
            "T|20.4.2a|[  1235.00 ns]",
            "T|20.4.2b|[1235.00 ns]",
            "T|20.4.2c|[1us]",
            "T|20.4.2d|[  1500000.000ps]",
            "T|20.4.2e|[  1235000.000ps]",
            "T|20.5a|5.000000 -3 3",
            "T|20.5b|3ff0000000000000",
            "T|20.5c|2.000000",
            "T|20.5d|3fc00000 3.141593",
            "T|20.5e|-4 12",
            "T|20.6.2a|8 4",
            "T|20.6.3|0 0",
            "T|20.7a|4 2 1",
            "T|20.7b|2 5 2 5 -1 4",
            "T|20.7e|7 1",
            "T|20.8.1a|0 0 1 3 32",
            "T|20.8.2a|0.000000 3.000000 1.000000 4.000000",
            "T|20.8.2b|8.000000 -2.000000 -1.000000 0.000000",
            "T|20.8.2c|1.000000 0.000000 1.570796 0.000000",
            "T|20.8.2d|0.785398 0.785398 5.000000 0.000000",
            "T|20.8.2e|1.000000 0.000000 0.000000 0.000000 0.000000",
            "T|20.9a|3 2 3",
            "T|20.9b|4 0",
            "T|20.9c|1 0 0",
            "T|20.9d|1 1 0",
            "T|20.9e|1 0 1",
            "T|20.15.3a|same=1",
            "T|20.15.3|range ok",
            "T|20.10|after error",
            "T|20.2|before finish t=2000000.000ps",
            "T|20.2c|final",
        ],
        &[
            "T|20.6.1|",
            "T|20.6.1b|",
            "T|20.8.2f|",
            "T|20.6.2r|",
            "T|20.6.2b|",
            "T|20.7c|",
            "T|20.7d|",
            "T|20.7f|",
            "T|20.15.1a|",
            "T|20.15.2a|",
            "T|20.15.2b|",
            "T|20.15.2c|",
            "T|20.15.2d|",
            "T|20.15.1b|",
        ],
    );
}

#[test]
fn itor_preserves_width_and_rounds_real_arguments_1641() {
    let sim = xezim::simulate(
        r#"
module top;
  localparam real folded_wide = $itor(80'h1_0000_0000_0000_0000);
  localparam real folded_real = $itor(5.75);
  localparam real folded_signed = $itor(-80'sd18446744073709551616);
  bit [63:0] u = 64'hffff_ffff_ffff_ffff;
  bit [79:0] w = 80'h2_0000_0000_0000_0000;
  logic signed [79:0] s = -80'sd18446744073709551616;
  real fraction = -2.5;
  real near_zero = -0.4;
  initial begin
    #1;
    $display("itor=%b%b%b%b%b%b%b%b",
        folded_wide == 18446744073709551616.0,
        folded_real == 6.0,
        folded_signed == -18446744073709551616.0,
        $itor(u) == 18446744073709551616.0,
        $itor(w) == 36893488147419103232.0,
        $itor(s) == -18446744073709551616.0,
        $itor(fraction) == -3.0,
        $realtobits($itor(near_zero)) === 64'd0);
    $finish;
  end
endmodule
"#,
        1_000_000,
    )
    .expect("simulation");
    assert!(
        sim.output
            .iter()
            .any(|line| line.message.trim() == "itor=11111111"),
        "{:?}",
        sim.output
    );
}
