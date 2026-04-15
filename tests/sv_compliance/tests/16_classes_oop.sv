`include "../common/svtest_defs.svh"

class base_c;
  int v;
  function new(int x = 1);
    v = x;
  endfunction
  virtual function int f();
    return v;
  endfunction
endclass

class derived_c extends base_c;
  function new(int x = 2);
    super.new(x);
  endfunction
  function int f();
    return v + 10;
  endfunction
endclass

module test_classes_oop;
  `SVTEST_INIT

  base_c b;
  derived_c d;

  initial begin
    d = new(5);
    b = d;

    `SVTEST_CHECK(b.f() == 15, "virtual dispatch through base handle failed")
    `SVTEST_CHECK(d.f() == 15, "derived class method failed")

    `SVTEST_PASSFAIL
  end
endmodule
