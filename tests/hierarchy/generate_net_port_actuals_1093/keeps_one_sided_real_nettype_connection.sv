// IEEE 1800-2023 §23.3.3: when only one connection has a user nettype, matching data types use "a continuous assignment from source to sink"; §§6.12,10.3.2 copy 3.0 without merging.
// When only the actual is a user nettype, a real variable formal retains its continuous assignment and real representation.
`timescale 1ns/1ps
package p;
 function automatic real avg(input real d[]);real z;z=0;foreach(d[k])z+=d[k];return z/d.size();endfunction
 nettype real N with avg;nettype real U;
endpackage
import p::*;
module sink(input var real r);real y;always @* y=r;endmodule
module top;
 for(genvar k=0;k<2;k++)begin:g N n;U u;assign n=3.0;assign u=3.0;end
 sink n(g[1].n);sink u(g[1].u);
 initial begin #1;$display("T|real %0.2f %0.2f %0.2f %0.2f",n.r,n.y,u.r,u.y);$finish;end
endmodule
