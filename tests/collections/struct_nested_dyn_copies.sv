// Dynamic arrays of structs whose elements have dynamic members (AA.d of
// A), copied whole: assignment, return, formals, deeper nesting, element
// writes from calls and queues.
module t;
  typedef struct { longint l; longint r; bit dn; byte unsigned d[]; } A;
  typedef struct { longint l; longint r; bit dn; A d[]; } AA;
  typedef struct { int tag; AA m; } W;
  function automatic A mk(longint r, byte unsigned t[]);
    A a; a.l = 0; a.r = r; a.dn = 0; a.d = t; return a;
  endfunction
  function automatic AA mk2(longint r, A t[]);
    AA a; a.l = 0; a.r = r; a.dn = 0; a.d = t; return a;
  endfunction
  function automatic W mkw(int tag, AA m);
    W w; w.tag = tag; w.m = m; return w;
  endfunction
  function automatic W idw(W w); return w; endfunction
  function automatic AA cp(AA x);
    AA y;
    y = x;
    return y;
  endfunction
  function automatic byte unsigned at(A x, longint i);
    return x.d[i - x.l];
  endfunction
  A rows[];
  AA g, h;
  AA ga[];
  W w, w2;
  A qa[$];
  AA qaa[$];
  A pa;
  AA paa;
  initial begin
    byte unsigned t[];
    t = new[3];
    foreach (t[i]) t[i] = 40 + i;
    rows = new[2];
    rows[0] = mk(1, t);
    rows[1] = mk(2, t);
    $display("T|rows %0d %0d %0d", rows[1].r, rows[1].d.size(), rows[1].d[2]);
    g = mk2(5, rows);
    h = cp(g);
    $display("T|cp %0d %0d %0d %0d", h.r, h.d.size(), h.d[1].r, at(h.d[1], 2));
    h = g;
    $display("T|copy %0d %0d %0d %0d", h.r, h.d.size(), h.d[1].r, h.d[1].d[1]);
    w = mkw(3, g);
    w2 = idw(w);
    $display("T|deep %0d %0d %0d %0d", w2.tag, w2.m.d.size(), w2.m.d[1].r, w2.m.d[1].d[2]);
    w2 = w;
    $display("T|deep-asg %0d %0d %0d", w2.m.d.size(), w2.m.d[0].d.size(), w2.m.d[0].d[0]);
    ga = new[2];
    ga[1] = mk2(6, rows);
    $display("T|dyn-of-AA %0d %0d %0d", ga[1].r, ga[1].d[1].r, ga[1].d[1].d.size());
    ga[0] = g;
    $display("T|dyn-of-AA-var %0d %0d", ga[0].d[0].r, ga[0].d[0].d[2]);
    qa.push_back(mk(7, t));
    qa.push_back(rows[0]);
    pa = qa.pop_front();
    $display("T|queue-A %0d %0d %0d %0d", pa.r, pa.d.size(), pa.d[1], qa.size());
    qaa.push_back(mk2(8, rows));
    qaa.push_back(g);
    paa = qaa.pop_front();
    $display("T|queue-AA %0d %0d %0d %0d", paa.r, paa.d[1].r, paa.d[1].d.size(), qaa.size());
    $display("T|queue-AA-elem %0d %0d", qaa[0].d[0].r, qaa[0].d[0].d[2]);
  end
endmodule
