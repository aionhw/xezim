//! #940 review: the joint-constraint graph memo and the foreach `dist`
//! geometry memo must not outlive the values they were built from.
//!
//! §18.5: a constraint reads module and package variables, static
//! properties, other objects' members, array elements, function results and
//! value parameters; each of them can change between two `randomize()`
//! calls on one object, and a cached graph that folded the old value in
//! kept solving `x == 4` after the variable became 200. A class declared in
//! a module reads each instance's own variables. §18.5.4: a `dist` inside a
//! `foreach` may read the loop index, a rand variable or a property, so its
//! bounds and weights are not the same for every element. Expected values
//! come from the reference simulator.

use xezim::simulate;

fn lines(src: &str, prefix: &str) -> Vec<String> {
    simulate(src, 1000)
        .expect("simulate failed")
        .output
        .iter()
        .map(|o| o.message.trim_end().to_string())
        .filter(|l| l.starts_with(prefix))
        .collect()
}

const CSP: &str = r#"
package p;
  int plim = 3;
  int ptbl[4];
endpackage
module top;
  int lim;
  function automatic int f(); return lim + 2; endfunction
  class Cfg; int lim = 4; endclass
  class CM;   // module variable
    rand bit [7:0] x, y;
    constraint c { x >= lim; x <= lim; y < 16; solve y before x; }
  endclass
  class CS;   // static property
    static int lim = 4;
    rand bit [7:0] x, y;
    constraint c { x >= lim; x <= lim; y < 16; solve y before x; }
  endclass
  class CO;   // another object's member
    Cfg cfg = new;
    rand bit [7:0] x, y;
    constraint c { x >= cfg.lim; x <= cfg.lim; y < 16; solve y before x; }
  endclass
  class CP;   // package variable
    rand bit [7:0] x, y;
    constraint c { x >= p::plim; x <= p::plim; y < 16; solve y before x; }
  endclass
  class CA;   // module array element
    rand bit [7:0] x, y;
    constraint c { x >= p::ptbl[1]; x <= p::ptbl[1]; y < 16; solve y before x; }
  endclass
  class CF;   // function call
    rand bit [7:0] x, y;
    constraint c { x >= f(); x <= f(); y < 16; solve y before x; }
  endclass
  class CE;   // foreach over a rand array, indexing a module array
    rand bit [7:0] a[4];
    rand bit [7:0] y;
    constraint c { foreach (a[i]) { a[i] >= p::ptbl[i]; a[i] <= p::ptbl[i]; } y < 16; solve y before a; }
  endclass
  class COWN; // own non-rand property
    int lim = 4;
    rand bit [7:0] x, y;
    constraint c { x >= lim; x <= lim; y < 16; solve y before x; }
  endclass
  class CPar #(int L = 4); // value parameter, two specializations
    rand bit [7:0] x, y;
    constraint c { x >= L; x <= L; y < 16; solve y before x; }
  endclass
  initial begin
    CM cm = new; CS cs = new; CO co = new; CP cp = new; CA ca = new;
    CF cf = new; CE ce = new; COWN cw = new;
    CPar #(4) c4 = new; CPar #(9) c9 = new;
    int ok;
    lim = 4; p::ptbl = '{1, 4, 2, 3};
    repeat (3) begin
      ok = cm.randomize(); ok = cs.randomize(); ok = co.randomize(); ok = cp.randomize();
      ok = ca.randomize(); ok = cf.randomize(); ok = ce.randomize(); ok = cw.randomize();
      ok = c4.randomize();
    end
    $display("C1 cm=%0d cs=%0d co=%0d cp=%0d ca=%0d cf=%0d ce=%0d,%0d,%0d,%0d cw=%0d c4=%0d",
             cm.x, cs.x, co.x, cp.x, ca.x, cf.x, ce.a[0], ce.a[1], ce.a[2], ce.a[3], cw.x, c4.x);
    lim = 200; CS::lim = 201; co.cfg.lim = 202; p::plim = 203; p::ptbl = '{11, 204, 12, 13};
    cw.lim = 205;
    repeat (3) begin
      ok = cm.randomize(); ok = cs.randomize(); ok = co.randomize(); ok = cp.randomize();
      ok = ca.randomize(); ok = cf.randomize(); ok = ce.randomize(); ok = cw.randomize();
      ok = c9.randomize();
    end
    $display("C2 cm=%0d cs=%0d co=%0d cp=%0d ca=%0d cf=%0d ce=%0d,%0d,%0d,%0d cw=%0d c9=%0d",
             cm.x, cs.x, co.x, cp.x, ca.x, cf.x, ce.a[0], ce.a[1], ce.a[2], ce.a[3], cw.x, c9.x);
  end
endmodule
"#;

#[test]
fn constraint_memo_sees_every_value_a_constraint_reads() {
    assert_eq!(
        lines(CSP, "C"),
        [
            "C1 cm=4 cs=4 co=4 cp=3 ca=4 cf=6 ce=1,4,2,3 cw=4 c4=4",
            "C2 cm=200 cs=201 co=202 cp=203 ca=204 cf=202 ce=11,204,12,13 cw=205 c9=9",
        ]
    );
}

const SCOPE: &str = r#"
// The same class in two instances of one module reads each instance's own
// variable.
module m #(int P = 1) (output int got);
  int lim = P;
  class C;
    rand bit [7:0] x, y;
    constraint c { x >= lim; x <= lim; y < 16; solve y before x; }
  endclass
  initial begin
    C c = new;
    #1;
    repeat (3) void'(c.randomize());
    got = c.x;
  end
endmodule
module top;
  int g1, g2;
  m #(3) u1(g1);
  m #(8) u2(g2);
  initial begin
    #2 $display("S u1=%0d u2=%0d", g1, g2);
  end
endmodule
"#;

#[test]
fn constraint_memo_reads_each_instance_scope() {
    assert_eq!(lines(SCOPE, "S "), ["S u1=3 u2=8"]);
}

const DIST: &str = r#"
module top;
  typedef struct packed { bit [7:0] m; bit [7:0] n; } s_t;
  class D;
    rand s_t arr[];
    rand bit [7:0] len;
    rand bit [7:0] k;
    int base = 5;
    constraint c {
      len == 4;
      arr.size() == len;
      k inside {[20:30]};
      foreach (arr[i]) arr[i].m dist { i + base := 1 };
      foreach (arr[i]) arr[i].n dist { 0 := 3 - i, 1 := i };
    }
  endclass
  class E;
    rand s_t arr[];
    rand bit [7:0] k;
    constraint c {
      arr.size() == 3;
      k inside {[20:30]};
      foreach (arr[i]) arr[i].m dist { k := 1 };
    }
  endclass
  initial begin
    D d = new;
    E e = new;
    int bad_m = 0, bad_n = 0, bad_k = 0, fails = 0;
    repeat (50) begin
      if (!d.randomize()) fails++;
      foreach (d.arr[i]) if (d.arr[i].m != i + 5) bad_m++;
      if (d.arr[0].n != 0 || d.arr[3].n != 1) bad_n++;
      if (!e.randomize()) fails++;
      foreach (e.arr[i]) if (e.arr[i].m != e.k) bad_k++;
    end
    d.base = 9;
    if (!d.randomize()) fails++;
    $display("D m=%0d,%0d,%0d,%0d bad_m=%0d bad_n=%0d bad_k=%0d fails=%0d",
             d.arr[0].m, d.arr[1].m, d.arr[2].m, d.arr[3].m, bad_m, bad_n, bad_k, fails);
  end
endmodule
"#;

#[test]
fn foreach_dist_reads_index_rand_and_property_per_element() {
    assert_eq!(
        lines(DIST, "D "),
        ["D m=9,10,11,12 bad_m=0 bad_n=0 bad_k=0 fails=0"]
    );
}
