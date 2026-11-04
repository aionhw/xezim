//! IEEE 1800-2023 §13.5 argument passing for collections and `ref` formals
//! (LRM-audit High findings). Expected values come from the reference
//! simulator.
//!
//! - §13.5.1: a queue / dynamic-array / associative `input` formal is a COPY.
//!   The formal lived under its bare name, so an actual with the formal's own
//!   name WAS the formal and the callee's writes reached the caller.
//! - §13.5.2: a `ref` queue formal is an ALIAS. It was copy-in/copy-out, so
//!   two concurrently forked tasks pushing into one `ref` queue lost a push.
//! - §13.5.2: `@(posedge sig)` on a `ref` formal waits on the actual; it
//!   resolved to no signal and returned in the same time step.
//! - §7.10 / §13.5: a queue slice used directly as an actual (`q[1:$]`)
//!   bound as a one-element scalar, so a recursive `rsum(q[1:$])` never
//!   reached its base case and overflowed the stack.
//!
//! Expectations updated for #566: `%p` prints vector members as `'h` hex,
//! a packed union's first member, `%0p` as `%p`, and no space before an
//! associative array's closing brace.

use xezim::simulate;

fn tagged(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000_000).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.trim().to_string())
        .filter(|l| l.starts_with("T|"))
        .collect()
}

/// Audit repro `13.5.1_dynarray_by_value`: formals named like their actuals.
#[test]
fn dynamic_array_and_queue_inputs_are_copies() {
    let src = r#"
module fdv;
  function automatic void modv(int d[]); d[0] = 99; endfunction
  function automatic void modq(int q[$]); q[0] = 98; q.push_back(5); endfunction
  int d[]; int q[$];
  initial begin
    d = '{1, 2}; q = {1, 2};
    modv(d); modq(q);
    $display("T|13.5.1d|d=%p q=%p", d, q);
  end
endmodule
"#;
    assert_eq!(tagged(src), vec!["T|13.5.1d|d='{1, 2} q='{1, 2}"]);
}

/// §13.5.1 across collection kinds, tasks, output/inout and class methods.
#[test]
fn collection_formals_by_value_family() {
    let src = r#"
module p1;
  function automatic void modv(int d[]); d[0] = 99; endfunction
  function automatic void modq(int q[$]); q[0] = 98; q.push_back(5); endfunction
  function automatic void moda(int a[string]); a["x"] = 77; a["new"] = 1; endfunction
  function automatic void mods(string s); s = "changed"; endfunction
  task automatic tmodv(int d[]); d[0] = 91; #1; d[1] = 92; endtask
  task automatic tmodq(int q[$]); q[0] = 93; q.push_back(6); #1; endtask
  task automatic tmoda(int a[string]); a["x"] = 78; #1; endtask
  function automatic void outq(output int q[$]); q.push_back(7); endfunction
  function automatic void outv(output int d[]); d = new[3]; d[2] = 8; endfunction
  function automatic void inoutq(inout int q[$]); q.push_back(9); endfunction
  function automatic int sumq(int q[$]); int s = 0; foreach (q[i]) s += q[i]; q.delete(); return s; endfunction
  class C;
    function void cm(int q[$]); q[0] = 55; q.push_back(56); endfunction
    function void cmv(int d[]); d[0] = 57; endfunction
    task tc(int q[$]); q.push_back(58); #1; endtask
  endclass
  int d[]; int q[$]; int a[string]; string s; int oq[$]; int ov[]; int ioq[$];
  C c;
  initial begin
    d = '{1, 2}; q = {1, 2}; a["x"] = 3; s = "orig";
    modv(d); modq(q); moda(a); mods(s);
    $display("T|f|d=%p q=%p a=%p s=%s", d, q, a, s);
    tmodv(d); tmodq(q); tmoda(a);
    $display("T|t|d=%p q=%p a=%p", d, q, a);
    oq = {1}; outq(oq); ov = '{4, 4}; outv(ov); ioq = {3}; inoutq(ioq);
    $display("T|o|oq=%p ov=%p ioq=%p", oq, ov, ioq);
    $display("T|s|%0d q=%p", sumq(q), q);
    c = new; c.cm(q); c.cmv(d); c.tc(q);
    $display("T|c|q=%p d=%p", q, d);
  end
endmodule
"#;
    assert_eq!(
        tagged(src),
        vec![
            r#"T|f|d='{1, 2} q='{1, 2} a='{"x":3} s=orig"#,
            r#"T|t|d='{1, 2} q='{1, 2} a='{"x":3}"#,
            "T|o|oq='{7} ov='{0, 0, 8} ioq='{3, 9}",
            "T|s|3 q='{1, 2}",
            "T|c|q='{1, 2} d='{1, 2}",
        ]
    );
}

/// More §13.5 shapes: a by-value copy passed on by `ref`, ref chains, element
/// types (string, class handle, unpacked struct), an actual read after its
/// same-named formal is bound, `new[]` on a copy, associative output / inout /
/// ref, and a task-local queue shared by `ref` among forked tasks.
#[test]
fn collection_formals_mixed_shapes() {
    let src = r#"
module p7;
  typedef struct { int a; string s; } rec_t;
  class Obj; int v; function new(int x); v = x; endfunction endclass
  function automatic void gref(ref int q[$]); q.push_back(100); q[0] = -1; endfunction
  function automatic int fval(int q[$]); gref(q); return q.size() * 1000 + q[0] + 5000; endfunction
  task automatic t2(ref int q[$]); #1 q.push_back(7); endtask
  task automatic t1(ref int q[$]); q.push_back(6); t2(q); endtask
  task automatic same(ref int lg[$]); #1 lg.push_back(8); endtask
  function automatic void strq(string q[$]); q[0] = "zz"; q.push_back("yy"); endfunction
  function automatic int objq(Obj q[$]); q[0] = new(99); return q.size(); endfunction
  function automatic void recq(rec_t q[$]); q[0].a = 55; q[0].s = "mod"; endfunction
  function automatic void recq_ref(ref rec_t q[$]); q[0].a = 66; q.push_back('{7, "new"}); endfunction
  function automatic int both(int q[$], int n); return q.size() * 100 + n; endfunction
  function automatic void newd(int d[]); d = new[5]; d[4] = 1; endfunction
  function automatic void ao(output int a[int]); a[9] = 9; endfunction
  function automatic void aio(inout int a[int]); a[8] = 8; endfunction
  function automatic void aref(ref int a[int]); a[7] = 7; endfunction
  task automatic local_fork(output int n);
    int lq[$];
    fork t2(lq); t2(lq); same(lq); join
    n = lq.size();
  endtask
  int lg[$]; int q[$]; string sq[$]; Obj oq[$]; rec_t rq[$]; int d[]; int aa[int]; int n;
  initial begin
    q = {1, 2};
    $display("T|a|%0d q=%p", fval(q), q);
    q = {};
    t1(q); $display("T|b|q=%p", q);
    lg = {1}; fork same(lg); same(lg); join $display("T|c|lg=%p", lg);
    sq = {"a", "b"}; strq(sq); $display("T|d|%p", sq);
    begin Obj o0 = new(1); oq.push_back(o0); end $display("T|e|%0d %0d", objq(oq), oq[0].v);
    rq.push_back('{1, "orig"}); recq(rq); $display("T|f|%0d %s", rq[0].a, rq[0].s);
    recq_ref(rq); $display("T|g|%0d %0d %s", rq.size(), rq[0].a, rq[1].s);
    q = {4, 5, 6}; $display("T|h|%0d", both(q, q.size()));
    d = '{3, 4}; newd(d); $display("T|i|%p", d);
    aa[1] = 1; ao(aa); $display("T|j|%p", aa);
    aa[1] = 1; aio(aa); $display("T|k|%p", aa);
    aref(aa); $display("T|l|%p", aa);
    local_fork(n); $display("T|m|%0d", n);
  end
endmodule
"#;
    assert_eq!(
        tagged(src),
        vec![
            "T|a|7999 q='{1, 2}",
            "T|b|q='{6, 7}",
            "T|c|lg='{1, 8, 8}",
            r#"T|d|'{"a", "b"}"#,
            "T|e|1 1",
            "T|f|1 orig",
            "T|g|2 66 new",
            "T|h|303",
            "T|i|'{3, 4}",
            "T|j|'{9:9}",
            "T|k|'{1:1, 8:8, 9:9}",
            "T|l|'{1:1, 7:7, 8:8, 9:9}",
            "T|m|3",
        ]
    );
}

/// Audit repro `13.5.2_ref_queue_concurrent`: a `ref` queue shared by
/// concurrently forked tasks keeps both pushes.
#[test]
fn ref_queue_shared_by_forked_tasks() {
    let src = r#"
module rrc;
  string lg[$];
  int cnt;
  task automatic push(int d, string nm, ref string q[$]); #d q.push_back(nm); endtask
  task automatic incr(int d, ref int c); #d c = c + 1; endtask
  initial begin
    fork
      push(3, "A", lg);
      push(1, "B", lg);
    join
    $display("T|r1|%p", lg);
    cnt = 0;
    fork
      incr(3, cnt);
      incr(1, cnt);
    join
    $display("T|r2|%0d", cnt);
    cnt = 0;
    fork
      incr(2, cnt);
      #1 $display("T|r3|during=%0d", cnt);
      #3 $display("T|r4|after=%0d", cnt);
    join
    fork
      incr(2, cnt);
      #1 cnt = 100;
    join
    $display("T|r5|%0d", cnt);
  end
endmodule
"#;
    assert_eq!(
        tagged(src),
        vec![
            r#"T|r1|'{"B", "A"}"#,
            "T|r2|2",
            "T|r3|during=0",
            "T|r4|after=1",
            "T|r5|101",
        ]
    );
}

/// §13.5.2 for dynamic and associative `ref` containers, a writer racing a
/// `ref` reader, and a `ref` queue passed on from a task to a function.
#[test]
fn ref_containers_are_aliases() {
    let src = r#"
module p3;
  string lg[$];
  int dv[];
  int aa[int];
  int fq[$];
  task automatic push(int d, string nm, ref string q[$]); #d q.push_back(nm); endtask
  task automatic setd(int d, int i, int v, ref int x[]); #d x[i] = v; endtask
  task automatic seta(int d, int k, int v, ref int x[int]); #d x[k] = v; endtask
  task automatic peek(int d, ref int q[$], output int sz); #d sz = q.size(); endtask
  function automatic void fpush(ref int q[$], input int v); q.push_back(v); endfunction
  task automatic nested(ref int q[$]); fpush(q, 1); #1 fpush(q, 2); endtask
  int sz;
  initial begin
    fork push(3, "A", lg); push(1, "B", lg); join
    $display("T|r1|%p", lg);
    dv = new[3];
    fork setd(3, 0, 10, dv); setd(1, 1, 20, dv); #2 dv[2] = 30; join
    $display("T|r2|%p", dv);
    fork seta(3, 5, 50, aa); seta(1, 6, 60, aa); join
    $display("T|r3|%p", aa);
    fq = {1};
    fork peek(2, fq, sz); #1 fq.push_back(2); join
    $display("T|r4|%0d", sz);
    fq = {};
    fork nested(fq); #0 fq.push_back(9); join
    $display("T|r5|%p", fq);
  end
endmodule
"#;
    assert_eq!(
        tagged(src),
        vec![
            r#"T|r1|'{"B", "A"}"#,
            "T|r2|'{10, 20, 30}",
            "T|r3|'{5:50, 6:60}",
            "T|r4|2",
            "T|r5|'{1, 9, 2}",
        ]
    );
}

/// Audit repro `13.5.2_ref_arg_event_wait`: `@(posedge sig)` on a `ref`
/// formal waits for the actual's edge.
#[test]
fn event_control_on_ref_formal_waits_on_actual() {
    let src = r#"
module rrw;
  logic clk = 0;
  time tt;
  int seen;
  task automatic refwait(ref logic sig, output time t); @(posedge sig); t = $time; endtask
  task automatic refwait2(ref logic sig); wait (sig == 1); seen = $time; endtask
  initial begin
    fork refwait(clk, tt); #4 clk = 1; join
    $display("T|r1|tt=%0t now=%0t", tt, $time);
    clk = 0;
    fork refwait2(clk); #3 clk = 1; join
    $display("T|r2|seen=%0d", seen);
  end
endmodule
"#;
    assert_eq!(tagged(src), vec!["T|r1|tt=4 now=4", "T|r2|seen=7"]);
}

/// §13.5.2 event controls on `ref` formals: any edge, negedge, a vector, a
/// bit of a vector, an array element, and a class-method task.
#[test]
fn event_control_on_ref_formal_family() {
    let src = r#"
module p2;
  logic clk = 0;
  logic [3:0] bus = 0;
  logic arr [4];
  time t1, t2, t3, t4, t5, t6;
  task automatic w_pos(ref logic sig, output time t); @(posedge sig); t = $time; endtask
  task automatic w_any(ref logic sig, output time t); @(sig); t = $time; endtask
  task automatic w_neg(ref logic sig, output time t); @(negedge sig); t = $time; endtask
  task automatic w_bus(ref logic [3:0] b, output time t); @(b); t = $time; endtask
  task automatic w_bus2(ref logic [3:0] b, output time t); @(posedge b[2]); t = $time; endtask
  class K;
    task kw(ref logic sig, output time t); @(posedge sig); t = $time; endtask
  endclass
  K k;
  initial begin
    for (int i = 0; i < 4; i++) arr[i] = 0;
    k = new;
    fork w_pos(clk, t1); #4 clk = 1; join
    $display("T|pos|t=%0t now=%0t", t1, $time);
    fork w_neg(clk, t2); #2 clk = 0; join
    $display("T|neg|t=%0t", t2);
    fork w_any(clk, t3); #3 clk = 1; join
    $display("T|any|t=%0t", t3);
    fork w_bus(bus, t4); #2 bus = 5; join
    $display("T|bus|t=%0t", t4);
    fork w_bus2(bus, t5); #2 bus = 4'b0100; #3 bus = 4'b0000; #4 bus = 4'b0100; join
    $display("T|bus2|t=%0t", t5);
    fork w_pos(arr[2], t6); #2 arr[2] = 1; join
    $display("T|arr|t=%0t", t6);
    clk = 0;
    fork k.kw(clk, t1); #5 clk = 1; join
    $display("T|cls|t=%0t", t1);
  end
endmodule
"#;
    assert_eq!(
        tagged(src),
        vec![
            "T|pos|t=4 now=4",
            "T|neg|t=6",
            "T|any|t=9",
            "T|bus|t=11",
            "T|bus2|t=15",
            "T|arr|t=17",
            "T|cls|t=22",
        ]
    );
}

/// Audit repros `13.5_queue_slice_argument` and its recursion crash.
#[test]
fn queue_slice_actual_binds_the_slice() {
    let src = r#"
module r13a2;
  function automatic int rsum(int q[$]); if (q.size() == 0) return 0; return q[0] + rsum(q[1:$]); endfunction
  function automatic int qsz(int q[$]); return q.size(); endfunction
  int src[$];
  initial begin
    src = {1, 2, 3, 4};
    $display("T|r1|%0d %0d %0d", qsz(src[1:$]), qsz(src[3:$]), qsz(src[4:$]));
    $display("T|r2|%0d", rsum(src));
  end
endmodule
"#;
    assert_eq!(tagged(src), vec!["T|r1|3 1 0", "T|r2|10"]);
}

/// §7.10.1: slices of queues, dynamic and fixed arrays as actuals and as
/// values; `q[a:b]` with a > b is empty.
#[test]
fn queue_slice_family() {
    let src = r#"
module p4;
  function automatic int qsz(int q[$]); return q.size(); endfunction
  function automatic int qsum(int q[$]); int s = 0; foreach (q[i]) s += q[i]; return s; endfunction
  task automatic tsz(int q[$], output int n); n = q.size(); endtask
  int src[$]; int dyn[]; int fx[6]; int n;
  initial begin
    src = {1, 2, 3, 4};
    $display("T|a|%0d %0d %0d %0d", qsz(src[1:$]), qsz(src[3:$]), qsz(src[4:$]), qsz(src[0:1]));
    $display("T|b|%0d %0d", qsum(src[1:2]), qsum(src[1:$]));
    $display("T|d|%0d %0d", qsz(src[3:1]), qsz(src[2:2]));
    tsz(src[1:$], n); $display("T|e|%0d", n);
    $display("T|f|%p %p %p", src[3:1], src[1:$], src[4:$]);
    dyn = '{5, 6, 7, 8, 9};
    $display("T|g|%0d", qsz(dyn));
    for (int i = 0; i < 6; i++) fx[i] = i * 10;
    $display("T|h|%0d", qsum(fx[1:3]));
  end
endmodule
"#;
    assert_eq!(
        tagged(src),
        vec![
            "T|a|3 1 0 2",
            "T|b|5 9",
            "T|d|0 1",
            "T|e|3",
            "T|f|'{} '{2, 3, 4} '{}",
            "T|g|5",
            "T|h|60",
        ]
    );
}

/// §8.10 / §13.5: a method's collection formal shadows a same-named STATIC
/// class collection — the pattern of `uvm_packer::put_bits(ref bit
/// bitstream[])` beside `static bit bitstream[]`. Once formals got their own
/// storage, element reads went to the static member (UVM unpack read 0s).
#[test]
fn collection_formal_shadows_static_class_member() {
    let src = r#"
module u6;
  class P;
    static bit bs[];
    int got;
    function void take(ref bit bs[]);
      $display("T|ref|%0d %0d%0d%0d", bs.size(), bs[0], bs[1], bs[2]);
      foreach (bs[i]) got = got * 2 + bs[i];
      bs[0] = 0;
    endfunction
    function void putv(bit bs[]);
      $display("T|val|%0d %0d%0d%0d", bs.size(), bs[0], bs[1], bs[2]);
      bs[1] = 1;
    endfunction
  endclass
  class T;
    task run();
      bit bits[];
      P p = new;
      bits = new[3]; bits[0] = 1; bits[2] = 1;
      p.take(bits);
      $display("T|after|%0d %p", p.got, bits);
      p.putv(bits);
      $display("T|after2|%p %0d", bits, P::bs.size());
    endtask
  endclass
  initial begin T t; t = new; t.run(); end
endmodule
"#;
    assert_eq!(
        tagged(src),
        vec![
            "T|ref|3 101",
            "T|after|5 '{'h0, 'h0, 'h1}",
            "T|val|3 001",
            "T|after2|'{'h0, 'h0, 'h1} 0",
        ]
    );
}
