`timescale 1ns/1ps
// IEEE 1800-2023 §16.4.2, §13.3: task completion is not a deferred
// assertion flush point. Fork children reuse each task call site (§9.3.2).
// Each task contains a completed fork so reused sites remain eligible to share.
// Calls complete before the next stimulus; sample after all report actions.
module top;
  task automatic observed();
    fork begin #1; end join
    assert #0 (1) $display("T|observed");
  endtask
  task automatic postponed();
    fork begin #1; end join
    assert final (1) $display("T|postponed");
  endtask
  initial begin
    repeat (3) fork observed(); join
    repeat (3) fork postponed(); join
    #1; $display("T|deferred done"); $finish;
  end
endmodule
