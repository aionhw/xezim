// EXPECT: compile_fail
module neg04_nonconstant_generate_if(input logic sel);
  generate
    if (sel) begin : g1
    end
  endgenerate
endmodule
