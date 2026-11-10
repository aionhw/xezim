//! `foreach` constraints that apply `%` (modulo) to array elements
//! (IEEE 1800-2017 §18.5.7 / §11.4.4).
//!
//! The per-element repair in `solve_forced_array_elem` recognises
//! equality / relational / `inside` bodies; a modulo-equality body
//! (`(arr[i] % k) == r`) fell through to the generate-and-test backstop,
//! whose 24 full-width draws almost never land on the congruence class,
//! after which the strict §18.6.2 final check rejected every trial —
//! `randomize()` returned 0 on a satisfiable constraint set. A repair
//! shape now draws the element directly from the admitted congruence
//! class (verifying the draw through the ordinary evaluator); the final
//! check itself is unchanged.

use xezim::simulate;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
}

/// `(base[i] % 256) == 16` over every element: satisfiable, and every
/// drawn element must be congruent to 16 (mod 256).
#[test]
fn foreach_modulo_remainder() {
    const SRC: &str = "class c_rem;
  rand int unsigned base[3];
  constraint c_al { foreach (base[i]) (base[i] % 256) == 16; }
endclass

module tb;
  int ok = 0;
  int failures = 0;
  initial begin
    c_rem c = new;
    ok = c.randomize();
    if (ok != 1) failures++;
    for (int i = 0; i < 3; i++)
      if ((c.base[i] % 256) != 16) failures++;
  end
endmodule
";
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "ok"), 1, "randomize must succeed");
    assert_eq!(
        u(&sim, "failures"),
        0,
        "every element must satisfy (base[i] % 256) == 16"
    );
}

/// Variation: the modulus is ANOTHER array's element
/// (`(base[i] % sz[i]) == 0`, `sz[i]` drawn from an `inside` set on a
/// base class) — the cross-array shape.
#[test]
fn foreach_modulo_cross_array() {
    const SRC: &str = "class sizes_only;
  rand int unsigned sz[3];
  constraint c_sz { foreach (sz[i]) sz[i] inside {256, 512, 1024}; }
endclass

class with_alignment extends sizes_only;
  rand int unsigned base[3];
  constraint c_al { foreach (base[i]) (base[i] % sz[i]) == 0; }
endclass

module tb;
  int ok = 0;
  int failures = 0;
  initial begin
    with_alignment c = new;
    ok = c.randomize();
    if (ok != 1) failures++;
    for (int i = 0; i < 3; i++) begin
      if (!(c.sz[i] inside {256, 512, 1024})) failures++;
      if ((c.base[i] % c.sz[i]) != 0) failures++;
    end
  end
endmodule
";
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "ok"), 1, "randomize must succeed");
    assert_eq!(
        u(&sim, "failures"),
        0,
        "every base must be a multiple of its sz"
    );
}

/// Negative control: a remainder that can never occur (`% 256 == 300`)
/// is genuinely UNSAT — `randomize()` must still return 0. The repair
/// must never manufacture a solution the final check would reject.
#[test]
fn foreach_modulo_unsat_still_fails() {
    const SRC: &str = "class c_unsat;
  rand int unsigned base[3];
  constraint c_al { foreach (base[i]) (base[i] % 256) == 300; }
endclass

module tb;
  int ok = 1;
  initial begin
    c_unsat c = new;
    ok = c.randomize();
  end
endmodule
";
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(
        u(&sim, "ok"),
        0,
        "a remainder >= the modulus is UNSAT and must return 0"
    );
}
