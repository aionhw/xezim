module child #(parameter logic [15:0] P=0, Q=0, parameter real R=0);
  localparam int V=1;
endmodule
module top;
  localparam A = (3'd7 * 3'd7) + 8'd0;
  localparam B = ((3'd7 * 3'd7) == 8'd49);
  localparam C = (3'd7 << 1) + 8'd0;
  localparam D = 8'h80 >> (2'b11 + 2'b01);
  localparam E = 8'(3'd7 * 3'd7);
  localparam F = {3'd7 * 3'd7} + 8'd0;
  localparam G = 1'b1 ? 4'shf : 8'd0;
  localparam H = 1'bx ? 8'hf0 : 4'h0;
  localparam logic signed [7:0] J=(4'sb1000 + 4'sbx000) >>> 1;
  localparam int V=9;
  function automatic logic [2:0] narrow(); return 3'd7*3'd7; endfunction
  child u();
  defparam u.P = (4'(V) - 5'd10) + 8'd0;
  defparam u.Q = 1'b1 ? narrow() : 8'd0;
  defparam u.R = 5.0 / 2;
  initial begin
    #1;
    $display("T|context %0d %0d %0d %0d %0d %0d %0d %b", A,B,C,D,E,F,G,H);
    $display("T|unknown %b", J);
    $display("T|override %h %h %f", u.P,u.Q,u.R);
    $finish;
  end
endmodule
