// IEEE 1800-2023 §6.7.1: net default "shall be the value z"; §6.6.5 gives tri0/tri1 continuous pull drivers; §23.3.3 net output does not contribute a variable x driver.
// An undriven child net output contributes z, preserving parent wire and pull defaults.
`timescale 1ns/1ps
typedef struct {logic [7:0] value;} S;
module stub(output wire S o);endmodule
module top;
 wire S w;tri0 S l;tri1 S h;stub a(w);stub b(l);stub c(h);
 initial begin #2;$display("T|idle %h %h %h",w.value,l.value,h.value);$finish;end
endmodule
