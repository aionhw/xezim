// §16.12.9 / Table 16-3: followed-by accepts a right-associated property.
module top;
 bit clk=0,a=0,b=0,c=0;int k=0;int counts[8];
 always #1 clk=~clk;
 function void count(int idx);counts[idx]++;endfunction
 ap: assert property (@(posedge clk) a #-# b |-> c) count(0);else count(1);
 aq: assert property (@(posedge clk) not(a |-> not(b |-> c))) count(2);else count(3);
 ar: assert property (@(posedge clk) (a ##1 b) #=# c) count(4);else count(5);
 as_: assert property (@(posedge clk) not((a ##1 b) |=> not c)) count(6);else count(7);
 initial begin
  for(int j=0;j<20;j++)begin a=j%2;b=j%3;c=j%4;@(posedge clk);@(negedge clk);end
  #1;
  if(counts[0]==10 && counts[1]==10 && counts[2]==10 && counts[3]==10 && counts[4]==3 && counts[5]==16 && counts[6]==3 && counts[7]==16) $display("PASS followed neighbors");
  else $display("FAIL followed neighbors %p",counts);
  $finish;
 end
endmodule
