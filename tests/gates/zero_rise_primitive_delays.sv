`timescale 1ns/1ns
module top;
  logic a, en;
  wire alone, trs, both, mixed;
  buf #(0,5) g1(alone,a);
  bufif1 #(0,5) g2(trs,a,en);
  buf #(0,5) g3(both,a);  buf g4(both,a);
  buf #(0,5) g5(mixed,a); assign #(1,1) mixed = a;
  initial begin
    $monitor("%0t a=%b en=%b | alone=%b trs=%b both=%b mixed=%b", $time,a,en,alone,trs,both,mixed);
    #1 a=1; en=1;
    #20 a=0;
    #20 a=1;
    #20 en=0;
    #20 a=0;
    #20 en=1;
    #20 $finish;
  end
endmodule
