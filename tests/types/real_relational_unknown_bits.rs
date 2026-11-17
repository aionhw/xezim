//! IEEE 1800-2023 §11.4.4, §6.12.1: convert integral X/Z bits to zero
//! whenever either relational operand is real; retain integral X propagation.
use xezim::simulate;

#[test]
fn real_relations_convert_unknown_bits() {
    let src = r#"
`define REL(a,b) {((a)<(b)),((a)<=(b)),((a)>(b)),((a)>=(b)),((b)<(a)),((b)<=(a)),((b)>(a)),((b)>=(a))}
module top;
  timeunit 1ns; timeprecision 1ps;
  parameter logic [3:0] P = 4'b1xz1;
  localparam logic [7:0] C = `REL(P, 10.0);
  logic [3:0] v;
  real r;
  shortreal sr;
  realtime rt;
  logic [7:0] continuous_result, comb_result;
  assign continuous_result = `REL(v, r);
  always_comb comb_result = `REL(v, r);
  function automatic logic [7:0] compare(input logic [3:0] a, input real b);
    return `REL(a, b);
  endfunction
  initial begin
    v = 4'b1xz1; r = 10.0; sr = 10.0; rt = 10.0;
    #1;
    $display("T|constant=%b", C);
    $display("T|continuous=%b", continuous_result);
    $display("T|comb=%b", comb_result);
    $display("T|function=%b", compare(v, r));
    $display("T|shortreal=%b", `REL(v, sr));
    $display("T|realtime=%b", `REL(v, rt));
    $display("T|ternary=%0d", (v < r) ? 7 : 3);
    if (v < r) $display("T|if=1"); else $display("T|if=0");
    $display("T|integral=%b", `REL(v, 4'd10));
    v = 4'bxzxx; r = 0.0;
    #1;
    $display("T|zero=%b", continuous_result);
    v = 4'd9; r = 10.0;
    #1;
    $display("T|known=%b", comb_result);
  end
endmodule
`undef REL
"#;
    let sim = simulate(src, 100).expect("simulate mixed-real relations");
    let lines: Vec<_> = sim
        .output
        .iter()
        .map(|line| line.message.trim())
        .filter(|line| line.starts_with("T|"))
        .collect();
    assert_eq!(
        lines,
        [
            "T|constant=11000011",
            "T|continuous=11000011",
            "T|comb=11000011",
            "T|function=11000011",
            "T|shortreal=11000011",
            "T|realtime=11000011",
            "T|ternary=7",
            "T|if=1",
            "T|integral=xxxxxxxx",
            "T|zero=01010101",
            "T|known=11000011",
        ]
    );
}
