// IEEE 1800-2023 §23.10.1 permits hierarchical defparam override P=1; §6.6.7 makes it "an error" for an unresolved nettype to have multiple drivers; there is one driver per element: a[1] drives 2 and a[0] drives 6.
// Do not reject legal distinct unresolved elements due to an unapplied top defparam; omit main-wrong P itself.
`timescale 1ns/1ps
nettype real N;
module source #(parameter real V=2)(output N n);assign n=V;endmodule
module tb;
 parameter int P=0;defparam tb.P=1;
 interconnect a[2];source s(a[P]);source #(6) t(a[0]);
 initial begin #1;$display("T|ports %g %g",s.n,t.n);$finish;end
endmodule
