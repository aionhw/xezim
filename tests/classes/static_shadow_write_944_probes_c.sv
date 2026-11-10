// instance-vs-instance shadow through references (§8.14)
class B; int x; B nxt;
  function int bget(); return x; endfunction
  function void bset(int v); x = v; endfunction
endclass
class D extends B; int x;
  function int dget(); return x; endfunction
endclass
module top;
  initial begin
    D d = new; B b = d; B q[$]; D d2 = new;
    b.x = 5;
    $display("T|I1 b.x=%0d d.x=%0d bget=%0d dget=%0d", b.x, d.x, d.bget(), d.dget());
    d.x = 6;
    $display("T|I2 b.x=%0d d.x=%0d bget=%0d dget=%0d", b.x, d.x, d.bget(), d.dget());
    q.push_back(d); q[0].x = 7;
    $display("T|I3 q0.x=%0d bget=%0d dget=%0d", q[0].x, d.bget(), d.dget());
    d.nxt = d2; d.nxt.x = 8;
    $display("T|I4 nxt.x=%0d bget=%0d dget=%0d", d.nxt.x, d2.bget(), d2.dget());
  end
endmodule
