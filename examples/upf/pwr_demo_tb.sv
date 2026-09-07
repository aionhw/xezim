// Testbench for the power-aware example: drives the supplies through the
// standard UPF package and walks the block through a clean power cycle.
module pwr_demo_tb;
  import UPF::*;
  logic clk = 0, rst_n = 0, add = 0, cfg_we = 0, acc_on = 1, acc_iso_en = 0;
  logic [7:0] din = 0, cfg_d = 0;
  logic [7:0] sum, cfg_q; logic nz;
  pwr_demo_top dut(.*);
  always #5 clk = ~clk;

  int fails = 0;
  task check(string what, logic [7:0] got, logic [7:0] exp);
    if (got !== exp) begin fails++; $display("FAIL %s: got %h expected %h", what, got, exp); end
    else $display("ok   %s = %h", what, got);
  endtask
  task cycles(int n); repeat (n) @(posedge clk); #1; endtask

  initial begin
    int st;
    #1;
    st = supply_on("/pwr_demo_tb/dut/VDD", 1.0);
    st = supply_on("/pwr_demo_tb/dut/VSS", 0.0);
    cycles(2); rst_n = 1; cycles(1);

    // Normal operation: accumulate 3 + 4, program the config register.
    din = 8'd3; add = 1; cycles(1); din = 8'd4; cycles(1); add = 0; cycles(1);
    check("sum after 3+4", sum, 8'd7); check("nz", nz, 1);
    cfg_d = 8'ha5; cfg_we = 1; cycles(1); cfg_we = 0; cycles(1);
    check("cfg_q written", cfg_q, 8'ha5);

    // Clean power-down: isolate first, then open the switch.
    acc_iso_en = 1; #1;
    check("sum clamped", sum, 8'd0); check("nz clamped", nz, 0);
    acc_on = 0; #1;
    check("acc corrupted", dut.u_acc.acc, 8'bx);
    check("cfg retained", dut.u_cfg.r, 8'ha5);
    check("sum still clamped", sum, 8'd0);
    cycles(3);

    // Power-up: reset the accumulator, then release isolation.
    acc_on = 1; #1;
    rst_n = 0; cycles(2); rst_n = 1; cycles(1);
    acc_iso_en = 0; #1;
    check("sum after power-up", sum, 8'd0);
    check("cfg still retained", cfg_q, 8'ha5);
    din = 8'd5; add = 1; cycles(1); add = 0; cycles(1);
    check("sum after +5", sum, 8'd5);
    check("VDD_ACC on", get_supply_on_state("/pwr_demo_tb/dut/VDD_ACC"), 1);

    if (fails == 0) $display("UPF_EXAMPLE_PASS"); else $display("UPF_EXAMPLE_FAIL %0d", fails);
    $finish;
  end
endmodule
