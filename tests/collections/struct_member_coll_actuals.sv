// A dynamic-array or queue member of a struct passed as a collection actual.
module top;
  typedef struct { longint l; longint r; bit dn; int d[]; } A;
  typedef struct { int k; int q[$]; } Q;
  typedef struct { int n; Q in; } N;

  function automatic A from_(longint l, bit dn, int t[]);
    A a;
    a.l = l;
    a.dn = dn;
    a.r = l + t.size() - 1;
    a.d = t;
    return a;
  endfunction
  function automatic A mk(int n, int base);
    int t[];
    t = new[n];
    foreach (t[i]) t[i] = base + i;
    return from_(0, 0, t);
  endfunction
  function automatic A conv(A tgt, A v);
    return from_(tgt.l, tgt.dn, v.d);
  endfunction
  function automatic int total(int t[]);
    int s = 0;
    foreach (t[i]) s += t[i];
    return s;
  endfunction
  function automatic int qsum(int t[$]);
    int s = 0;
    foreach (t[i]) s += t[i];
    return s * 100 + t.size();
  endfunction
  function automatic int member_total(A v);
    return total(v.d);
  endfunction
  function automatic void bump(ref int t[]);
    t[0] += 100;
  endfunction
  task automatic grow(ref int t[$]);
    t.push_back(t.size() + 1);
  endtask
  task automatic fill(output int t[]);
    t = new[2];
    t[1] = 55;
  endtask

  initial begin
    A x;
    Q s;
    N nn;
    x = conv(mk(3, 0), mk(3, 20));
    $display("T|conv %0d %0d %0d %0d", x.d.size(), x.d[0], x.d[2], x.r);
    $display("T|input %0d %0d", total(x.d), member_total(x));
    bump(x.d);
    $display("T|ref %0d %0d", x.d[0], x.d.size());
    fill(x.d);
    $display("T|output %0d %0d %0d", x.d.size(), x.d[0], x.d[1]);
    s.q = {4, 5};
    grow(s.q);
    $display("T|queue %0d %0d", s.q.size(), qsum(s.q));
    nn.in.q = {7};
    grow(nn.in.q);
    $display("T|nested %0d %0d", nn.in.q.size(), qsum(nn.in.q));
  end
endmodule
