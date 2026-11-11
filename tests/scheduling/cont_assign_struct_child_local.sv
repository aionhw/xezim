`timescale 1ns/1ps
// IEEE 1800-2023 §7.2.2/§7.4.2/§7.4.5: struct arrays and selects.
// §13.4/§13.5.1: automatic input function, no time controls.
// §10.3.2/§20.3.1: only operand changes require a new evaluation time.
// §4.7/§6.8: one write per positive slot, observations a unit later.
// §23.3.1/§23.6: absolute paths refer to the same declaration from a child.
typedef struct { int v; } S;
function automatic longint stamp(input S unused[2]); return $time; endfunction
module leaf;
  S arr[2];
  longint result;
  assign result = stamp(arr);
endmodule
module top;
  S arr[2];
  leaf u();
  initial begin
    #1 arr[0].v = 1;
    #1 if (u.result === 0) $display("PASS top_quiet");
       else $display("FAIL top_quiet got=%0d expected=0", u.result);
    #1 u.arr[1].v = 3;
    #1 if (u.result === 3) $display("PASS child_member");
       else $display("FAIL child_member got=%0d expected=3", u.result);
    #1 arr[1].v = 5;
    #1 if (u.result === 3) $display("PASS top_still_quiet");
       else $display("FAIL top_still_quiet got=%0d expected=3", u.result);
    $finish;
  end
endmodule
