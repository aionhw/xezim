//! Case statements, case lookup tables and set membership.

use super::*;

impl<'a> BytecodeCompiler<'a> {
    /// Detect the ROM `case` shape: every arm `L = <const>` with constant
    /// patterns, optional `default: L = <const>`. Returns the shared lvalue,
    /// the dense table (default-filled holes), the default value, and the
    /// result width. Bounded at 4096 entries so a sparse 32-bit decode cannot
    /// balloon the block.
    pub(super) fn case_lut_shape(
        &mut self,
        items: &[crate::ast::stmt::CaseItem],
    ) -> Option<(Expression, Vec<Value>, Value, u32)> {
        let mut lhs: Option<Expression> = None;
        let mut entries: Vec<(u64, Value)> = Vec::new();
        let mut default: Option<Value> = None;
        let same_lhs = |l: &Expression, lhs: &mut Option<Expression>| -> bool {
            let ExprKind::Ident(h) = &l.kind else {
                return false;
            };
            if h.path.iter().any(|s| !s.selects.is_empty()) {
                return false;
            }
            match lhs {
                None => {
                    *lhs = Some(l.clone());
                    true
                }
                Some(prev) => {
                    let ExprKind::Ident(ph) = &prev.kind else {
                        return false;
                    };
                    Self::hier_raw_name(ph) == Self::hier_raw_name(h)
                }
            }
        };
        let const_of = |me: &mut Self, e: &Expression| -> Option<Value> {
            let v = match &e.kind {
                ExprKind::Number(n) => me.eval_number_static(n)?,
                ExprKind::Ident(h) => me.lookup_param_value(h)?,
                _ => return None,
            };
            (!v.has_xz() && !v.is_real).then_some(v)
        };
        for it in items {
            if it.pattern.is_some() || it.guard.is_some() {
                return None;
            }
            let StatementKind::BlockingAssign { lvalue, rvalue } = &it.stmt.kind else {
                return None;
            };
            if !same_lhs(lvalue, &mut lhs) {
                return None;
            }
            let rv = const_of(self, rvalue)?;
            if it.is_default {
                if default.is_some() {
                    return None;
                }
                default = Some(rv);
                continue;
            }
            for pat in &it.patterns {
                let pv = const_of(self, pat)?;
                let idx = pv.to_u64()?;
                if idx >= 4096 {
                    return None;
                }
                entries.push((idx, rv.clone()));
            }
        }
        let lhs = lhs?;
        let default = default?; // no default: keep the generic chain (lhs holds)
        let res_w = entries
            .iter()
            .map(|(_, v)| v.width)
            .chain(std::iter::once(default.width))
            .max()
            .unwrap_or(1);
        let size = entries.iter().map(|(i, _)| *i + 1).max().unwrap_or(0) as usize;
        let mut table = vec![default.clone(); size];
        for (i, v) in entries {
            table[i as usize] = v;
        }
        Some((lhs, table, default, res_w))
    }

    /// Computed-goto lowering for a dense plain `case`: constant, fully
    /// defined patterns dispatch straight to their arm's entry pc, replacing
    /// the compare-and-branch pair executed per SKIPPED arm. Unlike
    /// `case_lut_shape` the arm bodies are arbitrary compilable statements,
    /// so this covers the readback-mux / decoder shape that dominates the
    /// RISC-V core bench. Falls back to the generic chain (returns false,
    /// nothing emitted) on any non-conforming item.
    ///
    /// Correctness gates:
    /// - selector must be UNSIGNED and <=64 bits wide: dispatch compares raw
    ///   numeric values, which equals `===` only under zero-extension
    ///   (§11.8.1) and only when `to_u64` cannot truncate high bits;
    /// - patterns constant, x/z-free, < 4096 (table stays small);
    /// - >=8 pattern values, or 4..7 values with a dense dispatch table;
    /// - §12.5 first-match-wins kept for duplicate pattern values.
    pub(super) fn compile_case_jump(
        &mut self,
        expr: &Expression,
        items: &[crate::ast::stmt::CaseItem],
    ) -> bool {
        if self.expr_signedness(expr) != Some(false) || self.expr_max_width(expr) > 64 {
            return false;
        }
        // Bisect knobs (mirrors XEZIM_CAST_COMPILE_LIMIT): OFF disables the
        // lowering entirely, LIMIT=n keeps only the first n emissions.
        if std::env::var("XEZIM_CASEJUMP_OFF").is_ok() {
            return false;
        }
        // Shape scan first — nothing is emitted until every pattern proves
        // constant, so the common non-conforming case costs no rollback.
        let mut default_seen = false;
        let mut arm_vals: Vec<Vec<u64>> = Vec::with_capacity(items.len());
        let mut n_entries = 0usize;
        let mut max_idx = 0u64;
        for it in items {
            if it.pattern.is_some() || it.guard.is_some() {
                return false;
            }
            if it.is_default {
                if default_seen {
                    return false;
                }
                default_seen = true;
                arm_vals.push(Vec::new());
                continue;
            }
            let mut vals = Vec::with_capacity(it.patterns.len());
            for pat in &it.patterns {
                let pv = match &pat.kind {
                    ExprKind::Number(n) => self.eval_number_static(n),
                    ExprKind::Ident(h) => self.lookup_param_value(h),
                    _ => None,
                };
                let Some(pv) = pv else { return false };
                if pv.has_xz() || pv.is_real || pv.width > 64 {
                    return false;
                }
                let idx = match pv.to_u64() {
                    Some(i) if i < 4096 => i,
                    _ => return false,
                };
                max_idx = max_idx.max(idx);
                vals.push(idx);
            }
            n_entries += vals.len();
            arm_vals.push(vals);
        }
        // Admit short dense decoders as well as the existing >=8-entry cases.
        // A sparse short case keeps the compare chain rather than allocating
        // thousands of table slots to replace only a few comparisons (§12.5).
        if n_entries < 4 || (n_entries < 8 && max_idx > 2 * n_entries as u64 + 16) {
            return false;
        }
        // Emit with all-or-nothing rollback so a body that fails to compile
        // leaves the generic chain (and its own fallback path) untouched.
        let start = self.insns.len();
        let start_reg = self.next_reg;
        let Some(sel) = self.compile_expr(expr, 0) else {
            self.truncate_insns(start);
            self.next_reg = start_reg;
            return false;
        };
        let cj_idx = self.insns.len();
        self.emit(Insn::CaseJump(
            sel,
            Box::new(CaseJumpData {
                table: Vec::new(),
                default: 0,
            }),
        ));
        let mut table: Vec<Option<u32>> = vec![None; max_idx as usize + 1];
        let mut end_jumps: Vec<usize> = Vec::new();
        let mut default_entry: Option<u32> = None;
        // Arms are mutually exclusive; recycle their temporaries exactly as
        // the generic chain does so many-armed cases don't exhaust reg ids.
        let arm_reg_start = self.next_reg;
        let mut peak_reg = arm_reg_start;
        let mut ok = true;
        for (it, vals) in items.iter().zip(&arm_vals) {
            let entry = self.insns.len() as u32;
            if it.is_default {
                default_entry = Some(entry);
            } else {
                for &v in vals {
                    let slot = &mut table[v as usize];
                    if slot.is_none() {
                        *slot = Some(entry);
                    }
                }
            }
            if !self.compile_stmt(&it.stmt) {
                ok = false;
                break;
            }
            end_jumps.push(self.insns.len());
            self.emit(Insn::Jump(0));
            peak_reg = peak_reg.max(self.next_reg);
            self.next_reg = arm_reg_start;
        }
        if !ok {
            self.truncate_insns(start);
            self.next_reg = start_reg;
            return false;
        }
        self.next_reg = peak_reg;
        let end = self.insns.len() as u32;
        for idx in end_jumps {
            self.insns[idx] = Insn::Jump(end);
        }
        let default = default_entry.unwrap_or(end);
        let table: Vec<u32> = table.into_iter().map(|t| t.unwrap_or(default)).collect();
        static CJ_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = CJ_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let Ok(lim) = std::env::var("XEZIM_CASEJUMP_LIMIT") {
            if let Ok(lim) = lim.parse::<usize>() {
                if n >= lim {
                    self.truncate_insns(start);
                    self.next_reg = start_reg;
                    return false;
                }
            }
        }
        if std::env::var("XEZIM_CASEJUMP_TRACE").is_ok() {
            eprintln!(
                "[CASEJUMP] #{n} arms={} table={} scope={:?}",
                n_entries,
                table.len(),
                self.scope_hint
            );
        }
        self.insns[cj_idx] = Insn::CaseJump(sel, Box::new(CaseJumpData { table, default }));
        true
    }

    /// Two-level lowering for dense `casez`/`casex` with constant patterns
    /// (wildcard bits allowed). Picks the contiguous selector-bit window in
    /// which EVERY pattern is fully defined that best discriminates the
    /// patterns, then emits:
    ///
    ///   CaseMaskJump sel -> table[window bits] -> per-bucket residual chain
    ///                    -> xz_path (full chain) when the window has x/z
    ///
    /// Buckets are mutually exclusive for a defined-window selector (two
    /// patterns in different buckets differ in a bit both define), so
    /// per-bucket order == global order and §12.5 first-match-wins holds.
    /// The selector gate mirrors `compile_case_jump`: unsigned and <=64 bits,
    /// so extension is zero-fill and the window read equals the chain's
    /// CasezEq/CasexEq view of those bits.
    pub(super) fn compile_case_mask_jump(
        &mut self,
        kind: &crate::ast::stmt::CaseKind,
        expr: &Expression,
        items: &[crate::ast::stmt::CaseItem],
    ) -> bool {
        use crate::ast::stmt::CaseKind;
        let casex = matches!(kind, CaseKind::Casex);
        if self.expr_signedness(expr) != Some(false) {
            return false;
        }
        let sel_w = self.expr_max_width(expr);
        if sel_w == 0 || sel_w > 64 {
            return false;
        }
        if std::env::var("XEZIM_CASEJUMP_OFF").is_ok() {
            return false;
        }
        // ---- shape scan: constant patterns, wildcard masks ----
        let mut default_seen = false;
        // (item index, value bits, defined mask) per pattern
        let mut pats: Vec<(usize, u64, u64)> = Vec::new();
        let mut w_cmp = sel_w;
        for (ii, it) in items.iter().enumerate() {
            if it.pattern.is_some() || it.guard.is_some() {
                return false;
            }
            if it.is_default {
                if default_seen {
                    return false;
                }
                default_seen = true;
                continue;
            }
            for pat in &it.patterns {
                let pv = match &pat.kind {
                    ExprKind::Number(n) => self.eval_number_static(n),
                    ExprKind::Ident(h) => self.lookup_param_value(h),
                    _ => None,
                };
                let Some(pv) = pv else { return false };
                if pv.is_real || pv.is_signed || pv.width == 0 || pv.width > 64 || pv.is_fill {
                    return false;
                }
                let Some((v, xz)) = pv.inline_bits() else {
                    return false;
                };
                let m = if pv.width >= 64 {
                    u64::MAX
                } else {
                    (1u64 << pv.width) - 1
                };
                let (v, xz) = (v & m, xz & m);
                // casez: Z (val&xz both set) is wild; a plain X in the
                // pattern can match nothing defined -> keep it on the chain.
                let wild = if casex { xz } else { v & xz };
                if !casex && (xz & !wild) != 0 {
                    return false;
                }
                w_cmp = w_cmp.max(pv.width);
                // Bits past the pattern's width zero-extend (defined 0s).
                pats.push((ii, v & !wild, !wild | !m));
            }
        }
        if pats.len() < 8 {
            return false;
        }
        // ---- window choice: contiguous, all-defined, best discrimination ----
        let all_defined = pats.iter().fold(u64::MAX, |acc, (_, _, d)| acc & d)
            & if w_cmp >= 64 {
                u64::MAX
            } else {
                (1u64 << w_cmp) - 1
            };
        let mut best: Option<(u32, u32, usize)> = None; // (lo, width, distinct)
        for lo in 0..w_cmp {
            if all_defined & (1u64 << lo) == 0 {
                continue;
            }
            let max_k = 12.min(w_cmp - lo);
            for k in 1..=max_k {
                if all_defined & (1u64 << (lo + k - 1)) == 0 {
                    break;
                }
                let wmask = (1u64 << k) - 1;
                let mut vals: Vec<u64> = pats.iter().map(|(_, v, _)| (v >> lo) & wmask).collect();
                vals.sort_unstable();
                vals.dedup();
                let distinct = vals.len();
                let better = match best {
                    None => true,
                    Some((_, bk, bd)) => distinct > bd || (distinct == bd && k < bk),
                };
                if better {
                    best = Some((lo, k, distinct));
                }
            }
        }
        let Some((lo, wk, distinct)) = best else {
            return false;
        };
        // A window that cannot split the patterns at all buys nothing.
        if distinct < 2 {
            return false;
        }
        let wmask = (1u64 << wk) - 1;
        // ---- emission with all-or-nothing rollback ----
        let start = self.insns.len();
        let start_reg = self.next_reg;
        let Some(sel) = self.compile_expr(expr, 0) else {
            self.truncate_insns(start);
            self.next_reg = start_reg;
            return false;
        };
        let mj_idx = self.insns.len();
        self.emit(Insn::CaseMaskJump(
            sel,
            Box::new(CaseMaskJumpData {
                lo,
                width: wk,
                table: Vec::new(),
                xz_path: 0,
            }),
        ));
        // Arm bodies, compiled once each, ending in Jump(end).
        let mut body_entry: Vec<u32> = vec![0; items.len()];
        let mut default_entry: Option<u32> = None;
        let mut end_jumps: Vec<usize> = Vec::new();
        let arm_reg_start = self.next_reg;
        let mut peak_reg = arm_reg_start;
        let mut ok = true;
        // Layout: the per-bucket wildcard chains and the x/z chain come FIRST
        // and the arm bodies AFTER them, so every chain -> arm jump is
        // forward. Arms first (chains at the end jumping back into them)
        // gave every casez decoder a backward branch, and the two-state
        // lowering refuses a block with a backward branch as soon as any
        // register is reused at a different width — which the arm temporaries
        // always are. `arm_fixups` patches the chain jumps once the arm
        // entries are known.
        let mut arm_fixups: Vec<(usize, usize)> = Vec::new();
        let mut bucket_entries: Vec<u32> = Vec::new();
        let mut xz_entry: u32 = 0;
        if ok {
            // Per-bucket residual chains.
            let n_buckets = 1usize << wk;
            bucket_entries = vec![0; n_buckets];
            'buckets: for b in 0..n_buckets {
                let subset: Vec<usize> = pats
                    .iter()
                    .enumerate()
                    .filter(|(_, (_, v, _))| ((v >> lo) & wmask) as usize == b)
                    .map(|(pi, _)| pi)
                    .collect();
                if subset.is_empty() {
                    bucket_entries[b] = u32::MAX; // patched to default later
                    continue;
                }
                bucket_entries[b] = self.insns.len() as u32;
                for pi in subset {
                    let (ii, _, _) = pats[pi];
                    let pat_expr = Self::nth_pattern(items, pi);
                    let Some(pat_reg) = self.compile_expr(pat_expr, 0) else {
                        ok = false;
                        break 'buckets;
                    };
                    let cmp = self.alloc_reg();
                    self.emit(if casex {
                        Insn::CasexEq(cmp, sel, pat_reg)
                    } else {
                        Insn::CasezEq(cmp, sel, pat_reg)
                    });
                    let bidx = self.insns.len();
                    self.emit(Insn::BranchIfFalse(cmp, 0));
                    arm_fixups.push((self.insns.len(), ii));
                    self.emit(Insn::Jump(0));
                    let next = self.insns.len() as u32;
                    self.insns[bidx] = Insn::BranchIfFalse(cmp, next);
                    peak_reg = peak_reg.max(self.next_reg);
                    self.next_reg = arm_reg_start;
                }
                let dj = self.insns.len();
                self.emit(Insn::Jump(0)); // -> default, patched below
                end_jumps.push(usize::MAX - dj); // marker: default jump
            }
        }
        if ok {
            // Full chain for wildcard selectors.
            xz_entry = self.insns.len() as u32;
            for pi in 0..pats.len() {
                let (ii, _, _) = pats[pi];
                let pat_expr = Self::nth_pattern(items, pi);
                let Some(pat_reg) = self.compile_expr(pat_expr, 0) else {
                    ok = false;
                    break;
                };
                let cmp = self.alloc_reg();
                self.emit(if casex {
                    Insn::CasexEq(cmp, sel, pat_reg)
                } else {
                    Insn::CasezEq(cmp, sel, pat_reg)
                });
                let bidx = self.insns.len();
                self.emit(Insn::BranchIfFalse(cmp, 0));
                arm_fixups.push((self.insns.len(), ii));
                self.emit(Insn::Jump(0));
                let next = self.insns.len() as u32;
                self.insns[bidx] = Insn::BranchIfFalse(cmp, next);
                peak_reg = peak_reg.max(self.next_reg);
                self.next_reg = arm_reg_start;
            }
            let dj = self.insns.len();
            self.emit(Insn::Jump(0));
            end_jumps.push(usize::MAX - dj);
        }
        if ok {
            for (ii, it) in items.iter().enumerate() {
                let entry = self.insns.len() as u32;
                body_entry[ii] = entry;
                if it.is_default {
                    default_entry = Some(entry);
                }
                if !self.compile_stmt(&it.stmt) {
                    ok = false;
                    break;
                }
                end_jumps.push(self.insns.len());
                self.emit(Insn::Jump(0));
                peak_reg = peak_reg.max(self.next_reg);
                self.next_reg = arm_reg_start;
            }
        }
        if !ok {
            self.truncate_insns(start);
            self.next_reg = start_reg;
            return false;
        }
        for (idx, ii) in arm_fixups {
            self.insns[idx] = Insn::Jump(body_entry[ii]);
        }
        self.next_reg = peak_reg;
        let end = self.insns.len() as u32;
        let default = default_entry.unwrap_or(end);
        for ej in end_jumps {
            if ej > usize::MAX / 2 {
                self.insns[usize::MAX - ej] = Insn::Jump(default);
            } else {
                self.insns[ej] = Insn::Jump(end);
            }
        }
        let table: Vec<u32> = bucket_entries
            .into_iter()
            .map(|t| if t == u32::MAX { default } else { t })
            .collect();
        self.insns[mj_idx] = Insn::CaseMaskJump(
            sel,
            Box::new(CaseMaskJumpData {
                lo,
                width: wk,
                table,
                xz_path: xz_entry,
            }),
        );
        static CMJ_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = CMJ_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if std::env::var("XEZIM_CASEJUMP_TRACE").is_ok() {
            eprintln!(
                "[CASEMASKJUMP] #{n} pats={} lo={lo} w={wk} distinct={distinct} scope={:?}",
                pats.len(),
                self.scope_hint
            );
        }
        true
    }

    /// The `pi`-th pattern in flat (item, pattern) order — the order
    /// `compile_case_mask_jump`'s scan built its `pats` list in.
    fn nth_pattern<'e>(items: &'e [crate::ast::stmt::CaseItem], pi: usize) -> &'e Expression {
        let mut k = 0usize;
        for it in items {
            if it.is_default {
                continue;
            }
            for pat in &it.patterns {
                if k == pi {
                    return pat;
                }
                k += 1;
            }
        }
        unreachable!("pattern index out of range")
    }

    /// §11.4.13: the compiled membership test compares integral operands
    /// as unsigned values sized to the widest of them, which is `==?` for
    /// x/z-free members. A real operand needs `==` with each member
    /// converted to real by its own sign and a fill member kept at one bit
    /// (§11.4.5, §5.7.1), so an operand that may be real keeps the
    /// interpreter. The operand's type is not always known here (an element,
    /// a member, or a continuous assignment compiled without signal types),
    /// so a member whose value differs between the two comparisons — a
    /// negative signed constant or a fill — keeps the interpreter too.
    pub(super) fn inside_needs_numeric_eq(&self, e: &Expression, members: &[Value]) -> bool {
        self.expr_may_be_real(e)
            || self.expr_is_real_static(e)
            || members
                .iter()
                .any(|v| v.is_fill || (v.is_signed && v.to_i64().is_some_and(|x| x < 0)))
    }

    /// §12.5.4: true when the compiled chain matches every item of a `case
    /// inside` as the `inside` operator does: an x/z-free integral constant
    /// by `===`, an unsigned constant with x/z bits by the masked compare of
    /// `case_inside_wildcard_item` (an unsigned selector of at most 64 bits,
    /// `wild_sel_w`), and a selector compared as an integral (see
    /// `inside_needs_numeric_eq`). A range, an array or any other item is
    /// not.
    pub(super) fn case_inside_matches_by_case_eq(
        &mut self,
        sel: &Expression,
        items: &[crate::ast::stmt::CaseItem],
        wild_sel_w: Option<u32>,
    ) -> bool {
        let mut members: Vec<Value> = Vec::new();
        for it in items.iter().filter(|it| !it.is_default) {
            if it.pattern.is_some() || it.guard.is_some() {
                return false;
            }
            for p in &it.patterns {
                if let Some(v) = self.inside_member_const(p) {
                    members.push(v);
                } else if wild_sel_w.is_none_or(|w| self.case_inside_wildcard_item(p, w).is_none())
                {
                    return false;
                }
            }
        }
        !self.inside_needs_numeric_eq(sel, &members)
    }

    /// §11.4.6, §12.5.4: an unsigned integral constant `case inside` item
    /// with x/z bits as the (care mask, value) pair its `==?` match compares
    /// under, both 64 bits wide: the item's x/z bits match anything, and
    /// every other bit — the zero extension to the selector's width
    /// included — must be known and equal. `sel_w` is the unsigned
    /// selector's width; the selector register, zero-extended, is masked and
    /// compared by `===`, so an x/z selector bit under the mask matches
    /// nothing, as `==?` makes it x.
    pub(super) fn case_inside_wildcard_item(
        &mut self,
        p: &Expression,
        sel_w: u32,
    ) -> Option<(Value, Value)> {
        let v = match &p.kind {
            ExprKind::Number(num) => self.eval_number_static(num)?,
            ExprKind::Ident(h) => self.lookup_param_value(h)?,
            ExprKind::Paren(inner) => return self.case_inside_wildcard_item(inner, sel_w),
            _ => return None,
        };
        if v.is_real || v.is_fill || v.is_signed || v.width == 0 || v.width > 64 || sel_w > 64 {
            return None;
        }
        let (val, xz) = v.inline_bits()?;
        let width_mask = if v.width >= 64 {
            u64::MAX
        } else {
            (1u64 << v.width) - 1
        };
        if xz & width_mask == 0 {
            return None;
        }
        let care = !(xz & width_mask);
        Some((
            Value::from_u64(care, 64),
            Value::from_u64(val & care & width_mask, 64),
        ))
    }

    /// A set-membership member as a compile-time CONSTANT with no x/z bits —
    /// the only shape `compile_inside` accepts (see there).
    pub(super) fn inside_member_const(&mut self, m: &Expression) -> Option<Value> {
        let v = match &m.kind {
            ExprKind::Number(num) => self.eval_number_static(num)?,
            ExprKind::Ident(h) => self.lookup_param_value(h)?,
            ExprKind::Paren(inner) => return self.inside_member_const(inner),
            _ => return None,
        };
        if v.has_xz() || v.is_real {
            return None;
        }
        Some(v)
    }
}
