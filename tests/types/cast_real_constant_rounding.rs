//! §6.12.1 and §6.24.1: an integral cast rounds as assignment does, to the
//! nearest integer with exact half-integer ties away from zero.
//! The expected values were cross-checked with iverilog or Verilator.

use xezim::simulate;

fn check(src: &str, expected: &[&str]) {
    let sim = simulate(src, 10).expect("simulate failed");
    let lines: Vec<_> = sim.output.iter().map(|line| line.message.trim()).collect();
    for want in expected {
        assert!(lines.contains(want), "missing {want:?} in {lines:?}");
    }
}

#[test]
fn constant_integral_cast_rounds_near_half_like_runtime() {
    check(
        r#"
module top;
  localparam real BELOW = 0.49999999999999994;
  localparam int POS = int'(BELOW);
  localparam shortint NEG = shortint'(-BELOW);
  localparam longint LARGE_POS = longint'(4503599627370497.0);
  localparam longint LARGE_NEG = longint'(-4503599627370497.0);
  localparam time UNSIGNED_LARGE = time'(9223372036854775808.0);
  real live;
  wire [31:0] compiled_result = int'(live);
  initial begin
    live = BELOW;
    #1;
    $display("near %0d %0d %0d %0d", POS, NEG, int'(-live), compiled_result);
    $display("large %0d %0d", LARGE_POS, LARGE_NEG);
    $display("time %0d", UNSIGNED_LARGE);
    $finish;
  end
endmodule
"#,
        &[
            "near 0 0 0 0",
            "large 4503599627370497 -4503599627370497",
            "time 9223372036854775808",
        ],
    );
}

/// A constant function (§13.4.3) can cast to a wide signed integral typedef.
#[test]
fn constant_integral_cast_keeps_wide_twos_complement_bits() {
    check(
        r#"
module top;
  typedef logic signed [95:0] wide_t;
  function automatic wide_t convert(input real operand);
    return wide_t'(operand);
  endfunction
  localparam wide_t NEG = convert(-2.75);
  localparam wide_t BIG = convert(18446744073709551616.0);
  initial begin
    $display("wide %024h %024h", NEG, BIG);
    $finish;
  end
endmodule
"#,
        &["wide fffffffffffffffffffffffd 000000010000000000000000"],
    );
}

/// The enum base type sets rounding and operand width (§6.19.4, §13.4.3).
#[test]
fn constant_function_cast_uses_enum_base_type() {
    check(
        r#"
module top;
  typedef enum bit [2:0] {SMALL_ZERO, SMALL_ONE} small_t;
  typedef enum int {WORD_ZERO, WORD_ONE} word_t;
  function automatic int round_enum(input real operand);
    return int'(small_t'(operand));
  endfunction
  function automatic int add_enum(input bit [2:0] a, b);
    return int'(word_t'(a + b));
  endfunction
  localparam int POS = round_enum(0.49999999999999994);
  localparam int NEG = round_enum(-0.49999999999999994);
  localparam int TIE = round_enum(0.5);
  localparam int SUM = add_enum(3'd7, 3'd6);
  initial begin
    $display("enum %0d %0d %0d %0d", POS, NEG, TIE, SUM);
    $finish;
  end
endmodule
"#,
        &["enum 0 0 1 13"],
    );
}

/// Named targets convert the real before arithmetic or range evaluation
/// (§6.19.4, §6.20.3, §6.24.1, §7.2.1, §7.4.1).
#[test]
fn constant_named_integral_cast_converts_before_using_result() {
    check(
        r#"
package cast_types;
  typedef int word_t;
endpackage
module top #(parameter type T = byte);
  typedef int word_t;
  typedef enum int {ZERO, ONE} enum_t;
  typedef struct packed { bit [3:0] hi, lo; } packed_t;
  localparam A = word_t'(0.4) * 3;
  localparam B = cast_types::word_t'(2.5) * 2;
  localparam C = T'(-2.75) * 3;
  localparam D = enum_t'(0.4) * 3;
  localparam E = packed_t'(0.4) * 3;
  localparam F = 5'(word_t'(0.49999999999999994));
  logic [word_t'(1.4) + 2:0] v;
  initial begin
    $display("named %0d %0d %0d %0d %0d %0d %0d", A, B, C, D, E, F, $bits(v));
    $finish;
  end
endmodule
"#,
        &["named 0 6 -9 0 0 0 4"],
    );
}

/// Guard exact ties, signed and unsigned targets, and the context width
/// of an integral operand (§6.24.1, §11.6.1).
#[test]
fn constant_integral_cast_preserves_ties_signing_and_operand_width() {
    check(
        r#"
class real_base;
  typedef real word_t;
endclass
class real_derived extends real_base;
endclass
module top;
  typedef int word_t;
  localparam int POS_TIE = int'(0.5), NEG_TIE = int'(-0.5);
  localparam int ORDINARY = int'(-2.75);
  localparam int ABOVE = int'(0.5000000000000001);
  localparam int SIGNED_BYTE = int'(byte'(-2.75));
  localparam int UNSIGNED_BIT = int'(bit'(-0.5));
  localparam int WIDTH_CTX = int'(3'd7 + 3'd6);
  localparam int SIZE_CTX = 5'(3'd7 + 3'd6);
  localparam real SCOPED_REAL = real_derived::word_t'(0.4);
  initial begin
    $display("round %0d %0d %0d %0d", POS_TIE, NEG_TIE, ORDINARY, ABOVE);
    $display("types %0d %0d %0d %0d", SIGNED_BYTE, UNSIGNED_BIT, WIDTH_CTX, SIZE_CTX);
    $display("scope %0.1f", SCOPED_REAL);
    $finish;
  end
endmodule
"#,
        &["round 1 -1 -3 1", "types -3 1 13 13", "scope 0.4"],
    );
}
