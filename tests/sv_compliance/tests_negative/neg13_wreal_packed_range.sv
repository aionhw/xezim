// EXPECT: compile_fail
//
// Verilog-AMS 2.4.0 §3.7: a `wreal` net carries one REAL value. The grammar
// there is `wreal [ discipline_identifier ] [ range ] list_of_net_identifiers`,
// so a range is legal syntax — xezim simply does not model a VECTOR of reals,
// and rejects it rather than flattening it. Accepting one is not harmless -- the net
// then behaves as an ordinary 4-bit wire and silently ROUNDS every value
// written to it (2.5 reads back 3.0), which is the exact corruption `wreal`
// exists to prevent, with nothing reported. Both spellings below were once
// accepted in silence; the port form reaches the check by a different path
// than the declaration form, so both are pinned here.
module neg13_wreal_packed_range (input wreal [3:0] p);
  wreal [3:0] w;
  assign w = 2.5;
endmodule
