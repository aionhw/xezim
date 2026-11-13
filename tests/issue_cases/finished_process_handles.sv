// IEEE 1800-2023 §9.3.2, §9.6.1–§9.7: unique process handles retain
// their terminal state after later forks; await wakes at termination.
module top;
  process finished_job, killed_job, fresh_job;
  int completed;
  initial begin
    fork begin finished_job = process::self(); #3; end join_none
    #1;
    finished_job.await();
    if ($time != 3 || finished_job.status() != process::FINISHED)
      $fatal(1, "await completion changed");
    fork begin killed_job = process::self(); #100; completed++; end join_none
    #1;
    killed_job.kill();
    killed_job.await();
    if (killed_job.status() != process::KILLED) $fatal(1, "kill state lost");
    repeat (200) begin
      fork
        begin fresh_job = process::self(); #2; completed++; end
        begin #1; end
      join
      if (fresh_job == finished_job || fresh_job == killed_job)
        $fatal(1, "process handle reused");
      if (fresh_job.status() != process::FINISHED ||
          finished_job.status() != process::FINISHED ||
          killed_job.status() != process::KILLED) $fatal(1, "terminal state changed");
    end
    fork begin fresh_job = process::self(); #10; completed++; end join_none
    #1;
    killed_job.kill();
    fresh_job.suspend();
    #3;
    if (fresh_job.status() != process::SUSPENDED) $fatal(1, "suspend state changed");
    fresh_job.resume();
    wait fork;
    if (completed != 201) $fatal(1, "live child or killed child ran incorrectly");
    $display("HANDLES_PASS completed=%0d t=%0t", completed, $time);
    $finish;
  end
endmodule
