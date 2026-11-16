// IEEE 1800-2023 §11.8.1: "Concatenation results are unsigned, regardless of
// the operands." `{1{x}}` is a one-part concatenation (replication), so its
// result is unsigned and zero-extends to a wider target; the operand `x`
// itself keeps its signedness. Regression for #1095.
`timescale 1ns/1ps
module top;
  typedef struct { longint a[2]; } I64;
  int x;
  longint y;
  I64 k, d;
  assign y = {1{x}};
  assign k = '{a:'{1:{1{x}}, 0:x}};
  assign d = '{a:'{default:{1{x}}}};
  initial begin
    #1 x = -5;
    #1 if (y == 64'd4294967291) $display("PASS plain y=%0d", y);
       else $display("FAIL plain y=%0d (expected 4294967291)", y);
       if (k.a[1] == 64'd4294967291 && k.a[0] == -5) $display("PASS keyed k=%0d,%0d", k.a[0], k.a[1]);
       else $display("FAIL keyed k=%0d,%0d (expected -5,4294967291)", k.a[0], k.a[1]);
       if (d.a[0] == 64'd4294967291 && d.a[1] == 64'd4294967291) $display("PASS default d=%0d,%0d", d.a[0], d.a[1]);
       else $display("FAIL default d=%0d,%0d (expected 4294967291,4294967291)", d.a[0], d.a[1]);
    $finish;
  end
endmodule