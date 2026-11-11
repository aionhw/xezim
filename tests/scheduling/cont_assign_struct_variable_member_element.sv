`timescale 1ns/1ps
// IEEE 1800-2023 §7.2.2/§7.4.2/§7.4.5: retain a member-element suffix.
// §10.3.2/§20.3.1: only the actual member subtree requires reevaluation.
// §13.4/§13.5.1 and §4.7/§6.8: ignored input, isolated writes and later checks.
typedef struct { int v; } inner_t;
typedef struct { inner_t inner[2]; int tag; } outer_t;
module top;
  outer_t arr[2];
  int idx = 0;
  longint result;
  function automatic longint stamp(input inner_t unused); return $time; endfunction
  assign result = stamp(arr[idx].inner[0]);
  initial begin
    #1 arr[0].tag = 1;
    #1 if (result === 0) $display("PASS sibling"); else $display("FAIL sibling got=%0d",result);
    #1 arr[0].inner[1].v = 3;
    #1 if (result === 0) $display("PASS other_inner"); else $display("FAIL other_inner got=%0d",result);
    #1 arr[0].inner[0].v = 5;
    #1 if (result === 5) $display("PASS selected"); else $display("FAIL selected got=%0d",result);
    $finish;
  end
endmodule
