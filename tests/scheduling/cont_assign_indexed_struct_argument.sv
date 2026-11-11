`timescale 1ns/1ps
// IEEE 1800-2023 §7.2.2/§7.4.2, §13.5.1, §10.3.2:
// the selected struct is a whole actual even if ignored by its callee.
// §4.7/§6.8: drive and observe in separate time slots.
typedef struct { int v; int tag; } S;
function automatic int observe(input S arg);
  $display("eval %0t", $time);
  return 1;
endfunction
module top;
  S a[2];
  int idx, r;
  // §11.4.3: the call is nested in an arithmetic RHS expression.
  assign r = 1 + observe((a[idx]));
  initial begin
    #1 idx = 0; a[0] = '{2,3}; a[1] = '{5,7};
    #1 a[0].v = 9;
    #1 idx = 1;
    #1 a[1].tag = 11;
    #1 $display("indexed %0d", r); // 1 + 1
       $finish;
  end
endmodule
