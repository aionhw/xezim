int drv = 3;
wire [7:0] n_init = drv + 1;
wire [7:0] n_ca;
wire n_two;
tri  n_tri;
supply1 n_vdd;
supply0 n_gnd;
wand n_and;
module d1; assign n_ca = 8'd5; assign n_two = 1'b1; assign n_and = 1'b1; assign n_tri = 1'bz; endmodule
module d2(input logic en); assign n_two = en ? 1'b0 : 1'bz; assign n_and = en ? 1'b0 : 1'b1; assign n_tri = en; endmodule
module rd #(parameter int ID=0)(input wire [7:0] p);
  always @(n_two) $display("T|rd%0d n_two=%b at %0t", ID, n_two, $time);
  initial begin #1; $display("T|rd%0d %0d %0d %b %b %b %b %b p=%0d", ID, n_init, n_ca, n_two, n_tri, n_vdd, n_gnd, n_and, p); end
endmodule
module top;
  logic en = 0;
  d1 a(); d2 b(en); rd #(1) r1(n_init); rd #(2) r2(n_ca);
  initial begin
    #2 en = 1; drv = 9;
    #1 $display("T|top %0d %0d %b %b %b", n_init, n_ca, n_two, n_tri, n_and);
    $finish;
  end
endmodule
