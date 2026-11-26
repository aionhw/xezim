int g = 5;
int h = 6;
module child; genvar i; for (i = 0; i < 1; i++) begin : cg initial begin #1; $display("T|child.cg g=%0d h=%0d", g, h); g++; end end endmodule
module top;
  int g = 77;
  child c();
  genvar i;
  for (i = 0; i < 2; i++) begin : tg
    int h = 10 + i;
    initial begin #2; $display("T|top.tg%0d g=%0d ug=%0d h=%0d uh=%0d", i, g, $unit::g, h, $unit::h); end
  end
  initial begin #3; $display("T|top g=%0d ug=%0d h=%0d", g, $unit::g, h); $finish; end
endmodule
