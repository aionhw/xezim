// EXPECT: compile_fail
// A wreal net carries a real value; an explicit data type is illegal
// (Verilog-AMS 2.4 §3.8). Silently accepting `wreal logic [3:0]` as a logic
// vector reintroduces the integer-rounding corruption wreal exists to avoid.
module neg14_wreal_explicit_type;
  wreal logic [3:0] p;
  assign p = 1.25;
endmodule
