module array_cond_operands;
  typedef int arr_t [0:2];
  arr_t u, v, w;
  int m [2][2];
  int a2 [2][2];
  int b2 [2][2];
  bit c;
  initial begin
    u = '{1, 2, 3};
    v = '{4, 5, 6};
    c = 1;
    w = c ? u : v;
    $display("T|then %0d %0d %0d", w[0], w[1], w[2]);
    c = 0;
    w = c ? u : v;
    $display("T|else %0d %0d %0d", w[0], w[1], w[2]);
    w = c ? u : (!c ? v : u);
    $display("T|nested %0d %0d %0d", w[0], w[1], w[2]);
    a2 = '{'{1, 2}, '{3, 4}};
    b2 = '{'{5, 6}, '{7, 8}};
    m = !c ? b2 : a2;
    $display("T|2d %0d %0d %0d %0d", m[0][0], m[0][1], m[1][0], m[1][1]);
  end
endmodule
