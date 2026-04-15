`include "../common/svtest_defs.svh"

function automatic logic resolve_or(input logic drivers[]);
  logic acc;
  int i;
  begin
    acc = 1'b0;
    foreach (drivers[i]) acc |= drivers[i];
    return acc;
  end
endfunction

nettype logic or_net_t with resolve_or;

module test_user_defined_nettypes;
  `SVTEST_INIT

  or_net_t y;

  assign y = 1'b0;
  assign y = 1'b1;

  initial begin
    #0;
    `SVTEST_CHECK(y === 1'b1, "user-defined nettype resolution failed")
    `SVTEST_PASSFAIL
  end
endmodule
