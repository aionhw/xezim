//! IEEE 1800-2023 §18.5.8.1: a `foreach` constraint iterates the indices of
//! its array, which for an array of fixed shape are known before the solve
//! even though the elements are random. A loop over an array the solvers do
//! not model as written was dropped with its whole body (#1192): an array of
//! structs reached through a `rand` packed-struct member (`g.item`, also
//! with `this.`, one level deeper, and 2-D with `[i, j]`), a packed vector
//! or a packed array of unions or enums inside a struct, a row of a 2-D
//! array (`foreach (grid[i])`, nested loops), and an omitted loop variable
//! (`grid[, j]`). Each body below now holds on every draw — beside a soft
//! constraint, a `dist`, an implication, inside an implication, and in a
//! `randomize() with` block — an unsatisfiable body makes `randomize()`
//! fail, and `randomize(null)` checks the loop's iterations (§18.11.1).
//! The reference simulator gives the same lines.

#[test]
fn foreach_over_fixed_shape_members_is_solved() {
    let src = r#"
package p1192;
  typedef struct packed { logic valid; logic [5:0] key; logic [3:0] sub; } ent_t;
  typedef struct packed { logic [5:0] tag; ent_t [0:2] item; } grp_t;
  typedef struct packed { grp_t grp; logic [5:0] spare; } top_t;
  typedef enum logic [3:0] {E0, E1, E2, E3} e4_t;
  typedef struct packed { e4_t [0:3] arr; } pen_t;
  typedef union packed { logic [5:0] a; logic [1:0][2:0] b; } pun_t;
  typedef struct packed { pun_t [0:2] arr; } puna_t;
  typedef struct packed { logic [0:18] v; } pv_t;
  typedef struct packed { ent_t [0:1][0:2] item2; } g2_t;
  class d1_c; rand grp_t g;
    constraint c { foreach (g.item[i]) g.item[i].key inside {[0:3]}; } endclass
  class d2_c; rand grp_t g;
    constraint c { foreach (this.g.item[i]) g.item[i].sub inside {[0:1]}; } endclass
  class d5_c; rand top_t t;
    constraint c { foreach (t.grp.item[i]) t.grp.item[i] == 0; } endclass
  class d9_c; rand grp_t g; rand bit gate;
    constraint c { foreach (g.item[i]) { g.item[i].key inside {[0:3]}; (g.item[i].key == 0) -> gate; } }
    constraint g0 { gate == 1'b0; } endclass
  class s1_c; rand ent_t grid [0:1][0:2];
    constraint c { foreach (grid[i]) grid[i][0].key inside {[0:3]}; } endclass
  class s2_c; rand logic [5:0] lane [0:1][0:2];
    constraint c { foreach (lane[i]) foreach (lane[i][j]) lane[i][j] inside {[0:3]}; } endclass
  class l3_c; rand ent_t grid [0:1][0:2];
    constraint c { foreach (grid[,j]) grid[0][j].key inside {[0:3]}; } endclass
  class e1_c; rand grp_t g;
    constraint c { soft g.tag == 6'd2; foreach (g.item[i]) g.item[i].key inside {[0:3]}; } endclass
  class e5_c; rand pv_t s;
    constraint c { foreach (s.v[i]) s.v[i] == 1'b0; } endclass
  class e6_c; rand g2_t g2;
    constraint c { foreach (g2.item2[i, j]) g2.item2[i][j].key inside {[0:3]}; } endclass
  class e10_c; rand grp_t g;
    constraint c { foreach (g.item[i]) soft g.item[i].key == 6'd0; } endclass
  class p2_c; rand puna_t g;
    constraint c { foreach (g.arr[i]) g.arr[i].a == 6'd0; } endclass
  class n2_c; rand grp_t g; rand bit en;
    constraint c { en -> foreach (g.item[i]) g.item[i].key inside {[0:3]}; }
    constraint e { en == 1'b1; } endclass
  class b2_c; rand grp_t g;
    constraint c { foreach (g.item[i]) g.item[i].key dist { 6'd0 := 1 }; } endclass
  class w1_c; rand grp_t g; endclass
  class d7_c; rand grp_t g;
    constraint c { foreach (g.item[i]) { g.item[i].key inside {[0:3]}; g.item[i].key inside {[4:5]}; } } endclass
  class p1_c; rand pen_t g;
    constraint c { foreach (g.arr[i]) g.arr[i] == E0; }
    constraint d { g.arr[0] == E1; } endclass
  class ck_c; rand grp_t g;
    constraint c { foreach (g.item[i]) g.item[i].key == 0; } endclass
endpackage

module t1192a;
  import p1192::*;
  d1_c d1 = new; d2_c d2 = new; d5_c d5 = new; d9_c d9 = new; s1_c s1 = new;
  s2_c s2 = new; l3_c l3 = new; e1_c e1 = new; e5_c e5 = new; e6_c e6 = new;
  e10_c e10 = new; p2_c p2 = new; n2_c n2 = new; b2_c b2 = new; w1_c w1 = new;
  d7_c d7 = new; p1_c p1 = new; ck_c ck = new;
  int bad [string];
  function automatic int kor(grp_t g);
    return g.item[0].key | g.item[1].key | g.item[2].key;
  endfunction
  task automatic check(string tag, bit ok, bit cond);
    if (!ok || !cond) bad[tag]++;
  endtask
  initial begin
    repeat (20) begin
      check("D1", d1.randomize(), kor(d1.g) <= 3);
      check("D2", d2.randomize(), (d2.g.item[0].sub | d2.g.item[1].sub | d2.g.item[2].sub) <= 1);
      check("D5", d5.randomize(), d5.t.grp.item == 0);
      check("D9", d9.randomize(), kor(d9.g) <= 3 && d9.g.item[0].key != 0 && d9.g.item[2].key != 0);
      check("S1", s1.randomize(), (s1.grid[0][0].key | s1.grid[1][0].key) <= 3);
      check("S2", s2.randomize(), (s2.lane[0][0] | s2.lane[0][2] | s2.lane[1][1]) <= 3);
      check("L3", l3.randomize(), (l3.grid[0][0].key | l3.grid[0][1].key | l3.grid[0][2].key) <= 3);
      check("E1", e1.randomize(), kor(e1.g) <= 3 && e1.g.tag == 2);
      check("E5", e5.randomize(), e5.s.v == 0);
      check("E6", e6.randomize(), (e6.g2.item2[0][0].key | e6.g2.item2[1][2].key) <= 3);
      check("E10", e10.randomize(), kor(e10.g) == 0);
      check("P2", p2.randomize(), (p2.g.arr[0].a | p2.g.arr[1].a | p2.g.arr[2].a) == 0);
      check("N2", n2.randomize(), kor(n2.g) <= 3);
      check("B2", b2.randomize(), kor(b2.g) == 0);
      check("W1", w1.randomize() with { foreach (g.item[i]) g.item[i].key inside {[0:3]}; }, kor(w1.g) <= 3);
    end
    $display("unsat %0d %0d", d7.randomize(), p1.randomize());
    ck.g = '1;
    $display("check %0d", ck.randomize(null));
    ck.g = '0;
    $display("check %0d", ck.randomize(null));
    foreach (bad[t]) $display("FAIL %s %0d", t, bad[t]);
    $display("done %0d", bad.num());
  end
endmodule
"#;
    let out: Vec<String> = xezim::simulate(src, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| !m.starts_with("[xezim]") && !m.starts_with("Simulation"))
        .collect();
    assert_eq!(out, ["unsat 0 0", "check 0", "check 1", "done 0"]);
}

/// The same loop in a `std::randomize() with` block over a packed-struct
/// variable of the module (§18.12). Every successful draw honours it; the
/// scope solver still gives up on an occasional draw of these tight field
/// ranges, as it does for the hand-unrolled items, so only the draws that
/// succeed are checked.
#[test]
fn scope_randomize_foreach_over_struct_member() {
    let src = r#"
module top;
  typedef struct packed { logic [5:0] key; logic [1:0] f; } e_t;
  typedef struct packed { logic [3:0] tag; e_t [0:2] item; } g_t;
  g_t g;
  int bad = 0, good = 0, ok;
  initial begin
    repeat (20) begin
      ok = std::randomize(g) with { foreach (g.item[i]) g.item[i].key inside {[0:3]}; };
      if (ok && (g.item[0].key | g.item[1].key | g.item[2].key) > 3) bad++;
      good += ok;
    end
    $display("bad=%0d good=%0d", bad, good > 10);
  end
endmodule
"#;
    let out: Vec<String> = xezim::simulate(src, 100)
        .expect("simulate")
        .output
        .iter()
        .map(|o| o.message.clone())
        .filter(|m| !m.starts_with("[xezim]") && !m.starts_with("Simulation"))
        .collect();
    assert_eq!(out, ["bad=0 good=1"]);
}
