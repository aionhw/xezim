// IEEE 1800-2023 §6.6.5 tri1 has "a continuous 1 value of pull strength driving it"; §§28.3.2,28.10 set weak primitive outputs; §28.12.1 stronger pull 1 dominates weak 0.
// Undelayed primitive strengths on structure members must preserve pull strength in module, child and generated scopes.
`timescale 1ns/1ps
typedef struct {logic value;} S;
module child;tri1 S a;pulldown (weak0) pd(a.value);initial #2 $display("T|child %b",a.value);endmodule
module top;
 tri1 S a;pulldown (weak0) pd(a.value);tri1 S b;buf (weak1,weak0) g(b.value,1'b0);
 child c();
 if(1)begin:q tri1 S a;pulldown (weak0) pd(a.value);initial #2 $display("T|generated %b",a.value);end
 initial begin #3;$display("T|primitive %b %b",a.value,b.value);$finish;end
endmodule
