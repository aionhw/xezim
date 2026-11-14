// IEEE 1800-2023 §28.3.2 assigns output drive strength; §28.6 enables bufif1; §28.12.1: "the stronger signal shall dominate all the weaker drivers"; §21.2.1.4 scalar %v.
// Explicitly weak primitive drivers on wired nets must not acquire strong strength.
`timescale 1ns/1ps
module top;
 wand a;wor o;
 bufif1 (weak0,strong1) ga(a,1'b0,1'b1),go(o,1'b0,1'b1);
 assign (pull0,pull1) a=1'b0;assign (pull0,pull1) o=1'b0;
 initial begin #1;$display("T|gates %b%b %v %v",a,o,a,o);$finish;end
endmodule
