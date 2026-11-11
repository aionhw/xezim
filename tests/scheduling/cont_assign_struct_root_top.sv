`timescale 1ns/1ps
// IEEE 1800-2023 §7.2.2/§7.4.2/§7.4.5: struct arrays and selects.
// §13.4/§13.5.1: automatic input function, no time controls.
// §10.3.2/§20.3.1: only operand changes require a new evaluation time.
// §4.7/§6.8: one write per positive slot, observations a unit later.
// §23.3.1/§23.6: $root.top is the top scope, not the instance top.
typedef struct { int v; } S;
function automatic longint stamp(input S unused[2]); return $time; endfunction
module mid_top;
  S arr[2];
endmodule
module top;
  S arr[2];
  mid_top top();
  longint result;
  assign result = stamp($root.top.arr);
  initial begin
    #1 top.arr[0].v = 1;
    #1 if (result === 0) $display("PASS instance_quiet");
       else $display("FAIL instance_quiet got=%0d expected=0", result);
    #1 arr[1].v = 3;
    #1 if (result === 3) $display("PASS root_member");
       else $display("FAIL root_member got=%0d expected=3", result);
    #1 top.arr[1].v = 5;
    #1 if (result === 3) $display("PASS instance_still_quiet");
       else $display("FAIL instance_still_quiet got=%0d expected=3", result);
    $finish;
  end
endmodule
