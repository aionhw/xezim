// IEEE 1800-2023 §23.3.1 gives "priority to the local scope"; §10.11: "the scope of an alias is limited"; child 0 is independent of top 11.
// A child named like the top cannot be stripped as a top prefix.
`timescale 1ns/1ps
module child;wire a=0;endmodule
module m;
 wire a,b;alias a=b;assign b=1;child m();
 if(1)begin:n wire a,b;alias a=b;assign b=0;end
 initial begin #1;$display("T|scope %b%b %b",a,b,m.a);$finish;end
endmodule
