// IEEE 1800-2023 §6.6.6 supply nets have supply strength; §28.12.1: "the stronger signal shall dominate all the weaker drivers". Supply0 remains 0000 and supply1 1111.
// Supply members keep intrinsic drivers under conflicting strong drivers and z; outside/mirror fields are omitted.
`timescale 1ns/1ps
typedef struct {logic [3:0] v;} S;
module top;
 if(1)begin:g
 supply0 S low;supply1 S high;logic [3:0] d=4'b1010;
 assign low='{d};assign high='{~d};
 initial begin #1;$display("T|supply %b %b",low.v,high.v);d='z;#1;
 $display("T|floating %b %b",low.v,high.v);$finish;end
 end
endmodule
