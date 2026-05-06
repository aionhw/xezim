//! Regression guards for hot-path struct sizes.
//! These are compile-time packed by the current layout — if a future
//! change adds fields or inlines a larger variant, this test catches
//! the regression before it lands in c910-scale perf numbers.

use xezim::compiler::bytecode::Insn;
use xezim_core::value::Value;

#[test]
fn insn_size_fits_cache_line() {
    let sz = std::mem::size_of::<Insn>();
    eprintln!("size_of Insn = {}", sz);
    // After boxing Concat's Vec and StmtFallback's payload (in
    // addition to the prior LoadConst/LoadArrayElem/NbaAssignArray
    // boxes), the enum sits at 24 B. Going back to 32 B is a 33%
    // bytecode-footprint regression on dense designs — investigate
    // which variant got fat.
    assert!(
        sz <= 24,
        "Insn enum grew to {} B (max-variant needs a Box?)",
        sz
    );
}

#[test]
fn value_size_bounded() {
    let sz = std::mem::size_of::<Value>();
    eprintln!("size_of Value = {}", sz);
    assert!(
        sz <= 32,
        "Value grew to {} B (A1 candidate: strip is_signed/is_real)",
        sz
    );
}
