// Power-aware example design: an accumulator block that can be switched off,
// with a configuration register that is retained across the power-down, under
// an always-on top level. The power intent lives in pwr_demo.upf.
module acc_unit(input logic clk, input logic rst_n, input logic add,
                input logic [7:0] din,
                output logic [7:0] sum, output logic nz);
  logic [7:0] acc;
  always @(posedge clk)
    if (!rst_n) acc <= 8'd0;
    else if (add) acc <= acc + din;
  assign sum = acc;
  assign nz = (acc != 8'd0);
endmodule

module cfg_reg(input logic clk, input logic we, input logic [7:0] d,
               output logic [7:0] q);
  logic [7:0] r;
  always @(posedge clk) if (we) r <= d;
  assign q = r;
endmodule

module pwr_demo_top(input logic clk, input logic rst_n,
                    input logic add, input logic [7:0] din,
                    input logic cfg_we, input logic [7:0] cfg_d,
                    input logic acc_on, input logic acc_iso_en,
                    output logic [7:0] sum, output logic nz,
                    output logic [7:0] cfg_q);
  acc_unit u_acc(.clk(clk), .rst_n(rst_n), .add(add), .din(din), .sum(sum), .nz(nz));
  cfg_reg  u_cfg(.clk(clk), .we(cfg_we), .d(cfg_d), .q(cfg_q));
endmodule
