// IEEE 1800-2023 §23.3.3.7.1: interconnect port connections "shall merge"; §§6.6.7,6.6.8 resolve drivers 2 and 0 to average 1.
// The already-working scalar path must keep all ports on one resolved net.
`timescale 1ns/1ps

package p;
 function automatic real avg(input real d[]);real z;z=0;foreach(d[k])z+=d[k];return z/d.size();endfunction
 nettype real N with avg;
endpackage
import p::*;
module source #(parameter real V=2)(output N n);assign n=V;endmodule
module reader(input N n);endmodule

module load(inout N n);assign n=0.0;endmodule
module top;
 interconnect a;source s(a);load l(a);reader r(a);
 initial begin #1;$display("T|scalar %g %g %g",s.n,l.n,r.n);$finish;end
endmodule
