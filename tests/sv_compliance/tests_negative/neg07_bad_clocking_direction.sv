// EXPECT: compile_fail
module neg07_bad_clocking_direction;
  logic clk;
  logic d;

  clocking cb @(posedge clk);
    input d;
  endclocking

  initial begin
    cb.d <= 1'b1;
  end
endmodule
