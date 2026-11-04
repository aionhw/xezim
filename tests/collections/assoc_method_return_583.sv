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

module test_class_method_assoc;

  `SVTEST_INIT

  typedef bit [31:0] word_map_t [int];

  class Builder;
    function word_map_t build();
      word_map_t w;
      w[200] = 32'hcafef00d;            // whole-element write only
      return w;
    endfunction
  endclass

  initial begin
    Builder    o;
    word_map_t w;
    o = new();
    w = o.build();                      // direct (non-virtual) call
    `SVTEST_CHECK(w.exists(200) && w[200] == 32'hcafef00d && w.size() == 1,
                  "class method: assoc whole-element write lost on return (want EXISTS val=cafef00d size=1)")
    `SVTEST_PASSFAIL
    $finish;
  end
endmodule

