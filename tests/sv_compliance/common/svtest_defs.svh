`ifndef SVTEST_DEFS_SVH
`define SVTEST_DEFS_SVH

`define SVTEST_INIT \
  int failures = 0;

`define SVTEST_CHECK(expr, msg) \
  if (!(expr)) begin \
    failures++; \
    $display("FAIL: %s", msg); \
  end

`define SVTEST_PASSFAIL \
  if (failures == 0) begin \
    $display("TEST_PASS"); \
  end else begin \
    $display("TEST_FAIL count=%0d", failures); \
    $fatal(1); \
  end

`endif
