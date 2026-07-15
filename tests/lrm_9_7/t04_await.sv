module top;
  process job;
  initial begin
    fork
      begin job = process::self(); #30; end
    join_none
    #0;                                // let the forked child run first (assigns job)
    wait(job != null);                 // guard against null handle
    job.await();                       // blocks until child finishes at #30
    $display("RESULT await_done_at=%0t", $time);
    $display("RESULT status_after=%0d", job.status());  // -> 0 FINISHED
  end
endmodule
