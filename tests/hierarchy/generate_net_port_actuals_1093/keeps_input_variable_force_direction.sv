// IEEE 1800-2023 §23.3.3.2: variable input connection is an implied continuous assignment; §10.6.2 force overrides only that variable, leaving source 1010. The report names plain module-scope actuals as main-wrong; those are omitted. §6.5: "a continuous assignment is implied" at variable port connections.
// Forcing an explicit variable input must not propagate backwards to its generated wire source; release value of the formal is omitted.
`timescale 1ns/1ps
module leaf(input var logic [3:0] p);endmodule
module top;
 for(genvar k=0;k<2;k++)begin:g wire [3:0] w=4'b1010;end
 leaf ordered(g[1].w);leaf named(.p(g[0].w));
 initial begin #1;force ordered.p=4'b0101;force named.p=4'b0101;#1;
 $display("T|forced %b %b %b %b",g[1].w,ordered.p,g[0].w,named.p);
 release ordered.p;release named.p;#1;
 $display("T|source %b %b",g[1].w,g[0].w);$finish;end
endmodule
