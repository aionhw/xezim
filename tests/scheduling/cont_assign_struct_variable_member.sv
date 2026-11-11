`timescale 1ns/1ps
// IEEE 1800-2023 §7.2.2/§7.4.2/§7.4.5: variable-index struct members.
// §13.4/§13.5.1: automatic input function with no time controls.
// §10.3.2/§20.3.1: only the .sub operand's leaves cause evaluations.
// §4.7/§6.8: one write per positive slot; check after settling.
typedef struct { int v; } inner_t;
typedef struct { inner_t sub; int tag; } item_t;
module top;
  item_t arr[2];
  int idx = 0;
  longint result;
  function automatic longint stamp(input inner_t unused); return $time; endfunction
  assign result = stamp(arr[idx].sub);
  initial begin
    #1 arr[0].tag = 1;
    #1 if (result === 0) $display("PASS sibling_quiet");
       else $display("FAIL sibling_quiet got=%0d expected=0", result);
    #1 arr[0].sub.v = 3;
    #1 if (result === 3) $display("PASS member");
       else $display("FAIL member got=%0d expected=3", result);
    #1 arr[1].tag = 5;
    #1 if (result === 3) $display("PASS other_sibling_quiet");
       else $display("FAIL other_sibling_quiet got=%0d expected=3", result);
    $finish;
  end
endmodule
