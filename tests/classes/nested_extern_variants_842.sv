// IEEE 1800-2023 8.23, 8.24: the same construct as repro.sv (a method of a nested
// class, declared `extern` in the nested class and defined out of block in the body
// of the enclosing class) in other settings: the enclosing class in a package, a
// constructor, a virtual method called through a base-class handle, and an
// enclosing class with a type parameter. Every line should print PASS. With the bug
// each body is dropped: the call returns the default value, with no diagnostic.
package pk;
  // a: the enclosing class is in a package
  class outer_a;
    class inner;
      int v = 0;
      extern function int twice(int x);
    endclass
    function int inner::twice(int x);
      return 2 * x;
    endfunction
  endclass

  // b: a constructor and a method that reads what it set
  class outer_b;
    class inner;
      int k;
      extern function new(int k);
      extern function int doubled();
    endclass
    function inner::new(int k);
      this.k = k;
    endfunction
    function int inner::doubled();
      return 2 * k;
    endfunction
  endclass

  // c: a virtual method, called through a handle of the base class
  class base;
    virtual function int who();
      return 1;
    endfunction
  endclass
  class outer_c;
    class inner extends base;
      extern virtual function int who();
    endclass
    function int inner::who();
      return 2;
    endfunction
  endclass

  // d: the enclosing class has a type parameter
  class outer_d #(type T = int);
    class inner;
      T v;
      extern function T get();
    endclass
    function T inner::get();
      return v;
    endfunction
  endclass
endpackage

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
    pk::outer_a::inner       a;
    pk::outer_b::inner       b;
    pk::outer_c::inner       c;
    pk::base                 h;
    pk::outer_d#(int)::inner d;

    a = new();
    b = new(6);
    c = new();
    h = c;
    d = new();

    check("a: enclosing class in a package",          a.twice(21),  42);
    check("b: constructor body sets the member",       b.k,          6);
    check("b: method reads the member",                b.doubled(),  12);
    check("c: virtual method, nested handle",          c.who(),      2);
    check("c: virtual method, base handle",            h.who(),      2);
    d.v = 11;
    check("d: enclosing class with a type parameter",  d.get(),      11);

    if (fails == 0) $display("ALL PASS"); else $display("%0d FAILED", fails);
    $finish;
  end
endmodule
