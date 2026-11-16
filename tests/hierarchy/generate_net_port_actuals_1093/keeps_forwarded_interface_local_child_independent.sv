// IEEE 1800-2023 §23.3.3.4 permits a "higher level interface port"; §23.6 gives the local children independent identities, each driven only by 0110.
// Forwarding an interface through a second port must not make leaf drivers corrupt a middle-module child with the interface instance name.
// §25.3, §23.3.3.4: an interface port passed through a second interface port, where the MIDDLE
// module has a local instance with the name of the top's interface instance. The leaf drives
// k.g[1].w and k.sig: these are nets of tb.one, never of m.one.
`timescale 1ns/1ps
interface ifc; wire [3:0] sig; assign sig = 4'b1010;
  for (genvar k = 0; k < 2; k++) begin : g wire [3:0] w; assign w = 4'b1010; end
endinterface
module other; wire [3:0] sig; assign sig = 4'b0110;
  for (genvar k = 0; k < 2; k++) begin : g wire [3:0] w; assign w = 4'b0110; end
endmodule
module drv(inout wire [3:0] p); assign p = 4'b0001; endmodule
module rd(input wire [3:0] p, output logic [3:0] s); always_comb s = p; endmodule
module leaf(ifc j [1:0], ifc k);
  logic [3:0] s, t;
  drv d(k.g[1].w); drv e(j[1].sig); rd r(k.g[0].w, s); rd q(j[0].sig, t);
endmodule
module mid(ifc i [1:0], ifc o);
  other bus [1:0] (); other one ();
  leaf l(i, o);
endmodule
module tb;
  ifc bus [1:0] (); ifc one ();
  mid m(bus, one);
  initial begin #1;
    $display("T|forward %b %b",m.one.g[1].w,m.bus[1].sig);
    $finish; end
endmodule
