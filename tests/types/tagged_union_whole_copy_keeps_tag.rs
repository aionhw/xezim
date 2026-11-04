//! IEEE 1800-2017 §11.9: a whole-union assignment (`b = a`) carries the
//! source's ACTIVE tag with the value, so a later `b.Valid` is a read of the
//! active member, not of an inactive one. The inactive-member check (#470)
//! only recorded tags on `tagged` assignments, so a stale tag from an earlier
//! `b = tagged Invalid` made the copy's `b.Valid` a spurious run-time error
//! (and bumped the error count). A non-tagged write that carries no tag (a
//! pattern, a function result) drops the stale tag instead.
//!
//! The VALUE of a whole tagged-union copy is not checked here: on current main
//! a tagged-union variable only receives a value through the `tagged` form
//! (`b = a` and `d = raw` leave `b`/`d` unchanged — a separate, pre-existing
//! bug), so the tag is observed through `matches` and the absence of the
//! §11.9 error.

use xezim::simulate;

fn messages(sim: &xezim::compiler::Simulator) -> Vec<String> {
    sim.output.iter().map(|o| o.message.trim().to_string()).collect()
}

#[test]
fn whole_union_copy_carries_the_active_tag() {
    let sim = simulate(
        r#"
module top;
  typedef union tagged { void Invalid; int Valid; } u_t;
  u_t a, b;
  initial begin
    a = tagged Valid (42);
    b = tagged Invalid;
    b = a;
    if (b matches tagged Valid .v) $display("TAG=Valid");
    else $display("TAG=other");
    $display("B=%0d", b.Valid);
    $finish;
  end
endmodule
"#,
        10,
    )
    .expect("sim");
    let msgs = messages(&sim);
    assert!(msgs.iter().any(|m| m == "TAG=Valid"), "{msgs:?}");
    assert!(
        !msgs.iter().any(|m| m.contains("inactive tagged union member")),
        "spurious §11.9 error after a whole-union copy: {msgs:?}"
    );
}
