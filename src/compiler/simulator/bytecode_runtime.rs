//! Simulator bytecode runtime.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    /// Execute bytecode instructions in isolation (no &mut self).
    /// Returns NBA entries produced. Used for parallel edge block execution.
    pub(super) fn exec_insns_isolated(
        insns: &[super::bytecode::Insn],
        signal_table: &[Value],
        _signal_signed: &[bool],
        signal_name_to_id: &NameMap,
        array_first_id: &HashMap<Arc<str>, (usize, i64, i64)>,
        // Read-only view of the packed arena. With None a packed cell
        // reads x.
        packed: Option<&PackedMem>,
        vm_regs: &mut Vec<Value>,
        block_index: u32,
        // CompiledBlock::nba_dup_targets — false for the overwhelming majority
        // of blocks, which lets the whole-value NBA arms skip the linear
        // last-write-wins scan entirely.
        nba_dup: bool,
    ) -> Vec<NbaFast> {
        use super::bytecode::Insn;
        let mut nba_out: Vec<NbaFast> = Vec::new();
        let mut pc: usize = 0;
        let len = insns.len();
        while pc < len {
            match &insns[pc] {
                // Process-FSM wait insns never appear in comb/edge blocks;
                // an isolated evaluator cannot suspend — stop the block.
                Insn::WaitDelayReg(..) | Insn::WaitEdge(..) => {
                    debug_assert!(false, "wait insn in isolated comb exec");
                    break;
                }
                Insn::LoadClassMember(..) | Insn::StoreClassMember(..) => {
                    debug_assert!(false, "class-member insn in isolated comb exec");
                    break;
                }
                Insn::CallMethod(..)
                | Insn::CallScopedMethod(..)
                | Insn::CallStaticScoped(..)
                | Insn::CallFreeFunction(..)
                | Insn::LoadClassStatic(..)
                | Insn::StoreClassStatic(..)
                | Insn::ConstructObject(..) => {
                    debug_assert!(false, "class-member insn in isolated comb exec");
                    break;
                }
                Insn::CallCollMethod(..) => {
                    debug_assert!(false, "class-member insn in isolated comb exec");
                    break;
                }
                Insn::LoadCollElem(..) | Insn::StoreCollElem(..) => {
                    debug_assert!(false, "collection-elem insn in isolated comb exec");
                    break;
                }
                Insn::InitLocalColl(..) => {
                    debug_assert!(false, "InitLocalColl in isolated comb exec");
                    break;
                }
                Insn::Cast(..) => {
                    debug_assert!(false, "cast insn in isolated comb exec");
                    break;
                }
                Insn::ForeachKeys(..) | Insn::ForeachNext(..) => {
                    debug_assert!(false, "foreach insn in isolated comb exec");
                    break;
                }
                Insn::RetCollection(..) | Insn::AssignPendingColl(..) => {
                    debug_assert!(false, "ret-collection insn in isolated comb exec");
                    break;
                }
                Insn::LoadConst(dest, val) => {
                    vm_regs[*dest as usize] = (**val).clone();
                }
                Insn::LoadSignal(dest, sig_id) => {
                    let sig_id = &(*sig_id as usize);
                    vm_regs[*dest as usize] = signal_table[*sig_id].clone();
                }
                Insn::LoadSignalSigned(dest, sig_id) => {
                    let sig_id = &(*sig_id as usize);
                    let mut v = signal_table[*sig_id].clone();
                    v.is_signed = true;
                    vm_regs[*dest as usize] = v;
                }
                Insn::LoadProcessLocal(dest, _) => {
                    vm_regs[*dest as usize] = Value::new(1);
                }
                Insn::Resize(reg, width) => {
                    let r = *reg as usize;
                    if vm_regs[r].width != *width {
                        let resized = vm_regs[r].resize(*width);
                        vm_regs[r] = resized;
                    }
                }
                Insn::Add(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].add(&vm_regs[*r as usize]);
                }
                Insn::Sub(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].sub(&vm_regs[*r as usize]);
                }
                Insn::Mul(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].mul(&vm_regs[*r as usize]);
                }
                Insn::Div(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].div(&vm_regs[*r as usize]);
                }
                Insn::Mod(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].modulo(&vm_regs[*r as usize]);
                }
                Insn::BitAnd(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].bitwise_and(&vm_regs[*r as usize]);
                }
                Insn::BitOr(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].bitwise_or(&vm_regs[*r as usize]);
                }
                Insn::BitXor(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].bitwise_xor(&vm_regs[*r as usize]);
                }
                Insn::BitXnor(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize]
                        .bitwise_xor(&vm_regs[*r as usize])
                        .bitwise_not();
                }
                Insn::LogAnd(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].logic_and(&vm_regs[*r as usize]);
                }
                Insn::LogOr(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].logic_or(&vm_regs[*r as usize]);
                }
                Insn::Eq(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].is_equal(&vm_regs[*r as usize]);
                }
                Insn::Neq(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].is_not_equal(&vm_regs[*r as usize]);
                }
                Insn::CaseEq(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].case_eq(&vm_regs[*r as usize]);
                }
                Insn::CaseLut(d, src, lut) => {
                    let s = &vm_regs[*src as usize];
                    let hit = if s.has_xz() {
                        None
                    } else {
                        s.to_u64().and_then(|i| lut.table.get(i as usize))
                    };
                    vm_regs[*d as usize] = hit.unwrap_or(&lut.default).clone();
                }
                // Fused `LoadConst` + binop. Character-for-character the
                // `Add`/`Eq`/`CaseEq` arms above with `&vm_regs[r]` replaced by
                // the embedded constant, which the elided `LoadConst` would
                // have cloned into `vm_regs[r]` verbatim — so the 4-state,
                // signedness and §5.7.1 `is_fill` rules cannot drift.
                Insn::BinOpConstAdd2(a) => {
                    vm_regs[a.d1 as usize] = vm_regs[a.s1 as usize].add(&a.k1);
                    vm_regs[a.d2 as usize] = vm_regs[a.s2 as usize].add(&a.k2);
                }
                Insn::BinOpConst(d, s, k, kind) => {
                    use super::bytecode::BinOpConstKind as K;
                    let (d, s) = (*d as usize, *s as usize);
                    match kind {
                        K::Add => vm_regs[d] = vm_regs[s].add(&**k),
                        K::Eq => vm_regs[d] = vm_regs[s].is_equal(&**k),
                        K::CaseEq => vm_regs[d] = vm_regs[s].case_eq(&**k),
                        K::Xor => vm_regs[d] = vm_regs[s].bitwise_xor(&**k),
                        K::And => vm_regs[d] = vm_regs[s].bitwise_and(&**k),
                        K::Or => vm_regs[d] = vm_regs[s].bitwise_or(&**k),
                        K::Mul => vm_regs[d] = vm_regs[s].mul(&**k),
                        K::Sub => vm_regs[d] = vm_regs[s].sub(&**k),
                    }
                }
                Insn::CasezEq(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].casez_eq(&vm_regs[*r as usize]);
                }
                Insn::CasexEq(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].casex_eq(&vm_regs[*r as usize]);
                }
                Insn::Lt(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].less_than(&vm_regs[*r as usize]);
                }
                Insn::Leq(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].less_equal(&vm_regs[*r as usize]);
                }
                Insn::Gt(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].greater_than(&vm_regs[*r as usize]);
                }
                Insn::Geq(d, l, r) => {
                    vm_regs[*d as usize] =
                        vm_regs[*l as usize].greater_equal(&vm_regs[*r as usize]);
                }
                Insn::Shl(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].shift_left(&vm_regs[*r as usize]);
                }
                Insn::Shr(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].shift_right(&vm_regs[*r as usize]);
                }
                Insn::AShr(d, l, r) => {
                    vm_regs[*d as usize] =
                        vm_regs[*l as usize].arith_shift_right(&vm_regs[*r as usize]);
                }
                Insn::BitNot(d, s) => {
                    vm_regs[*d as usize] = vm_regs[*s as usize].bitwise_not();
                }
                Insn::LogNot(d, s) => {
                    vm_regs[*d as usize] = vm_regs[*s as usize].logic_not();
                }
                Insn::Negate(d, s) => {
                    let w = vm_regs[*s as usize].width;
                    let mut r = Value::zero(w).sub(&vm_regs[*s as usize]).resize(w);
                    r.is_signed = vm_regs[*s as usize].is_signed;
                    vm_regs[*d as usize] = r;
                }
                Insn::ReduceAnd(d, s) => {
                    vm_regs[*d as usize] = vm_regs[*s as usize].reduce_and();
                }
                Insn::ReduceOr(d, s) => {
                    vm_regs[*d as usize] = vm_regs[*s as usize].reduce_or();
                }
                Insn::ReduceXor(d, s) => {
                    vm_regs[*d as usize] = vm_regs[*s as usize].reduce_xor();
                }
                Insn::BitSelect(d, base, idx) => {
                    // §11.5.1: an x/z or negative index reads x.
                    vm_regs[*d as usize] = match vm_regs[*idx as usize].to_index() {
                        Some(i) if i >= 0 => vm_regs[*base as usize].bit_select(i as usize),
                        _ => Value::new(1),
                    };
                }
                Insn::BitSelectConst(d, base, idx) => {
                    vm_regs[*d as usize] = vm_regs[*base as usize].bit_select(*idx as usize);
                }
                Insn::RangeSelectW(d, base, idx, w, up) => {
                    // §11.5.1: an x/z index reads a whole-x select of the
                    // declared width; otherwise the bounds are formed here.
                    let dv = match vm_regs[*idx as usize].to_index() {
                        None => Value::new(*w),
                        Some(i) => {
                            let i = i as i32 as i64;
                            let (lo, hi) = if *up {
                                (i, i + *w as i64 - 1)
                            } else {
                                (i - *w as i64 + 1, i)
                            };
                            oob_range_select(&vm_regs[*base as usize], hi, lo)
                        }
                    };
                    vm_regs[*d as usize] = dv;
                }
                Insn::RangeSelect(d, base, l, r) => {
                    // §11.5.1: bounds are 32-bit index arithmetic; a `[l -: W]`
                    // whose low bound went negative wraps to a large u32. Recover
                    // the signed value and, only when the low bound is <0, take
                    // the x-filling signed path (the fast path handles the common
                    // in-range + high-overrun cases).
                    // §11.5.1: an x/z bound selects nothing: whole-x at the
                    // select's width, recovered from the masked bound values
                    // (an indexed form's bounds differ by the constant width
                    // even when the base is unknown).
                    let (av, bv) = (&vm_regs[*l as usize], &vm_regs[*r as usize]);
                    let (Some(a), Some(b)) = (av.to_index(), bv.to_index()) else {
                        let a = av.to_u64().unwrap_or(0) as u32 as i32 as i64;
                        let b = bv.to_u64().unwrap_or(0) as u32 as i32 as i64;
                        vm_regs[*d as usize] = Value::new(((a - b).unsigned_abs() + 1) as u32);
                        pc += 1;
                        continue;
                    };
                    let (a, b) = (a as i32 as i64, b as i32 as i64);
                    let (lo, hi) = (a.min(b), a.max(b));
                    vm_regs[*d as usize] = oob_range_select(&vm_regs[*base as usize], hi, lo);
                }
                Insn::RangeSelectConst(d, base, l, r) => {
                    vm_regs[*d as usize] = oob_range_select(
                        &vm_regs[*base as usize],
                        (*l).max(*r) as i64,
                        (*l).min(*r) as i64,
                    );
                }
                // Fused load+select (finish() peephole).
                Insn::LoadSignalRangeDyn(d, sig_id, lo_reg, w) => {
                    // §11.5.1: an x/z offset selects nothing — whole-x at the
                    // slice's width. Otherwise read the slice in place; the
                    // signal is never copied.
                    let lo_v = &vm_regs[*lo_reg as usize];
                    vm_regs[*d as usize] = match lo_v.to_index() {
                        Some(lo) => {
                            let lo = lo as i32 as i64;
                            oob_range_select(
                                &signal_table[*sig_id as usize],
                                lo + *w as i64 - 1,
                                lo,
                            )
                        }
                        None => Value::new(*w),
                    };
                }
                Insn::LoadSignalRange(d, sig_id, l, r) => {
                    let sig_id = &(*sig_id as usize);
                    // §11.5.1: a select past the signal's MSB is partially
                    // out of range (see `oob_range_select`).
                    vm_regs[*d as usize] = oob_range_select(
                        &signal_table[*sig_id],
                        (*l).max(*r) as i64,
                        (*l).min(*r) as i64,
                    );
                }
                Insn::LoadSignalBit(d, sig_id, idx) => {
                    let sig_id = &(*sig_id as usize);
                    vm_regs[*d as usize] = signal_table[*sig_id].bit_select(*idx as usize);
                }
                Insn::Concat(d, part_regs) => {
                    // `concat(&clones)` is literally `concat_refs(slice.iter())`
                    // (see `Value::concat`), so the `Vec<Value>` — and the heap
                    // allocation every `Wide` clone in it performs — was pure
                    // overhead. Borrow the registers directly.
                    let result =
                        Value::concat_refs(part_regs.iter().map(|r| &vm_regs[*r as usize]));
                    vm_regs[*d as usize] = result;
                }
                Insn::Replicate(d, s, n) => {
                    let val = vm_regs[*s as usize].clone();
                    if *n == 0 {
                        vm_regs[*d as usize] = Value::zero(0);
                    } else if *n == 1 {
                        vm_regs[*d as usize] = val;
                    } else {
                        let result = Value::concat_refs(std::iter::repeat_n(&val, *n as usize));
                        vm_regs[*d as usize] = result;
                    }
                }
                Insn::BranchIfFalse(reg, target) => {
                    if !vm_regs[*reg as usize].is_true() {
                        pc = *target as usize;
                        continue;
                    }
                }
                Insn::NbaAssignConst(sig_id, k, _width) => {
                    let sig_id = &(*sig_id as usize);
                    // Const pre-resized at fuse time: compare, clone only on change.
                    // §10.4.2 last-write-wins: an entry already queued for this
                    // signal supersedes signal_table, so compare against it
                    // rather than eliding against a stale current value.
                    if let Some(i) = if nba_dup {
                        nba_out.iter().rposition(|n| n.signal_id == *sig_id)
                    } else {
                        None
                    } {
                        nba_out[i].value = (**k).clone();
                    } else if signal_table[*sig_id] != **k {
                        nba_out.push(NbaFast {
                            signal_id: *sig_id,
                            value: (**k).clone(),
                            block_index,
                        });
                    }
                }
                Insn::BranchUnlessZero(reg, target) => {
                    // Fused LogNot+BranchIfFalse: jump unless DEFINITE zero —
                    // exact composition of logic_not with !is_true (X branches).
                    if vm_regs[*reg as usize].is_nonzero() != Some(false) {
                        pc = *target as usize;
                        continue;
                    }
                }
                Insn::BranchIfSignalFalse(sig_id, target, bit) => {
                    let sig_id = &(*sig_id as usize);
                    // `bit == u32::MAX` tests the whole signal; otherwise the
                    // fused form also folds in a constant bit-select. Written
                    // as `bit_select(..).is_true()` rather than open-coded bit
                    // arithmetic so the 4-state X/Z rule stays exactly the one
                    // the unfused `LoadSignalBit ; BranchIfFalse` applied.
                    let cond_true = if *bit == u32::MAX {
                        signal_table[*sig_id].is_true()
                    } else {
                        signal_table[*sig_id].bit_select(*bit as usize).is_true()
                    };
                    if !cond_true {
                        pc = *target as usize;
                        continue;
                    }
                }
                Insn::Select(dest, cond, then_r, else_r) => {
                    // §11.4.11 + §5.7.1: a fill branch ('z/'1/...) takes the
                    // width of the other branch (tristate lowering builds
                    // `cond ? then : 'z`; a 1-bit z leaking through would
                    // zero-extend at the assign instead of z-filling).
                    {
                        let (tw, tf) = (
                            vm_regs[*then_r as usize].width,
                            vm_regs[*then_r as usize].is_fill,
                        );
                        let (ew, ef) = (
                            vm_regs[*else_r as usize].width,
                            vm_regs[*else_r as usize].is_fill,
                        );
                        if tf && !ef {
                            vm_regs[*then_r as usize] = vm_regs[*then_r as usize].resize(ew);
                        } else if ef && !tf {
                            vm_regs[*else_r as usize] = vm_regs[*else_r as usize].resize(tw);
                        }
                    }
                    let v = if vm_regs[*cond as usize].has_unknown() {
                        let t = &vm_regs[*then_r as usize];
                        let e = &vm_regs[*else_r as usize];
                        // §11.3.1: a real result can't hold X, so bit-merging the
                        // branches reads their IEEE-754 bits as garbage (a real
                        // 1000.0 came out as 4.65e18). Return a defined real value
                        // (x condition → the else/false branch) instead.
                        if t.is_real || e.is_real {
                            e.clone()
                        } else {
                            t.merge_unknown(e)
                        }
                    } else if vm_regs[*cond as usize].is_true() {
                        vm_regs[*then_r as usize].clone()
                    } else {
                        vm_regs[*else_r as usize].clone()
                    };
                    vm_regs[*dest as usize] = v;
                }
                Insn::Jump(target) => {
                    pc = *target as usize;
                    continue;
                }
                Insn::CaseJump(src, cj) => {
                    // Compile gate guarantees the selector is unsigned and
                    // <=64 bits, so raw numeric dispatch matches the ===
                    // chain it replaces; any x/z bit matches no constant
                    // pattern -> default (table holes also hold default).
                    let s = &vm_regs[*src as usize];
                    let t = if s.has_xz() {
                        cj.default
                    } else {
                        match s.to_u64() {
                            Some(i) => cj.table.get(i as usize).copied().unwrap_or(cj.default),
                            None => cj.default,
                        }
                    };
                    pc = t as usize;
                    continue;
                }
                Insn::CaseMaskJump(src, mj) => {
                    // Dispatch on the always-defined window; any x/z there
                    // (or non-inline storage) walks the full chain instead.
                    let s = &vm_regs[*src as usize];
                    let t = match s.inline_bits() {
                        Some((v, xz)) if !s.is_fill => {
                            let m = if s.width >= 64 {
                                u64::MAX
                            } else {
                                (1u64 << s.width) - 1
                            };
                            let wmask = (1u64 << mj.width) - 1;
                            if ((xz & m) >> mj.lo) & wmask == 0 {
                                mj.table[(((v & m) >> mj.lo) & wmask) as usize]
                            } else {
                                mj.xz_path
                            }
                        }
                        _ => mj.xz_path,
                    };
                    pc = t as usize;
                    continue;
                }
                Insn::Format(dst, f) => {
                    vm_regs[*dst as usize] = Self::exec_format(f, vm_regs);
                }
                Insn::StrOp(dst, kind, args) => {
                    vm_regs[*dst as usize] = Self::exec_str_op(*kind, args, vm_regs);
                }
                Insn::NbaAssign(sig_id, val_reg, width) => {
                    let sig_id = &(*sig_id as usize);
                    // Unchanged-value elision BEFORE the resize: an equal-width,
                    // non-fill, non-real register resizes to an exact copy of
                    // itself, so comparing the register against the signal table
                    // decides the elision without cloning the value first (flop
                    // outputs reload the same value most cycles — the clone was
                    // made only to be dropped).
                    {
                        let reg = &vm_regs[*val_reg as usize];
                        if !nba_dup
                            && *width != 0
                            && reg.width == *width
                            && !reg.is_fill
                            && !reg.is_real
                            && signal_table[*sig_id] == *reg
                        {
                            pc += 1;
                            continue;
                        }
                    }
                    let val = vm_regs[*val_reg as usize].resize_for_assign(*width);
                    // Eval-time elision: skip the queue push when the
                    // value already matches signal_table.  apply_nba_entry
                    // would do the same check at apply time and drop the
                    // entry; doing it here saves the push + queue traversal
                    // + ~70% of apply_nba's per-entry work on c910 (flop Q
                    // outputs reload the same value most cycles).
                    // §10.4.2 last-write-wins: see the NbaAssignConst arm.
                    if let Some(i) = if nba_dup {
                        nba_out.iter().rposition(|n| n.signal_id == *sig_id)
                    } else {
                        None
                    } {
                        nba_out[i].value = val;
                    } else if signal_table[*sig_id] != val {
                        nba_out.push(NbaFast {
                            signal_id: *sig_id,
                            value: val,
                            block_index,
                        });
                    }
                }
                Insn::NbaAssignRange(sig_id, hi, lo, val_reg) => {
                    let sig_id = &(*sig_id as usize);
                    let (low, high) = if hi >= lo { (*lo, *hi) } else { (*hi, *lo) };
                    if oob_write_discard(high as i64, low as i64, signal_table[*sig_id].width) {
                        pc += 1;
                        continue;
                    }
                    let w = high - low + 1;
                    let val = vm_regs[*val_reg as usize].resize(w);
                    let existing = nba_out.iter().rposition(|n| n.signal_id == *sig_id);
                    if let Some(i) = existing {
                        // In-flight queue entry — merge into it.  Don't try
                        // to elide here: the original write was a real
                        // change (else it would have been elided already);
                        // this update may or may not return to no-op state
                        // but apply_nba's slot check will catch it.
                        let mut new_val = nba_out[i].value.clone();
                        for bit_pos in low..=high {
                            new_val
                                .set_bit(bit_pos as usize, val.get_bit((bit_pos - low) as usize));
                        }
                        nba_out[i].value = new_val;
                    } else {
                        // No queue entry: build the merged value from
                        // signal_table and elide if the range bits already
                        // match (the partial-range counterpart of the
                        // simple NbaAssign eval-time elision).
                        // Compare FIRST (no clone): flop reloads usually match, and
                        // the unconditional clone of a wide signal (1 byte/bit Vec)
                        // was a top allocator cost on c910. Clone + merge only from
                        // the first differing bit.
                        let mut first_diff = None;
                        {
                            let src = &signal_table[*sig_id];
                            for bit_pos in low..=high {
                                if src.get_bit(bit_pos as usize)
                                    != val.get_bit((bit_pos - low) as usize)
                                {
                                    first_diff = Some(bit_pos);
                                    break;
                                }
                            }
                        }
                        if let Some(start) = first_diff {
                            let mut new_val = signal_table[*sig_id].clone();
                            for bit_pos in start..=high {
                                new_val.set_bit(
                                    bit_pos as usize,
                                    val.get_bit((bit_pos - low) as usize),
                                );
                            }
                            nba_out.push(NbaFast {
                                signal_id: *sig_id,
                                value: new_val,
                                block_index,
                            });
                        }
                    }
                }
                Insn::NbaAssignBitDyn(sig_id, idx_reg, val_reg) => {
                    let sig_id = &(*sig_id as usize);
                    // §11.5.1: an x/z index modifies no bits.
                    if let Some(idx) = dyn_bit_index(&vm_regs[*idx_reg as usize]) {
                        let bit = vm_regs[*val_reg as usize].get_bit(0);
                        let existing = nba_out.iter().rposition(|n| n.signal_id == *sig_id);
                        if let Some(i) = existing {
                            let mut new_val = nba_out[i].value.clone();
                            new_val.set_bit(idx, bit);
                            nba_out[i].value = new_val;
                        } else {
                            let cur = &signal_table[*sig_id];
                            if cur.get_bit(idx) != bit {
                                let mut new_val = cur.clone();
                                new_val.set_bit(idx, bit);
                                nba_out.push(NbaFast {
                                    signal_id: *sig_id,
                                    value: new_val,
                                    block_index,
                                });
                            }
                        }
                    }
                }
                Insn::LoadArrayElem(dest, array_name, idx_reg) => {
                    // Direct compute via array_first_id — no format!() / no
                    // HashMap miss on cell access. Parallel-path equivalent
                    // of the sequential array_elem_ids cache.
                    // §7.4.6 / §11.5.1: an UNKNOWN index reads x, NOT element 0.
                    // `to_u64()` returns None for an x/z value, and the old
                    // `unwrap_or(0)` silently read arr[0] — an AES S-box
                    // indexed by an uninitialized input returned real table
                    // data, so downstream logic resolved to a definite WRONG
                    // value instead of propagating x. A sentinel below `lo`
                    // makes the element resolve MISS, taking the existing
                    // out-of-range arm that already yields x.
                    let idx = {
                        let ir = &vm_regs[*idx_reg as usize];
                        // x/z index: force a resolve MISS so the existing
                        // out-of-range arm yields x. `to_u64()` masks x bits to
                        // ZERO and never returns None, so it cannot be used to
                        // detect this -- the old code silently read arr[0].
                        if ir.has_xz() {
                            i64::MIN
                        } else {
                            ir.to_u64().unwrap_or(0) as i64
                        }
                    };
                    let id = resolve_bytecode_array_elem(
                        array_name,
                        idx,
                        array_first_id,
                        signal_name_to_id,
                    );
                    if let Some(eid) = id.filter(|&e| !is_packed_id(e)) {
                        vm_regs[*dest as usize] = signal_table[eid].clone();
                    } else if let Some(eid) = id {
                        vm_regs[*dest as usize] = match packed {
                            Some(p) => p.read(eid),
                            None => Value::new(1),
                        };
                    } else {
                        let w = match (bytecode_array_first_id(array_name, array_first_id), packed)
                        {
                            (Some(f), Some(p)) if is_packed_id(f) => p.width(f),
                            _ => {
                                bytecode_array_elem_width(array_name, array_first_id, signal_table)
                            }
                        };
                        vm_regs[*dest as usize] = Value::new(w);
                    }
                }
                Insn::NbaAssignArray(array_name, idx_reg, val_reg, width) => {
                    // §11.5.1: an x/z index selects no element: the write
                    // is discarded (an out-of-range index is dropped by the
                    // element resolver below).
                    let Some(idx) = vm_regs[*idx_reg as usize].to_index() else {
                        pc += 1;
                        continue;
                    };
                    let id = resolve_bytecode_array_elem(
                        array_name,
                        idx,
                        array_first_id,
                        signal_name_to_id,
                    );
                    if let Some(eid) = id {
                        let val = vm_regs[*val_reg as usize].resize(*width);
                        // §10.4.2 last-write-wins: see the NbaAssign arm.
                        if let Some(i) = if nba_dup {
                            nba_out.iter().rposition(|n| n.signal_id == eid)
                        } else {
                            None
                        } {
                            nba_out[i].value = val;
                        } else if is_packed_id(eid) || signal_table[eid] != val {
                            nba_out.push(NbaFast {
                                signal_id: eid,
                                value: val,
                                block_index,
                            });
                        }
                    }
                }
                // Fused LoadSignal + LoadArrayElem + NbaAssign. Composition of
                // the three arms above, with the two intermediate register
                // round-trips removed: the index comes straight from
                // signal_table (what `LoadSignal` would have copied into a
                // register) and the element value feeds `resize_for_assign`
                // directly. The unresolved-element fallback reproduces
                // `LoadArrayElem`'s 1-bit X exactly.
                Insn::NbaAssignArrayRead(sig_id, array_name, idx_sig, width) => {
                    // §11.5.1: an x/z index reads an all-x element (it used to
                    // read element 0 through `to_u64`, which masks x to 0).
                    let val = match signal_table[*idx_sig as usize].to_index() {
                        None => Value::new((*width).max(1)),
                        Some(idx) => match resolve_bytecode_array_elem(
                            array_name,
                            idx,
                            array_first_id,
                            signal_name_to_id,
                        ) {
                            Some(eid) if is_packed_id(eid) => match packed {
                                Some(p) => p.read(eid).resize_for_assign(*width),
                                None => Value::new((*width).max(1)),
                            },
                            Some(eid) => signal_table[eid].resize_for_assign(*width),
                            None => Value::new((*width).max(1)),
                        },
                    };
                    let sig_id = &(*sig_id as usize);
                    // §10.4.2 last-write-wins: see the NbaAssign arm.
                    if let Some(i) = if nba_dup {
                        nba_out.iter().rposition(|n| n.signal_id == *sig_id)
                    } else {
                        None
                    } {
                        nba_out[i].value = val;
                    } else if signal_table[*sig_id] != val {
                        nba_out.push(NbaFast {
                            signal_id: *sig_id,
                            value: val,
                            block_index,
                        });
                    }
                }
                Insn::Move(d, s) => {
                    vm_regs[*d as usize] = vm_regs[*s as usize].clone();
                }
                Insn::MoveResize(d, s, w) => {
                    // Exact composition of Move+Resize, including the Resize
                    // arm's runtime same-width skip (which preserves a fill
                    // value's `is_fill` verbatim).
                    let sv = &vm_regs[*s as usize];
                    vm_regs[*d as usize] = if sv.width != *w {
                        sv.resize(*w)
                    } else {
                        sv.clone()
                    };
                }
                Insn::CmpBranchConst(kind, src, k, _tmp, target) => {
                    use super::bytecode::BinOpConstKind as K;
                    let a = &vm_regs[*src as usize];
                    let res = match kind {
                        K::Eq => a.is_equal(k),
                        K::CaseEq => a.case_eq(k),
                        _ => unreachable!("constant branch requires an equality comparison"),
                    };
                    if !res.is_true() {
                        pc = *target as usize;
                        continue;
                    }
                }
                Insn::CmpBranch(kind, l, r, _tmp, target) => {
                    use super::bytecode::CmpKind as CK;
                    let a = &vm_regs[*l as usize];
                    let b = &vm_regs[*r as usize];
                    let res = match kind {
                        CK::Eq => a.is_equal(b),
                        CK::Neq => a.is_not_equal(b),
                        CK::CaseEq => a.case_eq(b),
                        CK::Lt => a.less_than(b),
                        CK::Leq => a.less_equal(b),
                        CK::Gt => a.greater_than(b),
                        CK::Geq => a.greater_equal(b),
                    };
                    if !res.is_true() {
                        pc = *target as usize;
                        continue;
                    }
                }
                Insn::SetSigned(reg) => {
                    vm_regs[*reg as usize].is_signed = true;
                }
                Insn::ClearSigned(reg) => {
                    vm_regs[*reg as usize].is_signed = false;
                }
                Insn::DropXz(reg) => drop_xz(&mut vm_regs[*reg as usize]),
                Insn::Pow(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].power(&vm_regs[*r as usize]);
                }
                // These should never appear in parallel-eligible blocks
                Insn::StmtFallback(..)
                | Insn::EvalExprFallback(..)
                | Insn::BlockingAssign(..)
                | Insn::BlockingAssignString(..)
                | Insn::BlockingAssignRange(..)
                | Insn::BlockingAssignRangeDyn(..)
                | Insn::BlockingAssignBitDyn(..)
                | Insn::NbaAssignRangeDyn(..)
                | Insn::BlockingAssignArray(..)
                | Insn::NbaAssignArrayRange(..)
                | Insn::BlockingAssignArrayRange(..) => {
                    unreachable!(
                        "parallel block should not contain fallback/blocking/NbaRangeDyn instructions"
                    );
                }
                // Parallel eligibility keeps instrumented blocks sequential;
                // an isolated run has no counters to add to.
                Insn::Nop | Insn::CovHit(_) => {}
            }
            pc += 1;
        }
        nba_out
    }

    /// Comb-settle counterpart of `exec_insns_isolated`. Evaluates a
    /// compiled comb block (`CompiledContAssign` / `CompiledAlwaysBlock`)
    /// against a mutable signal `view`:
    ///   - `BlockingAssign*` writes land IMMEDIATELY in `view` and append
    ///     the dirtied signal id to `dirtied` (for worklist propagation),
    ///     matching the immediate-write semantics of comb settle.
    ///   - `NbaAssign*` writes are DEFERRED into the returned queue, just
    ///     like the sequential settle queues comb-block NBAs for end-of-
    ///     tick application.
    /// Expression arms are identical to `exec_insns_isolated` but read
    /// from `view`.
    ///
    /// Returns `(deferred_nbas, unsupported)`. `unsupported=true` means the
    /// block contains an insn this evaluator can't handle — the caller must
    /// run that entry on the main thread (the `ast_ca=6` / exotic cases).
    pub(super) fn exec_comb_block_isolated(
        insns: &[super::bytecode::Insn],
        view: &mut [Value],
        signal_widths: &[u32],
        signal_signed: &[bool],
        signal_name_to_id: &NameMap,
        array_first_id: &HashMap<Arc<str>, (usize, i64, i64)>,
        vm_regs: &mut Vec<Value>,
        dirtied: &mut Vec<u32>,
        block_index: u32,
        // See `exec_insns_isolated`: CompiledBlock::nba_dup_targets.
        nba_dup: bool,
    ) -> (Vec<NbaFast>, bool) {
        use super::bytecode::Insn;
        let mut nba_out: Vec<NbaFast> = Vec::new();
        let mut unsupported = false;
        // Immediate blocking write with change detection.
        macro_rules! comb_write_full {
            ($sig_id:expr_2021, $val:expr_2021) => {{
                let sid = $sig_id;
                if view[sid] != $val {
                    view[sid] = $val;
                    dirtied.push(sid as u32);
                }
            }};
        }
        let mut pc: usize = 0;
        let len = insns.len();
        while pc < len {
            match &insns[pc] {
                // Process-FSM wait insns never appear in comb/edge blocks;
                // an isolated evaluator cannot suspend — stop the block.
                Insn::WaitDelayReg(..) | Insn::WaitEdge(..) => {
                    debug_assert!(false, "wait insn in isolated comb exec");
                    break;
                }
                Insn::LoadClassMember(..) | Insn::StoreClassMember(..) => {
                    debug_assert!(false, "class-member insn in isolated comb exec");
                    break;
                }
                Insn::CallMethod(..)
                | Insn::CallScopedMethod(..)
                | Insn::CallStaticScoped(..)
                | Insn::CallFreeFunction(..)
                | Insn::LoadClassStatic(..)
                | Insn::StoreClassStatic(..)
                | Insn::ConstructObject(..) => {
                    debug_assert!(false, "CallMethod insn in isolated comb exec");
                    break;
                }
                Insn::CallCollMethod(..) => {
                    debug_assert!(false, "CallCollMethod insn in isolated comb exec");
                    break;
                }
                Insn::InitLocalColl(..) => {
                    debug_assert!(false, "InitLocalColl in isolated comb exec");
                    break;
                }
                Insn::LoadCollElem(..) | Insn::StoreCollElem(..) => {
                    debug_assert!(false, "collection-elem insn in isolated comb exec");
                    break;
                }
                Insn::Cast(..) => {
                    debug_assert!(false, "cast insn in isolated comb exec");
                    break;
                }
                Insn::ForeachKeys(..) | Insn::ForeachNext(..) => {
                    debug_assert!(false, "foreach insn in isolated comb exec");
                    break;
                }
                Insn::RetCollection(..) | Insn::AssignPendingColl(..) => {
                    debug_assert!(false, "ret-collection insn in isolated comb exec");
                    break;
                }
                Insn::LoadConst(dest, val) => {
                    vm_regs[*dest as usize] = (**val).clone();
                }
                Insn::LoadSignal(dest, sig_id) => {
                    let sig_id = &(*sig_id as usize);
                    vm_regs[*dest as usize] = view[*sig_id].clone();
                }
                Insn::LoadSignalSigned(dest, sig_id) => {
                    let sig_id = &(*sig_id as usize);
                    let mut v = view[*sig_id].clone();
                    v.is_signed = true;
                    vm_regs[*dest as usize] = v;
                }
                Insn::LoadProcessLocal(dest, _) => {
                    vm_regs[*dest as usize] = Value::new(1);
                    unsupported = true;
                }
                Insn::Resize(reg, width) => {
                    let r = *reg as usize;
                    if vm_regs[r].width != *width {
                        let resized = vm_regs[r].resize(*width);
                        vm_regs[r] = resized;
                    }
                }
                Insn::Add(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].add(&vm_regs[*r as usize]);
                }
                Insn::Sub(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].sub(&vm_regs[*r as usize]);
                }
                Insn::Mul(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].mul(&vm_regs[*r as usize]);
                }
                Insn::Div(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].div(&vm_regs[*r as usize]);
                }
                Insn::Mod(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].modulo(&vm_regs[*r as usize]);
                }
                Insn::BitAnd(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].bitwise_and(&vm_regs[*r as usize]);
                }
                Insn::BitOr(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].bitwise_or(&vm_regs[*r as usize]);
                }
                Insn::BitXor(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].bitwise_xor(&vm_regs[*r as usize]);
                }
                Insn::BitXnor(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize]
                        .bitwise_xor(&vm_regs[*r as usize])
                        .bitwise_not();
                }
                Insn::LogAnd(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].logic_and(&vm_regs[*r as usize]);
                }
                Insn::LogOr(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].logic_or(&vm_regs[*r as usize]);
                }
                Insn::Eq(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].is_equal(&vm_regs[*r as usize]);
                }
                Insn::Neq(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].is_not_equal(&vm_regs[*r as usize]);
                }
                Insn::CaseEq(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].case_eq(&vm_regs[*r as usize]);
                }
                Insn::CaseLut(d, src, lut) => {
                    let s = &vm_regs[*src as usize];
                    let hit = if s.has_xz() {
                        None
                    } else {
                        s.to_u64().and_then(|i| lut.table.get(i as usize))
                    };
                    vm_regs[*d as usize] = hit.unwrap_or(&lut.default).clone();
                }
                // Fused `LoadConst` + binop. Character-for-character the
                // `Add`/`Eq`/`CaseEq` arms above with `&vm_regs[r]` replaced by
                // the embedded constant, which the elided `LoadConst` would
                // have cloned into `vm_regs[r]` verbatim — so the 4-state,
                // signedness and §5.7.1 `is_fill` rules cannot drift.
                Insn::BinOpConstAdd2(a) => {
                    vm_regs[a.d1 as usize] = vm_regs[a.s1 as usize].add(&a.k1);
                    vm_regs[a.d2 as usize] = vm_regs[a.s2 as usize].add(&a.k2);
                }
                Insn::BinOpConst(d, s, k, kind) => {
                    use super::bytecode::BinOpConstKind as K;
                    let (d, s) = (*d as usize, *s as usize);
                    match kind {
                        K::Add => vm_regs[d] = vm_regs[s].add(&**k),
                        K::Eq => vm_regs[d] = vm_regs[s].is_equal(&**k),
                        K::CaseEq => vm_regs[d] = vm_regs[s].case_eq(&**k),
                        K::Xor => vm_regs[d] = vm_regs[s].bitwise_xor(&**k),
                        K::And => vm_regs[d] = vm_regs[s].bitwise_and(&**k),
                        K::Or => vm_regs[d] = vm_regs[s].bitwise_or(&**k),
                        K::Mul => vm_regs[d] = vm_regs[s].mul(&**k),
                        K::Sub => vm_regs[d] = vm_regs[s].sub(&**k),
                    }
                }
                Insn::CasezEq(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].casez_eq(&vm_regs[*r as usize]);
                }
                Insn::CasexEq(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].casex_eq(&vm_regs[*r as usize]);
                }
                Insn::Lt(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].less_than(&vm_regs[*r as usize]);
                }
                Insn::Leq(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].less_equal(&vm_regs[*r as usize]);
                }
                Insn::Gt(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].greater_than(&vm_regs[*r as usize]);
                }
                Insn::Geq(d, l, r) => {
                    vm_regs[*d as usize] =
                        vm_regs[*l as usize].greater_equal(&vm_regs[*r as usize]);
                }
                Insn::Shl(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].shift_left(&vm_regs[*r as usize]);
                }
                Insn::Shr(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].shift_right(&vm_regs[*r as usize]);
                }
                Insn::AShr(d, l, r) => {
                    vm_regs[*d as usize] =
                        vm_regs[*l as usize].arith_shift_right(&vm_regs[*r as usize]);
                }
                Insn::BitNot(d, s) => {
                    vm_regs[*d as usize] = vm_regs[*s as usize].bitwise_not();
                }
                Insn::LogNot(d, s) => {
                    vm_regs[*d as usize] = vm_regs[*s as usize].logic_not();
                }
                Insn::Negate(d, s) => {
                    let w = vm_regs[*s as usize].width;
                    let mut r = Value::zero(w).sub(&vm_regs[*s as usize]).resize(w);
                    r.is_signed = vm_regs[*s as usize].is_signed;
                    vm_regs[*d as usize] = r;
                }
                Insn::ReduceAnd(d, s) => {
                    vm_regs[*d as usize] = vm_regs[*s as usize].reduce_and();
                }
                Insn::ReduceOr(d, s) => {
                    vm_regs[*d as usize] = vm_regs[*s as usize].reduce_or();
                }
                Insn::ReduceXor(d, s) => {
                    vm_regs[*d as usize] = vm_regs[*s as usize].reduce_xor();
                }
                Insn::BitSelect(d, base, idx) => {
                    // §11.5.1: an x/z or negative index reads x.
                    vm_regs[*d as usize] = match vm_regs[*idx as usize].to_index() {
                        Some(i) if i >= 0 => vm_regs[*base as usize].bit_select(i as usize),
                        _ => Value::new(1),
                    };
                }
                Insn::BitSelectConst(d, base, idx) => {
                    vm_regs[*d as usize] = vm_regs[*base as usize].bit_select(*idx as usize);
                }
                Insn::RangeSelectW(d, base, idx, w, up) => {
                    // §11.5.1: an x/z index reads a whole-x select of the
                    // declared width; otherwise the bounds are formed here.
                    let dv = match vm_regs[*idx as usize].to_index() {
                        None => Value::new(*w),
                        Some(i) => {
                            let i = i as i32 as i64;
                            let (lo, hi) = if *up {
                                (i, i + *w as i64 - 1)
                            } else {
                                (i - *w as i64 + 1, i)
                            };
                            oob_range_select(&vm_regs[*base as usize], hi, lo)
                        }
                    };
                    vm_regs[*d as usize] = dv;
                }
                Insn::RangeSelect(d, base, l, r) => {
                    // §11.5.1: bounds are 32-bit index arithmetic; a `[l -: W]`
                    // whose low bound went negative wraps to a large u32. Recover
                    // the signed value and, only when the low bound is <0, take
                    // the x-filling signed path (the fast path handles the common
                    // in-range + high-overrun cases).
                    // §11.5.1: an x/z bound selects nothing: whole-x at the
                    // select's width, recovered from the masked bound values
                    // (an indexed form's bounds differ by the constant width
                    // even when the base is unknown).
                    let (av, bv) = (&vm_regs[*l as usize], &vm_regs[*r as usize]);
                    let (Some(a), Some(b)) = (av.to_index(), bv.to_index()) else {
                        let a = av.to_u64().unwrap_or(0) as u32 as i32 as i64;
                        let b = bv.to_u64().unwrap_or(0) as u32 as i32 as i64;
                        vm_regs[*d as usize] = Value::new(((a - b).unsigned_abs() + 1) as u32);
                        pc += 1;
                        continue;
                    };
                    let (a, b) = (a as i32 as i64, b as i32 as i64);
                    let (lo, hi) = (a.min(b), a.max(b));
                    vm_regs[*d as usize] = oob_range_select(&vm_regs[*base as usize], hi, lo);
                }
                Insn::RangeSelectConst(d, base, l, r) => {
                    vm_regs[*d as usize] = oob_range_select(
                        &vm_regs[*base as usize],
                        (*l).max(*r) as i64,
                        (*l).min(*r) as i64,
                    );
                }
                // Fused load+select (finish() peephole).
                Insn::LoadSignalRangeDyn(d, sig_id, lo_reg, w) => {
                    // §11.5.1: an x/z offset selects nothing — whole-x at the
                    // slice's width. Otherwise read the slice in place; the
                    // signal is never copied.
                    let lo_v = &vm_regs[*lo_reg as usize];
                    vm_regs[*d as usize] = match lo_v.to_index() {
                        Some(lo) => {
                            let lo = lo as i32 as i64;
                            oob_range_select(&view[*sig_id as usize], lo + *w as i64 - 1, lo)
                        }
                        None => Value::new(*w),
                    };
                }
                Insn::LoadSignalRange(d, sig_id, l, r) => {
                    let sig_id = &(*sig_id as usize);
                    // §11.5.1: a select past the MSB is partially out of range.
                    vm_regs[*d as usize] =
                        oob_range_select(&view[*sig_id], (*l).max(*r) as i64, (*l).min(*r) as i64);
                }
                Insn::LoadSignalBit(d, sig_id, idx) => {
                    let sig_id = &(*sig_id as usize);
                    vm_regs[*d as usize] = view[*sig_id].bit_select(*idx as usize);
                }
                Insn::Concat(d, part_regs) => {
                    // `concat(&clones)` is literally `concat_refs(slice.iter())`
                    // (see `Value::concat`), so the `Vec<Value>` — and the heap
                    // allocation every `Wide` clone in it performs — was pure
                    // overhead. Borrow the registers directly.
                    let result =
                        Value::concat_refs(part_regs.iter().map(|r| &vm_regs[*r as usize]));
                    vm_regs[*d as usize] = result;
                }
                Insn::Replicate(d, s, n) => {
                    let val = vm_regs[*s as usize].clone();
                    if *n == 0 {
                        vm_regs[*d as usize] = Value::zero(0);
                    } else if *n == 1 {
                        vm_regs[*d as usize] = val;
                    } else {
                        let result = Value::concat_refs(std::iter::repeat_n(&val, *n as usize));
                        vm_regs[*d as usize] = result;
                    }
                }
                Insn::BranchIfFalse(reg, target) => {
                    if !vm_regs[*reg as usize].is_true() {
                        pc = *target as usize;
                        continue;
                    }
                }
                Insn::NbaAssignConst(sig_id, k, _width) => {
                    let sig_id = &(*sig_id as usize);
                    // Const pre-resized at fuse time: compare, clone only on change.
                    // §10.4.2 last-write-wins: an entry already queued for this
                    // signal supersedes the snapshot view.
                    if let Some(i) = if nba_dup {
                        nba_out.iter().rposition(|n| n.signal_id == *sig_id)
                    } else {
                        None
                    } {
                        nba_out[i].value = (**k).clone();
                    } else if view[*sig_id] != **k {
                        nba_out.push(NbaFast {
                            signal_id: *sig_id,
                            value: (**k).clone(),
                            block_index,
                        });
                    }
                }
                Insn::BranchUnlessZero(reg, target) => {
                    // Fused LogNot+BranchIfFalse: jump unless DEFINITE zero —
                    // exact composition of logic_not with !is_true (X branches).
                    if vm_regs[*reg as usize].is_nonzero() != Some(false) {
                        pc = *target as usize;
                        continue;
                    }
                }
                Insn::BranchIfSignalFalse(sig_id, target, bit) => {
                    let sig_id = &(*sig_id as usize);
                    let cond_true = if *bit == u32::MAX {
                        view[*sig_id].is_true()
                    } else {
                        view[*sig_id].bit_select(*bit as usize).is_true()
                    };
                    if !cond_true {
                        pc = *target as usize;
                        continue;
                    }
                }
                Insn::Select(dest, cond, then_r, else_r) => {
                    // §11.4.11 + §5.7.1: a fill branch ('z/'1/...) takes the
                    // width of the other branch (tristate lowering builds
                    // `cond ? then : 'z`; a 1-bit z leaking through would
                    // zero-extend at the assign instead of z-filling).
                    {
                        let (tw, tf) = (
                            vm_regs[*then_r as usize].width,
                            vm_regs[*then_r as usize].is_fill,
                        );
                        let (ew, ef) = (
                            vm_regs[*else_r as usize].width,
                            vm_regs[*else_r as usize].is_fill,
                        );
                        if tf && !ef {
                            vm_regs[*then_r as usize] = vm_regs[*then_r as usize].resize(ew);
                        } else if ef && !tf {
                            vm_regs[*else_r as usize] = vm_regs[*else_r as usize].resize(tw);
                        }
                    }
                    let v = if vm_regs[*cond as usize].has_unknown() {
                        let t = &vm_regs[*then_r as usize];
                        let e = &vm_regs[*else_r as usize];
                        // §11.3.1: a real result can't hold X, so bit-merging the
                        // branches reads their IEEE-754 bits as garbage (a real
                        // 1000.0 came out as 4.65e18). Return a defined real value
                        // (x condition → the else/false branch) instead.
                        if t.is_real || e.is_real {
                            e.clone()
                        } else {
                            t.merge_unknown(e)
                        }
                    } else if vm_regs[*cond as usize].is_true() {
                        vm_regs[*then_r as usize].clone()
                    } else {
                        vm_regs[*else_r as usize].clone()
                    };
                    vm_regs[*dest as usize] = v;
                }
                Insn::Jump(target) => {
                    pc = *target as usize;
                    continue;
                }
                Insn::CaseJump(src, cj) => {
                    // Compile gate guarantees the selector is unsigned and
                    // <=64 bits, so raw numeric dispatch matches the ===
                    // chain it replaces; any x/z bit matches no constant
                    // pattern -> default (table holes also hold default).
                    let s = &vm_regs[*src as usize];
                    let t = if s.has_xz() {
                        cj.default
                    } else {
                        match s.to_u64() {
                            Some(i) => cj.table.get(i as usize).copied().unwrap_or(cj.default),
                            None => cj.default,
                        }
                    };
                    pc = t as usize;
                    continue;
                }
                Insn::CaseMaskJump(src, mj) => {
                    // Dispatch on the always-defined window; any x/z there
                    // (or non-inline storage) walks the full chain instead.
                    let s = &vm_regs[*src as usize];
                    let t = match s.inline_bits() {
                        Some((v, xz)) if !s.is_fill => {
                            let m = if s.width >= 64 {
                                u64::MAX
                            } else {
                                (1u64 << s.width) - 1
                            };
                            let wmask = (1u64 << mj.width) - 1;
                            if ((xz & m) >> mj.lo) & wmask == 0 {
                                mj.table[(((v & m) >> mj.lo) & wmask) as usize]
                            } else {
                                mj.xz_path
                            }
                        }
                        _ => mj.xz_path,
                    };
                    pc = t as usize;
                    continue;
                }
                Insn::Format(dst, f) => {
                    vm_regs[*dst as usize] = Self::exec_format(f, vm_regs);
                }
                Insn::StrOp(dst, kind, args) => {
                    vm_regs[*dst as usize] = Self::exec_str_op(*kind, args, vm_regs);
                }
                Insn::Move(d, s) => {
                    vm_regs[*d as usize] = vm_regs[*s as usize].clone();
                }
                Insn::MoveResize(d, s, w) => {
                    // Exact composition of Move+Resize, including the Resize
                    // arm's runtime same-width skip (which preserves a fill
                    // value's `is_fill` verbatim).
                    let sv = &vm_regs[*s as usize];
                    vm_regs[*d as usize] = if sv.width != *w {
                        sv.resize(*w)
                    } else {
                        sv.clone()
                    };
                }
                Insn::CmpBranchConst(kind, src, k, _tmp, target) => {
                    use super::bytecode::BinOpConstKind as K;
                    let a = &vm_regs[*src as usize];
                    let res = match kind {
                        K::Eq => a.is_equal(k),
                        K::CaseEq => a.case_eq(k),
                        _ => unreachable!("constant branch requires an equality comparison"),
                    };
                    if !res.is_true() {
                        pc = *target as usize;
                        continue;
                    }
                }
                Insn::CmpBranch(kind, l, r, _tmp, target) => {
                    use super::bytecode::CmpKind as CK;
                    let a = &vm_regs[*l as usize];
                    let b = &vm_regs[*r as usize];
                    let res = match kind {
                        CK::Eq => a.is_equal(b),
                        CK::Neq => a.is_not_equal(b),
                        CK::CaseEq => a.case_eq(b),
                        CK::Lt => a.less_than(b),
                        CK::Leq => a.less_equal(b),
                        CK::Gt => a.greater_than(b),
                        CK::Geq => a.greater_equal(b),
                    };
                    if !res.is_true() {
                        pc = *target as usize;
                        continue;
                    }
                }
                Insn::SetSigned(reg) => {
                    vm_regs[*reg as usize].is_signed = true;
                }
                Insn::ClearSigned(reg) => {
                    vm_regs[*reg as usize].is_signed = false;
                }
                Insn::DropXz(reg) => drop_xz(&mut vm_regs[*reg as usize]),
                Insn::Pow(d, l, r) => {
                    vm_regs[*d as usize] = vm_regs[*l as usize].power(&vm_regs[*r as usize]);
                }
                Insn::LoadArrayElem(dest, array_name, idx_reg) => {
                    // §7.4.6 / §11.5.1: an UNKNOWN index reads x, NOT element 0.
                    // `to_u64()` returns None for an x/z value, and the old
                    // `unwrap_or(0)` silently read arr[0] — an AES S-box
                    // indexed by an uninitialized input returned real table
                    // data, so downstream logic resolved to a definite WRONG
                    // value instead of propagating x. A sentinel below `lo`
                    // makes the element resolve MISS, taking the existing
                    // out-of-range arm that already yields x.
                    let idx = {
                        let ir = &vm_regs[*idx_reg as usize];
                        // x/z index: force a resolve MISS so the existing
                        // out-of-range arm yields x. `to_u64()` masks x bits to
                        // ZERO and never returns None, so it cannot be used to
                        // detect this -- the old code silently read arr[0].
                        if ir.has_xz() {
                            i64::MIN
                        } else {
                            ir.to_u64().unwrap_or(0) as i64
                        }
                    };
                    let id = resolve_bytecode_array_elem(
                        array_name,
                        idx,
                        array_first_id,
                        signal_name_to_id,
                    );
                    // A packed-arena memory has no slot in `view` (not even
                    // for the element width of an out-of-range read): hand
                    // the entry back to the main thread.
                    if bytecode_array_first_id(array_name, array_first_id).is_some_and(is_packed_id)
                    {
                        unsupported = true;
                        break;
                    }
                    if let Some(eid) = id {
                        vm_regs[*dest as usize] = view[eid].clone();
                    } else {
                        vm_regs[*dest as usize] =
                            Value::new(bytecode_array_elem_width(array_name, array_first_id, view));
                    }
                }
                // ── Immediate (blocking) writes — comb semantics ──
                Insn::BlockingAssign(sig_id, val_reg, width) => {
                    let sig_id = &(*sig_id as usize);
                    // Mirror the canonical `exec_insns` BlockingAssign exactly
                    // so the isolated evaluator is bit-identical (incl X) to
                    // the monolithic settle. The fast path MASKS v/x to the
                    // assign width (clearing any stray high X bits that
                    // resize_for_assign can leave — the source of the PLIC
                    // `*_eq_vec_exp` wider-X mismatch) and stamps is_signed
                    // (the source of the 522 is_signed repr_diffs).
                    let id = *sig_id;
                    let w = *width;
                    let mut handled = false;
                    if w > 0 && w <= 64 && signal_widths[id] == w {
                        let mask = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
                        let (sv, sx) = vm_regs[*val_reg as usize].raw_bits();
                        let (sv, sx) = (sv & mask, sx & mask);
                        let (dv, dx) = view[id].raw_bits();
                        if sv == (dv & mask) && sx == (dx & mask) {
                            handled = true;
                        } else if view[id].set_inline_bits(sv, sx) {
                            view[id].is_signed = signal_signed[id];
                            dirtied.push(id as u32);
                            handled = true;
                        }
                    }
                    if !handled {
                        let mut val = vm_regs[*val_reg as usize].resize(w);
                        val.is_signed = signal_signed[id];
                        comb_write_full!(id, val);
                    }
                }
                Insn::BlockingAssignString(sig_id, val_reg) => {
                    // §6.16: store as-is — the table width is a placeholder
                    // and a resize would drop the front of the text.
                    let id = *sig_id as usize;
                    let val = vm_regs[*val_reg as usize].clone();
                    comb_write_full!(id, val);
                }
                Insn::BlockingAssignRange(sig_id, hi, lo, val_reg) => {
                    let sig_id = &(*sig_id as usize);
                    let (low, high) = if hi >= lo { (*lo, *hi) } else { (*hi, *lo) };
                    let w = high - low + 1;
                    let val = vm_regs[*val_reg as usize].resize(w);
                    // Compare FIRST (no clone): flop reloads usually match, and
                    // the unconditional clone of a wide signal (1 byte/bit Vec)
                    // was a top allocator cost on c910. Clone + merge only from
                    // the first differing bit.
                    let mut first_diff = None;
                    {
                        let src = &view[*sig_id];
                        for bit_pos in low..=high {
                            if src.get_bit(bit_pos as usize)
                                != val.get_bit((bit_pos - low) as usize)
                            {
                                first_diff = Some(bit_pos);
                                break;
                            }
                        }
                    }
                    if let Some(start) = first_diff {
                        let mut new_val = view[*sig_id].clone();
                        for bit_pos in start..=high {
                            new_val
                                .set_bit(bit_pos as usize, val.get_bit((bit_pos - low) as usize));
                        }
                        view[*sig_id] = new_val;
                        dirtied.push(*sig_id as u32);
                    }
                }
                Insn::BlockingAssignBitDyn(sig_id, idx_reg, val_reg) => {
                    let sig_id = &(*sig_id as usize);
                    // §11.5.1: an x/z index modifies no bits.
                    if let Some(idx) = dyn_bit_index(&vm_regs[*idx_reg as usize]) {
                        let bit = vm_regs[*val_reg as usize].get_bit(0);
                        if view[*sig_id].get_bit(idx) != bit {
                            let mut new_val = view[*sig_id].clone();
                            new_val.set_bit(idx, bit);
                            view[*sig_id] = new_val;
                            dirtied.push(*sig_id as u32);
                        }
                    }
                }
                Insn::BlockingAssignRangeDyn(sig_id, hi_reg, lo_reg, val_reg) => {
                    let sig_id = &(*sig_id as usize);
                    // §11.5.1: an x/z bound discards the write.
                    let (Some(hi), Some(lo)) = (
                        vm_regs[*hi_reg as usize].to_index(),
                        vm_regs[*lo_reg as usize].to_index(),
                    ) else {
                        pc += 1;
                        continue;
                    };
                    let (hi, lo) = (hi as u32, lo as u32);
                    let (low, high) = if hi >= lo { (lo, hi) } else { (hi, lo) };
                    let w = high - low + 1;
                    let val = vm_regs[*val_reg as usize].resize(w);
                    let id = *sig_id;
                    let sig_w = signal_widths[id];
                    let high_eff = high.min(sig_w.saturating_sub(1));
                    if oob_write_discard(high as i64, low as i64, sig_w) {
                        pc += 1;
                        continue;
                    }
                    if low == 0 && high_eff + 1 >= sig_w {
                        let mut v = val.resize_for_assign(sig_w);
                        v.is_signed = signal_signed[id];
                        if view[id] != v {
                            view[id] = v;
                            dirtied.push(id as u32);
                        }
                    } else {
                        let mut new_val = view[id].clone();
                        let mut changed = false;
                        for bit_pos in low..=high_eff {
                            let src_bit = val.get_bit((bit_pos - low) as usize);
                            if new_val.get_bit(bit_pos as usize) != src_bit {
                                new_val.set_bit(bit_pos as usize, src_bit);
                                changed = true;
                            }
                        }
                        if changed {
                            new_val.is_signed = signal_signed[id];
                            view[id] = new_val;
                            dirtied.push(id as u32);
                        }
                    }
                }
                Insn::BlockingAssignArray(array_name, idx_reg, val_reg, width) => {
                    // §11.5.1: an x/z index selects no element: the write
                    // is discarded (an out-of-range index is dropped by the
                    // element resolver below).
                    let Some(idx) = vm_regs[*idx_reg as usize].to_index() else {
                        pc += 1;
                        continue;
                    };
                    let id = resolve_bytecode_array_elem(
                        array_name,
                        idx,
                        array_first_id,
                        signal_name_to_id,
                    );
                    if let Some(eid) = id {
                        if is_packed_id(eid) {
                            unsupported = true;
                            break;
                        }
                        let val = vm_regs[*val_reg as usize].resize(*width);
                        comb_write_full!(eid, val);
                    }
                }
                // ── Deferred (non-blocking) writes — queued like settle ──
                Insn::NbaAssign(sig_id, val_reg, width) => {
                    let sig_id = &(*sig_id as usize);
                    let val = vm_regs[*val_reg as usize].resize_for_assign(*width);
                    // §10.4.2 last-write-wins: see the NbaAssignConst arm.
                    if let Some(i) = if nba_dup {
                        nba_out.iter().rposition(|n| n.signal_id == *sig_id)
                    } else {
                        None
                    } {
                        nba_out[i].value = val;
                    } else if view[*sig_id] != val {
                        nba_out.push(NbaFast {
                            signal_id: *sig_id,
                            value: val,
                            block_index,
                        });
                    }
                }
                // Fused LoadSignal + LoadArrayElem + NbaAssign — see the
                // `exec_insns_isolated` sibling; reads come from `view`.
                Insn::NbaAssignArrayRead(sig_id, array_name, idx_sig, width) => {
                    // §11.5.1: an x/z index reads an all-x element.
                    let val = match view[*idx_sig as usize].to_index() {
                        None => Value::new((*width).max(1)),
                        Some(idx) => match resolve_bytecode_array_elem(
                            array_name,
                            idx,
                            array_first_id,
                            signal_name_to_id,
                        ) {
                            Some(eid) if is_packed_id(eid) => {
                                unsupported = true;
                                break;
                            }
                            Some(eid) => view[eid].resize_for_assign(*width),
                            None => Value::new((*width).max(1)),
                        },
                    };
                    let sig_id = &(*sig_id as usize);
                    if let Some(i) = if nba_dup {
                        nba_out.iter().rposition(|n| n.signal_id == *sig_id)
                    } else {
                        None
                    } {
                        nba_out[i].value = val;
                    } else if view[*sig_id] != val {
                        nba_out.push(NbaFast {
                            signal_id: *sig_id,
                            value: val,
                            block_index,
                        });
                    }
                }
                Insn::NbaAssignRange(sig_id, hi, lo, val_reg) => {
                    let sig_id = &(*sig_id as usize);
                    let (low, high) = if hi >= lo { (*lo, *hi) } else { (*hi, *lo) };
                    if oob_write_discard(high as i64, low as i64, signal_widths[*sig_id]) {
                        pc += 1;
                        continue;
                    }
                    let w = high - low + 1;
                    let val = vm_regs[*val_reg as usize].resize(w);
                    let existing = nba_out.iter().rposition(|n| n.signal_id == *sig_id);
                    if let Some(i) = existing {
                        let mut new_val = nba_out[i].value.clone();
                        for bit_pos in low..=high {
                            new_val
                                .set_bit(bit_pos as usize, val.get_bit((bit_pos - low) as usize));
                        }
                        nba_out[i].value = new_val;
                    } else {
                        // Compare FIRST (no clone): flop reloads usually match, and
                        // the unconditional clone of a wide signal (1 byte/bit Vec)
                        // was a top allocator cost on c910. Clone + merge only from
                        // the first differing bit.
                        let mut first_diff = None;
                        {
                            let src = &view[*sig_id];
                            for bit_pos in low..=high {
                                if src.get_bit(bit_pos as usize)
                                    != val.get_bit((bit_pos - low) as usize)
                                {
                                    first_diff = Some(bit_pos);
                                    break;
                                }
                            }
                        }
                        if let Some(start) = first_diff {
                            let mut new_val = view[*sig_id].clone();
                            for bit_pos in start..=high {
                                new_val.set_bit(
                                    bit_pos as usize,
                                    val.get_bit((bit_pos - low) as usize),
                                );
                            }
                            nba_out.push(NbaFast {
                                signal_id: *sig_id,
                                value: new_val,
                                block_index,
                            });
                        }
                    }
                }
                Insn::NbaAssignBitDyn(sig_id, idx_reg, val_reg) => {
                    let sig_id = &(*sig_id as usize);
                    // §11.5.1: an x/z index modifies no bits.
                    if let Some(idx) = dyn_bit_index(&vm_regs[*idx_reg as usize]) {
                        let bit = vm_regs[*val_reg as usize].get_bit(0);
                        let existing = nba_out.iter().rposition(|n| n.signal_id == *sig_id);
                        if let Some(i) = existing {
                            let mut new_val = nba_out[i].value.clone();
                            new_val.set_bit(idx, bit);
                            nba_out[i].value = new_val;
                        } else if view[*sig_id].get_bit(idx) != bit {
                            let mut new_val = view[*sig_id].clone();
                            new_val.set_bit(idx, bit);
                            nba_out.push(NbaFast {
                                signal_id: *sig_id,
                                value: new_val,
                                block_index,
                            });
                        }
                    }
                }
                Insn::NbaAssignArray(array_name, idx_reg, val_reg, width) => {
                    // §11.5.1: an x/z index selects no element: the write
                    // is discarded (an out-of-range index is dropped by the
                    // element resolver below).
                    let Some(idx) = vm_regs[*idx_reg as usize].to_index() else {
                        pc += 1;
                        continue;
                    };
                    let id = resolve_bytecode_array_elem(
                        array_name,
                        idx,
                        array_first_id,
                        signal_name_to_id,
                    );
                    if let Some(eid) = id {
                        if is_packed_id(eid) {
                            unsupported = true;
                            break;
                        }
                        let val = vm_regs[*val_reg as usize].resize(*width);
                        // §10.4.2 last-write-wins: see the NbaAssign arm.
                        if let Some(i) = if nba_dup {
                            nba_out.iter().rposition(|n| n.signal_id == eid)
                        } else {
                            None
                        } {
                            nba_out[i].value = val;
                        } else if view[eid] != val {
                            nba_out.push(NbaFast {
                                signal_id: eid,
                                value: val,
                                block_index,
                            });
                        }
                    }
                }
                Insn::Nop => {}
                // Insns this evaluator doesn't handle (dynamic-range writes,
                // array-range writes, AST fallback) — caller runs the entry
                // on the main thread.
                // EXHAUSTIVE ON PURPOSE — do not reintroduce a `_` arm here.
                //
                // These are the only `Insn`s this isolated evaluator cannot
                // run; hitting one makes the caller redo the entry on the main
                // thread. Listing them explicitly means a NEWLY ADDED opcode
                // fails to compile instead of silently landing here and being
                // treated as unsupported — which would look like a correct
                // fallback while quietly disabling parallel evaluation for
                // every block containing it. That silent-acceptance mode is
                // how this codebase has lost coverage before (the JIT's
                // `is_supported` list rotted to 28% with no build error), and
                // this match is the safety net for porting the exec loops to a
                // new instruction representation, so it must not have a hole.
                Insn::NbaAssignRangeDyn(..)
                | Insn::NbaAssignArrayRange(..)
                | Insn::BlockingAssignArrayRange(..)
                | Insn::StmtFallback(..)
                | Insn::EvalExprFallback(..)
                | Insn::CovHit(..) => {
                    unsupported = true;
                }
            }
            pc += 1;
        }
        (nba_out, unsupported)
    }

    /// Execute a compiled bytecode block. Returns true if executed successfully.
    #[inline]
    /// XEZIM_TRACE_ALWAYS: emit one line per always-block firing. `label`
    /// is built lazily by the caller only when tracing is on.
    pub(super) fn trace_always_fire(&self, label: &str) {
        if let Some(filter) = &self.trace_always {
            if filter.is_empty() || label.contains(filter.as_str()) {
                eprintln!("[AWTRACE] t={} {}", self.time, label);
            }
        }
    }

    /// Label for a comb entry: kind, index, scope, first written signals.
    pub(super) fn comb_entry_trace_label(&self, eidx: usize, entry: &CombEntry) -> String {
        let kind = match &entry.item {
            CombItem::AlwaysBlock { is_always_comb, .. }
            | CombItem::CompiledAlwaysBlock { is_always_comb, .. } => {
                if *is_always_comb {
                    "always_comb"
                } else {
                    "always@*"
                }
            }
            CombItem::ContAssign { .. } | CombItem::CompiledContAssign { .. } => "assign",
            _ => "comb",
        };
        let writes: Vec<&str> = entry
            .cold
            .write_signal_ids
            .iter()
            .take(3)
            .filter_map(|&id| self.name_opt(id))
            .collect();
        format!(
            "{}#{} scope={} writes={}",
            kind,
            eidx,
            entry.cold.scope_hint.as_deref().unwrap_or("-"),
            writes.join(",")
        )
    }

    /// `--trace-always` label construction for `exec_bytecode`. Kept `#[cold]`
    /// and out of line: it needs two `HashSet<String>`, a `Vec<String>` and a
    /// `format!` of stack scratch, and inlining that into `exec_bytecode` — a
    /// function whose whole job is a few loads and a call — inflated its frame
    /// and prologue for a branch that is not taken on any production run.
    #[cold]
    #[inline(never)]
    pub(super) fn exec_bytecode_trace(&mut self, block_idx: usize) {
        let label = {
            let blk = &self.edge_blocks[block_idx];
            let mut writes: HashSet<String> = HashSet::default();
            let mut reads: HashSet<String> = HashSet::default();
            Self::collect_stmt_reads(&blk.stmt, &self.module, &mut reads, &mut writes);
            let mut ws: Vec<String> = writes.into_iter().take(3).collect();
            ws.sort();
            format!(
                "always_ff#{} scope={} writes={}",
                block_idx,
                if blk.scope.is_empty() {
                    "-"
                } else {
                    blk.scope.as_str()
                },
                ws.join(",")
            )
        };
        self.trace_always_fire(&label);
    }

    /// Record the instance scope of the block about to run, for `%m`.
    /// `clear`+`push_str` keeps the existing allocation; the equality guard
    /// skips even the memcpy for the common case of consecutive blocks in one
    /// scope. See the `m_block_scope` field doc.
    #[inline]
    pub(super) fn set_m_block_scope(&mut self, scope: Option<&str>) {
        // Contents no longer correspond to an interned edge-block scope.
        self.m_block_scope_id = u32::MAX;
        self.m_block_active = true;
        match scope {
            Some(s) => {
                if self.m_block_scope != s {
                    self.m_block_scope.clear();
                    self.m_block_scope.push_str(s);
                }
            }
            None => self.m_block_scope.clear(),
        }
    }

    /// Scope for a `TsInsn::Fallback` of a comb entry: what the four-state
    /// comb path sets before evaluating the entry (`%m`, bare-name hint).
    #[cold]
    #[inline(never)]
    pub(super) fn ts_fallback_enter(&mut self) -> Option<(Option<String>, Option<String>)> {
        let e = self.ts_cur_eidx as usize;
        if e == u32::MAX as usize {
            return None;
        }
        let (p, n) = self.settle_entries_view;
        let hint = if !p.is_null() && e < n {
            // SAFETY: the settle published the taken vector's buffer, which is
            // heap-stable and outlives this call (restored before returning).
            unsafe { (*p.add(e)).cold.scope_hint.clone() }
        } else {
            self.comb_entries
                .get(e)
                .and_then(|en| en.cold.scope_hint.clone())
        };
        let prev_hint = self.name_resolve_hint.borrow().clone();
        let prev_ts = self.timescale_scope_override.take();
        self.set_m_block_scope(hint.as_deref());
        if hint.is_some() {
            *self.name_resolve_hint.borrow_mut() = hint.clone();
        }
        // `$time`/`%t` in the fallback scale to the entry's module timescale,
        // as the four-state arm arranges.
        self.timescale_scope_override = hint;
        Some((prev_hint, prev_ts))
    }

    #[cold]
    #[inline(never)]
    pub(super) fn ts_fallback_exit(&mut self, saved: Option<(Option<String>, Option<String>)>) {
        if let Some((prev_hint, prev_ts)) = saved {
            *self.name_resolve_hint.borrow_mut() = prev_hint;
            self.timescale_scope_override = prev_ts;
        }
    }

    pub(super) fn exec_bytecode(&mut self, block_idx: usize) -> bool {
        // Install THIS block's activation scope (see `activation_scope`) —
        // the same contract as run_process_stmts. exec_bytecode is the edge
        // block's activation entry from every caller (sequential dispatch,
        // the needs_hint AST-fallback branch, parallel), and its interpreter
        // can execute StmtFallback insns — raw AST statements whose
        // bare names must resolve in this block's scope even after an
        // earlier statement's hierarchical reference ratcheted the resolve
        // hint into an instance (a `u_child.x = 1` write must not redirect
        // the block's own bare names into u_child). The needs_hint branch
        // re-installs the same value around its post-return AST fallback.
        // Fast path: a nested activation whose scope is already installed
        // skips the clone and RefCell churn.
        //
        // A light block (see `edge_block_light`) never resolves a name, so
        // it needs no activation scope at all. Returning before the install
        // keeps the per-firing String clone off the dispatch of the
        // overwhelming majority of RTL blocks.
        if self
            .edge_block_light
            .get(block_idx)
            .copied()
            .unwrap_or(false)
        {
            return self.exec_bytecode_inner(block_idx);
        }
        let already = {
            let scope = self
                .edge_blocks
                .get(block_idx)
                .map(|b| b.scope.as_str())
                .unwrap_or("");
            self.activation_scope.borrow().as_deref() == Some(scope)
        };
        if already {
            return self.exec_bytecode_inner(block_idx);
        }
        let saved_activation_scope = self.activation_scope.replace(Some(
            self.edge_blocks
                .get(block_idx)
                .map(|b| b.scope.clone())
                .unwrap_or_default(),
        ));
        let ran = self.exec_bytecode_inner(block_idx);
        *self.activation_scope.borrow_mut() = saved_activation_scope;
        ran
    }

    pub(super) fn exec_bytecode_inner(&mut self, block_idx: usize) -> bool {
        if self.trace_always.is_some() {
            self.exec_bytecode_trace(block_idx);
        }
        // Scope bookkeeping (`%m` buffer + `name_resolve_hint`) only matters
        // to blocks that can print `%m` or resolve a bare name — fallbacks
        // and `Format` insns. A "light" block (the overwhelming majority in
        // an RTL run) skips both; the hint clone in particular was a String
        // allocation on EVERY firing (~580 blocks/tick on c906).
        let light = self
            .edge_block_light
            .get(block_idx)
            .copied()
            .unwrap_or(false);
        if !light {
            self.m_block_active = true;
            // Record the firing block's instance scope for `%m`. Short-string
            // compare that usually falls through; only a genuine scope CHANGE
            // touches the buffer, and then `take`/put-back reuses the existing
            // allocation (the borrow checker needs the buffer out of `self`
            // to copy into it).
            let scope_id = self
                .edge_block_scope_id
                .get(block_idx)
                .copied()
                .unwrap_or(u32::MAX);
            if self.m_block_scope_id != scope_id {
                let mut mbs = std::mem::take(&mut self.m_block_scope);
                mbs.clear();
                mbs.push_str(&self.edge_blocks[block_idx].scope);
                self.m_block_scope = mbs;
                self.m_block_scope_id = scope_id;
            }
            // `name_resolve_hint` is a ratchet every resolution advances;
            // without a reset here, a fallback stmt in THIS block resolves
            // bare names under whatever scope the previous block or process
            // left behind — a TOP-module `$display(d)` after a sub-instance
            // block read the instance's same-named port copy. Install this
            // block's own scope (None for a top block) so intra-block
            // ratcheting still works but nothing leaks across blocks.
            let scope = &self.edge_blocks[block_idx].scope;
            *self.name_resolve_hint.borrow_mut() = if scope.is_empty() {
                None
            } else {
                Some(scope.clone())
            };
        }
        // Templated AOT slot first (same rc protocol as the JitFn path:
        // 0 = done, 1 = transient X/Z bail — interpreter runs this firing,
        // slot stays armed — >=2 = permanent, disarm).
        #[cfg(feature = "jit")]
        if let Some(Some((tf, toff, _tlen))) = self.edge_tpl_fns.get(block_idx).copied() {
            let map = unsafe { self.edge_tpl_map_blob.as_ptr().add(toff as usize) };
            let self_ptr: *mut u8 = self as *mut Self as *mut u8;
            match unsafe { tf(self_ptr, map) } {
                0 => {
                    self.prof_insns_executed += self
                        .edge_block_insn_len
                        .get(block_idx)
                        .copied()
                        .unwrap_or(0) as u64;
                    return true;
                }
                1 => {}
                _ => {
                    self.edge_tpl_fns[block_idx] = None;
                }
            }
        }
        // Fast path: if we JIT-compiled this block, call the native fn
        // directly. Zero-cost when the jit feature is off (jit_fns stays
        // empty; index returns None which short-circuits).
        if let Some(Some(jit_fn)) = self.jit_fns.get(block_idx).copied() {
            let self_ptr: *mut u8 = self as *mut Self as *mut u8;
            let rc = unsafe { jit_fn(self_ptr) };
            match rc {
                0 => {
                    // Success — JIT executed the entire block.
                    self.prof_insns_executed += self
                        .edge_block_insn_len
                        .get(block_idx)
                        .copied()
                        .unwrap_or(0) as u64;
                    return true;
                }
                1 => {
                    // Transient fallback: pre-check found X/Z on at least
                    // one input. JIT bailed BEFORE executing any side-
                    // effecting Insn — fall through to the interpreter
                    // for THIS execution but keep the JIT armed for next
                    // time (X/Z usually clears post-reset). Path B from
                    // the JIT roadmap.
                }
                _ => {
                    // Permanent fallback (rc >= 2): block hit a runtime
                    // condition the JIT can't recover from. Disable JIT
                    // for this block for the rest of the run.
                    self.jit_fns[block_idx] = None;
                }
            }
        }
        // P5: X-free edge bodies run on u64 words. NBA queue semantics are
        // preserved by StoreNba; blocking stores record §9.2 edge writes
        // through after_signal_write. The resolved slot is checked FIRST so
        // non-lowerable blocks pay one enum test; only the initial lowering
        // takes the block out (borrow) and puts it back.
        self.ts_cur_eidx = u32::MAX;
        match self.ts_edge.get(block_idx) {
            Some(TsSlot::No) => {}
            Some(TsSlot::Yes(ts)) => {
                // Raw view instead of an `Arc` clone per fire: the slot is
                // only replaced below, after the executor has returned.
                let tp: *const super::bytecode::TwoStateBlock = std::sync::Arc::as_ptr(ts);
                let ok = self.ts_guard_and_exec(unsafe { &*tp });
                if ok {
                    return true;
                }
                if self.ts_last_xbail {
                    self.ts_xbail_note(2, block_idx);
                }
                if self.ts_exec_aborted {
                    if block_idx >= self.ts_edge_abortn.len() {
                        self.ts_edge_abortn.resize(block_idx + 1, 0);
                    }
                    self.ts_edge_abortn[block_idx] =
                        self.ts_edge_abortn[block_idx].saturating_add(1);
                    if self.ts_edge_abortn[block_idx] >= TS_ABORT_DEMOTE {
                        self.ts_edge[block_idx] = TsSlot::No;
                    }
                }
            }
            _ => {
                if let Some(cb) = self.compiled_edge_blocks[block_idx].take() {
                    // Every body is offered to the lowering, tiny ones
                    // included: with registers used in place and no per-fire
                    // `Arc` traffic the two-state entry is cheaper than the
                    // interpreter even for a 4-insn 2-flop sync (the old
                    // `>= 8` gate cost 0.85% of c906 memcpy cycles).
                    let ran = self.try_two_state_in(block_idx, &cb, true);
                    self.compiled_edge_blocks[block_idx] = Some(cb);
                    if ran {
                        return true;
                    }
                }
            }
        }
        // Interpreter path.
        if !self.ts_sleep.is_empty() {
            if let Some(c) = self.ts_asleep_edge.get_mut(block_idx) {
                if *c != 0 {
                    *c = c.saturating_add(1);
                }
            }
        }
        let (insns_ptr, insns_len, num_regs) = match &self.compiled_edge_blocks[block_idx] {
            Some(cb) => (
                cb.instructions.as_ptr(),
                cb.instructions.len(),
                cb.num_regs as usize,
            ),
            None => return false,
        };
        if self.vm_regs.len() < num_regs {
            self.vm_regs.resize(num_regs, Value::zero(1));
        }
        let insns = unsafe { std::slice::from_raw_parts(insns_ptr, insns_len) };
        if self.trace_comb_paths {
            let at = self.prof_insns_executed;
            self.exec_insns(insns);
            if block_idx >= self.edge_path_counts.len() {
                self.edge_path_counts.resize(block_idx + 1, [0; 2]);
            }
            self.edge_path_counts[block_idx][0] += 1;
            self.edge_path_counts[block_idx][1] += self.prof_insns_executed - at;
            return true;
        }
        self.exec_insns(insns);
        true
    }

    /// Keep class-only dispatch out of the common VM loop.
    #[inline(never)]
    pub(super) fn exec_class_opcode(&mut self, insn: &super::bytecode::Insn) -> u64 {
        use super::bytecode::{CastDest, Insn};
        let mut local_count = 0;
        match insn {
            Insn::CallMethod(dest, handle_reg, method, arg_start, n_args) => {
                let method: &str = &method.0;
                let handle = self.vm_regs[*handle_reg as usize].to_u64().unwrap_or(0) as usize;
                let base = *arg_start as usize;
                let n = *n_args as usize;
                // class-perf P3: direct VM->VM dispatch when the callee
                // itself has a compiled fast-call entry — skips arg
                // materialization (the sized-hex round-trip) and the
                // interpreter's binding loop. Guards inside return
                // None for every shape the interpreter handles
                // specially; the historic path below is unchanged.
                if let Some(v) = self.vm_try_direct_call(handle, method, base, n) {
                    self.vm_regs[*dest as usize] = v;
                    self.vm_ret_coll = self.pending_ret_collection.clone();
                    local_count += 1;
                } else {
                    // class-perf row 2: resolve the callee once more for the
                    // write-back temp set. A wrong guess here (ctor-stack
                    // dispatch edge) merely keeps the constant wrapper —
                    // the historic behavior — never a wrong write-back.
                    let callee_class = self
                        .heap
                        .get(handle)
                        .and_then(|i| i.as_ref().map(|inst| inst.class_name.clone()));
                    let (args, temps) = match callee_class {
                        Some(cn) => self.vm_args_with_writeback(&cn, method, base, n),
                        None => {
                            let mut a = Vec::with_capacity(n);
                            for i in 0..n {
                                let v = self
                                    .vm_regs
                                    .get(base + i)
                                    .cloned()
                                    .unwrap_or_else(|| Value::zero(32));
                                a.push(self.value_method_arg_expr(&v));
                            }
                            (a, false)
                        }
                    };
                    let result = self.exec_method_call(handle, method, &args);
                    if temps {
                        self.vm_drain_arg_temps(base, n);
                    }
                    self.vm_regs[*dest as usize] = result;
                    // class-perf collection-return: only a following
                    // `AssignPendingColl` can consume a collection result, so
                    // move it there; a `pending_ret_collection` surviving here
                    // could only leak into an outer, unrelated assign (the AST
                    // path's `assign_pending_ret_collection` TAKES it too).
                    self.vm_ret_coll = self.pending_ret_collection.take();
                    local_count += 1;
                }
            }
            Insn::CallScopedMethod(dest, this_reg, names, arg_start, n_args) => {
                let (start_class, method) = (names.0.as_ref(), names.1.as_ref());
                let handle = self.vm_regs[*this_reg as usize].to_u64().unwrap_or(0) as usize;
                let base = *arg_start as usize;
                let n = *n_args as usize;
                // Value-const actuals + row-2 write-back temps, exactly
                // like CallMethod. The start class was baked at compile
                // time (§8.15 super / §8.20 non-virtual bare call): run
                // the body via the hierarchy walk from that class —
                // non-virtual, ctor chaining included.
                let (args, temps) = self.vm_args_with_writeback(start_class, method, base, n);
                let result = if handle == 0 {
                    // Null receiver (unreachable for instance frames;
                    // a static frame never lowers a scoped call).
                    Value::zero(32)
                } else {
                    self.exec_method_in_class_hierarchy(handle, start_class, method, &args)
                };
                if temps {
                    self.vm_drain_arg_temps(base, n);
                }
                self.vm_regs[*dest as usize] = result;
                self.vm_ret_coll = self.pending_ret_collection.clone();
                local_count += 1;
            }
            Insn::CallFreeFunction(dest, fname, arg_start, n_args) => 'call: {
                let base = *arg_start as usize;
                let n = *n_args as usize;
                // The callee resolves from the SAME table the compiler
                // admitted it from; a vanishing entry (module swap)
                // yields a benign zero like every other missing fn.
                let fd = self.fn_decl_rc(&fname.0);
                // §26.3: the package the callee's body runs in. A call the
                // compiler bound to `P::name` names it outright; any other
                // callee runs in its declaring scope.
                let pkg = match &fd {
                    Some(fd) => self.free_call_pkg(&fname.0, &fd.name.name.name),
                    None => None,
                };
                // Fast dispatch when this callee has a compiled block
                // (compiled→compiled chain): it reads the argument registers
                // directly, so no AST actuals are synthesized on this route.
                if let Some(fd) = &fd
                    && let Some(v) = self.vm_try_direct_free_fn_call(fd, pkg.as_deref(), base, n)
                {
                    self.vm_regs[*dest as usize] = v;
                    self.vm_ret_coll = self.pending_ret_collection.clone();
                    local_count += 1;
                    break 'call;
                }
                // DPI imports are NOT in the function table. When the callee
                // resolves to an import, `output` formals must be written
                // back into the argument registers: the DPI bridge assigns
                // through `assign_value` on the synthesized actuals, so an
                // `output` position gets a scratch lvalue whose landing in
                // `self.signals` is copied back into `vm_regs` after the
                // call. The compiled caller then moves the register into its
                // own lvalue (compile-time copyback).
                let import_ports: Option<Vec<crate::ast::decl::FunctionPort>> = if fd.is_none() {
                    self.module
                        .dpi_imports
                        .get(fname.0.as_ref())
                        .and_then(|s| match &s.proto {
                            crate::ast::decl::DPIProto::Function(f) => Some(f.ports.clone()),
                            _ => None,
                        })
                } else {
                    None
                };
                let mut out_slots: Vec<(usize, String)> = Vec::new();
                let port_types: Option<&Vec<crate::ast::decl::FunctionPort>> =
                    fd.as_ref().map(|f| &f.ports).or(import_ports.as_ref());
                let mut args = Vec::with_capacity(n);
                for i in 0..n {
                    let v = self
                        .vm_regs
                        .get(base + i)
                        .cloned()
                        .unwrap_or_else(|| Value::zero(32));
                    let port_is_string = port_types.and_then(|p| p.get(i)).is_some_and(|p| {
                        matches!(
                            &p.data_type,
                            crate::ast::types::DataType::Simple {
                                kind: crate::ast::types::SimpleType::String,
                                ..
                            }
                        )
                    });
                    let is_output = import_ports
                        .as_ref()
                        .and_then(|p| p.get(i))
                        .is_some_and(|p| p.direction == PortDirection::Output);
                    if is_output {
                        let nm = format!("__xz_dpi_out_{i}");
                        args.push(Expression::new(
                            ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                                root: None,
                                path: vec![crate::ast::expr::HierPathSegment {
                                    name: crate::ast::Identifier {
                                        name: nm.clone(),
                                        span: crate::ast::Span::dummy(),
                                    },
                                    selects: Vec::new(),
                                }],
                                span: crate::ast::Span::dummy(),
                                cached_signal_id: std::cell::Cell::new(None),
                                cached_resolved_name: std::cell::OnceCell::new(),
                            }),
                            crate::ast::Span::dummy(),
                        ));
                        out_slots.push((base + i, nm));
                    } else if port_is_string && !v.is_real {
                        // A string actual must arrive as a STRING expression:
                        // the callee's §13.5 binding and the DPI string
                        // readers consume the text, not the packed bits.
                        args.push(Expression::new(
                            ExprKind::StringLiteral(v.to_sv_string()),
                            crate::ast::Span::dummy(),
                        ));
                    } else {
                        args.push(self.value_method_arg_expr(&v));
                    }
                }
                let result = match fd {
                    Some(fd) => {
                        self.pending_pkg_scope = pkg.map(|p| p.to_string());
                        self.exec_function_call(&fd, &args)
                    }
                    None => self
                        .exec_dpi_import_call(&fname.0, &args)
                        .unwrap_or_else(|| Value::zero(32)),
                };
                for (slot, nm) in out_slots {
                    if let Some(v) = self.signals.remove(&nm) {
                        if slot < self.vm_regs.len() {
                            self.vm_regs[slot] = v;
                        }
                    }
                }
                self.vm_regs[*dest as usize] = result;
                self.vm_ret_coll = self.pending_ret_collection.clone();
                local_count += 1;
            }
            Insn::CallStaticScoped(dest, names, arg_start, n_args) => {
                let (recv, method) = (names.0.as_ref(), names.1.as_ref());
                // Step 9h: class-scope static call — marshal register
                // args and re-enter the interpreter's SHARED resolution
                // (type-param binding, embedded spec, class name, typedef
                // aliases), exactly the AST member-call path.
                let base = *arg_start as usize;
                let n = *n_args as usize;
                // class-perf Change 3: a no-arg static call may run the
                // callee's compiled block in place, skipping the per-call
                // resolution walk + `try_run_compiled_method` prologue
                // (singleton getters in hot loops). `None` falls through to
                // the historic interpreter route, byte-identical.
                let v = if n == 0 && self.module.classes.contains_key(recv) {
                    self.vm_try_direct_static_call(recv, method, base, n)
                        .unwrap_or_else(|| {
                            let (args, temps) = self.vm_args_with_writeback(recv, method, base, n);
                            let v = self
                                .exec_class_scope_static_call(recv, method, &args)
                                .unwrap_or_else(|| Value::zero(32));
                            if temps {
                                self.vm_drain_arg_temps(base, n);
                            }
                            v
                        })
                } else {
                    let (args, temps) = self.vm_args_with_writeback(recv, method, base, n);
                    let v = self
                        .exec_class_scope_static_call(recv, method, &args)
                        .unwrap_or_else(|| Value::zero(32));
                    if temps {
                        self.vm_drain_arg_temps(base, n);
                    }
                    v
                };
                self.vm_regs[*dest as usize] = v;
                self.vm_ret_coll = self.pending_ret_collection.clone();
                local_count += 1;
            }
            Insn::LoadClassStatic(dest, names) => {
                let (class, prop) = (names.0.as_ref(), names.1.as_ref());
                // Step 9g: static-property read — class_static_get does
                // the parent-chain walk AND the per-specialization
                // keying from the baked defining class, mirroring the
                // AST's unqualified-static fallback (lexical ctx).
                let v = self
                    .class_static_get(class, prop)
                    .unwrap_or_else(|| Value::zero(32));
                self.vm_regs[*dest as usize] = v;
                local_count += 1;
            }
            Insn::StoreClassStatic(names, src) => {
                let (class, prop) = (names.0.as_ref(), names.1.as_ref());
                // Step 9g: static-property store — the Value is stored
                // UNRESIZED (class_static_set semantics), immediately
                // visible to the interpreter.
                let v = self.vm_regs[*src as usize].clone();
                self.class_static_set(class, prop, v);
            }
            Insn::ConstructObject(dest, class, arg_start, n_args) => {
                let class: &str = &class.0;
                // Step 9g: `new(args)` / bare `new` — §8.8 constructor
                // of the baked DEFINING class. Allocation, property
                // inits, and the ctor chain all stay in
                // instantiate_class (interpreter-owned).
                let base = *arg_start as usize;
                let mut argvals: Vec<Value> = Vec::with_capacity(*n_args as usize);
                for i in 0..*n_args as usize {
                    argvals.push(
                        self.vm_regs
                            .get(base + i)
                            .cloned()
                            .unwrap_or(Value::zero(32)),
                    );
                }
                let args: Vec<Expression> = argvals
                    .iter()
                    .map(|v| self.value_method_arg_expr(v))
                    .collect();
                let v = if let Some(cd) = self.module.classes.get(class).cloned() {
                    self.instantiate_class(&cd, &args)
                } else {
                    Value::zero(32)
                };
                self.vm_regs[*dest as usize] = v;
                local_count += 1;
            }
            Insn::CallCollMethod(dest, handle_reg, names, arg_start, n_args, bare) => {
                let (member, method) = (names.0.as_ref(), names.1.as_ref());
                let handle = self.vm_regs[*handle_reg as usize].to_u64().unwrap_or(0) as usize;
                let base = *arg_start as usize;
                let n = *n_args as usize;
                // Value-const actuals + row-2 write-back temps (the
                // builtin arms read/eval them through the ordinary
                // expression paths — §7.10.2.3 queue append, §15.5.5
                // event keys, string keys — all carried).
                let callee_class = self
                    .heap
                    .get(handle)
                    .and_then(|i| i.as_ref().map(|inst| inst.class_name.clone()));
                let (mut args, mut temps) = (Vec::with_capacity(n), false);
                match callee_class {
                    Some(cn) => {
                        let (a, t) = self.vm_args_with_writeback(&cn, method, base, n);
                        args = a;
                        temps = t;
                    }
                    None => {
                        for i in 0..n {
                            let v = self
                                .vm_regs
                                .get(base + i)
                                .cloned()
                                .unwrap_or_else(|| Value::zero(32));
                            args.push(self.value_method_arg_expr(&v));
                        }
                    }
                }
                let result = if let Some(bare_name) = Self::split_local_coll_marker(member) {
                    // Row 3: a LOCAL collection declared in this body
                    // (the decl ran as a StmtFallback, registering the
                    // per-call `@name#id` key in the current frame).
                    // Resolve the store through the interpreter's own
                    // rename map — innermost frame first, a same-named
                    // `this` member is SHADOWED — and dispatch the
                    // builtin on it. The key contains `#`, so the
                    // builtin's bare-name rewrites never fire.
                    let m: &str = &method;
                    let store = self.local_coll_store(bare_name);
                    match self.eval_builtin_method(&store, m, &args) {
                        Some(v) => v,
                        None if handle == 0 => Value::zero(32),
                        None => self.exec_method_call(handle, m, &args),
                    }
                } else if handle == 0 && *bare == 0 {
                    // Null DOTTED receiver (`obj.coll.m()` with obj == null):
                    // the AST funnel falls through to `Value::zero(32)` (no
                    // storage, no fault). A BARE receiver must NOT take this
                    // path: in a frame with no `this` (a static method, or an
                    // instance method dispatched through a class-scope static
                    // call) the member can still be a STATIC collection of
                    // the lexical class — `instance_assoc_member` below
                    // resolves it through `class_context_stack` to the same
                    // per-spec store the interpreter uses, so a bare
                    // `q.push_back(v)` / `q.size()` in such frames must reach
                    // that arm instead of being dropped here (its writes
                    // silently vanished otherwise).
                    Value::zero(32)
                } else if *bare != 0 {
                    // Bare receiver (`q.size()`): the receiver IS
                    // `this`. Resolve the member's instance store FIRST
                    // (`instance_assoc_member`, the same resolution the
                    // AST funnel's `expr_assoc_name` does for a member
                    // receiver) and hand the builtin dispatcher the
                    // SCOPED name. Feeding the dispatcher the bare name
                    // instead let its §8.10 rewrite consult
                    // `dyn_name_lookup` first, so a same-named local of
                    // an ENCLOSING interpreter frame (e.g. UVM report
                    // hooks all declare `elements[$]`) hijacked the
                    // member store and answered a foreign snapshot's
                    // size. Scoped names contain `#`, so the rewrite
                    // chain is skipped entirely — parity by delegation
                    // to the exact store the funnel would use,
                    // including param-bound collections and per-spec
                    // static keys.
                    let m: &str = &method;
                    match self.instance_assoc_member(member) {
                        Some(scoped) => match self.eval_builtin_method(&scoped, m, &args) {
                            Some(v) => v,
                            None => self.exec_method_call(handle, m, &args),
                        },
                        // Not a member collection of `this` (unreachable
                        // for admitted shapes): keep the historic bare
                        // delegation so behavior never changes for the
                        // fallback.
                        None => match self.eval_builtin_method(member, m, &args) {
                            Some(v) => v,
                            None => self.exec_method_call(handle, m, &args),
                        },
                    }
                } else {
                    // Dotted receiver (`obj.coll.meth()`): resolve the
                    // instance-scoped store `<handle>#coll` exactly like
                    // the funnel's `expr_assoc_name` receiver arm, then
                    // dispatch the builtin on it. Scoped names contain
                    // `#`, so the builtin's bare-name rewrites never
                    // fire — pure storage ops.
                    let m: &str = &method;
                    match self.handle_collection_name(handle, member) {
                        Some(scoped) => match self.eval_builtin_method(&scoped, m, &args) {
                            Some(v) => v,
                            None => self.exec_method_call(handle, m, &args),
                        },
                        // Undotted-scoped member (param-bound collection
                        // the plan set excluded): interpret the call as
                        // the ordinary funnel would.
                        None => self.exec_method_call(handle, m, &args),
                    }
                };
                if temps {
                    self.vm_drain_arg_temps(base, n);
                }
                self.vm_regs[*dest as usize] = result;
                // class-perf collection-return leak guard (see CallMethod).
                self.vm_ret_coll = self.pending_ret_collection.take();
                local_count += 1;
            }
            // Step 9b-ii: element READ of a class member collection
            // (`q[i]`, `aa[k]`, `this.aa[k]`, `obj.aa[k]`) inside a
            // compiled method body. Mirrors the `expr_assoc_name` arm of
            // the interpreter's Index eval (the only earlier guard that
            // arm could take for these shapes is the `$` index reject,
            // which the compiler refuses to emit): resolve the store,
            // narrow the key (assoc_key_str), read `store[key]` with the
            // zero(32) missing-key default, NO width fit (the compiler
            // emitted Resize for the context width; `q[$]` is excluded
            // so dollar_bound is irrelevant).
            Insn::LoadCollElem(dest, handle_reg, member, idx_reg) => {
                let idx_val = self.vm_regs[*idx_reg as usize].clone();
                let store: Option<String> = if let Some(bare_name) =
                    Self::split_local_coll_marker(member)
                {
                    // Row 3: local collection — the interpreter's
                    // per-call `@name#id` store (innermost frame wins;
                    // a same-named `this` member is shadowed).
                    Some(self.local_coll_store(bare_name))
                } else if *handle_reg == 0 {
                    // Bare receiver: the collection is a member of
                    // `this` — the interpreter's own resolution chain
                    // (instance member, then static / param-bound
                    // special keys). A None here is unreachable for
                    // admitted shapes (the compile-side plan set is a
                    // subset); interpret as zero, like a null read.
                    self.instance_assoc_member(member)
                } else {
                    let handle = self.vm_regs[*handle_reg as usize].to_u64().unwrap_or(0) as usize;
                    // Dotted receiver: null handle reads zero, per the
                    // funnel's fall-through (`Value::zero(32)`).
                    if handle == 0 {
                        None
                    } else {
                        self.handle_collection_name(handle, member)
                    }
                };
                let value = match store {
                    Some(an) => {
                        let key = self.assoc_key_str(&an, &idx_val);
                        // class-perf: use the pre-sized manual renderer instead
                        // of `format!` — this runs once per element access on
                        // associative-array procedural loops.
                        let elem = Self::name_with_key(&an, &key);
                        self.signals
                            .get(&elem)
                            .cloned()
                            .unwrap_or_else(|| Value::zero(32))
                    }
                    None => Value::zero(32),
                };
                self.vm_regs[*dest as usize] = value;
                local_count += 1;
            }
            // Element WRITE counterpart. Mirrors the `expr_assoc_name`
            // arm of `assign_value_index` (51285): queue/dynamic-array
            // append growth, §10.7 width fit off the live store, exact
            // changed-detect insert, touch_queue on change — then the
            // same waiter notify `assign_value` would run (the AST path
            // reaches that check through the `assign_value` wrapper;
            // a synthetic name-only lvalue walks exactly like the real
            // one because the name collectors never inspect indexes).
            Insn::StoreCollElem(handle_reg, member, idx_reg, val_reg) => {
                let idx_val = self.vm_regs[*idx_reg as usize].clone();
                let val = self.vm_regs[*val_reg as usize].clone();
                let store: Option<String> = if let Some(bare_name) =
                    Self::split_local_coll_marker(member)
                {
                    // Row 3: local collection — the interpreter's
                    // per-call `@name#id` store (innermost frame wins;
                    // a same-named `this` member is shadowed).
                    Some(self.local_coll_store(bare_name))
                } else if *handle_reg == 0 {
                    self.instance_assoc_member(member)
                } else {
                    let handle = self.vm_regs[*handle_reg as usize].to_u64().unwrap_or(0) as usize;
                    if handle == 0 {
                        // Null-receiver store: the AST funnel's write
                        // resolves no store and drops silently.
                        None
                    } else {
                        self.handle_collection_name(handle, member)
                    }
                };
                if let Some(an) = store {
                    let key = self.assoc_key_str(&an, &idx_val);
                    // class-perf: pre-sized manual renderer, as in `LoadCollElem`.
                    let elem_name = Self::name_with_key(&an, &key);
                    // Keep the established queue-unpack growth behavior in
                    // sync with the AST path.
                    if !self.is_associative_array(&an) {
                        let kval = idx_val.to_i64().unwrap_or(0);
                        if kval >= 0 && kval as u64 >= self.get_queue_size(&an) {
                            self.set_queue_size(&an, kval as u64 + 1);
                        }
                    }
                    // §10.7: fit to the DECLARED element width when one
                    // was recorded; otherwise store as-is.
                    let fitted = match self.assoc_elem_width(&an) {
                        Some(w) if w != val.width && !val.is_real => val.resize_for_assign(w),
                        _ => val.clone(),
                    };
                    let changed = self.signals.get(&elem_name) != Some(&fitted);
                    self.signals.insert(elem_name, fitted);
                    if changed {
                        self.touch_queue(&an);
                        // Waiter notify, as `assign_value` does for the
                        // AST path. The synthetic lvalue is name-only —
                        // the target-name collectors ignore indexes.
                        if !self.condition_waiters.is_empty() {
                            let lhs = Expression::new(
                                ExprKind::Ident(HierarchicalIdentifier {
                                    root: None,
                                    path: vec![HierPathSegment {
                                        name: crate::ast::Identifier {
                                            name: member.to_string(),
                                            span: crate::ast::Span::dummy(),
                                        },
                                        selects: Vec::new(),
                                    }],
                                    span: crate::ast::Span::dummy(),
                                    cached_signal_id: std::cell::Cell::new(None),
                                    cached_resolved_name: std::cell::OnceCell::new(),
                                }),
                                crate::ast::Span::dummy(),
                            );
                            self.check_condition_waiters_for_write(&lhs);
                        }
                        // The AST statement route settles the comb
                        // layer after every blocking element write
                        // (`exec_stmt_blocking_assign` →
                        // `settle_after_proc_write`), even when the
                        // value did not change. Here the settle runs
                        // only on a change: with no cell changed it is
                        // a pure recompute (no observable effect), and
                        // skipping it avoids paying a full comb pass
                        // for same-value stores.
                        self.settle_after_proc_write();
                    }
                }
                local_count += 1;
            }
            // class-perf Step 9c: `$cast(dest, src)` — the runtime
            // compatibility check runs the interpreter's own
            // `cast_type_ok` on the carried dest AST (the type overlays
            // it reads are LIVE during compiled exec: formals and the
            // return cell were recorded by the binding loop, top-level
            // body decls are pre-recorded at method entry). Assignment
            // routes mirror the AST funnel: Reg = the frame arm's fit
            // into the VM register; Member = StoreClassMember semantics
            // (fit_class_prop + heap); Scope = `assign_value` on the
            // carried AST. The §8.16 task form (stmt_form) prints the
            // same runtime error on failure that the statement
            // interpreter does; the function form is silent.
            Insn::Cast(ci, src_reg, out) => {
                let (dest, dest_expr, stmt_form) = (&ci.dest, &ci.expr, ci.stmt_form);
                let v = self.vm_regs[*src_reg as usize].clone();
                let ok = self.cast_type_ok_ex(dest_expr, &v, ci.decl_class.as_deref());
                if ok {
                    match dest {
                        CastDest::Reg(r) => {
                            // Frame-arm fit (assign_value's local slot):
                            // resize against the slot's CURRENT
                            // width/realness, real→int conversion
                            // included — byte-identical to the AST
                            // path's frame write.
                            let r = *r as usize;
                            let prev = Some(self.vm_regs[r].clone());
                            let fitted = match prev.as_ref() {
                                Some(p) if p.is_real && !v.is_real => Value::from_f64(v.to_f64()),
                                Some(p) if !p.is_real && !v.is_real && p.width > 0 => {
                                    v.resize(p.width)
                                }
                                Some(p) if !p.is_real && v.is_real => {
                                    let mut f = Self::real_to_int(v.to_f64(), p.width.max(1));
                                    f.is_signed = p.is_signed;
                                    f
                                }
                                _ => v.clone(),
                            };
                            self.vm_regs[r] = fitted;
                        }
                        CastDest::Member(hreg, field) => {
                            let h = self.vm_regs[*hreg as usize].to_u64().unwrap_or(0) as usize;
                            let fitted = self.fit_class_prop(h, field, &v);
                            if let Some(o) = self.heap.get_mut(h).and_then(|o| o.as_mut()) {
                                o.properties.insert(field.to_string(), fitted);
                            }
                        }
                        CastDest::Scope => {
                            self.assign_value(dest_expr, &v);
                        }
                    }
                } else if stmt_form {
                    let m = format!(
                        "[xezim][error] $cast: source object is not \
                         assignment-compatible with the destination (t={})",
                        self.time
                    );
                    self.record_output(m.clone());
                    self.stdout_writeln(&m);
                }
                self.vm_regs[*out as usize] = if ok {
                    Value::from_u64(1, 32)
                } else {
                    Value::zero(32)
                };
                local_count += 1;
            }
            // class-perf Step 9d: materialize the iteration keys of a
            // member collection for a compiled foreach — the same store
            // resolution as LoadCollElem (bare → `instance_assoc_member`,
            // dotted → `handle_collection_name`, null receiver → no
            // store), then the interpreter's OWN key walk: `array_iter_keys`
            // (dense `.size` range for queue/dynamic, sparse first-key-seg
            // scan for assoc, string vs numeric §7.9 ordering) and
            // `assoc_index_width_for` (§7.8.2 key width/signedness). Each
            // key's loop-var Value is precomputed with the sync foreach
            // arm's exact conversion so ForeachNext can only move it.
            Insn::ForeachKeys(slot, handle_reg, member) => {
                let store: Option<String> = if let Some(bare_name) =
                    Self::split_local_coll_marker(member)
                {
                    // Row 3: local collection — the interpreter's
                    // per-call `@name#id` store (innermost frame wins;
                    // a same-named `this` member is shadowed).
                    Some(self.local_coll_store(bare_name))
                } else if *handle_reg == 0 {
                    self.instance_assoc_member(member)
                } else {
                    let handle = self.vm_regs[*handle_reg as usize].to_u64().unwrap_or(0) as usize;
                    if handle == 0 {
                        // Null receiver: no store → no keys → zero
                        // iterations, matching the interpreter's
                        // expr_assoc_name fall-through.
                        None
                    } else {
                        self.handle_collection_name(handle, member)
                    }
                };
                let (keys, is_str, kw, ks_sign) = match store {
                    Some(an) => {
                        let (ks, is_str) = self.array_iter_keys(&an);
                        let (kw, ks_sign) = self.assoc_index_width_for(&an).unwrap_or((32, false));
                        (ks, is_str, kw, ks_sign)
                    }
                    None => (Vec::new(), false, 32, false),
                };
                // Precompute each iteration's loop-var Value exactly as
                // the sync foreach arm does (string → from_string;
                // else signed parse at the declared key width).
                let vals: Vec<Value> = keys
                    .iter()
                    .map(|key| {
                        if is_str {
                            Value::from_string(key)
                        } else {
                            let mut v = Value::from_u64(key.parse::<i64>().unwrap_or(0) as u64, kw);
                            v.is_signed = ks_sign;
                            v
                        }
                    })
                    .collect();
                let s = *slot as usize;
                if s >= self.foreach_arena.len() {
                    self.foreach_arena.resize(s + 1, (Vec::new(), 0));
                }
                self.foreach_arena[s] = (vals, 0);
                local_count += 1;
            }
            // class-perf collection-return: materialize the returned literal
            // into the reserved interpreter snapshot store and hand it
            // through `pending_ret_collection`, byte-identical to the AST
            // path's `populate_queue_from_init(RET_SNAP, e)` + hand-off.
            Insn::RetCollection(arg_start, n) => {
                const RET_SNAP: &str = "__xz_ret_coll__";
                let base = *arg_start as usize;
                let n = *n as usize;
                // Fast channel: capture the flat element values straight out
                // of the callee registers so AssignPendingColl can copy them
                // into a local queue without the signal-snapshot round trip.
                let mut vals = Vec::with_capacity(n);
                for r in base..base + n {
                    vals.push(
                        self.vm_regs
                            .get(r)
                            .cloned()
                            .unwrap_or_else(|| Value::zero(32)),
                    );
                }
                self.vm_ret_vals = Some(vals);
                // Defer the string-keyed `__xz_ret_coll__[i]` snapshot: it is
                // only ever materialized on the destination path that cannot
                // consume the flat `vm_ret_vals` fast channel (see
                // `assign_pending_ret_collection`), so writing the signal cells
                // + width rows + `.size` here, before we know the destination
                // resolves to a writable base, is wasted on every hot
                // collection-return call.
                self.pending_ret_collection = Some(RET_SNAP.to_string());
                local_count += 1;
            }
            // §13.4 `q = f(..)` into a bare local collection: the tail of the
            // interpreter's blocking assign after a call RHS. The collection
            // the call handed back is copied whole into `q`; with none (a
            // plain value return) the call's result is assigned instead.
            Insn::AssignPendingColl(key, val) => {
                let bare = Self::split_local_coll_marker(key).unwrap_or(key);
                let span = crate::ast::Span::dummy();
                let lv = Self::hier_ident_expr(Self::iface_path_segs(bare, span), span);
                // Fast path: a compiled `RetCollection` stashed the flat
                // element values. Write them straight onto the resolved
                // destination (element cells + size) without materializing the
                // `__xz_ret_coll__` signal snapshot or the per-element reads
                // in `copy_whole_queue` (which scans the class table and
                // rebuilds formatted keys per element). The destination is
                // resolved by the exact helper `assign_pending_ret_collection`
                // uses, so when it cannot resolve a writable base we fall back
                // to the historic path, byte-identical.
                let resolved_dst = self.pending_coll_dst(&lv);
                if let Some(vals) = self.vm_ret_vals.take() {
                    self.pending_ret_collection = self.vm_ret_coll.take();
                    match resolved_dst {
                        Some(dst) => {
                            // Consumed: no by-name reader may see it again.
                            self.pending_ret_collection = None;
                            for (i, v) in vals.iter().enumerate() {
                                self.set_signal_value_by_name(&format!("{dst}[{i}]"), v.clone());
                            }
                            self.set_queue_size(&dst, vals.len() as u64);
                            self.settle_after_proc_write();
                        }
                        None => {
                            // Destination not resolvable to a writable base:
                            // put the values back on the fast channel and take
                            // the historic snapshot path (which also settles
                            // internally when it copies).
                            self.vm_ret_vals = Some(vals);
                            if !self.assign_pending_ret_collection(&lv) {
                                let v = self.vm_regs[*val as usize].clone();
                                self.assign_value(&lv, &v);
                                self.settle_after_proc_write();
                            }
                        }
                    }
                    local_count += 1;
                } else {
                    self.pending_ret_collection = self.vm_ret_coll.take();
                    if !self.assign_pending_ret_collection(&lv) {
                        let v = self.vm_regs[*val as usize].clone();
                        self.assign_value(&lv, &v);
                        self.settle_after_proc_write();
                    }
                    local_count += 1;
                }
            }
            _ => unreachable!("unexpected opcode in class executor"),
        }
        local_count
    }

    /// Core bytecode VM loop.
    #[inline]
    pub(super) fn exec_insns(&mut self, insns: &[super::bytecode::Insn]) {
        use super::bytecode::Insn;
        // Process-FSM resume point; 0 (the ordinary case) unless
        // `run_proc_fsm` set it just before this call.
        let mut pc: usize = std::mem::take(&mut self.fsm_start_pc) as usize;
        let len = insns.len();
        let mut local_count: u64 = 0;
        // XEZIM_OPCODE_CENSUS: dynamic opcode + adjacent-pair counts. Both
        // this flag and the SoA-read table length are hoisted into locals:
        // neither can change while the loop runs, and re-loading them from
        // `self` on every instruction measurably inflates the hot path (it
        // cost ~2.5% of retired instructions on the c906 memcpy when read
        // per-iteration).
        // Compile-time gated: the per-instruction census costs a load+branch
        // on the VM's critical path (measured ~1% of the whole c906 run, for a
        // tool that is off by default), and `perf annotate` shows this loop is
        // a latency chain — discriminant load, jump-table index, indirect jump
        // — where an extra dependent test is not free even when perfectly
        // predicted. Build with `--features opcode-census` to get it back;
        // `XEZIM_OPCODE_CENSUS=1` then still selects it at run time.
        #[cfg(feature = "opcode-census")]
        let census_on = self.census_enabled;
        let soa_len = self.soa_read_ok.len();
        #[cfg(feature = "opcode-census")]
        let mut census_prev: usize = usize::MAX;
        while pc < len {
            local_count += 1;
            #[cfg(feature = "opcode-census")]
            if census_on {
                const NOP: usize = super::dispatch::NUM_OPCODES;
                if self.census_counts.is_empty() {
                    self.census_counts = vec![0u64; NOP];
                    self.census_pairs = vec![0u64; NOP * NOP];
                }
                let op = super::dispatch::Opcode::from_insn(&insns[pc]) as usize;
                self.census_counts[op] += 1;
                if census_prev < NOP {
                    self.census_pairs[census_prev * NOP + op] += 1;
                }
                census_prev = op;
            }
            match &insns[pc] {
                // Process-FSM wait points (roadmap 11-12): record the resume
                // pc + payload and stop executing; `run_proc_fsm` parks the
                // process. Never present in edge/comb blocks.
                Insn::WaitDelayReg(r) => {
                    let v = self.vm_regs[*r as usize].clone();
                    let t = self.fsm_delay_ticks(&v, self.fsm_precision_diff);
                    self.fsm_suspend = Some((pc as u32 + 1, FsmWait::Delay(t)));
                    break;
                }
                Insn::WaitEdge(ix) => {
                    self.fsm_suspend = Some((pc as u32 + 1, FsmWait::Edge(*ix)));
                    break;
                }
                Insn::CaseLut(d, src, lut) => {
                    let sv = &self.vm_regs[*src as usize];
                    let hit = if sv.has_xz() {
                        None
                    } else {
                        sv.to_u64().and_then(|i| lut.table.get(i as usize))
                    };
                    let out = hit.unwrap_or(&lut.default).clone();
                    self.vm_regs[*d as usize] = out;
                }
                Insn::LoadConst(dest, val) => {
                    // Reuse vm_regs[dest]'s buffer via copy_from — no alloc.
                    self.vm_regs[*dest as usize].copy_from(val.as_ref());
                }
                Insn::LoadSignal(dest, sig_id) => {
                    let d = *dest as usize;
                    let s = *sig_id as usize;
                    // Prefetch the next LoadSignal's signal_table[s'] — for
                    // c910 with 465K signals × 32 B Value = 15 MB
                    // signal_table, the access pattern misses L3 often.
                    // Looking ahead one Insn gives the CPU's prefetcher a
                    // cycle of headstart on the next read.
                    #[cfg(target_arch = "x86_64")]
                    if pc + 1 < len {
                        match &insns[pc + 1] {
                            Insn::LoadSignal(_, s2) | Insn::LoadSignalSigned(_, s2) => {
                                let s2 = &(*s2 as usize);
                                let p = self.signal_table.as_ptr().wrapping_add(*s2);
                                unsafe {
                                    core::arch::x86_64::_mm_prefetch(
                                        p as *const i8,
                                        core::arch::x86_64::_MM_HINT_T0,
                                    );
                                }
                            }
                            _ => {}
                        }
                    }
                    // SoA read fast path (XEZIM_INLINE_BITS=1): rebuild the
                    // Value from the 16-byte hot store instead of touching the
                    // 32-byte signal_table cell. soa_read_ok is empty when the
                    // store is off, so the default path is one bounds check.
                    if s < soa_len && self.soa_read_ok[s] {
                        let [v, x] = self.signal_inline_bits[s];
                        if self.soa_shadow {
                            let (cv, cx) = self.signal_table[s].raw_bits();
                            if v != cv || x != cx {
                                eprintln!(
                                    "[SOA-SHADOW] LoadSignal mismatch sid={} inline=({:#x},{:#x}) table=({:#x},{:#x}) — aborting",
                                    s, v, x, cv, cx
                                );
                                self.abort_run();
                            }
                        }
                        let (w, sg) = (self.signal_widths[s], self.signal_signed[s]);
                        vm_store(&mut self.vm_regs[d], v, x, w, sg);
                    } else {
                        self.vm_regs[d].copy_from(&self.signal_table[s]);
                    }
                }
                Insn::LoadSignalSigned(dest, sig_id) => {
                    let d = *dest as usize;
                    let s = *sig_id as usize;
                    #[cfg(target_arch = "x86_64")]
                    if pc + 1 < len {
                        match &insns[pc + 1] {
                            Insn::LoadSignal(_, s2) | Insn::LoadSignalSigned(_, s2) => {
                                let s2 = &(*s2 as usize);
                                let p = self.signal_table.as_ptr().wrapping_add(*s2);
                                unsafe {
                                    core::arch::x86_64::_mm_prefetch(
                                        p as *const i8,
                                        core::arch::x86_64::_MM_HINT_T0,
                                    );
                                }
                            }
                            _ => {}
                        }
                    }
                    if s < soa_len && self.soa_read_ok[s] {
                        let [v, x] = self.signal_inline_bits[s];
                        if self.soa_shadow {
                            let (cv, cx) = self.signal_table[s].raw_bits();
                            if v != cv || x != cx {
                                eprintln!(
                                    "[SOA-SHADOW] LoadSignalSigned mismatch sid={} inline=({:#x},{:#x}) table=({:#x},{:#x}) — aborting",
                                    s, v, x, cv, cx
                                );
                                self.abort_run();
                            }
                        }
                        self.vm_regs[d] = Value::from_inline(v, x, self.signal_widths[s]);
                    } else {
                        self.vm_regs[d].copy_from(&self.signal_table[s]);
                    }
                    self.vm_regs[d].is_signed = true;
                }
                Insn::LoadProcessLocal(dest, name) => {
                    let value = self
                        .local_stack
                        .last()
                        .and_then(|frame| frame.get(name.as_ref()))
                        .cloned()
                        .unwrap_or_else(|| Value::new(1));
                    self.vm_regs[*dest as usize] = value;
                }
                // class-perf tier-1 opcodes: read/write an object member off
                // the heap, addressed by a register-held handle. `this` is
                // held in the `this_handle` VM register slot that the method-
                // body prologue seeds. Missing object / missing member yield
                // a zero value and a no-op drop write respectively.
                Insn::LoadClassMember(dest, handle, field) => {
                    let hreg = *handle as usize;
                    let h = self.vm_regs[hreg].to_u64().unwrap_or(0) as usize;
                    let value = self
                        .heap
                        .get(h)
                        .and_then(|o| o.as_ref())
                        .and_then(|inst| inst.properties.get(field.as_ref()))
                        .cloned()
                        .unwrap_or_else(|| Value::zero(1));
                    self.vm_regs[*dest as usize] = value;
                }
                Insn::StoreClassMember(handle, value, field) => {
                    let hreg = *handle as usize;
                    let h = self.vm_regs[hreg].to_u64().unwrap_or(0) as usize;
                    // Fit the value to the member's DECLARED width and
                    // signedness exactly as the interpreter's `obj.member = e`
                    // write does (`fit_class_prop`) — otherwise a compiled
                    // store could leave a differently-sized value in the heap
                    // than the AST path, diverging on the next read.
                    let fitted = {
                        let v = &self.vm_regs[*value as usize];
                        self.fit_class_prop(h, field, v)
                    };
                    if let Some(o) = self.heap.get_mut(h).and_then(|o| o.as_mut()) {
                        o.properties.insert(field.to_string(), fitted);
                    }
                }
                Insn::CallMethod(..)
                | Insn::CallScopedMethod(..)
                | Insn::CallFreeFunction(..)
                | Insn::CallStaticScoped(..)
                | Insn::LoadClassStatic(..)
                | Insn::StoreClassStatic(..)
                | Insn::ConstructObject(..)
                | Insn::CallCollMethod(..)
                | Insn::LoadCollElem(..)
                | Insn::StoreCollElem(..)
                | Insn::Cast(..)
                | Insn::ForeachKeys(..)
                | Insn::RetCollection(..)
                | Insn::AssignPendingColl(..) => {
                    local_count += self.exec_class_opcode(&insns[pc]);
                    if self.finished {
                        break;
                    }
                }
                // Advance a compiled foreach: pop the slot's next key into
                // the var register; `ok` = 0 at exhaustion (loop exit).
                Insn::ForeachNext(ok, slot, var) => {
                    let s = *slot as usize;
                    let exhausted = self
                        .foreach_arena
                        .get(s)
                        .map(|(vals, pos)| *pos as usize >= vals.len())
                        .unwrap_or(true);
                    let (o, v) = (*ok as usize, *var as usize);
                    if exhausted {
                        self.vm_regs[o] = Value::zero(32);
                    } else {
                        self.vm_regs[o] = Value::from_u64(1, 32);
                        let p = self.foreach_arena[s].1 as usize;
                        self.vm_regs[v] = self.foreach_arena[s].0[p].clone();
                        self.foreach_arena[s].1 += 1;
                    }
                    local_count += 1;
                }
                // 10.8% of all executed bytecode — the second most frequent
                // opcode. Rewriting the register's two words and its width in
                // place removes the fresh 32-byte `Value`, the 32-byte move
                // and the drop of the old contents that `= r.resize(w)` paid.
                Insn::Resize(reg, width) => {
                    let r = *reg as usize;
                    let w = *width;
                    let vr = &mut self.vm_regs[r];
                    if vr.width != w {
                        let (cw, sg) = (vr.width, vr.is_signed);
                        let bits = if vr.is_real || vr.is_fill {
                            None
                        } else {
                            vr.inline_bits()
                        };
                        match bits.and_then(|(v, x)| vm_resize_bits(v, x, cw, sg, w)) {
                            Some((nv, nx)) => {
                                vr.set_inline_bits(nv, nx);
                                vr.width = w;
                            }
                            None => {
                                let resized = self.vm_regs[r].resize(w);
                                self.vm_regs[r] = resized;
                            }
                        }
                    }
                }
                Insn::Add(d, l, r) => {
                    let (d, l, r) = (*d as usize, *l as usize, *r as usize);
                    match vm_add_sub(&self.vm_regs[l], &self.vm_regs[r], false) {
                        Some((v, x, w, s)) => vm_store(&mut self.vm_regs[d], v, x, w, s),
                        None => self.vm_regs[d] = self.vm_regs[l].add(&self.vm_regs[r]),
                    }
                }
                Insn::Sub(d, l, r) => {
                    let (d, l, r) = (*d as usize, *l as usize, *r as usize);
                    match vm_add_sub(&self.vm_regs[l], &self.vm_regs[r], true) {
                        Some((v, x, w, s)) => vm_store(&mut self.vm_regs[d], v, x, w, s),
                        None => self.vm_regs[d] = self.vm_regs[l].sub(&self.vm_regs[r]),
                    }
                }
                Insn::Mul(d, l, r) => {
                    self.vm_regs[*d as usize] =
                        self.vm_regs[*l as usize].mul(&self.vm_regs[*r as usize]);
                }
                Insn::Div(d, l, r) => {
                    self.vm_regs[*d as usize] =
                        self.vm_regs[*l as usize].div(&self.vm_regs[*r as usize]);
                }
                Insn::Mod(d, l, r) => {
                    self.vm_regs[*d as usize] =
                        self.vm_regs[*l as usize].modulo(&self.vm_regs[*r as usize]);
                }
                Insn::BitAnd(d, l, r) => {
                    let (d, l, r) = (*d as usize, *l as usize, *r as usize);
                    match vm_bitand(&self.vm_regs[l], &self.vm_regs[r]) {
                        Some((v, x, w, s)) => vm_store(&mut self.vm_regs[d], v, x, w, s),
                        None => self.vm_regs[d] = self.vm_regs[l].bitwise_and(&self.vm_regs[r]),
                    }
                }
                Insn::BitOr(d, l, r) => {
                    let (d, l, r) = (*d as usize, *l as usize, *r as usize);
                    match vm_bitor(&self.vm_regs[l], &self.vm_regs[r]) {
                        Some((v, x, w, s)) => vm_store(&mut self.vm_regs[d], v, x, w, s),
                        None => self.vm_regs[d] = self.vm_regs[l].bitwise_or(&self.vm_regs[r]),
                    }
                }
                Insn::BitXor(d, l, r) => {
                    let (d, l, r) = (*d as usize, *l as usize, *r as usize);
                    match vm_bitxor(&self.vm_regs[l], &self.vm_regs[r]) {
                        Some((v, x, w, s)) => vm_store(&mut self.vm_regs[d], v, x, w, s),
                        None => self.vm_regs[d] = self.vm_regs[l].bitwise_xor(&self.vm_regs[r]),
                    }
                }
                Insn::BitXnor(d, l, r) => {
                    let (d, l, r) = (*d as usize, *l as usize, *r as usize);
                    match vm_bitxnor(&self.vm_regs[l], &self.vm_regs[r]) {
                        Some((v, x, w, s)) => vm_store(&mut self.vm_regs[d], v, x, w, s),
                        None => {
                            self.vm_regs[d] =
                                self.vm_regs[l].bitwise_xor(&self.vm_regs[r]).bitwise_not()
                        }
                    }
                }
                Insn::LogAnd(d, l, r) => {
                    let (d, l, r) = (*d as usize, *l as usize, *r as usize);
                    match vm_logic2(&self.vm_regs[l], &self.vm_regs[r], false) {
                        Some((v, x)) => vm_store(&mut self.vm_regs[d], v, x, 1, false),
                        None => self.vm_regs[d] = self.vm_regs[l].logic_and(&self.vm_regs[r]),
                    }
                }
                Insn::LogOr(d, l, r) => {
                    let (d, l, r) = (*d as usize, *l as usize, *r as usize);
                    match vm_logic2(&self.vm_regs[l], &self.vm_regs[r], true) {
                        Some((v, x)) => vm_store(&mut self.vm_regs[d], v, x, 1, false),
                        None => self.vm_regs[d] = self.vm_regs[l].logic_or(&self.vm_regs[r]),
                    }
                }
                Insn::Eq(d, l, r) => {
                    let (d, l, r) = (*d as usize, *l as usize, *r as usize);
                    match vm_is_equal(&self.vm_regs[l], &self.vm_regs[r]) {
                        Some(e) => vm_store(&mut self.vm_regs[d], e as u64, 0, 1, false),
                        None => self.vm_regs[d] = self.vm_regs[l].is_equal(&self.vm_regs[r]),
                    }
                }
                Insn::Neq(d, l, r) => {
                    let (d, l, r) = (*d as usize, *l as usize, *r as usize);
                    match vm_is_equal(&self.vm_regs[l], &self.vm_regs[r]) {
                        Some(e) => vm_store(&mut self.vm_regs[d], !e as u64, 0, 1, false),
                        None => self.vm_regs[d] = self.vm_regs[l].is_not_equal(&self.vm_regs[r]),
                    }
                }
                Insn::CaseEq(d, l, r) => {
                    let (d, l, r) = (*d as usize, *l as usize, *r as usize);
                    match vm_case_eq(&self.vm_regs[l], &self.vm_regs[r]) {
                        Some(e) => vm_store(&mut self.vm_regs[d], e as u64, 0, 1, false),
                        None => self.vm_regs[d] = self.vm_regs[l].case_eq(&self.vm_regs[r]),
                    }
                }
                // Fused `LoadConst` + binop. Character-for-character the
                // `Add`/`Eq`/`CaseEq` arms above with `&self.vm_regs[r]`
                // replaced by the embedded constant, which the elided
                // `LoadConst` would have `copy_from`'d into `vm_regs[r]`
                // verbatim (width/is_signed/is_real/is_fill included) — same
                // `vm_*` fast helper, same `Value` fallback, so the 4-state,
                // signedness and §5.7.1 `is_fill` rules cannot drift.
                Insn::BinOpConstAdd2(a) => {
                    // Each half runs EXACTLY the K::Add path below — same
                    // vm_add_sub fast helper, same Value fallback — so the
                    // superinstruction cannot drift from the pair it merged.
                    match vm_add_sub(&self.vm_regs[a.s1 as usize], &a.k1, false) {
                        Some((v, x, w, sg)) => {
                            vm_store(&mut self.vm_regs[a.d1 as usize], v, x, w, sg)
                        }
                        None => {
                            self.vm_regs[a.d1 as usize] = self.vm_regs[a.s1 as usize].add(&a.k1)
                        }
                    }
                    match vm_add_sub(&self.vm_regs[a.s2 as usize], &a.k2, false) {
                        Some((v, x, w, sg)) => {
                            vm_store(&mut self.vm_regs[a.d2 as usize], v, x, w, sg)
                        }
                        None => {
                            self.vm_regs[a.d2 as usize] = self.vm_regs[a.s2 as usize].add(&a.k2)
                        }
                    }
                }
                Insn::BinOpConst(d, s, k, kind) => {
                    use super::bytecode::BinOpConstKind as K;
                    let (d, s) = (*d as usize, *s as usize);
                    match kind {
                        K::Add => match vm_add_sub(&self.vm_regs[s], &**k, false) {
                            Some((v, x, w, sg)) => vm_store(&mut self.vm_regs[d], v, x, w, sg),
                            None => self.vm_regs[d] = self.vm_regs[s].add(&**k),
                        },
                        K::Eq => match vm_is_equal(&self.vm_regs[s], &**k) {
                            Some(e) => vm_store(&mut self.vm_regs[d], e as u64, 0, 1, false),
                            None => self.vm_regs[d] = self.vm_regs[s].is_equal(&**k),
                        },
                        K::CaseEq => match vm_case_eq(&self.vm_regs[s], &**k) {
                            Some(e) => vm_store(&mut self.vm_regs[d], e as u64, 0, 1, false),
                            None => self.vm_regs[d] = self.vm_regs[s].case_eq(&**k),
                        },
                        K::Xor => self.vm_regs[d] = self.vm_regs[s].bitwise_xor(&**k),
                        K::And => self.vm_regs[d] = self.vm_regs[s].bitwise_and(&**k),
                        K::Or => self.vm_regs[d] = self.vm_regs[s].bitwise_or(&**k),
                        K::Mul => self.vm_regs[d] = self.vm_regs[s].mul(&**k),
                        K::Sub => self.vm_regs[d] = self.vm_regs[s].sub(&**k),
                    }
                }
                Insn::CasezEq(d, l, r) => {
                    self.vm_regs[*d as usize] =
                        self.vm_regs[*l as usize].casez_eq(&self.vm_regs[*r as usize]);
                }
                Insn::CasexEq(d, l, r) => {
                    self.vm_regs[*d as usize] =
                        self.vm_regs[*l as usize].casex_eq(&self.vm_regs[*r as usize]);
                }
                Insn::Lt(d, l, r) => {
                    let (d, l, r) = (*d as usize, *l as usize, *r as usize);
                    match vm_less(&self.vm_regs[l], &self.vm_regs[r], false) {
                        Some((v, x)) => vm_store(&mut self.vm_regs[d], v, x, 1, false),
                        None => self.vm_regs[d] = self.vm_regs[l].less_than(&self.vm_regs[r]),
                    }
                }
                Insn::Leq(d, l, r) => {
                    let (d, l, r) = (*d as usize, *l as usize, *r as usize);
                    match vm_less(&self.vm_regs[l], &self.vm_regs[r], true) {
                        Some((v, x)) => vm_store(&mut self.vm_regs[d], v, x, 1, false),
                        None => self.vm_regs[d] = self.vm_regs[l].less_equal(&self.vm_regs[r]),
                    }
                }
                // `greater_than`/`greater_equal` are `other.less_*(self)` —
                // the operands swap, nothing else.
                Insn::Gt(d, l, r) => {
                    let (d, l, r) = (*d as usize, *l as usize, *r as usize);
                    match vm_less(&self.vm_regs[r], &self.vm_regs[l], false) {
                        Some((v, x)) => vm_store(&mut self.vm_regs[d], v, x, 1, false),
                        None => self.vm_regs[d] = self.vm_regs[l].greater_than(&self.vm_regs[r]),
                    }
                }
                Insn::Geq(d, l, r) => {
                    let (d, l, r) = (*d as usize, *l as usize, *r as usize);
                    match vm_less(&self.vm_regs[r], &self.vm_regs[l], true) {
                        Some((v, x)) => vm_store(&mut self.vm_regs[d], v, x, 1, false),
                        None => self.vm_regs[d] = self.vm_regs[l].greater_equal(&self.vm_regs[r]),
                    }
                }
                Insn::Shl(d, l, r) => {
                    self.vm_regs[*d as usize] =
                        self.vm_regs[*l as usize].shift_left(&self.vm_regs[*r as usize]);
                }
                Insn::Shr(d, l, r) => {
                    self.vm_regs[*d as usize] =
                        self.vm_regs[*l as usize].shift_right(&self.vm_regs[*r as usize]);
                }
                Insn::AShr(d, l, r) => {
                    self.vm_regs[*d as usize] =
                        self.vm_regs[*l as usize].arith_shift_right(&self.vm_regs[*r as usize]);
                }
                Insn::BitNot(d, s) => {
                    let (d, s) = (*d as usize, *s as usize);
                    match vm_bitnot(&self.vm_regs[s]) {
                        Some((v, x, w, sg)) => vm_store(&mut self.vm_regs[d], v, x, w, sg),
                        None => self.vm_regs[d] = self.vm_regs[s].bitwise_not(),
                    }
                }
                Insn::LogNot(d, s) => {
                    let (d, s) = (*d as usize, *s as usize);
                    match vm_logic_not(&self.vm_regs[s]) {
                        Some((v, x)) => vm_store(&mut self.vm_regs[d], v, x, 1, false),
                        None => self.vm_regs[d] = self.vm_regs[s].logic_not(),
                    }
                }
                Insn::Negate(d, s) => {
                    let w = self.vm_regs[*s as usize].width;
                    let mut r = Value::zero(w).sub(&self.vm_regs[*s as usize]).resize(w);
                    r.is_signed = self.vm_regs[*s as usize].is_signed;
                    self.vm_regs[*d as usize] = r;
                }
                Insn::ReduceAnd(d, s) => {
                    self.vm_regs[*d as usize] = self.vm_regs[*s as usize].reduce_and();
                }
                Insn::ReduceOr(d, s) => {
                    self.vm_regs[*d as usize] = self.vm_regs[*s as usize].reduce_or();
                }
                Insn::ReduceXor(d, s) => {
                    self.vm_regs[*d as usize] = self.vm_regs[*s as usize].reduce_xor();
                }
                Insn::BitSelect(d, base, idx) => {
                    let (d, base) = (*d as usize, *base as usize);
                    // §11.5.1: an x/z or negative index reads x.
                    let Some(i) = self.vm_regs[*idx as usize].to_index().filter(|&i| i >= 0) else {
                        self.vm_regs[d] = Value::new(1);
                        pc += 1;
                        continue;
                    };
                    let i = i as usize;
                    match vm_bit_select(&self.vm_regs[base], i) {
                        Some((v, x)) => vm_store(&mut self.vm_regs[d], v, x, 1, false),
                        None => self.vm_regs[d] = self.vm_regs[base].bit_select(i),
                    }
                }
                Insn::BitSelectConst(d, base, idx) => {
                    let (d, base, i) = (*d as usize, *base as usize, *idx as usize);
                    match vm_bit_select(&self.vm_regs[base], i) {
                        Some((v, x)) => vm_store(&mut self.vm_regs[d], v, x, 1, false),
                        None => self.vm_regs[d] = self.vm_regs[base].bit_select(i),
                    }
                }
                Insn::RangeSelectW(d, base, idx, w, up) => {
                    // §11.5.1: an x/z index reads a whole-x select of the
                    // declared width; otherwise the bounds are formed here.
                    let dv = match self.vm_regs[*idx as usize].to_index() {
                        None => Value::new(*w),
                        Some(i) => {
                            let i = i as i32 as i64;
                            let (lo, hi) = if *up {
                                (i, i + *w as i64 - 1)
                            } else {
                                (i - *w as i64 + 1, i)
                            };
                            oob_range_select(&self.vm_regs[*base as usize], hi, lo)
                        }
                    };
                    self.vm_regs[*d as usize] = dv;
                }
                Insn::RangeSelect(d, base, l, r) => {
                    // §11.5.1 signed low-bound recovery — see the twin site.
                    // §11.5.1: an x/z bound selects nothing (see the twin site).
                    let (av, bv) = (&self.vm_regs[*l as usize], &self.vm_regs[*r as usize]);
                    let (Some(a), Some(b)) = (av.to_index(), bv.to_index()) else {
                        let a = av.to_u64().unwrap_or(0) as u32 as i32 as i64;
                        let b = bv.to_u64().unwrap_or(0) as u32 as i32 as i64;
                        self.vm_regs[*d as usize] = Value::new(((a - b).unsigned_abs() + 1) as u32);
                        pc += 1;
                        continue;
                    };
                    let (a, b) = (a as i32 as i64, b as i32 as i64);
                    let (lo, hi) = (a.min(b), a.max(b));
                    self.vm_regs[*d as usize] =
                        oob_range_select(&self.vm_regs[*base as usize], hi, lo);
                }
                Insn::RangeSelectConst(d, base, l, r) => {
                    let (d, base, l, r) = (*d as usize, *base as usize, *l as usize, *r as usize);
                    if l.max(r) >= self.vm_regs[base].width as usize {
                        // Partially out of range: §11.5.1 whole-x (see the helper).
                        self.vm_regs[d] =
                            oob_range_select(&self.vm_regs[base], l.max(r) as i64, l.min(r) as i64);
                    } else {
                        match vm_range_select(&self.vm_regs[base], l, r) {
                            Some((v, x, w)) => vm_store(&mut self.vm_regs[d], v, x, w, false),
                            None => self.vm_regs[d] = self.vm_regs[base].range_select(l, r),
                        }
                    }
                }
                // Fused load+select (finish() peephole): slice straight out of
                // the signal — no whole-Value copy into a register first.
                Insn::LoadSignalRangeDyn(d, sig_id, lo_reg, w) => {
                    // §11.5.1: an x/z offset selects nothing — whole-x at the
                    // slice's width. Otherwise read the slice in place.
                    let lo_v = &self.vm_regs[*lo_reg as usize];
                    let out = match lo_v.to_index() {
                        Some(lo) => {
                            let lo = lo as i32 as i64;
                            oob_range_select(
                                &self.signal_table[*sig_id as usize],
                                lo + *w as i64 - 1,
                                lo,
                            )
                        }
                        None => Value::new(*w),
                    };
                    self.vm_regs[*d as usize] = out;
                }
                Insn::LoadSignalRange(d, sig_id, l, r) => {
                    let sig_id = &(*sig_id as usize);
                    let (d, l, r) = (*d as usize, *l as usize, *r as usize);
                    if l.max(r) >= self.signal_table[*sig_id].width as usize {
                        // §11.5.1: partially out of range (see `oob_range_select`).
                        self.vm_regs[d] = oob_range_select(
                            &self.signal_table[*sig_id],
                            l.max(r) as i64,
                            l.min(r) as i64,
                        );
                    } else {
                        match vm_range_select(&self.signal_table[*sig_id], l, r) {
                            Some((v, x, w)) => vm_store(&mut self.vm_regs[d], v, x, w, false),
                            None => self.vm_regs[d] = self.signal_table[*sig_id].range_select(l, r),
                        }
                    }
                }
                Insn::LoadSignalBit(d, sig_id, idx) => {
                    let sig_id = &(*sig_id as usize);
                    let (d, idx) = (*d as usize, *idx as usize);
                    match vm_bit_select(&self.signal_table[*sig_id], idx) {
                        Some((v, x)) => vm_store(&mut self.vm_regs[d], v, x, 1, false),
                        None => self.vm_regs[d] = self.signal_table[*sig_id].bit_select(idx),
                    }
                }
                Insn::Concat(d, part_regs) => {
                    // `concat_refs` is a cross-crate call that walks the
                    // iterator twice (once to sum widths, once to pack) and
                    // returns a fresh `Value`. For a result that fits inline
                    // — the common RTL case — do the pack here and write the
                    // destination's two words in place. Bit-for-bit the same
                    // fold as `concat_refs`' `total_width <= 64` arm.
                    let d = *d as usize;
                    let mut total: u32 = 0;
                    for p in part_regs.iter() {
                        total += self.vm_regs[*p as usize].width;
                    }
                    if total <= 64 {
                        let (mut out_v, mut out_x, mut offset) = (0u64, 0u64, 0u32);
                        for p in part_regs.iter().rev() {
                            let val = &self.vm_regs[*p as usize];
                            if val.width == 0 {
                                continue;
                            }
                            let (v, x) = val.raw_bits();
                            let mask = vm_mask(val.width);
                            out_v |= (v & mask) << offset;
                            out_x |= (x & mask) << offset;
                            offset += val.width;
                        }
                        vm_store(&mut self.vm_regs[d], out_v, out_x, total, false);
                    } else if total <= 128 {
                        // Two 128-bit accumulators, written into the
                        // destination register IN PLACE: the register keeps
                        // its two-word allocation across evaluations instead
                        // of a fresh three-allocation `Value` per concat.
                        let (mut out_v, mut out_x, mut offset) = (0u128, 0u128, 0u32);
                        for p in part_regs.iter().rev() {
                            let val = &self.vm_regs[*p as usize];
                            if val.width == 0 {
                                continue;
                            }
                            let (v, x) = val.bits128();
                            out_v |= v << offset;
                            out_x |= x << offset;
                            offset += val.width;
                        }
                        self.vm_regs[d].assign_wide128(out_v, out_x, total);
                    } else {
                        let result = Value::concat_refs(
                            part_regs.iter().map(|r| &self.vm_regs[*r as usize]),
                        );
                        self.vm_regs[d] = result;
                    }
                }
                Insn::Replicate(d, s, n) => {
                    let (d, s, n) = (*d as usize, *s as usize, *n as usize);
                    if n == 0 {
                        vm_store(&mut self.vm_regs[d], 0, 0, 0, false); // Value::zero(0)
                    } else if n == 1 {
                        if d != s {
                            let v = self.vm_regs[s].clone();
                            self.vm_regs[d] = v;
                        }
                    } else {
                        // `{n{x}}` used to build a `Vec<Value>` of `n` clones
                        // (a heap allocation each, for `Wide`) and hand it to
                        // `Value::concat`. `concat_refs` — a cross-crate call
                        // that walks its iterator TWICE, once to sum the
                        // widths — was still 1.8% of the whole run.
                        // Every part here is the SAME value, so the width sum
                        // is one multiply and the fold needs no iterator at
                        // all. Bit-for-bit the `total_width <= 64` arm of
                        // `concat_refs`; wider results still go there.
                        match vm_replicate(&self.vm_regs[s], n) {
                            Some((v, x, total)) => {
                                vm_store(&mut self.vm_regs[d], v, x, total, false)
                            }
                            None => {
                                let src = &self.vm_regs[s];
                                if src.width == 1 && !src.is_fill && !src.is_real {
                                    // `{N{bit}}` over a wide bus: a word fill
                                    // written into the register in place.
                                    let (v, x) = src.raw_bits();
                                    let total = n as u32;
                                    self.vm_regs[d].assign_replicated_bit(v, x, total);
                                } else {
                                    let result = src.replicate(n);
                                    self.vm_regs[d] = result;
                                }
                            }
                        }
                    }
                }
                Insn::BranchIfFalse(reg, target) => {
                    if !self.vm_regs[*reg as usize].is_true() {
                        pc = *target as usize;
                        continue;
                    }
                }
                Insn::MoveResize(d, s, w) => {
                    // Exact composition of Move+Resize (same-width runtime
                    // skip preserves `is_fill` verbatim).
                    let sv = &self.vm_regs[*s as usize];
                    let nv = if sv.width != *w {
                        sv.resize(*w)
                    } else {
                        sv.clone()
                    };
                    self.vm_regs[*d as usize] = nv;
                }
                Insn::CmpBranchConst(kind, src, k, _tmp, target) => {
                    use super::bytecode::BinOpConstKind as K;
                    let a = &self.vm_regs[*src as usize];
                    let res = match kind {
                        K::Eq => a.is_equal(k),
                        K::CaseEq => a.case_eq(k),
                        _ => unreachable!("constant branch requires an equality comparison"),
                    };
                    if !res.is_true() {
                        pc = *target as usize;
                        continue;
                    }
                }
                Insn::CmpBranch(kind, l, r, _tmp, target) => {
                    use super::bytecode::CmpKind as CK;
                    let a = &self.vm_regs[*l as usize];
                    let b = &self.vm_regs[*r as usize];
                    let res = match kind {
                        CK::Eq => a.is_equal(b),
                        CK::Neq => a.is_not_equal(b),
                        CK::CaseEq => a.case_eq(b),
                        CK::Lt => a.less_than(b),
                        CK::Leq => a.less_equal(b),
                        CK::Gt => a.greater_than(b),
                        CK::Geq => a.greater_equal(b),
                    };
                    if !res.is_true() {
                        pc = *target as usize;
                        continue;
                    }
                }
                Insn::NbaAssignConst(sig_id, k, _width) => {
                    let sig_id = &(*sig_id as usize);
                    // Const pre-resized at fuse time: compare, clone only on change.
                    if let Some(i) = self.nba_fast_index.get(*sig_id) {
                        // §10.4.2 last-write-wins. A value already QUEUED for
                        // this signal in this time step is what it will become,
                        // so the elision below must not compare against
                        // signal_table — that is the stale pre-update value.
                        // Eliding here let an earlier NBA survive a later one
                        // that happened to match the old value, which is how
                        // `x <= a; if (rst) {x,y} <= 0;` kept `a` under reset.
                        self.nba_fast[i].value = (**k).clone();
                    } else if self.signal_table[*sig_id] != **k {
                        self.nba_fast_index.insert(*sig_id, self.nba_fast.len());
                        self.nba_fast.push(NbaFast {
                            block_index: 0,
                            signal_id: *sig_id,
                            value: (**k).clone(),
                        });
                    } else {
                        self.prof_nba_elided += 1;
                    }
                }
                Insn::BranchUnlessZero(reg, target) => {
                    // Fused LogNot+BranchIfFalse: jump unless DEFINITE zero —
                    // exact composition of logic_not with !is_true (X branches).
                    if self.vm_regs[*reg as usize].is_nonzero() != Some(false) {
                        // do-while back-edge: see `Insn::Jump`.
                        if (*target as usize) <= pc && loop_stop_requested() {
                            self.take_interrupt();
                            break;
                        }
                        pc = *target as usize;
                        continue;
                    }
                }
                Insn::BranchIfSignalFalse(sig_id, target, bit) => {
                    let sig_id = &(*sig_id as usize);
                    let sv = &self.signal_table[*sig_id];
                    let cond_true = if *bit == u32::MAX {
                        sv.is_true()
                    } else {
                        // `bit_select(..).is_true()` on a 1-bit inline result is
                        // exactly "that bit is a DEFINITE 1" (`is_nonzero()`
                        // returns Some(true) only for val=1,xz=0, and both
                        // Some(false) and None end up false through
                        // `unwrap_or(false)`). No Value is built.
                        match vm_bit_select(sv, *bit as usize) {
                            Some((v, x)) => v & !x != 0,
                            None => sv.bit_select(*bit as usize).is_true(),
                        }
                    };
                    if !cond_true {
                        pc = *target as usize;
                        continue;
                    }
                }
                Insn::Select(dest, cond, then_r, else_r) => {
                    // §11.4.11 + §5.7.1: a fill branch ('z/'1/...) takes the
                    // width of the other branch (tristate lowering builds
                    // `cond ? then : 'z`; a 1-bit z leaking through would
                    // zero-extend at the assign instead of z-filling).
                    {
                        let (tw, tf) = (
                            self.vm_regs[*then_r as usize].width,
                            self.vm_regs[*then_r as usize].is_fill,
                        );
                        let (ew, ef) = (
                            self.vm_regs[*else_r as usize].width,
                            self.vm_regs[*else_r as usize].is_fill,
                        );
                        if tf && !ef {
                            self.vm_regs[*then_r as usize] =
                                self.vm_regs[*then_r as usize].resize(ew);
                        } else if ef && !tf {
                            self.vm_regs[*else_r as usize] =
                                self.vm_regs[*else_r as usize].resize(tw);
                        }
                    }
                    let v = if self.vm_regs[*cond as usize].has_unknown() {
                        let t = self.vm_regs[*then_r as usize].clone();
                        let e = self.vm_regs[*else_r as usize].clone();
                        // §11.3.1: a real result can't hold X — bit-merging the
                        // branches would read their IEEE-754 bits as garbage.
                        // Return a defined real value (else branch) when either
                        // branch is real.
                        if t.is_real || e.is_real {
                            e
                        } else {
                            t.merge_unknown(&e)
                        }
                    } else if self.vm_regs[*cond as usize].is_true() {
                        self.vm_regs[*then_r as usize].clone()
                    } else {
                        self.vm_regs[*else_r as usize].clone()
                    };
                    self.vm_regs[*dest as usize] = v;
                }
                Insn::Jump(target) => {
                    // A backward jump closes a loop iteration: poll Ctrl-C
                    // here, or a long loop inside one time slot never
                    // reaches the event loop's slot-boundary poll.
                    if (*target as usize) <= pc && loop_stop_requested() {
                        self.take_interrupt();
                        break;
                    }
                    pc = *target as usize;
                    continue;
                }
                Insn::CaseJump(src, cj) => {
                    // Compile gate guarantees the selector is unsigned and
                    // <=64 bits, so raw numeric dispatch matches the ===
                    // chain it replaces; any x/z bit matches no constant
                    // pattern -> default (table holes also hold default).
                    let s = &self.vm_regs[*src as usize];
                    let t = if s.has_xz() {
                        cj.default
                    } else {
                        match s.to_u64() {
                            Some(i) => cj.table.get(i as usize).copied().unwrap_or(cj.default),
                            None => cj.default,
                        }
                    };
                    pc = t as usize;
                    continue;
                }
                Insn::CaseMaskJump(src, mj) => {
                    // Dispatch on the always-defined window; any x/z there
                    // (or non-inline storage) walks the full chain instead.
                    let s = &self.vm_regs[*src as usize];
                    let t = match s.inline_bits() {
                        Some((v, xz)) if !s.is_fill => {
                            let m = if s.width >= 64 {
                                u64::MAX
                            } else {
                                (1u64 << s.width) - 1
                            };
                            let wmask = (1u64 << mj.width) - 1;
                            if ((xz & m) >> mj.lo) & wmask == 0 {
                                mj.table[(((v & m) >> mj.lo) & wmask) as usize]
                            } else {
                                mj.xz_path
                            }
                        }
                        _ => mj.xz_path,
                    };
                    pc = t as usize;
                    continue;
                }
                Insn::Format(dst, f) => {
                    self.vm_regs[*dst as usize] = Self::exec_format(f, &self.vm_regs);
                }
                Insn::StrOp(dst, kind, args) => {
                    self.vm_regs[*dst as usize] = Self::exec_str_op(*kind, args, &self.vm_regs);
                }
                Insn::NbaAssign(sig_id, val_reg, width) => {
                    let sig_id = &(*sig_id as usize);
                    let val = self.vm_regs[*val_reg as usize].resize_for_assign(*width);
                    // Eval-time elision: skip the queue push when the
                    // value already matches signal_table.  See the
                    // exec_insns_isolated sibling.  Bumps prof_nba_elided.
                    //
                    // §10.4.2 last-write-wins: an entry already QUEUED for this
                    // signal is what it will become, so the elision must not
                    // compare against the stale signal_table value — overwrite
                    // the queued entry instead.
                    if let Some(i) = self.nba_fast_index.get(*sig_id) {
                        self.nba_fast[i].value = val;
                    } else if self.signal_table[*sig_id] != val {
                        // Update nba_fast_index too so a follow-up partial-range
                        // or bit NBA to the same signal merges into THIS new
                        // whole-value entry, not into a stale earlier partial.
                        self.nba_fast_index.insert(*sig_id, self.nba_fast.len());
                        self.nba_fast.push(NbaFast {
                            block_index: 0,
                            signal_id: *sig_id,
                            value: val,
                        });
                    } else {
                        self.prof_nba_elided += 1;
                    }
                }
                // Fused `LoadSignal ; LoadArrayElem ; NbaAssign` — an RTL
                // memory read feeding a flop, and 3.7% of the C906 memcpy
                // instruction stream. Exactly the composition of those three
                // arms with the two intermediate VM registers elided: the
                // index is read from signal_table (what `LoadSignal` copied
                // into a register), the element value is resized for assign
                // straight from signal_table (what `LoadArrayElem` copied),
                // and the queue/elision logic is the `NbaAssign` arm verbatim.
                // The unresolved-element fallback reproduces `LoadArrayElem`'s
                // 1-bit X. The SoA (`XEZIM_INLINE_BITS`) read shortcut is
                // deliberately not replicated — `soa_read_ok` only ever holds
                // for signals whose inline bits are asserted equal to the
                // canonical table cell, so reading the table is the same value.
                Insn::NbaAssignArrayRead(sig_id, array_name, idx_sig, width) => {
                    // §11.5.1: an x/z index reads an all-x element.
                    let Some(idx) = self.signal_table[*idx_sig as usize].to_index() else {
                        let val = Value::new((*width).max(1));
                        let sig_id = *sig_id as usize;
                        if let Some(i) = self.nba_fast_index.get(sig_id) {
                            self.nba_fast[i].value = val;
                        } else if self.signal_table[sig_id] != val {
                            self.nba_fast_index.insert(sig_id, self.nba_fast.len());
                            self.nba_fast.push(NbaFast {
                                block_index: 0,
                                signal_id: sig_id,
                                value: val,
                            });
                        }
                        pc += 1;
                        continue;
                    };
                    let val = match resolve_bytecode_array_elem(
                        array_name,
                        idx,
                        &self.array_first_id,
                        &self.signal_name_to_id,
                    ) {
                        Some(eid) if is_packed_id(eid) => {
                            let (v, x, _) = self.packed.raw(eid);
                            Value::from_inline(v, x, *width)
                        }
                        Some(eid) => self.signal_table[eid].resize_for_assign(*width),
                        None => Value::new((*width).max(1)),
                    };
                    let sig_id = *sig_id as usize;
                    if let Some(i) = self.nba_fast_index.get(sig_id) {
                        self.nba_fast[i].value = val;
                    } else if self.signal_table[sig_id] != val {
                        self.nba_fast_index.insert(sig_id, self.nba_fast.len());
                        self.nba_fast.push(NbaFast {
                            block_index: 0,
                            signal_id: sig_id,
                            value: val,
                        });
                    } else {
                        self.prof_nba_elided += 1;
                    }
                }
                Insn::NbaAssignRange(sig_id, hi, lo, val_reg) => {
                    let sig_id = &(*sig_id as usize);
                    // O(1) lookup via nba_fast_index instead of the prior
                    // O(N) `iter().rposition` scan. Mutate the existing
                    // entry in-place (no clone) when we find one — falls
                    // back to seeding from `signal_table[id].clone()` only
                    // for the first NBA to a given signal in this window.
                    //
                    // Eval-time elision: when there's no existing queue
                    // entry AND the merged result matches signal_table[id],
                    // skip the push (apply_nba would drop it anyway).
                    let (low, high) = if hi >= lo { (*lo, *hi) } else { (*hi, *lo) };
                    if oob_write_discard(high as i64, low as i64, self.signal_widths[*sig_id]) {
                        pc += 1;
                        continue;
                    }
                    // §11.5.1: labels above the MSB are outside the vector;
                    // only the in-range window is composed (the blocking arm
                    // clamps the same way — unclamped, `x[4:-1] <= v` put a
                    // fifth bit into a 4-bit signal).
                    let high = high.min(self.signal_widths[*sig_id].saturating_sub(1));
                    if low > high {
                        pc += 1;
                        continue;
                    }
                    let w = high - low + 1;
                    let val = self.vm_regs[*val_reg as usize].resize(w);
                    let id = *sig_id;
                    if self.signal_widths[id] <= 64 && !self.signal_real[id] {
                        let (src_v, src_x) = val.raw_bits();
                        if let Some(i) = self.nba_fast_index.get(id) {
                            let target = &mut self.nba_fast[i].value;
                            let (base_v, base_x) = target.raw_bits();
                            let (new_v, new_x) = Self::compose_inline_range_bits(
                                base_v, base_x, src_v, src_x, low, high,
                            );
                            target.set_inline_bits(new_v, new_x);
                            target.is_signed = self.signal_signed[id];
                        } else {
                            let (base_v, base_x) = self.signal_table[id].raw_bits();
                            let (new_v, new_x) = Self::compose_inline_range_bits(
                                base_v, base_x, src_v, src_x, low, high,
                            );
                            if new_v == base_v && new_x == base_x {
                                // No change — elide.
                                self.prof_nba_elided += 1;
                            } else {
                                let mut new_val =
                                    Value::from_inline(new_v, new_x, self.signal_widths[id]);
                                new_val.is_signed = self.signal_signed[id];
                                self.nba_fast_index.insert(id, self.nba_fast.len());
                                self.nba_fast.push(NbaFast {
                                    block_index: 0,
                                    signal_id: id,
                                    value: new_val,
                                });
                            }
                        }
                    } else if let Some(i) = self.nba_fast_index.get(id) {
                        let target = &mut self.nba_fast[i].value;
                        target.copy_bits_from(low as usize, &val, 0, w as usize);
                    } else {
                        let mut new_val = self.signal_table[id].clone();
                        if new_val.copy_bits_from(low as usize, &val, 0, w as usize) {
                            self.nba_fast_index.insert(id, self.nba_fast.len());
                            self.nba_fast.push(NbaFast {
                                block_index: 0,
                                signal_id: id,
                                value: new_val,
                            });
                        } else {
                            self.prof_nba_elided += 1;
                        }
                    }
                }
                Insn::NbaAssignRangeDyn(sig_id, hi_reg, lo_reg, val_reg) => {
                    let sig_id = &(*sig_id as usize);
                    // §11.5.1: an x/z bound discards the write.
                    let (Some(hi), Some(lo)) = (
                        self.vm_regs[*hi_reg as usize].to_index(),
                        self.vm_regs[*lo_reg as usize].to_index(),
                    ) else {
                        pc += 1;
                        continue;
                    };
                    // §11.5.1 with a negative base (`x[i +: 6]`, i = -1):
                    // labels below bit 0 are outside the vector, so the
                    // value's low bits are dropped and only the in-range
                    // window is written. Converting the bounds to u32 first
                    // wrapped -1 to 4294967295 and dropped the whole write.
                    let (low_i, high_i) = (hi.min(lo), hi.max(lo));
                    if oob_write_discard(high_i, low_i, self.signal_widths[*sig_id]) || high_i < 0 {
                        pc += 1;
                        continue;
                    }
                    let below = (-low_i).max(0) as u32;
                    let (low, high) = (low_i.max(0) as u32, high_i.min(u32::MAX as i64) as u32);
                    let w = high - low + 1;
                    let val = if below > 0 {
                        self.vm_regs[*val_reg as usize]
                            .shift_right(&Value::from_u64(below as u64, 32))
                            .resize(w)
                    } else {
                        self.vm_regs[*val_reg as usize].resize(w)
                    };
                    let id = *sig_id;
                    let sig_w = self.signal_widths[id];
                    let high_eff = high.min(sig_w.saturating_sub(1));
                    let copy_count = high_eff.saturating_add(1).saturating_sub(low) as usize;

                    if low == 0 && high_eff + 1 >= sig_w {
                        // Whole-signal write — same elision as the simple
                        // Insn::NbaAssign.
                        let mut v = val.resize_for_assign(sig_w);
                        v.is_signed = self.signal_signed[id];
                        if self.signal_table[id] != v {
                            self.nba_fast_index.insert(id, self.nba_fast.len());
                            self.nba_fast.push(NbaFast {
                                block_index: 0,
                                signal_id: id,
                                value: v,
                            });
                        } else {
                            self.prof_nba_elided += 1;
                        }
                    } else if self.signal_widths[id] <= 64 && !self.signal_real[id] {
                        let (src_v, src_x) = val.raw_bits();
                        if let Some(i) = self.nba_fast_index.get(id) {
                            let target = &mut self.nba_fast[i].value;
                            let (base_v, base_x) = target.raw_bits();
                            let (new_v, new_x) = Self::compose_inline_range_bits(
                                base_v, base_x, src_v, src_x, low, high_eff,
                            );
                            target.set_inline_bits(new_v, new_x);
                            target.is_signed = self.signal_signed[id];
                        } else {
                            let (base_v, base_x) = self.signal_table[id].raw_bits();
                            let (new_v, new_x) = Self::compose_inline_range_bits(
                                base_v, base_x, src_v, src_x, low, high_eff,
                            );
                            if new_v == base_v && new_x == base_x {
                                self.prof_nba_elided += 1;
                            } else {
                                let mut new_val = Value::from_inline(new_v, new_x, sig_w);
                                new_val.is_signed = self.signal_signed[id];
                                self.nba_fast_index.insert(id, self.nba_fast.len());
                                self.nba_fast.push(NbaFast {
                                    block_index: 0,
                                    signal_id: id,
                                    value: new_val,
                                });
                            }
                        }
                    } else {
                        if let Some(i) = self.nba_fast_index.get(id) {
                            let target = &mut self.nba_fast[i].value;
                            target.copy_bits_from(low as usize, &val, 0, copy_count);
                        } else {
                            let mut new_val = self.signal_table[id].clone();
                            if new_val.copy_bits_from(low as usize, &val, 0, copy_count) {
                                self.nba_fast_index.insert(id, self.nba_fast.len());
                                self.nba_fast.push(NbaFast {
                                    block_index: 0,
                                    signal_id: id,
                                    value: new_val,
                                });
                            } else {
                                self.prof_nba_elided += 1;
                            }
                        }
                    }
                }
                Insn::NbaAssignBitDyn(sig_id, idx_reg, val_reg) => {
                    let sig_id = &(*sig_id as usize);
                    // §11.5.1: an x/z index modifies no bits.
                    if let Some(idx) = dyn_bit_index(&self.vm_regs[*idx_reg as usize]) {
                        let bit = self.vm_regs[*val_reg as usize].get_bit(0);
                        let id = *sig_id;
                        if let Some(i) = self.nba_fast_index.get(id) {
                            self.nba_fast[i].value.set_bit(idx, bit);
                        } else {
                            // Single-bit elision: if the bit is already what
                            // we're about to write, skip the queue push.
                            if self.signal_table[id].get_bit(idx) == bit {
                                self.prof_nba_elided += 1;
                            } else {
                                let mut new_val = self.signal_table[id].clone();
                                new_val.set_bit(idx, bit);
                                self.nba_fast_index.insert(id, self.nba_fast.len());
                                self.nba_fast.push(NbaFast {
                                    block_index: 0,
                                    signal_id: id,
                                    value: new_val,
                                });
                            }
                        }
                    }
                }
                Insn::EvalExprFallback(payload, r, ctxw) => {
                    // Expression-level AST escape hatch: interpret one
                    // sub-expression; the rest of the statement stays
                    // compiled. Counted in the fallback_reason table so the
                    // remaining interpreter surface stays visible.
                    let e = payload.0.clone();
                    let reason = payload.1.clone();
                    self.prof_fallback_insns += 1;
                    let (r, ctxw) = (*r as usize, *ctxw);
                    let v = if payload.2.is_empty() {
                        self.eval_expr_ctx(&e, ctxw)
                    } else {
                        self.fb_eval_expr_with_locals(&payload.2, &e, ctxw)
                    };
                    self.vm_regs[r] = v;
                    let ent = self
                        .prof_fallback_by_reason
                        .entry(reason)
                        .or_insert((0u64, 0u64));
                    ent.0 += 1;
                    if self.finished {
                        break;
                    }
                }
                Insn::StmtFallback(payload) => {
                    let s = payload.0.clone();
                    self.prof_fallback_insns += 1;
                    let r = payload.1.clone();
                    let t0 = std::time::Instant::now();
                    if payload.2.is_empty() {
                        self.exec_statement(&s);
                    } else {
                        self.fb_exec_stmt_with_locals(&payload.2, &s);
                    }
                    let elapsed = t0.elapsed().as_nanos() as u64;
                    let e = self
                        .prof_fallback_by_reason
                        .entry(r)
                        .or_insert((0u64, 0u64));
                    e.0 += 1;
                    e.1 += elapsed;
                    // §20.2: an AST system task can terminate this stream.
                    if self.finished {
                        break;
                    }
                }
                Insn::InitLocalColl(payload) => {
                    // class-perf: a function-local QUEUE/unsized-dynarray decl
                    // with no init. Register empty storage through the exact
                    // interpreter path (`exec_statement`→`exec_stmt_var_decl`),
                    // byte-identical to the old `VarDecl_local_coll` fallback,
                    // but under a DEDICATED opcode so the enclosing block is
                    // NOT pessimized by `has_forbidden`.
                    let s = payload.0.clone();
                    self.exec_statement(&s);
                }
                Insn::BlockingAssign(sig_id, val_reg, width) => {
                    let sig_id = &(*sig_id as usize);
                    let id = *sig_id;
                    // §10.6.1/§10.6.2: a `force`d or procedurally-continuously-
                    // `assign`ed target ignores ordinary procedural writes until
                    // release/deassign. The slow path's `write_sig!` already
                    // honors this, but the raw-bit fast path below writes
                    // `signal_table` directly — so an edge/procedural block's
                    // blocking assign overrode a force. Treat a forced target as
                    // already handled (drop the write on BOTH paths).
                    let mut handled =
                        !self.forced_signals.is_empty() && self.forced_signals.contains_key(&id);
                    // 2-state targets take the slow path so X/Z is dropped
                    // (§6.11.1/§10.7); the raw-bit fast path would keep it.
                    if !handled
                        // Step 9e: width 0 (a class-handle signal — the plan
                        // widths of class formals are 0) must skip the raw-bit
                        // fast path: the mask degenerates to 0 and the write
                        // is silently dropped. The slow path stores the value
                        // untouched (fit_value_to_signal's w==0 rule), which
                        // is exactly what the AST does.
                        && *width > 0
                        && *width <= 64
                        && self.signal_widths[id] == *width
                        && !self.signal_real[id]
                        && !self.signal_two_state.get(id).copied().unwrap_or(false)
                    {
                        let mask = if *width >= 64 {
                            u64::MAX
                        } else {
                            (1u64 << *width) - 1
                        };
                        let (src_v, src_x) = self.vm_regs[*val_reg as usize].raw_bits();
                        let src_v = src_v & mask;
                        let src_x = src_x & mask;
                        let (dst_v, dst_x) = self.signal_table[id].raw_bits();
                        if self.warn_x && self.time > 0 {
                            self.warn_x_note_bits(id, dst_v & mask, dst_x & mask, src_v, src_x);
                        }
                        if src_v == (dst_v & mask) && src_x == (dst_x & mask) {
                            handled = true;
                        } else if self.signal_table[id].set_inline_bits(src_v, src_x) {
                            self.sync_mirror(id);
                            self.signal_table[id].is_signed = self.signal_signed[id];
                            if !self.dirty_signals[id] {
                                self.dirty_signals[id] = true;
                                self.dirty_list.push(id);
                            }
                            self.dirty_any = true;
                            self.table_modified = true;
                            self.after_signal_write(id);
                            handled = true;
                        }
                    }
                    if !handled {
                        let val = self.fit_value_to_signal(id, &self.vm_regs[*val_reg as usize]);
                        if self.signal_table[id] != val {
                            if !self.dirty_signals[id] {
                                self.dirty_signals[id] = true;
                                self.dirty_list.push(id);
                            }
                            self.dirty_any = true;
                            write_sig!(self, id, val);
                            self.table_modified = true;
                        }
                    }
                }
                Insn::BlockingAssignString(sig_id, val_reg) => {
                    // §6.16 string store: `fit_value_to_signal` keeps the
                    // value's own width (string-aware), no fast masking path.
                    let id = *sig_id as usize;
                    let forced =
                        !self.forced_signals.is_empty() && self.forced_signals.contains_key(&id);
                    if !forced {
                        let val = self.fit_value_to_signal(id, &self.vm_regs[*val_reg as usize]);
                        if self.signal_table[id] != val {
                            if !self.dirty_signals[id] {
                                self.dirty_signals[id] = true;
                                self.dirty_list.push(id);
                            }
                            self.dirty_any = true;
                            write_sig!(self, id, val);
                            self.table_modified = true;
                        }
                    }
                }
                Insn::BlockingAssignBitDyn(sig_id, idx_reg, val_reg) => {
                    let sig_id = &(*sig_id as usize);
                    // Hot path on c910: a single bit in some packed signal
                    // (e.g. `mem[i][7:0] = 0` after parser flatten). The
                    // previous version cloned the *entire* signal_table[id]
                    // — a wide signal can be many KB — just to flip one
                    // bit, then full-value compared to detect change. For
                    // wipe loops that touch thousands of cells per cycle
                    // this dominated allocator + memcpy time. Now: read
                    // current bit, skip the write if it matches; otherwise
                    // set in-place and mark dirty.
                    // §11.5.1: an x/z index modifies no bits. `to_u64` masks
                    // x bits to zero, so this cannot be folded into the range
                    // check below -- an unknown index looked like bit 0.
                    if let Some(idx) = dyn_bit_index(&self.vm_regs[*idx_reg as usize]) {
                        let bit = self.vm_regs[*val_reg as usize].get_bit(0);
                        let id = *sig_id;
                        if idx < self.signal_widths[id] as usize {
                            let cur = self.signal_table[id].get_bit(idx);
                            if cur != bit {
                                self.signal_table[id].set_bit(idx, bit);
                                self.sync_mirror(id);
                                self.signal_table[id].is_signed = self.signal_signed[id];
                                if !self.dirty_signals[id] {
                                    self.dirty_signals[id] = true;
                                    self.dirty_list.push(id);
                                }
                                self.dirty_any = true;
                                self.table_modified = true;
                                self.after_signal_write(id);
                            }
                        }
                    }
                }
                Insn::BlockingAssignRange(sig_id, hi, lo, val_reg) => {
                    let sig_id = &(*sig_id as usize);
                    let (low, high) = if hi >= lo { (*lo, *hi) } else { (*hi, *lo) };
                    let w = high - low + 1;
                    let val = self.vm_regs[*val_reg as usize].resize(w);
                    let id = *sig_id;
                    let sig_w = self.signal_widths[id];
                    let high_eff = high.min(sig_w.saturating_sub(1));
                    if oob_write_discard(high as i64, low as i64, sig_w) {
                        pc += 1;
                        continue;
                    }
                    let copy_count = high_eff.saturating_add(1).saturating_sub(low) as usize;

                    if low == 0 && high_eff + 1 >= sig_w {
                        let mut v = val.resize_for_assign(sig_w);
                        v.is_signed = self.signal_signed[id];
                        if self.signal_table[id] != v {
                            write_sig!(self, id, v);
                            if !self.dirty_signals[id] {
                                self.dirty_signals[id] = true;
                                self.dirty_list.push(id);
                            }
                            self.dirty_any = true;
                            self.table_modified = true;
                        }
                    } else if sig_w <= 64 && !self.signal_real[id] && low <= high_eff {
                        // `low > high_eff` means the select lies entirely
                        // ABOVE the signal's width — `high_eff` is clamped to
                        // `sig_w-1` but `low` is not. That happens when a
                        // declared width was itself clamped (a `[N*W-1:0]` wire
                        // over SANE_MAX_PACKED_WIDTH collapses to 1 bit) while
                        // the select still asks for high bits. Writing nothing
                        // is correct; computing `high_eff - low` underflows,
                        // panicking in debug and silently corrupting
                        // neighbouring bits in release.
                        let (src_v, src_x) = val.raw_bits();
                        let (base_v, base_x) = self.signal_table[id].raw_bits();
                        let (new_v, new_x) = Self::compose_inline_range_bits(
                            base_v, base_x, src_v, src_x, low, high_eff,
                        );
                        let mask_width = high_eff - low + 1;
                        let src_mask = if mask_width >= 64 {
                            u64::MAX
                        } else {
                            ((1u64 << mask_width) - 1) << low
                        };
                        if (base_v ^ new_v) & src_mask != 0 || (base_x ^ new_x) & src_mask != 0 {
                            self.signal_table[id].set_inline_bits(new_v, new_x);
                            self.sync_mirror(id);
                            self.signal_table[id].is_signed = self.signal_signed[id];
                            if !self.dirty_signals[id] {
                                self.dirty_signals[id] = true;
                                self.dirty_list.push(id);
                            }
                            self.dirty_any = true;
                            self.table_modified = true;
                            self.after_signal_write(id);
                        }
                    } else {
                        let changed =
                            self.signal_table[id].copy_bits_from(low as usize, &val, 0, copy_count);
                        if changed {
                            self.signal_table[id].is_signed = self.signal_signed[id];
                            if !self.dirty_signals[id] {
                                self.dirty_signals[id] = true;
                                self.dirty_list.push(id);
                            }
                            self.dirty_any = true;
                            self.table_modified = true;
                            self.after_signal_write(id);
                        }
                    }
                }
                Insn::BlockingAssignRangeDyn(sig_id, hi_reg, lo_reg, val_reg) => {
                    let sig_id = &(*sig_id as usize);
                    // §11.5.1: an x/z bound discards the write.
                    let (Some(hi), Some(lo)) = (
                        self.vm_regs[*hi_reg as usize].to_index(),
                        self.vm_regs[*lo_reg as usize].to_index(),
                    ) else {
                        pc += 1;
                        continue;
                    };
                    // §11.5.1 with a negative base (`x[i +: 6]`, i = -1):
                    // labels below bit 0 are outside the vector, so the
                    // value's low bits are dropped and only the in-range
                    // window is written. Converting the bounds to u32 first
                    // wrapped -1 to 4294967295 and dropped the whole write.
                    let (low_i, high_i) = (hi.min(lo), hi.max(lo));
                    if oob_write_discard(high_i, low_i, self.signal_widths[*sig_id]) || high_i < 0 {
                        pc += 1;
                        continue;
                    }
                    let below = (-low_i).max(0) as u32;
                    let (low, high) = (low_i.max(0) as u32, high_i.min(u32::MAX as i64) as u32);
                    let w = high - low + 1;
                    let val = if below > 0 {
                        self.vm_regs[*val_reg as usize]
                            .shift_right(&Value::from_u64(below as u64, 32))
                            .resize(w)
                    } else {
                        self.vm_regs[*val_reg as usize].resize(w)
                    };
                    let id = *sig_id;
                    let sig_w = self.signal_widths[id];
                    let high_eff = high.min(sig_w.saturating_sub(1));
                    let copy_count = high_eff.saturating_add(1).saturating_sub(low) as usize;

                    if low == 0 && high_eff + 1 >= sig_w {
                        let mut v = val.resize_for_assign(sig_w);
                        v.is_signed = self.signal_signed[id];
                        if self.signal_table[id] != v {
                            write_sig!(self, id, v);
                            if !self.dirty_signals[id] {
                                self.dirty_signals[id] = true;
                                self.dirty_list.push(id);
                            }
                            self.dirty_any = true;
                            self.table_modified = true;
                        }
                    } else if sig_w <= 64 && !self.signal_real[id] && low <= high_eff {
                        // `low > high_eff` means the select lies entirely
                        // ABOVE the signal's width — `high_eff` is clamped to
                        // `sig_w-1` but `low` is not. That happens when a
                        // declared width was itself clamped (a `[N*W-1:0]` wire
                        // over SANE_MAX_PACKED_WIDTH collapses to 1 bit) while
                        // the select still asks for high bits. Writing nothing
                        // is correct; computing `high_eff - low` underflows,
                        // panicking in debug and silently corrupting
                        // neighbouring bits in release.
                        let (src_v, src_x) = val.raw_bits();
                        let (base_v, base_x) = self.signal_table[id].raw_bits();
                        let (new_v, new_x) = Self::compose_inline_range_bits(
                            base_v, base_x, src_v, src_x, low, high_eff,
                        );
                        let mask_width = high_eff - low + 1;
                        let src_mask = if mask_width >= 64 {
                            u64::MAX
                        } else {
                            ((1u64 << mask_width) - 1) << low
                        };
                        if (base_v ^ new_v) & src_mask != 0 || (base_x ^ new_x) & src_mask != 0 {
                            self.signal_table[id].set_inline_bits(new_v, new_x);
                            self.sync_mirror(id);
                            self.signal_table[id].is_signed = self.signal_signed[id];
                            if !self.dirty_signals[id] {
                                self.dirty_signals[id] = true;
                                self.dirty_list.push(id);
                            }
                            self.dirty_any = true;
                            self.table_modified = true;
                            self.after_signal_write(id);
                        }
                    } else {
                        let changed =
                            self.signal_table[id].copy_bits_from(low as usize, &val, 0, copy_count);
                        if changed {
                            self.signal_table[id].is_signed = self.signal_signed[id];
                            if !self.dirty_signals[id] {
                                self.dirty_signals[id] = true;
                                self.dirty_list.push(id);
                            }
                            self.dirty_any = true;
                            self.table_modified = true;
                            self.after_signal_write(id);
                        }
                    }
                }
                Insn::LoadArrayElem(dest, array_name, idx_reg) => {
                    // Use the dense array_elem_ids Vec cache — eliminates
                    // the per-call `format!("{}[{}]", name, idx)` allocation
                    // and HashMap lookup, which alone cost ~500ns/call and
                    // dominated c910's per-cycle bytecode execution
                    // (thousands of array reads per clock). The cache is
                    // populated lazily by `get_array_elem_id` on first
                    // miss; subsequent calls are a Vec index away.
                    // §7.4.6 / §11.5.1: an UNKNOWN index reads x, NOT element 0.
                    // `to_u64()` returns None for an x/z value, and the old
                    // `unwrap_or(0)` silently read arr[0] — an AES S-box
                    // indexed by an uninitialized input returned real table
                    // data, so downstream logic resolved to a definite WRONG
                    // value instead of propagating x. A sentinel below `lo`
                    // makes the element resolve MISS, taking the existing
                    // out-of-range arm that already yields x.
                    let idx = {
                        let ir = &self.vm_regs[*idx_reg as usize];
                        // x/z index: force a resolve MISS so the existing
                        // out-of-range arm yields x. `to_u64()` masks x bits to
                        // ZERO and never returns None, so it cannot be used to
                        // detect this -- the old code silently read arr[0].
                        if ir.has_xz() {
                            i64::MIN
                        } else {
                            ir.to_u64().unwrap_or(0) as i64
                        }
                    };
                    if let Some(eid) = resolve_bytecode_array_elem(
                        array_name,
                        idx,
                        &self.array_first_id,
                        &self.signal_name_to_id,
                    ) {
                        // SoA read fast path (XEZIM_INLINE_BITS=1); with
                        // XEZIM_ARRAY_SOA_SHADOW=1 also assert coherence with
                        // the canonical signal_table before trusting it.
                        if is_packed_id(eid) {
                            let (v, x, w, signed) = self.packed.raw_signed(eid);
                            vm_store(&mut self.vm_regs[*dest as usize], v, x, w, signed);
                        } else if eid < soa_len && self.soa_read_ok[eid] {
                            let [v, x] = self.signal_inline_bits[eid];
                            if self.soa_shadow {
                                let (cv, cx) = self.cell_read(eid).raw_bits();
                                if v != cv || x != cx {
                                    eprintln!(
                                        "[SOA-SHADOW] LoadArrayElem mismatch eid={} inline=({:#x},{:#x}) table=({:#x},{:#x}) — aborting",
                                        eid, v, x, cv, cx
                                    );
                                    self.abort_run();
                                }
                            }
                            let (w, sg) = (self.signal_widths[eid], self.signal_signed[eid]);
                            vm_store(&mut self.vm_regs[*dest as usize], v, x, w, sg);
                        } else {
                            self.vm_regs[*dest as usize].copy_from(&self.signal_table[eid]);
                        }
                    } else {
                        // x/z or out-of-range index: x at the ELEMENT width.
                        let first_id = match array_name.as_ref() {
                            super::bytecode::ArrayOperand::Dense { first_id, .. } => {
                                Some(*first_id)
                            }
                            super::bytecode::ArrayOperand::Named(name) => self
                                .array_first_id
                                .get(name.as_str())
                                .map(|&(first, _, _)| first),
                        };
                        let w = first_id
                            .filter(|id| is_packed_id(*id))
                            .map(|id| self.packed.width(id))
                            .unwrap_or_else(|| {
                                bytecode_array_elem_width(
                                    array_name,
                                    &self.array_first_id,
                                    &self.signal_table,
                                )
                            });
                        let xm = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
                        vm_store(&mut self.vm_regs[*dest as usize], 0, xm, w, false);
                    }
                }
                Insn::NbaAssignArray(array_name, idx_reg, val_reg, width) => {
                    // §11.5.1: an x/z index selects no element: discarded.
                    let Some(idx) = self.vm_regs[*idx_reg as usize].to_index() else {
                        pc += 1;
                        continue;
                    };
                    if let Some(eid) = resolve_bytecode_array_elem(
                        array_name,
                        idx,
                        &self.array_first_id,
                        &self.signal_name_to_id,
                    ) {
                        let val = self.vm_regs[*val_reg as usize].resize(*width);
                        // §10.4.2 last-write-wins, THEN eval-time elision: an
                        // earlier NBA in this evaluation must be OVERWRITTEN,
                        // and the elision may only compare against the
                        // register when nothing is pending. Eliding on the
                        // register alone dropped a conditional override whose
                        // value happened to equal the CURRENT value while the
                        // unconditional default sat in the queue — a taken
                        // branch-to-self on the 4004 committed pc+2 (issue
                        // #142; every sibling arm already had this guard).
                        if is_packed_id(eid) {
                            // Issue #147: queue, do not commit — an NBA into
                            // a packed RAM must mature in the NBA region like
                            // any other, or a same-timestep reader in another
                            // edge block sees it a delta early.
                            let (bv, bx) = val.raw_bits();
                            self.queue_packed_nba(eid, bv, bx);
                        } else if let Some(i) = self.nba_fast_index.get(eid) {
                            self.nba_fast[i].value = val;
                        } else if self.signal_table[eid] != val {
                            self.nba_fast_index.insert(eid, self.nba_fast.len());
                            self.nba_fast.push(NbaFast {
                                block_index: 0,
                                signal_id: eid,
                                value: val,
                            });
                        } else {
                            self.prof_nba_elided += 1;
                        }
                    }
                }
                Insn::BlockingAssignArray(array_name, idx_reg, val_reg, width) => {
                    // §11.5.1: an x/z index selects no element: discarded.
                    let Some(idx) = self.vm_regs[*idx_reg as usize].to_index() else {
                        pc += 1;
                        continue;
                    };
                    if let Some(eid) = resolve_bytecode_array_elem(
                        array_name,
                        idx,
                        &self.array_first_id,
                        &self.signal_name_to_id,
                    ) {
                        if is_packed_id(eid) {
                            let val = self.vm_regs[*val_reg as usize].resize_for_assign(*width);
                            let (bv, bx) = val.raw_bits();
                            self.packed_store(eid, bv, bx);
                        } else {
                            let mut val = self.vm_regs[*val_reg as usize].resize_for_assign(*width);
                            // §6.11.2/§10.7: a 2-state element (`int a[4]`)
                            // stores x/z bits of the RHS as 0.
                            if self.signal_two_state[eid] {
                                val = val.to_two_state();
                            }
                            val.is_signed = self.signal_signed[eid];
                            if self.signal_table[eid] != val {
                                write_sig!(self, eid, val);
                                if !self.dirty_signals[eid] {
                                    self.dirty_signals[eid] = true;
                                    self.dirty_list.push(eid);
                                }
                                self.dirty_any = true;
                                self.table_modified = true;
                            }
                        }
                    }
                }
                Insn::NbaAssignArrayRange(array_name, idx_reg, hi_reg, lo_reg, val_reg) => {
                    // §11.5.1: an x/z index selects no element: discarded.
                    let Some(idx) = self.vm_regs[*idx_reg as usize].to_index() else {
                        pc += 1;
                        continue;
                    };
                    if let Some(eid) = resolve_bytecode_array_elem(
                        array_name,
                        idx,
                        &self.array_first_id,
                        &self.signal_name_to_id,
                    ) {
                        let hi = self.vm_regs[*hi_reg as usize].to_u64().unwrap_or(0) as u32;
                        let lo = self.vm_regs[*lo_reg as usize].to_u64().unwrap_or(0) as u32;
                        let (low, high) = if hi >= lo { (lo, hi) } else { (hi, lo) };
                        let w = high - low + 1;
                        let val = self.vm_regs[*val_reg as usize].resize(w);
                        if is_packed_id(eid) {
                            // Issue #147: no signal_widths/table slot for an
                            // arena id (the indexing below would panic).
                            // Compose the range into the PENDING queued value
                            // when one exists, else the cell, then queue.
                            let cell_w = self.packed.width(eid);
                            if oob_write_discard(high as i64, low as i64, cell_w) {
                                pc += 1;
                                continue;
                            }
                            let (mut cv, mut cx) =
                                match self.packed_nba.iter().find(|(i, _, _)| *i == eid) {
                                    Some(&(_, v, x)) => (v, x),
                                    None => {
                                        let (v, x, _) = self.packed.raw(eid);
                                        (v, x)
                                    }
                                };
                            let high_eff = high.min(cell_w.saturating_sub(1));
                            let (sv, sx) = val.raw_bits();
                            cv = splice_bits(cv, sv, low, high_eff);
                            cx = splice_bits(cx, sx, low, high_eff);
                            self.queue_packed_nba(eid, cv, cx);
                            // Advance past the store (see the blocking arm):
                            // a bare `continue` re-ran this NBA forever.
                            pc += 1;
                            continue;
                        }
                        let sig_w = self.signal_widths[eid];
                        let high_eff = high.min(sig_w.saturating_sub(1));
                        if oob_write_discard(high as i64, low as i64, sig_w) {
                            pc += 1;
                            continue;
                        }

                        if low == 0 && high_eff + 1 >= sig_w {
                            // Whole-element write — §10.4.2 last-write-wins
                            // against a pending entry FIRST, then elide if
                            // same as the register. Same defect class as the
                            // NbaAssignArray arm (issue #142): eliding on the
                            // register alone dropped an override equal to the
                            // current value while an earlier NBA sat queued.
                            let mut v = val.resize_for_assign(sig_w);
                            v.is_signed = self.signal_signed[eid];
                            if let Some(i) = self.nba_fast_index.get(eid) {
                                self.nba_fast[i].value = v;
                            } else if self.signal_table[eid] != v {
                                self.nba_fast_index.insert(eid, self.nba_fast.len());
                                self.nba_fast.push(NbaFast {
                                    block_index: 0,
                                    signal_id: eid,
                                    value: v,
                                });
                            } else {
                                self.prof_nba_elided += 1;
                            }
                        } else {
                            if let Some(i) = self.nba_fast_index.get(eid) {
                                let target = &mut self.nba_fast[i].value;
                                for bit_pos in low..=high_eff {
                                    target.set_bit(
                                        bit_pos as usize,
                                        val.get_bit((bit_pos - low) as usize),
                                    );
                                }
                            } else {
                                // Compare FIRST (no clone): flop reloads usually match, and
                                // the unconditional clone of a wide signal (1 byte/bit Vec)
                                // was a top allocator cost on c910. Clone + merge only from
                                // the first differing bit.
                                let mut first_diff = None;
                                {
                                    let src = &self.signal_table[eid];
                                    for bit_pos in low..=high_eff {
                                        if src.get_bit(bit_pos as usize)
                                            != val.get_bit((bit_pos - low) as usize)
                                        {
                                            first_diff = Some(bit_pos);
                                            break;
                                        }
                                    }
                                }
                                if let Some(start) = first_diff {
                                    let mut new_val = self.signal_table[eid].clone();
                                    for bit_pos in start..=high_eff {
                                        new_val.set_bit(
                                            bit_pos as usize,
                                            val.get_bit((bit_pos - low) as usize),
                                        );
                                    }
                                    self.nba_fast_index.insert(eid, self.nba_fast.len());
                                    self.nba_fast.push(NbaFast {
                                        block_index: 0,
                                        signal_id: eid,
                                        value: new_val,
                                    });
                                } else {
                                    self.prof_nba_elided += 1;
                                }
                            }
                        }
                    }
                }
                Insn::BlockingAssignArrayRange(array_name, idx_reg, hi_reg, lo_reg, val_reg) => {
                    // §11.5.1: an x/z index selects no element: discarded.
                    let Some(idx) = self.vm_regs[*idx_reg as usize].to_index() else {
                        pc += 1;
                        continue;
                    };
                    if let Some(eid) = resolve_bytecode_array_elem(
                        array_name,
                        idx,
                        &self.array_first_id,
                        &self.signal_name_to_id,
                    ) {
                        let hi = self.vm_regs[*hi_reg as usize].to_u64().unwrap_or(0) as u32;
                        let lo = self.vm_regs[*lo_reg as usize].to_u64().unwrap_or(0) as u32;
                        let (low, high) = if hi >= lo { (lo, hi) } else { (hi, lo) };
                        let w = high - low + 1;
                        let val = self.vm_regs[*val_reg as usize].resize(w);
                        if is_packed_id(eid) {
                            // Blocking splice into a packed cell — immediate,
                            // like assign_value's packed path; the per-signal
                            // indexing below would panic on the arena id.
                            let (mut cv, mut cx, cell_w) = self.packed.raw(eid);
                            if oob_write_discard(high as i64, low as i64, cell_w) {
                                pc += 1;
                                continue;
                            }
                            let high_eff = high.min(cell_w.saturating_sub(1));
                            let (sv, sx) = val.raw_bits();
                            cv = splice_bits(cv, sv, low, high_eff);
                            cx = splice_bits(cx, sx, low, high_eff);
                            self.packed_store(eid, cv, cx);
                            // Advance past the store: `continue` without it
                            // re-executed this instruction forever (the C906
                            // testbench's memory-image loop never finished
                            // with XEZIM_PACKED_MEM=1).
                            pc += 1;
                            continue;
                        }
                        let sig_w = self.signal_widths[eid];
                        let high_eff = high.min(sig_w.saturating_sub(1));
                        if oob_write_discard(high as i64, low as i64, sig_w) {
                            pc += 1;
                            continue;
                        }

                        if low == 0 && high_eff + 1 >= sig_w {
                            let mut v = val.resize_for_assign(sig_w);
                            v.is_signed = self.signal_signed[eid];
                            if self.signal_table[eid] != v {
                                write_sig!(self, eid, v);
                                if !self.dirty_signals[eid] {
                                    self.dirty_signals[eid] = true;
                                    self.dirty_list.push(eid);
                                }
                                self.dirty_any = true;
                                self.table_modified = true;
                            }
                        } else {
                            let mut changed = false;
                            for bit_pos in low..=high_eff {
                                let src_bit = val.get_bit((bit_pos - low) as usize);
                                if self.signal_table[eid].get_bit(bit_pos as usize) != src_bit {
                                    self.signal_table[eid].set_bit(bit_pos as usize, src_bit);
                                    self.sync_mirror(eid);
                                    changed = true;
                                }
                            }
                            if changed {
                                self.signal_table[eid].is_signed = self.signal_signed[eid];
                                if !self.dirty_signals[eid] {
                                    self.dirty_signals[eid] = true;
                                    self.dirty_list.push(eid);
                                }
                                self.dirty_any = true;
                                self.table_modified = true;
                                self.after_signal_write(eid);
                            }
                        }
                    }
                }
                Insn::Move(d, s) => {
                    let d = *d as usize;
                    let s = *s as usize;
                    if d != s {
                        // An inline source is four scalars — read them out and
                        // the aliasing problem disappears, so neither
                        // `split_at_mut` nor its two bounds checks are needed.
                        let src = &self.vm_regs[s];
                        let hdr = if src.is_real || src.is_fill {
                            None
                        } else {
                            src.inline_bits()
                                .map(|(v, x)| (v, x, src.width, src.is_signed))
                        };
                        match hdr {
                            Some((v, x, w, sg)) => vm_store(&mut self.vm_regs[d], v, x, w, sg),
                            None => {
                                // Split-borrow so we can copy_from in place without cloning.
                                let (lo, hi) = if d < s {
                                    let (a, b) = self.vm_regs.split_at_mut(s);
                                    (&mut a[d], &b[0])
                                } else {
                                    let (a, b) = self.vm_regs.split_at_mut(d);
                                    (&mut b[0], &a[s])
                                };
                                lo.copy_from(hi);
                            }
                        }
                    }
                }
                Insn::SetSigned(reg) => {
                    self.vm_regs[*reg as usize].is_signed = true;
                }
                Insn::ClearSigned(reg) => {
                    self.vm_regs[*reg as usize].is_signed = false;
                }
                Insn::DropXz(reg) => drop_xz(&mut self.vm_regs[*reg as usize]),
                Insn::Pow(d, l, r) => {
                    self.vm_regs[*d as usize] =
                        self.vm_regs[*l as usize].power(&self.vm_regs[*r as usize]);
                }
                Insn::CovHit(c) => self.cov_count(*c),
                Insn::Nop => {}
            }
            pc += 1;
        }
        self.prof_insns_executed += local_count;
    }
}
