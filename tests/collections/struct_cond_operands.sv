// Conditional expressions whose operands are unpacked structs with a
// dynamic member: as an actual, assigned, returned, nested, with variable
// operands, into an array element and a queue.
module t;
  typedef struct { longint l; longint r; bit dn; byte unsigned d[]; } A;
  function automatic A mk(string s);
    A a; byte unsigned t[]; t = new[s.len()]; foreach (t[i]) t[i] = s.getc(i);
    a.l = 1; a.r = s.len(); a.dn = 0; a.d = t; return a;
  endfunction
  function automatic string str(A a);
    string o = "";
    foreach (a.d[i]) o = {o, string'(a.d[i])};
    return $sformatf("%0d:%s", a.r, o);
  endfunction
  function automatic A pick(int c);
    return (c == 0) ? mk("zero") : mk("other");
  endfunction
  function automatic A pick3(int c);
    return (c == 0) ? mk("a") : (c == 1) ? mk("bb") : mk("ccc");
  endfunction
  A x, v, w;
  function automatic A pickv(int c);
    A loc;
    loc = mk("local");
    return c ? loc : w;
  endfunction
  function automatic A rw();
    return w;
  endfunction
  A arr[];
  A q[$];
  initial begin
    int d;
    d = 0;
    v = mk("vee"); w = mk("double");
    $display("T|arg %s %s", str((d == 0) ? mk("abc") : mk("de")), str((d != 0) ? mk("abc") : mk("de")));
    x = (d == 0) ? mk("assign") : mk("no");
    $display("T|assign %s", str(x));
    $display("T|return %s %s", str(pick(0)), str(pick(1)));
    $display("T|nested %s %s %s", str(pick3(0)), str(pick3(1)), str(pick3(2)));
    x = d ? v : w;
    $display("T|var %s %s", str(x), str(!d ? v : w));
    $display("T|return-var %s %s", str(pickv(1)), str(pickv(0)));
    $display("T|return-module %s", str(rw()));
    x = (d == 0) ? ((d == 1) ? mk("p") : mk("qq")) : mk("r");
    $display("T|nested-assign %s", str(x));
    arr = new[1];
    arr[0] = d ? v : mk("elem");
    q.push_back(!d ? v : w);
    $display("T|elem %s %s", str(arr[0]), str(q[0]));
  end
endmodule
