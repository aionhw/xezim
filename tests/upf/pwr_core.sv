// Renamed stand-in for a switched-domain core: a multiplier block that can
// be powered off (with isolation on its outputs), a low-voltage control /
// register pair (registers retained), and an always-on wrapper.
module mul_unit(input logic clk, input logic rst_n, input logic go,
                input logic [3:0] a, input logic [3:0] b,
                output logic [7:0] res, output logic zero);
  always @(posedge clk)
    if (!rst_n) res <= 8'd0;
    else if (go) res <= a * b;
  assign zero = (res == 8'd0);
endmodule

module ctl_unit(input logic clk, input logic rst_n, input logic en, output logic busy);
  logic [3:0] cnt;
  always @(posedge clk)
    if (!rst_n) cnt <= 4'd0;
    else if (en) cnt <= cnt + 4'd1;
  assign busy = cnt[0];
endmodule

module reg_bank(input logic clk, input logic we, input logic [7:0] d, output logic [7:0] q);
  logic [7:0] r;
  always @(posedge clk) if (we) r <= d;
  assign q = r;
endmodule

module core_blk(input logic clk, input logic rst_n, input logic go,
                input logic [3:0] a, input logic [3:0] b,
                input logic en, input logic we, input logic [7:0] d,
                input logic mul_off, input logic mul_iso,
                output logic [7:0] y, output logic zero,
                output logic busy, output logic [7:0] q);
  logic [7:0] mul_res;
  logic mul_zero;
  mul_unit u_mul(.clk(clk), .rst_n(rst_n), .go(go), .a(a), .b(b), .res(mul_res), .zero(mul_zero));
  ctl_unit u_ctl(.clk(clk), .rst_n(rst_n), .en(en), .busy(busy));
  reg_bank u_regs(.clk(clk), .we(we), .d(d), .q(q));
  assign y = mul_res;
  assign zero = mul_zero;
endmodule

module lp_soc_top(input logic clk, input logic rst_n, input logic go,
                  input logic [3:0] a, input logic [3:0] b,
                  input logic en, input logic we, input logic [7:0] d,
                  input logic mul_off, input logic mul_iso,
                  output logic [7:0] y, output logic zero,
                  output logic busy, output logic [7:0] q);
  core_blk u_core(.*);
endmodule
