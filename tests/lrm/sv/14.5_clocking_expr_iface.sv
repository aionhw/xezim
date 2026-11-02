// top: top
// §14.3, §14.5: a clockvar bound to an expression in an interface instance
// drives and samples that instance's signal; the same-named net `q` is not
// the clocking signal.
interface cif(input logic clk);
  logic q_drv;
  logic d_src;
  wire q;
  wire d;
  assign q = q_drv;
  assign d = 1'b0;
  clocking cb @(posedge clk);
    output q = q_drv;
    input d = d_src;
  endclocking
endinterface
module top;
  logic clk = 0;
  always #5 clk = ~clk;
  cif u1(clk);
  cif u2(clk);
  initial begin
    u1.d_src = 1;
    u2.d_src = 0;
    @(u1.cb);
    u1.cb.q <= 1;
    u2.cb.q <= 0;
    @(u1.cb);
    @(u1.cb);
    $display("T|q|u1=%b u2=%b", u1.q_drv, u2.q_drv);
    $display("T|d|u1=%b u2=%b", u1.cb.d, u2.cb.d);
    $finish;
  end
endmodule
