// IEEE 1800-2023 §9.4.2, §9.6.1–§9.6.3, §9.7: disabled event
// registrations cannot wake; retained handles still report KILLED.
module top;
  real temp = 25.0;
  process old_job, job;
  int bad;
  initial begin
    repeat (100) begin
      fork : deadline
        begin #2; end
        begin job = process::self(); @(temp); bad++; end
      join_any
      disable deadline;
      if (job.status() != process::KILLED) $fatal(1, "named disable changed");
      old_job = job;
      fork begin #2; end begin @(temp); bad++; end join_any
      disable fork;
      fork : background begin #100; bad++; end join_none
      #1; disable background;
      fork begin : labelled job = process::self(); #100; bad++; end join_none
      #1; disable labelled;
      if (job.status() != process::KILLED || old_job.status() != process::KILLED)
        $fatal(1, "label or retained status changed");
      temp += 1.0;
      #1;
      wait fork;
    end
    if (bad != 0) $fatal(1, "disabled child resumed");
    $display("DISABLE_PASS bad=%0d", bad);
    $finish;
  end
endmodule
