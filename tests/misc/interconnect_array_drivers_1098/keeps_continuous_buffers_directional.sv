// IEEE 1800-2023 §10.3.2 evaluates RHS and drives only LHS; §23.3.3.7.1 merges only "port connection" nets; §6.6.7 resolves each upstream network from its sole driver 2.
// A directional continuous identity assignment is not a physical port connection and cannot feed a downstream load back upstream.
`timescale 1ns/1ps

package p;
 function automatic real avg(input real d[]);real z;z=0;foreach(d[k])z+=d[k];return z/d.size();endfunction
 nettype real N with avg;
endpackage
import p::*;
module source #(parameter real V=2)(output N n);assign n=V;endmodule
module reader(input N n);endmodule

module buffer(input N i,output N o);assign o=i;endmodule
module expression_buffer(input N i,output N o);assign o=i+0.0;endmodule
module load(inout N n);assign n=6.0;endmodule
module top;
 interconnect a[2],b[2];source s(a[0]);reader r(a[0]);buffer u(a[0],a[1]);load l(a[1]);
 source t(b[0]);reader q(b[0]);expression_buffer v(b[0],b[1]);load k(b[1]);
 initial begin #1;$display("T|upstream %g %g %g %g %g %g",s.n,r.n,u.i,t.n,q.n,v.i);$finish;end
endmodule
