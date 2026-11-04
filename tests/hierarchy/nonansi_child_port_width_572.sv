// IEEE 1800-2017 23.2.2.1 / 23.3.3: a port connection is an assignment to the port,
// and the port keeps its declared type and width, whatever else it is connected to.
// A sub-instance declared with a non-ANSI header (`module k(a_in, y); input [7:0]
// a_in;`) must give the same answers as one declared with an ANSI header
// (`module k(input [7:0] a_in, ...)`). Every line should print PASS.
typedef struct packed { logic [3:0] hi; logic [3:0] lo; } pair_t;

module child_na(a_in, y);            // non-ANSI header: names only, directions in the body
  input  [7:0] a_in;
  output [7:0] y;
  assign y = a_in;
endmodule
module child_ansi(input [7:0] a_in, output [7:0] y);   // ANSI header
  assign y = a_in;
endmodule

// A 10-bit input fed an 8-bit actual: $bits(a) is 10 (the formal's width).
module w_na  (input [9:0] a, output [31:0] w, output [7:0] y); assign w = $bits(a); child_na   k(a[7:0], y); endmodule
module w_ansi(input [9:0] a, output [31:0] w, output [7:0] y); assign w = $bits(a); child_ansi k(a[7:0], y); endmodule
module w_none(input [9:0] a, output [31:0] w);                 assign w = $bits(a);                          endmodule

// A packed-struct input fed a flat actual: p.hi is the upper nibble.
module s_na  (input pair_t p, output [3:0] hi, output [7:0] y); assign hi = p.hi; child_na   k(p[7:0], y); endmodule
module s_ansi(input pair_t p, output [3:0] hi, output [7:0] y); assign hi = p.hi; child_ansi k(p[7:0], y); endmodule
module s_none(input pair_t p, output [3:0] hi);                 assign hi = p.hi;                           endmodule

module t;
  reg  [7:0]  v = 8'hc6;
  wire [31:0] bits_na, bits_ansi, bits_none;
  wire [3:0]  hi_na, hi_ansi, hi_none;
  wire [7:0]  y1, y2, y3, y4;
  int fails = 0;

  w_na   u1(.a(v), .w(bits_na),   .y(y1));
  w_ansi u2(.a(v), .w(bits_ansi), .y(y2));
  w_none u3(.a(v), .w(bits_none));
  s_na   u4(.p(v), .hi(hi_na),    .y(y3));
  s_ansi u5(.p(v), .hi(hi_ansi),  .y(y4));
  s_none u6(.p(v), .hi(hi_none));

  task automatic chk(string what, logic [31:0] got, logic [31:0] exp);
    if (got === exp) $display("PASS %s", what);
    else begin $display("FAIL %s: got %0d (%h), expected %0d (%h)", what, got, got, exp, exp); fails++; end
  endtask

  initial begin
    #1;
    chk("$bits(a), child with ANSI header",      bits_ansi, 10);
    chk("$bits(a), no child (control)",          bits_none, 10);
    chk("$bits(a), child with non-ANSI header",  bits_na,   10);
    chk("p.hi, child with ANSI header",          hi_ansi,   4'hc);
    chk("p.hi, no child (control)",              hi_none,   4'hc);
    chk("p.hi, child with non-ANSI header",      hi_na,     4'hc);
    chk("child output, ANSI",                    y2, 8'hc6);
    chk("child output, non-ANSI",                y1, 8'hc6);
    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
