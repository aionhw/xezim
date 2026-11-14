`timescale 1ns/1ps
// IEEE 1800-2023 §9.6.2: disabling from a fork child unwinds its
// enclosing task/block and cancels the other descendants.
module top;
  int hits, after_block;
  task automatic stop_task();
    fork
      begin #1; hits++; disable stop_task; end
      begin #4; $display("T|BAD sibling"); end
    join
    $display("T|BAD task tail");
  endtask
  task automatic stop_block();
    begin : inner
      fork
        begin #1; hits++; disable inner; end
        begin #4; $display("T|BAD block sibling"); end
      join
      $display("T|BAD block tail");
    end
    after_block++;
  endtask
  initial begin
    repeat (4) begin stop_task(); stop_block(); end
    #5;
    $display("T|disable %0d %0d",hits,after_block);
    $finish;
  end
endmodule
