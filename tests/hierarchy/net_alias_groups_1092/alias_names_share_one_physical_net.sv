// IEEE 1800-2023 §10.11: "The effects are cumulative"; §6.8 initializes logic to x; §6.6.3 resolves wor to 1 if any driver is 1.
// The upstream issue testcase, with every LRM value asserted.
`timescale 1ns/1ps
module shorter(inout wire p, inout wire q); alias p = q; endmodule
module top;
  // 1: a continuous identity copy into one alias name
  logic v;
  wire r, s;
  alias r = s;
  assign r = v;
  // 2: drivers on different names of one wired alias
  wor a, b;
  alias a = b;
  assign a = 1'b0;
  assign b = 1'b1;
  // 3: alias statements that name one net in another order
  wire c, d, e;
  alias c = d;
  alias e = d;
  assign c = 1'b1;
  // 4: alias statements in a generate block and in a child module
  wire g1, g2;
  if (1) begin : g alias g1 = g2; end
  assign g1 = 1'b1;
  wire c1, c2;
  shorter u(c1, c2);
  assign c1 = 1'b1;
  initial begin
    #1 if (r === 1'bx && s === 1'bx) $display("PASS copy r,s=%b%b", r, s);
       else $display("FAIL copy r,s=%b%b (expected xx)", r, s);
       if (a === 1'b1 && b === 1'b1) $display("PASS wor a,b=%b%b", a, b);
       else $display("FAIL wor a,b=%b%b (expected 11)", a, b);
       if (c === 1'b1 && d === 1'b1 && e === 1'b1) $display("PASS cumulative c,d,e=%b%b%b", c, d, e);
       else $display("FAIL cumulative c,d,e=%b%b%b (expected 111)", c, d, e);
       if (g2 === 1'b1) $display("PASS generate g2=%b", g2);
       else $display("FAIL generate g2=%b (expected 1)", g2);
       if (c2 === 1'b1) $display("PASS child c2=%b", c2);
       else $display("FAIL child c2=%b (expected 1)", c2);
    $finish;
  end
endmodule
