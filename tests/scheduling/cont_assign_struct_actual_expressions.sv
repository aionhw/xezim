`timescale 1ns/1ps
// IEEE 1800-2023 §10.3.2: every operand of a call actual requires
// reevaluation on change. §20.3.1: $time records that evaluation's time.
// §7.2.2/§7.4.2: unpacked aggregates; §7.4.5: slices;
// §10.9: assignment patterns; §11.4.11: conditional expressions.
// §13.4/§13.5.1: automatic functions with input arguments, no delays.
// §4.7/§6.8: each write is alone in its slot; check one unit later.
typedef struct { int value; } item_t;
typedef struct { item_t inner[2]; } outer_t;
module top;
  item_t big[4], indexed[4], elements[2], a[2], b[2];
  outer_t o, oa[2], dynamic_oa[2];
  int i;
  bit sel;
  function automatic longint stamp(input item_t unused[2]);
    return $time;
  endfunction
  longint slice_result, indexed_result, member_result, element_result,
          dynamic_result, pattern_result, conditional_result;
  assign slice_result = stamp(big[1:2]);
  assign indexed_result = stamp(indexed[i+:2]);
  assign member_result = stamp(o.inner);
  assign element_result = stamp(oa[1].inner);
  assign dynamic_result = stamp(dynamic_oa[i].inner);
  assign pattern_result = stamp('{elements[0], elements[1]});
  assign conditional_result = stamp(sel ? b : a);
  task automatic check(input string name, input longint got, input longint expected);
    if (got === expected) $display("PASS %s", name);
    else $display("FAIL %s got=%0d expected=%0d", name, got, expected);
  endtask
  initial begin
    #1 i = 0;
    #1 sel = 0;
    #1 big[2].value = 1;
    #1 check("slice", slice_result, 3);
    #1 indexed[1].value = 2;
    #1 check("indexed_slice", indexed_result, 5);
    #1 o.inner[1].value = 3;
    #1 check("member", member_result, 7);
    #1 oa[1].inner[0].value = 4;
    #1 check("element_member", element_result, 9);
    #1 dynamic_oa[0].inner[1].value = 5;
    #1 check("dynamic_member", dynamic_result, 11);
    #1 elements[0].value = 6;
    #1 check("pattern_first", pattern_result, 13);
    #1 elements[1].value = 7;
    #1 check("pattern_second", pattern_result, 15);
    #1 a[0].value = 8;
    #1 check("conditional_selected", conditional_result, 17);
    #1 b[1].value = 9;
    #1 check("conditional_other_operand", conditional_result, 19);
    $finish;
  end
endmodule
