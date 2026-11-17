// IEEE 1800-2023 §23.3.3.8: "The sign attribute shall not cross hierarchy."
// The whole-net cousin of the replication guard: a signed `input int p` bound
// to a same-width WHOLE-NET actual of the opposite sign (`wire [31:0] u`)
// must read as the signed value inside the child. The input connection is an
// implied copy, so the formal keeps its own storage and re-interprets u's bits
// through the formal's signed type; collapsing the copy onto the unsigned net
// would read u's bits with u's (unsigned) sign and the sign flip would stop
// (`q = p` would zero-extend 0xffff_fffb to 4294967291 instead of -5).
module child(input int p, output longint q);
  assign q = p;
endmodule
module top;
  wire [31:0] u = 32'hffff_fffb;
  longint q;
  child c(.p(u), .q(q));
  initial begin
    #1 if (q === -64'sd5) $display("PASS signed-formal q=%0d", q);
       else $display("FAIL signed-formal q=%0d (expected -5)", q);
    $finish;
  end
endmodule