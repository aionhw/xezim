// IEEE 1800-2023 8.23, 8.24: a class can be declared inside another class, and
// a method of it can be declared `extern` in the nested class and defined out of
// block, in the body of the enclosing class, with the nested class as its scope:
//
//     class outer;
//       class inner;  extern task run(int x);  endclass
//       task inner::run(int x); ... endtask      // still inside `outer`
//     endclass
//
// The body belongs to `inner`. Every line should print PASS. With the bug the
// body is never linked to `inner`: the call runs the empty prototype, returns the
// default value and prints no diagnostic.
class outer;
  int tag = 7;

  class inner;
    int v = 0;
    extern task          bump(int x);
    extern function int  twice(int x);
    extern static function int triple(int x);
    // defined in the nested class itself: a control
    function int plus_one(int x);
      return x + 1;
    endfunction
  endclass

  // Out-of-block definitions of the nested class methods, in the outer body.
  task inner::bump(int x);
    v = x + 1;
  endtask

  function int inner::twice(int x);
    return 2 * x;
  endfunction

  function int inner::triple(int x);
    return 3 * x;
  endfunction

  // a method of outer itself: a control
  function int shifted();
    return tag + 1;
  endfunction

  // The nested object is built and used inside the outer class.
  task go(output int r);
    inner ic = new();
    ic.bump(4);
    r = ic.v;
  endtask
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
    int          r;
    outer        o;
    outer::inner ic;

    o  = new();
    ic = new();

    o.go(r);
    check("task called inside the outer class",   r,                       5);
    ic.bump(9);
    check("task called from the module",          ic.v,                    10);
    check("function returns its value",           ic.twice(21),            42);
    check("static method",                        outer::inner::triple(5), 15);
    check("control: inline nested method",        ic.plus_one(1),          2);
    check("control: method of the outer class",   o.shifted(),             8);

    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
