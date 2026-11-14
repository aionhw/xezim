`timescale 1ns/1ps
// §13.3, §9.3.2: checkpoint must preserve suspended invocations.
// $save/$restart are optional system tasks of Annex D.9.
module top;
  int count;
  task automatic work(input int id);
    fork #2; #4; join
    count += id;
  endtask
  initial begin
    if ($test$plusargs("again")) $restart("state.ckpt");
  end
  initial begin
    repeat (4) begin work(3); #3; end
    $display("T|checkpoint %0d",count);
    $finish;
  end
  initial begin #5; $save("state.ckpt"); end
endmodule
