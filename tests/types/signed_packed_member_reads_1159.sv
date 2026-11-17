// IEEE 1800-2023 §7.2.1, §6.11.3, §11.8.3: a member of a packed structure
// keeps its declared type, so a signed member is sign-extended into a wider
// target and is signed in arithmetic, comparisons and arithmetic shifts.
// §11.6.1, §11.8.2: inside a concatenation an operator's operands are
// extended to the wider operand by their own sign.
module top;
  typedef struct packed { int f; } S1;
  typedef struct packed { int f; byte b; } S2;
  typedef enum int { NEG = -3, POS = 3 } e_t;
  typedef struct packed { e_t e; byte pad; } SE;
  typedef struct packed { int f; shortint h; byte b; logic signed [4:0] l; } S4;
  S1 s; S2 s2; SE se; S4 s4;
  S1 sa [0:1];
  longint q, q2, q3, q4, q5, q6, q7, q8, q9;
  logic c1;
  int r1;
  assign q = s.f;
  assign q2 = s2.f;
  assign q3 = sa[0].f;
  assign q4 = se.e;
  assign q5 = s.f + s2.b;
  always_comb q6 = s.f >>> 1;
  assign q7 = s4.h;
  assign q8 = s4.l;
  assign c1 = s4.h < 0;
  always_comb r1 = s4.b + 0;
  shortint vw; byte vb; longint k1;
  int pi; byte vx, vb2; longint k2;
  assign k1 = {vw + vb};
  assign k2 = {(pi + vx) ^ vb2};
  initial begin
    s.f = -1; s2.f = -2; s2.b = -3; sa[0].f = -4; se.e = NEG;
    s4.f = 0; s4.h = -2; s4.b = -3; s4.l = -4;
    vw = 16'h7fff; vb = -1; pi = -1; vx = -128; vb2 = -100;
    #1;
    $display("T|members %h %h %h %h", q, q2, q3, q4);
    $display("T|ops %h %h %h %h %b %0d", q5, q6, q7, q8, c1, r1);
    $display("T|concat %h %h", k1, k2);
    $finish;
  end
endmodule
