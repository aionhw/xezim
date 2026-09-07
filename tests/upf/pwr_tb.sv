module pwr_tb;
  import UPF::*;
  logic clk = 0, rst_n = 0, go = 0, en = 0, we = 0, mul_off = 0, mul_iso = 0;
  logic [3:0] a = 0, b = 0; logic [7:0] d = 0;
  logic [7:0] y, q; logic zero, busy;
  lp_soc_top dut(.*);
  always #5 clk = ~clk;
  int fails = 0;
  task check(string what, logic [7:0] got, logic [7:0] exp);
    if (got !== exp) begin fails++; $display("FAIL %s: got %h expected %h", what, got, exp); end
    else $display("ok   %s = %h", what, got);
  endtask
  task edge_(int n); repeat (n) @(posedge clk); #1; endtask
  task compute(input logic [3:0] x, input logic [3:0] z);
    a = x; b = z; go = 1; edge_(1); go = 0; edge_(1);
  endtask
  initial begin
    int st;
    #1;
    st = supply_on("/pwr_tb/dut/u_core/VMAIN", 1.2);
    st = supply_on("/pwr_tb/dut/u_core/VLOW", 0.8);
    st = supply_on("/pwr_tb/dut/u_core/GND", 0.0);
    check("get_supply_on_state(VMAIN)", get_supply_on_state("/pwr_tb/dut/u_core/VMAIN"), 1);
    edge_(2); rst_n = 1; edge_(1);
    compute(4'd3, 4'd4);
    check("y after 3*4", y, 8'd12); check("zero", zero, 0);
    // Clean power-down: isolate first, then switch off.
    mul_iso = 1; #1;
    check("y isolated", y, 8'd0); check("zero isolated (clamp 1)", zero, 1);
    check("internal res untouched", dut.u_core.u_mul.res, 8'd12);
    mul_off = 1; #1;
    check("internal res corrupted", dut.u_core.u_mul.res, 8'bx);
    check("y still clamped", y, 8'd0);
    edge_(3);
    mul_off = 0; #1;                       // power back up: still x until written
    check("res x after power-up", dut.u_core.u_mul.res, 8'bx);
    rst_n = 0; edge_(2); rst_n = 1; edge_(1);
    check("res reset after power-up", dut.u_core.u_mul.res, 8'd0);
    mul_iso = 0; #1;
    check("y follows again", y, 8'd0); check("zero follows again", zero, 1);
    compute(4'd5, 4'd6);
    check("y after 5*6", y, 8'd30);
    // Sloppy power-down: no isolation -> outputs go x and a warning is logged.
    mul_off = 1; #1;
    check("y x without isolation", y, 8'bx);
    mul_off = 0; #1; rst_n = 0; edge_(2); rst_n = 1; edge_(1);
    compute(4'd6, 4'd7);
    check("y after 6*7", y, 8'd42);
    // Low domain: registers retained, control counter corrupted.
    we = 1; d = 8'h9c; edge_(1); we = 0; en = 1; edge_(3);
    check("q written", q, 8'h9c);
    st = supply_off("/pwr_tb/dut/u_core/VLOW"); #1;
    check("ctl counter corrupted", {4'b0, dut.u_core.u_ctl.cnt}, {4'b0, 4'bx});
    check("regs retained", q, 8'h9c);
    st = supply_on("/pwr_tb/dut/u_core/VLOW", 0.8); #1;
    check("regs still retained", q, 8'h9c);
    check("voltage VLOW", get_supply_voltage("/pwr_tb/dut/u_core/VLOW") == 0.8, 1);
    if (fails == 0) $display("UPF_TEST_PASS"); else $display("UPF_TEST_FAIL %0d", fails);
    $finish;
  end
endmodule
