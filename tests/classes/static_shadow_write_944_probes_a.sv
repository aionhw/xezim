// neighbours of the shadow-write family
class B;
  rand int x;
  function void set(int v); this.x = v; endfunction
  function int get(); return x; endfunction
  function int get_this(); return this.x; endfunction
endclass
class DS extends B;
  static int x = 27;
  function int d_get(); return x; endfunction
  function int d_get_this(); return this.x; endfunction
  function void d_set(int v); this.x = v; endfunction
endclass
class DL extends B;
  localparam x = 27;
  function int d_get(); return x; endfunction
  function int d_get_this(); return this.x; endfunction
  function int d_get_super(); return super.x; endfunction
endclass
class E extends DS;   // grandchild, writes through this: nearest is DS static
  function void e_set(int v); this.x = v; endfunction
  function int e_get(); return x; endfunction
endclass
// inverse: base static, derived instance
class A2;
  static int y = 5;
  function void a_set(int v); this.y = v; endfunction
  function int a_get(); return y; endfunction
endclass
class B2 extends A2;
  int y;
  function void b_set(int v); this.y = v; endfunction
  function int b_get(); return y; endfunction
endclass
module top;
  initial begin
    DS ds = new; DL dl = new; E e = new; B b; DS dq[$]; A2 a2; B2 b2 = new;
    int ok;
    // N1 read via derived-typed handle of a static shadow
    $display("T|N1 ds.x=%0d", ds.x);
    // N2 write via derived-typed handle -> static
    ds.x = 5;
    $display("T|N2 ds.x=%0d DS::x=%0d get=%0d", ds.x, DS::x, ds.get());
    DS::x = 27;
    // N3 reads in the derived class
    $display("T|N3 dl.d_get=%0d d_get_this=%0d d_get_super=%0d ds.d_get=%0d ds.d_get_this=%0d",
             dl.d_get(), dl.d_get_this(), dl.d_get_super(), ds.d_get(), ds.d_get_this());
    // N4 derived-typed queue element read
    dq.push_back(ds);
    $display("T|N4 dq[0].x=%0d", dq[0].x);
    // N5 derived method this-write to static
    ds.d_set(8);
    $display("T|N5 DS::x=%0d get=%0d", DS::x, ds.get());
    DS::x = 27;
    // N6 grandchild this-write -> DS static
    e.e_set(9);
    $display("T|N6 DS::x=%0d e.get=%0d e_get=%0d", DS::x, e.get(), e.e_get());
    DS::x = 27;
    // N7 base-typed read of D object
    b = ds; ds.set(3);
    $display("T|N7 b.x=%0d get_this=%0d", b.x, ds.get_this());
    // N8 inverse shape
    b2.a_set(11); b2.b_set(12);
    $display("T|N8 A2::y=%0d a_get=%0d b_get=%0d b2.y=%0d", A2::y, b2.a_get(), b2.b_get(), b2.y);
    a2 = b2; a2.y = 13;
    $display("T|N8b A2::y=%0d b_get=%0d a2.y=%0d", A2::y, b2.b_get(), a2.y);
    // N9 inline constraint through base-typed / derived-typed handle
    b = dl;
    ok = b.randomize() with { x == 15; };
    $display("T|N9 ok=%0d get=%0d", ok, dl.get());
    ok = dl.randomize() with { x == 16; };
    $display("T|N9b ok=%0d get=%0d", ok, dl.get());
  end
endmodule
