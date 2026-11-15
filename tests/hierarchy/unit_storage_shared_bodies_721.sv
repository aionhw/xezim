int base = 40;
int unused_var = 7;
wire [7:0] derived = base + 2;
wire [7:0] unused_net = 8'd9;
function automatic int bump(int n); return n + base; endfunction
class reader; function int get(); return base + 100; endfunction endclass
module via_net(output int y); assign y = derived; endmodule
module via_func(output int y); assign y = bump(1); endmodule
module via_class(output int y); initial begin reader r = new; #0 y = r.get(); end endmodule
module shadow(output int y, output int z); int base = 5; assign y = base; assign z = $unit::base; endmodule
module lane #(parameter int IDX = 0)(input logic [7:0] src, output logic [7:0] y, output logic [7:0] z);
  wire [7:0] t = src + IDX;
  assign y = t ^ 8'h0f;
  if (IDX % 2 == 0) begin : even assign z = 8'd10 + IDX; end
  else begin : odd assign z = 8'd100 + IDX; end
endmodule
module top;
  int a, b, c, d, e;
  logic [7:0] src = 8'd1;
  logic [7:0] y [4], z [4];
  via_net u1(a);
  via_func u2(b);
  via_class u3(c);
  shadow u4(d, e);
  for (genvar i = 0; i < 4; i++) begin : g
    lane #(.IDX(i)) l(src, y[i], z[i]);
  end
  initial begin
    #1 $display("T|%0d|%0d|%0d|%0d|%0d|%0d", a, b, c, d, e, unused_var + unused_net);
    $display("T|%0d %0d %0d %0d|%0d %0d %0d %0d", y[0], y[1], y[2], y[3], z[0], z[1], z[2], z[3]);
    src = 8'd20;
    #1 $display("T|%0d %0d %0d %0d|%0d", y[0], y[1], y[2], y[3], g[3].l.t);
    $finish;
  end
endmodule
