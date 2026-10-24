// Combinational saturating 32-bit signed add.
module addsat (
  input  int a,
  input  int b,
  output int y
);
  always_comb begin
    longint s;
    s = longint'(a) + longint'(b);
    if (s > 32'sh7fffffff) y = 32'sh7fffffff;
    else if (s < -64'sd2147483648) y = 32'sh80000000;
    else y = int'(s);
  end
endmodule
