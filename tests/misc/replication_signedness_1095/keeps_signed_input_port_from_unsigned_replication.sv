// IEEE 1800-2023 §23.3.3.8: "The sign attribute shall not cross hierarchy."
// A signed `input int p` bound to the unsigned replication `{1{x}}` must read
// as the signed value inside the child; the connection assignment re-interprets
// the unsigned bits through the formal's signed type. This is the cousin of
// #1095: clearing the replication's sign (so `{1{x}}` is unsigned) must not
// make an equal-width signed input formal read as unsigned.
`timescale 1ns/1ps
module child(input int p, output longint q);
  assign q = p;
endmodule
module top;
  int x;
  longint q;
  child u(.p({1{x}}), .q(q));
  initial begin
    #1 x = -5;
    #1 if (q === -64'sd5) $display("PASS port q=%0d", q);
       else $display("FAIL port q=%0d (expected -5)", q);
    $finish;
  end
endmodule