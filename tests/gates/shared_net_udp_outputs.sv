`timescale 1ns/1ns
primitive udp_buf(output y, input a);
  table 0:0; 1:1; endtable
endprimitive
primitive udp_latch(output reg q, input d, input g);
  table
  // d g : q : q+
     0 1 : ? : 0;
     1 1 : ? : 1;
     ? 0 : ? : -;
  endtable
endprimitive
module top;
  logic a, b, g;
  wire plain, slow, pair, two, seq, alone;
  udp_buf k1(plain,a);          assign plain = b;
  udp_buf #2 k2(slow,a);        assign #1 slow = b;
  udp_buf #(1,5) k3(pair,a);    assign #(1,1) pair = b;
  udp_buf #2 k4(two,a);         udp_buf #3 k5(two,b);
  udp_latch k6(seq,a,g);        assign seq = b;
  udp_buf #(1,5) k7(alone,a);
  initial begin
    $monitor("%0t a=%b b=%b g=%b | plain=%b slow=%b pair=%b two=%b seq=%b alone=%b",
             $time,a,b,g,plain,slow,pair,two,seq,alone);
    #1 a=1; b=1; g=1;
    #20 a=0;
    #20 a=1;
    #20 b=0;
    #20 g=0;
    #20 a=0;
    #20 b=1'bz;
    #20 g=1;
    #20 $finish;
  end
endmodule
