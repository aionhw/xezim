class Env;
  bit a = 0;
  bit b = 0;
  bit [3:0] v = 0;
  int count_change = 0;
  int count_edge = 0;

  task watcher_change();
    forever begin
      @(a || b);
      count_change++;
    end
  endtask

  task watcher_edge();
    forever begin
      @(negedge (v[0] & a));
      count_edge++;
    end
  endtask

  task run();
    fork
      watcher_change();
      watcher_edge();
      begin
        #10;
        v = 4'b0001;
        a = 1; // a||b: 0->1 (change++); v[0]&a: 0->1 (posedge, not negedge)
        #10;
        b = 1; // a||b: unchanged (1); v[0]&a: unchanged (1)
        #10;
        a = 0; // a||b: 1||1 -> 0||1 (unchanged: 1); v[0]&a: 1->0 (negedge! edge++)
        #10;
        b = 0; // a||b: 0||0 (change++)
        #10;
        if (count_change == 2 && count_edge == 1) begin
          $display("PASS");
        end else begin
          $display("FAIL: change=%0d edge=%0d", count_change, count_edge);
        end
        $finish;
      end
    join
  endtask
endclass

module top;
  Env env;
  initial begin
    env = new();
    env.run();
  end
endmodule
