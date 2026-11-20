// §16.12.9: one successful match suffices after another candidate fails.
module top;
  initial $assertvacuousoff(0);
 bit clk=0,a=1,b=0,c=0;int hits=0,fails=0;
 always #1 clk=~clk;
 p:assert property(@(posedge clk) a |-> ((1'b1 ##[1:2] b) #-# c)) hits++;else fails++;
 q:assert property(@(posedge clk) a |-> ((1'b1 ##[0:1] b) #=# c)) hits++;else fails++;
 initial begin
  #2;a=0;b=1;c=0;
  #2;c=1;
  #3;if(hits==2 && fails==0)$display("PASS followed range neighbors");else $display("FAIL followed range neighbors %0d %0d",hits,fails);
  $finish;
 end
endmodule
