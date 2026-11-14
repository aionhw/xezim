// IEEE 1800-2023 §6.6.7 resolver receives the driver values; §23.3.3.7.1 merges ports on one element; average of 32 copies of 2 is 2. §23.3.3.7.1: "Any port connection with an interconnect net shall merge" the nets.
// Many equal output drivers retain 2. This is a functional fan-in guard; the historical quadratic cost finding requires separate scaling measurements.
`timescale 1ns/1ps

package p;
 function automatic real avg(input real d[]);real z;z=0;foreach(d[k])z+=d[k];return z/d.size();endfunction
 nettype real N with avg;
endpackage
import p::*;
module source #(parameter real V=2)(output N n);assign n=V;endmodule
module reader(input N n);endmodule

module top;
 interconnect a[1];
 for(genvar k=0;k<32;k++)begin:g source s(a[0]);end
 reader r(a[0]);
 initial begin #1;$display("T|fan-in %g %g",g[31].s.n,r.n);$finish;end
endmodule
