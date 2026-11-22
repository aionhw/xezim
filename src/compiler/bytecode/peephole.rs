//! Peephole passes over compiled instruction streams.

use super::*;

impl<'a> BytecodeCompiler<'a> {
    /// Does `insn` read register `r`? Conservative: unknown/AST-fallback
    /// instructions report `true`. Used by the fuse peephole's liveness
    /// check — a wrong `false` here would fuse away a load whose register
    /// is still consumed later, so every variant must be enumerated.
    /// (pub(crate): the FSM native generator in `aot.rs` uses this to prove
    /// a Real tick literal feeds only a delay wait.)
    pub(crate) fn insn_reads_reg(insn: &Insn, r: RegId) -> bool {
        let mut hit = false;
        !Self::insn_read_regs(insn, &mut |x| hit |= x == r) || hit
    }

    /// Every register occurrence `insn` reads (one read twice is reported
    /// twice). Returns `false` for the AST fallbacks, which may read any
    /// register. `insn_reads_reg` is defined through this, so the two cannot
    /// disagree.
    pub(crate) fn insn_read_regs(insn: &Insn, f: &mut impl FnMut(RegId)) -> bool {
        match insn {
            Insn::WaitDelayReg(d) => f(*d),
            Insn::WaitEdge(..) | Insn::CovHit(..) => {}
            Insn::CmpBranch(_, l, rr, _, _) => {
                f(*l);
                f(*rr);
            }
            Insn::MoveResize(_, s, _) | Insn::CmpBranchConst(_, s, _, _, _) => f(*s),
            Insn::CaseLut(_, src, _) => f(*src),
            Insn::CaseJump(src, _) => f(*src),
            Insn::CaseMaskJump(src, _) => f(*src),
            Insn::Format(_, fs) => fs.args.iter().for_each(|&a| f(a)),
            Insn::StrOp(_, _, args) => args.iter().for_each(|&a| f(a)),
            Insn::BlockingAssignString(_, v) => f(*v),
            Insn::LoadProcessStatus(_d, h) => {
                f(*h);
            }
            Insn::LoadSignalRangeDyn(_, _, lo, _) => f(*lo),
            Insn::LoadConst(..)
            | Insn::LoadSignal(..)
            | Insn::LoadSignalSigned(..)
            | Insn::LoadProcessLocal(..)
            | Insn::LoadSignalRange(..)
            | Insn::LoadSignalBit(..)
            | Insn::NbaAssignConst(..)
            | Insn::BranchIfSignalFalse(..)
            // Reads its index straight out of the signal table; no registers.
            | Insn::NbaAssignArrayRead(..)
            | Insn::Jump(..)
            | Insn::Nop => {}
            Insn::BranchUnlessZero(c, _) => f(*c),
            // In-place mutators read their register.
            Insn::Resize(a, _)
            | Insn::SetSigned(a)
            | Insn::ClearSigned(a)
            | Insn::DropXz(a)
            | Insn::RealToInt(a) => f(*a),
            // Class-member access: Load reads the handle; Store reads the
            // handle AND the stored value register.
            Insn::LoadClassMember(_, h, _) => f(*h),
            Insn::StoreClassMember(h, v, _) => {
                f(*h);
                f(*v);
            }
            Insn::CallMethod(_, h, _, a, n) => {
                f(*h);
                for x in *a as usize..*a as usize + *n as usize {
                    f(x as RegId);
                }
            }
            Insn::CallScopedMethod(_, t, _, a, n) => {
                f(*t);
                for x in *a as usize..*a as usize + *n as usize {
                    f(x as RegId);
                }
            }
            Insn::ConstructObject(_, _, a, n) => {
                for x in *a as usize..*a as usize + *n as usize {
                    f(x as RegId);
                }
            }
            // Step 9h: a class-scope static call reads its arg range only.
            Insn::CallStaticScoped(_, _, a, n) => {
                for x in *a as usize..*a as usize + *n as usize {
                    f(x as RegId);
                }
            }
            // class-perf P2: a free-function call reads its arg range only.
            Insn::CallFreeFunction(_, _, a, n) => {
                for x in *a as usize..*a as usize + *n as usize {
                    f(x as RegId);
                }
            }
            // class-perf collection-return: a contiguous arg range read.
            Insn::RetCollection(a, n) => {
                for x in *a as usize..*a as usize + *n as usize {
                    f(x as RegId);
                }
            }
            // class-perf collection-return store: reads the call's result.
            Insn::AssignPendingColl(_, v) => f(*v),
            // Step 9g: a static-property READ has no register inputs.
            Insn::LoadClassStatic(..) => {}
            Insn::StoreClassStatic(_, src) => f(*src),
            Insn::CallCollMethod(_, h, _, a, n, _) => {
                f(*h);
                for x in *a as usize..*a as usize + *n as usize {
                    f(x as RegId);
                }
            }
            // Collection element access: Load reads handle + index; Store
            // reads handle + index + value.
            Insn::LoadCollElem(_, h, _, i) => {
                f(*h);
                f(*i);
            }
            Insn::StoreCollElem(h, _, i, v) => {
                f(*h);
                f(*i);
                f(*v);
            }
            // Compiled foreach: ForeachKeys reads only the handle;
            // ForeachNext WRITES its var register (and the ok flag) — the
            // liveness check must not fuse away its load.
            Insn::ForeachKeys(_, h, _) => f(*h),
            Insn::ForeachNext(..) => {}
            // `$cast`: reads the source (and, for the register route, the
            // dest slot's current value for the width fit); the Member route
            // also reads the base handle.
            Insn::Cast(c, s, ..) => {
                let (dest, s) = (&c.dest, s);
                f(*s);
                match dest {
                    CastDest::Reg(d) => f(*d),
                    CastDest::Member(h, _) => f(*h),
                    CastDest::Scope => {}
                }
            }
            Insn::Pow(_, l, rr)
            | Insn::Add(_, l, rr)
            | Insn::Sub(_, l, rr)
            | Insn::Mul(_, l, rr)
            | Insn::Div(_, l, rr)
            | Insn::Mod(_, l, rr)
            | Insn::BitAnd(_, l, rr)
            | Insn::BitOr(_, l, rr)
            | Insn::BitXor(_, l, rr)
            | Insn::BitXnor(_, l, rr)
            | Insn::LogAnd(_, l, rr)
            | Insn::LogOr(_, l, rr)
            | Insn::Eq(_, l, rr)
            | Insn::Neq(_, l, rr)
            | Insn::CaseEq(_, l, rr)
            | Insn::CasezEq(_, l, rr)
            | Insn::CasexEq(_, l, rr)
            | Insn::Lt(_, l, rr)
            | Insn::Leq(_, l, rr)
            | Insn::Gt(_, l, rr)
            | Insn::Geq(_, l, rr)
            | Insn::Shl(_, l, rr)
            | Insn::Shr(_, l, rr)
            | Insn::AShr(_, l, rr) => {
                f(*l);
                f(*rr);
            }
            Insn::BitNot(_, s)
            | Insn::LogNot(_, s)
            | Insn::Negate(_, s)
            | Insn::ReduceAnd(_, s)
            | Insn::ReduceOr(_, s)
            | Insn::ReduceXor(_, s)
            | Insn::Move(_, s)
            | Insn::Replicate(_, s, _) => f(*s),
            // Its other operand is the embedded constant, not a register.
            Insn::BinOpConst(_, s, _, _) => f(*s),
            Insn::BinOpConstAdd2(a) => {
                f(a.s1);
                f(a.s2);
            }
            Insn::BitSelect(_, b, i) => {
                f(*b);
                f(*i);
            }
            Insn::BitSelectConst(_, b, _) => f(*b),
            Insn::RangeSelect(_, b, l, rr) => {
                f(*b);
                f(*l);
                f(*rr);
            }
            Insn::RangeSelectW(_, b, i, _, _) => {
                f(*b);
                f(*i);
            }
            Insn::RangeSelectConst(_, b, _, _) => f(*b),
            Insn::Concat(_, parts) => parts.iter().for_each(|&a| f(a)),
            Insn::BranchIfFalse(c, _) => f(*c),
            Insn::Select(_, c, t, e) => {
                f(*c);
                f(*t);
                f(*e);
            }
            Insn::NbaAssign(_, v, _) | Insn::BlockingAssign(_, v, _) => f(*v),
            Insn::NbaAssignRange(_, _, _, v) | Insn::BlockingAssignRange(_, _, _, v) => f(*v),
            Insn::NbaAssignRangeDyn(_, h, l, v) | Insn::BlockingAssignRangeDyn(_, h, l, v) => {
                f(*h);
                f(*l);
                f(*v);
            }
            Insn::NbaAssignBitDyn(_, i, v) | Insn::BlockingAssignBitDyn(_, i, v) => {
                f(*i);
                f(*v);
            }
            Insn::LoadArrayElem(_, _, i) | Insn::LoadDynElem(_, _, i) => f(*i),
            Insn::NbaAssignArray(_, i, v, _) | Insn::BlockingAssignArray(_, i, v, _) => {
                f(*i);
                f(*v);
            }
            Insn::NbaAssignArrayRange(_, i, h, l, v)
            | Insn::BlockingAssignArrayRange(_, i, h, l, v) => {
                f(*i);
                f(*h);
                f(*l);
                f(*v);
            }
            // AST fallback can read anything through the interpreter.
            Insn::StmtFallback(..) => return false,
            // A local-collection declaration has storage side effects.
            Insn::InitLocalColl(..) => return false,
            Insn::EvalExprFallback(..) => return false,
        }
        true
    }

    /// Peephole: fuse `LoadSignal(t, s); RangeSelectConst(d, t, l, r)` into
    /// `LoadSignalRange(d, s, l, r)` (and the BitSelectConst analogue) when
    /// the loaded register `t` is dead afterwards. The second slot becomes a
    /// `Nop` so every branch target in the block stays valid. Skipped when a
    /// jump lands ON the select (the fused load would then be bypassed), or
    /// when `t` is read again later — unless the select overwrote `t`
    /// itself (d == t), which destroys the raw value anyway.
    /// `XEZIM_FUSE=0` disables the pass (A/B escape hatch).
    pub(super) fn fuse_load_selects(insns: &mut [Insn]) {
        use std::sync::OnceLock;
        // Bit-set: 1 = load+range, 2 = load+bit, 4 = const-NBA, 8 = branch
        // fusions. Default = all on; named values select one family for A/B
        // bisection.
        static MODE: OnceLock<u8> = OnceLock::new();
        let mode = *MODE.get_or_init(|| match std::env::var("XEZIM_FUSE").as_deref() {
            Ok("0") => 0,
            Ok("range") => 1,
            Ok("bit") => 2,
            Ok("nba") => 4,
            Ok("branch") => 8,
            _ => 0xF,
        });
        if mode == 0 || insns.len() < 2 {
            return;
        }
        // Branch targets: fusing must not change what a jump lands on.
        let mut is_target = vec![false; insns.len() + 1];
        for insn in insns.iter() {
            match insn {
                Insn::CaseJump(_, cj) => {
                    for &t in cj.table.iter().chain(std::iter::once(&cj.default)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::CaseMaskJump(_, mj) => {
                    for &t in mj.table.iter().chain(std::iter::once(&mj.xz_path)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::BranchIfFalse(_, t)
                | Insn::BranchUnlessZero(_, t)
                | Insn::BranchIfSignalFalse(_, t, _)
                | Insn::CmpBranch(_, _, _, _, t)
                | Insn::CmpBranchConst(_, _, _, _, t)
                | Insn::Jump(t)
                    if (*t as usize) < is_target.len() =>
                {
                    is_target[*t as usize] = true;
                }
                _ => {}
            }
        }
        let mut rr = RegReaders::new(insns);
        // Second family: pairs whose fused form has NO destination register —
        // the first insn's register must simply be dead everywhere else.
        //   LoadConst K ; NbaAssign(sig, k, w)        → NbaAssignConst
        //   LogNot(d,s) ; BranchIfFalse(d, T)         → BranchUnlessZero(s, T)
        //   LoadSignal(t,s) ; BranchIfFalse(t, T)     → BranchIfSignalFalse(s, T)
        for i in 0..insns.len() - 1 {
            if is_target[i + 1] {
                continue;
            }
            let (dead_reg, repl) = match (&insns[i], &insns[i + 1]) {
                (Insn::LoadConst(c, k), &Insn::NbaAssign(sig, v, w))
                    if v == *c && (mode & 4) != 0 =>
                {
                    // Pre-resize at fuse time — the exec arm then only
                    // compares + clones-on-change, never resizes.
                    (
                        *c,
                        Insn::NbaAssignConst(sig, Box::new(k.resize_for_assign(w)), w),
                    )
                }
                (&Insn::LogNot(d, s), &Insn::BranchIfFalse(c, t)) if c == d && (mode & 8) != 0 => {
                    (d, Insn::BranchUnlessZero(s, t))
                }
                (&Insn::LoadSignal(r, sig), &Insn::BranchIfFalse(c, t))
                    if c == r && (mode & 8) != 0 =>
                {
                    (r, Insn::BranchIfSignalFalse(sig, t, u32::MAX))
                }
                _ => continue,
            };
            // The fused form never writes `dead_reg`, so ANY other read of it
            // in the block blocks the fusion (no d==t exemption here).
            if rr.read_outside(insns, dead_reg, &[i, i + 1]) {
                continue;
            }
            rr.remove(&insns[i]);
            rr.remove(&insns[i + 1]);
            rr.add(&repl);
            insns[i] = repl;
            insns[i + 1] = Insn::Nop;
        }

        // `if (a && b)` guard:
        //   LoadSignal(r1,s1); LoadSignal(r2,s2); LogAnd(d,r1,r2);
        //   BranchIfFalse(d,T)
        //       → BranchIfSignalFalse(s1,T); BranchIfSignalFalse(s2,T)
        // Equivalent under 4-state semantics: the body is skipped whenever
        // `a && b` is not true, and testing the operands in sequence skips in
        // exactly those cases (an X operand skips either way). Both operands
        // are bare signal loads, so dropping the second test on an early skip
        // loses no side effects.
        if (mode & 8) != 0 && insns.len() >= 4 {
            for i in 0..insns.len() - 3 {
                let (&Insn::LoadSignal(r1, s1), &Insn::LoadSignal(r2, s2)) =
                    (&insns[i], &insns[i + 1])
                else {
                    continue;
                };
                let Insn::LogAnd(d, a, b) = insns[i + 2] else {
                    continue;
                };
                if a != r1 || b != r2 {
                    continue;
                }
                let Insn::BranchIfFalse(cnd, t) = insns[i + 3] else {
                    continue;
                };
                if cnd != d {
                    continue;
                }
                if (i + 1..=i + 3).any(|x| is_target[x]) {
                    continue;
                }
                // r1, r2 and d must be dead outside the quad.
                let quad = [i, i + 1, i + 2, i + 3];
                if rr.read_outside(insns, r1, &quad)
                    || rr.read_outside(insns, r2, &quad)
                    || rr.read_outside(insns, d, &quad)
                {
                    continue;
                }
                for k in quad {
                    rr.remove(&insns[k]);
                }
                insns[i] = Insn::BranchIfSignalFalse(s1, t, u32::MAX);
                insns[i + 1] = Insn::BranchIfSignalFalse(s2, t, u32::MAX);
                insns[i + 2] = Insn::Nop;
                insns[i + 3] = Insn::Nop;
            }
        }

        for i in 0..insns.len() - 1 {
            let &Insn::LoadSignal(t, sig) = &insns[i] else {
                continue;
            };
            if is_target[i + 1] {
                continue;
            }
            let fused = match insns[i + 1] {
                Insn::RangeSelectConst(d, b, l, r) if b == t && (mode & 1) != 0 => {
                    Some((d, Insn::LoadSignalRange(d, sig, l, r)))
                }
                Insn::BitSelectConst(d, b, idx) if b == t && (mode & 2) != 0 => {
                    Some((d, Insn::LoadSignalBit(d, sig, idx)))
                }
                _ => None,
            };
            let Some((d, repl)) = fused else { continue };
            // Liveness: the raw loaded value must not be consumed anywhere
            // else in the block. Registers are allocated fresh per value
            // (alloc_reg never reuses ids within a block), so any read of
            // `t` outside the pair consumes THIS load — scan the whole
            // block (not just later pcs) so backward jumps can't smuggle a
            // read of `t` past a suffix-only check. d == t overwrites the
            // raw value in the same pair, making later reads safe.
            if d != t && rr.read_outside(insns, t, &[i, i + 1]) {
                continue;
            }
            rr.remove(&insns[i]);
            rr.remove(&insns[i + 1]);
            rr.add(&repl);
            insns[i] = repl;
            insns[i + 1] = Insn::Nop;
        }

        // Third family — census-driven. The pass above rewrites
        // `LoadSignal;BitSelectConst` into `LoadSignalBit`, leaving a `Nop`
        // where the second instruction was. That newly-created `LoadSignalBit`
        // very often feeds a branch:
        //
        //   LoadSignalBit(d,sig,i) ; [Nop…] ; BranchIfFalse(d,T)
        //       → BranchIfSignalFalse(sig, T, i)
        //
        // On the C906 memcpy census this is the single most frequent adjacent
        // pair (25.4 M, 4.8% of executed instructions) — it is what `if
        // (vec[i])` lowers to. Fusing removes one dispatch and one 32-byte
        // register write per execution. It must run AFTER that pass, since the
        // pair does not exist in the input stream.
        for i in 0..insns.len() {
            let &Insn::LoadSignalBit(d, sig, idx) = &insns[i] else {
                continue;
            };
            if (mode & 8) == 0 {
                continue;
            }
            // Skip the `Nop` placeholders the previous pass just wrote; they
            // are removed by `compact_nops`, so these two really are adjacent
            // in the stream that executes.
            let mut j = i + 1;
            while j < insns.len() && matches!(insns[j], Insn::Nop) {
                j += 1;
            }
            if j >= insns.len() {
                continue;
            }
            // Control must fall through from i to j: nothing in between (nor j
            // itself) may be a branch target, or the fused form would swallow
            // an entry point.
            if (i + 1..=j).any(|k| is_target[k]) {
                continue;
            }
            let Insn::BranchIfFalse(c, t) = insns[j] else {
                continue;
            };
            if c != d {
                continue;
            }
            // The fused form has no destination register, so ANY other read of
            // `d` in the block blocks the fusion.
            if rr.read_outside(insns, d, &[i, j]) {
                continue;
            }
            rr.remove(&insns[i]);
            rr.remove(&insns[j]);
            insns[i] = Insn::BranchIfSignalFalse(sig, t, idx);
            insns[j] = Insn::Nop;
        }
    }

    /// Peephole: collapse the RTL "memory read feeding a flop" idiom
    ///
    ///   LoadSignal(r1, idx_sig)         ; r1 = the array index, from a signal
    ///   LoadArrayElem(r2, array, r1)    ; r2 = array[r1]
    ///   NbaAssign(dst, r2, w)           ; dst <= r2
    ///       → NbaAssignArrayRead(dst, array, idx_sig, w)
    ///
    /// into one instruction, removing two dispatches and two 32-byte VM
    /// register writes per execution.
    ///
    /// The opcode census only proves ADJACENCY; the operand chain is verified
    /// HERE. `LoadArrayElem`'s index register must be exactly the
    /// `LoadSignal`'s destination, `NbaAssign`'s value register exactly the
    /// `LoadArrayElem`'s destination, and — since the fused form writes no
    /// register at all — neither intermediate may be read anywhere else in the
    /// block. Registers are allocated fresh per value (`alloc_reg` never
    /// reuses an id within a block), so any read of `r1`/`r2` outside the
    /// triple consumes THIS chain; the scan covers the whole block, not just
    /// the suffix, so a backward jump cannot smuggle one past it.
    ///
    /// Runs after `elide_redundant_resizes` (a `Resize` that pass is about to
    /// delete would otherwise hide the triple) and before `compact_nops`,
    /// skipping the `Nop` placeholders earlier fusions left behind — those are
    /// removed before execution, so the three really are consecutive in the
    /// stream that runs. `XEZIM_FUSE_ARRNBA=0` disables the pass (A/B escape
    /// hatch).
    /// §5.7.1: `LoadConst` of a FILL literal (`'0`/`'1`/`'x`) immediately
    /// resized to a concrete width becomes one concrete `LoadConst` —
    /// `Value::resize` on a fill replicates the bit and clears `is_fill`.
    /// Fill constants otherwise keep whole blocks off every fast path (the
    /// two-state lowering and both native backends reject them), and the
    /// ibex ALU/controller blocks all start with exactly this pair.
    /// True when any branch in the block targets an earlier pc. Forward-only
    /// blocks (the common case/if lowering) permit LINEAR liveness reasoning:
    /// a register unread after position i is dead there. With a backward
    /// branch, re-entry can reach earlier reads, so callers must fall back to
    /// whole-block read checks.
    pub(super) fn has_backward_branch(insns: &[Insn]) -> bool {
        insns.iter().enumerate().any(|(i, insn)| match insn {
            Insn::BranchIfFalse(_, t)
            | Insn::BranchUnlessZero(_, t)
            | Insn::BranchIfSignalFalse(_, t, _)
            | Insn::CmpBranch(_, _, _, _, t)
            | Insn::CmpBranchConst(_, _, _, _, t)
            | Insn::Jump(t) => (*t as usize) <= i,
            Insn::CaseJump(_, cj) => cj
                .table
                .iter()
                .chain(std::iter::once(&cj.default))
                .any(|&t| (t as usize) <= i),
            Insn::CaseMaskJump(_, mj) => mj
                .table
                .iter()
                .chain(std::iter::once(&mj.xz_path))
                .any(|&t| (t as usize) <= i),
            _ => false,
        })
    }

    /// Producer -> `Move` copy propagation: when a pure register-producing
    /// insn is immediately followed by `Move(d, a)` of its destination and
    /// `a` is never read again, retarget the producer to write `d` directly
    /// and drop the Move. `LoadConst -> Move` alone was 4.9% of all executed
    /// insns on the ibex CoreMark census.
    /// The register an instruction DEFINES outright (its old contents are
    /// irrelevant afterwards). None for stores, branches and waits, which
    /// define nothing, and for the in-place and opaque shapes, which
    /// `in_place_reg` / `is_opaque_insn` classify instead.
    pub(crate) fn dest_reg(insn: &Insn) -> Option<RegId> {
        match insn {
            Insn::LoadConst(d, _)
            | Insn::LoadSignal(d, _)
            | Insn::LoadSignalSigned(d, _)
            | Insn::LoadProcessLocal(d, _)
            | Insn::LoadProcessStatus(d, _)
            | Insn::LoadSignalRange(d, _, _, _)
            | Insn::LoadSignalBit(d, _, _)
            | Insn::LoadSignalRangeDyn(d, _, _, _)
            | Insn::LoadArrayElem(d, _, _)
            | Insn::LoadDynElem(d, _, _)
            | Insn::Pow(d, _, _)
            | Insn::Add(d, _, _)
            | Insn::Sub(d, _, _)
            | Insn::Mul(d, _, _)
            | Insn::Div(d, _, _)
            | Insn::Mod(d, _, _)
            | Insn::BitAnd(d, _, _)
            | Insn::BitOr(d, _, _)
            | Insn::BitXor(d, _, _)
            | Insn::BitXnor(d, _, _)
            | Insn::LogAnd(d, _, _)
            | Insn::LogOr(d, _, _)
            | Insn::Eq(d, _, _)
            | Insn::Neq(d, _, _)
            | Insn::CaseEq(d, _, _)
            | Insn::CasezEq(d, _, _)
            | Insn::CasexEq(d, _, _)
            | Insn::Lt(d, _, _)
            | Insn::Leq(d, _, _)
            | Insn::Gt(d, _, _)
            | Insn::Geq(d, _, _)
            | Insn::Shl(d, _, _)
            | Insn::Shr(d, _, _)
            | Insn::AShr(d, _, _)
            | Insn::BitNot(d, _)
            | Insn::LogNot(d, _)
            | Insn::Negate(d, _)
            | Insn::ReduceAnd(d, _)
            | Insn::ReduceOr(d, _)
            | Insn::ReduceXor(d, _)
            | Insn::Move(d, _)
            | Insn::MoveResize(d, _, _)
            | Insn::Replicate(d, _, _)
            | Insn::BinOpConst(d, _, _, _)
            | Insn::BitSelect(d, _, _)
            | Insn::BitSelectConst(d, _, _)
            | Insn::RangeSelect(d, _, _, _)
            | Insn::RangeSelectW(d, _, _, _, _)
            | Insn::RangeSelectConst(d, _, _, _)
            | Insn::Concat(d, _)
            | Insn::Select(d, _, _, _)
            | Insn::CaseLut(d, _, _)
            | Insn::CmpBranch(_, _, _, d, _) => Some(*d),
            // Four-state execution leaves the dead lowering scratch untouched.
            Insn::CmpBranchConst(..) => None,
            _ => None,
        }
    }

    /// The register an instruction modifies IN PLACE (read and written).
    pub(crate) fn in_place_reg(insn: &Insn) -> Option<RegId> {
        match insn {
            Insn::Resize(a, _)
            | Insn::SetSigned(a)
            | Insn::ClearSigned(a)
            | Insn::DropXz(a)
            | Insn::RealToInt(a) => Some(*a),
            _ => None,
        }
    }

    /// Instructions whose register traffic this file's rewrites do not
    /// model: interpreter escapes, formatting, string ops, and the fused
    /// pair form that only exists after every rewrite has run.
    pub(crate) fn is_opaque_insn(insn: &Insn) -> bool {
        matches!(
            insn,
            Insn::StmtFallback(..)
                | Insn::InitLocalColl(..)
                | Insn::EvalExprFallback(..)
                | Insn::Format(..)
                | Insn::StrOp(..)
                | Insn::BinOpConstAdd2(..)
                | Insn::CallMethod(..)
                | Insn::CallScopedMethod(..)
                | Insn::CallStaticScoped(..)
                | Insn::CallFreeFunction(..)
                | Insn::CallCollMethod(..)
                | Insn::ConstructObject(..)
                | Insn::ForeachNext(..)
                | Insn::Cast(..)
        )
    }

    /// Rewrite every SOURCE operand `from` of `insn` to `to`. False when the
    /// instruction has a source this function does not know how to rewrite,
    /// in which case nothing was changed.
    pub(super) fn replace_read_reg(insn: &mut Insn, from: RegId, to: RegId) -> bool {
        let sub = |r: &mut RegId| {
            if *r == from {
                *r = to;
            }
        };
        match insn {
            Insn::Pow(_, l, r)
            | Insn::Add(_, l, r)
            | Insn::Sub(_, l, r)
            | Insn::Mul(_, l, r)
            | Insn::Div(_, l, r)
            | Insn::Mod(_, l, r)
            | Insn::BitAnd(_, l, r)
            | Insn::BitOr(_, l, r)
            | Insn::BitXor(_, l, r)
            | Insn::BitXnor(_, l, r)
            | Insn::LogAnd(_, l, r)
            | Insn::LogOr(_, l, r)
            | Insn::Eq(_, l, r)
            | Insn::Neq(_, l, r)
            | Insn::CaseEq(_, l, r)
            | Insn::CasezEq(_, l, r)
            | Insn::CasexEq(_, l, r)
            | Insn::Lt(_, l, r)
            | Insn::Leq(_, l, r)
            | Insn::Gt(_, l, r)
            | Insn::Geq(_, l, r)
            | Insn::Shl(_, l, r)
            | Insn::Shr(_, l, r)
            | Insn::AShr(_, l, r)
            | Insn::CmpBranch(_, l, r, _, _)
            | Insn::BitSelect(_, l, r) => {
                sub(l);
                sub(r);
            }
            Insn::BitNot(_, a)
            | Insn::LogNot(_, a)
            | Insn::Negate(_, a)
            | Insn::ReduceAnd(_, a)
            | Insn::ReduceOr(_, a)
            | Insn::ReduceXor(_, a)
            | Insn::Move(_, a)
            | Insn::MoveResize(_, a, _)
            | Insn::Replicate(_, a, _)
            | Insn::BinOpConst(_, a, _, _)
            | Insn::CmpBranchConst(_, a, _, _, _)
            | Insn::BitSelectConst(_, a, _)
            | Insn::RangeSelectConst(_, a, _, _)
            | Insn::CaseLut(_, a, _)
            | Insn::CaseJump(a, _)
            | Insn::CaseMaskJump(a, _)
            | Insn::BranchIfFalse(a, _)
            | Insn::BranchUnlessZero(a, _)
            | Insn::LoadSignalRangeDyn(_, _, a, _)
            | Insn::LoadArrayElem(_, _, a)
            | Insn::LoadDynElem(_, _, a)
            | Insn::NbaAssign(_, a, _)
            | Insn::BlockingAssign(_, a, _)
            | Insn::NbaAssignRange(_, _, _, a)
            | Insn::BlockingAssignRange(_, _, _, a)
            | Insn::BlockingAssignString(_, a)
            | Insn::WaitDelayReg(a) => sub(a),
            Insn::RangeSelect(_, b, l, r) => {
                sub(b);
                sub(l);
                sub(r);
            }
            Insn::RangeSelectW(_, b, i, _, _) => {
                sub(b);
                sub(i);
            }
            Insn::Select(_, c, t, e) => {
                sub(c);
                sub(t);
                sub(e);
            }
            Insn::Concat(_, parts) => parts.iter_mut().for_each(sub),
            Insn::NbaAssignRangeDyn(_, h, l, v) | Insn::BlockingAssignRangeDyn(_, h, l, v) => {
                sub(h);
                sub(l);
                sub(v);
            }
            Insn::NbaAssignBitDyn(_, i, v)
            | Insn::BlockingAssignBitDyn(_, i, v)
            | Insn::NbaAssignArray(_, i, v, _)
            | Insn::BlockingAssignArray(_, i, v, _) => {
                sub(i);
                sub(v);
            }
            Insn::NbaAssignArrayRange(_, i, h, l, v)
            | Insn::BlockingAssignArrayRange(_, i, h, l, v) => {
                sub(i);
                sub(h);
                sub(l);
                sub(v);
            }
            _ => return !Self::insn_reads_reg(insn, from),
        }
        true
    }

    /// The consumer side of a register copy: after `Move(t, v)`, reads of
    /// `t` become reads of `v`, and the Move goes once nothing reads `t`.
    ///
    /// `propagate_copies` folds `producer ; Move` by retargeting the
    /// producer and `forward_move_into_assign` folds `Move ; store`; a loop
    /// variable is neither. Every read of a `for (int b ...)` counter
    /// compiles to a Move of its register into a fresh temporary, so
    /// `sbox_i[(b+m)&63][m]` spent 28% of a DRAM model's executed bytecode
    /// on `Move`. Forwarding covers the straight-line run after the Move:
    /// a read of `t` there, before `t` or `v` is written again, observes
    /// the Move's value whatever the block's branches do elsewhere. A
    /// branch TARGET ends the run, since control arriving there may carry
    /// a different `t`; a branch AWAY does not, its fall-through is still
    /// the same run. The Move itself is deleted only when no read of `t`
    /// survives anywhere in the block — with a backward branch, a read
    /// placed before the Move can still see it.
    pub(super) fn forward_copies(insns: &mut [Insn]) {
        use std::sync::OnceLock;
        static ON: OnceLock<bool> = OnceLock::new();
        if !*ON.get_or_init(|| {
            !matches!(std::env::var("XEZIM_FUSE").as_deref(), Ok("0"))
                && !matches!(std::env::var("XEZIM_FUSE_COPYFWD").as_deref(), Ok("0"))
        }) || insns.len() < 2
        {
            return;
        }
        let is_target = Self::branch_target_map(insns);
        for i in 0..insns.len() - 1 {
            let Insn::Move(md, ms) = insns[i] else {
                continue;
            };
            if md == ms {
                continue;
            }
            let mut j = i + 1;
            // Set when the run redefined `t` outright before any branch
            // could leave it: every path from the Move then reaches that
            // redefinition first, so the Move's value is unobservable and
            // reads of `t` elsewhere in the block do not keep it alive.
            let mut buried = false;
            let mut escaped = false;
            while j < insns.len() && !is_target[j] {
                let ins = &insns[j];
                if Self::is_opaque_insn(ins) {
                    break;
                }
                let ip = Self::in_place_reg(ins);
                if ip == Some(md) || ip == Some(ms) {
                    break;
                }
                // A self-move `Move(r, r)` is an observable-export marker
                // (method result, output formal, static local: the runtime
                // reads `r` after the block). Rewriting its read would turn
                // it into a real copy that this pass then drops as dead, and
                // the export would be lost; it keeps `md` live instead.
                if matches!(ins, Insn::Move(a, b) if a == b && *a == md) {
                    break;
                }
                if Self::insn_reads_reg(ins, md) && !Self::replace_read_reg(&mut insns[j], md, ms) {
                    break;
                }
                let ins = &insns[j];
                let dest = Self::dest_reg(ins);
                if dest == Some(md) {
                    buried = !escaped;
                    break;
                }
                if dest == Some(ms) || matches!(ins, Insn::Jump(_)) {
                    break;
                }
                if matches!(
                    ins,
                    Insn::CmpBranch(..)
                        | Insn::CmpBranchConst(..)
                        | Insn::BranchIfFalse(..)
                        | Insn::BranchUnlessZero(..)
                        | Insn::BranchIfSignalFalse(..)
                        | Insn::CaseJump(..)
                        | Insn::CaseMaskJump(..)
                ) {
                    escaped = true;
                }
                j += 1;
            }
            let live = !buried
                && insns
                    .iter()
                    .enumerate()
                    .any(|(k, x)| k != i && Self::insn_reads_reg(x, md));
            if !live {
                insns[i] = Insn::Nop;
                FUSED_COPY_FWD.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }

    pub(super) fn propagate_copies(insns: &mut [Insn]) {
        if insns.len() < 2 {
            return;
        }
        let mut is_target = vec![false; insns.len() + 1];
        for insn in insns.iter() {
            match insn {
                Insn::CaseJump(_, cj) => {
                    for &t in cj.table.iter().chain(std::iter::once(&cj.default)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::CaseMaskJump(_, mj) => {
                    for &t in mj.table.iter().chain(std::iter::once(&mj.xz_path)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::BranchIfFalse(_, t)
                | Insn::BranchUnlessZero(_, t)
                | Insn::BranchIfSignalFalse(_, t, _)
                | Insn::CmpBranch(_, _, _, _, t)
                | Insn::CmpBranchConst(_, _, _, _, t)
                | Insn::Jump(t)
                    if (*t as usize) < is_target.len() =>
                {
                    is_target[*t as usize] = true;
                }
                _ => {}
            }
        }
        let fwd_only = !Self::has_backward_branch(insns);
        // The producer's destination register, for the pure single-dest
        // shapes this pass retargets. Anything with side effects, in-place
        // semantics (`Resize`, `SetSigned`, ...) or multiple outputs is not
        // listed and never rewritten.
        fn producer_dest(insn: &mut Insn) -> Option<&mut u16> {
            match insn {
                Insn::LoadConst(d, _)
                | Insn::LoadSignal(d, _)
                | Insn::LoadSignalSigned(d, _)
                | Insn::LoadSignalBit(d, _, _)
                | Insn::LoadSignalRange(d, _, _, _)
                | Insn::LoadArrayElem(d, _, _)
                | Insn::LoadDynElem(d, _, _)
                | Insn::Add(d, _, _)
                | Insn::Sub(d, _, _)
                | Insn::Mul(d, _, _)
                | Insn::BitAnd(d, _, _)
                | Insn::BitOr(d, _, _)
                | Insn::BitXor(d, _, _)
                | Insn::BitXnor(d, _, _)
                | Insn::BitNot(d, _)
                | Insn::Negate(d, _)
                | Insn::LogAnd(d, _, _)
                | Insn::LogOr(d, _, _)
                | Insn::LogNot(d, _)
                | Insn::Eq(d, _, _)
                | Insn::Neq(d, _, _)
                | Insn::CaseEq(d, _, _)
                | Insn::CasezEq(d, _, _)
                | Insn::CasexEq(d, _, _)
                | Insn::Lt(d, _, _)
                | Insn::Leq(d, _, _)
                | Insn::Gt(d, _, _)
                | Insn::Geq(d, _, _)
                | Insn::Shl(d, _, _)
                | Insn::Shr(d, _, _)
                | Insn::AShr(d, _, _)
                | Insn::ReduceAnd(d, _)
                | Insn::ReduceOr(d, _)
                | Insn::ReduceXor(d, _)
                | Insn::Select(d, _, _, _)
                | Insn::Concat(d, _)
                | Insn::Replicate(d, _, _)
                | Insn::BinOpConst(d, _, _, _)
                | Insn::BitSelect(d, _, _)
                | Insn::BitSelectConst(d, _, _)
                | Insn::RangeSelect(d, _, _, _)
                | Insn::RangeSelectW(d, _, _, _, _)
                | Insn::RangeSelectConst(d, _, _, _)
                | Insn::CaseLut(d, _, _) => Some(d),
                _ => None,
            }
        }
        for i in 0..insns.len() - 1 {
            let Insn::Move(md, ms) = insns[i + 1] else {
                continue;
            };
            if is_target[i + 1] || md == ms {
                continue;
            }
            // The Move must be `ms`'s ONLY reader anywhere in the block —
            // not just after it: a backward branch can re-enter code before
            // the producer, so a linear "dead after" scan is not a liveness
            // proof. The producer must also not read `md` (its dest write
            // would clobber an operand) nor its own dest `ms` (an in-place
            // accumulate shape would change meaning once retargeted).
            if Self::insn_reads_reg(&insns[i], md) || Self::insn_reads_reg(&insns[i], ms) {
                continue;
            }
            let dead = if fwd_only {
                insns[i + 2..].iter().all(|x| !Self::insn_reads_reg(x, ms))
            } else {
                insns
                    .iter()
                    .enumerate()
                    .all(|(k, x)| k == i + 1 || !Self::insn_reads_reg(x, ms))
            };
            if !dead {
                continue;
            }
            let Some(dref) = producer_dest(&mut insns[i]) else {
                continue;
            };
            if *dref != ms {
                continue;
            }
            *dref = md;
            insns[i + 1] = Insn::Nop;
        }
    }

    pub(super) fn fold_fill_const_resize(insns: &mut [Insn]) {
        if insns.len() < 2 {
            return;
        }
        let mut is_target = vec![false; insns.len() + 1];
        for insn in insns.iter() {
            match insn {
                Insn::CaseJump(_, cj) => {
                    for &t in cj.table.iter().chain(std::iter::once(&cj.default)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::CaseMaskJump(_, mj) => {
                    for &t in mj.table.iter().chain(std::iter::once(&mj.xz_path)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::BranchIfFalse(_, t)
                | Insn::BranchUnlessZero(_, t)
                | Insn::BranchIfSignalFalse(_, t, _)
                | Insn::CmpBranch(_, _, _, _, t)
                | Insn::CmpBranchConst(_, _, _, _, t)
                | Insn::Jump(t)
                    if (*t as usize) < is_target.len() =>
                {
                    is_target[*t as usize] = true;
                }
                _ => {}
            }
        }
        for i in 0..insns.len() - 1 {
            let Insn::LoadConst(d, v) = &insns[i] else {
                continue;
            };
            if !v.is_fill || v.width > 64 {
                continue;
            }
            let d = *d;
            // The consumer must be the direct successor and not a jump
            // target (a branch landing on it would see the unfolded
            // constant), and — for the width-adapting folds — the register
            // must have no OTHER reader, since folding fixes its width.
            if is_target[i + 1] {
                continue;
            }
            let single_use = insns[i + 2..].iter().all(|x| !Self::insn_reads_reg(x, d));
            match &insns[i + 1] {
                Insn::Resize(rd, w) if *rd == d && *w <= 64 => {
                    let folded = v.resize(*w);
                    insns[i] = Insn::LoadConst(d, Box::new(folded));
                    insns[i + 1] = Insn::Nop;
                }
                // Fill flowing straight into a store: the store's width IS
                // the §5.7.1 consuming context.
                Insn::BlockingAssign(_, r, w) | Insn::NbaAssign(_, r, w)
                    if *r == d && *w <= 64 && single_use =>
                {
                    let folded = v.resize(*w);
                    insns[i] = Insn::LoadConst(d, Box::new(folded));
                }
                // Reductions read the fill at its own (1-bit) width; the
                // fold just clears is_fill.
                Insn::ReduceXor(_, r) | Insn::ReduceOr(_, r) | Insn::ReduceAnd(_, r)
                    if *r == d && single_use =>
                {
                    let folded = v.resize(v.width.max(1));
                    insns[i] = Insn::LoadConst(d, Box::new(folded));
                }
                _ => {}
            }
        }
    }

    /// Late pair fusions (census-driven; run last, after every fact-based
    /// pass, so no other pass has to understand the fused forms):
    ///   cmp(d,l,r) ; BranchIfFalse(d,T)   -> CmpBranch(kind,l,r,d,T)
    ///   EqC|CaseEqC(d,s,K) ; BranchIfFalse(d,T) -> CmpBranchConst(kind,s,K,d,T)
    ///   Move(d,s)  ; Resize(d,w)          -> MoveResize(d,s,w)
    ///   Resize(a,w); Move(d,a) [a dead]   -> MoveResize(d,a,w)
    /// `Lt -> BranchIfFalse` alone was 2.7% and `Resize -> Move` 3.1% of all
    /// executed insns on the ibex CoreMark census.
    pub(super) fn fuse_cmp_branch_move_resize(insns: &mut [Insn]) {
        use std::sync::OnceLock;
        static ON: OnceLock<bool> = OnceLock::new();
        if !*ON.get_or_init(|| !matches!(std::env::var("XEZIM_FUSE").as_deref(), Ok("0")))
            || insns.len() < 2
        {
            return;
        }
        static CONST_ON: OnceLock<bool> = OnceLock::new();
        let const_on =
            *CONST_ON.get_or_init(|| std::env::var_os("XEZIM_CMP_BRANCH_CONST_OFF").is_none());
        let mut is_target = vec![false; insns.len() + 1];
        for insn in insns.iter() {
            match insn {
                Insn::CaseJump(_, cj) => {
                    for &t in cj.table.iter().chain(std::iter::once(&cj.default)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::CaseMaskJump(_, mj) => {
                    for &t in mj.table.iter().chain(std::iter::once(&mj.xz_path)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::BranchIfFalse(_, t)
                | Insn::BranchUnlessZero(_, t)
                | Insn::BranchIfSignalFalse(_, t, _)
                | Insn::CmpBranch(_, _, _, _, t)
                | Insn::CmpBranchConst(_, _, _, _, t)
                | Insn::Jump(t)
                    if (*t as usize) < is_target.len() =>
                {
                    is_target[*t as usize] = true;
                }
                _ => {}
            }
        }
        let mut liveness = RegLiveness::new(insns.len());
        for i in 0..insns.len() - 1 {
            if matches!(insns[i], Insn::Nop) {
                continue;
            }
            // Find next non-Nop instruction:
            let mut j = i + 1;
            while j < insns.len() && matches!(insns[j], Insn::Nop) && !is_target[j] {
                j += 1;
            }
            if j >= insns.len() || is_target[j] {
                continue;
            }
            // cmp ; BranchIfFalse — the compare's dest must have no reader
            // other than the branch before its next definition, on either
            // successor path. A branch can bypass a textual redefinition.
            if let Insn::BranchIfFalse(c, t) = insns[j] {
                if const_on
                    && matches!(
                        insns[i],
                        Insn::BinOpConst(d, _, _, BinOpConstKind::Eq | BinOpConstKind::CaseEq)
                            if d == c
                    )
                    && !liveness.read_before_write(insns, &[j + 1, t as usize], c)
                {
                    let Insn::BinOpConst(_, src, k, kind) =
                        std::mem::replace(&mut insns[i], Insn::Nop)
                    else {
                        unreachable!("matched constant comparison")
                    };
                    insns[i] = Insn::CmpBranchConst(kind, src, k, c, t);
                    insns[j] = Insn::Nop;
                    continue;
                }
                let kind = match &insns[i] {
                    Insn::Eq(d, ..) if *d == c => Some(CmpKind::Eq),
                    Insn::Neq(d, ..) if *d == c => Some(CmpKind::Neq),
                    Insn::CaseEq(d, ..) if *d == c => Some(CmpKind::CaseEq),
                    Insn::Lt(d, ..) if *d == c => Some(CmpKind::Lt),
                    Insn::Leq(d, ..) if *d == c => Some(CmpKind::Leq),
                    Insn::Gt(d, ..) if *d == c => Some(CmpKind::Gt),
                    Insn::Geq(d, ..) if *d == c => Some(CmpKind::Geq),
                    _ => None,
                };
                if let Some(kind) = kind {
                    let only_reader = !liveness.read_before_write(insns, &[j + 1, t as usize], c);
                    if only_reader {
                        let (l, r) = match &insns[i] {
                            Insn::Eq(_, l, r)
                            | Insn::Neq(_, l, r)
                            | Insn::CaseEq(_, l, r)
                            | Insn::Lt(_, l, r)
                            | Insn::Leq(_, l, r)
                            | Insn::Gt(_, l, r)
                            | Insn::Geq(_, l, r) => (*l, *r),
                            _ => unreachable!(),
                        };
                        insns[i] = Insn::CmpBranch(kind, l, r, c, t);
                        insns[j] = Insn::Nop;
                        continue;
                    }
                }
            }
            // Move ; Resize of the same dest.
            if let (&Insn::Move(d, sr), &Insn::Resize(rd, w)) = (&insns[i], &insns[j]) {
                if rd == d && d != sr {
                    insns[i] = Insn::MoveResize(d, sr, w);
                    insns[j] = Insn::Nop;
                    continue;
                }
            }
            // Resize ; Move where the resized register dies at the Move:
            // the fused form reads the PRE-resize value and resizes it into
            // the Move's dest — identical result, and `a` stays stale-but-dead.
            if let (&Insn::Resize(a, w), &Insn::Move(d, ms)) = (&insns[i], &insns[j]) {
                if ms == a && d != a {
                    // The Resize itself reads `a` in place (the pre-resize
                    // value the fused form also reads), so it is excluded
                    // like the Move; counting it kept every loop body's
                    // `i = i + 1` on three instructions.
                    let only_reader = !liveness.read_before_write(insns, &[j + 1], a);
                    if only_reader {
                        insns[i] = Insn::MoveResize(d, a, w);
                        insns[j] = Insn::Nop;
                        continue;
                    }
                }
            }
        }
    }

    pub(super) fn fuse_array_read_nba(insns: &mut [Insn]) {
        use std::sync::OnceLock;
        static ON: OnceLock<bool> = OnceLock::new();
        if !*ON.get_or_init(|| !matches!(std::env::var("XEZIM_FUSE_ARRNBA").as_deref(), Ok("0")))
            || insns.len() < 3
        {
            return;
        }
        // Branch targets: fusing must not change what a jump lands on.
        let mut is_target = vec![false; insns.len() + 1];
        for insn in insns.iter() {
            match insn {
                Insn::CaseJump(_, cj) => {
                    for &t in cj.table.iter().chain(std::iter::once(&cj.default)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::CaseMaskJump(_, mj) => {
                    for &t in mj.table.iter().chain(std::iter::once(&mj.xz_path)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::BranchIfFalse(_, t)
                | Insn::BranchUnlessZero(_, t)
                | Insn::BranchIfSignalFalse(_, t, _)
                | Insn::CmpBranch(_, _, _, _, t)
                | Insn::CmpBranchConst(_, _, _, _, t)
                | Insn::Jump(t)
                    if (*t as usize) < is_target.len() =>
                {
                    is_target[*t as usize] = true;
                }
                _ => {}
            }
        }
        // Index of the next instruction that survives `compact_nops`.
        fn next_real(insns: &[Insn], from: usize) -> Option<usize> {
            let mut j = from;
            while j < insns.len() && matches!(insns[j], Insn::Nop) {
                j += 1;
            }
            (j < insns.len()).then_some(j)
        }
        for i in 0..insns.len() - 2 {
            let &Insn::LoadSignal(r1, idx_sig) = &insns[i] else {
                continue;
            };
            let Some(k) = next_real(insns, i + 1) else {
                continue;
            };
            let Insn::LoadArrayElem(r2, _, elem_idx_reg) = &insns[k] else {
                continue;
            };
            let (r2, elem_idx_reg) = (*r2, *elem_idx_reg);
            if elem_idx_reg != r1 {
                continue;
            }
            let Some(j) = next_real(insns, k + 1) else {
                continue;
            };
            let &Insn::NbaAssign(dst, val_reg, width) = &insns[j] else {
                continue;
            };
            if val_reg != r2 {
                continue;
            }
            // Control must fall through i → k → j: no branch may land on
            // anything from i+1 through j, or the fused form would swallow an
            // entry point.
            if (i + 1..=j).any(|x| is_target[x]) {
                continue;
            }
            let consumed = insns.iter().enumerate().any(|(x, ins)| {
                x != i
                    && x != k
                    && x != j
                    && (Self::insn_reads_reg(ins, r1) || Self::insn_reads_reg(ins, r2))
            });
            if consumed {
                continue;
            }
            // Take the boxed operand out of the `LoadArrayElem` rather than
            // cloning its name `String`.
            let Insn::LoadArrayElem(_, array, _) = std::mem::replace(&mut insns[k], Insn::Nop)
            else {
                unreachable!("just matched LoadArrayElem")
            };
            insns[i] = Insn::NbaAssignArrayRead(dst, array, idx_sig, width);
            insns[j] = Insn::Nop;
            FUSED_ARRAY_READ_NBA.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Peephole: absorb a constant load into the ALU op that consumes it.
    ///
    ///   LoadConst(c, K)          ; c = K
    ///   Add|Eq|CaseEq(d, l, c)   ; d = l <op> c
    ///       → BinOpConst(d, l, K, kind)
    ///
    /// `LoadConst` is the #2 opcode on the C906 memcpy census (49.7 M, 12.0%
    /// of executed bytecode) and 32.5 M of those — 7.9% of the whole stream —
    /// feed exactly these three operators. Each fusion removes one dispatch
    /// and one 32-byte VM register write. It also dissolves the `Add;LoadConst`
    /// pairs of an address-increment chain, whose `LoadConst` half is the same
    /// instruction seen from the other side.
    ///
    /// Only the RIGHT operand is fused, and that costs nothing: the compiler
    /// emits the left operand's code, then the right's, then the operator, so
    /// an IMMEDIATELY PRECEDING `LoadConst` is by construction the right
    /// operand. (A left-hand constant has the right operand's code in between
    /// and so is not an adjacent pair at all.) `l == c` — both operands the
    /// same constant register — is rejected, since the fused form no longer
    /// loads `c` for the left side to read.
    ///
    /// The census only proves ADJACENCY; the operand chain is verified HERE:
    /// the operator's right register must be exactly the `LoadConst`'s
    /// destination, and — since the fused form does not write `c` — `c` must
    /// not be read anywhere else in the block. Registers are allocated fresh
    /// per value (`alloc_reg` never reuses an id within a block), so any read
    /// of `c` outside the pair consumes THIS constant; the scan covers the
    /// whole block, not just the suffix, so a backward jump cannot smuggle one
    /// past it.
    ///
    /// Runs after `elide_redundant_resizes` — a `Resize` that pass is about to
    /// delete would otherwise hide the pair, and a `Resize` it KEEPS must
    /// still block the fusion (only `Nop`s are skipped) because the constant
    /// would then need resizing before use. Before `compact_nops`, which
    /// removes the `Nop`s left behind. `XEZIM_FUSE_CONST=0` disables the pass
    /// (A/B escape hatch).
    /// Register constant propagation. A `BinOpConst` whose source register
    /// holds a compile-time constant (a `LoadConst`, or an earlier fold)
    /// becomes a `LoadConst` of the result, computed with the SAME `Value`
    /// operators the interpreter applies, so nothing observable changes. A
    /// dynamic bit select or bit store whose index register is then constant
    /// takes its static form, and constant registers nothing reads afterwards
    /// are dropped. Knowledge is reset at every control-flow boundary.
    ///
    /// Why: an unrolled `i = 0; …; bus[i] = v; i = i + 1; …` decode block on
    /// the C906 compiled to 230 chained `+1`/`+5`/`+64` constant adds feeding
    /// 20 dynamic bit writes — 250 M interpreter instructions per CoreMark
    /// run doing arithmetic on numbers known at compile time.
    // Tests exercise the persisted default directly; production selects an
    // explicit mask for persisted bytecode or the lowering's private copy.
    #[cfg(test)]
    pub(super) fn fold_const_regs(insns: &mut Vec<Insn>, signal_widths: &[u32]) {
        Self::fold_const_regs_masked(insns, signal_widths, FOLD_STORES_PERSISTED);
    }

    /// `fold_const_regs` with an explicit store-fold mask (see
    /// `XEZIM_FOLD_STORES`). The array element folds (2, 4) are applied only
    /// on the two-state lowering's private copy of a stream: as ARRAY writes
    /// the edge-skip census arms the block on the element, so a sibling
    /// writing an overlapping slice re-fires it and it may still skip idle
    /// edges; as plain range stores on the element signal the census has to
    /// reject the overlap (147 C910 flops lost their skip).
    pub(crate) fn fold_const_regs_masked(insns: &mut Vec<Insn>, signal_widths: &[u32], mask: u32) {
        use std::sync::OnceLock;
        static ON: OnceLock<bool> = OnceLock::new();
        if !*ON
            .get_or_init(|| !matches!(std::env::var("XEZIM_FOLD_CONST_REGS").as_deref(), Ok("0")))
            || insns.is_empty()
        {
            return;
        }
        let n = insns.len();
        let mut is_target = vec![false; n + 1];
        for insn in insns.iter() {
            match insn {
                Insn::CaseJump(_, cj) => {
                    for &t in cj.table.iter().chain(std::iter::once(&cj.default)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::CaseMaskJump(_, mj) => {
                    for &t in mj.table.iter().chain(std::iter::once(&mj.xz_path)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::BranchIfFalse(_, t)
                | Insn::BranchUnlessZero(_, t)
                | Insn::BranchIfSignalFalse(_, t, _)
                | Insn::CmpBranch(_, _, _, _, t)
                | Insn::CmpBranchConst(_, _, _, _, t)
                | Insn::Jump(t)
                    if (*t as usize) < is_target.len() =>
                {
                    is_target[*t as usize] = true;
                }
                _ => {}
            }
        }
        let fold_stores = mask;
        // Static index from a constant register, mirroring `dyn_bit_index`:
        // an X/Z index drops the write at run time, so it stays dynamic.
        let const_index = |v: &Value| -> Option<u32> {
            if v.has_xz() {
                return None;
            }
            let u = v.to_u64()?;
            if u > u32::MAX as u64 {
                return None;
            }
            Some(u as u32)
        };
        let mut known: HashMap<RegId, Value> = HashMap::default();
        let mut folded = 0usize;
        for i in 0..n {
            if is_target[i] {
                known.clear();
            }
            // Register the instruction defines (None: defines no register).
            // Anything not listed clears all knowledge.
            let dest: Option<Option<RegId>> = match &insns[i] {
                // A fallback carrying locals writes them back: every register
                // it names is redefined.
                Insn::StmtFallback(p) if !p.2.is_empty() => None,
                Insn::EvalExprFallback(p, _, _) if !p.2.is_empty() => None,
                Insn::LoadConst(d, _)
                | Insn::LoadSignal(d, _)
                | Insn::LoadSignalSigned(d, _)
                | Insn::LoadProcessLocal(d, _)
                | Insn::LoadProcessStatus(d, _)
                | Insn::Resize(d, _)
                | Insn::Add(d, _, _)
                | Insn::Sub(d, _, _)
                | Insn::Mul(d, _, _)
                | Insn::Div(d, _, _)
                | Insn::Mod(d, _, _)
                | Insn::Pow(d, _, _)
                | Insn::BitAnd(d, _, _)
                | Insn::BitOr(d, _, _)
                | Insn::BitXor(d, _, _)
                | Insn::BitXnor(d, _, _)
                | Insn::LogAnd(d, _, _)
                | Insn::LogOr(d, _, _)
                | Insn::Eq(d, _, _)
                | Insn::Neq(d, _, _)
                | Insn::CaseEq(d, _, _)
                | Insn::CasezEq(d, _, _)
                | Insn::CasexEq(d, _, _)
                | Insn::Lt(d, _, _)
                | Insn::Leq(d, _, _)
                | Insn::Gt(d, _, _)
                | Insn::Geq(d, _, _)
                | Insn::Shl(d, _, _)
                | Insn::Shr(d, _, _)
                | Insn::AShr(d, _, _)
                | Insn::CaseLut(d, _, _)
                | Insn::MoveResize(d, _, _)
                | Insn::Format(d, _)
                | Insn::StrOp(d, _, _)
                | Insn::BitNot(d, _)
                | Insn::LogNot(d, _)
                | Insn::Negate(d, _)
                | Insn::ReduceAnd(d, _)
                | Insn::ReduceOr(d, _)
                | Insn::ReduceXor(d, _)
                | Insn::BitSelect(d, _, _)
                | Insn::BitSelectConst(d, _, _)
                | Insn::RangeSelect(d, _, _, _)
                | Insn::RangeSelectW(d, _, _, _, _)
                | Insn::RangeSelectConst(d, _, _, _)
                | Insn::Concat(d, _)
                | Insn::Replicate(d, _, _)
                | Insn::Select(d, _, _, _)
                | Insn::LoadArrayElem(d, _, _)
                | Insn::LoadDynElem(d, _, _)
                | Insn::Move(d, _)
                | Insn::EvalExprFallback(_, d, _)
                | Insn::SetSigned(d)
                | Insn::ClearSigned(d)
                | Insn::DropXz(d)
                | Insn::RealToInt(d)
                | Insn::LoadSignalRange(d, _, _, _)
                | Insn::LoadSignalBit(d, _, _)
                | Insn::BinOpConst(d, _, _, _) => Some(Some(*d)),
                Insn::NbaAssign(..)
                | Insn::NbaAssignRange(..)
                | Insn::NbaAssignRangeDyn(..)
                | Insn::NbaAssignBitDyn(..)
                | Insn::NbaAssignConst(..)
                | Insn::NbaAssignArray(..)
                | Insn::NbaAssignArrayRange(..)
                | Insn::NbaAssignArrayRead(..)
                | Insn::BlockingAssign(..)
                | Insn::BlockingAssignRange(..)
                | Insn::BlockingAssignRangeDyn(..)
                | Insn::BlockingAssignBitDyn(..)
                | Insn::BlockingAssignArray(..)
                | Insn::BlockingAssignArrayRange(..)
                | Insn::BlockingAssignString(..)
                | Insn::StmtFallback(..)
                | Insn::InitLocalColl(..)
                | Insn::WaitDelayReg(..)
                | Insn::WaitEdge(..)
                | Insn::Nop => Some(None),
                _ => None,
            };
            // Fold / rewrite before the definition is applied.
            match &insns[i] {
                Insn::BinOpConst(d, src, k, kind) => {
                    if let Some(v) = known.get(src) {
                        let r = match kind {
                            BinOpConstKind::Add => v.add(k),
                            BinOpConstKind::Eq => v.is_equal(k),
                            BinOpConstKind::CaseEq => v.case_eq(k),
                            BinOpConstKind::Xor => v.bitwise_xor(k),
                            BinOpConstKind::And => v.bitwise_and(k),
                            BinOpConstKind::Or => v.bitwise_or(k),
                            BinOpConstKind::Mul => v.mul(k),
                            BinOpConstKind::Sub => v.sub(k),
                        };
                        let d = *d;
                        insns[i] = Insn::LoadConst(d, Box::new(r.clone()));
                        known.insert(d, r);
                        folded += 1;
                        continue;
                    }
                }
                Insn::BlockingAssignBitDyn(sig, idx, val) => {
                    if let Some(u) = known.get(idx).and_then(|v| const_index(v)) {
                        let w = signal_widths.get(*sig as usize).copied().unwrap_or(0);
                        if u < w {
                            insns[i] = Insn::BlockingAssignRange(*sig, u, u, *val);
                            folded += 1;
                        }
                    }
                }
                Insn::NbaAssignBitDyn(sig, idx, val) => {
                    if let Some(u) = known.get(idx).and_then(|v| const_index(v)) {
                        let w = signal_widths.get(*sig as usize).copied().unwrap_or(0);
                        if u < w {
                            insns[i] = Insn::NbaAssignRange(*sig, u, u, *val);
                            folded += 1;
                        }
                    }
                }
                Insn::BitSelect(d, base, idx) => {
                    if let Some(u) = known.get(idx).and_then(|v| const_index(v)) {
                        insns[i] = Insn::BitSelectConst(*d, *base, u);
                        folded += 1;
                    }
                }
                // A range store whose bounds were computed from parameters
                // and only became constants here: the constant form lowers
                // to two-state, the dynamic one bailed every such flop on
                // two SoCs (16 on c906, 25 on c910).
                Insn::BlockingAssignRangeDyn(sig, h, l, val)
                | Insn::NbaAssignRangeDyn(sig, h, l, val)
                    if fold_stores & 1 != 0 =>
                {
                    let kh = known.get(h).and_then(|v| const_index(v));
                    let kl = known.get(l).and_then(|v| const_index(v));
                    if let (Some(uh), Some(ul)) = (kh, kl) {
                        let w = signal_widths.get(*sig as usize).copied().unwrap_or(0);
                        let (hi, lo) = (uh.max(ul), uh.min(ul));
                        if hi < w {
                            insns[i] = if matches!(insns[i], Insn::NbaAssignRangeDyn(..)) {
                                Insn::NbaAssignRange(*sig, hi, lo, *val)
                            } else {
                                Insn::BlockingAssignRange(*sig, hi, lo, *val)
                            };
                            folded += 1;
                        }
                    }
                }
                // A constant element of a dense array is a signal of its own:
                // `mem[K][hi:lo] <= v` is a range store on that signal (376
                // c910 flops bailed on the array form), and `mem[K] <= v` a
                // whole store when the widths agree.
                Insn::BlockingAssignArrayRange(arr, idx, h, l, val)
                | Insn::NbaAssignArrayRange(arr, idx, h, l, val)
                    if fold_stores & 2 != 0 =>
                {
                    let ArrayOperand::Dense {
                        first_id,
                        lo: alo,
                        hi: ahi,
                        ..
                    } = arr.as_ref()
                    else {
                        continue;
                    };
                    let ki = known.get(idx).and_then(|v| const_index(v));
                    let kh = known.get(h).and_then(|v| const_index(v));
                    let kl = known.get(l).and_then(|v| const_index(v));
                    if let (Some(ui), Some(uh), Some(ul)) = (ki, kh, kl) {
                        let ui = ui as i64;
                        if ui >= *alo && ui <= *ahi {
                            let elem = first_id + (ui - alo) as usize;
                            let w = signal_widths.get(elem).copied().unwrap_or(0);
                            let (hi, lo) = (uh.max(ul), uh.min(ul));
                            if hi < w {
                                let nba = matches!(insns[i], Insn::NbaAssignArrayRange(..));
                                insns[i] = if nba {
                                    Insn::NbaAssignRange(elem as SigId, hi, lo, *val)
                                } else {
                                    Insn::BlockingAssignRange(elem as SigId, hi, lo, *val)
                                };
                                folded += 1;
                            }
                        }
                    }
                }
                Insn::BlockingAssignArray(arr, idx, val, w)
                | Insn::NbaAssignArray(arr, idx, val, w)
                    if fold_stores & 4 != 0 =>
                {
                    let ArrayOperand::Dense {
                        first_id,
                        lo: alo,
                        hi: ahi,
                        ..
                    } = arr.as_ref()
                    else {
                        continue;
                    };
                    if let Some(ui) = known.get(idx).and_then(|v| const_index(v)) {
                        let ui = ui as i64;
                        if ui >= *alo && ui <= *ahi {
                            let elem = first_id + (ui - alo) as usize;
                            if signal_widths.get(elem).copied() == Some(*w) {
                                let nba = matches!(insns[i], Insn::NbaAssignArray(..));
                                insns[i] = if nba {
                                    Insn::NbaAssign(elem as SigId, *val, *w)
                                } else {
                                    Insn::BlockingAssign(elem as SigId, *val, *w)
                                };
                                folded += 1;
                            }
                        }
                    }
                }
                // `[l:r]` with both bounds folded to non-negative 32-bit
                // constants is exactly `RangeSelectConst` (the dynamic form
                // only differs for a negative low bound, which takes the
                // signed x-fill path). The const form lowers to two-state;
                // the dynamic one bails the whole block.
                Insn::RangeSelect(d, base, l, r) => {
                    let kl = known.get(l).and_then(|v| const_index(v));
                    let kr = known.get(r).and_then(|v| const_index(v));
                    if let (Some(ul), Some(ur)) = (kl, kr) {
                        if ul <= i32::MAX as u32 && ur <= i32::MAX as u32 {
                            insns[i] = Insn::RangeSelectConst(*d, *base, ul.max(ur), ul.min(ur));
                            folded += 1;
                        }
                    }
                }
                _ => {}
            }
            match dest {
                Some(Some(d)) => {
                    if let Insn::LoadConst(_, k) = &insns[i] {
                        known.insert(d, (**k).clone());
                    } else {
                        known.remove(&d);
                    }
                }
                Some(None) => {}
                None => known.clear(),
            }
            if matches!(
                insns[i],
                Insn::Jump(..)
                    | Insn::BranchIfFalse(..)
                    | Insn::BranchUnlessZero(..)
                    | Insn::BranchIfSignalFalse(..)
                    | Insn::CmpBranch(..)
                    | Insn::CmpBranchConst(..)
                    | Insn::CaseJump(..)
                    | Insn::CaseMaskJump(..)
            ) {
                known.clear();
            }
        }
        if folded == 0 {
            return;
        }
        // Drop constant loads nothing reads (the folded chain's temporaries).
        // A `LoadConst` and the `Nop` replacing it read no register, so the
        // counts stay exact through the loop.
        let mut dead = 0usize;
        let rr = RegReaders::new(insns);
        for i in 0..n {
            let Insn::LoadConst(d, _) = &insns[i] else {
                continue;
            };
            let d = *d;
            let read = rr.read_outside(insns, d, &[i]);
            if !read {
                insns[i] = Insn::Nop;
                dead += 1;
            }
        }
        static STATS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if *STATS.get_or_init(|| std::env::var_os("XEZIM_FOLD_CONST_STATS").is_some()) {
            eprintln!("[FOLD-CONST] folded={folded} dead_consts={dead} insns={n}");
        }
    }

    pub(super) fn fuse_binop_const(insns: &mut [Insn]) {
        use std::sync::OnceLock;
        static ON: OnceLock<bool> = OnceLock::new();
        if !*ON.get_or_init(|| !matches!(std::env::var("XEZIM_FUSE_CONST").as_deref(), Ok("0")))
            || insns.len() < 2
        {
            return;
        }
        // Branch targets: fusing must not change what a jump lands on.
        let mut is_target = vec![false; insns.len() + 1];
        for insn in insns.iter() {
            match insn {
                Insn::CaseJump(_, cj) => {
                    for &t in cj.table.iter().chain(std::iter::once(&cj.default)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::CaseMaskJump(_, mj) => {
                    for &t in mj.table.iter().chain(std::iter::once(&mj.xz_path)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::BranchIfFalse(_, t)
                | Insn::BranchUnlessZero(_, t)
                | Insn::BranchIfSignalFalse(_, t, _)
                | Insn::CmpBranch(_, _, _, _, t)
                | Insn::CmpBranchConst(_, _, _, _, t)
                | Insn::Jump(t)
                    if (*t as usize) < is_target.len() =>
                {
                    is_target[*t as usize] = true;
                }
                _ => {}
            }
        }
        let mut rr = RegReaders::new(insns);
        let mut liveness = RegLiveness::new(insns.len());
        for i in 0..insns.len() - 1 {
            let Insn::LoadConst(c, _) = &insns[i] else {
                continue;
            };
            let c = *c;
            // Index of the next instruction that survives `compact_nops`,
            // additionally stepping over `ClearSigned` sign scrubs: a scrub of
            // the CONSTANT's register is absorbed into the boxed constant at
            // fuse time; a scrub of any other register stays in place and
            // still executes before the fused op (which lands at `j`, not `i`).
            let mut j = i + 1;
            let mut const_scrub: Option<usize> = None;
            loop {
                if j >= insns.len() {
                    break;
                }
                match insns[j] {
                    Insn::Nop => j += 1,
                    Insn::ClearSigned(r) => {
                        if r == c {
                            const_scrub = Some(j);
                        }
                        j += 1;
                    }
                    _ => break,
                }
            }
            if j >= insns.len() {
                continue;
            }
            // NOTE (measured 2026-08-25): commutative `l == c` arms were
            // tried here and are UNREACHABLE — operands compile left-first,
            // so a LEFT constant's LoadConst is separated from the op by the
            // right operand's load and never sits in this pass's window. The
            // 76 M adjacent `LoadConst -> CaseEq` census pairs are therefore
            // const-RIGHT shapes rejected by the width conditions below.
            let (d, l, kind) = match insns[j] {
                Insn::Add(d, l, r) if r == c => (d, l, BinOpConstKind::Add),
                Insn::Eq(d, l, r) if r == c => (d, l, BinOpConstKind::Eq),
                Insn::CaseEq(d, l, r) if r == c => (d, l, BinOpConstKind::CaseEq),
                Insn::BitXor(d, l, r) if r == c => (d, l, BinOpConstKind::Xor),
                // Loop address arithmetic: `(b+m) & 63`, `i * 8`, `n - 1`.
                Insn::BitAnd(d, l, r) if r == c => (d, l, BinOpConstKind::And),
                Insn::BitOr(d, l, r) if r == c => (d, l, BinOpConstKind::Or),
                Insn::Mul(d, l, r) if r == c => (d, l, BinOpConstKind::Mul),
                Insn::Sub(d, l, r) if r == c => (d, l, BinOpConstKind::Sub),
                _ => continue,
            };
            // `op(d, c, c)`: the left operand is the constant register too,
            // and the fused form no longer materialises it.
            if l == c {
                continue;
            }
            // Control must fall through i → j: nothing from i+1 through j may
            // be a branch target, or the fused form would swallow an entry
            // point.
            if (i + 1..=j).any(|x| is_target[x]) {
                continue;
            }
            let consumed = match const_scrub {
                Some(sj) => rr.read_outside(insns, c, &[i, j, sj]),
                None => rr.read_outside(insns, c, &[i, j]),
            };
            if consumed && d != c && liveness.read_before_write(insns, &[j + 1], c) {
                continue;
            }
            rr.remove(&insns[j]);
            if let Some(sj) = const_scrub {
                rr.remove(&insns[sj]);
            }
            // Take the boxed constant out of the `LoadConst` rather than
            // cloning a possibly-`Wide` `Value`.
            let Insn::LoadConst(_, mut k) = std::mem::replace(&mut insns[i], Insn::Nop) else {
                unreachable!("just matched LoadConst")
            };
            if let Some(sj) = const_scrub {
                k.is_signed = false;
                insns[sj] = Insn::Nop;
            }
            // The fused op replaces the BINOP's slot so any surviving
            // `ClearSigned` of the left operand still runs first.
            insns[j] = Insn::BinOpConst(d, l, k, kind);
            rr.add(&insns[j]);
            FUSED_BINOP_CONST[kind as usize].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Static width inference over the emitted stream: delete every
    /// `Resize(r, w)` whose register is already provably `w` bits wide.
    ///
    /// The 27 `emit(Insn::Resize(..))` sites are unconditional — the compiler
    /// knows the target width but never asks whether the register already has
    /// it. On the C906 memcpy census 99.7% of the 243 M executed `Resize`es
    /// (10.8% of all bytecode, the second most frequent opcode) found
    /// `vr.width == w` and fell straight through: pure dispatch.
    ///
    /// The exec arms are `if vm_regs[r].width != w { .. }`, so an instruction
    /// this pass removes is one that would have done LITERALLY nothing —
    /// provided the width really does match. That makes the whole pass rest on
    /// a single invariant, and nothing else: whenever `rw[r]` holds
    /// `Some((w, _))`, at run time `vm_regs[r].width` is exactly `w`.
    ///
    /// Every rule below is justified from the `Value` method the matching exec
    /// arm calls, on EVERY path through it (X-propagation, `Wide` storage, the
    /// §5.7.1 fill widening and the `is_real` special cases each return their
    /// own freshly-built `Value`, and they do not all agree). Where a method
    /// can return a width other than the obvious one — `add` on a real operand
    /// is 64 bits, not `max`; `range_select` past `MAX_WIDTH` is clamped — the
    /// rule is dropped or guarded rather than approximated. **Unknown means
    /// keep the `Resize`**: a wrongly deleted one leaves a value at the wrong
    /// width, which in a 4-state simulator corrupts results silently.
    ///
    /// `plain` on a tracked width means additionally `!is_real && !is_fill`,
    /// which is what the arithmetic and `Select` rules need (see below).
    ///
    /// Control flow: any index a branch or jump can land on is a merge point
    /// where a register's width depends on which path arrived, so the table is
    /// cleared there. That covers backward jumps too (a loop head is a target),
    /// at the cost of giving up the first iteration's worth of knowledge inside
    /// loops. `XEZIM_RESIZE_ELIDE=0` disables the pass (A/B escape hatch).
    pub(super) fn elide_redundant_resizes(
        insns: &mut [Insn],
        signal_widths: &[u32],
        signal_real: Option<&[bool]>,
        num_regs: usize,
    ) {
        use std::sync::OnceLock;
        static ENABLED: OnceLock<bool> = OnceLock::new();
        if !*ENABLED
            .get_or_init(|| !matches!(std::env::var("XEZIM_RESIZE_ELIDE").as_deref(), Ok("0")))
        {
            return;
        }
        if insns.is_empty() {
            return;
        }

        /// A width is recorded only inside `1..=MAX_WIDTH`. Below that,
        /// `fill_at` rounds a zero width up to one; above it, `cap_width`
        /// clamps — in both cases the constructed `Value` would not have the
        /// width the rule claims.
        fn ok(width: u32) -> Option<u32> {
            (1..=Value::MAX_WIDTH).contains(&width).then_some(width)
        }
        fn fact(rw: &[Option<(u32, bool)>], r: RegId) -> Option<(u32, bool)> {
            rw.get(r as usize).copied().flatten()
        }
        fn store(rw: &mut [Option<(u32, bool)>], r: RegId, f: Option<(u32, bool)>) {
            if let Some(slot) = rw.get_mut(r as usize) {
                *slot = f;
            }
        }

        let mut is_target = vec![false; insns.len() + 1];
        for insn in insns.iter() {
            match insn {
                Insn::CaseJump(_, cj) => {
                    for &t in cj.table.iter().chain(std::iter::once(&cj.default)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::CaseMaskJump(_, mj) => {
                    for &t in mj.table.iter().chain(std::iter::once(&mj.xz_path)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::BranchIfFalse(_, t)
                | Insn::BranchUnlessZero(_, t)
                | Insn::BranchIfSignalFalse(_, t, _)
                | Insn::CmpBranch(_, _, _, _, t)
                | Insn::CmpBranchConst(_, _, _, _, t)
                | Insn::Jump(t)
                    if (*t as usize) < is_target.len() =>
                {
                    is_target[*t as usize] = true;
                }
                _ => {}
            }
        }

        let mut rw: Vec<Option<(u32, bool)>> = vec![None; num_regs];
        for i in 0..insns.len() {
            if is_target[i] {
                rw.iter_mut().for_each(|f| *f = None);
            }

            // Elide ordinary resizes whose source already has the target width.
            if let &Insn::Resize(r, width) = &insns[i] {
                let target = ok(width);
                let prev = fact(&rw, r);
                if target.is_some() && prev.map(|(pw, _)| pw) == target {
                    // The exec arm's `vr.width != w` test is already false:
                    // dead. The register keeps exactly the fact it had.
                    insns[i] = Insn::Nop;
                } else {
                    // `Value::resize` clears `is_fill` on every path, and
                    // clears `is_real` on every path but one: a real source
                    // resized to exactly 64 is returned by `self.clone()`.
                    let plain = width != 64 || prev.is_some_and(|(_, p)| p);
                    store(&mut rw, r, target.map(|t| (t, plain)));
                }
                continue;
            }

            match &insns[i] {
                // Handled above.
                Insn::Resize(..) => {}

                // Result width varies per entry; drop tracking for the dest.
                Insn::CaseLut(d, ..) => store(&mut rw, *d, None),
                // Class-member load: the width isn't statically known (the
                // member's Value width follows its runtime type) — bare
                // dest store, no width/plain bogus elision.
                Insn::LoadClassMember(d, ..) => store(&mut rw, *d, None),
                // Class-member store defines nothing (only the heap object).
                Insn::StoreClassMember(..) => {}
                // Method call: dest width isn't statically known (callee
                // return width follows its runtime type) — bare dest store,
                // and any writeback argument registers `a..a+n` may be
                // overwritten in place by `vm_drain_arg_temps`.
                Insn::CallMethod(d, _, _, a, n)
                | Insn::CallScopedMethod(d, _, _, a, n) => {
                    store(&mut rw, *d, None);
                    for x in *a..*a + (*n as RegId) {
                        store(&mut rw, x, None);
                    }
                }
                Insn::CallFreeFunction(d, _, a, n)
                | Insn::CallStaticScoped(d, _, a, n)
                | Insn::ConstructObject(d, _, a, n) => {
                    store(&mut rw, *d, None);
                    for x in *a..*a + (*n as RegId) {
                        store(&mut rw, x, None);
                    }
                }
                Insn::LoadClassStatic(d, ..) => store(&mut rw, *d, None),
                // Step 9g: static store defines no SSA value; a fresh
                // object handle has runtime-determined width (bare store).
                Insn::StoreClassStatic(..) => {}
                // Collection builtin: dest width follows the runtime member
                // (count / exists flag / popped element) — bare dest store.
                Insn::CallCollMethod(d, _, _, a, n, _) => {
                    store(&mut rw, *d, None);
                    for x in *a..*a + (*n as RegId) {
                        store(&mut rw, x, None);
                    }
                }
                // Collection element read: width follows the runtime store —
                // bare dest store (like LoadClassMember).
                Insn::LoadCollElem(d, ..) => store(&mut rw, *d, None),
                // Collection element store defines nothing (the storage key
                // is a flat string in `signals`).
                Insn::StoreCollElem(..) => {}
                // Collection-return hand-off defines no SSA value (the
                // result leaves via the side-channel snapshot store).
                Insn::RetCollection(..) => {}
                // The collection-return store writes a collection, not a
                // register (no SSA value defined).
                Insn::AssignPendingColl(..) => {}
                // `$cast`: the 1/0 result's width is statically 32, but the
                // register route also re-defines the dest slot at a runtime
                // width (the frame arm's fit) — drop both from tracking.
                Insn::Cast(c, _, d, ..) => {
                    store(&mut rw, *d, None);
                    if let CastDest::Reg(r) = &c.dest {
                        store(&mut rw, *r, None);
                    }
                }
                // Compiled foreach: ForeachKeys defines only the arena slot
                // (no register); ForeachNext writes the 1/0 ok flag and the
                // loop var at the runtime key width — drop both.
                Insn::ForeachKeys(..) => {}
                Insn::ForeachNext(o, _, v) => {
                    store(&mut rw, *o, None);
                    store(&mut rw, *v, None);
                }
                // Two dests; widths follow operand widths — drop tracking.
                Insn::BinOpConstAdd2(a) => {
                    store(&mut rw, a.d1, None);
                    store(&mut rw, a.d2, None);
                }
                // Control only; defines nothing.
                Insn::CaseJump(..) | Insn::CaseMaskJump(..) => {}
                // Fused compare+branch: the embedded scratch gets the 1-bit
                // compare result; the branch defines nothing else. (This
                // pass runs BEFORE the late fusion pass, so these arms only
                // matter for re-runs over already-fused streams.)
                Insn::CmpBranch(_, _, _, tmp, _) => store(&mut rw, *tmp, Some((1, true))),
                // The constant branch does not write its four-state scratch.
                Insn::CmpBranchConst(..) => {}
                // `dst = resize(src, w)`: width is known, but a same-width
                // source passes through `clone()` (fill/real preserved), so
                // never claim plain-ness — conservative fact, no bad elision.
                Insn::MoveResize(d, _, w) => {
                    store(&mut rw, *d, ok(*w).map(|t| (t, false)));
                }
                // String result: width is the text length, not static.
                Insn::Format(d, ..) => store(&mut rw, *d, None),
                Insn::StrOp(d, ..) => store(&mut rw, *d, None),
                Insn::BlockingAssignString(..) => {}

                // The exec arms clone the boxed `Value` verbatim.
                Insn::LoadConst(d, v) => {
                    let f = ok(v.width).map(|w| (w, !v.is_real && !v.is_fill));
                    store(&mut rw, *d, f);
                }
                // `signal_table[id].clone()`. `is_real` is a property of the
                // signal's DECLARED type, so it is plain exactly when
                // `signal_real[id]` is false. Without that table (no
                // `set_signal_real`) assume possibly-real, which only forgoes
                // elisions. `is_fill` never reaches a stored signal value:
                // `resize`/`resize_for_assign` clear it on the way in.
                Insn::LoadSignal(d, s) | Insn::LoadSignalSigned(d, s) => {
                    let plain = signal_real
                        .and_then(|sr| sr.get(*s as usize).copied())
                        .map(|is_real| !is_real)
                        .unwrap_or(false);
                    let f = signal_widths
                        .get(*s as usize)
                        .copied()
                        .and_then(ok)
                        .map(|w| (w, plain));
                    store(&mut rw, *d, f);
                }
                Insn::LoadProcessLocal(d, _) => store(&mut rw, *d, None),
                Insn::LoadProcessStatus(d, _) => store(&mut rw, *d, Some((32, true))),
                // `Value::bit_select` is 1 bit on every path, including the
                // §11.5.1 out-of-range read.
                Insn::LoadSignalBit(d, _, _)
                | Insn::BitSelect(d, _, _)
                | Insn::BitSelectConst(d, _, _) => store(&mut rw, *d, Some((1, true))),
                // `Value::range_select` is `|l-r|+1` on every path — except
                // `range_select_zext`'s guard against an underflowed index,
                // which returns a bounded all-X value instead; `ok` excludes
                // exactly the widths that can reach it.
                Insn::LoadSignalRangeDyn(d, _, _, w) => {
                    let f = ok(*w).map(|w| (w, true));
                    store(&mut rw, *d, f);
                }
                Insn::LoadSignalRange(d, _, l, r) | Insn::RangeSelectConst(d, _, l, r) => {
                    let f = l
                        .abs_diff(*r)
                        .checked_add(1)
                        .and_then(ok)
                        .map(|w| (w, true));
                    store(&mut rw, *d, f);
                }

                // Every comparison and logical/reduction operator returns
                // `from_u64(_, 1)` or `new(1)` — 1 bit on every path.
                Insn::Eq(d, ..)
                | Insn::Neq(d, ..)
                | Insn::CaseEq(d, ..)
                | Insn::CasezEq(d, ..)
                | Insn::CasexEq(d, ..)
                | Insn::Lt(d, ..)
                | Insn::Leq(d, ..)
                | Insn::Gt(d, ..)
                | Insn::Geq(d, ..)
                | Insn::LogAnd(d, ..)
                | Insn::LogOr(d, ..)
                | Insn::LogNot(d, _)
                | Insn::ReduceAnd(d, _)
                | Insn::ReduceOr(d, _)
                | Insn::ReduceXor(d, _) => store(&mut rw, *d, Some((1, true))),

                // `vm_regs[d] = vm_regs[s].clone()` / `copy_from`, both of
                // which copy `width` verbatim.
                Insn::Move(d, s) => {
                    let f = fact(&rw, *s);
                    store(&mut rw, *d, f);
                }
                // `bitwise_not` keeps `self.width` for both storage variants.
                Insn::BitNot(d, s) => {
                    let f = fact(&rw, *s).map(|(w, _)| (w, true));
                    store(&mut rw, *d, f);
                }
                // `negate` on a REAL returns a 64-bit `from_f64`, not
                // `self.width`, so the source must be known non-real.
                Insn::Negate(d, s) => {
                    let f = fact(&rw, *s).filter(|(_, p)| *p);
                    store(&mut rw, *d, f);
                }
                // All three shift helpers return `self.width` on every path,
                // real and fill operands included.
                Insn::Shl(d, l, _) | Insn::Shr(d, l, _) | Insn::AShr(d, l, _) => {
                    let f = fact(&rw, *l).map(|(w, _)| (w, true));
                    store(&mut rw, *d, f);
                }

                // `bitwise_*` take `max(width)` on every path: the fast arm,
                // the `Wide` arm (entered only for two equal declared widths),
                // `bitwise_op_slow`, and the §5.7.1 fill widening, which
                // normalises both operands to `max(w).max(1)` before
                // recursing. A real operand is not special-cased at all.
                Insn::BitAnd(d, l, r)
                | Insn::BitOr(d, l, r)
                | Insn::BitXor(d, l, r)
                | Insn::BitXnor(d, l, r) => {
                    let f = match (fact(&rw, *l), fact(&rw, *r)) {
                        (Some((a, _)), Some((b, _))) => ok(a.max(b)).map(|w| (w, true)),
                        _ => None,
                    };
                    store(&mut rw, *d, f);
                }
                // The arithmetic operators DO special-case a real operand
                // (`from_f64`, always 64 bits regardless of the operands'
                // widths), so both operands must be known non-real.
                Insn::Add(d, l, r)
                | Insn::Sub(d, l, r)
                | Insn::Mul(d, l, r)
                | Insn::Div(d, l, r)
                | Insn::Mod(d, l, r) => {
                    let f = match (fact(&rw, *l), fact(&rw, *r)) {
                        (Some((a, true)), Some((b, true))) => ok(a.max(b)).map(|w| (w, true)),
                        _ => None,
                    };
                    store(&mut rw, *d, f);
                }

                // Same rules as the unfused pair, with the constant's fact
                // read straight off the boxed `Value` instead of looked up —
                // which is strictly MORE inferable than the register form,
                // since `K` can never be unknown. The `Add` kind reuses the
                // arithmetic rule above verbatim (`max` of the operand widths,
                // both operands required non-real because `Value::add`
                // special-cases a real operand into a 64-bit `from_f64`, and
                // non-fill because §5.7.1 widening renormalises them);
                // `Eq`/`CaseEq` land in the 1-bit comparison rule, since
                // `is_equal`/`case_eq` return `from_u64(_, 1)` on every path.
                Insn::BinOpConst(d, s, k, kind) => {
                    let f = match kind {
                        BinOpConstKind::Eq | BinOpConstKind::CaseEq => Some((1, true)),
                        // `bitwise_xor` has no single-width guarantee worth
                        // proving here — leave the register width unknown.
                        BinOpConstKind::Xor
                        | BinOpConstKind::And
                        | BinOpConstKind::Or
                        | BinOpConstKind::Mul
                        | BinOpConstKind::Sub => None,
                        BinOpConstKind::Add => {
                            // Identical to the `Insn::LoadConst` arm's fact.
                            let kf = ok(k.width).map(|w| (w, !k.is_real && !k.is_fill));
                            match (fact(&rw, *s), kf) {
                                (Some((a, true)), Some((b, true))) => {
                                    ok(a.max(b)).map(|w| (w, true))
                                }
                                _ => None,
                            }
                        }
                    };
                    store(&mut rw, *d, f);
                }

                // `Select` is the one arm that writes registers it does not
                // name as its destination: it widens a §5.7.1 fill branch to
                // the other branch's width IN PLACE before choosing. Only when
                // both branches are known non-fill do they keep their widths —
                // and then all three outcomes (`merge_unknown`, or a clone of
                // either branch) are `max(tw, ew)` wide, which is a single
                // known width when the two agree. Store `dest` last: it may
                // alias a branch register.
                Insn::Select(d, _, t, e) => {
                    let (ft, fe) = (fact(&rw, *t), fact(&rw, *e));
                    let f = match (ft, fe) {
                        (Some((a, true)), Some((b, true))) if a == b => Some((a, true)),
                        _ => {
                            store(&mut rw, *t, ft.filter(|(_, p)| *p));
                            store(&mut rw, *e, fe.filter(|(_, p)| *p));
                            None
                        }
                    };
                    store(&mut rw, *d, f);
                }

                // `concat_refs` (and the exec arms' inline equivalents)
                // return the SUM of the operand widths. An overflowing sum
                // wraps in the exec arm, and one past `MAX_WIDTH` is clamped;
                // `checked_add` and `ok` between them exclude both.
                Insn::Concat(d, parts) => {
                    let mut sum = Some(0u32);
                    for p in parts.iter() {
                        sum = match (sum, fact(&rw, *p)) {
                            (Some(a), Some((b, _))) => a.checked_add(b),
                            _ => None,
                        };
                    }
                    store(&mut rw, *d, sum.and_then(ok).map(|w| (w, true)));
                }
                // `{1{x}}` hands the source `Value` through untouched (the
                // main exec arm does not even copy it when `d == s`), so it
                // inherits the source's fact exactly, `is_fill` included.
                // `{n{x}}` for n >= 2 concatenates n copies; n == 0 is a
                // zero-width value, which `ok` rejects.
                Insn::Replicate(d, s, n) => {
                    let f = if *n == 1 {
                        fact(&rw, *s)
                    } else {
                        fact(&rw, *s)
                            .and_then(|(w, _)| w.checked_mul(*n))
                            .and_then(ok)
                            .map(|w| (w, true))
                    };
                    store(&mut rw, *d, f);
                }

                // Destination width not established here: the bounds of a
                // dynamic range select are register values, and an array
                // element read that fails to resolve returns a 1-bit X
                // instead of the element.
                Insn::RangeSelect(d, _, _, _)
                | Insn::RangeSelectW(d, _, _, _, _)
                | Insn::LoadArrayElem(d, _, _)
                | Insn::LoadDynElem(d, _, _) => store(&mut rw, *d, None),

                // Stamp/clear `is_signed`; storage and width are untouched.
                Insn::SetSigned(_) | Insn::ClearSigned(_) | Insn::DropXz(_) => {}
                // A real becomes a 64-bit integer (its width already); any
                // other value is untouched, so the fact still holds.
                Insn::RealToInt(_) => {}

                // Result width is the (runtime) left operand's width — not
                // statically tracked here.
                Insn::Pow(d, _, _) => store(&mut rw, *d, None),

                // No register destination.
                Insn::WaitDelayReg(..)
                | Insn::WaitEdge(..)
                | Insn::CovHit(..)
                | Insn::BranchIfFalse(..)
                | Insn::BranchUnlessZero(..)
                | Insn::BranchIfSignalFalse(..)
                | Insn::Jump(..)
                | Insn::Nop
                | Insn::NbaAssign(..)
                | Insn::NbaAssignConst(..)
                | Insn::NbaAssignRange(..)
                | Insn::NbaAssignRangeDyn(..)
                | Insn::NbaAssignBitDyn(..)
                | Insn::NbaAssignArray(..)
                | Insn::NbaAssignArrayRange(..)
                | Insn::NbaAssignArrayRead(..)
                | Insn::BlockingAssign(..)
                | Insn::BlockingAssignRange(..)
                | Insn::BlockingAssignRangeDyn(..)
                | Insn::BlockingAssignBitDyn(..)
                | Insn::BlockingAssignArray(..)
                | Insn::BlockingAssignArrayRange(..) => {}

                // The AST interpreter runs with the whole machine in reach.
                Insn::StmtFallback(..) => rw.iter_mut().for_each(|f| *f = None),
                Insn::InitLocalColl(..) => rw.iter_mut().for_each(|f| *f = None),
                Insn::EvalExprFallback(..) => rw.iter_mut().for_each(|f| *f = None),
            }
        }
    }

    /// Drop `Nop`s left behind by pair fusion above, rewriting branch targets.
    ///
    /// Every fusion in this pass replaces a two-instruction pair with one real
    /// instruction plus a `Nop` placeholder, because collapsing the vector
    /// mid-pass would invalidate the indices the loops and `is_target` use.
    /// Those placeholders were never removed afterwards, so each one cost a
    /// dispatch on EVERY execution for the life of the run. On the C906 SoC
    /// running CoreMark they are 15-20% of the instructions in a compiled
    /// continuous assignment (e.g. the second most-executed RHS shape is
    /// `LoadRng,Nop,Resize,Move,Resize,AssignRng` — one of six).
    ///
    /// A branch target is an index into this vector, so removal must remap it.
    /// A target that pointed AT a removed `Nop` moves to the next surviving
    /// instruction, which is exactly where control would have arrived anyway;
    /// `len` (one past the end, used by loop exits) maps to the new length.
    /// Is any instruction after `j` reading `r`? Conservative liveness for
    /// the census-pair peepholes below: any later read — even one past a
    /// redefinition — counts as live, which can only forgo an optimization,
    /// never miscompile one.
    fn reg_read_after(insns: &[Insn], j: usize, r: RegId) -> bool {
        insns[j + 1..].iter().any(|x| Self::insn_reads_reg(x, r))
    }

    /// Branch-target map shared by the census-pair peepholes: rewriting or
    /// deleting an instruction is only sound when control cannot enter the
    /// pattern's interior from elsewhere.
    pub(super) fn branch_target_map(insns: &[Insn]) -> Vec<bool> {
        let mut is_target = vec![false; insns.len() + 1];
        for insn in insns.iter() {
            match insn {
                Insn::CaseJump(_, cj) => {
                    for &t in cj.table.iter().chain(std::iter::once(&cj.default)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::CaseMaskJump(_, mj) => {
                    for &t in mj.table.iter().chain(std::iter::once(&mj.xz_path)) {
                        if (t as usize) < is_target.len() {
                            is_target[t as usize] = true;
                        }
                    }
                }
                Insn::BranchIfFalse(_, t)
                | Insn::BranchUnlessZero(_, t)
                | Insn::BranchIfSignalFalse(_, t, _)
                | Insn::CmpBranch(_, _, _, _, t)
                | Insn::CmpBranchConst(_, _, _, _, t)
                | Insn::Jump(t)
                    if (*t as usize) < is_target.len() =>
                {
                    is_target[*t as usize] = true;
                }
                _ => {}
            }
        }
        is_target
    }

    /// `Move(d, s) ; <assign whose VALUE reg is d>` → assign reads `s`
    /// directly, Move deleted.
    ///
    /// `propagate_copies` above folds the PRODUCER side (`producer ; Move`
    /// retargets the producer), but the c906 opcode census showed the
    /// CONSUMER side alive at scale: `Move -> BlockingAssignRange` was 5.3%
    /// of all executed instructions (337 M dynamic pairs on cmark it1) —
    /// the Move's source is produced far earlier, so the producer-side pass
    /// never sees it. Sound when nothing can jump into the pair's interior
    /// and `d` has no later reader.
    pub(super) fn forward_move_into_assign(insns: &mut [Insn]) {
        use std::sync::OnceLock;
        static ON: OnceLock<bool> = OnceLock::new();
        if !*ON.get_or_init(|| {
            !matches!(std::env::var("XEZIM_FUSE").as_deref(), Ok("0"))
                && !matches!(std::env::var("XEZIM_FUSE_MOVEFWD").as_deref(), Ok("0"))
        }) || insns.len() < 2
            || Self::has_backward_branch(insns)
        {
            return;
        }
        let is_target = Self::branch_target_map(insns);
        for i in 0..insns.len() - 1 {
            let Insn::Move(d, s) = insns[i] else {
                continue;
            };
            if d == s {
                continue;
            }
            // Next surviving instruction; only Nops may sit between, and no
            // branch may land inside (i, j].
            let mut j = i + 1;
            let mut blocked = false;
            while j < insns.len() {
                if is_target[j] {
                    blocked = true;
                    break;
                }
                if !matches!(insns[j], Insn::Nop) {
                    break;
                }
                j += 1;
            }
            if blocked || j >= insns.len() {
                continue;
            }
            let vref = match &mut insns[j] {
                Insn::BlockingAssign(_, v, _)
                | Insn::NbaAssign(_, v, _)
                | Insn::BlockingAssignRange(_, _, _, v)
                | Insn::NbaAssignRange(_, _, _, v) => v,
                _ => continue,
            };
            if *vref != d {
                continue;
            }
            if Self::reg_read_after(insns, j, d) {
                continue;
            }
            match &mut insns[j] {
                Insn::BlockingAssign(_, v, _)
                | Insn::NbaAssign(_, v, _)
                | Insn::BlockingAssignRange(_, _, _, v)
                | Insn::NbaAssignRange(_, _, _, v) => *v = s,
                _ => unreachable!(),
            }
            insns[i] = Insn::Nop;
            FUSED_MOVE_FWD.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Delete `ClearSigned(r)` when `r` provably already holds an UNSIGNED
    /// value — the scrub is then a no-op and removing it cannot change
    /// anything.
    ///
    /// The c906 opcode census measured ClearSigned at 8.0% of the executed
    /// stream (506 M / run): sign scrubs are emitted defensively at §11.8.1
    /// mixed-signedness sites, but most registers already carry unsigned
    /// values. This is a single forward pass tracking a per-register
    /// "provably unsigned" set, built only from rules VERIFIED against the
    /// `Value` method each executor calls:
    ///
    /// * compares / case-compares / logical ops -> 1-bit unsigned
    /// * `bitwise_and/or/xor`, `range_select`, `bit_select`, `concat_refs`
    ///   -> `is_signed: false` unconditionally
    /// * `add/sub/mul` -> signed only when BOTH operands signed
    /// * `shift_left/right`, `bitwise_not`, replicate, `Move`/`MoveResize`
    ///   -> preserve/propagate the source flag
    /// * `Resize` extends by the value's own flag and keeps it — no change
    ///
    /// Every join point (branch target) wipes the set; any instruction not
    /// understood wipes the set; `SetSigned` un-cleans its register. All
    /// three fallbacks only FORGO deletions, never enable a wrong one.
    pub(super) fn elide_provably_unsigned_scrubs(insns: &mut [Insn], signal_signed: &[bool]) {
        use std::sync::OnceLock;
        static ON: OnceLock<bool> = OnceLock::new();
        if !*ON.get_or_init(|| {
            !matches!(std::env::var("XEZIM_FUSE").as_deref(), Ok("0"))
                && !matches!(std::env::var("XEZIM_FUSE_SCRUBS").as_deref(), Ok("0"))
        }) || insns.is_empty()
        {
            return;
        }
        let is_target = Self::branch_target_map(insns);
        let mut clean: std::collections::HashSet<RegId> = std::collections::HashSet::new();
        let sig_unsigned =
            |sig: u32| -> bool { signal_signed.get(sig as usize).is_some_and(|s| !*s) };
        for i in 0..insns.len() {
            if is_target[i] {
                clean.clear();
            }
            // set_to(d, c): record the DEST's new cleanliness. Computed before
            // mutation so a dest that is also a source reads its OLD state.
            let mut set_to: Option<(RegId, bool)> = None;
            let mut elide = false;
            match &insns[i] {
                Insn::Nop => {}
                // -- 1-bit unsigned results --
                Insn::Eq(d, _, _)
                | Insn::Neq(d, _, _)
                | Insn::CaseEq(d, _, _)
                | Insn::CasezEq(d, _, _)
                | Insn::CasexEq(d, _, _)
                | Insn::Lt(d, _, _)
                | Insn::Leq(d, _, _)
                | Insn::Gt(d, _, _)
                | Insn::Geq(d, _, _)
                | Insn::LogAnd(d, _, _)
                | Insn::LogOr(d, _, _)
                | Insn::LogNot(d, _) => set_to = Some((*d, true)),
                // -- unconditionally unsigned constructors --
                Insn::BitAnd(d, _, _)
                | Insn::BitOr(d, _, _)
                | Insn::BitXor(d, _, _)
                | Insn::LoadSignalBit(d, _, _)
                | Insn::LoadSignalRange(d, _, _, _)
                | Insn::Concat(d, _) => set_to = Some((*d, true)),
                // -- signed only when BOTH operands are --
                Insn::Add(d, l, r) | Insn::Sub(d, l, r) | Insn::Mul(d, l, r) => {
                    set_to = Some((*d, clean.contains(l) || clean.contains(r)))
                }
                // -- flag preserved / propagated from the source --
                Insn::BitNot(d, sr)
                | Insn::Shl(d, sr, _)
                | Insn::Shr(d, sr, _)
                | Insn::Replicate(d, sr, _)
                | Insn::Move(d, sr)
                | Insn::MoveResize(d, sr, _) => set_to = Some((*d, clean.contains(sr))),
                Insn::Resize(_, _) => {}
                Insn::LoadConst(d, v) => set_to = Some((*d, !v.is_signed && !v.is_real)),
                Insn::LoadSignal(d, sig) => set_to = Some((*d, sig_unsigned(*sig))),
                Insn::LoadSignalSigned(d, _) => set_to = Some((*d, false)),
                Insn::BinOpConst(d, sr, k, kind) => {
                    let c = match kind {
                        // `bitwise_and/or/xor` clear the flag unconditionally
                        // (the register-form rule above); losing that for
                        // the fused forms kept 14k scrubs on a C910 SoC.
                        BinOpConstKind::Eq
                        | BinOpConstKind::CaseEq
                        | BinOpConstKind::Xor
                        | BinOpConstKind::And
                        | BinOpConstKind::Or => true,
                        BinOpConstKind::Add | BinOpConstKind::Mul | BinOpConstKind::Sub => {
                            clean.contains(sr) || !k.is_signed
                        }
                    };
                    set_to = Some((*d, c));
                }
                Insn::ClearSigned(r) => {
                    if clean.contains(r) {
                        elide = true;
                    } else {
                        set_to = Some((*r, true));
                    }
                }
                Insn::SetSigned(r) => set_to = Some((*r, false)),
                // X/Z to 0 keeps the flag; a real rounds to an unsigned
                // integer, anything else is untouched.
                Insn::DropXz(_) | Insn::RealToInt(_) => {}
                // -- no register writes: state flows through --
                Insn::Jump(_)
                | Insn::BranchIfFalse(_, _)
                | Insn::BranchUnlessZero(_, _)
                | Insn::BranchIfSignalFalse(_, _, _)
                | Insn::CmpBranch(_, _, _, _, _)
                | Insn::CmpBranchConst(..)
                | Insn::BlockingAssign(_, _, _)
                | Insn::NbaAssign(_, _, _)
                | Insn::BlockingAssignRange(_, _, _, _)
                | Insn::NbaAssignRange(_, _, _, _)
                | Insn::BlockingAssignBitDyn(_, _, _)
                | Insn::NbaAssignBitDyn(_, _, _)
                | Insn::BlockingAssignArray(_, _, _, _)
                | Insn::NbaAssignArray(_, _, _, _)
                | Insn::BlockingAssignArrayRange(_, _, _, _, _)
                | Insn::NbaAssignArrayRange(_, _, _, _, _)
                | Insn::NbaAssignArrayRead(_, _, _, _) => {}
                // -- anything else: unknown effects, forget everything --
                _ => clean.clear(),
            }
            if elide {
                insns[i] = Insn::Nop;
                ELIDED_SCRUBS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                continue;
            }
            if let Some((d, c)) = set_to {
                if c {
                    clean.insert(d);
                } else {
                    clean.remove(&d);
                }
            }
        }
    }

    /// Merge two adjacent constant adds into one `BinOpConstAdd2` dispatch.
    /// Pure dispatch amortization — both adds still execute, in order, so
    /// chained (`s2 == d1`) and independent pairs are equally sound; the only
    /// constraint is that control cannot enter between them. OPT-IN
    /// (`XEZIM_FUSE_ADDC2=1`): the new variant must stay off until every
    /// backend either handles it (interpreter, two-state lowering) or bails
    /// per-block (JIT/AOT return None on unsupported insns) — forming it by
    /// default would silently change which blocks those backends accept.
    pub(super) fn fuse_addc2(insns: &mut [Insn]) {
        use std::sync::OnceLock;
        static ON: OnceLock<bool> = OnceLock::new();
        if !*ON.get_or_init(|| matches!(std::env::var("XEZIM_FUSE_ADDC2").as_deref(), Ok("1")))
            || insns.len() < 2
        {
            return;
        }
        let is_target = Self::branch_target_map(insns);
        let mut i = 0;
        while i + 1 < insns.len() {
            let Insn::BinOpConst(_, _, _, BinOpConstKind::Add) = insns[i] else {
                i += 1;
                continue;
            };
            let mut j = i + 1;
            let mut blocked = false;
            while j < insns.len() {
                if is_target[j] {
                    blocked = true;
                    break;
                }
                if !matches!(insns[j], Insn::Nop) {
                    break;
                }
                j += 1;
            }
            if blocked || j >= insns.len() {
                i += 1;
                continue;
            }
            let Insn::BinOpConst(_, _, _, BinOpConstKind::Add) = insns[j] else {
                i = j;
                continue;
            };
            let Insn::BinOpConst(d1, s1, k1, _) = std::mem::replace(&mut insns[i], Insn::Nop)
            else {
                unreachable!()
            };
            let Insn::BinOpConst(d2, s2, k2, _) = std::mem::replace(&mut insns[j], Insn::Nop)
            else {
                unreachable!()
            };
            insns[i] = Insn::BinOpConstAdd2(Box::new(AddC2 {
                d1,
                s1,
                k1: *k1,
                d2,
                s2,
                k2: *k2,
            }));
            FUSED_ADDC2.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            i = j + 1;
        }
    }

    pub(super) fn compact_nops(insns: &mut Vec<Insn>) {
        if !insns.iter().any(|i| matches!(i, Insn::Nop)) {
            return;
        }
        // old index -> new index; `map[len]` is the new one-past-the-end.
        let mut map = vec![0u32; insns.len() + 1];
        let mut new_idx = 0u32;
        for (old, insn) in insns.iter().enumerate() {
            map[old] = new_idx;
            if !matches!(insn, Insn::Nop) {
                new_idx += 1;
            }
        }
        map[insns.len()] = new_idx;
        insns.retain(|i| !matches!(i, Insn::Nop));
        for insn in insns.iter_mut() {
            match insn {
                Insn::CaseJump(_, cj) => {
                    for t in cj.table.iter_mut().chain(std::iter::once(&mut cj.default)) {
                        if let Some(&m) = map.get(*t as usize) {
                            *t = m;
                        }
                    }
                }
                Insn::CaseMaskJump(_, mj) => {
                    for t in mj.table.iter_mut().chain(std::iter::once(&mut mj.xz_path)) {
                        if let Some(&m) = map.get(*t as usize) {
                            *t = m;
                        }
                    }
                }
                Insn::BranchIfFalse(_, t)
                | Insn::BranchUnlessZero(_, t)
                | Insn::BranchIfSignalFalse(_, t, _)
                | Insn::CmpBranch(_, _, _, _, t)
                | Insn::CmpBranchConst(_, _, _, _, t)
                | Insn::Jump(t) => {
                    if let Some(&m) = map.get(*t as usize) {
                        *t = m;
                    }
                }
                _ => {}
            }
        }
    }
}
