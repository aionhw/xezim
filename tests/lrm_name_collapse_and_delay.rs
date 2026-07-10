//! Two silent-corruption bugs found by the clause-6 / clause-9 LRM audits.
//!
//! 1. Writing a struct member could clobber an UNRELATED module signal.
//!    `p.a = 5` is a member access, but name resolution treated the dotted name
//!    as a hierarchical instance path; when `p.a` was not in the compact signal
//!    table the suffix scan collapsed it to its LAST segment and wrote a
//!    module-scope signal that happened to be called `a`. Silent: `p.a` still
//!    read back 5, while `a` (or `a[0..]`) was destroyed.
//!
//! 2. `#5 -> ev;` dropped its delay. `->` is registered as the constraint
//!    implication infix operator, so the bare delay value was parsed with a
//!    full `parse_expression` and became `#(5 -> ev)`: the trigger was
//!    swallowed and the delay evaluated to 0. `#5; -> ev;`, `#5 ->> ev;` and
//!    `#5 begin -> ev; end` all worked, which is what hid it.

use xezim::simulate;

/// A struct member and a module signal that share a name.
const COLLIDE: &str = r#"
typedef struct { int a; int b; } pair_t;
module tb;
  pair_t p;
  logic [7:0] a;        // same name as p's member
  logic [7:0] arr [4];
  int seen_a, seen_pa, seen_pb;
  int arr0, arr1;
  initial begin
    a = 9;
    arr[0] = 9; arr[1] = 8;
    p.a = 5;
    p.b = 7;
    seen_a  = a;
    seen_pa = p.a;
    seen_pb = p.b;
    arr0 = arr[0];
    arr1 = arr[1];
  end
endmodule
"#;

/// The same collision against an unpacked ARRAY named like the member.
const COLLIDE_ARR: &str = r#"
typedef struct { int a; int b; } pair_t;
module tb;
  pair_t p;
  logic [7:0] a [4];
  int a0, a1, pa;
  initial begin
    a[0] = 9; a[1] = 8;
    p.a = 5;
    a0 = a[0];
    a1 = a[1];
    pa = p.a;
  end
endmodule
"#;

const DELAY_TRIGGER: &str = r#"
module tb;
  event ev;
  int t_trig, t_wake, t_plain, t_nb;
  initial begin
    #5 -> ev;                 // delay must apply to the trigger
    t_trig = $time;
  end
  initial begin
    @(ev);
    t_wake = $time;           // a waiter must actually be woken at t=5
  end
  initial begin
    #3 t_plain = $time;       // a delayed assignment still works
    #4 ->> ev;                // the nonblocking form still works
    t_nb = $time;
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
fn a_struct_member_write_does_not_clobber_a_same_named_signal() {
    let sim = simulate(COLLIDE, 100).expect("simulate failed");
    assert_eq!(u(&sim, "seen_pa"), 5);
    assert_eq!(u(&sim, "seen_pb"), 7);
    assert_eq!(u(&sim, "seen_a"), 9, "p.a = 5 overwrote the module signal `a`");
    assert_eq!(u(&sim, "arr0"), 9);
    assert_eq!(u(&sim, "arr1"), 8);
}

#[test]
fn a_struct_member_write_does_not_clobber_a_same_named_array() {
    let sim = simulate(COLLIDE_ARR, 100).expect("simulate failed");
    assert_eq!(u(&sim, "pa"), 5);
    assert_eq!(u(&sim, "a0"), 9, "p.a = 5 destroyed array `a`");
    assert_eq!(u(&sim, "a1"), 8);
}

#[test]
fn a_delay_applies_to_a_blocking_event_trigger() {
    let sim = simulate(DELAY_TRIGGER, 100).expect("simulate failed");
    assert_eq!(u(&sim, "t_trig"), 5, "#5 -> ev fired at time 0");
    assert_eq!(u(&sim, "t_wake"), 5, "no waiter was woken by the trigger");
    // The forms that already worked must keep working.
    assert_eq!(u(&sim, "t_plain"), 3);
    assert_eq!(u(&sim, "t_nb"), 7);
}
