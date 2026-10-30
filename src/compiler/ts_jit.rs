//! Native code for two-state streams (exploration, `XEZIM_TS_JIT=1`, jit
//! feature). A straight-line lowered stream becomes one cranelift function
//! `fn(sim, signal_table_base) -> u32` (0 ok, 1 x-read bail, 2 abort):
//! registers are SSA values, signal loads are direct 32-byte-stride loads
//! against the documented `Value` layout, stores call the simulator's
//! two-state store helpers. Anything else (control flow, wide registers,
//! element access) is left to the executor.

#[cfg(feature = "jit")]
pub mod enabled {
    use super::super::bytecode::TsInsn;
    use super::super::simulator::Simulator;
    use cranelift::codegen::ir::MemFlagsData;
    use cranelift::prelude::*;
    use cranelift_jit::{JITBuilder, JITModule as ClJitModule};
    use cranelift_module::{FuncId, Linkage, Module};
    use xezim_core::value::Value as XValue;

    pub type TsJitFn = unsafe extern "C" fn(sim: *mut u8, table: *const u8) -> u32;

    pub unsafe extern "C" fn xezim_tsjit_store(sim: *mut u8, id: u32, v: u64, mask: u64) {
        unsafe { (*(sim as *mut Simulator)).ts_store(id as usize, v, mask) }
    }
    pub unsafe extern "C" fn xezim_tsjit_range_store(
        sim: *mut u8,
        id: u32,
        v: u64,
        lo: u32,
        hi: u32,
    ) {
        unsafe { (*(sim as *mut Simulator)).ts_range_store(id as usize, v, lo, hi) }
    }
    pub unsafe extern "C" fn xezim_tsjit_store_nba(sim: *mut u8, id: u32, v: u64, w: u32) {
        unsafe { (*(sim as *mut Simulator)).ts_store_nba(id as usize, v, w) }
    }
    pub unsafe extern "C" fn xezim_tsjit_range_store_nba(
        sim: *mut u8,
        id: u32,
        v: u64,
        lo: u32,
        hi: u32,
    ) {
        unsafe { (*(sim as *mut Simulator)).ts_range_store_nba(id as usize, lo, hi, v) }
    }
    pub unsafe extern "C" fn xezim_tsjit_store_xz(sim: *mut u8, id: u32, v: u64, x: u64) {
        unsafe { (*(sim as *mut Simulator)).ts_store_xz(id as usize, v, x) }
    }
    pub unsafe extern "C" fn xezim_tsjit_range_store_xz(
        sim: *mut u8,
        id: u32,
        v: u64,
        x: u64,
        lo: u32,
        hi: u32,
    ) {
        unsafe { (*(sim as *mut Simulator)).ts_range_store_xz(id as usize, v, x, lo, hi) }
    }
    pub unsafe extern "C" fn xezim_tsjit_bit_store_dyn(
        sim: *mut u8,
        id: u32,
        idx: u64,
        bit: u64,
        w: u32,
    ) {
        if idx < w as u64 {
            unsafe {
                (*(sim as *mut Simulator)).ts_range_store(
                    id as usize,
                    bit & 1,
                    idx as u32,
                    idx as u32,
                )
            }
        }
    }

    pub struct TsJitModule {
        module: ClJitModule,
        next_id: u64,
        pub compiled: u64,
        pub rejected: u64,
        pub code_bytes: u64,
        /// Defined but not yet finalized functions: (comb entry, id).
        /// Finalizing per function put each one on its own page; `flush`
        /// finalizes a whole batch into one contiguous segment.
        pending: Vec<(usize, FuncId)>,
    }

    /// Expand a fused instruction into its component primitives (the fused
    /// forms are defined as exactly that sequence). Control-flow-bearing
    /// fusions return None.
    fn unfuse(i: &TsInsn) -> Option<Vec<TsInsn>> {
        use TsInsn as T;
        Some(match i {
            T::LoadSigNot { dl, d, sig } => vec![
                T::LoadSig { d: *dl, sig: *sig },
                T::LogNot { d: *d, s: *dl },
            ],
            T::SigRangeEqC {
                dr,
                d,
                sig,
                lo,
                w,
                k,
            } => vec![
                T::SigRange {
                    d: *dr,
                    sig: *sig,
                    lo: *lo as u16,
                    mask: if *w >= 64 { u64::MAX } else { (1u64 << *w) - 1 },
                },
                T::EqC {
                    d: *d,
                    s: *dr,
                    k: *k,
                },
            ],
            T::LoadSigLogAnd { dl, sig, d, a, b } => vec![
                T::LoadSig { d: *dl, sig: *sig },
                T::LogAnd {
                    d: *d,
                    a: *a,
                    b: *b,
                },
            ],
            T::LogNotAnd { dn, s, d, a, b } => vec![
                T::LogNot { d: *dn, s: *s },
                T::And {
                    d: *d,
                    a: *a,
                    b: *b,
                },
            ],
            T::LogNotLogAnd { dn, s, d, a, b } => vec![
                T::LogNot { d: *dn, s: *s },
                T::LogAnd {
                    d: *d,
                    a: *a,
                    b: *b,
                },
            ],
            T::LogAndStore { d, a, b, sig, mask } => vec![
                T::LogAnd {
                    d: *d,
                    a: *a,
                    b: *b,
                },
                T::Store {
                    sig: *sig,
                    s: *d,
                    mask: *mask,
                },
            ],
            T::AndRangeStore {
                d,
                a,
                b,
                sig,
                hi,
                lo,
            } => vec![
                T::And {
                    d: *d,
                    a: *a,
                    b: *b,
                },
                T::RangeStore {
                    sig: *sig,
                    hi: *hi,
                    lo: *lo,
                    s: *d,
                    mask: if *hi - *lo + 1 >= 64 {
                        u64::MAX
                    } else {
                        (1u64 << (*hi - *lo + 1)) - 1
                    },
                },
            ],
            T::SigBitNot { db, d, sig, bit } => vec![
                T::SigBit {
                    d: *db,
                    sig: *sig,
                    bit: *bit,
                },
                T::LogNot { d: *d, s: *db },
            ],
            T::LoadSig2 { d1, sig1, d2, sig2 } => vec![
                T::LoadSig { d: *d1, sig: *sig1 },
                T::LoadSig { d: *d2, sig: *sig2 },
            ],
            T::SigBit2 {
                d1,
                sig1,
                bit1,
                d2,
                sig2,
                bit2,
            } => vec![
                T::SigBit {
                    d: *d1,
                    sig: *sig1,
                    bit: *bit1,
                },
                T::SigBit {
                    d: *d2,
                    sig: *sig2,
                    bit: *bit2,
                },
            ],
            T::LoadSigLogOr { dl, sig, d, a, b } => vec![
                T::LoadSig { d: *dl, sig: *sig },
                T::LogOr {
                    d: *d,
                    a: *a,
                    b: *b,
                },
            ],
            T::LoadSigAnd { dl, sig, d, a, b } => vec![
                T::LoadSig { d: *dl, sig: *sig },
                T::And {
                    d: *d,
                    a: *a,
                    b: *b,
                },
            ],
            T::LoadSigRepl {
                dl,
                sig,
                d,
                w,
                count,
            } => vec![
                T::LoadSig { d: *dl, sig: *sig },
                T::Repl {
                    d: *d,
                    s: *dl,
                    w: *w,
                    count: *count,
                },
            ],
            T::LoadSigSigRange {
                dl,
                sig,
                d,
                sig2,
                lo,
                mask,
            } => vec![
                T::LoadSig { d: *dl, sig: *sig },
                T::SigRange {
                    d: *d,
                    sig: *sig2,
                    lo: *lo,
                    mask: *mask,
                },
            ],
            T::SigRangeAnd {
                dr,
                sig,
                lo,
                mask,
                d,
                a,
                b,
            } => vec![
                T::SigRange {
                    d: *dr,
                    sig: *sig,
                    lo: *lo,
                    mask: *mask,
                },
                T::And {
                    d: *d,
                    a: *a,
                    b: *b,
                },
            ],
            T::SigRangeEq {
                dr,
                sig,
                lo,
                mask,
                d,
                a,
                b,
            } => vec![
                T::SigRange {
                    d: *dr,
                    sig: *sig,
                    lo: *lo,
                    mask: *mask,
                },
                T::Eq {
                    d: *d,
                    a: *a,
                    b: *b,
                },
            ],
            T::ConstEq { dc, v, d, a, b } => vec![
                T::Const { d: *dc, v: *v },
                T::Eq {
                    d: *d,
                    a: *a,
                    b: *b,
                },
            ],
            T::LogOrStore { d, a, b, sig, mask } => vec![
                T::LogOr {
                    d: *d,
                    a: *a,
                    b: *b,
                },
                T::Store {
                    sig: *sig,
                    s: *d,
                    mask: *mask,
                },
            ],
            T::AndOr {
                d1,
                a1,
                b1,
                d,
                a,
                b,
            } => vec![
                T::And {
                    d: *d1,
                    a: *a1,
                    b: *b1,
                },
                T::Or {
                    d: *d,
                    a: *a,
                    b: *b,
                },
            ],
            T::OrRangeStore {
                d,
                a,
                b,
                sig,
                hi,
                lo,
            } => vec![
                T::Or {
                    d: *d,
                    a: *a,
                    b: *b,
                },
                T::RangeStore {
                    sig: *sig,
                    hi: *hi,
                    lo: *lo,
                    s: *d,
                    mask: if *hi - *lo + 1 >= 64 {
                        u64::MAX
                    } else {
                        (1u64 << (*hi - *lo + 1)) - 1
                    },
                },
            ],
            T::LoadSigBrNz { .. }
            | T::BrFalseLoadSig { .. }
            | T::EqBrFalse { .. }
            // EqC + BrFalse was already outside the straight-line subset.
            // Expanding the fusion here would also require target relocation.
            | T::EqCBrFalse { .. } => return None,
            T::Concat2 { d, a, wa, b, wb } => vec![T::Concat {
                d: *d,
                parts: vec![(*a, *wa), (*b, *wb)].into_boxed_slice(),
            }],
            T::Concat3 {
                d,
                a,
                wa,
                b,
                wb,
                c,
                wc,
            } => vec![T::Concat {
                d: *d,
                parts: vec![(*a, *wa), (*b, *wb), (*c, *wc)].into_boxed_slice(),
            }],
            T::ElemStoreNbaFromSig(f) => vec![
                T::LoadSig {
                    d: f.dl,
                    sig: f.sig,
                },
                T::ElemStoreNba(Box::new(f.op.clone())),
            ],
            other => vec![other.clone()],
        })
    }

    impl TsJitModule {
        pub fn new() -> Option<Self> {
            if !XValue::inline_layout_ok() {
                eprintln!("[TS-JIT] Value layout probe failed; native two-state path disabled");
                return None;
            }
            let isa_builder = cranelift_native::builder().ok()?;
            let mut flag_builder = settings::builder();
            let _ = flag_builder.set("opt_level", "speed");
            let isa = isa_builder
                .finish(settings::Flags::new(flag_builder))
                .ok()?;
            let mut builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
            // One reserved region for all streams: the default provider
            // starts a fresh page-sized mapping after every finalize, which
            // put each ~300-byte function on its own page (5.6x the iTLB
            // misses of the interpreter on c906).
            let reserve: usize = std::env::var("XEZIM_TS_JIT_RESERVE_MB")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(256)
                << 20;
            match cranelift_jit::ArenaMemoryProvider::new_with_size(reserve) {
                Ok(p) => {
                    builder.memory_provider(Box::new(p));
                }
                Err(e) => {
                    eprintln!(
                        "[TS-JIT] arena reserve failed ({e}); native two-state path disabled"
                    );
                    return None;
                }
            }
            builder.symbol("xezim_tsjit_store", xezim_tsjit_store as *const u8);
            builder.symbol(
                "xezim_tsjit_range_store",
                xezim_tsjit_range_store as *const u8,
            );
            builder.symbol("xezim_tsjit_store_nba", xezim_tsjit_store_nba as *const u8);
            builder.symbol(
                "xezim_tsjit_range_store_nba",
                xezim_tsjit_range_store_nba as *const u8,
            );
            builder.symbol("xezim_tsjit_store_xz", xezim_tsjit_store_xz as *const u8);
            builder.symbol(
                "xezim_tsjit_range_store_xz",
                xezim_tsjit_range_store_xz as *const u8,
            );
            builder.symbol(
                "xezim_tsjit_bit_store_dyn",
                xezim_tsjit_bit_store_dyn as *const u8,
            );
            Some(Self {
                module: ClJitModule::new(builder),
                next_id: 0,
                compiled: 0,
                rejected: 0,
                code_bytes: 0,
                pending: Vec::new(),
            })
        }

        /// Compile one straight-line stream for comb entry `eidx`; the
        /// function becomes callable after the next `flush`. False when any
        /// instruction is outside the supported subset.
        pub fn compile(&mut self, eidx: usize, insns: &[TsInsn], num_regs: u32) -> bool {
            let flat: Option<Vec<TsInsn>> =
                insns
                    .iter()
                    .try_fold(Vec::with_capacity(insns.len() + 8), |mut acc, i| {
                        acc.extend(unfuse(i)?);
                        Some(acc)
                    });
            let Some(flat) = flat else {
                self.rejected += 1;
                return false;
            };
            match self.codegen(&flat, num_regs) {
                Ok(id) => {
                    self.compiled += 1;
                    self.pending.push((eidx, id));
                    true
                }
                Err(()) => {
                    self.rejected += 1;
                    false
                }
            }
        }

        pub fn pending_len(&self) -> usize {
            self.pending.len()
        }

        /// Finalize every pending function; returns (entry, fn) pairs.
        pub fn flush(&mut self) -> Vec<(usize, TsJitFn)> {
            if self.pending.is_empty() {
                return Vec::new();
            }
            if self.module.finalize_definitions().is_err() {
                self.pending.clear();
                return Vec::new();
            }
            let out: Vec<(usize, TsJitFn)> = self
                .pending
                .drain(..)
                .map(|(eidx, id)| {
                    let code = self.module.get_finalized_function(id);
                    (eidx, unsafe {
                        std::mem::transmute::<*const u8, TsJitFn>(code)
                    })
                })
                .collect();
            out
        }

        fn codegen(&mut self, insns: &[TsInsn], num_regs: u32) -> Result<FuncId, ()> {
            use TsInsn as T;
            let ptr_t = self.module.target_config().pointer_type();
            let mut sig_store = self.module.make_signature();
            sig_store.params.push(AbiParam::new(ptr_t));
            sig_store.params.push(AbiParam::new(types::I32));
            sig_store.params.push(AbiParam::new(types::I64));
            sig_store.params.push(AbiParam::new(types::I64));
            let mut sig_range = self.module.make_signature();
            sig_range.params.push(AbiParam::new(ptr_t));
            sig_range.params.push(AbiParam::new(types::I32));
            sig_range.params.push(AbiParam::new(types::I64));
            sig_range.params.push(AbiParam::new(types::I32));
            sig_range.params.push(AbiParam::new(types::I32));
            let mut sig_nba = self.module.make_signature();
            sig_nba.params.push(AbiParam::new(ptr_t));
            sig_nba.params.push(AbiParam::new(types::I32));
            sig_nba.params.push(AbiParam::new(types::I64));
            sig_nba.params.push(AbiParam::new(types::I32));
            let mut sig_xz = self.module.make_signature();
            sig_xz.params.push(AbiParam::new(ptr_t));
            sig_xz.params.push(AbiParam::new(types::I32));
            sig_xz.params.push(AbiParam::new(types::I64));
            sig_xz.params.push(AbiParam::new(types::I64));
            let mut sig_rxz = sig_xz.clone();
            sig_rxz.params.push(AbiParam::new(types::I32));
            sig_rxz.params.push(AbiParam::new(types::I32));
            let mut sig_bit = self.module.make_signature();
            sig_bit.params.push(AbiParam::new(ptr_t));
            sig_bit.params.push(AbiParam::new(types::I32));
            sig_bit.params.push(AbiParam::new(types::I64));
            sig_bit.params.push(AbiParam::new(types::I64));
            sig_bit.params.push(AbiParam::new(types::I32));
            let f_store: FuncId = self
                .module
                .declare_function("xezim_tsjit_store", Linkage::Import, &sig_store)
                .map_err(|_| ())?;
            let f_range = self
                .module
                .declare_function("xezim_tsjit_range_store", Linkage::Import, &sig_range)
                .map_err(|_| ())?;
            let f_nba = self
                .module
                .declare_function("xezim_tsjit_store_nba", Linkage::Import, &sig_nba)
                .map_err(|_| ())?;
            let f_rnba = self
                .module
                .declare_function("xezim_tsjit_range_store_nba", Linkage::Import, &sig_range)
                .map_err(|_| ())?;
            let f_sxz = self
                .module
                .declare_function("xezim_tsjit_store_xz", Linkage::Import, &sig_xz)
                .map_err(|_| ())?;
            let f_rxz = self
                .module
                .declare_function("xezim_tsjit_range_store_xz", Linkage::Import, &sig_rxz)
                .map_err(|_| ())?;
            let f_bit = self
                .module
                .declare_function("xezim_tsjit_bit_store_dyn", Linkage::Import, &sig_bit)
                .map_err(|_| ())?;

            let mut ctx = self.module.make_context();
            ctx.func.signature.params.push(AbiParam::new(ptr_t));
            ctx.func.signature.params.push(AbiParam::new(ptr_t));
            ctx.func.signature.returns.push(AbiParam::new(types::I32));
            let mut fbc = FunctionBuilderContext::new();
            let mut b = FunctionBuilder::new(&mut ctx.func, &mut fbc);
            let r_store = self.module.declare_func_in_func(f_store, b.func);
            let r_range = self.module.declare_func_in_func(f_range, b.func);
            let r_nba = self.module.declare_func_in_func(f_nba, b.func);
            let r_rnba = self.module.declare_func_in_func(f_rnba, b.func);
            let r_sxz = self.module.declare_func_in_func(f_sxz, b.func);
            let r_rxz = self.module.declare_func_in_func(f_rxz, b.func);
            let r_bit = self.module.declare_func_in_func(f_bit, b.func);

            let entry = b.create_block();
            let bail_x = b.create_block();
            let bail_abort = b.create_block();
            b.append_block_params_for_function_params(entry);
            b.switch_to_block(entry);
            let sim = b.block_params(entry)[0];
            let table = b.block_params(entry)[1];
            let mut regs: Vec<Option<Value>> = vec![None; num_regs as usize];
            let flags = MemFlagsData::trusted();
            macro_rules! reg {
                ($r:expr) => {{
                    let i = $r as usize;
                    if i >= regs.len() {
                        return Err(());
                    }
                    match regs[i] {
                        Some(v) => v,
                        None => b.ins().iconst(types::I64, 0),
                    }
                }};
            }
            macro_rules! set {
                ($r:expr, $v:expr) => {{
                    let i = $r as usize;
                    if i >= regs.len() {
                        return Err(());
                    }
                    regs[i] = Some($v);
                }};
            }
            // Load a narrow signal's planes; branches to the bail blocks on a
            // non-inline tag (abort) or any x/z bit (x-read bail).
            macro_rules! load_sig {
                ($sig:expr) => {{
                    let off = ($sig as i64) * 32;
                    let base = b.ins().iadd_imm_s(table, off);
                    // One branch per load: a non-inline tag or any x/z bit
                    // both hand the block to the four-state path (the
                    // interpreter re-runs it either way; only the demotion
                    // counter differs, and a wide value cannot appear here
                    // once the stream lowered).
                    let tag =
                        b.ins()
                            .load(types::I8, flags, base, XValue::INLINE_TAG_OFFSET as i32);
                    let tag64 = b.ins().uextend(types::I64, tag);
                    let x = b
                        .ins()
                        .load(types::I64, flags, base, XValue::INLINE_XZ_OFFSET as i32);
                    let bad = b.ins().bor(tag64, x);
                    let ok = b.create_block();
                    b.ins().brif(bad, bail_x, &[], ok, &[]);
                    b.switch_to_block(ok);
                    b.ins()
                        .load(types::I64, flags, base, XValue::INLINE_VAL_OFFSET as i32)
                }};
            }
            macro_rules! bool64 {
                ($c:expr) => {{
                    let c = $c;
                    b.ins().uextend(types::I64, c)
                }};
            }
            for insn in insns {
                match insn {
                    T::LoadSig { d, sig } => {
                        let v = load_sig!(*sig);
                        set!(*d, v);
                    }
                    T::SigBit { d, sig, bit } => {
                        let v = load_sig!(*sig);
                        let s = b.ins().ushr_imm_s(v, *bit as i64);
                        let r = b.ins().band_imm_s(s, 1);
                        set!(*d, r);
                    }
                    T::SigRange { d, sig, lo, mask } => {
                        let v = load_sig!(*sig);
                        let s = b.ins().ushr_imm_s(v, *lo as i64);
                        let m = b.ins().iconst(types::I64, *mask as i64);
                        let r = b.ins().band(s, m);
                        set!(*d, r);
                    }
                    T::Const { d, v } => {
                        let c = b.ins().iconst(types::I64, *v as i64);
                        set!(*d, c);
                    }
                    T::Bit { d, s, bit } => {
                        let sv = reg!(*s);
                        let t = b.ins().ushr_imm_s(sv, *bit as i64);
                        let r = b.ins().band_imm_s(t, 1);
                        set!(*d, r);
                    }
                    T::Range { d, s, lo, mask } => {
                        let sv = reg!(*s);
                        let t = b.ins().ushr_imm_s(sv, *lo as i64);
                        let m = b.ins().iconst(types::I64, *mask as i64);
                        let r = b.ins().band(t, m);
                        set!(*d, r);
                    }
                    T::Xor { d, a, b: bb } => {
                        let (x, y) = (reg!(*a), reg!(*bb));
                        let r = b.ins().bxor(x, y);
                        set!(*d, r);
                    }
                    T::And { d, a, b: bb } => {
                        let (x, y) = (reg!(*a), reg!(*bb));
                        let r = b.ins().band(x, y);
                        set!(*d, r);
                    }
                    T::Or { d, a, b: bb } => {
                        let (x, y) = (reg!(*a), reg!(*bb));
                        let r = b.ins().bor(x, y);
                        set!(*d, r);
                    }
                    T::Sel { d, c, a, b: bb, .. } => {
                        let (cv, x, y) = (reg!(*c), reg!(*a), reg!(*bb));
                        let r = b.ins().select(cv, x, y);
                        set!(*d, r);
                    }
                    T::Not { d, s, mask } => {
                        let sv = reg!(*s);
                        let n = b.ins().bnot(sv);
                        let m = b.ins().iconst(types::I64, *mask as i64);
                        let r = b.ins().band(n, m);
                        set!(*d, r);
                    }
                    T::XorC { d, s, k } => {
                        let sv = reg!(*s);
                        let m = b.ins().iconst(types::I64, *k as i64);
                        let r = b.ins().bxor(sv, m);
                        set!(*d, r);
                    }
                    T::EqC { d, s, k } => {
                        let sv = reg!(*s);
                        let m = b.ins().iconst(types::I64, *k as i64);
                        let c = b.ins().icmp(IntCC::Equal, sv, m);
                        let r = bool64!(c);
                        set!(*d, r);
                    }
                    T::MaskEq { d, s, mask, v } => {
                        let sv = reg!(*s);
                        let m = b.ins().iconst(types::I64, *mask as i64);
                        let t = b.ins().band(sv, m);
                        let k = b.ins().iconst(types::I64, *v as i64);
                        let c = b.ins().icmp(IntCC::Equal, t, k);
                        let r = bool64!(c);
                        set!(*d, r);
                    }
                    T::Add { d, a, b: bb, mask }
                    | T::Sub { d, a, b: bb, mask }
                    | T::Mul { d, a, b: bb, mask } => {
                        let (x, y) = (reg!(*a), reg!(*bb));
                        let t = match insn {
                            T::Add { .. } => b.ins().iadd(x, y),
                            T::Sub { .. } => b.ins().isub(x, y),
                            _ => b.ins().imul(x, y),
                        };
                        let m = b.ins().iconst(types::I64, *mask as i64);
                        let r = b.ins().band(t, m);
                        set!(*d, r);
                    }
                    T::AddC { d, s, k, mask } => {
                        let sv = reg!(*s);
                        let kk = b.ins().iconst(types::I64, *k as i64);
                        let t = b.ins().iadd(sv, kk);
                        let m = b.ins().iconst(types::I64, *mask as i64);
                        let r = b.ins().band(t, m);
                        set!(*d, r);
                    }
                    T::Eq { d, a, b: bb }
                    | T::Neq { d, a, b: bb }
                    | T::Lt { d, a, b: bb }
                    | T::Leq { d, a, b: bb }
                    | T::Gt { d, a, b: bb }
                    | T::Geq { d, a, b: bb } => {
                        let (x, y) = (reg!(*a), reg!(*bb));
                        let cc = match insn {
                            T::Eq { .. } => IntCC::Equal,
                            T::Neq { .. } => IntCC::NotEqual,
                            T::Lt { .. } => IntCC::UnsignedLessThan,
                            T::Leq { .. } => IntCC::UnsignedLessThanOrEqual,
                            T::Gt { .. } => IntCC::UnsignedGreaterThan,
                            _ => IntCC::UnsignedGreaterThanOrEqual,
                        };
                        let c = b.ins().icmp(cc, x, y);
                        let r = bool64!(c);
                        set!(*d, r);
                    }
                    T::LogNot { d, s } => {
                        let sv = reg!(*s);
                        let c = b.ins().icmp_imm_s(IntCC::Equal, sv, 0);
                        let r = bool64!(c);
                        set!(*d, r);
                    }
                    T::LogAnd { d, a, b: bb } | T::LogOr { d, a, b: bb } => {
                        let (x, y) = (reg!(*a), reg!(*bb));
                        let cx = b.ins().icmp_imm_s(IntCC::NotEqual, x, 0);
                        let cy = b.ins().icmp_imm_s(IntCC::NotEqual, y, 0);
                        let c = if matches!(insn, T::LogAnd { .. }) {
                            b.ins().band(cx, cy)
                        } else {
                            b.ins().bor(cx, cy)
                        };
                        let r = bool64!(c);
                        set!(*d, r);
                    }
                    T::RedOr { d, s } => {
                        let sv = reg!(*s);
                        let c = b.ins().icmp_imm_s(IntCC::NotEqual, sv, 0);
                        let r = bool64!(c);
                        set!(*d, r);
                    }
                    T::RedAnd { d, s, mask } => {
                        let sv = reg!(*s);
                        let m = b.ins().iconst(types::I64, *mask as i64);
                        let c = b.ins().icmp(IntCC::Equal, sv, m);
                        let r = bool64!(c);
                        set!(*d, r);
                    }
                    T::Mask { d, mask } => {
                        let sv = reg!(*d);
                        let m = b.ins().iconst(types::I64, *mask as i64);
                        let r = b.ins().band(sv, m);
                        set!(*d, r);
                    }
                    T::Shl {
                        d,
                        a,
                        b: bb,
                        w,
                        mask,
                    } => {
                        let (x, amt) = (reg!(*a), reg!(*bb));
                        let ge =
                            b.ins()
                                .icmp_imm_s(IntCC::UnsignedGreaterThanOrEqual, amt, *w as i64);
                        let t = b.ins().ishl(x, amt);
                        let m = b.ins().iconst(types::I64, *mask as i64);
                        let shifted = b.ins().band(t, m);
                        let zero = b.ins().iconst(types::I64, 0);
                        let r = b.ins().select(ge, zero, shifted);
                        set!(*d, r);
                    }
                    T::Shr { d, a, b: bb, w } => {
                        let (x, amt) = (reg!(*a), reg!(*bb));
                        let ge =
                            b.ins()
                                .icmp_imm_s(IntCC::UnsignedGreaterThanOrEqual, amt, *w as i64);
                        let shifted = b.ins().ushr(x, amt);
                        let zero = b.ins().iconst(types::I64, 0);
                        let r = b.ins().select(ge, zero, shifted);
                        set!(*d, r);
                    }
                    T::Repl { d, s, w, count } => {
                        let part = reg!(*s);
                        let mut acc = b.ins().iconst(types::I64, 0);
                        for _ in 0..*count {
                            let sh = b.ins().ishl_imm_s(acc, *w as i64);
                            acc = b.ins().bor(sh, part);
                        }
                        set!(*d, acc);
                    }
                    T::Concat { d, parts } => {
                        let mut acc = b.ins().iconst(types::I64, 0);
                        for &(r, w) in parts.iter() {
                            let pv = reg!(r);
                            let sh = b.ins().ishl_imm_s(acc, w as i64);
                            acc = b.ins().bor(sh, pv);
                        }
                        set!(*d, acc);
                    }
                    T::Store { sig, s, mask } => {
                        let sv = reg!(*s);
                        let m = b.ins().iconst(types::I64, *mask as i64);
                        let v = b.ins().band(sv, m);
                        let id = b.ins().iconst(types::I32, *sig as i64);
                        b.ins().call(r_store, &[sim, id, v, m]);
                    }
                    T::RangeStore {
                        sig,
                        hi,
                        lo,
                        s,
                        mask,
                    } => {
                        let sv = reg!(*s);
                        let m = b.ins().iconst(types::I64, *mask as i64);
                        let v = b.ins().band(sv, m);
                        let id = b.ins().iconst(types::I32, *sig as i64);
                        let l = b.ins().iconst(types::I32, *lo as i64);
                        let h = b.ins().iconst(types::I32, *hi as i64);
                        b.ins().call(r_range, &[sim, id, v, l, h]);
                    }
                    T::StoreNba { sig, s, w, mask } => {
                        let sv = reg!(*s);
                        let m = b.ins().iconst(types::I64, *mask as i64);
                        let v = b.ins().band(sv, m);
                        let id = b.ins().iconst(types::I32, *sig as i64);
                        let ww = b.ins().iconst(types::I32, *w as i64);
                        b.ins().call(r_nba, &[sim, id, v, ww]);
                    }
                    T::ConstStoreNba { sig, v, w } => {
                        let vv = b.ins().iconst(types::I64, *v as i64);
                        let id = b.ins().iconst(types::I32, *sig as i64);
                        let ww = b.ins().iconst(types::I32, *w as i64);
                        b.ins().call(r_nba, &[sim, id, vv, ww]);
                    }
                    T::RangeStoreNba {
                        sig,
                        hi,
                        lo,
                        s,
                        mask,
                    } => {
                        let sv = reg!(*s);
                        let m = b.ins().iconst(types::I64, *mask as i64);
                        let v = b.ins().band(sv, m);
                        let id = b.ins().iconst(types::I32, *sig as i64);
                        let l = b.ins().iconst(types::I32, *lo as i64);
                        let h = b.ins().iconst(types::I32, *hi as i64);
                        b.ins().call(r_rnba, &[sim, id, v, l, h]);
                    }
                    T::ConstStoreX { sig, v, x } => {
                        let vv = b.ins().iconst(types::I64, *v as i64);
                        let xx = b.ins().iconst(types::I64, *x as i64);
                        let id = b.ins().iconst(types::I32, *sig as i64);
                        b.ins().call(r_sxz, &[sim, id, vv, xx]);
                    }
                    T::RangeStoreX(p) => {
                        let vv = b.ins().iconst(types::I64, p.v as i64);
                        let xx = b.ins().iconst(types::I64, p.x as i64);
                        let id = b.ins().iconst(types::I32, p.sig as i64);
                        let l = b.ins().iconst(types::I32, p.lo as i64);
                        let h = b.ins().iconst(types::I32, p.hi as i64);
                        b.ins().call(r_rxz, &[sim, id, vv, xx, l, h]);
                    }
                    T::BitStoreDyn { sig, i, s, w } if *w <= 64 => {
                        let idx = reg!(*i);
                        let bit = reg!(*s);
                        let id = b.ins().iconst(types::I32, *sig as i64);
                        let ww = b.ins().iconst(types::I32, *w as i64);
                        b.ins().call(r_bit, &[sim, id, idx, bit, ww]);
                    }
                    _ => return Err(()),
                }
            }
            let zero = b.ins().iconst(types::I32, 0);
            b.ins().return_(&[zero]);
            b.switch_to_block(bail_x);
            let one = b.ins().iconst(types::I32, 1);
            b.ins().return_(&[one]);
            b.switch_to_block(bail_abort);
            let two = b.ins().iconst(types::I32, 2);
            b.ins().return_(&[two]);
            let _ = &bail_abort;
            b.seal_all_blocks();
            b.finalize(self.module.target_config());
            if std::env::var_os("XEZIM_TS_JIT_CLIF").is_some() {
                eprintln!("[TS-JIT CLIF]\n{}", ctx.func.display());
            }
            self.next_id += 1;
            let name = format!("xezim_ts_{}", self.next_id);
            let fid = self
                .module
                .declare_function(&name, Linkage::Export, &ctx.func.signature)
                .map_err(|_| ())?;
            self.module.define_function(fid, &mut ctx).map_err(|_| ())?;
            if let Some(cc) = ctx.compiled_code() {
                self.code_bytes += cc.code_info().total_size as u64;
            }
            self.module.clear_context(&mut ctx);
            Ok(fid)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn constant_branch_fusion_preserves_native_eligibility() {
            let Some(mut jit) = TsJitModule::new() else {
                return; // Native code generation is unavailable on this target.
            };
            // Cover a live scratch, source/destination aliasing, a high-bit
            // immediate and a backward target. Every control-flow stream
            // remains with the executor, just as its unfused form did.
            for (d, s, k, t) in [(1, 0, u64::MAX, 3), (0, 0, 0, 3), (1, 0, 7, 0)] {
                let original = [
                    TsInsn::Const { d: s, v: k },
                    TsInsn::EqC { d, s, k },
                    TsInsn::BrFalse { s: d, t },
                ];
                let fused = [
                    TsInsn::Const { d: s, v: k },
                    TsInsn::EqCBrFalse { d, s, k, t },
                ];
                assert!(!jit.compile(0, &original, 2));
                assert!(!jit.compile(0, &fused, 2));
                assert_eq!(jit.pending_len(), 0);
            }
            assert_eq!(jit.rejected, 6);
            assert_eq!(jit.compiled, 0);

            // Rejections must not affect an otherwise eligible stream.
            assert!(jit.compile(
                1,
                &[
                    TsInsn::Const { d: 0, v: 7 },
                    TsInsn::EqC { d: 1, s: 0, k: 7 },
                ],
                2,
            ));
            let functions = jit.flush();
            assert_eq!(functions.len(), 1);
            assert_eq!(functions[0].0, 1);
            // The constant-only stream accesses neither simulator nor table.
            assert_eq!(
                unsafe { functions[0].1(std::ptr::null_mut(), std::ptr::null()) },
                0
            );
            assert_eq!(jit.compiled, 1);
        }
    }
}
