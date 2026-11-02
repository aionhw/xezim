//! §11.6.1 Table 11-21: each operand of `&&`, `||`, `->` and `<->` is
//! self-determined on its own. In compiled continuous assignments,
//! `always_comb` and `always_ff`, both operands were sized to the wider one,
//! counting a call as wide as its widest argument, so `~f(x)` of a 1-bit `f`
//! returning 1 was inverted at 8 bits and read true (#810). Expected values
//! are the reference simulator's.

use xezim::simulate;

const SRC: &str = r#"
module t;
  function automatic logic f(input logic [7:0] a);
    return a == 8'h40;
  endfunction
  function automatic logic [3:0] g(input logic [7:0] a);
    return a[3:0];
  endfunction
  logic [7:0] x = 8'h40;
  logic one = 1'b1, zero = 1'b0, fx;
  logic [7:0] wide = 8'h01;
  assign fx = f(x);
  wire or_call  = ~f(x) || zero;
  wire and_call = ~f(x) && one;
  wire or_wide  = ~f(x) || (wide == 8'h00);
  wire imp_call = ~f(x) -> zero;
  wire g_call   = ~g(x) && one;
  logic comb_or, ff_and;
  always_comb comb_or = ~f(x) || zero;
  logic clk = 0;
  always #1 clk = ~clk;
  always_ff @(posedge clk) ff_and <= ~f(x) && one;
  initial begin
    #3;
    $display("L|%b %b %b %b %b %b %b", or_call, and_call, or_wide, imp_call, g_call, comb_or, ff_and);
    $finish;
  end
endmodule
"#;

#[test]
fn logical_operands_are_self_determined_in_compiled_logic() {
    let lines: Vec<String> = simulate(SRC, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|l| l.message.clone())
        .filter(|m| m.starts_with("L|"))
        .collect();
    assert_eq!(lines, ["L|0 0 0 1 1 0 0"]);
}
