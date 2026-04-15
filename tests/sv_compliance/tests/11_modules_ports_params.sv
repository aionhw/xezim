`include "../common/svtest_defs.svh"

module adder #(
  parameter int W = 8
) (
  input  logic [W-1:0] a,
  input  logic [W-1:0] b,
  output logic [W-1:0] y
);
  assign y = a + b;
endmodule

module test_modules_ports_params;
  `SVTEST_INIT

  logic [7:0] a, b, y;

  adder #(.W(8)) u_adder (
    .a(a),
    .b(b),
    .y(y)
  );

  initial begin
    a = 8'd10;
    b = 8'd12;
    #0;
    `SVTEST_CHECK(y == 8'd22, "parameterized module port connection failed")

    `SVTEST_PASSFAIL
  end
endmodule
