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
//!   - VM registers → Cranelift stack slots: each `RegId` in an Insn
//!     stream maps to a function-local 8-byte stack slot holding a
//!     `u64` val_bits. On function entry all slots start uninitialized
//!     (zeroed); VM bytecode is SSA-ish — every Insn writes its
//!     destination before later Insns read it, so no cross-block reg
//!     state is needed.
//!
//!   - Signal reads / writes: FFI bridge calls into Rust code that
//!     handles all the Value-struct plumbing (dirty bits, widths,
//!     is_signed). The JIT pays ~10-20ns of FFI overhead per call
//!     but saves the ~40-50ns of interpreter dispatch + Value
//!     marshalling on every arithmetic op between loads/stores.
//!
//! ## Supported Insn variants (phase plan)
//!
//! Phase 1 (MVP, implemented here): LoadConst, LoadSignal, Move,
//!   BlockingAssign, Add, Sub, BitAnd, BitOr, BitXor, BitNot, Nop.
//! Phase 2: Eq, Neq, Lt, Leq, Gt, Geq (comparisons).
//! Phase 3: Shl, Shr, AShr, reductions.
//! Phase 4: BranchIfFalse / Jump (control flow).
//! Phase 5: NbaAssign*, BlockingAssignRange*, LoadArrayElem.
//!
//! Any block touching an unsupported Insn returns None from
//! `try_compile` → interpreter runs the whole block.

#![allow(dead_code)]
#![allow(unused_imports)]

use super::bytecode::Insn;

#[cfg(feature = "jit")]
pub use enabled::*;
#[cfg(not(feature = "jit"))]
pub use stub::*;

/// The JIT'd function signature: takes a pointer to the `Simulator`
/// (opaque to generated code) and runs the compiled block. Returns
/// 0 on success, non-zero to request interpreter re-run for this
/// block (e.g. if a runtime check found a Wide value).
pub type JitFn = unsafe extern "C" fn(sim: *mut u8) -> u32;

// ---------------------------------------------------------------------
// Bridge functions — exposed to JIT code as `extern "C"` imports.
//
// These are the only way the JIT interacts with `Simulator` state.
// They look up signals, apply writes (with dirty tracking), and fall
// back cleanly on X/Z or Wide values.
// ---------------------------------------------------------------------

/// Read `signal_table[id]` as a u64. If the Value is 4-state (has
/// X/Z bits set) or Wide (> 64 bits), sets the Simulator's
/// `jit_fallback_flag` so the caller knows to re-run via the
/// interpreter. Returns the best-effort `val_bits` anyway so the JIT
/// can keep executing without branching per load.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xezim_jit_load_signal(sim: *mut u8, id: u32) -> u64 { unsafe {
    let sim = &mut *(sim as *mut crate::compiler::simulator::Simulator);
    sim.jit_load_signal(id as usize)
}}

/// Write `signal_table[id] = val_bits` (width-masked) with full
/// dirty-tracking and mark_dirty_id behavior — i.e. matches
/// `Insn::BlockingAssign` semantics. Returns nothing; caller trusts
/// the bridge to propagate correctly.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xezim_jit_store_signal(sim: *mut u8, id: u32, val_bits: u64, width: u32) { unsafe {
    let sim = &mut *(sim as *mut crate::compiler::simulator::Simulator);
    sim.jit_store_signal(id as usize, val_bits, width);
}}

/// 4-STATE load: the X/Z plane of `signal_table[id]`.
///
/// The JIT used to be 2-state only and bailed whenever any input carried X/Z.
/// On real 4-state RTL that bail fired on the overwhelming majority of
/// executions (96.6% of eligible comb evaluations on the C906 SoC), so the
/// block paid a call plus a pre-check and then ran interpreted anyway.
/// Carrying a second plane per register removes the bail entirely.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xezim_jit_load_signal_xz(sim: *mut u8, id: u32) -> u64 { unsafe {
    let sim = &mut *(sim as *mut crate::compiler::simulator::Simulator);
    sim.jit_load_signal_xz(id as usize)
}}

/// AST-statement escape hatch: run ONE fallback statement (a $display /
/// $error / other non-compilable stmt) through the interpreter from JIT'd
/// code, with the owning block's scope hint applied around it exactly like
/// the interpreter's fallback path (timescale for %t, %m instance naming,
/// bare-name resolution). `stmt` is the Arc payload baked into the block's
/// StmtFallback insn — the CompiledBlock outlives the JIT fn, keeping it
/// alive. `hint` is a leaked per-block scope string (may be null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xezim_jit_stmt_fallback(
    sim: *mut u8,
    stmt: *const crate::ast::stmt::Statement,
    hint_ptr: *const u8,
    hint_len: usize,
) { unsafe {
    let sim = &mut *(sim as *mut crate::compiler::simulator::Simulator);
    let hint = if hint_ptr.is_null() || hint_len == 0 {
        None
    } else {
        std::str::from_utf8(std::slice::from_raw_parts(hint_ptr, hint_len)).ok()
    };
    sim.jit_exec_fallback_stmt(&*stmt, hint);
}}

/// Fused two-plane load: writes val/xz planes of `signal_table[id]`
/// through out-pointers (the JIT passes its register stack-slot
/// addresses), replacing two bridge calls per signal read with one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xezim_jit_load_signal2(
    sim: *mut u8,
    id: u32,
    out_v: *mut u64,
    out_x: *mut u64,
) { unsafe {
    let sim = &mut *(sim as *mut crate::compiler::simulator::Simulator);
    let (v, x) = sim.jit_load_signal2(id as usize);
    *out_v = v;
    *out_x = x;
}}

/// Slice load: value plane of `signal_table[id][lo +: w]` (any signal width,
/// w <= 64). Wide-signal escape hatch for `LoadSignalRange`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xezim_jit_load_signal_slice(
    sim: *mut u8,
    id: u32,
    lo: u32,
    w: u32,
) -> u64 { unsafe {
    let sim = &mut *(sim as *mut crate::compiler::simulator::Simulator);
    sim.jit_load_signal_slice(id as usize, lo, w)
}}

/// Slice load: xz plane sibling of `xezim_jit_load_signal_slice`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xezim_jit_load_signal_slice_xz(
    sim: *mut u8,
    id: u32,
    lo: u32,
    w: u32,
) -> u64 { unsafe {
    let sim = &mut *(sim as *mut crate::compiler::simulator::Simulator);
    sim.jit_load_signal_slice_xz(id as usize, lo, w)
}}

/// 4-STATE store: write both planes, with the same dirty-tracking and
/// `mark_dirty_id` behaviour as `Insn::BlockingAssign`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xezim_jit_store_signal_4s(
    sim: *mut u8,
    id: u32,
    val_bits: u64,
    xz_bits: u64,
    width: u32,
) { unsafe {
    let sim = &mut *(sim as *mut crate::compiler::simulator::Simulator);
    sim.jit_store_signal_4s(id as usize, val_bits, xz_bits, width);
}}

/// 4-STATE non-blocking schedule (see `jit_schedule_nba_4s`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xezim_jit_schedule_nba_4s(
    sim: *mut u8,
    id: u32,
    val_bits: u64,
    xz_bits: u64,
    width: u32,
) { unsafe {
    let sim = &mut *(sim as *mut crate::compiler::simulator::Simulator);
    sim.jit_schedule_nba_4s(id as usize, val_bits, xz_bits, width);
}}

/// Schedule a non-blocking assign: push `(signal_id, value)` to
/// `nba_fast` so the next `apply_nba` pass writes `signal_table[id]`.
/// Mirrors `Insn::NbaAssign` semantics.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xezim_jit_schedule_nba(sim: *mut u8, id: u32, val_bits: u64, width: u32) { unsafe {
    let sim = &mut *(sim as *mut crate::compiler::simulator::Simulator);
    sim.jit_schedule_nba(id as usize, val_bits, width);
}}

/// JIT Stage 4 Tier A — leaner NBA schedule variant.  Caller (JIT
/// codegen) emits a call to this only when:
///   - `id` is in range (validated at JIT-compile time)
///   - `val_bits` is already masked to `signal_widths[id]`
///     (the Insn::NbaAssign width arg matched signal_widths[id])
///
/// Skips: bounds check, width compare, Value::resize.  Still uses
/// the same nba_fast / nba_fast_index path so partial-bit NBAs to
/// the same signal merge correctly.  Net per-call saving: ~3-5 ns
/// (the bulk of NBA cost — HashMap insert + Vec push — remains in
/// Tier B/C territory).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xezim_jit_schedule_nba_fast(
    sim: *mut u8,
    id: u32,
    val_bits: u64,
) { unsafe {
    let sim = &mut *(sim as *mut crate::compiler::simulator::Simulator);
    sim.jit_schedule_nba_fast(id as usize, val_bits);
}}

/// Schedule a non-blocking assign to a dynamic bit-range: merges `val_bits`
/// at bits `[hi_bits:lo_bits]` into the current signal value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xezim_jit_schedule_nba_range_dyn(
    sim: *mut u8,
    id: u32,
    hi_bits: u64,
    lo_bits: u64,
    val_bits: u64,
    xz_bits: u64,
) { unsafe {
    let sim = &mut *(sim as *mut crate::compiler::simulator::Simulator);
    sim.jit_schedule_nba_range(id as usize, hi_bits as u32, lo_bits as u32, val_bits, xz_bits);
}}

/// Schedule a non-blocking assign to a dynamic bit-index.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xezim_jit_schedule_nba_bit_dyn(
    sim: *mut u8,
    id: u32,
    idx: u64,
    val_bits: u64,
    xz_bits: u64,
) { unsafe {
    let sim = &mut *(sim as *mut crate::compiler::simulator::Simulator);
    sim.jit_schedule_nba_bit(id as usize, idx as usize, val_bits, xz_bits);
}}

/// Perform a blocking assign to a dynamic bit-range.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xezim_jit_blocking_assign_range_dyn(
    sim: *mut u8,
    id: u32,
    hi_bits: u64,
    lo_bits: u64,
    val_bits: u64,
    xz_bits: u64,
) { unsafe {
    let sim = &mut *(sim as *mut crate::compiler::simulator::Simulator);
    sim.jit_blocking_assign_range(id as usize, hi_bits as u32, lo_bits as u32, val_bits, xz_bits);
}}

/// Load an array element value as u64.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xezim_jit_load_array_elem(sim: *mut u8, name_ptr: *const u8, idx: i64) -> u64 { unsafe {
    let sim = &mut *(sim as *mut crate::compiler::simulator::Simulator);
    let name = std::ffi::CStr::from_ptr(name_ptr as *const std::ffi::c_char).to_string_lossy();
    sim.jit_load_array_elem(&name, idx)
}}

/// Path B X/Z runtime pre-check. Reads the slice of `n` u32 sig_ids
/// pointed at by `ids_ptr` and returns 1 if ANY of those signals
/// currently have non-zero `xz_bits` (i.e. X/Z), else 0. Called from
/// the JIT prelude before any side-effecting Insn executes; the JIT
/// emits an `if (rc != 0) return 1` to bail out cleanly so the
/// interpreter can run the block safely. Keeps the JIT compatible
/// with NbaAssignRange et al. (which were previously OFF because
/// 2-state codegen mishandled X/Z).
///
/// SAFETY: caller must ensure `ids_ptr` points to `n` valid u32s for
/// the duration of the call. The Cranelift codegen materialises a
/// data symbol holding the per-block input list and passes it in,
/// satisfying this contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xezim_jit_inputs_have_xz(
    sim: *mut u8,
    ids_ptr: *const u32,
    n: u32,
) -> u32 { unsafe {
    let sim = &*(sim as *const crate::compiler::simulator::Simulator);
    let ids = std::slice::from_raw_parts(ids_ptr, n as usize);
    if sim.jit_inputs_have_xz(ids) {
        1
    } else {
        0
    }
}}

/// Stubs when the feature is disabled — everything is None / no-op so
/// `exec_bytecode` always falls through to the interpreter.
#[cfg(not(feature = "jit"))]
mod stub {
    use super::super::bytecode::Insn;
    use super::JitFn;

    pub fn first_unsupported(_insns: &[Insn]) -> Option<&'static str> {
        None
    }

    pub struct JitModule;
    impl JitModule {
        pub fn new() -> Option<Self> {
            None
        }
        pub fn try_compile(&mut self, _insns: &[Insn], _num_regs: u32) -> Option<JitFn> {
            None
        }
        pub fn try_compile_with_xz(
            &mut self,
            _insns: &[Insn],
            _num_regs: u32,
            _xz_ptr: u64,
            _xz_len: u32,
        ) -> Option<JitFn> {
            None
        }
        pub fn try_compile_with_xz_hint(
            &mut self,
            _insns: &[Insn],
            _num_regs: u32,
            _xz_ptr: u64,
            _xz_len: u32,
            _scope_hint: Option<&str>,
        ) -> Option<JitFn> {
            None
        }
        pub fn set_inline_bits_storage(&mut self, _ptr: u64, _len: u32) {}
        pub fn set_signal_widths(&mut self, _widths: Vec<u32>) {}
        pub fn set_signal_signed(&mut self, _signed: Vec<bool>) {}
        pub fn set_nba_side_queue(&mut self, _base_ptr: u64, _len_ptr: u64, _cap: u32) {}
    }
}

#[cfg(feature = "jit")]
mod enabled {
    use super::super::bytecode::Insn;
    use super::{
        xezim_jit_blocking_assign_range_dyn, xezim_jit_inputs_have_xz,
        xezim_jit_load_array_elem, xezim_jit_load_signal, xezim_jit_load_signal2,
        xezim_jit_stmt_fallback,
        xezim_jit_load_signal_slice,
        xezim_jit_load_signal_slice_xz, xezim_jit_load_signal_xz,
        xezim_jit_schedule_nba,
        xezim_jit_schedule_nba_bit_dyn, xezim_jit_schedule_nba_fast,
        xezim_jit_schedule_nba_4s, xezim_jit_schedule_nba_range_dyn, xezim_jit_store_signal,
        xezim_jit_store_signal_4s, JitFn,
    };
    use cranelift::codegen::ir::{BlockArg, BlockCall, FuncRef, JumpTableData, MemFlagsData, StackSlot};

    /// The two leaner NBA emission paths below move only the VALUE plane, so
    /// they cannot be used now that registers are 4-state. Left in place (and
    /// switched off here) because they are worth restoring once the side-queue
    /// entry grows an X/Z word.
    const FOUR_STATE_NBA_FAST_OK: bool = false;
    use cranelift::prelude::*;
    use cranelift_jit::{JITBuilder, JITModule as ClJitModule};
    use cranelift_module::{FuncId, Linkage, Module};

    pub struct JitModule {
        module: ClJitModule,
        next_id: u64,
        /// JIT Stage 2 (JIT-REDESIGN-NOTES.md): when set, codegen for
        /// LoadSignal / LoadSignalSigned reads `signal_inline_bits[id*16]`
        /// directly via a baked-in absolute pointer + offset instead of
        /// calling `xezim_jit_load_signal` across FFI.  Eliminates ~10-20 ns
        /// per signal load.  Set via `set_inline_bits_storage` after the
        /// JitModule is constructed, by the simulator caller that knows
        /// the pointer and length.
        inline_bits_ptr: Option<(u64, u32)>,
        /// Scope hint (leaked ptr, len) for the block currently being
        /// compiled; baked into StmtFallback bridge calls.
        block_scope_hint: Option<(u64, u64)>,
        /// JIT Stage 4 Tier A: snapshot of `Simulator::signal_widths` used
        /// at JIT-compile time to pick between the slow nba bridge (with
        /// width arg) and the leaner nba_fast bridge (assumes
        /// width == signal_widths[sig_id]).  Set via
        /// `set_signal_widths` after JitModule construction.
        signal_widths_snapshot: Vec<u32>,
        /// Snapshot of `Simulator::signal_signed` — drives per-register
        /// signedness for sign-extension on Resize/AShr and signed compares.
        signal_signed_snapshot: Vec<bool>,
        /// JIT Stage 4 Tier C Path C1: (base_ptr, len_ptr, capacity) of
        /// the simulator's `jit_nba_side_queue`.  When set, NbaAssign
        /// codegen emits an inline write to this queue (no FFI),
        /// bumping the length counter at len_ptr.  drain_jit_nba_side_queue
        /// transfers entries into nba_fast at apply_nba time.
        nba_side_queue: Option<(u64, u64, u32)>,
    }

    impl JitModule {
        pub fn new() -> Option<Self> {
            let isa_builder = cranelift_native::builder().ok()?;
            let mut flag_builder = settings::builder();
            // Cranelift defaults to opt_level=none (no regalloc opt / DCE /
            // stack-slot promotion), which produced JIT'd code SLOWER than the
            // tight interpreter (measured 79.5 vs 59.7 ns/insn). Enable the
            // optimizing pipeline so VM-reg stack slots get promoted to
            // registers and dead code is removed.
            let _ = flag_builder.set("opt_level", "speed");
            let isa = isa_builder
                .finish(settings::Flags::new(flag_builder))
                .ok()?;
            let mut builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
            // Register bridge function symbols so the JIT can link to them.
            builder.symbol("xezim_jit_load_signal", xezim_jit_load_signal as *const u8);
            builder.symbol("xezim_jit_load_signal2", xezim_jit_load_signal2 as *const u8);
            builder.symbol(
                "xezim_jit_stmt_fallback",
                xezim_jit_stmt_fallback as *const u8,
            );
            builder.symbol(
                "xezim_jit_load_signal_slice",
                xezim_jit_load_signal_slice as *const u8,
            );
            builder.symbol(
                "xezim_jit_load_signal_slice_xz",
                xezim_jit_load_signal_slice_xz as *const u8,
            );
            builder.symbol(
                "xezim_jit_load_signal_xz",
                xezim_jit_load_signal_xz as *const u8,
            );
            builder.symbol(
                "xezim_jit_store_signal_4s",
                xezim_jit_store_signal_4s as *const u8,
            );
            builder.symbol(
                "xezim_jit_schedule_nba_4s",
                xezim_jit_schedule_nba_4s as *const u8,
            );
            builder.symbol(
                "xezim_jit_store_signal",
                xezim_jit_store_signal as *const u8,
            );
            builder.symbol(
                "xezim_jit_schedule_nba",
                xezim_jit_schedule_nba as *const u8,
            );
            builder.symbol(
                "xezim_jit_schedule_nba_fast",
                xezim_jit_schedule_nba_fast as *const u8,
            );
            builder.symbol(
                "xezim_jit_schedule_nba_range",
                xezim_jit_schedule_nba_range_dyn as *const u8,
            );
            builder.symbol(
                "xezim_jit_schedule_nba_range_dyn",
                xezim_jit_schedule_nba_range_dyn as *const u8,
            );
            builder.symbol(
                "xezim_jit_schedule_nba_bit_dyn",
                xezim_jit_schedule_nba_bit_dyn as *const u8,
            );
            builder.symbol(
                "xezim_jit_blocking_assign_range_dyn",
                xezim_jit_blocking_assign_range_dyn as *const u8,
            );
            builder.symbol(
                "xezim_jit_load_array_elem",
                xezim_jit_load_array_elem as *const u8,
            );
            builder.symbol(
                "xezim_jit_inputs_have_xz",
                xezim_jit_inputs_have_xz as *const u8,
            );
            Some(Self {
                module: ClJitModule::new(builder),
                next_id: 0,
                inline_bits_ptr: None,
            block_scope_hint: None,
                signal_widths_snapshot: Vec::new(),
                signal_signed_snapshot: Vec::new(),
                nba_side_queue: None,
            })
        }

        /// Stage 2: enable inline LoadSignal codegen by providing the
        /// base pointer + element count of `Simulator::signal_inline_bits`.
        /// Must remain valid for the JitModule's lifetime.  Call BEFORE
        /// any `try_compile_*` invocation; existing JIT'd functions
        /// won't see the change.
        pub fn set_inline_bits_storage(&mut self, ptr: u64, len: u32) {
            self.inline_bits_ptr = Some((ptr, len));
        }

        /// Stage 4 Tier A: enable the leaner NBA bridge by providing a
        /// snapshot of signal widths.  JIT codegen uses this to detect
        /// when `Insn::NbaAssign`'s width arg matches signal_widths[id]
        /// — that's the common case where the slow bridge's width
        /// resize is redundant.  Call after JitModule construction.
        pub fn set_signal_widths(&mut self, widths: Vec<u32>) {
            self.signal_widths_snapshot = widths;
        }

        pub fn set_signal_signed(&mut self, signed: Vec<bool>) {
            self.signal_signed_snapshot = signed;
        }

        /// Stage 4 Tier C Path C1: enable inline NBA queue writes by
        /// providing the base pointer of `jit_nba_side_queue` + the
        /// pointer to its length counter.  When set + the width-
        /// matches condition holds, NbaAssign codegen emits a direct
        /// memory write to the side queue (no FFI).
        ///
        /// The pointers MUST remain valid for the JitModule's
        /// lifetime — the side queue is pre-allocated to fixed
        /// capacity and never resized.
        pub fn set_nba_side_queue(&mut self, base_ptr: u64, len_ptr: u64, cap: u32) {
            self.nba_side_queue = Some((base_ptr, len_ptr, cap));
        }

        /// Try to JIT-compile a block's instruction list. Returns None if
        /// any Insn is not yet supported; callers fall back to the
        /// interpreter in that case.
        pub fn try_compile(&mut self, insns: &[Insn], num_regs: u32) -> Option<JitFn> {
            for insn in insns {
                if !is_supported(insn) {
                    return None;
                }
            }
            // Collect the set of signal IDs this block reads via LoadSignal*.
            // The Path B X/Z prelude pre-checks these before letting the
            // (2-state) JIT body run. Sites the JIT writes to (Insn::*Assign*
            // sig_ids) don't need pre-checking — we only care about *inputs*
            // that could feed wrong-determinate values into arithmetic.
            let mut input_ids: Vec<u32> = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for insn in insns {
                let id_opt = match insn {
                    Insn::LoadSignal(_, sid) | Insn::LoadSignalSigned(_, sid) => Some(*sid as u32),
                    Insn::BranchIfSignalFalse(sid, _, _) => Some(*sid as u32),
                    _ => None,
                };
                if let Some(sid) = id_opt {
                    if seen.insert(sid) {
                        input_ids.push(sid);
                    }
                }
            }
            self.codegen_block(insns, num_regs, &input_ids, 0, 0).ok()
        }

        /// Like `try_compile`, but bakes the absolute address of the
        /// simulator's `signal_has_xz` byte array into the prelude so
        /// the X/Z input check is an inline load+OR instead of a C-call
        /// to `xezim_jit_inputs_have_xz`. `xz_ptr` must remain valid
        /// for the simulator's lifetime (true for `Vec<u8>` that's
        /// pre-sized once and never resized).
        pub fn try_compile_with_xz(
            &mut self,
            insns: &[Insn],
            num_regs: u32,
            xz_ptr: u64,
            xz_len: u32,
        ) -> Option<JitFn> {
            self.try_compile_with_xz_hint(insns, num_regs, xz_ptr, xz_len, None)
        }

        pub fn try_compile_with_xz_hint(
            &mut self,
            insns: &[Insn],
            num_regs: u32,
            xz_ptr: u64,
            xz_len: u32,
            scope_hint: Option<&str>,
        ) -> Option<JitFn> {
            for insn in insns {
                if !is_supported(insn) {
                    return None;
                }
            }
            let mut input_ids: Vec<u32> = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for insn in insns {
                let id_opt = match insn {
                    Insn::LoadSignal(_, sid) | Insn::LoadSignalSigned(_, sid) => Some(*sid as u32),
                    Insn::BranchIfSignalFalse(sid, _, _) => Some(*sid as u32),
                    _ => None,
                };
                if let Some(sid) = id_opt {
                    if seen.insert(sid) {
                        input_ids.push(sid);
                    }
                }
            }
            // Fallback stmts carry the block's scope hint into the bridge;
            // leak one copy per block (block count is static and small).
            self.block_scope_hint = scope_hint.map(|h| {
                let leaked: &'static str = Box::leak(h.to_string().into_boxed_str());
                (leaked.as_ptr() as u64, leaked.len() as u64)
            });
            self.codegen_block(insns, num_regs, &input_ids, xz_ptr, xz_len).ok()
        }

        fn codegen_block(
            &mut self,
            insns: &[Insn],
            num_regs: u32,
            input_ids: &[u32],
            xz_ptr: u64,
            xz_len: u32,
        ) -> Result<JitFn, ()> {
            let pointer_type = self.module.target_config().pointer_type();

            // Declare bridge signatures (shared across all compiled blocks).
            let mut load_sig = self.module.make_signature();
            load_sig.params.push(AbiParam::new(pointer_type)); // sim
            load_sig.params.push(AbiParam::new(types::I32)); // id
            load_sig.returns.push(AbiParam::new(types::I64)); // val_bits

            let mut store_sig = self.module.make_signature();
            store_sig.params.push(AbiParam::new(pointer_type)); // sim
            store_sig.params.push(AbiParam::new(types::I32)); // id
            store_sig.params.push(AbiParam::new(types::I64)); // val_bits
            store_sig.params.push(AbiParam::new(types::I32)); // width

            let mut store_4s_sig = self.module.make_signature();
            store_4s_sig.params.push(AbiParam::new(pointer_type)); // sim
            store_4s_sig.params.push(AbiParam::new(types::I32)); // id
            store_4s_sig.params.push(AbiParam::new(types::I64)); // val_bits
            store_4s_sig.params.push(AbiParam::new(types::I64)); // xz_bits
            store_4s_sig.params.push(AbiParam::new(types::I32)); // width

            let nba_sig = store_sig.clone();

            // Tier A leaner NBA bridge: (sim, id: u32, val_bits: u64).
            // Skips the width parameter — caller (JIT codegen) only
            // emits the call when width matches signal_widths[id] at
            // compile time, so no resize is needed.
            let mut nba_fast_sig = self.module.make_signature();
            nba_fast_sig.params.push(AbiParam::new(pointer_type)); // sim
            nba_fast_sig.params.push(AbiParam::new(types::I32)); // id
            nba_fast_sig.params.push(AbiParam::new(types::I64)); // val_bits

            // nba_bit_dyn bridge ABI: (sim, id: u32, idx: u64, val_bits: u64).
            // The dynamic bit/range schedulers take the bit-pos as u64
            // (matches Rust fn sig). Earlier we re-used `nba_sig` here,
            // which had the 4th arg typed i32, causing a cranelift verifier
            // failure on every block containing NbaAssignBitDyn.
            let mut nba_bit_sig = self.module.make_signature();
            nba_bit_sig.params.push(AbiParam::new(pointer_type));
            nba_bit_sig.params.push(AbiParam::new(types::I32));
            nba_bit_sig.params.push(AbiParam::new(types::I64));
            nba_bit_sig.params.push(AbiParam::new(types::I64));
            nba_bit_sig.params.push(AbiParam::new(types::I64)); // xz_bits (4-state)

            // nba_range / blk_range bridge ABI:
            // (sim, id: u32, hi: u64, lo: u64, val_bits: u64, xz_bits: u64).
            let mut nba_range_sig = self.module.make_signature();
            nba_range_sig.params.push(AbiParam::new(pointer_type));
            nba_range_sig.params.push(AbiParam::new(types::I32));
            nba_range_sig.params.push(AbiParam::new(types::I64));
            nba_range_sig.params.push(AbiParam::new(types::I64));
            nba_range_sig.params.push(AbiParam::new(types::I64));
            nba_range_sig.params.push(AbiParam::new(types::I64)); // xz_bits (4-state)

            // Path B: xz_check (sim, ids_ptr, n_ids) -> u32 (1 if any X/Z, else 0).
            let mut xz_check_sig = self.module.make_signature();
            xz_check_sig.params.push(AbiParam::new(pointer_type));
            xz_check_sig.params.push(AbiParam::new(pointer_type));
            xz_check_sig.params.push(AbiParam::new(types::I32));
            xz_check_sig.returns.push(AbiParam::new(types::I32));

            let load_id: FuncId = self
                .module
                .declare_function("xezim_jit_load_signal", Linkage::Import, &load_sig)
                .map_err(|_| ())?;
            let mut fb_sig = self.module.make_signature();
            fb_sig.params.push(AbiParam::new(pointer_type)); // sim
            fb_sig.params.push(AbiParam::new(pointer_type)); // stmt
            fb_sig.params.push(AbiParam::new(pointer_type)); // hint ptr
            fb_sig.params.push(AbiParam::new(types::I64)); // hint len
            let fb_id: FuncId = self
                .module
                .declare_function("xezim_jit_stmt_fallback", Linkage::Import, &fb_sig)
                .map_err(|_| ())?;
            let mut load2_sig = self.module.make_signature();
            load2_sig.params.push(AbiParam::new(pointer_type)); // sim
            load2_sig.params.push(AbiParam::new(types::I32)); // id
            load2_sig.params.push(AbiParam::new(pointer_type)); // out_v
            load2_sig.params.push(AbiParam::new(pointer_type)); // out_x
            let load2_id: FuncId = self
                .module
                .declare_function("xezim_jit_load_signal2", Linkage::Import, &load2_sig)
                .map_err(|_| ())?;
            let mut load_slice_sig = self.module.make_signature();
            load_slice_sig.params.push(AbiParam::new(pointer_type)); // sim
            load_slice_sig.params.push(AbiParam::new(types::I32)); // id
            load_slice_sig.params.push(AbiParam::new(types::I32)); // lo
            load_slice_sig.params.push(AbiParam::new(types::I32)); // w
            load_slice_sig.returns.push(AbiParam::new(types::I64));
            let load_slice_id: FuncId = self
                .module
                .declare_function("xezim_jit_load_signal_slice", Linkage::Import, &load_slice_sig)
                .map_err(|_| ())?;
            let load_slice_xz_id: FuncId = self
                .module
                .declare_function(
                    "xezim_jit_load_signal_slice_xz",
                    Linkage::Import,
                    &load_slice_sig,
                )
                .map_err(|_| ())?;
            let load_xz_id: FuncId = self
                .module
                .declare_function("xezim_jit_load_signal_xz", Linkage::Import, &load_sig)
                .map_err(|_| ())?;
            let nba_4s_id: FuncId = self
                .module
                .declare_function(
                    "xezim_jit_schedule_nba_4s",
                    Linkage::Import,
                    &store_4s_sig,
                )
                .map_err(|_| ())?;
            let store_4s_id: FuncId = self
                .module
                .declare_function(
                    "xezim_jit_store_signal_4s",
                    Linkage::Import,
                    &store_4s_sig,
                )
                .map_err(|_| ())?;
            let store_id: FuncId = self
                .module
                .declare_function("xezim_jit_store_signal", Linkage::Import, &store_sig)
                .map_err(|_| ())?;
            let nba_id: FuncId = self
                .module
                .declare_function("xezim_jit_schedule_nba", Linkage::Import, &nba_sig)
                .map_err(|_| ())?;
            let nba_fast_id: FuncId = self
                .module
                .declare_function(
                    "xezim_jit_schedule_nba_fast",
                    Linkage::Import,
                    &nba_fast_sig,
                )
                .map_err(|_| ())?;
            let nba_range_id: FuncId = self
                .module
                .declare_function(
                    "xezim_jit_schedule_nba_range_dyn",
                    Linkage::Import,
                    &nba_range_sig,
                )
                .map_err(|_| ())?;
            let nba_bit_id: FuncId = self
                .module
                .declare_function("xezim_jit_schedule_nba_bit_dyn", Linkage::Import, &nba_bit_sig)
                .map_err(|_| ())?;
            let blk_range_id: FuncId = self
                .module
                .declare_function(
                    "xezim_jit_blocking_assign_range_dyn",
                    Linkage::Import,
                    &nba_range_sig,
                )
                .map_err(|_| ())?;
            let load_array_id: FuncId = self
                .module
                .declare_function("xezim_jit_load_array_elem", Linkage::Import, &load_sig)
                .map_err(|_| ())?;
            let xz_check_id: FuncId = self
                .module
                .declare_function("xezim_jit_inputs_have_xz", Linkage::Import, &xz_check_sig)
                .map_err(|_| ())?;

            // Function signature: extern "C" fn(sim: *mut u8) -> u32
            let mut ctx = self.module.make_context();
            ctx.func.signature.params.push(AbiParam::new(pointer_type));
            ctx.func.signature.returns.push(AbiParam::new(types::I32));

            let mut builder_ctx = FunctionBuilderContext::new();
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut builder_ctx);

            // --- CFG construction ---
            //
            // Identify basic-block leaders (start-of-BB positions):
            //   * PC 0 is always a leader.
            //   * Any `BranchIfFalse`/`Jump` target is a leader.
            //   * The instruction AFTER a branch/jump is a leader.
            //
            // Create one Cranelift `Block` per leader plus one shared
            // `exit_block` that emits `return 0`. Out-of-range jump
            // targets redirect to `exit_block` (matches the interpreter's
            // behavior of falling off the end).
            let n = insns.len();
            let mut is_leader = vec![false; n.max(1)];
            is_leader[0] = true;
            for (i, insn) in insns.iter().enumerate() {
                let mut targets: Vec<usize> = Vec::new();
                match insn {
                    Insn::BranchIfFalse(_, t)
                    | Insn::Jump(t)
                    | Insn::BranchUnlessZero(_, t)
                    | Insn::CmpBranch(_, _, _, _, t)
                    | Insn::BranchIfSignalFalse(_, t, _) => targets.push(*t as usize),
                    Insn::CaseJump(_, cj) => {
                        targets.extend(cj.table.iter().map(|&t| t as usize));
                        targets.push(cj.default as usize);
                    }
                    _ => {}
                }
                if !targets.is_empty() {
                    for t in targets {
                        if t < n {
                            is_leader[t] = true;
                        }
                    }
                    if i + 1 < n {
                        is_leader[i + 1] = true;
                    }
                }
            }
            let mut pc_to_block: Vec<Option<cranelift::codegen::ir::Block>> = vec![None; n.max(1)];
            let exit_block = builder.create_block();
            for (i, &leader) in is_leader.iter().enumerate() {
                if leader {
                    pc_to_block[i] = Some(builder.create_block());
                }
            }

            // Path B X/Z prelude block: created BEFORE the original entry
            // (= pc_to_block[0]) and made the function's true entry. Reads
            // sim_ptr from function params, scans input_ids for X/Z, and
            // either bails (return 1) or jumps to the original entry with
            // sim_ptr passed as a block param.
            let prelude_block = builder.create_block();
            let fallback_block = builder.create_block();
            let entry_block = pc_to_block[0].expect("PC 0 is a leader");

            builder.append_block_params_for_function_params(prelude_block);
            // entry_block now receives sim_ptr from the prelude.
            builder.append_block_param(entry_block, pointer_type);

            builder.switch_to_block(prelude_block);
            let prelude_sim_ptr = builder.block_params(prelude_block)[0];

            // Import bridge functions into this function scope. Done up-front
            // so the prelude can call xz_check_ref before the per-Insn
            // codegen begins.
            let load_ref = self.module.declare_func_in_func(load_id, &mut builder.func);
            let fb_ref = self
                .module
                .declare_func_in_func(fb_id, &mut builder.func);
            let load2_ref = self
                .module
                .declare_func_in_func(load2_id, &mut builder.func);
            let load_slice_ref = self
                .module
                .declare_func_in_func(load_slice_id, &mut builder.func);
            let load_slice_xz_ref = self
                .module
                .declare_func_in_func(load_slice_xz_id, &mut builder.func);
            let load_xz_ref = self
                .module
                .declare_func_in_func(load_xz_id, &mut builder.func);
            let store_4s_ref = self
                .module
                .declare_func_in_func(store_4s_id, &mut builder.func);
            let nba_4s_ref = self
                .module
                .declare_func_in_func(nba_4s_id, &mut builder.func);
            let store_ref = self
                .module
                .declare_func_in_func(store_id, &mut builder.func);
            let nba_ref = self.module.declare_func_in_func(nba_id, &mut builder.func);
            let nba_fast_ref = self
                .module
                .declare_func_in_func(nba_fast_id, &mut builder.func);
            let nba_range_ref = self
                .module
                .declare_func_in_func(nba_range_id, &mut builder.func);
            let nba_bit_ref = self
                .module
                .declare_func_in_func(nba_bit_id, &mut builder.func);
            let blk_range_ref = self
                .module
                .declare_func_in_func(blk_range_id, &mut builder.func);
            let load_array_ref = self
                .module
                .declare_func_in_func(load_array_id, &mut builder.func);
            let xz_check_ref = self
                .module
                .declare_func_in_func(xz_check_id, &mut builder.func);

            // EXPERIMENT: skip the X/Z prelude entirely to measure how
            // much it costs vs the JIT body itself. This is UNSOUND for
            // signals that are X/Z at runtime — only safe to test on
            // workloads where signals are determinate after reset.
            // The codegen above is now 4-STATE: every register carries a
            // val plane and an xz plane, and each operator implements the
            // LRM's unknown-propagation rules. The pre-check that bailed the
            // whole block when any input carried X/Z is therefore obsolete —
            // and it was not cheap: on the C906 SoC it fired on 96.6% of
            // eligible comb evaluations, so those paid a call plus the scan
            // and then ran interpreted anyway. Kept behind
            // `XEZIM_JIT_XZ_BAIL=1` purely as a bisection aid.
            if std::env::var("XEZIM_JIT_XZ_BAIL").is_err()
                || std::env::var("XEZIM_JIT_SKIP_XZ").is_ok()
            {
                let _ = (xz_check_ref, fallback_block, &input_ids);
                builder.ins().jump(entry_block, &[BlockArg::Value(prelude_sim_ptr)]);
            } else if input_ids.is_empty() {
                // No reads — no X/Z risk. Fall straight through.
                builder.ins().jump(entry_block, &[BlockArg::Value(prelude_sim_ptr)]);
            } else if xz_ptr != 0 {
                // Inline prelude: load `signal_has_xz[id]` (a u8) for
                // each input id and OR them. Branches to fallback if
                // any byte is non-zero. Avoids the C-call to
                // `xezim_jit_inputs_have_xz` per block dispatch.
                let xz_base = builder.ins().iconst(pointer_type, xz_ptr as i64);
                let mut acc = builder.ins().iconst(types::I8, 0);
                for &id in input_ids.iter() {
                    if id >= xz_len {
                        // Skip out-of-range ids (keeps inline prelude
                        // safe even if signal_has_xz hasn't been
                        // resized). Matches the bridge fn's `continue`.
                        continue;
                    }
                    let byte = builder.ins().load(
                        types::I8,
                        MemFlagsData::trusted(),
                        xz_base,
                        id as i32,
                    );
                    acc = builder.ins().bor(acc, byte);
                }
                builder.ins().brif(acc, fallback_block, &[], entry_block, &[BlockArg::Value(prelude_sim_ptr)]);
            } else {
                // Materialise input_ids as a fixed stack-slot u32 array,
                // then call xezim_jit_inputs_have_xz(sim, ptr, n).
                let slot_size = (input_ids.len() * 4) as u32;
                let id_slot = builder.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    slot_size,
                    2,
                ));
                for (i, &id) in input_ids.iter().enumerate() {
                    let id_val = builder.ins().iconst(types::I32, id as i64);
                    builder.ins().stack_store(pointer_type, id_val, id_slot, (i * 4) as i32);
                }
                let ids_ptr = builder.ins().stack_addr(pointer_type, id_slot, 0);
                let n_val = builder.ins().iconst(types::I32, input_ids.len() as i64);
                let call = builder
                    .ins()
                    .call(xz_check_ref, &[prelude_sim_ptr, ids_ptr, n_val]);
                let xz_rc = builder.inst_results(call)[0];
                // Branch: rc != 0 → fallback_block (return 1); rc == 0 →
                // jump to entry_block with sim_ptr.
                builder
                    .ins()
                    .brif(xz_rc, fallback_block, &[], entry_block, &[BlockArg::Value(prelude_sim_ptr)]);
            }
            builder.seal_block(prelude_block);

            // Fallback block: return 1 (transient, exec_bytecode keeps JIT
            // armed and runs the interpreter for this execution).
            builder.switch_to_block(fallback_block);
            let one = builder.ins().iconst(types::I32, 1);
            builder.ins().return_(&[one]);
            builder.seal_block(fallback_block);

            // Switch to the original entry block (= pc_to_block[0]).
            builder.switch_to_block(entry_block);
            let sim_ptr = builder.block_params(entry_block)[0];

            // Allocate one 8-byte stack slot per VM register. For blocks
            // with very few registers this still only costs a few bytes.
            let reg_slots: Vec<StackSlot> = (0..num_regs as usize)
                .map(|_| {
                    builder.create_sized_stack_slot(StackSlotData::new(
                        StackSlotKind::ExplicitSlot,
                        8,
                        3,
                    ))
                })
                .collect();
            // Second plane: the X/Z bits of each VM register, same encoding as
            // `Value`'s inline form (val bit + xz bit per position). Without
            // it the JIT can only run when every input is 2-state, which on
            // 4-state RTL is the rare case, not the common one.
            let xz_slots: Vec<StackSlot> = (0..num_regs as usize)
                .map(|_| {
                    builder.create_sized_stack_slot(StackSlotData::new(
                        StackSlotKind::ExplicitSlot,
                        8,
                        3,
                    ))
                })
                .collect();

            let resolve_target = |t: usize,
                                  pc_to_block: &Vec<Option<cranelift::codegen::ir::Block>>|
             -> cranelift::codegen::ir::Block {
                if t < pc_to_block.len() {
                    pc_to_block[t].unwrap_or(exit_block)
                } else {
                    exit_block
                }
            };

            // Walk insns, switching blocks at leaders, emitting terminators
            // for branches/jumps. `live` tracks whether the current block
            // is still open (no terminator emitted yet).
            //
            // JIT Stage 4 Tier A: borrow the signal_widths snapshot so
            // the NbaAssign case below can compile-time-decide between
            // the slow xezim_jit_schedule_nba (with width arg) and the
            // leaner xezim_jit_schedule_nba_fast (no width arg, assumes
            // val_bits already matches signal_widths[sig_id]).
            let signal_widths = &self.signal_widths_snapshot;
            let signal_signed = &self.signal_signed_snapshot;
            // JIT Stage 4 Tier C Path C1: cache the side-queue config
            // so NbaAssign can emit an inline write to the queue
            // (no FFI) when conditions are met.
            let nba_side_queue = self.nba_side_queue;
            // JIT Stage 2: cache the inline_bits pointer so the
            // LoadSignal / LoadSignalSigned cases below can emit a
            // direct load instead of the FFI bridge call.  Stride is
            // 16 bytes per entry; offset 0 holds val_bits (the only
            // field the JIT needs since the X/Z prelude already
            // checked xz_bits for input ids).
            let inline_storage = self.inline_bits_ptr;
            let mut live = true;
            // Static per-register widths + signedness for the post-op
            // masking pass and the signed-op handling in emit_insn.
            let mut reg_widths: Vec<u32> = vec![0; num_regs as usize];
            let mut reg_signed: Vec<bool> = vec![false; num_regs as usize];
            for (i, insn) in insns.iter().enumerate() {
                if i != 0 && is_leader[i] {
                    let new_b = pc_to_block[i].unwrap();
                    if live {
                        builder.ins().jump(new_b, &[]);
                    }
                    builder.switch_to_block(new_b);
                    live = true;
                }
                match insn {
                    Insn::BranchIfFalse(cond, target) => {
                        // §12.4 truth = any DEFINITE 1 (v & ~x): with raw
                        // val planes a Z bit carries v=1, so the bare plane
                        // is no longer a valid truth test.
                        let cv0 = builder
                            .ins()
                            .stack_load(pointer_type, types::I64, reg_slots[*cond as usize], 0);
                        let cx0 = builder
                            .ins()
                            .stack_load(pointer_type, types::I64, xz_slots[*cond as usize], 0);
                        let ncx = builder.ins().bnot(cx0);
                        let cv = builder.ins().band(cv0, ncx);
                        let target_b = resolve_target(*target as usize, &pc_to_block);
                        let fall_b = if i + 1 < n {
                            pc_to_block[i + 1].unwrap_or(exit_block)
                        } else {
                            exit_block
                        };
                        // brif: if cv != 0 -> fall_b (cond true, don't branch)
                        //        else     -> target_b (cond false, take branch)
                        builder.ins().brif(cv, fall_b, &[], target_b, &[]);
                        live = false;
                    }
                    Insn::Jump(target) => {
                        let target_b = resolve_target(*target as usize, &pc_to_block);
                        builder.ins().jump(target_b, &[]);
                        live = false;
                    }
                    // Fused LogNot + BranchIfFalse: jump unless DEFINITE
                    // zero — an X condition takes the branch, exactly like
                    // the interpreter's `is_nonzero() != Some(false)`.
                    Insn::BranchUnlessZero(cond, target) => {
                        let (cv, cx) =
                            ld2(&mut builder, pointer_type, &reg_slots, &xz_slots, *cond);
                        let nz = builder.ins().bor(cv, cx);
                        let target_b = resolve_target(*target as usize, &pc_to_block);
                        let fall_b = if i + 1 < n {
                            pc_to_block[i + 1].unwrap_or(exit_block)
                        } else {
                            exit_block
                        };
                        builder.ins().brif(nz, target_b, &[], fall_b, &[]);
                        live = false;
                    }
                    // Fused compare+branch: compute the compare into the
                    // embedded dead register with the SAME emitters as the
                    // unfused insns, then branch on definite-true (v & ~x).
                    Insn::CmpBranch(kind, l, r, tmp, target) => {
                        use crate::compiler::bytecode::CmpKind as CK;
                        match kind {
                            CK::CaseEq => {
                                let (lv, lx) =
                                    ld2(&mut builder, pointer_type, &reg_slots, &xz_slots, *l);
                                let (rv, rx) =
                                    ld2(&mut builder, pointer_type, &reg_slots, &xz_slots, *r);
                                let veq = builder.ins().icmp(IntCC::Equal, lv, rv);
                                let xeq = builder.ins().icmp(IntCC::Equal, lx, rx);
                                let both = builder.ins().band(veq, xeq);
                                let ext = builder.ins().uextend(types::I64, both);
                                let zero = builder.ins().iconst(types::I64, 0);
                                st2(&mut builder, pointer_type, &reg_slots, &xz_slots, *tmp, ext, zero);
                            }
                            _ => {
                                let cc = match kind {
                                    CK::Eq => IntCC::Equal,
                                    CK::Neq => IntCC::NotEqual,
                                    CK::Lt => IntCC::UnsignedLessThan,
                                    CK::Leq => IntCC::UnsignedLessThanOrEqual,
                                    CK::Gt => IntCC::UnsignedGreaterThan,
                                    CK::Geq => IntCC::UnsignedGreaterThanOrEqual,
                                    CK::CaseEq => unreachable!(),
                                };
                                emit_cmp(
                                    &mut builder,
                                    pointer_type,
                                    &reg_slots,
                                    &xz_slots,
                                    &reg_widths,
                                    &reg_signed,
                                    *tmp,
                                    *l,
                                    *r,
                                    cc,
                                );
                            }
                        }
                        let (tv, tx) =
                            ld2(&mut builder, pointer_type, &reg_slots, &xz_slots, *tmp);
                        let ntx = builder.ins().bnot(tx);
                        let known1 = builder.ins().band(tv, ntx);
                        let target_b = resolve_target(*target as usize, &pc_to_block);
                        let fall_b = if i + 1 < n {
                            pc_to_block[i + 1].unwrap_or(exit_block)
                        } else {
                            exit_block
                        };
                        builder.ins().brif(known1, fall_b, &[], target_b, &[]);
                        live = false;
                    }
                    // Fused `if (!sig[bit]) goto target`. §12.4 truth: a bit
                    // is true iff it is a DEFINITE 1 (val=1, xz=0), so the
                    // branch condition is computed from both planes exactly —
                    // an X input takes the not-taken=false path just like the
                    // interpreter's `bit_select(..).is_true()`.
                    Insn::BranchIfSignalFalse(sig_id, target, bit) => {
                        let (v, x) = match raw_sig_loads(
                            &mut builder,
                            pointer_type,
                            inline_storage,
                            *sig_id,
                        ) {
                            Some(p) => p,
                            None => {
                                let id = builder.ins().iconst(types::I32, *sig_id as i64);
                                let call = builder.ins().call(load_ref, &[sim_ptr, id]);
                                let v = builder.inst_results(call)[0];
                                let xcall = builder.ins().call(load_xz_ref, &[sim_ptr, id]);
                                (v, builder.inst_results(xcall)[0])
                            }
                        };
                        let nx = builder.ins().bnot(x);
                        let mut known1 = builder.ins().band(v, nx);
                        if *bit != u32::MAX {
                            let sh = builder.ins().iconst(types::I64, *bit as i64);
                            let shifted = builder.ins().ushr(known1, sh);
                            let one = builder.ins().iconst(types::I64, 1);
                            known1 = builder.ins().band(shifted, one);
                        }
                        let target_b = resolve_target(*target as usize, &pc_to_block);
                        let fall_b = if i + 1 < n {
                            pc_to_block[i + 1].unwrap_or(exit_block)
                        } else {
                            exit_block
                        };
                        builder.ins().brif(known1, fall_b, &[], target_b, &[]);
                        live = false;
                    }
                    // Case jump table: x/z selector -> default (matches the
                    // interpreter: any unknown bit matches no constant
                    // pattern), out-of-range -> default via br_table.
                    Insn::CaseJump(src, cj) => {
                        let (v, x) =
                            ld2(&mut builder, pointer_type, &reg_slots, &xz_slots, *src);
                        let default_b = resolve_target(cj.default as usize, &pc_to_block);
                        let bounds_b = builder.create_block();
                        builder.ins().brif(x, default_b, &[], bounds_b, &[]);
                        builder.switch_to_block(bounds_b);
                        let len = builder.ins().iconst(types::I64, cj.table.len() as i64);
                        let oob = builder.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, v, len);
                        let dispatch_b = builder.create_block();
                        builder.ins().brif(oob, default_b, &[], dispatch_b, &[]);
                        builder.switch_to_block(dispatch_b);
                        let idx32 = builder.ins().ireduce(types::I32, v);
                        let arm_blocks: Vec<cranelift::codegen::ir::Block> = cj
                            .table
                            .iter()
                            .map(|&t| resolve_target(t as usize, &pc_to_block))
                            .collect();
                        let pool = &mut builder.func.dfg.value_lists;
                        let default_call =
                            BlockCall::new(default_b, core::iter::empty(), pool);
                        let arm_calls: Vec<BlockCall> = arm_blocks
                            .iter()
                            .map(|&b| BlockCall::new(b, core::iter::empty(), pool))
                            .collect();
                        let jt = builder
                            .create_jump_table(JumpTableData::new(default_call, &arm_calls));
                        builder.ins().br_table(idx32, jt);
                        live = false;
                    }
                    // JIT Stage 4 Tier C Path C1: when width matches AND
                    // the side queue is configured, emit a full inline
                    // write — load len, write (sig_id, val_bits) into
                    // queue[len], bump len.  No FFI call.  Saves ~25-30 ns
                    // per NBA vs the Tier A fast bridge.
                    // DISABLED for 4-state: the side-queue entry is 16 bytes
                    // (u32 id + pad + u64 val) with no room for the X/Z plane,
                    // so this path silently dropped every unknown. Re-enabling
                    // it needs a wider entry.
                    Insn::NbaAssign(sig_id, val_reg, width)
                        if FOUR_STATE_NBA_FAST_OK
                            && signal_widths
                                .get(*sig_id as usize)
                                .map_or(false, |&w| w == *width)
                            && nba_side_queue.is_some() =>
                    {
                        let (base_ptr, len_ptr, _cap) = nba_side_queue.unwrap();
                        let v = builder
                            .ins()
                            .stack_load(pointer_type, types::I64, reg_slots[*val_reg as usize], 0);
                        // Load current length (u32) from *len_ptr.
                        let len_addr = builder.ins().iconst(pointer_type, len_ptr as i64);
                        let len = builder.ins().load(
                            types::I32,
                            MemFlagsData::trusted(),
                            len_addr,
                            0,
                        );
                        // Compute slot address: base + len * 16 (sizeof JitNbaSideEntry).
                        let base = builder.ins().iconst(pointer_type, base_ptr as i64);
                        let len64 = builder.ins().uextend(types::I64, len);
                        let entry_size = builder.ins().iconst(types::I64, 16);
                        let offset = builder.ins().imul(len64, entry_size);
                        let slot = builder.ins().iadd(base, offset);
                        // Write signal_id (u32) at offset 0.
                        let sid = builder.ins().iconst(types::I32, *sig_id as i64);
                        builder
                            .ins()
                            .store(MemFlagsData::trusted(), sid, slot, 0);
                        // Write val_bits (u64) at offset 8 (skip the 4-byte
                        // pad after signal_id).
                        builder
                            .ins()
                            .store(MemFlagsData::trusted(), v, slot, 8);
                        // Increment len.
                        let one = builder.ins().iconst(types::I32, 1);
                        let new_len = builder.ins().iadd(len, one);
                        builder
                            .ins()
                            .store(MemFlagsData::trusted(), new_len, len_addr, 0);
                    }
                    // JIT Stage 4 Tier A: when the Insn::NbaAssign's
                    // width matches the signal's declared width, emit
                    // a call to the leaner nba_fast bridge (3-arg, no
                    // width).  Otherwise fall through to the existing
                    // 4-arg bridge below.
                    // DISABLED for 4-state: the 3-arg fast bridge carries only
                    // the value plane.
                    Insn::NbaAssign(sig_id, val_reg, width)
                        if FOUR_STATE_NBA_FAST_OK
                            && signal_widths
                                .get(*sig_id as usize)
                                .map_or(false, |&w| w == *width) =>
                    {
                        let v = builder
                            .ins()
                            .stack_load(pointer_type, types::I64, reg_slots[*val_reg as usize], 0);
                        let id = builder.ins().iconst(types::I32, *sig_id as i64);
                        builder.ins().call(nba_fast_ref, &[sim_ptr, id, v]);
                    }
                    // JIT Stage 2: inline LoadSignal / LoadSignalSigned
                    // when signal_inline_bits storage is available.
                    // Replaces an FFI call (~10-20 ns per load) with a
                    // single u64 load from a baked-in pointer + offset.
                    // Falls through to the FFI path when storage is
                    // unset (XEZIM_INLINE_BITS=0) or sid out of range.
                    Insn::LoadSignal(dest, sig_id) | Insn::LoadSignalSigned(dest, sig_id)
                        if inline_storage
                            .map_or(false, |(_, len)| (*sig_id as u32) < len) =>
                    {
                        let (base_ptr, _len) = inline_storage.unwrap();
                        let base = builder.ins().iconst(pointer_type, base_ptr as i64);
                        // Each `[u64; 2]` entry is 16 bytes: val_bits at offset
                        // 0, xz_bits at offset 8. BOTH planes must be written.
                        // Storing only the value plane left `xz_slots[dest]`
                        // holding whatever the register last had, so every X/Z
                        // signal read back as a determinate value — silently
                        // wrong results, not a crash. (Verified: it wedges the
                        // C906 under `XEZIM_JIT=1 XEZIM_INLINE_BITS=1`.) This
                        // is the same trap that took the NBA fast paths out of
                        // service in `FOUR_STATE_NBA_FAST_OK` above; the read
                        // path was missed at the time.
                        let offset = (*sig_id as i32) * 16;
                        let val =
                            builder
                                .ins()
                                .load(types::I64, MemFlagsData::trusted(), base, offset);
                        let xzv =
                            builder
                                .ins()
                                .load(types::I64, MemFlagsData::trusted(), base, offset + 8);
                        st2(&mut builder, pointer_type, &reg_slots, &xz_slots, *dest, val, xzv);
                    }
                    // AST fallback statement: one bridge call into the
                    // interpreter, scope hint applied around it. The Arc'd
                    // Statement pointer is baked as a constant; the owning
                    // CompiledBlock keeps it alive for the JIT fn's life.
                    Insn::StmtFallback(payload) => {
                        let stmt_ptr = std::sync::Arc::as_ptr(&payload.0) as i64;
                        let sp = builder.ins().iconst(pointer_type, stmt_ptr);
                        let (hp, hl) = self.block_scope_hint.unwrap_or((0, 0));
                        let hpc = builder.ins().iconst(pointer_type, hp as i64);
                        let hlc = builder.ins().iconst(types::I64, hl as i64);
                        builder.ins().call(fb_ref, &[sim_ptr, sp, hpc, hlc]);
                    }
                    other => {
                        let emitted = emit_insn(
                            &mut builder,
                            other,
                            sim_ptr,
                            &reg_slots,
                            &xz_slots,
                            &reg_widths,
                            &reg_signed,
                            signal_widths,
                            inline_storage,
                            load_ref,
                            load_xz_ref,
                            load2_ref,
                            load_slice_ref,
                            load_slice_xz_ref,
                            store_4s_ref,
                            nba_4s_ref,
                            store_ref,
                            nba_ref,
                            nba_range_ref,
                            nba_bit_ref,
                            blk_range_ref,
                            load_array_ref,
                            pointer_type,
                        );
                        if emitted.is_err() {
                            if std::env::var("XEZIM_JIT_BAIL_TRACE").is_ok() {
                                eprintln!(
                                    "[JITBAIL] pc={} {}",
                                    i,
                                    crate::compiler::bytecode::insn_opcode_name(other)
                                );
                            }
                            return Err(());
                        }
                        // Width mask: keep the result within the width the
                        // interpreter's `Value` would have had (see
                        // `insn_result_width`). Both planes — the whole-X
                        // paths write -1 into the xz plane.
                        if let Some((d, w)) =
                            insn_result_width(other, &reg_widths, signal_widths)
                        {
                            let mask = (1u64 << w) - 1;
                            let mc = builder.ins().iconst(types::I64, mask as i64);
                            let v =
                                builder.ins().stack_load(pointer_type, types::I64, reg_slots[d as usize], 0);
                            let mv = builder.ins().band(v, mc);
                            builder.ins().stack_store(pointer_type, mv, reg_slots[d as usize], 0);
                            let x =
                                builder.ins().stack_load(pointer_type, types::I64, xz_slots[d as usize], 0);
                            let mx = builder.ins().band(x, mc);
                            builder.ins().stack_store(pointer_type, mx, xz_slots[d as usize], 0);
                        }
                    }
                }
                // Register width/signedness tracking runs for EVERY insn —
                // the inline arms above (Stage-2 loads, fast NBA, branches)
                // used to skip it, so a LoadSignalSigned handled inline left
                // its register unsigned and every later compare/mask on it
                // was wrong (clearsigned battery: signed compares read
                // unsigned once the SoA loads made the inline arm the
                // default path).
                update_reg_meta(
                    insn,
                    &mut reg_widths,
                    &mut reg_signed,
                    signal_widths,
                    signal_signed,
                );
            }
            // If control falls off the end still live, jump to exit.
            if live {
                builder.ins().jump(exit_block, &[]);
            }
            // Emit return in exit_block.
            builder.switch_to_block(exit_block);
            let zero = builder.ins().iconst(types::I32, 0);
            builder.ins().return_(&[zero]);
            builder.seal_all_blocks();
            builder.finalize(self.module.target_config());
            if std::env::var("XEZIM_JIT_CLIF").is_ok() {
                eprintln!("[CLIF]\n{}", ctx.func.display());
            }

            // Define + finalize the function.
            let fn_name = {
                self.next_id += 1;
                format!("xezim_block_{}", self.next_id)
            };
            let func_id = self
                .module
                .declare_function(&fn_name, Linkage::Export, &ctx.func.signature)
                .map_err(|_| ())?;
            self.module
                .define_function(func_id, &mut ctx)
                .map_err(|_| ())?;
            self.module.clear_context(&mut ctx);
            self.module.finalize_definitions().map_err(|_| ())?;

            let code = self.module.get_finalized_function(func_id);
            Ok(unsafe { std::mem::transmute::<*const u8, JitFn>(code) })
        }
    }

    fn emit_insn(
        builder: &mut FunctionBuilder,
        insn: &Insn,
        sim_ptr: Value,
        regs: &[StackSlot],
        xz: &[StackSlot],
        reg_w: &[u32],
        reg_s: &[bool],
        sig_w: &[u32],
        inline_storage: Option<(u64, u32)>,
        load_ref: FuncRef,
        load_xz_ref: FuncRef,
        load2_ref: FuncRef,
        load_slice_ref: FuncRef,
        load_slice_xz_ref: FuncRef,
        store_4s_ref: FuncRef,
        nba_4s_ref: FuncRef,
        store_ref: FuncRef,
        nba_ref: FuncRef,
        nba_range_ref: FuncRef,
        nba_bit_ref: FuncRef,
        blk_range_ref: FuncRef,
        _load_array_ref: FuncRef,
        // cranelift 0.134 made `stack_load`/`stack_store` take the target
        // pointer type as their first argument.
        pointer_type: Type,
    ) -> Result<(), ()> {
        use Insn::*;
        match insn {
            Nop => {}
            // Fused LoadConst+binop (§ the interpreter's BinOpConst arm):
            // identical semantics to the reg-reg forms with the constant's
            // planes folded in at compile time. The block gate already
            // rejected fill/real constants.
            BinOpConst(d, sr, k, kind) => {
                use crate::compiler::bytecode::BinOpConstKind as K;
                let (kv0, kx0) = k.raw_bits();
                let (sv, sx) = ld2(builder, pointer_type, regs, xz, *sr);
                match kind {
                    K::Add => {
                        if kx0 != 0 {
                            let zero = builder.ins().iconst(types::I64, 0);
                            let ones = builder.ins().iconst(types::I64, -1);
                            st2(builder, pointer_type, regs, xz, *d, zero, ones);
                        } else {
                            let kc = builder.ins().iconst(types::I64, kv0 as i64);
                            let res = builder.ins().iadd(sv, kc);
                            let zero = builder.ins().iconst(types::I64, 0);
                            let ones = builder.ins().iconst(types::I64, -1);
                            let unk = builder.ins().icmp(IntCC::NotEqual, sx, zero);
                            let out_v = builder.ins().select(unk, zero, res);
                            let out_x = builder.ins().select(unk, ones, zero);
                            st2(builder, pointer_type, regs, xz, *d, out_v, out_x);
                        }
                    }
                    K::Xor => {
                        let kvc = builder.ins().iconst(types::I64, kv0 as i64);
                        let kxc = builder.ins().iconst(types::I64, kx0 as i64);
                        let unk = builder.ins().bor(sx, kxc);
                        let x0 = builder.ins().bxor(sv, kvc);
                        let nunk = builder.ins().bnot(unk);
                        let rv = builder.ins().band(x0, nunk);
                        st2(builder, pointer_type, regs, xz, *d, rv, unk);
                    }
                    K::CaseEq => {
                        let kvc = builder.ins().iconst(types::I64, kv0 as i64);
                        let kxc = builder.ins().iconst(types::I64, kx0 as i64);
                        let veq = builder.ins().icmp(IntCC::Equal, sv, kvc);
                        let xeq = builder.ins().icmp(IntCC::Equal, sx, kxc);
                        let both = builder.ins().band(veq, xeq);
                        let ext = builder.ins().uextend(types::I64, both);
                        let zero = builder.ins().iconst(types::I64, 0);
                        st2(builder, pointer_type, regs, xz, *d, ext, zero);
                    }
                    K::Eq => {
                        // Mirror emit_cmp's Equal path with the rhs constant.
                        let both_signed =
                            reg_s.get(*sr as usize).copied().unwrap_or(false) && k.is_signed;
                        let lw = reg_w.get(*sr as usize).copied().unwrap_or(0);
                        let (mut lv, mut lx) = (sv, sx);
                        let (mut kv, mut kx) = (kv0, kx0);
                        if both_signed {
                            if lw > 0 && lw < 64 {
                                let sh = builder.ins().iconst(types::I64, (64 - lw) as i64);
                                let vs = builder.ins().ishl(lv, sh);
                                lv = builder.ins().sshr(vs, sh);
                                let xs = builder.ins().ishl(lx, sh);
                                lx = builder.ins().sshr(xs, sh);
                            }
                            let kw = k.width;
                            if kw > 0 && kw < 64 {
                                kv = (((kv << (64 - kw)) as i64) >> (64 - kw)) as u64;
                                kx = (((kx << (64 - kw)) as i64) >> (64 - kw)) as u64;
                            }
                        }
                        let kvc = builder.ins().iconst(types::I64, kv as i64);
                        let kxc = builder.ins().iconst(types::I64, kx as i64);
                        let cmp = builder.ins().icmp(IntCC::Equal, lv, kvc);
                        let ext = builder.ins().uextend(types::I64, cmp);
                        let anyx = builder.ins().bor(lx, kxc);
                        let zero = builder.ins().iconst(types::I64, 0);
                        let one = builder.ins().iconst(types::I64, 1);
                        let unknown = builder.ins().icmp(IntCC::NotEqual, anyx, zero);
                        let out_v = builder.ins().select(unknown, zero, ext);
                        let out_x = builder.ins().select(unknown, one, zero);
                        let known = builder.ins().bnot(anyx);
                        let diffv = builder.ins().bxor(lv, kvc);
                        let diff_known = builder.ins().band(diffv, known);
                        let decided = builder.ins().icmp(IntCC::NotEqual, diff_known, zero);
                        let out_v = builder.ins().select(decided, zero, out_v);
                        let out_x = builder.ins().select(decided, zero, out_x);
                        st2(builder, pointer_type, regs, xz, *d, out_v, out_x);
                    }
                }
            }
            // §11.4.12.1 replication: n copies, high copy first — same
            // unrolled shift/or shape as Concat.
            Replicate(d, sr, cnt) => {
                let pw = reg_w.get(*sr as usize).copied().unwrap_or(0);
                if pw == 0 || (pw as u64) * (*cnt as u64) > 64 {
                    return Err(());
                }
                if *cnt == 0 {
                    let zero = builder.ins().iconst(types::I64, 0);
                    st2(builder, pointer_type, regs, xz, *d, zero, zero);
                } else {
                    let (pv, px) = ld2(builder, pointer_type, regs, xz, *sr);
                    let m = builder.ins().iconst(types::I64, (mask_of(pw)) as i64);
                    let pv = builder.ins().band(pv, m);
                    let px = builder.ins().band(px, m);
                    let mut av = pv;
                    let mut ax = px;
                    for _ in 1..*cnt {
                        let sh = builder.ins().iconst(types::I64, pw as i64);
                        let avs = builder.ins().ishl(av, sh);
                        av = builder.ins().bor(avs, pv);
                        let axs = builder.ins().ishl(ax, sh);
                        ax = builder.ins().bor(axs, px);
                    }
                    st2(builder, pointer_type, regs, xz, *d, av, ax);
                }
            }
            // §11.4.9 reduction AND: 1 iff every bit is a known 1, 0 if any
            // bit is a known 0, else x. Needs the operand's width for the
            // all-ones compare — available from the reg-width table now.
            ReduceAnd(d, sr) => {
                let sw = reg_w.get(*sr as usize).copied().unwrap_or(0);
                if sw == 0 {
                    return Err(());
                }
                let m = mask_of(sw);
                let (v, x) = ld2(builder, pointer_type, regs, xz, *sr);
                let nx = builder.ins().bnot(x);
                let known1 = builder.ins().band(v, nx);
                let mc = builder.ins().iconst(types::I64, m as i64);
                let all1 = builder.ins().icmp(IntCC::Equal, known1, mc);
                let nv = builder.ins().bnot(v);
                let known0m = builder.ins().band(nv, nx);
                let known0m = builder.ins().band(known0m, mc);
                let zero = builder.ins().iconst(types::I64, 0);
                let any0 = builder.ins().icmp(IntCC::NotEqual, known0m, zero);
                let one = builder.ins().iconst(types::I64, 1);
                let t = builder.ins().uextend(types::I64, all1);
                let known = builder.ins().bor(all1, any0);
                let out_x = builder.ins().select(known, zero, one);
                st2(builder, pointer_type, regs, xz, *d, t, out_x);
            }
            // Value-table lookup (fused constant case): x/z or out-of-range
            // selector picks the default. The tables are leaked host-side
            // and addressed as absolute constants, like the xz prelude.
            CaseLut(d, src, lut) => {
                let n_e = lut.table.len();
                let mut vt: Vec<u64> = Vec::with_capacity(n_e + 1);
                let mut xt: Vec<u64> = Vec::with_capacity(n_e + 1);
                for e in lut.table.iter().chain(std::iter::once(&lut.default)) {
                    if e.is_fill || e.is_real || e.width > 64 {
                        return Err(());
                    }
                    let (ev, ex) = e.raw_bits();
                    vt.push(ev);
                    xt.push(ex);
                }
                let vt: &'static [u64] = Box::leak(vt.into_boxed_slice());
                let xt: &'static [u64] = Box::leak(xt.into_boxed_slice());
                let (sv, sx) = ld2(builder, pointer_type, regs, xz, *src);
                let len = builder.ins().iconst(types::I64, n_e as i64);
                let zero = builder.ins().iconst(types::I64, 0);
                let defi = builder.ins().iconst(types::I64, n_e as i64);
                let sx_nz = builder.ins().icmp(IntCC::NotEqual, sx, zero);
                let oob = builder.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, sv, len);
                let bad = builder.ins().bor(sx_nz, oob);
                let idx = builder.ins().select(bad, defi, sv);
                let eight = builder.ins().iconst(types::I64, 8);
                let off = builder.ins().imul(idx, eight);
                let vb = builder.ins().iconst(pointer_type, vt.as_ptr() as i64);
                let xb = builder.ins().iconst(pointer_type, xt.as_ptr() as i64);
                let va = builder.ins().iadd(vb, off);
                let xa = builder.ins().iadd(xb, off);
                let out_v = builder.ins().load(types::I64, MemFlagsData::trusted(), va, 0);
                let out_x = builder.ins().load(types::I64, MemFlagsData::trusted(), xa, 0);
                st2(builder, pointer_type, regs, xz, *d, out_v, out_x);
            }
            // Signedness lives in the compile-time reg_s table
            // (update_reg_meta); the planes are unchanged.
            ClearSigned(_) => {}
            // §11.4.12: left part lands in the high bits. Widths are the
            // compile-time reg widths; any unknown part width (or a total
            // over 64) aborts this block's compilation — the interpreter
            // keeps it.
            Concat(d, parts) => {
                let mut total: u32 = 0;
                for p in parts.iter() {
                    let w = reg_w.get(*p as usize).copied().unwrap_or(0);
                    if w == 0 {
                        return Err(());
                    }
                    total += w;
                }
                if total > 64 {
                    return Err(());
                }
                let mut acc_v = builder.ins().iconst(types::I64, 0);
                let mut acc_x = builder.ins().iconst(types::I64, 0);
                for p in parts.iter() {
                    let w = reg_w[*p as usize];
                    let (pv, px) = ld2(builder, pointer_type, regs, xz, *p);
                    let (pv, px) = if w < 64 {
                        let mask = builder.ins().iconst(types::I64, ((1u64 << w) - 1) as i64);
                        (builder.ins().band(pv, mask), builder.ins().band(px, mask))
                    } else {
                        (pv, px)
                    };
                    let sh = builder.ins().iconst(types::I64, w as i64);
                    acc_v = builder.ins().ishl(acc_v, sh);
                    acc_v = builder.ins().bor(acc_v, pv);
                    acc_x = builder.ins().ishl(acc_x, sh);
                    acc_x = builder.ins().bor(acc_x, px);
                }
                st2(builder, pointer_type, regs, xz, *d, acc_v, acc_x);
            }
            LoadConst(dest, v) => {
                // `to_u64()` drops X/Z; take the raw planes so an `'x` literal
                // stays unknown instead of silently becoming 0.
                let (vb, xb) = v.raw_bits();
                let c = builder.ins().iconst(types::I64, vb as i64);
                let cx = builder.ins().iconst(types::I64, xb as i64);
                st2(builder, pointer_type, regs, xz, *dest, c, cx);
            }
            LoadSignal(dest, sig_id) | LoadSignalSigned(dest, sig_id) => {
                // SoA raw loads when the plane mirror covers the id (the
                // default); one fused bridge call otherwise.
                match raw_sig_loads(builder, pointer_type, inline_storage, *sig_id) {
                    Some((v, x)) => st2(builder, pointer_type, regs, xz, *dest, v, x),
                    None => {
                        let id = builder.ins().iconst(types::I32, *sig_id as i64);
                        let vp =
                            builder.ins().stack_addr(pointer_type, regs[*dest as usize], 0);
                        let xp = builder.ins().stack_addr(pointer_type, xz[*dest as usize], 0);
                        builder.ins().call(load2_ref, &[sim_ptr, id, vp, xp]);
                    }
                }
            }
            Move(d, s) => {
                let (v, x) = ld2(builder, pointer_type, regs, xz, *s);
                st2(builder, pointer_type, regs, xz, *d, v, x);
            }
            Add(d, l, r) => {
                emit_binop_arith(builder, pointer_type, regs, xz, *d, *l, *r, |b, x, y| b.ins().iadd(x, y))
            }
            Sub(d, l, r) => {
                emit_binop_arith(builder, pointer_type, regs, xz, *d, *l, *r, |b, x, y| b.ins().isub(x, y))
            }
            Mul(d, l, r) => {
                emit_binop_arith(builder, pointer_type, regs, xz, *d, *l, *r, |b, x, y| b.ins().imul(x, y))
            }
            // §11.4.8: bitwise operators propagate X PER BIT — `1'b0 & 1'bx`
            // is 0, not x — so these cannot use the whole-result-X rule the
            // arithmetic ops use. Known-1 / known-0 planes make each table a
            // handful of word ops.
            BitAnd(d, l, r) => {
                let (av, ax) = ld2(builder, pointer_type, regs, xz, *l);
                let (bv, bx) = ld2(builder, pointer_type, regs, xz, *r);
                let na = builder.ins().bnot(ax);
                let nb = builder.ins().bnot(bx);
                let a1 = builder.ins().band(av, na);
                let b1 = builder.ins().band(bv, nb);
                let nav = builder.ins().bnot(av);
                let nbv = builder.ins().bnot(bv);
                let a0 = builder.ins().band(nav, na);
                let b0 = builder.ins().band(nbv, nb);
                let one = builder.ins().band(a1, b1);
                let zero = builder.ins().bor(a0, b0);
                let n_one = builder.ins().bnot(one);
                let n_zero = builder.ins().bnot(zero);
                let rx = builder.ins().band(n_one, n_zero);
                st2(builder, pointer_type, regs, xz, *d, one, rx);
            }
            BitOr(d, l, r) => {
                let (av, ax) = ld2(builder, pointer_type, regs, xz, *l);
                let (bv, bx) = ld2(builder, pointer_type, regs, xz, *r);
                let na = builder.ins().bnot(ax);
                let nb = builder.ins().bnot(bx);
                let a1 = builder.ins().band(av, na);
                let b1 = builder.ins().band(bv, nb);
                let nav = builder.ins().bnot(av);
                let nbv = builder.ins().bnot(bv);
                let a0 = builder.ins().band(nav, na);
                let b0 = builder.ins().band(nbv, nb);
                let one = builder.ins().bor(a1, b1);
                let zero = builder.ins().band(a0, b0);
                let n_one = builder.ins().bnot(one);
                let n_zero = builder.ins().bnot(zero);
                let rx = builder.ins().band(n_one, n_zero);
                st2(builder, pointer_type, regs, xz, *d, one, rx);
            }
            BitXor(d, l, r) => {
                let (av, ax) = ld2(builder, pointer_type, regs, xz, *l);
                let (bv, bx) = ld2(builder, pointer_type, regs, xz, *r);
                let unk = builder.ins().bor(ax, bx);
                let x0 = builder.ins().bxor(av, bv);
                let nunk = builder.ins().bnot(unk);
                let rv = builder.ins().band(x0, nunk);
                st2(builder, pointer_type, regs, xz, *d, rv, unk);
            }
            BitNot(d, s) => {
                // ~: known bits invert, X/Z stay unknown.
                let (v, x) = ld2(builder, pointer_type, regs, xz, *s);
                let nv = builder.ins().bnot(v);
                let nx = builder.ins().bnot(x);
                let rv = builder.ins().band(nv, nx);
                st2(builder, pointer_type, regs, xz, *d, rv, x);
            }
            Eq(d, l, r) => emit_cmp(builder, pointer_type, regs, xz, reg_w, reg_s, *d, *l, *r, IntCC::Equal),
            Neq(d, l, r) => emit_cmp(builder, pointer_type, regs, xz, reg_w, reg_s, *d, *l, *r, IntCC::NotEqual),
            Lt(d, l, r) => emit_cmp(builder, pointer_type, regs, xz, reg_w, reg_s, *d, *l, *r, IntCC::UnsignedLessThan),
            Leq(d, l, r) => emit_cmp(builder, pointer_type, regs, xz, reg_w, reg_s, *d, *l, *r, IntCC::UnsignedLessThanOrEqual),
            Gt(d, l, r) => emit_cmp(builder, pointer_type, regs, xz, reg_w, reg_s, *d, *l, *r, IntCC::UnsignedGreaterThan),
            Geq(d, l, r) => emit_cmp(builder, pointer_type, regs, xz, reg_w, reg_s, *d, *l, *r, IntCC::UnsignedGreaterThanOrEqual),
            Shl(d, l, r) => {
                emit_shift(builder, pointer_type, regs, xz, *d, *l, *r, |b, x, y| b.ins().ishl(x, y))
            }
            Shr(d, l, r) => {
                emit_shift(builder, pointer_type, regs, xz, *d, *l, *r, |b, x, y| b.ins().ushr(x, y))
            }
            AShr(d, l, r) => {
                // §11.4.10.1: `>>>` shifts in copies of the SIGN BIT — which
                // lives at the operand's declared width, not at bit 63.
                // Sign-extend to 64 first so sshr fills correctly; the
                // post-op width mask trims the result back.
                let lw = reg_w.get(*l as usize).copied().unwrap_or(0);
                let signed = reg_s.get(*l as usize).copied().unwrap_or(false);
                if signed && lw > 0 && lw < 64 {
                    let (lv, lx) = ld2(builder, pointer_type, regs, xz, *l);
                    let (ve, xe) = sext_planes(builder, lv, lx, lw);
                    st2(builder, pointer_type, regs, xz, *l, ve, xe);
                }
                emit_shift(builder, pointer_type, regs, xz, *d, *l, *r, |b, x, y| b.ins().sshr(x, y))
            }
            BitXnor(d, l, r) => {
                let (av, ax) = ld2(builder, pointer_type, regs, xz, *l);
                let (bv, bx) = ld2(builder, pointer_type, regs, xz, *r);
                let unk = builder.ins().bor(ax, bx);
                let x0 = builder.ins().bxor(av, bv);
                let inv = builder.ins().bnot(x0);
                let nunk = builder.ins().bnot(unk);
                let rv = builder.ins().band(inv, nunk);
                st2(builder, pointer_type, regs, xz, *d, rv, unk);
            }
            LogAnd(d, l, r) => {
                // §11.4.7: `0 && x` is 0, not x — an operand that is DEFINITELY
                // false decides the result even when the other is unknown.
                let (lv, lx) = ld2(builder, pointer_type, regs, xz, *l);
                let (rv, rx) = ld2(builder, pointer_type, regs, xz, *r);
                let zero = builder.ins().iconst(types::I64, 0);
                let one = builder.ins().iconst(types::I64, 1);
                let (lt, lf) = truthiness(builder, lv, lx);
                let (rt, rf) = truthiness(builder, rv, rx);
                let is_true = builder.ins().band(lt, rt);
                let is_false = builder.ins().bor(lf, rf);
                let tv = builder.ins().uextend(types::I64, is_true);
                let known = builder.ins().bor(is_true, is_false);
                let out_x = builder.ins().select(known, zero, one);
                st2(builder, pointer_type, regs, xz, *d, tv, out_x);
            }
            LogOr(d, l, r) => {
                // §11.4.7: `1 || x` is 1.
                let (lv, lx) = ld2(builder, pointer_type, regs, xz, *l);
                let (rv, rx) = ld2(builder, pointer_type, regs, xz, *r);
                let zero = builder.ins().iconst(types::I64, 0);
                let one = builder.ins().iconst(types::I64, 1);
                let (lt, lf) = truthiness(builder, lv, lx);
                let (rt, rf) = truthiness(builder, rv, rx);
                let is_true = builder.ins().bor(lt, rt);
                let is_false = builder.ins().band(lf, rf);
                let tv = builder.ins().uextend(types::I64, is_true);
                let known = builder.ins().bor(is_true, is_false);
                let out_x = builder.ins().select(known, zero, one);
                st2(builder, pointer_type, regs, xz, *d, tv, out_x);
            }
            LogNot(d, s) => {
                let (v, x) = ld2(builder, pointer_type, regs, xz, *s);
                let zero = builder.ins().iconst(types::I64, 0);
                let one = builder.ins().iconst(types::I64, 1);
                let (t, f) = truthiness(builder, v, x);
                let ext = builder.ins().uextend(types::I64, f);
                let known = builder.ins().bor(t, f);
                let out_x = builder.ins().select(known, zero, one);
                st2(builder, pointer_type, regs, xz, *d, ext, out_x);
            }
            Negate(d, s) => {
                // §11.4.3: unary minus of an unknown operand is unknown.
                let (v, x) = ld2(builder, pointer_type, regs, xz, *s);
                let neg = builder.ins().ineg(v);
                let zero = builder.ins().iconst(types::I64, 0);
                let ones = builder.ins().iconst(types::I64, -1);
                let unk = builder.ins().icmp(IntCC::NotEqual, x, zero);
                let out_v = builder.ins().select(unk, zero, neg);
                let out_x = builder.ins().select(unk, ones, zero);
                st2(builder, pointer_type, regs, xz, *d, out_v, out_x);
            }
            // ReduceAnd intentionally NOT JIT'd: requires width info to
            // compare val against the width-specific all-ones mask, and
            // the Insn doesn't carry width directly. Stays in is_supported
            // = false and routes through the interpreter.
            //
            ReduceOr(d, s) => {
                // §11.4.9: a known 1 anywhere decides the result even with
                // unknown bits present; otherwise any unknown bit makes it x.
                let (v, x) = ld2(builder, pointer_type, regs, xz, *s);
                let zero = builder.ins().iconst(types::I64, 0);
                let one = builder.ins().iconst(types::I64, 1);
                let (t, f) = truthiness(builder, v, x);
                let ext = builder.ins().uextend(types::I64, t);
                let known = builder.ins().bor(t, f);
                let out_x = builder.ins().select(known, zero, one);
                st2(builder, pointer_type, regs, xz, *d, ext, out_x);
            }
            ReduceXor(d, s) => {
                // §11.4.9: parity is unknown if ANY bit is unknown.
                let (v, x) = ld2(builder, pointer_type, regs, xz, *s);
                let pc = builder.ins().popcnt(v);
                let one = builder.ins().iconst(types::I64, 1);
                let parity = builder.ins().band(pc, one);
                let zero = builder.ins().iconst(types::I64, 0);
                let unk = builder.ins().icmp(IntCC::NotEqual, x, zero);
                let out_v = builder.ins().select(unk, zero, parity);
                let out_x = builder.ins().select(unk, one, zero);
                st2(builder, pointer_type, regs, xz, *d, out_v, out_x);
            }
            // NOTE: an earlier 2-state Select arm lived here and was
            // intentionally left out of is_supported (it mismatched the
            // interpreter's 4-state semantics on picorv32 — TRAP at 87,467
            // cycles instead of 520,326). When Select was added back to
            // is_supported for the 4-state codegen, THIS dead arm silently
            // reactivated and shadowed the 4-state arm below it in the same
            // match — Select ran 2-state, never wrote the xz slot, and the
            // following store read an uninitialized stack slot as the X/Z
            // plane. The 4-state arm below (truthiness + §11.4.11 merge) is
            // the only Select emission now; per-register width tracking
            // (`update_reg_width`) addresses the old arm's width concern.
            CaseEq(d, l, r) => {
                // SV `===` compares X/Z LITERALLY (§11.4.6): both planes must
                // match, and the result is always a known 0/1. The old
                // val-plane-only compare was correct solely because the X/Z
                // pre-check kept unknowns out; with 4-state registers it
                // would have called `4'bxxxx === 4'b0000` equal.
                let (lv, lx) = ld2(builder, pointer_type, regs, xz, *l);
                let (rv, rx) = ld2(builder, pointer_type, regs, xz, *r);
                let veq = builder.ins().icmp(IntCC::Equal, lv, rv);
                let xeq = builder.ins().icmp(IntCC::Equal, lx, rx);
                let both = builder.ins().band(veq, xeq);
                let ext = builder.ins().uextend(types::I64, both);
                let zero = builder.ins().iconst(types::I64, 0);
                st2(builder, pointer_type, regs, xz, *d, ext, zero);
            }
            SetSigned(_) => {
                // No-op in 2-state JIT: signedness is a per-Value flag the
                // bytecode propagates, but the JIT operates on raw u64
                // val_bits. The interpreter / bridge re-applies signedness
                // when materialising results into signal_table.
            }
            BlockingAssign(sig_id, val_reg, width) => {
                let (v, x) = ld2(builder, pointer_type, regs, xz, *val_reg);
                let id = builder.ins().iconst(types::I32, *sig_id as i64);
                let w = builder.ins().iconst(types::I32, *width as i64);
                builder.ins().call(store_4s_ref, &[sim_ptr, id, v, x, w]);
            }
            BlockingAssignRangeDyn(sig_id, hi_reg, lo_reg, val_reg) => {
                let (v, x) = ld2(builder, pointer_type, regs, xz, *val_reg);
                let id = builder.ins().iconst(types::I32, *sig_id as i64);
                let hi = builder
                    .ins()
                    .stack_load(pointer_type, types::I64, regs[*hi_reg as usize], 0);
                let lo = builder
                    .ins()
                    .stack_load(pointer_type, types::I64, regs[*lo_reg as usize], 0);
                builder
                    .ins()
                    .call(blk_range_ref, &[sim_ptr, id, hi, lo, v, x]);
            }
            // Constant-bounds forms of the same stores: materialize the
            // bounds and share the dynamic bridges.
            BlockingAssignRange(sig_id, hi, lo, val_reg) => {
                let (v, x) = ld2(builder, pointer_type, regs, xz, *val_reg);
                let id = builder.ins().iconst(types::I32, *sig_id as i64);
                let hi_v = builder.ins().iconst(types::I64, *hi as i64);
                let lo_v = builder.ins().iconst(types::I64, *lo as i64);
                builder
                    .ins()
                    .call(blk_range_ref, &[sim_ptr, id, hi_v, lo_v, v, x]);
            }
            BlockingAssignBitDyn(sig_id, idx_reg, val_reg) => {
                let (v, x) = ld2(builder, pointer_type, regs, xz, *val_reg);
                let id = builder.ins().iconst(types::I32, *sig_id as i64);
                let idx = builder
                    .ins()
                    .stack_load(pointer_type, types::I64, regs[*idx_reg as usize], 0);
                // A 1-bit write at [idx:idx] — same semantics, same bridge.
                builder
                    .ins()
                    .call(blk_range_ref, &[sim_ptr, id, idx, idx, v, x]);
            }
            NbaAssign(sig_id, val_reg, width) => {
                let (nv, nx) = ld2(builder, pointer_type, regs, xz, *val_reg);
                let nid = builder.ins().iconst(types::I32, *sig_id as i64);
                let nw = builder.ins().iconst(types::I32, *width as i64);
                builder.ins().call(nba_4s_ref, &[sim_ptr, nid, nv, nx, nw]);
                return Ok(());
                #[allow(unreachable_code)]
                let v = builder
                    .ins()
                    .stack_load(pointer_type, types::I64, regs[*val_reg as usize], 0);
                let id = builder.ins().iconst(types::I32, *sig_id as i64);
                let w = builder.ins().iconst(types::I32, *width as i64);
                builder.ins().call(nba_ref, &[sim_ptr, id, v, w]);
            }
            // Fused LoadConst + NbaAssign: both planes are compile-time
            // constants. The gate rejected fill/real constants.
            NbaAssignConst(sig_id, k, width) => {
                let (kv, kx) = k.raw_bits();
                let vc = builder.ins().iconst(types::I64, kv as i64);
                let xc = builder.ins().iconst(types::I64, kx as i64);
                let id = builder.ins().iconst(types::I32, *sig_id as i64);
                let wc = builder.ins().iconst(types::I32, *width as i64);
                builder.ins().call(nba_4s_ref, &[sim_ptr, id, vc, xc, wc]);
            }
            // Dense-array non-blocking element store (`mem[addr] <= data`):
            // same index rules as the blocking form, scheduled via the NBA
            // bridge. Out-of-range (or unresolvable) indices store nowhere.
            NbaAssignArray(arr, idx_reg, val_reg, width) => {
                let crate::compiler::bytecode::ArrayOperand::Dense {
                    first_id, lo, hi, ..
                } = arr.as_ref()
                else {
                    return Err(());
                };
                let (iv, ix) = ld2(builder, pointer_type, regs, xz, *idx_reg);
                let (vv, vx) = ld2(builder, pointer_type, regs, xz, *val_reg);
                let nix = builder.ins().bnot(ix);
                let eff = builder.ins().band(iv, nix);
                let lo_c = builder.ins().iconst(types::I64, *lo);
                let hi_c = builder.ins().iconst(types::I64, *hi);
                let ge = builder.ins().icmp(IntCC::SignedGreaterThanOrEqual, eff, lo_c);
                let le = builder.ins().icmp(IntCC::SignedLessThanOrEqual, eff, hi_c);
                let inb = builder.ins().band(ge, le);
                let store_b = builder.create_block();
                let join_b = builder.create_block();
                builder.ins().brif(inb, store_b, &[], join_b, &[]);
                builder.switch_to_block(store_b);
                let off = builder.ins().isub(eff, lo_c);
                let first_c = builder.ins().iconst(types::I64, *first_id as i64);
                let eid = builder.ins().iadd(first_c, off);
                let eid32 = builder.ins().ireduce(types::I32, eid);
                let wc = builder.ins().iconst(types::I32, *width as i64);
                builder
                    .ins()
                    .call(nba_4s_ref, &[sim_ptr, eid32, vv, vx, wc]);
                builder.ins().jump(join_b, &[]);
                builder.switch_to_block(join_b);
            }
            // Fused `rdata <= mem[addr]` (LoadSignal + LoadArrayElem +
            // NbaAssign): index comes straight from a signal, the element
            // value goes straight to the NBA queue. Out-of-range reads
            // schedule a 1-bit X, exactly like the interpreter composition.
            NbaAssignArrayRead(dst_sig, arr, idx_sig, width) => {
                let crate::compiler::bytecode::ArrayOperand::Dense {
                    first_id, lo, hi, ..
                } = arr.as_ref()
                else {
                    return Err(());
                };
                let (iv, ix) = match raw_sig_loads(builder, pointer_type, inline_storage, *idx_sig)
                {
                    Some(p) => p,
                    None => {
                        let idc = builder.ins().iconst(types::I32, *idx_sig as i64);
                        let call = builder.ins().call(load_ref, &[sim_ptr, idc]);
                        let iv = builder.inst_results(call)[0];
                        let xcall = builder.ins().call(load_xz_ref, &[sim_ptr, idc]);
                        (iv, builder.inst_results(xcall)[0])
                    }
                };
                let nix = builder.ins().bnot(ix);
                let eff = builder.ins().band(iv, nix);
                let lo_c = builder.ins().iconst(types::I64, *lo);
                let hi_c = builder.ins().iconst(types::I64, *hi);
                let ge = builder.ins().icmp(IntCC::SignedGreaterThanOrEqual, eff, lo_c);
                let le = builder.ins().icmp(IntCC::SignedLessThanOrEqual, eff, hi_c);
                let inb = builder.ins().band(ge, le);
                let off = builder.ins().isub(eff, lo_c);
                let first_c = builder.ins().iconst(types::I64, *first_id as i64);
                let eid = builder.ins().iadd(first_c, off);
                let eid_safe = builder.ins().select(inb, eid, first_c);
                let (ev, ex) = match inline_storage {
                    Some((base_ptr, len)) if (*first_id as u64) < len as u64 => {
                        raw_sig_loads_dyn(builder, pointer_type, base_ptr, eid_safe)
                    }
                    _ => {
                        let eid32 = builder.ins().ireduce(types::I32, eid_safe);
                        let vcall = builder.ins().call(load_ref, &[sim_ptr, eid32]);
                        let ev = builder.inst_results(vcall)[0];
                        let excall = builder.ins().call(load_xz_ref, &[sim_ptr, eid32]);
                        (ev, builder.inst_results(excall)[0])
                    }
                };
                let zero = builder.ins().iconst(types::I64, 0);
                let one = builder.ins().iconst(types::I64, 1);
                let minus1 = builder.ins().iconst(types::I64, -1);
                let inb_mask = builder.ins().select(inb, minus1, zero);
                let v = builder.ins().band(ev, inb_mask);
                let x = builder.ins().select(inb, ex, one);
                let dc = builder.ins().iconst(types::I32, *dst_sig as i64);
                let wc = builder.ins().iconst(types::I32, *width as i64);
                builder.ins().call(nba_4s_ref, &[sim_ptr, dc, v, x, wc]);
            }
            NbaAssignRange(sig_id, hi, lo, val_reg) => {
                let (v, x) = ld2(builder, pointer_type, regs, xz, *val_reg);
                let id = builder.ins().iconst(types::I32, *sig_id as i64);
                let hi_v = builder.ins().iconst(types::I64, *hi as i64);
                let lo_v = builder.ins().iconst(types::I64, *lo as i64);
                builder
                    .ins()
                    .call(nba_range_ref, &[sim_ptr, id, hi_v, lo_v, v, x]);
            }
            NbaAssignRangeDyn(sig_id, hi_reg, lo_reg, val_reg) => {
                let (v, x) = ld2(builder, pointer_type, regs, xz, *val_reg);
                let id = builder.ins().iconst(types::I32, *sig_id as i64);
                let hi = builder
                    .ins()
                    .stack_load(pointer_type, types::I64, regs[*hi_reg as usize], 0);
                let lo = builder
                    .ins()
                    .stack_load(pointer_type, types::I64, regs[*lo_reg as usize], 0);
                builder
                    .ins()
                    .call(nba_range_ref, &[sim_ptr, id, hi, lo, v, x]);
            }
            NbaAssignBitDyn(sig_id, idx_reg, val_reg) => {
                let (v, x) = ld2(builder, pointer_type, regs, xz, *val_reg);
                let id = builder.ins().iconst(types::I32, *sig_id as i64);
                let idx = builder
                    .ins()
                    .stack_load(pointer_type, types::I64, regs[*idx_reg as usize], 0);
                builder.ins().call(nba_bit_ref, &[sim_ptr, id, idx, v, x]);
            }
            // NbaAssignRangeDyn / NbaAssignBitDyn still left out — they
            // need dynamic hi/lo from VM regs, requiring extra value
            // shuffling. Tractable next step but not in this slice.
            // Fused load+select forms: two bridge calls then shift/mask on
            // both planes. The block-level width filter guarantees the source
            // signal fits u64, so the extraction is pure register math.
            LoadSignalBit(dest, sig_id, bit) => {
                // §11.5.1: a bit index beyond the signal's width reads x —
                // known at compile time here, so it becomes a constant.
                let sw = sig_w.get(*sig_id as usize).copied().unwrap_or(0);
                if sw > 0 && *bit >= sw {
                    let zero = builder.ins().iconst(types::I64, 0);
                    let one = builder.ins().iconst(types::I64, 1);
                    st2(builder, pointer_type, regs, xz, *dest, zero, one);
                } else if sw > 64 && *bit >= 64 {
                    // WIDE signal, bit above the low plane word. The fast
                    // path below loads only that low word and emits
                    // `v >> bit`; a shift amount >= 64 is masked to
                    // `bit & 63`, so `sig[242]` silently returned bit 50 of
                    // the low word — a wrong-but-plausible value that
                    // propagated as real data. (`block_signals_fit_u64`
                    // admits this shape on the stated assumption that bits
                    // >= 64 take the slice bridge; only `LoadSignalRange`
                    // actually did.) Take the same bridge here: it addresses
                    // any window of a wide signal and returns it at bit 0.
                    let id = builder.ins().iconst(types::I32, *sig_id as i64);
                    let loc = builder.ins().iconst(types::I32, *bit as i64);
                    let wc = builder.ins().iconst(types::I32, 1);
                    let call = builder.ins().call(load_slice_ref, &[sim_ptr, id, loc, wc]);
                    let v = builder.inst_results(call)[0];
                    let xcall = builder.ins().call(load_slice_xz_ref, &[sim_ptr, id, loc, wc]);
                    let x = builder.inst_results(xcall)[0];
                    let one = builder.ins().iconst(types::I64, 1);
                    let vb = builder.ins().band(v, one);
                    let xb = builder.ins().band(x, one);
                    st2(builder, pointer_type, regs, xz, *dest, vb, xb);
                } else {
                    let (v, x) = match raw_sig_loads(builder, pointer_type, inline_storage, *sig_id)
                    {
                        Some(p) => p,
                        None => {
                            let id = builder.ins().iconst(types::I32, *sig_id as i64);
                            let call = builder.ins().call(load_ref, &[sim_ptr, id]);
                            let v = builder.inst_results(call)[0];
                            let xcall = builder.ins().call(load_xz_ref, &[sim_ptr, id]);
                            (v, builder.inst_results(xcall)[0])
                        }
                    };
                    let sh = builder.ins().iconst(types::I64, *bit as i64);
                    let one = builder.ins().iconst(types::I64, 1);
                    let vs = builder.ins().ushr(v, sh);
                    let vb = builder.ins().band(vs, one);
                    let xs = builder.ins().ushr(x, sh);
                    let xb = builder.ins().band(xs, one);
                    st2(builder, pointer_type, regs, xz, *dest, vb, xb);
                }
            }
            LoadSignalRange(dest, sig_id, left, right) => {
                let lo = (*left).min(*right);
                let hi = (*left).max(*right);
                let w = left.abs_diff(*right) + 1;
                if w > 64 {
                    return Err(());
                }
                // §11.5.1: positions beyond the signal's width read x.
                let sw = sig_w.get(*sig_id as usize).copied().unwrap_or(0);
                let full = mask_of(w);
                let oor: u64 = if sw > 0 && lo >= sw {
                    full
                } else if sw > 0 && hi >= sw {
                    (full >> (sw - lo)) << (sw - lo)
                } else {
                    0
                };
                if sw > 64 {
                    // Wide signal: the slice bridges hand back exactly the
                    // addressed window, already shifted to bit 0.
                    let id = builder.ins().iconst(types::I32, *sig_id as i64);
                    let loc = builder.ins().iconst(types::I32, lo as i64);
                    let wc = builder.ins().iconst(types::I32, w as i64);
                    let call = builder.ins().call(load_slice_ref, &[sim_ptr, id, loc, wc]);
                    let v = builder.inst_results(call)[0];
                    let xcall =
                        builder.ins().call(load_slice_xz_ref, &[sim_ptr, id, loc, wc]);
                    let x = builder.inst_results(xcall)[0];
                    let keepc = builder.ins().iconst(types::I64, (full & !oor) as i64);
                    let vm = builder.ins().band(v, keepc);
                    let xm0 = builder.ins().band(x, keepc);
                    let oc = builder.ins().iconst(types::I64, oor as i64);
                    let xm = builder.ins().bor(xm0, oc);
                    st2(builder, pointer_type, regs, xz, *dest, vm, xm);
                    return Ok(());
                }
                let (v, x) = match raw_sig_loads(builder, pointer_type, inline_storage, *sig_id) {
                    Some(p) => p,
                    None => {
                        let id = builder.ins().iconst(types::I32, *sig_id as i64);
                        let call = builder.ins().call(load_ref, &[sim_ptr, id]);
                        let v = builder.inst_results(call)[0];
                        let xcall = builder.ins().call(load_xz_ref, &[sim_ptr, id]);
                        (v, builder.inst_results(xcall)[0])
                    }
                };
                let sh = builder.ins().iconst(types::I64, lo as i64);
                let keepc = builder.ins().iconst(types::I64, (full & !oor) as i64);
                let vs = builder.ins().ushr(v, sh);
                let vm = builder.ins().band(vs, keepc);
                let xs = builder.ins().ushr(x, sh);
                let xm0 = builder.ins().band(xs, keepc);
                let oc = builder.ins().iconst(types::I64, oor as i64);
                let xm = builder.ins().bor(xm0, oc);
                st2(builder, pointer_type, regs, xz, *dest, vm, xm);
            }
            // §11.4.11: `c ? t : e` — a definitely-true condition takes t, a
            // definitely-false one takes e, and an UNKNOWN condition merges:
            // bits where both branches are known and agree keep their value,
            // every other bit reads x.
            Select(dest, c, t, e) => {
                let (cv, cx) = ld2(builder, pointer_type, regs, xz, *c);
                let (tv, tx) = ld2(builder, pointer_type, regs, xz, *t);
                let (ev, ex) = ld2(builder, pointer_type, regs, xz, *e);
                let (ct, cf) = truthiness(builder, cv, cx);
                let ntx = builder.ins().bnot(tx);
                let nex = builder.ins().bnot(ex);
                let both_known = builder.ins().band(ntx, nex);
                let diff = builder.ins().bxor(tv, ev);
                let ndiff = builder.ins().bnot(diff);
                let agree = builder.ins().band(both_known, ndiff);
                let mv = builder.ins().band(tv, agree);
                let mx = builder.ins().bnot(agree);
                let sel_ve = builder.ins().select(cf, ev, mv);
                let sel_xe = builder.ins().select(cf, ex, mx);
                let out_v = builder.ins().select(ct, tv, sel_ve);
                let out_x = builder.ins().select(ct, tx, sel_xe);
                st2(builder, pointer_type, regs, xz, *dest, out_v, out_x);
            }
            // Fused Move+Resize: resize the SOURCE register's planes into
            // the destination. Same §11.8.1 sign-extension rule as Resize.
            MoveResize(d, sr, width) => {
                if *width > 64 {
                    return Err(());
                }
                let (mut v, mut x) = ld2(builder, pointer_type, regs, xz, *sr);
                let cur_w = reg_w.get(*sr as usize).copied().unwrap_or(0);
                if reg_s.get(*sr as usize).copied().unwrap_or(false)
                    && cur_w > 0
                    && *width > cur_w
                {
                    let (ve, xe) = sext_planes(builder, v, x, cur_w);
                    v = ve;
                    x = xe;
                }
                let mask: u64 = if *width >= 64 {
                    u64::MAX
                } else {
                    (1u64 << *width) - 1
                };
                let mc = builder.ins().iconst(types::I64, mask as i64);
                let mv = builder.ins().band(v, mc);
                let mx = builder.ins().band(x, mc);
                st2(builder, pointer_type, regs, xz, *d, mv, mx);
            }
            Resize(reg, width) => {
                // Emulates Value::resize: narrowing masks; widening a SIGNED
                // register SIGN-EXTENDS from its current width (§11.8.1 —
                // `-1` at 32 bits resized to a 64-bit context must become
                // 64-bit -1, which is what makes `(-1)*(-1)` equal 1). The
                // xz plane extends identically so an X sign bit fills as X.
                let (mut v, mut x) = ld2(builder, pointer_type, regs, xz, *reg);
                let cur_w = reg_w.get(*reg as usize).copied().unwrap_or(0);
                if reg_s.get(*reg as usize).copied().unwrap_or(false)
                    && cur_w > 0
                    && *width > cur_w
                {
                    let (ve, xe) = sext_planes(builder, v, x, cur_w);
                    v = ve;
                    x = xe;
                }
                let mask: u64 = if *width >= 64 {
                    u64::MAX
                } else {
                    (1u64 << *width) - 1
                };
                let mc = builder.ins().iconst(types::I64, mask as i64);
                let mv = builder.ins().band(v, mc);
                let mx = builder.ins().band(x, mc);
                st2(builder, pointer_type, regs, xz, *reg, mv, mx);
            }
            // §11.5.1 — all four select forms below are 4-STATE and
            // OUT-OF-RANGE AWARE: a position outside the base's declared
            // width reads x, and a `-:` select can carry a NEGATIVE low
            // bound at runtime (the old arm's unsigned min/max picked the
            // wrong lsb for those, and every arm dropped the xz plane
            // entirely — surfaced the moment comb entries began compiling).
            BitSelect(dest, base, idx) => {
                let w = reg_w.get(*base as usize).copied().unwrap_or(0);
                if w == 0 {
                    return Err(());
                }
                let (bv, bx) = ld2(builder, pointer_type, regs, xz, *base);
                let i = builder.ins().stack_load(pointer_type, types::I64, regs[*idx as usize], 0);
                // One UNSIGNED compare covers both ends: a negative index
                // wraps to a huge u64 and fails `idx < w`.
                let wv = builder.ins().iconst(types::I64, w as i64);
                let inr = builder.ins().icmp(IntCC::UnsignedLessThan, i, wv);
                let one = builder.ins().iconst(types::I64, 1);
                let zero = builder.ins().iconst(types::I64, 0);
                let vs = builder.ins().ushr(bv, i);
                let vb = builder.ins().band(vs, one);
                let xs = builder.ins().ushr(bx, i);
                let xb = builder.ins().band(xs, one);
                let out_v = builder.ins().select(inr, vb, zero);
                let out_x = builder.ins().select(inr, xb, one);
                st2(builder, pointer_type, regs, xz, *dest, out_v, out_x);
            }
            BitSelectConst(dest, base, idx) => {
                let w = reg_w.get(*base as usize).copied().unwrap_or(0);
                if w == 0 {
                    return Err(());
                }
                if *idx >= w {
                    // Compile-time out of range: constant x.
                    let zero = builder.ins().iconst(types::I64, 0);
                    let one = builder.ins().iconst(types::I64, 1);
                    st2(builder, pointer_type, regs, xz, *dest, zero, one);
                } else {
                    let (bv, bx) = ld2(builder, pointer_type, regs, xz, *base);
                    let one = builder.ins().iconst(types::I64, 1);
                    let vs = builder.ins().ushr_imm(bv, *idx as i64);
                    let vb = builder.ins().band(vs, one);
                    let xs = builder.ins().ushr_imm(bx, *idx as i64);
                    let xb = builder.ins().band(xs, one);
                    st2(builder, pointer_type, regs, xz, *dest, vb, xb);
                }
            }
            RangeSelect(dest, base, left_r, right_r) => {
                let w = reg_w.get(*base as usize).copied().unwrap_or(0);
                if w == 0 {
                    return Err(());
                }
                let (bv, bx) = ld2(builder, pointer_type, regs, xz, *base);
                let mut l = builder
                    .ins()
                    .stack_load(pointer_type, types::I64, regs[*left_r as usize], 0);
                let mut r = builder
                    .ins()
                    .stack_load(pointer_type, types::I64, regs[*right_r as usize], 0);
                // Match the interpreter exactly: bounds are 32-bit index
                // arithmetic, truncated to u32 and REINTERPRETED as i32
                // unconditionally (`[1 -: 4]`'s low bound arrives as
                // 0xFFFF_FFFE and must read as -2). No signedness heuristics
                // — the interpreter applies none either.
                let c32 = builder.ins().iconst(types::I64, 32);
                for br in [&mut l, &mut r] {
                    let up = builder.ins().ishl(*br, c32);
                    *br = builder.ins().sshr(up, c32);
                }
                // SIGNED min/max: `[1 -: 4]` reaches here with lsb = -2.
                let le = builder.ins().icmp(IntCC::SignedLessThanOrEqual, l, r);
                let lsb = builder.ins().select(le, l, r);
                let msb = builder.ins().select(le, r, l);
                let zero = builder.ins().iconst(types::I64, 0);
                let ones = builder.ins().iconst(types::I64, -1);
                let c64 = builder.ins().iconst(types::I64, 64);
                // Value/xz planes shifted toward bit 0: right by lsb when
                // lsb >= 0, left by -lsb otherwise. Shift amounts >= 64
                // wrap on x86, so they are selected to a zero result.
                let neg = builder.ins().icmp(IntCC::SignedLessThan, lsb, zero);
                let nlsb = builder.ins().ineg(lsb);
                let sr = builder.ins().select(neg, zero, lsb);
                let sl = builder.ins().select(neg, nlsb, zero);
                let sr_big = builder.ins().icmp(IntCC::SignedGreaterThanOrEqual, sr, c64);
                let sl_big = builder.ins().icmp(IntCC::SignedGreaterThanOrEqual, sl, c64);
                let vsr = builder.ins().ushr(bv, sr);
                let vsr = builder.ins().select(sr_big, zero, vsr);
                let vsl = builder.ins().ishl(vsr, sl);
                let v2 = builder.ins().select(sl_big, zero, vsl);
                let xsr = builder.ins().ushr(bx, sr);
                let xsr = builder.ins().select(sr_big, zero, xsr);
                let xsl = builder.ins().ishl(xsr, sl);
                let x2 = builder.ins().select(sl_big, zero, xsl);
                // Result-width mask: resw = msb - lsb + 1 (>= 1).
                let one = builder.ins().iconst(types::I64, 1);
                let diff = builder.ins().isub(msb, lsb);
                let resw = builder.ins().iadd(diff, one);
                let inv = builder.ins().isub(c64, resw);
                let resm0 = builder.ins().ushr(ones, inv);
                let resw_big = builder.ins().icmp(IntCC::SignedGreaterThanOrEqual, resw, c64);
                let resm = builder.ins().select(resw_big, ones, resm0);
                // Out-of-range positions read x. Low side: result indices
                // below -lsb (only when lsb < 0). High side: indices at or
                // above w - lsb.
                let lo_cnt0 = builder.ins().select(neg, nlsb, zero);
                let lo_cnt_big = builder.ins().icmp(IntCC::SignedGreaterThanOrEqual, lo_cnt0, c64);
                let lo_inv = builder.ins().isub(c64, lo_cnt0);
                let lo_m0 = builder.ins().ushr(ones, lo_inv);
                let lo_zero = builder.ins().icmp(IntCC::Equal, lo_cnt0, zero);
                let lo_m1 = builder.ins().select(lo_zero, zero, lo_m0);
                let xmask_lo = builder.ins().select(lo_cnt_big, ones, lo_m1);
                let wv = builder.ins().iconst(types::I64, w as i64);
                let hi_start0 = builder.ins().isub(wv, lsb);
                let hs_neg = builder.ins().icmp(IntCC::SignedLessThan, hi_start0, zero);
                let hi_start = builder.ins().select(hs_neg, zero, hi_start0);
                let hs_big = builder.ins().icmp(IntCC::SignedGreaterThanOrEqual, hi_start, c64);
                let hi_m0 = builder.ins().ishl(ones, hi_start);
                let xmask_hi = builder.ins().select(hs_big, zero, hi_m0);
                let oor = builder.ins().bor(xmask_lo, xmask_hi);
                let noor = builder.ins().bnot(oor);
                let vkeep = builder.ins().band(v2, noor);
                let out_v = builder.ins().band(vkeep, resm);
                let xall = builder.ins().bor(x2, oor);
                let out_x = builder.ins().band(xall, resm);
                st2(builder, pointer_type, regs, xz, *dest, out_v, out_x);
            }
            RangeSelectConst(dest, base, l_imm, r_imm) => {
                let w = reg_w.get(*base as usize).copied().unwrap_or(0);
                if w == 0 {
                    return Err(());
                }
                let lsb = (*l_imm).min(*r_imm);
                let msb = (*l_imm).max(*r_imm);
                let resw = msb - lsb + 1;
                if resw >= 64 {
                    return Err(());
                }
                let resm: u64 = (1u64 << resw) - 1;
                // Compile-time OOR mask (bounds are unsigned here — the
                // negative-low form arrives as dynamic RangeSelect).
                let oor: u64 = if lsb >= w {
                    resm
                } else if msb >= w {
                    (resm >> (w - lsb)) << (w - lsb)
                } else {
                    0
                };
                let (bv, bx) = ld2(builder, pointer_type, regs, xz, *base);
                let vs = builder.ins().ushr_imm(bv, lsb as i64);
                let xs = builder.ins().ushr_imm(bx, lsb as i64);
                let keep = builder.ins().iconst(types::I64, (resm & !oor) as i64);
                let out_v = builder.ins().band(vs, keep);
                let xm = builder.ins().band(xs, keep);
                let oc = builder.ins().iconst(types::I64, oor as i64);
                let out_x = builder.ins().bor(xm, oc);
                st2(builder, pointer_type, regs, xz, *dest, out_v, out_x);
            }
            // ArrayOperand is no longer a raw C string. Passing its String
            // buffer to xezim_jit_load_array_elem (which expects a CStr) is
            // both unterminated and loses the dense-array metadata. Keep
            // dynamic array blocks on the interpreter until the JIT bridge
            // accepts a resolved array descriptor.
            // Dense arrays only: the element signal id is arithmetic on
            // (first_id, lo, hi), so no name lookup is needed. Mirrors the
            // interpreter exactly: an X/Z index reads as index 0
            // (`to_u64().unwrap_or(0)`), and an out-of-range index yields a
            // 1-bit X. Named arrays abort the block's compilation.
            LoadArrayElem(d, arr, idx_reg) => {
                let crate::compiler::bytecode::ArrayOperand::Dense {
                    first_id, lo, hi, ..
                } = arr.as_ref()
                else {
                    return Err(());
                };
                let (iv, ix) = ld2(builder, pointer_type, regs, xz, *idx_reg);
                let zero = builder.ins().iconst(types::I64, 0);
                // Interpreter parity: `to_u64()` zeroes X/Z bits PER BIT
                // (idx 4'bx01x reads element 2), it does not collapse the
                // whole index to zero.
                let nix = builder.ins().bnot(ix);
                let eff = builder.ins().band(iv, nix);
                let lo_c = builder.ins().iconst(types::I64, *lo);
                let hi_c = builder.ins().iconst(types::I64, *hi);
                let ge = builder.ins().icmp(IntCC::SignedGreaterThanOrEqual, eff, lo_c);
                let le = builder.ins().icmp(IntCC::SignedLessThanOrEqual, eff, hi_c);
                let inb = builder.ins().band(ge, le);
                let off = builder.ins().isub(eff, lo_c);
                let first_c = builder.ins().iconst(types::I64, *first_id as i64);
                let eid = builder.ins().iadd(first_c, off);
                let eid_safe = builder.ins().select(inb, eid, first_c);
                let (val, xzv) = match inline_storage {
                    Some((base_ptr, len)) if (*first_id as u64) < len as u64 => {
                        raw_sig_loads_dyn(builder, pointer_type, base_ptr, eid_safe)
                    }
                    _ => {
                        let eid32 = builder.ins().ireduce(types::I32, eid_safe);
                        let call = builder.ins().call(load_ref, &[sim_ptr, eid32]);
                        let val = builder.inst_results(call)[0];
                        let xcall = builder.ins().call(load_xz_ref, &[sim_ptr, eid32]);
                        (val, builder.inst_results(xcall)[0])
                    }
                };
                let minus1 = builder.ins().iconst(types::I64, -1);
                let inb_mask = builder.ins().select(inb, minus1, zero);
                let v = builder.ins().band(val, inb_mask);
                let one = builder.ins().iconst(types::I64, 1);
                let x = builder.ins().select(inb, xzv, one);
                st2(builder, pointer_type, regs, xz, *d, v, x);
            }
            // Dense-array blocking element store. Same index rules as the
            // load; an out-of-range (or unresolvable) index stores nowhere.
            BlockingAssignArray(arr, idx_reg, val_reg, width) => {
                let crate::compiler::bytecode::ArrayOperand::Dense {
                    first_id, lo, hi, ..
                } = arr.as_ref()
                else {
                    return Err(());
                };
                let (iv, ix) = ld2(builder, pointer_type, regs, xz, *idx_reg);
                let (vv, vx) = ld2(builder, pointer_type, regs, xz, *val_reg);
                // Same per-bit X/Z zeroing as the load (to_u64 parity).
                let nix = builder.ins().bnot(ix);
                let eff = builder.ins().band(iv, nix);
                let lo_c = builder.ins().iconst(types::I64, *lo);
                let hi_c = builder.ins().iconst(types::I64, *hi);
                let ge = builder.ins().icmp(IntCC::SignedGreaterThanOrEqual, eff, lo_c);
                let le = builder.ins().icmp(IntCC::SignedLessThanOrEqual, eff, hi_c);
                let inb = builder.ins().band(ge, le);
                let store_b = builder.create_block();
                let join_b = builder.create_block();
                builder.ins().brif(inb, store_b, &[], join_b, &[]);
                builder.switch_to_block(store_b);
                let off = builder.ins().isub(eff, lo_c);
                let first_c = builder.ins().iconst(types::I64, *first_id as i64);
                let eid = builder.ins().iadd(first_c, off);
                let eid32 = builder.ins().ireduce(types::I32, eid);
                let wc = builder.ins().iconst(types::I32, *width as i64);
                builder
                    .ins()
                    .call(store_4s_ref, &[sim_ptr, eid32, vv, vx, wc]);
                builder.ins().jump(join_b, &[]);
                builder.switch_to_block(join_b);
            }
            _ => return Err(()),
        }
        Ok(())
    }

    /// §4.1.6/§11.4.5: a relational or equality operator with ANY unknown bit
    /// in either operand yields 1'bx, not a 0/1 answer.
    fn emit_cmp(
        builder: &mut FunctionBuilder,
        pointer_type: Type,
        regs: &[StackSlot],
        xz: &[StackSlot],
        reg_w: &[u32],
        reg_s: &[bool],
        d: u16,
        l: u16,
        r: u16,
        cc: IntCC,
    ) {
        // §11.4.4: a relational compares SIGNED only when BOTH operands are
        // signed — then each operand's value is its sign-extended form.
        let both_signed = reg_s.get(l as usize).copied().unwrap_or(false)
            && reg_s.get(r as usize).copied().unwrap_or(false);
        let cc = if both_signed {
            match cc {
                IntCC::UnsignedLessThan => IntCC::SignedLessThan,
                IntCC::UnsignedLessThanOrEqual => IntCC::SignedLessThanOrEqual,
                IntCC::UnsignedGreaterThan => IntCC::SignedGreaterThan,
                IntCC::UnsignedGreaterThanOrEqual => IntCC::SignedGreaterThanOrEqual,
                other => other,
            }
        } else {
            cc
        };
        let mut lv = builder.ins().stack_load(pointer_type, types::I64, regs[l as usize], 0);
        let mut rv = builder.ins().stack_load(pointer_type, types::I64, regs[r as usize], 0);
        let mut lx = builder.ins().stack_load(pointer_type, types::I64, xz[l as usize], 0);
        let mut rx = builder.ins().stack_load(pointer_type, types::I64, xz[r as usize], 0);
        if both_signed {
            let lw = reg_w.get(l as usize).copied().unwrap_or(0);
            let rw = reg_w.get(r as usize).copied().unwrap_or(0);
            // Extend BOTH planes: the known-bit mask below must see the x
            // plane in the same (sign-extended) bit positions as the values.
            if lw > 0 && lw < 64 {
                let (ve, xe) = sext_planes(builder, lv, lx, lw);
                lv = ve;
                lx = xe;
            }
            if rw > 0 && rw < 64 {
                let (ve, xe) = sext_planes(builder, rv, rx, rw);
                rv = ve;
                rx = xe;
            }
        }
        let cmp = builder.ins().icmp(cc, lv, rv);
        // Cranelift icmp returns an I8 (boolean). Extend to I64 for
        // storage; Verilog relational ops produce a 1-bit value where
        // 0 = false, 1 = true.
        let ext = builder.ins().uextend(types::I64, cmp);
        let anyx = builder.ins().bor(lx, rx);
        let zero = builder.ins().iconst(types::I64, 0);
        let one = builder.ins().iconst(types::I64, 1);
        let unknown = builder.ins().icmp(IntCC::NotEqual, anyx, zero);
        let out_v = builder.ins().select(unknown, zero, ext);
        let out_x = builder.ins().select(unknown, one, zero);
        // §11.4.5: for == / != a bit that is KNOWN in both operands and
        // differs decides the result even when other bits are X/Z —
        // `8'b1010_1x10 == 8'h6B` is a definite 0, not x. (Relational
        // < / <= / > / >= stay whole-X: ordering can flip on an unknown.)
        let (out_v, out_x) = if matches!(cc, IntCC::Equal | IntCC::NotEqual) {
            let known = builder.ins().bnot(anyx);
            let diffv = builder.ins().bxor(lv, rv);
            let diff_known = builder.ins().band(diffv, known);
            let decided = builder.ins().icmp(IntCC::NotEqual, diff_known, zero);
            let dv = if matches!(cc, IntCC::NotEqual) { one } else { zero };
            (
                builder.ins().select(decided, dv, out_v),
                builder.ins().select(decided, zero, out_x),
            )
        } else {
            (out_v, out_x)
        };
        builder.ins().stack_store(pointer_type, out_v, regs[d as usize], 0);
        builder.ins().stack_store(pointer_type, out_x, xz[d as usize], 0);
    }

    /// Static width propagation for the masking pass in `codegen_block`.
    ///
    /// The interpreter's `Value` carries its width intrinsically, so `~a` on a
    /// 1-bit operand IS 1 bit. A JIT register is a bare u64 pair — after
    /// `BitNot`, the 63 bits above the operand's width are garbage ones, and
    /// the bytecode does NOT always follow with a `Resize` (the interpreter
    /// never needed one). `if (~rst_b)` then read those garbage bits as TRUE
    /// and a c910-shaped FSM held itself in reset forever. Returns
    /// `(dest reg, result width)` for the ops whose result can EXCEED the
    /// width the interpreter would have kept — the codegen masks both planes
    /// after emitting them. Width 0 = unknown; no mask.
    pub fn insn_result_width(insn: &Insn, reg_w: &[u32], sig_w: &[u32]) -> Option<(u16, u32)> {
        use Insn::*;
        let rw = |r: &u16| reg_w.get(*r as usize).copied().unwrap_or(0);
        let need_mask = |d: u16, w: u32| if w >= 1 && w < 64 { Some((d, w)) } else { None };
        match insn {
            BitNot(d, s) | Negate(d, s) => need_mask(*d, rw(s)),
            BitXnor(d, l, r) | Add(d, l, r) | Sub(d, l, r) | Mul(d, l, r) => {
                let (lw, rw_) = (rw(l), rw(r));
                if lw == 0 || rw_ == 0 {
                    None
                } else {
                    need_mask(*d, lw.max(rw_))
                }
            }
            Shl(d, l, _) | AShr(d, l, _) => need_mask(*d, rw(l)),
            BinOpConst(d, sr, k, kind) => {
                use crate::compiler::bytecode::BinOpConstKind as K;
                match kind {
                    K::Add => {
                        let sw = rw(sr);
                        if sw == 0 || k.width == 0 {
                            None
                        } else {
                            need_mask(*d, sw.max(k.width))
                        }
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Track the width each register holds, in program order. Regs are
    /// allocated fresh per value by the bytecode compiler, so a linear walk is
    /// sound; anything not understood records width 0 (= unknown, no mask).
    pub fn update_reg_meta(
        insn: &Insn,
        reg_w: &mut [u32],
        reg_s: &mut [bool],
        sig_w: &[u32],
        sig_signed: &[bool],
    ) {
        use Insn::*;
        // Signedness propagation mirrors §11.8.1: an operation is signed only
        // when every operand is; loads take the signal's declared signedness;
        // `SetSigned` marks a register explicitly (parameters, casts).
        match insn {
            LoadConst(d, v) => reg_s[*d as usize] = v.is_signed,
            LoadSignal(d, _) => reg_s[*d as usize] = false,
            LoadSignalSigned(d, sid) => {
                reg_s[*d as usize] = sig_signed.get(*sid as usize).copied().unwrap_or(true)
            }
            SetSigned(r) => reg_s[*r as usize] = true,
            ClearSigned(r) => reg_s[*r as usize] = false,
            Concat(d, _) => reg_s[*d as usize] = false,
            Replicate(d, ..) | CaseLut(d, ..) => reg_s[*d as usize] = false,
            MoveResize(d, sr, _) => reg_s[*d as usize] = reg_s[*sr as usize],
            CmpBranch(_, _, _, tmp, _) => reg_s[*tmp as usize] = false,
            BinOpConst(d, sr, k, kind) => {
                use crate::compiler::bytecode::BinOpConstKind as K;
                reg_s[*d as usize] = match kind {
                    K::Add | K::Xor => reg_s[*sr as usize] && k.is_signed,
                    K::Eq | K::CaseEq => false,
                }
            }
            LoadArrayElem(d, arr, _) => {
                reg_s[*d as usize] = match arr.as_ref() {
                    crate::compiler::bytecode::ArrayOperand::Dense { first_id, .. } => {
                        sig_signed.get(*first_id).copied().unwrap_or(false)
                    }
                    _ => false,
                }
            }
            Move(d, s2) => reg_s[*d as usize] = reg_s[*s2 as usize],
            BitNot(d, s2) | Negate(d, s2) => reg_s[*d as usize] = reg_s[*s2 as usize],
            Add(d, l, r) | Sub(d, l, r) | Mul(d, l, r) | BitAnd(d, l, r) | BitOr(d, l, r)
            | BitXor(d, l, r) | BitXnor(d, l, r) => {
                reg_s[*d as usize] = reg_s[*l as usize] && reg_s[*r as usize]
            }
            Shl(d, l, _) | Shr(d, l, _) | AShr(d, l, _) => {
                reg_s[*d as usize] = reg_s[*l as usize]
            }
            Eq(d, ..) | Neq(d, ..) | CaseEq(d, ..) | CasezEq(d, ..) | CasexEq(d, ..)
            | Lt(d, ..) | Leq(d, ..) | Gt(d, ..) | Geq(d, ..) | LogAnd(d, ..)
            | LogOr(d, ..) | LogNot(d, _) | ReduceAnd(d, _) | ReduceOr(d, _)
            | ReduceXor(d, _) | BitSelect(d, ..) | BitSelectConst(d, ..) => {
                reg_s[*d as usize] = false
            }
            Select(d, _, t, e) => {
                reg_s[*d as usize] = reg_s[*t as usize] && reg_s[*e as usize]
            }
            _ => {}
        }
        update_reg_width_only(insn, reg_w, sig_w);
    }

    pub fn update_reg_width_only(insn: &Insn, reg_w: &mut [u32], sig_w: &[u32]) {
        use Insn::*;
        let set = |reg_w: &mut [u32], d: &u16, w: u32| {
            if let Some(slot) = reg_w.get_mut(*d as usize) {
                *slot = w;
            }
        };
        let get = |reg_w: &[u32], r: &u16| reg_w.get(*r as usize).copied().unwrap_or(0);
        match insn {
            LoadConst(d, v) => set(reg_w, d, v.width),
            LoadSignal(d, sid) | LoadSignalSigned(d, sid) => {
                set(reg_w, d, sig_w.get(*sid as usize).copied().unwrap_or(0))
            }
            LoadSignalBit(d, _, _) => set(reg_w, d, 1),
            LoadSignalRange(d, _, hi, lo) => set(reg_w, d, hi.abs_diff(*lo) + 1),
            // Same shape as `LoadSignalRange` (the unfused form of it). Needed
            // for parity with the bytecode compiler's `elide_redundant_resizes`
            // pass: a `Resize` it removes because it proved the width leaves
            // this table to derive that width by the same rules, and only these
            // two and `Select` were missing.
            RangeSelectConst(d, _, hi, lo) => set(reg_w, d, hi.abs_diff(*lo) + 1),
            // Both `?:` outcomes and the X-condition `merge_unknown` are
            // `max(then, else)` wide; unknown unless the two branches agree.
            Select(d, _, t, e) => {
                let (tw, ew) = (get(reg_w, t), get(reg_w, e));
                set(reg_w, d, if tw == ew { tw } else { 0 })
            }
            Move(d, s) => {
                let w = get(reg_w, s);
                set(reg_w, d, w)
            }
            LoadArrayElem(d, arr, _) => {
                let w = match arr.as_ref() {
                    crate::compiler::bytecode::ArrayOperand::Dense { first_id, .. } => {
                        sig_w.get(*first_id).copied().unwrap_or(0)
                    }
                    _ => 0,
                };
                set(reg_w, d, w)
            }
            Concat(d, parts) => {
                let mut total = 0u32;
                let mut known = true;
                for p in parts.iter() {
                    let w = get(reg_w, p);
                    if w == 0 {
                        known = false;
                        break;
                    }
                    total += w;
                }
                set(reg_w, d, if known && total <= 64 { total } else { 0 })
            }
            Resize(d, w) => set(reg_w, d, *w),
            MoveResize(d, _, w) => set(reg_w, d, *w),
            CmpBranch(_, _, _, tmp, _) => set(reg_w, tmp, 1),
            Replicate(d, sr, cnt) => {
                let sw = get(reg_w, sr);
                let total = (sw as u64) * (*cnt as u64);
                set(reg_w, d, if sw != 0 && total <= 64 { total as u32 } else { 0 })
            }
            CaseLut(d, _, lut) => {
                let w0 = lut.default.width;
                let uniform = lut.table.iter().all(|e| e.width == w0);
                set(reg_w, d, if uniform && w0 <= 64 { w0 } else { 0 })
            }
            BinOpConst(d, sr, k, kind) => {
                use crate::compiler::bytecode::BinOpConstKind as K;
                match kind {
                    K::Eq | K::CaseEq => set(reg_w, d, 1),
                    K::Add | K::Xor => {
                        let sw = get(reg_w, sr);
                        set(reg_w, d, if sw == 0 || k.width == 0 { 0 } else { sw.max(k.width) })
                    }
                }
            }
            BitNot(d, s) | Negate(d, s) => {
                let w = get(reg_w, s);
                set(reg_w, d, w)
            }
            BitAnd(d, l, r) | BitOr(d, l, r) | BitXor(d, l, r) | BitXnor(d, l, r)
            | Add(d, l, r) | Sub(d, l, r) | Mul(d, l, r) => {
                let (lw, rw) = (get(reg_w, l), get(reg_w, r));
                set(reg_w, d, if lw == 0 || rw == 0 { 0 } else { lw.max(rw) })
            }
            Shl(d, l, _) | Shr(d, l, _) | AShr(d, l, _) => {
                let w = get(reg_w, l);
                set(reg_w, d, w)
            }
            Eq(d, ..) | Neq(d, ..) | CaseEq(d, ..) | CasezEq(d, ..) | CasexEq(d, ..)
            | Lt(d, ..) | Leq(d, ..) | Gt(d, ..) | Geq(d, ..) | LogAnd(d, ..)
            | LogOr(d, ..) | LogNot(d, _) | ReduceAnd(d, _) | ReduceOr(d, _)
            | ReduceXor(d, _) => set(reg_w, d, 1),
            BitSelect(d, ..) | BitSelectConst(d, ..) => set(reg_w, d, 1),
            _ => {}
        }
    }

    fn mask_of(w: u32) -> u64 {
        if w >= 64 { !0 } else { (1u64 << w) - 1 }
    }

    /// Sign-extend both planes of a (v, x) pair from bit `w-1` to 64 bits.
    ///
    /// §11.8.1/§11.3.3: a SIGNED operand fills its context with copies of its
    /// sign bit. A JIT register is a zero-extended u64, so `-1` stored at
    /// width 32 is 0xFFFF_FFFF — multiplying two of those gives
    /// 0xFFFF_FFFE_0000_0001, not 1, and `>>> `shifts in zeros. The xz plane
    /// extends the same way: an X sign bit must replicate as X.
    fn sext_planes(
        builder: &mut FunctionBuilder,
        v: Value,
        x: Value,
        w: u32,
    ) -> (Value, Value) {
        if w == 0 || w >= 64 {
            return (v, x);
        }
        let sh = builder.ins().iconst(types::I64, (64 - w) as i64);
        let vs = builder.ins().ishl(v, sh);
        let ve = builder.ins().sshr(vs, sh);
        let xs = builder.ins().ishl(x, sh);
        let xe = builder.ins().sshr(xs, sh);
        (ve, xe)
    }

    /// §11.4.7 truthiness of a 4-state word: `(definitely_true,
    /// definitely_false)`. True iff some bit is a known 1; false iff every bit
    /// is a known 0. Neither holds when the only set bits are unknown.
    fn truthiness(builder: &mut FunctionBuilder, v: Value, x: Value) -> (Value, Value) {
        let zero = builder.ins().iconst(types::I64, 0);
        let nx = builder.ins().bnot(x);
        let known_ones = builder.ins().band(v, nx);
        let t = builder.ins().icmp(IntCC::NotEqual, known_ones, zero);
        let any = builder.ins().bor(v, x);
        let f = builder.ins().icmp(IntCC::Equal, any, zero);
        (t, f)
    }

    /// Two-plane load straight from the always-maintained SoA planes
    /// (`signal_inline_bits`): one 16-byte stride entry per signal, val at
    /// +0, xz at +8. Returns None when the id is outside the mirror (the
    /// caller keeps its FFI path).
    fn raw_sig_loads(
        builder: &mut FunctionBuilder,
        pointer_type: Type,
        inline_storage: Option<(u64, u32)>,
        sig_id: u32,
    ) -> Option<(Value, Value)> {
        let (base_ptr, len) = inline_storage?;
        if sig_id >= len {
            return None;
        }
        let base = builder.ins().iconst(pointer_type, base_ptr as i64);
        let off = (sig_id as i32) * 16;
        let v = builder.ins().load(types::I64, MemFlagsData::trusted(), base, off);
        let x = builder.ins().load(types::I64, MemFlagsData::trusted(), base, off + 8);
        Some((v, x))
    }

    /// Runtime-indexed two-plane load: `entry = base + eid*16`. The caller
    /// guarantees `eid` is a valid signal id (array spans are contiguous
    /// valid ids, bounds-checked before the id is formed).
    fn raw_sig_loads_dyn(
        builder: &mut FunctionBuilder,
        pointer_type: Type,
        base_ptr: u64,
        eid: Value,
    ) -> (Value, Value) {
        let base = builder.ins().iconst(pointer_type, base_ptr as i64);
        let sixteen = builder.ins().iconst(types::I64, 16);
        let off = builder.ins().imul(eid, sixteen);
        let addr = builder.ins().iadd(base, off);
        let v = builder.ins().load(types::I64, MemFlagsData::trusted(), addr, 0);
        let x = builder.ins().load(types::I64, MemFlagsData::trusted(), addr, 8);
        (v, x)
    }

    /// Load both planes of a VM register.
    fn ld2(
        builder: &mut FunctionBuilder,
        pointer_type: Type,
        regs: &[StackSlot],
        xz: &[StackSlot],
        r: u16,
    ) -> (Value, Value) {
        (
            builder.ins().stack_load(pointer_type, types::I64, regs[r as usize], 0),
            builder.ins().stack_load(pointer_type, types::I64, xz[r as usize], 0),
        )
    }

    /// Store both planes of a VM register.
    fn st2(
        builder: &mut FunctionBuilder,
        pointer_type: Type,
        regs: &[StackSlot],
        xz: &[StackSlot],
        d: u16,
        v: Value,
        x: Value,
    ) {
        builder.ins().stack_store(pointer_type, v, regs[d as usize], 0);
        builder.ins().stack_store(pointer_type, x, xz[d as usize], 0);
    }

    /// §11.4.10: a shift by a KNOWN amount shifts both planes — `4'bxxxx << 1`
    /// is `xxx0`, because the bit shifted in is a known 0, not an unknown. Only
    /// an unknown shift AMOUNT makes the whole result unknown.
    fn emit_shift(
        builder: &mut FunctionBuilder,
        pointer_type: Type,
        regs: &[StackSlot],
        xz: &[StackSlot],
        d: u16,
        l: u16,
        r: u16,
        op: impl Fn(&mut FunctionBuilder, Value, Value) -> Value,
    ) {
        let (lv, lx) = ld2(builder, pointer_type, regs, xz, l);
        let (rv, rx) = ld2(builder, pointer_type, regs, xz, r);
        let sv = op(builder, lv, rv);
        let sx = op(builder, lx, rv);
        let zero = builder.ins().iconst(types::I64, 0);
        let ones = builder.ins().iconst(types::I64, -1);
        let amt_unknown = builder.ins().icmp(IntCC::NotEqual, rx, zero);
        let out_v = builder.ins().select(amt_unknown, zero, sv);
        let out_x = builder.ins().select(amt_unknown, ones, sx);
        st2(builder, pointer_type, regs, xz, d, out_v, out_x);
    }

    /// §11.4.3 arithmetic / §11.4.10 shifts: any unknown bit in an operand
    /// makes the WHOLE result unknown. Emits `v = unk ? 0 : f(a,b)` and
    /// `x = unk ? ~0 : 0`; the destination width mask is applied by the
    /// following `Resize` or by the store bridge.
    fn emit_binop_arith(
        builder: &mut FunctionBuilder,
        pointer_type: Type,
        regs: &[StackSlot],
        xz: &[StackSlot],
        d: u16,
        l: u16,
        r: u16,
        op: impl FnOnce(&mut FunctionBuilder, Value, Value) -> Value,
    ) {
        let (lv, lx) = ld2(builder, pointer_type, regs, xz, l);
        let (rv, rx) = ld2(builder, pointer_type, regs, xz, r);
        let result = op(builder, lv, rv);
        let anyx = builder.ins().bor(lx, rx);
        let zero = builder.ins().iconst(types::I64, 0);
        let ones = builder.ins().iconst(types::I64, -1);
        let unknown = builder.ins().icmp(IntCC::NotEqual, anyx, zero);
        let out_v = builder.ins().select(unknown, zero, result);
        let out_x = builder.ins().select(unknown, ones, zero);
        st2(builder, pointer_type, regs, xz, d, out_v, out_x);
    }

    fn emit_binop(
        builder: &mut FunctionBuilder,
        pointer_type: Type,
        regs: &[StackSlot],
        d: u16,
        l: u16,
        r: u16,
        op: impl FnOnce(&mut FunctionBuilder, Value, Value) -> Value,
    ) {
        let lv = builder.ins().stack_load(pointer_type, types::I64, regs[l as usize], 0);
        let rv = builder.ins().stack_load(pointer_type, types::I64, regs[r as usize], 0);
        let result = op(builder, lv, rv);
        builder.ins().stack_store(pointer_type, result, regs[d as usize], 0);
    }

    /// Diagnostic: name of the first insn `is_supported` rejects, for the
    /// XEZIM_COMB_PATHS coverage report. None = whole stream compilable.
    pub fn first_unsupported(insns: &[Insn]) -> Option<&'static str> {
        insns
            .iter()
            .find(|i| !is_supported(i))
            .map(crate::compiler::bytecode::insn_opcode_name)
    }

    /// Allowlist: MVP coverage. Keep in sync with `emit_insn` +
    /// the CFG-construction code in `codegen_block`.
    fn is_supported(insn: &Insn) -> bool {
        use Insn::*;
        // XEZIM_JIT_DENY=Op1,Op2 — runtime opt-out per opcode name
        // (as printed by `insn_opcode_name`), for bisecting a suspect
        // codegen arm without a rebuild.
        static DENY: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
        let deny = DENY.get_or_init(|| {
            std::env::var("XEZIM_JIT_DENY")
                .map(|v| v.split(',').map(|x| x.trim().to_string()).collect())
                .unwrap_or_default()
        });
        if !deny.is_empty()
            && deny
                .iter()
                .any(|d| d == crate::compiler::bytecode::insn_opcode_name(insn))
        {
            return false;
        }
        matches!(
            insn,
            LoadConst(..)
                | LoadSignal(..)
                | LoadSignalSigned(..)
                | Move(..)
                | BlockingAssign(..)
                | NbaAssign(..)
                | NbaAssignRange(..)
                | Add(..)
                | Sub(..)
                | Mul(..)
                | BitAnd(..)
                | BitOr(..)
                | BitXor(..)
                | BitXnor(..)
                | BitNot(..)
                | LogAnd(..)
                | LogOr(..)
                | LogNot(..)
                | Negate(..)
                | Eq(..)
                | Neq(..)
                | CaseEq(..)
                | Lt(..)
                | Leq(..)
                | Gt(..)
                | Geq(..)
                | Shl(..)
                | Shr(..)
                | AShr(..)
                | ReduceOr(..)
                | ReduceXor(..)
                | SetSigned(..)
                | Resize(..)
                | BitSelect(..)
                | BitSelectConst(..)
                | RangeSelect(..)
                | RangeSelectConst(..)
                | LoadArrayElem(..)
                | BranchIfFalse(..)
                | Jump(..)
                | NbaAssignBitDyn(..)
                | NbaAssignRangeDyn(..)
                | BlockingAssignRangeDyn(..)
                | BlockingAssignRange(..)
                | Select(..)
                | BlockingAssignBitDyn(..)
                | LoadSignalBit(..)
                | LoadSignalRange(..)
                | ClearSigned(..)
                | Concat(..)
                | BlockingAssignArray(..)
                | BinOpConst(..)
                | NbaAssignConst(..)
                | NbaAssignArray(..)
                | StmtFallback(..)
                | NbaAssignArrayRead(..)
                | CmpBranch(..)
                | MoveResize(..)
                | Replicate(..)
                | CaseLut(..)
                | ReduceAnd(..)
                | BranchUnlessZero(..)
                | BranchIfSignalFalse(..)
                | CaseJump(..)
                | Nop
        )
    }
}
