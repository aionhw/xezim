//! Cranelift-backed JIT for bytecode blocks.
//!
//! Feature-gated behind `jit`. When enabled, xezim attempts to compile
//! each `CompiledBlock`'s `Insn[]` to native code at elaboration time.
//! At VM-dispatch time, `exec_bytecode` calls the JIT'd function if
//! present; otherwise falls back to the interpreter. Blocks containing
//! any unsupported Insn are left un-JIT'd (the compiler returns None).
//!
//! # Design
//!
//! ## Register / signal model
//!
//! The interpreter stores VM registers as `Vec<Value>` — a struct with
//! an enum `storage` field that the JIT can't cheaply manipulate. To
//! bridge this:
//!
//!   - VM registers → Cranelift SSA values: each `RegId` in an Insn
//!     stream maps to a stack slot holding a `u64` val_bits. On
//!     function entry we initialize all slots to the current
//!     `vm_regs[i].to_u64()`; on exit we write back via `from_u64`
//!     through a bridge call. Inside the function body, arithmetic
//!     operates on SSA values loaded from these stack slots.
//!
//!   - Signal reads / writes: FFI bridge calls into Rust code that
//!     handles all the Value-struct plumbing (dirty bits, widths,
//!     is_signed). The JIT pays ~10-20ns of FFI overhead per call
//!     but saves the ~40-50ns of interpreter dispatch + Value
//!     marshalling on every arithmetic op between loads/stores.
//!
//! ## Bridge functions (exposed to JIT code via function-pointer imports)
//!
//!   - `bridge_load_signal(sim, id) -> u64` — reads `signal_table[id]`
//!     and returns the `val_bits`. Sets a sticky "has-xz" error flag
//!     on the Simulator if the loaded value is 4-state; JIT code
//!     checks this at exit and returns failure → interpreter re-runs.
//!   - `bridge_store_signal(sim, id, val_bits, width)` — mirrors
//!     `Insn::BlockingAssign`: compare-then-write, mark dirty,
//!     propagate. Pure Rust; no codegen for dirty bookkeeping.
//!   - `bridge_schedule_nba(sim, id, val_bits, width)` — mirrors
//!     `Insn::NbaAssign`: pushes an `NbaFast` entry.
//!   - `bridge_fallback(sim)` — called from JIT emit for any Insn the
//!     codegen decides is "too hot to inline now" (e.g. Div which
//!     wants X/Z handling); marks the function as "don't re-JIT"
//!     and returns to interpreter.
//!
//! ## Supported Insn variants (phase plan)
//!
//! Phase 1 (MVP, target first): LoadConst, LoadSignal, BlockingAssign,
//!   Add, Sub, BitAnd, BitOr, BitXor, BitNot, Move.
//! Phase 2: Eq, Neq, Lt, Leq, Gt, Geq (comparisons).
//! Phase 3: Shl, Shr, AShr, reductions.
//! Phase 4: BranchIfFalse / Jump (control flow).
//! Phase 5: NbaAssign*, BlockingAssignRange*, LoadArrayElem.
//!
//! Any block touching an unsupported Insn returns None from
//! `try_compile` → interpreter runs the whole block.
//!
//! ## Expected wins on c910
//!
//! 1.1B native insns at interpreter 117ns/op → 129s total. If JIT
//! averages 20-30ns/op across the supported subset (pure register
//! arithmetic) and 80-100ns/op on bridge calls (loads/stores), we'd
//! expect 40-60% of edge_exec eliminated = 50-80s savings.

#![allow(dead_code)]
#![allow(unused_imports)]

#[cfg(feature = "jit")]
pub use enabled::*;
#[cfg(not(feature = "jit"))]
pub use stub::*;

/// The JIT'd function signature: takes a pointer to the `Simulator`
/// (opaque to generated code) and runs the compiled block. Returns
/// 0 on success, non-zero to request interpreter fallback for this
/// block going forward (e.g. if a runtime check found a Wide value
/// where only Inline was expected).
pub type JitFn = unsafe extern "C" fn(sim: *mut u8) -> u32;

/// Stubs when the feature is disabled — everything is None / no-op so
/// `exec_bytecode` always falls through to the interpreter.
#[cfg(not(feature = "jit"))]
mod stub {
    use super::super::bytecode::Insn;
    use super::JitFn;

    pub struct JitModule;
    impl JitModule {
        pub fn new() -> Option<Self> { None }
        pub fn try_compile(&mut self, _insns: &[Insn], _num_regs: u32) -> Option<JitFn> { None }
    }
}

#[cfg(feature = "jit")]
mod enabled {
    use super::super::bytecode::Insn;
    use super::JitFn;
    use cranelift::prelude::*;
    use cranelift_jit::{JITBuilder, JITModule as ClJitModule};
    use cranelift_module::{Linkage, Module};

    pub struct JitModule {
        module: ClJitModule,
    }

    impl JitModule {
        pub fn new() -> Option<Self> {
            let isa_builder = cranelift_native::builder().ok()?;
            let flag_builder = settings::builder();
            let isa = isa_builder
                .finish(settings::Flags::new(flag_builder))
                .ok()?;
            let builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
            Some(Self { module: ClJitModule::new(builder) })
        }

        /// Try to JIT-compile a block's instruction list. Returns None if
        /// any Insn is not yet supported; callers fall back to the
        /// interpreter in that case.
        pub fn try_compile(&mut self, insns: &[Insn], _num_regs: u32) -> Option<JitFn> {
            // First pass: reject blocks that touch any Insn we don't
            // yet support. This lets us land codegen one variant at
            // a time while shipping an always-correct fallback path.
            for insn in insns {
                if !is_supported(insn) {
                    return None;
                }
            }
            // Phase-1 MVP codegen is not yet wired; returning None until
            // the codegen pass (emit_insn per variant) is implemented.
            // See `codegen_block` (TODO) and the phase plan in this
            // module's doc comment.
            None
        }
    }

    /// Set of Insns the current phase is willing to JIT. Anything else
    /// forces the whole block to the interpreter (safer than partial
    /// JIT + mid-block bail, which would complicate register state).
    fn is_supported(insn: &Insn) -> bool {
        use Insn::*;
        matches!(insn,
            LoadConst(..)
            | LoadSignal(..)
            | LoadSignalSigned(..)
            | Move(..)
            | BlockingAssign(..)
            | Add(..) | Sub(..)
            | BitAnd(..) | BitOr(..) | BitXor(..) | BitNot(..)
            | Nop
        )
    }
}
