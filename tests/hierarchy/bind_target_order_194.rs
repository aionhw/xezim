//! IEEE 1800-2023 §23.11 (xezim-core#194): a bound instance is elaborated as
//! if it were written at the end of its target's items, also when it is
//! bound into a scope another bind created. Its processes run after the
//! target's own and before the target's later siblings, and the target reads
//! the bound scope by name. Expected lines are the reference simulator's.

use xezim::simulate;

const SRC: &str = r#"
module watch #(parameter int W = 4) (input logic [W-1:0] a, input logic clk);
  logic [W-1:0] seen;
  wire flag = 1'b1;
  always @(posedge clk) seen <= a;
  initial $display("T|watch %m W=%0d", W);
endmodule
module deep (input logic clk);
  int ticks;
  always @(posedge clk) ticks++;
  initial $display("T|deep %m");
endmodule
module element #(parameter int P = 3) (input logic clk, input logic [7:0] din);
  initial $display("T|element %m P=%0d", P);
  initial #21 $display("T|sees %m flag=%b seen=%h", w.flag, w.seen);
endmodule
module other;
  initial $display("T|other %m");
endmodule
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  logic [7:0] d0 = 8'h5a, d1 = 8'ha5;
  element #(.P(4)) u0 (.clk(clk), .din(d0));
  other o0 ();
  element #(.P(6)) u1 (.clk(clk), .din(d1));
  for (genvar i = 0; i < 2; i++) begin : g
    element #(.P(2 + i)) e (.clk(clk), .din(8'(i * 3)));
  end
  initial begin
    #22;
    $display("T|seen %h %h %h", u0.w.seen, u1.w.seen, g[1].e.w.seen);
    $display("T|ticks %0d %0d", u0.w.dd.ticks, g[0].e.w.dd.ticks);
    $finish;
  end
endmodule
bind element watch #(.W(P)) w (.a(din[P-1:0]), .clk(clk));
bind watch deep dd (.clk(clk));
"#;

const EXPECTED: [&str; 19] = [
    "T|element top.u0 P=4",
    "T|watch top.u0.w W=4",
    "T|deep top.u0.w.dd",
    "T|other top.o0",
    "T|element top.u1 P=6",
    "T|watch top.u1.w W=6",
    "T|deep top.u1.w.dd",
    "T|element top.g[0].e P=2",
    "T|watch top.g[0].e.w W=2",
    "T|deep top.g[0].e.w.dd",
    "T|element top.g[1].e P=3",
    "T|watch top.g[1].e.w W=3",
    "T|deep top.g[1].e.w.dd",
    "T|sees top.u0 flag=1 seen=a",
    "T|sees top.u1 flag=1 seen=25",
    "T|sees top.g[0].e flag=1 seen=0",
    "T|sees top.g[1].e flag=1 seen=3",
    "T|seen a 25 3",
    "T|ticks 2 2",
];

#[test]
fn nested_bound_instances_keep_their_place_after_the_target() {
    let sim = simulate(SRC, 100).expect("simulate");
    let lines: Vec<&str> = sim
        .output
        .iter()
        .map(|o| o.message.as_str())
        .filter(|l| l.starts_with("T|"))
        .collect();
    assert_eq!(lines, EXPECTED);
}
