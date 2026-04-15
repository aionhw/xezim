// EXPECT: compile_fail
module neg12_duplicate_case_item(input logic [1:0] sel, output logic y);
  always_comb begin
    unique case (sel)
      2'b00: y = 1'b0;
      2'b00: y = 1'b1;
      default: y = 1'b0;
    endcase
  end
endmodule
