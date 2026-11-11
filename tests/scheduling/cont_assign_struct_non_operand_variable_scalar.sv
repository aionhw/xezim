`timescale 1ns/1ps
// IEEE 1800-2023 §7.2.2/§7.4.2/§7.4.5: struct arrays and selects.
// §13.4/§13.5.1: automatic input function, no time controls.
// §10.3.2/§20.3.1: only operand changes require a new evaluation time.
// §4.7/§6.8: one write per positive slot, observations a unit later.
typedef struct { int v; int tag; } S;
module top;
  S arr[2];
  int idx = 0;
  function automatic longint stamp(input int unused); return $time; endfunction
  longint result;
  assign result = stamp(arr[idx].v);
  initial begin
    #1 arr[0].tag = 1;
    #1 if (result === 0) $display("PASS sibling_quiet");
       else $display("FAIL sibling_quiet got=%0d expected=0", result);
    #1 arr[0].v = 3;
    #1 if (result === 3) $display("PASS selected_scalar");
       else $display("FAIL selected_scalar got=%0d expected=3", result);
    #1 idx = 1;
    #1 if (result === 5) $display("PASS index");
       else $display("FAIL index got=%0d expected=5", result);
    #1 arr[1].tag = 7;
    #1 if (result === 5) $display("PASS other_sibling_quiet");
       else $display("FAIL other_sibling_quiet got=%0d expected=5", result);
    $finish;
  end
endmodule
