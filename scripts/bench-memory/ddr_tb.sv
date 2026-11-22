`timescale 1ps/1ps
module ddr_tb;
  localparam integer T = 2500;
  localparam integer WL = 5;
  reg rst_n, ck, cke, clocks_on;
  reg cs_n, ras_n, cas_n, we_n;
  reg [1:0] dm;
  reg odt;
  reg [2:0] ba;
  reg [15:0] addr;
  reg [15:0] tx;
  reg strobe, strobe_oe, data_oe;
  tri [15:0] dq;
  tri [1:0] dqs, dqs_n;
  assign dq = data_oe ? tx : 16'bz;
  assign dqs = strobe_oe ? {2{strobe}} : 2'bz;
  assign dqs_n = strobe_oe ? {2{~strobe}} : 2'bz;
  ddr3 #(.MEM_BITS(8)) dut (.rst_n(rst_n), .ck(ck), .ck_n(~ck), .cke(cke),
    .cs_n(cs_n), .ras_n(ras_n), .cas_n(cas_n), .we_n(we_n), .dm_tdqs(dm),
    .ba(ba), .addr(addr), .dq(dq), .dqs(dqs), .dqs_n(dqs_n), .tdqs_n(), .odt(odt));

  initial begin
    wait (clocks_on);
    forever #(T/2) ck = ~ck;
  end

  task automatic command(input [2:0] op, input [2:0] bank, input [15:0] a);
    @(negedge ck);
    #100;
    {cs_n,ras_n,cas_n,we_n} = {1'b0,op};
    ba = bank;
    addr = a;
    @(posedge ck);
    cs_n <= #(T/2) 1'b1;
  endtask

  function automatic [15:0] pattern(input integer burst_id, input integer beat);
    pattern = 16'h639a ^ (burst_id * 16'h11b) ^ (beat * 16'h2471);
  endfunction

  integer count = 4;
  integer dqs_phase = 100;
  integer b, j;
  reg [31:0] sum = 0;
  initial begin
    if ($value$plusargs("bursts=%d", count)) begin end
    if ($value$plusargs("dqs_phase=%d", dqs_phase)) begin end
    #10;
    {rst_n, ck, cke, clocks_on, strobe, strobe_oe, data_oe, odt} = 0;
    {cs_n, ras_n, cas_n, we_n} = 4'b1111;
    {ba, addr, tx, dm} = 0;
    #200100000 rst_n = 1;
    #501000000 clocks_on = 1;
    repeat (32) @(negedge ck);
    #100 cke = 1;
    repeat (128) @(posedge ck);
    command(3'b000, 2, 16'h0000);
    repeat (8) @(posedge ck);
    command(3'b000, 3, 16'h0000);
    repeat (8) @(posedge ck);
    command(3'b000, 1, 16'h0000);
    repeat (8) @(posedge ck);
    command(3'b000, 0, 16'h1520);
    repeat (16) @(posedge ck);
    command(3'b110, 0, 16'h0400);
    repeat (600) @(posedge ck);
    command(3'b011, 0, 16'h0000);
    repeat (12) @(posedge ck);
    for (b = 0; b < count; b = b + 1) begin
      command(3'b100, 0, (b & 63)*8);
      #(T*(WL-1)+dqs_phase);
      strobe_oe = 1;
      strobe = 0;
      #(T*3/4);
      for (j = 0; j < 8; j = j + 1) begin
        tx = pattern(b, j);
        data_oe = 1;
        #(T/4) strobe = ~strobe;
        #(T/4);
      end
      strobe_oe = 0;
      data_oe = 0;
      repeat (16) @(posedge ck);
      command(3'b101, 0, (b & 63)*8);
      @(posedge dqs[0]);
      for (j = 0; j < 8; j = j + 1) begin
        #(T/8);
        if (dq !== pattern(b, j)) $fatal(1, "Readback burst %0d beat %0d got %h expected %h", b,j,dq,pattern(b,j));
        sum = sum + dq;
        if (j < 7) @(dqs[0]);
      end
      repeat (12) @(posedge ck);
    end
    command(3'b010, 0, 16'h0400);
    repeat (12) @(posedge ck);
    command(3'b001, 0, 16'h0000);
    repeat (160) @(posedge ck);
    $display("DDR_READBACK bursts=%0d beats=%0d checksum=%08h", count, count*8, sum);
    $display("TEST PASSED");
    $finish;
  end
  initial begin
    #2000000000;
    $fatal(1, "DDR watchdog");
  end
endmodule
