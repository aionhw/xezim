// IEEE 1800-2023 §23.3.1: "giving priority to the local scope"; §23.8 hierarchical search is independent of declaration order; §23.3.3.1 input connection copies local 0.
// A local child net or variable takes precedence even when declared after its consumer.
`timescale 1ns/1ps
module netchild;for(genvar i=0;i<2;i++)begin:g wire node=0;end endmodule
module varchild;for(genvar i=0;i<2;i++)begin:g logic node=0;end endmodule
module load(input wire n);endmodule
module tb;
 for(genvar i=0;i<2;i++)begin:g wire node=1;end
 load l(tb.g[1].node);netchild tb();
 load v(vars.g[1].node);varchild vars();
 initial begin #1;$display("T|shadow %b %b %b %b %b",g[1].node,l.n,tb.g[1].node,v.n,vars.g[1].node);$finish;end
endmodule
