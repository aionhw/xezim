// An SV counter beside the VHDL one: `svq` samples the VHDL register on the
// clock edge (one cycle behind), the VHDL side samples `svc` the same way.
module mixed_clocked;
  logic clk = 0, rst = 1;
  logic [3:0] svc, svq;
  wire [3:0] q, seen, dec;

  counter dut (.clk(clk), .rst(rst), .sv_q(svc), .q(q), .seen(seen), .dec(dec));

  always #5 clk = ~clk;

  always @(posedge clk or posedge rst)
    if (rst) begin
      svc <= 4'd8;
      svq <= 4'd0;
    end else begin
      svc <= svc + 1;
      svq <= q;
    end

  initial begin
    #12 rst = 0;
    repeat (8) begin
      @(negedge clk);
      $display("%0d q=%0d dec=%0d svq=%0d svc=%0d seen=%0d", $time, q, dec, svq, svc, seen);
    end
    $finish;
  end
endmodule
