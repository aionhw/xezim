`timescale 1ns/1ps
// IEEE 1800-2023 §13.3.1–§13.3.2, §13.5: invocation storage and copy-out.
// §9.3.2, §9.6.1–§9.6.3: joins, waiting and cancellation.
module top;
  int done_count, cancelled, detached;
  int a, b, c, d;
  task automatic work(input int id, output int result, inout int accum);
    int local_value = id;
    fork
      begin #2; local_value += 10; end
      begin #1; end
    join
    result = local_value;
    accum += id;
  endtask
  // Both processes reach the same work() call site.
  task automatic invoke(input int id, output int result, inout int accum);
    work(id, result, accum);
  endtask
  task static counter(output int result);
    static int n = 0;
    fork #1; #2; join
    n++;
    result = n;
  endtask
  task automatic recursive(input int depth, output int result);
    int child;
    fork #1; #2; join
    if (depth != 0) begin
      recursive(depth - 1, child);
      result = child + depth;
    end else result = 0;
  endtask
  task automatic named_exit();
    begin : inner
      fork #1; #2; join
      disable inner;
      $display("T|BAD block");
    end
    done_count++;
    fork #1; #2; join
    disable named_exit;
    $display("T|BAD task");
  endtask
  task automatic first_child();
    fork
      begin #1; done_count++; end
      begin #5; cancelled++; end
    join_any
    disable fork;
  endtask
  task automatic all_children();
    fork
      begin #1; done_count++; end
      begin #3; done_count++; end
    join_none
    wait fork;
  endtask
  task automatic launch();
    fork begin #2; detached++; end join_none
  endtask
  initial begin
    c=100; d=200;
    fork invoke(3,a,c); invoke(7,b,d); join
    $display("T|automatic %0d %0d %0d %0d",a,b,c,d);
    repeat (3) begin counter(a); $display("T|static %0d",a); end
    recursive(3,a); $display("T|recursive %0d",a);
    repeat (3) named_exit();
    repeat (3) first_child();
    repeat (3) all_children();
    repeat (3) begin launch(); #3; end
    #6;
    $display("T|control %0d %0d %0d",done_count,cancelled,detached);
    $finish;
  end
endmodule
