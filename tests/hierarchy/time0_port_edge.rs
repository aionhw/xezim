//! IEEE 1800-2023 §23.3.3, §9.4.2: an input port is a continuous assignment
//! from its actual, so an actual holding its declaration initializer
//! reaches the port as a time-0 update: from x for a 4-state formal, from 0
//! for a 2-state one. In the instance, an edge-sensitive always block, or a
//! waiter of an always procedure, that watches the port sees that edge
//! (`logic clk = 0` is a negedge there). The declaring scope's own blocks on
//! the variable do not (§6.8). The port's reads were renamed to the actual,
//! so these blocks fired only from the first later change. Neither an
//! initial procedure's waiter nor an `iff` term takes the update. Expected
//! values come from the reference simulator.

use xezim::simulate;

fn t_lines(src: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

const SHAPES: &str = r#"
module leaf (input logic clk);
  integer nn = 0, np = 0, ne = 0, nl = 0, ni = 0, nw = 0, nf = 0, ng = 0;
  logic en = 1;
  always @(negedge clk) nn++;
  always @(posedge clk) np++;
  always @(edge clk) ne++;
  always @(clk) nl++;
  always @(negedge clk iff en) nf++;
  always begin @(negedge clk); nw++; end
  initial begin @(negedge clk); ni = 1; end
  initial begin @(clk); ng = 1; end
endmodule
module leafb (input bit clk);
  integer nn = 0, np = 0;
  always @(negedge clk) nn++;
  always @(posedge clk) np++;
endmodule
module mid (input logic clk);
  leaf s (.clk(clk));
endmodule
module tb;
  logic c0 = 0;
  logic c1 = 1;
  bit b0 = 0;
  bit b1 = 1;
  leaf a (.clk(c0));
  leaf b (.clk(c1));
  leaf c (.clk(b0));
  leaf d (.clk(b1));
  leafb e (.clk(c0));
  leafb f (.clk(c1));
  mid m (.clk(c0));
  initial begin
    #1;
    $display("T|a nn=%0d np=%0d ne=%0d nl=%0d nf=%0d nw=%0d ni=%0d ng=%0d", a.nn, a.np, a.ne, a.nl, a.nf, a.nw, a.ni, a.ng);
    $display("T|b nn=%0d np=%0d ne=%0d nl=%0d nf=%0d nw=%0d ni=%0d ng=%0d", b.nn, b.np, b.ne, b.nl, b.nf, b.nw, b.ni, b.ng);
    $display("T|c nn=%0d np=%0d ne=%0d nl=%0d", c.nn, c.np, c.ne, c.nl);
    $display("T|d nn=%0d np=%0d ne=%0d nl=%0d", d.nn, d.np, d.ne, d.nl);
    $display("T|e nn=%0d np=%0d f nn=%0d np=%0d", e.nn, e.np, f.nn, f.np);
    $display("T|m.s nn=%0d np=%0d ni=%0d", m.s.nn, m.s.np, m.s.ni);
    $finish;
  end
endmodule

"#;

const RESET: &str = r#"
module ff (input logic clk, input logic rst_n, input logic [3:0] d, output logic [3:0] q);
  integer nr = 0, nc = 0;
  always_ff @(posedge clk or negedge rst_n)
    if (!rst_n) begin q <= 4'h0; nr++; end
    else begin q <= d; nc++; end
endmodule
module na (clk);
  input clk;
  integer nn = 0;
  always @(negedge clk) nn++;
endmodule
module cnt (input logic clk);
  integer np = 0;
  always @(posedge clk) np++;
endmodule
module tb;
  logic clk = 1;
  logic rst_n = 0;
  logic [3:0] d = 4'h5;
  logic [3:0] q;
  logic c0 = 0;
  logic dclk;
  initial begin #2 dclk = 0; forever #5 dclk = ~dclk; end
  ff u (.clk(clk), .rst_n(rst_n), .d(d), .q(q));
  na n (.clk(c0));
  for (genvar i = 0; i < 2; i++) begin : g
    na u (.clk(c0));
  end
  na arr [1:0] (.clk(c0));
  cnt k (.clk(dclk));
  initial begin
    #0 $display("T|t0 q=%h", q);
    #1;
    $display("T|ff nr=%0d nc=%0d q=%h", u.nr, u.nc, q);
    $display("T|na n=%0d g0=%0d g1=%0d arr0=%0d arr1=%0d", n.nn, g[0].u.nn, g[1].u.nn, arr[0].nn, arr[1].nn);
    #20;
    $display("T|dclk k=%0d", k.np);
    $finish;
  end
endmodule
"#;

const CLOCKED: &str = r#"
module leaf (input logic clk);
  integer nn = 0, np = 0;
  always @(negedge clk) nn++;
  always @(posedge clk) np++;
endmodule
module tb;
  logic clk = 0;
  logic one = 1;
  always #5 clk = ~clk;
  leaf v (.clk(clk));
  leaf x (.clk(one));
  integer pn = 0, pp = 0;
  always @(negedge clk) pn++;
  always @(posedge clk) pp++;
  initial begin
    #57;
    $display("T|child nn=%0d np=%0d const np=%0d parent pn=%0d pp=%0d", v.nn, v.np, x.np, pn, pp);
    $finish;
  end
endmodule
"#;

/// Each event shape, 4-state and 2-state actuals and formals, a nested
/// instance.
#[test]
fn time0_port_update_reaches_each_event_shape() {
    assert_eq!(
        t_lines(SHAPES),
        [
            "T|a nn=1 np=0 ne=1 nl=1 nf=0 nw=1 ni=0 ng=0",
            "T|b nn=0 np=1 ne=1 nl=1 nf=0 nw=0 ni=0 ng=0",
            "T|c nn=1 np=0 ne=1 nl=1",
            "T|d nn=0 np=1 ne=1 nl=1",
            "T|e nn=0 np=0 f nn=0 np=1",
            "T|m.s nn=1 np=0 ni=0",
        ]
    );
}

/// An asynchronous reset held low by its initializer runs at time 0; a
/// non-ANSI port, generate and array instances, and a clock that starts
/// later.
#[test]
fn time0_port_update_resets_and_reaches_every_instance_kind() {
    assert_eq!(
        t_lines(RESET),
        [
            "T|t0 q=x",
            "T|ff nr=1 nc=0 q=0",
            "T|na n=1 g0=1 g1=1 arr0=1 arr1=1",
            "T|dclk k=2",
        ]
    );
}

/// A running clock: the child counts the time-0 negedge, the parent does
/// not.
#[test]
fn time0_port_update_leaves_the_declaring_scope_alone() {
    assert_eq!(
        t_lines(CLOCKED),
        ["T|child nn=6 np=6 const np=1 parent pn=5 pp=6",]
    );
}
