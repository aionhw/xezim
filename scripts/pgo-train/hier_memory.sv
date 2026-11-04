// Bounded synthetic SRAM model, including bank decoding, synchronous
// read-before-write behavior and direct hierarchical array access.
// IEEE 1800 §§4.6, 7.4.1, 23.6: NBA ordering, unpacked arrays and hierarchy.
// MEMORY_FLAT selects the same model without its eight wrapper levels.
// This is a memory-model workload, not a DRAM protocol implementation.
`timescale 1ns/1ps
`define MEMORY_PORTS input logic clk, cs, we, input logic [3:0] bank_en, input logic [9:0] addr, input logic [63:0] wdata, output logic [63:0] q
`define MEMORY_ARGS .clk(clk), .cs(cs), .we(we), .bank_en(bank_en), .addr(addr), .wdata(wdata), .q(q)

module modeled_sram(`MEMORY_PORTS);
  logic [63:0] mem [0:1023];
  logic [3:0] decoded;
  logic selected;
  initial q = 0;
  always_comb begin
    decoded = (4'b0001 << addr[9:8]) & bank_en;
    selected = cs && (|decoded);
  end
  always @(posedge clk) begin
    if (selected) begin
      if (we) mem[addr] <= wdata;
      q <= mem[addr];
    end
  end
endmodule

module memory_depth_1(`MEMORY_PORTS); modeled_sram u(`MEMORY_ARGS); endmodule
module memory_depth_2(`MEMORY_PORTS); memory_depth_1 u(`MEMORY_ARGS); endmodule
module memory_depth_3(`MEMORY_PORTS); memory_depth_2 u(`MEMORY_ARGS); endmodule
module memory_depth_4(`MEMORY_PORTS); memory_depth_3 u(`MEMORY_ARGS); endmodule
module memory_depth_5(`MEMORY_PORTS); memory_depth_4 u(`MEMORY_ARGS); endmodule
module memory_depth_6(`MEMORY_PORTS); memory_depth_5 u(`MEMORY_ARGS); endmodule
module memory_depth_7(`MEMORY_PORTS); memory_depth_6 u(`MEMORY_ARGS); endmodule
module memory_depth_8(`MEMORY_PORTS); memory_depth_7 u(`MEMORY_ARGS); endmodule

`ifdef MEMORY_FLAT
  `define MEMORY_CELLS dut.mem
`else
  `define MEMORY_CELLS dut.u.u.u.u.u.u.u.u.mem
`endif

module tb;
`ifdef MEMORY_SMALL
  localparam int CYCLES = 200;
`elsif MEMORY_LARGE
  localparam int CYCLES = 200000;
`else
  localparam int CYCLES = 20000;
`endif
  logic clk = 0, cs = 0, we = 0;
  logic [3:0] bank_en = 0;
  logic [9:0] addr = 0;
  logic [63:0] wdata = 0, q;
  logic [63:0] reference [0:1023];
  logic [63:0] expected = 0, checksum = 0;
  logic [31:0] state = 32'h1a2b3c4d;
`ifdef MEMORY_FLAT
  modeled_sram dut(`MEMORY_ARGS);
`else
  memory_depth_8 dut(`MEMORY_ARGS);
`endif
  always #1 clk = ~clk;

  initial begin
    for (int i = 0; i < 1024; i++) begin
      reference[i] = 64'h9e3779b97f4a7c15 ^ (64'(i) * 64'h0000000100000001);
      `MEMORY_CELLS[i] = reference[i];
    end
    for (int cycle = 0; cycle < CYCLES; cycle++) begin
      @(negedge clk);
      state = {state[30:0], state[31] ^ state[21] ^ state[1] ^ state[0]};
      addr = state[9:0];
      cs = (cycle % 7) != 0;
      we = (cycle % 3) == 0;
      bank_en = ~(4'b0001 << (cycle % 4));
      wdata = {state, state ^ (32'h9e3779b9 * (cycle + 1))};
      if (cs && bank_en[addr[9:8]]) begin
        expected = reference[addr];
        if (we) reference[addr] = wdata;
      end
      @(posedge clk);
      #0.1;
      if (q !== expected)
        $fatal(1, "FAIL hier_memory read cycle=%0d expected=%h actual=%h", cycle, expected, q);
      if (`MEMORY_CELLS[addr] !== reference[addr])
        $fatal(1, "FAIL hier_memory hierarchical readback cycle=%0d", cycle);
      checksum = {checksum[56:0], checksum[63:57]} ^ q ^ {32'b0, state};
    end
    for (int i = 0; i < 1024; i++) begin
      if (`MEMORY_CELLS[i] !== reference[i])
        $fatal(1, "FAIL hier_memory final contents index=%0d", i);
      checksum = checksum ^ (`MEMORY_CELLS[i] + 64'(i));
    end
    $display("BENCH hier_memory PASS");
    $display("CHECKSUM hier_memory %016h", checksum);
    $display("WORK hier_memory %0d clocked_accesses", CYCLES);
    $finish;
  end
endmodule

`undef MEMORY_CELLS
`undef MEMORY_ARGS
`undef MEMORY_PORTS
