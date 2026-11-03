// Instantiates rec_sub (mixed_record_subtype.vhd).
module mixed_record_subtype;
  logic clk = 0;
  wire [7:0] q;

  rec_sub dut (.clk(clk), .q(q));

  always #5 clk = ~clk;

  initial begin
    #1;
    $display("init q=%b", q);
    @(posedge clk);
    #1;
    $display("q=%b", q);
    @(posedge clk);
    #1;
    $display("q=%b", q);
    $finish;
  end
endmodule
