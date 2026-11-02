// The packed layout of mixed_record.vhd: left index and first field are
// the most significant bits, one bit per std_logic.
module mixed_record;
  logic clk = 0;
  logic [9:0] d;
  wire [9:0] q;
  wire [39:0] a;

  rec_io dut (.clk(clk), .d(d), .q(q), .a(a));

  assign d = 10'b1111000010;
  always #5 clk = ~clk;

  initial begin
    #1;
    $display("q=%b", q);
    @(posedge clk);
    #1;
    $display("a=%b", a);
    if (q === 10'b1111000010 &&
        a === 40'b1010010110001111000100010001110000000000)
      $display("PASS");
    else
      $display("FAIL");
    $finish;
  end
endmodule
