// The SV side of mixed_nested_record_port.vhd: fld_arr(0 to 1, 0 to 1,
// 0 to 2) of a two-bit record, element (i, j, k) at bits 23 - 2 * n and
// 22 - 2 * n (value, swmod) with n = 6 * i + 3 * j + k.
`timescale 1ns/1ns
module mixed_nested_record_port;
  logic clk = 0, d = 0;
  wire [23:0] r1;
  wire en, v102;
  wrap dut (.clk(clk), .d(d), .hwif_out_r1(r1), .en(en), .v102(v102));
  initial begin
    #1 d = 1; clk = 1; #1 clk = 0;
    #1 $display("r1=%b en=%b v102=%b", r1, en, v102);
    d = 0; clk = 1; #1 clk = 0;
    #1 $display("r1=%b en=%b v102=%b", r1, en, v102);
    $finish;
  end
endmodule
