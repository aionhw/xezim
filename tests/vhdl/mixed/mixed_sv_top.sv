// A SystemVerilog testbench around a VHDL entity (mixed_sv_top.vhd).
`timescale 1ns/1ps
module mixed_sv_top;
  localparam W = 6;
  localparam BASE = 2;
  logic clk = 0, rst;
  logic [W-1:0] din;
  logic [3:0] asc;
  wire [W-1:0] q;
  wire [7:0] sum;
  wire carry, zero;
  wire [8:0] nine;
  wire [3:0] seen;
  bit b_in;
  wire b_out;
  int i_in;
  wire signed [31:0] i_out;
  real r_in, r_out;

  acc #(.width(W), .init(BASE * 2 - 1), .name("acc0"), .neg(1), .scale(0.5), .pat(4'b1010))
    u_acc (.clk(clk), .rst(rst), .din(din), .asc(asc), .q(q), .sum(sum), .carry(carry),
           .zero(zero), .nine(nine), .seen(seen), .b_in(b_in), .b_out(b_out),
           .i_in(i_in), .i_out(i_out), .r_in(r_in), .r_out(r_out));

  always #5 clk = ~clk;

  // The accumulator after each rising edge.
  always @(posedge clk)
    #1 $display("%0d q=%h sum=%h carry=%b zero=%b count=%0d", $time, q, sum, carry, zero,
                u_acc.count);

  initial begin
    rst = 1;
    din = 0;
    asc = 4'b01xz;
    b_in = 0;
    i_in = 5;
    r_in = 3.0;
    #1 $display("nine=%b seen=%b b_out=%b i_out=%0d r_out=%f", nine, seen, b_out, i_out, r_out);
    #11 rst = 0;
    repeat (12) begin
      @(negedge clk);
      din = din + 7;
    end
    asc = 4'b1100;
    b_in = 1;
    i_in = -20;
    r_in = -1.5;
    #1 $display("seen=%b b_out=%b i_out=%0d r_out=%f", seen, b_out, i_out, r_out);
    // A combinational output, sampled between input changes.
    for (int k = 0; k < 5; k++) begin
      i_in = k * 3;
      #1 $display("i_in=%0d i_out=%0d", i_in, i_out);
    end
    $finish;
  end

endmodule
