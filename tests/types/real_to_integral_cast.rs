//! §6.24.1 and §6.12.1: an integral type cast rounds a real as an assignment
//! would, with ties away from zero.

use std::path::Path;
use std::process::Command;

fn run(name: &str, source: &str, jit: bool) -> String {
    let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}.sv"));
    std::fs::write(&path, source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_xezim"))
        .args(["--no-cache", "-s", "top"])
        .arg(path)
        .env("XEZIM_JIT", if jit { "1" } else { "0" })
        .env("XEZIM_FALLBACK_SITES", "1")
        .env("XEZIM_METHOD_TIER", "0")
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "simulation failed:\n{text}");
    text
}

fn check(name: &str, source: &str, expected: &[&str]) {
    for jit in [false, true] {
        let text = run(name, source, jit);
        for want in expected {
            assert!(
                text.lines().any(|line| line == *want),
                "missing `{want}` (jit={jit}):\n{text}"
            );
        }
    }
}

/// Continuous assignments (§10.3), net declaration assignments (§6.7.1),
/// input port connections (§23.3.3), and event controls (§9.4.2) all yield
/// an integral value. A following operator uses that value.
#[test]
fn real_to_integral_cast_rounds_in_compiled_expressions() {
    let src = r#"
module sink(input wire [63:0] value, output wire [63:0] echo);
  assign echo = value;
endmodule
module top;
  typedef bit [63:0] word_t;
  typedef longint unsigned unsigned_word_t;
  real r;
  wire [63:0] continuous_value;
  assign continuous_value = longint'(r);
  wire [63:0] net_value = longint'(r);
  wire [63:0] port_value;
  sink u(.value(longint'(r)), .echo(port_value));
  logic [63:0] always_value;
  always @(r) always_value = longint'(r);
  wire [63:0] named_value = word_t'(r);
  wire [63:0] unsigned_value = unsigned_word_t'(r);
  wire [63:0] incremented = longint'(r) + 1;
  initial begin
    #5 r = 2.5;
    #1 $display("positive %h %h %h %h %h %h %h",
      continuous_value, net_value, port_value, always_value, named_value,
      unsigned_value, incremented);
    #5 r = -2.5;
    #1 $display("negative %h %h %h %h %h %h %h",
      continuous_value, net_value, port_value, always_value, named_value,
      unsigned_value, incremented);
    $finish;
  end
endmodule
"#;
    check(
        "real_cast_contexts",
        src,
        &[
            concat!(
                "positive 0000000000000003 0000000000000003 0000000000000003 ",
                "0000000000000003 0000000000000003 0000000000000003 0000000000000004"
            ),
            concat!(
                "negative fffffffffffffffd fffffffffffffffd fffffffffffffffd ",
                "fffffffffffffffd fffffffffffffffd fffffffffffffffd fffffffffffffffe"
            ),
        ],
    );
}

/// The cast lends its context width (§10.7, §11.6.1), sign-extends a signed
/// integral operand (§10.7), and converts X/Z to zero for longint (§6.11.2).
#[test]
fn real_to_integral_cast_preserves_integral_operands() {
    let src = r#"
module top;
  bit [7:0] a = 200, b = 100;
  wire [63:0] sum = longint'(a+b);
  wire [63:0] sign_extended = longint'(8'shf4);
  wire [63:0] without_xz = longint'(8'shx4);
  typedef logic [63:0] word_t;
  logic [63:0] four_state;
  wire [63:0] copy = word_t'(four_state);
  initial begin
    #4 four_state = 64'h0123_x45z_6789_abcd;
    #1 $display("integral %0d %h %0d %h", sum, sign_extended, without_xz, copy);
    $finish;
  end
endmodule
"#;
    check(
        "real_cast_guards",
        src,
        &["integral 300 fffffffffffffff4 4 0123x45z6789abcd"],
    );
}

/// §12.7.1: a real loop variable retains its declared type. The cast in a
/// comparison and the cast inside a compiled class method round before use.
#[test]
fn real_to_integral_cast_in_loop_and_method() {
    let src = r#"
class Converter;
  function longint twice(bit positive);
    return longint'(positive ? 2.5 : -2.5) * 2;
  endfunction
endclass
module top;
  real r;
  logic [63:0] rounded;
  bit matched;
  always @(r) begin
    rounded = 0;
    for (real x = r; x <= r; x += 1.0) rounded = longint'(x);
  end
  always @(r) matched = longint'(r) == 3;
  initial begin
    Converter c;
    c = new;
    #5 r = 2.5;
    #1 $display("positive %0d %b %0d", $signed(rounded), matched, c.twice(1));
    #5 r = -2.5;
    #1 $display("negative %0d %b %0d", $signed(rounded), matched, c.twice(0));
    $finish;
  end
endmodule
"#;
    check(
        "real_cast_loop_and_method",
        src,
        &["positive 3 1 6", "negative -3 0 -6"],
    );
}

/// §11.8.1: a part-select is unsigned even when its source is signed.
/// The surrounding casts exercise register reuse in a fused native block.
#[test]
fn real_to_integral_cast_preserves_unsigned_select() {
    let src = r#"
module top;
  typedef logic [63:0] lg_t;
  typedef logic signed [63:0] lgs_t;
  logic signed [63:0] sv = -64'sd3;
  wire [63:0] w1 = lg_t'(sv);
  wire [63:0] w2 = lgs_t'(sv);
  wire [63:0] w13 = lg_t'(sv[63:1]);
  wire [63:0] w15 = lg_t'(sv ? sv : 64'sd0);
  initial begin #1 $display("T|%h", w13); $finish; end
endmodule
"#;
    check("real_cast_signed_select", src, &["T|7ffffffffffffffe"]);
}

/// §6.12, §6.16.1 and §7.4: nested array and bit selects, register-backed
/// array elements and string lengths have integral types in a clocked block.
#[test]
fn real_to_integral_cast_native_select_values() {
    let src = r#"
module top;
  typedef logic [63:0] lg_t;
  typedef logic signed [63:0] lgs_t;
  logic [63:0] v = 64'h1234_5678_9abc_def0;
  logic signed [7:0] s = -8'sd5;
  wire [63:0] w1 = lg_t'(v[47:16]);
  wire [63:0] w2 = lgs_t'(s[7:0]);
  wire [63:0] w3 = lg_t'(v);
  wire [63:0] w4 = lgs_t'(s);
  initial begin #1 $display("T|%h %h %h %h", w1, w2, w3, w4); v = ~v; #1 $display("T|%h %h %h %h", w1, w2, w3, w4); $finish; end
endmodule
"#;
    check(
        "real_cast_native_select",
        src,
        &[
            "T|0000000056789abc 00000000000000fb 123456789abcdef0 fffffffffffffffb",
            "T|00000000a9876543 00000000000000fb edcba9876543210f fffffffffffffffb",
        ],
    );
}
