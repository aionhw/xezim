// The packed layout of mixed_record.vhd (mixed_record.lrm): left index and
// first field are the most significant bits, one bit per std_logic. The
// output array is printed element by element in that order.
module mixed_record;
  logic clk = 0;
  logic [9:0] d;
  wire [9:0] q;
  wire [7:0] da, ae;
  wire db, dc, af;
  logic [79:0] ai;
  wire [79:0] a;

  rec_io dut (.clk(clk), .d(d), .q(q), .da(da), .db(db), .dc(dc),
              .ai(ai), .ae(ae), .af(af), .a(a));

  // d: a = D2, b = 1, c = 0. ai: element k (in layout order) has
  // a = 8'h10 + k * 8'h11, b = 0, and c = 1 only for k = 4.
  assign d = {8'hD2, 1'b1, 1'b0};
  always_comb
    for (int k = 0; k < 8; k++)
      ai[79 - 10 * k -: 10] = {8'h10 + 8'(k * 8'h11), 1'b0, k == 4};
  always #5 clk = ~clk;

  initial begin
    #1;
    $display("q=%b da=%h db=%b dc=%b", q, da, db, dc);
    $display("ae=%h af=%b", ae, af);
    @(posedge clk);
    #1;
    for (int k = 0; k < 8; k++)
      $display("a[%0d] a=%h b=%b c=%b", k, a[79 - 10 * k -: 8], a[71 - 10 * k], a[70 - 10 * k]);
    $finish;
  end
endmodule
