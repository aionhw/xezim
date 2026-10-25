// A SystemVerilog module instantiated as a component of a VHDL testbench
// (mixed_vhdl_top.vhd).
module Counter #(parameter WIDTH = 4, parameter string NAME = "c", parameter real STEP = 1.0) (
  input logic clk, rst,
  input logic [WIDTH-1:0] load,
  output logic [WIDTH-1:0] Count,
  output logic wrap,
  output int steps,
  output real scaled);
  always @(posedge clk) begin
    if (rst) Count <= load;
    else Count <= Count + 1;
  end
  assign wrap = &Count;
  assign steps = Count * 3;
  assign scaled = Count * STEP;
  initial #2 $display("%s: width %0d step %0.2f", NAME, WIDTH, STEP);
endmodule
