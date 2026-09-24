//! id-collision-in-heap: the ORIGINAL failure matrix, ported verbatim from
//! the reproducer (44 self-checks, 22 of which failed before the fix). The
//! companion `heap_id_collision_member_access.rs` distills the core gate
//! shapes into ten readable pins; THIS test is the exhaustive net that must
//! stay green so no gate arm can silently rot:
//!
//!   writes — queue element (t3), flat variable (c1), fixed array (c2),
//!   dynamic array (c3), 2-segment chained struct head (c4), associative
//!   array (c7), cross-module hierarchical (c9), interface-instance
//!   hierarchical (f6), NBA queue+flat (f3), foreach loop head (f4),
//!   ref formal (f5), 3-segment chained (f10), compound RMW (f13),
//!   part-selected element field (f14);
//!
//!   reads — flat member with an armed heap object (c6), 2-segment chained
//!   (f8), submodule hierarchical (f9), 3-segment chained (f11),
//!   part-selected field (f14), method-name field, not dispatch (c10);
//!
//!   controls that must keep landing — non-class field names (c5, f1, f2),
//!   method-name writes (f12), chained access through a REAL class handle
//!   without polluting the colliding object (c8), non-colliding variants
//!   (c9/f4 controls), and heap-object hygiene throughout.
//!
//! The design is deterministic (no plusargs/seed needed): every colliding
//! value is built from whole-assignments and member writes issued while the
//! base still holds 0, so only the diverted paths — never the seeding —
//! decide the outcome.

use xezim::simulate;

#[test]
fn id_collision_in_heap_full_matrix() {
    let sim = simulate(include_str!("id_collision_in_heap.sv"), 1_000_000)
        .unwrap_or_else(|e| panic!("simulate failed: {}", e));
    let msgs: Vec<&str> = sim.output.iter().map(|o| o.message.trim()).collect();
    let fails: Vec<&str> = msgs.iter().copied().filter(|m| m.starts_with("FAIL")).collect();
    assert!(
        fails.is_empty(),
        "id-collision-in-heap regressions (diverted struct member writes/reads):\n{}",
        fails.join("\n")
    );
    assert!(
        msgs.iter().any(|m| *m == "TEST_PASS"),
        "never reached TEST_PASS — simulation died early:\n{}",
        msgs.join("\n")
    );
}