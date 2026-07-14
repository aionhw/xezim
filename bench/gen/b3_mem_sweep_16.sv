// B3 mem-sweep: working set = 2^16 x 32b = 256 KiB
module bench_mem_sweep;
  bit clk = 0;
  logic [31:0] mem [65536];
  logic [31:0] lfsr = 32'h1234_5678;
  logic [31:0] acc = 0;
  int cyc = 0;
  always #1 clk = ~clk;
  always_ff @(posedge clk) begin
    // xorshift keeps the address stream unpredictable
    lfsr <= lfsr ^ (lfsr << 13);
    acc  <= acc + mem[lfsr[15:0]];
    mem[(lfsr >> 7) & 65535] <= acc ^ lfsr;
    cyc  <= cyc + 1;
  end
  initial begin
    #(200000);
    $display("BENCH_DONE cycles=%0d checksum=%0d", cyc, acc);
    $finish;
  end
endmodule
