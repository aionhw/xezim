`include "../common/svtest_defs.svh"

// §14.3: a `clocking @(negedge clk)` block samples and advances on the NEGedge,
// not the posedge. With clk initialized at declaration (0, no x→0 glitch) the
// first `@(cb)` resumes at the first real negedge; a negedge-driven counter is
// then one ahead of the preponed sample (verified vs a reference simulator:
// cb.d==0/d==1 at the first negedge, cb.d==1/d==2 at the second).
module test_clocking_negedge;
  `SVTEST_INIT

  logic clk = 0;
  logic [7:0] d = 0;

  always #5 clk = ~clk;          // posedge at 5,15,..; negedge at 10,20,..
  always @(negedge clk) d <= d + 1;

  clocking cb @(negedge clk);
    input d;
  endclocking

  logic [7:0] s1, s2, d1, d2;
  initial begin
    @(cb); s1 = cb.d; d1 = d;    // t=10: d 0->1, preponed cb.d = 0
    @(cb); s2 = cb.d; d2 = d;    // t=20: d 1->2, preponed cb.d = 1
    `SVTEST_CHECK(s1 == 8'd0 && d1 == 8'd1, "negedge clocking wrong at first negedge")
    `SVTEST_CHECK(s2 == 8'd1 && d2 == 8'd2, "negedge clocking did not advance on negedge")
    `SVTEST_PASSFAIL
    $finish;
  end
endmodule
