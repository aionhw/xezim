// Part 1 synthetic: IBUF entry-decoder + pop0_shift_vld extractor.
// Mirrors the cr_ifu_ibuf.v decoder pattern that triggered the E902 stall.
//   - bit-slice sensitivity: always @(... or pop0_shift[5:0] or ...)
//   - case decode based on one-hot pop0_shift selecting which entry's vld
//   - default branch assigns 'bx (X-fill)
//
// The original xezim bug: bit-slice sensitivity dropped → always-block
// only fired once at time 0 with X inputs → pop0_shift_vld stuck at X
// → ibuf_ifctrl_inst_vld stuck at X → CPU stall.
//
// Test: walk pop0_shift through valid one-hot codes with entry vlds
// staged, check pop0_shift_vld + ibuf_ifctrl_inst_vld track correctly.
`timescale 1ns/100ps
module top;
  reg clk = 0;
  always #5 clk = ~clk;

  reg [5:0] pop0_shift = 6'b000000;     // initially 0 (no match → default → X)
  reg entry0_vld = 0, entry1_vld = 0, entry2_vld = 0;
  reg entry3_vld = 0, entry4_vld = 0, entry5_vld = 0;
  reg [15:0] entry0_inst = 0, entry1_inst = 0, entry2_inst = 0;
  reg [15:0] entry3_inst = 0, entry4_inst = 0, entry5_inst = 0;

  reg pop0_shift_vld;
  reg [15:0] pop0_shift_inst;

  // Same pattern as cr_ifu_ibuf.v:925 — bit-slice in sensitivity
  always @( entry1_vld
         or pop0_shift[5:0]
         or entry2_inst[15:0]
         or entry5_inst[15:0]
         or entry5_vld
         or entry4_inst[15:0]
         or entry0_vld
         or entry2_vld
         or entry1_inst[15:0]
         or entry3_vld
         or entry4_vld
         or entry3_inst[15:0]
         or entry0_inst[15:0])
  begin
    case(pop0_shift[5:0])
    6'b0001: begin pop0_shift_vld = entry0_vld; pop0_shift_inst = entry0_inst; end
    6'b0010: begin pop0_shift_vld = entry1_vld; pop0_shift_inst = entry1_inst; end
    6'b0100: begin pop0_shift_vld = entry2_vld; pop0_shift_inst = entry2_inst; end
    6'b1000: begin pop0_shift_vld = entry3_vld; pop0_shift_inst = entry3_inst; end
    6'b10000: begin pop0_shift_vld = entry4_vld; pop0_shift_inst = entry4_inst; end
    6'b100000: begin pop0_shift_vld = entry5_vld; pop0_shift_inst = entry5_inst; end
    default: begin pop0_shift_vld = 1'bx; pop0_shift_inst = 16'bx; end
    endcase
  end

  reg [31:0] tcyc = 0;
  always @(posedge clk) begin
    tcyc <= tcyc + 1;
    $display("CYC %0d shift=%b vld=%b inst=%h", tcyc, pop0_shift, pop0_shift_vld, pop0_shift_inst);
  end

  initial begin
    // Stage 0: pop0_shift=0 (default → X)
    #11;
    // Stage 1: pop0_shift=000010, entry1_vld=0
    pop0_shift = 6'b000010;
    entry1_inst = 16'h1234;
    #10;
    // Stage 2: entry1_vld=1
    entry1_vld = 1;
    #10;
    // Stage 3: pop0_shift=000100, entry2_vld=1, inst=ABCD
    pop0_shift = 6'b000100;
    entry2_vld = 1;
    entry2_inst = 16'hABCD;
    #10;
    // Stage 4: pop0_shift=000001 (entry0_vld stays 0)
    pop0_shift = 6'b000001;
    #10;
    // Stage 5: rotate back to 010 (entry1_vld=1, inst=1234)
    pop0_shift = 6'b000010;
    #10;
    // Stage 6: invalid one-hot 6'b110000 → default → X
    pop0_shift = 6'b110000;
    #10;
    $finish;
  end
endmodule
