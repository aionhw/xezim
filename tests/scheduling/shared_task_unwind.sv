`timescale 1ns/1ps
// IEEE 1800-2023 §13.3, §9.6.2: early return and task/block disable
// unwind each invocation, then execute the caller's next statement.
// §9.3.2: join completes before unwinding; each site runs before/after sharing.
module top;
  int count = 0;
  int tails = 0;
  task automatic work(input int mode);
    fork begin #1; end join
    begin : inner
      #1;
      if (mode == 0) return;
      if (mode == 1) disable work;
      if (mode == 2) disable inner;
      count += 100;
    end
    count += 10;
  endtask
  initial begin
    repeat (3) begin
      work(0); tails++;
      work(1); tails++;
      work(2); tails++;
      work(3); tails++;
    end
    if (count != 360 || tails != 12) $fatal(1, "unwind changed");
    $display("T|unwind %0d %0d", count, tails);
    $finish;
  end
endmodule
