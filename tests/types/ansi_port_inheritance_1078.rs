//! §23.2.2.3: a later port of an ANSI list that omits its direction, port
//! kind and data type takes all three from the previous port. One that gives
//! any of them takes only an omitted direction; an omitted port kind follows
//! the first-port rules and an omitted data type is `logic`. So in
//! `input logic [7:0] d, input clk, rst_n` the port `rst_n` is a scalar like
//! `clk`; it took the type of `d`, the last port that gave one
//! (aionhw/xezim#1078, reported by ds54e). A list whose first port gives only
//! a port kind or packed dimensions (`(wire x)`, `([5:0] x, wire y)`) is an
//! ANSI list too; it failed to parse. Expected values are the reference
//! simulator's.

use xezim::simulate;

fn lines(src: &str) -> Vec<String> {
    let sim = simulate(src, 100).expect("simulate failed");
    let mut out: Vec<String> = sim.output.iter().map(|o| o.message.clone()).collect();
    out.sort();
    out
}

#[test]
fn reporter_testcase() {
    let src = "module leaf(input logic [7:0] d, input clk, rst_n);
  initial #1 begin
    if ($bits(clk) == 1) $display(\"PASS $bits(clk) = %0d\", $bits(clk));
    else $display(\"FAIL $bits(clk) = %0d (expected 1)\", $bits(clk));
    if ($bits(rst_n) == 1) $display(\"PASS $bits(rst_n) = %0d\", $bits(rst_n));
    else $display(\"FAIL $bits(rst_n) = %0d (expected 1)\", $bits(rst_n));
  end
endmodule
module top;
  logic [7:0] d = 8'h5a;
  logic clk = 1'b0, rst_n = 1'b1;
  leaf u(.d(d), .clk(clk), .rst_n(rst_n));
endmodule
";
    assert_eq!(lines(src), ["PASS $bits(clk) = 1", "PASS $bits(rst_n) = 1"]);
}

/// The LRM's examples `mh16`-`mh20` and the kinds, `var` and interface ports
/// carried across a comma.
#[test]
fn lrm_examples_and_carried_attributes() {
    let src = "module mh16([5:0] x, wire y);
  initial #1 $display(\"mh16 x=%0d y=%0d\", $bits(x), $bits(y));
endmodule
module mh3(signed [3:0] s, u);
  initial #1 $display(\"mh3 s=%0d u=%0d\", $bits(s), $bits(u));
endmodule
module x6(inout [5:0] x, wire y);
  initial #1 $display(\"x6 x=%0d y=%0d\", $bits(x), $bits(y));
endmodule
module mh19(output signed [5:0] x, integer y);
  initial begin y = -3; #1 $display(\"mh19 x=%0d y=%0d y=%0d\", $bits(x), $bits(y), y); end
endmodule
module mh20(ref [5:0] x, y);
  initial #1 $display(\"mh20 x=%0d y=%0d\", $bits(x), $bits(y));
endmodule
module ow(output wand [3:0] a, logic [3:0] b);
  initial begin b = 4'h9; #1 $display(\"ow a=%0d b=%0d b=%h\", $bits(a), $bits(b), b); end
endmodule
module tv(output var logic [3:0] a, b, input wire [2:0] c, e);
  initial #1 $display(\"tv a=%0d b=%0d c=%0d e=%0d\", $bits(a), $bits(b), $bits(c), $bits(e));
endmodule
interface bus; logic [7:0] w; modport m(input w); endinterface
module ifp(bus.m p, q, input [3:0] k, l);
  initial #1 $display(\"ifp p.w=%0d q.w=%0d k=%0d l=%0d\", $bits(p.w), $bits(q.w), $bits(k), $bits(l));
endmodule
module top;
  wire [5:0] x; wire y;
  wire [3:0] s; wire u;
  wire [5:0] x2; wire y2;
  wire [5:0] x3; int y3;
  logic [5:0] rx, ry;
  wire [3:0] a4; logic [3:0] b4;
  logic [3:0] ta, tb; logic [2:0] tc, te;
  bus b1(); bus b2(); logic [3:0] k, l;
  mh16 m16(x, y);
  mh3 m3(s, u);
  x6 u6(x2, y2);
  mh19 m19(x3, y3);
  mh20 m20(rx, ry);
  ow uow(a4, b4);
  tv utv(ta, tb, tc, te);
  ifp uifp(b1, b2, k, l);
endmodule
";
    assert_eq!(
        lines(src),
        [
            "ifp p.w=8 q.w=8 k=4 l=4",
            "mh16 x=6 y=1",
            "mh19 x=6 y=32 y=-3",
            "mh20 x=6 y=6",
            "mh3 s=4 u=4",
            "ow a=4 b=4 b=9",
            "tv a=4 b=4 c=3 e=3",
            "x6 x=6 y=1",
        ]
    );
}
