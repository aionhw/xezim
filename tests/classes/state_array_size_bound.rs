//! IEEE 1800-2017 §18.3, §7.5.2, §7.9.1: a collection reached through a state
//! handle has a fixed size during `randomize()`, so `idx < cfg.ranges.size()`
//! is a constant bound. Read as an opaque call over every rand member, it made
//! the joint solver give up on an otherwise satisfiable set.

use xezim::simulate;

fn passes(src: &str, marker: &str) {
    let sim = simulate(src, 1000).expect("design must run");
    let out = sim.output.iter().map(|o| o.message.as_str()).collect::<Vec<_>>().join("\n");
    assert!(out.contains(marker) && !out.contains("Fatal"), "{out}");
}

#[test]
fn dynamic_array_size_of_a_state_handle_bounds_an_index() {
    passes(include_str!("../fixtures/state_array_size_bound.sv"), "STATE_ARRAY_SIZE_BOUND_PASS");
}

#[test]
fn associative_array_size_of_a_state_handle_bounds_an_index() {
    passes(include_str!("../fixtures/state_assoc_size_bound.sv"), "STATE_ASSOC_SIZE_BOUND_PASS");
}
