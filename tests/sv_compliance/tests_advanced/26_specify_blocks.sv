`include "../common/svtest_defs.svh"

module buf_with_path(input wire a, output wire y);
  assign y = a;
  specify
    (a => y) = (1, 1);
  endspecify
endmodule

module test_specify_blocks;
  `SVTEST_INIT

  logic a;
  wire y;
  time t_drive;
  time t_seen;

  buf_with_path dut(.a(a), .y(y));

  initial begin
    a = 1'b0;
    #1;

    t_drive = $time;
    a = 1'b1;
    wait (y === 1'b1);
    t_seen = $time;

    `SVTEST_CHECK(t_seen > t_drive, "specify path delay was not observed")
    `SVTEST_PASSFAIL
  end
endmodule
