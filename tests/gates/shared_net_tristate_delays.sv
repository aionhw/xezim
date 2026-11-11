`timescale 1ns/1ns
module top;
  logic a, b, en1, en2;
  wire bus, bus2, mix, mixd, mos, inv;
  bufif1 #2 g1(bus,a,en1);        bufif1 #2 g2(bus,b,en2);
  bufif1 #(1,5) g3(bus2,a,en1);   bufif1 #(3,2) g4(bus2,b,en2);
  bufif1 #2 g5(mix,a,en1);        assign mix = en2 ? b : 1'bz;
  bufif1 #(1,5) g6(mixd,a,en1);   assign #(1,1) mixd = b;
  nmos #(1,5) g7(mos,a,en1);      assign #(1,1) mos = b;
  notif0 #(2,3) g8(inv,a,en2);    assign #(1,1) inv = b;
  initial begin
    $monitor("%0t a=%b b=%b en1=%b en2=%b | bus=%b bus2=%b mix=%b mixd=%b mos=%b inv=%b",
             $time,a,b,en1,en2,bus,bus2,mix,mixd,mos,inv);
    #1 a=1; b=0; en1=1; en2=0;
    #20 a=0;
    #20 en2=1;
    #20 a=1;
    #20 en1=0;
    #20 b=1;
    #20 en2=0;
    #20 en1=1; en2=1;
    #20 b=0;
    #20 $finish;
  end
endmodule
