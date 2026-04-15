// EXPECT: compile_fail
module neg09_duplicate_port_name(
  input  logic a,
  output logic a
);
endmodule
