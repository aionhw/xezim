// IEEE 1800-2017 14.16.2: "When more than one synchronous drive on the same
// clocking block output (or inout) is scheduled to mature in the same Re-NBA
// region of the same time step, the last value is the only value driven onto
// the output signal. This is true whether the synchronous drives execute at
// times coincident with clocking events, or at times in between clocking
// events (within the same clock cycle)."
// Each output below is driven 1 and then 0, and both drives mature on one
// negedge, so the output never changes and nothing waiting on it wakes.
// a: 1 at 5.001 (between events), 0 at the event at 15; both mature at 20.
// b: 1 at 5.001, 0 at 7.001; both mature at 20.
// c: `##1` 1 at the event at 5, 0 at the event at 15; both mature at 20.
// d: as a, through `output #0`; both mature at 15.
`timescale 1ns/1ps
module c14lg;
  logic clk = 0;
  always #5 clk = ~clk;          // posedge 5, 15, 25, 35; negedge 10, 20, 30
  logic a = 0, b = 0, c = 0, d = 0;
  clocking cb @(posedge clk);
    default output negedge;
    output a, b, c;
    output #0 d;
  endclocking
  always @(a) $display("T|wake|t=%0t a=%b", $realtime, a);
  always @(posedge a) $display("T|rise|t=%0t a", $realtime);
  always @(b) $display("T|wake|t=%0t b=%b", $realtime, b);
  always @(posedge b) $display("T|rise|t=%0t b", $realtime);
  always @(c) $display("T|wake|t=%0t c=%b", $realtime, c);
  always @(posedge c) $display("T|rise|t=%0t c", $realtime);
  always @(d) $display("T|wake|t=%0t d=%b", $realtime, d);
  always @(posedge d) $display("T|rise|t=%0t d", $realtime);
  initial begin
    @(posedge clk);              // t=5
    #1ps;                        // t=5.001
    cb.a <= 1'b1;
    cb.b <= 1'b1;
    cb.d <= 1'b1;
    #2 cb.b <= 1'b0;             // t=7.001
    @(cb);                       // t=15
    cb.a <= 1'b0;
    cb.d <= 1'b0;
  end
  initial begin
    @(cb);                       // t=5
    cb.c <= ##1 1'b1;
    @(cb);                       // t=15
    cb.c <= 1'b0;
  end
  initial begin
    #40 $display("T|end a=%b b=%b c=%b d=%b", a, b, c, d);
    $finish;
  end
endmodule
