// Synthetic banked memory, not a JEDEC DRAM timing model. Each bank has a
// clocked read port and byte-enabled NBA writes. Burst commands write the
// selected row in every bank, stressing several pending writes per clock.
// Reads return the pre-write word on the next rising edge (read before write).
//
// Top: tb. MEMORY_SMALL selects 128 commands/32 rows; default is 2048/256;
// MEMORY_LARGE selects 20000/1024. BANKED_BANKS changes the bank count (8).
// The scoreboard checks every response and finally reads every stored word.
// Set XEZIM_LARGE_ARRAY_NAME_THRESHOLD=0 separately to exercise the packed
// arena with this small model; keep diagnostic settings out of timing runs.

`ifndef BANKED_BANKS
`define BANKED_BANKS 8
`endif

module banked_nba_leaf #(parameter int ROW_W = 8) (
  input logic clk, enable, write_enable,
  input logic [ROW_W-1:0] row,
  input logic [3:0] byte_enable,
  input logic [31:0] write_data,
  output logic [31:0] read_data
);
  logic [31:0] memory [0:(1 << ROW_W)-1];
  initial begin
    read_data = 0;
    for (int i = 0; i < (1 << ROW_W); i++) memory[i] = 0;
  end
  always @(posedge clk) begin
    if (enable) begin
      read_data <= memory[row];
      if (write_enable)
        for (int lane = 0; lane < 4; lane++)
          if (byte_enable[lane])
            memory[row][lane * 8 +: 8] <= write_data[lane * 8 +: 8];
    end
  end
endmodule

module banked_nba_model #(
  parameter int BANKS = 8,
  parameter int ROW_W = 8,
  parameter int BANK_W = BANKS > 1 ? $clog2(BANKS) : 1
) (
  input logic clk, enable, write_enable, burst,
  input logic [BANK_W+ROW_W-1:0] address,
  input logic [3:0] byte_enable,
  input logic [31:0] write_data,
  output wire [31:0] read_data
);
  wire [BANK_W-1:0] bank = address[ROW_W +: BANK_W];
  wire [ROW_W-1:0] row = address[ROW_W-1:0];
  wire [BANKS-1:0][31:0] bank_read;
  for (genvar b = 0; b < BANKS; b++) begin : banks
    banked_nba_leaf #(.ROW_W(ROW_W)) ram (
      .clk(clk), .enable(enable),
      .write_enable(write_enable && (burst || bank == b)),
      .row(row), .byte_enable(byte_enable),
      .write_data(write_data ^ (32'h0101_0101 * b)),
      .read_data(bank_read[b])
    );
  end
  assign read_data = bank_read[bank];
endmodule

module tb;
  localparam int BANKS = `BANKED_BANKS;
  localparam int BANK_W = BANKS > 1 ? $clog2(BANKS) : 1;
`ifdef MEMORY_SMALL
  localparam int ROW_W = 5;
  localparam int COMMANDS = 128;
  localparam logic [63:0] GOLDEN = 64'hf77d_2184_671e_df01;
`elsif MEMORY_LARGE
  localparam int ROW_W = 10;
  localparam int COMMANDS = 20000;
  localparam logic [63:0] GOLDEN = 64'hfbf7_7206_6a8b_73fd;
`else
  localparam int ROW_W = 8;
  localparam int COMMANDS = 2048;
  localparam logic [63:0] GOLDEN = 64'h9225_d1e8_0fa2_baa1;
`endif
  localparam int ROWS = 1 << ROW_W;
  logic clk = 0;
  logic enable = 0, write_enable = 0, burst = 0;
  logic [BANK_W+ROW_W-1:0] address = 0;
  logic [3:0] byte_enable = 0;
  logic [31:0] write_data = 0;
  wire [31:0] read_data;
  logic [31:0] expected_memory [0:BANKS-1][0:ROWS-1];
  logic [31:0] expected, salted;
  logic [63:0] checksum = 0;
  int selected_bank, selected_row, work = 0;

  always #5 clk = ~clk;
  banked_nba_model #(.BANKS(BANKS), .ROW_W(ROW_W)) dut (
    .clk(clk), .enable(enable), .write_enable(write_enable), .burst(burst),
    .address(address), .byte_enable(byte_enable), .write_data(write_data),
    .read_data(read_data)
  );

  initial begin
    if (BANKS < 1) $fatal(1, "BANKED_BANKS must be positive");
    for (int b = 0; b < BANKS; b++)
      for (int r = 0; r < ROWS; r++) expected_memory[b][r] = 0;
    for (int t = 0; t < COMMANDS; t++) begin
      @(negedge clk);
      selected_bank = (t * 5 + t / 11) % BANKS;
      selected_row = (t * 37 + t / 7) & (ROWS - 1);
      enable = 1;
      address = (selected_bank << ROW_W) | selected_row;
      write_enable = (t % 5) != 0;
      burst = (t % 3) == 0;
      byte_enable = (t ^ (t >> 2)) | 1;
      write_data = 32'h1020_3040 + t * 32'h9e37_79b9;
      expected = expected_memory[selected_bank][selected_row];
      @(posedge clk); #1;
      if (read_data !== expected)
        $fatal(1, "banked response command=%0d bank=%0d row=%0d got=%h expected=%h",
               t, selected_bank, selected_row, read_data, expected);
      checksum = (checksum << 7) ^ (checksum >> 57) ^ {32'd0, read_data} ^ work;
      work++;
      if (write_enable)
        for (int b = 0; b < BANKS; b++)
          if (burst || b == selected_bank) begin
            salted = write_data ^ (32'h0101_0101 * b);
            for (int lane = 0; lane < 4; lane++)
              if (byte_enable[lane])
                expected_memory[b][selected_row][lane * 8 +: 8] = salted[lane * 8 +: 8];
          end
    end
    // Verify all banks, including lanes not read by the command stream.
    for (int r = 0; r < ROWS; r++)
      for (int b = 0; b < BANKS; b++) begin
        @(negedge clk);
        write_enable = 0;
        address = (b << ROW_W) | r;
        expected = expected_memory[b][r];
        @(posedge clk); #1;
        if (read_data !== expected)
          $fatal(1, "banked readback bank=%0d row=%0d got=%h expected=%h",
                 b, r, read_data, expected);
        checksum = (checksum << 7) ^ (checksum >> 57) ^ {32'd0, read_data} ^ work;
        work++;
      end
    // Unknown bank/row must propagate X rather than read row or bank zero.
    @(negedge clk); address = 'x;
    @(posedge clk); #1;
    if (read_data !== 32'hxxxx_xxxx) $fatal(1, "unknown bank/row did not read X");
    work++;
    // Independent integer reference calculation supplies the eight-bank
    // goldens. Other bank counts still check every response and stored byte.
    if (BANKS == 8 && checksum !== GOLDEN) $fatal(1, "banked checksum mismatch: %h", checksum);
    $display("BENCH banked_nba_memory PASS");
    $display("CHECKSUM banked_nba_memory %016h", checksum);
    $display("WORK banked_nba_memory %0d transactions", work);
    $finish;
  end
endmodule
