// IEEE 1800-2023 14.4, 14.16: the output skew of a renamed clockvar
// (`output #2 q = q_drv;`) applies to a drive issued between clocking events.
// The drive at t=6 acts as if executed at the next clocking event (t=15) and
// the net changes skew units later, at t=17 (#824).
`timescale 1ns/1ns
module c144r;
  logic clk = 0;
  always #5 clk = ~clk;
  logic q_drv, r_drv;
  clocking cb @(posedge clk);
    output #2 q = q_drv;
    output #3 r = r_drv;
  endclocking
  initial begin
    @(cb); #1 cb.q <= 1;
    @(posedge q_drv);
    $display("T|14.4|q t=%0t", $time);
    @(cb); cb.r <= 1;
    @(posedge r_drv);
    $display("T|14.4|r t=%0t", $time);
    $finish;
  end
endmodule
