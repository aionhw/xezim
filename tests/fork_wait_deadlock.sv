//----------------------------------------------------------------------
// Minimal regression test for the fork-local variable sharing fix.
//
// Root cause: xezim gave each fork...join_none child a COPY of the
// parent's local variables (see inherit_fork_child_context →
// snapshot_process_context). When a child wrote to a TASK-LOCAL
// automatic variable, the parent never saw the change. Per IEEE
// 1800-2023 §9.3.2, fork children SHARE automatic variables with the
// parent scope.
//
// This is the exact idiom UVM 1800.2-2020.3.1 uses in
// uvm_sequencer_param_base::m_safe_select_item:
//
//   task m_safe_select_item(...);
//      process select_process;           // <-- TASK-LOCAL
//      fork
//         begin
//            select_process = process::self();   // child writes parent local
//            ...
//         end
//      join_none
//      wait(select_process != null);     // parent waits — deadlocked!
//      m_req_fifo.peek(t);
//   endtask
//
// Without the fix, `wait(select_process != null)` parks forever
// (the child's copy never reaches the parent), `peek` is never
// reached, the sequencer rendezvous stalls, the run-phase objection
// never drops, and the sim runs to max_time.
//
// This test distills the idiom using a TASK-LOCAL variable (not a
// module-level signal — those are already shared and would NOT catch
// the bug). It does NOT need the UVM library.
//
//   xezim --simulate -s top fork_wait_deadlock.sv
//
// Before fix: hangs → watchdog prints FAIL at t=1000.
// After fix:  parent wakes → prints PASS → $finish at t=11.
//----------------------------------------------------------------------

module top;

   bit done;

   // A task with a LOCAL automatic variable that a fork child writes
   // and the parent reads after join_none. This mirrors m_safe_select_item.
   task automatic handshake();
      int local_flag;          // TASK-LOCAL (not a module signal!)
      begin
         local_flag = 0;

         fork
            begin
               #1;
               local_flag = 42;    // child writes the parent's local
               $display("[T1] child wrote local_flag = %0d", local_flag);
            end
         join_none

         // Parent waits for the child's write to propagate through the
         // shared automatic. Before the fix this deadlocked.
         wait(local_flag != 0);
         $display("[T0] parent woke — local_flag = %0d", local_flag);
         done = 1;
      end
   endtask

   initial begin
      handshake();
      #10;
      $display("PASS: fork-local variable sharing works");
      $finish;
   end

   // Watchdog: if the wait deadlocks, fail visibly instead of hanging.
   initial begin
      #1000;
      $display("FAIL: wait(local_flag != 0) never woke — fork-local deadlock");
      $finish;
   end

endmodule
