// Run-time slices of unpacked arrays (IEEE 1800-2017 7.4.6, 7.6, 10.3,
// 10.4.2, 13.5, 23.3.3). Every line prints the target, left element first.
module child (input logic [7:0] di [5:0], output logic [7:0] dout [5:0]);
  for (genvar k = 0; k < 6; k++) begin : g
    assign dout[k] = di[k] + 8'h01;
  end
endmodule

module t;
  localparam int W = 6;
  logic [7:0] a [7:0];         // descending, a[7] leftmost
  logic [7:0] b [5:0];
  logic [7:0] d [0:7];         // ascending, d[0] leftmost
  logic [7:0] e6 [0:5];
  logic [7:0] w [63:0];
  logic [7:0] m [1:0][5:0];    // two rows of six
  logic [7:0] c [3:0][5:0];
  logic [7:0] c2 [1:0][5:0];
  logic [7:0] x3 [2:0];
  logic [7:0] cw [5:0], cv [5:0], ct [7:0];
  logic [7:0] pw [15:0], pres [39:0], po [5:0], pv [5:0];
  int i = 1, j = 7, k = 10;

  assign cw = w[58+:W];          // constant base
  assign cv = w[k+:W];           // variable base, follows k
  assign ct[6:1] = b;            // a slice as the target

  child u0 (.di(pw[10+:W]), .dout(po));
  child u1 (.di(pw[5:0]), .dout(pres[30+:W]));
  child u2 (.di(pw[k-:W]), .dout(pv));

  function automatic logic [7:0] leftmost(input logic [7:0] x [5:0]);
    return x[5];
  endfunction
  function automatic logic [7:0] rightmost(input logic [7:0] x [5:0]);
    return x[0];
  endfunction
  task automatic fill(output logic [7:0] x [5:0]);
    for (int n = 0; n < 6; n++) x[n] = 8'hE0 + 8'(n);
  endtask

  function automatic string s6(input logic [7:0] x [5:0]);
    return $sformatf("%h %h %h %h %h %h", x[5], x[4], x[3], x[2], x[1], x[0]);
  endfunction
  function automatic string s8(input logic [7:0] x [7:0]);
    return $sformatf("%h %h %h %h %h %h %h %h", x[7], x[6], x[5], x[4], x[3], x[2], x[1], x[0]);
  endfunction
  function automatic string d8(input logic [7:0] x [0:7]);
    return $sformatf("%h %h %h %h %h %h %h %h", x[0], x[1], x[2], x[3], x[4], x[5], x[6], x[7]);
  endfunction
  function automatic string e6s(input logic [7:0] x [0:5]);
    return $sformatf("%h %h %h %h %h %h", x[0], x[1], x[2], x[3], x[4], x[5]);
  endfunction

  task automatic init();
    for (int n = 0; n < 8; n++)  a[n] = 8'h10 + 8'(n);
    for (int n = 0; n < 6; n++)  b[n] = 8'hA0 + 8'(n);
    for (int n = 0; n < 8; n++)  d[n] = 8'h30 + 8'(n);
    for (int n = 0; n < 6; n++)  e6[n] = 8'h50 + 8'(n);
    for (int n = 0; n < 64; n++) w[n] = 8'(n);
    for (int r = 0; r < 2; r++) for (int n = 0; n < 6; n++) m[r][n] = 8'hC0 + 8'(6 * r + n);
    for (int r = 0; r < 4; r++) for (int n = 0; n < 6; n++) c[r][n] = 8'h60 + 8'(6 * r + n);
    for (int r = 0; r < 2; r++) for (int n = 0; n < 6; n++) c2[r][n] = 8'h90 + 8'(6 * r + n);
    for (int n = 0; n < 3; n++)  x3[n] = 8'h00;
    for (int n = 0; n < 16; n++) pw[n] = 8'h70 + 8'(n);
  endtask

  initial begin
    init(); #1;
    b = a[7-:W];        $display("T|01 b=a[7-:W]      %s", s6(b));
    init(); b = a[7:2]; $display("T|02 b=a[7:2]       %s", s6(b));
    init(); b = a[i+:W]; $display("T|03 b=a[i+:W]      %s", s6(b));
    init(); b = a[j-:W]; $display("T|04 b=a[j-:W]      %s", s6(b));
    init(); b = w[63-:W]; $display("T|05 b=w[63-:W]     %s", s6(b));
    init(); a[0+:W] = b; $display("T|06 a[0+:W]=b      %s", s8(a));
    init(); a[7:2] = b;  $display("T|07 a[7:2]=b       %s", s8(a));
    init(); a[j-:W] = b; $display("T|08 a[j-:W]=b      %s", s8(a));
    init(); e6 = d[2+:6]; $display("T|09 e6=d[2+:6]     %s", e6s(e6));
    init(); e6 = d[2:7];  $display("T|10 e6=d[2:7]      %s", e6s(e6));
    init(); d[1+:6] = e6; $display("T|11 d[1+:6]=e6     %s", d8(d));
    init(); b = d[1+:6];  $display("T|12 b=d[1+:6]      %s", s6(b));
    init(); a[0+:6] = d[2+:6]; $display("T|13 a[0+:6]=d[2+:6] %s", s8(a));
    init(); a[2+:6] = a[0+:6]; $display("T|14 a[2+:6]=a[0+:6] %s", s8(a));
    init(); m[1] = w[58+:W]; $display("T|15 m[1]=w[58+:W]  %s", s6(m[1]));
    init(); c[0+:2] = c2; $display("T|16 c[0+:2]=c2     %s / %s", s6(c[1]), s6(c[0]));
    init(); m[1][0+:3] = x3; $display("T|17 m[1][0+:3]=x3  %s", s6(m[1]));
    init(); x3 = m[1][1+:3]; $display("T|18 x3=m[1][1+:3]  %h %h %h", x3[2], x3[1], x3[0]);
    init(); a[0+:W] <= b; #1 $display("T|19 a[0+:W]<=b     %s", s8(a));
    init(); b <= a[7-:W]; #1 $display("T|20 b<=a[7-:W]     %s", s6(b));
    init(); m[0] <= w[i+:W]; #1 $display("T|21 m[0]<=w[i+:W]  %s", s6(m[0]));
    init(); $display("T|22 leftmost(w[58+:W]) %h rightmost(a[j-:W]) %h", leftmost(w[58+:W]), rightmost(a[j-:W]));
    init(); fill(a[0+:W]); $display("T|23 fill(a[0+:W])  %s", s8(a));
    init(); fill(d[2:7]); $display("T|24 fill(d[2:7])   %s", d8(d));
    $display("T|25 size %0d %0d %0d", $size(a[0+:W]), $size(w[63-:W]), $size(a[7:2]));
    init(); #1 $display("T|26 cw=w[58+:W]     %s", s6(cw));
    $display("T|27 cv=w[k+:W]      %s", s6(cv));
    k = 20; #1 $display("T|28 cv k=20         %s", s6(cv));
    $display("T|29 ct[6:1]=b       %s", s8(ct));
    $display("T|30 ports po=%s pres=%h %h pv=%s", s6(po), pres[35], pres[30], s6(pv));
    k = 12; #1 $display("T|31 port k=12 pv=%s", s6(pv));
    $finish;
  end
endmodule
