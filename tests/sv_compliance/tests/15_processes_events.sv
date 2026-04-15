`include "../common/svtest_defs.svh"

module test_processes_events;
  `SVTEST_INIT

  event ev;
  int seen;
  semaphore sem;
  int protected_counter;

  initial begin
    seen = 0;
    protected_counter = 0;
    sem = new(1);

    fork
      begin
        #1;
        -> ev;
      end
      begin
        @ev;
        seen = 1;
      end
    join

    `SVTEST_CHECK(seen == 1, "named event synchronization failed")

    fork
      begin
        sem.get();
        protected_counter = protected_counter + 1;
        sem.put();
      end
      begin
        sem.get();
        protected_counter = protected_counter + 1;
        sem.put();
      end
    join

    `SVTEST_CHECK(protected_counter == 2, "semaphore synchronization failed")

    `SVTEST_PASSFAIL
  end
endmodule
