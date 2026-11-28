//! Assignment targets: packed paths, nonblocking, blocking and continuous assignments.

use super::*;

impl<'a> BytecodeCompiler<'a> {
    /// Resolve `arr[i].m` into the operands an array-range store needs:
    /// (array, index reg, hi reg, lo reg, value resized to the member width).
    /// An indexed base keeps the lvalue a `MemberAccess` node — only the bare
    /// `s.m` form collapses to a dotted `Ident` — so both the NBA and the
    /// blocking arm land here. Shared so the two cannot drift apart, which
    /// this member/container pair has done twice.
    ///
    /// Emits the index and constant loads, so a `None` after that point would
    /// leave dead insns behind; every caller bails the whole block on `None`,
    /// which discards them.
    fn packed_array_member_store(
        &mut self,
        base: &Expression,
        member: &str,
        val_reg: RegId,
    ) -> Option<(Box<ArrayOperand>, RegId, RegId, RegId, RegId)> {
        let ExprKind::Index {
            expr: arr_expr,
            index,
        } = &base.kind
        else {
            return None;
        };
        let ExprKind::Ident(hier) = &arr_expr.kind else {
            return None;
        };
        let (_, layout) = self.packed_struct_layout_for_hier(hier)?;
        let f = layout.get(member)?;
        let (off, mw) = (f.offset, f.width);
        if mw == 0 {
            return None;
        }
        let name = self.lookup_array_name(hier)?;
        let idx_reg = self.compile_expr(index, 0)?;
        let resized = self.alloc_reg();
        self.emit(Insn::Move(resized, val_reg));
        self.emit(Insn::Resize(resized, mw));
        let hi_reg = self.alloc_reg();
        self.emit(Insn::LoadConst(
            hi_reg,
            Box::new(Value::from_u64((off + mw - 1) as u64, 32)),
        ));
        let lo_reg = self.alloc_reg();
        self.emit(Insn::LoadConst(
            lo_reg,
            Box::new(Value::from_u64(off as u64, 32)),
        ));
        let (array, idx_reg) = self.array_operand_index(name, idx_reg)?;
        Some((array, idx_reg, hi_reg, lo_reg, resized))
    }

    /// A packed lvalue PATH that mixes several dynamic steps:
    /// `sig[i][j]`, `sig[i].m`, `sig[i][hi:lo]`, `sig[i][b +: w]` and the
    /// dotted-member roots those accept (`s.arr[i].f`).
    ///
    /// Each store arm above resolves ONE dynamic component and bails on a
    /// combination — and a bail demotes the whole statement, plus any loop
    /// holding it, to the AST interpreter. The levels compose arithmetically:
    /// an index contributes `slot * <width of what it selects>`, a member its
    /// static field offset, a part-select its low bound. Summing them gives
    /// one dynamic bit offset into the ROOT signal, which the existing
    /// dynamic-range stores take as-is.
    ///
    /// Returns the root signal id and where the store goes in it, or `None`
    /// when a level is not statically resolvable — the caller's bail then
    /// stands. An x/z index poisons the offset arithmetic, so the
    /// dynamic-range store discards the write exactly as the single-level
    /// arms do (§11.5.1).
    fn packed_path_dyn_range(&mut self, lhs: &Expression) -> Option<(usize, PackedPathTarget, Option<RegId>)> {
        // A member or part-select may only sit at the LEAF; everything above
        // it must be an index chain over a packed root.
        let mut tail_member: Option<&str> = None;
        let mut tail_range: Option<(&Expression, &Expression, &RangeKind)> = None;
        let mut cur = lhs;
        match &cur.kind {
            ExprKind::MemberAccess { expr, member } => {
                tail_member = Some(member.name.as_str());
                cur = expr;
            }
            ExprKind::RangeSelect {
                expr,
                left,
                right,
                kind,
            } => {
                tail_range = Some((left, right, kind));
                cur = expr;
            }
            _ => {}
        }
        let mut idx_nodes: Vec<&Expression> = Vec::new();
        while let ExprKind::Index { expr, .. } = &cur.kind {
            idx_nodes.push(cur);
            cur = expr;
        }
        if idx_nodes.is_empty() {
            // Single-level shapes already have their own arms.
            return None;
        }
        idx_nodes.reverse();
        let ExprKind::Ident(hier) = &cur.kind else {
            return None;
        };
        // Unpacked arrays, associative arrays and collections keep their own
        // stores — this is the packed-vector path only.
        if self.lookup_array_name(hier).is_some()
            || self.is_multi_dim_array(hier)
            || self.is_assoc_target(hier)
            || self.collection_store_denied(hier)
        {
            return None;
        }
        // The root is either a signal of its own or a member of one: a
        // struct member (`hc.lane_dec`) has no signal, only a bit range
        // inside its carrier, so start the offset at that range's base.
        let (id, mut const_off, member_w) = match self.lookup_signal_id(hier) {
            Some(id) => (id, 0u32, None),
            None => {
                let (base_id, off, mw) = self.packed_struct_member_target(hier)?;
                (base_id, off, Some(mw))
            }
        };
        let dims: Vec<(i64, i64)> = self.packed_full_dims_of(hier)?.to_vec();
        if dims.is_empty() || dims.len() < idx_nodes.len() {
            return None;
        }
        // Strides come from the DECLARED dimensions, not from
        // `infer_lhs_width` of the sub-expression: width inference does not
        // model these nested selects (it answers 32 for `q[i][j]`), which is
        // why they reach this path at all. `packed_full_dims_of` lists every
        // packed dimension down to the bit, element types included, so the
        // stride of level k is the product of the dimensions below it and the
        // product of them all must be the signal's width — if it is not, the
        // layout is something this arithmetic does not describe.
        let counts: Vec<u64> = dims
            .iter()
            .map(|&(l, r)| (l.max(r) - l.min(r) + 1) as u64)
            .collect();
        let root_w = match member_w {
            Some(mw) => mw as u64,
            None => self.infer_lhs_width(cur) as u64,
        };
        if root_w == 0 || counts.iter().product::<u64>() != root_w {
            return None;
        }
        let stride = |k: usize| -> u64 { counts[k + 1..].iter().product::<u64>() };
        // §7.4.1: an ascending dimension mirrors its labels. Those bases are
        // AST-only everywhere else in this compiler; keep it that way rather
        // than introduce a second mapping rule here.
        if dims
            .iter()
            .take(idx_nodes.len())
            .any(|&d| Self::dim_is_ascending(Some(d)))
        {
            return None;
        }

        let mut acc: Option<RegId> = None;
        let mut valid: Option<RegId> = None;
        let mut slots: Option<(RegId, u64, u32, u64)> = None;
        let add_term = |c: &mut Self, term: RegId, acc: &mut Option<RegId>| match *acc {
            None => *acc = Some(term),
            Some(a) => {
                let sum = c.alloc_reg();
                c.emit(Insn::Add(sum, a, term));
                *acc = Some(sum);
            }
        };
        for (level, node) in idx_nodes.iter().enumerate() {
            let ExprKind::Index { index, .. } = &node.kind else {
                return None;
            };
            // The width of what THIS index selects is the stride of the level.
            let elem_w = u32::try_from(stride(level)).ok()?;
            if elem_w == 0 {
                return None;
            }
            if let Some(k) = self.fold_const_i64(index) {
                let lsb = Self::packed_elem_lsb(Some(dims[level]), k, elem_w);
                if lsb < 0 {
                    return None;
                }
                const_off = const_off.checked_add(u32::try_from(lsb).ok()?)?;
                continue;
            }
            let dim = dims[level];
            let proven = self.packed_index_bounds(index)
                .is_some_and(|(lo, hi)| lo >= dim.0.min(dim.1) && hi <= dim.0.max(dim.1));
            let slot = if proven {
                if (0..=i64::from(i32::MAX)).contains(&dim.1) {
                    let index = self.compile_expr(index, 0)?;
                    self.emit_packed_slot_index(Some(dim), index)
                } else {
                    self.compile_packed_store_slot(index, dim)?
                }
            } else {
                let slot = self.compile_packed_store_slot(index, dim)?;
                // §7.4.6: a subscript outside this dimension cannot address
                // a neighbouring element, even if its combined offset is valid.
                let count = self.alloc_reg();
                self.emit(Insn::LoadConst(count, Box::new(Value::from_u64(counts[level], 64))));
                let in_bounds = self.alloc_reg();
                self.emit(Insn::Lt(in_bounds, slot, count));
                valid = Some(if let Some(previous) = valid {
                    let both = self.alloc_reg();
                    self.emit(Insn::LogAnd(both, previous, in_bounds));
                    both
                } else {
                    in_bounds
                });
                slot
            };
            let term = if elem_w == 1 {
                slot
            } else {
                let w_reg = self.alloc_reg();
                self.emit(Insn::LoadConst(
                    w_reg,
                    Box::new(Value::from_u64(elem_w as u64, 32)),
                ));
                let t = self.alloc_reg();
                self.emit(Insn::Mul(t, slot, w_reg));
                t
            };
            add_term(self, term, &mut acc);
        }

        let width = if let Some(m) = tail_member {
            // The layout map holds the ELEMENT struct's fields for an
            // array-of-struct signal (as `packed_array_member_store` uses it).
            let (_, layout) = self.packed_struct_layout_for_hier(hier)?;
            let f = layout.get(m)?;
            let (off, mw) = (f.offset, f.width);
            if mw == 0 {
                return None;
            }
            const_off = const_off.checked_add(off)?;
            mw
        } else if let Some((left, right, kind)) = tail_range
            && idx_nodes.len() < dims.len()
        {
            // §7.4.3, §11.5.1: the part-select addresses the NEXT declared
            // dimension, whose elements are `ew` bits wide — on
            // `logic [3:0][5:0][7:0] a`, `a[0][5:0]` is six bytes. Treating
            // its labels as bit offsets wrote 6 of the 48 bits.
            let level = idx_nodes.len();
            let dim = dims[level];
            let ew = u32::try_from(stride(level)).ok()?;
            let count = counts[level] as i64;
            let in_dim = |s: i64| (0..count).contains(&s);
            let n = match kind {
                RangeKind::Constant => {
                    let a = Self::label_to_phys(dim, self.eval_const_bound(left)?);
                    let b = Self::label_to_phys(dim, self.eval_const_bound(right)?);
                    // A partly out-of-range select keeps the interpreter's
                    // per-bit placement (§11.5.1).
                    if a.min(b) >= count || a.max(b) < 0 {
                        return Some((id, PackedPathTarget::Ignore, None));
                    }
                    if !in_dim(a) || !in_dim(b) {
                        return None;
                    }
                    let lsb = u32::try_from(a.min(b)).ok()?.checked_mul(ew)?;
                    const_off = const_off.checked_add(lsb)?;
                    a.abs_diff(b) + 1
                }
                RangeKind::IndexedUp | RangeKind::IndexedDown => {
                    let w = self
                        .fold_const(right)
                        .and_then(|v| v.to_u64())
                        .filter(|&w| w > 0 && w <= count as u64)?;
                    // On an ascending dimension `+:` runs down the slots.
                    let up = (*kind == RangeKind::IndexedUp) != Self::dim_is_ascending(Some(dim));
                    if let Some(b) = self.fold_const_i64(left) {
                        let first = Self::label_to_phys(dim, b);
                        let low = if up { first } else { first - (w as i64 - 1) };
                        let high = low.checked_add(w as i64 - 1)?;
                        if low >= count || high < 0 {
                            return Some((id, PackedPathTarget::Ignore, None));
                        }
                        if !in_dim(low) || !in_dim(high) {
                            return None;
                        }
                        let lsb = u32::try_from(low).ok()?.checked_mul(ew)?;
                        const_off = const_off.checked_add(lsb)?;
                    } else {
                        // §11.5.1: a run-time base can run past either end
                        // of the dimension; only the slots inside it are
                        // written (see `emit_slot_run_store`).
                        let slot = self.compile_packed_store_slot(left, dim)?;
                        let low = if up {
                            slot
                        } else {
                            let out = self.alloc_reg();
                            self.emit(Insn::BinOpConst(
                                out,
                                slot,
                                Box::new(Value::from_u64(w - 1, 32)),
                                BinOpConstKind::Sub,
                            ));
                            out
                        };
                        let offset = w as i64 - 1;
                        let inside = self.packed_index_bounds(left).is_some_and(|(a, b)| {
                            let a = Self::label_to_phys(dim, a);
                            let b = Self::label_to_phys(dim, b);
                            if up {
                                in_dim(a.min(b)) && b.max(a).checked_add(offset).is_some_and(in_dim)
                            } else {
                                in_dim(a.max(b)) && a.min(b).checked_sub(offset).is_some_and(in_dim)
                            }
                        });
                        if inside {
                            let scaled = if ew == 1 { low } else { self.emit_slot_bit(low, ew, 0) };
                            add_term(self, scaled, &mut acc);
                        } else {
                            slots = Some((low, w, ew, count as u64));
                        }
                    }
                    w
                }
            };
            u32::try_from(n).ok()?.checked_mul(ew)?
        } else if tail_range.is_some() {
            return None;
        } else {
            u32::try_from(stride(idx_nodes.len() - 1)).ok()?
        };
        if width == 0 {
            return None;
        }

        let lo_reg = match acc {
            None => {
                let r = self.alloc_reg();
                self.emit(Insn::LoadConst(
                    r,
                    Box::new(Value::from_u64(const_off as u64, 32)),
                ));
                r
            }
            Some(a) if const_off == 0 => a,
            Some(a) => {
                let c = self.alloc_reg();
                self.emit(Insn::LoadConst(
                    c,
                    Box::new(Value::from_u64(const_off as u64, 32)),
                ));
                let sum = self.alloc_reg();
                self.emit(Insn::Add(sum, a, c));
                sum
            }
        };
        Some(match slots {
            Some((first, n, ew, count)) => (
                id,
                PackedPathTarget::Slots(SlotRun {
                    base: lo_reg,
                    first,
                    n,
                    ew,
                    count,
                }),
                valid,
            ),
            None => (id, PackedPathTarget::Range(lo_reg, width), valid),
        })
    }

    /// Bounds implied by an unsigned width or a constant-count loop.
    fn packed_index_bounds(&mut self, expr: &Expression) -> Option<(i64, i64)> {
        if let Some(index) = self.fold_const_i64(expr) {
            return Some((index, index));
        }
        if let ExprKind::Ident(h) = &expr.kind
            && let Some((reg, _)) = self.local_var_reg_of(h)
            && let Some(&range) = self.loop_index_bounds.get(&reg)
        {
            return Some(range);
        }
        let width = self.lrm_self_width(expr);
        if let ExprKind::Binary { op: BinaryOp::Mul, left, right } = &expr.kind
            && let Some((a, b)) = self.packed_index_bounds(left)
            && let Some((c, d)) = self.packed_index_bounds(right)
            && a >= 0 && c >= 0 && width > 0
        {
            let high = b.checked_mul(d)?;
            let magnitude = width - u32::from(self.expr_signedness(expr)?);
            if magnitude >= 63 || high < (1i64 << magnitude) {
                return Some((a.checked_mul(c)?, high));
            }
        }
        (width > 0 && width < 63 && self.expr_signedness(expr) == Some(false))
            .then(|| (0, (1i64 << width) - 1))
    }

    /// Translate a signed or unsigned label to its LSB-first slot before any
    /// truncation. Widen small signed indices before subtracting the origin.
    fn compile_packed_store_slot(&mut self, expr: &Expression, dim: (i64, i64)) -> Option<RegId> {
        let index = self.compile_expr(expr, 0)?;
        let index = if self.lrm_self_width(expr) < 64 {
            let wide = self.alloc_reg();
            self.emit(Insn::MoveResize(wide, index, 64));
            wide
        } else {
            index
        };
        let (left, right) = dim;
        if left >= right && right == 0 {
            return Some(index);
        }
        let origin = self.alloc_reg();
        self.emit(Insn::LoadConst(origin, Box::new(Value::from_u64(right as u64, 64))));
        let slot = self.alloc_reg();
        self.emit(if left >= right {
            Insn::Sub(slot, index, origin)
        } else {
            Insn::Sub(slot, origin, index)
        });
        Some(slot)
    }

    /// §11.5.1: store a part-select whose base is only known at run time,
    /// writing only its slots inside the dimension. The store instructions
    /// clip to the whole ROOT signal, so a slice that ran past the end of its
    /// own dimension wrote into the neighbouring element: `a[0][j +: 2]` on
    /// `logic [3:0][5:0][7:0] a` with `j = 5` put its second byte in `a[1][0]`.
    ///
    /// When the whole run is inside the dimension, one store of fixed width
    /// does it, as before. Otherwise each slot is stored on its own, if it is
    /// inside; every store then has a constant width, which the two-state
    /// executor needs. A run longer than `MAX_SLOT_RUN` instead stores the
    /// part inside as one range whose ends are clipped at run time, which
    /// keeps its registers and instructions fixed. An x/z base fails every
    /// check and writes nothing.
    fn emit_slot_run_store(&mut self, sig: SigId, run: SlotRun, val_reg: RegId, nba: bool) {
        let store = |c: &mut Self, lo: RegId, w: u32, v: RegId| {
            let hi = if w == 1 {
                lo
            } else {
                let hi = c.alloc_reg();
                c.emit(Insn::BinOpConst(
                    hi,
                    lo,
                    Box::new(Value::from_u64(u64::from(w) - 1, 32)),
                    BinOpConstKind::Add,
                ));
                hi
            };
            c.emit(if nba {
                Insn::NbaAssignRangeDyn(sig, hi, lo, v)
            } else {
                Insn::BlockingAssignRangeDyn(sig, hi, lo, v)
            });
        };
        let slot_bit = |c: &mut Self, slot: RegId| {
            let rel = c.emit_slot_bit(slot, run.ew, 0);
            let bit = c.alloc_reg();
            c.emit(Insn::Add(bit, run.base, rel));
            bit
        };
        let total = u32::try_from(run.n)
            .unwrap_or(u32::MAX)
            .saturating_mul(run.ew);
        let value = self.alloc_reg();
        self.emit(Insn::MoveResize(value, val_reg, total));
        let count = self.alloc_reg();
        self.emit(Insn::LoadConst(
            count,
            Box::new(Value::from_u64(run.count, 32)),
        ));
        // Unsigned compares: a slot below 0 has wrapped to a large value.
        let last = self.alloc_reg();
        self.emit(Insn::BinOpConst(
            last,
            run.first,
            Box::new(Value::from_u64(run.n - 1, 32)),
            BinOpConstKind::Add,
        ));
        let first_in = self.alloc_reg();
        self.emit(Insn::Lt(first_in, run.first, count));
        let last_in = self.alloc_reg();
        self.emit(Insn::Lt(last_in, last, count));
        let inside = self.alloc_reg();
        self.emit(Insn::LogAnd(inside, first_in, last_in));
        let to_slots = self.insns.len();
        self.emit(Insn::BranchIfFalse(inside, 0));
        let lo = slot_bit(self, run.first);
        store(self, lo, total, value);
        let to_end = self.insns.len();
        self.emit(Insn::Jump(0));
        let slots_pc = self.insns.len() as u32;
        self.insns[to_slots] = Insn::BranchIfFalse(inside, slots_pc);
        if run.n > MAX_SLOT_RUN {
            let any = self.alloc_reg();
            self.emit(Insn::LogOr(any, first_in, last_in));
            let skip = self.insns.len();
            self.emit(Insn::BranchIfFalse(any, 0));
            let pick = |c: &mut Self, cond: RegId, t: RegId, e: RegId| {
                let out = c.alloc_reg();
                c.emit(Insn::Select(out, cond, t, e));
                out
            };
            // An end outside the dimension moves in to that end of it.
            let lo_full = slot_bit(self, run.first);
            let lo = pick(self, first_in, lo_full, run.base);
            let hi_rel = self.emit_slot_bit(last, run.ew, run.ew - 1);
            let hi_full = self.alloc_reg();
            self.emit(Insn::Add(hi_full, run.base, hi_rel));
            let top = self.alloc_reg();
            self.emit(Insn::BinOpConst(
                top,
                run.base,
                Box::new(Value::from_u64(run.count * u64::from(run.ew) - 1, 32)),
                BinOpConstKind::Add,
            ));
            let hi = pick(self, last_in, hi_full, top);
            // A run starting below slot 0 loses its low `n - 1 - last`
            // elements: shift their bits out of the value.
            let n_last = self.alloc_reg();
            self.emit(Insn::LoadConst(
                n_last,
                Box::new(Value::from_u64(run.n - 1, 32)),
            ));
            let below = self.alloc_reg();
            self.emit(Insn::Sub(below, n_last, last));
            let below_bits = self.emit_slot_bit(below, run.ew, 0);
            let zero = self.alloc_reg();
            self.emit(Insn::LoadConst(zero, Box::new(Value::from_u64(0, 32))));
            let shift = pick(self, first_in, zero, below_bits);
            let shifted = self.alloc_reg();
            self.emit(Insn::Shr(shifted, value, shift));
            self.emit(if nba {
                Insn::NbaAssignRangeDyn(sig, hi, lo, shifted)
            } else {
                Insn::BlockingAssignRangeDyn(sig, hi, lo, shifted)
            });
            let end = self.insns.len() as u32;
            self.insns[skip] = Insn::BranchIfFalse(any, end);
            self.insns[to_end] = Insn::Jump(end);
            return;
        }
        for k in 0..run.n {
            let slot = if k == 0 {
                run.first
            } else {
                let s = self.alloc_reg();
                self.emit(Insn::BinOpConst(
                    s,
                    run.first,
                    Box::new(Value::from_u64(k, 32)),
                    BinOpConstKind::Add,
                ));
                s
            };
            let ok = self.alloc_reg();
            self.emit(Insn::Lt(ok, slot, count));
            let skip = self.insns.len();
            self.emit(Insn::BranchIfFalse(ok, 0));
            let part = self.alloc_reg();
            let k = k as u32;
            self.emit(Insn::RangeSelectConst(
                part,
                value,
                (k + 1) * run.ew - 1,
                k * run.ew,
            ));
            let lo = slot_bit(self, slot);
            store(self, lo, run.ew, part);
            let next = self.insns.len() as u32;
            self.insns[skip] = Insn::BranchIfFalse(ok, next);
        }
        let end = self.insns.len() as u32;
        self.insns[to_end] = Insn::Jump(end);
    }

    /// `(hi, lo)` registers and the value resized to `width`, for the dynamic
    /// range stores that `packed_path_dyn_range` feeds.
    fn packed_path_store_regs(
        &mut self,
        lo_reg: RegId,
        width: u32,
        val_reg: RegId,
    ) -> (RegId, RegId) {
        let resized = self.alloc_reg();
        self.emit(Insn::Move(resized, val_reg));
        self.emit(Insn::Resize(resized, width));
        let hi_reg = if width == 1 {
            lo_reg
        } else {
            let d = self.alloc_reg();
            self.emit(Insn::LoadConst(
                d,
                Box::new(Value::from_u64((width - 1) as u64, 32)),
            ));
            let hi = self.alloc_reg();
            self.emit(Insn::Add(hi, lo_reg, d));
            hi
        };
        (hi_reg, resized)
    }

    /// Try the composed packed path, emitting `store` on success. The
    /// resolver emits address arithmetic as it descends, so a level it cannot
    /// resolve must leave NOTHING behind: the instruction stream, the register
    /// counter and the bail reason are all restored before returning false.
    fn try_packed_path(&mut self, lhs: &Expression, val_reg: RegId, nba: bool) -> bool {
        let start = self.insns.len();
        let start_reg = self.next_reg;
        let saved_reason = self.bail_reason;
        if let Some((id, target, valid)) = self.packed_path_dyn_range(lhs) {
            let sig = as_sig_id(id);
            let guard = valid.map(|condition| {
                let branch = self.insns.len();
                self.emit(Insn::BranchIfFalse(condition, 0));
                (branch, condition)
            });
            match target {
                PackedPathTarget::Ignore => {}
                PackedPathTarget::Range(lo, width) => {
                    let (hi, resized) = self.packed_path_store_regs(lo, width, val_reg);
                    self.emit(if nba {
                        Insn::NbaAssignRangeDyn(sig, hi, lo, resized)
                    } else {
                        Insn::BlockingAssignRangeDyn(sig, hi, lo, resized)
                    });
                }
                PackedPathTarget::Slots(run) => self.emit_slot_run_store(sig, run, val_reg, nba),
            }
            if let Some((branch, condition)) = guard {
                self.insns[branch] = Insn::BranchIfFalse(condition, self.insns.len() as u32);
            }
            return true;
        }
        self.truncate_insns(start);
        self.next_reg = start_reg;
        self.bail_reason = saved_reason;
        false
    }

    fn try_packed_path_nba(&mut self, lhs: &Expression, val_reg: RegId) -> bool {
        self.try_packed_path(lhs, val_reg, true)
    }

    fn try_packed_path_blocking(&mut self, lhs: &Expression, val_reg: RegId) -> bool {
        self.try_packed_path(lhs, val_reg, false)
    }

    /// §23.8: a frame declaration shadows the design namespace even when
    /// this compiler cannot lower its selected write. Decline that write;
    /// resolving a homonymous module signal would change a different object.
    pub(super) fn frame_bound_target(&self, lhs: &Expression) -> bool {
        let mut base = lhs;
        while let ExprKind::Index { expr, .. }
        | ExprKind::RangeSelect { expr, .. }
        | ExprKind::MemberAccess { expr, .. }
        | ExprKind::Paren(expr) = &base.kind
        {
            base = expr;
        }
        let ExprKind::Ident(h) = &base.kind else {
            return false;
        };
        if h.root.is_some() || h.path.len() != 1 {
            return false;
        }
        let name = h.path[0].name.name.as_str();
        self.local_var_regs.contains_key(name)
            || self.local_var_array.contains_key(name)
            || self.local_array_regs.contains_key(name)
            || (self.method_mode
                && (self.bare_member_names.contains(name)
                    || self.static_member_names.contains(name)
                    || self.class_shadow_names.contains(name)))
    }

    pub(super) fn compile_nba_target(
        &mut self,
        lhs: &Expression,
        val_reg: RegId,
        width: u32,
    ) -> bool {
        if self.frame_bound_target(lhs) {
            self.bail("nba_frame_target");
            return false;
        }
        match &lhs.kind {
            ExprKind::Ident(hier) => {
                if let Some(id) = self.lookup_signal_id(hier) {
                    self.emit(Insn::NbaAssign(as_sig_id(id), val_reg, width));
                    true
                } else if let Some((base_id, off, mw)) = self.packed_struct_member_target(hier) {
                    // Packed-struct member NBA (`s.m0 <= …`): mirror of the
                    // blocking arm — splice into `[off + mw - 1 : off]` of the
                    // container. Range NBAs compose onto a pending nba_fast
                    // entry, so several members of one container written in the
                    // same cycle each keep their own slice.
                    let resized = self.alloc_reg();
                    self.emit(Insn::Move(resized, val_reg));
                    self.emit(Insn::Resize(resized, mw));
                    self.emit(Insn::NbaAssignRange(
                        as_sig_id(base_id),
                        off + mw - 1,
                        off,
                        resized,
                    ));
                    true
                } else {
                    self.bail("nba_ident_unresolved");
                    false
                }
            }
            ExprKind::Index { expr, index } => {
                // §7.4.1/§11.5.1: ascending or element-of-collection bases
                // need label mapping the stores below do not emit — AST only.
                if self.sel_base_needs_ast(expr) {
                    self.bail("nba_sel_base_maps");
                    return false;
                }
                if let Some(id) = self.const_multi_dim_array_elem_signal_id(lhs) {
                    self.emit(Insn::NbaAssign(as_sig_id(id), val_reg, width));
                    return true;
                }
                if let ExprKind::Ident(hier) = &expr.kind {
                    if self.is_assoc_target(hier) {
                        self.bail("nba_target_assoc");
                        return false;
                    }
                    if self.collection_store_denied(hier) {
                        self.bail("nba_target_collection");
                        return false;
                    }
                    if let Some(name) = self.lookup_array_name(hier) {
                        if let Some(idx_reg) = self.compile_expr(index, 0) {
                            let Some((array, idx_reg)) = self.array_operand_index(name, idx_reg)
                            else {
                                return false;
                            };
                            self.emit(Insn::NbaAssignArray(array, idx_reg, val_reg, width));
                            return true;
                        }
                    }
                    if let Some(id) = self.lookup_signal_id(hier) {
                        // Packed multi-D NBA: `mem[i] <= data` must write the
                        // W-bit slice at `i*W +: W`. Mirrors compile_blocking_target.
                        let elem_w = self.packed_elem_width_of(hier);
                        if let Some(elem_w) = elem_w {
                            if let Some(idx_reg) = self.compile_expr(index, 0) {
                                // Normalize the index to a 0-based, LSB-first
                                // slot using the DECLARED outer range.
                                let dim = self.packed_outer_dim(hier);
                                let idx_reg = self.emit_packed_slot_index(dim, idx_reg);
                                let elem_w_reg = self.alloc_reg();
                                self.emit(Insn::LoadConst(
                                    elem_w_reg,
                                    Box::new(Value::from_u64(elem_w as u64, 32)),
                                ));
                                let lo_reg = self.alloc_reg();
                                self.emit(Insn::Mul(lo_reg, idx_reg, elem_w_reg));
                                let em1_reg = self.alloc_reg();
                                self.emit(Insn::LoadConst(
                                    em1_reg,
                                    Box::new(Value::from_u64((elem_w - 1) as u64, 32)),
                                ));
                                let hi_reg = self.alloc_reg();
                                self.emit(Insn::Add(hi_reg, lo_reg, em1_reg));
                                self.emit(Insn::NbaAssignRangeDyn(
                                    as_sig_id(id),
                                    hi_reg,
                                    lo_reg,
                                    val_reg,
                                ));
                                return true;
                            }
                        }
                        if let Some(idx_reg) = self.compile_expr(index, 0) {
                            // §7.4.1: rebase for non-zero-based vectors.
                            let idx_reg = self.emit_rebased_index(hier, idx_reg);
                            self.emit(Insn::NbaAssignBitDyn(as_sig_id(id), idx_reg, val_reg));
                            return true;
                        }
                    }
                }
                if let Some(id) = self.flattened_outer_const_signal_id(expr) {
                    if let Some(idx_reg) = self.compile_expr(index, 0) {
                        self.emit(Insn::NbaAssignBitDyn(as_sig_id(id), idx_reg, val_reg));
                        return true;
                    }
                }
                // `a[i][j] <= v` on a 2-D unpacked array: same row-major
                // addressing the READ path already uses, then an ordinary
                // array store. Elements are materialized contiguously, so the
                // flat index and its out-of-range guard are shared with
                // `compile_2d_array_read`. Without this the whole statement —
                // and any loop containing it — stayed on the AST path, which
                // measured ~24x slower than the 1-D equivalent.
                if let ExprKind::Index {
                    expr: outer,
                    index: j_expr,
                } = &lhs.kind
                    && let ExprKind::Index {
                        expr: base,
                        index: i_expr,
                    } = &outer.kind
                    && let ExprKind::Ident(hier) = &base.kind
                    && let Some((array, flat)) = self.compile_2d_flat_index(hier, i_expr, j_expr)
                {
                    self.emit(Insn::NbaAssignArray(array, flat, val_reg, width));
                    return true;
                }
                if self.try_packed_path_nba(lhs, val_reg) {
                    return true;
                }
                self.bail("nba_index_other");
                false
            }
            ExprKind::RangeSelect {
                expr,
                left,
                right,
                kind,
            } => {
                // §7.4.1/§11.5.1: ascending or element-of-collection bases
                // need label mapping the stores below do not emit — AST only.
                // So does a multidimensional packed base, whose select names
                // elements, not bits (the blocking store bails on it too).
                let packed_elements = match &expr.kind {
                    ExprKind::Ident(h) => self.packed_elem_width_of(h).is_some_and(|w| w > 1),
                    _ => false,
                };
                if self.sel_base_needs_ast(expr) || packed_elements {
                    self.bail("nba_range_base_maps");
                    return false;
                }
                if let ExprKind::Ident(hier) = &expr.kind {
                    // §7.4.1, §7.4.3: on a multi-D PACKED vector the labels
                    // count ELEMENTS (`pv[4:3] <= v` on `logic [5:0][7:0] pv`
                    // is 16 bits) — the interpreter's element-aware store, as
                    // in the blocking arm. The bit range below wrote 2 bits.
                    if self.packed_elem_width_of(hier).is_some() {
                        self.bail("nba_range_packed_multid");
                        return false;
                    }
                    if let Some(id) = self.lookup_signal_id(hier) {
                        // §7.4.1: rebase declared indices to physical offsets
                        // for a non-zero-based vector (see the blocking arm).
                        let base_lo = self.declared_low_bound(hier);
                        match kind {
                            RangeKind::Constant => {
                                if let (Some(hi), Some(lo)) =
                                    (self.eval_const_bound(left), self.eval_const_bound(right))
                                {
                                    // §11.5.1: labels below the declared low
                                    // bound are outside the vector; only the
                                    // in-range labels are written, from the
                                    // value's corresponding bits. Entirely
                                    // below: nothing is written.
                                    let (phys_hi, phys_lo) =
                                        (hi as i64 - base_lo, lo as i64 - base_lo);
                                    let (phys_hi, phys_lo) =
                                        (phys_hi.max(phys_lo), phys_hi.min(phys_lo));
                                    if phys_hi < 0
                                        || (phys_lo < 0
                                            && crate::compiler::simulator::oob_select_whole())
                                    {
                                        return true;
                                    }
                                    let val_reg = if phys_lo < 0 {
                                        let shifted = self.alloc_reg();
                                        let amt = self.alloc_reg();
                                        self.emit(Insn::LoadConst(
                                            amt,
                                            Box::new(Value::from_u64((-phys_lo) as u64, 32)),
                                        ));
                                        self.emit(Insn::Shr(shifted, val_reg, amt));
                                        shifted
                                    } else {
                                        val_reg
                                    };
                                    let (hi, lo) = (phys_hi as u32, phys_lo.max(0) as u32);
                                    self.emit(Insn::NbaAssignRange(as_sig_id(id), hi, lo, val_reg));
                                    return true;
                                }
                            }
                            RangeKind::IndexedUp | RangeKind::IndexedDown => {
                                let width = match self.eval_const_expr(right) {
                                    Some(w) if w > 0 => w,
                                    _ => {
                                        self.bail("nba_range_width_nonconst");
                                        return false;
                                    }
                                };
                                let resized = self.alloc_reg();
                                self.emit(Insn::Move(resized, val_reg));
                                self.emit(Insn::Resize(resized, width));
                                let Some(idx) = self.compile_expr(left, 0) else {
                                    self.bail("nba_range_base");
                                    return false;
                                };
                                // §7.4.1: rebase for non-zero-based vectors.
                                let idx = self.emit_rebased_index(hier, idx);
                                let (hi_reg, lo_reg) = if width == 1 {
                                    (idx, idx)
                                } else {
                                    let delta = self.alloc_reg();
                                    self.emit(Insn::LoadConst(
                                        delta,
                                        Box::new(Value::from_u64((width - 1) as u64, 32)),
                                    ));
                                    let other = self.alloc_reg();
                                    if *kind == RangeKind::IndexedUp {
                                        self.emit(Insn::Add(other, idx, delta));
                                        (other, idx)
                                    } else {
                                        self.emit(Insn::Sub(other, idx, delta));
                                        (idx, other)
                                    }
                                };
                                self.emit(Insn::NbaAssignRangeDyn(
                                    as_sig_id(id),
                                    hi_reg,
                                    lo_reg,
                                    resized,
                                ));
                                return true;
                            }
                        }
                    }
                }
                // §7.4.3: after an index into a multi-D packed value the
                // labels count ELEMENTS of the next dimension; the arms below
                // place bit ranges. The packed path scales them, else the
                // interpreter's store does.
                if !matches!(expr.kind, ExprKind::Ident(_)) && self.part_select_elem(expr).is_some()
                {
                    if self.try_packed_path_nba(lhs, val_reg) {
                        return true;
                    }
                    self.bail("nba_range_elem_slice");
                    return false;
                }
                if let Some(id) = self.flattened_outer_const_signal_id(expr) {
                    match kind {
                        RangeKind::Constant => {
                            if let (Some(hi), Some(lo)) =
                                (self.eval_const_bound(left), self.eval_const_bound(right))
                            {
                                // Flattened element targets: a negative label is left to the interpreter.
                                if hi < 0 || lo < 0 {
                                    self.bail("range_target_negative_bound");
                                    return false;
                                }
                                let (hi, lo) = (hi as u32, lo as u32);
                                self.emit(Insn::NbaAssignRange(as_sig_id(id), hi, lo, val_reg));
                                return true;
                            }
                        }
                        RangeKind::IndexedUp | RangeKind::IndexedDown => {
                            let width = match self.eval_const_expr(right) {
                                Some(w) if w > 0 => w,
                                _ => {
                                    self.bail("nba_range_width_nonconst");
                                    return false;
                                }
                            };
                            let resized = self.alloc_reg();
                            self.emit(Insn::Move(resized, val_reg));
                            self.emit(Insn::Resize(resized, width));
                            let Some(idx) = self.compile_expr(left, 0) else {
                                self.bail("nba_range_base");
                                return false;
                            };
                            let (hi_reg, lo_reg) = if width == 1 {
                                (idx, idx)
                            } else {
                                let delta = self.alloc_reg();
                                self.emit(Insn::LoadConst(
                                    delta,
                                    Box::new(Value::from_u64((width - 1) as u64, 32)),
                                ));
                                let other = self.alloc_reg();
                                if *kind == RangeKind::IndexedUp {
                                    self.emit(Insn::Add(other, idx, delta));
                                    (other, idx)
                                } else {
                                    self.emit(Insn::Sub(other, idx, delta));
                                    (idx, other)
                                }
                            };
                            self.emit(Insn::NbaAssignRangeDyn(
                                as_sig_id(id),
                                hi_reg,
                                lo_reg,
                                resized,
                            ));
                            return true;
                        }
                    }
                }
                if *kind == RangeKind::Constant {
                    if let Some((id, hi, lo)) = self.flattened_const_range_target(expr, left, right)
                    {
                        self.emit(Insn::NbaAssignRange(as_sig_id(id), hi, lo, val_reg));
                        return true;
                    }
                }
                // Handle mem[i][range] <= val — ALL range kinds. This arm
                // used to pass `left`/`right` straight through as (hi, lo),
                // which is only correct for the constant `[hi:lo]` form: for
                // `[base +: W]` they are (base, WIDTH), so a masked byte-lane
                // RAM write (`mem[addr][i*8 +: 8] <= ...`, the lowRISC
                // prim_ram_2p shape) wrote a 9-bit slice at the WRONG offset
                // and corrupted neighbouring lanes. Convert per §11.5.1
                // before emitting.
                if let ExprKind::Index {
                    expr: arr_expr,
                    index,
                } = &expr.kind
                {
                    if let ExprKind::Ident(hier) = &arr_expr.kind {
                        if let Some(name) = self.lookup_array_name(hier) {
                            if let Some(idx_reg) = self.compile_expr(index, 0) {
                                let regs = match kind {
                                    RangeKind::Constant => {
                                        match (
                                            self.compile_expr(left, 0),
                                            self.compile_expr(right, 0),
                                        ) {
                                            (Some(h), Some(l)) => Some((h, l)),
                                            _ => None,
                                        }
                                    }
                                    RangeKind::IndexedUp | RangeKind::IndexedDown => {
                                        let base = self.compile_expr(left, 0);
                                        let w = self
                                            .fold_const(right)
                                            .and_then(|v| v.to_u64())
                                            .filter(|&w| w > 0);
                                        match (base, w) {
                                            (Some(b), Some(w)) => {
                                                let other = self.alloc_reg();
                                                let delta = Value::from_u64(w - 1, 32);
                                                if matches!(kind, RangeKind::IndexedUp) {
                                                    // hi = base + w - 1, lo = base
                                                    self.emit(Insn::BinOpConst(
                                                        other,
                                                        b,
                                                        Box::new(delta),
                                                        BinOpConstKind::Add,
                                                    ));
                                                    Some((other, b))
                                                } else {
                                                    // hi = base, lo = base - w + 1
                                                    let dreg = self.alloc_reg();
                                                    self.emit(Insn::LoadConst(
                                                        dreg,
                                                        Box::new(delta),
                                                    ));
                                                    self.emit(Insn::Sub(other, b, dreg));
                                                    Some((b, other))
                                                }
                                            }
                                            _ => None,
                                        }
                                    }
                                };
                                if let Some((hi_reg, lo_reg)) = regs {
                                    let Some((array, idx_reg)) =
                                        self.array_operand_index(name, idx_reg)
                                    else {
                                        return false;
                                    };
                                    self.emit(Insn::NbaAssignArrayRange(
                                        array, idx_reg, hi_reg, lo_reg, val_reg,
                                    ));
                                    return true;
                                }
                            }
                        }
                    }
                }
                if self.try_packed_path_nba(lhs, val_reg) {
                    return true;
                }
                self.bail("nba_range_unresolved");
                false
            }
            ExprKind::Concatenation(parts) => {
                // {a, b, c} <= value: split value into per-part bit ranges and NBA each part.
                // Concatenation is MSB-first: parts[0] is the highest bits.
                // The RHS may be narrower than the concat width (e.g. $signed of a
                // 12-bit expression assigned to a 32-bit concat LHS). Widen first
                // so the per-part RangeSelects see properly sign/zero-extended bits.
                if width > 0 {
                    self.emit(Insn::Resize(val_reg, width));
                }
                let mut part_widths = Vec::with_capacity(parts.len());
                for p in parts {
                    let w = self.infer_lhs_width(p);
                    // A heap-field store uses width 0 to defer sizing to its
                    // receiver. Splitting a concatenation requires concrete
                    // widths; leave this assignment to the interpreter.
                    if w == 0 {
                        self.bail("concat_lhs_dynamic_width");
                        return false;
                    }
                    part_widths.push(w);
                }
                let mut bit_offset: u32 = 0;
                for (i, p) in parts.iter().enumerate().rev() {
                    let pw = part_widths[i];
                    let lo_reg = self.alloc_reg();
                    self.emit(Insn::LoadConst(
                        lo_reg,
                        Box::new(Value::from_u64(bit_offset as u64, 32)),
                    ));
                    let hi_val = bit_offset + pw - 1;
                    let hi_reg = self.alloc_reg();
                    self.emit(Insn::LoadConst(
                        hi_reg,
                        Box::new(Value::from_u64(hi_val as u64, 32)),
                    ));
                    let part_reg = self.alloc_reg();
                    self.emit(Insn::RangeSelect(part_reg, val_reg, hi_reg, lo_reg));
                    self.emit(Insn::Resize(part_reg, pw));
                    if !self.compile_nba_target(p, part_reg, pw) {
                        return false;
                    }
                    bit_offset += pw;
                }
                true
            }
            ExprKind::MemberAccess { expr, member } => {
                // `arr[i].m <= v`: splice the member's static bit range into
                // the selected ELEMENT, via the same NbaAssignArrayRange the
                // `arr[i][hi:lo]` form uses.
                if let Some((array, idx_reg, hi_reg, lo_reg, resized)) =
                    self.packed_array_member_store(expr, &member.name, val_reg)
                {
                    self.emit(Insn::NbaAssignArrayRange(
                        array, idx_reg, hi_reg, lo_reg, resized,
                    ));
                    return true;
                }
                if self.try_packed_path_nba(lhs, val_reg) {
                    return true;
                }
                // `s.e[1].dw[0].value <= v`: one static range of `s`.
                if let Some((base_id, key, member, off, mw)) =
                    self.packed_const_member_path_target(lhs)
                    && let Some(two_state) =
                        self.packed_member_store_two_state(base_id, &key, &member)
                {
                    let resized = self.alloc_reg();
                    self.emit(Insn::Move(resized, val_reg));
                    self.emit(Insn::Resize(resized, mw));
                    if two_state && !self.stored_value_two_state(resized) {
                        self.emit(Insn::DropXz(resized));
                    }
                    self.emit(Insn::NbaAssignRange(
                        as_sig_id(base_id),
                        off + mw - 1,
                        off,
                        resized,
                    ));
                    return true;
                }
                self.bail("nba_member_access");
                false
            }
            _ => {
                self.bail("nba_other");
                false
            }
        }
    }

    /// class-perf P2: a STRING member store target — bare this-relative
    /// (`m_leaf_name = name`) or a dotted handle-chain leaf
    /// (`obj.m_name = s`). Only admits names in `string_member_names`
    /// (the P1 load universe — bare-key stable, instance members); the
    /// RHS has already been checked string-valued by the caller. Unlike
    /// the integral chain store, the dotted form needs no shadow guard (see
    /// the arm below).
    pub(super) fn method_string_store_target(&mut self, lhs: &Expression, val_reg: RegId) -> bool {
        if let ExprKind::Ident(h) = &lhs.kind
            && self.method_mode
            && h.root.is_none()
            && h.path.len() == 1
            && h.path[0].selects.is_empty()
        {
            let bare = h.path[0].name.name.as_str();
            if !self.local_var_regs.contains_key(bare) && self.string_member_names.contains(bare) {
                let Some(this) = self.method_this_reg else {
                    return false;
                };
                self.emit(Insn::StoreClassMember(
                    this,
                    val_reg,
                    bare.to_string().into_boxed_str(),
                ));
                return true;
            }
            return false;
        }
        // Like the dotted string READ (Step 9a), a string member's heap slot
        // holds the text verbatim under its bare key, so `this.name = name`
        // (a formal shadowing the member) stores faithfully too.
        if let ExprKind::MemberAccess { expr: base, member } = &lhs.kind
            && self.method_mode
            && self.method_handle_chain_ok(base)
            && self.method_instance_field_ok(base, &member.name)
            && self.string_member_names.contains(member.name.as_str())
        {
            let handle_reg = match self.method_member_handle_reg(base) {
                Some(r) => r,
                None => match self.compile_expr(base, 0) {
                    Some(r) => r,
                    None => return false,
                },
            };
            self.emit(Insn::StoreClassMember(
                handle_reg,
                val_reg,
                member.name.clone().into_boxed_str(),
            ));
            return true;
        }
        false
    }

    /// `arr[i][j] = v` on a 1-D unpacked array, or `m[i][j][k] = v` on a
    /// 2-D/N-D one with constant array indices: a store into ONE BIT of one
    /// element (§7.4.2: every element is a packed vector of its own, and the
    /// trailing select addresses a bit inside it, §11.5.1). A 1-D element
    /// takes the store the `arr[i][j:j]` part-select arm emits; a
    /// constant N-D element names its own signal. `elem` is the element
    /// reference, `bit` the trailing select.
    ///
    /// Only a CONSTANT bit of a plain vector element is handled: a multi-D
    /// packed element's trailing index selects a slice, and a string's
    /// selects a character. `None` leaves every other shape to the caller.
    /// Label-mapped elements (ascending or non-zero-based) never get here:
    /// `sel_base_needs_ast` sends them to the interpreter first.
    fn compile_unpacked_elem_bit_store(
        &mut self,
        elem: &Expression,
        bit: &Expression,
        val_reg: RegId,
    ) -> Option<bool> {
        let target = self.plain_unpacked_elem(elem)?;
        let b = u32::try_from(self.eval_const_bound(bit)?).ok()?;
        match target {
            Ok(name) => {
                let ExprKind::Index {
                    index: elem_idx, ..
                } = &elem.kind
                else {
                    return None;
                };
                let Some(idx_reg) = self.compile_expr(elem_idx, 0) else {
                    return Some(false);
                };
                let b_reg = self.alloc_reg();
                self.emit(Insn::LoadConst(
                    b_reg,
                    Box::new(Value::from_u64(b as u64, 32)),
                ));
                let Some((array, idx_reg)) = self.array_operand_index(name, idx_reg) else {
                    return Some(false);
                };
                self.emit(Insn::BlockingAssignArrayRange(
                    array, idx_reg, b_reg, b_reg, val_reg,
                ));
            }
            Err(id) => {
                if b >= self.signal_widths[id] {
                    return None;
                }
                let resized = self.alloc_reg();
                self.emit(Insn::Move(resized, val_reg));
                self.emit(Insn::Resize(resized, 1));
                self.emit(Insn::BlockingAssignRange(as_sig_id(id), b, b, resized));
            }
        }
        Some(true)
    }

    /// Is `elem` one element of an unpacked array whose elements are plain
    /// vectors, so that one more index selects a single BIT (§11.5.1)?
    /// `Ok(array)` for an element of a 1-D array (any index), `Err(id)` for a
    /// 2-D/N-D element named by constant indices (its own signal). A
    /// multi-D packed element (the index selects a slice) and a string (a
    /// character) are not.
    pub(super) fn plain_unpacked_elem(&self, elem: &Expression) -> Option<Result<String, usize>> {
        let ExprKind::Index { expr: arr_expr, .. } = &elem.kind else {
            return None;
        };
        let mut root = arr_expr.as_ref();
        while let ExprKind::Index { expr, .. } = &root.kind {
            root = expr;
        }
        let ExprKind::Ident(root_hier) = &root.kind else {
            return None;
        };
        if self.is_packed_multi_dim(root_hier)
            || self
                .packed_full_dims_of(root_hier)
                .is_some_and(|d| d.len() > 1)
            || self.signal_is_string_name(root_hier)
        {
            return None;
        }
        if let ExprKind::Ident(ah) = &arr_expr.kind {
            return self.lookup_array_name(ah).map(Ok);
        }
        self.const_multi_dim_array_elem_signal_id(elem).map(Err)
    }

    /// §11.5.1/§11.5.2 bit or part-select STORE into a register-backed
    /// integral local — `v[6:4] = c[3:1]`, `cur[b*8 +: 8] = d`, `x[i] = b` —
    /// as a mask splice on the register: `x = (x & ~(m << lo)) | ((v & m) << lo)`.
    /// 4-state exact: the constant mask passes untouched bits through and
    /// lands x/z of `v` as x/z. None when `lhs` is not such a store (the
    /// caller's other paths decide); Some(false) on a compile failure.
    ///
    /// Only locals whose declared range is `[N-1:0]` qualify (`LocalKind::Int`
    /// records exactly those), so an index is a bit offset. A constant select
    /// writes exactly its in-range bits; a dynamic one needs an unsigned
    /// index, and one with an x/z bit writes nothing.
    fn compile_local_select_store(&mut self, lhs: &Expression, val_reg: RegId) -> Option<bool> {
        // (base ident, lsb expression or constant range, field width)
        enum Sel<'e> {
            Bit(&'e Expression),
            Range(RangeKind, &'e Expression, &'e Expression),
        }
        let (h, sel) = match &lhs.kind {
            ExprKind::Index { expr, index } => match &expr.kind {
                ExprKind::Ident(h) => (h, Sel::Bit(index)),
                _ => return None,
            },
            ExprKind::RangeSelect {
                expr,
                kind,
                left,
                right,
            } => match &expr.kind {
                ExprKind::Ident(h) => (h, Sel::Range(*kind, left, right)),
                _ => return None,
            },
            ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.len() == 1 => {
                (h, Sel::Bit(&h.path[0].selects[0]))
            }
            _ => return None,
        };
        if h.root.is_some() || h.path.len() != 1 {
            return None;
        }
        let name = h.path[0].name.name.as_str();
        let &(yreg, yw) = self.local_var_regs.get(name)?;
        if !matches!(self.local_kinds.get(&yreg), Some(LocalKind::Int { .. }))
            || self.local_var_elem.contains_key(name)
            || self.process_local_names.contains(name)
            || yw == 0
        {
            return None;
        }
        // (constant lsb, or dynamic lsb register) and the field width.
        let (lo_const, lo_expr, fw): (Option<i64>, Option<&Expression>, i64) = match sel {
            Sel::Bit(ix) => match self.fold_const_i64(ix) {
                Some(c) => (Some(c), None, 1),
                None => (None, Some(ix), 1),
            },
            Sel::Range(RangeKind::Constant, l, r) => {
                let (Some(l), Some(r)) = (self.fold_const_i64(l), self.fold_const_i64(r)) else {
                    return None;
                };
                if l < r {
                    // A reversed select on a descending local is illegal.
                    return None;
                }
                (Some(r), None, l - r + 1)
            }
            Sel::Range(kind, b, w) => {
                let w = self.fold_const_i64(w).filter(|&w| w > 0)?;
                match (kind, self.fold_const_i64(b)) {
                    (RangeKind::IndexedUp, Some(c)) => (Some(c), None, w),
                    (RangeKind::IndexedDown, Some(c)) => (Some(c - w + 1), None, w),
                    (RangeKind::IndexedUp, None) => (None, Some(b), w),
                    // `b -: w` at run time can reach below bit 0.
                    _ => return None,
                }
            }
        };
        if fw > 64 {
            return None;
        }
        let fw = fw as u32;
        let ones = |w: u32| -> Value {
            if w >= 64 {
                Value::from_u64(u64::MAX, 64)
            } else {
                Value::from_u64((1u64 << w) - 1, w)
            }
        };
        match (lo_const, lo_expr) {
            (Some(lo), _) => {
                // Clip the field to the local's bits: below 0 drops the
                // value's low bits, above N-1 its high bits.
                let (mut lo, mut fw, mut vshift) = (lo, fw as i64, 0i64);
                if lo < 0 {
                    vshift = -lo;
                    fw += lo;
                    lo = 0;
                }
                if fw <= 0 || lo >= yw as i64 {
                    return Some(true);
                }
                let fw = fw.min(yw as i64 - lo) as u32;
                let lo = lo as u32;
                let mut keep = Value::ones(yw);
                for b in lo..lo + fw {
                    keep.set_bit(b as usize, xezim_core::value::LogicBit::Zero);
                }
                let keep_reg = self.alloc_reg();
                self.emit(Insn::LoadConst(keep_reg, Box::new(keep)));
                let cleared = self.alloc_reg();
                self.emit(Insn::BitAnd(cleared, yreg, keep_reg));
                let vex = self.alloc_reg();
                self.emit(Insn::Move(vex, val_reg));
                if vshift > 0 {
                    let k = self.alloc_reg();
                    self.emit(Insn::LoadConst(
                        k,
                        Box::new(Value::from_u64(vshift as u64, 32)),
                    ));
                    self.emit(Insn::ClearSigned(vex));
                    self.emit(Insn::Shr(vex, vex, k));
                }
                self.emit(Insn::Resize(vex, fw));
                self.emit(Insn::ClearSigned(vex));
                self.emit(Insn::Resize(vex, yw));
                let merged = self.alloc_reg();
                if lo > 0 {
                    let k = self.alloc_reg();
                    self.emit(Insn::LoadConst(k, Box::new(Value::from_u64(lo as u64, 32))));
                    let vsh = self.alloc_reg();
                    self.emit(Insn::Shl(vsh, vex, k));
                    self.emit(Insn::BitOr(merged, cleared, vsh));
                } else {
                    self.emit(Insn::BitOr(merged, cleared, vex));
                }
                self.emit(Insn::Move(yreg, merged));
                self.emit(Insn::Resize(yreg, yw));
                self.emit_local_fit(yreg);
                Some(true)
            }
            (None, Some(ix)) => {
                let signed = self.expr_signedness(ix)?;
                // A negative part-select base may still overlap the local.
                if signed && fw != 1 {
                    return None;
                }
                let Some(lo_reg) = self.compile_expr(ix, 0) else {
                    return Some(false);
                };
                // §11.5.1: an unknown or out-of-range bit index writes
                // nothing. Test the full signed value before shifting. A
                // narrow -1 can otherwise address a valid positive bit, and
                // a wide positive index can wrap when converted to a shift.
                let known = self.alloc_reg();
                if signed {
                    let zero = self.alloc_reg();
                    let limit = self.alloc_reg();
                    let mut zero_val = Value::from_u64(0, 64);
                    let mut limit_val = Value::from_u64(u64::from(yw), 64);
                    zero_val.is_signed = true;
                    limit_val.is_signed = true;
                    self.emit(Insn::LoadConst(zero, Box::new(zero_val)));
                    self.emit(Insn::LoadConst(limit, Box::new(limit_val)));
                    let below_end = self.alloc_reg();
                    self.emit(Insn::Geq(known, lo_reg, zero));
                    self.emit(Insn::Lt(below_end, lo_reg, limit));
                    self.emit(Insn::BitAnd(known, known, below_end));
                } else {
                    self.emit(Insn::Eq(known, lo_reg, lo_reg));
                }
                let skip = self.insns.len();
                self.emit(Insn::BranchIfFalse(known, 0));
                let mask_reg = self.alloc_reg();
                self.emit(Insn::LoadConst(mask_reg, Box::new(ones(fw).resize(yw))));
                let shifted = self.alloc_reg();
                self.emit(Insn::Shl(shifted, mask_reg, lo_reg));
                let inv = self.alloc_reg();
                self.emit(Insn::BitNot(inv, shifted));
                let cleared = self.alloc_reg();
                self.emit(Insn::BitAnd(cleared, yreg, inv));
                let vex = self.alloc_reg();
                self.emit(Insn::Move(vex, val_reg));
                self.emit(Insn::Resize(vex, fw));
                self.emit(Insn::ClearSigned(vex));
                self.emit(Insn::Resize(vex, yw));
                let vsh = self.alloc_reg();
                self.emit(Insn::Shl(vsh, vex, lo_reg));
                let merged = self.alloc_reg();
                self.emit(Insn::BitOr(merged, cleared, vsh));
                self.emit(Insn::Move(yreg, merged));
                self.emit(Insn::Resize(yreg, yw));
                self.emit_local_fit(yreg);
                let end = self.insns.len() as u32;
                self.insns[skip] = Insn::BranchIfFalse(known, end);
                Some(true)
            }
            (None, None) => None,
        }
    }

    /// Whether `e` selects, through two or more indices, an array member
    /// reached through a select (`lane[0].nest[1][2]`).
    pub(super) fn inst_array_multidim_elem(&self, e: &Expression) -> bool {
        let ExprKind::Index { expr: inner, .. } = &e.kind else {
            return false;
        };
        if !matches!(inner.kind, ExprKind::Index { .. })
            || !crate::compiler::simulator::Simulator::select_entered_member_shape(inner)
        {
            return false;
        }
        // An element of a variable (`sa[1].f0[3][7]` of an array of
        // structures) is not a scope; it compiles as before.
        let Some(root) = crate::compiler::simulator::Simulator::expr_root_first_seg(inner) else {
            return false;
        };
        !self.arrays.contains_key(root)
            && !self.scope_hint.as_ref().is_some_and(|scope| {
                self.arrays
                    .contains_key(format!("{}.{}", scope, root).as_str())
            })
    }

    pub(super) fn compile_blocking_target(
        &mut self,
        lhs: &Expression,
        val_reg: RegId,
        width: u32,
    ) -> bool {
        if self.inst_array_multidim_elem(lhs) {
            self.bail("inst_array_member");
            return false;
        }
        // class-perf method-mode: `this.<member> = e` / `<classlocal>.<member> = e`
        // is an OBJECT-field store off the heap.
        if self.method_mode
            && let ExprKind::MemberAccess { expr: base, member } = &lhs.kind
            && self.method_handle_chain_ok(base)
        {
            if !self.method_instance_field_ok(base, &member.name) {
                self.bail("method_receiver_field");
                return false;
            }
            // Step 6: the base may be a chain (`a.b.c = v`) or a bare member
            // (`m.f = v` stores through this.m, loaded here) — lower it to
            // its handle register first.
            let handle_reg = match self.method_member_handle_reg(base) {
                Some(r) => r,
                None => match self.compile_expr(base, 0) {
                    Some(r) => r,
                    None => return false,
                },
            };
            // Same guard as the rvalue path: the dotted STORE also uses the
            // runtime bare key, so extends-chain instance properties are
            // faithful (`member_safe_names`); statics, enclosing-class
            // members, and non-integral types keep the AST store semantics.
            if self.class_shadow_names.contains(member.name.as_str())
                && !self.member_safe_names.contains(member.name.as_str())
            {
                self.bail("method_member_noload");
                return false;
            }
            self.emit(Insn::StoreClassMember(
                handle_reg,
                val_reg,
                member.name.clone().into_boxed_str(),
            ));
            return true;
        }
        // Packed element WRITE on a register-backed local (`y[i] = v` on a
        // `u8_vec16_t y` inside an inlined function): mask-splice with plain
        // ALU insns — y = (y & ~(elem_mask << i*ew)) | ((v & elem_mask) << i*ew).
        // 4-state correct via the per-bit AND/OR plane tables (constant mask
        // bits are known, so untouched bits pass through and X in `v` lands
        // as X). An out-of-range index shifts the mask past the register's
        // width and the write becomes a no-op. Assumes [N-1:0] outer bounds
        // (the packed-of-packed typedef shape); other locals have no elem
        // entry and keep the bail-to-AST behavior.
        // Element WRITE of a register-bound LOCAL ARRAY (`row[n] = v`,
        // either parse shape).
        if !self.local_var_array.is_empty() {
            let (aname, idx): (Option<&str>, Option<&Expression>) = match &lhs.kind {
                ExprKind::Index { expr, index } => match &expr.kind {
                    ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                        (Some(h.path[0].name.name.as_str()), Some(index))
                    }
                    _ => (None, None),
                },
                ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.len() == 1 => (
                    Some(h.path[0].name.name.as_str()),
                    Some(&h.path[0].selects[0]),
                ),
                _ => (None, None),
            };
            if let (Some(n), Some(ix)) = (aname, idx) {
                if self.local_var_array.contains_key(n) {
                    let n = n.to_string();
                    let ix = ix.clone();
                    return match self.compile_local_array_write(&n, &ix, val_reg) {
                        Some(ok) => ok,
                        None => {
                            self.bail("local_array_write");
                            false
                        }
                    };
                }
            }
        }
        // §7.4.1/§11.5.1: ascending or element-of-collection select bases
        // need label mapping the stores below do not emit — AST only.
        if let ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } = &lhs.kind {
            if self.sel_base_needs_ast(expr) {
                self.bail("blocking_sel_base_maps");
                return false;
            }
        }
        if let ExprKind::Index { expr, index } = &lhs.kind {
            if let ExprKind::Ident(h) = &expr.kind {
                let raw = Self::hier_raw_name(h);
                if let Some(&ew) = self.local_var_elem.get(&raw).filter(|&&ew| ew <= 64) {
                    if let Some((yreg, yw)) = self.local_var_reg_of(h) {
                        if yw > 0 {
                            let Some(idx_reg) = self.compile_expr(index, 0) else {
                                self.bail("local_elem_idx");
                                return false;
                            };
                            let ew_reg = self.alloc_reg();
                            self.emit(Insn::LoadConst(
                                ew_reg,
                                Box::new(Value::from_u64(ew as u64, 32)),
                            ));
                            let lo_reg = self.alloc_reg();
                            self.emit(Insn::Mul(lo_reg, idx_reg, ew_reg));
                            self.emit(Insn::Resize(lo_reg, 32));
                            // elem mask at the container's width
                            let mask_reg = self.alloc_reg();
                            let mut mv = if ew >= 64 {
                                Value::from_u64(u64::MAX, 64)
                            } else {
                                Value::from_u64((1u64 << ew) - 1, ew)
                            };
                            mv = mv.resize(yw);
                            self.emit(Insn::LoadConst(mask_reg, Box::new(mv)));
                            let shifted_mask = self.alloc_reg();
                            self.emit(Insn::Shl(shifted_mask, mask_reg, lo_reg));
                            let inv = self.alloc_reg();
                            self.emit(Insn::BitNot(inv, shifted_mask));
                            let cleared = self.alloc_reg();
                            self.emit(Insn::BitAnd(cleared, yreg, inv));
                            // value: mask to elem width, widen, shift into place
                            let vex = self.alloc_reg();
                            self.emit(Insn::Move(vex, val_reg));
                            self.emit(Insn::Resize(vex, ew));
                            self.emit(Insn::Resize(vex, yw));
                            let vsh = self.alloc_reg();
                            self.emit(Insn::Shl(vsh, vex, lo_reg));
                            let merged = self.alloc_reg();
                            self.emit(Insn::BitOr(merged, cleared, vsh));
                            self.emit(Insn::Move(yreg, merged));
                            self.emit(Insn::Resize(yreg, yw));
                            return true;
                        }
                    }
                }
            }
        }
        // Register-bank local array element (constant index only).
        if let ExprKind::Index { expr, index } = &lhs.kind {
            if let ExprKind::Ident(h) = &expr.kind {
                if h.root.is_none() && h.path.len() == 1 && h.path[0].selects.is_empty() {
                    if let Some(&LocalArrayBank {
                        base,
                        width: ew,
                        len,
                        lo,
                        ..
                    }) = self.local_array_regs.get(&h.path[0].name.name)
                    {
                        let Some(iv) = self.fold_const(index).and_then(|v| v.to_u64()) else {
                            self.bail("local_array_dyn_index");
                            return false;
                        };
                        let slot = (iv as i64) - lo;
                        if slot < 0 || slot as usize >= len {
                            self.bail("local_array_oob");
                            return false;
                        }
                        let dst = base + slot as RegId;
                        self.emit(Insn::Move(dst, val_reg));
                        self.emit(Insn::Resize(dst, ew));
                        return true;
                    }
                }
            }
        }
        // Bit / part-select store into a register-backed integral local.
        if let Some(done) = self.compile_local_select_store(lhs, val_reg) {
            return done;
        }
        // Assignment to a register-backed block local (the loop variable of an
        // enclosing `for (int i = ...)`).
        if let ExprKind::Ident(hier) = &lhs.kind {
            if let Some((dst, w)) = self.local_var_reg_of(hier) {
                let name = &hier.path[0].name.name;
                if self.process_local_names.contains(name) {
                    self.bail("blocking_process_local");
                    return false;
                }
                self.emit(Insn::Move(dst, val_reg));
                if self.local_var_is_real.contains(name.as_str()) {
                    // §13.3.1: a real local/formal/result converts its RHS
                    // numerically; a Resize would bit-truncate instead.
                    self.emit_to_real(dst);
                } else if w > 0 {
                    self.emit(Insn::Resize(dst, w));
                    self.emit_local_fit(dst);
                }
                return true;
            }
        }
        if let ExprKind::MemberAccess { expr, member } = &lhs.kind {
            // §13.4.1 `fname.member = …`: a write to a member of the
            // function's own RETURN VARIABLE, which an inlined body holds
            // in a register rather than a signal — so neither the signal
            // splice above nor the array path applies. Mask-splice the
            // member's static bit range into that register:
            //   r = (r & ~(mask << off)) | ((v & mask) << off)
            // The layout is registered per function at compile start
            // ("fn ret:<name>"), since the return variable is not a signal
            // and nothing else records one for it.
            if let ExprKind::Ident(bh) = &expr.kind
                && let Some((slot, rw)) = self.local_var_reg_of(bh)
                && let Some(layout) = self
                    .packed_struct_layouts
                    .and_then(|m| m.get(format!("fn ret:{}", Self::hier_raw_name(bh)).as_str()))
                && let Some(f) = layout.get(&member.name)
                && f.width > 0
                && rw > 0
            {
                let (off, mw) = (f.offset, f.width);
                let fields_w = rw;
                // value & mask, widened, shifted into place
                let vex = self.alloc_reg();
                self.emit(Insn::Move(vex, val_reg));
                self.emit(Insn::Resize(vex, mw));
                self.emit(Insn::Resize(vex, fields_w));
                let sh = self.alloc_reg();
                self.emit(Insn::LoadConst(
                    sh,
                    Box::new(Value::from_u64(off as u64, 32)),
                ));
                let vsh = self.alloc_reg();
                self.emit(Insn::Shl(vsh, vex, sh));
                // clear the destination window
                let mut mask = Value::from_u64(0, fields_w);
                for b in 0..mw {
                    mask.set_bit((off + b) as usize, xezim_core::value::LogicBit::One);
                }
                let mreg = self.alloc_reg();
                self.emit(Insn::LoadConst(mreg, Box::new(mask)));
                let inv = self.alloc_reg();
                self.emit(Insn::BitNot(inv, mreg));
                let cleared = self.alloc_reg();
                self.emit(Insn::BitAnd(cleared, slot, inv));
                let merged = self.alloc_reg();
                self.emit(Insn::BitOr(merged, cleared, vsh));
                self.emit(Insn::Move(slot, merged));
                self.emit(Insn::Resize(slot, fields_w));
                return true;
            }
        }
        if self.frame_bound_target(lhs)
            && !matches!(&lhs.kind, ExprKind::Ident(h) if h.path[0].selects.is_empty())
        {
            self.bail("blocking_frame_target");
            return false;
        }
        match &lhs.kind {
            // Handle `base.field` for unpacked struct member signals.
            // e.g. `a.field1 = Tsum(...).field1;` where `a.field1` is a separate signal.
            ExprKind::MemberAccess { expr, member } => {
                if let ExprKind::Ident(hier) = &expr.kind {
                    if hier.path.len() == 1 {
                        let base_name = hier.path[0].name.name.as_str();
                        let dotted = format!("{}.{}", base_name, member.name);
                        if let Some(id) = self.lookup_signal_id_by_name(&dotted) {
                            let w = if width == 0 {
                                self.signal_widths.get(id).copied().unwrap_or(0)
                            } else {
                                width
                            };
                            self.emit(Insn::BlockingAssign(as_sig_id(id), val_reg, w));
                            return true;
                        }
                    }
                }
                // `arr[i].m = v` — same operands as the NBA arm.
                if let Some((array, idx_reg, hi_reg, lo_reg, resized)) =
                    self.packed_array_member_store(expr, &member.name, val_reg)
                {
                    self.emit(Insn::BlockingAssignArrayRange(
                        array, idx_reg, hi_reg, lo_reg, resized,
                    ));
                    return true;
                }
                // `s.e[1].dw[0].value = v`: one static range of `s`.
                if let Some((base_id, key, member, off, mw)) =
                    self.packed_const_member_path_target(lhs)
                    && let Some(two_state) =
                        self.packed_member_store_two_state(base_id, &key, &member)
                {
                    let resized = self.alloc_reg();
                    self.emit(Insn::Move(resized, val_reg));
                    self.emit(Insn::Resize(resized, mw));
                    if two_state && !self.stored_value_two_state(resized) {
                        self.emit(Insn::DropXz(resized));
                    }
                    self.emit(Insn::BlockingAssignRange(
                        as_sig_id(base_id),
                        off + mw - 1,
                        off,
                        resized,
                    ));
                    return true;
                }
                self.bail("blocking_target_member_access");
                false
            }
            ExprKind::Ident(hier) => {
                // Step 9e: in method mode, a BARE member store (`cnt = v`)
                // resolves to class scope first — exactly like the bare
                // READ path (§8.23: the instance member shadows a
                // same-named module signal). `bare_member_names` is
                // instance-only and bare-key-stable by construction;
                // STRING member stores stay AST (Step 9a's width-hazard
                // note — the string set is disjoint from the bare set).
                if self.method_mode
                    && hier.root.is_none()
                    && hier.path.len() == 1
                    && hier.path[0].selects.is_empty()
                {
                    let bare = hier.path[0].name.name.as_str();
                    if !self.local_var_regs.contains_key(bare)
                        && self.bare_member_names.contains(bare)
                    {
                        let Some(this) = self.method_this_reg else {
                            self.bail("blocking_target");
                            return false;
                        };
                        self.emit(Insn::StoreClassMember(
                            this,
                            val_reg,
                            bare.to_string().into_boxed_str(),
                        ));
                        return true;
                    }
                    // Step 9g: a bare STATIC-property store —
                    // class_static_set semantics (Value unresized). Class
                    // localparams (class_const_names) stay AST — a store
                    // to a constant is illegal SV.
                    if !self.local_var_regs.contains_key(bare)
                        && self.static_member_names.contains(bare)
                    {
                        self.emit(Insn::StoreClassStatic(
                            Box::new(NamePair(
                                self.method_class
                                    .clone()
                                    .unwrap_or_default()
                                    .into_boxed_str(),
                                bare.to_string().into_boxed_str(),
                            )),
                            val_reg,
                        ));
                        return true;
                    }
                    // Any other class-scope name (a collection member,
                    // static or not) must not fall through to a same-named
                    // module signal, as on the bare read path.
                    if !self.local_var_regs.contains_key(bare)
                        && self.class_shadow_names.contains(bare)
                    {
                        self.bail("method_class_shadow");
                        return false;
                    }
                }
                if let Some(id) = self.lookup_signal_id(hier) {
                    if self.signal_is_string_name(hier) {
                        self.emit(Insn::BlockingAssignString(as_sig_id(id), val_reg));
                    } else {
                        // Step 9e: a class-HANDLE store (width 0 — the plan
                        // widths of class formals/locals) must keep the
                        // runtime's tagged handle bits intact. Width 0 would
                        // fail the executor's raw-bit fast path and `resize`
                        // the handle into an untagged integer (uvm_printer::
                        // set_default stored a number where the AST read a
                        // handle — 7515_printer). Store at the SIGNAL's
                        // declared width: the fast path copies raw bits,
                        // preserving handle identity exactly like the AST.
                        let w = if width == 0 {
                            self.signal_widths.get(id).copied().unwrap_or(0)
                        } else {
                            width
                        };
                        self.emit(Insn::BlockingAssign(as_sig_id(id), val_reg, w));
                    }
                    true
                } else if let Some((base_id, off, mw)) = self.packed_struct_member_target(hier) {
                    // Packed-struct member write (`s.m0 = …`): splice the value
                    // into `[off + mw - 1 : off]` of the container signal.
                    let resized = self.alloc_reg();
                    self.emit(Insn::Move(resized, val_reg));
                    self.emit(Insn::Resize(resized, mw));
                    self.emit(Insn::BlockingAssignRange(
                        as_sig_id(base_id),
                        off + mw - 1,
                        off,
                        resized,
                    ));
                    true
                } else {
                    self.bail("blocking_target");
                    false
                }
            }
            ExprKind::Index { expr, index } => {
                if let Some(id) = self.const_multi_dim_array_elem_signal_id(lhs) {
                    self.emit(Insn::BlockingAssign(as_sig_id(id), val_reg, width));
                    return true;
                }
                if let ExprKind::Ident(hier) = &expr.kind {
                    if self.is_assoc_target(hier) {
                        self.bail("blocking_target_assoc");
                        return false;
                    }
                    if self.collection_store_denied(hier) {
                        self.bail("blocking_target_collection");
                        return false;
                    }
                    if let Some(name) = self.lookup_array_name(hier) {
                        if let Some(idx_reg) = self.compile_expr(index, 0) {
                            let Some((array, idx_reg)) = self.array_operand_index(name, idx_reg)
                            else {
                                return false;
                            };
                            self.emit(Insn::BlockingAssignArray(array, idx_reg, val_reg, width));
                            return true;
                        }
                    }
                    if let Some(id) = self.lookup_signal_id(hier) {
                        // §6.16: a character-indexed write into a STRING
                        // signal (`s[2] = "r"`) is a string splice, not a bit
                        // write, but `BlockingAssignBitDyn` below would
                        // bit-bash the byte-vector container. Defer the whole
                        // target to the AST interpreter, which routes it
                        // through the string-char arm of `assign_value_index`.
                        if self.signal_is_string_name(hier) {
                            self.bail("blocking_target_string_char");
                            return false;
                        }
                        // Packed multi-D LHS: `mem_n[i] = data_i` for
                        // `logic [N-1:0][W-1:0] mem_n` must write a W-bit
                        // slice at `i*W +: W`, not a single bit. Emit a
                        // RangeDyn write of `(i*W+W-1):(i*W)` instead.
                        let elem_w = self.packed_elem_width_of(hier);
                        if let Some(elem_w) = elem_w {
                            if let Some(idx_reg) = self.compile_expr(index, 0) {
                                // lo = slot * elem_w, where `slot` normalizes
                                // the index against the DECLARED outer range.
                                let dim = self.packed_outer_dim(hier);
                                let idx_reg = self.emit_packed_slot_index(dim, idx_reg);
                                let elem_w_reg = self.alloc_reg();
                                self.emit(Insn::LoadConst(
                                    elem_w_reg,
                                    Box::new(Value::from_u64(elem_w as u64, 32)),
                                ));
                                let lo_reg = self.alloc_reg();
                                self.emit(Insn::Mul(lo_reg, idx_reg, elem_w_reg));
                                // hi = lo + elem_w - 1
                                let em1_reg = self.alloc_reg();
                                self.emit(Insn::LoadConst(
                                    em1_reg,
                                    Box::new(Value::from_u64((elem_w - 1) as u64, 32)),
                                ));
                                let hi_reg = self.alloc_reg();
                                self.emit(Insn::Add(hi_reg, lo_reg, em1_reg));
                                self.emit(Insn::BlockingAssignRangeDyn(
                                    as_sig_id(id),
                                    hi_reg,
                                    lo_reg,
                                    val_reg,
                                ));
                                return true;
                            }
                        }
                        if let Some(idx_reg) = self.compile_expr(index, 0) {
                            // §7.4.1: rebase a declared bit index to a
                            // physical offset on a non-zero-based vector
                            // (`logic [3:1] w; w[3] = …` writes offset 2).
                            let base_lo = self.declared_low_bound(hier);
                            let idx_reg = if base_lo != 0 {
                                let base_reg = self.alloc_reg();
                                self.emit(Insn::LoadConst(
                                    base_reg,
                                    Box::new(Value::from_u64(base_lo as u64, 32)),
                                ));
                                let adj = self.alloc_reg();
                                self.emit(Insn::Sub(adj, idx_reg, base_reg));
                                adj
                            } else {
                                idx_reg
                            };
                            self.emit(Insn::BlockingAssignBitDyn(as_sig_id(id), idx_reg, val_reg));
                            return true;
                        }
                    }
                }
                if let Some(id) = self.flattened_outer_const_signal_id(expr) {
                    if let Some(idx_reg) = self.compile_expr(index, 0) {
                        self.emit(Insn::BlockingAssignBitDyn(as_sig_id(id), idx_reg, val_reg));
                        return true;
                    }
                }
                if let Some(done) = self.compile_unpacked_elem_bit_store(expr, index, val_reg) {
                    return done;
                }
                if self.try_packed_path_blocking(lhs, val_reg) {
                    return true;
                }
                self.bail("blocking_target");
                false
            }
            ExprKind::RangeSelect {
                expr,
                left,
                right,
                kind,
            } => {
                if let ExprKind::Ident(hier) = &expr.kind {
                    // §7.4.1: a range select on a multi-D PACKED vector
                    // (`logic [1:16][7:0] s; s[1:8] = …`) selects ELEMENTS,
                    // not bits — bail to the interpreter's element-aware path.
                    if self.packed_elem_width_of(hier).is_some() {
                        self.bail("blocking_range_packed_multid");
                        return false;
                    }
                    if let Some(id) = self.lookup_signal_id(hier) {
                        // §7.4.1: declared indices on a non-zero-based vector
                        // (`logic [3:1] w; assign w[2:1] = …`) must be rebased
                        // to physical offsets — the read path already does
                        // this; the write path never did, so the write landed
                        // one position high and the low bit stayed x.
                        let base_lo = self.declared_low_bound(hier);
                        match kind {
                            RangeKind::Constant => {
                                if let (Some(hi), Some(lo)) =
                                    (self.eval_const_bound(left), self.eval_const_bound(right))
                                {
                                    // §11.5.1: see the NBA arm — labels below
                                    // the low bound are dropped, the value is
                                    // shifted to its in-range bits.
                                    let (phys_hi, phys_lo) =
                                        (hi as i64 - base_lo, lo as i64 - base_lo);
                                    let (phys_hi, phys_lo) =
                                        (phys_hi.max(phys_lo), phys_hi.min(phys_lo));
                                    if phys_hi < 0
                                        || (phys_lo < 0
                                            && crate::compiler::simulator::oob_select_whole())
                                    {
                                        return true;
                                    }
                                    let val_reg = if phys_lo < 0 {
                                        let shifted = self.alloc_reg();
                                        let amt = self.alloc_reg();
                                        self.emit(Insn::LoadConst(
                                            amt,
                                            Box::new(Value::from_u64((-phys_lo) as u64, 32)),
                                        ));
                                        self.emit(Insn::Shr(shifted, val_reg, amt));
                                        shifted
                                    } else {
                                        val_reg
                                    };
                                    let (hi, lo) = (phys_hi as u32, phys_lo.max(0) as u32);
                                    let (low, high) = if hi >= lo { (lo, hi) } else { (hi, lo) };
                                    if let Some(range_w) =
                                        high.checked_sub(low).and_then(|w| w.checked_add(1))
                                    {
                                        let resized = self.alloc_reg();
                                        self.emit(Insn::Move(resized, val_reg));
                                        self.emit(Insn::Resize(resized, range_w));
                                        self.emit(Insn::BlockingAssignRange(
                                            as_sig_id(id),
                                            hi,
                                            lo,
                                            resized,
                                        ));
                                        return true;
                                    }
                                }
                                if let (Some(hi_reg), Some(lo_reg)) =
                                    (self.compile_expr(left, 0), self.compile_expr(right, 0))
                                {
                                    self.emit(Insn::BlockingAssignRangeDyn(
                                        as_sig_id(id),
                                        hi_reg,
                                        lo_reg,
                                        val_reg,
                                    ));
                                    return true;
                                }
                            }
                            RangeKind::IndexedUp | RangeKind::IndexedDown => {
                                let width = match self.eval_const_expr(right) {
                                    Some(w) if w > 0 => w,
                                    _ => {
                                        self.bail("blocking_range_width_nonconst");
                                        return false;
                                    }
                                };
                                let resized = self.alloc_reg();
                                self.emit(Insn::Move(resized, val_reg));
                                self.emit(Insn::Resize(resized, width));
                                // Static bounds for a constant base index (see
                                // the read arm): a ranged store instead of a
                                // register-computed one.
                                if self.packed_elem_width_of(hier).filter(|&w| w > 1).is_none() {
                                    if let Some(c) = self.fold_const(left).and_then(|v| v.to_i64())
                                    {
                                        let (mut l, mut r) = if *kind == RangeKind::IndexedUp {
                                            (c + width as i64 - 1, c)
                                        } else {
                                            (c, c - width as i64 + 1)
                                        };
                                        if let Some((dl, dr)) = self.packed_outer_dim(hier) {
                                            let lo_b = dl.min(dr);
                                            if lo_b != 0 {
                                                l -= lo_b;
                                                r -= lo_b;
                                            }
                                        }
                                        if r >= 0 {
                                            self.emit(Insn::BlockingAssignRange(
                                                as_sig_id(id),
                                                l as u32,
                                                r as u32,
                                                resized,
                                            ));
                                            return true;
                                        }
                                    }
                                }
                                let Some(idx) = self.compile_expr(left, 0) else {
                                    self.bail("blocking_range_base");
                                    return false;
                                };
                                // §7.4.1: rebase for non-zero-based vectors.
                                let idx = self.emit_rebased_index(hier, idx);
                                let (hi_reg, lo_reg) = if width == 1 {
                                    (idx, idx)
                                } else {
                                    let delta = self.alloc_reg();
                                    self.emit(Insn::LoadConst(
                                        delta,
                                        Box::new(Value::from_u64((width - 1) as u64, 32)),
                                    ));
                                    let other = self.alloc_reg();
                                    if *kind == RangeKind::IndexedUp {
                                        self.emit(Insn::Add(other, idx, delta));
                                        (other, idx)
                                    } else {
                                        self.emit(Insn::Sub(other, idx, delta));
                                        (idx, other)
                                    }
                                };
                                self.emit(Insn::BlockingAssignRangeDyn(
                                    as_sig_id(id),
                                    hi_reg,
                                    lo_reg,
                                    resized,
                                ));
                                return true;
                            }
                        }
                    }
                }
                // §7.4.3: after an index into a multi-D packed value the
                // labels count ELEMENTS of the next dimension; the arms below
                // place bit ranges (`assign a[0][5:0] = b` on
                // `logic [3:0][5:0][7:0] a` wrote bits 5..0 of `a[0]`). The
                // packed path scales them, else the interpreter's store does.
                if !matches!(expr.kind, ExprKind::Ident(_)) && self.part_select_elem(expr).is_some()
                {
                    if self.try_packed_path_blocking(lhs, val_reg) {
                        return true;
                    }
                    self.bail("blocking_range_elem_slice");
                    return false;
                }
                if let Some(id) = self.flattened_outer_const_signal_id(expr) {
                    match kind {
                        RangeKind::Constant => {
                            if let (Some(hi), Some(lo)) =
                                (self.eval_const_bound(left), self.eval_const_bound(right))
                            {
                                // Flattened element targets: a negative label is left to the interpreter.
                                if hi < 0 || lo < 0 {
                                    self.bail("range_target_negative_bound");
                                    return false;
                                }
                                let (hi, lo) = (hi as u32, lo as u32);
                                let (low, high) = if hi >= lo { (lo, hi) } else { (hi, lo) };
                                if let Some(range_w) =
                                    high.checked_sub(low).and_then(|w| w.checked_add(1))
                                {
                                    let resized = self.alloc_reg();
                                    self.emit(Insn::Move(resized, val_reg));
                                    self.emit(Insn::Resize(resized, range_w));
                                    self.emit(Insn::BlockingAssignRange(
                                        as_sig_id(id),
                                        hi,
                                        lo,
                                        resized,
                                    ));
                                    return true;
                                }
                            }
                            if let (Some(hi_reg), Some(lo_reg)) =
                                (self.compile_expr(left, 0), self.compile_expr(right, 0))
                            {
                                self.emit(Insn::BlockingAssignRangeDyn(
                                    as_sig_id(id),
                                    hi_reg,
                                    lo_reg,
                                    val_reg,
                                ));
                                return true;
                            }
                        }
                        RangeKind::IndexedUp | RangeKind::IndexedDown => {
                            let width = match self.eval_const_expr(right) {
                                Some(w) if w > 0 => w,
                                _ => {
                                    self.bail("blocking_range_width_nonconst");
                                    return false;
                                }
                            };
                            let resized = self.alloc_reg();
                            self.emit(Insn::Move(resized, val_reg));
                            self.emit(Insn::Resize(resized, width));
                            let Some(idx) = self.compile_expr(left, 0) else {
                                self.bail("blocking_range_base");
                                return false;
                            };
                            let (hi_reg, lo_reg) = if width == 1 {
                                (idx, idx)
                            } else {
                                let delta = self.alloc_reg();
                                self.emit(Insn::LoadConst(
                                    delta,
                                    Box::new(Value::from_u64((width - 1) as u64, 32)),
                                ));
                                let other = self.alloc_reg();
                                if *kind == RangeKind::IndexedUp {
                                    self.emit(Insn::Add(other, idx, delta));
                                    (other, idx)
                                } else {
                                    self.emit(Insn::Sub(other, idx, delta));
                                    (idx, other)
                                }
                            };
                            self.emit(Insn::BlockingAssignRangeDyn(
                                as_sig_id(id),
                                hi_reg,
                                lo_reg,
                                resized,
                            ));
                            return true;
                        }
                    }
                }
                if *kind == RangeKind::Constant {
                    if let Some((id, hi, lo)) = self.flattened_const_range_target(expr, left, right)
                    {
                        let range_w = hi - lo + 1;
                        let resized = self.alloc_reg();
                        self.emit(Insn::Move(resized, val_reg));
                        self.emit(Insn::Resize(resized, range_w));
                        self.emit(Insn::BlockingAssignRange(as_sig_id(id), hi, lo, resized));
                        return true;
                    }
                }
                // `m[i][j][hi:lo] = v`: a part of one constant element of a
                // 2-D/N-D unpacked array, which is a signal of its own.
                if let Some(Err(id)) = self.plain_unpacked_elem(expr) {
                    let bounds = match kind {
                        RangeKind::Constant => self
                            .eval_const_bound(left)
                            .zip(self.eval_const_bound(right))
                            .filter(|(l, r)| l >= r),
                        RangeKind::IndexedUp | RangeKind::IndexedDown => {
                            let w = self.eval_const_expr(right).filter(|&w| w > 0);
                            self.eval_const_bound(left).zip(w).map(|(b, w)| {
                                if *kind == RangeKind::IndexedUp {
                                    (b + w as i64 - 1, b)
                                } else {
                                    (b, b - w as i64 + 1)
                                }
                            })
                        }
                    };
                    if let Some((hi, lo)) =
                        bounds.filter(|&(hi, lo)| lo >= 0 && hi < self.signal_widths[id] as i64)
                    {
                        let (hi, lo) = (hi as u32, lo as u32);
                        let resized = self.alloc_reg();
                        self.emit(Insn::Move(resized, val_reg));
                        self.emit(Insn::Resize(resized, hi - lo + 1));
                        self.emit(Insn::BlockingAssignRange(as_sig_id(id), hi, lo, resized));
                        return true;
                    }
                }
                // Handle mem[i][hi:lo] = val
                if let ExprKind::Index {
                    expr: arr_expr,
                    index,
                } = &expr.kind
                {
                    if let ExprKind::Ident(hier) = &arr_expr.kind {
                        if let Some(name) = self.lookup_array_name(hier) {
                            if let Some(idx_reg) = self.compile_expr(index, 0) {
                                // §11.5.1: in `[base +: w]` / `[base -: w]` the
                                // RIGHT operand is a WIDTH, not an index. This
                                // arm emitted it as `lo` regardless of `kind`,
                                // so `arr[n][64 +: 32]` became the 33-bit window
                                // `[64:32]`: the payload landed 32 bits low and
                                // the top bit was clipped. The flat-signal arm
                                // above and the NBA array arm both convert —
                                // only this one did not, which wedged the C910
                                // PLIC's prefix-OR chain (plic_hreg_busif.v
                                // builds `mie_lst_read_tmp[n][32*(m+1)+:32]`
                                // from the previous slice, so every stage read
                                // back x/z and the settle never converged).
                                let regs = if *kind == RangeKind::Constant {
                                    match (self.compile_expr(left, 0), self.compile_expr(right, 0))
                                    {
                                        (Some(h), Some(l)) => Some((h, l)),
                                        _ => None,
                                    }
                                } else {
                                    match self.eval_const_expr(right) {
                                        Some(width) if width > 0 => {
                                            match self.compile_expr(left, 0) {
                                                Some(base) if width == 1 => Some((base, base)),
                                                Some(base) => {
                                                    let delta = self.alloc_reg();
                                                    self.emit(Insn::LoadConst(
                                                        delta,
                                                        Box::new(Value::from_u64(
                                                            (width - 1) as u64,
                                                            32,
                                                        )),
                                                    ));
                                                    let other = self.alloc_reg();
                                                    if *kind == RangeKind::IndexedUp {
                                                        // lo = base, hi = base + w - 1
                                                        self.emit(Insn::Add(other, base, delta));
                                                        Some((other, base))
                                                    } else {
                                                        // hi = base, lo = base - w + 1
                                                        self.emit(Insn::Sub(other, base, delta));
                                                        Some((base, other))
                                                    }
                                                }
                                                None => None,
                                            }
                                        }
                                        _ => None,
                                    }
                                };
                                if let Some((hi_reg, lo_reg)) = regs {
                                    let Some((array, idx_reg)) =
                                        self.array_operand_index(name, idx_reg)
                                    else {
                                        return false;
                                    };
                                    self.emit(Insn::BlockingAssignArrayRange(
                                        array, idx_reg, hi_reg, lo_reg, val_reg,
                                    ));
                                    return true;
                                }
                            }
                        }
                    }
                }
                if self.try_packed_path_blocking(lhs, val_reg) {
                    return true;
                }
                self.bail("blocking_target");
                false
            }
            ExprKind::Concatenation(parts) => {
                let mut part_widths = Vec::with_capacity(parts.len());
                for p in parts {
                    let w = self.infer_lhs_width(p);
                    // A heap-field store uses width 0 to defer sizing to its
                    // receiver. Splitting a concatenation requires concrete
                    // widths; leave this assignment to the interpreter.
                    if w == 0 {
                        self.bail("concat_lhs_dynamic_width");
                        return false;
                    }
                    part_widths.push(w);
                }
                let mut bit_offset: u32 = 0;
                for (i, p) in parts.iter().enumerate().rev() {
                    let pw = part_widths[i];
                    let lo_reg = self.alloc_reg();
                    self.emit(Insn::LoadConst(
                        lo_reg,
                        Box::new(Value::from_u64(bit_offset as u64, 32)),
                    ));
                    let hi_val = bit_offset + pw - 1;
                    let hi_reg = self.alloc_reg();
                    self.emit(Insn::LoadConst(
                        hi_reg,
                        Box::new(Value::from_u64(hi_val as u64, 32)),
                    ));
                    let part_reg = self.alloc_reg();
                    self.emit(Insn::RangeSelect(part_reg, val_reg, hi_reg, lo_reg));
                    self.emit(Insn::Resize(part_reg, pw));
                    if !self.compile_blocking_target(p, part_reg, pw) {
                        return false;
                    }
                    bit_offset += pw;
                }
                true
            }
            _ => {
                self.bail("blocking_target");
                false
            }
        }
    }

    /// Compile a continuous assign: evaluate RHS, write to pre-resolved LHS.
    /// Returns true if compiled successfully.
    pub fn compile_cont_assign(&mut self, rhs: &Expression, dst_id: usize, width: u32) -> bool {
        // Verilog context width = max(LHS width, RHS self-determined width).
        // Using just the LHS width truncates intermediates when operands
        // (e.g. 32-bit parameters) are wider than the target wire — but the
        // RHS width must be the LRM §11.6.1 SELF width, not the carry-aware
        // expr_max_width: the inflated context leaked dropped carries back
        // into shift results (`assign r = (a<<4)>>2` on 8-bit r computed the
        // inner shift at 12 bits and read 0x8c for 0x0c — while the IDENTICAL
        // always_comb, compiled with the plain LHS width, was correct).
        let ctx = width.max(self.lrm_self_width(rhs));
        if let Some(val_reg) = self.compile_expr(rhs, ctx) {
            if self.register_overflow {
                self.bail("bytecode_register_limit");
                return false;
            }
            // §10.3.2, §6.12.1: an integral net or variable driven by a real
            // takes the real rounded (`wire [63:0] w = r;`).
            if self.signal_real.is_some() {
                if self.signal_is_real(dst_id) {
                    self.emit_real_target_fit(val_reg, rhs);
                } else {
                    self.emit_real_to_int_fit(val_reg, rhs, width);
                }
            }
            self.emit(Insn::Resize(val_reg, width));
            self.emit(Insn::BlockingAssign(as_sig_id(dst_id), val_reg, width));
            true
        } else {
            false
        }
    }

    /// Compile a continuous assign with bit-select, part-select, or concat LHS:
    /// `assign d[i] = rhs`, `assign d[hi:lo] = rhs`, `assign {a,b} = rhs`.
    /// Reuses compile_blocking_target which emits BlockingAssignBitDyn /
    /// BlockingAssignRange / concat-split insns — same sub-range semantics
    /// as the interpreted assign_value path, but at bytecode speed.
    /// Yosys gate-level netlists emit hundreds of per-bit assigns that used
    /// to fall through to the interpreter on every settle iteration.
    pub fn compile_cont_assign_lhs(
        &mut self,
        lhs: &Expression,
        rhs: &Expression,
        lhs_width: u32,
    ) -> bool {
        let ctx = lhs_width.max(self.expr_max_width(rhs));
        if let Some(val_reg) = self.compile_expr(rhs, ctx) {
            if self.register_overflow {
                self.bail("bytecode_register_limit");
                return false;
            }
            if !self.value_may_be_real(lhs) {
                self.emit_real_to_int_fit(val_reg, rhs, lhs_width);
            }
            self.emit(Insn::Resize(val_reg, lhs_width));
            self.compile_blocking_target(lhs, val_reg, lhs_width)
        } else {
            false
        }
    }
}
