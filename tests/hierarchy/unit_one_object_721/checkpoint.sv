int g = 5; int q [$]; string s = "a";
function automatic void addg(int by); g += by; q.push_back(g); endfunction
module child; initial forever begin #1; addg(1); s = {s, "x"}; end endmodule
module top; child c(); int g = 100;
  always @($unit::g) g = $unit::g * 2;
  initial begin #10 $display("T|g=%0d ug=%0d q=%p s=%s", g, $unit::g, q, s); $finish; end
endmodule
