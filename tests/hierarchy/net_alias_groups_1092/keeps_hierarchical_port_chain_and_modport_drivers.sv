// IEEE 1800-2023 §§10.11, 6.6.3 require 1; §23.3.3: inout assignment is a "non-strength-reducing transistor connection"; §§23.6,25.5 permit these paths.
// Hierarchical port-target fan-out and modport drivers must not corrupt a name main already resolves correctly.
`timescale 1ns/1ps
module holder(inout wor p);endmodule
module middle(inout wor p);holder h(p);endmodule
interface busif(inout wire p);modport m(inout p);endinterface
module drv(busif.m b,input wire d);assign b.p=d;endmodule
module top;
  wire low=0,high=1;
  wor a,b;alias a=b;assign a=low;middle u(b);assign u.h.p=high;
  wor c,d;alias c=d;busif q(c);drv v(q,low);assign d=high;
  initial begin #1;$display("T|protected %b %b %b",b,u.h.p,d);$finish;end
endmodule
