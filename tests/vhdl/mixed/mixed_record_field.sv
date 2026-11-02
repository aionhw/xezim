// Instantiates rec_field. The golden is xezim's result for this bug, not
// a Questa record-port connection (see mixed_record).
module mixed_record_field;
  logic clk = 0;
  wire [7:0] q;

  rec_field dut (.clk(clk), .q(q));

  always #5 clk = ~clk;

  initial begin
    #1;
    @(posedge clk);
    #1;
    $display("q=%b", q);
    if (q === 8'b10100101)
      $display("PASS");
    else
      $display("FAIL");
    $finish;
  end
endmodule
