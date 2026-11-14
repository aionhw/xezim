// IEEE 1800-2023 §9.3.2, §9.6.2, §38.36: callbacks associated
// with old child processes must not bind to newly spawned children.
`timescale 1ns/1ns
module top;
  initial begin
    repeat (5) begin
      fork : jobs
        begin $watch; #20; $fatal(1, "disabled child ran"); end
        begin #2; end
      join_any
      disable jobs;
      #1;
    end
    $display("VPI_FORK_PASS t=%0t", $time);
    $finish;
  end
endmodule
