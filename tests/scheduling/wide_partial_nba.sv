module tb;
  reg clk=0;
  reg [7:0] a=0, b=0;
  reg [137:0] q=0;
  reg write_second=0;
  always @(posedge clk) begin
    q[67:60] <= a;
    if (write_second) q[71:64] <= b;
  end
  task tick;
    #1 clk=1; #1 $display("NBA %b %b %b",q[72:59],q[119],q[0]); clk=0;
  endtask
  initial begin
    q[119]=1'bz; q[0]=1'bx;
    tick;
    a=8'ha5; tick; tick;
    write_second=1; b=8'hbc; tick;
    a=8'h05; b=0; tick;
    a=8'bz011xx00; b=8'b001xz0z1; tick;
    a=0; b=0; tick; tick;
    $finish;
  end
endmodule
