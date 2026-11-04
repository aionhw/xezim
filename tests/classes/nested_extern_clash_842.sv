// IEEE 1800-2023 8.23, 8.24: the outer class and the nested class each have a
// method `describe`; the nested one is declared `extern` and defined out of block
// in the body of the enclosing class. The two are different methods of different
// classes. Every line should print PASS. With the bug the nested body is attached
// to the outer class under the bare name `describe`, replaces the outer class own
// method, and the nested class keeps its empty prototype.
class outer;
  int tag = 7;

  class inner;
    extern function int describe();
  endclass

  function int describe();             // the outer class own method, first
    return 200 + tag;
  endfunction

  function int inner::describe();      // the nested class method, defined out of block
    return 100;
  endfunction
endclass

module t;
  int fails = 0;

  task automatic check(string label, int got, int expected);
    if (got == expected) $display("PASS %s", label);
    else begin
      $display("FAIL %s: got %0d, expected %0d", label, got, expected);
      fails++;
    end
  endtask

  initial begin
    outer        o;
    outer::inner ic;

    o  = new();
    ic = new();

    check("nested describe is the nested body",  ic.describe(), 100);
    check("outer describe is the outer body",    o.describe(),  207);

    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
