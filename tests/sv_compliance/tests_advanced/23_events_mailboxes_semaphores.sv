`include "../common/svtest_defs.svh"

module test_events_mailboxes_semaphores;
  `SVTEST_INIT

  event ev;
  semaphore sem;
  mailbox #(int) mb;
  bit seen_ev;
  int got;

  initial begin
    sem = new(1);
    mb  = new();
    seen_ev = 0;

    fork
      begin
        #1;
        -> ev;
      end
      begin
        @ev;
        seen_ev = 1;
      end
    join

    `SVTEST_CHECK(seen_ev == 1'b1, "event trigger/wait failed")

    sem.get(1);
    sem.put(1);

    mb.put(32'h1234);
    mb.get(got);
    `SVTEST_CHECK(got == 32'h1234, "mailbox put/get failed")

    `SVTEST_PASSFAIL
  end
endmodule
