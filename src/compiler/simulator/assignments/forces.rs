//! `force` and `release` of whole and partial targets.

use super::*;

impl Simulator {
    pub(in crate::compiler::simulator) fn remove_active_force_expr(&mut self, key: &str) {
        self.active_force_exprs.retain(|entry| {
            if entry.key != key {
                return true;
            }
            self.unresolved_force_count -= usize::from(entry.has_unresolved_reads);
            false
        });
    }

    fn push_active_force_expr(&mut self, entry: ActiveForceExpr) {
        if self.active_force_signal_epochs.len() < self.signal_table.len() {
            self.active_force_signal_epochs
                .resize(self.signal_table.len(), 0);
        }
        self.unresolved_force_count += usize::from(entry.has_unresolved_reads);
        self.active_force_exprs.push(entry);
    }

    /// Resolve a `force`/`release`/`assign`/`deassign` lvalue (LRM §10.6,
    /// §10.6.1) to its override-tracking key: the flattened signal name plus
    /// the compact-table id (`None` when the value lives only in the runtime
    /// `signals` map). Only whole-signal identifiers are tracked; §10.6 also
    /// allows bit/part-selects and concatenations on nets — those return
    /// `None` and degrade to an untracked plain write.
    /// Track an active force/procedural-assign whose RHS reads any signal,
    /// replacing a previous entry for the same target (§10.6.1/§10.6.2).
    pub(in crate::compiler::simulator) fn arm_force_expr(
        &mut self,
        key: String,
        lvalue: &Expression,
        rvalue: &Expression,
    ) {
        self.remove_active_force_expr(&key);
        // A constant RHS never changes — no need to track it.
        let mut reads: HashSet<String> = HashSet::default();
        Self::collect_expr_reads(rvalue, &self.module, &mut reads);
        if reads.is_empty() {
            return;
        }
        let hint = self.name_resolve_hint.borrow().clone();
        let top_prefix = format!("{}.", self.module.name);
        let mut read_signal_ids = Vec::with_capacity(reads.len());
        let mut has_unresolved_reads = false;
        for read in reads {
            let mut id = self.resolve_read_name(&read, &top_prefix);
            // A procedural assignment is armed while its process's instance
            // scope is active. Bare RHS names therefore need the same scoped
            // lookup as expression evaluation. Walk parent scopes as a
            // conservative aid for nested blocks whose hint includes a block
            // label below the declaring instance.
            if id.is_none()
                && let Some(scope_hint) = hint.as_deref()
            {
                let mut scope = scope_hint;
                loop {
                    let qualified = format!("{}.{}", scope, read);
                    id = self.resolve_read_name(&qualified, &top_prefix);
                    if id.is_some() {
                        break;
                    }
                    let Some((parent, _)) = scope.rsplit_once('.') else {
                        break;
                    };
                    scope = parent;
                }
            }
            if let Some(id) = id {
                read_signal_ids.push(id);
            } else {
                has_unresolved_reads = true;
            }
        }
        read_signal_ids.sort_unstable();
        read_signal_ids.dedup();
        let read_epochs = read_signal_ids
            .iter()
            .map(|&id| {
                self.active_force_signal_epochs
                    .get(id)
                    .copied()
                    .unwrap_or(0)
            })
            .collect();
        self.push_active_force_expr(ActiveForceExpr {
            key,
            lvalue: lvalue.clone(),
            rvalue: rvalue.clone(),
            hint,
            read_signal_ids,
            read_epochs,
            has_unresolved_reads,
            partial: false,
        });
    }

    /// §10.6.2: re-evaluate expression-RHS overrides whose operands changed
    /// and write through when their value changed. Re-scan after each update
    /// so reverse-ordered dependency chains reach a fixpoint in one settle.
    /// Expressions containing non-signal reads retain the conservative prior
    /// behavior and run once per refresh.
    pub(in crate::compiler::simulator) fn refresh_active_forces(&mut self) {
        if self.active_force_exprs.is_empty() {
            return;
        }
        // Nothing an armed force reads has changed since the last completed
        // pass, and no entry needs the conservative once-per-refresh re-eval
        // — skip the whole scan. Arming a new force (arm_force_expr) always
        // evaluates the expression at arm time, so a freshly armed override
        // is already current.
        if self.force_epoch_gen == self.force_refresh_done_gen && self.unresolved_force_count == 0
        {
            return;
        }
        let entry_count = self.active_force_exprs.len();
        let mut conservative_done = vec![false; entry_count];
        let mut evaluated = vec![false; entry_count];
        // Malformed cyclic overrides can keep changing forever. The normal
        // simulator settle loop also has a convergence limit; keep this local
        // work bounded and allow a later settle call to continue propagation.
        let budget = entry_count.saturating_mul(4).saturating_add(16);
        let mut work = 0usize;

        while work < budget {
            let mut next = None;
            for (idx, entry) in self.active_force_exprs.iter().enumerate() {
                let needs_eval = if entry.has_unresolved_reads {
                    !conservative_done[idx]
                } else {
                    entry
                        .read_signal_ids
                        .iter()
                        .zip(entry.read_epochs.iter())
                        .any(|(&id, &seen)| {
                            self.active_force_signal_epochs
                                .get(id)
                                .copied()
                                .unwrap_or(0)
                                != seen
                        })
                };
                if needs_eval {
                    next = Some(idx);
                    break;
                }
            }
            let Some(idx) = next else {
                break;
            };
            work += 1;
            self.active_force_evals += 1;
            evaluated[idx] = true;

            let (lvalue, rvalue, hint, is_conservative) = {
                let entry = &self.active_force_exprs[idx];
                (
                    entry.lvalue.clone(),
                    entry.rvalue.clone(),
                    entry.hint.clone(),
                    entry.has_unresolved_reads,
                )
            };
            if is_conservative {
                conservative_done[idx] = true;
            }
            if self.active_force_exprs[idx].partial {
                if let Some((_, Some(id))) = self.force_target(&lvalue) {
                    self.refresh_partial_force(id, &lvalue);
                }
                let ids = self.active_force_exprs[idx].read_signal_ids.clone();
                self.active_force_exprs[idx].read_epochs = ids
                    .iter()
                    .map(|&id| {
                        self.active_force_signal_epochs
                            .get(id)
                            .copied()
                            .unwrap_or(0)
                    })
                    .collect();
                continue;
            }
            let saved_hint = self.name_resolve_hint.borrow().clone();
            *self.name_resolve_hint.borrow_mut() = hint;
            let v = self.eval_expr(&rvalue);
            let target = self.force_target(&lvalue);
            match target {
                Some((_, Some(id))) if is_packed_id(id) => {
                    self.packed.release_forced(id);
                    self.assign_value(&lvalue, &v);
                    self.force_cell(id, v);
                }
                Some((_, Some(id))) => {
                    if self.forced_signals.get(&id) != Some(&v) {
                        self.forced_signals.remove(&id);
                        self.assign_value(&lvalue, &v);
                        let stored = self.signal_table.get(id).cloned().unwrap_or(v);
                        self.forced_signals.insert(id, stored);
                    }
                }
                Some((name, None)) => {
                    self.forced_names.remove(&name);
                    self.assign_value(&lvalue, &v);
                    self.forced_names.insert(name);
                }
                None => {}
            }
            *self.name_resolve_hint.borrow_mut() = saved_hint;

            // Snapshot after evaluation and target write. This prevents a
            // self-referential override from immediately waking itself while
            // still exposing a changed target to downstream entries.
            let ids = self.active_force_exprs[idx].read_signal_ids.clone();
            self.active_force_exprs[idx].read_epochs = ids
                .iter()
                .map(|&id| {
                    self.active_force_signal_epochs
                        .get(id)
                        .copied()
                        .unwrap_or(0)
                })
                .collect();
        }

        self.force_refresh_done_gen = self.force_epoch_gen;
        self.active_force_skips += self
            .active_force_exprs
            .iter()
            .enumerate()
            .filter(|(idx, entry)| !entry.has_unresolved_reads && !evaluated[*idx])
            .count() as u64;
    }

    /// §10.6.2 `release` / §10.6.1 `deassign` of `target` (the name and table
    /// slot `force_target` resolved, if any).
    pub(in crate::compiler::simulator) fn release_override(
        &mut self,
        target: Option<(&str, Option<usize>)>,
    ) {
        if let Some((name, id)) = target {
            if let Some(id) = id {
                self.partial_forces.remove(&id);
            }
            self.forced_names.remove(name);
            self.remove_active_force_expr(name);
            if let Some(id) = id.filter(|&id| is_packed_id(id)) {
                // A packed-arena cell is always a variable: it keeps the
                // forced value and takes ordinary stores again (§10.6.2).
                self.packed.release_forced(id);
            } else if let Some(id) = id {
                self.forced_signals.remove(&id);
                // Re-evaluate continuous drivers (nets). For a variable no
                // comb entry writes it, so the scan finds nothing and the
                // value is retained.
                self.redrive_released_signal(id);
                // The net takes its drivers' value at once: the reference
                // reads the driven value in the statement after `release`.
                if !self.settling && !self.in_edge_block {
                    let drivers: Vec<CombEntry> = self
                        .comb_writers_of(id)
                        .into_iter()
                        .map(|eidx| &self.comb_entries[eidx])
                        .filter(|e| !e.cold.read_signal_ids.is_empty())
                        .cloned()
                        .collect();
                    for entry in &drivers {
                        self.eval_released_comb_entry(entry);
                    }
                }
            }
        }
        // A released signal that is a gateable flop's output (`q` in
        // `always @(posedge clk) q<=d`) may no longer equal its data input —
        // the force broke the "Q provably unchanged" invariant the EVENT_EDGE
        // skip relies on. Invalidate every flop snapshot so the next edge
        // actually re-fires and re-drives the released output, instead of
        // being skipped for "no data change" and leaving Q stuck at the forced
        // value. Release is rare, so a one-shot re-fire of all flops is cheap.
        self.edge_block_snap_valid
            .iter_mut()
            .for_each(|v| *v = false);
        self.edge_block_armed
            .iter_mut()
            .for_each(|v| *v |= EDGE_ARMED);
        self.dirty_any = true;
    }

    /// IEEE 1800-2023 §10.6.2: a constant bit- or part-select of a whole
    /// NET (`w[0]`, `w[7:6]`, `w[i +: 2]`): the net's id, its base
    /// identifier expression, and the physical (lo bit, width) selected.
    pub(in crate::compiler::simulator) fn force_select_range(
        &mut self,
        lv: &Expression,
    ) -> Option<(usize, Expression, u32, u32)> {
        let (base, a, b) = match &lv.kind {
            ExprKind::Index { expr, index } => {
                let l = self.eval_expr(index).to_i64()?;
                (expr, l, l)
            }
            ExprKind::RangeSelect {
                expr,
                kind,
                left,
                right,
            } => {
                let l = self.eval_expr(left).to_i64()?;
                let r = self.eval_expr(right).to_i64()?;
                match kind {
                    RangeKind::Constant => (expr, l, r),
                    RangeKind::IndexedUp if r > 0 => (expr, l + r - 1, l),
                    RangeKind::IndexedDown if r > 0 => (expr, l, l - r + 1),
                    _ => return None,
                }
            }
            _ => return None,
        };
        let ExprKind::Ident(h) = &base.kind else {
            return None;
        };
        if !h.path.iter().all(|s| s.selects.is_empty()) {
            return None;
        }
        let name = self.resolve_hier_name(h).to_string();
        if !self.module.nets.contains(&name)
            || self.module.arrays.contains_key(&name)
            || self
                .module
                .packed_signal_elem_widths
                .get(&name)
                .copied()
                .unwrap_or(0)
                > 1
        {
            return None;
        }
        let id = *self.signal_name_to_id.get(name.as_str())?;
        if is_packed_id(id) || id >= self.signal_widths.len() {
            return None;
        }
        let phys = |lab: i64| -> i64 {
            if let Some(&(_, hi_l)) = self.module.ascending_packed.get(&name) {
                hi_l - lab
            } else {
                let off = self
                    .module
                    .packed_full_dims
                    .get(&name)
                    .and_then(|d| d.first())
                    .map(|&(dl, dr)| dl.min(dr))
                    .unwrap_or(0);
                lab - off
            }
        };
        let (pa, pb) = (phys(a), phys(b));
        let lo = pa.min(pb);
        let hi = pa.max(pb);
        let w = self.signal_widths[id] as i64;
        if lo < 0 || hi >= w {
            return None;
        }
        Some((id, (**base).clone(), lo as u32, (hi - lo + 1) as u32))
    }

    /// §10.6.2 `force` of a net bit/part-select: those bits take `rvalue`
    /// (tracked like a whole-net force expression); the others keep
    /// following the net's drivers.
    pub(in crate::compiler::simulator) fn force_partial(
        &mut self,
        sel: (usize, Expression, u32, u32),
        rvalue: &Expression,
    ) {
        let (id, base, lo, width) = sel;
        let w = self.signal_widths[id] as usize;
        let hint = self.name_resolve_hint.borrow().clone();
        let pf = self
            .partial_forces
            .entry(id)
            .or_insert_with(|| PartialForce {
                bits: vec![None; w],
                pieces: Vec::new(),
            });
        let p = pf.pieces.len();
        pf.pieces.push((rvalue.clone(), width, hint));
        for k in 0..width {
            if let Some(slot) = pf.bits.get_mut((lo + k) as usize) {
                *slot = Some((p, k));
            }
        }
        self.arm_partial_force(id, &base);
        self.refresh_partial_force(id, &base);
    }

    /// §10.6.2 `release` of a net bit/part-select: those bits return to the
    /// net's drivers at once; the net is fully released with its last bit.
    pub(in crate::compiler::simulator) fn release_partial(
        &mut self,
        sel: (usize, Expression, u32, u32),
    ) {
        let (id, base, lo, width) = sel;
        let Some(pf) = self.partial_forces.get_mut(&id) else {
            return;
        };
        for k in 0..width {
            if let Some(slot) = pf.bits.get_mut((lo + k) as usize) {
                *slot = None;
            }
        }
        if pf.bits.iter().all(|b| b.is_none()) {
            let target = self.force_target(&base);
            self.release_override(target.as_ref().map(|(n, id)| (n.as_str(), *id)));
            return;
        }
        self.refresh_partial_force(id, &base);
    }

    /// Track the reads a partial force depends on: the net's drivers and its
    /// forcing expressions (see `refresh_active_forces`).
    pub(in crate::compiler::simulator) fn arm_partial_force(
        &mut self,
        id: usize,
        base: &Expression,
    ) {
        let Some((key, _)) = self.force_target(base) else {
            return;
        };
        self.remove_active_force_expr(&key);
        // §10.6.2: the net's drivers from the writers index, as the release
        // paths find them; a scan of every entry here ran on each force of
        // part of the net.
        let mut ids: Vec<usize> = Vec::new();
        for eidx in self.comb_writers_of(id) {
            ids.extend(self.comb_entries[eidx].cold.read_signal_ids.iter().copied());
        }
        let mut reads: HashSet<String> = HashSet::default();
        if let Some(pf) = self.partial_forces.get(&id) {
            for (rv, _, _) in &pf.pieces {
                Self::collect_expr_reads(rv, &self.module, &mut reads);
            }
        }
        let top_prefix = format!("{}.", self.module.name);
        let hint = self.name_resolve_hint.borrow().clone();
        let mut has_unresolved_reads = false;
        for read in reads {
            let mut rid = self.resolve_read_name(&read, &top_prefix);
            if rid.is_none()
                && let Some(scope_hint) = hint.as_deref()
            {
                let mut scope = scope_hint;
                loop {
                    rid = self.resolve_read_name(&format!("{}.{}", scope, read), &top_prefix);
                    if rid.is_some() {
                        break;
                    }
                    let Some((parent, _)) = scope.rsplit_once('.') else {
                        break;
                    };
                    scope = parent;
                }
            }
            match rid {
                Some(r) => ids.push(r),
                None => has_unresolved_reads = true,
            }
        }
        ids.sort_unstable();
        ids.dedup();
        let read_epochs = ids
            .iter()
            .map(|&r| self.active_force_signal_epochs.get(r).copied().unwrap_or(0))
            .collect();
        self.push_active_force_expr(ActiveForceExpr {
            key,
            lvalue: base.clone(),
            rvalue: Expression::new(ExprKind::Empty, base.span),
            hint,
            read_signal_ids: ids,
            read_epochs,
            has_unresolved_reads,
            partial: true,
        });
    }

    /// Recompute a partially forced net: lift the override, re-evaluate its
    /// drivers, then pin the forced bits over the driven value.
    pub(in crate::compiler::simulator) fn refresh_partial_force(
        &mut self,
        id: usize,
        base: &Expression,
    ) {
        let Some(pf) = self.partial_forces.get(&id) else {
            return;
        };
        let pieces: Vec<(Expression, u32, Option<String>)> = pf.pieces.clone();
        let bits = pf.bits.clone();
        let mut vals: Vec<Option<Value>> = vec![None; pieces.len()];
        for (pi, _) in bits.iter().flatten() {
            if vals[*pi].is_none() {
                let (rv, w, h) = &pieces[*pi];
                let saved = self.name_resolve_hint.borrow().clone();
                *self.name_resolve_hint.borrow_mut() = h.clone();
                let v = self.eval_expr(rv).resize(*w);
                *self.name_resolve_hint.borrow_mut() = saved;
                vals[*pi] = Some(v);
            }
        }
        self.forced_signals.remove(&id);
        if !self.settling && !self.in_edge_block {
            let drivers: Vec<CombEntry> = self
                .comb_writers_of(id)
                .into_iter()
                .map(|eidx| self.comb_entries[eidx].clone())
                .collect();
            for entry in &drivers {
                self.eval_comb_entry_full(entry);
            }
        }
        let mut merged = self.signal_table[id].clone();
        for (i, b) in bits.iter().enumerate() {
            if let Some((pi, k)) = b {
                if let Some(v) = &vals[*pi] {
                    merged.set_bit(i, v.get_bit(*k as usize));
                }
            }
        }
        if merged != self.signal_table[id] {
            self.assign_value(base, &merged);
        }
        // A gate driving the net must see the override (see the whole-net
        // force path).
        self.ctl_note_deposit(base);
        let stored = self.signal_table[id].clone();
        self.forced_signals.insert(id, stored);
    }

    pub(in crate::compiler::simulator) fn force_target(
        &mut self,
        lv: &Expression,
    ) -> Option<(String, Option<usize>)> {
        if let Some(target) = self.inst_array_member_canon(lv) {
            return self.force_target(&target);
        }
        if let ExprKind::Ident(hier) = &lv.kind {
            if hier.path.iter().all(|s| s.selects.is_empty()) {
                let name = self.resolve_hier_name(hier);
                let id = self.signal_name_to_id.get(name.as_ref()).copied();
                return Some((name.to_string(), id));
            }
        }
        // §10.6.2: a HIERARCHICAL target (`force u.internal = v`) parses as
        // MemberAccess, not a dotted Ident, so it fell past both arms and
        // returned None — the force degraded to a plain write that the
        // target's own driver overwrote on its next evaluation (and a
        // collapsed output port never showed it at all). Flatten the member
        // chain and accept it only when it names real storage, so unrelated
        // MemberAccess shapes (struct fields, class properties) keep the
        // previous behaviour.
        if let Some(name) = self.flat_member_name(lv) {
            if let Some(&id) = self.signal_name_to_id.get(name.as_str()) {
                return Some((name, Some(id)));
            }
            if self.signals.contains_key(&name) {
                return Some((name, None));
            }
            // Instance-scoped force issued from inside a scope: retry under
            // the active resolution hint, as the name-keyed paths do.
            let hinted = self
                .name_resolve_hint
                .borrow()
                .as_ref()
                .map(|h| format!("{}.{}", h, name));
            if let Some(hn) = hinted {
                if let Some(&id) = self.signal_name_to_id.get(hn.as_str()) {
                    return Some((hn, Some(id)));
                }
                if self.signals.contains_key(&hn) {
                    return Some((hn, None));
                }
            }
        }
        // §10.6.2: `force mem[i] = v` on a FIXED-array element. The element
        // has a real signal id, so the id-keyed force guard protects it like
        // any scalar. Previously this shape returned None and the force
        // silently degraded to a plain write — the next procedural write
        // simply overrode it.
        if let ExprKind::Index { expr: base, index } = &lv.kind {
            if let ExprKind::Ident(bh) = &base.kind {
                if bh.path.iter().all(|s| s.selects.is_empty()) {
                    let bname = self.resolve_hier_name(bh);
                    if self.module.arrays.contains_key(&*bname)
                        && !self.module.dynamic_arrays.contains(&*bname)
                        && !self.module.queue_vars.contains(&*bname)
                        && !self.module.associative_arrays.contains_key(&*bname)
                    {
                        let idx = self.eval_expr(index).to_i64()?;
                        if let Some(id) = self.get_array_elem_id(&bname, idx) {
                            // A packed-arena cell is pinned inside the arena
                            // (`force_cell` / `release_cell`), not through the
                            // signal-table `forced_signals` map.
                            return Some((format!("{}[{}]", bname, idx), Some(id)));
                        }
                    }
                }
            }
        }
        None
    }
}
