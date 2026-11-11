`timescale 1ns/1ps
// IEEE 1800-2023 §7.2, §7.4.2/§7.4.4: unpacked structs and arrays.
typedef struct { int value; int tag; } item_t;
// §13.4/§13.5.1: automatic functions copy their input arrays by value.
function automatic item_t sum(input item_t items[2]);
  $display("eval one %0t", $time);
  return '{items[0].value + items[1].value,
            items[0].tag + items[1].tag}; // §10.9 assignment pattern
endfunction
module top;
  item_t source[2], items[2];
  item_t result;
  // §10.3.2: the whole actual is a right-hand operand.
  assign result = sum(source);
  initial begin
    // §4.7/§6.8: initialize after time zero and observe in later time slots.
    #1 source[0] = '{2,3}; source[1] = '{5,7};
    #1 $display("initial %0d %0d", result.value, result.tag);
       source[1].value = 23;
    #1 $display("value %0d", result.value);
       source[0].tag = 31;
    #1 $display("tag %0d", result.tag);
    // The input copies shadow these caller arrays (§13.5.1).
    // Do not constrain extra evaluations from conservative body dependencies.
    #1 items[0] = '{41,43}; items[1] = '{47,53};
    #1 $display("unchanged %0d %0d", result.value, result.tag);
       $finish;
  end
endmodule
