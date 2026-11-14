// IEEE 1800-2023 §23.3.3.7 Table 23-1 selects the internal tri0/tri1 kind; §6.6.5: tri1 has "a continuous 1 value of pull strength driving it" (dual tri0); §10.11 shares it.
// Implicit pull ports dominate wire connections even with a z driver; assert only main-correct names.
`timescale 1ns/1ps
module up(inout tri1 p);endmodule
module down(inout tri0 p);endmodule
module top;
 wire a,b,c;alias a=b;alias c=b;assign a=1'bz;up u(c);
 wire d,e,f;alias d=e;alias f=e;assign d=1'bz;down v(f);
 initial begin #1;$display("T|pull %b%b%b %b%b%b",b,c,u.p,e,f,v.p);$finish;end
endmodule
