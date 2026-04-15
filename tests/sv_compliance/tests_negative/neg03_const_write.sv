// EXPECT: compile_fail
module neg03_const_write;
  const int x = 1;
  initial begin
    x = 2;
  end
endmodule
