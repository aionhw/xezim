`include "../common/svtest_defs.svh"

typedef union tagged packed {
  logic [7:0]  b;
  logic [15:0] h;
} tagged_u_t;

module test_tagged_union_patterns;
  `SVTEST_INIT

  tagged_u_t u_byte;
  tagged_u_t u_half;

  initial begin
    u_byte = tagged '{b: 8'h5a};
    u_half = tagged '{h: 16'h1234};

    `SVTEST_CHECK(u_byte.b == 8'h5a,  "tagged union byte variant failed")
    `SVTEST_CHECK(u_half.h == 16'h1234, "tagged union halfword variant failed")

    `SVTEST_PASSFAIL
  end
endmodule
