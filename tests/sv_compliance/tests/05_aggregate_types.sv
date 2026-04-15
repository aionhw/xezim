`include "../common/svtest_defs.svh"

typedef enum logic [1:0] {
  IDLE = 2'b00,
  BUSY = 2'b01,
  DONE = 2'b10
} state_t;

typedef struct packed {
  logic [3:0] a;
  logic [3:0] b;
} nibble_pair_t;

typedef union packed {
  nibble_pair_t s;
  logic [7:0] raw;
} pair_u;

module test_aggregate_types;
  `SVTEST_INIT

  state_t st;
  nibble_pair_t pair;
  pair_u u;

  initial begin
    st = DONE;
    pair = '{a:4'hA, b:4'h5};
    u.raw = 8'h3C;

    `SVTEST_CHECK(st == DONE, "enum assignment failed")
    `SVTEST_CHECK(pair.a == 4'hA && pair.b == 4'h5, "packed struct assignment failed")
    `SVTEST_CHECK(u.s.a == 4'h3 && u.s.b == 4'hC, "packed union reinterpretation failed")

    `SVTEST_PASSFAIL
  end
endmodule
