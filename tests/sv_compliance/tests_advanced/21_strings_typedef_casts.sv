`include "../common/svtest_defs.svh"

typedef enum logic [1:0] {
  ST_IDLE = 2'd0,
  ST_BUSY = 2'd1,
  ST_DONE = 2'd2
} state_e;

module test_strings_typedef_casts;
  `SVTEST_INIT

  string s;
  string t;
  state_e st;

  initial begin
    s = "sv";
    t = {s, "_lrm"};

    `SVTEST_CHECK(s.len() == 2, "string len() failed")
    `SVTEST_CHECK(t == "sv_lrm", "string concatenation failed")
    `SVTEST_CHECK(t.substr(0, 1) == "sv", "string substr() failed")

    st = state_e'(2);
    `SVTEST_CHECK(st == ST_DONE, "enum cast failed")
    `SVTEST_CHECK($bits(state_e) == 2, "$bits on typedef enum failed")

    `SVTEST_PASSFAIL
  end
endmodule
