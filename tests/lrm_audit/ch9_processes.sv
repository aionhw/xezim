// IEEE 1800-2017 Ch.9 — processes
module tb;
  int fails = 0;
  `define CK(name, cond) if (!(cond)) begin $display("FAIL[9] %s", name); fails++; end
  int order[$];
  event ev;
  initial begin
    begin // 9.3.2 fork/join variants
      int a, b;
      fork
        #2 a = 1;
        #1 b = 1;
      join
      `CK("fork join waits all", a == 1 && b == 1 && $time == 2)
      fork
        #5 order.push_back(99);
        #1 order.push_back(1);
      join_any
      `CK("join_any returns at first", $time == 3)
      wait (order.size() == 2);
      `CK("both children finish", order[1] == 99)
    end
    begin // join_none + automatic capture
      int seen[$];
      for (int i = 0; i < 3; i++) begin
        automatic int li = i;
        fork
          #1 seen.push_back(li);
        join_none
      end
      #2;
      `CK("join_none spawned all", seen.size() == 3)
      `CK("automatic captured 0", seen[0] inside {0,1,2})
      `CK("distinct captures", seen[0]+seen[1]+seen[2] == 3)
    end
    begin // 9.4 event control, ->, wait
      int got;
      fork
        begin @(ev); got = 1; end
        begin #1; ->ev; end
      join
      `CK("named event", got == 1)
    end
    begin // 9.6.2 disable named block
      int cnt;
      cnt = 0;
      begin : blk
        for (int i = 0; i < 10; i++) begin
          if (i == 4) disable blk;
          cnt++;
        end
      end
      `CK("disable block", cnt == 4)
    end
    begin // 9.6.3 disable fork
      int x;
      x = 0;
      fork
        #10 x = 1;
      join_none
      #1 disable fork;
      #12;
      `CK("disable fork kills child", x == 0)
    end
    begin // 9.4.5 intra-assignment / #0 ordering
      int v;
      v = 0;
      fork
        v = #2 5;
      join_none
      #1 `CK("intra-assign delay pending", v == 0)
      #2 `CK("intra-assign landed", v == 5)
    end
    $display("CH9 CHECKS DONE fails=%0d", fails);
  end
endmodule
