// IEEE 1800-2023 §6.16 string stores the assigned characters; §6.16: "The string data type is an ordered collection of characters"; §21.2.1.7 formats the string characters without packed-vector padding. Expected string lengths are 5 and 3.
// Registering a generated string member must not introduce leading spaces in its hierarchical %s or $sformatf.
`timescale 1ns/1ps
typedef struct {int i;string s;} S;
module top;
 if(1)begin:g S a;initial begin a.s="hello";#2;a.s="bye";end end
 initial begin #1;$display("T|string [%s] [%s] %0d",g.a.s,$sformatf("%s",g.a.s),g.a.s.len());
 #2;$display("T|string [%s] [%s] %0d",g.a.s,$sformatf("%s",g.a.s),g.a.s.len());$finish;end
endmodule
