module top;
  process job;
  initial begin
    fork
      begin
        job = process::self();
        #10;  $display("RESULT step_a_at=%0t", $time);
        #40;  $display("RESULT step_b_at=%0t", $time);  // pushed by resume
      end
    join_none
    #5;
    job.suspend();
    $display("RESULT suspended_at=%0t status=%0d", $time, job.status()); // status=3 SUSPENDED
    #50;
    job.resume();
    $display("RESULT resumed_at=%0t", $time);
    #100;
  end
endmodule
