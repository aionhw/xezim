`include "../common/svtest_defs.svh"

module test_disable_fork;
  `SVTEST_INIT

  int seen_fast;
  int seen_slow;

  initial begin
    seen_fast = 0;
    seen_slow = 0;

    fork : worker_group
      begin
        #1;
        seen_fast = 1;
      end
      begin
        #10;
        seen_slow = 1;
      end
    join_none

    #2;
    disable worker_group;

    #1;
    `SVTEST_CHECK(seen_fast == 1, "fast fork branch did not complete before disable")
    `SVTEST_CHECK(seen_slow == 0, "disable fork failed to terminate slower branch")

    `SVTEST_PASSFAIL
  end
endmodule
