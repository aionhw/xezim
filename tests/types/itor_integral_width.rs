//! IEEE 1800-2023 §6.11.3, §6.12.1 and §20.5: `$itor` converts the
//! integer's signed value or unsigned magnitude to the nearest binary64.
//! A real argument rounds as in an assignment to an integer, ties away from zero.

fn check(src: &str, expected: &[&str]) {
    let result = xezim::simulate(src, 10_000).expect("simulate $itor conversions");
    let output: Vec<_> = result
        .output
        .iter()
        .map(|line| line.message.trim())
        .filter(|line| line.starts_with("T|"))
        .collect();
    assert_eq!(output, expected);
}

#[test]
fn runtime_preserves_full_integer_value() {
    check(
        r#"
`timescale 1ns/1ps
module top;
  bit [63:0] ones = 64'hffff_ffff_ffff_ffff;
  bit [63:0] high = 64'h8000_0000_0000_0000;
  bit [64:0] wide = 65'h1_ffff_ffff_ffff_ffff;
  bit [127:0] below_top = 128'h0000_0000_0000_0001_0000_0000_0000_0005;
  bit signed [64:0] signed_wide = 65'sh1_0000_0000_0000_0000;
  bit [64:0] tie_up = 65'h1_0000_0000_0000_1800;
  bit signed [65:0] negative_tie = -66'sd18446744073709553664;
  logic [64:0] unknown_top = {1'bx, 1'b1, 62'b0, 1'bz};
  real converted;
  assign converted = $itor(ones); // Variable continuous assignment (§6.5, §10.3).

  initial begin
    #1;
    $display("T|ones|%b", $itor(ones) == 18446744073709551616.0);
    $display("T|high|%b", $itor(high) == 9223372036854775808.0);
    $display("T|wide|%b", $itor(wide) == 36893488147419103232.0);
    $display("T|below top|%b", $itor(below_top) == 18446744073709551616.0);
    $display("T|signed wide|%b", $itor(signed_wide) == -18446744073709551616.0);
    // Integral-to-binary64 rounding at halfway points chooses the even significand.
    $display("T|wide tie up|%b", $itor(tie_up) == 18446744073709559808.0);
    $display("T|wide negative tie|%b", $itor(negative_tie) == -18446744073709551616.0);
    $display("T|unknown top|%b", $itor(unknown_top) == 9223372036854775808.0);
    $display("T|shift|%b", $itor(ones >> 0) == 18446744073709551616.0);
    $display("T|continuous|%b", converted == 18446744073709551616.0);
    ones = high;
    #1;
    $display("T|continuous update|%b", converted == 9223372036854775808.0);
    $finish;
  end
endmodule
"#,
        &[
            "T|ones|1",
            "T|high|1",
            "T|wide|1",
            "T|below top|1",
            "T|signed wide|1",
            "T|wide tie up|1",
            "T|wide negative tie|1",
            "T|unknown top|1",
            "T|shift|1",
            "T|continuous|1",
            "T|continuous update|1",
        ],
    );
}

#[test]
fn real_arguments_round_before_conversion() {
    check(
        r#"
`timescale 1ns/1ps
module top;
  // §11.2.1, §20.5: conversion system functions in constant expressions.
  localparam real CONSTANT = $itor(5.75);
  localparam real CONSTANT_NEGATIVE_TIE = $itor(-2.5);
  localparam real CONSTANT_ZERO = $itor(-0.4);
  real v = 5.75;
  real negative_fraction = -0.4;
  initial begin
    #1;
    $display("T|real literal|%b", $itor(5.75) == 6.0);
    $display("T|real expression|%b", $itor(v * 2.0) == 12.0);
    $display("T|nested conversion|%b", $itor($itor(7)) == 7.0);
    // $sqrt returns a real (§20.8.2).
    $display("T|nested real function|%b", $itor($sqrt(16.0)) == 4.0);
    $display("T|positive tie|%b", $itor(2.5) == 3.0);
    $display("T|negative tie|%b", $itor(-2.5) == -3.0);
    $display("T|constant real|%b", CONSTANT == 6.0);
    $display("T|constant negative tie|%b", CONSTANT_NEGATIVE_TIE == -3.0);
    // Integer zero converts to +0.0. Check the sign with $realtobits (§20.5).
    $display("T|runtime positive zero|%b", $realtobits($itor(negative_fraction)) == 64'h0);
    $display("T|constant positive zero|%b", $realtobits(CONSTANT_ZERO) == 64'h0);
    $finish;
  end
endmodule
"#,
        &[
            "T|real literal|1",
            "T|real expression|1",
            "T|nested conversion|1",
            "T|nested real function|1",
            "T|positive tie|1",
            "T|negative tie|1",
            "T|constant real|1",
            "T|constant negative tie|1",
            "T|runtime positive zero|1",
            "T|constant positive zero|1",
        ],
    );
}

#[test]
fn nearby_conversions_keep_their_values() {
    check(
        r#"
`timescale 1ns/1ps
module top;
  // Conversion system functions in constant expressions (§11.2.1, §13.4.3).
  function automatic real convert(input bit [63:0] arg);
    return $itor(arg);
  endfunction
  localparam real DIRECT = $itor(64'hffff_ffff_ffff_ffff);
  localparam real THROUGH_FUNCTION = convert(64'h8000_0000_0000_0000);
  bit [63:0] u = '1;
  bit signed [63:0] s = 64'sh8000_0000_0000_0000;
  logic [7:0] mixed = 8'b1x0z_01xz;
  real implicit_conversion;
  initial begin
    #1;
    implicit_conversion = u;
    $display("T|constant direct|%b", DIRECT == 18446744073709551616.0);
    $display("T|constant function|%b", THROUGH_FUNCTION == 9223372036854775808.0);
    $display("T|cast|%b", real'(u) == 18446744073709551616.0);
    $display("T|assignment|%b", implicit_conversion == 18446744073709551616.0);
    $display("T|multiply|%b", u * 1.0 == 18446744073709551616.0);
    $display("T|signed 64|%b", $itor(s) == -9223372036854775808.0);
    $display("T|signed literal|%b", $itor(-5) == -5.0);
    $display("T|unsigned 32|%b", $itor(32'hffff_ffff) == 4294967295.0);
    $display("T|unknown bits|%b", $itor(mixed) == 132.0);
    $display("T|round trip|%0d", $rtoi($itor(-5)));
    $finish;
  end
endmodule
"#,
        &[
            "T|constant direct|1",
            "T|constant function|1",
            "T|cast|1",
            "T|assignment|1",
            "T|multiply|1",
            "T|signed 64|1",
            "T|signed literal|1",
            "T|unsigned 32|1",
            "T|unknown bits|1",
            "T|round trip|-5",
        ],
    );
}
