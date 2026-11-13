// IEEE 1800-2023 §9.3.2, §9.6, §13.4.4, §8, §23, §27:
// children of functions, automatic tasks and methods keep their scope;
// sample after child completion, with no sibling writes to the same data.
package work_pkg;
  class worker;
    int value;
    task run();
      fork begin #2; value++; end begin #1; end join
    endtask
  endclass
endpackage
module worker_unit #(parameter int ID=0)(output int result);
  import work_pkg::*;
  int local_count;
  worker w;
  function automatic void spawn();
    fork begin #2; local_count++; end join_none
  endfunction
  task automatic step(input int n);
    logic signed [7:0] bits_local;
    string text_local;
    bits_local = -8'sd3;
    text_local = "kept";
    fork
      begin #2;
        if (bits_local !== -8'sd3 || text_local != "kept") $fatal(1, "capture changed");
        result += n;
      end
      begin #1; end
    join
  endtask
  initial begin : work
    w = new();
    repeat (10) begin
      spawn(); wait fork;
      step(ID+1);
      w.run();
      fork
        begin #2; end
        begin #100; result = -1000; end
      join_any
      disable fork;
      fork begin fork begin #100; result = -2000; end join_none end join
      wait fork;
      disable fork;
    end
    if (local_count != 10 || w.value != 10 || result != 10*(ID+1))
      $fatal(1, "scope or descendant lifetime changed: local=%0d class=%0d result=%0d", local_count, w.value, result);
  end
endmodule
module top;
  for (genvar i=0; i<2; i++) begin : units
    int result;
    worker_unit #(.ID(i)) dut(result);
  end
  initial begin
    #200;
    if (units[0].result != 10 || units[1].result != 20) $fatal(1, "instance scope changed");
    $display("SCOPES_PASS a=%0d b=%0d", units[0].result, units[1].result);
    $finish;
  end
endmodule
