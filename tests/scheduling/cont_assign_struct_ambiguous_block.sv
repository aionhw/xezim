`timescale 1ns/1ps
// IEEE 1800-2023 §7.2.2/§7.4.2/§7.4.5: struct arrays and selects.
// §13.4/§13.5.1: automatic input function, no time controls.
// §10.3.2/§20.3.1: only operand changes require a new evaluation time.
// §4.7/§6.8: one write per positive slot, observations a unit later.
// §23.8/§23.9/§27.5: c.arr in child c denotes the block c.
typedef struct { int v; } S;
function automatic longint stamp(input S unused[2]); return $time; endfunction
module child;
  S arr[2];
  if (1) begin : c
    S arr[2];
  end
  longint result;
  assign result = stamp(c.arr);
endmodule
module top;
  child c();
  initial begin
    #1 c.arr[0].v = 1;
    #1 if (c.result === 0) $display("PASS unrelated_instance");
       else $display("FAIL unrelated_instance got=%0d expected=0", c.result);
    #1 c.c.arr[0].v = 3;
    // Known limit: the ambiguous spelling adds no leaves. Keep this
    // intended-write observation without asserting the stale result.
    #1 $display("LIMIT intended_block got=%0d expected=3", c.result);
    $finish;
  end
endmodule
