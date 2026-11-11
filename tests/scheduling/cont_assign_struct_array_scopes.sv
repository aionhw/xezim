`timescale 1ns/1ps
// IEEE 1800-2023 §23.9: local declarations hide outer names; §27.5:
// conditional generate scopes. §7.2.2/§7.4.2: unpacked struct arrays.
// §10.3.2 requires reevaluation on a member change; §20.3.1 gives its time.
// §13.4/§13.5.1: automatic input function, without time controls.
// §4.7/§6.8: isolated writes after time zero, observations a unit later.
typedef struct { int value; } item_t;
function automatic longint stamp(input item_t unused[2]);
  return $time;
endfunction
module leaf;
  item_t arr[2];
endmodule
module child;
  item_t arr[2];
  longint result;
  assign result = stamp(arr);
  leaf c();
  longint downward_result, block_result;
  assign downward_result = stamp(c.arr);
  if (1) begin : h
    item_t arr[2];
  end
  assign block_result = stamp(h.arr);
endmodule
module top;
  item_t arr[2];
  longint result;
  child c();
  if (1) begin : h
    item_t arr[2];
  end
  assign result = stamp(arr);
  if (1) begin : g
    item_t arr[2];
    longint result;
    assign result = stamp(arr);
  end
  task automatic check(input string name, input longint got, input longint expected);
    if (got === expected) $display("PASS %s", name);
    else $display("FAIL %s got=%0d expected=%0d", name, got, expected);
  endtask
  initial begin
    #1 arr[0].value = 1;
    #1 check("top", result, 1);
    #1 c.arr[1].value = 2;
    #1 check("child", c.result, 3);
    #1 g.arr[0].value = 3;
    #1 check("generate", g.result, 5);
    #1 c.c.arr[0].value = 4;
    #1 check("downward_same_name", c.downward_result, 7);
    #1 c.h.arr[1].value = 5;
    #1 check("downward_block", c.block_result, 9);
    $finish;
  end
endmodule
