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
#[no_mangle]
pub unsafe extern "C" fn xezim_jit_load_signal(sim: *mut u8, id: u32) -> u64 {
    let sim = &mut *(sim as *mut crate::compiler::simulator::Simulator);
    sim.jit_load_signal(id as usize)
}

/// Write `signal_table[id] = val_bits` (width-masked) with full
/// dirty-tracking and mark_dirty_id behavior — i.e. matches
/// `Insn::BlockingAssign` semantics. Returns nothing; caller trusts
/// the bridge to propagate correctly.
#[no_mangle]
pub unsafe extern "C" fn xezim_jit_store_signal(sim: *mut u8, id: u32, val_bits: u64, width: u32) {
    let sim = &mut *(sim as *mut crate::compiler::simulator::Simulator);
    sim.jit_store_signal(id as usize, val_bits, width);
}

/// Schedule a non-blocking assign: push `(signal_id, value)` to
/// `nba_fast` so the next `apply_nba` pass writes `signal_table[id]`.
/// Mirrors `Insn::NbaAssign` semantics.
#[no_mangle]
pub unsafe extern "C" fn xezim_jit_schedule_nba(sim: *mut u8, id: u32, val_bits: u64, width: u32) {
    let sim = &mut *(sim as *mut crate::compiler::simulator::Simulator);
    sim.jit_schedule_nba(id as usize, val_bits, width);
}

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
#[no_mangle]
pub unsafe extern "C" fn xezim_jit_schedule_nba_fast(
    sim: *mut u8,
    id: u32,
    val_bits: u64,
) {
    let sim = &mut *(sim as *mut crate::compiler::simulator::Simulator);
    sim.jit_schedule_nba_fast(id as usize, val_bits);
}

/// Schedule a non-blocking assign to a dynamic bit-range: merges `val_bits`
/// at bits `[hi_bits:lo_bits]` into the current signal value.
#[no_mangle]
pub unsafe extern "C" fn xezim_jit_schedule_nba_range_dyn(
    sim: *mut u8,
    id: u32,
    hi_bits: u64,
    lo_bits: u64,
    val_bits: u64,
) {
    let sim = &mut *(sim as *mut crate::compiler::simulator::Simulator);
    sim.jit_schedule_nba_range(id as usize, hi_bits as u32, lo_bits as u32, val_bits);
}

/// Schedule a non-blocking assign to a dynamic bit-index.
#[no_mangle]
pub unsafe extern "C" fn xezim_jit_schedule_nba_bit_dyn(
    sim: *mut u8,
    id: u32,
    idx: u64,
    val_bits: u64,
) {
    let sim = &mut *(sim as *mut crate::compiler::simulator::Simulator);
    sim.jit_schedule_nba_bit(id as usize, idx as usize, val_bits != 0);
}

/// Perform a blocking assign to a dynamic bit-range.
#[no_mangle]
pub unsafe extern "C" fn xezim_jit_blocking_assign_range_dyn(
    sim: *mut u8,
    id: u32,
    hi_bits: u64,
    lo_bits: u64,
    val_bits: u64,
) {
    let sim = &mut *(sim as *mut crate::compiler::simulator::Simulator);
    sim.jit_blocking_assign_range(id as usize, hi_bits as u32, lo_bits as u32, val_bits);
}

/// Load an array element value as u64.
#[no_mangle]
pub unsafe extern "C" fn xezim_jit_load_array_elem(sim: *mut u8, name_ptr: *const u8, idx: i64) -> u64 {
    let sim = &mut *(sim as *mut crate::compiler::simulator::Simulator);
    let name = std::ffi::CStr::from_ptr(name_ptr as *const i8).to_string_lossy();
    sim.jit_load_array_elem(&name, idx)
}

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
#[no_mangle]
pub unsafe extern "C" fn xezim_jit_inputs_have_xz(
    sim: *mut u8,
    ids_ptr: *const u32,
    n: u32,
) -> u32 {
    let sim = &*(sim as *const crate::compiler::simulator::Simulator);
    let ids = std::slice::from_raw_parts(ids_ptr, n as usize);
    if sim.jit_inputs_have_xz(ids) {
        1
    } else {
        0
    }
}

/// Stubs when the feature is disabled — everything is None / no-op so
/// `exec_bytecode` always falls through to the interpreter.
#[cfg(not(feature = "jit"))]
mod stub {
    use super::super::bytecode::Insn;
    use super::JitFn;

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
        pub fn set_inline_bits_storage(&mut self, _ptr: u64, _len: u32) {}
        pub fn set_signal_widths(&mut self, _widths: Vec<u32>) {}
        pub fn set_nba_side_queue(&mut self, _base_ptr: u64, _len_ptr: u64, _cap: u32) {}
    }
}

#[cfg(feature = "jit")]
mod enabled {
    use super::super::bytecode::Insn;
    use super::{
        xezim_jit_blocking_assign_range_dyn, xezim_jit_inputs_have_xz,
        xezim_jit_load_array_elem, xezim_jit_load_signal, xezim_jit_schedule_nba,
        xezim_jit_schedule_nba_bit_dyn, xezim_jit_schedule_nba_fast,
        xezim_jit_schedule_nba_range_dyn, xezim_jit_store_signal, JitFn,
    };
    use cranelift::codegen::ir::{FuncRef, StackSlot};
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
        /// JIT Stage 4 Tier A: snapshot of `Simulator::signal_widths` used
        /// at JIT-compile time to pick between the slow nba bridge (with
        /// width arg) and the leaner nba_fast bridge (assumes
        /// width == signal_widths[sig_id]).  Set via
        /// `set_signal_widths` after JitModule construction.
        signal_widths_snapshot: Vec<u32>,
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
                signal_widths_snapshot: Vec::new(),
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
                    _ => None,
                };
                if let Some(sid) = id_opt {
                    if seen.insert(sid) {
                        input_ids.push(sid);
                    }
                }
            }
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

            // nba_range / blk_range bridge ABI:
            // (sim, id: u32, hi: u64, lo: u64, val_bits: u64).
            let mut nba_range_sig = self.module.make_signature();
            nba_range_sig.params.push(AbiParam::new(pointer_type));
            nba_range_sig.params.push(AbiParam::new(types::I32));
            nba_range_sig.params.push(AbiParam::new(types::I64));
            nba_range_sig.params.push(AbiParam::new(types::I64));
            nba_range_sig.params.push(AbiParam::new(types::I64));

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
                let target = match insn {
                    Insn::BranchIfFalse(_, t) | Insn::Jump(t) => Some(*t as usize),
                    _ => None,
                };
                if let Some(t) = target {
                    if t < n {
                        is_leader[t] = true;
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
            if std::env::var("XEZIM_JIT_SKIP_XZ").is_ok() {
                let _ = (xz_check_ref, fallback_block, &input_ids);
                builder.ins().jump(entry_block, &[prelude_sim_ptr]);
            } else if input_ids.is_empty() {
                // No reads — no X/Z risk. Fall straight through.
                builder.ins().jump(entry_block, &[prelude_sim_ptr]);
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
                        MemFlags::trusted(),
                        xz_base,
                        id as i32,
                    );
                    acc = builder.ins().bor(acc, byte);
                }
                builder.ins().brif(acc, fallback_block, &[], entry_block, &[prelude_sim_ptr]);
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
                    builder.ins().stack_store(id_val, id_slot, (i * 4) as i32);
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
                    .brif(xz_rc, fallback_block, &[], entry_block, &[prelude_sim_ptr]);
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
                        let cv = builder
                            .ins()
                            .stack_load(types::I64, reg_slots[*cond as usize], 0);
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
                    // JIT Stage 4 Tier C Path C1: when width matches AND
                    // the side queue is configured, emit a full inline
                    // write — load len, write (sig_id, val_bits) into
                    // queue[len], bump len.  No FFI call.  Saves ~25-30 ns
                    // per NBA vs the Tier A fast bridge.
                    Insn::NbaAssign(sig_id, val_reg, width)
                        if signal_widths
                            .get(*sig_id)
                            .map_or(false, |&w| w == *width)
                            && nba_side_queue.is_some() =>
                    {
                        let (base_ptr, len_ptr, _cap) = nba_side_queue.unwrap();
                        let v = builder
                            .ins()
                            .stack_load(types::I64, reg_slots[*val_reg as usize], 0);
                        // Load current length (u32) from *len_ptr.
                        let len_addr = builder.ins().iconst(pointer_type, len_ptr as i64);
                        let len = builder.ins().load(
                            types::I32,
                            MemFlags::trusted(),
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
                            .store(MemFlags::trusted(), sid, slot, 0);
                        // Write val_bits (u64) at offset 8 (skip the 4-byte
                        // pad after signal_id).
                        builder
                            .ins()
                            .store(MemFlags::trusted(), v, slot, 8);
                        // Increment len.
                        let one = builder.ins().iconst(types::I32, 1);
                        let new_len = builder.ins().iadd(len, one);
                        builder
                            .ins()
                            .store(MemFlags::trusted(), new_len, len_addr, 0);
                    }
                    // JIT Stage 4 Tier A: when the Insn::NbaAssign's
                    // width matches the signal's declared width, emit
                    // a call to the leaner nba_fast bridge (3-arg, no
                    // width).  Otherwise fall through to the existing
                    // 4-arg bridge below.
                    Insn::NbaAssign(sig_id, val_reg, width)
                        if signal_widths
                            .get(*sig_id)
                            .map_or(false, |&w| w == *width) =>
                    {
                        let v = builder
                            .ins()
                            .stack_load(types::I64, reg_slots[*val_reg as usize], 0);
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
                        // Each [u64; 2] entry is 16 bytes; val_bits is at offset 0.
                        let offset = (*sig_id as i32) * 16;
                        let val = builder.ins().load(
                            types::I64,
                            MemFlags::trusted(),
                            base,
                            offset,
                        );
                        builder.ins().stack_store(val, reg_slots[*dest as usize], 0);
                    }
                    other => {
                        emit_insn(
                            &mut builder,
                            other,
                            sim_ptr,
                            &reg_slots,
                            load_ref,
                            store_ref,
                            nba_ref,
                            nba_range_ref,
                            nba_bit_ref,
                            blk_range_ref,
                            load_array_ref,
                        )?;
                    }
                }
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
            builder.finalize();

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
        load_ref: FuncRef,
        store_ref: FuncRef,
        nba_ref: FuncRef,
        nba_range_ref: FuncRef,
        nba_bit_ref: FuncRef,
        blk_range_ref: FuncRef,
        _load_array_ref: FuncRef,
    ) -> Result<(), ()> {
        use Insn::*;
        match insn {
            Nop => {}
            LoadConst(dest, v) => {
                let bits = v.to_u64().unwrap_or(0);
                let c = builder.ins().iconst(types::I64, bits as i64);
                builder.ins().stack_store(c, regs[*dest as usize], 0);
            }
            LoadSignal(dest, sig_id) | LoadSignalSigned(dest, sig_id) => {
                let id = builder.ins().iconst(types::I32, *sig_id as i64);
                let call = builder.ins().call(load_ref, &[sim_ptr, id]);
                let val = builder.inst_results(call)[0];
                builder.ins().stack_store(val, regs[*dest as usize], 0);
            }
            Move(d, s) => {
                let v = builder.ins().stack_load(types::I64, regs[*s as usize], 0);
                builder.ins().stack_store(v, regs[*d as usize], 0);
            }
            Add(d, l, r) => emit_binop(builder, regs, *d, *l, *r, |b, x, y| b.ins().iadd(x, y)),
            Sub(d, l, r) => emit_binop(builder, regs, *d, *l, *r, |b, x, y| b.ins().isub(x, y)),
            Mul(d, l, r) => emit_binop(builder, regs, *d, *l, *r, |b, x, y| b.ins().imul(x, y)),
            BitAnd(d, l, r) => emit_binop(builder, regs, *d, *l, *r, |b, x, y| b.ins().band(x, y)),
            BitOr(d, l, r) => emit_binop(builder, regs, *d, *l, *r, |b, x, y| b.ins().bor(x, y)),
            BitXor(d, l, r) => emit_binop(builder, regs, *d, *l, *r, |b, x, y| b.ins().bxor(x, y)),
            BitNot(d, s) => {
                let v = builder.ins().stack_load(types::I64, regs[*s as usize], 0);
                let neg = builder.ins().bnot(v);
                builder.ins().stack_store(neg, regs[*d as usize], 0);
            }
            Eq(d, l, r) => emit_cmp(builder, regs, *d, *l, *r, IntCC::Equal),
            Neq(d, l, r) => emit_cmp(builder, regs, *d, *l, *r, IntCC::NotEqual),
            Lt(d, l, r) => emit_cmp(builder, regs, *d, *l, *r, IntCC::UnsignedLessThan),
            Leq(d, l, r) => emit_cmp(builder, regs, *d, *l, *r, IntCC::UnsignedLessThanOrEqual),
            Gt(d, l, r) => emit_cmp(builder, regs, *d, *l, *r, IntCC::UnsignedGreaterThan),
            Geq(d, l, r) => emit_cmp(builder, regs, *d, *l, *r, IntCC::UnsignedGreaterThanOrEqual),
            Shl(d, l, r) => emit_binop(builder, regs, *d, *l, *r, |b, x, y| b.ins().ishl(x, y)),
            Shr(d, l, r) => emit_binop(builder, regs, *d, *l, *r, |b, x, y| b.ins().ushr(x, y)),
            AShr(d, l, r) => emit_binop(builder, regs, *d, *l, *r, |b, x, y| b.ins().sshr(x, y)),
            BitXnor(d, l, r) => emit_binop(builder, regs, *d, *l, *r, |b, x, y| {
                let xor = b.ins().bxor(x, y);
                b.ins().bnot(xor)
            }),
            LogAnd(d, l, r) => {
                // LogAnd: (l != 0) & (r != 0). Result is 1 or 0.
                let lv = builder.ins().stack_load(types::I64, regs[*l as usize], 0);
                let rv = builder.ins().stack_load(types::I64, regs[*r as usize], 0);
                let zero = builder.ins().iconst(types::I64, 0);
                let lb = builder.ins().icmp(IntCC::NotEqual, lv, zero);
                let rb = builder.ins().icmp(IntCC::NotEqual, rv, zero);
                let and = builder.ins().band(lb, rb);
                let ext = builder.ins().uextend(types::I64, and);
                builder.ins().stack_store(ext, regs[*d as usize], 0);
            }
            LogOr(d, l, r) => {
                let lv = builder.ins().stack_load(types::I64, regs[*l as usize], 0);
                let rv = builder.ins().stack_load(types::I64, regs[*r as usize], 0);
                let zero = builder.ins().iconst(types::I64, 0);
                let lb = builder.ins().icmp(IntCC::NotEqual, lv, zero);
                let rb = builder.ins().icmp(IntCC::NotEqual, rv, zero);
                let or = builder.ins().bor(lb, rb);
                let ext = builder.ins().uextend(types::I64, or);
                builder.ins().stack_store(ext, regs[*d as usize], 0);
            }
            LogNot(d, s) => {
                let v = builder.ins().stack_load(types::I64, regs[*s as usize], 0);
                let zero = builder.ins().iconst(types::I64, 0);
                let eq = builder.ins().icmp(IntCC::Equal, v, zero);
                let ext = builder.ins().uextend(types::I64, eq);
                builder.ins().stack_store(ext, regs[*d as usize], 0);
            }
            Negate(d, s) => {
                let v = builder.ins().stack_load(types::I64, regs[*s as usize], 0);
                let neg = builder.ins().ineg(v);
                builder.ins().stack_store(neg, regs[*d as usize], 0);
            }
            // ReduceAnd intentionally NOT JIT'd: requires width info to
            // compare val against the width-specific all-ones mask, and
            // the Insn doesn't carry width directly. Stays in is_supported
            // = false and routes through the interpreter.
            //
            // ReduceOr / ReduceXor assume X-free inputs (Path B pre-check
            // guarantees no X/Z when the JIT body executes), so width-
            // independent emissions on the u64 val_bits are correct.
            ReduceOr(d, s) => {
                // val != 0 → 1, else 0. Width-independent (any nonzero bit
                // means OR is 1).
                let v = builder.ins().stack_load(types::I64, regs[*s as usize], 0);
                let zero = builder.ins().iconst(types::I64, 0);
                let nonzero = builder.ins().icmp(IntCC::NotEqual, v, zero);
                let ext = builder.ins().uextend(types::I64, nonzero);
                builder.ins().stack_store(ext, regs[*d as usize], 0);
            }
            ReduceXor(d, s) => {
                // Parity: popcnt(val) & 1.
                let v = builder.ins().stack_load(types::I64, regs[*s as usize], 0);
                let pc = builder.ins().popcnt(v);
                let one = builder.ins().iconst(types::I64, 1);
                let parity = builder.ins().band(pc, one);
                builder.ins().stack_store(parity, regs[*d as usize], 0);
            }
            // Select intentionally NOT in is_supported. The naive
            // 2-state codegen below mismatched the interpreter's
            // 4-state semantics on picorv32 RTL (TRAP after 87,467
            // cycles instead of 520,326), even with Path B's X/Z
            // pre-check on inputs. Suspected cause: intermediate
            // values that pass through Move/RangeSelect can have
            // upper-bit garbage (the JIT u64 stack slots are full
            // 64-bit; the interpreter masks to width). Eq/Lt etc.
            // use icmp on the full u64, which can disagree with the
            // interpreter's width-masked compare. Re-enable once
            // either (a) Select is taught to mask its cond to width,
            // or (b) the JIT tracks per-register width and inserts
            // mask insns before width-sensitive ops.
            Select(d, cond, t, e) => {
                let cv = builder
                    .ins()
                    .stack_load(types::I64, regs[*cond as usize], 0);
                let tv = builder.ins().stack_load(types::I64, regs[*t as usize], 0);
                let ev = builder.ins().stack_load(types::I64, regs[*e as usize], 0);
                let zero = builder.ins().iconst(types::I64, 0);
                let cb = builder.ins().icmp(IntCC::NotEqual, cv, zero);
                let res = builder.ins().select(cb, tv, ev);
                builder.ins().stack_store(res, regs[*d as usize], 0);
                let _ = (d, cond, t, e);
            }
            CaseEq(d, l, r) => {
                // SV `===`: with Path B's X-free guarantee, equivalent to
                // ordinary Eq. (CaseEq differs from Eq only when X/Z is
                // present in either operand.)
                let lv = builder.ins().stack_load(types::I64, regs[*l as usize], 0);
                let rv = builder.ins().stack_load(types::I64, regs[*r as usize], 0);
                let eq = builder.ins().icmp(IntCC::Equal, lv, rv);
                let ext = builder.ins().uextend(types::I64, eq);
                builder.ins().stack_store(ext, regs[*d as usize], 0);
            }
            SetSigned(_) => {
                // No-op in 2-state JIT: signedness is a per-Value flag the
                // bytecode propagates, but the JIT operates on raw u64
                // val_bits. The interpreter / bridge re-applies signedness
                // when materialising results into signal_table.
            }
            BlockingAssign(sig_id, val_reg, width) => {
                let v = builder
                    .ins()
                    .stack_load(types::I64, regs[*val_reg as usize], 0);
                let id = builder.ins().iconst(types::I32, *sig_id as i64);
                let w = builder.ins().iconst(types::I32, *width as i64);
                builder.ins().call(store_ref, &[sim_ptr, id, v, w]);
            }
            BlockingAssignRangeDyn(sig_id, hi_reg, lo_reg, val_reg) => {
                let v = builder
                    .ins()
                    .stack_load(types::I64, regs[*val_reg as usize], 0);
                let id = builder.ins().iconst(types::I32, *sig_id as i64);
                let hi = builder
                    .ins()
                    .stack_load(types::I64, regs[*hi_reg as usize], 0);
                let lo = builder
                    .ins()
                    .stack_load(types::I64, regs[*lo_reg as usize], 0);
                builder
                    .ins()
                    .call(blk_range_ref, &[sim_ptr, id, hi, lo, v]);
            }
            NbaAssign(sig_id, val_reg, width) => {
                let v = builder
                    .ins()
                    .stack_load(types::I64, regs[*val_reg as usize], 0);
                let id = builder.ins().iconst(types::I32, *sig_id as i64);
                let w = builder.ins().iconst(types::I32, *width as i64);
                builder.ins().call(nba_ref, &[sim_ptr, id, v, w]);
            }
            NbaAssignRange(sig_id, hi, lo, val_reg) => {
                let v = builder
                    .ins()
                    .stack_load(types::I64, regs[*val_reg as usize], 0);
                let id = builder.ins().iconst(types::I32, *sig_id as i64);
                let hi_v = builder.ins().iconst(types::I64, *hi as i64);
                let lo_v = builder.ins().iconst(types::I64, *lo as i64);
                builder
                    .ins()
                    .call(nba_range_ref, &[sim_ptr, id, hi_v, lo_v, v]);
            }
            NbaAssignRangeDyn(sig_id, hi_reg, lo_reg, val_reg) => {
                let v = builder
                    .ins()
                    .stack_load(types::I64, regs[*val_reg as usize], 0);
                let id = builder.ins().iconst(types::I32, *sig_id as i64);
                let hi = builder
                    .ins()
                    .stack_load(types::I64, regs[*hi_reg as usize], 0);
                let lo = builder
                    .ins()
                    .stack_load(types::I64, regs[*lo_reg as usize], 0);
                builder
                    .ins()
                    .call(nba_range_ref, &[sim_ptr, id, hi, lo, v]);
            }
            NbaAssignBitDyn(sig_id, idx_reg, val_reg) => {
                let v = builder
                    .ins()
                    .stack_load(types::I64, regs[*val_reg as usize], 0);
                let id = builder.ins().iconst(types::I32, *sig_id as i64);
                let idx = builder
                    .ins()
                    .stack_load(types::I64, regs[*idx_reg as usize], 0);
                builder.ins().call(nba_bit_ref, &[sim_ptr, id, idx, v]);
            }
            // NbaAssignRangeDyn / NbaAssignBitDyn still left out — they
            // need dynamic hi/lo from VM regs, requiring extra value
            // shuffling. Tractable next step but not in this slice.
            Resize(reg, width) => {
                // Mask the value to the target width. Loads from stack,
                // applies mask, stores back. Emulates Value::resize for
                // the common narrowing case; widening is already zero-ext
                // since stack slots are u64.
                let v = builder.ins().stack_load(types::I64, regs[*reg as usize], 0);
                let mask: u64 = if *width >= 64 {
                    u64::MAX
                } else {
                    (1u64 << *width) - 1
                };
                let mc = builder.ins().iconst(types::I64, mask as i64);
                let masked = builder.ins().band(v, mc);
                builder.ins().stack_store(masked, regs[*reg as usize], 0);
            }
            BitSelect(dest, base, idx) => {
                // dest = (base >> idx) & 1
                let b = builder
                    .ins()
                    .stack_load(types::I64, regs[*base as usize], 0);
                let i = builder.ins().stack_load(types::I64, regs[*idx as usize], 0);
                let shifted = builder.ins().ushr(b, i);
                let one = builder.ins().iconst(types::I64, 1);
                let result = builder.ins().band(shifted, one);
                builder.ins().stack_store(result, regs[*dest as usize], 0);
            }
            BitSelectConst(dest, base, idx) => {
                let b = builder
                    .ins()
                    .stack_load(types::I64, regs[*base as usize], 0);
                let i = builder.ins().iconst(types::I64, *idx as i64);
                let shifted = builder.ins().ushr(b, i);
                let one = builder.ins().iconst(types::I64, 1);
                let result = builder.ins().band(shifted, one);
                builder.ins().stack_store(result, regs[*dest as usize], 0);
            }
            RangeSelect(dest, base, left_r, right_r) => {
                // dest = (base >> min(l,r)) & ((1 << (|l-r|+1)) - 1)
                // Computed via `(~0u64) >> (64 - width)` which safely
                // gives u64::MAX at width=64 thanks to x86/riscv's
                // shift-amount-masked-to-low-bits semantics (which is
                // also what cranelift's ushr lowers to).
                let b = builder
                    .ins()
                    .stack_load(types::I64, regs[*base as usize], 0);
                let l = builder
                    .ins()
                    .stack_load(types::I64, regs[*left_r as usize], 0);
                let r = builder
                    .ins()
                    .stack_load(types::I64, regs[*right_r as usize], 0);
                let le = builder.ins().icmp(IntCC::UnsignedLessThanOrEqual, l, r);
                let lsb = builder.ins().select(le, l, r);
                let msb = builder.ins().select(le, r, l);
                let shifted = builder.ins().ushr(b, lsb);
                let one = builder.ins().iconst(types::I64, 1);
                let diff = builder.ins().isub(msb, lsb);
                let width = builder.ins().iadd(diff, one);
                let sixty_four = builder.ins().iconst(types::I64, 64);
                let shift_amt = builder.ins().isub(sixty_four, width);
                let all_ones = builder.ins().iconst(types::I64, -1);
                let mask = builder.ins().ushr(all_ones, shift_amt);
                let result = builder.ins().band(shifted, mask);
                builder.ins().stack_store(result, regs[*dest as usize], 0);
            }
            RangeSelectConst(dest, base, l_imm, r_imm) => {
                let b = builder
                    .ins()
                    .stack_load(types::I64, regs[*base as usize], 0);
                let lsb = (*l_imm).min(*r_imm);
                let msb = (*l_imm).max(*r_imm);
                let width = msb - lsb + 1;
                let shifted = builder.ins().ushr_imm(b, lsb as i64);
                let mask: u64 = if width >= 64 {
                    u64::MAX
                } else {
                    (1u64 << width) - 1
                };
                let mc = builder.ins().iconst(types::I64, mask as i64);
                let result = builder.ins().band(shifted, mc);
                builder.ins().stack_store(result, regs[*dest as usize], 0);
            }
            // ArrayOperand is no longer a raw C string. Passing its String
            // buffer to xezim_jit_load_array_elem (which expects a CStr) is
            // both unterminated and loses the dense-array metadata. Keep
            // dynamic array blocks on the interpreter until the JIT bridge
            // accepts a resolved array descriptor.
            LoadArrayElem(..) => return Err(()),
            _ => return Err(()),
        }
        Ok(())
    }

    fn emit_cmp(
        builder: &mut FunctionBuilder,
        regs: &[StackSlot],
        d: u16,
        l: u16,
        r: u16,
        cc: IntCC,
    ) {
        let lv = builder.ins().stack_load(types::I64, regs[l as usize], 0);
        let rv = builder.ins().stack_load(types::I64, regs[r as usize], 0);
        let cmp = builder.ins().icmp(cc, lv, rv);
        // Cranelift icmp returns an I8 (boolean). Extend to I64 for
        // storage; Verilog relational ops produce a 1-bit value where
        // 0 = false, 1 = true.
        let ext = builder.ins().uextend(types::I64, cmp);
        builder.ins().stack_store(ext, regs[d as usize], 0);
    }

    fn emit_binop(
        builder: &mut FunctionBuilder,
        regs: &[StackSlot],
        d: u16,
        l: u16,
        r: u16,
        op: impl FnOnce(&mut FunctionBuilder, Value, Value) -> Value,
    ) {
        let lv = builder.ins().stack_load(types::I64, regs[l as usize], 0);
        let rv = builder.ins().stack_load(types::I64, regs[r as usize], 0);
        let result = op(builder, lv, rv);
        builder.ins().stack_store(result, regs[d as usize], 0);
    }

    /// Allowlist: MVP coverage. Keep in sync with `emit_insn` +
    /// the CFG-construction code in `codegen_block`.
    fn is_supported(insn: &Insn) -> bool {
        use Insn::*;
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
                | Nop
        )
    }
}
