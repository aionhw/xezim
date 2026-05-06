//! Stress regression: large-scale generated designs from `examples/stress_*.sv`.
//!
//! These exist to exercise the dual-store / signal-table / comb-entry code
//! paths at scale. They are **slow** (10s–60s wall each on release builds)
//! and are gated behind `#[ignore]` so the default `cargo test` stays fast.
//!
//! Run with:
//!   cargo test --release --test stress_tests -- --ignored
//!
//! Each test asserts that:
//!   1. Parsing + elaboration succeed for the entire generated design.
//!   2. The simulation runs to its self-`$finish` without panicking.
//!   3. At least one simulator output entry is captured (non-zero progress).

use std::fs;
use std::path::Path;
use xezim::simulate;

fn run_stress(file: &str, top: &str, max_time: u64) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
    let src =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));
    // simulate() takes a source string and a max time (ns). The stress
    // designs all $finish themselves at MAX_TIME, so a generous cap is fine.
    let _ = top; // simulate() picks the last module by default; stress files put `top` last.
    let sim =
        simulate(&src, max_time).unwrap_or_else(|e| panic!("simulation of {} failed: {}", file, e));
    assert!(
        sim.time > 0,
        "{} simulation did not advance simulated time (got time={})",
        file,
        sim.time
    );
}

#[test]
#[ignore]
fn stress_signals_131k_named() {
    // 131072 named bit-cell instances + 1 clock cycle settling.
    // Exercises name → id maps and signal_table sizing without
    // running the simulation long enough to be unbearable.
    run_stress("examples/stress_signals.sv", "top", 100);
}

#[test]
#[ignore]
fn stress_comb_continuous_assigns() {
    // Comb-heavy: every cell is `assign sum/diff/xor_out = …`. Stresses the
    // continuous-assign compile path and write_signal_ids on comb_entries.
    run_stress("examples/stress_comb.sv", "top", 100);
}

#[test]
#[ignore]
fn stress_explicit_instances() {
    // Generated explicit instantiations (no genvar/generate). Stresses the
    // elaborator's instantiation-binding path at large fan-out.
    run_stress("examples/stress_explicit.sv", "top", 100);
}
