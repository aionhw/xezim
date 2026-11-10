// A class declared in a module hides a compilation-unit ($unit) class of the same name inside the
// module (IEEE 1800-2023 3.12.1, 23.9, 3.13). Every line should print PASS.
//
//   xezim --simulate -s top --no-cache repro.sv
//
// With the bug the module's `base` is not what the module's code names: `extends base`, `new`, a
// variable of type `base`, a typedef of it and `base::` all reach the compilation unit's class.
// The compilation unit's own code (class `holder`) names its class, as it must.

class base;
  int x = 5;
  static int sv = 50;
  static function int sf(); return 500; endfunction
endclass

class holder;                       // compilation-unit code: its `base` is the compilation unit's
  base b;
  function new(); b = new(); endfunction
endclass

module top;
  class base;                       // hides the compilation unit's `base` inside this module
    int x = 7;
    static int sv = 70;
    static function int sf(); return 700; endfunction
  endclass
  class derived extends base;
    function int get(); return x; endfunction
  endclass
  typedef base bt;

  task automatic chk(string what, int got, int exp);
    if (got === exp) $display("PASS %s", what);
    else $display("FAIL %s: got %0d, expected %0d", what, got, exp);
  endtask

  initial begin
    derived d = new();
    base    b = new();
    bt      t = new();
    holder  h = new();
    chk("extends base: inherited x",     d.get(),    7);
    chk("variable of type base, new",    b.x,        7);
    chk("typedef of base",               t.x,        7);
    chk("base::sv (static data member)", base::sv,   70);
    chk("base::sf() (static method)",    base::sf(), 700);
    chk("compilation-unit class holder", h.b.x,      5);
  end
endmodule
