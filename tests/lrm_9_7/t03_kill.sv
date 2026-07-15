module top;
  process job;
  initial begin
    fork
      begin
        job = process::self();
        #100;
        $display("RESULT VICTIM_RAN");   // must NOT print (killed at #10)
      end
    join_none
    #10;
    job.kill();
    $display("RESULT killed_status=%0d", job.status());  // -> 4 (KILLED)
    #100;
    $display("RESULT done");
  end
endmodule
