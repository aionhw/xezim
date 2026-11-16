// IEEE 1800-2023 §10.11 cumulative identity; §6.6.6 supply0/1; §6.6.4: trireg "shall store a value" after its driver floats.
// Forward-chain aliases, intrinsic supplies and driven trireg charge are main-correct limits; no trireg force or supply release.
`timescale 1ns/1ps
module top;
 wire d,e,f;alias d=e;alias e=f;assign d=1'b1;
 supply0 l,m;alias l=m;supply1 h,j;alias h=j;
 logic source;trireg a,b;alias a=b;assign a=source;
 initial begin source=1;#1;source=1'bz;#1;
 $display("T|stable %b%b%b %b%b %b%b %b",d,e,f,l,m,h,j,b);$finish;end
endmodule
