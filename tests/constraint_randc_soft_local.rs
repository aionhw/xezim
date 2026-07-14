//! Constraint-solver compliance for IEEE 1800-2017 §18.4 (`randc`), §18.5.14
//! (soft constraints), §18.7.1 (`local::`) and §18.11 (the in-line constraint
//! checker, `randomize(null)`). GitHub issue #30.
//!
//! - §18.4: a `randc` variable must visit EVERY value of its range exactly once
//!   before any value repeats. The solver retries a failed solution up to 1000
//!   times, and each retry used to draw (and burn) another value from the
//!   permutation cycle — so a 2-bit `randc` alongside a hard-to-satisfy
//!   constraint would skip values, i.e. stop being cyclic.
//! - §18.5.14.2: a `soft` constraint in a DERIVED class overrides a conflicting
//!   `soft` constraint inherited from its base. The base's soft constraint used
//!   to be applied last and clobber the derived one.
//! - §18.7.1: inside an in-line `with {}` constraint a bare name binds to the
//!   OBJECT's member; `local::name` reaches the CALLER's same-named variable.
//! - §18.11: `obj.randomize(null)` randomizes nothing — it CHECKS the object's
//!   current values against its constraints and returns 0 when one is violated.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
        & 0xFFFF_FFFF
}

/// §18.4 — a `randc bit [1:0]` must yield a full permutation of {0,1,2,3} in
/// every group of four calls. `data` carries a constraint the random seeding
/// only satisfies about 1 time in 16, forcing the solver to retry: those
/// retries must not consume values out of the cyclic permutation.
const RANDC_SRC: &str = r#"
class Bus;
  rand  bit [15:0] addr;
  rand  bit [31:0] data;
  randc bit [1:0]  cyclic_id;
  constraint word_align { addr[1:0] == 2'b0; }
  constraint data_bounds {
    data >= 32'h1000_0000;
    data <= 32'h2000_0000;
  }
endclass

module tb;
  int missed_in_cycle;   // values never seen within a 4-draw cycle
  int bad_status;
  int bad_addr;
  int bad_data;
  initial begin
    Bus bus = new();
    bit seen[4];
    // 25 back-to-back permutation cycles.
    for (int c = 0; c < 25; c++) begin
      for (int i = 0; i < 4; i++) seen[i] = 0;
      repeat (4) begin
        if (bus.randomize() != 1) bad_status++;
        seen[bus.cyclic_id] = 1;
        if (bus.addr[1:0] != 2'b0) bad_addr++;
        if (!(bus.data >= 32'h1000_0000 && bus.data <= 32'h2000_0000)) bad_data++;
      end
      for (int i = 0; i < 4; i++) if (!seen[i]) missed_in_cycle++;
    end
  end
endmodule
"#;

#[test]
fn randc_visits_every_value_once_per_permutation_cycle() {
    let sim = simulate(RANDC_SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "bad_status"), 0, "randomize() failed");
    assert_eq!(
        u(&sim, "missed_in_cycle"),
        0,
        "randc skipped a value: the cycle is not a permutation (§18.4)"
    );
    // The retried trials must still honour the ordinary constraints.
    assert_eq!(u(&sim, "bad_addr"), 0, "word_align violated");
    assert_eq!(u(&sim, "bad_data"), 0, "data_bounds violated");
}

/// §18.5.14.2 / §18.5.14.3 — the derived class's `soft len == 20` overrides the
/// `soft len == 10` it inherits, while the base's non-conflicting
/// `soft val inside {[0:50]}` survives. §18.5.14.1: a hard in-line constraint
/// overrides ANY layer of the soft hierarchy.
const SOFT_SRC: &str = r#"
class BasePacket;
  rand bit [7:0] len;
  rand bit [7:0] val;
  constraint base_soft_rules {
    soft len == 10;
    soft val inside {[0:50]};
  }
endclass

class DerivedPacket extends BasePacket;
  constraint derived_soft_rules {
    soft len == 20;
  }
endclass

module tb;
  int derived_soft_lost;   // base soft clobbered the derived one
  int base_soft_dropped;   // non-conflicting base soft was lost
  int inline_hard_lost;    // hard inline failed to beat the soft hierarchy
  int bad_status;
  initial begin
    DerivedPacket pkt = new();
    repeat (20) begin
      if (pkt.randomize() != 1) bad_status++;
      if (pkt.len != 20) derived_soft_lost++;
      if (!(pkt.val >= 0 && pkt.val <= 50)) base_soft_dropped++;

      if (pkt.randomize() with { len == 200; val == 222; } != 1) bad_status++;
      if (pkt.len != 200 || pkt.val != 222) inline_hard_lost++;
    end
  end
endmodule
"#;

#[test]
fn derived_soft_constraint_overrides_the_inherited_one() {
    let sim = simulate(SOFT_SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "bad_status"), 0, "randomize() failed");
    assert_eq!(
        u(&sim, "derived_soft_lost"),
        0,
        "base `soft len == 10` overrode derived `soft len == 20` (§18.5.14.2)"
    );
    assert_eq!(
        u(&sim, "base_soft_dropped"),
        0,
        "non-conflicting base soft constraint was dropped (§18.5.14.3)"
    );
    assert_eq!(
        u(&sim, "inline_hard_lost"),
        0,
        "hard in-line constraint did not override the soft hierarchy (§18.5.14.1)"
    );
}

/// §18.11 / §18.11.1 — `randomize(null)` is a CHECKER: it assigns nothing and
/// reports whether the object's current values satisfy the active class
/// constraints plus any in-line `with {}` ones.
const CHECK_SRC: &str = r#"
class Bus;
  rand bit [7:0] data_val;
  constraint legal { data_val inside {8'h05, [8'hA0:8'hAF]}; }
endclass

module tb;
  int ok_valid;        // expect 1: current value satisfies the constraint
  int ok_illegal;      // expect 0: 8'h55 is outside `legal`
  int ok_inline_hit;   // expect 1: inline matches the current value
  int ok_inline_miss;  // expect 0: inline contradicts the current value
  int mutated;         // the checker must not modify the object
  initial begin
    Bus bus = new();

    bus.data_val = 8'h05;
    ok_valid = bus.randomize(null);

    bus.data_val = 8'h55;
    ok_illegal = bus.randomize(null);
    if (bus.data_val != 8'h55) mutated++;   // randomize(null) randomizes nothing

    bus.data_val = 8'hA5;
    ok_inline_hit  = bus.randomize(null) with { data_val == 8'hA5; };
    ok_inline_miss = bus.randomize(null) with { data_val == 8'h00; };
    if (bus.data_val != 8'hA5) mutated++;
  end
endmodule
"#;

#[test]
fn randomize_null_checks_instead_of_randomizing() {
    let sim = simulate(CHECK_SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "ok_valid"), 1, "checker rejected a legal value (§18.11)");
    assert_eq!(
        u(&sim, "ok_illegal"),
        0,
        "checker failed to catch an illegal value (§18.11)"
    );
    assert_eq!(
        u(&sim, "ok_inline_hit"),
        1,
        "checker rejected a value its in-line constraint accepts (§18.11.1)"
    );
    assert_eq!(
        u(&sim, "ok_inline_miss"),
        0,
        "checker failed to catch an unmatched in-line condition (§18.11.1)"
    );
    assert_eq!(u(&sim, "mutated"), 0, "randomize(null) changed a rand variable");
}

/// §18.7.1 — `local::data_val` inside the in-line constraint names the CALLER's
/// `data_val`, not the object's same-named rand member (which is what the bare
/// name would bind to). Without it the constraint is the tautology
/// `data_val == data_val` and the object keeps an arbitrary value.
const LOCAL_SRC: &str = r#"
class Bus;
  rand bit [7:0] data_val;
  constraint legal { data_val inside {[8'h00:8'hFF]}; }
endclass

module tb;
  int bad_status;
  int wrong_binding;
  initial begin
    Bus bus = new();
    bit [7:0] data_val;    // shadows the class member's name
    for (int i = 0; i < 16; i++) begin
      data_val = 8'hA0 + i;
      if (bus.randomize() with { data_val == local::data_val; } != 1) bad_status++;
      if (bus.data_val != data_val) wrong_binding++;
    end
  end
endmodule
"#;

#[test]
fn local_scope_binds_the_callers_variable() {
    let sim = simulate(LOCAL_SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "bad_status"), 0, "randomize() with local:: failed");
    assert_eq!(
        u(&sim, "wrong_binding"),
        0,
        "local::data_val did not resolve to the caller's variable (§18.7.1)"
    );
}
