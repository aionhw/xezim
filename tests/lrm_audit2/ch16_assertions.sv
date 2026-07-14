// Ch.16 immediate assertions + basic deferred
module tb;
  int fails = 0;
  int pass_cnt = 0, fail_cnt = 0;
  `define CK(name, cond) if (!(cond)) begin $display("FAIL[16] %s", name); fails++; end
  initial begin
    begin
      int x;
      x = 5;
      assert (x == 5) pass_cnt++; else fail_cnt++;
      assert (x == 6) pass_cnt++; else fail_cnt++;
      `CK("immediate assert branches", pass_cnt == 1 && fail_cnt == 1)
      assert (x inside {[1:10]}) else fails++;
    end
    begin // assert with $error severity should not kill sim
      int y;
      y = 1;
      assert (y == 1) else $error("nope");
      `CK("continue after assert", 1)
    end
    $display("CH16 CHECKS DONE fails=%0d", fails);
  end
endmodule
