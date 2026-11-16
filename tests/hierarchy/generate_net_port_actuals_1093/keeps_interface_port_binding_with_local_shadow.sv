// IEEE 1800-2023 §23.3.3.4: interface port connects to "an interface instance or a higher level interface port"; §§25.3,23.8 give read 1010 and independent locals 0110. Main-wrong driven interface fields are omitted.
// An interface-port actual names the connected interface; the independent local child must retain its driver.
// Minimal form of s06, named connection and an input port. LRM 25.3, 23.3.3:
// i[1].sig is the net of the interface instance connected to port i (tb.bus[1].sig), never ma.bus[1].sig.
`timescale 1ns/1ps
interface ifc; wire [3:0] sig; assign sig = 4'b1010; endinterface
module rd(input wire [3:0] p, output logic [3:0] s); always_comb s = p; endmodule
module drv(inout wire [3:0] p); assign p = 4'b0001; endmodule
module other; wire [3:0] sig; assign sig = 4'b0110; endmodule
module mid(ifc i [1:0]);
  other bus [1:0] ();                 // a local instance array; the top's interface array has the same name
  logic [3:0] s;
  rd  r(.p(i[1].sig), .s(s));         // reads the interface net: 1010
  drv d(.p(i[0].sig));                // drives the interface net: tb.bus[0].sig = 1010 with 0001 = x0xx
endmodule
module tb;
  ifc bus [1:0] ();
  mid ma(bus);
  initial begin #1;
    $display("T|iface %b %b %b %b",ma.s,ma.r.p,ma.bus[1].sig,ma.bus[0].sig);
    $finish; end
endmodule
