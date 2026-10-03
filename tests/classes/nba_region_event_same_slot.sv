// Late-region same-slot event delivery (IEEE 1800-2017 4.4/4.7): a process
// resumed from an NBA-region wait (the `nba <= v; @(nba)` idiom used by
// `uvm_wait_for_nba_region`) that fires a named event must wake `@(e)`
// waiters in the SAME time slot. Those late-region resumes run after the
// tick's main edge pass; a bare event flip schedules nothing by itself, so
// without a post-drain re-pass the waiter only wakes in the NEXT slot's
// edge pass — one full time unit late. Reference simulators deliver the
// flip in the same slot (verified byte-for-byte).
module top;
  event e;            // grant event, fired by the late-resumed process
  bit   done  = 0;
  int   woke = -1;    // time at which the @(e) waiter ran

  // uvm_wait_for_nba_region idiom: package-level task (STATIC storage, as in
  // the library) with a task-local variable, NBA update, wait on it. The
  // waiter is resumed from the Re-NBA region drain.
  task nba_region();
    int nba, next_nba;
    next_nba = nba + 1;
    nba <= next_nba;
    @(nba);
  endtask

  // "sequencer": waits the NBA region, then hands the grant. The event fire
  // happens after the tick's main edge pass — only the late-region re-pass
  // can deliver it in this slot.
  initial begin
    #1;
    nba_region();
    -> e;
  end

  // "DUT": must observe the grant at t == 1.
  initial begin
    @(e);
    woke = $time;
  end

  initial begin
    #3;
    if (woke == 1) $display("TAG_PASS");
    else           $display("TAG_FAIL woke=%0d", woke);
    done = 1;
    $finish;
  end
endmodule
