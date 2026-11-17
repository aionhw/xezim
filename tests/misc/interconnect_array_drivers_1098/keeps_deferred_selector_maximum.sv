// IEEE 1800-2023 §20.9 $isunknown of x is 1; §6.6.7 resolver maximum of drivers 6 and 2 is 6; §23.3.3.7.1 merges the ports of element 1. §23.3.3.7.1: "Any port connection with an interconnect net shall merge" the nets.
// A constant system-call selector alongside a plain index must preserve the main-correct maximum in every port.
`timescale 1ns/1ps
// IEEE 1800-2023 §6.6.7: max(2,6)=6, independently of driver order.
// §20.9: $isunknown(1'bx)=1; §4.9.1: sample after initial settling.
package p;
 function automatic real maximum(input real d[]);
  real z; z=0.0; foreach(d[i]) if(d[i]>z) z=d[i]; return z;
 endfunction
 nettype real N with maximum;
endpackage
import p::*;
module load #(parameter real V=6)(inout N n);assign n=V;endmodule
module reader(input N n);endmodule
module tb;
 interconnect a[2];
 load l0(a[$isunknown(1'bx)]);load #(2) l1(a[1]);reader r(a[1]);
 initial begin #1;$display("T|max %g %g %g",l0.n,l1.n,r.n);$finish;end
endmodule
