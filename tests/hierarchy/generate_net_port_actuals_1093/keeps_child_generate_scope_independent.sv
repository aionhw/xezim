// IEEE 1800-2023 §23.8: "Look in the current scope" before enclosing scopes; §23.6 gives distinct hierarchical objects; top has only driver 1010 (§10.3.2).
// A path in a child body must not capture the top net with the same generate path.
`timescale 1ns/1ps
module drv(inout wire [3:0] p);assign p=4'b0001;endmodule
module mid;
 for(genvar k=0;k<2;k++)begin:g wire [3:0] w=4'b0011;end
 drv d(g[1].w);
endmodule
module top;
 for(genvar k=0;k<2;k++)begin:g wire [3:0] w=4'b1010;end
 mid m();
 initial begin #1;$display("T|outer %b",g[1].w);$finish;end
endmodule
