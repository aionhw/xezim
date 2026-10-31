//! Simulator statement assignments.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    pub(super) fn eval_number(&self, num: &NumberLiteral) -> Value {
        match num {
            NumberLiteral::Integer {
                size,
                signed,
                base,
                value,
                cached_val,
            } => {
                let r_for_w = match base {
                    NumberBase::Binary => 2,
                    NumberBase::Octal => 8,
                    NumberBase::Hex => 16,
                    NumberBase::Decimal => 10,
                };
                // §5.7.1: an UNSIZED literal is at least 32 bits, but must not
                // drop digits the source wrote — `'h123456789ABCDEF0` is 64 bits
                // of value and a flat 32 kept only the low half, silently.
                // The cached triple already carries the width the literal
                // was parsed at; an UNSIZED literal used to recompute it from
                // its text on every evaluation.
                let cached = cached_val.get();
                let w = match size {
                    Some(sz) => *sz,
                    None => match cached {
                        Some((_, _, cw)) => cw,
                        None => Value::unsized_literal_width(value, r_for_w),
                    },
                };
                // §5.7.1: an unsized all-x/all-z literal ('bx, 'hz, 'b?) is a
                // FILL — it must replicate to the consuming context, not stop
                // at 32 bits with zero-extension above.
                let xz_fill = size.is_none() && Value::unsized_xz_fill_char(value).is_some();
                // Fast path: return cached value (avoids re-parsing string)
                if let Some((vb, xz, cw)) = cached {
                    if cw == w {
                        let mut v = Value::from_inline(vb, xz, w);
                        v.is_signed = *signed;
                        v.is_fill = xz_fill;
                        return v;
                    }
                }
                let r = match base {
                    NumberBase::Binary => 2,
                    NumberBase::Octal => 8,
                    NumberBase::Hex => 16,
                    NumberBase::Decimal => 10,
                };
                // §5.7 (issue #31): a PROCEDURAL-only literal never passes
                // through elaboration const-eval, so the wrap warning has to
                // fire here too. Slow path only (the cached-value fast path
                // above returns first), and the shared dedupe means a literal
                // already reported at elaboration stays quiet.
                super::elaborate::warn_unsized_decimal_wrap(*size, base, value);
                let mut v = Value::from_str_radix(value, r, w);
                // Cache inline values (width <= 64)
                if w <= 64 {
                    if let Some((vb, xz)) = v.inline_bits() {
                        cached_val.set(Some((vb, xz, w)));
                    }
                }
                v.is_signed = *signed;
                v.is_fill = xz_fill;
                v
            }
            NumberLiteral::Real(f) => Value::from_f64(*f),
            // §5.8: a time literal in a value context scales to the MODULE's
            // timeunit, snapped to the precision so the result is exact —
            // `30000ps` under `timescale 1ns/1ps` must compare EQUAL to a
            // `$time` delta of 30, not 29.999999999999996 (the raw
            // secs/unit_s float residue). Computed as
            // round(secs/prec) / 10^(unit−prec): both operands are integral
            // f64s, so the division is exact for real timescale ratios.
            // (Delay contexts route through eval_delay_ticks instead.)
            NumberLiteral::Time(s) => {
                let (unit_exp, prec_exp) = self.current_timescale_exp();
                let prec_s = 10f64.powi(prec_exp);
                let ratio = 10f64.powi(unit_exp - prec_exp);
                Value::from_f64((*s / prec_s).round() / ratio)
            }
            // §5.7.1: unbased-unsized literal — a 1-bit FILL value; binary ops
            // and resize replicate it to the consuming context's width.
            NumberLiteral::UnbasedUnsized(c) => Value::fill_of(*c),
        }
    }

    /// Active tag of a tagged-union subject (§7.3.2), if it is one.
    pub(super) fn subject_union_tag(&self, subject: &Expression) -> Option<String> {
        if let ExprKind::Ident(h) = &subject.kind {
            let n = self.resolve_hier_name(h);
            return self.active_union_tag.get(&*n).cloned();
        }
        None
    }

    /// Test a §12.6 pattern against `subject`, collecting `.v` bindings.
    pub(super) fn match_pattern(
        &mut self,
        subject: &Expression,
        pat: &Pattern,
        binds: &mut Vec<(String, Value)>,
    ) -> bool {
        match pat {
            Pattern::Wildcard => true,
            Pattern::Binding(id) => {
                let v = self.eval_expr(subject);
                binds.push((id.name.clone(), v));
                true
            }
            Pattern::Tagged { tag, inner } => {
                // Subject must be a tagged union currently holding this member.
                match self.subject_union_tag(subject) {
                    Some(cur) if cur == tag.name => match inner {
                        // The union's stored value IS the member payload.
                        Some(ip) => self.match_pattern(subject, ip, binds),
                        None => true,
                    },
                    _ => false,
                }
            }
            Pattern::Expr(e) => {
                let a = self.eval_expr(subject);
                let b = self.eval_expr(e);
                a.case_eq(&b).is_true()
            }
            // §12.6 structure pattern `'{a: .x, b: 3}`: members bind by name
            // when named, positionally otherwise.
            Pattern::Struct(members) => {
                let Some(ordered) = self.struct_members_ordered(subject) else {
                    return false;
                };
                for (i, (name, sub)) in members.iter().enumerate() {
                    let target = match name {
                        Some(id) => ordered
                            .iter()
                            .find(|(n, _)| *n == id.name)
                            .map(|(_, e)| e.clone()),
                        None => ordered.get(i).map(|(_, e)| e.clone()),
                    };
                    let Some(t) = target else { return false };
                    if !self.match_pattern(&t, sub, binds) {
                        return false;
                    }
                }
                true
            }
        }
    }

    /// Make pattern bindings visible as locals; returns state to undo them.
    pub(super) fn push_pattern_bindings(
        &mut self,
        binds: &[(String, Value)],
    ) -> (bool, Vec<(String, Option<Value>)>) {
        let pushed = if self.local_stack.is_empty() {
            self.push_local_frame(HashMap::default());
            true
        } else {
            false
        };
        let mut saved = Vec::with_capacity(binds.len());
        if let Some(f) = self.local_stack.last_mut() {
            for (n, v) in binds {
                saved.push((n.clone(), f.insert(n.clone(), v.clone())));
            }
        }
        (pushed, saved)
    }

    pub(super) fn pop_pattern_bindings(&mut self, st: (bool, Vec<(String, Option<Value>)>)) {
        let (pushed, saved) = st;
        if let Some(f) = self.local_stack.last_mut() {
            for (n, prev) in saved {
                match prev {
                    Some(v) => {
                        f.insert(n, v);
                    }
                    None => {
                        f.remove(&n);
                    }
                }
            }
        }
        if pushed {
            self.pop_local_frame();
        }
    }

    /// §12.6.1: run the first item whose pattern matches (and whose `&&&`
    /// guard holds), with its `.v` bindings in scope; else the `default` item.
    pub(super) fn exec_case_matches(&mut self, subject: &Expression, items: &[CaseItem]) {
        let mut default_idx: Option<usize> = None;
        for (i, item) in items.iter().enumerate() {
            if item.is_default {
                default_idx = Some(i);
                continue;
            }
            let Some(pat) = &item.pattern else { continue };
            let mut binds: Vec<(String, Value)> = Vec::new();
            if !self.match_pattern(subject, pat, &mut binds) {
                continue;
            }
            // Bindings are visible to both the guard and the body.
            let st = self.push_pattern_bindings(&binds);
            let pass = match &item.guard {
                Some(g) => self.eval_expr(g).is_true(),
                None => true,
            };
            if pass {
                self.exec_statement(&item.stmt);
                self.pop_pattern_bindings(st);
                return;
            }
            self.pop_pattern_bindings(st);
        }
        if let Some(i) = default_idx {
            self.exec_statement(&items[i].stmt);
        }
    }

    /// IEEE 1800-2017 §9.3.2. A declaration at the head of a `fork` belongs to
    /// the fork BLOCK, not to a spawned process. Execute those in the forking
    /// process, then return only the statements that are actually processes.
    ///
    /// Every top-level item between `fork` and `join*` used to be spawned,
    /// declarations included. So `fork automatic int j = i; begin ... j ... end
    /// join_none` spawned two siblings with independently cloned frames: the
    /// declaration landed in one frame and the body read `j` from the other,
    /// as X. It appeared to work outside a loop only because a process with no
    /// captured loop variable gets no local frame at all, and the declaration
    /// then fell through to the global signal map, which the sibling shared.
    ///
    /// Running them here, BEFORE `inherit_fork_child_context` snapshots the
    /// context, gives each child its own copy at fork time — the per-spawn
    /// capture the construct exists for.
    /// Returns the statements to spawn. The declared names are appended to
    /// `auto_loop_vars` for the caller's spawn loop, which is what makes
    /// `inherit_fork_child_context` copy them into each child's frame by value.
    /// The caller must truncate `auto_loop_vars` back afterwards.
    ///
    /// Without that, a declaration executed in a process that has no local
    /// frame lands in the shared `signals` map, so every child reads whatever
    /// the LAST fork iteration wrote — the exact capture bug the construct
    /// exists to avoid (all five children saw `local_i == 4`).
    pub(super) fn exec_fork_block_decls<'a>(
        &mut self,
        stmts: &'a [Statement],
    ) -> (Vec<&'a Statement>, usize) {
        let saved_auto_len = self.auto_loop_vars.len();
        let mut processes = Vec::with_capacity(stmts.len());
        for s in stmts {
            if let StatementKind::VarDecl { declarators, .. } = &s.kind {
                self.exec_statement(s);
                for d in declarators {
                    self.auto_loop_vars.push(d.name.name.clone());
                }
            } else {
                processes.push(s);
            }
        }
        (processes, saved_auto_len)
    }

    /// Collapse a canonical full-dimension packed fill into one write.
    ///
    /// The hot stimulus shape is
    /// `for (int i = LO; i <= HI; i++) packed_prefix[i] = invariant;`.
    /// Interpreting it element-by-element is expensive because the generic
    /// packed-index lvalue path rebuilds dimension and name state per store.
    /// When the loop covers one complete declared packed dimension and the RHS
    /// is pure and loop-invariant, the final operation is one replication.
    /// Uncertain shapes use the ordinary loop.
    pub(super) fn try_exec_full_packed_fill(
        &mut self,
        init: &[ForInit],
        condition: Option<&Expression>,
        step: &[Expression],
        body: &Statement,
    ) -> bool {
        if self.warn_x
            || self.vcd_writer.is_some()
            || self.xtrace_file.is_some()
            || self.xtrace_writer.is_some()
            || self.fst_file.is_some()
            || self.fst_writer.is_some()
            || !self.forced_signals.is_empty()
            || !self.forced_names.is_empty()
            || !self.active_force_exprs.is_empty()
        {
            return false;
        }

        let [
            ForInit::VarDecl {
                name: loop_id,
                init: start_expr,
                ..
            },
        ] = init
        else {
            return false;
        };
        let loop_name = loop_id.name.as_str();
        let Some(condition) = condition else {
            return false;
        };
        let ExprKind::Binary {
            op: cmp,
            left,
            right: bound_expr,
        } = &condition.kind
        else {
            return false;
        };
        if !matches!(cmp, BinaryOp::Lt | BinaryOp::Leq)
            || Self::plain_ident_name(left).as_deref() != Some(loop_name)
            || !Self::packed_fill_expr_is_pure(bound_expr, loop_name)
        {
            return false;
        }
        let [step_expr] = step else { return false };
        let ExprKind::Unary { op, operand } = &step_expr.kind else {
            return false;
        };
        if !matches!(op, UnaryOp::PreIncr | UnaryOp::PostIncr)
            || Self::plain_ident_name(operand).as_deref() != Some(loop_name)
            || !Self::packed_fill_expr_is_pure(start_expr, loop_name)
        {
            return false;
        }
        // The collapse assumes the trip count cannot change while procedural
        // writes trigger eager settle work. Numeric constant expressions are
        // sufficient for the hot stimulus loops; parameter/signal bounds use
        // the normal implementation until const metadata is available here.
        let mut bound_reads: HashSet<String> = HashSet::default();
        Self::collect_leaf_idents(start_expr, &mut bound_reads);
        Self::collect_leaf_idents(bound_expr, &mut bound_reads);
        if !bound_reads.is_empty() {
            return false;
        }

        let Some(start) = self.eval_expr(start_expr).to_i64() else {
            return false;
        };
        let Some(bound) = self.eval_expr(bound_expr).to_i64() else {
            return false;
        };
        let end = if matches!(cmp, BinaryOp::Lt) {
            match bound.checked_sub(1) {
                Some(v) => v,
                None => return false,
            }
        } else {
            bound
        };
        if end < start {
            return false;
        }
        let count = match usize::try_from(end - start + 1) {
            Ok(n) if n > 1 && n <= (1 << 20) => n,
            _ => return false,
        };

        let body_stmts: &[Statement] = match &body.kind {
            StatementKind::SeqBlock { stmts, .. } => stmts,
            _ => std::slice::from_ref(body),
        };
        if body_stmts.is_empty() {
            return false;
        }

        // Evaluate invariant RHSes only after proving none reads a destination
        // written by this collapsed loop.
        let mut dest_roots: HashSet<String> = HashSet::default();
        for st in body_stmts {
            let StatementKind::BlockingAssign { lvalue, .. } = &st.kind else {
                return false;
            };
            let ExprKind::Index {
                expr: prefix,
                index,
            } = &lvalue.kind
            else {
                return false;
            };
            if Self::plain_ident_name(index).as_deref() != Some(loop_name) {
                return false;
            }
            let Some((root, _)) = Self::packed_fill_root_and_depth(prefix) else {
                return false;
            };
            let Some(leaf) = root.path.last().map(|s| s.name.name.clone()) else {
                return false;
            };
            if !dest_roots.insert(leaf) {
                return false;
            }
        }

        let mut writes: Vec<(Expression, Value)> = Vec::with_capacity(body_stmts.len());
        for st in body_stmts {
            let StatementKind::BlockingAssign { lvalue, rvalue } = &st.kind else {
                return false;
            };
            if !Self::packed_fill_expr_is_pure(rvalue, loop_name) {
                return false;
            }
            let mut rhs_reads: HashSet<String> = HashSet::default();
            Self::collect_leaf_idents(rvalue, &mut rhs_reads);
            if rhs_reads.iter().any(|n| dest_roots.contains(n)) {
                return false;
            }

            let ExprKind::Index { expr: prefix, .. } = &lvalue.kind else {
                return false;
            };
            let Some((root, depth)) = Self::packed_fill_root_and_depth(prefix) else {
                return false;
            };
            let root_name = self.resolve_hier_name(root);
            if self.module.arrays.contains_key(&*root_name)
                || self.module.arrays_2d.contains_key(&*root_name)
                || self.module.arrays_nd.contains_key(&*root_name)
                || self.module.dynamic_arrays.contains(&*root_name)
                || self.module.associative_arrays.contains_key(&*root_name)
            {
                return false;
            }
            let Some(&(decl_l, decl_r)) = self
                .module
                .packed_full_dims
                .get(&*root_name)
                .and_then(|dims| dims.get(depth))
            else {
                return false;
            };
            let (decl_lo, decl_hi) = (decl_l.min(decl_r), decl_l.max(decl_r));
            if start != decl_lo || end != decl_hi || count != (decl_hi - decl_lo + 1) as usize {
                return false;
            }

            let elem_w = self.infer_lhs_width(lvalue).max(1);
            let prefix_w = self.infer_lhs_width(prefix).max(1);
            if (elem_w as usize).saturating_mul(count) != prefix_w as usize {
                return false;
            }
            let piece = self.eval_expr(rvalue).resize_for_assign(elem_w);
            let fill = Value::concat_refs(std::iter::repeat_n(&piece, count));
            writes.push(((**prefix).clone(), fill));
        }

        for (target, fill) in writes {
            self.assign_value(&target, &fill);
            self.settle_after_proc_write();
        }
        self.packed_blocking_fill_collapses += 1;
        true
    }

    /// Root identifier and number of already-consumed packed dimensions for a
    /// plain `root[i][j]...` prefix.
    pub(super) fn packed_fill_root_and_depth(
        mut expr: &Expression,
    ) -> Option<(&HierarchicalIdentifier, usize)> {
        let mut depth = 0usize;
        while let ExprKind::Index { expr: inner, .. } = &expr.kind {
            depth += 1;
            expr = inner;
        }
        match &expr.kind {
            ExprKind::Ident(h) => Some((h, depth)),
            _ => None,
        }
    }

    pub(super) fn packed_fill_expr_is_pure(expr: &Expression, loop_name: &str) -> bool {
        match &expr.kind {
            ExprKind::Number(_) | ExprKind::StringLiteral(_) => true,
            ExprKind::Ident(h) => {
                !(h.path.len() == 1 && h.path[0].name.name == loop_name)
                    && h.path.iter().all(|p| {
                        p.selects
                            .iter()
                            .all(|s| Self::packed_fill_expr_is_pure(s, loop_name))
                    })
            }
            ExprKind::Paren(e) => Self::packed_fill_expr_is_pure(e, loop_name),
            ExprKind::Unary { op, operand } => {
                !matches!(
                    op,
                    UnaryOp::PreIncr | UnaryOp::PostIncr | UnaryOp::PreDecr | UnaryOp::PostDecr
                ) && Self::packed_fill_expr_is_pure(operand, loop_name)
            }
            ExprKind::Binary { op, left, right } => {
                !matches!(op, BinaryOp::Assign)
                    && Self::packed_fill_expr_is_pure(left, loop_name)
                    && Self::packed_fill_expr_is_pure(right, loop_name)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::packed_fill_expr_is_pure(condition, loop_name)
                    && Self::packed_fill_expr_is_pure(then_expr, loop_name)
                    && Self::packed_fill_expr_is_pure(else_expr, loop_name)
            }
            ExprKind::Concatenation(parts) => parts
                .iter()
                .all(|e| Self::packed_fill_expr_is_pure(e, loop_name)),
            ExprKind::Replication { count, exprs } => {
                Self::packed_fill_expr_is_pure(count, loop_name)
                    && exprs
                        .iter()
                        .all(|e| Self::packed_fill_expr_is_pure(e, loop_name))
            }
            ExprKind::Index { expr, index } => {
                Self::packed_fill_expr_is_pure(expr, loop_name)
                    && Self::packed_fill_expr_is_pure(index, loop_name)
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                Self::packed_fill_expr_is_pure(expr, loop_name)
                    && Self::packed_fill_expr_is_pure(left, loop_name)
                    && Self::packed_fill_expr_is_pure(right, loop_name)
            }
            _ => false,
        }
    }

    /// §7.6/§13.4.1: snapshot a fixed unpacked return in declaration
    /// order before its local storage is restored or destroyed. The reserved
    /// return store has zero-based indices, independently of the source range.
    pub(super) fn snapshot_ret_fixed(&mut self, name: &str) -> bool {
        if self.module.dynamic_arrays.contains(name) {
            return false;
        }
        let Some(&(lo, hi, _)) = self.module.arrays.get(name) else {
            return false;
        };
        let descending = self.module.descending_arrays.contains(name);
        const RET_SNAP: &str = "__xz_ret_coll__";
        for pos in 0..=(hi - lo) {
            let idx = if descending { hi - pos } else { lo + pos };
            if let Some(v) = self.get_signal_value_by_name(&format!("{name}[{idx}]")) {
                self.set_signal_value_by_name(&format!("{RET_SNAP}[{pos}]"), v);
            }
        }
        self.set_queue_size(RET_SNAP, (hi - lo + 1) as u64);
        self.pending_ret_collection = Some(RET_SNAP.to_string());
        true
    }

    /// The full path's step after evaluating a call RHS: a collection the
    /// call just returned (`pending_ret_collection`) is copied whole into a
    /// collection target. `true` when the assignment is done.
    pub(super) fn assign_pending_ret_collection(&mut self, lvalue: &Expression) -> bool {
        let Some(src) = self.take_pending_ret_collection() else {
            return false;
        };
        // §13.4, §7.9.9: an associative array returned by the call.
        if src == Self::RET_ASSOC_SNAP {
            let Some(dst) = self.ret_assoc_operand_name(lvalue) else {
                return false;
            };
            self.copy_assoc_entries(&dst, &src);
            self.settle_after_proc_write();
            return true;
        }
        let dst: Option<String> = self.pending_coll_dst(lvalue);
        let Some(dst) = dst else {
            return false;
        };
        if dst != src {
            if let Some(&(lo, hi, _)) = self.module.arrays.get(&dst)
                && !self.module.dynamic_arrays.contains(&dst)
            {
                // §7.6: whole-array assignment pairs elements in each
                // declaration's order, including descending/nonzero ranges.
                let descending = self.module.descending_arrays.contains(&dst);
                for pos in 0..=(hi - lo) {
                    let idx = if descending { hi - pos } else { lo + pos };
                    if let Some(v) = self.get_signal_value_by_name(&format!("{src}[{pos}]")) {
                        self.set_signal_value_by_name(&format!("{dst}[{idx}]"), v);
                    }
                }
            } else {
                self.copy_whole_queue(&dst, &src);
            }
        }
        self.settle_after_proc_write();
        true
    }

    /// Take the pending collection return for a consumer that reads it by
    /// name. A compiled `RetCollection` defers its `__xz_ret_coll__[i]`
    /// snapshot (the values wait in `vm_ret_vals`); materialize it here so
    /// every by-name reader sees this call's elements, not an older snapshot.
    pub(super) fn take_pending_ret_collection(&mut self) -> Option<String> {
        const RET_SNAP: &str = "__xz_ret_coll__";
        let src = self.pending_ret_collection.take()?;
        if src == RET_SNAP
            && let Some(vals) = self.vm_ret_vals.take()
        {
            for (i, v) in vals.iter().enumerate() {
                let name = format!("{RET_SNAP}[{i}]");
                self.set_signal_value_by_name(&name, v.clone());
                self.widths.entry(name).or_insert(32);
            }
            self.set_queue_size(RET_SNAP, vals.len() as u64);
        }
        Some(src)
    }

    /// Resolve the whole-collection storage name that a collection-return
    /// assignment `q = f(..)` writes into, for the destination expression
    /// `lvalue`. This is the exact resolution shared by the historic
    /// `assign_pending_ret_collection` copy and the compiled flat-value fast
    /// path: a rename-frame local / dynamic-array name (via [`dyn_name_lookup`])
    /// or an [`expr_assoc_name`] instance-scoped collection; the associative
    /// case is excluded (an assoc assign keeps the AST copy path).
    pub(super) fn pending_coll_dst(&mut self, lvalue: &Expression) -> Option<String> {
        let byname = if let ExprKind::Ident(h) = &lvalue.kind {
            let bare = h.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
            let n = self
                .dyn_name_lookup(bare)
                .map(|s| s.to_string())
                .unwrap_or_else(|| self.resolve_hier_name(h).into_owned());
            (self.module.dynamic_arrays.contains(&n) || self.module.arrays.contains_key(&n))
                .then_some(n)
        } else {
            None
        };
        byname.or_else(|| {
            self.expr_assoc_name(lvalue)
                .filter(|an| !self.is_associative_array(an))
        })
    }
    /// The extra conditions for `n = r` (both bare names) to reduce to a
    /// plain evaluate + assign: no `new`, no vif binding to carry, no
    /// struct copy or spread, and no collection / array / associative copy
    /// (the full path's whole-aggregate forms all need the target to be one).
    pub(super) fn simple_ident_copy_ok(
        &mut self,
        lh: &HierarchicalIdentifier,
        n: &str,
        r: &str,
    ) -> bool {
        if r == "new"
            || self.vif_name_possible(r)
            || self.struct_capable_names().contains(r)
            || self.module.dynamic_arrays.contains(n)
            || self.instance_assoc_member(n).is_some()
        {
            return false;
        }
        let ln = self.resolve_hier_name(lh);
        if self.module.arrays.contains_key(ln.as_ref())
            || self.module.arrays_2d.contains_key(ln.as_ref())
            || self.module.arrays_nd.contains_key(ln.as_ref())
            || self.is_associative_array(&ln)
        {
            return false;
        }
        if !self.struct_root_possible(n) {
            return true;
        }
        let dst = match self.dyn_name_lookup(n) {
            Some(uq) => uq.to_string(),
            None => n.to_string(),
        };
        let struct_decl = self.module.var_decl_types.get(&dst).is_some_and(|dt| {
            matches!(self.resolve_dt_ref(dt), DataType::Struct(su) if Self::spreads_member_wise(su))
        });
        // §8.10 / §13.3: a bare name that is an INTEGRAL property of the
        // running object (`this.s`) is a scalar, not the module/package-scope
        // variable that a same-named STRUCT type may declare (`var_decl_types`
        // is keyed by the bare name and shared by every scope). UVM's
        // field-automation macros assign `s = __rdone__` for an `int s` while
        // some unrelated global `s` is a struct: without this, the whole-struct
        // copy path diverts the write and `s` silently stays 0.
        // Checked only once a struct target was found, the one answer it
        // can change.
        (!struct_decl && self.struct_copy_target(dst).1.is_none()) || self.this_integral_prop(n)
    }

    /// Does `flat_member_name(e)` evaluate nothing but plain reads? Its only
    /// evaluations are the indices of an `Index` chain (a selected
    /// identifier yields no name); a literal or a name the plain read path
    /// answers is one.
    pub(super) fn flat_name_pure(&self, e: &Expression) -> bool {
        match &e.kind {
            ExprKind::MemberAccess { expr, .. } => {
                matches!(expr.kind, ExprKind::This) || self.flat_name_pure(expr)
            }
            ExprKind::Index { expr, index } => {
                self.flat_name_pure(expr)
                    && match &index.kind {
                        ExprKind::Number(_) => true,
                        ExprKind::Ident(ih) => self.plain_ident_read(ih).is_some(),
                        _ => false,
                    }
            }
            _ => true,
        }
    }

    /// The handle held by `name`, with its declared class, when `name` is a
    /// class-handle variable of the innermost frame. Value and type both come
    /// from that one frame: its slot in `local_type_stack`, which is kept in
    /// lockstep with `local_stack`, not an outer frame's same-named local.
    /// Inside a class method the frame must be one of the method's own (see
    /// `local_class_type_of`).
    pub(super) fn frame_class_handle(&self, name: &str) -> Option<(usize, &str)> {
        let depth = self.local_stack.len().checked_sub(1)?;
        if self.method_local_base.last().is_some_and(|&b| depth < b) {
            return None;
        }
        let v = self.local_stack[depth].get(name)?;
        let cls = self.local_type_stack.get(depth)?.0.get(name)?;
        Some((v.to_u64()? as usize, cls.as_str()))
    }

    /// Whether `name` is a scalar INTEGRAL property of the running method's
    /// object (`this`), regardless of any same-named variable or struct type
    /// in module/package scope. Returns false when `name` is currently a
    /// frame local (the innermost frame shadows the property) or when no
    /// `this` is bound. Used to keep the whole-struct copy branch from
    /// diverting `s = v` when a global `s` is an unrelated struct.
    pub(super) fn this_integral_prop(&self, name: &str) -> bool {
        // A frame local shadows the property (§13.3): a scalar local under its
        // name, an unpacked-struct local under its `<name>.` marker (its
        // leaves live under `<name>.<member>`; the bare name is never a key).
        if self
            .local_stack
            .last()
            .is_some_and(|f| f.contains_key(name) || f.contains_key(&format!("{}.", name)))
        {
            return false;
        }
        let Some(Some(h)) = self.this_stack.last().copied() else {
            return false;
        };
        let Some(Some(inst)) = self.heap.get(h) else {
            return false;
        };
        let owners = self.prop_owners(&inst.class_name);
        let Some((_, cd)) = owners.get(name) else {
            return false;
        };
        let Some(prop) = cd.properties.get(name) else {
            return false;
        };
        // Collections and structs: not an integral scalar target.
        if cd.array_properties.contains_key(name)
            || cd.assoc_properties.contains_key(name)
            || cd.queue_properties.contains_key(name)
            || cd.array_nd_properties.contains_key(name)
        {
            return false;
        }
        // A class-handle / enum / covergroup-typed property is routed via
        // `class_prop_type_named`; leave it to the other guards.
        if self.class_prop_type_named(&inst.class_name, name).is_some() {
            return false;
        }
        // A struct-typed property (declared via typedef) is a real struct
        // copy target, not an integral scalar.
        if let Some(tn) = prop.type_name.as_ref() {
            if self.module.typedef_types.get(tn).is_some_and(|dt| {
                matches!(
                    Self::resolve_type_ref(dt, &self.module.typedef_types),
                    crate::ast::types::DataType::Struct(_)
                )
            }) {
                return false;
            }
        }
        true
    }

    /// Whether `name`, not a variable of the innermost frame, is a
    /// class-handle property of the running method's object — the next scope
    /// out (§8.11) after the frame, ahead of any same-named variable outside
    /// the class.
    pub(super) fn this_class_handle_prop(&self, name: &str) -> bool {
        if self
            .local_stack
            .last()
            .is_some_and(|f| f.contains_key(name))
        {
            return false;
        }
        let Some(Some(h)) = self.this_stack.last().copied() else {
            return false;
        };
        let Some(Some(inst)) = self.heap.get(h) else {
            return false;
        };
        // §8.25: a property typed by a TYPE PARAMETER (`REQ req;` of a
        // `uvm_sequence #(item)` base) is a class handle when the parameter
        // is bound to a class. Only the parameter's name came back, so the
        // property was taken for a same-named struct variable elsewhere —
        // a monitor task's `output pkt_s req` formal while it waits — and
        // `req = item::type_id::create()` spread the handle over struct
        // members, leaving `req` unchanged.
        match self.class_prop_type_named(&inst.class_name, name) {
            Some(t) if self.module.classes.contains_key(&t) => true,
            Some(t) => self.bound_prop_class(inst, name, &t).is_some(),
            None => false,
        }
    }

    /// §8.11 / §13.3: a member path through a class-handle variable of the
    /// innermost frame (`item.item`, `item.item.len`), read by following the
    /// handles. The flat name of the same spelling may belong to an unrelated
    /// struct variable elsewhere — a module-scope `item` with a member `item`
    /// — which the local name shadows (#258). `None` for anything else, or
    /// when a segment is not a plain property, so the general paths decide.
    #[inline(never)]
    pub(super) fn frame_handle_path_read(&self, expr: &Expression) -> Option<Value> {
        fn names<'a>(e: &'a Expression, out: &mut Vec<&'a str>) -> Option<()> {
            match &e.kind {
                ExprKind::Ident(h)
                    if h.root.is_none() && h.path.iter().all(|s| s.selects.is_empty()) =>
                {
                    out.extend(h.path.iter().map(|s| s.name.name.as_str()));
                    Some(())
                }
                ExprKind::MemberAccess { expr, member } => {
                    names(expr, out)?;
                    out.push(member.name.as_str());
                    Some(())
                }
                _ => None,
            }
        }
        let root = match &expr.kind {
            ExprKind::Ident(h) if h.path.len() >= 2 => h.path[0].name.name.as_str(),
            ExprKind::MemberAccess { .. } => {
                let mut e = expr;
                while let ExprKind::MemberAccess { expr: inner, .. } = &e.kind {
                    e = inner;
                }
                match &e.kind {
                    ExprKind::Ident(h) => h.path.first()?.name.name.as_str(),
                    _ => return None,
                }
            }
            _ => return None,
        };
        let (mut h, cls) = self.frame_class_handle(root)?;
        // §13.5.2: a `ref` formal is read THROUGH to the actual; the frame's
        // copy goes stale once the caller reassigns the handle while this
        // call waits. A formal aliased to module-visible storage reads the
        // handle from that storage now; one bound by identity is left to the
        // general path.
        if self.ref_redirect_possible() {
            if let Some(storage) = self.ref_alias_stack.last().and_then(|m| m.get(root)) {
                h = self.get_signal_value_by_name(storage)?.to_u64()? as usize;
            } else if self
                .ref_identity_stack
                .last()
                .is_some_and(|v| v.iter().any(|n| n == root))
            {
                return None;
            }
        }
        let mut cls = cls.to_string();
        let mut path = Vec::new();
        names(expr, &mut path)?;
        let segs = &path[1..];
        for (i, seg) in segs.iter().enumerate() {
            // §8.10 / §8.14: the member is the one the handle's DECLARED
            // class sees. A static, or a property the object's class
            // re-declares, is stored elsewhere — the general paths decide.
            let decl = self.prop_owners(&cls);
            let (owner, cd) = decl.get(*seg)?;
            if cd.static_properties.contains(*seg) {
                return None;
            }
            let inst = self.heap.get(h)?.as_ref()?;
            if self.prop_owners(&inst.class_name).get(*seg)?.0 != *owner {
                return None;
            }
            let v = inst.properties.get(*seg)?;
            if i + 1 == segs.len() {
                return Some(v.clone());
            }
            cls = self
                .class_prop_type_named(owner, seg)
                .filter(|t| self.module.classes.contains_key(t))?;
            h = v.to_u64()? as usize;
        }
        None
    }

    /// The member-wise struct type of whole-value copy target `dst`, with the
    /// name it was found under: `dst` itself or, when `dst` has no declared
    /// type, `dst` under the resolution hint.
    pub(super) fn struct_copy_target(
        &self,
        dst: String,
    ) -> (String, Option<crate::ast::types::StructUnionType>) {
        // A whole dynamic/fixed/associative ARRAY of structs is not itself a
        // struct target: `b = a` copies elements (the collection copy), and
        // spreading it member-wise would drop every element's leaves.
        if self.module.dynamic_arrays.contains(&dst)
            || self.module.arrays.contains_key(&dst)
            || self.module.associative_arrays.contains_key(&dst)
        {
            return (dst, None);
        }
        // Only a member-wise struct target needs its type owned; every
        // other assignment answers the predicate by borrow. `None`: the name
        // has no declared type at all.
        let spread_of = |sim: &Self, name: &str| {
            sim.p_elem_type_ref(name)
                .map(|dt| match sim.resolve_dt_ref(&dt) {
                    DataType::Struct(su) if Self::spreads_member_wise(su) => Some(su.clone()),
                    _ => None,
                })
        };
        let (scoped, su) = match spread_of(self, &dst) {
            Some(su) => (None, su),
            None => match self.hint_scoped(&dst) {
                Some(scoped) => match spread_of(self, &scoped) {
                    Some(su) => (Some(scoped), su),
                    None => (None, None),
                },
                None => (None, None),
            },
        };
        // §13.3 / §8.11: a class-handle variable of the running subroutine,
        // or a class-handle property of its object, is not a struct, whatever
        // a same-named variable elsewhere is declared as — `var_decl_types`
        // is keyed by the bare name and shared by every process (#258).
        // Checked only once a struct was found, the one answer it can change.
        if su.is_some()
            && (self.frame_class_handle(&dst).is_some() || self.this_class_handle_prop(&dst))
        {
            return (dst, None);
        }
        (scoped.unwrap_or(dst), su)
    }

    /// `name = <literal | unary | binary | name>` on a select-free
    /// single-segment target. Every special form the full path tests for
    /// keys on an aggregate, call, pattern or `new` RHS, on a selected or
    /// dotted target, or on a target that is an event, a virtual-interface
    /// variable, a vif property or a possibly-struct property; none can
    /// match here (a bare-name RHS adds the conditions in
    /// `simple_ident_copy_ok`), so the statement is the full path's final
    /// width-sized evaluate + assign.
    pub(super) fn simple_blocking_assign(
        &mut self,
        lvalue: &Expression,
        rvalue: &Expression,
    ) -> bool {
        // A call RHS: the full path's call forms are `new`, a queue pop and
        // an array locator, each named by the callee's last segment.
        let rhs_call = match &rvalue.kind {
            ExprKind::Call { func, .. } => {
                let callee = match &func.kind {
                    ExprKind::Ident(fh) => fh.path.last().map(|s| s.name.name.as_str()),
                    ExprKind::MemberAccess { member, .. } => Some(member.name.as_str()),
                    _ => None,
                };
                match callee {
                    Some(m)
                        if m != "new"
                            && m != "pop_front"
                            && m != "pop_back"
                            && !Self::is_locator_method(m) => {}
                    _ => return false,
                }
                true
            }
            _ => false,
        };
        let rhs_ident = match &rvalue.kind {
            ExprKind::Number(_) | ExprKind::Binary { .. } | ExprKind::Unary { .. } => None,
            ExprKind::Call { .. } => None,
            ExprKind::Ident(rh)
                if rh.path.len() == 1 && rh.root.is_none() && rh.path[0].selects.is_empty() =>
            {
                Some(rh.path[0].name.name.as_str())
            }
            _ => return false,
        };
        let ExprKind::Ident(h) = &lvalue.kind else {
            return false;
        };
        if h.path.len() != 1 || h.root.is_some() || !h.path[0].selects.is_empty() {
            return false;
        }
        let n = h.path[0].name.name.as_str();
        // §8.10 / §13.3: a struct elsewhere (another class's struct property,
        // a module struct) named like an integral property of `this` does
        // not make the bare name a struct target (see `simple_ident_copy_ok`).
        if self.module.events.contains(n)
            || self.class_member_names().vif_props.contains(n)
            || (self.struct_capable_names().contains(n) && !self.this_integral_prop(n))
            || self
                .module
                .var_decl_types
                .get(n)
                .is_some_and(|dt| self.is_virtual_iface_type(dt))
        {
            return false;
        }
        if let Some(r) = rhs_ident {
            if !self.simple_ident_copy_ok(h, n, r) {
                return false;
            }
        }
        // A call's value is spread over a member-wise struct target.
        if rhs_call && self.struct_root_possible(n) {
            let dst = match self.dyn_name_lookup(n) {
                Some(uq) => uq.to_string(),
                None => n.to_string(),
            };
            // §8.10: an integral property of `this` is a scalar target even
            // when a same-named struct exists elsewhere (see
            // `simple_ident_copy_ok`).
            if self.struct_copy_target(dst).1.is_some() && !self.this_integral_prop(n) {
                return false;
            }
        }
        let w = {
            let iw = self.infer_lhs_width(lvalue);
            if iw == 0 { 32 } else { iw }
        };
        let w = if Self::rhs_takes_context(rvalue) {
            self.assign_rhs_ctx(lvalue, w)
        } else {
            w
        };
        let val = self.eval_expr_ctx(rvalue, w);
        if rhs_call && self.assign_pending_ret_collection(lvalue) {
            return true;
        }
        self.assign_value(lvalue, &val);
        self.settle_after_proc_write();
        true
    }

    /// A member chain of select-free names ending in `this` or a one-segment
    /// identifier (`a.b.c`, `this.a`): evaluating or naming it evaluates no
    /// index and calls nothing.
    pub(super) fn member_chain_is_plain(e: &Expression) -> bool {
        match &e.kind {
            ExprKind::MemberAccess { expr, .. } => Self::member_chain_is_plain(expr),
            ExprKind::This => true,
            ExprKind::Ident(h) => h.root.is_none() && h.path.iter().all(|s| s.selects.is_empty()),
            _ => false,
        }
    }

    /// Does a member chain below its last member name a method anywhere (a
    /// parenless call when evaluated)?
    pub(super) fn chain_names_method(&self, e: &Expression) -> bool {
        match &e.kind {
            ExprKind::MemberAccess { expr, member } => {
                self.class_member_names()
                    .methods
                    .contains(member.name.as_str())
                    || self.chain_names_method(expr)
            }
            _ => false,
        }
    }

    /// `h.p = rhs` / `this.p = rhs`, `h` a select-free one-segment name and
    /// `p` a property no class can hold as a vif, a struct or a collection.
    /// The full path's special forms key on a pattern, `new`, pop, locator,
    /// stream, tagged, concatenation, conditional or struct-property RHS, on
    /// a vif, struct or collection target (the class name sets), or on a
    /// declared struct type for `h` or `h.p` (probed); none can match here,
    /// so the statement is the full path's width-sized evaluate + assign.
    pub(super) fn member_target_assign(
        &mut self,
        lvalue: &Expression,
        rvalue: &Expression,
    ) -> bool {
        let ExprKind::MemberAccess { expr: base, member } = &lvalue.kind else {
            return false;
        };
        let recv: Option<&str> = match &base.kind {
            ExprKind::This => None,
            ExprKind::Ident(h)
                if h.path.len() == 1 && h.root.is_none() && h.path[0].selects.is_empty() =>
            {
                let n = h.path[0].name.name.as_str();
                if matches!(n, "super" | "this") {
                    return false;
                }
                Some(n)
            }
            _ => return false,
        };
        // The full path's call forms are `new`, a queue pop and an array
        // locator, each named by the callee's last segment; a member or
        // dotted RHS naming a locator is a locator call too.
        let rhs_call = match &rvalue.kind {
            ExprKind::Number(_)
            | ExprKind::Null
            | ExprKind::This
            | ExprKind::Unary { .. }
            | ExprKind::Binary { .. } => false,
            ExprKind::Ident(rh) => {
                if rh.root.is_some()
                    || rh.path.len() > 2
                    || rh.path.iter().any(|s| !s.selects.is_empty())
                    || rh.path.last().is_some_and(|s| {
                        s.name.name == "new" || Self::is_locator_method(&s.name.name)
                    })
                {
                    return false;
                }
                false
            }
            ExprKind::MemberAccess {
                expr: rb,
                member: rm,
            } => {
                // A deeper chain's receiver is evaluated by the collection
                // probes too; it must be a plain read (no parenless call).
                if Self::is_locator_method(&rm.name)
                    || !Self::member_chain_is_plain(rvalue)
                    || self.chain_names_method(rb)
                {
                    return false;
                }
                false
            }
            ExprKind::Call { func, .. } => {
                let callee = match &func.kind {
                    ExprKind::Ident(fh) => fh.path.last().map(|s| s.name.name.as_str()),
                    ExprKind::MemberAccess { member, .. } => Some(member.name.as_str()),
                    _ => None,
                };
                match callee {
                    Some(m)
                        if m != "new"
                            && m != "pop_front"
                            && m != "pop_back"
                            && !Self::is_locator_method(m) => {}
                    _ => return false,
                }
                // The pop probe names a member callee's receiver; an index in
                // it would be evaluated there as well.
                if let ExprKind::MemberAccess { expr: fr, .. } = &func.kind {
                    if !Self::member_chain_is_plain(fr) {
                        return false;
                    }
                }
                true
            }
            _ => return false,
        };
        let p = member.name.as_str();
        if self.class_member_names().vif_props.contains(p)
            || self.struct_prop_name_possible(p)
            || self.coll_member_possible(p)
            || Self::receiver_prop_name(rvalue).is_some_and(|q| self.struct_prop_name_possible(q))
        {
            return false;
        }
        if let Some(n) = recv {
            if self.struct_prop_name_possible(n)
                || self.dyn_name_lookup(n).is_some()
                || self.module.classes.contains_key(n)
                || self.module.packages.contains(n)
            {
                return false;
            }
            // The whole-struct copy targets: `h.p` declared, or `h` a struct
            // with a member `p` (the flat-path type), or either under the
            // scope hint.
            let mut dst = String::with_capacity(n.len() + p.len() + 1);
            dst.push_str(n);
            dst.push('.');
            dst.push_str(p);
            if self.module.var_decl_types.contains_key(dst.as_str())
                || self
                    .module
                    .var_decl_types
                    .get(n)
                    .is_some_and(|dt| matches!(self.resolve_dt_ref(dt), DataType::Struct(_)))
            {
                return false;
            }
            if self.name_resolve_hint.borrow().is_some() && self.struct_copy_target(dst).1.is_some()
            {
                return false;
            }
        }
        let w = {
            let iw = self.infer_lhs_width(lvalue);
            if iw == 0 { 32 } else { iw }
        };
        let val = self.eval_expr_ctx(rvalue, w);
        if rhs_call {
            // The full path takes a returned collection here; a member
            // target is never its destination.
            self.pending_ret_collection = None;
        }
        self.assign_value(lvalue, &val);
        self.settle_after_proc_write();
        true
    }

    pub(super) fn exec_stmt_blocking_assign(
        &mut self,
        stmt: &Statement,
        lvalue: &Expression,
        rvalue: &Expression,
    ) {
        if (Self::recv_chain_call(rvalue).is_some() || Self::recv_chain_call(lvalue).is_some())
            && let Some(base) = self.pin_stmt_recv_calls(&[rvalue, lvalue])
        {
            // Rebuilt aggregate paths must retain the held receiver too;
            // cloning a call would give it a new memo key (§11.4.11).
            if self.is_unpacked_aggregate_operand(lvalue) || self.is_unpacked_aggregate_operand(rvalue) {
                let rvalue = self.freeze_conditional_path(rvalue, true);
                let lvalue = self.freeze_conditional_path(lvalue, true);
                self.exec_stmt_blocking_assign_body(stmt, &lvalue, &rvalue);
            } else {
                self.exec_stmt_blocking_assign_body(stmt, lvalue, rvalue);
            }
            self.recv_call_memo.truncate(base);
            return;
        }
        if let ExprKind::SystemCall { name, args } = &rvalue.kind
            && name == "$__xz_named_cast"
            && self.unpacked_bitstream_cast_assign(stmt, lvalue, args)
        {
            return;
        }
        self.exec_stmt_blocking_assign_body(stmt, lvalue, rvalue)
    }

    /// `lhs = T'(operand)` with `T` an unpacked array / queue type: assign
    /// the operand's bit stream (`unpacked_bitstream_cast_rhs`). Returns
    /// false (nothing done) for any other cast.
    #[cold]
    #[inline(never)]
    pub(super) fn unpacked_bitstream_cast_assign(
        &mut self,
        stmt: &Statement,
        lvalue: &Expression,
        args: &[Expression],
    ) -> bool {
        let Some(stream) = self.unpacked_bitstream_cast_rhs(args) else {
            return false;
        };
        self.exec_stmt_blocking_assign_body(stmt, lvalue, &stream);
        true
    }

    /// §6.24.3: a bit-stream cast to an UNPACKED array / queue type
    /// (`barr_t'(32'h01020304)`, `q8_t'(24'habcdef)`, `q8_t'(ba)`) unpacks
    /// the operand's bit stream into the target's elements, leftmost element
    /// first — exactly `{>>{operand}}` assigned to the target (§11.4.14), whose
    /// unpacking also sizes a dynamic array or queue from the stream length.
    /// The cast was a pass-through of the packed operand, which an unpacked
    /// target stored as all-zero (or x) elements. A call operand keeps the
    /// collection-return path of the cast itself.
    pub(super) fn unpacked_bitstream_cast_rhs(&self, args: &[Expression]) -> Option<Expression> {
        let inner = args.get(1)?;
        if matches!(inner.kind, ExprKind::Call { .. }) {
            return None;
        }
        let nm = self.named_cast_key(args.first()?)?;
        if !self.type_is_unpacked_collection_name(&nm) {
            return None;
        }
        Some(Expression::new(
            ExprKind::StreamOp {
                left_to_right: false,
                slice_size: None,
                exprs: vec![inner.clone()],
            },
            inner.span,
        ))
    }

    /// §7.6 / §8.3.2: copy a class/package array constant in declared order.
    fn try_class_const_array_copy(&mut self, lhs: &Expression, rhs: &Expression) -> bool {
        if !matches!(lhs.kind, ExprKind::Ident(_)) {
            return false;
        }
        let Some(leaf) = Self::receiver_prop_name(rhs) else {
            return false;
        };
        if !self.class_member_names().constant_arrays.contains(leaf)
            && !matches!(rhs.kind, ExprKind::MemberAccess { .. })
        {
            return false;
        }
        let Some(name) = self.array_operand_name(lhs) else {
            return false;
        };
        if self.module.dynamic_arrays.contains(&name) {
            return false;
        }
        let Some(&(lo, hi, _)) = self.module.arrays.get(&name) else {
            return false;
        };
        let Some(c) = self.resolve_class_scope_array_const(rhs) else {
            return false;
        };
        let count = (hi - lo + 1).min(c.hi - c.lo + 1).max(0) as usize;
        let (left, right) = self
            .module
            .unpacked_decl_dims
            .get(&name)
            .and_then(|d| d.first())
            .copied()
            .unwrap_or_else(|| {
                if self.module.descending_arrays.contains(&name) {
                    (hi, lo)
                } else {
                    (lo, hi)
                }
            });
        for i in 0..count {
            let src = if c.left > c.right {
                c.left - i as i64
            } else {
                c.left + i as i64
            };
            let dst = if left > right {
                left - i as i64
            } else {
                left + i as i64
            };
            let value = self
                .class_scope_array_const_elem(&c, src)
                .unwrap_or(Value::zero(32));
            let key = format!("{name}[{dst}]");
            let value = self.fit_block_local_array_elem(&name, &key, value);
            self.set_signal_value_by_name(&key, value);
        }
        true
    }

    pub(super) fn exec_stmt_blocking_assign_body(
        &mut self,
        stmt: &Statement,
        lvalue: &Expression,
        rvalue: &Expression,
    ) {
        if let Some(selected) = self.selected_aggregate_rhs(Some(lvalue), rvalue) {
            self.exec_stmt_blocking_assign(stmt, lvalue, &selected);
            return;
        }
        // §25.9: a static virtual-interface property binds by name before
        // any value path below stores (and returns).
        self.bind_static_vif(lvalue, rvalue);
        // §11.9: a whole-union write that is NOT a `tagged` expression moves
        // the active tag along with the value — `b = a` makes `b.Valid` a read
        // of the member `a` holds — and any other such write (a function
        // result, a pattern) drops the tag a previous `tagged` assignment
        // left, or the inactive-member check refused `b.Valid` against a
        // stale `Invalid` and raised a spurious run-time error. Ahead of the
        // simple-assign fast path, which is where `b = a` stores and returns;
        // a design without tagged unions pays the one `is_empty` test.
        if !self.active_union_tag.is_empty() && !matches!(rvalue.kind, ExprKind::Tagged { .. }) {
            if let ExprKind::Ident(lh) = &lvalue.kind {
                if lh.path.len() == 1 && lh.path[0].selects.is_empty() {
                    let lname = self.resolve_hier_name(lh);
                    let src_tag = match &rvalue.kind {
                        ExprKind::Ident(rh)
                            if rh.path.len() == 1 && rh.path[0].selects.is_empty() =>
                        {
                            let rname = self.resolve_hier_name(rh);
                            self.active_union_tag.get(&*rname).cloned()
                        }
                        _ => None,
                    };
                    match src_tag {
                        Some(tag) => {
                            self.active_union_tag.insert(lname.to_string(), tag);
                        }
                        None => {
                            self.active_union_tag.remove(&*lname);
                        }
                    }
                }
            }
        }
        if self.simple_blocking_assign(lvalue, rvalue) || self.member_target_assign(lvalue, rvalue)
        {
            return;
        }
        // (A side-effecting lvalue index is folded first, below, and
        // comes back through here.)
        if self.indexed_colls_live()
            && (self.may_name_indexed_coll(lvalue) || self.may_name_indexed_coll(rvalue))
            && !Self::lvalue_has_incdec_index(lvalue)
            && self.try_indexed_coll_assign(lvalue, rvalue)
        {
            self.settle_after_proc_write();
            return;
        }
        if (matches!(rvalue.kind, ExprKind::AssignmentPattern(_))
            || self.lvalue_may_name_struct_coll(lvalue))
            && self.try_struct_aggregate_assign(stmt, lvalue, rvalue)
        {
            self.settle_after_proc_write();
            return;
        }
        // §15.5.5: `event_var = other_event / null / q[i]` re-binds
        // the HANDLE, it does not copy a value.
        if self.try_event_handle_assign(lvalue, rvalue) {
            return;
        }
        // §11.4.1: an lvalue index with a side effect (`x[i++] = v`)
        // is evaluated exactly ONCE. The assign_value machinery
        // re-evaluates the index in several candidate branches, so
        // fold side-effecting indices to literals up front. A
        // compound assign (`x[i++] += 2`) was desugared by the parser
        // into `x[i++] = x[i++] + 2` with a CLONED lvalue (same span)
        // on the RHS — point that clone at the same folded index so
        // the increment still happens once.
        if Self::lvalue_has_incdec_index(lvalue) {
            if let Some(new_lhs) = self.rewrite_lvalue_index_side_effects(lvalue) {
                let new_rhs: Expression = match &rvalue.kind {
                    ExprKind::Binary { op, left, right }
                        if left.span == lvalue.span
                            && std::mem::discriminant(&left.kind)
                                == std::mem::discriminant(&lvalue.kind) =>
                    {
                        Expression::new(
                            ExprKind::Binary {
                                op: *op,
                                left: Box::new(new_lhs.clone()),
                                right: right.clone(),
                            },
                            rvalue.span,
                        )
                    }
                    _ => (*rvalue).clone(),
                };
                // The frozen target can still be an unpacked aggregate.
                // Keep its member-wise assignment paths instead of forcing
                // the value-only scalar store.
                self.exec_stmt_blocking_assign_body(stmt, &new_lhs, &new_rhs);
                return;
            }
        }
        // LRM §25.8 virtual-interface binding. When the LHS is a
        // class-property write whose property is declared
        // `virtual <iface_t>`, the assignment isn't a value copy:
        // we capture the RHS interface instance's NAME and store
        // it as a binding. Later `obj.vif.member` accesses look up
        // the binding and dispatch to `<bound>.<member>`. This
        // short-circuits the normal Value-copy assignment path.
        if self.try_bind_virtual_iface(lvalue, rvalue) {
            return;
        }
        // IEEE 1800-2017 §10.9.2/§10.10: an assignment pattern written
        // to an aggregate whose leaves are separate signals (unpacked
        // struct, unpacked array of them, associative array) must be
        // spread, not collapsed into one packed value. This is also
        // what makes a DECLARATION initializer work — elaboration
        // lowers `T v[int] = '{...}` to `v = '{...}` in an initial block.
        // §10.9.2: a whole-ARRAY pattern on a CLASS PROPERTY whose
        // elements are unpacked structs — expand per element so each
        // lands in the cells the READ path uses, by the same store rule
        // as the whole-element write above. Must precede the spread
        // below: `assign_class_fixed_array_pattern` writes the
        // instance-scoped SIGNAL names (`<h>#arr[i].<m>`) that nothing
        // reads back, which is what left every element 0.
        if let ExprKind::AssignmentPattern(items) = &rvalue.kind {
            if let Some((_, _, su)) = self
                .lvalue_root_is_unpacked_struct_prop(lvalue, true)
                .then(|| self.class_unpacked_array_prop_of(lvalue))
                .flatten()
            {
                let probe0 = Self::index_expr(lvalue, 0);
                if self.class_unpacked_elem_is_heap_owned(&probe0) {
                    let ord: Vec<Expression> =
                        self.pattern_ordered(items).into_iter().cloned().collect();
                    if !ord.is_empty() {
                        for (i, item) in ord.iter().enumerate() {
                            let elem = Self::index_expr(lvalue, i as i64);
                            self.assign_class_unpacked_elem(&elem, item, &su);
                        }
                        self.settle_after_proc_write();
                        return;
                    }
                }
            }
        }
        if let ExprKind::AssignmentPattern(items) = &rvalue.kind {
            let spread = self.assign_class_fixed_array_pattern(lvalue, items)
                || self.assign_class_collection_pattern(lvalue, items)
                || if matches!(&lvalue.kind, ExprKind::Ident(_)) {
                    self.pattern_aggregate_names(lvalue)
                        .into_iter()
                        .any(|n| self.assign_pattern_aggregate(&n, items))
                } else {
                    // `assoc[key] = '{...}` resolves its key precisely;
                    // everything else goes through the flattened path.
                    self.assign_coll_struct_pattern(lvalue, items)
                        || self.assign_pattern_element(lvalue, rvalue)
                        || match self.flat_member_name(lvalue) {
                            Some(flat) => self.assign_pattern_flat(&flat, items),
                            None => false,
                        }
                };
            if spread {
                self.settle_after_proc_write();
                return;
            }
        }
        // A named/ordered/default pattern written to a PACKED struct
        // must be laid out by member OFFSET, not blind-concatenated:
        // an unsized `'0` (32 bits) or any other non-member-width item
        // would otherwise shift every field. Fall through untouched when
        // the lvalue is not a registered packed struct.
        if let ExprKind::AssignmentPattern(items) = &rvalue.kind {
            if self.try_assign_packed_struct_pattern(lvalue, items) {
                self.settle_after_proc_write();
                return;
            }
            // §10.9.2: the same per-ELEMENT expansion the continuous
            // and nonblocking paths already do. Without it a BLOCKING
            // `v = '{default:1'b1};` on a packed target fell through to
            // the generic concat, which built a value out of the items'
            // own widths (one bit) instead of one item per element.
            if let Some(v) = self.packed_pattern_for_lhs(lvalue, items) {
                self.assign_value(lvalue, &v);
                self.settle_after_proc_write();
                return;
            }
        }
        // `d[i] = new[n]` on an array of dynamic arrays (§7.5.1): the
        // ELEMENT is the dynamic array. Only a bare-identifier lvalue ever
        // got its size shadow written, so `d[i].size()` reported the
        // backing buffer instead of `n`.
        // §7.5.1 dynamic-array `new[n]` / `new[n](src)`.
        //   - `new[n](src)` copies min(n, src.size()) elements from `src`
        //     and default-initialises the rest. It was ignored entirely:
        //     the size came out as 1 and the data was lost.
        //   - `d[i] = new[n]` on an array of dynamic arrays sizes the
        //     ELEMENT; only a bare-identifier lvalue ever got a size.
        // §7.5.1 dynamic-array `new[n]` and `new[n](src)`.
        //   `new[n]`      parses as Call{Ident(new), [n]}
        //   `new[n](src)` parses as Call{Call{Ident(new), [n]}, [src]}
        // The second form was never recognised: the size came out as 1
        // and the source data was lost. `new[n](src)` copies
        // min(n, src.size()) elements and default-initialises the rest.
        // A `d[i] = new[n]` lvalue sizes the ELEMENT of an array of
        // dynamic arrays.
        if let Some((n_expr, src_expr)) = Self::sized_new_parts(rvalue) {
            let target = match &lvalue.kind {
                ExprKind::Ident(lh) => Some(self.resolve_hier_name(lh)),
                // `dq[1] = new[n]` on an array-of-dynamic-arrays
                // class property: the element collection's key.
                _ => self
                    .class_array_of_coll_key(lvalue)
                    .or_else(|| self.flat_member_name(lvalue))
                    .map(std::borrow::Cow::Owned),
            };
            // A class-property dynamic array (`c.p = new[n]`)
            // lives under the instance-scoped `<handle>#p`, which
            // is never in the module-scope `dynamic_arrays` set —
            // fall back to the collection resolver so the size and
            // element cells are written. Restricted to
            // non-associative collections (dynamic arrays /
            // queues); `new[n]` is invalid for assoc arrays.
            //
            // §8.9/§10 scope: an unqualified single-segment name in a
            // class METHOD/constructor resolves to a class member of
            // `this` BEFORE an ENCLOSING (inlined) frame's same-named
            // dynamic-array local or `ref` formal. The current frame's
            // OWN local/formal shadows the member (§8.10). Without
            // this, a constructor's `value = new[1]` with `value` both
            // a member of the new object and a live `ref` formal of
            // the caller's task resolved the LHS to the caller's
            // formal and sized the CALLER's array instead of the
            // object's member (UVM 4251: burst_read's 64-element
            // `value` collapsed to 1). Prefer the `this`-scoped member
            // store over the enclosing-frame/renamed resolution below.
            let member_store: Option<std::borrow::Cow<str>> = if let ExprKind::Ident(lh) =
                &lvalue.kind
                && lh.path.len() == 1
                && lh.path[0].selects.is_empty()
            {
                let bare = &lh.path[0].name.name;
                let current_frame_local = self.dyn_name_lookup_innermost(bare).is_some();
                if !current_frame_local {
                    self.instance_assoc_member(bare)
                        .map(std::borrow::Cow::Owned)
                } else {
                    None
                }
            } else {
                None
            };
            let name = member_store.or_else(|| {
                target
                    .filter(|n| self.module.dynamic_arrays.contains(n.as_ref()))
                    .or_else(|| {
                        self.expr_assoc_name(lvalue)
                            .filter(|an| !self.is_associative_array(an))
                            .map(std::borrow::Cow::Owned)
                    })
            });
            if let Some(name) = name {
                self.fill_sized_new(&name, n_expr, src_expr);
                self.settle_after_proc_write();
                return;
            }
        }
        // §7.2.1: `b = a` between unpacked-struct VARS whose type has
        // a DYNAMIC member (queue/dynamic/assoc) — the pack/scatter
        // value path cannot carry variable-size members, so `b.d[]`
        // arrived empty. Copy member-wise (the copier now clones
        // dynamic members element-by-element).
        // Only flatten when the LHS root can possibly hold an unpacked
        // struct or an element of a struct collection; `rflat` is computed
        // lazily only once the LHS is confirmed to be a struct target.
        let l_may_be_struct = match &lvalue.kind {
            ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                let r = h.path[0].name.name.as_str();
                self.struct_root_possible(r)
                    || self.struct_prop_name_possible(r)
                    || self.coll_member_possible(r)
                    || self.dyn_name_lookup(r).is_some()
            }
            _ => {
                let _ = self.struct_root_possible("");
                !self.struct_root_names.is_empty() || !self.struct_capable_names().is_empty()
            }
        };
        let l_pure = l_may_be_struct && self.flat_name_pure(lvalue);
        // These are aggregate-shape probes, not the actual assignment.
        // Leave mutating selectors for the eventual read/write path.
        let l_probe_safe = l_may_be_struct && Self::cond_expr_is_effect_free(lvalue, false);
        let mut lflat = l_probe_safe
            .then(|| self.flat_member_name(lvalue))
            .flatten();
        let mut rflat_cache: Option<Option<String>> = None;
        let eval_rflat = |sim: &mut Self, cache: &mut Option<Option<String>>| -> Option<String> {
            if let Some(cached) = cache {
                if sim.flat_name_pure(rvalue) {
                    return cached.clone();
                }
            }
            let v = Self::cond_expr_is_effect_free(rvalue, false)
                .then(|| sim.flat_member_name(rvalue))
                .flatten();
            *cache = Some(v.clone());
            v
        };
        if let Some(dst) = &lflat {
            let su_opt = self.module.var_decl_types.get(dst).cloned().and_then(|dt| {
                match self.resolve_dt(&dt) {
                    DataType::Struct(su) if Self::spreads_member_wise(&su) => Some(su),
                    _ => None,
                }
            });
            if let Some(su) = su_opt {
                let dst = dst.clone();
                if let Some(src) = eval_rflat(self, &mut rflat_cache)
                    && dst != src
                {
                    let has_dyn = su.members.iter().any(|m| {
                        m.declarators.iter().any(|d| {
                            d.dimensions.iter().any(|dim| {
                                matches!(
                                    dim,
                                    crate::ast::types::UnpackedDimension::Unsized(_)
                                        | crate::ast::types::UnpackedDimension::Queue { .. }
                                        | crate::ast::types::UnpackedDimension::Associative { .. }
                                )
                            })
                        })
                    });
                    // A COLLECTION of structs (`ab = aa`) is not one struct:
                    // its declared type is the element's. The collection
                    // copies below own it.
                    let whole_coll = |sim: &Self, n: &str| {
                        sim.module.associative_arrays.contains_key(n)
                            || sim.module.dynamic_arrays.contains(n)
                            || sim.module.arrays.contains_key(n)
                    };
                    let has_dyn = has_dyn && !whole_coll(self, &dst) && !whole_coll(self, &src);
                    if has_dyn && self.struct_storage_exists(&src, &su) {
                        self.copy_unpacked_struct(&dst, &src, &su);
                        self.settle_after_proc_write();
                        return;
                    }
                }
            }
        }
        if !l_pure && l_probe_safe {
            lflat = self.flat_member_name(lvalue);
        }
        // `s = q.pop_front()` on a queue of unpacked structs: the popped
        // element's members must reach `s` before the queue shifts — a
        // packed return value cannot carry them.
        if let Some(dst) = &lflat {
            if let Some((obj, pop)) = self.queue_pop_call(rvalue) {
                if let Some(su) = self.queue_elem_struct(&obj) {
                    let sz = self.get_queue_size(&obj);
                    if sz > 0 {
                        let idx = if pop == "pop_front" { 0 } else { sz - 1 };
                        let src = format!("{}[{}]", obj, idx);
                        self.copy_unpacked_struct(dst, &src, &su);
                    }
                    self.eval_builtin_method(&obj, &pop, &[]);
                    self.settle_after_proc_write();
                    return;
                }
            }
        }
        // IEEE 1800-2023 §10.6: whole-struct assignment whose TARGET
        // is an unpacked-struct CLASS property (`c.first = rhs`,
        // `first = rhs.first` inside `copy`). Decompose member-wise so
        // each side is read/written by its (working) field path — the
        // RHS is never materialized as a single (broken) struct Value.
        if self
            .try_decompose_struct_class_prop_assign(lvalue, rvalue)
            .is_some()
        {
            self.settle_after_proc_write();
            return;
        }
        // §10.6 mirror case: the SOURCE is an unpacked-struct class
        // property (`p = o.orig`, incl. through a ternary — UVM's
        // factory pair copy). The whole-struct read of a class prop
        // through a frame-held handle has no flat signals to
        // assemble, so decompose member-wise from the RHS side too.
        if self
            .try_decompose_struct_class_prop_read_assign(lvalue, rvalue)
            .is_some()
        {
            self.settle_after_proc_write();
            return;
        }
        // §7.2: whole-struct write into an ELEMENT of an
        // instance-scoped collection (`slices[i] = slice;` inside a
        // class method — UVM's uvm_hdl_path_concat::add_slice). The
        // generic arm below can't resolve the container (`p_elem_type`
        // only knows module-scope names), so the copy fell to a packed
        // store and every member of the element stayed blank.
        // §7.2/§18.4: ...unless the READ path owns this element's
        // leaves. A CLASS PROPERTY that is a fixed array of unpacked
        // structs is resolved on the read side by `class_unpacked_leaf`
        // into the instance property map (`arr[i].<m>`) — which is also
        // where a per-leaf write (`arr[i].m = v`) and a scalar-struct
        // property write land. Only the whole-element path below routed
        // to the instance-scoped SIGNAL name `<h>#arr[i].<m>`, so the
        // value was written and then unreachable and every such element
        // read back 0. Ask the read resolver where the leaves live and
        // decompose into the same cells. A genuine queue/dynamic/assoc
        // element is NOT claimed by it and keeps the signal-name copy
        // below (UVM uvm_hdl_path_concat::add_slice).
        if self.lvalue_root_is_unpacked_struct_prop(lvalue, false)
            && self.queue_pop_call(rvalue).is_none()
            && self.class_unpacked_elem_is_heap_owned(lvalue)
        {
            if let Some((_, _, su)) = self.class_unpacked_array_prop_of(lvalue) {
                self.assign_class_unpacked_elem(lvalue, rvalue, &su);
                self.settle_after_proc_write();
                return;
            }
        }
        if l_may_be_struct && self.queue_pop_call(rvalue).is_none() {
            if !l_pure && l_probe_safe {
                lflat = self.flat_member_name(lvalue);
            }
            if let Some(dst) = &lflat {
                // Only a BARE element lvalue (`slices[i] = ...`) — a
                // member path (`arr[0].meta.f = v`) must fall through
                // to the leaf/aggregate arms, else a scalar write was
                // spread over the whole element.
                if let Some(cut) = dst.rfind('[').filter(|_| dst.ends_with(']')) {
                    let cont = dst[..cut].to_string();
                    if !cont.is_empty() {
                        let cont_s = self.resolve_locator_storage(&cont);
                        if let Some(su) = self.queue_elem_struct(&cont_s) {
                            // This is the actual aggregate copy, not a shape
                            // probe. Capture an impure RHS selector once so
                            // the pure name probes can retain every member.
                            if Self::lvalue_has_incdec_index(rvalue)
                                && let Some(rhs) = self.rewrite_lvalue_index_side_effects(rvalue)
                            {
                                self.exec_stmt_blocking_assign_body(stmt, lvalue, &rhs);
                                return;
                            }
                            let dst_s = format!("{}{}", cont_s, &dst[cut..]);
                            let src = eval_rflat(self, &mut rflat_cache);
                            if let Some(src) = src {
                                if self.struct_storage_exists(&src, &su) {
                                    self.copy_unpacked_struct(&dst_s, &src, &su);
                                    self.settle_after_proc_write();
                                    return;
                                }
                            }
                            if Self::is_struct_elem_ref(rvalue) {
                                self.assign_struct_memberwise(lvalue, rvalue, &su, 0);
                                self.settle_after_proc_write();
                                return;
                            }
                            let v = self.eval_expr(rvalue);
                            if self.spread_into_unpacked_struct(&dst_s, &su, &v) {
                                self.settle_after_proc_write();
                                return;
                            }
                        }
                    }
                }
            }
        }
        // IEEE 1800-2017 §7.2: assigning one unpacked struct to another
        // copies every member. Their leaves live in separate signals, so
        // evaluating the RHS to a packed value and storing it writes a
        // container nobody ever reads — `y = x` left every member of `y`
        // at X. Covers struct variables, array/queue elements and
        // struct members, in any combination.
        if !l_pure && l_probe_safe {
            lflat = self.flat_member_name(lvalue);
        }
        if let Some(dst) = lflat {
            // §7.2/§23.10: inside an INLINED child's process the lvalue
            // may still carry the child's own spelling — a reference
            // through a nested instance (`li.us2` in a non-top holder)
            // is not in `local_names`, so the inlining rewrite leaves
            // it unprefixed. Reads survive via the runtime scope hint;
            // this type lookup bypassed it, so the whole-struct copy
            // silently declined and every member stayed x — while the
            // identical code at top level worked.
            let (dst, spread_su) = self.struct_copy_target(dst);
            if let Some(su) = spread_su {
                let dst_is_collection = self.module.dynamic_arrays.contains(&dst)
                    || self.module.arrays.contains_key(&dst)
                    || self.module.associative_arrays.contains_key(&dst);
                // Freeze only after the destination is known to be a struct;
                // rejected speculative probes must not execute selectors. The
                // LHS was already frozen by the assignment-entry path above.
                if !dst_is_collection
                    && Self::lvalue_has_incdec_index(rvalue)
                    && let Some(rhs) = self.rewrite_lvalue_index_side_effects(rvalue)
                {
                    self.exec_stmt_blocking_assign_body(stmt, lvalue, &rhs);
                    return;
                }
                let src = eval_rflat(self, &mut rflat_cache);
                if let Some(src) = src {
                    if src != dst && self.struct_storage_exists(&src, &su) {
                        self.copy_unpacked_struct(&dst, &src, &su.clone());
                        self.settle_after_proc_write();
                        return;
                    }
                }
                // §7.2: an element of a CLASS-PROPERTY collection
                // (`r = q[i]` in a method, `r = c.q[i]`) has its
                // leaves under the instance, not under the flat name.
                if Self::is_struct_elem_ref(rvalue)
                    && !self.module.dynamic_arrays.contains(&dst)
                    && !self.module.arrays.contains_key(&dst)
                    && !self.module.associative_arrays.contains_key(&dst)
                {
                    self.assign_struct_memberwise(lvalue, rvalue, &su, 0);
                    self.settle_after_proc_write();
                    return;
                }
                // §13.4.1: the RHS is a VALUE, not a set of
                // named leaves — a call returning a struct, or
                // a CLASS PROPERTY whose members live in the
                // instance rather than the signal table.
                // Scatter it across the members rather than
                // storing a container nobody reads.
                // A COLLECTION target is not a single struct:
                // `q2 = q1` copies elements, and spreading one
                // packed value into `q2.<member>` would destroy
                // that.
                if !dst_is_collection
                    && matches!(
                        &rvalue.kind,
                        ExprKind::Call { .. }
                            | ExprKind::Ident(_)
                            | ExprKind::MemberAccess { .. }
                            | ExprKind::Index { .. }
                    )
                {
                    self.pending_ret_struct = None;
                    let v = self.eval_expr(rvalue);
                    if self.spread_into_unpacked_struct(&dst, &su.clone(), &v) {
                        if self.pending_ret_struct.is_some() {
                            self.copy_ret_struct_colls(&dst, &su);
                        }
                        self.settle_after_proc_write();
                        return;
                    }
                }
            }
        }
        // `collection[key] = new(...)`: a bare `new` assigned to an
        // associative-array / array ELEMENT of class type. The element's
        // class comes from the COLLECTION's element type (keyed by the
        // base name), not from a var-named lvalue — so the generic
        // `new`-resolution (which keys off `var_class_types[lhs_var]`)
        // can't see it and stores X. Handles context-keyed array element
        // allocations (`m_rsc[cntxt] = new`); without this the stored element
        // remains X/null.
        if let ExprKind::Index { expr: base, .. } = &lvalue.kind {
            let is_new = match &rvalue.kind {
                ExprKind::Call { func, .. } => matches!(&func.kind,
                    ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].name.name == "new"),
                ExprKind::Ident(h) => h.path.len() == 1 && h.path[0].name.name == "new",
                _ => false,
            };
            if is_new {
                // §15.3/§15.4: the element type may be a MAILBOX or
                // SEMAPHORE rather than a class. Everything below
                // resolves a class and gives up when it finds none, so
                // `mb[i] = new()` on a container array fell through
                // leaving the element unallocated — a non-null handle
                // with nothing behind it, so every put vanished.
                // Allocate here, before the class walk.
                if let Some(kind) = self.lvalue_container_kind(lvalue) {
                    let handle = self.heap.len();
                    self.heap.push(Some(Box::new(ClassInstance {
                        class_name: kind.to_string(),
                        properties: PropMap::default(),
                        type_bindings: HashMap::default(),
                        spec: None,
                        creation_scope: self.active_instance_scope(),
                    })));
                    let cargs: &[Expression] = match &rvalue.kind {
                        ExprKind::Call { args, .. } => args,
                        _ => &[],
                    };
                    if kind == "semaphore" {
                        let n = cargs
                            .first()
                            .map(|a| self.eval_expr(a).to_u64().unwrap_or(0) as i64)
                            .unwrap_or(0);
                        self.semaphores.insert(handle, n);
                    } else {
                        self.mailboxes
                            .insert(handle, std::collections::VecDeque::new());
                        self.record_mailbox_bound(handle, cargs);
                    }
                    self.assign_value(lvalue, &Value::from_u64(handle as u64, 32));
                    self.settle_after_proc_write();
                    return;
                }
                // Walk a chained-Index base (`foo[i][j] = new(...)` on a
                // multi-dim member) to the root Ident — the element
                // class is keyed by the base collection's name.
                let mut root = base.as_ref();
                while let ExprKind::Index { expr: inner, .. } = &root.kind {
                    root = inner.as_ref();
                }
                // §4c/: `obj.member[k] = new(...)` — the collection is
                // a MEMBER of a class handle (`mid.base[i] = new(...)`
                // in do_unpack, or the flattened nested
                // `a.mid.base[i] = new(...)`). The root unwraps to a
                // MemberAccess or a flattened >=3-seg Ident. Resolve
                // the element class from the RECEIVER's runtime class
                // property so the bare `new(...)` constructs the
                // declared element type instead of the enclosing
                // `this` class (which would run a component ctor and
                // trip the UVM ILLCRT guard deep inside the library).
                let recv_member: Option<(Expression, String)> = match &root.kind {
                    ExprKind::MemberAccess { expr: recv, member } => {
                        Some(((**recv).clone(), member.name.clone()))
                    }
                    ExprKind::Ident(ih) if ih.path.len() >= 3 => {
                        let mut p = ih.clone();
                        let member = p.path.pop().unwrap().name.name.clone();
                        p.cached_signal_id = std::cell::Cell::new(None);
                        Some((Expression::new(ExprKind::Ident(p), lvalue.span), member))
                    }
                    _ => None,
                };
                if let Some((recv, member_name)) = recv_member {
                    let this_recv = matches!(&recv.kind, ExprKind::Ident(rh)
                        if rh.path.len() == 1 && rh.path[0].name.name == "this");
                    let handle = if this_recv {
                        self.this_stack.last().copied().flatten().unwrap_or(0)
                    } else {
                        self.eval_handle_expr(&recv).unwrap_or(0)
                    };
                    if handle != 0 {
                        let hcn = self
                            .heap
                            .get(handle)
                            .and_then(|o| o.as_ref())
                            .map(|i| i.class_name.clone());
                        let mut mtn: Option<String> = None;
                        let mut cur = hcn;
                        while let Some(cn) = cur {
                            if let Some(cd) = self.module.classes.get(&cn) {
                                if let Some(tn) = cd
                                    .properties
                                    .get(&member_name)
                                    .and_then(|s| s.type_name.clone())
                                {
                                    mtn = Some(tn);
                                    break;
                                }
                                cur = cd.extends.clone();
                            } else {
                                break;
                            }
                        }
                        // §8.25: an element type that is a class type
                        // parameter takes this object's binding (#576).
                        if mtn
                            .as_deref()
                            .is_some_and(|t| !self.module.classes.contains_key(t))
                            && let Some(b) = self.prop_class_type(handle, &member_name)
                        {
                            mtn = Some(b);
                        }
                        if let Some(mcls) = mtn.as_deref().and_then(|t| {
                            self.resolve_typedef_spec(t)
                                .map(|(b, s)| (b.clone(), Some((b, s))))
                                .or_else(|| {
                                    self.resolve_simple_typedef_class(t)
                                        .filter(|c| self.module.classes.contains_key(c))
                                        .map(|c| (c, None))
                                })
                                .or_else(|| {
                                    self.extract_spec_from_string(t)
                                        .filter(|(b, _)| self.module.classes.contains_key(b))
                                        .map(|(b, s)| (b.clone(), Some((b, s))))
                                })
                                .or_else(|| {
                                    self.module
                                        .classes
                                        .contains_key(t)
                                        .then(|| (t.to_string(), None))
                                })
                        }) {
                            let ctor_args: &[Expression] = match &rvalue.kind {
                                ExprKind::Call { args, .. } => args,
                                _ => &[],
                            };
                            let (cn, spec) = mcls;
                            if let Some(cd) = self.module.classes.get(&cn).cloned() {
                                let v = if let Some((b, s)) = spec {
                                    let saved = self.current_spec.take();
                                    self.current_spec = Some((b, s));
                                    let r = self.instantiate_class(&cd, ctor_args);
                                    self.current_spec = saved;
                                    r
                                } else {
                                    self.instantiate_class(&cd, ctor_args)
                                };
                                self.assign_value(lvalue, &v);
                                return;
                            }
                        }
                    }
                }
                if let ExprKind::Ident(bh) = &root.kind {
                    let bname = bh
                        .path
                        .last()
                        .map(|s| s.name.name.clone())
                        .unwrap_or_default();
                    // §4c: `h.assoc[k] = new` — the collection is a
                    // property of the object `h` (a handle variable,
                    // module scope included). Resolve the element
                    // class from the RECEIVER's runtime class; the
                    // maps below only cover module-scope collections
                    // and the current class context.
                    let recv_elem_cls: Option<String> =
                        if bh.path.len() == 2 && bh.path[0].selects.is_empty() {
                            let recv = self
                                .eval_ident_handle(&bh.path[0].name.name)
                                .filter(|&h| h != 0);
                            // §8.25: an element type that is a class type
                            // parameter takes the receiver's binding (#576).
                            let bound = recv.and_then(|h| {
                                self.prop_class_type(h, &bname)
                                    .filter(|_| !self.module.classes.contains_key(&bname))
                            });
                            bound.or_else(|| recv
                                .and_then(|h| {
                                    self.heap
                                        .get(h)
                                        .and_then(|o| o.as_ref())
                                        .map(|i| i.class_name.clone())
                                })
                                .and_then(|mut cn| {
                                    loop {
                                        let cd = self.module.classes.get(&cn)?;
                                        if let Some(tn) = cd
                                            .properties
                                            .get(&bname)
                                            .and_then(|sig| sig.type_name.clone())
                                        {
                                            break Some(tn);
                                        }
                                        match cd.extends.clone() {
                                            Some(p) => cn = p,
                                            None => break None,
                                        }
                                    }
                                }))
                        } else {
                            None
                        };
                    // §8.10: when the collection is a MEMBER of the
                    // enclosing class, its declared element type wins
                    // over any global bare-name lookup.
                    // `declared_collection_elem_class` matches
                    // `var_decl_types` by bare/leaf name across the
                    // whole design, so `pool[key] = new(key)` inside
                    // uvm_pool::get picked up an unrelated `pool`
                    // declared elsewhere and constructed THAT class —
                    // UVM's hdl-path pool handed back a uvm_pool where
                    // a uvm_queue belonged, and every backdoor path
                    // read empty.
                    let member_elem_cls: Option<String> = {
                        let mut found = None;
                        let mut cur = self.class_context_stack.last().cloned().flatten();
                        let mut seen: HashSet<String> = HashSet::default();
                        while let Some(cn) = cur {
                            if !seen.insert(cn.clone()) {
                                break;
                            }
                            match self.module.classes.get(&cn) {
                                Some(cd) => {
                                    if let Some(tn) =
                                        cd.properties.get(&bname).and_then(|s| s.type_name.clone())
                                    {
                                        found = Some(tn);
                                        break;
                                    }
                                    cur = cd.extends.clone();
                                }
                                None => break,
                            }
                        }
                        found
                    };
                    // §8.10 precedence: a true frame-LOCAL shadows a
                    // class member, which in turn shadows any
                    // global-by-bare-name map. `var_class_types` and
                    // `declared_collection_elem_class` are both keyed
                    // by bare name across the whole design, so
                    // `pool[key] = new(key)` inside uvm_pool::get
                    // picked up an unrelated `pool` declared elsewhere
                    // and constructed THAT class — UVM's hdl-path pool
                    // handed back a uvm_pool where a uvm_queue
                    // belonged, and every backdoor path read empty.
                    // A method-LOCAL collection stores under a
                    // per-process RENAMED key; its declared element
                    // type shadows a same-named class member (§8.10).
                    let local_elem_cls = self.local_class_type_of(&bname).or_else(|| {
                        self.dyn_name_lookup(&bname)
                            .map(str::to_string)
                            .and_then(|rn| {
                                self.module
                                    .array_elem_class
                                    .get(&rn)
                                    .cloned()
                                    .or_else(|| self.declared_collection_elem_class(&rn))
                            })
                    });
                    let elem_cls = recv_elem_cls
                        .or_else(|| local_elem_cls)
                        .or_else(|| member_elem_cls)
                        .or_else(|| self.module.array_elem_class.get(&bname).cloned())
                        .or_else(|| self.var_class_types.get(&bname).cloned())
                        .or_else(|| self.declared_collection_elem_class(&bname))
                        .or_else(|| {
                            // A method-LOCAL collection stores under a
                            // per-process RENAMED key — consult it too.
                            self.dyn_name_lookup(&bname)
                                .map(str::to_string)
                                .and_then(|rn| {
                                    self.module
                                        .array_elem_class
                                        .get(&rn)
                                        .cloned()
                                        .or_else(|| self.declared_collection_elem_class(&rn))
                                })
                        })
                        .or_else(|| {
                            // Class-MEMBER collection (static or instance):
                            // resolve the element class from the current
                            // class's property type, walking the hierarchy.
                            // This is static `m_rsc[cntxt]=new`
                            // (a pool per context) — the local/array
                            // maps above don't cover class members.
                            let mut cur = self.class_context_stack.last().cloned().flatten();
                            while let Some(cn) = cur {
                                if let Some(cd) = self.module.classes.get(&cn) {
                                    if let Some(tn) =
                                        cd.properties.get(&bname).and_then(|s| s.type_name.clone())
                                    {
                                        return Some(tn);
                                    }
                                    cur = cd.extends.clone();
                                } else {
                                    break;
                                }
                            }
                            None
                        });
                    // `obj.p[i] = new(...)` from OUTSIDE a class
                    // method: resolve `obj`'s handle (incl. `this`),
                    // read its class, and look up the property's
                    // element type. Sequential (not an or_else
                    // closure) because it needs `&mut self`.
                    let elem_cls = if elem_cls.is_some() {
                        elem_cls
                    } else if bh.path.len() >= 2 {
                        let obj_name = bh.path[0].name.name.clone();
                        let member = bh.path.last().unwrap().name.name.clone();
                        let handle = if obj_name == "this" {
                            self.this_stack.last().copied().flatten().unwrap_or(0)
                        } else {
                            self.eval_ident_handle(&obj_name).unwrap_or(0)
                        };
                        let mut found = None;
                        if handle != 0 {
                            let hcn = self
                                .heap
                                .get(handle)
                                .and_then(|x| x.as_ref())
                                .map(|i| i.class_name.clone());
                            let mut cur = hcn;
                            while let Some(cn) = cur {
                                if let Some(cd) = self.module.classes.get(&cn) {
                                    if let Some(tn) =
                                        cd.properties.get(&member).and_then(|s| s.type_name.clone())
                                    {
                                        found = Some(tn);
                                        break;
                                    }
                                    cur = cd.extends.clone();
                                } else {
                                    break;
                                }
                            }
                        }
                        found
                    } else {
                        None
                    };
                    // The element type is often a TYPEDEF alias (e.g.
                    // `typedef elem_pool pool_t;` or a parameterized alias).
                    // The maps above record the TYPEDEF name, which isn't a key in
                    // `module.classes` — so filtering by `contains_key`
                    // alone rejected it, `new` was dropped, and the
                    // stored element had no resolvable class. Method calls
                    // on it (`add`/`num`/`exists`) then silently no-op'd,
                    // breaking assoc-array-of-typedef'd-class construction.
                    // Resolve the typedef to the underlying class, and
                    // carry a `(base, sig)` spec for parameterized
                    // typedefs so the element's type-param members bind.
                    let elem_cls: Option<(String, Option<(String, String)>)> = match elem_cls {
                        Some(cn) if self.module.classes.contains_key(&cn) => Some((cn, None)),
                        // §6.20.3: the declared element type is a
                        // class TYPE PARAMETER (`PARAM pool[string]`
                        // in generic `class Base#(type PARAM)` —
                        // i.e. `uvm_pool`'s `T`). It is neither a
                        // module.classes key nor a typedef; bind it
                        // to the concrete class via the active
                        // instance/spec so `pool[key] = new` builds
                        // the right element class (e.g. `myitem` for
                        // `Base#(myitem)`, or `uvm_event#(uvm_object)`
                        // for `uvm_object_string_pool`).
                        //
                        // evaluate a single time (the bound class's
                        // keys would falsify the `contains_key(c)`
                        // guard that the typedef arms rely on).
                        Some(cn)
                            if self
                                .resolve_type_param_binding(&cn)
                                .map(|b| {
                                    let base = self
                                        .extract_spec_from_string(&b)
                                        .map(|(base, _)| base)
                                        .unwrap_or(b);
                                    self.module.classes.contains_key(&base)
                                })
                                .unwrap_or(false) =>
                        {
                            let bound = self.resolve_type_param_binding(&cn).unwrap();
                            let tup = self
                                .extract_spec_from_string(&bound)
                                .unwrap_or_else(|| (bound.clone(), String::new()));
                            let (base, sig) = tup;
                            let spec = if sig.is_empty() {
                                None
                            } else {
                                Some((base.clone(), sig))
                            };
                            Some((base, spec))
                        }
                        Some(cn) => self
                            .resolve_simple_typedef_class(&cn)
                            .filter(|c| self.module.classes.contains_key(c))
                            .map(|c| (c, None))
                            .or_else(|| {
                                self.resolve_typedef_spec(&cn)
                                    .filter(|(b, _)| self.module.classes.contains_key(b))
                                    .map(|(b, s)| (b.clone(), Some((b, s))))
                            })
                            .or_else(|| {
                                // §8.25: the binding is itself
                                // a SPECIALIZED name
                                // (`queue#(string)` — UVM's
                                // string pool of hdl-path
                                // queues).
                                self.extract_spec_from_string(&cn)
                                    .filter(|(b, _)| self.module.classes.contains_key(b))
                                    .map(|(b, s)| (b.clone(), Some((b, s))))
                            }),
                        None => None,
                    };
                    if let Some((cn, spec)) = elem_cls {
                        let ctor_args: &[Expression] = match &rvalue.kind {
                            ExprKind::Call { args, .. } => args,
                            _ => &[],
                        };
                        // `arr[i] = new src` is the §8.12 SHALLOW COPY
                        // constructor, not construction. This block used
                        // to match any `new` and hand the source to
                        // `instantiate_class` as a constructor argument,
                        // where it was ignored: the element became a
                        // freshly default-constructed object, with newly
                        // allocated nested handles instead of the
                        // source's. The same guards as the plain-handle
                        // path (`h = new src`): exactly one argument,
                        // and it must be a STATICALLY class-typed
                        // handle expression naming a live object —
                        // never `new(size)` / `new(int_var)` (an int
                        // whose value collides with a live heap index
                        // is construction, not a copy).
                        let arg_could_be_handle = ctor_args
                            .first()
                            .map(|a| {
                                matches!(
                                    &a.kind,
                                    ExprKind::Ident(_)
                                        | ExprKind::MemberAccess { .. }
                                        | ExprKind::Index { .. }
                                ) && self.expr_is_class_handle(a)
                            })
                            .unwrap_or(false);
                        if ctor_args.len() == 1 && arg_could_be_handle {
                            let src_h =
                                self.eval_expr(&ctor_args[0]).to_u64().unwrap_or(0) as usize;
                            if src_h != 0 && matches!(self.heap.get(src_h), Some(Some(_))) {
                                let h = self.copy_construct(src_h);
                                self.assign_value(lvalue, &h);
                                return;
                            }
                        }
                        if let Some(cd) = self.module.classes.get(&cn).cloned() {
                            // The collection's element may be a
                            // DIRECTLY parameterized class
                            // (`param_obj #(int) m_aa[string]`). The
                            // base-class maps above record only
                            // `param_obj`; recover the `#(int)` args
                            // from the collection's declared element
                            // type and bind them so the instance
                            // records the correct specialization
                            // (its `type_name`/`get_object_type`),
                            // mirroring the plain `lvalue = new()`
                            // path via `instantiate_class_with_type_args`.
                            let mut ta: Option<Vec<Expression>> = None;
                            if let Some(dt) = self.module.var_decl_types.get(&bname).or_else(|| {
                                self.module.var_decl_types.get(
                                    &self
                                        .dyn_name_lookup(&bname)
                                        .map(str::to_string)
                                        .unwrap_or_default(),
                                )
                            }) {
                                if let crate::ast::types::DataType::TypeReference {
                                    type_args,
                                    ..
                                } = dt
                                {
                                    if !type_args.is_empty() {
                                        ta = Some(type_args.clone());
                                    }
                                }
                            }
                            if ta.is_none() {
                                ta = self
                                    .module
                                    .class_type_args
                                    .get(&bname)
                                    .cloned()
                                    .or_else(|| self.this_property_type_args(&bname))
                                    .or_else(|| self.var_type_args.get(&bname).cloned());
                            }
                            let v = if let Some(ta_args) = ta {
                                self.instantiate_class_with_type_args(
                                    &cd,
                                    ctor_args,
                                    Some(ta_args.as_slice()),
                                )
                            } else if let Some((b, s)) = spec {
                                let saved = self.current_spec.take();
                                self.current_spec = Some((b, s));
                                let r = self.instantiate_class(&cd, ctor_args);
                                self.current_spec = saved;
                                r
                            } else {
                                self.instantiate_class(&cd, ctor_args)
                            };
                            self.assign_value(lvalue, &v);
                            return;
                        }
                    }
                }
            }
        }
        // Dynamic-array sizing: `arr = new[size]`. The postfix `[size]`
        // parses as `Call{ Ident(new), [size] }`. When the lvalue is a
        // dynamic array (not a class handle — that would be a `new obj`
        // copy), register `size` zero-initialised elements.
        if let ExprKind::Call { func, args } = &rvalue.kind {
            let is_new = matches!(&func.kind, ExprKind::Ident(h)
                if h.path.len() == 1 && h.path[0].name.name == "new");
            if is_new && args.len() == 1 {
                if let ExprKind::Ident(lh) = &lvalue.kind {
                    let mut lname = self.resolve_hier_name(lh);
                    if !(self.module.dynamic_arrays.contains(&*lname)
                        || self.module.arrays.contains_key(&*lname))
                    {
                        // A class-member dynamic array reached either
                        // via the CURRENT instance (`base` → this
                        // context) or through an OBJECT HANDLE path
                        // (`a.mid.base`, a flattened 3+ segment Ident).
                        // Per-instance storage is `<handle>#member`,
                        // not a module-scope array. Resolve that
                        // scoped name so `= new[size]` sizes the right
                        // instance's collection.
                        let member = lh.path.last().map(|s| s.name.name.clone());
                        let scoped: Option<String> = self
                            .instance_assoc_member(&lname)
                            .or_else(|| {
                                // Receiver path = all-but-last segment:
                                // `a.mid.base` → receiver `a.mid`.
                                let recv: Option<Expression> = if lh.path.len() >= 2 {
                                    let p = hier_prefix(lh);
                                    Some(Expression::new(ExprKind::Ident(p), lvalue.span))
                                } else {
                                    None
                                };
                                let h = recv
                                    .as_ref()
                                    .and_then(|r| self.eval_handle_expr(r))
                                    .unwrap_or(0);
                                let m = member.clone().unwrap_or_default();
                                if h != 0 {
                                    self.handle_collection_name(h, &m)
                                } else {
                                    None
                                }
                            })
                            .or_else(|| self.expr_assoc_name(lvalue));
                        if let Some(s) = scoped {
                            lname = std::borrow::Cow::Owned(s);
                        }
                    }
                    if self.module.dynamic_arrays.contains(&*lname)
                        || self.module.arrays.contains_key(&*lname)
                        || lname.contains('#')
                    {
                        let size = self.eval_expr(&args[0]).to_u64().unwrap_or(0);
                        let w = self.module.arrays.get(&*lname).map(|t| t.2).unwrap_or(32);
                        // Drop any stale elements, then size + zero-fill.
                        let prefix = format!("{}[", lname);
                        let stale: Vec<String> = self.signals.keys_with_elem_prefix(&prefix);
                        for k in stale {
                            self.signals.remove(&k);
                        }
                        self.module.dynamic_arrays.insert(lname.to_string());
                        self.module
                            .arrays
                            .entry(lname.to_string())
                            .or_insert((0, 63, w));
                        // A STRUCT-element array keeps each member in its
                        // own leaf signal (`arr[i].path`); registering a
                        // packed container instead made every later
                        // member read slice a zero container while the
                        // member write went to the leaf — `slices[t].path`
                        // read blank (UVM's uvm_hdl_path_concat).
                        let elem_su = self.queue_elem_struct(&lname);
                        for i in 0..size {
                            let elem = format!("{}[{}]", lname, i);
                            match &elem_su {
                                Some(su) => {
                                    for (k, lw, is_real) in
                                        self.unpacked_struct_leaf_keys(&elem, su)
                                    {
                                        let sv = if is_real {
                                            Value::from_f64(0.0)
                                        } else {
                                            Value::zero(lw)
                                        };
                                        self.signals.insert(k, sv);
                                    }
                                }
                                None => {
                                    self.set_signal_value_by_name(&elem, Value::zero(w));
                                }
                            }
                        }
                        self.set_queue_size(&lname, size);
                        self.settle_after_proc_write();
                        return;
                    }
                }
            }
        }
        // Tagged union assignment: un = tagged Name (inner);
        if let ExprKind::Tagged { tag, inner } = &rvalue.kind {
            if let ExprKind::Ident(lh) = &lvalue.kind {
                let lname = self.resolve_hier_name(lh);
                // §7.3.2: the pattern literal `tagged '{ member: value }`
                // carries no member name between `tagged` and `'{` — the
                // tag IS the first named item of the pattern. Without it
                // `active_union_tag` stays empty and every later
                // `u.member` read (tagged_union_member) returns x.
                let tag_name = if tag.name.is_empty() {
                    inner.as_ref().map_or(String::new(), |ie| {
                        if let ExprKind::AssignmentPattern(items) = &ie.kind {
                            items
                                .iter()
                                .find_map(|it| match it {
                                    AssignmentPatternItem::Named(id, _) => Some(id.name.clone()),
                                    _ => None,
                                })
                                .unwrap_or_default()
                        } else {
                            String::new()
                        }
                    })
                } else {
                    tag.name.clone()
                };
                self.active_union_tag.insert(lname.to_string(), tag_name);
                if let Some(inner_expr) = inner {
                    let v = self.eval_expr(inner_expr);
                    self.set_signal_value_by_name(&lname, v);
                } else {
                    let w = self.lookup_signal_width(&lname).unwrap_or(1);
                    self.set_signal_value_by_name(&lname, Value::zero(w));
                }
                self.settle_after_proc_write();
                return;
            }
        }
        // String-typed lvalue assigned a concatenation:
        // `str = {a, b, ...}` where `str` is a declared `string`.
        // The lvalue type forces string (byte) concatenation even
        // when the operands look like packed bit vectors — otherwise
        // ' ' (0x20) would shift into '@' (0x40) and produce garbage.
        if let (ExprKind::Ident(lh), ExprKind::Concatenation(parts)) = (&lvalue.kind, &rvalue.kind)
        {
            let lname = self.resolve_hier_name(lh);
            // Take the byte-concat path when EITHER the lvalue is a
            // declared `string` OR the RHS is itself string-valued (a
            // concat containing a string literal/operand). The latter is
            // needed because a `string` local declared inside a class
            // method isn't always registered in `string_signals`, so
            // `lk = {a, "_delimiter_", b}` would otherwise fall to
            // numeric bit-concat and read as "".
            // A string QUEUE is in `string_signals` too (so its
            // elements render as strings), but `q = {...}` on it is a
            // QUEUE assignment — `q = {}` clears it — not a byte concat.
            let in_frame = matches!(self.local_stack.last(), Some(f) if f.contains_key(&*lname));
            let lhs_is_queue = self.module.dynamic_arrays.contains(&*lname)
                || self.module.arrays.contains_key(&*lname)
                || self
                    .instance_assoc_member(&lname)
                    .is_some_and(|scoped| !in_frame && self.class_queue_prop(&scoped));
            if !lhs_is_queue
                && (self.string_signals.contains(&*lname)
                    || self.string_signals.contains(
                        &lh.path
                            .last()
                            .map(|s| s.name.name.clone())
                            .unwrap_or_default(),
                    )
                    || self.expr_is_string_valued(rvalue))
            {
                let mut s = String::new();
                for p in parts {
                    let pv = self.eval_expr(p);
                    s.push_str(&pv.to_sv_string());
                }
                let sval = Value::from_string(&s);
                // Store the byte-concat result with correct scoping:
                //  - a method-LOCAL string lives in the current call frame
                //    (VarDecl frame isolation), and reads consult the frame
                //    BEFORE signals, so write the frame slot directly;
                //  - otherwise route through `assign_value`, which scopes a
                //    class MEMBER lvalue to its instance (`<handle>#m_name`)
                //    — writing the bare name lost `component::m_name =
                //    {m_parent.get_full_name(), ".", get_name()}`, leaving
                //    every non-top component's get_full_name() empty.
                if self
                    .local_stack
                    .last()
                    .is_some_and(|f| f.contains_key(&*lname))
                {
                    self.local_stack
                        .last_mut()
                        .unwrap()
                        .insert(lname.to_string(), sval);
                } else {
                    self.assign_value(lvalue, &sval);
                }
                self.settle_after_proc_write();
                return;
            }
        }
        // Class associative-array member element write
        // (`m[key] = rhs` or `obj.m[key] = rhs`) — route through
        // `assign_value`, which maps the base to its instance-
        // scoped store.
        if let ExprKind::Index { expr: base, .. } = &lvalue.kind {
            if self.expr_assoc_name(base).is_some() {
                let v = self.eval_expr(rvalue);
                self.assign_value(lvalue, &v);
                self.settle_after_proc_write();
                return;
            }
        }
        // Whole associative-array copy between class members
        // (`a.m = b.m`) — clear the destination, copy every entry.
        // §13.5.2: a bare-ident operand that names the CURRENT call's
        // own queue/dyn-array formal (bound in the top `local_dyn`
        // frame by `bind_queue_param`) must not be hijacked here by a
        // same-named property — `expr_assoc_name` resolves it to
        // `<handle>#member`, so `get_value_array(ref u64_t value[])`
        // with body `value = this.value` resolved BOTH sides to the
        // property and self-copied (the formal's ref writeback stayed
        // empty), and the mirror `this.value = value` did the same in
        // reverse (the property kept its stale content). Decline so
        // the statement reaches the `resolve_coll` ladder, which
        // prefers the current frame's formal over the property.
        let bare_is_current_formal = |e: &Expression| {
            matches!(e.kind, ExprKind::Ident(ref lh)
            if lh.path.len() == 1
                && self
                    .local_dyn
                    .last()
                    .is_some_and(|f| f.iter().any(|(k, _)| k == &lh.path[0].name.name)))
        };
        // §7.9.9: a whole associative-array copy with a class property on ONE
        // side and a plain module, local or formal associative variable on the
        // other (`h.m = src`, `dst = h.m`, `this.m = s` with `s` a formal).
        // Neither this path (which wanted a property on both sides) nor the
        // plain-variable copy (which wants a plain name on the left) took it,
        // so it fell to a scalar assignment that copied no entries. A bare
        // formal resolves through the call frame, never to a same-named
        // property (see above).
        let (lf, rf) = (
            bare_is_current_formal(lvalue),
            bare_is_current_formal(rvalue),
        );
        let formal_operand = lf || rf;
        let (l_assoc, r_assoc) = if formal_operand {
            (None, None)
        } else {
            (self.expr_assoc_name(lvalue), self.expr_assoc_name(rvalue))
        };
        // §7.9.9 / §7.10.2.5: a whole associative-array copy with a plain
        // variable on one side, or a popped associative element (see
        // `assoc_copy_with_plain_side`).
        if !self.module.associative_arrays.is_empty()
            && (formal_operand
            || l_assoc.is_some() != r_assoc.is_some()
            // §7.9.9: plain associative assignment replaces all entries too.
            || (l_assoc.is_none() && r_assoc.is_none()
                && matches!(&lvalue.kind, ExprKind::Ident(h) if h.path.len() == 1
                    && self.module.associative_arrays.contains_key(
                        self.dyn_name_lookup(&h.path[0].name.name).unwrap_or(&h.path[0].name.name)))
                && matches!(&rvalue.kind, ExprKind::Ident(_)))
            || matches!(
                &rvalue.kind,
                ExprKind::Call { func, args }
                    if args.is_empty()
                        && self.callee_method_name_is(func, |m| matches!(m, "pop_front" | "pop_back"))
            ))
            && self.assoc_copy_with_plain_side(lvalue, rvalue, lf, rf, &l_assoc, &r_assoc)
        {
            return;
        }
        if !formal_operand {
            if let (Some(dst), Some(src)) = (l_assoc, r_assoc) {
                // A QUEUE / DYNAMIC-ARRAY copy (`a.q = b.q`) must copy
                // elements + size, not re-key assoc entries. Distinguish
                // by whether the collection is a true assoc array.
                let dst_is_assoc = self.is_associative_array(&dst);
                if !dst_is_assoc
                    && (self.module.dynamic_arrays.contains(&dst)
                        || self.signals.contains_key(&format!("{}.size", dst)))
                {
                    let sz = self.get_queue_size(&src);
                    // Clear stale dst elements beyond the new size.
                    let old_sz = self.get_queue_size(&dst);
                    for i in sz..old_sz {
                        self.signals.remove(&format!("{}[{}]", dst, i));
                    }
                    // An unpacked-struct element has no `<src>[i]` signal,
                    // only its member leaves.
                    let member_wise = self.queue_elem_struct(&src).is_some();
                    for i in 0..sz {
                        if member_wise {
                            self.queue_copy_elem(&src, i, &dst, i);
                        } else if let Some(v) =
                            self.get_signal_value_by_name(&format!("{}[{}]", src, i))
                        {
                            self.set_signal_value_by_name(&format!("{}[{}]", dst, i), v);
                        }
                    }
                    self.set_queue_size(&dst, sz);
                    if !self.in_edge_block {
                        self.settle_combinatorial();
                    }
                    return;
                }
                let dst_prefix = format!("{}[", dst);
                let dst_keys: Vec<String> = self.signals.keys_with_elem_prefix(&dst_prefix);
                for k in dst_keys {
                    self.signals.remove(&k);
                }
                // Re-key each `<src>[k]` entry as `<dst>[k]` — the
                // suffix from `src.len()` already includes `[k]`.
                let src_prefix = format!("{}[", src);
                let entries: Vec<(String, Value)> = self
                    .signals
                    .keys_with_elem_prefix(&src_prefix)
                    .into_iter()
                    .filter_map(|k| {
                        let v = self.signals.get(&k)?.clone();
                        Some((format!("{}{}", dst, &k[src.len()..]), v))
                    })
                    .collect();
                for (k, v) in entries {
                    self.signals.insert(k, v);
                }
                return;
            }
        } // !formal_operand
        // Slice assignment for N-dimensional unpacked arrays where LHS
        // and RHS both supply fewer indices than dimensions:
        //   B[i][j] = A[p][q];   // with A, B both 3D ⇒ copy inner dim
        {
            fn unwrap_nd(e: &Expression) -> (&Expression, Vec<&Expression>) {
                let mut cur = e;
                let mut rev_idxs: Vec<&Expression> = Vec::new();
                while let ExprKind::Index {
                    expr: inner_e,
                    index: inner_i,
                } = &cur.kind
                {
                    rev_idxs.push(inner_i.as_ref());
                    cur = inner_e.as_ref();
                }
                (cur, rev_idxs)
            }
            let (lbase, lrev) = unwrap_nd(lvalue);
            let (rbase, rrev) = unwrap_nd(rvalue);
            if let (ExprKind::Ident(lh), ExprKind::Ident(rh)) = (&lbase.kind, &rbase.kind) {
                let lname = self.resolve_hier_name(lh);
                let rname = self.resolve_hier_name(rh);
                let lshape = self.module.arrays_nd.get(&*lname).cloned();
                let rshape = self.module.arrays_nd.get(&*rname).cloned();
                if let (Some((ls, _)), Some((rs, _))) = (lshape, rshape) {
                    if ls.len() == rs.len() && lrev.len() < ls.len() && lrev.len() == rrev.len() {
                        let given = lrev.len();
                        let mut l_prefix = lname.clone();
                        let mut r_prefix = rname.clone();
                        for i in (0..given).rev() {
                            let lv = self.eval_expr(lrev[i]).to_u64().unwrap_or(0) as i64;
                            let rv = self.eval_expr(rrev[i]).to_u64().unwrap_or(0) as i64;
                            l_prefix = std::borrow::Cow::Owned(format!("{}[{}]", l_prefix, lv));
                            r_prefix = std::borrow::Cow::Owned(format!("{}[{}]", r_prefix, rv));
                        }
                        let remaining: Vec<(i64, i64)> = ls[given..].to_vec();
                        let rem_r: Vec<(i64, i64)> = rs[given..].to_vec();
                        if remaining == rem_r {
                            fn enum_idx(
                                dims: &[(i64, i64)],
                                lp: String,
                                rp: String,
                                out: &mut Vec<(String, String)>,
                            ) {
                                if dims.is_empty() {
                                    out.push((lp, rp));
                                    return;
                                }
                                let (lo, hi) = dims[0];
                                for i in lo..=hi {
                                    enum_idx(
                                        &dims[1..],
                                        format!("{}[{}]", lp, i),
                                        format!("{}[{}]", rp, i),
                                        out,
                                    );
                                }
                            }
                            let mut pairs = Vec::new();
                            enum_idx(
                                &remaining,
                                l_prefix.into_owned(),
                                r_prefix.into_owned(),
                                &mut pairs,
                            );
                            for (lp, rp) in pairs {
                                let v =
                                    self.get_signal_value_by_name(&rp).unwrap_or(Value::zero(1));
                                self.set_signal_value_by_name(&lp, v);
                            }
                            self.settle_after_proc_write();
                            return;
                        }
                    }
                }
            }
        }
        // Handle LHS streaming concat: {<<slice {a, b, c, ...}} = rhs;
        // Handle RHS streaming concat to a dynamic array/queue target:
        //   queue_of_packed = {<<slice {a, b, c, ...}};
        // Distribute the streamed bits element-by-element (MSB first).
        if let ExprKind::StreamOp { .. } = &rvalue.kind {
            if let ExprKind::Ident(lh) = &lvalue.kind {
                let lname = self.resolve_hier_name(lh).to_string();
                // §11.4.14.3: a FIXED-size target fills its elements in
                // stream order (leftmost element first: a descending `[2:0]`
                // array from index 2, a 2-D array row by row); elements the
                // stream does not reach are zero.
                if !self.module.dynamic_arrays.contains(&lname)
                    && (self.module.arrays.contains_key(&lname)
                        || self.module.arrays_2d.contains_key(&lname)
                        || self.module.arrays_nd.contains_key(&lname))
                    && let Some((names, elem_w)) = self.stream_elem_order(&lname)
                {
                    let sv = self.eval_expr_ctx(rvalue, 0);
                    let total = sv.width as usize;
                    let ew = elem_w as usize;
                    for (k, en) in names.iter().enumerate() {
                        let piece = if (k + 1) * ew <= total {
                            sv.range_select(total - k * ew - 1, total - (k + 1) * ew)
                        } else if k * ew < total {
                            // A partial last element: the stream's low bits,
                            // left-justified (zero padding on the right).
                            let rem = total - k * ew;
                            let mut v = Value::zero(elem_w);
                            for b in 0..rem {
                                v.set_bit(ew - rem + b, sv.get_bit(b));
                            }
                            v
                        } else {
                            Value::zero(elem_w)
                        };
                        self.set_signal_value_by_name(en, piece.resize(elem_w));
                    }
                    self.settle_after_proc_write();
                    return;
                }
                if self.module.arrays.contains_key(&*lname) {
                    let elem_w = self
                        .lookup_signal_width(&format!("{}[0]", lname))
                        .unwrap_or_else(|| {
                            self.module.arrays.get(&*lname).map(|t| t.2).unwrap_or(8)
                        })
                        .max(1);
                    // Evaluate stream raw (no pad) by using ctx 0.
                    let sv = self.eval_expr_ctx(rvalue, 0);
                    let total = sv.width as usize;
                    let n_elems = total.div_ceil(elem_w as usize);
                    self.set_queue_size(&lname, n_elems as u64);
                    for k in 0..n_elems {
                        let hi = total.saturating_sub(k * elem_w as usize).saturating_sub(1);
                        let lo_raw = total.saturating_sub((k + 1) * elem_w as usize);
                        let lo = lo_raw;
                        let piece = if hi >= lo {
                            sv.range_select(hi, lo)
                        } else {
                            Value::zero(elem_w)
                        };
                        // Write through the canonical setter: elements
                        // of a registered queue/array live in the
                        // COMPACT signal table, and a raw
                        // `signals.insert` leaves that table's zeros
                        // shadowing the streamed bytes on read.
                        let ename = format!("{}[{}]", lname, k);
                        self.set_signal_value_by_name(&ename, piece.resize(elem_w));
                        self.widths.insert(ename, elem_w);
                    }
                    self.settle_after_proc_write();
                    return;
                }
            }
        }
        if let ExprKind::StreamOp {
            left_to_right,
            slice_size,
            exprs,
        } = &lvalue.kind
        {
            // Gather RHS bits: if it's a dynamic array/queue of a packed
            // element type, concatenate elements with index 0 at the MSB.
            let rhs_val = if let ExprKind::Ident(rhier) = &rvalue.kind {
                let rname = self.resolve_hier_name(rhier);
                if self.module.arrays.contains_key(&*rname)
                    || self.module.dynamic_arrays.contains(&*rname)
                {
                    let n = self.get_queue_size(&rname) as usize;
                    let elem_w = self
                        .lookup_signal_width(&format!("{}[0]", rname))
                        .unwrap_or_else(|| {
                            self.module.arrays.get(&*rname).map(|t| t.2).unwrap_or(8)
                        });
                    let total_w = (n as u32) * elem_w;
                    let mut packed = Value::zero(total_w);
                    for i in 0..n {
                        let ev = self
                            .get_signal_value_by_name(&format!("{}[{}]", rname, i))
                            .unwrap_or(Value::zero(elem_w));
                        // pack at position [total_w-1-i*elem_w .. total_w-(i+1)*elem_w]
                        let dst_base = total_w as usize - (i + 1) * elem_w as usize;
                        for b in 0..elem_w as usize {
                            packed.set_bit(dst_base + b, ev.get_bit(b));
                        }
                    }
                    packed
                } else {
                    self.eval_expr(rvalue)
                }
            } else {
                self.eval_expr(rvalue)
            };
            // Apply inverse stream op on rhs_val: the stream op is its own inverse
            // per slice, so we re-apply it to obtain the "ordered" bits.
            let total_w = rhs_val.width as usize;
            let ordered = if !*left_to_right {
                rhs_val
            } else {
                let slice = slice_size
                    .as_ref()
                    .map(|e| self.eval_expr(e).to_u64().unwrap_or(1) as usize)
                    .unwrap_or(1)
                    .max(1);
                let mut out = Value::zero(total_w as u32);
                let full_chunks = total_w / slice;
                let remainder = total_w - full_chunks * slice;
                for k in 0..full_chunks {
                    for b in 0..slice {
                        let src_bit = k * slice + b;
                        let dst_bit = (full_chunks - 1 - k) * slice + b + remainder;
                        if src_bit < total_w && dst_bit < total_w {
                            out.set_bit(dst_bit, rhs_val.get_bit(src_bit));
                        }
                    }
                }
                for b in 0..remainder {
                    let src_bit = full_chunks * slice + b;
                    let dst_bit = b;
                    if src_bit < total_w {
                        out.set_bit(dst_bit, rhs_val.get_bit(src_bit));
                    }
                }
                out
            };
            // Distribute `ordered` MSB-first to each target in source order.
            let total = ordered.width as usize;
            // Compute fixed-width targets; last target may be dynamic (remainder).
            let mut fixed_ws: Vec<u32> = Vec::new();
            let mut dyn_last: Option<String> = None;
            let mut dyn_last_elem_w: u32 = 8;
            for (i, e) in exprs.iter().enumerate() {
                let is_last = i == exprs.len() - 1;
                if is_last {
                    if let ExprKind::Ident(h) = &e.kind {
                        let n = self.resolve_hier_name(h);
                        if self.module.dynamic_arrays.contains(&*n)
                            || self.module.arrays.contains_key(&*n)
                            || self.module.arrays_2d.contains_key(&*n)
                            || self.module.arrays_nd.contains_key(&*n)
                        {
                            let ew = match self.stream_elem_order(&n) {
                                Some((_, ew)) if !self.module.dynamic_arrays.contains(&*n) => ew,
                                _ => self
                                    .lookup_signal_width(&format!("{}[0]", n))
                                    .unwrap_or_else(|| {
                                        self.module.arrays.get(&*n).map(|t| t.2).unwrap_or(8)
                                    }),
                            };
                            dyn_last = Some(n.to_string());
                            dyn_last_elem_w = ew;
                            continue;
                        }
                    }
                }
                fixed_ws.push(self.infer_lhs_width(e));
            }
            let fixed_total: u32 = fixed_ws.iter().sum();
            let mut msb = total; // bit position just above the MSB
            for (i, e) in exprs.iter().enumerate() {
                let is_last = i == exprs.len() - 1;
                if is_last && dyn_last.is_some() {
                    let remaining = (total as u32).saturating_sub(fixed_total);
                    let elem_w = dyn_last_elem_w.max(1);
                    let n_elems = (remaining / elem_w) as usize;
                    let dname = dyn_last.clone().unwrap();
                    // §11.4.14.3: a FIXED array fills in stream order (the
                    // left bound first, row by row); only a dynamic array or
                    // queue is resized and filled from index 0.
                    let fixed_order = if self.module.dynamic_arrays.contains(&dname) {
                        None
                    } else {
                        self.stream_elem_order(&dname).map(|(names, _)| names)
                    };
                    if fixed_order.is_none() {
                        self.set_queue_size(&dname, n_elems as u64);
                    }
                    for k in 0..n_elems {
                        let hi = msb.saturating_sub(k * elem_w as usize).saturating_sub(1);
                        let lo = msb.saturating_sub((k + 1) * elem_w as usize);
                        if hi >= lo {
                            let slice_v = ordered.range_select(hi, lo);
                            let ename = match &fixed_order {
                                Some(names) => match names.get(k) {
                                    Some(n) => n.clone(),
                                    None => break,
                                },
                                None => format!("{}[{}]", dname, k),
                            };
                            self.set_signal_value_by_name(&ename, slice_v);
                        }
                    }
                } else {
                    let w = self.infer_lhs_width(e) as usize;
                    if w == 0 {
                        continue;
                    }
                    let hi = msb.saturating_sub(1);
                    let lo = msb.saturating_sub(w);
                    let slice_v = if hi >= lo {
                        ordered.range_select(hi, lo)
                    } else {
                        Value::zero(w as u32)
                    };
                    self.assign_value(e, &slice_v);
                    msb = lo;
                }
            }
            self.settle_after_proc_write();
            return;
        }
        // Handle array locator methods with `with` clause: qs = arr.find with (filter)
        // §7.12.1 array locator methods (`find`, `min`, `max`, `unique`,
        // and the `_index` forms), with or without a `with (...)`
        // clause. They RETURN a queue and never modify the source.
        // `q.min()` parses as a Call, which the old match missed, so the
        // result was a packed scalar written to a queue container signal
        // that nothing reads — the destination came back as X.
        if let Some((arr_name, mname, filter, iter_name)) = self.locator_call(rvalue) {
            // §7.12.1: the destination may be a collection reached through
            // selects — a struct's dynamic member (`fa[1].q`), an element of
            // a fixed array of queues (`q3[0]`, `s.sp[3]`).
            let lsel = match &lvalue.kind {
                ExprKind::Ident(_) => None,
                _ => self.expr_assoc_name(lvalue),
            };
            if let Some(lname) = lsel {
                let idxs = self.locator_indices_named(
                    &arr_name,
                    &mname,
                    filter.as_ref(),
                    iter_name.as_deref(),
                );
                self.materialize_locator(&lname, &arr_name, &mname, &idxs);
                self.settle_after_proc_write();
                return;
            }
            if let ExprKind::Ident(lhier) = &lvalue.kind {
                // The destination may itself be a renamed subroutine
                // local or a class-member queue — same storage rules
                // as the receiver.
                let lname = self.resolve_locator_storage(&self.resolve_hier_name(lhier));
                let idxs = self.locator_indices_named(
                    &arr_name,
                    &mname,
                    filter.as_ref(),
                    iter_name.as_deref(),
                );
                self.materialize_locator(&lname, &arr_name, &mname, &idxs);
                self.settle_after_proc_write();
                return;
            }
        }
        if let ExprKind::WithClause {
            expr: wexpr,
            filter,
        } = &rvalue.kind
        {
            // Accept both:
            //   `arr.find with (...)`               (wexpr = MemberAccess)
            //   `arr.find(item[, idx]) with (...)`  (wexpr = Call wrapping MemberAccess)
            let (ma_expr, iter_args): (&Expression, &[Expression]) = match &wexpr.kind {
                ExprKind::MemberAccess { .. } => (wexpr.as_ref(), &[][..]),
                ExprKind::Call { func, args } => (func.as_ref(), args.as_slice()),
                _ => (wexpr.as_ref(), &[][..]),
            };
            if let ExprKind::MemberAccess {
                expr: arr_expr,
                member,
            } = &ma_expr.kind
            {
                if let ExprKind::Ident(hier) = &arr_expr.kind {
                    let arr_name = {
                        let n = self.resolve_hier_name(hier);
                        self.resolve_locator_storage(&n)
                    };
                    let mname = member.name.as_str();
                    if matches!(
                        mname,
                        "find"
                            | "find_first"
                            | "find_last"
                            | "find_index"
                            | "find_first_index"
                            | "find_last_index"
                            | "unique"
                            | "unique_index"
                            | "min"
                            | "max"
                            | "map"
                    ) {
                        let extract_ident = |e: &Expression| -> Option<String> {
                            if let ExprKind::Ident(h) = &e.kind {
                                if h.path.len() == 1 && h.path[0].selects.is_empty() {
                                    return Some(h.path[0].name.name.clone());
                                }
                            }
                            None
                        };
                        let iter_name = iter_args
                            .first()
                            .and_then(extract_ident)
                            .unwrap_or_else(|| "item".to_string());
                        let idx_name = iter_args.get(1).and_then(extract_ident);
                        if let ExprKind::Ident(lhier) = &lvalue.kind {
                            let lname = {
                                let n = self.resolve_hier_name(lhier);
                                self.resolve_locator_storage(&n)
                            };
                            let mut cur_size = self.get_queue_size(&arr_name) as usize;
                            if cur_size == 0 {
                                let mut k = 0usize;
                                while self
                                    .get_signal_value_by_name(&format!("{}[{}]", arr_name, k))
                                    .is_some()
                                {
                                    k += 1;
                                }
                                cur_size = k;
                            }
                            let struct_fields =
                                self.module.packed_struct_fields.get(&arr_name).cloned();
                            let has_idx_field = struct_fields
                                .as_ref()
                                .is_some_and(|fs| fs.iter().any(|(f, _, _)| f == "index"));
                            let mut results = Vec::new();
                            for i in 0..cur_size {
                                if let Some(v) =
                                    self.get_signal_value_by_name(&format!("{}[{}]", arr_name, i))
                                {
                                    let mut locals = self.take_pooled_frame();
                                    locals.insert(iter_name.clone(), v.clone());
                                    if let Some(fields) = &struct_fields {
                                        for (fname, off, w) in fields {
                                            let slice = v.range_select(
                                                (off + w - 1) as usize,
                                                *off as usize,
                                            );
                                            locals
                                                .insert(format!("{}.{}", iter_name, fname), slice);
                                        }
                                    }
                                    if idx_name.is_none() && !has_idx_field {
                                        locals.insert(
                                            format!("{}.index", iter_name),
                                            Value::from_u64(i as u64, 32),
                                        );
                                    }
                                    if let Some(idx) = &idx_name {
                                        locals.insert(idx.clone(), Value::from_u64(i as u64, 32));
                                    }
                                    self.push_local_frame(locals);
                                    let cond = self.eval_expr(filter);
                                    self.pop_local_frame();
                                    if mname == "map" {
                                        results.push(cond);
                                    } else if cond.is_true() {
                                        if mname.contains("index") {
                                            results.push(Value::from_u64(i as u64, 32));
                                        } else {
                                            results.push(v);
                                        }
                                    }
                                }
                            }
                            if mname.contains("first") {
                                results.truncate(1);
                            }
                            if mname.contains("last") && !results.is_empty() {
                                let last = results.pop().unwrap();
                                results = vec![last];
                            }
                            for (i, v) in results.iter().enumerate() {
                                self.set_signal_value_by_name(
                                    &format!("{}[{}]", lname, i),
                                    v.clone(),
                                );
                            }
                            self.set_queue_size(&lname, results.len() as u64);
                            if self.module.dynamic_arrays.contains(&lname) {
                                self.set_queue_size(&lname, results.len() as u64);
                            }
                        }
                        self.settle_after_proc_write();
                        return;
                    }
                }
            }
        }
        // Handle queue = array.locator_method (no with clause)
        // Detect via MemberAccess or hierarchical ident (e.g. s.unique_index)

        // An untracked lvalue (e.g. a module-scope class handle like
        // `component c;`) infers width 0; assigning then truncates
        // the RHS to 0 bits — wrong for a handle. Default unknown width
        // to 32 (handles/ints) so `c = factory.create_...()` keeps the
        // returned handle.
        let w = {
            let iw = self.infer_lhs_width(lvalue);
            if iw == 0 { 32 } else { iw }
        };
        // §8.7 / §8.8: `c = new;` (bare, no parens) or `c = new(args)` (with
        // constructor arguments).  Resolve the class type from the
        // lvalue's declared type, then construct via
        // `instantiate_class_with_type_args`.  The bare `new` expression
        // is `Ident([new])`; `new(args)` is `Call(func=Ident([new]), args=...)`.
        // §8.7 / §8.8: `c = new;` (bare, no parens) or `c = new(args)` (with
        // constructor arguments).  Resolve the class type from the
        // lvalue's declared type, then construct via
        // `instantiate_class_with_type_args`.  The bare `new` expression
        // is `Ident([new])`; `new(args)` is `Call(func=Ident([new]), args=...)`.
        // §8.7 / §8.8: `c = new;` (bare, no parens) or `c = new()`/`c = new(args)`
        // (with constructor arguments).  When args are non-empty AND the
        // lvalue is typed as a class handle (not a dynamic array), route
        // through instantiation so the constructor dispatches correctly
        // (the heartbeat fix: `new(name, cntxt, objection)` with a class
        // handle arg).  The `sized_new` handler above catches dynamic
        // array allocation. The parentheseless shallow-copy form
        // `new src` is a distinct `ShallowCopy` node (LRM 8.8 fn 23) and
        // never reaches here — so a parenthesized `new(args)` is ALWAYS
        // a constructor call, even a single class-handle argument.
        let (is_new_call, ctor_args): (bool, &[Expression]) = match &rvalue.kind {
            ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].name.name == "new" => (true, &[]),
            ExprKind::Call { func, args }
                if matches!(
                    &func.kind,
                    ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].name.name == "new"
                ) && (args.is_empty() || {
                    // Non-empty args: only treat as class instantiation when
                    // the lvalue's type is a class AND it's NOT a single-arg
                    // class-handle (shallow copy).
                    let is_class = self
                        .get_expr_type_name(lvalue)
                        .and_then(|tn| self.resolve_type_param_binding(&tn).or(Some(tn)))
                        .map(|tn| {
                            self.resolve_simple_typedef_class(&tn)
                                .as_ref()
                                .map(|r| {
                                    self.module.classes.contains_key(r)
                                        || self.module.covergroups.contains_key(r)
                                })
                                .unwrap_or_else(|| {
                                    if self.module.classes.contains_key(&tn)
                                        || self.module.covergroups.contains_key(&tn)
                                    {
                                        return true;
                                    }
                                    // §8.25: a type parameter may bind to
                                    // a SPECIALIZED class name `Base#(args)`
                                    // (e.g. `T obj` where T = `pkg#(int)` —
                                    // factory registry create_object).
                                    // The specialized name isn't a key in
                                    // module.classes, but its base is.
                                    self.extract_spec_from_string(&tn)
                                        .map(|(base, _)| self.module.classes.contains_key(&base))
                                        .unwrap_or(false)
                                        // A typedef alias to a parameterized
                                        // class (`typedef C#(int) c_t;`).
                                        || self.resolve_typedef_spec(&tn).is_some()
                                })
                        })
                        .unwrap_or(false);
                    if !is_class { false } else { true }
                }) =>
            {
                (true, args.as_slice())
            }
            _ => (false, &[]),
        };
        if is_new_call {
            // §15.3/§15.4: a parenthesis-less `new` into a mailbox or
            // semaphore (`mb = new;`) arrives here as a bare identifier;
            // the argument-carrying form is handled by the container arm
            // below, which only sees `new(...)` calls.
            if let Some(kind) = self.lvalue_container_kind(lvalue) {
                let handle = self.alloc_builtin_container(kind, ctor_args);
                self.assign_value(lvalue, &Value::from_u64(handle as u64, 32).resize(w));
                self.settle_after_proc_write();
                return;
            }
            let type_name = self.get_expr_type_name(lvalue);
            // §6.20.3 type-parameter construction: if the declared
            // type is a class type parameter (e.g. `T obj = new;`),
            // resolve it through the current instance's bindings to
            // the concrete class. A non-parameter name resolves to
            // None and passes through unchanged.
            let type_name =
                type_name.map(|tn| self.resolve_type_param_binding(&tn).unwrap_or(tn.clone()));
            // §8.25: a member target (`other.h = new`) takes the binding of
            // the object that owns the member, not the running
            // specialization's (#596).
            let type_name = self.member_target_class(lvalue).or(type_name);
            if let Some(tname) = type_name {
                // §8.25: the type-parameter resolved to a SPECIALIZED
                // class name `Base#(args)` (e.g. `T obj = new()` where
                // `T` is bound to `pkg#(int)` — a factory's `create_object`
                // doing `T obj; obj = new(...)`). The plain-name lookup
                // below only finds the unspecialized `Base` in module.classes,
                // so extract the specialization and carry it through
                // `current_spec`: `instantiate_class_with_type_args`
                // snapshots `current_spec` onto the instance's `spec`,
                // and later virtual dispatch on the instance restores
                // it (so the instance's inherited methods resolve `T`
                // correctly). Mirrors the factory `create` path.
                if let Some((base, sig)) = self.extract_spec_from_string(&tname) {
                    if let Some(class_def) = self.module.classes.get(&base).cloned() {
                        self.ensure_spec_statics(&base, &sig);
                        let saved_spec = self.current_spec.clone();
                        self.current_spec = Some((base.clone(), sig));
                        let handle = self.instantiate_class(&class_def, ctor_args);
                        self.current_spec = saved_spec;
                        self.assign_value(lvalue, &handle.resize(w));
                        self.settle_after_proc_write();
                        return;
                    }
                }
                // A class-MEMBER `prop = new(...)` whose declared type
                // is a TYPEDEF ALIAS to a parameterized class (UVM's
                // `uvm_component.event_pool` / `uvm_event_pool` =
                // `uvm_object_string_pool#(uvm_event#(uvm_object))`).
                // `get_expr_type_name` above returned just the BASE
                // class (no `#(...)`), so neither `extract_spec_..`
                // nor `resolve_ctor_type_args` finds the type args,
                // and the pool would be built with `T` = its default
                // (`uvm_object`). Recover the spec from the member's
                // alias and construct the correct specialization.
                // The same alias on a module-scope or procedural variable
                // (`typedef C#(int) c_t; c_t h; h = new;`): the lookup below
                // only follows NON-parameterized aliases, so `h` stayed null
                // and every call through it was silently dropped.
                let member_spec = match self.receiver_member_spec(lvalue) {
                    Some(s) => Some(s),
                    None => self
                        .this_member_typedef_spec(lvalue)
                        .or_else(|| self.resolve_typedef_spec(&tname)),
                };
                if let Some((base, sig)) = member_spec {
                    if let Some(class_def) = self.module.classes.get(&base).cloned() {
                        self.ensure_spec_statics(&base, &sig);
                        let saved_spec = self.current_spec.clone();
                        self.current_spec = Some((base.clone(), sig));
                        let handle = self.instantiate_class(&class_def, ctor_args);
                        self.current_spec = saved_spec;
                        self.assign_value(lvalue, &handle.resize(w));
                        self.settle_after_proc_write();
                        return;
                    }
                }
                let cls = if self.module.classes.contains_key(&tname) {
                    tname.clone()
                } else if let Some(resolved) = self.resolve_simple_typedef_class(&tname) {
                    resolved
                } else {
                    tname.clone()
                };
                if let Some(class_def) = self.module.classes.get(&cls).cloned() {
                    let lname_opt = if let ExprKind::Ident(lh) = &lvalue.kind {
                        Some(self.resolve_hier_name(lh))
                    } else {
                        None
                    };
                    let ta_cloned = self.resolve_ctor_type_args(lvalue, lname_opt.as_deref());
                    let handle = self.instantiate_class_with_type_args(
                        &class_def,
                        ctor_args,
                        ta_cloned.as_deref(),
                    );
                    self.assign_value(lvalue, &handle.resize(w));
                    self.settle_after_proc_write();
                    return;
                }
                // Bare `cg c1 = new;` / `cg c1 = new();` —
                // same handle-as-u32 form as classes, just routed
                // through the covergroup heap so sample_covergroup /
                // instantiate_covergroup wire up.
                if let Some(cg_def) = self.covergroup_def_for(&tname) {
                    let handle = self.instantiate_covergroup(&cg_def, ctor_args);
                    self.assign_value(lvalue, &handle.resize(w));
                    self.settle_after_proc_write();
                    return;
                }
            }
        }
        // Struct assignment-pattern decomposition for UNPACKED structs.
        // xezim stores unpacked-struct members as separate signals
        // (`LHS.field1`, `LHS.field2`). A whole-struct assignment
        // `t = '{1.5, 1'b1}` must therefore write each pattern element
        // to its per-field signal — building a single packed blob and
        // storing it under `t` would leave `t.field1` stale.
        //
        // Fires only when (a) RHS is an ordered AssignmentPattern and
        // (b) the LHS resolves to an unpacked struct typedef whose
        // members each have a registered `<lhs>.<member>` signal.
        // Ordered patterns align positionally with declaration order.
        if let ExprKind::AssignmentPattern(items) = &rvalue.kind {
            if items
                .iter()
                .all(|it| matches!(it, AssignmentPatternItem::Ordered(_)))
            {
                let lhs_name_opt = if let ExprKind::Ident(lh) = &lvalue.kind {
                    Some(self.resolve_hier_name(lh))
                } else {
                    None
                };
                if let Some(lhs_name) = lhs_name_opt {
                    let tname = self.get_expr_type_name(lvalue);
                    let struct_members: Option<Vec<(String, u32, u32, bool)>> = tname
                        .as_ref()
                        .and_then(|tn| self.module.typedef_types.get(tn.as_str()))
                        .and_then(|dt| {
                            let resolved = Self::resolve_type_ref(dt, &self.module.typedef_types);
                            Self::struct_field_layout(&resolved)
                        });
                    if let Some(members) = struct_members {
                        // Declaration order = reverse of struct_field_layout
                        // (which packs the last member at the LSB).
                        let mut decl_order = members.clone();
                        decl_order.reverse();
                        let n_match = decl_order.len() == items.len();
                        // Storage model detection:
                        //  - whole-value local (initial/method local stored in
                        //    a call frame, or a signal var WITHOUT per-field
                        //    signals) → update field bits in the whole Value.
                        //  - per-field signals (`<lhs>.<field>` exist, the
                        //    unpacked-struct elaboration scheme) → write each
                        //    field to its own signal.
                        let is_frame_local = self
                            .local_stack
                            .last()
                            .is_some_and(|f| f.contains_key(&*lhs_name));
                        let has_field_sigs = decl_order.iter().all(|(fn_, _, _, _)| {
                            let dotted = format!("{}.{}", lhs_name, fn_);
                            self.signals.contains_key(&dotted)
                                || self.signal_name_to_id.contains_key(dotted.as_str())
                        });
                        let whole_value_store = is_frame_local || !has_field_sigs;
                        if n_match {
                            if whole_value_store {
                                // Update field bits in the whole Value.
                                let cur = if is_frame_local {
                                    self.local_stack
                                        .last()
                                        .unwrap()
                                        .get(&*lhs_name)
                                        .cloned()
                                        .unwrap_or_else(|| Value::zero(65))
                                } else {
                                    self.get_signal_value_by_name(&lhs_name)
                                        .unwrap_or_else(|| Value::zero(65))
                                };
                                let mut cur = cur;
                                // Ensure cur is wide enough for the struct.
                                let total: u32 = decl_order.iter().map(|(_, _, w, _)| *w).sum();
                                if cur.width < total {
                                    cur = cur.resize(total);
                                }
                                for ((_fn_, off, w, fr), item) in
                                    decl_order.iter().zip(items.iter())
                                {
                                    let fr = *fr;
                                    let w = *w;
                                    let off = *off;
                                    let fv = self.eval_expr(item.expr());
                                    let mut piece = if fr && !fv.is_real {
                                        Value::from_f64(fv.to_f64())
                                    } else {
                                        fv
                                    };
                                    let _ = fr;
                                    piece = piece.resize(w);
                                    for b in 0..w {
                                        cur.set_bit((off + b) as usize, piece.get_bit(b as usize));
                                    }
                                }
                                if is_frame_local {
                                    self.local_stack
                                        .last_mut()
                                        .unwrap()
                                        .insert(lhs_name.to_string(), cur);
                                } else {
                                    self.set_signal_value_by_name(&lhs_name, cur);
                                }
                            } else {
                                // Per-field separate signals.
                                for ((fn_, _off, fw, fr), item) in
                                    decl_order.iter().zip(items.iter())
                                {
                                    let fr = *fr;
                                    let fw = *fw;
                                    let fv = self.eval_expr(item.expr());
                                    let dotted = format!("{}.{}", lhs_name, fn_);
                                    let mut fv = fv;
                                    if fr && !fv.is_real {
                                        fv = Value::from_f64(fv.to_f64());
                                    }
                                    let _ = fw;
                                    self.set_signal_value_by_name(&dotted, fv);
                                }
                            }
                            self.settle_after_proc_write();
                            return;
                        }
                    }
                }
            }
        }
        // Resolve a whole constant array before scalar evaluation or the
        // generic collection resolver can treat a class constant as a property.
        if self.try_class_const_array_copy(lvalue, rvalue) {
            self.settle_after_proc_write();
            return;
        }
        let val = if let ExprKind::Call { func, args } = &rvalue.kind {
            if let ExprKind::Ident(hier) = &func.kind {
                let method_name = hier.path.last().unwrap().name.name.as_str();
                if method_name == "new" {
                    // `mailbox`/`semaphore` allocation (§15.3/§15.4):
                    // `m = new(bound)`. This must be detected BEFORE the
                    // copy-constructor heuristic below — `new(size)` with
                    // an int variable `size` would otherwise be mistaken
                    // for `new <handle>` (its value collides with a live
                    // heap index) and shallow-copy the wrong object. The
                    // declared container type is found via the class
                    // property / local-decl maps, which get_expr_type_name
                    // misreports (it returns the enclosing class name for
                    // a property, or None for an untracked local).
                    if let Some(kind) = self.lvalue_container_kind(lvalue) {
                        let handle = self.alloc_builtin_container(kind, args);
                        self.assign_value(lvalue, &Value::from_u64(handle as u64, 32).resize(w));
                        self.settle_after_proc_write();
                        return;
                    }
                    // Copy constructor `new <obj>` (SV 8.13): a single
                    // argument that resolves to a live object handle is
                    // shallow-copied into a fresh instance. riscv-dv's
                    // get_rand_instr does `new instr_template[name]`.
                    // Guard against confusing an integer with a
                    // handle: only treat the arg as a copy source
                    // when it's a STATICALLY class-typed handle
                    // expression (Ident/MemberAccess/Index path) —
                    // never a Number, Binary, SystemCall, or an
                    // int var whose value collides with a live
                    // heap index (`new(i)` in a loop).
                    let arg_could_be_handle = args
                        .first()
                        .map(|a| {
                            matches!(
                                &a.kind,
                                ExprKind::Ident(_)
                                    | ExprKind::MemberAccess { .. }
                                    | ExprKind::Index { .. }
                            ) && self.expr_is_class_handle(a)
                        })
                        .unwrap_or(false);
                    if args.len() == 1 && arg_could_be_handle {
                        let src_h = self.eval_expr(&args[0]).to_u64().unwrap_or(0) as usize;
                        if src_h != 0 {
                            if let Some(Some(_)) = self.heap.get(src_h) {
                                let h = self.copy_construct(src_h);
                                self.assign_value(lvalue, &h.resize(w));
                                self.settle_after_proc_write();
                                return;
                            }
                        }
                    }
                    let type_name = self.get_expr_type_name(lvalue);
                    // §6.20.3 type-parameter construction: resolve a
                    // declared type that is a class type parameter (e.g.
                    // `T obj; obj = new(n);`) through the current
                    // instance's bindings to the concrete class so its
                    // real `new` runs. Plain class names are unchanged.
                    let type_name =
                        type_name.map(|tn| self.resolve_type_param_binding(&tn).unwrap_or(tn));
                    // §8.25: `o.h = new()`, `h` declared by a type parameter:
                    // the active specialization is not `o`'s (outside the
                    // class, or inside another specialization), so the name
                    // stayed the parameter's or took the wrong binding. Bind
                    // it through the object that owns the member (#576).
                    // The owner's binding comes first even when the running
                    // specialization names a class: inside `W #(A)`,
                    // `other.h = new()` with `other` a `W #(B)` builds a `B`
                    // (#596).
                    let type_name = self.member_target_class(lvalue).or(type_name);
                    if let Some(tname) = type_name {
                        if tname == "semaphore" {
                            let handle = self.heap.len();
                            self.heap.push(Some(Box::new(ClassInstance {
                                class_name: tname.clone(),
                                properties: PropMap::default(),
                                type_bindings: HashMap::default(),
                                spec: None,
                                creation_scope: self.active_instance_scope(),
                            })));
                            let initial_count = args
                                .first()
                                .map(|a| self.eval_expr(a).to_u64().unwrap_or(0) as i64)
                                .unwrap_or(0);
                            self.semaphores.insert(handle, initial_count);
                            Value::from_u64(handle as u64, 32)
                        } else if tname == "mailbox" {
                            let handle = self.heap.len();
                            self.heap.push(Some(Box::new(ClassInstance {
                                class_name: tname.clone(),
                                properties: PropMap::default(),
                                type_bindings: HashMap::default(),
                                spec: None,
                                creation_scope: self.active_instance_scope(),
                            })));
                            self.mailboxes
                                .insert(handle, std::collections::VecDeque::new());
                            self.record_mailbox_bound(handle, args);
                            Value::from_u64(handle as u64, 32)
                        } else if let Some(class_def) = self.module.classes.get(&tname).cloned() {
                            let lname_opt = if let ExprKind::Ident(lh) = &lvalue.kind {
                                Some(self.resolve_hier_name(lh))
                            } else {
                                None
                            };
                            let ta_cloned =
                                self.resolve_ctor_type_args(lvalue, lname_opt.as_deref());
                            self.instantiate_class_with_type_args(
                                &class_def,
                                args,
                                ta_cloned.as_deref(),
                            )
                        } else if let Some(cg_def) = self.covergroup_def_for(&tname) {
                            self.instantiate_covergroup(&cg_def, args)
                        } else {
                            // Could be dynamic array new[size]
                            if let Some(arg) = args.first() {
                                let size = self.eval_expr(arg);
                                // `d = new[n]` and also `d[i] = new[n]` on an
                                // array of dynamic arrays — the element is the
                                // dynamic array, named flat as `d[i]`.
                                let target = match &lvalue.kind {
                                    ExprKind::Ident(lhier) => Some(self.resolve_hier_name(lhier)),
                                    _ => {
                                        let f = self.flat_member_name(lvalue);
                                        f.filter(|n| self.module.dynamic_arrays.contains(n))
                                            .map(std::borrow::Cow::Owned)
                                    }
                                };
                                if let Some(name) = target {
                                    self.signals.insert(format!("{}.size", name), size.clone());
                                }
                                // Do not assign to lvalue (array) itself
                                self.settle_after_proc_write();
                                return;
                            } else {
                                // `lhs = new()` where the LHS type
                                // (`tname`) is known but not a modelled
                                // class/covergroup — typically a typedef
                                // of a parameterized class such as
                                // `typedef pool#(...) id_actions_array`.
                                // Construct an opaque instance tagged
                                // with that type. CRUCIAL: never fall
                                // back to evaluating the bare `new()` as
                                // a generic call — `new` on an
                                // assignment RHS denotes the LHS type's
                                // constructor, but the generic path
                                // re-binds it to the *enclosing* class's
                                // `new`, which for initialization routines
                                // (`id_actions = new();`) recurses
                                // new -> initialize -> new … forever.
                                let h = self.heap.len();
                                self.heap.push(Some(Box::new(ClassInstance {
                                    class_name: tname.clone(),
                                    properties: PropMap::default(),
                                    type_bindings: HashMap::default(),
                                    spec: None,
                                    creation_scope: self.active_instance_scope(),
                                })));
                                Value::from_u64(h as u64, 32)
                            }
                        }
                    } else {
                        // Dynamic array new[size] without explicit type name
                        if let Some(arg) = args.first() {
                            let size = self.eval_expr(arg);
                            if let ExprKind::Ident(lhier) = &lvalue.kind {
                                let name = self.resolve_hier_name(lhier);
                                self.signals.insert(format!("{}.size", name), size.clone());
                            }
                            self.settle_after_proc_write();
                            return;
                        } else {
                            // `lhs = new()` with an unknown LHS type:
                            // opaque instance. NEVER eval the bare
                            // `new()` generically — it re-binds to the
                            // enclosing class's constructor and
                            // recurses.
                            let h = self.heap.len();
                            self.heap.push(Some(Box::new(ClassInstance {
                                class_name: String::new(),
                                properties: PropMap::default(),
                                type_bindings: HashMap::default(),
                                spec: None,
                                creation_scope: self.active_instance_scope(),
                            })));
                            Value::from_u64(h as u64, 32)
                        }
                    }
                } else {
                    self.eval_assign_rhs(lvalue, rvalue, w)
                }
            } else {
                self.eval_assign_rhs(lvalue, rvalue, w)
            }
        } else {
            self.eval_assign_rhs(lvalue, rvalue, w)
        };
        // A collection RETURNED by the call in `rvalue` (recorded at
        // its `return` statement): copy it into a collection lvalue.
        if matches!(&rvalue.kind, ExprKind::Call { .. }) {
            if let Some(src) = self.take_pending_ret_collection() {
                let dst: Option<std::borrow::Cow<str>> = {
                    let byname = if let ExprKind::Ident(h) = &lvalue.kind {
                        // The LHS may itself be a renamed local dyn
                        // (`byte_array b;` inside an initial/method).
                        let bare = h.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
                        let n = self
                            .dyn_name_lookup(bare)
                            .map(|s| std::borrow::Cow::Owned(s.to_string()))
                            .unwrap_or_else(|| self.resolve_hier_name(h));
                        self.module.dynamic_arrays.contains(&*n).then_some(n)
                    } else {
                        None
                    };
                    byname.or_else(|| {
                        self.expr_assoc_name(lvalue)
                            .filter(|an| !self.is_associative_array(an))
                            .map(std::borrow::Cow::Owned)
                    })
                };
                if let Some(dst) = dst {
                    if dst != src {
                        self.copy_whole_queue(&dst, &src);
                    }
                    self.settle_after_proc_write();
                    return;
                }
            }
        }
        let resolve_coll = |sim: &mut Self, e: &Expression| -> Option<(String, bool)> {
            if let ExprKind::Ident(h) = &e.kind {
                // A method-local queue / dynamic array lives under its
                // per-call storage key (`declare_local_dyn`), which shadows
                // any same-named property.
                if h.path.len() == 1 && h.path[0].selects.is_empty() {
                    let bare = h.path[0].name.name.as_str();
                    if let Some(uq) = sim.dyn_name_lookup(bare).filter(|uq| *uq != bare) {
                        if sim.module.dynamic_arrays.contains(uq) {
                            return Some((uq.to_string(), false));
                        }
                    }
                }
                if !sim.ident_shadowed_by_non_collection(h) {
                    let n = Self::resolve_hier_name_static(h, &sim.module);
                    // A bare identifier naming a COLLECTION MEMBER of
                    // `this` must resolve to its `<handle>#member` storage
                    // even when a same-named value-param / dyn-array is
                    // registered from an enclosing frame. Prefer the
                    // `this`-scoped member; a true local (instance_assoc_member
                    // returns None) falls through to the bare name below.
                    // Without this, a deep `Store::do_write` whose `val = t`
                    // ran while an outer `make(T val)` value-param `val` was
                    // still live in `module.dynamic_arrays` resolved the LHS
                    // to bare `"val"` and the queue copy was dropped (§13.5.2).
                    // §13.5.2/§8.24 name resolution: the CURRENT call's own
                    // queue/dyn-array formal (bound in the top `local_dyn`
                    // frame by `bind_queue_param`) shadows a same-named
                    // property — only an ENCLOSING frame's registration
                    // loses to the property. Without this, a whole-array
                    // copy whose LHS formal shadows the property
                    // (`uvm_reg_item::get_value_array(ref u64_t value[])`
                    // — body `value = this.value`) resolved the LHS to the
                    // property store: the property was clobbered and the
                    // ref writeback shipped an empty array.
                    let formal_in_current_frame = sim
                        .local_dyn
                        .last()
                        .is_some_and(|f| f.iter().any(|(k, _)| k == &n));
                    if sim.module.dynamic_arrays.contains(&n) {
                        if !formal_in_current_frame {
                            if let Some(m) = sim.instance_assoc_member(&n) {
                                return Some((m, true));
                            }
                        }
                        return Some((n, false));
                    }
                }
            }
            // An object's collection PROPERTY (`d.data`, `this.data`)
            // lives under "<handle>#member" — resolve it like element
            // accesses already do, else a whole-array copy involving
            // a property is silently dropped.
            // A FIXED multi-dimensional member is not a collection: it
            // copies element-wise by shape (`copy_multi_dim_array`).
            sim.expr_assoc_name(e)
                .filter(|an| !sim.is_associative_array(an))
                .filter(|an| {
                    sim.module.dynamic_arrays.contains(an)
                        || !(sim.module.arrays_2d.contains_key(an)
                            || sim.module.arrays_nd.contains_key(an))
                })
                .map(|an| (an, true))
        };
        let l_res = resolve_coll(self, lvalue);
        let r_res = resolve_coll(self, rvalue);
        // Extract the resolved names now — the both-collection `if let`
        // below consumes `l_res`/`r_res`. Used by the mixed
        // dynamic↔fixed path after it.
        let l_dyn = l_res.as_ref().map(|(n, _)| n.clone());
        let r_dyn = r_res.as_ref().map(|(n, _)| n.clone());
        if xz_copy_debug_enabled() {
            eprintln!("[CPDBG] l={:?} r={:?}", l_res, r_res);
        }
        if let (Some((lname, l_is_prop)), Some((rname, r_is_prop))) = (l_res, r_res) {
            // `r = q` between queues / dynamic arrays (§7.6): copy every
            // element and the size. Nothing did this — the RHS was read as
            // a scalar container signal that queues do not have, so the
            // destination stayed empty whatever its element type.
            if lname != rname
                && (l_is_prop || self.module.dynamic_arrays.contains(&lname))
                && (r_is_prop || self.module.dynamic_arrays.contains(&rname))
            {
                // Rows that are themselves dynamic arrays or queues
                // (`int d[][]`) are copied with their sizes and elements.
                self.copy_dyn_tree(&lname, &rname);
                self.settle_after_proc_write();
                return;
            }
        }
        // Mixed dynamic↔fixed whole-array copy (`c.p = tmp`): one side is
        // a dynamic array / collection property (resolved above via
        // resolve_coll) and the other is a fixed array. The
        // both-dynamic case above and the both-fixed case below
        // already have dedicated paths; this only catches the
        // cross-kind mix. Element copy uses descending-index handling
        // on the fixed side (§7.2.1) and 0-based indices on the
        // dynamic side. A dynamic-array destination is resized to the
        // fixed source's length (§7.6).
        let l_fix = self.fixed_array_operand(lvalue);
        let r_fix = self.fixed_array_operand(rvalue);
        // LHS dynamic array/prop  ←  RHS fixed array
        if let (Some(dst), Some((src, lo, hi))) = (l_dyn.clone(), r_fix) {
            let desc = self.module.descending_arrays.contains(&src);
            let n = (hi - lo + 1) as u64;
            for i in 0..n {
                let ridx = if desc { hi - i as i64 } else { lo + i as i64 };
                let v = self
                    .get_signal_value_by_name(&format!("{}[{}]", src, ridx))
                    .unwrap_or_else(|| Value::zero(32));
                self.set_signal_value_by_name(&format!("{}[{}]", dst, i), v);
            }
            self.set_queue_size(&dst, n);
            if !self.in_edge_block {
                self.settle_combinatorial();
            }
            return;
        }
        // LHS fixed array  ←  RHS dynamic array/prop
        // Multi-dimensional unpacked arrays: whole copy (`w = v`) and
        // row copy (`r = v[1]`, `v[1] = r`) go element-wise by suffix;
        // the arms below only know one dimension.
        if self.copy_multi_dim_array(lvalue, rvalue) {
            self.settle_after_proc_write();
            return;
        }
        if let (Some((dst, lo, hi)), Some(src)) = (l_fix, r_dyn.clone()) {
            let desc = self.module.descending_arrays.contains(&dst);
            let cap = (hi - lo + 1) as u64;
            let n = self.get_queue_size(&src).min(cap);
            for i in 0..n {
                let lidx = if desc { hi - i as i64 } else { lo + i as i64 };
                let v = self
                    .get_signal_value_by_name(&format!("{}[{}]", src, i))
                    .unwrap_or_else(|| Value::zero(32));
                self.set_signal_value_by_name(&format!("{}[{}]", dst, lidx), v);
            }
            if !self.in_edge_block {
                self.settle_combinatorial();
            }
            return;
        }
        // Assoc-to-assoc and fixed-array-to-fixed-array copies key on
        // the PLAIN resolved names (they predate the collection
        // resolver above and must not be gated behind it — `s = a`
        // between fixed arrays broke when they were). A dotted path rooted
        // at a class-handle variable (`c2.bits = c1.bits`) is a class-member
        // access (collection members were already handled by `resolve_coll`
        // above under `<handle>#member`), never a module-scope array whose
        // bare name happens to match the leaf segment.
        if let (ExprKind::Ident(lhier), ExprKind::Ident(rhier)) = (&lvalue.kind, &rvalue.kind)
            && !self.ident_shadowed_by_non_collection(lhier)
            && !self.ident_shadowed_by_non_collection(rhier)
        {
            let lname = self.resolve_hier_name(lhier);
            let rname = self.resolve_hier_name(rhier);
            if self.is_associative_array(&lname) && self.is_associative_array(&rname) {
                let prefix = format!("{}[", rname);
                let entries: Vec<(String, Value)> = self
                    .signals
                    .keys_with_elem_prefix(&prefix)
                    .into_iter()
                    .filter(|k| k.ends_with(']'))
                    .filter_map(|k| {
                        let v = self.signals.get(&k)?.clone();
                        let key = &k[prefix.len()..k.len() - 1];
                        Some((format!("{}[{}]", lname, key), v))
                    })
                    .collect();
                for (k, v) in entries {
                    self.signals.insert(k, v);
                }
                // §7.2/§7.2.1: a struct element's members are leaves under
                // the element (`aa[k].m`), and its dynamic members are
                // collections with sizes of their own — not cells of the
                // array the raw copy above moved. Copy each element whole.
                if let Some(su) = self.queue_elem_struct(&rname) {
                    let (lname, rname) = (lname.into_owned(), rname.into_owned());
                    for key in self.assoc_top_level_keys(&rname) {
                        let (d, s) = (format!("{}[{}]", lname, key), format!("{}[{}]", rname, key));
                        self.copy_unpacked_struct(&d, &s, &su);
                        if self.get_signal_value_by_name(&d).is_none() {
                            self.set_signal_value_by_name(&d, Value::zero(1));
                        }
                    }
                }
                self.settle_after_proc_write();
                return;
            }
            if self.module.arrays.contains_key(&*lname) && self.module.arrays.contains_key(&*rname)
            {
                let (llo, lhi, _) = self.module.arrays[&*lname];
                let (rlo, rhi, _) = self.module.arrays[&*rname];
                let lsize = (lhi - llo + 1) as usize;
                let rsize = (rhi - rlo + 1) as usize;
                // §7.2 / §7.6: an unpacked struct element consists of
                // member leaves; its container scalar carries no value.
                if let Some(su) = self.queue_elem_struct(&rname) {
                    let dynamic_dst = self.module.dynamic_arrays.contains(&*lname);
                    let count = if dynamic_dst { self.get_queue_size(&rname) as usize }
                        else { lsize.min(rsize) };
                    let l_desc = self.module.descending_arrays.contains(&*lname);
                    let r_desc = self.module.descending_arrays.contains(&*rname);
                    for i in 0..count {
                        let li = if dynamic_dst { i as i64 } else if l_desc { lhi-i as i64 } else { llo+i as i64 };
                        let ri = if r_desc { rhi-i as i64 } else { rlo+i as i64 };
                        self.copy_unpacked_struct(&format!("{lname}[{li}]"), &format!("{rname}[{ri}]"), &su);
                    }
                    if dynamic_dst { self.set_queue_size(&lname, count as u64); }
                    self.settle_after_proc_write();
                    return;
                }
                // §7.6: assigning a FIXED array to a DYNAMIC array
                // resizes the destination to the source's size —
                // the placeholder (0,63) registration made this look
                // like fixed-to-fixed and left the old size in place.
                if self.module.dynamic_arrays.contains(&*lname)
                    && !self.module.dynamic_arrays.contains(&*rname)
                {
                    let r_desc = self.module.descending_arrays.contains(&*rname);
                    for i in 0..rsize {
                        let ridx = if r_desc {
                            rhi - i as i64
                        } else {
                            rlo + i as i64
                        };
                        let rval = self
                            .get_signal_value_by_name(&format!("{}[{}]", rname, ridx))
                            .unwrap_or(Value::zero(32));
                        self.set_signal_value_by_name(&format!("{}[{}]", lname, i), rval);
                    }
                    self.set_queue_size(&lname, rsize as u64);
                    self.settle_after_proc_write();
                    return;
                }
                let count = lsize.min(rsize);
                let l_desc = self.module.descending_arrays.contains(&*lname);
                let r_desc = self.module.descending_arrays.contains(&*rname);
                for i in 0..count {
                    let ridx = if r_desc {
                        rhi - i as i64
                    } else {
                        rlo + i as i64
                    };
                    let lidx = if l_desc {
                        lhi - i as i64
                    } else {
                        llo + i as i64
                    };
                    let rval = self
                        .get_signal_value_by_name(&format!("{}[{}]", rname, ridx))
                        .unwrap_or(Value::zero(32));
                    self.set_signal_value_by_name(&format!("{}[{}]", lname, lidx), rval);
                }
                self.settle_after_proc_write();
                return;
            }
        }
        if let ExprKind::Ident(lhier) = &lvalue.kind {
            let mut lname = if self.ident_shadowed_by_non_collection(lhier) {
                std::borrow::Cow::Borrowed("")
            } else {
                let mut n = self.resolve_hier_name(lhier);
                // An instance-scoped queue/array member (`q` inside a method,
                // or `obj.q`) lives under `<handle>#q`, not the bare name.
                if let Some(s) = self.instance_assoc_member(&n) {
                    n = std::borrow::Cow::Owned(s);
                }
                n
            };
            // §10.10: a class queue / dynamic-array member (`<handle>#q`)
            // has no `arrays` row, but `obj.q = {obj.q, v}` is the same
            // unpacked-array concatenation.
            let mut class_dyn = false;
            if matches!(rvalue.kind, ExprKind::Concatenation(_))
                && !self.module.arrays.contains_key(&*lname)
                && let Some(n) = self.expr_assoc_name(lvalue)
                && self.class_queue_prop(&n)
            {
                lname = std::borrow::Cow::Owned(n);
                class_dyn = true;
            }
            if self.module.arrays.contains_key(&*lname) || class_dyn {
                // Queue/array slice assignment: lq = rq[a:b]
                if let ExprKind::RangeSelect {
                    expr: rbase,
                    left,
                    right,
                    ..
                } = &rvalue.kind
                {
                    if let ExprKind::Ident(rhier) = &rbase.kind {
                        let mut rname = self.resolve_hier_name(rhier);
                        // Same instance scoping as the LHS: a member queue
                        // slice (`instr_list[a:b]`) reads `<handle>#name`.
                        if let Some(s) = self.instance_assoc_member(&rname) {
                            rname = std::borrow::Cow::Owned(s);
                        }
                        if self.module.arrays.contains_key(&*rname) {
                            let (r_lo_a, r_hi_a, _) = self.module.arrays[&*rname];
                            let r_is_dyn = self.module.dynamic_arrays.contains(&*rname);
                            let r_upper: i64 = if r_is_dyn {
                                (self.get_queue_size(&rname) as i64) - 1
                            } else {
                                r_hi_a
                            };
                            self.dollar_bound.push(r_upper);
                            let l = self.eval_expr(left).to_i64().unwrap_or(0);
                            let r = self.eval_expr(right).to_i64().unwrap_or(0);
                            self.dollar_bound.pop();
                            // Per IEEE 7.10.1: if l > r the slice is empty.
                            let results: Vec<Value> = if l > r {
                                Vec::new()
                            } else {
                                let lo = l.max(r_lo_a);
                                let hi = r.min(r_upper);
                                if hi < lo {
                                    Vec::new()
                                } else {
                                    (lo..=hi)
                                        .map(|idx| {
                                            self.get_signal_value_by_name(&format!(
                                                "{}[{}]",
                                                rname, idx
                                            ))
                                            .unwrap_or(Value::zero(32))
                                        })
                                        .collect()
                                }
                            };
                            let (l_lo, l_hi, _) = self.module.arrays[&*lname];
                            let is_dyn = self.module.dynamic_arrays.contains(&*lname);
                            for (i, v) in results.iter().enumerate() {
                                let idx = l_lo + i as i64;
                                // Dynamic arrays / queues start
                                // with `l_hi = -1` (empty). Skip
                                // the bound check there — the
                                // assignment is what GROWS them.
                                if !is_dyn && idx > l_hi {
                                    break;
                                }
                                self.set_signal_value_by_name(
                                    &format!("{}[{}]", lname, idx),
                                    v.clone(),
                                );
                            }
                            if is_dyn {
                                self.set_queue_size(&lname, results.len() as u64);
                            }
                            self.settle_after_proc_write();
                            return;
                        }
                    }
                }
                if let ExprKind::AssignmentPattern(items) = &rvalue.kind {
                    let is_dyn = self.module.dynamic_arrays.contains(&*lname);
                    let (lo, hi, _w) = self
                        .module
                        .arrays
                        .get(&*lname)
                        .copied()
                        .unwrap_or((0, -1, 32));
                    let descending = self.module.descending_arrays.contains(&*lname);
                    for (i, item) in items.iter().enumerate() {
                        let idx = if descending {
                            hi - i as i64
                        } else {
                            lo + i as i64
                        };
                        // Dynamic arrays grow to fit; fixed arrays clip.
                        if !is_dyn && (idx < lo || idx > hi) {
                            break;
                        }
                        let v = self.eval_expr(item.expr());
                        self.set_signal_value_by_name(&format!("{}[{}]", lname, idx), v);
                    }
                    if is_dyn {
                        self.set_queue_size(&lname, items.len() as u64);
                    }
                    self.settle_after_proc_write();
                    return;
                }
                if let ExprKind::Concatenation(exprs) = &rvalue.kind {
                    // IEEE 1800 §10.10.1: in an unpacked-array
                    // concatenation each item is read in the context of
                    // the ELEMENT type, so a nested plain-brace concat
                    // is itself an array value whose elements SPLICE
                    // into the outer one — `a = {1, {2, 3}}` assigns
                    // `{1, 2, 3}` — i.e. nested braces flatten. Only
                    // flatten when the element type is not itself an
                    // array (for a multi-dimensional target an inner
                    // brace denotes one whole sub-array element).
                    // Assignment patterns `'{...}` never flatten; they
                    // take the AssignmentPattern branch above.
                    fn flatten_unpacked_concat<'a>(
                        exprs: &'a [Expression],
                        out: &mut Vec<&'a Expression>,
                    ) {
                        for e in exprs {
                            if let ExprKind::Concatenation(inner) = &e.kind {
                                flatten_unpacked_concat(inner, out);
                            } else {
                                out.push(e);
                            }
                        }
                    }
                    let elem_is_scalar = self.foreach_dims(&lname).is_none_or(|d| d.len() == 1);
                    let items: Vec<&Expression> = if elem_is_scalar {
                        let mut v = Vec::new();
                        flatten_unpacked_concat(exprs, &mut v);
                        v
                    } else {
                        exprs.iter().collect()
                    };
                    // §10.10 / §7.8: in a queue (or array) of ASSOCIATIVE
                    // arrays, an associative operand (`q = {src}`) is one
                    // whole element: its entries are copied into it.
                    // Evaluated as one value, the element kept no entries.
                    let assoc_srcs: Vec<Option<String>> =
                        if self.module.associative_arrays.is_empty() {
                            Vec::new()
                        } else {
                            items.iter().map(|e| self.assoc_value_operand(e)).collect()
                        };
                    if assoc_srcs.iter().any(Option::is_some) {
                        let is_dyn = class_dyn || self.module.dynamic_arrays.contains(&*lname);
                        let lo = self.module.arrays.get(&*lname).map_or(0, |t| t.0);
                        for (i, (e, src)) in items.iter().zip(assoc_srcs).enumerate() {
                            let elem = format!("{}[{}]", lname, lo + i as i64);
                            match src {
                                Some(src) => self.store_assoc_elem(&elem, &src),
                                None => {
                                    let v = self.eval_expr(e);
                                    self.set_signal_value_by_name(&elem, v);
                                }
                            }
                        }
                        if is_dyn {
                            self.set_queue_size(&lname, items.len() as u64);
                        }
                        self.settle_after_proc_write();
                        return;
                    }
                    // Expand queue/array elements in concat (e.g. q = {q, 4})
                    let all_vals = self.unpacked_concat_operand_values(items);
                    let is_dyn = class_dyn || self.module.dynamic_arrays.contains(&*lname);
                    let (lo, hi, _w) = self
                        .module
                        .arrays
                        .get(&*lname)
                        .copied()
                        .unwrap_or((0, -1, 32));
                    let n_vals = all_vals.len();
                    for (i, v) in all_vals.into_iter().enumerate() {
                        let idx = lo + i as i64;
                        // Dynamic arrays/queues grow to fit; only fixed
                        // arrays are bounded by their declared range.
                        if !is_dyn && idx > hi {
                            break;
                        }
                        // §10.10: each item is an element-typed value.
                        let v = if is_dyn {
                            self.fit_queue_push_elem(&lname, v)
                        } else {
                            v
                        };
                        self.set_signal_value_by_name(&format!("{}[{}]", lname, idx), v);
                    }
                    if is_dyn {
                        self.set_queue_size(&lname, n_vals as u64);
                    }
                    self.settle_after_proc_write();
                    return;
                }
            }
        }
        // When assigning a locator/reduction method result to a queue, store as single-element queue
        if let ExprKind::Ident(lhier) = &lvalue.kind
            && !self.ident_shadowed_by_non_collection(lhier)
        {
            let lname = self.resolve_hier_name(lhier);
            if self.module.dynamic_arrays.contains(&*lname)
                && self.module.arrays.contains_key(&*lname)
            {
                if let ExprKind::MemberAccess { member, .. } = &rvalue.kind {
                    let mname = member.name.as_str();
                    if matches!(
                        mname,
                        "min"
                            | "max"
                            | "unique"
                            | "find"
                            | "find_first"
                            | "find_last"
                            | "find_index"
                            | "find_first_index"
                            | "find_last_index"
                            | "sum"
                            | "product"
                    ) {
                        if !val.has_xz() {
                            self.set_signal_value_by_name(&format!("{}[0]", lname), val);
                            self.set_queue_size(&lname, 1);
                        } else {
                            self.set_queue_size(&lname, 0);
                        }
                        self.settle_after_proc_write();
                        return;
                    }
                }
            }
        }
        self.assign_value(lvalue, &val);
        self.settle_after_proc_write();
    }
}
