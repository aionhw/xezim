//! #987: §13.4.1, §11.6.1: `return e` is evaluated as if assigned to the
//! function's return variable, so the operand of `~` (and of unary `-`,
//! `+`, `<<`) is widened to the return width before the operator applies:
//! `return ~a` with a 4-bit `a` in a `logic [7:0]` function gives
//! 8'b11111101 for a = 4'b0010, the same as `result = ~a`, and not the
//! zero-extended 4-bit result 8'b00001101. The calls run from `$display`,
//! an `always` block and a continuous assignment. Expected lines are the
//! reference simulator's.

use xezim::simulate;

#[test]
fn return_operand_takes_the_return_width() {
    let src = r#"
module test;
  reg [3:0] a;
  reg [7:0] r1, r2, r3, r4, r5, r6, r7, r8;
  reg clk = 0;
  function logic [7:0] f_not(input logic [3:0] a);   return ~a;        endfunction
  function logic [7:0] f_neg(input logic [3:0] a);   return -a;        endfunction
  function logic [7:0] f_add(input logic [3:0] a);   return a + 4'hF;  endfunction
  function logic [7:0] f_shl(input logic [3:0] a);   return a << 4;    endfunction
  function logic [7:0] f_nam(input logic [3:0] a);   f_nam = ~a;       endfunction
  function automatic logic [7:0] f_aut(input logic [3:0] a); return ~a; endfunction
  function logic [7:0] f_cond(input logic [3:0] a, input bit s); return s ? ~a : a; endfunction
  function logic [15:0] f_w(input logic [3:0] a);    return {~a};      endfunction
  function bit signed [7:0] f_sx(input bit signed [3:0] a); return ~a; endfunction
  function int f_int(input logic [3:0] a);           return ~a;        endfunction
  wire [7:0] w_not = f_not(a);
  always @(posedge clk) begin
    r1 = f_not(a); r2 = f_neg(a); r3 = f_add(a); r4 = f_shl(a);
    r5 = f_nam(a); r6 = f_aut(a); r7 = f_cond(a, 1); r8 = f_sx(a);
  end
  initial begin
    a = 4'b0010;
    #1 clk = 1;
    #1;
    $display("disp not=%b neg=%b add=%b shl=%b nam=%b aut=%b cond=%b w=%h sx=%b int=%h",
             f_not(a), f_neg(a), f_add(a), f_shl(a), f_nam(a), f_aut(a), f_cond(a, 1), f_w(a),
             f_sx(a), f_int(a));
    $display("proc not=%b neg=%b add=%b shl=%b nam=%b aut=%b cond=%b sx=%b",
             r1, r2, r3, r4, r5, r6, r7, r8);
    $display("cont not=%b", w_not);
    $display("expr %b %b", f_not(a) >> 4, {f_not(a)});
  end
endmodule
"#;
    let sim = simulate(src, 1000).expect("simulate failed");
    let out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    assert_eq!(
        out,
        [
            "disp not=11111101 neg=11111110 add=00010001 shl=00100000 nam=11111101 \
             aut=11111101 cond=11111101 w=000d sx=11111101 int=fffffffd",
            "proc not=11111101 neg=11111110 add=00010001 shl=00100000 nam=11111101 \
             aut=11111101 cond=11111101 sx=11111101",
            "cont not=11111101",
            "expr 00001111 11111101",
        ]
    );
}
