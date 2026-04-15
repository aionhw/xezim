`include "../common/svtest_defs.svh"

class packet_c;
  rand bit [3:0] len;
  rand bit [3:0] tag;

  constraint c_len { len inside {[3:6]}; }
  constraint c_tag { tag < len; }
endclass

module test_randomization_constraints;
  `SVTEST_INIT

  packet_c p;
  int k;

  initial begin
    p = new();

    for (k = 0; k < 10; k++) begin
      `SVTEST_CHECK(p.randomize(), "randomize() failed")
      `SVTEST_CHECK((p.len >= 3) && (p.len <= 6), "constraint range failed")
      `SVTEST_CHECK(p.tag < p.len, "constraint relation failed")
    end

    `SVTEST_PASSFAIL
  end
endmodule
