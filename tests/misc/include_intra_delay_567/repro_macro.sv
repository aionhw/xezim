// IEEE 1800-2017 9.4.5 / 22.5.1: text a macro expands to is compiled as if it
// were written in place, so a delay in a macro body delays the assignment.
// The macros live in an `include file (inc/delay_macros.svh).
// Every line should print PASS.
`timescale 1ns/1ns
`include "delay_macros.svh"

module t;
  logic a = 0, b = 0;
  int fails = 0;
  task automatic chk(string what, logic got, logic exp);
    if (got === exp) $display("PASS %s", what);
    else begin $display("FAIL %s: got %b, expected %b", what, got, exp); fails++; end
  endtask
  initial begin
    `DELAYED_NBA(a, 4, 1'b1)           // a changes at t=4
    #2; chk("nonblocking, at t=2 (a still 0)", a, 1'b0);
    #3; chk("nonblocking, at t=5 (a is 1)", a, 1'b1);
    `DELAYED_BLK(b, 3, 1'b1)           // blocks 3 ticks, then b = 1
    chk("blocking, after its delay", b, 1'b1);
    if ($time == 8) $display("PASS blocking assignment blocked for 3 ticks");
    else begin $display("FAIL blocking assignment: now at %0t, expected 8", $time); fails++; end
    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
