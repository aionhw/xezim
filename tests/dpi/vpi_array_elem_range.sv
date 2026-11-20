// Unpacked-array ports of the top module, driven only by the VPI library
// tests/dpi/vpi_array_elem_range.c (see tests/strings/vpi_array_elem_range.rs):
// the shape of the I3C core's `priv_ids_i`, a descending array of ascending
// vectors, and a two-dimensional array.
module top (
  input  logic [31:0] ids [4],
  input  logic [0:7]  down [3:0],
  input  logic [15:8] grid [0:1][2:0],
  output logic [31:0] sum
);
  always_comb begin
    sum = '0;
    for (int i = 0; i < 4; i++) sum += ids[i];
  end
  initial #10 $finish;
endmodule
