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
    // Post-B1+B2 with LoadConst/LoadArrayElem/NbaAssignArray boxed, the
    // enum sits at 32 B. Going back to 40 B is a hot-path footprint
    // regression — investigate which variant got fat.
    assert!(sz <= 32, "Insn enum grew to {} B (max-variant needs a Box?)", sz);
}

#[test]
fn value_size_bounded() {
    let sz = std::mem::size_of::<Value>();
    eprintln!("size_of Value = {}", sz);
    assert!(sz <= 32, "Value grew to {} B (A1 candidate: strip is_signed/is_real)", sz);
}
