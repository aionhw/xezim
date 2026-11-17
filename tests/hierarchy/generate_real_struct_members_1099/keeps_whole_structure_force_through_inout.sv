// IEEE 1800-2023 §10.6.2: a force "shall override all drivers of the net"; §23.3.3 inout is a "non-strength-reducing transistor connection"; §7.2 member values are the forced pattern bytes.
// A whole-net force reaches member reads, including when written on a differently named inout formal; omit main-wrong outside/release fields.
`timescale 1ns/1ps
typedef struct {logic [7:0] i;} S;
module child(inout wire S p);initial #1 force p='{8'hcd};endmodule
module top;
 if(1)begin:g
 wire S direct,connected;assign direct='{8'h56};assign connected='{8'h56};
 child c(connected);
 initial begin #1;force direct='{8'hab};#2;
 $display("T|force %h %h",direct.i,connected.i);$finish;end
 end
endmodule
