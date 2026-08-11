`include "../common/svtest_defs.svh"

// Clocking audit vs a reference simulator: §14.3 signal renaming
// (input alias = net), §14.12 standalone `default clocking cb;`, §14.13
// `@(cb.sig)` events (fire at the clock edge, only on a sample change),
// clocking reads inside functions/tasks, and §14.16 `cb.out <= ##N v`.
`timescale 1ns/1ps
module test_clocking_audit;
  `SVTEST_INIT

  bit clk = 0;
  always #5 clk = ~clk;
  logic [7:0] raw = 0;
  logic [7:0] drv = 0;
  logic [7:0] q   = 0;

  clocking cb @(posedge clk);
    input  aliased = raw;      // §14.3 renaming
    output oalias  = drv;
    output q;
  endclocking
  default clocking cb;         // §14.12 standalone designation

  always @(posedge clk) raw <= raw + 1;

  function automatic [7:0] rd(); return cb.aliased; endfunction

  int ev_hits = 0;
  int last_seen = -1;
  always @(cb.aliased) begin ev_hits++; last_seen = cb.aliased; end

  initial begin
    // renaming: input samples the aliased net (preponed = one behind)
    repeat (3) @(cb);
    `SVTEST_CHECK(cb.aliased == 8'd2 && raw == 8'd3, "input renaming wrong")
    `SVTEST_CHECK(rd() == 8'd2, "clocking read in function wrong")

    // renaming: output alias drives the bound net
    cb.oalias <= 8'h77;
    @(cb); #1;
    `SVTEST_CHECK(drv == 8'h77, "output renaming drive wrong")

    // §14.16 ##N cycle-delayed clocking drive
    @(cb);
    q <= 8'h00; @(cb);         // settle q known
    cb.q <= ##2 8'hBB;
    @(cb); #1 `SVTEST_CHECK(q !== 8'hBB, "##N drive matured too early")
    @(cb); #1 `SVTEST_CHECK(q === 8'hBB, "##N drive did not mature")

    // §14.11: ##0 SYNCHRONIZES to the clocking event (reference-validated):
    // executed off the edge (here: edge+1ns) it waits for the next event;
    // executed at the event's own slot it is a no-op.
    begin time t0 = $time; ##0; `SVTEST_CHECK($time > t0, "##0 off-edge must wait for the event") end
    begin time t1 = $time; ##0; `SVTEST_CHECK($time == t1, "##0 at the event must not wait") end

    // §14.13: the sample-change event fired (once per changed edge, no runts)
    `SVTEST_CHECK(ev_hits >= 3 && last_seen >= 0, "@(cb.sig) event did not fire on samples")

    `SVTEST_PASSFAIL
  end
endmodule
