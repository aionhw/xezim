// IEEE 1800-2017 Ch.8 — classes
module tb;
  int fails = 0;
  `define CK(name, cond) if (!(cond)) begin $display("FAIL[8] %s", name); fails++; end

  class Base;
    int x = 1;
    static int count = 0;
    function new(int v = 1); x = v; count++; endfunction
    virtual function int get(); return x; endfunction
    function int base_get(); return x; endfunction
  endclass

  class Derived extends Base;
    int y = 2;
    function new(int v = 5); super.new(v + 1); y = v; endfunction
    virtual function int get(); return x + y; endfunction
    function int sup(); return super.get(); endfunction
  endclass

  class Param #(type T = int, int W = 8);
    T val;
    function int width(); return W; endfunction
  endclass

  initial begin
    Base b; Derived d; Base bd;
    Param #(byte, 16) p;
    `CK("null default", b == null)
    b = new(7);
    `CK("ctor arg", b.x == 7)
    d = new(3);
    `CK("super.new chain", d.x == 4)
    `CK("derived prop", d.y == 3)
    bd = d;
    `CK("virtual dispatch via base", bd.get() == 7)
    `CK("super.method", d.sup() == 4)  // §8.15: super.get() is Base::get, x only
    `CK("static prop", Base::count == 2)
    begin
      Derived dcast;
      `CK("$cast downcast ok", $cast(dcast, bd) == 1)
      `CK("cast result", dcast.y == 3)
      bd = new(1);
      `CK("$cast fails on base obj", $cast(dcast, bd) == 0)
    end
    p = new();
    `CK("param class W", p.width() == 16)
    p.val = 8'h7F;
    `CK("param class T", p.val == 127)
    begin // 8.12 handle equality, shallow copy
      Base b1, b2;
      b1 = new(9);
      b2 = b1;
      `CK("handle alias", b2.x == 9)
      b2.x = 11;
      `CK("aliased write", b1.x == 11)
      b2 = new b1;   // shallow copy
      b2.x = 20;
      `CK("shallow copy independent", b1.x == 11 && b2.x == 20)
    end
    $display("CH8 CHECKS DONE fails=%0d", fails);
  end
endmodule
