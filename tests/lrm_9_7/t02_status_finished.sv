module top;
  process job;
  initial begin
    fork
      begin job = process::self(); #5; end
    join_none
    #20;
    $display("RESULT finished_status=%0d", job.status());  // -> 0 (FINISHED)
  end
endmodule
