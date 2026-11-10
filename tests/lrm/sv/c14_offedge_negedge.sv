// IEEE 1800-2017 14.16: a drive that executes between clocking events "shall
// perform their drive action as if they had executed at the time of the next
// clocking event". 14.16.2: "When more than one synchronous drive on the same
// clocking block output (or inout) is scheduled to mature in the same Re-NBA
// region of the same time step, the last value is the only value driven onto
// the output signal. This is true whether the synchronous drives execute at
// times coincident with clocking events, or at times in between clocking
// events (within the same clock cycle)."
// `branch <= 1` at 5.001 and `branch <= 0` at the event at 15 both mature on
// the negedge at 20; 0 wins, so branch never goes high.
`timescale 1ns/1ps
module c14on;
  logic clk = 0;
  always #5 clk = ~clk;          // posedge 5, 15, 25, 35; negedge 10, 20, 30
  logic branch = 0;
  clocking cb @(posedge clk);
    default output negedge;
    output branch;
  endclocking
  initial begin
    @(posedge clk);              // t=5
    #1ps;                        // t=5.001
    cb.branch <= 1'b1;
    @(cb);                       // t=15
    cb.branch <= 1'b0;
  end
  always @(branch) $display("T|chg|t=%0t branch=%b", $realtime, branch);
  always @(posedge clk) $display("T|pos|t=%0t branch=%b", $time, branch);
  initial begin repeat (4) @(posedge clk); #1 $finish; end
endmodule
