// IEEE 1800-2023 §10.11: "The continuous assign statement is a unidirectional assignment"; §§10.3.2,6.6.3 leave the driven source aliases at 1; §14.16 supplies the clocking driver.
// Concatenated, hierarchical and clocking writes to an ordinary copied net cannot backdrive the source alias; omit wrong downstream fields.
`timescale 1ns/1ps
module top;
 wire high=1;logic drive;logic clk=0;
 wor a,b;alias a=b;assign b=high;wor y;wire spare;assign y=a;
 assign {y,spare}={drive,1'b0};
 wor c,d;alias c=d;assign d=high;wor z;assign z=c;assign top.z=drive;
 wor e,f;alias e=f;assign f=high;wor w;assign w=e;
 clocking cb @(posedge clk);output w;endclocking
 initial begin
  #1 drive=1;clk=1;cb.w<=1;
  #1 $display("T|one %b%b%b",b,d,f);drive=0;clk=0;
  #1 clk=1;cb.w<=0;
  #1 $display("T|zero %b%b%b",b,d,f);drive=1;
  #1 $display("T|again %b%b%b",b,d,f);$finish;
 end
endmodule
