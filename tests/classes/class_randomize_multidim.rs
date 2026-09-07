//! `obj.randomize()` over multi-dimensional `rand` properties. Four defects
//! hid behind one symptom (return 1, elements 0 or stale): a packed
//! multi-dimensional property had no element geometry, so `d[i][j]` reads and
//! writes were single-bit selects and `foreach (d[i, j])` ran once; 2-D array
//! elements wider than 64 bits were never drawn; arrays under a foreach were
//! skipped by the draw and repaired from their previous values, so
//! `e[i] < 100` kept zeros and repeated calls returned the same values; and
//! the draw ran after the repair pass, clobbering element pins (`a[0] == 5`)
//! and foreach bodies that read another drawn array
//! (`$countones(m[i][j]) == e[i][j]`). Reference-verified expectations.
use xezim::simulate;

fn messages(src: &str) -> Vec<String> {
    let sim = simulate(src, 1_000).expect("simulate failed");
    sim.output.iter().map(|o| o.message.clone()).collect()
}

fn expect(msgs: &[String], wants: &[&str]) {
    for want in wants {
        assert!(msgs.iter().any(|m| m == want), "missing {want}; got {msgs:?}");
    }
}

const PRELUDE: &str = "typedef bit [6:0] u7_t;\n";

#[test]
fn packed_property_foreach_and_element_access() {
    let msgs = messages(&format!(
        "{PRELUDE}
class C;
  u7_t [4:0][1:0] d;
  u7_t e [5][2];
  function int cnt_d(); int n = 0; foreach (d[i, j]) n++; return n; endfunction
  function int cnt_e(); int n = 0; foreach (e[i, j]) n++; return n; endfunction
endclass
module tb;
  C c = new; int nd = 0, ne = 0;
  initial begin
    c.d = '1;
    c.d[2][1] = 77;
    c.d[1][0] = 33;
    foreach (c.d[i, j]) nd++;
    foreach (c.e[i, j]) ne++;
    $display(\"FE %0d %0d %0d %0d\", nd, ne, c.cnt_d(), c.cnt_e());
    $display(\"EL %0d %0d %0d %0d\", c.d[0][0], c.d[2][1], c.d[1][0], c.d[4][1]);
    $finish;
  end
endmodule
"
    ));
    expect(&msgs, &["FE 10 10 10 10", "EL 127 77 33 127"]);
}

#[test]
fn coupled_countones_and_packed_bounds() {
    let msgs = messages(&format!(
        "{PRELUDE}
class C;
  rand u7_t [4:0][1:0] d;
  rand u7_t e [5][2];
  rand bit [99:0] m [5][2];
  constraint c1 {{ foreach (d[i, j]) d[i][j] < 100; }}
  constraint c2 {{ foreach (e[i, j]) e[i][j] < 100; }}
  constraint c3 {{ foreach (e[i, j]) $countones(m[i][j]) == e[i][j]; }}
endclass
module tb;
  C c = new; int ok, nzd, bad_d, bad_e, nzm;
  initial begin
    ok = c.randomize();
    nzd = 0; bad_d = 0; foreach (c.d[i, j]) begin if (c.d[i][j] != 0) nzd++; if (c.d[i][j] >= 100) bad_d++; end
    bad_e = 0; nzm = 0; foreach (c.e[i, j]) begin if (c.e[i][j] != $countones(c.m[i][j]) || c.e[i][j] >= 100) bad_e++; if (c.m[i][j] != 0) nzm++; end
    $display(\"CLS ok=%0d nzd=%0d bad_d=%0d bad_e=%0d nzm=%0d\", ok, nzd, bad_d, bad_e, nzm);
    $finish;
  end
endmodule
"
    ));
    expect(&msgs, &["CLS ok=1 nzd=10 bad_d=0 bad_e=0 nzm=10"]);
}

#[test]
fn element_pin_survives_array_draw() {
    let msgs = messages(
        "class P;
  rand bit [7:0] a [8];
  constraint c { a[0] == 5; a[1] > 250; }
endclass
module tb;
  P p = new; int ok;
  initial begin
    ok = p.randomize();
    $display(\"PIN ok=%0d a0=%0d hi=%0d\", ok, p.a[0], p.a[1] > 250);
    $finish;
  end
endmodule
",
    );
    expect(&msgs, &["PIN ok=1 a0=5 hi=1"]);
}

#[test]
fn foreach_bounded_arrays_are_drawn_fresh() {
    let msgs = messages(&format!(
        "{PRELUDE}
class A;
  rand u7_t e [5][2];
  rand u7_t f [10];
  constraint c {{ foreach (e[i, j]) e[i][j] < 100; foreach (f[i]) f[i] < 100; }}
endclass
class Q;
  rand bit [7:0] a [8];
  constraint c {{ foreach (a[i]) a[i] inside {{[10:200]}}; }}
endclass
module tb;
  A aa = new; Q q = new; int ok, bad, nz, same; bit [7:0] prev [8];
  initial begin
    ok = aa.randomize(); bad = 0; nz = 0;
    foreach (aa.e[i, j]) begin if (aa.e[i][j] >= 100) bad++; if (aa.e[i][j] != 0) nz++; end
    foreach (aa.f[i]) begin if (aa.f[i] >= 100) bad++; if (aa.f[i] != 0) nz++; end
    $display(\"LT ok=%0d bad=%0d drawn=%0d\", ok, bad, nz > 10);
    ok = q.randomize(); foreach (q.a[i]) prev[i] = q.a[i];
    ok = q.randomize(); same = 0; bad = 0;
    foreach (q.a[i]) begin if (q.a[i] == prev[i]) same++; if (q.a[i] < 10 || q.a[i] > 200) bad++; end
    $display(\"FRESH ok=%0d bad=%0d sticky=%0d\", ok, bad, same == 8);
    $finish;
  end
endmodule
"
    ));
    expect(&msgs, &["LT ok=1 bad=0 drawn=1", "FRESH ok=1 bad=0 sticky=0"]);
}
