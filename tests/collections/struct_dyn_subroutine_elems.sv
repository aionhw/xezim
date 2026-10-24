// Element writes into the dynamic members of subroutine-local and formal
// unpacked structs: functions and tasks, automatic and static, byte and int
// elements, read back inside the subroutine and handed out by `return` or an
// `output` formal.
module top;
  typedef struct { longint l; byte unsigned d[]; } bs_t;
  typedef struct { int tag; int d[]; int q[$]; } is_t;
  bs_t m;
  is_t mi;

  function automatic bs_t fill_auto(int n);
    bs_t a;
    a.d = new[n];
    for (int i = 0; i < n; i++) a.d[i] = i + 65;
    return a;
  endfunction
  function bs_t fill_static(int n);
    bs_t a;
    a.d = new[n];
    for (int i = 0; i < n; i++) a.d[i] = i + 97;
    return a;
  endfunction
  function automatic int read_back(int n);
    bs_t a;
    a.d = new[n];
    a.d[0] = 42;
    a.d[n - 1] = 43;
    return a.d[0] * 100 + a.d[n - 1];
  endfunction
  function int read_back_static(int n);
    is_t a;
    a.d = new[n];
    a.d[1] = -7;
    a.q.push_back(1);
    a.q[0] = 9;
    return a.d[1] * 10 + a.q[0];
  endfunction
  task automatic make_auto(input int n, output is_t o);
    is_t t;
    t.tag = n;
    t.d = new[n];
    foreach (t.d[i]) t.d[i] = i * 11;
    t.q.push_back(0);
    t.q[0] = 5;
    $display("T|task-local %0d %0d %0d", t.d[n - 1], t.q[0], t.d.size());
    o = t;
  endtask
  task make_static(input int n, output is_t o);
    o.tag = n;
    o.d = new[n];
    for (int i = 0; i < n; i++) o.d[i] = 1000 + i;
    $display("T|task-output %0d %0d", o.d[0], o.d[n - 1]);
  endtask
  function automatic int formal_write(is_t a);
    a.d[0] = 77;
    a.q[0] = 88;
    return a.d[0] + a.q[0];
  endfunction

  initial begin
    m = fill_auto(3);
    $display("T|fill-auto %0d %0d %0d %0d", m.d.size(), m.d[0], m.d[1], m.d[2]);
    m = fill_static(2);
    $display("T|fill-static %0d %0d %0d", m.d.size(), m.d[0], m.d[1]);
    $display("T|read-back %0d %0d", read_back(2), read_back_static(3));
    make_auto(3, mi);
    $display("T|make-auto %0d %0d %0d %0d %0d", mi.tag, mi.d.size(), mi.d[2], mi.q.size(),
             mi.q[0]);
    make_static(2, mi);
    $display("T|make-static %0d %0d %0d %0d", mi.tag, mi.d.size(), mi.d[0], mi.d[1]);
    mi.d = new[1];
    mi.q = {3};
    $display("T|formal %0d caller %0d %0d", formal_write(mi), mi.d[0], mi.q[0]);
  end
endmodule
