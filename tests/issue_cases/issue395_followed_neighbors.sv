// §16.12.9 / §16.14.8: settle at the first successful consequent tick,
// preserving vacuity and preferring simultaneous nonvacuous successes.
module followed_case #(bit SHIFT = 0, bit STRONG = 0, bit FAIL_FIRST = 0);
  initial $assertvacuousoff(0);
  logic clk = 0, a = 0, b = 0, c = 0, d = 0;
  int passes = 0, hits = 0, failures = 0;
  always #5 clk = ~clk;
  generate
    if (SHIFT) begin
      if (STRONG) begin
        ap: assert property (@(posedge clk) (a ##[0:2] b) #=# (c |-> s_eventually 1'b0))
          passes++; else failures++;
        cp: cover property (@(posedge clk) (a ##[0:2] b) #=# (c |-> s_eventually 1'b0)) hits++;
      end else begin
        ap: assert property (@(posedge clk) (a ##[0:2] b) #=# (c |-> d))
          passes++; else failures++;
        cp: cover property (@(posedge clk) (a ##[0:2] b) #=# (c |-> d)) hits++;
      end
    end else begin
      if (STRONG) begin
        ap: assert property (@(posedge clk) (a ##[0:2] b) #-# (c |-> s_eventually 1'b0))
          passes++; else failures++;
        cp: cover property (@(posedge clk) (a ##[0:2] b) #-# (c |-> s_eventually 1'b0)) hits++;
      end else begin
        ap: assert property (@(posedge clk) (a ##[0:2] b) #-# (c |-> d))
          passes++; else failures++;
        cp: cover property (@(posedge clk) (a ##[0:2] b) #-# (c |-> d)) hits++;
      end
    end
  endgenerate
  initial begin
    @(negedge clk) begin a = 1; b = 1; c = FAIL_FIRST; end
    @(negedge clk) begin a = 0; c = SHIFT ? FAIL_FIRST : 1; d = !SHIFT; end
    @(negedge clk) begin b = 0; c = SHIFT; d = SHIFT; end
    @(negedge clk) begin c = 0; d = 0; end
    @(negedge clk);
  end
  final $display("T|follow|%0d %0d %0d|%0d %0d %0d", SHIFT, STRONG, FAIL_FIRST, passes, hits, failures);
endmodule

module followed_simultaneous #(bit REVERSE = 0);
  initial $assertvacuousoff(0);
  logic clk = 0, a = 0, d = 1;
  int passes = 0, hits = 0, failures = 0;
  always #5 clk = ~clk;
  generate
    if (REVERSE) begin
      ap: assert property (@(posedge clk) a #-# (a or (1'b0 |-> d))) passes++; else failures++;
      cp: cover property (@(posedge clk) a #-# (a or (1'b0 |-> d))) hits++;
    end else begin
      ap: assert property (@(posedge clk) a #-# ((1'b0 |-> d) or a)) passes++; else failures++;
      cp: cover property (@(posedge clk) a #-# ((1'b0 |-> d) or a)) hits++;
    end
  endgenerate
  initial begin
    @(negedge clk) a = 1;
    @(negedge clk) a = 0;
    repeat (3) @(negedge clk);
  end
  final $display("T|mixed|%0d|%0d %0d %0d", REVERSE, passes, hits, failures);
endmodule

module top;
  initial $assertvacuousoff(0);
  followed_case overlap_vacuous();
  followed_case #(0, 1, 0) overlap_strong();
  followed_case #(0, 0, 1) overlap_failure_then_pass();
  followed_case #(1, 0, 0) nonoverlap_vacuous();
  followed_case #(1, 1, 0) nonoverlap_strong();
  followed_case #(1, 0, 1) nonoverlap_failure_then_pass();
  followed_simultaneous mixed_vacuous_first();
  followed_simultaneous #(1) mixed_nonvacuous_first();
  initial #50 $finish;
endmodule
