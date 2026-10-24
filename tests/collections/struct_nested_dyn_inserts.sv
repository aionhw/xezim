// Function results with nested dynamic members inserted into a queue, and
// task output formals of a struct holding a dynamic array of structs.
module t;
  typedef struct { longint l; longint r; bit dn; byte unsigned d[]; } A;
  typedef struct { longint l; longint r; bit dn; A d[]; } AA;
  function automatic A mk(longint r, byte unsigned t[]);
    A a; a.l = 0; a.r = r; a.dn = 0; a.d = t; return a;
  endfunction
  task automatic mk2t(input longint r, input A t[], output AA o);
    o.r = r; o.d = t;
  endtask
  task automatic cpt(input AA i, output AA o);
    o = i;
  endtask
  A qa[$];
  AA x, y;
  A rows[];
  initial begin
    byte unsigned t[];
    t = new[2];
    t[0] = 9; t[1] = 8;
    qa.push_front(mk(1, t));
    qa.push_front(mk(2, t));
    qa.insert(1, mk(3, t));
    $display("T|front %0d %0d %0d %0d", qa.size(), qa[0].r, qa[1].r, qa[2].r);
    $display("T|front-d %0d %0d %0d", qa[0].d.size(), qa[1].d.size(), qa[2].d[1]);
    rows = new[2];
    rows[1] = mk(4, t);
    mk2t(6, rows, x);
    $display("T|task-out %0d %0d %0d %0d", x.r, x.d.size(), x.d[1].r, x.d[1].d[0]);
    cpt(x, y);
    $display("T|task-copy %0d %0d %0d", y.r, y.d[1].r, y.d[1].d.size());
  end
endmodule
