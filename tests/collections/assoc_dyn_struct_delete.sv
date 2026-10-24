// delete(index), exists, num/size and first/next after deletes, on
// associative arrays of structs (with and without a dynamic member) and of
// ints, with longint and int keys.
module t;
  typedef struct { longint l; longint r; bit dn; byte unsigned d[]; } A;
  typedef struct { int a; int b; } P;
  A heap[longint];
  A hi[int];
  P ph[longint];
  int ih[longint];
  int ii[int];
  function automatic A mk(string s);
    A a; byte unsigned t[]; t = new[s.len()]; foreach (t[i]) t[i] = s.getc(i);
    a.l = 1; a.r = s.len(); a.dn = 0; a.d = t; return a;
  endfunction
  function automatic void put(longint k, A v); heap[k] = v; endfunction
  initial begin
    longint k; int j; A x;
    x = mk("xy");
    heap[1] = mk("abc"); heap[2] = x; put(3, mk("q")); heap[5] = mk("zz");
    heap.delete(2);
    $display("T|heap %0d %0d %0d %0d", heap.exists(2), heap.exists(1), heap.num(), heap.size());
    heap.delete(1);
    $display("T|heap2 %0d %0d %0d", heap.exists(1), heap.exists(3), heap.num());
    k = 0; void'(heap.first(k)); $display("T|heap-first %0d %0d", k, heap[k].d.size());
    void'(heap.next(k)); $display("T|heap-next %0d %0d", k, heap[k].d[1]);
    heap.delete(3);
    $display("T|heap3 %0d %0d", heap.exists(3), heap.num());
    hi[7] = mk("seven"); hi[-2] = mk("m");
    hi.delete(7);
    j = 0; void'(hi.first(j));
    $display("T|int-key %0d %0d %0d %0d", hi.exists(7), hi.num(), j, hi[j].d.size());
    ph[4] = '{1, 2}; ph[9] = '{3, 4};
    ph.delete(4);
    $display("T|plain-struct %0d %0d", ph.exists(4), ph.num());
    ih[10] = 1; ih[20] = 2; ih.delete(10);
    ii[3] = 1; ii[-3] = 2; ii.delete(-3);
    $display("T|ints %0d %0d %0d %0d", ih.exists(10), ih.num(), ii.exists(-3), ii.num());
    heap.delete();
    $display("T|all %0d %0d", heap.num(), heap.exists(5));
  end
endmodule
