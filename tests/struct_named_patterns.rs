//! Member-NAMED struct assignment patterns (IEEE 1800-2017 §10.9.2,
//! `member_identifier: expression` keys), alone and mixed with `default:`,
//! onto both unpacked (member-wise spread) and packed (bit-layout) structs.
//!
//! These regressed when block-local `typedef` statements were discarded at
//! parse time, so the pattern had no struct type to bind names against; the
//! module-scope cases here pin the named-key binding itself.

use xezim::simulate;

const SRC: &str = r#"
module tb;
  typedef struct { int x; byte y; } st_t;
  typedef struct packed { logic [3:0] hi; logic [3:0] lo; } p_t;
  typedef struct { st_t a; int b; } outer_t;

  st_t s1, s2;
  p_t p;
  outer_t o;

  initial begin
    s1 = '{x: 5, y: 8'h22};          // named keys only
    s2 = '{default: 0, x: 3};        // named key wins over default:, which fills y
    p  = '{hi: 4'hA, lo: 4'h5};      // named keys onto a PACKED struct
    o  = '{a: '{x: 1, y: 2}, b: 3};  // named key whose value is itself a pattern
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
fn named_keys_bind_unpacked_struct_members() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "s1.x"), 5);
    assert_eq!(u(&sim, "s1.y"), 0x22);
}

#[test]
fn named_key_wins_over_default_which_fills_the_rest() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "s2.x"), 3);
    assert_eq!(u(&sim, "s2.y"), 0);
}

#[test]
fn named_keys_place_packed_struct_fields_by_bit_layout() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "p"), 0xA5);
}

#[test]
fn named_key_value_may_itself_be_a_pattern() {
    let sim = simulate(SRC, 100).expect("simulate failed");
    assert_eq!(u(&sim, "o.a.x"), 1);
    assert_eq!(u(&sim, "o.a.y"), 2);
    assert_eq!(u(&sim, "o.b"), 3);
}
