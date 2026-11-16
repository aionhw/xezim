// IEEE 1800-2023 §§23.3.1,23.6,27.3 give the local block its own declaration; §10.11 limits the alias to top a,b. §10.11: "same physical net".
// Generate scope shadowing must not redirect the independent local net into a top alias group.
`timescale 1ns/1ps
module m;
 wire a,b;alias a=b;assign b=1;
 if(1)begin:m wire a;assign a=0;end
 initial begin #1;$display("T|scope %b%b %b",a,b,m.a);$finish;end
endmodule
