//! Serialization for the process-wide Verilog-AMS parser gate.
//!
//! `sv_parser::set_ams` flips one atomic for the whole process, but `cargo
//! test` runs a group's cases on several threads at once. A test that needs
//! AMS keywords reserved and one that needs them lexing as ordinary
//! identifiers would otherwise race, and the loser fails with a parse error
//! that has nothing to do with what it was checking.
//!
//! Every test in this group that cares about the gate goes through
//! [`with_ams`] / [`without_ams`], which hold one mutex for the whole
//! parse. Tests that do not care (plain SystemVerilog RNM) need neither.

use std::sync::{Mutex, MutexGuard, OnceLock};

fn lock() -> MutexGuard<'static, ()> {
    static AMS_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    // A panicking test poisons the mutex; the gate is reset on entry either
    // way, so recovering keeps one failure from cascading into every later
    // AMS test in the group.
    match AMS_LOCK.get_or_init(|| Mutex::new(())).lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Run `f` with Verilog-AMS syntax enabled, then restore the default (off).
pub fn with_ams<T>(f: impl FnOnce() -> T) -> T {
    let _g = lock();
    sv_parser::set_ams(true);
    let out = f();
    sv_parser::set_ams(false);
    out
}

/// Run `f` with Verilog-AMS syntax explicitly DISABLED — the shipping
/// default, and what every non-AMS design in the suite parses under.
pub fn without_ams<T>(f: impl FnOnce() -> T) -> T {
    let _g = lock();
    sv_parser::set_ams(false);
    f()
}
