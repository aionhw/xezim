// IEEE 1800-2017 9.4.5 / 22.4: an intra-assignment delay is a timing control, and
// `include text is compiled exactly as if it appeared in place. The interface in
// inc/gen_clk_if.svh toggles `c` every 5 ticks and updates `q` 3 ticks after each
// posedge of `c`, so the checks below hold. Every line should print PASS.
`timescale 1ns/1ns
`include "gen_clk_if.svh"

module t;
  gen_clk_if g();
  int fails = 0;
  task automatic chk(string what, logic got, logic exp);
    if (got === exp) $display("PASS %s", what);
    else begin $display("FAIL %s: got %b, expected %b", what, got, exp); fails++; end
  endtask
  initial begin
    #4;  chk("c at t=4 (first toggle at 5)", g.c, 1'b0);
    #2;  chk("c at t=6", g.c, 1'b1);
    #1;  chk("q at t=7 (posedge at 5, update at 8)", g.q, 1'b0);
    #2;  chk("q at t=9", g.q, 1'b1);
    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
