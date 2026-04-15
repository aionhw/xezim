// EXPECT: compile_fail
module neg02_undeclared_identifier;
  initial begin
    missing_signal = 1'b1;
  end
endmodule
