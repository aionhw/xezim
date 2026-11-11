// §13.3 automatic task inputs; §11.4.6 case equality checks x/z.
// IEEE 1800-2023 §28.6/§28.16: fall is the second delay;
// §10.3.2/§28.12: the other driver is z. Stimulus starts at t1 (§4.7/§6.8).
`timescale 1ns/1ps
module top;
 logic data, en; wire b1,b0,n1,n0,two,mix,mixb0,mixn1,mixn0;
 bufif1 #(2,5) g1(b1,data,en);
 bufif0 #(2,5) g0(b0,data,!en);
 notif1 #(2,5) h1(n1,!data,en);
 notif0 #(2,5) h0(n0,!data,!en);
 bufif1 #(2,5) g2(two,data,en);
 bufif1 #(2,5) gm(mix,data,en);
 assign #(1,1) mix=1'bz;
 bufif0 #(2,5) gmb0(mixb0,data,!en); assign #(1,1) mixb0=1'bz;
 notif1 #(2,5) gmn1(mixn1,!data,en); assign #(1,1) mixn1=1'bz;
 notif0 #(2,5) gmn0(mixn0,!data,!en); assign #(1,1) mixn0=1'bz;
 task automatic ck(input string label_, input logic got, expected);
 if (got === expected) $display("PASS %s got=%b",label_,got);
 else $display("FAIL %s got=%b expected=%b",label_,got,expected);
 endtask
 initial begin
 #1 data=1; en=1;
 #10 data=0;
 #3 ck("bufif1 fall pending",b1,1); ck("bufif0 fall pending",b0,1); ck("notif1 fall pending",n1,1); ck("notif0 fall pending",n0,1); ck("mixed fall pending",mix,1); ck("mixed bufif0 fall pending",mixb0,1); ck("mixed notif1 fall pending",mixn1,1); ck("mixed notif0 fall pending",mixn0,1);
 #4 ck("bufif1 fallen",b1,0); ck("mixed fallen",mix,0);
 #3 en=0;
 #3 ck("bufif1 off settled",b1,1'bz); ck("bufif0 off settled",b0,1'bz); ck("notif1 off settled",n1,1'bz); ck("notif0 off settled",n0,1'bz); ck("two-delay off",two,1'bz); ck("mixed off settled",mix,1'bz);
 #7 ck("bufif1 off",b1,1'bz); ck("bufif0 off",b0,1'bz); ck("notif1 off",n1,1'bz); ck("notif0 off",n0,1'bz); ck("mixed off",mix,1'bz); $finish;
 end
endmodule
