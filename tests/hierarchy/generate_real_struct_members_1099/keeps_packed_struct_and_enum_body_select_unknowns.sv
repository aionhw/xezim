// IEEE 1800-2023 §§7.2,6.19 permit packed dimensions after bodies; §7.4.1: packed array is "represented as a contiguous set of bits"; §20.9 $isunknown detects preserved x bits in the high subarray.
// Dimensions after struct/enum bodies still select X-containing high subarrays; omit main-wrong element width/low fields.
`timescale 1ns/1ps
// §7.2, §7.4.1: packed dimensions may follow the aggregate/enum body.
typedef struct packed {logic [3:0] n;} [1:0][1:0] P;
typedef enum logic [3:0] { ZERO=0 } [1:0][1:0] E;
typedef struct { P pk; } S;
typedef struct { E ek; } T;
module top;
 if(1) begin:g
  S s; T t;
  assign s='{P'(16'hxx34)};
  assign t='{E'(16'hxx34)};
  initial begin
   #2;
   $display("T|struct-unknown %0d",$isunknown(s.pk[1]));
   $display("T|enum-unknown %0d",$isunknown(t.ek[1]));
   $finish;
  end
 end
endmodule
