// Instantiates rec_field (mixed_record_field.vhd): its ports are
// std_logic and std_logic_vector, so Questa runs it.
module mixed_record_field;
  logic clk = 0;
  wire [7:0] q;
  wire f;

  rec_field dut (.clk(clk), .q(q), .f(f));

  always #5 clk = ~clk;

  initial begin
    #1;
    $display("init q=%b f=%b", q, f);
    @(posedge clk);
    #1;
    $display("q=%b f=%b", q, f);
    $finish;
  end
endmodule
