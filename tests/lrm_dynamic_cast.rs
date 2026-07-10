//! IEEE 1800-2017 §6.24.1 `$cast` — the language's only runtime type check.
//!
//! It reported success for everything:
//!   - `cast_type_ok` looked the destination's class up in `var_class_types`,
//!     which only records PROCEDURAL locals. A module-scope class handle fell
//!     into the permissive "unknown dest type" branch, so every downcast —
//!     including a sibling-class cast — returned 1 and assigned.
//!   - An enum destination was never checked at all: the integer source is not
//!     a live heap object, so the "not a class object" escape hatch waved it
//!     through and `$cast(e, 5)` on `enum {A,B,C}` returned 1.
//!
//! A failing `$cast` must return 0 and leave the destination unchanged.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  class Base;    int x; endclass
  class Derived extends Base; int y; endclass
  class Other   extends Base; int z; endclass

  typedef enum { A, B, C } e_t;

  Base    b, b2;
  Derived d;
  Other   o;
  e_t     e;

  int up, down_valid, down_invalid, sibling, dest_changed;
  int enum_valid, enum_bad, enum_after;
  int null_ok;

  initial begin
    // An upcast is always legal.
    d = new();
    up = $cast(b, d);

    // A downcast of a Base handle that really points at a Derived is legal.
    b2 = d;
    down_valid = $cast(d, b2);

    // A downcast of a pure Base must fail and leave the destination alone.
    b = new();
    d = null;
    down_invalid = $cast(d, b);
    dest_changed = (d == null) ? 0 : 1;

    // A sibling cast must fail.
    sibling = $cast(o, b2);

    // null assigns through.
    b = null;
    null_ok = $cast(d, b);

    // Enum destinations: in range succeeds, out of range fails.
    e = A;
    enum_valid = $cast(e, 2);
    enum_bad   = $cast(e, 5);
    enum_after = e;          // must still hold the value from the good cast
  end
endmodule
"#;

fn u(sim: &xezim::compiler::Simulator, n: &str) -> u64 {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
        .to_u64()
        .unwrap_or_else(|| panic!("{} not u64-able", n))
        & 0xFFFF_FFFF
}

#[test]
fn a_valid_class_cast_succeeds() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "up"), 1, "an upcast must succeed");
    assert_eq!(u(&sim, "down_valid"), 1, "a downcast to the real type must succeed");
    assert_eq!(u(&sim, "null_ok"), 1, "null assigns through");
}

#[test]
fn an_invalid_class_cast_fails_and_leaves_the_destination_alone() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "down_invalid"), 0, "downcasting a pure Base must fail");
    assert_eq!(u(&sim, "dest_changed"), 0, "a failed $cast must not assign");
    assert_eq!(u(&sim, "sibling"), 0, "a sibling-class cast must fail");
}

#[test]
fn an_enum_cast_checks_the_value_is_a_member() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "enum_valid"), 1);
    assert_eq!(u(&sim, "enum_bad"), 0, "$cast to an enum accepted an out-of-range value");
    // The failed cast must not have clobbered the value the good one wrote.
    assert_eq!(u(&sim, "enum_after"), 2);
}
