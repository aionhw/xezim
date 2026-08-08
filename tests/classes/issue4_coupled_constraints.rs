//! Issue #4: COUPLED multi-variable constraints in the constraint solver.
//!
//! IEEE 1800-2017 §18.5.12 solves a constraint set as a WHOLE. xezim's solver
//! narrowed each rand variable's [lo,hi] interval INDEPENDENTLY, so a
//! constraint that references two targets (`A + B < 1000`, `A < B`,
//! `A + B == 50`) narrowed nothing: both variables were drawn full-range and
//! the coupled constraint was essentially never satisfied by a random draw, so
//! `std::randomize` honestly reported 0 (and the class path 1000-trial retry
//! also gave up). The reporter's `std::randomize(A, B) with {(A+B) < 1000;}`
//! returned 0 where commercial simulators return 1.
//!
//! The fix adds a sequential / propagation solve (`solve_coupled_affine`):
//! order the coupled variables, assign them one at a time treating assigned
//! ones as constants and still-unassigned ones at the domain extreme that
//! keeps a feasible completion, and draw each uniformly from its narrowed
//! range. It is wired into BOTH the `std::randomize` path (`eval_randomize_with`)
//! and the class `randomize()` path (`exec_randomize_inner`).

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("test.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} is X", n))
}

/// Run `src` under a specific `+seed` and return the finished simulator.
fn run_seed(src: &str, seed: u64) -> xezim::compiler::Simulator {
    let plus = vec![format!("seed={}", seed)];
    xezim::simulate_multi(
        &[src.to_string()],
        1000,
        xezim::SimOptions {
            plusargs: plus.to_vec(),
            ..Default::default()
        },
    )
    .expect("simulate failed")
}

/// The reporter's exact case: `std::randomize(A, B) with {(A + B) < 1000;}`
/// must report success AND actually hold `A + B < 1000`, across seeds, with the
/// solved values varying (a real distribution, not one fixed point).
#[test]
fn std_randomize_a_plus_b_lt_1000() {
    const SRC: &str = r#"
module test;
    logic [31:0] A, B;
    int rstatus = 99;
    logic [31:0] gotA, gotB;
    initial begin
        rstatus = std::randomize(A, B) with {(A + B) < 1000;};
        gotA = A;
        gotB = B;
    end
endmodule
"#;
    let mut seen = std::collections::HashSet::new();
    for seed in [1u64, 2, 3, 7, 42, 99, 12345] {
        let sim = run_seed(SRC, seed);
        assert_eq!(u(&sim, "rstatus"), 1, "seed {}: must report success", seed);
        let a = u(&sim, "gotA");
        let b = u(&sim, "gotB");
        assert!(
            a + b < 1000,
            "seed {}: A={} B={} sum={} must be < 1000",
            seed,
            a,
            b,
            a + b
        );
        seen.insert((a, b));
    }
    assert!(seen.len() > 1, "solved values must vary across seeds");
}

/// The class-`randomize()` path must solve the same coupled constraint.
#[test]
fn class_randomize_a_plus_b_lt_1000() {
    const SRC: &str = r#"
class C; rand logic [31:0] A, B; endclass
module test;
    int rstatus = 99;
    logic [31:0] gotA, gotB;
    initial begin
        C c = new();
        rstatus = c.randomize() with {(A + B) < 1000;};
        gotA = c.A;
        gotB = c.B;
    end
endmodule
"#;
    let mut seen = std::collections::HashSet::new();
    for seed in [1u64, 2, 3, 7, 42, 99, 12345] {
        let sim = run_seed(SRC, seed);
        assert_eq!(u(&sim, "rstatus"), 1, "seed {}: must report success", seed);
        let a = u(&sim, "gotA");
        let b = u(&sim, "gotB");
        assert!(
            a + b < 1000,
            "seed {}: A={} B={} sum={} must be < 1000",
            seed,
            a,
            b,
            a + b
        );
        seen.insert((a, b));
    }
    assert!(seen.len() > 1, "solved values must vary across seeds");
}

/// A chain of coupled + single-variable bounds: `A < B; A > 100; B < 200` must
/// yield `100 < A < B < 200`, on both entry points and across seeds.
#[test]
fn coupled_chain_a_lt_b_bounded() {
    let cases = [
        (
            "std",
            r#"
module test;
    logic [31:0] A, B;
    int rstatus = 99;
    logic [31:0] gotA, gotB;
    initial begin
        rstatus = std::randomize(A, B) with {A < B; A > 100; B < 200;};
        gotA = A; gotB = B;
    end
endmodule
"#,
        ),
        (
            "class",
            r#"
class C; rand logic [31:0] A, B; endclass
module test;
    int rstatus = 99;
    logic [31:0] gotA, gotB;
    initial begin
        C c = new();
        rstatus = c.randomize() with {A < B; A > 100; B < 200;};
        gotA = c.A; gotB = c.B;
    end
endmodule
"#,
        ),
    ];
    for (label, src) in cases {
        for seed in [1u64, 2, 3, 7, 42, 99] {
            let sim = run_seed(src, seed);
            assert_eq!(u(&sim, "rstatus"), 1, "{} seed {}: must succeed", label, seed);
            let a = u(&sim, "gotA");
            let b = u(&sim, "gotB");
            assert!(
                a > 100 && a < b && b < 200,
                "{} seed {}: need 100<A<B<200, got A={} B={}",
                label,
                seed,
                a,
                b
            );
        }
    }
}

/// Coupled EQUALITY `A + B == 50`: the sum must be exactly 50 (the last solved
/// variable is pinned to the required value), across seeds, and vary.
#[test]
fn coupled_equality_sum() {
    let cases = [
        (
            "std",
            r#"
module test;
    logic [31:0] A, B;
    int rstatus = 99;
    logic [31:0] gotA, gotB;
    initial begin
        rstatus = std::randomize(A, B) with {A + B == 50;};
        gotA = A; gotB = B;
    end
endmodule
"#,
        ),
        (
            "class",
            r#"
class C; rand logic [31:0] A, B; endclass
module test;
    int rstatus = 99;
    logic [31:0] gotA, gotB;
    initial begin
        C c = new();
        rstatus = c.randomize() with {A + B == 50;};
        gotA = c.A; gotB = c.B;
    end
endmodule
"#,
        ),
    ];
    for (label, src) in cases {
        let mut seen = std::collections::HashSet::new();
        for seed in [1u64, 2, 3, 7, 42, 99] {
            let sim = run_seed(src, seed);
            assert_eq!(u(&sim, "rstatus"), 1, "{} seed {}: must succeed", label, seed);
            let a = u(&sim, "gotA");
            let b = u(&sim, "gotB");
            assert_eq!(a + b, 50, "{} seed {}: A={} B={} sum must be 50", label, seed, a, b);
            seen.insert(a);
        }
        assert!(seen.len() > 1, "{}: A must vary across seeds", label);
    }
}

/// Coupled equality `A - B == 0` forces `A == B` (equal, non-trivial values).
#[test]
fn coupled_equality_difference() {
    const SRC: &str = r#"
module test;
    logic [31:0] A, B;
    int rstatus = 99;
    logic [31:0] gotA, gotB;
    initial begin
        rstatus = std::randomize(A, B) with {A - B == 0;};
        gotA = A; gotB = B;
    end
endmodule
"#;
    let mut seen = std::collections::HashSet::new();
    for seed in [1u64, 2, 3, 7, 42, 99] {
        let sim = run_seed(SRC, seed);
        assert_eq!(u(&sim, "rstatus"), 1, "seed {}: must succeed", seed);
        let a = u(&sim, "gotA");
        let b = u(&sim, "gotB");
        assert_eq!(a, b, "seed {}: A={} B={} must be equal", seed, a, b);
        seen.insert(a);
    }
    assert!(seen.len() > 1, "A==B value must vary across seeds");
}

/// §18.11 honesty preserved: a genuinely UNSATISFIABLE coupled set
/// (`A + B < 1000 AND A + B > 2000`) must report 0, not a false 1.
#[test]
fn coupled_unsat_reports_zero() {
    const STD: &str = r#"
module test;
    logic [31:0] A, B;
    int rstatus = 99;
    initial begin
        rstatus = std::randomize(A, B) with {(A + B) < 1000; (A + B) > 2000;};
    end
endmodule
"#;
    const CLS: &str = r#"
class C; rand logic [31:0] A, B; endclass
module test;
    int rstatus = 99;
    initial begin
        C c = new();
        rstatus = c.randomize() with {(A + B) < 1000; (A + B) > 2000;};
    end
endmodule
"#;
    for (label, src) in [("std", STD), ("class", CLS)] {
        let sim = simulate(src, 1000).expect("simulate failed");
        assert_eq!(
            u(&sim, "rstatus"),
            0,
            "{}: unsatisfiable coupled set must report 0",
            label
        );
    }
}

/// §18.3: `std::randomize` of an enum-typed variable draws only DECLARED
/// member values — a raw width draw yielded out-of-member encodings (8..15
/// for an 8-member `enum bit [3:0]`), breaking bounds checks and `.name()`.
#[test]
fn std_randomize_enum_draws_declared_members_only() {
    const SRC: &str = r#"
package p;
  typedef enum bit [3:0] { A0=0, A1=1, A2=2, A3=3, A4=4, A5=5, A6=6, A7=7 } op_e;
endpackage
module tb;
  import p::*;
  op_e op;
  int bad = 0;
  int uniq = 0;
  bit seen [16];
  initial begin
    repeat (200) begin
      void'(std::randomize(op));
      if (!(op inside {A0,A1,A2,A3,A4,A5,A6,A7})) bad++;
      seen[op] = 1;
    end
    foreach (seen[i]) if (seen[i]) uniq++;
  end
endmodule
"#;
    let sim = simulate(SRC, 10_000).expect("simulate failed");
    assert_eq!(u(&sim, "bad"), 0, "every draw must be a declared enum member");
    assert!(u(&sim, "uniq") > 1, "distribution must cover multiple members");
}

/// §18.3 siblings of `std_randomize_enum_draws_declared_members_only`: the
/// STRUCT-field, ARRAY-element and QUEUE-element paths of `exec_std_randomize`
/// still drew raw width-masked bits for an enum-typed destination, yielding
/// out-of-member encodings (8..15 for an 8-member `enum bit [3:0]`).
#[test]
fn std_randomize_enum_struct_array_queue_declared_members_only() {
    const SRC: &str = r#"
package p;
  typedef enum bit [3:0] { A0=0, A1=1, A2=2, A3=3, A4=4, A5=5, A6=6, A7=7 } op_e;
endpackage
module tb;
  import p::*;
  typedef struct { op_e op; int x; } s_t;
  s_t s;
  op_e arr [4];
  op_e q [$];
  int bad_s = 0, bad_a = 0, bad_q = 0;
  int uniq = 0;
  bit seen [16];
  initial begin
    q.push_back(A0); q.push_back(A0);
    repeat (50) begin
      void'(std::randomize(s));
      if (!(s.op inside {A0,A1,A2,A3,A4,A5,A6,A7})) bad_s++;
      seen[s.op] = 1;
      void'(std::randomize(arr));
      foreach (arr[i]) begin
        if (!(arr[i] inside {A0,A1,A2,A3,A4,A5,A6,A7})) bad_a++;
        seen[arr[i]] = 1;
      end
      void'(std::randomize(q));
      foreach (q[i]) begin
        if (!(q[i] inside {A0,A1,A2,A3,A4,A5,A6,A7})) bad_q++;
        seen[q[i]] = 1;
      end
    end
    foreach (seen[i]) if (seen[i]) uniq++;
  end
endmodule
"#;
    for seed in [1u64, 7, 42] {
        let sim = run_seed(SRC, seed);
        assert_eq!(u(&sim, "bad_s"), 0, "seed {}: struct enum field out of member set", seed);
        assert_eq!(u(&sim, "bad_a"), 0, "seed {}: array enum element out of member set", seed);
        assert_eq!(u(&sim, "bad_q"), 0, "seed {}: queue enum element out of member set", seed);
        assert!(u(&sim, "uniq") > 1, "seed {}: distribution must cover multiple members", seed);
    }
}
