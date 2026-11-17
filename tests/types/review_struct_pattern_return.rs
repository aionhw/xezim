//! §10.9, §13.4.1: follow-up cases for an assignment pattern in return
//! position (#1170): the return type named by a class type parameter, a
//! queue or dynamic array of unpacked structs, struct members that are
//! queues or dynamic arrays, a pattern assigned to the function name inside
//! a method, and a string member selected directly from the call.
//!
//! Expected values are the reference simulator's, except the two `default:`
//! keys over string members (`DF`, the last element of `SD`): §10.9.2 gives
//! every member the default value, and the reference leaves them empty.

use xezim::simulate;

#[test]
fn struct_pattern_return_shapes() {
    let src = r#"
package pk;
  typedef struct { string a; string b; } ss_t;
  function automatic ss_t pk_ret(); return '{ "pa", "pb" }; endfunction
endpackage
module top;
  import pk::*;
  typedef struct { int a; string b; } is_t;
  typedef struct { ss_t inner; string tail; } nest_t;
  typedef struct { ss_t prs[2]; string tag; } arrst_t;
  typedef struct { int q[$]; string s; } qs_t;
  typedef struct { int d[]; string s; } ds_t;
  typedef struct { real r; string s; byte b; } rs_t;
  typedef ss_t ssq_t[$];
  typedef ss_t ssd_t[];
  typedef struct packed { logic [7:0] hi; logic [7:0] lo; } pk_t;

  class P #(type T = int);
    function T get(); return '{ "ga", "gb" }; endfunction
    function T getk(); return '{ b: "kb", a: "ka" }; endfunction
    static function ss_t sget(); return '{ "sa", "sb" }; endfunction
  endclass
  class Q #(type T = int);
    function T get(); return '{ 8'h12, 8'h34 }; endfunction
  endclass

  class C;
    string m = "mem";
    function ss_t ret_rep(); return '{ 2{"rr"} }; endfunction
    function ss_t ret_dflt(); return '{ default: "dd" }; endfunction
    function is_t ret_tkey(); return '{ int: 5, string: "tk" }; endfunction
    function arrst_t ret_arrst(); return '{ '{ '{"a0","b0"}, '{"a1","b1"} }, "tg" }; endfunction
    function qs_t ret_qs(); return '{ '{1,2,3}, "qs" }; endfunction
    function qs_t ret_qsk(); return '{ s: "qk", q: '{7,8} }; endfunction
    function qs_t ret_qse(); return '{ '{}, "qe" }; endfunction
    function ds_t ret_ds(); return '{ '{4,5}, "ds" }; endfunction
    function rs_t ret_rs(); return '{ 1.5, "rs", 8'h41 }; endfunction
    function ssq_t ret_q(); return '{ '{"q0a","q0b"}, '{"q1a","q1b"} }; endfunction
    function ssq_t ret_qe(); return '{}; endfunction
    function ssd_t ret_d(); return '{ '{"d0a","d0b"}, '{ b: "d1b", a: "d1a" }, '{ default: "dx" } }; endfunction
    function ssq_t ret_qc(int c);
      return c ? '{ '{"c0a","c0b"} } : '{ '{"e0a","e0b"}, '{"e1a","e1b"} };
    endfunction
    function ss_t ret_mem(); return '{ m, {m, "2"} }; endfunction
    function ss_t ret_nc(int c, int d);
      ss_t loc = '{"la","lb"};
      return c ? '{ "c1", "c2" } : d ? '{ "d1", "d2" } : loc;
    endfunction
    function ss_t ret_if(int c);
      if (c) return '{ "i1", "i2" };
      ret_if = '{ "n1", "n2" };
    endfunction
    function nest_t ret_nk(); return '{ tail: "t", inner: '{ b: "ib", a: "ia" } }; endfunction
    virtual function ss_t vret(); return '{ "v1", "v2" }; endfunction
  endclass
  class D extends C;
    virtual function ss_t vret(); return '{ "dv1", "dv2" }; endfunction
  endclass

  function ss_t static_ret(); return '{ "st1", "st2" }; endfunction
  function automatic qs_t free_qs(); return '{ '{9,8,7,6}, "fq" }; endfunction
  function automatic ssq_t free_q(); return '{ '{"f0a","f0b"} }; endfunction
  function automatic void show(ss_t x); $display("SH|%s|%s", x.a, x.b); endfunction

  initial begin
    C c = new; D d = new; P#(ss_t) p = new; Q#(pk_t) qq = new;
    ss_t s; is_t m; nest_t g; arrst_t a; qs_t q; ds_t ds; rs_t r; ssq_t sq; ssd_t sd; pk_t pv;
    s = c.ret_rep(); $display("REP|%s|%s", s.a, s.b);
    s = c.ret_dflt(); $display("DF|%s|%s", s.a, s.b);
    m = c.ret_tkey(); $display("TK|%0d|%s", m.a, m.b);
    a = c.ret_arrst(); $display("AS|%s|%s|%s|%s|%s", a.prs[0].a, a.prs[0].b, a.prs[1].a, a.prs[1].b, a.tag);
    q = c.ret_qs(); $display("QS|%0d|%0d|%s", q.q.size(), q.q.size() == 3 ? q.q[2] : -1, q.s);
    q = c.ret_qsk(); $display("QK|%0d|%0d|%s", q.q.size(), q.q.size() == 2 ? q.q[1] : -1, q.s);
    q = c.ret_qse(); $display("QE|%0d|%s", q.q.size(), q.s);
    ds = c.ret_ds(); $display("DS|%0d|%0d|%s", ds.d.size(), ds.d.size() == 2 ? ds.d[1] : -1, ds.s);
    q = free_qs(); $display("FQ|%0d|%0d|%s", q.q.size(), q.q.size() == 4 ? q.q[3] : -1, q.s);
    r = c.ret_rs(); $display("RS|%0.1f|%s|%0d", r.r, r.s, r.b);
    sq = c.ret_q(); $display("SQ|%0d|%s|%s|%s|%s", sq.size(), sq[0].a, sq[0].b, sq[1].a, sq[1].b);
    sq = c.ret_qe(); $display("SQE|%0d", sq.size());
    sd = c.ret_d(); $display("SD|%0d|%s|%s|%s|%s|%s|%s", sd.size(), sd[0].a, sd[0].b, sd[1].a, sd[1].b, sd[2].a, sd[2].b);
    sq = c.ret_qc(1); $display("QC1|%0d|%s|%s", sq.size(), sq[0].a, sq[0].b);
    sq = c.ret_qc(0); $display("QC0|%0d|%s|%s", sq.size(), sq[1].a, sq[1].b);
    sq = free_q(); $display("FSQ|%0d|%s|%s", sq.size(), sq[0].a, sq[0].b);
    s = c.ret_mem(); $display("ME|%s|%s", s.a, s.b);
    s = c.ret_nc(1,0); $display("NC1|%s|%s", s.a, s.b);
    s = c.ret_nc(0,1); $display("NC2|%s|%s", s.a, s.b);
    s = c.ret_nc(0,0); $display("NC3|%s|%s", s.a, s.b);
    s = c.ret_if(1); $display("IF1|%s|%s", s.a, s.b);
    s = c.ret_if(0); $display("IF0|%s|%s", s.a, s.b);
    g = c.ret_nk(); $display("NK|%s|%s|%s", g.inner.a, g.inner.b, g.tail);
    c = d; s = c.vret(); $display("V|%s|%s", s.a, s.b);
    s = p.get(); $display("PG|%s|%s", s.a, s.b);
    s = p.getk(); $display("PGK|%s|%s", s.a, s.b);
    s = P#(ss_t)::sget(); $display("PS|%s|%s", s.a, s.b);
    pv = qq.get(); $display("PQ|%h|%h", pv.hi, pv.lo);
    s = pk_ret(); $display("PK|%s|%s", s.a, s.b);
    s = pk::pk_ret(); $display("PKQ|%s|%s", s.a, s.b);
    s = static_ret(); $display("ST|%s|%s", s.a, s.b);
    $display("DIR|%s|%s|%0d", c.ret_rep().a, c.ret_mem().b, c.ret_mem().b.len());
    $display("DIRN|%s|%s", c.ret_nk().inner.b, c.ret_arrst().prs[1].a);
    show(c.ret_rep());
    begin
      ss_t i = c.ret_mem();
      $display("INIT|%s|%s", i.a, i.b);
    end
    $display("EQ|%0d|%0d", c.ret_rep() == c.ret_rep(), c.ret_rep() == c.ret_mem());
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("runs");
    let got: Vec<&str> = sim.output.iter().map(|o| o.message.as_str()).collect();
    let want = [
        "REP|rr|rr",
        "DF|dd|dd",
        "TK|5|tk",
        "AS|a0|b0|a1|b1|tg",
        "QS|3|3|qs",
        "QK|2|8|qk",
        "QE|0|qe",
        "DS|2|5|ds",
        "FQ|4|6|fq",
        "RS|1.5|rs|65",
        "SQ|2|q0a|q0b|q1a|q1b",
        "SQE|0",
        "SD|3|d0a|d0b|d1a|d1b|dx|dx",
        "QC1|1|c0a|c0b",
        "QC0|2|e1a|e1b",
        "FSQ|1|f0a|f0b",
        "ME|mem|mem2",
        "NC1|c1|c2",
        "NC2|d1|d2",
        "NC3|la|lb",
        "IF1|i1|i2",
        "IF0|n1|n2",
        "NK|ia|ib|t",
        "V|dv1|dv2",
        "PG|ga|gb",
        "PGK|ka|kb",
        "PS|sa|sb",
        "PQ|12|34",
        "PK|pa|pb",
        "PKQ|pa|pb",
        "ST|st1|st2",
        "DIR|rr|mem2|4",
        "DIRN|ib|a1",
        "SH|rr|rr",
        "INIT|mem|mem2",
        "EQ|1|0",
    ];
    for w in want {
        assert!(got.iter().any(|l| *l == w), "missing line {w:?} in {got:?}");
    }
    assert_eq!(got.len(), want.len(), "unexpected output: {got:?}");
}
