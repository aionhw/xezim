`include "../common/svtest_defs.svh"

// §14.13: a process resuming on a clocking event (`@(cb)` / `##N`) runs in the
// Reactive region — AFTER this edge's NBA updates commit. So a counter bumped
// by `always @(posedge clk) c<=c+1` is already incremented when the `@(cb)`
// continuation observes it (verified against a reference simulator: c==1 after
// one `@(cb)`, c==4 after a further `##3`). A raw `@(posedge clk)` waiter keeps
// the "resume before same-edge blocks" behavior; only clocking events defer.
module test_clocking_resume;
  `SVTEST_INIT

  logic clk;
  int   c;

  initial clk = 0;
  always #1 clk = ~clk;
  initial c = 0;
  always @(posedge clk) c <= c + 1;

  default clocking cb @(posedge clk);
  endclocking

  initial begin
    @(cb);
    `SVTEST_CHECK(c == 1, "@(cb) did not resume after same-edge NBA update")
    ##3;
    `SVTEST_CHECK(c == 4, "##3 did not advance to post-edge state")

    `SVTEST_PASSFAIL
    $finish;
  end
endmodule
