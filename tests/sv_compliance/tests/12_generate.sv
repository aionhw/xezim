`include "../common/svtest_defs.svh"

module gen_block #(
  parameter int N = 4,
  parameter bit INVERT = 0
) (
  input  logic [N-1:0] a,
  output logic [N-1:0] y
);
  genvar i;
  generate
    if (INVERT) begin : g_inv
      for (i = 0; i < N; i++) begin : g_loop
        assign y[i] = ~a[i];
      end
    end else begin : g_passthru
      for (i = 0; i < N; i++) begin : g_loop
        assign y[i] = a[i];
      end
    end
  endgenerate
endmodule

module test_generate;
  `SVTEST_INIT

  logic [3:0] a;
  logic [3:0] y0, y1;

  gen_block #(.N(4), .INVERT(0)) u0 (.a(a), .y(y0));
  gen_block #(.N(4), .INVERT(1)) u1 (.a(a), .y(y1));

  initial begin
    a = 4'b1010;
    #0;
    `SVTEST_CHECK(y0 == 4'b1010, "if-generate passthrough failed")
    `SVTEST_CHECK(y1 == 4'b0101, "if-generate invert failed")

    `SVTEST_PASSFAIL
  end
endmodule
