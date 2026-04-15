`include "../common/svtest_defs.svh"

module test_fork_join_wait;
  `SVTEST_INIT

  int finished;

  initial begin
    finished = 0;

    fork
      begin #1 finished++; end
      begin #3 finished++; end
    join_any

    `SVTEST_CHECK(finished == 1, "join_any did not resume after first process")

    wait (finished == 2);
    `SVTEST_CHECK(finished == 2, "second forked process did not finish")

    finished = 0;

    fork
      begin #1 finished++; end
      begin #2 finished++; end
    join_none

    wait fork;
    `SVTEST_CHECK(finished == 2, "join_none/wait fork failed")

    `SVTEST_PASSFAIL
  end
endmodule
