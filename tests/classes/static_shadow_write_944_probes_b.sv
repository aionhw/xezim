// handle-path neighbours: base-typed / derived-typed references of non-variable shape
class B;
  int x;
  B child;
  B pool[$];
  static B slot;
  function int get(); return x; endfunction
  function void set_child(int v); this.child.x = v; endfunction
  function int get_child(); return this.child.x; endfunction
  function void set_pool(int v); this.pool[0].x = v; endfunction
  function int get_pool(); return this.pool[0].x; endfunction
endclass
class DS extends B;
  static int x = 27;
  DS dchild;
  function void set_dchild(int v); this.dchild.x = v; endfunction
  function int get_dchild(); return this.dchild.x; endfunction
endclass
class DL extends B;
  localparam x = 27;
endclass
class Box; static B sb; static DS sd; endclass
module top;
  initial begin
    B qb[$]; DS qd[$]; B ab[string]; DS ad[string];
    DS ds = new; DS ds2 = new; DL dl = new; DS h;
    qb.push_back(ds); qd.push_back(ds2); ab["k"] = ds; ad["k"] = ds2;
    // reads through base-typed non-variable references hit the instance
    ds.x = 0; // static
    DS::x = 27;
    $display("T|R1 qb[0].x=%0d ab=%0d qd[0].x=%0d ad=%0d", qb[0].x, ab["k"].x, qd[0].x, ad["k"].x);
    qb[0].x = 3;     // instance of ds
    $display("T|W1 ds.get=%0d DS::x=%0d", ds.get(), DS::x);
    qd[0].x = 4;     // static
    $display("T|W2 ds2.get=%0d DS::x=%0d", ds2.get(), DS::x);
    DS::x = 27;
    ab["k"].x = 5;   // instance
    ad["k"].x = 6;   // static
    $display("T|W3 ds.get=%0d ds2.get=%0d DS::x=%0d", ds.get(), ds2.get(), DS::x);
    DS::x = 27;
    Box::sb = ds; Box::sd = ds2;
    Box::sb.x = 7; Box::sd.x = 8;
    $display("T|W4 ds.get=%0d ds2.get=%0d DS::x=%0d sb.x=%0d", ds.get(), ds2.get(), DS::x, Box::sb.x);
    DS::x = 27;
    ds.child = ds2; ds.dchild = ds2;
    ds.set_child(9);
    $display("T|W5 ds2.get=%0d DS::x=%0d get_child=%0d", ds2.get(), DS::x, ds.get_child());
    ds.set_dchild(10);
    $display("T|W6 ds2.get=%0d DS::x=%0d get_dchild=%0d", ds2.get(), DS::x, ds.get_dchild());
    DS::x = 27;
    ds.pool.push_back(ds2);
    ds.set_pool(11);
    $display("T|W7 ds2.get=%0d DS::x=%0d get_pool=%0d", ds2.get(), DS::x, ds.get_pool());
    foreach (qb[i]) qb[i].x = 12;
    foreach (qd[i]) qd[i].x = 13;
    $display("T|W8 ds.get=%0d ds2.get=%0d DS::x=%0d", ds.get(), ds2.get(), DS::x);
    DS::x = 27;
    ds.child = dl; ds.set_child(14);
    $display("T|W9 dl.get=%0d get_child=%0d", dl.get(), ds.get_child());
  end
endmodule
