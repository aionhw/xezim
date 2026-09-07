//! §18.5.7/§18.6.2: conditional bodies inside a `foreach` over a FIXED-shape
//! rand array — and the honesty of the result code.
//!
//! Three defects fixed together:
//! 1. `solve_forced_array_elem` had no `if/else` (or implication, or `soft`)
//!    arm, so every index-dependent branch of a fixed-array foreach body was
//!    silently DROPPED — `if (i == j) arr[i][j] % 2 == 0; else …` constrained
//!    nothing, in any number of dimensions.
//! 2. Bodies the structural repairer cannot solve (`% 2 == 0` is not an
//!    equality on the element) had no generate-and-test backstop on the
//!    fixed-array path, so even the unconditional form went unenforced.
//! 3. The final satisfaction check skipped fixed-array foreach items as
//!    unmodeled, so randomize() returned 1 WITH VIOLATING VALUES — including
//!    for provably unsatisfiable systems (the report's 3-D mesh with an
//!    EVEN = ODD parity contradiction). It now checks strictly: unsatisfied
//!    bodies retry the trial, and a persistent failure returns 0.
//!
//! The reference simulator agrees on every expectation here (verdicts and
//! per-element validity; it never terminated on the full 10x10x10 mesh, which
//! is why the UNSAT case below is a small one).

use xezim::simulate;

fn out(src: &str) -> String {
    let sim = simulate(src, 100).expect("simulate failed");
    sim.output
        .iter()
        .map(|o| o.message.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn conditional_foreach_bodies_hold_in_all_dimensions() {
    // 1-D, 2-D and 3-D fixed arrays with an index-conditional parity body.
    // The testbench itself re-checks every element and reports a violation
    // count; the expectation is zero across all three shapes.
    let o = out(r#"
module top;
  class c1;
    rand int g[4];
    constraint m { foreach (g[i]) { g[i] inside {[1:500]};
      if (i == 0) g[i] % 2 == 0; else g[i] % 2 != 0; } }
  endclass
  class c2;
    rand int g[3][3];
    constraint m { foreach (g[i, j]) { g[i][j] inside {[1:500]};
      if (i == j) g[i][j] % 2 == 0; else g[i][j] % 2 != 0; } }
  endclass
  class c3;
    rand int g[2][2][2];
    constraint m { foreach (g[i, j, k]) { g[i][j][k] inside {[1:500]};
      if (i == j && j == k) g[i][j][k] % 2 == 0; else g[i][j][k] % 2 != 0; } }
  endclass
  initial begin
    c1 a = new(); c2 b = new(); c3 c = new();
    int r1, r2, r3, v;
    r1 = a.randomize(); r2 = b.randomize(); r3 = c.randomize();
    v = 0;
    for (int i = 0; i < 4; i++) begin
      if (!(a.g[i] >= 1 && a.g[i] <= 500)) v++;
      if ((i == 0) != (a.g[i] % 2 == 0)) v++;
    end
    for (int i = 0; i < 3; i++) for (int j = 0; j < 3; j++) begin
      if (!(b.g[i][j] >= 1 && b.g[i][j] <= 500)) v++;
      if ((i == j) != (b.g[i][j] % 2 == 0)) v++;
    end
    for (int i = 0; i < 2; i++) for (int j = 0; j < 2; j++) for (int k = 0; k < 2; k++) begin
      if (!(c.g[i][j][k] >= 1 && c.g[i][j][k] <= 500)) v++;
      if ((i == j && j == k) != (c.g[i][j][k] % 2 == 0)) v++;
    end
    $display("R=%0d%0d%0d V=%0d", r1, r2, r3, v);
  end
endmodule
"#);
    assert!(o.contains("R=111 V=0"), "conditional foreach bodies violated:\n{o}");
}

#[test]
fn an_unsatisfiable_foreach_body_fails_randomize() {
    // `g[1]` must be both even and odd: no assignment exists, so randomize()
    // must return 0 (§18.6.2) — it used to return 1 with violating values.
    // The satisfiable sibling in the same run guards against over-rejection.
    let o = out(r#"
module top;
  class cu;
    rand int g[3];
    constraint m { foreach (g[i]) { g[i] inside {[1:10]};
      if (i == 1) { g[i] % 2 == 0; g[i] % 2 != 0; } } }
  endclass
  class cs;
    rand int g[3];
    constraint m { foreach (g[i]) { g[i] inside {[1:10]};
      if (i == 1) g[i] % 2 == 0; else g[i] % 2 != 0; } }
  endclass
  initial begin
    cu u = new(); cs s = new();
    int ru, rs, v;
    ru = u.randomize(); rs = s.randomize();
    v = 0;
    for (int i = 0; i < 3; i++) begin
      if (!(s.g[i] >= 1 && s.g[i] <= 10)) v++;
      if ((i == 1) != (s.g[i] % 2 == 0)) v++;
    end
    $display("UNSAT=%0d SAT=%0d V=%0d", ru, rs, v);
  end
endmodule
"#);
    assert!(o.contains("UNSAT=0 SAT=1 V=0"), "unsat/sat verdicts wrong:\n{o}");
}

#[test]
fn a_globally_coupled_monotone_mesh_is_solved_not_given_up_on() {
    // SATISFIABLE but every element depends on three predecessors, so uniform
    // per-element repicks exhaust `[1:2000]` along a nine-step chain and the
    // strict foreach check failed trial after trial — the give-up cap then
    // returned 0 on a valid constraint set. After a few strict-check
    // failures the relational repicker switches to the BOUND value
    // (minimal feasible, intersected with the body's `inside` range), which
    // solves a monotone chain greedily in index order. The reference solves
    // this too (it reaches the corner at 2000; we settle for the minimal
    // 19 — both are valid solutions).
    let o = out(r#"
module top;
  class valid_mesh_test;
    rand int grid[4][4][4];
    constraint c {
      foreach (grid[i, j, k]) {
        grid[i][j][k] inside {[1 : 2000]};
        if (i > 0) { grid[i][j][k] > grid[i-1][j][k]; }
        if (j > 0) { grid[i][j][k] > grid[i][j-1][k]; }
        if (k > 0) { grid[i][j][k] > grid[i][j][k-1]; }
        if (i == 0 && j == 0 && k == 0) { grid[i][j][k] == 10; }
      }
    }
  endclass
  initial begin
    valid_mesh_test o = new();
    int r, viol = 0;
    r = o.randomize();
    for (int i = 0; i < 4; i++) for (int j = 0; j < 4; j++) for (int k = 0; k < 4; k++) begin
      if (!(o.grid[i][j][k] >= 1 && o.grid[i][j][k] <= 2000)) viol++;
      if (i > 0 && !(o.grid[i][j][k] > o.grid[i-1][j][k])) viol++;
      if (j > 0 && !(o.grid[i][j][k] > o.grid[i][j-1][k])) viol++;
      if (k > 0 && !(o.grid[i][j][k] > o.grid[i][j][k-1])) viol++;
    end
    if (o.grid[0][0][0] != 10) viol++;
    $display("MESH r=%0d viol=%0d", r, viol);
  end
endmodule
"#);
    assert!(o.contains("MESH r=1 viol=0"), "valid monotone mesh not solved:\n{o}");
}

