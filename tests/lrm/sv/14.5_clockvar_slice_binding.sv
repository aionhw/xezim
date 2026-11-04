// IEEE 1800-2023 14.5: a clockvar bound to a part-select of a signal. The
// input samples the slice at the clocking event; a drive of the output writes
// the slice of the bound variable and leaves its other bits alone (#823).
module c145s;
  logic clk = 0;
  always #5 clk = ~clk;
  logic [7:0] bus = 8'h5a;
  logic [7:0] o_drv = 8'h00;
  clocking cb @(posedge clk);
    input a = bus[3:0];
    input hi = bus[7:4];
    output o = o_drv[3:0];
    output oh = o_drv[7:6];
  endclocking
  initial begin
    @(cb); cb.o <= 4'h9;
    @(cb); @(cb);
    $display("T|14.5|in a=%h hi=%h", cb.a, cb.hi);
    $display("T|14.5|out o_drv=%h", o_drv);
    cb.oh <= 2'b11;
    @(cb);
    $display("T|14.5|out2 o_drv=%h", o_drv);
    bus = 8'hc3;
    $display("T|14.5|stale a=%h", cb.a);
    @(cb);
    $display("T|14.5|fresh a=%h hi=%h", cb.a, cb.hi);
    $finish;
  end
endmodule
