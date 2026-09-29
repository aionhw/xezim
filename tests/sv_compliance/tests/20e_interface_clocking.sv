`include "../common/svtest_defs.svh"

// §14.3 interface-scoped clocking: `@(iface_inst.cb)` synchronizes to the
// interface block's clock, `iface_inst.cb.<in>` samples with #1step skew, and
// `iface_inst.cb.<out> <= v` drives the interface net at the clock edge. The
// block is declared inside the interface and reached through the instance.
// Verified against a reference simulator.
interface sif20e(input bit clk);
  logic [7:0] d;
  clocking cb  @(posedge clk); input  d; endclocking
  clocking cbo @(negedge clk); output d; endclocking
endinterface

module test_interface_clocking;
  `SVTEST_INIT

  bit clk = 0;
  always #5 clk = ~clk;
  sif20e u(.clk(clk));
  initial u.d = 0;
  always @(posedge clk) u.d <= u.d + 1;   // negedge-free producer on posedge

  logic [7:0] s1, dd1;
  time t0, t1;
  initial begin
    @(u.cb);                       // sync to first posedge (t=5)
    `SVTEST_CHECK($time == 5, "interface @(cb) did not sync to clock edge")
    @(u.cb); s1 = u.cb.d; dd1 = u.d;
    // preponed sample is one behind the live (post-NBA) value
    `SVTEST_CHECK(dd1 == 8'd2 && s1 == 8'd1, "interface cb input skew/sample wrong")
    t0 = $time;
    repeat(3) @(u.cb);
    t1 = $time;
    `SVTEST_CHECK((t1 - t0) == 30, "interface cycle delay (repeat @(cb)) wrong")
    `SVTEST_PASSFAIL
    $finish;
  end
endmodule
