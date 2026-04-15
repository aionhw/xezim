`include "../common/svtest_defs.svh"

class adv_item;
  rand int unsigned a;
  rand int unsigned b;

  constraint c_a { a inside {[1:5]}; }
  constraint c_b { b == (a << 1); }
endclass

module test_constraints_advanced;
  `SVTEST_INIT

  adv_item it;
  int i;

  initial begin
    it = new();

    for (i = 0; i < 5; i++) begin
      `SVTEST_CHECK(it.randomize(), "randomize() failed")
      `SVTEST_CHECK((it.a inside {[1:5]}), "range constraint failed")
      `SVTEST_CHECK(it.b == (it.a << 1), "relation constraint failed")
    end

    `SVTEST_PASSFAIL
  end
endmodule
