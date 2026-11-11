`timescale 1ns/1ps
// IEEE 1800-2023 §7.2.2, §7.4.2, §13.5.1: copy whole structs and arrays.
// §23.3.3, §10.3.2: a port actual reaches a continuous assignment's call.
// The free-running clock is required: main's unresolved-read fallback
// depends on its settles to propagate the port copies.
// §4.7/§6.8: stimulus and observation avoid clock edges and time zero.
typedef struct { int value; int tag; } item_t;
function automatic int total(input item_t a [2]);
  return a[0].value + a[1].value + a[0].tag + a[1].tag;
endfunction
function automatic int one(input item_t s);
  return s.value * 10 + s.tag;
endfunction
module child(input item_t pin [2], input item_t ps,
             output int pout, output int sout);
  assign pout = total(pin);
  assign sout = one(ps);
endmodule
module top;
  item_t src [2], s;
  int pout, sout;
  logic clk = 1'b0;
  logic clk_n;
  assign clk_n = ~clk;
  always #1 clk = ~clk;
  child c(.pin(src), .ps(s), .pout(pout), .sout(sout));
  initial begin
    #10.5 src[0] = '{1, 2}; src[1] = '{3, 4}; s = '{4, 5}; // §10.9
    #5 $display("port initial %0d %0d", pout, sout); // 1+2+3+4; 4*10+5
    #5 src[1].tag = 9; s.tag = 1;
    #5 $display("port changed %0d %0d", pout, sout); // 1+2+3+9; 4*10+1
    $finish;
  end
endmodule
