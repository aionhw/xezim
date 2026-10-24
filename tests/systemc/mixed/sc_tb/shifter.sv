// SystemVerilog DUT: an 8-bit shift register with parallel load, used from
// SystemC through the sc_foreign_module declaration in tb.cpp.
module shifter (
  input  logic       clk,
  input  logic       load,
  input  logic [7:0] din,
  input  logic       sin,
  output logic [7:0] q
);
  always_ff @(posedge clk)
    if (load) q <= din;
    else q <= {q[6:0], sin};
endmodule
