// IEEE 1800-2023 §6.12: "The shortreal data type is the same as a C float"; §20.5 uses 32-bit IEEE representation, whose spacing here is 2, so 16777216.25 rounds to 16777216.0.
// Both direct and typedef shortreal members round to binary32; only main-correct hierarchical fields are asserted.
`timescale 1ns/1ps
typedef shortreal F;
typedef struct {shortreal direct; F alias_value;} S;
module top;
 S m;
 assign m='{16777216.25,16777216.25};
 if(1)begin:g
   S a;
   assign a='{16777216.25,16777216.25};

 end
 initial begin #3;

   $display("T|outside %0.2f %0.2f delta %0.2f %0.2f",g.a.direct,g.a.alias_value,g.a.direct-16777216.0,g.a.alias_value-16777216.0);
   $finish; end
endmodule
