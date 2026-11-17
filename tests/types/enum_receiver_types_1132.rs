//! §6.19.5: the enumeration methods take the enumeration from the
//! receiver's DECLARED type, whatever expression the receiver is (#1132): a
//! variable declared through a typedef alias or an array typedef, a
//! `for`-header variable, an array element, a struct or union member at any
//! depth, a property reached through class handles or `this`, a function
//! result and its members, and the result of another enumeration method.
//! A value that is not a member has the empty name (§6.19.5.6); other
//! enumerations of the design that hold the value do not name it. A queue,
//! dynamic or associative array of enumeration elements keeps its own
//! `num`/`first`/`last` (§7.9, §7.10), and the members of an enumeration
//! declared in a procedural block are constants of that block (§6.19).
//!
//! Every expected line is the reference simulator's.

use xezim::simulate;

#[test]
fn enum_methods_resolve_the_receivers_declared_type() {
    let src = r#"
package ep;
  typedef enum bit [1:0] { L0, L1, L2, L3 } level_e;
  typedef enum bit [7:0] { T0, T1, T2, T3, T4, T5, T6, T7 } tag_e;
  typedef enum bit [1:0] { M0, M1, M2, M3 } mode_e;
  typedef level_e alias_e;
  typedef alias_e alias2_e;
  typedef struct packed { bit [3:0] code; level_e lvl; } pitem_t;
  typedef struct { level_e lvl; int c; } pplain_t;
  typedef union { level_e ulvl; int uraw; } puun_t;
  typedef level_e plarr_t[2];
  typedef enum logic [1:0] { Q0, Q1 } q4_e;
  typedef enum bit [1:0] { R0, R1 } pair_e;
endpackage
module top;
  import ep::*;
  typedef struct packed { pitem_t inner; } outer_t;
  typedef union packed { bit [1:0] raw; level_e lvl; } un_t;
  class node;
    level_e arr[2];
    level_e q[$];
    level_e aa[string];
    pitem_t st;
    pitem_t psa[2];
    plarr_t parr;
    pplain_t pt;
    alias_e ap;
    struct { level_e lvl; int c; } inl;
    function new();
      arr[0] = L1; arr[1] = L3; q.push_back(L1); q.push_back(L3); aa["k"] = L2;
      st.lvl = L2; psa[0].lvl = L1; psa[1].lvl = L3; parr[0] = L1; parr[1] = L3;
      pt.lvl = L2; ap = L2; inl.lvl = L2;
    endfunction
    function string via_this();
      return {this.arr[0].name(), this.q[1].name(), this.aa["k"].name(), this.st.lvl.name(),
              this.psa[1].lvl.name(), this.parr[0].name()};
    endfunction
    function string bare();
      return {arr[1].name(), q[0].name(), aa["k"].name(), st.lvl.name(), psa[0].lvl.name(), ap.name()};
    endfunction
    function int coll(); return aa.num() * 10 + q.size(); endfunction
    function pitem_t item(); pitem_t t; t.lvl = L2; return t; endfunction
  endclass
  class holder; node n; function new(); n = new(); endfunction endclass
  class walker;
    int iters; string first;
    function void walk();
      iters = 0;
      for (level_e s = s.first(); 1; s = s.next()) begin
        if (iters == 0) first = s.name();
        iters++;
        if (s == s.last() || iters > 20) break;
      end
    endfunction
    function void from_mode(); mode_e s; s = M1; walk(); endfunction
    function void from_tag(); tag_e s; s = T1; walk(); endfunction
  endclass
  function automatic level_e pick(int i); return level_e'(i); endfunction
  function automatic pitem_t fitem(); pitem_t t; t.lvl = L2; return t; endfunction
  function automatic node fnode(); node n = new; return n; endfunction
  function automatic string stat(); static level_e sl = L1; return sl.name(); endfunction
  plarr_t parr; level_e pa[2]; outer_t o1; un_t u1;
  node carr[2]; node cq[$]; node caa[string];
  ep::pitem_t pk_pst; ep::pplain_t pk_ust; ep::puun_t pk_un; ep::plarr_t pk_arr; ep::alias_e pk_als; alias2_e md_als;
  struct { struct { level_e lvl; int c; } inner; } nest;
  level_e ar; int cnt[level_e]; level_e eaa[int];
  initial begin
    walker w = new; node n = new; holder h = new; node tmp;
    alias_e v; alias2_e v2; level_e larr[2]; level_e da[]; pitem_t sarr[2];
    q4_e xv; pair_e pv; node an[level_e]; node a2d[2][2];
    w.walk(); $display("W1|%0d|%s", w.iters, w.first);
    w.from_mode(); $display("W2|%0d|%s", w.iters, w.first);
    w.from_tag(); $display("W3|%0d|%s", w.iters, w.first);
    v = L1; v2 = L2;
    $display("AL|%s|%0d|%0d|%s|%0d|%0d", v.name(), v.last(), v.num(), v2.name(), v2.next(), v2.prev(2));
    $display("FN|%s|%0d|%0d|%0d|%s", pick(1).name(), pick(2).first(), pick(2).last(), pick(2).num(), pick(3).prev().name());
    larr[0] = L1; larr[1] = L3; da = new[2]; da[0] = L1; da[1] = L3;
    $display("AR|%s|%0d|%s|%0d", larr[0].name(), larr[1].first(), da[0].name(), da[1].last());
    sarr[1].lvl = L3;
    $display("SA|%s|%0d", sarr[1].lvl.name(), sarr[1].lvl.prev());
    parr[0] = L1; pa[0] = L1;
    $display("TA|%s|%0d|%0d|%0d|%s", parr[0].name(), parr[0].last(), parr[0].num(), parr[0].next(), pa[0].name());
    o1.inner.lvl = L2; u1.lvl = L3;
    $display("NS|%s|%0d|%s|%0d", o1.inner.lvl.name(), o1.inner.lvl.next(), u1.lvl.name(), u1.lvl.prev());
    $display("ST|%s", stat());
    $display("TH|%s|%s|%0d", n.via_this(), n.bare(), n.coll());
    $display("CP|%s|%s|%s|%s|%s|%s|%0d", n.st.lvl.name(), n.psa[1].lvl.name(), n.pt.lvl.name(), n.inl.lvl.name(), n.ap.name(), n.parr[1].name(), n.pt.lvl.num());
    carr[0] = new(); tmp = new(); cq.push_back(tmp); caa["k"] = new(); an[L1] = new(); a2d[0][1] = new();
    $display("HC|%s|%s|%s|%s|%s", carr[0].arr[0].name(), cq[0].arr[1].name(), caa["k"].arr[0].name(), an[L1].arr[1].name(), a2d[0][1].q[0].name());
    $display("NH|%s|%s|%s|%s", h.n.st.lvl.name(), h.n.pt.lvl.name(), h.n.parr[0].name(), fnode().arr[0].name());
    pk_pst.lvl = L1; pk_ust.lvl = L2; pk_un.ulvl = L2; pk_arr[1] = L3; pk_als = L2; md_als = L3; nest.inner.lvl = L2;
    $display("MS|%s|%s|%s|%s|%s|%s|%s|%0d", pk_pst.lvl.name(), pk_ust.lvl.name(), pk_un.ulvl.name(), pk_arr[1].name(), pk_als.name(), md_als.name(), nest.inner.lvl.name(), nest.inner.lvl.last());
    $display("FR|%s|%s|%0d", fitem().lvl.name(), n.item().lvl.name(), fitem().lvl.num());
    xv = Q1; $display("X1|%s", xv.name());
    xv = q4_e'('x); $display("X2|%s|%0d|%b", xv.name(), xv.num(), xv.next());
    xv = q4_e'(2'b0x); $display("X3|%s", xv.name());
    pv = pair_e'(2); $display("OR|%s|%0d|%0d", pv.name(), pv.num(), pv.next());
    ar = L1; $display("CH|%0d|%s|%0d|%s", ar.next(2), ar.next(2).name(), ar.next(2).last(), ar.next().name());
    cnt[L1] = 1; cnt[L3] = 2; eaa[4] = L2; eaa[9] = L3;
    begin
      level_e k; int i;
      void'(cnt.first(k)); void'(eaa.last(i));
      $display("AA|%0d|%s|%0d|%0d|%s", cnt.num(), k.name(), eaa.num(), i, eaa[i].name());
    end
    begin
      typedef enum bit [1:0] { BL0, BL1, BL2, BL3 } blevel_e;
      blevel_e bv; enum { A0, A1 } x;
      bv = BL2; x = A1;
      $display("BE|%0d|%s|%0d|%0d|%s", bv, bv.name(), bv.num(), x, x.name());
    end
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("runs");
    let got: Vec<&str> = sim.output.iter().map(|o| o.message.as_str()).collect();
    let want = [
        "W1|4|L0",
        "W2|4|L0",
        "W3|4|L0",
        "AL|L1|3|4|L2|3|0",
        "FN|L1|0|3|4|L2",
        "AR|L1|0|L1|3",
        "SA|L3|2",
        "TA|L1|3|4|2|L1",
        "NS|L2|3|L3|2",
        "ST|L1",
        "TH|L1L3L2L2L3L1|L3L1L2L2L1L2|12",
        "CP|L2|L3|L2|L2|L2|L3|4",
        "HC|L1|L3|L1|L3|L1",
        "NH|L2|L2|L1|L1",
        "MS|L1|L2|L2|L3|L2|L3|L2|3",
        "FR|L2|L2|4",
        "X1|Q1",
        "X2||2|xx",
        "X3|",
        "OR||2|0",
        "CH|3|L3|3|L2",
        "AA|2|L1|2|9|L3",
        "BE|2|BL2|4|1|A1",
    ];
    for w in want {
        assert!(got.iter().any(|l| *l == w), "missing line {w:?} in {got:?}");
    }
    assert_eq!(got.len(), want.len(), "unexpected output: {got:?}");
}

/// §6.19: a member of a 4-state enumeration may hold x or z bits. A value
/// with the same bits is that member; any other value with x or z bits is
/// no member.
#[test]
fn enum_members_with_x_bits_keep_their_names() {
    let src = r#"
module top;
  typedef enum logic [1:0] {XA = 2'b00, XB = 2'bx1, XC = 2'b10} xe_t;
  initial begin
    xe_t x = XB;
    $display("A|%b|%s|%b|%b", x, x.name(), x.next(), x.prev());
    x = xe_t'(2'bxx); $display("B|%b|%s|%b", x, x.name(), x.next());
    x = XA; $display("C|%s|%b", x.name(), x.next());
  end
endmodule
"#;
    let sim = simulate(src, 10).expect("runs");
    let got: Vec<&str> = sim.output.iter().map(|o| o.message.as_str()).collect();
    assert_eq!(got, ["A|x1|XB|10|00", "B|xx||xx", "C|XA|x1"]);
}
