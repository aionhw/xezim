module t;
  logic clk = 0;
  always #5 clk = ~clk;
  logic [7:0] a [7:0], b [5:0], r [5:0], s [7:0];
  int ia [0:9], ib [0:3], ic [0:3];
  int i = 1, j = 6, n = 0;
  always @(posedge clk) begin
    a[j-:6] <= b;
    r <= a[i+:6];
  end
  always_comb begin
    s[7:2] = b;
    s[1:0] = a[1:0];
  end
  always @(posedge clk) begin
    ic = ia[n+:4];
    ia[2+:4] <= ib;
  end
  initial begin
    for (int k = 0; k < 8; k++) a[k] = 8'h10 + 8'(k);
    for (int k = 0; k < 6; k++) b[k] = 8'hA0 + 8'(k);
    for (int k = 0; k < 10; k++) ia[k] = 100 + k;
    for (int k = 0; k < 4; k++) ib[k] = -k;
    @(posedge clk); #1;
    $display("T|ff a=%h %h %h %h %h %h %h %h", a[7], a[6], a[5], a[4], a[3], a[2], a[1], a[0]);
    $display("T|ff r=%h %h %h %h %h %h", r[5], r[4], r[3], r[2], r[1], r[0]);
    $display("T|comb s=%h %h %h %h %h %h %h %h", s[7], s[6], s[5], s[4], s[3], s[2], s[1], s[0]);
    $display("T|int ic=%0d %0d %0d %0d ia=%0d %0d %0d %0d %0d %0d", ic[0], ic[1], ic[2], ic[3], ia[1], ia[2], ia[3], ia[4], ia[5], ia[6]);
    i = 2; n = 3; b[5] = 8'hFF;
    @(posedge clk); #1;
    $display("T|ff2 r=%h %h %h %h %h %h s7=%h", r[5], r[4], r[3], r[2], r[1], r[0], s[7]);
    $display("T|int2 ic=%0d %0d %0d %0d", ic[0], ic[1], ic[2], ic[3]);
    $finish;
  end
endmodule
