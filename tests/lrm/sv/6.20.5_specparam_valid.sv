module spv();
  specparam delay = 50;
  
  // Valid: specparam used in specify block
  specify
    (a => b) = delay;
    $setup(delay, clk, 10);
  endspecify
  
  // Valid: specparam used in timing check
  always @(posedge clk) begin
    $setuphold(delay, data, clk, 5);
  end
  
  logic a, b, clk, data;
endmodule