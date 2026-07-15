// §9.7 status(): a process blocked in a delay reports WAITING (2),
// and a process actively executing reports RUNNING (1).
module top;
  process job;
  int seen_waiting;
  initial begin
    fork
      begin
        job = process::self();
        seen_waiting = job.status();   // executing right now -> RUNNING(1)
        #50;                            // now it blocks -> WAITING(2)
      end
    join_none
    #10;
    $display("RESULT blocking_status=%0d", job.status());  // -> 2 (WAITING)
    $display("RESULT running_status=%0d", seen_waiting);   // -> 1 (RUNNING)
    #100;
  end
endmodule
