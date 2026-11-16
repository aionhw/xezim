// IEEE 1800-2023 §23.3.3.7.1: "Any port connection with an interconnect net shall merge" the nets; §§6.6.7, 6.6.8 give the element nettype N and (2+0)/2=1.
// The upstream issue testcase, with every LRM value asserted.
`timescale 1ns/1ps
package p;
 typedef struct {real V;} E;
 function automatic E avg(input E d[]);E z;z.V=0;foreach(d[k])z.V+=d[k].V;z.V/=d.size();return z;endfunction
 nettype E N with avg;
endpackage
import p::*;
module load(inout N n);assign n='{0.0};endmodule
module source(output N n);assign n='{2.0};endmodule
module probe(input N n,output real v);assign v=n.V;endmodule
module tb;
 interconnect a[1];real v;
 source s(a[0]);load l(a[0]);probe p0(a[0],v);
 initial begin #1;
 if(v==1.0)$display("PASS expected=1 actual=%f",v);
 else $display("FAIL expected=1 actual=%f",v);
 $finish;end
endmodule
