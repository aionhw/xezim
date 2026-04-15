// EXPECT: compile_fail
module neg11_child(input logic a, output logic y);
endmodule

module neg11_missing_named_port;
  logic a;
  logic y;

  neg11_child u0(
    .a(a),
    .missing(y)
  );
endmodule
