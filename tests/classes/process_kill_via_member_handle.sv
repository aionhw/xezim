module top;
  class Obj;
    int data = 0;
    int triggered = 0;
    process watched_proc;
    process watcher_proc;
  endclass

  task automatic worker(Obj obj);
    obj.watched_proc = process::self();
    #20;
    obj.data = 42; $display("worker done at t=%0t data=%0d", $time, obj.data);
  endtask

  function automatic void start_watcher(Obj obj);
    fork
      begin
        process p;
        p = obj.watched_proc;
        obj.watcher_proc = process::self();
        p.await();
        obj.triggered++;
      end
    join_none
  endfunction

  initial begin
    automatic Obj obj1 = new();
    automatic Obj obj2 = new();

    // Test 1: watcher awaits worker, worker finishes at t=20 -> obj1.triggered increments
    fork
      worker(obj1);
    join_none
    #1;
    start_watcher(obj1);

    // Test 2: watcher is killed before worker finishes -> obj2.triggered must NOT increment
    fork
      worker(obj2);
    join_none
    #1;
    start_watcher(obj2);
    #5;
    // Kill watcher while worker(obj2) is still running
    $display("t=%0t killing watcher_proc=%0d", $time, obj2.watcher_proc); obj2.watcher_proc.kill();

    #30; // t=36: worker has finished at t=21

    if (obj1.triggered == 1 && obj2.triggered == 0 && obj1.data == 42 && obj2.data == 42) begin
      $display("TAG_PASS");
    end else begin
      $display("TAG_FAIL: trig1=%0d trig2=%0d data1=%0d data2=%0d",
               obj1.triggered, obj2.triggered, obj1.data, obj2.data);
    end
  end
endmodule
