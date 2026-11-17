// IEEE 1800-2023 §7.4.2: each unpacked element has its declared type; §23.3.3.3 permits inout net elements, carrying all four zero bits. §6.5: "a continuous assignment is implied" at variable port connections.
// An unpacked array element is a four-bit actual; do not rewrite its base into a one-bit path or emit false width warnings.
`timescale 1ns/1ps
module load(inout wire [3:0] n);endmodule
module top;
 for(genvar i=0;i<2;i++)begin:g
  wire [3:0] node[0:1];assign node[0]=4'b1111;assign node[1]=4'b0000;
  if(i==0)begin:r load named(.n(g[1].node[1]));load ordered(g[1].node[1]);end
 end
 initial begin #1;$display("T|elements %b %b",g[0].r.named.n,g[0].r.ordered.n);$finish;end
endmodule
