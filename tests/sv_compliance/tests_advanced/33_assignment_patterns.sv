`include "../common/svtest_defs.svh"

typedef struct packed {
  logic [7:0]  addr;
  logic [15:0] data;
  logic        valid;
} packet_t;

module test_assignment_patterns;
  `SVTEST_INIT

  packet_t pkt_named;
  packet_t pkt_ordered;

  initial begin
    pkt_named   = '{addr: 8'h12, data: 16'h3456, valid: 1'b1};
    pkt_ordered = '{8'hab, 16'hcdef, 1'b0};

    `SVTEST_CHECK(pkt_named.addr  == 8'h12,   "named assignment pattern field failed")
    `SVTEST_CHECK(pkt_named.data  == 16'h3456, "named assignment pattern payload failed")
    `SVTEST_CHECK(pkt_named.valid == 1'b1,    "named assignment pattern valid bit failed")

    `SVTEST_CHECK(pkt_ordered.addr  == 8'hab,   "ordered assignment pattern field failed")
    `SVTEST_CHECK(pkt_ordered.data  == 16'hcdef, "ordered assignment pattern payload failed")
    `SVTEST_CHECK(pkt_ordered.valid == 1'b0,    "ordered assignment pattern valid bit failed")

    `SVTEST_PASSFAIL
  end
endmodule
