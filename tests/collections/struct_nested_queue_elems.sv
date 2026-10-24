// Queues of structs with a nested unpacked struct member: whole-struct items
// of an assignment pattern, and members of a popped element.
module top;
  typedef struct { int f0; byte unsigned f2; } R;
  typedef struct { longint t; R v; } TR;
  TR q[$];
  TR e;
  R x, y;

  function automatic R mkr(int a, byte unsigned b);
    R r;
    r.f0 = a;
    r.f2 = b;
    return r;
  endfunction
  function automatic void push(R v, longint tt);
    TR qq[$];
    qq = q;
    qq.push_back('{tt, v});
    q = qq;
  endfunction

  initial begin
    x = '{-1, 113};
    push(x, 5);
    $display("T|push %0d %0d %0d %0d", q.size(), q[0].t, q[0].v.f0, q[0].v.f2);
    e = '{6, x};
    $display("T|var %0d %0d %0d", e.t, e.v.f0, e.v.f2);
    q.push_back('{7, mkr(-3, 9)});
    q.push_back('{t: 8, v: x});
    $display("T|items %0d %0d %0d %0d", q[1].v.f0, q[1].v.f2, q[2].t, q[2].v.f0);
    q.push_back(e);
    y = q.pop_front().v;
    $display("T|pop-member %0d %0d %0d", y.f0, y.f2, q.size());
    $display("T|pop-leaf %0d %0d %0d", q.pop_front().v.f0, q.pop_front().t, q.size());
    y = q.pop_back().v;
    $display("T|pop-back %0d %0d %0d", y.f0, y.f2, q.size());
  end
endmodule
