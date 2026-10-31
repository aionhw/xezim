`timescale 1ns/1ps
// Synthetic TileLink-UL subset endpoint; NOT a coherent TileLink-C implementation.
// One request beat and one response beat. Unique source while outstanding.
module tlul_mem_endpoint #(
  parameter integer WORDS = 256,
  parameter integer SOURCES = 16
) (
  input wire clk, input wire rst_n,
  input wire request_throttle,
  input wire a_valid, output wire a_ready,
  input wire [2:0] a_opcode, input wire [2:0] a_size,
  input wire [3:0] a_source, input wire [15:0] a_address,
  input wire [3:0] a_mask, input wire [31:0] a_data,
  output reg d_valid, input wire d_ready,
  output reg [2:0] d_opcode, output reg [2:0] d_size,
  output reg [3:0] d_source, output reg [31:0] d_data,
  output reg d_error
);
  reg [31:0] memory [0:WORDS-1];
  reg pending [0:SOURCES-1];
  reg [2:0] response_opcode [0:SOURCES-1];
  reg [2:0] response_size [0:SOURCES-1];
  reg [31:0] response_data [0:SOURCES-1];
  reg [7:0] countdown [0:SOURCES-1];
  reg [3:0] next_pick;
  integer cycle_count;
  integer i, j, k, select_idx;
  integer word_index;
  reg [31:0] write_word;

  // Backpressure is a continuous-net input so external force/release changes
  // behavior without modifying the functional output or reference checker.
  assign a_ready = rst_n && !request_throttle && !pending[a_source] &&
                   ((cycle_count % 11) != 8) && ((cycle_count % 11) != 9);

  always @(posedge clk) begin
    if (!rst_n) begin
      cycle_count <= 0;
      next_pick <= 0;
      d_valid <= 0;
      d_opcode <= 0;
      d_size <= 0;
      d_source <= 0;
      d_data <= 0;
      d_error <= 0;
      for (i=0; i<WORDS; i=i+1) memory[i] <= 32'd0;
      for (i=0; i<SOURCES; i=i+1) begin
        pending[i] <= 0;
        response_opcode[i] <= 0;
        response_size[i] <= 0;
        response_data[i] <= 0;
        countdown[i] <= 0;
      end
    end else begin
      cycle_count <= cycle_count + 1;
      for (i=0; i<SOURCES; i=i+1)
        if (pending[i] && countdown[i] != 0)
          countdown[i] <= countdown[i] - 1;

      if (a_valid && a_ready) begin
        word_index = a_address >> 2;
        pending[a_source] <= 1;
        response_opcode[a_source] <= (a_opcode == 3'd4) ? 3'd1 : 3'd0;
        response_size[a_source] <= a_size;
        // Different banks and source IDs complete at different times.
        countdown[a_source] <= 8'd12 + ((a_address[5:2] * 7 + a_source * 3) % 23);
        if (a_opcode == 3'd4) begin
          response_data[a_source] <= memory[word_index];
        end else begin
          write_word = memory[word_index];
          if (a_mask[0]) write_word[7:0]   = a_data[7:0];
          if (a_mask[1]) write_word[15:8]  = a_data[15:8];
          if (a_mask[2]) write_word[23:16] = a_data[23:16];
          if (a_mask[3]) write_word[31:24] = a_data[31:24];
          memory[word_index] <= write_word;
          response_data[a_source] <= 0;
        end
      end

      if (d_valid && d_ready) begin
        pending[d_source] <= 0;
        d_valid <= 0;
      end

      // Stable D payload under backpressure. Rotate among ready sources,
      // permitting completion reordering across distinct source IDs.
      if (!d_valid) begin
        select_idx = -1;
        for (j=0; j<SOURCES; j=j+1) begin
          k = (next_pick + j) % SOURCES;
          if (select_idx < 0 && pending[k] && countdown[k] == 0)
            select_idx = k;
        end
        if (select_idx >= 0) begin
          d_valid <= 1;
          d_opcode <= response_opcode[select_idx];
          d_size <= response_size[select_idx];
          d_source <= select_idx;
          d_data <= response_data[select_idx];
          d_error <= 0;
          next_pick <= (select_idx + 1) % SOURCES;
        end
      end
    end
  end
endmodule
