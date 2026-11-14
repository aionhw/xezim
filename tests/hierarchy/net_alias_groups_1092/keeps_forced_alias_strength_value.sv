// IEEE 1800-2023 §10.6.2: force "shall override all drivers of the net"; §21.2.1.4: "The third character indicates the current logic value of the scalar"; §10.11 shares force/release.
// A forced alias must format its forced logic value. Strength letters during force are deliberately not pinned.
`timescale 1ns/1ps
module top;
 wire z,a;alias a=z;assign (weak0,weak1) z=0;string strength;
 initial begin #1;force z=1;#1;strength=$sformatf("%v",a);
 $display("T|forced %b %c",a,strength.getc(2));release a;#1;
 $display("T|released %b",a);$finish;end
endmodule
