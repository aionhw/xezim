// Exercise read-before-write SRAM in word and bit-lane layouts.
`ifndef MEM_CELLS
`define MEM_CELLS 0
`endif
`ifndef MEM_DEEP
`define MEM_DEEP 0
`endif
module memory_leaf(input clk, we, input [5:0] addr, input [63:0] data,
                   output reg [63:0] q);
  generate if (`MEM_CELLS == 2) begin : bit_lanes
    for (genvar lane = 0; lane < 64; lane = lane + 1) begin : lanes
      reg mem [0:63];
      always @(posedge clk) begin
        if (we) mem[addr] <= data[lane];
        q[lane] <= mem[addr];
      end
    end
  end else if (`MEM_CELLS == 1) begin : bit_cells
    wire [4095:0] state;
    for (genvar row = 0; row < 64; row = row + 1) begin : rows
      for (genvar bitno = 0; bitno < 64; bitno = bitno + 1) begin : bits
        reg stored;
        always @(posedge clk)
          if (we && addr == row) stored <= data[bitno];
        assign state[row*64+bitno] = stored;
      end
    end
    always @(posedge clk) q <= state[addr*64 +: 64];
  end else begin : array_words
    reg [63:0] mem [0:63];
    always @(posedge clk) begin
      if (we) mem[addr] <= data;
      q <= mem[addr];
    end
  end endgenerate
endmodule

module memory_wrap1(input clk,we,input [5:0] addr,input [63:0] data,output [63:0] q);
  memory_leaf u(clk,we,addr,data,q);
endmodule
module memory_wrap2(input clk,we,input [5:0] addr,input [63:0] data,output [63:0] q);
  memory_wrap1 u(clk,we,addr,data,q);
endmodule
module memory_wrap3(input clk,we,input [5:0] addr,input [63:0] data,output [63:0] q);
  memory_wrap2 u(clk,we,addr,data,q);
endmodule
module memory_wrap4(input clk,we,input [5:0] addr,input [63:0] data,output [63:0] q);
  memory_wrap3 u(clk,we,addr,data,q);
endmodule
module memory_wrap5(input clk,we,input [5:0] addr,input [63:0] data,output [63:0] q);
  memory_wrap4 u(clk,we,addr,data,q);
endmodule
module memory_wrap6(input clk,we,input [5:0] addr,input [63:0] data,output [63:0] q);
  memory_wrap5 u(clk,we,addr,data,q);
endmodule
module memory_wrap7(input clk,we,input [5:0] addr,input [63:0] data,output [63:0] q);
  memory_wrap6 u(clk,we,addr,data,q);
endmodule
module memory_wrap8(input clk,we,input [5:0] addr,input [63:0] data,output [63:0] q);
  memory_wrap7 u(clk,we,addr,data,q);
endmodule

module top;
  reg clk=0, we=0;
  reg [5:0] addr=0;
  reg [63:0] data=0, expected;
  wire [63:0] q;
  reg [63:0] oracle [0:63];
  reg [31:0] checksum=0;
  integer i, cycles=20000;
  generate if (`MEM_DEEP) begin : deep
    memory_wrap8 dut(clk,we,addr,data,q);
  end else begin : flat
    memory_leaf dut(clk,we,addr,data,q);
  end endgenerate
  initial begin
    if ($value$plusargs("cycles=%d",cycles)) begin end
    for (i=0; i<cycles+64; i=i+1) begin
      addr=i;
      data={(i*32'hbb67ae85)^(i>>5), (i*32'h9e3779b9)^(i>>6)};
      we=(i<64) || (i%3==0);
      expected=oracle[addr];
      if (we) oracle[addr]=data;
      #1; clk=1;
      #1; clk=0;
      if (i>=64) begin
        if (q !== expected) $fatal(1,"memory mismatch at %d: %h %h",i,q,expected);
        checksum=(checksum*33) ^ q;
      end
    end
    $display("CHECKSUM %h",checksum);
    $display("TEST PASSED");
    $finish;
  end
endmodule
