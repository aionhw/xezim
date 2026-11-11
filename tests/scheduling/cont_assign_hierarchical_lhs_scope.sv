`timescale 1ns/1ps
// IEEE 1800-2023: a continuous assignment whose left-hand side is a
// hierarchical reference into a child (§10.3.2, §23.6) reads top-level
// operands that share their names with unpacked structs local to that child.
// The right-hand names resolve in the scope of the assignment, the top
// module (§23.9), so §10.3.2 ("whenever an operand in the right-hand
// expression changes value, the whole right-hand side shall be evaluated")
// needs the TOP operands. Writes and checks are one unit apart (§4.7, §6.8).
// §7.2.2/§7.4.2: fixed unpacked aggregates; §10.9: assignment patterns.
// §13.5.1: automatic function inputs copy their actual values.
typedef struct { int value; int tag; } item_t;
function automatic int total(input item_t a [2]);
  return a[0].value + a[1].value + a[0].tag + a[1].tag;
endfunction
module child;
  item_t s;               // same name as top.s (an int)
  item_t arr [2];         // same name as top.arr (a struct array)
  wire [31:0] w1, w2;
  initial begin
    #1 s = '{100, 100}; arr[0] = '{100, 100}; arr[1] = '{100, 100};
  end
endmodule
module top;
  int s;
  item_t arr [2];
  child c();
  assign c.w1 = s + 1;          // top.s
  assign c.w2 = total(arr);     // top.arr
  task automatic check(input string n, input int got, input int exp);
    if (got === exp) $display("PASS %s got=%0d", n, got);
    else $display("FAIL %s got=%0d expected=%0d", n, got, exp);
  endtask
  initial begin
    #2 s = 5; #1 arr[0] = '{1, 2}; arr[1] = '{3, 4};
    #1 check("int_init", c.w1, 6); check("array_init", c.w2, 10);
       s = 9; #1 arr[1].tag = 9;
    #1 check("int_change", c.w1, 10); check("array_change", c.w2, 15);
    $display("DONE");
    $finish;
  end
endmodule
