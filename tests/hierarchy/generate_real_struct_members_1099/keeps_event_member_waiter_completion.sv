// IEEE 1800-2023 §6.17 initializes a new event object; §15.5.1: named event triggers "unblock all processes currently waiting"; §15.5.2 waiter starts before the #1 trigger, giving seen=1 at #2.
// Keep eventual event waiter completion; main resumes too early, so no trigger-time assertion is made.
`timescale 1ns/1ps
typedef struct {int tag; event e;} S;
module top;
 if (1) begin:g
  S a;
  int seen=0;
  initial begin
   fork begin @(a.e); seen=1; end join_none
   #1 -> a.e;
   #1 $display("T|seen %0d",seen);
   $finish;
  end
 end
endmodule
