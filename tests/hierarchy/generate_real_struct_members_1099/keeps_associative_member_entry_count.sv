// IEEE 1800-2023 §7.9.1: num() returns "the number of entries in the associative array"; two distinct string keys give 2.
// Inside-block associative num() must count entries, not placeholder array metadata. Keep the original queue/dynamic-array activity that exposes the main-correct count; omit those collections from assertions.
`timescale 1ns/1ps
typedef struct {int i;int aa[string];int ai[int];int q[$];int da[];} S;
module top;
 S m;initial begin m.aa["x"]=40;m.aa["y"]=41;#2;$display("T|module-entries %0d",m.aa.num());end
 if(1)begin:g S a;initial begin a.aa["x"]=40;a.aa["y"]=41;a.ai[7]=70;a.q.push_back(1);a.q.push_back(2);a.da=new[3];#1;
 $display("T|entries %0d",a.aa.num());end end
 initial #3 $finish;
endmodule
