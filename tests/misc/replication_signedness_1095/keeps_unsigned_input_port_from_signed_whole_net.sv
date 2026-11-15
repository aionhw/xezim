// IEEE 1800-2023 §23.3.3.8: "The sign attribute shall not cross hierarchy."
// The mirror whole-net direction: an UNSIGNED `input [31:0] p` bound to a
// SIGNED `int s` actual must read as the unsigned value inside the child.
// The formal keeps its own storage and re-interprets the signed actual's bits
// through the formal's unsigned type; substituting the formal onto the signed
// actual would read s with s's (signed) sign and zero- vs sign-extend the
// wrong way (`q = p` would then sign-extend -5 to -5 instead of 4294967291).
module child(input [31:0] p, output longint q);
  assign q = p;
endmodule
module top;
  int s;
  longint q;
  child c(.p(s), .q(q));
  initial begin
    #1 s = -5;
    #1 if (q === 64'd4294967291) $display("PASS unsigned-formal q=%0d", q);
       else $display("FAIL unsigned-formal q=%0d (expected 4294967291)", q);
    $finish;
  end
endmodule