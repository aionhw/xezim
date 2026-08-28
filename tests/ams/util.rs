//! Shared lookup helpers for the `ams` group.
//!
//! Signals of the top module are reachable by their bare name; nested ones
//! need the dotted path. Every case here would otherwise carry its own copy of
//! that fallback, and a case that guessed wrong failed with a bare "signal not
//! found" that looked like a simulation bug rather than a test bug.

pub fn sig<'a>(sim: &'a xezim::compiler::Simulator, n: &str) -> &'a xezim_core::Value {
    sim.get_signal(n)
        .or_else(|| sim.get_signal(&format!("tb.{}", n)))
        .unwrap_or_else(|| panic!("signal not found: {}", n))
}

/// The f64 value of `n`.
pub fn r(sim: &xezim::compiler::Simulator, n: &str) -> f64 {
    sig(sim, n).to_f64()
}

/// The unsigned integer value of `n`, or `None` if it holds x/z.
pub fn u(sim: &xezim::compiler::Simulator, n: &str) -> Option<u64> {
    sig(sim, n).to_u64()
}
