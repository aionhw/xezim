// Count per IEEE 1800-2023 §11.6/§11.8.2: 18. §10.9.2: two members; mismatches intentionally illegal.
`timescale 1ns/1ps
module top;
typedef struct {int a,b;} T;
typedef struct packed {byte a,b;} P;
T s,c,arg;
class H; T v; endclass
H h;
function automatic T identity(input T v);return v;endfunction
localparam P K='{(1 ? (3'sd3*3'sd3*3'sd2) : 6'sd0){8'd7}};
assign c='{(1 ? (3'sd3*3'sd3*3'sd2) : 6'sd0){7}};
assign arg=identity('{(1 ? (3'sd3*3'sd3*3'sd2) : 6'sd0){7}});
initial begin h=new; s='{1,2}; h.v='{1,2};
#1 s='{(1 ? (3'sd3*3'sd3*3'sd2) : 6'sd0){7}}; h.v='{(1 ? (3'sd3*3'sd3*3'sd2) : 6'sd0){7}};
#1 $display("OBSERVE procedural=%0d,%0d continuous=%0d,%0d argument=%0d,%0d constant=%h class=%0d,%0d",s.a,s.b,c.a,c.b,arg.a,arg.b,K,h.v.a,h.v.b); $finish; end
endmodule
