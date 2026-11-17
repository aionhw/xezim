// IEEE 1800-2023 §10.3.4: unspecified strength "shall default to (strong1, strong0)"; §6.6.6 supply strength; §21.2.1.4 defines scalar %v.
// Storage sharing by an identity-copy optimizer does not copy drive strength.
`timescale 1ns/1ps
module top;
 logic d;wire (weak1,weak0) s=d;wire z;assign z=s;
 supply1 vdd;supply0 gnd;wire n,m;assign n=vdd;assign m=gnd;
 initial begin d=1;#1;$display("T|strength %v %v %v %v %v %v",s,z,vdd,n,gnd,m);$finish;end
endmodule
