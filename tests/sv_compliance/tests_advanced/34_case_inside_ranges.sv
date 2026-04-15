`include "../common/svtest_defs.svh"

module test_case_inside_ranges;
  `SVTEST_INIT

  int sel;
  int out;

  task automatic decode(input int v);
    begin
      case (v) inside
        [0:3]:   out = 1;
        [4:7]:   out = 2;
        8, 9:    out = 3;
        default: out = 4;
      endcase
    end
  endtask

  initial begin
    decode(2);
    `SVTEST_CHECK(out == 1, "case inside low range failed")

    decode(6);
    `SVTEST_CHECK(out == 2, "case inside mid range failed")

    decode(9);
    `SVTEST_CHECK(out == 3, "case inside discrete item failed")

    decode(20);
    `SVTEST_CHECK(out == 4, "case inside default failed")

    `SVTEST_PASSFAIL
  end
endmodule
