//! Issue #30 — randomization of COLLECTION members, `unique {}`, static
//! constraint blocks and `solve … before …` variable ordering.
//!
//! IEEE 1800-2017:
//!   * §18.4    — a rand dynamic array whose `.size()` is constrained must be
//!                SIZED before its elements exist; only then can the element
//!                constraints (`foreach`) be solved.
//!   * §18.5.5  — `unique {c}` over a queue / associative array: every element
//!                pairwise distinct, inside the element's legal domain.
//!   * §18.5.9  — global (cross-object) constraints: a `rand` class handle is
//!                randomized recursively, and the enclosing object's
//!                constraints may pin the sub-object's rand members.
//!   * §18.5.10 — variable ordering. WITHOUT `solve X before Y` the solver must
//!                distribute over the joint SOLUTION SPACE (so an implication
//!                that leaves a single solution for X == 0 makes X == 0 rare);
//!                WITH it, X is drawn first and Y solved against it (so X == 0
//!                is drawn about half the time).
//!   * §18.5.11 — a `static constraint` block is shared by every instance:
//!                `constraint_mode()` on it through one instance changes it for
//!                all of them.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
}

/// §18.4 — dynamic arrays, queues, associative arrays and a 2-D dynamic array,
/// all sized from `.size()` constraints and then element-solved by `foreach`.
/// The `% 2 == 0` body is the shape no structural pattern matcher covers: it
/// exercises the bounded generate-and-test backstop.
#[test]
fn sized_dynamic_arrays_and_foreach_elements() {
    const SRC: &str = r#"
class Bus;
  rand bit [7:0] dyn_arr[];
  rand bit [7:0] q[$];
  rand bit [7:0] aa[bit [3:0]];
  rand bit [7:0] grid[][];
  int off = 10;

  constraint sizes {
    dyn_arr.size() inside {[3:6]};
    q.size() == 4;
    grid.size() == 3;
  }
  constraint elems {
    foreach (dyn_arr[j]) { dyn_arr[j] == (j * 5) + off; }
    foreach (q[k])       { q[k] % 2 == 0; }
    foreach (grid[i])    { grid[i].size() == (i + 2); }
    foreach (grid[i, j]) { grid[i][j] == (i * 10) + j; }
  }
  function void pre_randomize();
    aa[4'h2] = 8'hAA;
    aa[4'h5] = 8'hBB;
  endfunction
endclass

module tb;
  int failures = 0;
  int status_bad = 0;
  initial begin
    Bus b = new();
    int st;
    repeat (10) begin
      st = b.randomize();
      if (st != 1) status_bad++;

      if (b.dyn_arr.size() < 3 || b.dyn_arr.size() > 6) failures++;
      foreach (b.dyn_arr[j])
        if (b.dyn_arr[j] != (j * 5) + b.off) failures++;

      if (b.q.size() != 4) failures++;
      foreach (b.q[k])
        if (b.q[k] % 2 != 0) failures++;

      // Associative elements are re-drawn (not left at their seeds).
      if (b.aa[4'h2] == 8'hAA) failures++;
      if (b.aa[4'h5] == 8'hBB) failures++;

      if (b.grid.size() != 3) failures++;
      foreach (b.grid[i])
        if (b.grid[i].size() != (i + 2)) failures++;
      foreach (b.grid[i, j])
        if (b.grid[i][j] != (i * 10) + j) failures++;
    end
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(
        u(&sim, "status_bad"),
        0,
        "§18.4 randomize() of sized collections must return 1"
    );
    assert_eq!(
        u(&sim, "failures"),
        0,
        "§18.4/§18.5.7 collection size + element constraints must all hold"
    );
}

/// §18.5.5 `unique {}` over a queue and an associative array, plus §18.5.9 a
/// cross-object constraint through a `rand` class handle.
#[test]
fn unique_collections_and_cross_object_constraint() {
    const SRC: &str = r#"
class Sub;
  rand bit [7:0] sub_data;
endclass

class Top;
  rand Sub        sub_inst;
  rand bit [7:0]  q[$];
  rand bit [7:0]  aa[int];

  constraint rules {
    q.size() == 4;
    foreach (q[i]) { q[i] inside {[1:50]}; }
    unique { q };
    unique { aa };
    sub_inst.sub_data == q[0] + 8'd5;
  }
  function new();
    sub_inst = new();
    aa[100] = 0;
    aa[200] = 0;
    aa[300] = 0;
  endfunction
endclass

module tb;
  int failures = 0;
  initial begin
    Top t = new();
    int st;
    repeat (20) begin
      st = t.randomize();
      if (st != 1) failures++;
      // §18.5.9 cross-object equality solved against the queue's element.
      if (t.sub_inst.sub_data != t.q[0] + 8'd5) failures++;
      // §18.5.5 distinct elements, inside the element domain.
      foreach (t.q[i])
        if (t.q[i] < 1 || t.q[i] > 50) failures++;
      if (t.q[0] == t.q[1] || t.q[0] == t.q[2] || t.q[0] == t.q[3]) failures++;
      if (t.q[1] == t.q[2] || t.q[1] == t.q[3] || t.q[2] == t.q[3]) failures++;
      if (t.aa[100] == t.aa[200]) failures++;
      if (t.aa[200] == t.aa[300]) failures++;
      if (t.aa[100] == t.aa[300]) failures++;
    end
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(
        u(&sim, "failures"),
        0,
        "§18.5.5 unique{{}} over queue/assoc and §18.5.9 cross-object equality"
    );
}

/// §18.5.11 — a `static constraint` block is shared by ALL instances: turning
/// it off through instance A must turn it off for instance B, and randomizing
/// afterwards must be free of the (now inactive) bound. §18.7 inline `with`
/// constraints must also reach a queue ELEMENT.
#[test]
fn static_constraint_block_is_shared_across_instances() {
    const SRC: &str = r#"
class Bus;
  rand bit [7:0] q[$];

  static constraint static_range {
    foreach (q[i]) { q[i] inside {[1:50]}; }
  }
  constraint sizing { q.size() == 4; }
endclass

module tb;
  int failures = 0;
  initial begin
    Bus a = new();
    Bus b = new();
    int st;

    // Baseline: active on both instances.
    if (a.static_range.constraint_mode() != 1) failures++;
    if (b.static_range.constraint_mode() != 1) failures++;
    st = a.randomize();
    if (st != 1) failures++;
    foreach (a.q[i])
      if (a.q[i] < 1 || a.q[i] > 50) failures++;

    // Disabling through A must be visible through B (§18.5.11).
    a.static_range.constraint_mode(0);
    if (b.static_range.constraint_mode() != 0) failures++;

    // With the block off, an inline constraint can leave its old range.
    st = a.randomize() with { q[0] == 8'd99; };
    if (st != 1) failures++;
    if (a.q[0] != 8'd99) failures++;

    // Re-enabling through B restores it for A.
    b.static_range.constraint_mode(1);
    if (a.static_range.constraint_mode() != 1) failures++;
    st = a.randomize();
    if (st != 1) failures++;
    foreach (a.q[i])
      if (a.q[i] < 1 || a.q[i] > 50) failures++;
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(
        u(&sim, "failures"),
        0,
        "§18.5.11 static constraint block state must be class-wide"
    );
}

/// §18.5.10 — variable ordering.
///
/// `(c == 0) -> (d == 0)` leaves ONE solution with c == 0 and 256 with c == 1.
/// Solved simultaneously (no ordering hint) the draw is spread over the joint
/// solution space, so c == 0 is rare (~1/257). `solve c before d` draws c
/// first, so c == 0 comes up about half the time — that distribution shift IS
/// the observable effect the LRM ascribes to the ordering hint.
#[test]
fn solve_before_shifts_the_distribution() {
    const SRC: &str = r#"
class Base;
  rand bit       c;
  rand bit [7:0] d;
  constraint imp { (c == 1'b0) -> (d == 8'h00); }
endclass

class Ordered extends Base;
  constraint ord { solve c before d; }
endclass

module tb;
  int base_zero = 0;
  int ordered_zero = 0;
  int bad = 0;
  initial begin
    Base    b = new();
    Ordered o = new();
    repeat (200) begin
      if (b.randomize() != 1) bad++;
      if (o.randomize() != 1) bad++;
      // The constraint itself must hold in BOTH cases.
      if (b.c == 1'b0 && b.d != 8'h00) bad++;
      if (o.c == 1'b0 && o.d != 8'h00) bad++;
      if (b.c == 1'b0) base_zero++;
      if (o.c == 1'b0) ordered_zero++;
    end
  end
endmodule
"#;
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "bad"), 0, "the implication must hold on every draw");
    let base_zero = u(&sim, "base_zero");
    let ordered_zero = u(&sim, "ordered_zero");
    assert!(
        base_zero < 20,
        "§18.5.10: without `solve … before …` the solution space (1 of 257 has \
         c == 0) must make c == 0 rare; got {}/200",
        base_zero
    );
    assert!(
        ordered_zero > 60,
        "§18.5.10: `solve c before d` must draw c first, so c == 0 lands about \
         half the time; got {}/200",
        ordered_zero
    );
}
