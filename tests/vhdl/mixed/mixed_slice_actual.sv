// The SV side of mixed_slice_actual.vhd: a 2 x 8 bit packed input.
`timescale 1ns/1ps
module sink(input logic [15:0] d);
  initial begin
    #1 $display("d=%h", d);
    #2 $display("d=%h", d);
  end
endmodule
