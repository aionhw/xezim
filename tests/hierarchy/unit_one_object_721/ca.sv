int base = 40;
wire [7:0] un = 8'd3;
module vq(output int y, output logic [7:0] z); int base = 5; wire [7:0] un = 8'd9;
  assign y = $unit::base + base; assign z = $unit::un + un; endmodule
module vb(output int y); assign y = base + un; endmodule
module top; int a, b; logic [7:0] z; vq u1(a, z); vb u2(b);
  initial begin #1 $display("T|%0d %0d %0d", a, z, b); base = 50; #1 $display("T|%0d %0d %0d", a, z, b); $finish; end
endmodule
