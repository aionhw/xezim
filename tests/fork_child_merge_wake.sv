//----------------------------------------------------------------------
// Regression test: a fork...join_none child whose FIRST action is a #0
// deferral runs AFTER the parent parks, so its write to the parent's
// TASK-LOCAL automatic is delivered back to the suspended parent via the
// fork-child merge path — not by a synchronous live-share.
//
// This mirrors the UVM sequencer `m_safe_select_item` rendezvous:
//
//     process select_process;
//     fork
//        begin
//           select_process = process::self();  // child writes parent local
//           forever begin ... process_id.await(); ... end
//        end
//     join_none
//     wait (select_process != null);   // parent parks on the child's write
//
// A #0 deferral between the fork and the write guarantees the parent runs on
// to `wait(...)` first and the write reaches it through the child->parent
// frame merge, exercising the same code path that Fix #2's wake-promotion
// touches. The value must still wake the parked parent (level-sensitive `wait`
// re-checks the propagated activation frame).
//
//   xezim --simulate -s top fork_child_merge_wake.sv
//   Questa: PASS result=1  (byte-for-byte identical)
//----------------------------------------------------------------------

module top;

   int result;

   task automatic handshake();
      int local_flag;           // TASK-LOCAL (not a module signal!)
      begin
         local_flag = 0;

         fork
            begin
               #0;              // defer so the parent parks first
               local_flag = 1;  // then write the parent's automatic
            end
         join_none

         // Parent parks until the child's deferred write is propagated back
         // into this activation and the level-sensitive wait re-checks it.
         wait(local_flag != 0);
         result = result + 1;
      end
   endtask

   initial begin
      handshake();
      #2;
      $display("PASS result=%0d", result);
      $finish;
   end

   // Watchdog: if the wait never wakes, fail visibly instead of hanging.
   initial begin
      #2000;
      $display("FAIL: wait(local_flag != 0) never woke — deferred child write lost");
      $finish;
   end

endmodule