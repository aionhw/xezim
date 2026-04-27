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
pub unsafe extern "C" fn xezim_jit_store_signal(
    sim: *mut u8,
    id: u32,
    val_bits: u64,
    width: u32,
) {
    let sim = &mut *(sim as *mut crate::compiler::simulator::Simulator);
    sim.jit_store_signal(id as usize, val_bits, width);
}

/// Schedule a non-blocking assign: push `(signal_id, value)` to
/// `nba_fast` so the next `apply_nba` pass writes `signal_table[id]`.
/// Mirrors `Insn::NbaAssign` semantics.
#[no_mangle]
pub unsafe extern "C" fn xezim_jit_schedule_nba(
    sim: *mut u8,
    id: u32,
    val_bits: u64,
    width: u32,
) {
    let sim = &mut *(sim as *mut crate::compiler::simulator::Simulator);
    sim.jit_schedule_nba(id as usize, val_bits, width);
}

/// Schedule a non-blocking assign to a bit-range: merges `val_bits`
/// at bits `[hi:lo]` into the current signal value (or in-flight NBA
/// entry) and pushes to `nba_fast`. Mirrors `Insn::NbaAssignRange` +
/// `Insn::NbaAssignRangeDyn`.
#[no_mangle]
pub unsafe extern "C" fn xezim_jit_schedule_nba_range(
    sim: *mut u8,
    id: u32,
    hi: u32,
    lo: u32,
    val_bits: u64,
) {
    let sim = &mut *(sim as *mut crate::compiler::simulator::Simulator);
    sim.jit_schedule_nba_range(id as usize, hi, lo, val_bits);
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
    if sim.jit_inputs_have_xz(ids) { 1 } else { 0 }
}

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
    use super::{JitFn, xezim_jit_load_signal, xezim_jit_store_signal, xezim_jit_schedule_nba, xezim_jit_schedule_nba_range, xezim_jit_inputs_have_xz};
    use cranelift::prelude::*;
    use cranelift::codegen::ir::{StackSlot, FuncRef};
    use cranelift_jit::{JITBuilder, JITModule as ClJitModule};
    use cranelift_module::{Linkage, Module, FuncId};

    pub struct JitModule {
        module: ClJitModule,
        next_id: u64,
    }

    impl JitModule {
        pub fn new() -> Option<Self> {
            let isa_builder = cranelift_native::builder().ok()?;
            let flag_builder = settings::builder();
            let isa = isa_builder
                .finish(settings::Flags::new(flag_builder))
                .ok()?;
            let mut builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
            // Register bridge function symbols so the JIT can link to them.
            builder.symbol("xezim_jit_load_signal", xezim_jit_load_signal as *const u8);
            builder.symbol("xezim_jit_store_signal", xezim_jit_store_signal as *const u8);
            builder.symbol("xezim_jit_schedule_nba", xezim_jit_schedule_nba as *const u8);
            builder.symbol("xezim_jit_schedule_nba_range", xezim_jit_schedule_nba_range as *const u8);
            builder.symbol("xezim_jit_inputs_have_xz", xezim_jit_inputs_have_xz as *const u8);
            Some(Self { module: ClJitModule::new(builder), next_id: 0 })
        }

        /// Try to JIT-compile a block's instruction list. Returns None if
        /// any Insn is not yet supported; callers fall back to the
        /// interpreter in that case.
        pub fn try_compile(&mut self, insns: &[Insn], num_regs: u32) -> Option<JitFn> {
            for insn in insns {
                if !is_supported(insn) { return None; }
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
                    if seen.insert(sid) { input_ids.push(sid); }
                }
            }
            self.codegen_block(insns, num_regs, &input_ids).ok()
        }

        fn codegen_block(&mut self, insns: &[Insn], num_regs: u32, input_ids: &[u32]) -> Result<JitFn, ()> {
            let pointer_type = self.module.target_config().pointer_type();

            // Declare bridge signatures (shared across all compiled blocks).
            let mut load_sig = self.module.make_signature();
            load_sig.params.push(AbiParam::new(pointer_type));   // sim
            load_sig.params.push(AbiParam::new(types::I32));     // id
            load_sig.returns.push(AbiParam::new(types::I64));    // val_bits

            let mut store_sig = self.module.make_signature();
            store_sig.params.push(AbiParam::new(pointer_type));  // sim
            store_sig.params.push(AbiParam::new(types::I32));    // id
            store_sig.params.push(AbiParam::new(types::I64));    // val_bits
            store_sig.params.push(AbiParam::new(types::I32));    // width

            let nba_sig = store_sig.clone();

            // nba_range: (sim, id, hi, lo, val_bits) — 5 args.
            let mut nba_range_sig = self.module.make_signature();
            nba_range_sig.params.push(AbiParam::new(pointer_type));
            nba_range_sig.params.push(AbiParam::new(types::I32));
            nba_range_sig.params.push(AbiParam::new(types::I32));
            nba_range_sig.params.push(AbiParam::new(types::I32));
            nba_range_sig.params.push(AbiParam::new(types::I64));

            // Path B: xz_check (sim, ids_ptr, n_ids) -> u32 (1 if any X/Z, else 0).
            let mut xz_check_sig = self.module.make_signature();
            xz_check_sig.params.push(AbiParam::new(pointer_type));
            xz_check_sig.params.push(AbiParam::new(pointer_type));
            xz_check_sig.params.push(AbiParam::new(types::I32));
            xz_check_sig.returns.push(AbiParam::new(types::I32));

            let load_id: FuncId = self.module
                .declare_function("xezim_jit_load_signal", Linkage::Import, &load_sig)
                .map_err(|_| ())?;
            let store_id: FuncId = self.module
                .declare_function("xezim_jit_store_signal", Linkage::Import, &store_sig)
                .map_err(|_| ())?;
            let nba_id: FuncId = self.module
                .declare_function("xezim_jit_schedule_nba", Linkage::Import, &nba_sig)
                .map_err(|_| ())?;
            let nba_range_id: FuncId = self.module
                .declare_function("xezim_jit_schedule_nba_range", Linkage::Import, &nba_range_sig)
                .map_err(|_| ())?;
            let xz_check_id: FuncId = self.module
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
                    if t < n { is_leader[t] = true; }
                    if i + 1 < n { is_leader[i + 1] = true; }
                }
            }
            let mut pc_to_block: Vec<Option<cranelift::codegen::ir::Block>> = vec![None; n.max(1)];
            let exit_block = builder.create_block();
            for (i, &leader) in is_leader.iter().enumerate() {
                if leader { pc_to_block[i] = Some(builder.create_block()); }
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
            let store_ref = self.module.declare_func_in_func(store_id, &mut builder.func);
            let nba_ref = self.module.declare_func_in_func(nba_id, &mut builder.func);
            let nba_range_ref = self.module.declare_func_in_func(nba_range_id, &mut builder.func);
            let xz_check_ref = self.module.declare_func_in_func(xz_check_id, &mut builder.func);

            if input_ids.is_empty() {
                // No reads — no X/Z risk. Fall straight through.
                builder.ins().jump(entry_block, &[prelude_sim_ptr]);
            } else {
                // Materialise input_ids as a fixed stack-slot u32 array,
                // then call xezim_jit_inputs_have_xz(sim, ptr, n).
                let slot_size = (input_ids.len() * 4) as u32;
                let id_slot = builder.create_sized_stack_slot(
                    StackSlotData::new(StackSlotKind::ExplicitSlot, slot_size, 2));
                for (i, &id) in input_ids.iter().enumerate() {
                    let id_val = builder.ins().iconst(types::I32, id as i64);
                    builder.ins().stack_store(id_val, id_slot, (i * 4) as i32);
                }
                let ids_ptr = builder.ins().stack_addr(pointer_type, id_slot, 0);
                let n_val = builder.ins().iconst(types::I32, input_ids.len() as i64);
                let call = builder.ins().call(xz_check_ref, &[prelude_sim_ptr, ids_ptr, n_val]);
                let xz_rc = builder.inst_results(call)[0];
                // Branch: rc != 0 → fallback_block (return 1); rc == 0 →
                // jump to entry_block with sim_ptr.
                builder.ins().brif(xz_rc, fallback_block, &[], entry_block, &[prelude_sim_ptr]);
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
                .map(|_| builder.create_sized_stack_slot(
                    StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 3)
                ))
                .collect();

            let resolve_target = |t: usize, pc_to_block: &Vec<Option<cranelift::codegen::ir::Block>>|
                -> cranelift::codegen::ir::Block
            {
                if t < pc_to_block.len() {
                    pc_to_block[t].unwrap_or(exit_block)
                } else {
                    exit_block
                }
            };

            // Walk insns, switching blocks at leaders, emitting terminators
            // for branches/jumps. `live` tracks whether the current block
            // is still open (no terminator emitted yet).
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
                        let cv = builder.ins().stack_load(types::I64, reg_slots[*cond as usize], 0);
                        let target_b = resolve_target(*target as usize, &pc_to_block);
                        let fall_b = if i + 1 < n {
                            pc_to_block[i + 1].unwrap_or(exit_block)
                        } else { exit_block };
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
                    other => {
                        emit_insn(&mut builder, other, sim_ptr, &reg_slots, load_ref, store_ref, nba_ref, nba_range_ref)?;
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
            let fn_name = { self.next_id += 1; format!("xezim_block_{}", self.next_id) };
            let func_id = self.module
                .declare_function(&fn_name, Linkage::Export, &ctx.func.signature)
                .map_err(|_| ())?;
            self.module.define_function(func_id, &mut ctx).map_err(|_| ())?;
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
        _nba_range_ref: FuncRef,
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
            BitAnd(d, l, r) => emit_binop(builder, regs, *d, *l, *r, |b, x, y| b.ins().band(x, y)),
            BitOr(d, l, r)  => emit_binop(builder, regs, *d, *l, *r, |b, x, y| b.ins().bor(x, y)),
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
            BlockingAssign(sig_id, val_reg, width) => {
                let v = builder.ins().stack_load(types::I64, regs[*val_reg as usize], 0);
                let id = builder.ins().iconst(types::I32, *sig_id as i64);
                let w = builder.ins().iconst(types::I32, *width as i64);
                builder.ins().call(store_ref, &[sim_ptr, id, v, w]);
            }
            NbaAssign(sig_id, val_reg, width) => {
                let v = builder.ins().stack_load(types::I64, regs[*val_reg as usize], 0);
                let id = builder.ins().iconst(types::I32, *sig_id as i64);
                let w = builder.ins().iconst(types::I32, *width as i64);
                builder.ins().call(nba_ref, &[sim_ptr, id, v, w]);
            }
            // NbaAssignRange: now safe under Path B (X/Z pre-check at
            // function entry; if any input has X/Z we bail before any
            // side effects). The c910 regression (9m30s → 12m40s) was
            // caused by 2-state bridge values diverging from the
            // interpreter's 4-state semantics during reset — Path B
            // skips the JIT body entirely while reset is asserted, so
            // the wrong-cascade can't happen.
            NbaAssignRange(sig_id, hi, lo, val_reg) => {
                let v = builder.ins().stack_load(types::I64, regs[*val_reg as usize], 0);
                let id = builder.ins().iconst(types::I32, *sig_id as i64);
                let hi_v = builder.ins().iconst(types::I32, *hi as i64);
                let lo_v = builder.ins().iconst(types::I32, *lo as i64);
                builder.ins().call(_nba_range_ref, &[sim_ptr, id, hi_v, lo_v, v]);
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
                let mask: u64 = if *width >= 64 { u64::MAX } else { (1u64 << *width) - 1 };
                let mc = builder.ins().iconst(types::I64, mask as i64);
                let masked = builder.ins().band(v, mc);
                builder.ins().stack_store(masked, regs[*reg as usize], 0);
            }
            BitSelect(dest, base, idx) => {
                // dest = (base >> idx) & 1
                let b = builder.ins().stack_load(types::I64, regs[*base as usize], 0);
                let i = builder.ins().stack_load(types::I64, regs[*idx as usize], 0);
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
                let b = builder.ins().stack_load(types::I64, regs[*base as usize], 0);
                let l = builder.ins().stack_load(types::I64, regs[*left_r as usize], 0);
                let r = builder.ins().stack_load(types::I64, regs[*right_r as usize], 0);
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
            _ => return Err(()),
        }
        Ok(())
    }

    fn emit_cmp(
        builder: &mut FunctionBuilder,
        regs: &[StackSlot],
        d: u16, l: u16, r: u16,
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
        d: u16, l: u16, r: u16,
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
        matches!(insn,
            LoadConst(..) | LoadSignal(..) | LoadSignalSigned(..)
            | Move(..)
            | BlockingAssign(..) | NbaAssign(..) | NbaAssignRange(..)
            | Add(..) | Sub(..)
            | BitAnd(..) | BitOr(..) | BitXor(..) | BitXnor(..) | BitNot(..)
            | LogAnd(..) | LogOr(..) | LogNot(..)
            | Negate(..)
            | Eq(..) | Neq(..) | Lt(..) | Leq(..) | Gt(..) | Geq(..)
            | Shl(..) | Shr(..) | AShr(..)
            | Resize(..)
            | BitSelect(..) | RangeSelect(..)
            | BranchIfFalse(..) | Jump(..)
            | Nop
        )
    }
}
