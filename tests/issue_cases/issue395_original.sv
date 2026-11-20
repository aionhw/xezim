// IEEE 1800-2023 16.12.9: `s #-# p` is equivalent to `not (s |-> not p)`, so
// an attempt succeeds at the first tick at which an evaluation of p, started
// at an end point of a match of s, succeeds. 16.14.8: the attempt is vacuous
// when that evaluation of p is vacuous.
//
// Only the attempt at edge 1 has matches of (a ##[0:1] b): at edge 1 and at
// edge 2. At edge 1, c is 0, so (c |-> ...) succeeds vacuously and the attempt
// succeeds there, vacuously. Every other attempt fails (a is 0).
module top;
  initial $assertvacuousoff(0);
  logic clk = 0, a = 0, b = 0, c = 0, d = 0;
  int pass_times[$], fails1 = 0, fails2 = 0, hits = 0;
  always #5 clk = ~clk;

  // Decided (vacuously) at edge 1: no pass action, no cover hit. The
  // nonvacuous success of the match at edge 2 comes after the decision.
  a1: assert property (@(posedge clk) (a ##[0:1] b) #-# (c |-> d))
        pass_times.push_back($time); else fails1++;
  c1: cover property (@(posedge clk) (a ##[0:1] b) #-# (c |-> d)) hits++;
  // The match at edge 2 starts an obligation that never completes, but the
  // attempt already succeeded at edge 1: it must not fail at the end.
  a2: assert property (@(posedge clk) (a ##[0:1] b) #-# (c |-> s_eventually 1'b0))
        else fails2++;

  initial begin
    @(negedge clk) begin a = 1; b = 1; c = 0; end          // sampled at edge 1 (15)
    @(negedge clk) begin a = 0; b = 1; c = 1; d = 1; end  // sampled at edge 2 (25)
    @(negedge clk) begin b = 0; c = 0; d = 0; end
    @(negedge clk);
    @(negedge clk) $finish;                                // edges 0..4 sampled
  end
  final begin
    // 4 attempts fail outright (edges 0, 2, 3, 4); the one at edge 1 succeeds
    // vacuously at edge 1.
    if (pass_times.size() == 0 && hits == 0 && fails1 == 4 && fails2 == 4)
      $display("PASS");
    else
      $display("FAIL: pass actions at %p, %0d cover hits, %0d and %0d failures (expected none, 0, 4 and 4)",
               pass_times, hits, fails1, fails2);
  end
endmodule
