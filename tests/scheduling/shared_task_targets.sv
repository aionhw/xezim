`timescale 1ns/1ps
// IEEE 1800-2023 §13.3, §23.3, §25.2, §26: resolved task bodies
// must stay separate across instances, interfaces and packages.
package workers;
  task automatic work(input int id, output int result);
    fork #1; #2; join
    result = id + 30;
  endtask
endpackage
interface worker_if;
  task automatic work(input int id, output int result);
    fork #1; #2; join
    result = id + 40;
  endtask
endinterface
module worker #(parameter int BASE=10);
  task automatic work(input int id, output int result);
    fork #1; #2; join
    result = id + BASE;
  endtask
endmodule
module top;
  worker #(10) left();
  worker #(20) right();
  worker_if bus();
  int a,b,c,d;
  initial begin
    repeat (3) begin
      fork
        left.work(1,a);
        right.work(2,b);
        workers::work(3,c);
        bus.work(4,d);
      join
      $display("T|targets %0d %0d %0d %0d",a,b,c,d);
    end
    $finish;
  end
endmodule
