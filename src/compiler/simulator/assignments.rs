//! Simulator assignments.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    /// Resolve a fully-qualified signal name (possibly an array element
    /// like `"tb.foo.mem[5]"`) to a signal_id. Tries the dense
    /// `signal_name_to_id` HashMap first; if that misses and the name has
    /// the form `<base>[<idx>]`, parses the suffix and uses
    /// `array_first_id` to compute the id without ever materialising
    /// per-element HashMap entries. Returns None if neither path resolves.
    #[inline]
    pub(super) fn resolve_signal_id(&self, name: &str) -> Option<usize> {
        if let Some(&id) = self.signal_name_to_id.get(name) {
            return Some(id);
        }
        resolve_array_elem_id(name, &self.array_first_id)
    }

    /// Look up flat signal_id for array element `name[idx]`. With the
    /// compact `array_first_id` resolver, 1D arrays compute the id directly
    /// as `first_id + (idx - lo)` — no per-element HashMap entry is needed.
    /// 2D/ND arrays still fall through to the legacy `array_elem_ids` cache
    /// path which iterates signal_name_to_id; those arrays are small in
    /// practice on real designs.
    pub(super) fn get_array_elem_id(&mut self, name: &str, idx: i64) -> Option<usize> {
        // Fast path: 1D array via compact resolver.
        if let Some(&(first_id, lo, hi)) = self.array_first_id.get(name) {
            if idx >= lo && idx <= hi {
                return Some(first_id + (idx - lo) as usize);
            }
            return None;
        }
        let (arr_lo, arr_hi) = match self.module.arrays.get(name) {
            Some(&(lo, hi, _)) => (lo, hi),
            None => return None,
        };
        if idx < arr_lo || idx > arr_hi {
            return None;
        }
        let offset = (idx - arr_lo) as usize;
        if !self.array_elem_ids.contains_key(name) {
            let size = (arr_hi - arr_lo + 1) as usize;
            let mut v = vec![-1i64; size];
            for i in 0..size {
                let elem_idx = arr_lo + i as i64;
                let elem_name = format!("{}[{}]", name, elem_idx);
                if let Some(&id) = self.signal_name_to_id.get(elem_name.as_str()) {
                    v[i] = id as i64;
                }
            }
            self.array_elem_ids.insert(name.to_string(), v);
        }
        let v = self.array_elem_ids.get(name).unwrap();
        let id = v[offset];
        if id < 0 { None } else { Some(id as usize) }
    }

    /// `format!("{}#{}::{}", class, sig, member)` (with a sig) or
    /// `format!("{}::{}", class, member)` without the formatting machinery.
    pub(super) fn spec_member_key(class: &str, sig: Option<&str>, member: &str) -> String {
        let mut out =
            String::with_capacity(class.len() + sig.map_or(0, |s| s.len() + 1) + member.len() + 2);
        out.push_str(class);
        if let Some(sig) = sig {
            out.push('#');
            out.push_str(sig);
        }
        out.push_str("::");
        out.push_str(member);
        out
    }

    /// `format!("{}.{}", hint, name)` under the current scope hint, if any,
    /// built without cloning the hint.
    pub(super) fn hint_scoped(&self, name: &str) -> Option<String> {
        let hint = self.name_resolve_hint.borrow();
        let h = hint.as_deref()?;
        let mut out = String::with_capacity(h.len() + 1 + name.len());
        out.push_str(h);
        out.push('.');
        out.push_str(name);
        Some(out)
    }

    pub(super) fn resolve_type_width_in_process_scope(
        &mut self,
        dt: &crate::ast::types::DataType,
    ) -> u32 {
        use crate::compiler::scoped_arrays;
        let mut view = HashMap::default();
        for arg in scoped_arrays::query_operands(dt) {
            let Some(name) = scoped_arrays::operand_name(arg) else {
                continue;
            };
            let lexical = self.dyn_name_lookup(&name).map(str::to_string);
            let scope = self
                .name_resolve_hint
                .borrow()
                .clone()
                .or_else(|| self.process_scope_hint.get(&self.current_pid).cloned());
            let canonical = lexical
                .or_else(|| {
                    scope.as_ref().and_then(|scope| {
                        let key = format!("{scope}.{name}");
                        self.module.arrays.contains_key(&key).then_some(key)
                    })
                })
                .or_else(|| self.array_operand_name(arg))
                .or_else(|| self.module.arrays.contains_key(&name).then(|| name.clone()))
                .or_else(|| {
                    let key = name.replace('.', "::");
                    self.module.arrays.contains_key(&key).then_some(key)
                });
            if let Some((key, shape)) = canonical.and_then(|key| {
                self.module
                    .arrays
                    .get(&key)
                    .copied()
                    .map(|shape| (key, shape))
            }) {
                view.insert(name, shape);
                view.insert(key, shape);
            }
        }
        scoped_arrays::resolve_type_width(
            dt,
            Some(&self.module.parameters),
            Some(&self.module.typedefs),
            &view,
        )
    }

    /// `format!("{}{}", base, suffix)` without the formatting machinery.
    pub(super) fn name_with_suffix(base: &str, suffix: &str) -> String {
        let mut out = String::with_capacity(base.len() + suffix.len());
        out.push_str(base);
        out.push_str(suffix);
        out
    }

    /// `format!("{}[{}]", base, key)` for an already-rendered key.
    pub(super) fn name_with_key(base: &str, key: &str) -> String {
        let mut out = String::with_capacity(base.len() + key.len() + 2);
        out.push_str(base);
        out.push('[');
        out.push_str(key);
        out.push(']');
        out
    }

    /// Flatten an `Ident` / `Index` / `MemberAccess` chain into the dotted-and-
    /// indexed signal name the elaborator pre-registers for unpacked aggregates
    /// (`c.nodes[1].status`, `arr[0].tag`). `None` for shapes it can't flatten
    /// (non-constant index, ranges, calls, …).
    /// `<base>[<i>]` without the `core::fmt` machinery — this runs once per
    /// dimension per element access on procedural array loops, where it and
    /// its allocations were the single hottest thing in the profile.
    pub(super) fn name_with_index(base: &str, i: i64) -> String {
        let mut out = String::with_capacity(base.len() + 14);
        out.push_str(base);
        out.push('[');
        Self::push_i64(&mut out, i);
        out.push(']');
        out
    }

    pub(super) fn push_i64(out: &mut String, i: i64) {
        if i < 0 {
            out.push('-');
        }
        let mut n = i.unsigned_abs();
        let mut buf = [0u8; 20];
        let mut k = buf.len();
        loop {
            k -= 1;
            buf[k] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        out.push_str(std::str::from_utf8(&buf[k..]).unwrap_or("0"));
    }

    pub(super) fn flat_member_name(&mut self, e: &Expression) -> Option<String> {
        match &e.kind {
            ExprKind::Ident(h) => {
                if h.path.iter().any(|s| !s.selects.is_empty()) {
                    return None;
                }
                // Apply the local-dyn-array rename so callers (e.g. `%p`
                // queue formatting, indexed writes) hit the process-unique
                // storage key instead of the bare name. `resolve_hier_name`
                // already does this, but `flat_member_name` bypasses it.
                if h.path.len() == 1 {
                    if let Some(uq) = self.dyn_name_lookup(&h.path[0].name.name) {
                        return Some(uq.to_string());
                    }
                    // §8/§23.9: in a method of a class declared inside a
                    // module, the instance's own struct / variable.
                    if self.modclass_depth.get() > 0
                        && let Some(q) = self.modclass_item(&h.path[0].name.name)
                    {
                        return Some(q);
                    }
                    // §8.9/§8.25: inside a STATIC method of a parameterized
                    // class, a bare static collection names this
                    // specialization's store (`C#int::q`), the one its
                    // builtin methods and `C#(int)::q[i].m` use, so an
                    // element write `q[k] = s` spreads its members there.
                    if let Some(k) = self.static_method_coll_key(&h.path[0].name.name) {
                        return Some(k);
                    }
                }
                // §26.3: strip a PACKAGE prefix — `P::s` flattens to the bare
                // storage name `s` (see resolve_hier_name), so member paths
                // like `P::s.x` become `s.x`.
                let skip = usize::from(
                    h.path.len() >= 2
                        && self.module.packages.contains(&h.path[0].name.name)
                        && !self
                            .signal_name_to_id
                            .contains_key(h.path[0].name.name.as_str())
                        && !self.signals.contains_key(&h.path[0].name.name),
                );
                let segs = &h.path[skip..];
                // The overwhelmingly common shape is a single segment; the
                // Vec + join below allocates twice for it. This is the
                // innermost call of every nested-index name build, so it runs
                // once per element access in a procedural array loop.
                if segs.len() == 1 {
                    return Some(segs[0].name.name.clone());
                }
                Some(
                    segs.iter()
                        .map(|s| s.name.name.as_str())
                        .collect::<Vec<_>>()
                        .join("."),
                )
            }
            ExprKind::Index { expr, index } => {
                let base = match &expr.kind {
                    // `this.q[i]`: name the receiver like `obj.q[i]`; the
                    // storage resolvers map `this.q` through the handle.
                    ExprKind::MemberAccess { expr: recv, member }
                        if matches!(recv.kind, ExprKind::This)
                            && self.this_stack.last().copied().flatten().is_some() =>
                    {
                        format!("this.{}", member.name)
                    }
                    _ => match self.class_scoped_elem_store(expr) {
                        Some(k) => k,
                        None => self.flat_member_name(expr)?,
                    },
                };
                // §7.10.1: `$` names the last element of the queue indexed.
                let iv = if Self::expr_contains_dollar(index) {
                    self.eval_index_of_queue(&base, index)
                } else {
                    self.eval_expr(index)
                };
                // §8.10: inside a method a bare property name is the
                // object's store, keyed in ITS format, even when a package
                // variable of the same name is registered under the bare name.
                // §7.8: an ASSOC element key must use the canonical store
                // format — a STRING key rendered via to_i64 collapsed to its
                // byte value ("sa[k]" became "sa[107]"), so writes and the
                // registered read paths disagreed on the element's name.
                if self.is_associative_array(&base) {
                    if self.this_stack.last().copied().flatten().is_some()
                        && let Some(st) = self.instance_assoc_member(&base)
                        && self.is_associative_array(&st)
                    {
                        let key = self.assoc_key_str(&st, &iv);
                        return Some(format!("{}[{}]", base, key));
                    }
                    let key = self.assoc_key_str(&base, &iv);
                    let mut out = String::with_capacity(base.len() + key.len() + 2);
                    out.push_str(&base);
                    out.push('[');
                    out.push_str(&key);
                    out.push(']');
                    return Some(out);
                }
                // A CLASS-MEMBER associative array is registered only under
                // its instance-scoped name (`<h>#aa`); key it by that store's
                // format too, or `aa["k"]` in a method names element `aa[107]`
                // while every read keys it `aa[k]`.
                if !self.no_class_objects()
                    && !self.module.arrays.contains_key(&base)
                    && !self.module.dynamic_arrays.contains(&base)
                {
                    let st = self.resolve_locator_storage(&base);
                    if st != base && self.is_associative_array(&st) {
                        let key = self.assoc_key_str(&st, &iv);
                        return Some(format!("{}[{}]", base, key));
                    }
                }
                let i = iv.to_i64()?;
                // `format!` drags in the whole core::fmt machinery, which the
                // profile showed dominating this path. A nested index builds
                // one of these per dimension, per access.
                Some(Self::name_with_index(&base, i))
            }
            // §8.25: `C#(args)::m` names the static property `m` of THAT
            // specialization, whose store is keyed per specialization
            // (`C#args::m`, the key a static method of the class uses), so
            // `C#(int)::q[0].f` reaches the element members the class wrote.
            // A static FIXED array keeps one store per declaring class
            // (`C::m`), the one its methods use.
            ExprKind::MemberAccess { expr, member }
                if matches!(expr.kind, ExprKind::Specialization { .. }) =>
            {
                let spec = self
                    .resolve_call_spec_params(Self::extract_call_spec(expr), &self.current_spec)?;
                if let Some(k) = self.static_fixed_key_in(&spec.0, &member.name) {
                    return Some(k);
                }
                let saved = self.current_spec.replace(spec.clone());
                let key = self.static_prop_key(&spec.0, &member.name);
                self.current_spec = saved;
                key
            }
            ExprKind::MemberAccess { expr, member } => {
                let mut base = self.flat_member_name(expr)?;
                base.reserve(member.name.len() + 1);
                base.push('.');
                base.push_str(&member.name);
                Some(base)
            }
            _ => None,
        }
    }

    /// Resolve a `force`/`release`/`assign`/`deassign` lvalue (LRM §10.6,
    /// §10.6.1) to its override-tracking key: the flattened signal name plus
    /// the compact-table id (`None` when the value lives only in the runtime
    /// `signals` map). Only whole-signal identifiers are tracked; §10.6 also
    /// allows bit/part-selects and concatenations on nets — those return
    /// `None` and degrade to an untracked plain write.
    /// Track an active force/procedural-assign whose RHS reads any signal,
    /// replacing a previous entry for the same target (§10.6.1/§10.6.2).
    pub(super) fn arm_force_expr(&mut self, key: String, lvalue: &Expression, rvalue: &Expression) {
        self.active_force_exprs.retain(|entry| entry.key != key);
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
        self.active_force_exprs.push(ActiveForceExpr {
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
    pub(super) fn refresh_active_forces(&mut self) {
        if self.active_force_exprs.is_empty() {
            return;
        }
        // Nothing an armed force reads has changed since the last completed
        // pass, and no entry needs the conservative once-per-refresh re-eval
        // — skip the whole scan. Arming a new force (arm_force_expr) always
        // evaluates the expression at arm time, so a freshly armed override
        // is already current.
        if self.force_epoch_gen == self.force_refresh_done_gen
            && !self
                .active_force_exprs
                .iter()
                .any(|e| e.has_unresolved_reads)
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
    pub(super) fn release_override(&mut self, target: Option<(&str, Option<usize>)>) {
        if let Some((name, id)) = target {
            if let Some(id) = id {
                self.partial_forces.remove(&id);
            }
            self.forced_names.remove(name);
            self.active_force_exprs.retain(|entry| entry.key != name);
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
                        .comb_entries
                        .iter()
                        .filter(|e| e.cold.write_signal_ids.contains(&id))
                        .cloned()
                        .collect();
                    for entry in &drivers {
                        self.eval_comb_entry_full(entry);
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
    pub(super) fn force_select_range(
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
    pub(super) fn force_partial(
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
    pub(super) fn release_partial(&mut self, sel: (usize, Expression, u32, u32)) {
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
    pub(super) fn arm_partial_force(&mut self, id: usize, base: &Expression) {
        let Some((key, _)) = self.force_target(base) else {
            return;
        };
        self.active_force_exprs.retain(|entry| entry.key != key);
        let mut ids: Vec<usize> = Vec::new();
        for e in self.comb_entries.iter() {
            if e.cold.write_signal_ids.contains(&id) {
                ids.extend(e.cold.read_signal_ids.iter().copied());
            }
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
        self.active_force_exprs.push(ActiveForceExpr {
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
    pub(super) fn refresh_partial_force(&mut self, id: usize, base: &Expression) {
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
                .comb_entries
                .iter()
                .filter(|e| e.cold.write_signal_ids.contains(&id))
                .cloned()
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

    pub(super) fn force_target(&mut self, lv: &Expression) -> Option<(String, Option<usize>)> {
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

    /// The registered packed-struct base of a dotted member lvalue, as
    /// `(base_name, bit_offset, width)` of the selected member within it.
    ///
    /// `packed_struct_fields[base]` is FLATTENED — a nested member is a key in
    /// its own right (`"hdr.d.x"`), and no entry exists for an intermediate
    /// member on its own (`packed_struct_fields["f.hdr"]` is None). So the
    /// single split at the last dot only ever resolved a DEPTH-1 member, and
    /// `f.hdr.d = v` inside a subroutine — where the lvalue parses as
    /// MemberAccess rather than a dotted Ident — matched nothing and was
    /// silently dropped. Walk the split points LONGEST BASE FIRST, exactly as
    /// `resolve_packed_struct_target` does for assignment patterns (which is
    /// why the same write with a `'{...}` RHS always worked): the base may
    /// itself be instance-scoped (`u.r2`), so the first-dot split is wrong too.
    pub(super) fn packed_struct_member_slice(
        &mut self,
        lvalue: &Expression,
    ) -> Option<(String, u32, u32)> {
        let flat = self.flat_member_name(lvalue)?;
        for (i, _) in flat
            .match_indices('.')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            let (base, prefix) = (&flat[..i], &flat[i + 1..]);
            let Some(layout) = self.module.packed_struct_fields.get(base) else {
                continue;
            };
            if let Some((_, o, w)) = layout.iter().find(|(k, _, _)| k == prefix) {
                return Some((base.to_string(), *o, *w));
            }
        }
        None
    }

    /// Resolve a packed-struct lvalue to `(base_signal, field_prefix, layout,
    /// base_offset, width)` for assignment-pattern packing. `layout` is the
    /// base signal's flattened `(dotted_field, offset, width)` list; `prefix`
    /// is the field path being written (`""` for the whole base signal);
    /// `base_offset`/`width` locate that field's slice within the base value.
    /// Returns None when the lvalue is not a registered packed struct (caller
    /// then falls back to the generic path).
    pub(super) fn resolve_packed_struct_target(
        &mut self,
        lvalue: &Expression,
        items: &[AssignmentPatternItem],
    ) -> Option<(String, String, Vec<(String, u32, u32)>, u32, u32)> {
        // Keyed/Typed items denote associative or type-keyed aggregates, not a
        // by-member struct pack — leave those to the existing paths.
        if items.iter().any(|it| {
            matches!(
                it,
                AssignmentPatternItem::Keyed(..) | AssignmentPatternItem::Typed(..)
            )
        }) {
            return None;
        }
        let flat = self.flat_member_name(lvalue)?;
        // The base may be INSTANCE-SCOPED ("u.r2"), so the first-dot split
        // misread the instance prefix as the struct base and every lookup
        // missed. Try each split point, LONGEST registered base first (the
        // whole name, then head-of-last-dot, …), with the remainder as the
        // member prefix.
        let mut candidates: Vec<(String, String)> = vec![(flat.clone(), String::new())];
        for (i, _) in flat
            .match_indices('.')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            candidates.push((flat[..i].to_string(), flat[i + 1..].to_string()));
        }
        for (obj_name, prefix) in candidates {
            let Some(layout) = self.module.packed_struct_fields.get(&obj_name).cloned() else {
                continue;
            };
            let (base_offset, width) = if prefix.is_empty() {
                // Whole base signal: span = max (offset + width) over fields.
                let total = layout.iter().map(|(_, o, w)| o + w).max().unwrap_or(0);
                (0, total)
            } else {
                // A member: must itself be a registered field.
                match layout.iter().find(|(k, _, _)| *k == prefix) {
                    Some((_, o, w)) => (*o, *w),
                    None => continue,
                }
            };
            return Some((obj_name, prefix, layout, base_offset, width));
        }
        None
    }

    /// Pack a named/ordered/default assignment pattern into a PACKED struct
    /// target and store it in place (blocking assign).
    ///
    /// The generic `eval_expr` path concatenates pattern items blind to the
    /// target layout, so any member expression that isn't exactly its member
    /// width (e.g. an unsized `'0`, which is 32 bits) shifts every subsequent
    /// field and corrupts the packed value. Here each item is resized to its
    /// own member width and dropped at its absolute bit offset instead; nested
    /// patterns on sub-struct members recurse. Returns false (caller falls back
    /// to the generic path) when the lvalue is neither a registered packed
    /// struct, a packed-struct class property (`obj.prop`, `this.prop`, or a
    /// bare property inside a method), nor a packed-struct element of a fixed
    /// array resolved through its declared type.
    pub(super) fn try_assign_packed_struct_pattern(
        &mut self,
        lvalue: &Expression,
        items: &[AssignmentPatternItem],
    ) -> bool {
        if let Some((obj_name, prefix, layout, _base_off, _w)) =
            self.resolve_packed_struct_target(lvalue, items)
        {
            // Write straight into the whole base value at absolute offsets, so a
            // member assign leaves its sibling members untouched.
            if let Some(mut acc) = self.get_local_or_signal(&obj_name)
                && self.apply_packed_struct_pattern(&mut acc, &layout, &prefix, 0, items)
            {
                self.set_local_or_signal(&obj_name, acc);
                return true;
            }
        }
        // Class-property destination (`obj.prop = '{...}`, `this.prop`, or a
        // bare `prop` inside a method): the storage is the heap object's
        // `properties` cell, which the registered-signal path above cannot
        // see (`packed_struct_fields` has no entry for a dotted class
        // property). Lay the pattern out at the property's declared member
        // offsets so later member reads — which use those offsets — agree.
        if let Some((handle, prop, layout)) = self.class_prop_pattern_target(lvalue, items) {
            let total = layout.iter().map(|(_, o, w)| o + w).max().unwrap_or(0);
            let mut acc = Value::zero(total);
            if let Some(seed) = self
                .heap
                .get(handle)
                .and_then(|o| o.as_ref())
                .and_then(|i| i.properties.get(&prop))
            {
                for b in 0..total {
                    acc.set_bit(b as usize, seed.get_bit(b as usize));
                }
            }
            if self.apply_packed_struct_pattern(&mut acc, &layout, "", 0, items) {
                // `set_prop_if_changed` reports whether the VALUE changed, not
                // whether the assignment was handled — a pattern equal to the
                // current contents (e.g. re-assigning the same literal) must
                // still count as handled, or the caller falls back to the
                // generic path and clobbers the correctly-laid-out value.
                self.set_prop_if_changed(handle, &prop, acc);
                return true;
            }
            return false;
        }
        // Declared-type destination the registered-signal table cannot name:
        // a whole packed-struct element of a fixed array on an instance
        // (`u_ctrl.cfg_arr[2] = '{...}`, packed or unpacked). Resolve through
        // the read-side `declared_packed_access` so the write lands at the
        // same bit offsets the reads use.
        if let Some((storage, off, width, layout)) = self.declared_elem_pattern_target(lvalue, items)
        {
            if let Some(mut base_val) = self.get_local_or_signal(&storage) {
                let mut acc = Value::zero(width);
                for b in 0..width {
                    acc.set_bit(b as usize, base_val.get_bit((off + b) as usize));
                }
                if self.apply_packed_struct_pattern(&mut acc, &layout, "", 0, items) {
                    for b in 0..width {
                        base_val.set_bit((off + b) as usize, acc.get_bit(b as usize));
                    }
                    self.set_local_or_signal(&storage, base_val);
                    return true;
                }
            }
        }
        false
    }

    /// Evaluate a named/ordered/default assignment pattern targeting a PACKED
    /// struct lvalue and return the correctly-laid-out value at the LVALUE's
    /// own width (0-based within the target field). For NBA / continuous
    /// assigns, whose scheduling drives the resolved (possibly sub-member)
    /// target — driving the whole base signal instead would clobber sibling
    /// members updated in the same step. Returns None when the lvalue is not
    /// a registered packed struct, a packed-struct class property, or a
    /// packed-struct element of a fixed array.
    pub(super) fn eval_packed_struct_pattern(
        &mut self,
        lvalue: &Expression,
        items: &[AssignmentPatternItem],
    ) -> Option<Value> {
        if let Some((obj_name, prefix, layout, base_off, width)) =
            self.resolve_packed_struct_target(lvalue, items)
        {
            // Seed from the current field value so members the pattern omits keep
            // their prior contents.
            if let Some(base_val) = self.get_local_or_signal(&obj_name) {
                let mut acc = Value::zero(width);
                for b in 0..width {
                    acc.set_bit(b as usize, base_val.get_bit((base_off + b) as usize));
                }
                if self.apply_packed_struct_pattern(&mut acc, &layout, &prefix, base_off, items) {
                    return Some(acc);
                }
            }
        }
        // Class-property destination: evaluate at the property's declared
        // layout, seeded from the current property value. The scheduled
        // store (NBA / settling) then writes this value through the normal
        // class-property path.
        if let Some((handle, prop, layout)) = self.class_prop_pattern_target(lvalue, items) {
            let total = layout.iter().map(|(_, o, w)| o + w).max().unwrap_or(0);
            let mut acc = Value::zero(total);
            if let Some(seed) = self
                .heap
                .get(handle)
                .and_then(|o| o.as_ref())
                .and_then(|i| i.properties.get(&prop))
            {
                for b in 0..total {
                    acc.set_bit(b as usize, seed.get_bit(b as usize));
                }
            }
            if self.apply_packed_struct_pattern(&mut acc, &layout, "", 0, items) {
                return Some(acc);
            }
            return None;
        }
        // Declared-type destination (a fixed-array struct element on an
        // instance): evaluate at the element's own layout, 0-based within
        // the element — the scheduled store places the value.
        if let Some((storage, off, width, layout)) = self.declared_elem_pattern_target(lvalue, items)
        {
            if let Some(base_val) = self.get_local_or_signal(&storage) {
                let mut acc = Value::zero(width);
                for b in 0..width {
                    acc.set_bit(b as usize, base_val.get_bit((off + b) as usize));
                }
                if self.apply_packed_struct_pattern(&mut acc, &layout, "", 0, items) {
                    return Some(acc);
                }
            }
        }
        None
    }

    /// Resolve a class-property destination for a packed-struct assignment
    /// pattern: `obj.prop = '{...}` (a handle-valued name), `this.prop` /
    /// `super.prop`, or a bare `prop` inside a method. Returns the heap
    /// handle, the property's storage key, and the property's declared
    /// packed-struct member layout. The registered-signal path cannot see
    /// these destinations: their storage is the heap object's `properties`
    /// cell, not a signal, and `packed_struct_fields` has no entry for a
    /// dotted class-property name.
    fn class_prop_pattern_target(
        &mut self,
        lvalue: &Expression,
        items: &[AssignmentPatternItem],
    ) -> Option<(usize, String, Vec<(String, u32, u32)>)> {
        // §8.14/§8.15: `super.p`, and `h.p` / `this.p` naming a property a
        // derived class redeclares, select a base class's cell; take the
        // spelling `assign_value` writes, never the leaf's `p`.
        if let Some(rw) = self
            .super_rewrite(lvalue)
            .or_else(|| self.shadowed_member_rewrite(lvalue))
        {
            return self.class_prop_pattern_target(&rw, items);
        }
        // Keyed/Typed items denote associative or type-keyed aggregates, not
        // a by-member struct pack — leave those to the existing paths.
        if items.iter().any(|it| {
            matches!(
                it,
                AssignmentPatternItem::Keyed(..) | AssignmentPatternItem::Typed(..)
            )
        }) {
            return None;
        }
        let cand: Option<(usize, String)> = match &lvalue.kind {
            // `obj.prop` as a dotted identifier.
            ExprKind::Ident(hier)
                if hier.path.len() == 2 && hier.path.iter().all(|s| s.selects.is_empty()) =>
            {
                // `this.prop` / `super.prop` fold into a dotted identifier
                // whose head is the keyword, not a handle variable: the head
                // cannot be evaluated by name, so resolve it through
                // this_stack (the object `super` denotes here as well).
                if hier.path[0].name.name.as_str() == "this"
                    || hier.path[0].name.name.as_str() == "super"
                {
                    self.this_stack
                        .last()
                        .copied()
                        .flatten()
                        .map(|hd| (hd, hier.path[1].name.name.clone()))
                } else {
                    let head = hier_prefix(hier);
                    if !self.head_denotes_class_object(&head) {
                        return None;
                    }
                    let handle = self
                        .eval_expr(&Expression::new(ExprKind::Ident(head), lvalue.span))
                        .to_u64()? as usize;
                    Some((handle, hier.path[1].name.name.clone()))
                }
            }
            // `recv.prop` where `recv` is a bare handle name or `this`/`super`.
            ExprKind::MemberAccess { expr, member } => match &expr.kind {
                // `this.prop` with the keyword as its own expression form.
                ExprKind::This => self
                    .this_stack
                    .last()
                    .copied()
                    .flatten()
                    .map(|hd| (hd, member.name.clone())),
                ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                    let name = h.path[0].name.name.as_str();
                    if name == "this" || name == "super" {
                        self.this_stack
                            .last()
                            .copied()
                            .flatten()
                            .map(|hd| (hd, member.name.clone()))
                    } else {
                        // Only a genuine class channel: a plain integral
                        // value must not be misread as a heap handle.
                        if !self.bare_name_is_class_channel(name) {
                            return None;
                        }
                        self.eval_ident_handle(name).map(|hd| (hd, member.name.clone()))
                    }
                }
                _ => None,
            },
            // Bare `prop` inside a method: the current `this` object. A
            // shadowing local/argument was already served by the
            // registered-signal path above; only a real property remains.
            ExprKind::Ident(hier) if hier.path.len() == 1 && hier.path[0].selects.is_empty() => {
                let handle = self.this_stack.last().copied().flatten()?;
                let name = hier.path[0].name.name.clone();
                let key = self.shadowed_prop_key(&name, false).unwrap_or(name);
                if !self
                    .heap
                    .get(handle)?
                    .as_ref()?
                    .properties
                    .contains_key(&key)
                {
                    return None;
                }
                Some((handle, key))
            }
            _ => None,
        };
        let (handle, prop) = cand?;
        // A live object holding this property, whose declared type is a
        // packed struct.
        if !self
            .heap
            .get(handle)?
            .as_ref()?
            .properties
            .contains_key(&prop)
        {
            return None;
        }
        let su = self.class_prop_struct(handle, &prop)?;
        if !su.packed {
            return None;
        }
        let layout = super::elaborate::packed_struct_field_layout(
            &DataType::Struct(su),
            &self.module.parameters,
            &self.module.typedefs,
            &self.module.typedef_types,
        )?;
        Some((handle, prop, layout))
    }

    /// Resolve a declared-type destination for a packed-struct assignment
    /// pattern that the registered-signal table cannot name: a whole
    /// packed-struct element of a FIXED array (`u_ctrl.cfg_arr[2] = '{...}`,
    /// packed or unpacked). Uses the read-side resolver
    /// (`declared_packed_access`) so the write lands at the same bit offsets
    /// the reads use. Returns the storage signal, the element's bit offset
    /// and width within it, and the element type's member layout.
    fn declared_elem_pattern_target(
        &mut self,
        lvalue: &Expression,
        items: &[AssignmentPatternItem],
    ) -> Option<(String, u32, u32, Vec<(String, u32, u32)>)> {
        if items.iter().any(|it| {
            matches!(
                it,
                AssignmentPatternItem::Keyed(..) | AssignmentPatternItem::Typed(..)
            )
        }) {
            return None;
        }
        let access = self.declared_packed_access(lvalue)?;
        // A whole packed struct: no remaining packed dimensions, and not an
        // element still inside an unpacked dimension run.
        if access.unpacked != 0 || !access.dimensions.is_empty() {
            return None;
        }
        // Queue / dynamic / associative element writes maintain sizes and
        // have their own pattern paths — do not bypass them here.
        let base = match access.storage.rfind('[') {
            Some(i) => access.storage[..i].to_string(),
            None => access.storage.clone(),
        };
        if self.module.queue_vars.contains(&base)
            || self.module.dynamic_arrays.contains(&base)
            || self.module.associative_arrays.contains_key(&base)
        {
            return None;
        }
        if !matches!(&access.element_type, DataType::Struct(su) if su.packed) {
            return None;
        }
        let layout = super::elaborate::packed_struct_field_layout(
            &access.element_type,
            &self.module.parameters,
            &self.module.typedefs,
            &self.module.typedef_types,
        )?;
        let total = layout.iter().map(|(_, o, w)| o + w).max().unwrap_or(0);
        if total == 0 || total != access.width {
            return None;
        }
        Some((access.storage, access.offset, access.width, layout))
    }

    /// Write each member of a packed-struct assignment pattern into `acc` at
    /// `absolute_offset - base_offset`. `layout` is the base signal's flattened
    /// `(dotted_field, offset, width)` list; `prefix` is the field path of the
    /// struct being written (`""` for the whole base signal). Pass
    /// `base_offset = 0` with a whole-base `acc` to write at absolute offsets,
    /// or the field's offset with a field-width `acc` to write 0-based within
    /// it. Recurses for nested sub-struct patterns. Returns false when `prefix`
    /// names no struct (it has no direct child fields in the layout).
    pub(super) fn apply_packed_struct_pattern(
        &mut self,
        acc: &mut Value,
        layout: &[(String, u32, u32)],
        prefix: &str,
        base_offset: u32,
        items: &[AssignmentPatternItem],
    ) -> bool {
        let pfx = if prefix.is_empty() {
            String::new()
        } else {
            format!("{}.", prefix)
        };
        // Direct children of `prefix`: keys of the form "<pfx><leaf>" whose leaf
        // has no further '.' (deeper leaves belong to a nested sub-struct).
        let mut children: Vec<(String, String, u32, u32)> = Vec::new();
        for (k, off, w) in layout {
            if let Some(rest) = k.strip_prefix(&pfx) {
                if !rest.is_empty() && !rest.contains('.') {
                    children.push((k.clone(), rest.to_string(), *off, *w));
                }
            }
        }
        if children.is_empty() {
            return false;
        }
        // The first-declared member is the MSB, i.e. the highest offset. Ordered
        // items bind in declaration order.
        children.sort_by(|a, b| b.2.cmp(&a.2));

        let mut named: HashMap<&str, &Expression> = HashMap::default();
        let mut ordered: Vec<&Expression> = Vec::new();
        let mut default: Option<&Expression> = None;
        for it in items {
            match it {
                AssignmentPatternItem::Named(id, e) => {
                    named.insert(id.name.as_str(), e);
                }
                AssignmentPatternItem::Ordered(e) => ordered.push(e),
                AssignmentPatternItem::Default(e) => default = Some(e),
                _ => {}
            }
        }

        for (i, (full, leaf, off, w)) in children.iter().enumerate() {
            let keyed = named
                .get(leaf.as_str())
                .copied()
                .or_else(|| ordered.get(i).copied());
            // §10.9.2: `default:` reaches into a nested struct member and
            // fills each of its members (`'{trans_id: id, default: 1}` sets
            // `exception.cause` and `exception.valid` to 1);
            // the whole member took the value, so only its LSB was set.
            if keyed.is_none()
                && let Some(d) = default
                && !matches!(d.kind, ExprKind::AssignmentPattern(_))
                && layout
                    .iter()
                    .any(|(k, _, _)| k.len() > full.len() + 1 && k.starts_with(&format!("{full}.")))
            {
                let sub = [AssignmentPatternItem::Default(d.clone())];
                if self.apply_packed_struct_pattern(acc, layout, full, base_offset, &sub) {
                    continue;
                }
            }
            let expr = keyed.or(default);
            let Some(e) = expr else { continue };
            // A nested pattern on a sub-struct member recurses so each of ITS
            // members lands at the right offset too.
            if let ExprKind::AssignmentPattern(sub) = &e.kind {
                if self.apply_packed_struct_pattern(acc, layout, full, base_offset, sub) {
                    continue;
                }
            }
            let piece = if let ExprKind::AssignmentPattern(sub) = &e.kind {
                // A nested pattern on a NON-struct member (a packed vector,
                // `bit [2:0][3:0] y` with `'{4,5,6}`): split the member width
                // evenly across the positional items — the generic eval would
                // concat the items at their own widths. The empty-dims call
                // derives the element count from the item list itself.
                self.packed_pattern_value(sub, &[], *w)
                    .unwrap_or_else(|| self.eval_expr_ctx(e, *w).resize(*w))
            } else {
                let raw = self.eval_expr_ctx(e, *w);
                if raw.is_real {
                    // §6.12.2: a real item CONVERTS (rounds) to the member
                    // type; resize would reinterpret its IEEE-754 bits.
                    Self::real_to_int(raw.to_f64(), *w)
                } else {
                    raw.resize(*w)
                }
            };
            let dst = off - base_offset;
            for b in 0..*w {
                acc.set_bit((dst + b) as usize, piece.get_bit(b as usize));
            }
        }
        true
    }

    /// Resolve `arr[i]…[j].field…[k]` over a PACKED array of packed structs to
    /// the `(backing_signal, lsb, width)` slice it denotes, where `base` is the
    /// expression left of the trailing `[k]` and `idx` is that index.
    ///
    /// Returns None for anything that is not this shape — an unpacked array of
    /// structs (whose elements ARE separate signals), a plain struct, or a base
    /// with no registered element/field metadata — so callers fall through to
    /// the existing paths unchanged.
    /// Walk a `.field` / `[const]` chain down to its root identifier, returning
    /// the root's flat storage name and the steps below it, outermost first.
    /// A step is `(Some(field), _)` or `(None, index)`.
    pub(super) fn packed_path_steps<'e>(
        &mut self,
        e: &'e Expression,
    ) -> Option<(String, Vec<(Option<&'e str>, i64)>)> {
        let mut steps: Vec<(Option<&'e str>, i64)> = Vec::new();
        let mut cur: &'e Expression = e;
        loop {
            match &cur.kind {
                ExprKind::MemberAccess { expr, member } => {
                    steps.push((Some(member.name.as_str()), 0));
                    cur = expr.as_ref();
                }
                ExprKind::Index { expr, index } => {
                    steps.push((None, self.eval_expr(index).to_i64()?));
                    cur = expr.as_ref();
                }
                _ => break,
            }
        }
        steps.reverse();
        let ExprKind::Ident(h) = &cur.kind else {
            return None;
        };
        Some((self.resolve_hier_name(h).into_owned(), steps))
    }

    /// Resolve a walked path over a PACKED aggregate to the
    /// `(backing_signal, lsb, width)` slice it denotes.
    ///
    /// The steps alternate in the nested case — `node.words[0].words[0]` is field,
    /// index, field, index — so the path is rendered into the dotted-with-
    /// indices key elaboration already registers (`words[0].words`) rather than
    /// being matched against a fixed "indices then fields" shape. Matching that
    /// fixed shape resolved the first group and then gave up, so every access
    /// below one level of nesting read as 0.
    ///
    /// Returns None for anything not this shape, so callers fall through to the
    /// existing paths unchanged.
    pub(super) fn packed_path_slice(
        &mut self,
        root: &str,
        steps: &[(Option<&str>, i64)],
    ) -> Option<(String, u32, u32)> {
        // Inside an inlined instance a member access folds into the root
        // (`cw.loc.descriptors`): that name carries an element width but no
        // field layout, so every nested read below it came back 0. Peel
        // trailing segments back into field steps until the root names the
        // struct signal itself.
        if !self.module.packed_struct_fields.contains_key(root)
            && !self.module.arrays.contains_key(root)
        {
            if let Some((head, seg)) = root.rsplit_once('.') {
                let mut widened: Vec<(Option<&str>, i64)> = Vec::with_capacity(steps.len() + 1);
                widened.push((Some(seg), 0));
                widened.extend_from_slice(steps);
                return self.packed_path_slice(head, &widened);
            }
        }
        // Leading indices apply to the root's own dimensions; the rest form the
        // field path, which may carry indices of its own.
        let lead_len = steps
            .iter()
            .take_while(|(field, _)| field.is_none())
            .count();
        let rest = &steps[lead_len..];
        if rest.is_empty() {
            return None; // a pure index chain is not a field path
        }
        // An UNPACKED dimension stores each element as its own signal, so the
        // leading indices covering it belong in the signal NAME; the layout
        // stays keyed by the array's base name.
        let mut storage = root.to_string();
        let mut consumed = 0usize;
        if self.module.arrays.contains_key(root) {
            for &(_, i) in &steps[..lead_len] {
                let cand = format!("{}[{}]", storage, i);
                if self.get_signal_value_by_name(&cand).is_some() {
                    storage = cand;
                    consumed += 1;
                } else {
                    break;
                }
            }
            if consumed == 0 {
                return None;
            }
        }
        let pre = &steps[consumed..lead_len];
        let fields = self.module.packed_struct_fields.get(root)?;
        // Width of ONE struct, taken from the field layout. NOT
        // `packed_signal_elem_widths[root]`: that is the width of an element of
        // the OUTERMOST dimension, which for `t [0:0][1:0]` is the whole [1:0]
        // sub-array (two structs), not one.
        let struct_w = fields.iter().map(|(_, o, w)| o + w).max()?;
        // Flatten the leading chain to a slot. `packed_full_dims` carries the
        // declared ranges so a non-zero-based or ascending dimension maps the
        // way §7.4.1 labels it; without dims fall back to row-major.
        let slot = if pre.is_empty() {
            0
        } else {
            let dims = self.module.packed_full_dims.get(root).map(Vec::as_slice);
            Self::flatten_packed_slot_iter(pre.iter().map(|(_, index)| *index), dims)?
        };
        // Render the field path with indices inline, matching the registered key.
        let render_key = |steps: &[(Option<&str>, i64)]| -> String {
            let mut key = String::new();
            for (f, i) in steps {
                match f {
                    Some(f) => {
                        if !key.is_empty() {
                            key.push('.');
                        }
                        key.push_str(f);
                    }
                    None => {
                        key.push('[');
                        Self::push_i64(&mut key, *i);
                        key.push(']');
                    }
                }
            }
            key
        };
        let key = render_key(rest);
        // Checked: an out-of-range index must miss, not wrap into another bit.
        let base = slot.checked_mul(struct_w)?;
        // Exact hit: the whole path names a registered field.
        if let Some((_, off, w)) = fields.iter().find(|(m, _, _)| m == &key) {
            let (off, w) = (*off, *w);
            let total = self.get_signal_value_by_name(&storage)?.width;
            let lsb = base.checked_add(off)?;
            return (lsb.checked_add(w)? <= total).then_some((storage, lsb, w));
        }
        // Otherwise walk the path: fields and indices interleaved at any
        // depth (`wdata[0]`, `descriptors[0].region`, `m[1].words[2]`). Every
        // registered key is element-0-relative along its whole path, so the
        // deepest field supplies the offset and each index step adds its
        // member's element displacement on top. The trailing-index-only
        // shape used to be the ceiling; a field AFTER an index read 0.
        // Layouts come in two key conventions: element-relative
        // (`descriptors.region`) and expanded per element with the index
        // inline (`wdata[0].wdata`, `wdata[1].wdata`). A field step tries the
        // path with its ACTUAL indices first (absolute offset, so the index
        // displacement collected so far is dropped), then the index-free
        // form, then the element-0 form (both element-0-relative, so the
        // displacement stays).
        let mut norm = String::new();
        let mut actual = String::new();
        let mut zero = String::new();
        let mut off: u32 = 0;
        let mut fw: u32 = struct_w;
        let mut disp: u32 = 0;
        let mut i = 0;
        while i < rest.len() {
            match rest[i] {
                (Some(f), _) => {
                    for k in [&mut norm, &mut actual, &mut zero] {
                        if !k.is_empty() {
                            k.push('.');
                        }
                        k.push_str(f);
                    }
                    if let Some((_, o, w)) = fields.iter().find(|(m, _, _)| m == &actual) {
                        off = *o;
                        fw = *w;
                        disp = 0;
                    } else {
                        let (_, o, w) = fields.iter().find(|(m, _, _)| m == &norm || m == &zero)?;
                        off = *o;
                        fw = *w;
                    }
                    i += 1;
                }
                (None, _) => {
                    if norm.is_empty() {
                        return None;
                    }
                    let mkey = format!("{}.{}", root, norm);
                    let mut idxs: Vec<i64> = Vec::new();
                    while i < rest.len() && rest[i].0.is_none() {
                        idxs.push(rest[i].1);
                        actual.push('[');
                        Self::push_i64(&mut actual, rest[i].1);
                        actual.push(']');
                        zero.push_str("[0]");
                        i += 1;
                    }
                    let ew = self.module.packed_signal_elem_widths.get(&mkey).copied();
                    let (step_w, slot) = if idxs.len() == 1 {
                        // A member with no element width is a plain vector:
                        // the index is a bit select.
                        (ew.unwrap_or(1), u32::try_from(idxs[0]).ok()?)
                    } else {
                        let ew = ew?;
                        let dims = self.module.packed_full_dims.get(&mkey)?;
                        let slot = Self::flatten_packed_slot_iter(
                            idxs.iter().copied(),
                            Some(dims.as_slice()),
                        )?;
                        let inner: u32 = dims
                            .iter()
                            .skip(1)
                            .take(idxs.len() - 1)
                            .map(|(l, r)| ((l - r).unsigned_abs() + 1) as u32)
                            .product();
                        (ew / inner.max(1), slot)
                    };
                    disp = disp.checked_add(slot.checked_mul(step_w)?)?;
                    fw = step_w.min(fw);
                }
            }
        }
        let lsb = base.checked_add(off)?.checked_add(disp)?;
        let total = self.get_signal_value_by_name(&storage)?.width;
        (lsb.checked_add(fw)? <= total).then_some((storage, lsb, fw))
    }

    /// Resolve `arr[i]…[j].field…[k]` over a PACKED array of packed structs to
    /// the `(backing_signal, lsb, width)` slice it denotes, where `base` is the
    /// expression left of the trailing `[k]` and `idx` is that index.
    pub(super) fn packed_struct_elem_slice(
        &mut self,
        base: &Expression,
        idx: &Expression,
    ) -> Option<(String, u32, u32)> {
        // Left of the trailing index must be `<...>.field`.
        if !matches!(base.kind, ExprKind::MemberAccess { .. }) {
            return None;
        }
        let (root, mut steps) = self.packed_path_steps(base)?;
        // With no index anywhere but the trailing one this is a plain struct
        // member select, which the existing member path already handles — leave
        // it there rather than re-deriving it here.
        if steps.iter().all(|(f, _)| f.is_some()) {
            return None;
        }
        steps.push((None, self.eval_expr(idx).to_i64()?));
        self.packed_path_slice(&root, &steps)
    }

    /// Resolve `<...>.member` where the receiver is itself an indexed packed
    /// aggregate (`n.wdata[0].amask`). Used as a LAST resort, after every other
    /// member path has declined, so established lookups keep priority.
    pub(super) fn packed_member_slice<'e>(
        &mut self,
        expr: &'e Expression,
        member: &'e str,
    ) -> Option<(String, u32, u32)> {
        if !matches!(expr.kind, ExprKind::Index { .. }) {
            return None;
        }
        let (root, mut steps) = self.packed_path_steps(expr)?;
        steps.push((Some(member), 0));
        self.packed_path_slice(&root, &steps)
    }

    /// Row-major flatten of an index chain over a packed array's declared
    /// dimensions. `dims` are outermost-first `(left, right)` declared ranges;
    /// the trailing entries that describe the ELEMENT type are ignored.
    pub(super) fn flatten_packed_slot(indices: &[u64], dims: Option<&[(i64, i64)]>) -> Option<u32> {
        Self::flatten_packed_slot_iter(indices.iter().map(|&index| index as i64), dims)
    }

    pub(super) fn flatten_packed_slot_iter(
        indices: impl IntoIterator<Item = i64>,
        dims: Option<&[(i64, i64)]>,
    ) -> Option<u32> {
        let Some(dims) = dims else {
            // No declared dims on file: treat the chain as row-major over
            // equal-sized dimensions, which is the historical behaviour.
            return indices
                .into_iter()
                .last()
                .and_then(|index| u32::try_from(index).ok());
        };
        let mut slot: u64 = 0;
        let mut any = false;
        for (n, i) in indices.into_iter().enumerate() {
            any = true;
            let (l, r) = *dims.get(n)?;
            let (lo, hi) = (l.min(r), l.max(r));
            let count = (hi - lo + 1) as u64;
            let off = (i - lo) as u64;
            if off >= count {
                return None;
            }
            // §7.4.1: a descending range labels the LEFT bound as the most
            // significant element; an ascending one reverses the slot order.
            let s = if l >= r { off } else { count - 1 - off };
            slot = slot * count + s;
        }
        any.then(|| u32::try_from(slot).ok()).flatten()
    }

    pub(super) fn assign_value(&mut self, lhs: &Expression, val: &Value) -> bool {
        // §20.2: a function can terminate simulation while its caller is
        // evaluating an assignment's RHS. Do not commit that caller's write.
        // Final procedures run with `finished` cleared (§9.2.3).
        if self.finished {
            return false;
        }
        if matches!(&lhs.kind, ExprKind::MemberAccess { expr, .. }
                if !matches!(&expr.kind, ExprKind::Ident(h) if h.path.len() == 1))
            || matches!(&lhs.kind, ExprKind::Ident(h) if h.path.len() >= 3)
        {
            if let Some(ok) = self.assign_static_through_chain(lhs, val) {
                return ok;
            }
        }
        if let ExprKind::MemberAccess { expr: b, .. }
        | ExprKind::Index { expr: b, .. }
        | ExprKind::RangeSelect { expr: b, .. } = &lhs.kind
            && matches!(
                b.kind,
                ExprKind::Call { .. } | ExprKind::MemberAccess { .. }
            )
            && let Some(changed) = self.recv_call_once_assign(lhs, val)
        {
            return changed;
        }
        if !self.static_local_names.is_empty()
            && !self.in_const_param_eval
            && let Some((n, key)) = self.static_select_target(lhs)
        {
            return self.assign_static_select(n, &key, lhs, val);
        }
        // §6.8/§6.11.1: a write to a 2-state procedural local drops X/Z
        // (whole, bit- or part-select). Probed only for an X/Z value.
        let changed = if val.has_xz() {
            self.assign_value_xz(lhs, val)
        } else {
            self.assign_value_inner(lhs, val)
        };
        if changed && !self.condition_waiters.is_empty() {
            self.check_condition_waiters_for_write(lhs);
        }
        changed
    }

    /// §6.21: `assign_value` for a select or member write to the `static`
    /// subroutine local `name`, stored under `key` (`a[7:0] = v`; see
    /// `static_select_target`).
    /// The write updates the frame copy in place, while reads come from the
    /// live shared cell, so bring the copy up to date first and store it
    /// back after, or the write is lost.
    #[inline(never)]
    pub(super) fn assign_static_select(
        &mut self,
        name: &str,
        key: &str,
        lhs: &Expression,
        val: &Value,
    ) -> bool {
        // An index that calls a function can write the same static, so
        // evaluate the indices (once, §11.4.1) before taking the copy. An
        // x/z index selects nothing, so the write is discarded (§11.5.1).
        let frozen;
        let lhs = if Self::select_indices_plain(lhs) {
            lhs
        } else {
            let mut bad = false;
            frozen = self.freeze_select_indices(lhs, &mut bad);
            if bad {
                return false;
            }
            &frozen
        };
        self.static_cell_sync(name, key, false);
        let changed = if val.has_xz() {
            self.assign_value_xz(lhs, val)
        } else {
            self.assign_value_inner(lhs, val)
        };
        self.static_cell_sync(name, key, true);
        if changed && !self.condition_waiters.is_empty() {
            self.check_condition_waiters_for_write(lhs);
        }
        changed
    }

    /// `assign_value_inner` for a value carrying X/Z: a 2-state procedural
    /// local target (`lvalue_is_two_state_local`) receives it with X/Z
    /// mapped to 0.
    #[cold]
    #[inline(never)]
    pub(super) fn assign_value_xz(&mut self, lhs: &Expression, val: &Value) -> bool {
        // §7.2.1: a 2-state member of a 4-state packed struct.
        if !self.mixed_packed_vars.is_empty()
            && let Some(v) = self.mixed_packed_member_write(lhs, val)
        {
            return self.assign_value_inner(lhs, &v);
        }
        if self.lvalue_is_two_state_local(lhs) {
            let v = val.to_two_state();
            self.assign_value_inner(lhs, &v)
        } else {
            self.assign_value_inner(lhs, val)
        }
    }

    /// Does `lhs` (a bare local, or a bit/part/element select of one) write
    /// a procedural local declared with a 2-state type? Its declared type is
    /// the latest `var_decl_types` entry for the name (every declaration
    /// replaces it; a subroutine's are restored on return), so nothing is
    /// paid per declaration — only a write carrying X/Z asks. Table signals
    /// carry their own `signal_two_state` mark, a class property or module
    /// variable seen from a subroutine is no local, and a fallback's carried
    /// locals are fitted on copy-back.
    pub(super) fn lvalue_is_two_state_local(&self, lhs: &Expression) -> bool {
        let mut root = lhs;
        loop {
            match &root.kind {
                ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => root = expr,
                ExprKind::Paren(inner) => root = inner,
                _ => break,
            }
        }
        let ExprKind::Ident(h) = &root.kind else {
            return false;
        };
        if h.root.is_some() || h.path.len() != 1 {
            return false;
        }
        let name = h.path[0].name.name.as_str();
        let in_frame = self
            .local_stack
            .last()
            .is_some_and(|f| f.contains_key(name));
        if in_frame {
            if self.fb_frame_depths.last().copied() == Some(self.local_stack.len() - 1) {
                return false;
            }
        } else if !self.in_process_block_scope()
            || self.signal_name_to_id.contains_key(name)
            || !(self.signals.contains_key(name) || self.module.arrays.contains_key(name))
        {
            // Inside a subroutine every local is in its frame: a name
            // outside it is a class property or a module variable. (A
            // process block's static locals stay in the signal table under
            // a loop's variable frame, §6.21.)
            return false;
        }
        self.module.var_decl_types.get(name).is_some_and(|dt| {
            !super::elaborate::is_type_real(dt)
                && super::elaborate::is_type_two_state_resolved(dt, &self.module.typedef_types)
        })
    }

    /// §13.5.2: an element, bit or part-select access on a `ref` formal
    /// (`a[i]`, `a[i][j]`, `a[7:0]`, `a[i].f`) goes THROUGH to the caller's
    /// actual — for a formal aliased to module-visible storage the storage
    /// name replaces the formal; for an aggregate bound by identity (no local
    /// copy exists) the actual's own hierarchical name is extended with the
    /// formal's selects. Whole-name reads and writes keep their existing
    /// routing (see `ref_alias_stack`); this covers the selected forms, which
    /// previously wrote a dead local copy while the matching read went to
    /// the storage — `a[1] = 9` inside the task read back x.
    pub(super) fn ref_formal_redirect_hier(
        &self,
        hier: &crate::ast::expr::HierarchicalIdentifier,
        selected: bool,
    ) -> Option<crate::ast::expr::HierarchicalIdentifier> {
        if hier.root.is_some() || hier.path.is_empty() {
            return None;
        }
        // A bare name is redirected only underneath an `Index`/`RangeSelect`
        // wrapper (`selected`); the whole-name forms keep their own routing.
        if !selected && hier.path.len() == 1 && hier.path[0].selects.is_empty() {
            return None;
        }
        let name = hier.path[0].name.name.as_str();
        // §13.5.2: an UNPACKED-struct formal binds member-wise into the frame
        // (its `<formal>.` marker) and copies back on return, as the
        // MemberAccess arm of `ref_formal_redirect_inner` already honours.
        // Redirecting a member ELEMENT (`r.arr[i]`, `r.mm[i][j]`) to the
        // actual bypassed that copy, and the stale frame value then
        // clobbered the write at return. Only a member whose storage is
        // per-element leaves (no frame key of its own) is exempt; a member
        // that is one frame leaf keeps the redirect. Probed only for a name
        // that would redirect.
        let member_wise_struct = |sim: &Self| {
            if !sim.any_struct_formal_markers
                || hier.path.len() < 2
                || hier.path.iter().any(|s| !s.selects.is_empty())
            {
                return false;
            }
            let Some(frame) = sim.local_stack.last() else {
                return false;
            };
            let path = hier
                .path
                .iter()
                .map(|s| s.name.name.as_str())
                .collect::<Vec<_>>()
                .join(".");
            frame.contains_key(format!("{}.", name).as_str()) && !frame.contains_key(path.as_str())
        };
        if let Some(storage) = self.ref_alias_stack.last().and_then(|m| m.get(name)) {
            // A formal bound to an actual of the SAME name (`run_checks(ref
            // u7_t [..] cnt)` called as `run_checks(cnt)`) rewrote the
            // identifier to itself, and the evaluator re-entered this
            // redirect on the rewritten node until the stack overflowed.
            // Plain resolution of the name already reaches that storage.
            if storage == name || member_wise_struct(self) {
                return None;
            }
            let mut path = hier.path.clone();
            path[0].name.name = storage.clone();
            return Some(crate::ast::expr::HierarchicalIdentifier {
                root: None,
                path,
                span: hier.span,
                cached_signal_id: std::cell::Cell::new(None),
                cached_resolved_name: std::cell::OnceCell::new(),
            });
        }
        // Identity-bound aggregate: the formal has no storage of its own.
        // Formals the copy-in machineries own (scalars with a local slot, 1-D
        // arrays, associative arrays, queues — copied back on return) are not
        // listed: a direct write to the actual would be undone by the stale
        // copy-back.
        if !self
            .ref_identity_stack
            .last()
            .is_some_and(|v| v.iter().any(|n| n == name))
            || member_wise_struct(self)
        {
            return None;
        }
        let actual = self.ref_binding_stack.last().and_then(|m| m.get(name))?;
        let ExprKind::Ident(ah) = &actual.kind else {
            return None;
        };
        if ah.path.is_empty() {
            return None;
        }
        // A self-mapped binding (`a` -> `a`) would rewrite forever.
        if ah.path.len() == 1 && ah.path[0].selects.is_empty() && ah.path[0].name.name == name {
            return None;
        }
        let mut path = ah.path.clone();
        let mut rest = hier.path.clone();
        let first = rest.remove(0);
        path.last_mut().unwrap().selects.extend(first.selects);
        path.extend(rest);
        Some(crate::ast::expr::HierarchicalIdentifier {
            root: ah.root.clone(),
            path,
            span: hier.span,
            cached_signal_id: std::cell::Cell::new(None),
            cached_resolved_name: std::cell::OnceCell::new(),
        })
    }

    /// `ref_formal_redirect_hier` applied through `Index`/`RangeSelect`
    /// wrappers down to the root identifier of an lvalue or operand.
    pub(super) fn ref_formal_redirect_expr(&self, e: &Expression) -> Option<Expression> {
        if !self.ref_redirect_possible() {
            return None;
        }
        self.ref_formal_redirect_inner(e, false)
    }

    /// IEEE 1800-2023 §13.5.2: a `ref` formal names the actual's storage,
    /// so an event control on it (`@(posedge sig)`, `@(sig)`, `@(b[2])`)
    /// waits for a change of the ACTUAL. The formal itself is a frame value
    /// with no signal behind it: the wait resolved to nothing, fell through
    /// to the NBA-region fallback and resumed in the same time step. Each
    /// term naming a formal aliased to module-visible storage (or an
    /// identity-bound aggregate) is rewritten onto the actual. `None` when
    /// no term changes.
    pub(super) fn ref_event_control(&self, event: &EventControl) -> Option<EventControl> {
        let redirect = |e: &Expression| -> Option<Expression> {
            let mut inner = e;
            while let ExprKind::Paren(x) = &inner.kind {
                inner = x;
            }
            self.ref_formal_redirect_inner(inner, true)
        };
        match event {
            EventControl::HierIdentifier(e) => redirect(e).map(EventControl::HierIdentifier),
            EventControl::Identifier(id) => {
                let e = Expression::new(
                    ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                        root: None,
                        path: vec![crate::ast::expr::HierPathSegment {
                            name: id.clone(),
                            selects: Vec::new(),
                        }],
                        span: id.span,
                        cached_signal_id: std::cell::Cell::new(None),
                        cached_resolved_name: std::cell::OnceCell::new(),
                    }),
                    id.span,
                );
                redirect(&e).map(EventControl::HierIdentifier)
            }
            EventControl::EventExpr(terms) => {
                let mut out: Option<Vec<crate::ast::stmt::EventExpr>> = None;
                for (k, t) in terms.iter().enumerate() {
                    if let Some(ne) = redirect(&t.expr) {
                        let v = out.get_or_insert_with(|| terms.clone());
                        v[k].expr = ne;
                    }
                }
                out.map(EventControl::EventExpr)
            }
            EventControl::Star | EventControl::ParenStar => None,
        }
    }

    /// Cheap reject for the hot paths: only a call with an aliased or an
    /// identity-bound `ref` formal can redirect anything.
    #[inline(always)]
    pub(super) fn ref_redirect_possible(&self) -> bool {
        self.ref_redirect_hot
    }

    pub(super) fn refresh_ref_redirect_hot(&mut self) {
        self.ref_redirect_hot = self.ref_alias_stack.last().is_some_and(|m| !m.is_empty())
            || self
                .ref_identity_stack
                .last()
                .is_some_and(|v| !v.is_empty());
    }

    pub(super) fn ref_formal_redirect_inner(
        &self,
        e: &Expression,
        selected: bool,
    ) -> Option<Expression> {
        match &e.kind {
            ExprKind::Ident(h) => self
                .ref_formal_redirect_hier(h, selected)
                .map(|nh| Expression::new(ExprKind::Ident(nh), e.span)),
            ExprKind::Index { expr, index } => {
                self.ref_formal_redirect_inner(expr, true).map(|ne| {
                    Expression::new(
                        ExprKind::Index {
                            expr: Box::new(ne),
                            index: index.clone(),
                        },
                        e.span,
                    )
                })
            }
            ExprKind::RangeSelect {
                expr,
                kind,
                left,
                right,
            } => self.ref_formal_redirect_inner(expr, true).map(|ne| {
                Expression::new(
                    ExprKind::RangeSelect {
                        expr: Box::new(ne),
                        kind: kind.clone(),
                        left: left.clone(),
                        right: right.clone(),
                    },
                    e.span,
                )
            }),
            // §13.5.2: `r.f = v` on a `ref` formal — the MemberAccess shape
            // (how a member write parses inside a subroutine body) missed
            // the redirect, so the write fell to the member-write
            // fall-throughs, which read the formal's integral value back as
            // a heap handle (id-collision-in-heap) or vanished on a phantom
            // name. Redirect the RECEIVER to the caller's actual and rebuild
            // the access. An UNPACKED-struct formal is exempt: it binds
            // member-wise (frame keys `<formal>.<member>`) and the return
            // copy-back delivers the frame copy to the caller's actual —
            // redirecting here would bypass that copy and let the stale
            // call-time value clobber the write at return.
            ExprKind::MemberAccess { expr, member } => {
                if let ExprKind::Ident(rh) = &expr.kind {
                    if rh.path.len() == 1 && rh.path[0].selects.is_empty() {
                        let key = format!("{}.{}", rh.path[0].name.name, member.name);
                        // A member with per-element (or nested) leaves has
                        // no key of its own; the formal's `<formal>.` marker
                        // still says it is bound member-wise.
                        if self.local_stack.last().is_some_and(|f| {
                            f.contains_key(&key)
                                || (self.any_struct_formal_markers
                                    && f.contains_key(
                                        format!("{}.", rh.path[0].name.name).as_str(),
                                    ))
                        }) {
                            return None;
                        }
                    }
                }
                self.ref_formal_redirect_inner(expr, true).map(|ne| {
                    Expression::new(
                        ExprKind::MemberAccess {
                            expr: Box::new(ne),
                            member: member.clone(),
                        },
                        e.span,
                    )
                })
            }
            _ => None,
        }
    }

    /// Store `val` into select `index` of `elem`, one element (signal `id`)
    /// of the unpacked array `arr`: a bit, or a whole sub-vector when the
    /// element is a multi-dimensional PACKED array (a lane of `logic
    /// [1:0][63:0] q [0:2]` is 64 bits wide). `None` for a label this plain
    /// 0-based / ascending mapping does not cover (negative or otherwise
    /// exotic packed labels, `logic [-1:-5][31:0] d [-2:-4]`), which the
    /// normalizing paths further down resolve.
    pub(super) fn store_unpacked_elem_select(
        &mut self,
        arr: &str,
        elem: &str,
        id: usize,
        index: &Expression,
        val: &Value,
    ) -> Option<bool> {
        let w = self.signal_widths[id];
        let Some(raw) = self.eval_expr(index).to_index() else {
            return Some(false); // §11.5.1: x/z index writes nothing
        };
        let pos = match self.module.ascending_packed.get(arr).copied() {
            Some((lo_l, hi_l)) if raw >= lo_l && raw <= hi_l => Some((hi_l - raw) as u32),
            Some(_) => None,
            None if raw >= 0 => Some(raw as u32),
            None => None,
        };
        let ew = self
            .module
            .packed_signal_elem_widths
            .get(arr)
            .copied()
            .unwrap_or(1)
            .max(1);
        let lo = pos
            .map(|p| p.saturating_mul(ew))
            .filter(|&lo| lo.checked_add(ew).is_some_and(|hi| hi <= w))?;
        let mut cur = self.signal_table[id].clone();
        let prev = cur.clone();
        for b in 0..ew {
            cur.set_bit((lo + b) as usize, val.get_bit(b as usize));
        }
        let changed = cur != prev;
        if changed {
            if !self.dirty_signals[id] {
                self.dirty_signals[id] = true;
                self.dirty_list.push(id);
            }
            self.dirty_any = true;
            write_sig!(self, id, cur);
            self.table_modified = true;
            self.after_signal_write(id);
            self.mark_dirty(elem);
        }
        Some(changed)
    }

    /// §6.16: a character-indexed write `s[i] = byte` through a base the
    /// flat-name string path cannot see. The string-char block in
    /// `assign_value_index` keys on `string_signals` + a resolvable flat
    /// `name`, so it misses:
    ///   - a CLASS-member string (`this.s[i]`, `c.s[i]`, `C::s[i]`,
    ///     `c.b.s[i]`), a `MemberAccess` or dotted base with no flat signal
    ///     name, and
    ///   - a string-array / string-queue ELEMENT (`sa[k][i]`, `q[k][i]`,
    ///     `s2[a][b][i]`), which arrives here with `expr` = `sa[k]` (an
    ///     `Index`), not the collection name.
    /// Both would otherwise byte-select into the packed container (bit-bash)
    /// or vanish. Reads the current string, replaces byte `i`, and stores the
    /// whole string back via `assign_value` — exactly what the flat-name block
    /// does. Returns `None` unless `expr` is provably a SCALAR string (see
    /// `string_char_receiver_is_scalar`), so callers fall through to the
    /// existing arms unchanged.
    pub(super) fn try_string_index_char_write(
        &mut self,
        expr: &Expression,
        index: &Expression,
        val: &Value,
    ) -> Option<bool> {
        // Only the shapes the flat-name block can't reach: a member, an
        // element, or a dotted ident (`c.s` parses as a two-segment
        // HierarchicalIdentifier when the receiver is a module-scope handle,
        // vs. the MemberAccess shape `this.s` gets inside a method). A
        // single-segment bare Ident in `string_signals` already lands in the
        // flat-name block.
        let is_member = matches!(&expr.kind, ExprKind::MemberAccess { .. });
        let is_elem = matches!(&expr.kind, ExprKind::Index { .. });
        let is_dotted_ident = matches!(&expr.kind, ExprKind::Ident(h) if h.path.len() >= 2);
        if !is_member && !is_elem && !is_dotted_ident {
            return None;
        }
        // §7.4.4 / §7.8 / §7.10: a select on a string COLLECTION, or on a
        // partial element of a multi-dimensional one, picks a whole element
        // (`c.b.q[1] = "x"`, `s2[1][2] = "x"`), not a character.
        if !self.string_char_receiver_is_scalar(expr) {
            return None;
        }
        // §7.8: a row of an associative array of collections
        // (`string m[int][string]`, `m[3]["x"] = "hi"`) is itself a
        // collection; `[k]` selects its element. Taken for a character
        // index, the write was dropped.
        if let ExprKind::Index { expr: arr, .. } = &expr.kind
            && let ExprKind::Ident(h) = &arr.kind
            && h.path.len() == 1
            && h.path[0].selects.is_empty()
        {
            let name = self.resolve_hier_name(h).into_owned();
            let name = self.instance_assoc_member(&name).unwrap_or(name);
            if self.is_associative_array(&name) && self.assoc_has_coll_elems(&name) {
                return None;
            }
        }
        let mut b = self.eval_expr(expr).sv_string_bytes();
        let i = self.eval_expr(index).to_u64().unwrap_or(0) as usize;
        let cbyte = (val.resize(8).to_u64().unwrap_or(0) & 0xFF) as u8;
        if i < b.len() && cbyte != 0 {
            b[i] = cbyte;
            // A `Value` may carry string metadata; store the freshly rebuilt
            // string so the element keeps its string-ness.
            return Some(self.assign_value(expr, &Self::sv_string_from_bytes(&b)));
        }
        Some(false)
    }

    /// Whether the associative array stored at `name` has collections for
    /// elements: a module-scope or block-local declaration recorded by core,
    /// or an object's member (`<h>#m`) whose class records it.
    fn assoc_has_coll_elems(&self, name: &str) -> bool {
        if self.module.assoc_coll_elems.contains(name) {
            return true;
        }
        if let Some((h, prop)) = name.split_once('#')
            && let Ok(handle) = h.parse::<usize>()
            && let Some(cd) = self.class_prop_declarer(handle, prop)
        {
            return cd.assoc_coll_properties.contains(prop);
        }
        false
    }

    /// §7.8.6/§11.5.1: a bit write into an associative element whose
    /// element type is integral (`m[k][3] = 1`, `sb["k"][2] = 1`), absent or
    /// not: read it (an absent one as the element type's default), set the
    /// bit and write it back. An absent element was created with the bit
    /// dropped, and a present one's bit went under a bogus `m[k][3]` key.
    /// None when `expr` is not such an element.
    /// §7.8/§7.10: a write into a row of an associative array of
    /// collections (`aq[6][0] = 9` on `int aq[int][$]`) whose row does not
    /// exist yet creates the row as the collection its declaration names,
    /// as `push_back` does, so the element is stored in that row (the write
    /// left a bare `aq[6][0]` cell no queue read, `%p` or `foreach` saw).
    /// Every level of the key chain is created in turn: `a["p"][7]["q"] = 9`
    /// on `byte a[string][int][string]` makes the rows `a["p"]` and
    /// `a["p"][7]`, each keyed by its own declared key type.
    /// Returns the innermost row when it is a QUEUE, for the append rule.
    fn ensure_assoc_row(&mut self, row_expr: &Expression) -> Option<String> {
        // A row is a select (`m[k]`); a plain `a[i] = v` has none to create.
        if !matches!(row_expr.kind, ExprKind::Index { .. }) {
            return None;
        }
        // An associative root names its keys by level; any other root (a
        // queue of associative arrays, a local's `@l#n`) by its flat name.
        let row = self
            .nested_index_name(row_expr)
            .or_else(|| self.flat_member_name(row_expr))?;
        if !self.is_declared_row(&row) || !self.ensure_row(&row) {
            return None;
        }
        matches!(self.declared_dim(&row), Some(Dim::Queue)).then_some(row)
    }

    fn assoc_elem_bit_write(
        &mut self,
        expr: &Expression,
        index: &Expression,
        val: &Value,
    ) -> Option<bool> {
        let ExprKind::Index {
            expr: inner_expr,
            index: inner_idx,
        } = &expr.kind
        else {
            return None;
        };
        let ExprKind::Ident(hier) = &inner_expr.kind else {
            return None;
        };
        let name = self.resolve_hier_name(hier).into_owned();
        let name = self.instance_assoc_member(&name).unwrap_or(name);
        if !self.is_associative_array(&name) || self.assoc_has_coll_elems(&name) {
            return None;
        }
        let default = self.assoc_missing_elem_value(&name)?;
        let kv = self.eval_expr(inner_idx);
        if kv.has_xz() && !kv.is_real && !self.is_string_keyed_array(&name) {
            self.warn_invalid_assoc_index(&name);
            return Some(false);
        }
        let bv = self.eval_expr(index);
        let Some(b) = bv.to_i64().filter(|_| !bv.has_xz()) else {
            return Some(false);
        };
        let key = self.assoc_key_str(&name, &kv);
        let elem = format!("{}[{}]", name, key);
        // A row that is itself a collection (`aq[k]` of `int aq[int][$]`):
        // `[j]` selects its element, which the collection paths handle.
        if self.is_associative_array(&elem)
            || self.module.dynamic_arrays.contains(&elem)
            || self
                .signals
                .contains_key(&Self::name_with_suffix(&elem, ".size"))
        {
            return None;
        }
        let fresh = !self.signals.contains_key(&elem);
        let mut cur = self.get_signal_value_by_name(&elem).unwrap_or(default);
        let phys = match self.select_base_elem_dim(expr) {
            Some((dl, dr)) if dl < dr => dr - b,
            Some((dl, dr)) => b - dl.min(dr),
            None => b,
        };
        if phys < 0 || phys as u32 >= cur.width {
            return Some(false);
        }
        let nb = val.get_bit(0);
        let changed = cur.get_bit(phys as usize) != nb;
        cur.set_bit(phys as usize, nb);
        if changed || fresh {
            self.set_signal_value_by_name(&elem, cur);
            self.mark_dirty(&elem);
            self.touch_queue(&name);
        }
        Some(changed)
    }

    /// Outlined from `assign_value_inner` (see `exec_stmt_blocking_assign`): keeps
    /// the dispatcher's stack frame small.
    #[inline(never)]
    pub(super) fn assign_value_index(
        &mut self,
        lhs: &Expression,
        val: &Value,
        expr: &Box<Expression>,
        index: &Box<Expression>,
    ) -> bool {
        if let Some(r) = self.try_string_index_char_write(expr, index, val) {
            return r;
        }
        if let Some(r) = self.assoc_elem_bit_write(expr, index, val) {
            return r;
        }
        if self.has_coll_rows
            && let Some(row) = self.ensure_assoc_row(expr)
        {
            // §7.10.1: a write at index == size APPENDS to a queue row
            // (`q["x"][5][0] = 7` on an empty row makes it '{7}).
            let iv = self.eval_expr(index);
            let sz = self.get_queue_size(&row);
            if !iv.has_xz() && iv.to_u64() == Some(sz) {
                self.set_queue_size(&row, sz + 1);
            }
        }
        // §11.5.1: a hierarchical class property's selected element lives
        // in the heap/shared static cell, rather than a synthesized signal.
        if let ExprKind::Ident(hier) = &expr.kind
            && let Some(h) = self.hier_parent_handle(hier)
        {
            let prop = &hier.path.last().unwrap().name.name;
            let cur = self
                .static_read_via_handle(h, None, None, prop)
                .or_else(|| self.read_member_value(h, prop));
            if let Some(mut cur) = cur {
                let idx = self.eval_expr(index);
                if idx.has_xz() {
                    return false;
                }
                let range = if let Some(dims) = self.class_prop_packed_dims(h, prop) {
                    Self::packed_chain_slice(cur.width as u64, &dims, &[idx])
                } else {
                    idx.to_i64()
                        .filter(|&i| i >= 0 && i < cur.width as i64)
                        .map(|i| (i as u64, 1))
                };
                let Some((lo, width)) = range else {
                    return false;
                };
                let rhs = val.resize_for_assign(width as u32);
                for bit in 0..width as usize {
                    cur.set_bit(lo as usize + bit, rhs.get_bit(bit));
                }
                return self.set_prop_if_changed(h, prop, cur);
            }
        }
        // §7.4.1 / §11.5.1: BIT write into an ELEMENT of an unpacked
        // array/queue/assoc whose declared packed range needs label
        // mapping (`logic [31:8] a[0:0]; a[0][8] = b` — storage bit
        // 0, not 8; ascending mirrors). The arms below all treat the
        // trailing index as a raw storage offset, so the write landed
        // on the wrong bit. Width must match the dim exactly — a
        // multi-packed element's trailing index selects a SLICE and
        // belongs to the packed-element machinery below.
        if let Some((dl, dr)) = self.select_base_elem_dim(expr) {
            if dl < dr || dl.min(dr) != 0 {
                let dim_w = (dl - dr).unsigned_abs() + 1;
                if let Some(en) = self.flat_member_name(expr) {
                    if let Some(cur) = self.get_signal_value_by_name(&en) {
                        if cur.width as u64 == dim_w {
                            let idx_v = self.eval_expr(index);
                            if idx_v.has_xz() {
                                return false; // §11.5.1: x/z index selects nothing
                            }
                            let l = idx_v.to_i64().unwrap_or(i64::MIN);
                            let phys = if dl >= dr { l - dr } else { dr - l };
                            if phys < 0 || phys >= cur.width as i64 {
                                return false; // out-of-range bit write discarded
                            }
                            let nb = val.get_bit(0);
                            let changed = cur.get_bit(phys as usize) != nb;
                            if changed {
                                let mut nv = cur;
                                nv.set_bit(phys as usize, nb);
                                self.set_signal_value_by_name(&en, nv);
                            }
                            return changed;
                        }
                    }
                }
            }
        }
        // §7.4.1: packed ELEMENT write on a plain packed-of-packed
        // variable (`u8_vec16_t y; y[i] = v;`) — splice
        // [i*ew +: ew], honoring the by-NAME storage of
        // subroutine locals and formals. MUST run before the
        // collection fallback below, which would otherwise invent a
        // flat `y[i]` entry disconnected from the packed value (the
        // pixel-conversion battery's silent-agreement bug).
        if let ExprKind::Ident(h) = &expr.kind {
            if h.path.len() == 1 && h.path[0].selects.is_empty() {
                let base = self.resolve_hier_name(h);
                let bare = &h.path[0].name.name;
                let elem_key = if self.module.packed_signal_elem_widths.contains_key(&*base) {
                    Some(base.to_string())
                } else if self
                    .module
                    .packed_signal_elem_widths
                    .contains_key(bare.as_str())
                {
                    Some(bare.clone())
                } else {
                    None
                };
                // Collections keep their own semantics.
                let is_collection = self.module.arrays.contains_key(&*base)
                    || self.module.arrays.contains_key(bare.as_str())
                    || self.module.dynamic_arrays.contains(&*base)
                    || self.module.associative_arrays.contains_key(&*base);
                if let Some(pkey) = elem_key.filter(|_| !is_collection) {
                    let elem_w = *self.module.packed_signal_elem_widths.get(&pkey).unwrap();
                    let idx_v = self.eval_expr(index);
                    if idx_v.has_xz() {
                        return false; // §11.5.1: x/z index selects nothing
                    }
                    let idx = idx_v.to_i64().unwrap_or(i64::MIN);
                    // Frame-aware storage: subroutine locals live in
                    // the call frame, module vars in the signal maps —
                    // get/set_local_or_signal covers both.
                    let storage_key = if self.get_local_or_signal(&base).is_some() {
                        Some(base.to_string())
                    } else if self.get_local_or_signal(bare).is_some() {
                        Some(bare.clone())
                    } else {
                        None
                    };
                    if let Some(key) = storage_key {
                        if let Some(mut cur) = self.get_local_or_signal(&key) {
                            if cur.width > elem_w {
                                let piece = val.resize(elem_w);
                                let Some(lo) = self.packed_elem_lsb(&pkey, idx, elem_w) else {
                                    return false;
                                };
                                if lo + (elem_w as usize) <= cur.width as usize {
                                    let prev = cur.clone();
                                    for i in 0..(elem_w as usize) {
                                        cur.set_bit(lo + i, piece.get_bit(i));
                                    }
                                    let changed = cur != prev;
                                    if changed {
                                        self.set_local_or_signal(&key, cur);
                                    }
                                    return changed;
                                }
                                return false;
                            }
                        }
                    }
                }
            }
        }
        // GitHub #86 fast path: `mem[i] = v` on a module-scope 1-D
        // FIXED unpacked array resolves to a contiguous signal id with
        // no string work (`array_first_id`), skipping the guard chain
        // below (packed-struct slice, class ND, queue, assoc) that a
        // memory fill paid per element (~2 µs each, all of it name
        // formatting + hashing). Guards: no live call frames (a
        // task/function local could shadow the name), and the name
        // must not be a queue/dynamic/associative collection —
        // `module.arrays` gives those a fake 0..63 backing range, and
        // their writes need the collection semantics below.
        if self.local_stack.is_empty() && self.no_class_objects() {
            if let ExprKind::Ident(h) = &expr.kind {
                if h.path.iter().all(|seg| seg.selects.is_empty()) {
                    let name = self.resolve_hier_name(h);
                    if let Some(&(first_id, lo, hi)) = self.array_first_id.get(name.as_ref()) {
                        if !self.module.queue_vars.contains(&*name)
                            && !self.module.dynamic_arrays.contains(&*name)
                            && !self.module.associative_arrays.contains_key(&*name)
                            // string elements are DYNAMIC-length; the
                            // resize-to-declared-width below would
                            // truncate them (a 200-char element came
                            // back 16 chars) — the general path stores
                            // strings unresized.
                            && !self.module.string_signals.contains(&*name)
                        {
                            let idx = self.eval_expr(index).to_i64().unwrap_or(i64::MIN);
                            if idx >= lo && idx <= hi {
                                let id = first_id + (idx - lo) as usize;
                                return self.fast_signal_write_id(id, val);
                            }
                            // out of range: fall through to the general
                            // path for its existing diagnostics/no-op.
                        }
                    }
                }
            }
        }
        // §7.4.2: `arr[i].field[k] = v` where `arr` is a PACKED array
        // of packed structs. `arr[i]` is not a signal of its own — it
        // is a slice of one backing vector — so the write must splice
        // at `slot*struct_w + field_off + k*member_elem_w`.
        //
        // The MemberAccess arm below already handles `arr[i].field = v`
        // (no trailing index), but this form's outermost node is an
        // Index, so it never reached that code and the write was
        // silently DROPPED — the read path resolved it correctly all
        // along, which is what made the value look like it was never
        // driven rather than never stored.
        if let Some((sig, lsb, w)) = self.packed_struct_elem_slice(expr, index) {
            if let Some(cur_sig) = self.get_signal_value_by_name(&sig) {
                let mut cur = cur_sig.clone();
                let piece = val.resize(w);
                let mut changed = false;
                for i in 0..w {
                    let nb = piece.get_bit(i as usize);
                    if cur.get_bit((lsb + i) as usize) != nb {
                        cur.set_bit((lsb + i) as usize, nb);
                        changed = true;
                    }
                }
                if changed {
                    self.set_signal_value_by_name(&sig, cur);
                }
                return changed;
            }
        }

        // Element of an unpacked ARRAY OF QUEUES: `q[i][k] = v` writes
        // element k of the queue `q[i]` (§7.4.5), not a bit of a scalar.
        // §7.4.2: a write to an N-D fixed-array class-property
        // element lands in its per-instance cell, like the read.
        if !self.no_class_objects() {
            if let Some(nm) = self.class_nd_elem_name(expr, index) {
                let prev = self.get_signal_value_by_name(&nm);
                let fitted = match prev.as_ref() {
                    Some(p) if !p.is_real && !val.is_real && p.width > 0 => val.resize(p.width),
                    _ => val.clone(),
                };
                let changed = prev.as_ref() != Some(&fitted);
                if changed {
                    self.signals.insert(nm, fitted);
                }
                return changed;
            }
        }
        // Element write into an ARRAY-OF-COLLECTIONS class property
        // (`expected[1][0][k] = v`, `dq[1][2] = v`, `aa[0][key] = v`):
        // the base names one element collection (see
        // `class_array_of_coll_key`); write its element.
        if let Some(row) = self.class_array_of_coll_key(expr) {
            if self.module.dynamic_arrays.contains(&row) {
                self.dollar_bound.push(self.get_queue_size(&row) as i64 - 1);
                let k = self.eval_expr(index).to_i64().unwrap_or(0);
                self.dollar_bound.pop();
                if k >= 0 && k as u64 >= self.get_queue_size(&row) {
                    self.set_queue_size(&row, k as u64 + 1);
                }
                let elem = format!("{}[{}]", row, k);
                // §10.6/§5.7.1: the element takes the row's element width
                // (a `'1` fill fills every bit of a struct element).
                let val = match self.module.arrays.get(&row) {
                    Some(&(_, _, w)) if w != val.width || val.is_fill => val.resize_for_assign(w),
                    _ => val.clone(),
                };
                let prev = self.get_signal_value_by_name(&elem);
                let changed = prev.as_ref() != Some(&val);
                self.set_signal_value_by_name(&elem, val);
                return changed;
            }
            if self.is_associative_array(&row) {
                let kv = self.eval_expr(index);
                let key = self.assoc_key_str(&row, &kv);
                let elem = format!("{}[{}]", row, key);
                let prev = self.get_signal_value_by_name(&elem);
                let changed = prev.as_ref() != Some(val);
                self.set_signal_value_by_name(&elem, val.clone());
                return changed;
            }
        }
        if let Some(qn) = self.indexed_queue_base(expr) {
            // `$` is the last valid index of THIS queue.
            self.dollar_bound.push(self.get_queue_size(&qn) as i64 - 1);
            let k = self.eval_expr(index).to_i64().unwrap_or(0);
            self.dollar_bound.pop();
            // §7.10.2.3 / §7.4: assigning `q[i]` with `i >= size`
            // APPENDS to a queue (auto-grows), size becomes `i+1`.
            // UVM's `uvm_unpack_queueN` fills an EMPTY queue exactly
            // this way (`for i in 0..sz VAR[i] = …`), so without the
            // growth the unpacked `.size()` stayed 0 and every
            // subsequent field read desynced.
            if k >= 0 && k as u64 >= self.get_queue_size(&qn) {
                self.set_queue_size(&qn, k as u64 + 1);
            }
            let elem = format!("{}[{}]", qn, k);
            let prev = self.get_signal_value_by_name(&elem);
            let changed = prev.as_ref() != Some(val);
            self.set_signal_value_by_name(&elem, val.clone());
            return changed;
        }
        // `a[i][key] = v` on an array of associative arrays.
        if let Some(elem) = self.indexed_assoc_elem(expr, index) {
            let prev = self.get_signal_value_by_name(&elem);
            let changed = prev.as_ref() != Some(val);
            self.set_signal_value_by_name(&elem, val.clone());
            return changed;
        }
        // Multidimensional associative-array write
        // `m[k1][k2]...[kn] = v`: store under the flat compound key
        // `m[K1][K2]...[Kn]` (see `multidim_assoc_elem`). Without this
        // the write is silently lost and later reads / `.exists()` /
        // recursion guards see nothing (IEEE 1800-2023 §7.8.1).
        if let Some((_, key)) = self.multidim_assoc_elem(expr, index) {
            let changed = self.signals.get(&key).is_none_or(|p| *p != *val);
            if changed {
                self.signals.insert(key, val.clone());
            }
            return changed;
        }
        // `s.arr[i] = v` where `arr` is an UNPACKED ARRAY MEMBER of an
        // unpacked struct. Its elements are individual signals
        // (`s.arr[0]` …) and the BASE `s.arr` is not a signal at all,
        // so every arm here missed and the write degraded into a
        // bit-select of a phantom scalar — silently discarded. The
        // member then read back x forever, which made a scoreboard
        // built on such a struct compare x against every DUT output.
        //
        // Guarded on the base having no signal of its own, so an
        // ordinary packed-vector bit-write (`v[3] = 1`, where `v` IS a
        // signal) never takes this path.
        {
            // `may_create` only for a plain identifier base: that is
            // the procedural-local struct case, where nothing is
            // pre-registered. A MemberAccess base (`q[0].arr[i]`) must
            // find an EXISTING element — creating one there would
            // hijack class-member arrays and virtual-interface writes,
            // which own their own storage.
            let (base, may_create) = match &expr.kind {
                ExprKind::Ident(h) => (Some(self.resolve_hier_name(h)), true),
                ExprKind::MemberAccess { .. } => (
                    self.flat_member_name(expr).map(std::borrow::Cow::Owned),
                    false,
                ),
                _ => (None, false),
            };
            // A member of a PACKED struct is a bit-slice of the
            // parent signal, not element storage — `foo.a[5] = v` on
            // `struct packed { bit [7:0][7:0] a; ... } foo;` must
            // splice into `foo`. Creating a phantom "foo.a[5]" here
            // swallowed the write silently (the RANGE form worked,
            // which is how it hid). Skip so it reaches the packed
            // element/bit-select arms below.
            let base_is_packed_member = |sim: &Self, b: &str| -> bool {
                if sim.module.packed_signal_elem_widths.contains_key(b) {
                    return true;
                }
                b.rsplit_once('.').is_some_and(|(root, leaf)| {
                    sim.module.packed_struct_fields.get(root).is_some_and(|fs| {
                        fs.iter()
                            .any(|(f, _, _)| f == leaf || f.starts_with(&format!("{}.", leaf)))
                    })
                })
            };
            // §8.3 / §23.9: inside a method, a BARE array name that the
            // object (or its class's static store) declares is the
            // member — a same-named module-level array's elements, found
            // in the signal table first, took `a[i] = v` from a method's
            // `foreach (a[i])` and the object's `a` stayed 0 (sibling of
            // #519 / #534: the name-keyed store must not outrank the
            // member lift that the read side and `expr_assoc_name` use).
            let member_shadows = |sim: &Self, b: &str| -> bool {
                !b.contains('.')
                    && matches!(sim.this_stack.last(), Some(Some(_)))
                    && sim.instance_assoc_member(b).is_some()
            };
            if let Some(base) = base.filter(|b| !base_is_packed_member(self, b)) {
                // An ASSOCIATIVE member (`s.aa["k"] = v`) is keyed, not
                // indexed: the keyed write below formats its key (§7.8).
                if !self.signal_name_to_id.contains_key(base.as_ref())
                    && !self.signals.contains_key(&*base)
                    && !self.module.associative_arrays.contains_key(base.as_ref())
                    && !member_shadows(self, &base)
                {
                    let i = self.eval_expr(index).to_i64().unwrap_or(0);
                    // §7.4.1: a bit-select into a FRAME-local unpacked-struct
                    // MEMBER leaf (`rw_access.byte_en[z] = ...` inside a
                    // procedural block / foreach): the member is pre-
                    // registered as its own leaf in the current call/local
                    // frame (`rw_access.byte_en`), NOT as a registered design
                    // signal, so the generic arms below would mint a phantom
                    // `rw_access.byte_en[z]` element that no whole-member read
                    // consults — the write silently vanished and the reg
                    // bit-bash predictor saw byte-enable 0. Presence of the
                    // leaf (`base` in the frame) is the discriminator: a packed
                    // member is one leaf (splice the bit); an unpacked array
                    // member is per-element leaves (`s.arr[i]`), so `base` is
                    // not itself a leaf and the phantom-element path below
                    // still applies.
                    if i >= 0
                        && let Some(cur) = self
                            .local_stack
                            .last()
                            .and_then(|f| f.get(base.as_ref()))
                            .cloned()
                        && !self.frame_name_is_string(&base)
                    {
                        let bit = i as usize;
                        if bit < cur.width as usize {
                            let nb = val.bits_first();
                            let old = cur.get_bit(bit);
                            let changed = old != nb;
                            if changed {
                                let mut nv = cur;
                                nv.set_bit(bit, nb);
                                if let Some(frame) = self.local_stack.last_mut() {
                                    frame.insert(base.to_string(), nv);
                                }
                            }
                            return changed;
                        }
                    }
                    let elem = Self::name_with_index(&base, i);
                    // A module-level struct has its element signals
                    // pre-registered. A PROCEDURAL-LOCAL one does not —
                    // nothing is registered for it at all — so create the
                    // element on first write. Restricted to a member path
                    // (`base` contains a dot), so a plain local vector's
                    // bit-write is never diverted here.
                    if self.signal_name_to_id.contains_key(elem.as_str())
                        || self.signals.contains_key(&elem)
                        || (may_create && base.contains('.'))
                    {
                        // A block-local fixed array's element is fitted to
                        // its declared type (see `fit_block_local_array_elem`).
                        let fitted = self.fit_block_local_array_elem(&base, &elem, val.clone());
                        let prev = self.get_signal_value_by_name(&elem);
                        let changed = prev.as_ref() != Some(&fitted);
                        self.set_signal_value_by_name(&elem, fitted);
                        return changed;
                    }
                    // An element of a CLASS-PROPERTY collection of structs
                    // (`q[0].arr[i]` in a method) keeps its leaves under the
                    // instance (`<h>#q[0].arr[i]`).
                    if let Some(elem) = self.class_coll_elem_leaf(&base, i) {
                        let prev = self.get_signal_value_by_name(&elem);
                        let changed = prev.as_ref() != Some(val);
                        self.set_signal_value_by_name(&elem, val.clone());
                        return changed;
                    }
                }
            }
        }
        // BIT-select write into a PACKED-STRUCT MEMBER
        // (`t.f2[3] = 1'b1`): the member is a bit-slice of the parent
        // signal, and no arm below handles a MemberAccess base — the
        // write fell through and vanished (the RANGE form worked,
        // which is how it hid). Read the member slice, set the bit,
        // splice back through the central by-name setter.
        if matches!(&expr.kind, ExprKind::MemberAccess { .. })
            || matches!(&expr.kind, ExprKind::Ident(h) if h.path.len() > 1)
        {
            if let Some(base) = self.flat_member_name(expr) {
                // Only the plain single-descending-dimension member:
                // a MULTI-DIM member (`foo.a[5]` on bit [7:0][7:0])
                // selects an ELEMENT, and an ascending member mirrors
                // its bit labels — both belong to the arms below.
                let multi_dim = self
                    .module
                    .packed_signal_elem_widths
                    .get(&base)
                    .is_some_and(|&ew| ew > 1)
                    || self
                        .module
                        .packed_full_dims
                        .get(&base)
                        .is_some_and(|d| d.len() > 1);
                let ascending = base
                    .rsplit_once('.')
                    .is_some_and(|(root, leaf)| self.struct_member_ascending(root, leaf).is_some());
                if !multi_dim && !ascending && self.packed_leaf_of_hier(&base).is_some() {
                    let idx_v = self.eval_expr(index);
                    if idx_v.has_xz() {
                        return false; // §11.5.1: x/z index selects nothing
                    }
                    let i = idx_v.to_i64().unwrap_or(0);
                    if let Some(mut cur) = self.get_signal_value_by_name(&base) {
                        if i >= 0 && (i as u32) < cur.width {
                            let prev = cur.clone();
                            cur.set_bit(i as usize, val.get_bit(0));
                            let changed = cur != prev;
                            if changed {
                                self.set_signal_value_by_name(&base, cur);
                            }
                            return changed;
                        }
                        return false; // out-of-range bit write discarded
                    }
                }
            }
        }
        // Ascending packed vector bit-write (`logic [0:7] pa; pa[i]=b`):
        // label i targets internal bit (W-1)-i (LRM §7.4.1, §11.5.1).
        if let ExprKind::Ident(h) = &expr.kind {
            let nm = self.resolve_hier_name(h);
            if let Some((lo_l, hi_l)) = self.module.ascending_packed.get(&*nm).copied() {
                if let Some(&id) = self.signal_name_to_id.get(nm.as_ref()) {
                    // §11.5.1: an x/z or out-of-range label writes nothing.
                    let Some(i) = self.eval_expr(index).to_index() else {
                        return false;
                    };
                    if i >= lo_l && i <= hi_l {
                        let pos = (hi_l - i) as usize;
                        let mut cur = self.signal_table[id].clone();
                        let prev = cur.clone();
                        cur.set_bit(pos, val.get_bit(0));
                        let changed = cur != prev;
                        if changed {
                            self.signal_table[id] = cur;
                            self.sync_mirror(id);
                            self.table_modified = true;
                            self.after_signal_write(id);
                            self.mark_dirty(&nm);
                        }
                        return changed;
                    }
                }
            }
        }
        // Class associative-array member element write — base may be
        // a bare member (`m[k]=v`) or another object's (`obj.m[k]=v`).
        // Not a BIT of an associative element whose element is a plain
        // integral (`m[k][3] = 1`): that is no collection, and storing the
        // value under `m[k][3]` dropped the bit (§11.5.1).
        let elem_bit = |me: &mut Self| -> bool {
            let ExprKind::Index { expr: b, .. } = &expr.kind else {
                return false;
            };
            let ExprKind::Ident(h) = &b.kind else {
                return false;
            };
            let n = me.resolve_hier_name(h).into_owned();
            let n = me.instance_assoc_member(&n).unwrap_or(n);
            me.is_associative_array(&n)
                && !me.assoc_has_coll_elems(&n)
                && me.assoc_missing_elem_value(&n).is_some()
        };
        if let Some(an) = self.expr_assoc_name(expr).filter(|_| !elem_bit(self)) {
            let idx_val = self.eval_expr(index);
            let idx_str = self.assoc_key_str(&an, &idx_val);
            let elem_name = Self::name_with_key(&an, &idx_str);
            // §7.10.2.3 / §7.4: for a QUEUE / dynamic array (NOT a
            // true assoc array) `q[i] = v` with `i >= size` APPENDS —
            // the size becomes `i+1`. UVM's `uvm_unpack_queueN` fills
            // an EMPTY queue exactly this way
            // (`for i in 0..sz VAR[i] = …`); without this growth the
            // unpacked `.size()` stayed 0 and every later field read
            // desynced (PCKSZ errors / X compare).
            if !self.is_associative_array(&an) {
                let kval = idx_val.to_i64().unwrap_or(0);
                if kval >= 0 && kval as u64 >= self.get_queue_size(&an) {
                    self.set_queue_size(&an, kval as u64 + 1);
                }
            }
            // §10.7: fit to the DECLARED element width when one was
            // recorded; class-member collections often have none, in
            // which case the value is stored as-is (unchanged).
            let mut fitted = match self.assoc_elem_width(&an) {
                Some(w) if w != val.width && !val.is_real => val.resize_for_assign(w),
                _ => self.fit_block_local_array_elem(&an, &elem_name, val.clone()),
            };
            // The element takes its declared signedness, not the RHS's
            // (`byte m[int]; m[k] = 8'hF0` is -16).
            if let Some(sg) = self
                .assoc_elem_signed(&an)
                .filter(|_| !fitted.is_real && self.is_associative_array(&an))
            {
                fitted.is_signed = sg;
            } else if !self.is_associative_array(&an) {
                // §7.10/§10.7: a queue or dynamic-array element takes the
                // element type (an `int unsigned` element of -1 is 2**32-1).
                fitted = self.fit_queue_push_elem(&an, fitted);
                if !fitted.is_real && !fitted.is_signed && self.signed_class_coll(&an) {
                    fitted = self.fit_signed_class_coll_elem(&an, fitted);
                }
            }
            let changed = self.signals.get(&elem_name) != Some(&fitted);
            self.signals.insert(elem_name, fitted);
            if changed {
                self.touch_queue(&an);
            }
            return changed;
        }
        // N-dimensional (N >= 3) unpacked array element assignment
        {
            let mut cur = expr.as_ref();
            let mut rev_idxs: Vec<&Expression> = vec![index];
            while let ExprKind::Index {
                expr: inner_e,
                index: inner_i,
            } = &cur.kind
            {
                rev_idxs.push(inner_i.as_ref());
                cur = inner_e.as_ref();
            }
            if let ExprKind::Ident(hier) = &cur.kind {
                let mut base_name = self.resolve_hier_name(hier);
                // Bare class-member N-D array inside a method:
                // resolve to its per-instance `<handle>#<member>`.
                // A member shadows a module array of the same name (§8.10).
                if !self.module.arrays_nd.contains_key(&*base_name)
                    || (hier.path.len() == 1
                        && hier.path[0].selects.is_empty()
                        && *base_name == *hier.path[0].name.name)
                {
                    if let Some(s) = self.instance_assoc_member(&base_name) {
                        base_name = std::borrow::Cow::Owned(s);
                    }
                }
                // `o.m[i][j][k]` on an object's member: its store is
                // `<handle>#m` (§7.4.5).
                if !self.module.arrays_nd.contains_key(&*base_name) && hier.path.len() >= 2 {
                    if let Some(s) = self
                        .expr_assoc_name(cur)
                        .filter(|s| self.module.arrays_nd.contains_key(s))
                    {
                        base_name = std::borrow::Cow::Owned(s);
                    }
                }
                if let Some((shape, _w)) = self.module.arrays_nd.get(&*base_name).cloned() {
                    if rev_idxs.len() == shape.len() {
                        let mut name = base_name.clone();
                        for i in (0..rev_idxs.len()).rev() {
                            let v = self.eval_expr(rev_idxs[i]).to_u64().unwrap_or(0) as i64;
                            name = std::borrow::Cow::Owned(format!("{}[{}]", name, v));
                        }
                        if let Some(&id) = self.signal_name_to_id.get(name.as_ref()) {
                            let width = self.signal_widths[id];
                            let resized = val.resize(width);
                            let changed = self.signal_table[id] != resized;
                            if changed {
                                if !self.dirty_signals[id] {
                                    self.dirty_signals[id] = true;
                                    self.dirty_list.push(id);
                                }
                                self.dirty_any = true;
                                write_sig!(self, id, resized);
                                self.table_modified = true;
                            }
                            return changed;
                        }
                        let changed = self.signals.get(&*name).is_none_or(|p| *p != *val);
                        if changed {
                            self.signals.insert(name.to_string(), val.clone());
                            self.mark_dirty(&name);
                        }
                        return changed;
                    }
                }
                // §7.4.5 / §11.5.1: one index MORE than a 2-D/N-D array has
                // unpacked dimensions selects INSIDE one element
                // (`logic [2:0] m [1:0][1:0]; m[i][j][k] = b`): the element
                // `m[i][j]` is a signal of its own and `[k]` a bit of it (a
                // slice, for a multi-D packed element). No arm took that
                // shape, so the write was silently dropped, procedurally and
                // as a continuous assign alike.
                if rev_idxs.len() >= 3 {
                    let depth = match self.module.arrays_nd.get(&*base_name) {
                        Some((shape, _)) => shape.len(),
                        None if self.module.arrays_2d.contains_key(&*base_name) => 2,
                        None => 0,
                    };
                    if depth >= 2 && rev_idxs.len() == depth + 1 {
                        let mut elem = base_name.to_string();
                        for ix in rev_idxs[1..].iter().rev() {
                            let v = self.eval_expr(ix);
                            let Some(k) = v.to_i64().filter(|_| !v.has_xz()) else {
                                return false; // §7.4.6: an x/z index writes nothing
                            };
                            elem.push_str(&format!("[{}]", k));
                        }
                        if let Some(&id) = self.signal_name_to_id.get(elem.as_str()) {
                            if let Some(changed) =
                                self.store_unpacked_elem_select(&base_name, &elem, id, index, val)
                            {
                                return changed;
                            }
                        }
                    }
                }
            }
        }
        // §7.4.1: bit-select of an element of a 1-D UNPACKED array
        // (`logic [1:0] q [0:1]; q[i][j] = b`). Each element is its own
        // signal (`q[i]`), so this is a BIT write into that signal — not
        // a 2-D array index. The `arrays_2d` arm below never matched it
        // and every other arm fell through, so the write was silently
        // DROPPED: a pipeline written as `vld_q[i+1][j] <= vld_q[i][j]`
        // never advanced and its output stayed x forever.
        if let ExprKind::Index {
            expr: inner_expr,
            index: inner_idx,
        } = &expr.kind
        {
            if let ExprKind::Ident(hier) = &inner_expr.kind {
                let mut name = self.resolve_hier_name(hier);
                if !self.module.arrays.contains_key(&*name) {
                    if let Some(s) = self.instance_assoc_member(&name) {
                        name = std::borrow::Cow::Owned(s);
                    }
                }
                // Only a plain unpacked array: an `arrays_2d` entry is a
                // genuine two-dimensional index and is handled below.
                if self.module.arrays.contains_key(&*name)
                    && !self.module.arrays_2d.contains_key(&*name)
                    && !self.module.arrays_nd.contains_key(&*name)
                {
                    let i = self.eval_expr(inner_idx).to_i64().unwrap_or(0);
                    let elem = format!("{}[{}]", name, i);
                    if let Some(&id) = self.signal_name_to_id.get(elem.as_str()) {
                        if let Some(changed) =
                            self.store_unpacked_elem_select(&name, &elem, id, index, val)
                        {
                            return changed;
                        }
                    }
                }
                // §7.4.1 / §10.4: bit-select write into an element of
                // an UNREGISTERED (block-local / automatic
                // subroutine-local) unpacked array. The guarded path
                // above needs a module `arrays` registration + compact
                // id; a block-local `larr[1][2] = 1'b1` had neither,
                // and every later arm either targets arrays_2d or
                // invents a bogus "larr[1][2]" key that element reads
                // never see — the write silently vanished. RMW the
                // element through the canonical by-name accessors.
                // Block-local arrays DO appear in `module.arrays`
                // (the declaration registers them at runtime, after
                // the compact tables were built), so the only reliable
                // discriminator is the missing compact id itself.
                // Collection elements (array-of-queue, where `[j]`
                // selects a sub-element, not a bit) were already taken
                // by the earlier queue/assoc branches — and are
                // excluded again here for safety.
                {
                    let iv = self.eval_expr(inner_idx);
                    if let Some(i) = iv.to_i64().filter(|_| !iv.has_xz()) {
                        let elem = format!("{}[{}]", name, i);
                        // Only a RUNTIME-declared element (no compact
                        // id) takes this route: a registered element
                        // that fell through above did so deliberately
                        // (negative packed labels etc.) and must reach
                        // the pre-existing normalizing arms below.
                        let elem_is_collection = self.module.dynamic_arrays.contains(&elem)
                            || self.is_associative_array(&elem);
                        if !elem_is_collection
                            && !self.signal_name_to_id.contains_key(elem.as_str())
                        {
                            if let Some(cur) = self.get_signal_value_by_name(&elem) {
                                let bv = self.eval_expr(index);
                                if let Some(b) = bv.to_i64().filter(|_| !bv.has_xz()) {
                                    if b >= 0 && (b as u32) < cur.width {
                                        let mut nv = cur.clone();
                                        let nb = val.get_bit(0);
                                        let changed = nv.get_bit(b as usize) != nb;
                                        if changed {
                                            nv.set_bit(b as usize, nb);
                                            self.set_signal_value_by_name(&elem, nv);
                                            self.mark_dirty(&elem);
                                        }
                                        return changed;
                                    }
                                    // Out-of-range bit write on a
                                    // runtime-declared element: no-op.
                                    return false;
                                }
                            }
                        }
                    }
                }
            }
        }
        // 2D array element assignment: mem[i][j] = val
        if let ExprKind::Index {
            expr: inner_expr,
            index: inner_idx,
        } = &expr.kind
        {
            if let ExprKind::Ident(hier) = &inner_expr.kind {
                let mut name = self.resolve_hier_name(hier);
                // Bare class-member 2D array inside a method: resolve to its
                // per-instance `<handle>#<member>`. The member shadows a
                // module array of the same name (§8.10); taking the module's
                // first sent the write to it and left the member at 0 (#597).
                let bare_member = hier.path.len() == 1
                    && hier.path[0].selects.is_empty()
                    && *name == *hier.path[0].name.name;
                if !self.module.arrays_2d.contains_key(&*name) || bare_member {
                    if let Some(s) = self.instance_assoc_member(&name) {
                        name = std::borrow::Cow::Owned(s);
                    }
                }
                // `o.m[i][j] = v` on an object's member: its store is
                // `<handle>#m` (§7.4.5).
                if !self.module.arrays_2d.contains_key(&*name) && hier.path.len() >= 2 {
                    if let Some(s) = self
                        .expr_assoc_name(inner_expr)
                        .filter(|s| self.module.arrays_2d.contains_key(s))
                    {
                        name = std::borrow::Cow::Owned(s);
                    }
                }
                // A bare name written from inside a SUBMODULE — e.g. the
                // body of a `foreach` in an `always_comb`, evaluated from
                // the settle path where no scope hint is installed and the
                // per-node name cache may already hold the unscoped form.
                // Try the hint, then a UNIQUE suffix match, so the write
                // lands instead of being silently dropped.
                if !self.module.arrays_2d.contains_key(&*name) && !name.contains('.') {
                    let hinted = self
                        .name_resolve_hint
                        .borrow()
                        .as_ref()
                        .map(|h| format!("{}.{}", h, name));
                    if let Some(h) = hinted.filter(|h| self.module.arrays_2d.contains_key(h)) {
                        name = std::borrow::Cow::Owned(h);
                    } else {
                        let suffix = format!(".{}", name);
                        let mut hit: Option<String> = None;
                        let mut many = false;
                        for k in self.module.arrays_2d.keys() {
                            if k.ends_with(&suffix) {
                                if hit.is_some() {
                                    many = true;
                                    break;
                                }
                                hit = Some(k.clone());
                            }
                        }
                        if let (Some(k), false) = (hit, many) {
                            name = std::borrow::Cow::Owned(k);
                        }
                    }
                }
                if self.module.arrays_2d.contains_key(&*name) {
                    // §7.4.5 / §11.5.1: unpacked indices are signed;
                    // an unknown or out-of-bounds index performs no write.
                    let (Some(i), Some(j)) = (
                        self.eval_expr(inner_idx).to_index(),
                        self.eval_expr(index).to_index(),
                    ) else {
                        return false;
                    };
                    let &((il, ih), (jl, jh), _) = &self.module.arrays_2d[&*name];
                    let (outer_lo, outer_hi) = if self.module.dynamic_arrays.contains(&*name) {
                        // The 2-D table's initial allocation is a placeholder;
                        // new[N] sets the live outer dimension (§7.5.1).
                        (0, self.get_queue_size(&name) as i64 - 1)
                    } else {
                        (il.min(ih), il.max(ih))
                    };
                    if i < outer_lo || i > outer_hi || j < jl.min(jh) || j > jl.max(jh) {
                        return false;
                    }
                    let elem_name = format!("{}[{}][{}]", name, i, j);
                    if let Some(&id) = self.signal_name_to_id.get(elem_name.as_str()) {
                        let width = self.signal_widths[id];
                        let resized = val.resize(width);
                        let changed = self.signal_table[id] != resized;
                        if changed {
                            if !self.dirty_signals[id] {
                                self.dirty_signals[id] = true;
                                self.dirty_list.push(id);
                            }
                            self.dirty_any = true;
                            write_sig!(self, id, resized);
                            self.table_modified = true;
                        }
                        return changed;
                    }
                    let changed = self.signals.get(&elem_name).is_none_or(|p| *p != *val);
                    if changed {
                        self.signals.insert(elem_name.clone(), val.clone());
                        self.mark_dirty(&elem_name);
                        // §9.2.2.2: a queue/dyn/assoc ELEMENT has no
                        // signal id of its own — comb readers depend on
                        // the collection's `.size` proxy, so mark it or
                        // `always_comb` blocks reading `q[i]` never
                        // re-fire on an element write.
                        if let Some(b) = elem_name.find('[').map(|i| &elem_name[..i]) {
                            if self.module.dynamic_arrays.contains(b)
                                || self.module.associative_arrays.contains_key(b)
                            {
                                let b = b.to_string();
                                self.touch_queue(&b);
                            }
                        }
                    }
                    return changed;
                }
            }
        }
        // §7.4.2: an element of a struct member with two or more unpacked
        // dimensions is its own leaf (see `struct_member_elem_leaf`).
        let elem_leaf = if matches!(expr.kind, ExprKind::Index { .. })
            && self.multidim_members != Some(false)
        {
            self.struct_member_elem_leaf(expr, index)
        } else {
            None
        };
        if let Some(leaf) = elem_leaf {
            if let Some(&id) = self.signal_name_to_id.get(leaf.as_str()) {
                let resized = self.fit_value_to_signal(id, val);
                let changed = self.signal_table[id] != resized;
                if changed {
                    if !self.dirty_signals[id] {
                        self.dirty_signals[id] = true;
                        self.dirty_list.push(id);
                    }
                    self.dirty_any = true;
                    write_sig!(self, id, resized);
                    self.table_modified = true;
                }
                return changed;
            }
            let fitted = match self.signals.get(&leaf) {
                Some(p) if p.is_real && !val.is_real => Value::from_f64(val.to_f64()),
                Some(p) if !p.is_real && val.is_real => {
                    let mut f = Self::real_to_int(val.to_f64(), p.width.max(1));
                    f.is_signed = p.is_signed;
                    f
                }
                Some(p) if !p.is_real && p.width > 0 => {
                    let mut f = val.resize(p.width);
                    f.is_signed = p.is_signed;
                    f
                }
                _ => val.clone(),
            };
            let changed = self.signals.get(&leaf) != Some(&fitted);
            if changed {
                self.signals.insert(leaf.clone(), fitted);
                self.mark_dirty(&leaf);
            }
            return changed;
        }
        // Nested packed multi-D element write (`a[i][j] = v` on
        // `reg [1:0][15:0][7:0] a;`) — splice the slice into the flat
        // backing vector, normalizing each index against its declared
        // dimension bounds (ascending / non-zero-based / negative,
        // LRM §7.4.1).
        if let Some((base, lo_opt, w)) = self.packed_nested_select(expr, index) {
            let lo = match lo_opt {
                Some(lo) => lo,
                None => {
                    return false; // out-of-range write dropped §11.5.1
                }
            };
            if let Some(cur_sig) = self.get_signal_value_by_name(&base) {
                let total_w = cur_sig.width as usize;
                if lo + w as usize <= total_w {
                    let mut nv = cur_sig.clone();
                    let piece = val.resize(w);
                    let mut changed = false;
                    for i in 0..w as usize {
                        let nb = piece.get_bit(i);
                        if nv.get_bit(lo + i) != nb {
                            nv.set_bit(lo + i, nb);
                            changed = true;
                        }
                    }
                    if changed {
                        self.set_signal_value_by_name(&base, nv);
                        self.mark_dirty(&base);
                    }
                    return changed;
                }
            }
        }
        // Packed multi-D element bit-select write: `p2d[i][j] = bit`
        // where `p2d` is `logic [..][W-1:0]`. The inner Index selects
        // element i (a W-bit slice at bits [i*W +: W]); the outer index
        // j selects bit j within it. Resolve to the flat bit offset
        // i*W + j and write a single bit (LRM §7.4.1, §11.5.1). Without
        // this the lvalue's `expr` is an Index (not an Ident), so the
        // single-index path below is skipped and the write is lost.
        if let ExprKind::Index {
            expr: inner_expr,
            index: outer_idx,
        } = &expr.kind
        {
            if let ExprKind::Ident(hier) = &inner_expr.kind {
                let base = self.resolve_hier_name(hier);
                if let Some(&elem_w) = self.module.packed_signal_elem_widths.get(&*base) {
                    if let Some(&id) = self.signal_name_to_id.get(base.as_ref()) {
                        let total_w = self.signal_widths[id] as usize;
                        let i = self.eval_expr(outer_idx).to_u64().unwrap_or(0) as usize;
                        let j = self.eval_expr(index).to_u64().unwrap_or(0) as usize;
                        let lo = i * (elem_w as usize) + j;
                        if lo < total_w {
                            let mut cur = self.signal_table[id].clone();
                            let prev = cur.clone();
                            cur.set_bit(lo, val.get_bit(0));
                            let changed = cur != prev;
                            if changed {
                                self.signal_table[id] = cur;
                                self.sync_mirror(id);
                                self.table_modified = true;
                                self.after_signal_write(id);
                                self.mark_dirty(&base);
                            }
                            return changed;
                        }
                    }
                    // Block-local packed multi-D var: lives in the
                    // runtime map, not the compact table.
                    if let Some(cur) = self.get_signal_value_by_name(&base) {
                        let total_w = cur.width as usize;
                        let i = self.eval_expr(outer_idx).to_u64().unwrap_or(0) as usize;
                        let j = self.eval_expr(index).to_u64().unwrap_or(0) as usize;
                        let lo = i * (elem_w as usize) + j;
                        if lo < total_w {
                            let mut nv = cur.clone();
                            nv.set_bit(lo, val.get_bit(0));
                            let changed = nv != cur;
                            if changed {
                                self.set_signal_value_by_name(&base, nv);
                            }
                            return changed;
                        }
                    }
                }
            }
        }
        // LRM §25.8 — `vif.member[i] = ...` (bit-select write
        // through a virtual-interface property). The lvalue parses as
        // Index { expr: MemberAccess { Ident(vif_prop), signal_member },
        //          index }.  The scalar-write path (ExprKind::MemberAccess
        // arm below) handles `vif.member = ...` but misses the indexed
        // form, causing all bit-select writes to vif-bound signals to be
        // silently lost.  Detect and redirect here before the generic
        // flat-name path, which has no knowledge of vif bindings.
        if let ExprKind::MemberAccess {
            expr: ma_base,
            member: ma_member,
        } = &expr.kind
        {
            if let ExprKind::Ident(ma_hier) = &ma_base.kind {
                if ma_hier.path.len() == 1 {
                    let vif_prop = ma_hier.path[0].name.name.as_str();
                    let sig_member = ma_member.name.as_str();
                    // Resolve virtual-interface binding (same lookup as
                    // the scalar MemberAccess arm below ~line 20401).
                    let binding = self
                        .this_stack
                        .last()
                        .copied()
                        .flatten()
                        .and_then(|this_h| {
                            let cls_name = self
                                .heap
                                .get(this_h)
                                .and_then(|o| o.as_ref())
                                .map(|i| i.class_name.clone())?;
                            self.class_vif_decl(&cls_name, vif_prop).map(|_| ()).and(
                                self.virtual_iface_bindings
                                    .get(&(this_h, vif_prop.to_string()))
                                    .cloned(),
                            )
                        });
                    if let Some((bound_name, _modport)) = binding {
                        let target = format!("{}.{}", bound_name, sig_member);
                        let idx = self.eval_expr(index).to_u64().unwrap_or(0) as usize;
                        if let Some(prev) = self.get_signal_value_by_name(&target) {
                            let w = prev.width as usize;
                            if idx < w {
                                let nb = val.get_bit(0);
                                let old = prev.get_bit(idx);
                                let changed = old != nb;
                                if changed {
                                    let mut nv = prev.clone();
                                    nv.set_bit(idx, nb);
                                    self.set_signal_value_by_name(&target, nv);
                                }
                                return changed;
                            }
                        }
                    }
                }
            }
        }
        // The array base may be a flat/hier Ident (`mem[i]`,
        // rewritten submodule refs) OR a MemberAccess chain
        // (`u_h.mem[q]` from a parent scope) — the latter never
        // matched here, so hierarchical element writes were silently
        // dropped while reads (which go through flat_member_name)
        // worked.
        // Element write into a packed multi-dimensional CLASS property:
        // read-modify-write the addressed slice of the whole value.
        if let Some((h, prop, idx_exprs)) = self.prop_index_chain(lhs) {
            if let Some(dims) = self.class_prop_packed_dims(h, &prop) {
                if idx_exprs.len() <= dims.len() {
                    let idx: Vec<Value> = idx_exprs.iter().map(|ie| self.eval_expr(ie)).collect();
                    if let Some(cur) = self.read_member_value(h, &prop) {
                        if let Some((lsb, w)) =
                            Self::packed_chain_slice(cur.width as u64, &dims, &idx)
                        {
                            let piece = val.resize(w as u32);
                            let mut nv = cur.clone();
                            for i in 0..(w as usize) {
                                nv.set_bit(lsb as usize + i, piece.get_bit(i));
                            }
                            let changed = nv != cur;
                            if changed {
                                if let Some(Some(inst)) = self.heap.get_mut(h) {
                                    inst.properties.insert(prop, nv);
                                }
                            }
                            return changed;
                        }
                        return false;
                    }
                }
            }
        }
        let (base_name, hier_opt): (
            Option<std::borrow::Cow<str>>,
            Option<&crate::ast::expr::HierarchicalIdentifier>,
        ) = match &expr.kind {
            ExprKind::Ident(hier) => (Some(self.resolve_hier_name(hier)), Some(hier)),
            ExprKind::MemberAccess { .. } => (
                self.flat_member_name(expr).map(std::borrow::Cow::Owned),
                None,
            ),
            _ => (None, None),
        };
        if let Some(mut name) = base_name {
            if let Some(scoped) = self.instance_assoc_member(&name) {
                name = std::borrow::Cow::Owned(scoped);
            }
            // §8.9: `Cls::S[i] = v` reaches here with the resolver's
            // bare-leaf collapse of the 2-segment form; the original
            // segments name the qualified store.
            if !self.module.arrays.contains_key(&*name) {
                if let Some(h) = hier_opt {
                    if let Some(k) = self.static_fixed_key_from_hier(h) {
                        name = std::borrow::Cow::Owned(k);
                    }
                }
            }
            // A bare name inside a SUBMODULE process resolves under
            // the process's instance scope (name_resolve_hint), like
            // scalar reads/writes already do.
            if !self.module.arrays.contains_key(&*name) && !self.is_associative_array(&name) {
                if let Some(scoped) = self.hint_scoped(&name) {
                    if self.module.arrays.contains_key(&scoped)
                        || self.is_associative_array(&scoped)
                    {
                        name = std::borrow::Cow::Owned(scoped);
                    }
                }
            }
            let idx_val = self.eval_expr(index);
            // §7.8.2: an associative index containing x/z is invalid.
            // The write is ignored (with a warning) — it must not fold
            // the unknown bits away and clobber a real element.
            if self.is_associative_array(&name) && idx_val.has_xz() {
                self.warn_invalid_assoc_index(&name);
                return false;
            }
            let idx_str = if self.is_associative_array(&name) {
                self.assoc_key_str(&name, &idx_val)
            } else {
                idx_val.to_u64().unwrap_or(0).to_string()
            };
            // Check if this is an array element assignment
            if self.module.arrays.contains_key(&*name) || self.is_associative_array(&name) {
                // Compact-resolver fast path for 1D arrays: compute
                // signal_id directly without `format!()` + HashMap
                // lookup. Required for correctness now that 1D
                // array elements are no longer materialised as
                // per-element entries in signal_name_to_id.
                if let Some(&(first_id, lo, hi)) = self.array_first_id.get(name.as_ref()) {
                    let idx = idx_val.to_u64().unwrap_or(0) as i64;
                    if idx >= lo && idx <= hi {
                        let id = first_id + (idx - lo) as usize;
                        if is_packed_id(id) {
                            return self.fast_signal_write_id(id, val);
                        }
                        let width = self.signal_widths[id];
                        // §6.16: a string element has no declared
                        // length — the width is a placeholder.
                        let mut resized = if self.signal_is_string.get(id).copied().unwrap_or(false)
                        {
                            val.clone()
                        } else {
                            val.resize(width)
                        };
                        // §6.11.2/§10.7: a 2-state element (`int a[4]`)
                        // stores an x or z bit of the RHS as 0. Stored
                        // verbatim, `a[k] = a[k] * 3 + x_bit` kept x.
                        if self.signal_two_state.get(id).copied().unwrap_or(false) {
                            resized = resized.to_two_state();
                        }
                        // Adopt the element's declared signedness — a
                        // signed RHS (e.g. an `integer`) stored verbatim
                        // made `reg [15:0] a[]` elements read as signed.
                        resized.is_signed = self.signal_signed[id];
                        let changed = self.signal_table[id] != resized;
                        if changed {
                            if !self.dirty_signals[id] {
                                self.dirty_signals[id] = true;
                                self.dirty_list.push(id);
                            }
                            self.dirty_any = true;
                            write_sig!(self, id, resized);
                            self.table_modified = true;
                        }
                        return changed;
                    }
                }
                let elem_name = format!("{}[{}]", name, idx_str);
                if let Some(&id) = self.signal_name_to_id.get(elem_name.as_str()) {
                    let width = self.signal_widths[id];
                    let mut resized = if self.signal_is_string.get(id).copied().unwrap_or(false) {
                        val.clone()
                    } else {
                        val.resize(width)
                    };
                    // §6.11.2: a 2-state element takes x/z bits as 0.
                    if self.signal_two_state.get(id).copied().unwrap_or(false) {
                        resized = resized.to_two_state();
                    }
                    resized.is_signed = self.signal_signed[id];
                    let changed = self.signal_table[id] != resized;
                    if changed {
                        if !self.dirty_signals[id] {
                            self.dirty_signals[id] = true;
                            self.dirty_list.push(id);
                        }
                        self.dirty_any = true;
                        write_sig!(self, id, resized);
                        self.table_modified = true;
                    }
                    return changed;
                }
                // Fallback: slow path / associative array.
                // §10.7: an assoc element has no entry in the typed
                // signal table, so without the elaborator's recorded
                // element width the RHS was stored at ITS OWN size —
                // `logic [3:0] aa[string]; aa["k"] = 8'hEF` kept all
                // eight bits, and `$bits` reported the stored width
                // rather than the declared one.
                // §6.16: a string element has no fixed length — the
                // recorded width is only the elaborator's placeholder,
                // and fitting to it would chop the text.
                let elem_is_string = self.is_string_collection(&name);
                let mut fitted = match self.assoc_elem_width(&name) {
                    Some(w) if w != val.width && !val.is_real && !elem_is_string => {
                        val.resize_for_assign(w)
                    }
                    _ => val.clone(),
                };
                if !elem_is_string {
                    fitted = self.fit_block_local_array_elem(&name, &elem_name, fitted);
                }
                // The element takes its declared signedness, not the RHS's
                // (`bit [3:0] aa[int]; aa[k] = 9` is 9, not -7).
                if let Some(sg) = self.assoc_elem_signed(&name).filter(|_| {
                    !fitted.is_real && !elem_is_string && self.is_associative_array(&name)
                }) {
                    fitted.is_signed = sg;
                } else if !fitted.is_real && !fitted.is_signed && self.signed_class_coll(&name) {
                    fitted = self.fit_signed_class_coll_elem(&name, fitted);
                }
                if self.warn_x && self.time > 0 {
                    let prev = self.signals.get(&elem_name).cloned();
                    self.warn_x_note_named(&elem_name, prev.as_ref(), &fitted);
                }
                let changed = self.signals.get(&elem_name).is_none_or(|p| *p != fitted);
                if changed {
                    sim_dbg_eprintln!("[DEBUG] signal {} changed to {:?}", elem_name, fitted);
                    self.signals.insert(elem_name.clone(), fitted);
                    self.mark_dirty(&elem_name);
                    // §9.2.2.2: comb readers of `aa[k]` depend on the
                    // collection's `.size` proxy, not the per-key
                    // entry — mark it or they never re-fire.
                    if self.is_associative_array(&name)
                        || self.module.dynamic_arrays.contains(&*name)
                    {
                        self.touch_queue(&name);
                    }
                }
                return changed;
            }
            // String character assignment: `s[i] = "."` replaces the
            // i-th character (byte). `s[0]` is the leftmost char, which
            // sits in the most-significant byte of the packed value.
            if self.frame_name_is_string(&name) {
                // Replace the i-th character of the string CONTENT (not
                // a byte of the fixed-width container, which would write
                // the wrong position and grow the string). Mirrors putc.
                let mut b = self.eval_expr(expr).sv_string_bytes();
                let i = idx_val.to_u64().unwrap_or(0) as usize;
                let cbyte = (val.resize(8).to_u64().unwrap_or(0) & 0xFF) as u8;
                if i < b.len() && cbyte != 0 {
                    b[i] = cbyte;
                    return self.assign_value(expr, &Self::sv_string_from_bytes(&b));
                }
                return false;
            }
            // Packed multi-D signal element assignment (LRM §7.4.1):
            // `w[i] = val` writes an elem_w-bit slice into bits
            // [i*elem_w +: elem_w], not a single bit. Triggered when
            // either `name` (resolved hier) or the dotted-Ident form
            // (`pp.words`) is in packed_signal_elem_widths.
            let pkt_key: Option<String> = {
                if self.module.packed_signal_elem_widths.contains_key(&*name) {
                    Some(name.to_string())
                } else if let ExprKind::Ident(h) = &expr.kind {
                    if h.path.len() >= 2 {
                        let dotted: String = h
                            .path
                            .iter()
                            .map(|s| s.name.name.as_str())
                            .collect::<Vec<_>>()
                            .join(".");
                        if self.module.packed_signal_elem_widths.contains_key(&dotted) {
                            Some(dotted)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            };
            if let Some(pkey) = pkt_key {
                let elem_w = *self.module.packed_signal_elem_widths.get(&pkey).unwrap();
                let idx = idx_val.to_u64().unwrap_or(0) as usize;
                // Resolve underlying signal by either `name` or `pkey`.
                let sig_id = self
                    .signal_name_to_id
                    .get(name.as_ref())
                    .copied()
                    .or_else(|| self.signal_name_to_id.get(pkey.as_str()).copied());
                if let Some(id) = sig_id {
                    let total_w = self.signal_widths[id];
                    let mut cur = self.signal_table[id].clone();
                    let piece = val.resize(elem_w);
                    // Normalized for ascending / non-zero-based outer
                    // dims (§7.4.1) — raw idx*w mirrored [0:N-1] slots.
                    let Some(lo) = self.packed_elem_lsb(&pkey, idx as i64, elem_w) else {
                        return false;
                    };
                    if lo + (elem_w as usize) <= total_w as usize {
                        let prev = cur.clone();
                        for i in 0..(elem_w as usize) {
                            cur.set_bit(lo + i, piece.get_bit(i));
                        }
                        let changed = cur != prev;
                        if changed {
                            self.signal_table[id] = cur;
                            self.sync_mirror(id);
                            self.table_modified = true;
                            self.after_signal_write(id);
                            self.mark_dirty(&pkey);
                        }
                        return changed;
                    }
                }
                // By-NAME storage (function/task locals, formals):
                // the same elem splice against the runtime signal map.
                // Without this a `u8_vec16_t y; y[i] = v;` inside a
                // subroutine degraded to a single-BIT write (the
                // pixel-conversion battery's silent-wrong-agreement
                // bug: reference models and DUT shared the defect).
                {
                    let key = if self.get_signal_value_by_name(&name).is_some() {
                        Some(name.to_string())
                    } else if self.get_signal_value_by_name(&pkey).is_some() {
                        Some(pkey.clone())
                    } else {
                        None
                    };
                    if let Some(key) = key {
                        if let Some(mut cur) = self.get_signal_value_by_name(&key) {
                            let piece = val.resize(elem_w);
                            let Some(lo) = self.packed_elem_lsb(&pkey, idx as i64, elem_w) else {
                                return false;
                            };
                            if lo + (elem_w as usize) <= cur.width as usize {
                                let prev = cur.clone();
                                for i in 0..(elem_w as usize) {
                                    cur.set_bit(lo + i, piece.get_bit(i));
                                }
                                let changed = cur != prev;
                                if changed {
                                    self.set_signal_value_by_name(&key, cur);
                                }
                                return changed;
                            }
                            return false;
                        }
                    }
                }
                // Fallback: for struct field write where the underlying
                // signal is the whole struct (not the field by name),
                // route via packed_struct_fields field offset.
                if let ExprKind::Ident(h) = &expr.kind {
                    if h.path.len() == 2 {
                        let struct_var = &h.path[0].name.name;
                        let field_name = &h.path[1].name.name;
                        if let Some(fields) = self
                            .module
                            .packed_struct_fields
                            .get(struct_var.as_str())
                            .cloned()
                        {
                            if let Some((_, field_off, _field_w)) =
                                fields.iter().find(|(m, _, _)| m == field_name).cloned()
                            {
                                if let Some(&sid) = self.signal_name_to_id.get(struct_var.as_str())
                                {
                                    let total_w = self.signal_widths[sid];
                                    let mut cur = self.signal_table[sid].clone();
                                    let piece = val.resize(elem_w);
                                    let lo = (field_off as usize) + idx * (elem_w as usize);
                                    if lo + (elem_w as usize) <= total_w as usize {
                                        let prev = cur.clone();
                                        for i in 0..(elem_w as usize) {
                                            cur.set_bit(lo + i, piece.get_bit(i));
                                        }
                                        let changed = cur != prev;
                                        if changed {
                                            self.signal_table[sid] = cur;
                                            self.sync_mirror(sid);
                                            self.table_modified = true;
                                            self.after_signal_write(sid);
                                            self.mark_dirty(struct_var);
                                        }
                                        return changed;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // Fall back to bit select assignment
            let idx = idx_val.to_u64().unwrap_or(0) as usize;
            // Bit-select write on a LOCAL packed vector (e.g.
            // `bit [7:0] seen; seen[i] = 1'b1;` inside an initial /
            // task). Locals live in local_stack, not signal_table —
            // mutate the Value in place.
            if let (false, Some(hier)) = (self.local_stack.is_empty(), hier_opt) {
                let last_idx = self.local_stack.len() - 1;
                if let Some(cur) = self.local_stack[last_idx].get(&hier.path[0].name.name) {
                    if idx < cur.width as usize {
                        let nb = val.bits_first();
                        let old = cur.get_bit(idx);
                        let c = old != nb;
                        if c {
                            let mut nv = cur.clone();
                            nv.set_bit(idx, nb);
                            self.local_stack[last_idx].insert(hier.path[0].name.name.clone(), nv);
                        }
                        return c;
                    }
                }
            }
            // Bit-select on a `this.<prop>` instance field.
            if let (Some(Some(handle)), Some(hier)) = (self.this_stack.last().copied(), hier_opt) {
                if hier.path.len() == 1 {
                    let pname = hier.path[0].name.name.clone();
                    if let Some(Some(inst)) = self.heap.get_mut(handle) {
                        if let Some(cur) = inst.properties.get(&pname).cloned() {
                            if idx < cur.width as usize {
                                let nb = val.bits_first();
                                let old = cur.get_bit(idx);
                                let c = old != nb;
                                if c {
                                    let mut nv = cur.clone();
                                    nv.set_bit(idx, nb);
                                    inst.properties.insert(pname, nv);
                                }
                                return c;
                            }
                        }
                    }
                }
            }
            // Bit select needs signal_table
            if let Some(&id) = self.signal_name_to_id.get(name.as_ref()) {
                if idx < self.signal_widths[id] as usize {
                    let nb = val.bits_first();
                    let old = self.signal_table[id].get_bit(idx);
                    let c = old != nb;
                    if c {
                        self.signal_table[id].set_bit(idx, nb);
                        self.sync_mirror(id);
                        self.table_modified = true;
                        self.after_signal_write(id);
                        self.mark_dirty(&name);
                    }
                    return c;
                }
            }
            // Slow-path bit-select on signals HashMap (block-local
            // packed vectors hoisted to module signals without a
            // signal_id, e.g. `bit [7:0] x;` declared inside an
            // initial begin/task). Without this branch the write
            // would silently no-op.
            if let Some(cur) = self.signals.get(&*name).cloned() {
                if idx < cur.width as usize {
                    let nb = val.bits_first();
                    let old = cur.get_bit(idx);
                    let c = old != nb;
                    if c {
                        let mut nv = cur;
                        nv.set_bit(idx, nb);
                        self.signals.insert(name.to_string(), nv);
                        self.mark_dirty(&name);
                    }
                    return c;
                }
            }
            // Bit-select write into a FRAME-owned (block-local) struct
            // /vector member leaf. A whole-member store (`s.m = v`) puts
            // the leaf in the current call/local frame under `s.m`, but the
            // indexed form (`s.m[i] = v`) only probed the signal maps above
            // and silently dropped the write — `uvm_reg_bus_op.byte_en[z] =
            // be[…]` in uvm_reg_map::do_bus_access left every byte_en 0, so
            // the reg bit-bash predictor saw a disabled byte and failed.
            if let Some(cur) = self
                .local_stack
                .last()
                .and_then(|f| f.get(name.as_ref()))
                .cloned()
            {
                if idx < cur.width as usize {
                    let nb = val.bits_first();
                    let old = cur.get_bit(idx);
                    let c = old != nb;
                    if c {
                        let mut nv = cur;
                        nv.set_bit(idx, nb);
                        if let Some(frame) = self.local_stack.last_mut() {
                            frame.insert(name.to_string(), nv);
                        }
                    }
                    return c;
                }
            }
        }
        false
    }

    /// `pkg::x`, `pkg::arr[i]`, `pkg::v[h:l]` as an assignment TARGET: package
    /// variables live in the table under their bare name (the read side
    /// strips the package prefix and re-enters, see `eval_expr_ctx`), but a
    /// write under a task/function frame went through the dotted-name path
    /// as `pkg.x` and vanished. Returns the target with the prefix stripped.
    pub(super) fn strip_package_lvalue(&self, lhs: &Expression) -> Option<Expression> {
        let is_pkg = |name: &str| {
            self.module.packages.contains(name)
                && !self.signal_name_to_id.contains_key(name)
                && !self.signals.contains_key(name)
                && !self.module.classes.contains_key(name)
        };
        match &lhs.kind {
            ExprKind::MemberAccess { expr, member } => {
                if let ExprKind::Ident(h) = &expr.kind {
                    if h.path.len() == 1
                        && h.path[0].selects.is_empty()
                        && is_pkg(&h.path[0].name.name)
                    {
                        let mut ident = h.clone();
                        ident.path[0].name = member.clone();
                        // §26.3: a contested package variable has its own
                        // per-package storage under `<pkg>::<name>`.
                        if let Some(q) = self.pkg_var_storage(&h.path[0].name.name, &member.name) {
                            ident.path[0].name.name = q;
                        }
                        // The stripped name names a NEW signal and must resolve
                        // under its own column, not the field the package
                        // qualifier's node carried. `h` here is the
                        // `Ident(<pkg>)` reference, which — once the package
                        // name has itself been resolved/memoized (a
                        // `(pkg).static` element read hashes the package) —
                        // holds `cached_resolved_name == <pkg>`. Cloning it
                        // preserves that cache, so the stripped `<member>`
                        // wrongly resolves back to the PACKAGE name, and every
                        // element read returns all-x. Reset the cache so the
                        // new name resolves from scratch.
                        ident.cached_resolved_name = std::cell::OnceCell::new();
                        return Some(Expression::new(ExprKind::Ident(ident), lhs.span));
                    }
                }
                None
            }
            ExprKind::Ident(h)
                if h.path.len() >= 2
                    && h.path[0].selects.is_empty()
                    && is_pkg(&h.path[0].name.name) =>
            {
                let mut ident = h.clone();
                ident.path.remove(0);
                // §26.3: a contested package variable's own storage.
                if let Some(q) = self.pkg_var_storage(&h.path[0].name.name, &h.path[1].name.name) {
                    ident.path[0].name.name = q;
                }
                // See the MemberAccess arm above: the cloned node inherits
                // `cached_resolved_name` from the `<pkg>::<member>` reference;
                // reset it so the bare `<member>` resolves fresh.
                ident.cached_resolved_name = std::cell::OnceCell::new();
                Some(Expression::new(ExprKind::Ident(ident), lhs.span))
            }
            ExprKind::Index { expr, index } => {
                let base = self.strip_package_lvalue(expr)?;
                Some(Expression::new(
                    ExprKind::Index {
                        expr: Box::new(base),
                        index: index.clone(),
                    },
                    lhs.span,
                ))
            }
            ExprKind::RangeSelect {
                expr,
                kind,
                left,
                right,
            } => {
                let base = self.strip_package_lvalue(expr)?;
                Some(Expression::new(
                    ExprKind::RangeSelect {
                        expr: Box::new(base),
                        kind: *kind,
                        left: left.clone(),
                        right: right.clone(),
                    },
                    lhs.span,
                ))
            }
            _ => None,
        }
    }

    /// §23.6: an absolute path written inside a subroutine body (`tb.u.sig`,
    /// `tb.u.sig[3:0]`, `tb.u.mem[i]`) arrives as a MemberAccess chain, and
    /// the lvalue arms looked it up in a signal table that keys the top
    /// module's contents without its name — the write was silently dropped.
    /// Rebuild it as the flat Ident elaboration gives module-scope code,
    /// keeping the select wrappers, so it resolves the same way.
    pub(super) fn top_rooted_member_lvalue(&self, e: &Expression) -> Option<Expression> {
        match &e.kind {
            ExprKind::Index { expr, index } => {
                let inner = self.top_rooted_member_lvalue(expr)?;
                Some(Expression::new(
                    ExprKind::Index {
                        expr: Box::new(inner),
                        index: index.clone(),
                    },
                    e.span,
                ))
            }
            ExprKind::RangeSelect {
                expr,
                kind,
                left,
                right,
            } => {
                let inner = self.top_rooted_member_lvalue(expr)?;
                Some(Expression::new(
                    ExprKind::RangeSelect {
                        expr: Box::new(inner),
                        kind: kind.clone(),
                        left: left.clone(),
                        right: right.clone(),
                    },
                    e.span,
                ))
            }
            ExprKind::MemberAccess { .. } => {
                let mut root = e;
                while let ExprKind::MemberAccess { expr, .. } = &root.kind {
                    root = expr;
                }
                let ExprKind::Ident(h) = &root.kind else {
                    return None;
                };
                let top = self.module.name.as_str();
                if h.root.is_some()
                    || h.path.len() != 1
                    || !h.path[0].selects.is_empty()
                    || h.path[0].name.name != top
                    || self.local_stack.last().is_some_and(|l| l.contains_key(top))
                    || self.signal_name_to_id.contains_key(top)
                    || self.signals.contains_key(top)
                {
                    return None;
                }
                let path = Self::flatten_member_path(e)?
                    .into_iter()
                    .map(|name| crate::ast::expr::HierPathSegment {
                        name: crate::ast::Identifier { name, span: e.span },
                        selects: Vec::new(),
                    })
                    .collect();
                Some(Expression::new(
                    ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                        root: None,
                        path,
                        span: e.span,
                        cached_signal_id: std::cell::Cell::new(None),
                        cached_resolved_name: std::cell::OnceCell::new(),
                    }),
                    e.span,
                ))
            }
            _ => None,
        }
    }

    pub(super) fn assign_value_inner(&mut self, lhs: &Expression, val: &Value) -> bool {
        // §19.7 `cg.option.<name> = v` on a covergroup instance.
        if let ExprKind::Ident(h) = &lhs.kind {
            if h.path.len() == 3 && h.path[1].name.name == "option" {
                if let Some(idx) = self.cg_option_target(h) {
                    if let Some(Some(inst)) = self.cg_heap.get_mut(idx) {
                        inst.options
                            .insert(h.path[2].name.name.clone(), val.clone());
                    }
                    return true;
                }
            }
        }
        if !self.module.packages.is_empty() {
            if let Some(stripped) = self.strip_package_lvalue(lhs) {
                return self.assign_value_inner(&stripped, val);
            }
        }
        if let Some(flat) = self.top_rooted_member_lvalue(lhs) {
            return self.assign_value_inner(&flat, val);
        }
        // §25.9: a just-returned vif (recorded by the Return arm) binds to
        // the FIRST assignment target after the call — `value = r.read(c)`
        // in uvm_config_db::get (issue #113). One-shot; also cleared at the
        // next function-call entry so it cannot leak further than that.
        // The marker is probed only when a `return <vif>` left one: the
        // signal-map lookup was a string hash on EVERY assignment.
        if self.vif_return_pending
            && let Some(nm) = self
                .signals
                .remove("__vif_return__")
                .map(|x| x.to_sv_string())
                .filter(|s| !s.is_empty())
        {
            self.vif_return_pending = false;
            let synth = Expression::new(
                ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                    root: None,
                    path: vec![crate::ast::expr::HierPathSegment {
                        name: crate::ast::Identifier {
                            name: nm,
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
            // §25.9/§8.9: a static virtual-interface property binds too.
            self.bind_static_vif(lhs, &synth);
            let _ = self.try_bind_virtual_iface(lhs, &synth);
        }
        // §13.5.2: a selected write through a `ref` formal lands on the
        // caller's actual; an aliased formal's local copy is refreshed so
        // any unredirected read path stays coherent.
        if let Some(rewritten) = self.ref_formal_redirect_expr(lhs) {
            let changed = self.assign_value(&rewritten, val);
            let mut root = lhs;
            loop {
                match &root.kind {
                    ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => {
                        root = expr
                    }
                    _ => break,
                }
            }
            if let ExprKind::Ident(h) = &root.kind {
                let fname = h.path[0].name.name.clone();
                if let Some(storage) = self
                    .ref_alias_stack
                    .last()
                    .and_then(|m| m.get(&fname))
                    .cloned()
                {
                    if let Some(full) = self.get_signal_value_by_name(&storage) {
                        if let Some(frame) = self.local_stack.last_mut() {
                            if let Some(slot) = frame.get_mut(&fname) {
                                *slot = full;
                            }
                        }
                    }
                }
            }
            return changed;
        }
        if let ExprKind::Ident(h) = &lhs.kind {
            if h.path.len() == 3 && h.path[1].name.name == "type_option" {
                if let Some((cg, field)) = self.cg_type_option_path(h) {
                    // Seed the table (defaults + body items) before the write.
                    let _ = self.cg_type_option_get(&cg, &field);
                    if let Some(t) = self.cg_type_options.get_mut(&cg) {
                        t.insert(field, val.clone());
                    }
                    return true;
                }
            }
        }
        if let Some(rewritten) = self.super_rewrite(lhs) {
            return self.assign_value(&rewritten, val);
        }
        // §25.8/§25.9: a write through a bound virtual interface lands on the
        // bound instance, whatever the chain shape.
        if let Some(rewritten) = self.vif_rebase_expr(lhs) {
            return self.assign_value(&rewritten, val);
        }
        // §11.4.12: `$` in an index is the last valid index of the collection
        // being indexed. The READ paths install `dollar_bound` before
        // evaluating; no lvalue path did, so `$` fell through to its
        // `u64::MAX` default and the write went to a nonexistent element.
        // Normalising the whole lvalue ONCE here covers every arm below —
        // plain `q[$]`, a part-select of it (`q[$][3:0]`), and nested forms —
        // rather than each arm having to remember to push the bound. Gated on
        // a cheap scan, so an lvalue without `$` is untouched.
        let dollar_free;
        let lhs: &Expression = if Self::lvalue_mentions_dollar(lhs) {
            dollar_free = self.resolve_dollar_in_lvalue(lhs);
            &dollar_free
        } else {
            lhs
        };

        // Packed geometry applies after reference, interface and $ rewrites.
        if (self.needs_declared_packed_access(lhs) || self.needs_static_packed_access(lhs))
            && let Some(changed) = self.write_declared_packed_access(lhs, val)
        {
            return changed;
        }

        // §7.4.6 / §11.5.1: an ELEMENT index containing x or z selects nothing,
        // so the write is discarded — `a['hx] = v` must leave every element
        // alone. The index folded to 0 through `to_u64`, so it silently
        // clobbered a[0]. The right-hand side is still evaluated by the caller
        // (its side effects stand); only the store is dropped.
        if let ExprKind::Index { expr: base, index } = &lhs.kind {
            if !Self::expr_is_stream(base) {
                let iv = self.eval_expr(index);
                if iv.has_xz() && !iv.is_real {
                    return false;
                }
                // §7.10.1: a QUEUE write at index == size APPENDS (`q[0] = v`
                // on an empty queue makes it '{v}). Bump the size shadow and
                // let the normal element-write arms store the value. Gated on
                // declared-queue vars so a dynamic array keeps its
                // discard-out-of-range semantics.
                if let ExprKind::Ident(h) = &base.kind {
                    let qn = self.resolve_hier_name(h);
                    if self.module.queue_vars.contains(&*qn) {
                        let sz = self.get_queue_size(&qn);
                        let within_max = self
                            .module
                            .queue_max_sizes
                            .get(&*qn)
                            .is_none_or(|&m| sz < m as u64);
                        if within_max && iv.to_u64() == Some(sz) {
                            self.set_queue_size(&qn, sz + 1);
                        }
                    }
                }
            }
        }

        // §18.4: writing a member of a struct/union CLASS property goes to the
        // aggregate's own storage — for a packed struct/union that means
        // splicing the member's bits into the property's raw integral value, so
        // the whole/field views (and a union's overlaid members) stay coherent.
        // Both parse shapes (`MemberAccess` and a flattened `Ident([a, b])`)
        // route here, so this sits ahead of the per-kind arms. A member with
        // an index or part-select below it (`obj.pkt.mem[i] = v`,
        // `obj.pkt.mem[1][255:248] = v`) resolves through the same call — the
        // select-aware walk in `class_agg_member` splices the element's bits.
        if matches!(
            lhs.kind,
            ExprKind::MemberAccess { .. }
                | ExprKind::Ident(_)
                | ExprKind::Index { .. }
                | ExprKind::RangeSelect { .. }
        ) {
            if let Some(r) = self.class_agg_member(lhs) {
                return self.write_class_agg(&r, val);
            }
        }
        // §8.9: assignment to a STATIC class property (`obj.static_prop = ...`,
        // `this.static_prop = ...`) writes the single shared cell, not per-
        // instance storage. A named reference resolves by its declared type
        // and specialization (§8.25), so a null handle works and a derived
        // object's same-named instance property is not mistaken for it; this
        // arm runs before the instance-property arms below. A non-static
        // member falls through to them.
        {
            let obj_prop: Option<(String, String)> = match &lhs.kind {
                ExprKind::Ident(h)
                    if h.path.len() == 2 && h.path.iter().all(|s| s.selects.is_empty()) =>
                {
                    Some((h.path[0].name.name.clone(), h.path[1].name.name.clone()))
                }
                ExprKind::MemberAccess { expr, member } => match &expr.kind {
                    ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                        Some((h.path[0].name.name.clone(), member.name.clone()))
                    }
                    _ => None,
                },
                _ => None,
            };
            if let Some((obj, prop)) = obj_prop {
                if obj != "this" && obj != "super" {
                    let handle = self.eval_ident_handle(&obj).unwrap_or(0);
                    let resolved = match &lhs.kind {
                        ExprKind::Ident(h) => {
                            let head_id = HierarchicalIdentifier {
                                root: h.root.clone(),
                                path: vec![h.path[0].clone()],
                                span: h.span,
                                cached_signal_id: std::cell::Cell::new(None),
                                cached_resolved_name: std::cell::OnceCell::new(),
                            };
                            Some(self.resolve_hier_name(&head_id).into_owned())
                        }
                        ExprKind::MemberAccess { expr, .. } => match &expr.kind {
                            ExprKind::Ident(h) => Some(self.resolve_hier_name(h).into_owned()),
                            _ => None,
                        },
                        _ => None,
                    };
                    if let Some(ok) = self.static_write_via_handle(
                        handle,
                        Some(&obj),
                        resolved.as_deref(),
                        &prop,
                        val.clone(),
                    ) {
                        return ok;
                    }
                }
                let cn = if obj == "this" {
                    self.this_stack
                        .last()
                        .copied()
                        .flatten()
                        .and_then(|h| self.heap.get(h))
                        .and_then(|o| o.as_ref())
                        .map(|i| i.class_name.clone())
                } else {
                    // A named reference was resolved above, by its
                    // declared type.
                    None
                };
                if let Some(cn) = cn {
                    if self.static_prop_key(&cn, &prop).is_some() {
                        let mut v = val.clone();
                        if let Some(w) = self.class_prop_width(&cn, &prop) {
                            if w != v.width && !v.is_real {
                                v = v.resize_for_assign(w);
                            }
                        }
                        return self.class_static_set(&cn, &prop, v);
                    }
                }
            }
        }
        // §7.8.6/§8.4: `coll[key].prop = v` where the element is a CLASS
        // HANDLE writes the heap object the handle points at — not a
        // composed `coll[key].prop` signal (which silently forks the
        // element's identity from the stored handle: writes through the
        // element stopped being visible through other handles to the same
        // object). Gated on the receiver's value resolving to a live heap
        // instance that declares the property, so struct-element member
        // writes (composed-name storage) are untouched.
        // §4c: `h.prop = v` where `h` is a MODULE-SCOPE class-handle variable
        // (either parse shape). A REAL hierarchical signal `h.prop` must win —
        // only route to the heap when no such signal exists and the head's
        // value is a live object declaring the property. Previously the write
        // landed on a phantom flattened name and silently vanished.
        {
            let head_field: Option<(String, String)> = match &lhs.kind {
                ExprKind::Ident(h2)
                    if h2.path.len() == 2 && h2.path.iter().all(|s| s.selects.is_empty()) =>
                {
                    Some((h2.path[0].name.name.clone(), h2.path[1].name.name.clone()))
                }
                ExprKind::MemberAccess { expr: recv, member } => match &recv.kind {
                    ExprKind::Ident(h2) if h2.path.len() == 1 && h2.path[0].selects.is_empty() => {
                        Some((h2.path[0].name.name.clone(), member.name.clone()))
                    }
                    _ => None,
                },
                _ => None,
            };
            if let Some((head, field)) = head_field {
                let is_local = self
                    .local_stack
                    .last()
                    .is_some_and(|l| l.contains_key(&head));
                let dotted = format!("{}.{}", head, field);
                let dotted_signal = self.signal_name_to_id.contains_key(dotted.as_str())
                    || self.signals.contains_key(&dotted);
                if !is_local && !dotted_signal {
                    // §23.7: `<top>.<sig>` from a class method writes the top
                    // module's own variable (silently dropped before).
                    if head == self.module.name
                        && (self.signal_name_to_id.contains_key(field.as_str())
                            || self.signals.contains_key(&field))
                    {
                        self.set_signal_value_by_name(&dotted, val.clone());
                        return true;
                    }
                    // id-collision-in-heap: only a head that STATICALLY denotes
                    // a class-object lvalue may reinterpret its stored value as
                    // a heap handle here. A packed-struct variable splices its
                    // members into one integral value (§7.8), so
                    // `sv.delay_counter` with `sv == 32'd1` diverted the field
                    // write into `heap[1].delay_counter` — the write vanished
                    // from the struct and clobbered an unrelated object. The
                    // gate mirrors `bare_receiver_is_class_handle`'s static
                    // conjunct (enum locals shadow the flat class map) without
                    // its runtime-liveness conjunct: the colliding value is
                    // live by construction, so only the declared shape of the
                    // name discriminates.
                    if self.bare_name_is_class_channel(&head) {
                        if let Some(h) = self.eval_ident_handle(&head) {
                            if h != 0 {
                                let holds = self
                                    .heap
                                    .get(h)
                                    .and_then(|o| o.as_ref())
                                    .is_some_and(|inst| inst.properties.contains_key(&field));
                                if holds {
                                    let fitted = self.fit_class_prop(h, &field, val);
                                    if let Some(Some(inst)) = self.heap.get_mut(h) {
                                        inst.properties.insert(field, fitted);
                                        return true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if let ExprKind::MemberAccess { expr: recv, member } = &lhs.kind {
            if let ExprKind::Index { expr: base, .. } = &recv.kind {
                // id-collision-in-heap: an indexed receiver's VALUE is a heap
                // handle only when the collection's ELEMENT type is a class
                // (`ch[i].prop = v`). A struct element's packed payload is
                // data, not a handle — rerouting it into the heap stole the
                // write from the element. Non-ident bases (method results,
                // nested selects) keep the legacy value-based check.
                let base_is_handle_container = match &base.kind {
                    ExprKind::Ident(bh) if bh.path.len() == 1 && bh.path[0].selects.is_empty() => {
                        // The element-class map also records STRUCT typedef
                        // names (elaboration keys every collection), so verify
                        // the resolved element type really is a class before
                        // treating the element's value as a heap handle.
                        self.elem_class_of_arr(&bh.path[0].name.name)
                            .is_some_and(|c| self.module.classes.contains_key(&c))
                    }
                    _ => true,
                };
                if base_is_handle_container {
                    let h = self.eval_expr(recv).to_u64().unwrap_or(0) as usize;
                    if h != 0 && h < self.heap.len() {
                        let holds = self
                            .heap
                            .get(h)
                            .and_then(|o| o.as_ref())
                            .is_some_and(|inst| inst.properties.contains_key(&member.name));
                        if holds {
                            let fitted = self.fit_class_prop(h, &member.name, val);
                            if let Some(Some(inst)) = self.heap.get_mut(h) {
                                inst.properties.insert(member.name.clone(), fitted);
                                return true;
                            }
                        }
                    }
                }
            }
        }
        // §7.4.1: `<obj>.<prop>[i] = ...` on a multi-dimensional PACKED array
        // property splices the element's bits. Without this the write fell
        // through to a bit-select path that dropped it entirely.
        if let ExprKind::Index { expr, index } = &lhs.kind {
            // `class_packed_elem_ref` takes references and bails immediately on
            // an empty heap, so the old `expr.clone()` / `index.clone()` deep-
            // copied two expression trees on EVERY indexed write — including
            // designs with no classes at all, where the callee then returned
            // None. That was pure allocator traffic on the hot array-store path
            // (a 256K-element memory fill pays it 256K times). Pass the borrows
            // and skip the call entirely when no objects exist.
            if !self.no_class_objects()
                && matches!(
                    &expr.kind,
                    ExprKind::MemberAccess { .. } | ExprKind::Ident(_) | ExprKind::This
                )
            {
                if let Some(r) = self.class_packed_elem_ref(expr, index) {
                    return self.write_class_agg(&r, val);
                }
            }
        }
        // §7.4.2/§7.4.1: an indexed/part-select write INTO an element of an
        // unpacked-array class property whose element type is a
        // multi-dimensional packed array (`d.arr[1][2] = v`,
        // `d.arr[1][8:0] = v` on `bit [8:0][7:0] arr[3]`; bare in a method
        // too). The element lives whole under the per-instance key
        // `<h>#arr[1]`, and the trailing select names a packed ELEMENT (an
        // ew-bit slice) or an element RANGE — but the generic arms below
        // treat it as a bit-select/bit-range of a phantom scalar, so the
        // write was silently dropped (bit form) or spliced at raw bit
        // offsets (range form). The read side (`class_packed_operand`)
        // already does this walk; mirror it for the store.
        if !self.no_class_objects()
            && match &lhs.kind {
                ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => matches!(
                    &expr.kind,
                    ExprKind::Index { .. } | ExprKind::MemberAccess { .. } | ExprKind::Ident(_)
                ),
                _ => false,
            }
            && let Some(r) = self.class_packed_elem_sel_write(lhs, val)
        {
            return r;
        }
        // §13.5.2: a write to an ALIASED `ref` formal writes THROUGH to the
        // caller's storage immediately, so concurrent observers of the actual
        // see it mid-call (see `ref_alias_stack`). The storage is written BY
        // NAME — evaluating the bound ident would hit the callee frame's own
        // copy when the formal is named like its actual. The local copy is
        // refreshed too, keeping any unredirected read path coherent.
        if !self.ref_alias_stack.is_empty() {
            if let ExprKind::Ident(h) = &lhs.kind {
                if h.path.len() == 1 && h.path[0].selects.is_empty() {
                    let fname = h.path[0].name.name.as_str();
                    if let Some(storage) = self
                        .ref_alias_stack
                        .last()
                        .and_then(|m| m.get(fname))
                        .cloned()
                    {
                        let fname = fname.to_string();
                        if let Some(frame) = self.local_stack.last_mut() {
                            if let Some(slot) = frame.get_mut(&fname) {
                                let fitted = if !slot.is_real && !val.is_real && slot.width > 0 {
                                    val.resize(slot.width)
                                } else {
                                    val.clone()
                                };
                                *slot = fitted;
                            }
                        }
                        if self.vif_frame_alias(&fname).is_some() {
                            // §13.5.2: ref rebinding is visible immediately.
                            if let Some(bound) = val
                                .to_u64()
                                .and_then(|t| self.iface_by_token(t))
                                .map(str::to_string)
                            {
                                self.viface_var_aliases.insert(storage.clone(), bound);
                            } else {
                                self.viface_var_aliases.remove(&storage);
                            }
                        }
                        let prev = self.get_signal_value_by_name(&storage);
                        self.set_signal_value_by_name(&storage, val.clone());
                        return prev.as_ref() != self.get_signal_value_by_name(&storage).as_ref();
                    }
                }
            }
        }
        // §7.2/§13.4.2: a leaf of a frame-owned unpacked struct — at any depth,
        // and through an index. The arms below all resolve against the module
        // signal table, so without this the write left the frame's copy alone
        // and the matching read returned an outer scope's value.
        if !self.local_stack.is_empty() {
            if let Some(key) = self.frame_leaf_key_of(lhs) {
                let prev = self.frame_get(&key).cloned();
                let fitted = match prev.as_ref() {
                    Some(p) if p.is_real && !val.is_real => Value::from_f64(val.to_f64()),
                    Some(p) if !p.is_real && !val.is_real && p.width > 0 => val.resize(p.width),
                    Some(p) if !p.is_real && val.is_real => {
                        let mut f = Self::real_to_int(val.to_f64(), p.width.max(1));
                        f.is_signed = p.is_signed;
                        f
                    }
                    _ => val.clone(),
                };
                let changed = prev.as_ref() != Some(&fitted);
                if let Some(frame) = self.local_stack.last_mut() {
                    frame.insert(key, fitted);
                }
                return changed;
            }
            // §7.4.1/§11.5.1: a nested select of a packed variable the frame
            // owns (`p[1][3:0] = v`, `a[i].f[3] = b`).
            if Self::select_of_select(lhs)
                && let Some(r) = self.frame_packed_select_write(lhs, val)
            {
                return r;
            }
        }
        match &lhs.kind {
            ExprKind::Ident(hier) => {
                // §7.2.1/§23.7: preserve the full registered struct path
                // before leaf-name fallback can select an unrelated observer.
                if !self.module.classes.is_empty()
                    && !self.module.packed_struct_fields.is_empty()
                    && hier.path.len() > 2
                    && hier.path.iter().all(|s| s.selects.is_empty())
                {
                    let dotted = hier
                        .path
                        .iter()
                        .map(|s| s.name.name.as_str())
                        .collect::<Vec<_>>()
                        .join(".");
                    let name = dotted
                        .strip_prefix(&format!("{}.", self.module.name))
                        .unwrap_or(&dotted);
                    if let Some((base, off, width)) = self.packed_leaf_of_hier(name)
                        && let Some(cur) = self.get_signal_value_by_name(&base)
                    {
                        let mut next = cur.clone();
                        let rhs =
                            self.packed_member_piece(&base, name, val.resize_for_assign(width));
                        for bit in 0..width as usize {
                            next.set_bit(off as usize + bit, rhs.get_bit(bit));
                        }
                        if next == cur {
                            return false;
                        }
                        self.set_signal_value_by_name(&base, next);
                        return true;
                    }
                }
                // LRM §25.9: virtual-interface alias (task/function
                // formal frame, or a plain `virtual <iface>` variable) —
                // rewrite `vif.member` to `<bound>.member`. Must fire
                // before the local-stack/class-prop checks so `vif`
                // doesn't shadow a same-named local of unrelated type.
                if hier.path.len() >= 2 {
                    {
                        let first = hier.path[0].name.name.as_str();
                        if let Some(bound) = self.iface_alias_for(first) {
                            let rest = hier
                                .path
                                .iter()
                                .skip(1)
                                .map(|s| s.name.name.as_str())
                                .collect::<Vec<_>>()
                                .join(".");
                            let target = format!("{}.{}", bound, rest);
                            let prev = self.get_signal_value_by_name(&target);
                            let w = prev.as_ref().map(|v| v.width).unwrap_or(val.width);
                            let resized = val.resize(w);
                            let changed = prev.as_ref() != Some(&resized);
                            self.set_signal_value_by_name(&target, resized);
                            return changed;
                        }
                    }
                }
                // A DOTTED local: an inlined-instance function's return
                // variable lives in the frame under its registered name
                // ("dut.z"), and the rewritten body assigns exactly that
                // dotted lvalue — which skipped the single-segment local
                // check below and leaked to the global map, so every
                // hier-named function returned its never-written init
                // (br_gh277b: x, formerly a masking 0).
                if hier.path.len() > 1 && hier.path.iter().all(|s| s.selects.is_empty()) {
                    if !self.local_stack.is_empty() {
                        let joined = hier
                            .path
                            .iter()
                            .map(|s| s.name.name.as_str())
                            .collect::<Vec<_>>()
                            .join(".");
                        let last_idx = self.local_stack.len() - 1;
                        if self.local_stack[last_idx].contains_key(joined.as_str()) {
                            // §6.21/§10.7: a frame-local integral value keeps its
                            // declared width and signing on every store. Bare-name
                            // metadata may currently describe a different process's
                            // same-named declaration, so it cannot type this store.
                            let is_str = hier.path.last().is_some_and(|s| {
                                self.string_signals.contains(&s.name.name)
                                    || self.p_local_is_string(&s.name.name)
                            });
                            let fitted = if !val.is_real && !is_str {
                                if let Some(&target_w) = self.widths.get(&joined) {
                                    if val.width != target_w {
                                        val.resize_for_assign(target_w)
                                    } else {
                                        val.clone()
                                    }
                                } else {
                                    val.clone()
                                }
                            } else {
                                val.clone()
                            };
                            self.local_stack[last_idx].insert(joined, fitted);
                            return true;
                        }
                    }
                }
                // §8.4: `tb.u_leaf.obj.field = v` writes the OBJECT.  The dotted
                // name used to fall through to the signal table, which resolves
                // the leaf flat and landed on tb's own `field` — the object kept
                // its old value while a read through a handle showed the new one.
                // Only taken when the parent really is a live object that declares
                // the member, so struct/net lvalues are untouched.
                if hier.path.len() >= 2 && hier.path.iter().all(|s| s.selects.is_empty()) {
                    if let Some(h) = self.hier_parent_handle(hier) {
                        let member_name = hier.path[hier.path.len() - 1].name.name.as_str();
                        if self.read_member_value(h, member_name).is_some()
                            || self
                                .static_member_target(h, None, None, member_name)
                                .is_some()
                        {
                            if let Some(ok) = self.static_write_via_handle(
                                h,
                                None,
                                None,
                                member_name,
                                val.clone(),
                            ) {
                                return ok;
                            }
                            let fitted = self.fit_class_prop(h, member_name, &val);
                            return self.set_prop_if_changed(h, member_name, fitted);
                        }
                    }
                }
                if hier.path.len() == 1 && hier.path[0].selects.is_empty() {
                    let name = &hier.path[0].name.name;
                    // Check local stack
                    if !self.local_stack.is_empty() {
                        let last_idx = self.local_stack.len() - 1;
                        if let Some((prev_is_real, local_width, local_signed)) = self.local_stack
                            [last_idx]
                            .get(name)
                            .map(|p| (p.is_real, p.width, p.is_signed))
                        {
                            // §10.7: on assignment to a sized local, resize
                            // the RHS to the local's DECLARED width so a
                            // narrow signed literal sign-extends (e.g.
                            // `logic [63:0] mask = -1` must yield all-ones).
                            // The declared width is read from the `widths`
                            // table (populated at VarDecl time), NOT from the
                            // currently-stored value — the stored width drifts
                            // as differently-sized RHS values flow through and
                            // would corrupt string/handle locals. If no
                            // declared width is recorded (method formals,
                            // pattern bindings), leave the value untouched.
                            //
                            // WIDTH truncation: a value WIDER than the
                            // declared width is TRUNCATED (e.g. `byte b; b =
                            // $urandom` keeps only the low 8 bits). STRING
                            // locals are exempt (1024-bit internal packed
                            // text).
                            let is_str = self.p_local_is_string(name);
                            let fitted = if prev_is_real && !val.is_real && !is_str {
                                // §6.12.2: an integral value assigned to a REAL
                                // local converts; stored as integral bits, a
                                // `real t; t = k; h = t / 2;` divided as integers.
                                Value::from_f64(val.to_f64())
                            } else if !val.is_real && !is_str {
                                if local_width > 0 {
                                    let target_w = local_width;
                                    let mut f = if val.width != target_w {
                                        val.resize_for_assign(target_w)
                                    } else {
                                        val.clone()
                                    };
                                    // §6.11.1: storage keeps the LOCAL'S
                                    // declared signedness, not the rvalue's —
                                    // same rule the id-signal path applies.
                                    // `for (i = 0; ...)` on a `bit [3:0] i`
                                    // otherwise stored the unsized literal's
                                    // SIGNED stamp, and the counter wrapped
                                    // negative at 8 (ivtest sv_foreach3 hang).
                                    f.is_signed = local_signed;
                                    f
                                } else {
                                    val.clone()
                                }
                            } else if val.is_real && !is_str && !prev_is_real {
                                // §6.12.2: a real assigned to an INTEGRAL local
                                // rounds to the local's type — the module-variable
                                // path did, the frame local kept the real, so an
                                // `int baudDivisor = freq / rate;` compared as
                                // 32.55 forever (uart AVIP baud clock never toggled).
                                let (w, signed) = self.local_stack[last_idx]
                                    .get(name)
                                    .map(|p| (p.width.max(1), p.is_signed))
                                    .unwrap_or((32, true));
                                let mut f = Self::real_to_int(val.to_f64(), w);
                                f.is_signed = signed;
                                f
                            } else {
                                val.clone()
                            };
                            self.local_stack[last_idx].insert(name.clone(), fitted.clone());
                            // §6.21: write a `static` subroutine local through
                            // to its live shared cell so other simultaneous
                            // activations (recursive phase re-entry / method)
                            // observe the mutation immediately. Skipped during
                            // elaboration-time constant-function evaluation,
                            // where each call is independent.
                            if !self.in_const_param_eval
                                && self.static_local_names.contains(name.as_str())
                                && let Some(k) = self.static_local_key_for(name)
                                && self.static_local_keys.contains(&k)
                            {
                                self.static_local_vars.insert(k, fitted);
                            }
                            return true;
                        }
                    }
                    // Check 'this' properties
                    if let Some(Some(handle)) = self.this_stack.last() {
                        let handle = *handle;
                        // §8.10: a base method's write to a shadowed name must
                        // land in the base's own copy, or it silently leaks
                        // into the derived's.
                        let key = self
                            .shadowed_prop_key(name, false)
                            .unwrap_or_else(|| name.clone());
                        let holds = self
                            .heap
                            .get(handle)
                            .and_then(|o| o.as_ref())
                            .is_some_and(|i| i.properties.contains_key(&key));
                        if holds {
                            let fitted = self.fit_class_prop(handle, &key, val);
                            if let Some(Some(instance)) = self.heap.get_mut(handle) {
                                instance.properties.insert(key, fitted);
                                return true;
                            }
                        }
                    }
                    // Static class property assigned bare inside a method.
                    if let Some(Some(ctx)) = self.class_context_stack.last().cloned() {
                        let nm = name.clone();
                        if self.class_static_set(&ctx, &nm, val.clone()) {
                            return true;
                        }
                    }
                } else if hier.path.len() > 1 {
                    // Property write through a class handle evaluated from the prefix
                    // (e.g. `stor.handle.val = 777`). Not for a hierarchical
                    // signal (`u_sub.sig`): its prefix is an instance, whose
                    // value is no handle.
                    if hier.path.len() >= 2
                        && !self
                            .signal_name_to_id
                            .contains_key(self.resolve_hier_name(hier).as_ref())
                    {
                        let prop_name = &hier.path.last().unwrap().name.name;
                        // id-collision-in-heap: the head's VALUE is a heap
                        // handle only when the head statically names one — a
                        // class-typed root (`h.direct`), a frame-dotted local
                        // member holding a handle (`stor.handle`), a
                        // class-typed composed signal (`mod.h`), or a
                        // handle-collection element (`ch[0]`). Struct or
                        // hierarchical heads (`ov.inner`, `u.sv`, `u_if.iv`,
                        // `q[0]` of struct elements) splice their members into
                        // integral values that must never be read back as
                        // handles.
                        let head_hier = hier_prefix(hier);
                        if self.head_denotes_class_object(&head_hier) {
                            let head_expr = Expression::new(ExprKind::Ident(head_hier), lhs.span);
                            let base_val = self.eval_expr(&head_expr);
                            let handle = base_val.to_u64().unwrap_or(0) as usize;
                            if handle != 0
                                && handle < self.heap.len()
                                && self.heap[handle].is_some()
                            {
                                let fitted = self.fit_class_prop(handle, prop_name, val);
                                return self.set_prop_if_changed(handle, prop_name, fitted);
                            }
                        }
                    }
                    // `ClassName::static_prop` — explicit static property write.
                    if hier.path.len() == 2 {
                        let cls = hier.path[0].name.name.clone();
                        let prop = hier.path[1].name.name.clone();
                        if self.module.classes.contains_key(&cls)
                            && !self
                                .signal_name_to_id
                                .contains_key(self.resolve_hier_name(hier).as_ref())
                            && self.class_static_set(&cls, &prop, val.clone())
                        {
                            return true;
                        }
                        // §8.25/§6.18: `alias_t::prop` through a typedef of a
                        // specialization writes THAT specialization's cell,
                        // the one its reads (and `C#(args)::prop`) name.
                        if !self.module.classes.contains_key(&cls)
                            && !self.signal_name_to_id.contains_key(cls.as_str())
                            && let Some((base, sig)) = self.scope_prefix_spec(&cls)
                        {
                            let saved = self.current_spec.replace((base.clone(), sig));
                            let ok = self.class_static_set(&base, &prop, val.clone());
                            self.current_spec = saved;
                            if ok {
                                return true;
                            }
                        }
                    }
                    // `ClassName::static_obj_handle.member...` (3+ segments):
                    // the head names a class whose STATIC property (segment 1)
                    // holds an object handle; the remaining segments are member
                    // writes on that object. Needed for
                    // `ClassName::obj.member = new(...)` / `= handle` so the
                    // write reaches the real heap object reached through the
                    // static-property chain.
                    if hier.path.len() >= 3 && hier.path.iter().all(|s| s.selects.is_empty()) {
                        let cls = &hier.path[0].name.name;
                        let sprop = &hier.path[1].name.name;
                        if self.module.classes.contains_key(cls)
                            && self.static_prop_key(cls, sprop).is_some()
                        {
                            let cur0 = self
                                .static_prop_key(cls, sprop)
                                .and_then(|k| self.class_statics.get(&k))
                                .and_then(|v| v.to_u64())
                                .map(|h| h as usize)
                                .unwrap_or(0);
                            if cur0 != 0 && cur0 < self.heap.len() {
                                let mut cur = cur0;
                                let mut done = false;
                                for i in 2..hier.path.len() {
                                    if cur == 0 || cur >= self.heap.len() {
                                        break;
                                    }
                                    let mname = hier.path[i].name.name.clone();
                                    if i == hier.path.len() - 1 {
                                        // If the final member is a STATIC
                                        // property of the reached object's
                                        // class, write the shared static cell
                                        // (the read path resolves it there).
                                        // Otherwise write the instance property.
                                        let cn = self
                                            .heap
                                            .get(cur)
                                            .and_then(|o| o.as_ref())
                                            .map(|inst| inst.class_name.clone());
                                        if let Some(cn) = cn {
                                            if self.static_prop_key(&cn, &mname).is_some()
                                                && self.class_static_set(&cn, &mname, val.clone())
                                            {
                                                done = true;
                                            }
                                        }
                                        if !done {
                                            let fitted = self.fit_class_prop(cur, &mname, val);
                                            if let Some(Some(inst)) = self.heap.get_mut(cur) {
                                                inst.properties.insert(mname, fitted);
                                                done = true;
                                            }
                                        }
                                    } else if let Some(h) = self.member_handle(cur, &mname) {
                                        cur = h;
                                    } else {
                                        break;
                                    }
                                }
                                if done {
                                    return true;
                                }
                            }
                        }
                    }
                    let obj_name = &hier.path[0].name.name;
                    {
                        let sub = hier.path[1..]
                            .iter()
                            .map(|s| s.name.name.as_str())
                            .collect::<Vec<_>>()
                            .join(".");
                        if let Some(fields) =
                            self.module.packed_struct_fields.get(obj_name).cloned()
                        {
                            if let Some((_, off, w)) =
                                fields.iter().find(|(m, _, _)| m == &sub).cloned()
                            {
                                // The container may be a procedural LOCAL (a
                                // packed struct declared inside a task), which
                                // `get/set_signal_value_by_name` cannot see.
                                if let Some(cur_sig) = self.get_local_or_signal(obj_name) {
                                    let total_w = cur_sig.width;
                                    let mut cur = cur_sig.resize(total_w);
                                    let piece = val.resize(w);
                                    for i in 0..w {
                                        let bit = piece.get_bit(i as usize);
                                        cur.set_bit((off + i) as usize, bit);
                                    }
                                    let prev = self.get_local_or_signal(obj_name);
                                    let changed = prev.as_ref() != Some(&cur);
                                    self.set_local_or_signal(obj_name, cur);
                                    return changed;
                                }
                            }
                        }
                    }
                    let obj_val = if let Some(locals) = self.local_stack.last() {
                        locals
                            .get(obj_name)
                            .cloned()
                            .or_else(|| self.get_signal_value_by_name(obj_name))
                    } else {
                        self.get_signal_value_by_name(obj_name)
                    };
                    // Mirror SV writes to the unpacked-struct member signal
                    // (e.g. `t_unpacked_struct.val_i`) so VPI-forced values
                    // and SV reads stay in sync. Both storage locations
                    // (heap property and separate signal) are written; the
                    // read path prefers the separate signal.
                    if hier.path.len() == 2 {
                        let dotted =
                            format!("{}.{}", hier.path[0].name.name, hier.path[1].name.name);
                        if self.signal_name_to_id.contains_key(dotted.as_str()) {
                            if let Some(id) = self.signal_name_to_id.get(dotted.as_str()).copied() {
                                let width = self.signal_widths[id];
                                let mut resized = if self.signal_real[id] {
                                    if val.is_real {
                                        val.clone()
                                    } else {
                                        Value::from_f64(val.to_f64())
                                    }
                                } else {
                                    if val.is_real {
                                        Self::real_to_int(val.to_f64(), width)
                                    } else {
                                        val.resize(width)
                                    }
                                };
                                resized.is_signed = self.signal_signed[id];
                                self.set_signal_value_by_name(&dotted, resized);
                                // Fall through to also update the heap so any
                                // existing read path that looks there stays
                                // consistent. Don't `return` — let the heap
                                // write happen too.
                            }
                        }
                    }
                    if let Some(v) = obj_val {
                        let mut cur_handle = v.to_u64().unwrap_or(0) as usize;
                        for i in 1..hier.path.len() {
                            if cur_handle == 0 || cur_handle >= self.heap.len() {
                                break;
                            }
                            let member_name = &hier.path[i].name.name;
                            if i == hier.path.len() - 1 {
                                let holds = self
                                    .heap
                                    .get(cur_handle)
                                    .and_then(|o| o.as_ref())
                                    .is_some_and(|inst| inst.properties.contains_key(member_name));
                                // id-collision-in-heap: the walk may reinterpret
                                // the head's integral value as a heap handle
                                // only when the head statically names a
                                // class-object lvalue — a struct variable or a
                                // struct-element collection splices its members
                                // into plain data (§7.8) that must stay out of
                                // the heap.
                                if holds && self.bare_name_is_class_channel(obj_name) {
                                    let fitted = self.fit_class_prop(cur_handle, member_name, val);
                                    if let Some(Some(inst)) = self.heap.get_mut(cur_handle) {
                                        inst.properties.insert(member_name.clone(), fitted);
                                        return true;
                                    }
                                }
                                break;
                            }
                            if let Some(Some(inst)) = self.heap.get(cur_handle) {
                                if let Some(mval) = inst.properties.get(member_name) {
                                    cur_handle = mval.to_u64().unwrap_or(0) as usize;
                                } else {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                    }
                    // FALLBACK: If the full dotted name exists as a separate signal
                    // (xezim's elaboration scheme for unpacked struct members),
                    // write to it directly.
                    let dotted_name = hier
                        .path
                        .iter()
                        .map(|s| s.name.name.as_str())
                        .collect::<Vec<_>>()
                        .join(".");
                    // Only a name WITHOUT a table slot lives in the name-keyed
                    // copy alone. After `sync_table_to_hashmap` (a `$monitor`,
                    // a dump, a `$readmemh`) every table signal has a copy
                    // there too; writing that copy loses the write at the next
                    // sync while every read uses the table (#568, §10.4.1).
                    if !self.signal_name_to_id.contains_key(dotted_name.as_str())
                        && let Some(existing_val) = self.signals.get(&dotted_name).cloned()
                    {
                        let w = existing_val.width;
                        let is_real =
                            self.real_signals.contains(&dotted_name) || existing_val.is_real;
                        let mut resized = if is_real {
                            if val.is_real {
                                val.clone()
                            } else {
                                Value::from_f64(val.to_f64())
                            }
                        } else {
                            if val.is_real {
                                Value::from_u64(val.to_f64() as u64, w)
                            } else {
                                val.resize(w)
                            }
                        };
                        resized.is_signed = self.signed_signals.contains(&dotted_name);
                        let changed = existing_val != resized;
                        if changed {
                            self.mark_dirty(&dotted_name);
                        }
                        self.signals.insert(dotted_name, resized);
                        return changed;
                    }
                }
                let is_ambiguous_leaf =
                    hier.path.len() == 1 && !hier.path[0].name.name.contains('.');
                if let Some(id) = hier.cached_signal_id.get() {
                    if !is_ambiguous_leaf {
                        let resized = self.fit_value_to_signal(id, val);
                        let changed = self.signal_table[id] != resized;
                        if changed {
                            self.mark_dirty_id(id);
                            write_sig!(self, id, resized);
                            self.table_modified = true;
                        }
                        return changed;
                    }
                }
                let name = self.resolve_hier_name(hier);
                if let Some(&id) = self.signal_name_to_id.get(name.as_ref()) {
                    hier.cached_signal_id.set(Some(id));
                    let resized = self.fit_value_to_signal(id, val);
                    let changed = self.signal_table[id] != resized;
                    if changed {
                        self.mark_dirty_id(id);
                        write_sig!(self, id, resized);
                        self.table_modified = true;
                    }
                    return changed;
                }
                // Fallback: signal not in signal_name_to_id (truly unknown,
                // e.g. testbench probe wires that didn't elaborate). Write
                // directly into the legacy `signals` HashMap without first
                // forcing a 35M-entry sync from signal_table — sync was the
                // dominant cost (20s × ~40 calls = 14 minutes) of c910 time-0
                // settle. We only write here, never read from self.signals
                // before this path, so the sync was unnecessary.
                // A 0 width here means the flat `widths` map was polluted
                // by a same-named variable from another scope — never a
                // valid lvalue width, so fall back to the value's width.
                // A member of a nested packed struct / untagged union aliases the
                // container's storage (§7.3): splice into it rather than create an
                // independent leaf that would stop aliasing its siblings. Its dotted
                // name parses as one hierarchical ident, so the root-keyed field
                // lookups above never matched it.
                if let Some((base, off, w)) = self.packed_leaf_of_hier(&name) {
                    if let Some(cur) = self.get_signal_value_by_name(&base) {
                        let mut next = cur.clone();
                        let mut piece = val.resize(w);
                        if !self.mixed_packed_vars.is_empty() {
                            piece = self.packed_member_piece(&base, &name, piece);
                        }
                        for i in 0..w {
                            next.set_bit((off + i) as usize, piece.get_bit(i as usize));
                        }
                        let changed = next != cur;
                        if changed {
                            self.set_signal_value_by_name(&base, next);
                        }
                        return changed;
                    }
                }
                // LRM §10.6/§10.6.1: a target under an active force or
                // procedural continuous assign ignores ordinary procedural
                // writes until the matching release/deassign.
                if self.forced_names.contains(&*name) {
                    return false;
                }
                let width = self
                    .widths
                    .get(&*name)
                    .copied()
                    .filter(|w| *w > 0)
                    .unwrap_or(val.width);
                let is_real = self.real_signals.contains(&*name);
                // §6.16: a string has no declared length — for a `string`
                // LOCAL the widths-map entry is the 1024-bit placeholder
                // `resolve_type_width` hands the dynamic type, and resizing to
                // it truncates the FRONT of the text (a packed string keeps
                // its text in the low bits). Module-level strings are exempt
                // via `signal_is_string` on the signal-table path; this
                // map-backed store is where PROCEDURAL LOCALS land, so exempt
                // them here too. Issue #64: a 129-char `$system` command came
                // back as 128 chars shifted one byte left ("echo …" → "cho …").
                let is_str = self.string_signals.contains(&*name)
                    || hier
                        .path
                        .last()
                        .is_some_and(|s| self.string_signals.contains(&s.name.name));
                let mut resized = if is_real {
                    if val.is_real {
                        val.clone()
                    } else {
                        Value::from_f64(val.to_f64())
                    }
                } else if is_str && !val.is_real {
                    val.clone()
                } else {
                    if val.is_real {
                        Self::real_to_int(val.to_f64(), width)
                    } else {
                        val.resize(width)
                    }
                };
                resized.is_signed = self.signed_signals.contains(&*name);
                let changed = self.signals.get(&*name).is_none_or(|p| *p != resized);
                if changed {
                    self.mark_dirty(&name);
                }
                self.signals.insert(name.to_string(), resized.clone());

                // If this is an array or queue, and we are assigning a packed value,
                // we might want to split it into elements.
                if env_set_cached!("XEZIM_A1_DBG") && name.contains('m') {
                    eprintln!(
                        "[A1DBG] whole-name write name={:?} in_arrays={} hint={:?}",
                        name,
                        self.module.arrays.contains_key(&*name),
                        self.name_resolve_hint.borrow().clone()
                    );
                }
                if let Some((lo, hi, elem_width)) = self.module.arrays.get(&*name).cloned() {
                    let num_elements = (resized.width / elem_width) as usize;
                    // For queues/dynamic arrays, we update the size
                    let is_dynamic = hi < lo || hi == 63 && lo == 0; // simplistic check for [lo:hi] vs []/[$]
                    if is_dynamic {
                        self.signals.insert(
                            format!("{}.size", name),
                            Value::from_u64(num_elements as u64, 32),
                        );
                    }
                    for i in 0..num_elements {
                        let l = (num_elements - 1 - i) * elem_width as usize
                            + (elem_width as usize - 1);
                        let r = (num_elements - 1 - i) * elem_width as usize;
                        let elem_val = resized.range_select(l, r);
                        self.signals.insert(format!("{}[{}]", name, i), elem_val);
                    }
                }

                changed
            }
            ExprKind::Index { expr, index } => self.assign_value_index(lhs, val, expr, index),
            ExprKind::RangeSelect {
                expr,
                left,
                right,
                kind,
            } => {
                // §11.5.1: a part-select bound may be NEGATIVE, and a write
                // through a partially out-of-range select stores only the bits
                // that DO land in the vector (`bit [3:0] x; x[4:-1] <= 6'b101010`
                // leaves x = 4'b0101). Reading the bounds through `to_u64` made
                // -1 a huge unsigned index, so `l + r - 1` overflowed and
                // aborted the run. `to_i64` sign-extends only a value whose
                // type IS signed, so ordinary indices are untouched.
                // §11.5.1: an index containing x or z selects nothing — the
                // write is DISCARDED, not applied at bit 0. `to_i64` reports
                // that as None; folding it to 0 silently wrote the low bits
                // (`i = 'hx; x[i +: 2] = 2'b11` left x = 4'b0011).
                let lval_b = self.eval_expr(left);
                let rval_b = self.eval_expr(right);
                if lval_b.has_xz() || rval_b.has_xz() {
                    return false;
                }
                let (Some(mut li), Some(mut ri)) = (lval_b.to_i64(), rval_b.to_i64()) else {
                    return false;
                };
                // Offset a part-select's bit indices by the declared lower
                // bound of a PLAIN packed vector (`logic [7:4] v; v[6:5]` ->
                // storage bits [2:1]), mirroring the read path (LRM §7.4.1 /
                // §11.5.1). Without this `w[3:2] = 2'b11` on `logic [3:1] w`
                // wrote storage bits [3:2] (one off the top of a 3-bit word)
                // instead of [2:1], landing on the wrong bits.
                //
                // The multi-D element branch below (packed_signal_elem_widths)
                // applies its OWN lo_b normalization inside `lsb_of`, and the
                // ascending / unpacked-array branches index by element rather
                // than bit — so each of those must keep the RAW indices and is
                // excluded here. Bit-selects are already correct in both
                // directions; this closes the part-select WRITE gap.
                let mut elem_ascending_dim: Option<(i64, i64)> = None;
                let class_dim = self.class_prop_select_dim(expr);
                if let Some((dl, dr)) = class_dim {
                    if dl < dr {
                        li = match kind {
                            RangeKind::Constant => dr - li,
                            RangeKind::IndexedUp => dr - li - (ri - 1),
                            RangeKind::IndexedDown => dr - li + (ri - 1),
                        };
                        if matches!(kind, RangeKind::Constant) {
                            ri = dr - ri;
                        }
                    } else {
                        li -= dr;
                        if matches!(kind, RangeKind::Constant) {
                            ri -= dr;
                        }
                    }
                } else if let ExprKind::Ident(h) = &expr.kind {
                    let nm = self.resolve_hier_name(h);
                    let is_multi_d = self
                        .module
                        .packed_signal_elem_widths
                        .get(&*nm)
                        .copied()
                        .unwrap_or(0)
                        > 1;
                    let is_ascending = self.module.ascending_packed.contains_key(&*nm);
                    let is_unpacked = self.module.arrays.contains_key(&*nm);
                    if !is_multi_d && !is_ascending && !is_unpacked {
                        if let Some(&(dl, dr)) = self
                            .module
                            .packed_full_dims
                            .get(&*nm)
                            .and_then(|d| d.first())
                        {
                            let lo_b = dl.min(dr);
                            if lo_b != 0 {
                                // Offset the base/lo INDEX only. In
                                // `[base +: W]` / `[base -: W]` the right
                                // operand is a WIDTH, not an index, so it must
                                // not be shifted — only the constant `[msb:lsb]`
                                // form has two indices (LRM §7.4.1 / §11.5.1).
                                li -= lo_b;
                                if matches!(kind, RangeKind::Constant) {
                                    ri -= lo_b;
                                }
                            }
                        }
                    }
                } else if let Some((dl, dr)) = self.select_base_elem_dim(expr) {
                    // ELEMENT of an unpacked array/queue/assoc, same mapping
                    // as the read side: descending offsets the labels here;
                    // an ascending dim mirrors msb/lsb after they are formed
                    // (the plain-Ident ascending block below stays untouched).
                    if dl >= dr {
                        let lo_b = dr;
                        if lo_b != 0 {
                            li -= lo_b;
                            if matches!(kind, RangeKind::Constant) {
                                ri -= lo_b;
                            }
                        }
                    } else {
                        elem_ascending_dim = Some((dl, dr));
                    }
                }
                let (msb_i, lsb_i): (i64, i64) = match kind {
                    RangeKind::Constant => (li.max(ri), li.min(ri)),
                    RangeKind::IndexedUp => (li + (ri - 1).max(0), li),
                    RangeKind::IndexedDown => (li, li - (ri - 1).max(0)),
                };
                // The SOURCE bit for target bit `i` is `i - lsb_i`, which stays
                // signed: with lsb_i = -1 the vector's bit 0 takes source bit 1.
                let mut src_base = lsb_i;
                let mut msb_i = msb_i;
                // Ascending vector: the value's bit 0 lands on the select's
                // LAST label, which is its LOWEST physical bit, so source
                // bits count up from `phys_lo` — the same as a descending
                // select. Kept for the partial-OOB case where the physical
                // range is clamped: the loop needs the UNCLAMPED high end.
                let mut asc_phys_hi: Option<i64> = None;
                let l = li.max(0) as usize;
                let r = ri.max(0) as usize;
                let (mut msb, mut lsb) = (msb_i.max(0) as usize, lsb_i.max(0) as usize);
                // §7.4.1 element-range WRITE on a packed multi-D base:
                // `wv[2:1] = 16'hBEEF` / `wv[1 +: 2] = …` targets whole
                // ELEMENTS. The bit-based msb/lsb above wrote a 2-bit sliver
                // instead. Scale by the registered element width, normalized
                // against the declared outer dimension (mirrors the read side).
                let mut elem_scaled = false;
                if let ExprKind::Ident(h) = &expr.kind {
                    let nm = self.resolve_hier_name(h);
                    if let Some(&elem_w) = self
                        .module
                        .packed_signal_elem_widths
                        .get(&*nm)
                        .filter(|&&w| w > 1)
                    {
                        let (lab_a, lab_b): (i64, i64) = match kind {
                            RangeKind::Constant => (l as i64, r as i64),
                            RangeKind::IndexedUp => (l as i64, l as i64 + r as i64 - 1),
                            RangeKind::IndexedDown => (l as i64, l as i64 - (r as i64 - 1)),
                        };
                        let dim = self
                            .module
                            .packed_full_dims
                            .get(&*nm)
                            .and_then(|d| d.first())
                            .copied();
                        let lsb_of = |idx: i64| -> i64 {
                            match dim {
                                None => idx * elem_w as i64,
                                Some((dl, dr)) => {
                                    let (lo_b, hi_b) = (dl.min(dr), dl.max(dr));
                                    let count = hi_b - lo_b + 1;
                                    let off = idx - lo_b;
                                    let slot = if dl >= dr { off } else { count - 1 - off };
                                    slot * elem_w as i64
                                }
                            }
                        };
                        let la = lsb_of(lab_a);
                        let lb = lsb_of(lab_b);
                        lsb = la.min(lb).max(0) as usize;
                        msb = (la.max(lb) + elem_w as i64 - 1).max(0) as usize;
                        elem_scaled = true;
                    }
                }
                // Ascending packed vector part-write (`logic [0:7] pa; pa[0:3]=v`):
                // labels index from the MSB end → internal [(W-1)-lsb : (W-1)-msb]
                // (LRM §7.4.1, §11.5.1).
                if !elem_scaled {
                    if let ExprKind::Ident(h) = &expr.kind {
                        let nm = self.resolve_hier_name(h);
                        if let Some((_, hi_l)) = self.module.ascending_packed.get(&*nm).copied() {
                            // Labels were kept raw above (no low-bound offset
                            // for ascending vectors); label p is physical bit
                            // `high - p`, so the select's high label is its
                            // LOW physical bit.
                            // The bounds reaching here are already resolved to a
                            // [msb:lsb] pair for every RangeKind, so one mapping
                            // serves the constant and indexed forms alike.
                            // Restricting it to the constant form wrote
                            // `aw[4 +: 4]` at the DESCENDING position.
                            // A label past the LSB gives a NEGATIVE physical
                            // low bit: keep it in `src_base` (the store loop
                            // maps source bits from there, as for a negative
                            // constant bound) and clamp only the loop bounds.
                            // A label before the MSB overruns the width; the
                            // loop's `min(width - 1)` drops those bits.
                            let phys_lo = hi_l - msb_i;
                            let phys_hi = hi_l - lsb_i;
                            src_base = phys_lo;
                            msb = phys_hi.max(0) as usize;
                            lsb = phys_lo.max(0) as usize;
                            msb_i = phys_hi;
                            asc_phys_hi = Some(phys_hi);
                        }
                    } else if let Some((dl, dr)) = elem_ascending_dim {
                        // Ascending ELEMENT dim (`logic [0:23] a[0:0];
                        // a[0][0:7] = v`): labels mirror against the right
                        // bound — physical = right - label — so the formed
                        // [msb:lsb] label pair swaps and shifts as one.
                        let _ = dl;
                        let (new_msb, new_lsb) = (
                            (dr - lsb as i64).max(0) as usize,
                            (dr - msb as i64).max(0) as usize,
                        );
                        msb = new_msb;
                        lsb = new_lsb;
                    }
                }
                // Unpacked array slice assignment: copy element-by-element
                if let ExprKind::Ident(hier) = &expr.kind {
                    let name = self.resolve_hier_name(hier);
                    if let Some(&(arr_lo, arr_hi, elem_w)) = self.module.arrays.get(&*name) {
                        let count = msb + 1 - lsb;
                        let descending = self.module.descending_arrays.contains(&*name);
                        let mut changed = false;
                        let _ = (arr_lo, descending);
                        for i in 0..count {
                            let lhs_idx = (lsb + i) as i64;
                            if lhs_idx < arr_lo || lhs_idx > arr_hi {
                                continue;
                            }
                            let elem_name = format!("{}[{}]", name, lhs_idx);
                            let new_val = if (val.width as usize) == count * elem_w as usize {
                                val.range_select((i + 1) * elem_w as usize - 1, i * elem_w as usize)
                            } else {
                                val.clone()
                            };
                            if self.get_signal_value_by_name(&elem_name).as_ref() != Some(&new_val)
                            {
                                self.set_signal_value_by_name(&elem_name, new_val);
                                changed = true;
                            }
                        }
                        return changed;
                    }
                }
                // A selected element of a fixed class-property array is stored
                // as an instance-scoped value rather than a signal-table slot.
                // Apply the part write to that element before the generic
                // signal-id path, which cannot resolve the bare property name.
                if let ExprKind::Index {
                    expr: array_expr,
                    index,
                } = &expr.kind
                {
                    let index_value = self.eval_expr(index);
                    if index_value.has_xz() && !index_value.is_real {
                        return false;
                    }
                    // §7.8.6: a select write to an ABSENT associative element
                    // reads it as the element type's default, replaces the
                    // selected bits and writes it back, creating it; the
                    // write was dropped. Its key takes the array's key form.
                    let mut assoc_default: Option<Value> = None;
                    let element_name = self.class_nd_elem_name(array_expr, index).or_else(|| {
                        // A module-scope or block-local array is not an
                        // object member: resolve the plain name too.
                        let base = self.expr_assoc_name(array_expr).or_else(|| {
                            let ExprKind::Ident(h) = &array_expr.kind else {
                                return None;
                            };
                            let n = self.resolve_hier_name(h).into_owned();
                            let n = self.instance_assoc_member(&n).unwrap_or(n);
                            self.is_associative_array(&n).then_some(n)
                        })?;
                        if self.is_associative_array(&base) {
                            let key = self.assoc_key_str(&base, &index_value);
                            assoc_default = self.assoc_missing_elem_value(&base);
                            return Some(format!("{}[{}]", base, key));
                        }
                        let index = index_value.to_i64()?;
                        Some(format!("{}[{}]", base, index))
                    });
                    if let Some(element_name) = element_name {
                        if let Some(mut current) = self
                            .get_signal_value_by_name(&element_name)
                            .or(assoc_default)
                        {
                            let width = current.width as usize;
                            let hi = msb.min(width.saturating_sub(1));
                            if oob_write_discard(msb_i, src_base, width as u32) {
                                return false;
                            }
                            let base = if src_base < 0 { src_base } else { lsb as i64 };
                            let mut changed = false;
                            if msb_i >= 0 && hi >= lsb {
                                for bit in lsb..=hi {
                                    let source = bit as i64 - base;
                                    if source < 0 {
                                        continue;
                                    }
                                    let next = val.get_bit(source as usize);
                                    if current.get_bit(bit) != next {
                                        current.set_bit(bit, next);
                                        changed = true;
                                    }
                                }
                            }
                            if changed || !self.signals.contains_key(&element_name) {
                                self.set_signal_value_by_name(&element_name, current);
                            }
                            return changed;
                        }
                    }
                }
                // §11.5.1: splice a selected class property back into the
                // object, including static and nested hierarchical receivers.
                if let ExprKind::Ident(hier) = &expr.kind
                    && let Some(h) = self.hier_parent_handle(hier)
                {
                    let member = &hier.path.last().unwrap().name.name;
                    if let Some(mut cur) = self
                        .static_read_via_handle(h, None, None, member)
                        .or_else(|| self.read_member_value(h, member))
                    {
                        if oob_write_discard(msb_i, src_base, cur.width) {
                            return false;
                        }
                        let rhs = val.resize_for_assign((msb_i - src_base + 1).max(1) as u32);
                        for bit in lsb..=msb.min(cur.width.saturating_sub(1) as usize) {
                            cur.set_bit(bit, rhs.get_bit((bit as i64 - src_base) as usize));
                        }
                        return self.set_prop_if_changed(h, member, cur);
                    }
                }
                // Fast path: resolve target signal_id directly (avoids format!
                // + HashMap lookup on every call for tight memory-init loops).
                let target_id: Option<usize> = match &expr.kind {
                    ExprKind::Ident(hier) => {
                        if self.bare_selected_storage_shadows_signal(hier) {
                            None
                        } else if let Some(id) = hier.cached_signal_id.get() {
                            Some(id)
                        } else {
                            let name = self.resolve_hier_name(hier);
                            if let Some(&id) = self.signal_name_to_id.get(name.as_ref()) {
                                hier.cached_signal_id.set(Some(id));
                                Some(id)
                            } else {
                                None
                            }
                        }
                    }
                    ExprKind::Index {
                        expr: arr_expr,
                        index,
                    } => {
                        if let ExprKind::Ident(hier) = &arr_expr.kind {
                            let name = self.resolve_hier_name(hier);
                            if self.module.arrays.contains_key(&*name) {
                                // §7.4.6: SIGNED index — a negative-lo array
                                // (`m[-2:1]`) loses its negative elements when
                                // the index goes through to_u64.
                                let idx = self.eval_expr(index).to_i64().unwrap_or(0);
                                self.get_array_elem_id(&name, idx)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(id) = target_id {
                    // Packed cell: no signal_table slot, so the whole-word
                    // path below cannot apply. Splice the bits into the cell.
                    if is_packed_id(id) {
                        let w = self.packed.width(id);
                        let mut cur = self.packed.read(id);
                        let hi = (msb as usize).min(w.saturating_sub(1) as usize);
                        if oob_write_discard(msb_i, src_base, w) {
                            return false;
                        }
                        for i in lsb..=hi {
                            let src = if src_base < 0 {
                                i as i64 + src_base
                            } else {
                                i as i64 - lsb as i64
                            };
                            if src >= 0 && (src as u32) < val.width {
                                cur.set_bit(i, val.get_bit(src as usize));
                            }
                        }
                        let changed = self.packed_store_value(id, &cur);
                        return changed;
                    }
                    let width = self.signal_widths[id] as usize;
                    // Whole-word fast path: if the range covers the whole
                    // signal, skip the per-bit loop. A NEGATIVE low bound is
                    // excluded — the clamped `lsb` is 0 there, but the source
                    // bits are SHIFTED (`x[4:-1]` puts source bit 1 in x[0]),
                    // so resizing the value and storing it whole picks the
                    // wrong bits.
                    if src_base >= 0 && lsb == 0 && msb + 1 >= width {
                        let resized = val.resize(width as u32);
                        let changed = self.signal_table[id] != resized;
                        if changed {
                            self.mark_dirty_id(id);
                            write_sig!(self, id, resized);
                            self.table_modified = true;
                        }
                        return changed;
                    }
                    let mut changed = false;
                    let hi = msb.min(width.saturating_sub(1));
                    if oob_write_discard(msb_i, src_base, width as u32) {
                        return false;
                    }
                    // The source offset is the EFFECTIVE low bit — `lsb` after
                    // the element-scaling and ascending-declaration rewrites
                    // above, which is what the loop always used. Only a
                    // NEGATIVE low bound needs the pre-clamp value, and those
                    // rewrites never apply to one (they clamp at 0).
                    let base = if src_base < 0 { src_base } else { lsb as i64 };
                    // `msb_i < 0` means the whole select sits below bit 0 —
                    // nothing is written (the clamped msb/lsb would otherwise
                    // both read 0 and clobber the low bit).
                    if msb_i >= 0 && hi >= lsb {
                        // Width of the SELECT in bits (unclamped), so a value
                        // wider than the select is truncated to it before the
                        // in-range bits are placed (§10.7 / §11.5.1).
                        // Only for a plain vector: an element-scaled select
                        // keeps its label-level `msb_i`/`src_base`, and its
                        // value is already element-sized.
                        let sel_w = if elem_scaled {
                            0
                        } else {
                            (msb_i - src_base + 1).max(0) as u32
                        };
                        let fitted;
                        let val: &Value = if sel_w > 0 && val.width > sel_w {
                            fitted = val.resize(sel_w);
                            &fitted
                        } else {
                            val
                        };
                        let _ = asc_phys_hi;
                        for i in lsb..=hi {
                            let src = i as i64 - base;
                            if src < 0 {
                                continue;
                            }
                            let nb = val.get_bit(src as usize);
                            if self.signal_table[id].get_bit(i) != nb {
                                self.signal_table[id].set_bit(i, nb);
                                self.sync_mirror(id);
                                changed = true;
                            }
                        }
                    }
                    if changed {
                        self.table_modified = true;
                        self.after_signal_write(id);
                        self.mark_dirty_id(id);
                    }
                    return changed;
                }
                // Fall-through for LHS range-select on storage NOT backed by
                // signal_table — local-stack vars (`logic [3:0] r; r[3:0] = ...`
                // inside an initial/task), this.<prop> instance fields, and
                // module signals only in the slow-path `signals` HashMap.
                // Without these branches `is_equal[3:0] = {4{...}}` in
                // cv32e40p_alu would silently no-op and leave is_equal at X,
                // breaking BLT → traps → infinite handler loop on every
                // compliance test.
                if let ExprKind::Ident(hier) = &expr.kind {
                    // Class instance field / packed struct field via flattened
                    // hier path: `h.field[msb:lsb]` and `s.field[msb:lsb]` both
                    // parse as Ident([root, field]) + RangeSelect.
                    if hier.path.len() == 2 {
                        let root = hier.path[0].name.name.clone();
                        let fname = hier.path[1].name.name.clone();
                        // (b) Packed struct: slice into the parent signal at
                        //     `packed_struct_fields[root] → (fname, off, w)`.
                        if let Some(fields) = self.module.packed_struct_fields.get(&root).cloned() {
                            if let Some((_, field_off, field_w)) =
                                fields.iter().find(|(m, _, _)| m == &fname).cloned()
                            {
                                // §7.4.1: an ASCENDING member dim mirrors the
                                // bit labels (`bit [0:15] b; b[14:15]` = the
                                // LOW two bits).
                                let (lsb, msb) =
                                    if let Some(mw) = self.struct_member_ascending(&root, &fname) {
                                        let mw = mw as usize;
                                        let m2 = (mw - 1).saturating_sub(lsb);
                                        let l2 = (mw - 1).saturating_sub(msb.min(mw - 1));
                                        (l2, m2)
                                    } else {
                                        (lsb, msb)
                                    };
                                if let Some(&id) = self.signal_name_to_id.get(root.as_str()) {
                                    let total_w = self.signal_widths[id] as usize;
                                    let lo = (field_off as usize) + lsb;
                                    let hi = (field_off as usize)
                                        + msb.min((field_w as usize).saturating_sub(1));
                                    if hi < total_w {
                                        let mut cur = self.signal_table[id].clone();
                                        let mut changed = false;
                                        for i in lo..=hi {
                                            let nb = val.get_bit(i - lo);
                                            if cur.get_bit(i) != nb {
                                                cur.set_bit(i, nb);
                                                changed = true;
                                            }
                                        }
                                        if changed {
                                            self.signal_table[id] = cur;
                                            self.sync_mirror(id);
                                            self.table_modified = true;
                                            self.after_signal_write(id);
                                            self.mark_dirty(&root);
                                        }
                                        return changed;
                                    }
                                }
                                if let Some(cur_sig) = self.signals.get(&root).cloned() {
                                    let total_w = cur_sig.width as usize;
                                    let lo = (field_off as usize) + lsb;
                                    let hi = (field_off as usize)
                                        + msb.min((field_w as usize).saturating_sub(1));
                                    if hi < total_w {
                                        let mut nv = cur_sig.clone();
                                        let mut changed = false;
                                        for i in lo..=hi {
                                            let nb = val.get_bit(i - lo);
                                            if nv.get_bit(i) != nb {
                                                nv.set_bit(i, nb);
                                                changed = true;
                                            }
                                        }
                                        if changed {
                                            self.signals.insert(root.clone(), nv);
                                            self.mark_dirty(&root);
                                        }
                                        return changed;
                                    }
                                }
                            }
                        }
                        // Resolve `root` to a handle from any storage class.
                        let root_handle: Option<usize> = {
                            let from_local = self
                                .local_stack
                                .last()
                                .and_then(|m| m.get(&root))
                                .and_then(|v| v.to_u64())
                                .map(|h| h as usize)
                                .filter(|&h| h != 0);
                            let from_this = self
                                .this_stack
                                .last()
                                .copied()
                                .flatten()
                                .and_then(|h| self.heap.get(h))
                                .and_then(|o| o.as_ref())
                                .and_then(|i| i.properties.get(&root))
                                .and_then(|v| v.to_u64())
                                .map(|h| h as usize)
                                .filter(|&h| h != 0);
                            let from_sig = self
                                .signals
                                .get(&root)
                                .and_then(|v| v.to_u64())
                                .map(|h| h as usize)
                                .filter(|&h| h != 0);
                            let from_table = self
                                .signal_name_to_id
                                .get(root.as_str())
                                .and_then(|&id| self.signal_table.get(id))
                                .and_then(|v| v.to_u64())
                                .map(|h| h as usize)
                                .filter(|&h| h != 0);
                            from_local.or(from_this).or(from_sig).or(from_table)
                        };
                        if let Some(handle) = root_handle {
                            // §7.4.1: an ASCENDING property (`bit [0:15] a`)
                            // labels its bits from the MSB end, so a part-write
                            // has to be mapped the way the module-signal path
                            // already maps one. Without it `a[4:7] = 4'hF`
                            // spliced the bits a DESCENDING vector would have,
                            // even though a single-bit write to the same
                            // property landed correctly. Resolved before the
                            // mutable borrow below.
                            let (msb, lsb) = match self.class_prop_packed_shape(handle, &fname) {
                                Some((_, total, left, right))
                                    if class_dim.is_none() && left < right && total > 0 =>
                                {
                                    let top = total as usize - 1;
                                    (top.saturating_sub(lsb), top.saturating_sub(msb))
                                }
                                _ => (msb, lsb),
                            };
                            if let Some(Some(inst)) = self.heap.get_mut(handle) {
                                if let Some(cur) = inst.properties.get(&fname).cloned() {
                                    let width = cur.width as usize;
                                    let mut nv = cur.clone();
                                    let mut changed = false;
                                    for i in lsb..=msb.min(width.saturating_sub(1)) {
                                        let nb = val.get_bit(i - lsb);
                                        if nv.get_bit(i) != nb {
                                            nv.set_bit(i, nb);
                                            changed = true;
                                        }
                                    }
                                    if changed {
                                        inst.properties.insert(fname, nv);
                                    }
                                    return changed;
                                }
                            }
                        }
                    }
                    if hier.path.len() == 1 {
                        let bare = hier.path[0].name.name.clone();
                        // local_stack frame
                        if !self.local_stack.is_empty() {
                            let last_idx = self.local_stack.len() - 1;
                            if let Some(cur) = self.local_stack[last_idx].get(&bare).cloned() {
                                let width = cur.width as usize;
                                let mut nv = cur.clone();
                                let mut changed = false;
                                for i in lsb..=msb.min(width.saturating_sub(1)) {
                                    let nb = val.get_bit(i - lsb);
                                    if nv.get_bit(i) != nb {
                                        nv.set_bit(i, nb);
                                        changed = true;
                                    }
                                }
                                if changed {
                                    self.local_stack[last_idx].insert(bare.clone(), nv);
                                }
                                return changed;
                            }
                        }
                        // this.<prop>
                        if let Some(Some(handle)) = self.this_stack.last().copied() {
                            if let Some(Some(inst)) = self.heap.get_mut(handle) {
                                if let Some(cur) = inst.properties.get(&bare).cloned() {
                                    let width = cur.width as usize;
                                    let mut nv = cur.clone();
                                    let mut changed = false;
                                    for i in lsb..=msb.min(width.saturating_sub(1)) {
                                        let nb = val.get_bit(i - lsb);
                                        if nv.get_bit(i) != nb {
                                            nv.set_bit(i, nb);
                                            changed = true;
                                        }
                                    }
                                    if changed {
                                        inst.properties.insert(bare, nv);
                                    }
                                    return changed;
                                }
                            }
                        }
                    }
                    // signals slow-path
                    let name = self.resolve_hier_name(hier);
                    if let Some(cur) = self.signals.get(&*name).cloned() {
                        let width = cur.width as usize;
                        let mut nv = cur.clone();
                        let mut changed = false;
                        for i in lsb..=msb.min(width.saturating_sub(1)) {
                            let nb = val.get_bit(i - lsb);
                            if nv.get_bit(i) != nb {
                                nv.set_bit(i, nb);
                                changed = true;
                            }
                        }
                        if changed {
                            self.signals.insert(name.to_string(), nv);
                            self.mark_dirty(&name);
                        }
                        return changed;
                    }
                }
                // MemberAccess base: `obj.field[msb:lsb]` or `s.field[msb:lsb]`.
                // Two storage classes:
                //   (a) `obj` is a class instance handle, `field` lives in
                //       `heap[handle].properties` — bit-loop into the property
                //       value.
                //   (b) `s` is a packed struct whose backing signal exists in
                //       signal_table / signals slow-path, and `field` is a
                //       slice at `packed_struct_fields[s] → (field, off, w)` —
                //       bit-loop into the parent signal at `off+lsb .. off+msb`.
                if let ExprKind::MemberAccess { expr: base, member } = &expr.kind {
                    // (a) class instance property.
                    let handle_opt: Option<usize> = match &base.kind {
                        ExprKind::This => self.this_stack.last().copied().flatten(),
                        _ => {
                            let v = self.eval_expr(base);
                            v.to_u64().map(|h| h as usize).filter(|&h| h != 0)
                        }
                    };
                    if let Some(handle) = handle_opt {
                        // id-collision-in-heap: the receiver's VALUE is a heap
                        // handle only when it statically names a class object.
                        // A part-selected member write on a struct-element
                        // collection (`q[0].f[3:0] = v`) splices plain data
                        // (§7.8) and must not be rerouted into the heap.
                        if self.receiver_may_be_handle(base) {
                            let pname = member.name.clone();
                            if let Some(Some(inst)) = self.heap.get_mut(handle) {
                                if let Some(cur) = inst.properties.get(&pname).cloned() {
                                    let width = cur.width as usize;
                                    let mut nv = cur.clone();
                                    let mut changed = false;
                                    for i in lsb..=msb.min(width.saturating_sub(1)) {
                                        let nb = val.get_bit(i - lsb);
                                        if nv.get_bit(i) != nb {
                                            nv.set_bit(i, nb);
                                            changed = true;
                                        }
                                    }
                                    if changed {
                                        inst.properties.insert(pname, nv);
                                    }
                                    return changed;
                                }
                            }
                        }
                    }
                    // (b) packed struct field. A struct of the running frame
                    // takes (c): it hides a module variable of the same name
                    // (§23.9), which this would write.
                    if let ExprKind::Ident(bh) = &base.kind
                        && self.frame_bound_bare_name(bh).is_none()
                    {
                        let base_name = self.resolve_hier_name(bh);
                        if let Some(fields) =
                            self.module.packed_struct_fields.get(&*base_name).cloned()
                        {
                            if let Some((_, field_off, field_w)) =
                                fields.iter().find(|(m, _, _)| m == &member.name).cloned()
                            {
                                // signal_table path
                                if let Some(&id) = self.signal_name_to_id.get(base_name.as_ref()) {
                                    let total_w = self.signal_widths[id] as usize;
                                    let lo = (field_off as usize) + lsb;
                                    let hi = (field_off as usize)
                                        + msb.min((field_w as usize).saturating_sub(1));
                                    if hi < total_w {
                                        let mut changed = false;
                                        let mut cur = self.signal_table[id].clone();
                                        for i in lo..=hi {
                                            let nb = val.get_bit(i - lo);
                                            if cur.get_bit(i) != nb {
                                                cur.set_bit(i, nb);
                                                changed = true;
                                            }
                                        }
                                        if changed {
                                            self.signal_table[id] = cur;
                                            self.sync_mirror(id);
                                            self.table_modified = true;
                                            self.after_signal_write(id);
                                            self.mark_dirty(&base_name);
                                        }
                                        return changed;
                                    }
                                }
                                // signals slow-path
                                if let Some(cur_sig) = self.signals.get(&*base_name).cloned() {
                                    let total_w = cur_sig.width as usize;
                                    let lo = (field_off as usize) + lsb;
                                    let hi = (field_off as usize)
                                        + msb.min((field_w as usize).saturating_sub(1));
                                    if hi < total_w {
                                        let mut nv = cur_sig.clone();
                                        let mut changed = false;
                                        for i in lo..=hi {
                                            let nb = val.get_bit(i - lo);
                                            if nv.get_bit(i) != nb {
                                                nv.set_bit(i, nb);
                                                changed = true;
                                            }
                                        }
                                        if changed {
                                            self.signals.insert(base_name.to_string(), nv);
                                            self.mark_dirty(&base_name);
                                        }
                                        return changed;
                                    }
                                }
                            }
                        }
                    }
                    // (c) any other member receiver — notably a struct
                    // variable or a struct-element collection whose VALUE
                    // must not be read back as a heap handle (the (a) gate
                    // above): read-modify-write the FIELD through the generic
                    // member paths — read `s.f` / `q[0].f`, splice the
                    // selected bits, write the whole field back.
                    {
                        let cur = self.eval_expr(expr);
                        let width = cur.width as usize;
                        let mut nv = cur.clone();
                        let mut changed = false;
                        for i in lsb..=msb.min(width.saturating_sub(1)) {
                            let nb = val.get_bit(i - lsb);
                            if nv.get_bit(i) != nb {
                                nv.set_bit(i, nb);
                                changed = true;
                            }
                        }
                        if changed {
                            return self.assign_value(expr, &nv);
                        }
                        return changed;
                    }
                }
                // Index base: `mat[i][msb:lsb]` for a packed multi-D vector
                // whose elements are elem_w bits each (registered in
                // `packed_signal_elem_widths`). Compute the slice offset
                // into the flat parent signal: `i*elem_w + lsb`.
                if let ExprKind::Index {
                    expr: arr_expr,
                    index,
                } = &expr.kind
                {
                    if let ExprKind::Ident(ah) = &arr_expr.kind {
                        let arr_name = self.resolve_hier_name(ah);
                        if let Some(&elem_w) = self.module.packed_signal_elem_widths.get(&*arr_name)
                        {
                            let idx = self.eval_expr(index).to_u64().unwrap_or(0) as usize;
                            let Some(base_off) =
                                self.packed_elem_lsb(&arr_name, idx as i64, elem_w)
                            else {
                                return false;
                            };
                            // Inner-dim labels mirror on an ascending
                            // `[0:W-1]` element type (§7.4.1).
                            let (Some(b_lsb), Some(b_msb)) = (
                                self.packed_inner_bit(&arr_name, lsb as i64, elem_w),
                                self.packed_inner_bit(
                                    &arr_name,
                                    msb.min((elem_w as usize).saturating_sub(1)) as i64,
                                    elem_w,
                                ),
                            ) else {
                                return false;
                            };
                            let lo = base_off + b_lsb.min(b_msb);
                            let hi = base_off + b_lsb.max(b_msb);
                            if let Some(&id) = self.signal_name_to_id.get(arr_name.as_ref()) {
                                let total_w = self.signal_widths[id] as usize;
                                if hi < total_w {
                                    let mut changed = false;
                                    let mut cur = self.signal_table[id].clone();
                                    for i in lo..=hi {
                                        let nb = val.get_bit(i - lo);
                                        if cur.get_bit(i) != nb {
                                            cur.set_bit(i, nb);
                                            changed = true;
                                        }
                                    }
                                    if changed {
                                        self.signal_table[id] = cur;
                                        self.sync_mirror(id);
                                        self.table_modified = true;
                                        self.after_signal_write(id);
                                        self.mark_dirty(&arr_name);
                                    }
                                    return changed;
                                }
                            }
                            if let Some(cur_sig) = self.signals.get(&*arr_name).cloned() {
                                let total_w = cur_sig.width as usize;
                                if hi < total_w {
                                    let mut nv = cur_sig.clone();
                                    let mut changed = false;
                                    for i in lo..=hi {
                                        let nb = val.get_bit(i - lo);
                                        if nv.get_bit(i) != nb {
                                            nv.set_bit(i, nb);
                                            changed = true;
                                        }
                                    }
                                    if changed {
                                        self.signals.insert(arr_name.to_string(), nv);
                                        self.mark_dirty(&arr_name);
                                    }
                                    return changed;
                                }
                            }
                            // `foo.a[i][msb:lsb]` where `a` is a packed
                            // multi-D MEMBER of a packed struct: no signal
                            // exists under the dotted name — splice at
                            // field_offset + i*elem_w + lsb within the ROOT
                            // signal (§7.4.1).
                            if let Some((root, leaf)) = arr_name.rsplit_once('.') {
                                let field_off =
                                    self.module.packed_struct_fields.get(root).and_then(|fs| {
                                        fs.iter().find(|(f, _, _)| f == leaf).map(|(_, o, _)| *o)
                                    });
                                if let (Some(field_off), Some(&id)) =
                                    (field_off, self.signal_name_to_id.get(root))
                                {
                                    let total_w = self.signal_widths[id] as usize;
                                    let alo = field_off as usize + lo;
                                    let ahi = field_off as usize + hi;
                                    if ahi < total_w {
                                        let mut cur = self.signal_table[id].clone();
                                        let mut changed = false;
                                        for i in alo..=ahi {
                                            let nb = val.get_bit(i - alo);
                                            if cur.get_bit(i) != nb {
                                                cur.set_bit(i, nb);
                                                changed = true;
                                            }
                                        }
                                        if changed {
                                            self.signal_table[id] = cur;
                                            self.sync_mirror(id);
                                            self.table_modified = true;
                                            self.after_signal_write(id);
                                            self.mark_dirty(root);
                                        }
                                        return changed;
                                    }
                                }
                            }
                        }
                    }
                }
                // §11.5.2 / §10.4: part-select WRITE through an UNPACKED
                // array element with no compact-table id — block-local and
                // automatic subroutine-local arrays live in slow-path
                // storage, and every handler above either requires a
                // signal_table id or a packed multi-D registration, so
                // `larr[1][7:4] = 4'hB` fell through to the silent `false`
                // and the write vanished. Read-modify-write the element
                // through the canonical by-name accessors instead.
                if let ExprKind::Index {
                    expr: arr_expr,
                    index,
                } = &expr.kind
                {
                    if let ExprKind::Ident(ah) = &arr_expr.kind {
                        let arr_name = self.resolve_hier_name(ah);
                        if !self
                            .module
                            .packed_signal_elem_widths
                            .contains_key(&*arr_name)
                        {
                            let idxv = self.eval_expr(index);
                            if let Some(idx) = idxv.to_i64().filter(|_| !idxv.has_xz()) {
                                let ename = format!("{}[{}]", arr_name, idx);
                                if let Some(cur) = self.get_signal_value_by_name(&ename) {
                                    let w = cur.width as usize;
                                    let mut nv = cur.clone();
                                    let mut changed = false;
                                    let hi2 = msb.min(w.saturating_sub(1));
                                    let base = if src_base < 0 { src_base } else { lsb as i64 };
                                    if msb_i >= 0 && hi2 >= lsb {
                                        for i in lsb..=hi2 {
                                            let src = i as i64 - base;
                                            if src < 0 {
                                                continue;
                                            }
                                            let nb = val.get_bit(src as usize);
                                            if nv.get_bit(i) != nb {
                                                nv.set_bit(i, nb);
                                                changed = true;
                                            }
                                        }
                                    }
                                    if changed {
                                        self.set_signal_value_by_name(&ename, nv);
                                        self.mark_dirty(&arr_name);
                                    }
                                    return changed;
                                }
                            }
                        }
                    }
                    // Deeper chains (array-of-queues element:
                    // `qa[0][0][31:24] = v`): the arm above resolves only ONE
                    // Index layer — build the flat element name for the whole
                    // chain and apply the same read-modify-write.
                    if let Some(ename) = self.flat_member_name(expr) {
                        if let Some(cur) = self.get_signal_value_by_name(&ename) {
                            let w = cur.width as usize;
                            let mut nv = cur.clone();
                            let mut changed = false;
                            let hi2 = msb.min(w.saturating_sub(1));
                            let base = if src_base < 0 { src_base } else { lsb as i64 };
                            if msb_i >= 0 && hi2 >= lsb {
                                for i in lsb..=hi2 {
                                    let src = i as i64 - base;
                                    if src < 0 {
                                        continue;
                                    }
                                    let nb = val.get_bit(src as usize);
                                    if nv.get_bit(i) != nb {
                                        nv.set_bit(i, nb);
                                        changed = true;
                                    }
                                }
                            }
                            if changed {
                                self.set_signal_value_by_name(&ename, nv);
                            }
                            return changed;
                        }
                    }
                }
                false
            }
            ExprKind::Concatenation(parts) => {
                let tw: u32 = parts.iter().map(|p| self.infer_width(p)).sum();
                let rv = val.resize(tw);
                let mut off = 0usize;
                let mut changed = false;
                for part in parts.iter().rev() {
                    let pw = self.infer_width(part);
                    let pv = rv.range_select(off + pw as usize - 1, off);
                    if self.assign_value(part, &pv) {
                        changed = true;
                    }
                    off += pw as usize;
                }
                changed
            }
            ExprKind::MemberAccess { expr, member } => {
                // §7.2.1/§7.4: a field of an element of a class array of
                // packed structs (`beats[i].last = 1`, `o.beats[i].mask =
                // m`) is a slice of the element: write the element with
                // the field's bits replaced. The read side
                // (`packed_struct_elem_type`) already slices it.
                if let ExprKind::Index { .. } = &expr.kind
                    && let Some(su) = self.packed_struct_elem_type(expr)
                {
                    let (fields, total) = self.packed_agg_layout(&su);
                    if let Some(&(_, off, w)) = fields.iter().find(|f| f.0 == member.name)
                        && total > 0
                    {
                        let mut elem = self.eval_expr(expr);
                        if elem.width != total {
                            elem = elem.resize(total);
                        }
                        let fv = val.resize(w);
                        for i in 0..w {
                            elem.set_bit((off + i) as usize, fv.get_bit(i as usize));
                        }
                        return self.assign_value(expr, &elem);
                    }
                }
                // §13.4.2: a member of a subroutine-local / formal unpacked
                // struct is owned by the call frame. Inside a subroutine body
                // `s.a = v` parses as MemberAccess (at module level the same
                // text is a dotted Ident, which the arm above already frames),
                // and every branch below looks in the module signal table — so
                // the write left the frame's copy untouched and the matching
                // read returned an outer scope's value.
                // LRM §25.9: virtual-interface alias (formal frame or
                // plain `virtual <iface>` variable). Same as the
                // hier-Ident path above but for the MemberAccess shape.
                if let ExprKind::Ident(hier) = &expr.kind {
                    if hier.path.len() == 1 {
                        {
                            let first = hier.path[0].name.name.as_str();
                            if let Some(bound) = self.iface_alias_for(first) {
                                let target = format!("{}.{}", bound, member.name);
                                let prev = self.get_signal_value_by_name(&target);
                                let w = prev.as_ref().map(|v| v.width).unwrap_or(val.width);
                                let resized = val.resize(w);
                                let changed = prev.as_ref() != Some(&resized);
                                self.set_signal_value_by_name(&target, resized);
                                return changed;
                            }
                        }
                    }
                }
                if let Some(bound) = self.indexed_iface_alias_for(expr) {
                    let target = format!("{}.{}", bound, member.name);
                    let prev = self.get_signal_value_by_name(&target);
                    let w = prev.as_ref().map(|v| v.width).unwrap_or(val.width);
                    let resized = val.resize(w);
                    let changed = prev.as_ref() != Some(&resized);
                    self.set_signal_value_by_name(&target, resized);
                    return changed;
                }
                // Unpacked aggregates keep every leaf in its own signal
                // (`arr[i].m`, `c.nodes[i].m`) — there is no contiguous bit
                // layout to slice, and a `real` member has no bit offset. If the
                // flattened name names a signal, write it directly, honouring the
                // declared width / signedness / real-ness.
                if let Some(flat) = self.flat_member_name(lhs) {
                    if let Some(&id) = self.signal_name_to_id.get(flat.as_str()) {
                        let width = self.signal_widths[id];
                        let mut resized = if self.signal_real[id] {
                            if val.is_real {
                                val.clone()
                            } else {
                                Value::from_f64(val.to_f64())
                            }
                        } else if val.is_real {
                            Self::real_to_int(val.to_f64(), width)
                        } else {
                            val.resize(width)
                        };
                        resized.is_signed = self.signal_signed[id];
                        let prev = self.get_signal_value_by_name(&flat);
                        let changed = prev.as_ref() != Some(&resized);
                        self.set_signal_value_by_name(&flat, resized);
                        return changed;
                    }
                    // Same leaf-per-member write for an element of a QUEUE /
                    // dynamic / associative collection whose element type is an
                    // unpacked struct. A top-level container happens to have
                    // table ids for its member leaves (caught above), an
                    // INSTANCE one does not — `u.q[0].latency = v` fell through
                    // every arm and vanished, so a router's per-element
                    // `q[i].latency--` never drained its queue.
                    if let Some((parent, _)) = flat.rsplit_once('.') {
                        if let Some(cont) = parent.rfind('[').map(|i| &parent[..i]) {
                            // A class-member collection stores under its
                            // instance-scoped name (`<h>#slices`), which the
                            // flattened bare name misses — `slices[0].path =
                            // v` inside a method fell through every arm and
                            // vanished (UVM's uvm_hdl_path_concat::add_path
                            // built empty slices, so every backdoor access
                            // read 0). Resolve the container through `this`
                            // before gating.
                            let registered = |s: &Self, c: &str| {
                                s.module.dynamic_arrays.contains(c)
                                    || s.module.arrays.contains_key(c)
                                    || s.module.associative_arrays.contains_key(c)
                            };
                            let (cont_s, flat_s) = if registered(self, cont) {
                                (cont.to_string(), flat.clone())
                            } else {
                                // this-relative member OR through an object
                                // handle path (`c.slices[0].path`).
                                let sc = self.resolve_locator_storage(cont);
                                if sc != cont {
                                    let f2 = format!("{}{}", sc, &flat[cont.len()..]);
                                    (sc, f2)
                                } else {
                                    // A struct element of a member array
                                    // inside a collection element
                                    // (`q[0].rows[1].m = v`): the collection
                                    // is the first select's base.
                                    let first = flat.find('[').filter(|&f| f < cont.len());
                                    match first
                                        .map(|f| (f, self.resolve_locator_storage(&flat[..f])))
                                    {
                                        Some((f, sc)) if sc != flat[..f] => {
                                            let f2 = format!("{}{}", sc, &flat[f..]);
                                            (sc, f2)
                                        }
                                        _ => (cont.to_string(), flat.clone()),
                                    }
                                }
                            };
                            if registered(self, &cont_s)
                                && self.queue_elem_struct(&cont_s).is_some()
                            {
                                let prev = self.get_signal_value_by_name(&flat_s);
                                let fitted = match prev.as_ref() {
                                    Some(p) if p.is_real && !val.is_real => {
                                        Value::from_f64(val.to_f64())
                                    }
                                    Some(p) if !p.is_real && !val.is_real => val.resize(p.width),
                                    _ => val.clone(),
                                };
                                let changed = prev.as_ref() != Some(&fitted);
                                self.set_signal_value_by_name(&flat_s, fitted);
                                return changed;
                            }
                        }
                    }
                    // A struct FORMAL bound member-wise lives in the call
                    // frame under its dotted key (`o.a`), not the signal
                    // table — the `Ident([o,a])` shape already checks
                    // `local_stack` (the dotted-local arm above), but the
                    // `MemberAccess{Ident(o),a}` shape missed and the body's
                    // `o.a = v` write was silently dropped. Gated on the
                    // dotted key actually being in the frame, so an ordinary
                    // top-level struct member (in `signal_name_to_id`) and a
                    // class property (handled by `class_agg_member`) never
                    // reach here.
                    if flat.contains('.') && !self.local_stack.is_empty() {
                        let last_idx = self.local_stack.len() - 1;
                        if self.local_stack[last_idx].contains_key(flat.as_str()) {
                            let fitted = if !val.is_real {
                                if let Some(&target_w) = self.widths.get(flat.as_str()) {
                                    if val.width < target_w {
                                        val.resize_for_assign(target_w)
                                    } else {
                                        val.clone()
                                    }
                                } else {
                                    val.clone()
                                }
                            } else {
                                val.clone()
                            };
                            self.local_stack[last_idx].insert(flat, fitted);
                            return true;
                        }
                    }
                }
                // A nested PACKED struct member inside an unpacked aggregate
                // (`arr[i].tag.vlan`): slice the parent's own signal.
                if let Some((base, off, w)) = self.packed_struct_member_slice(lhs) {
                    // The container may be a procedural LOCAL: a packed
                    // struct declared inside a task lives in the call
                    // frame, not the signal table, so a member write
                    // reached nothing and the struct read back as X.
                    if let Some(cur_sig) = self.get_local_or_signal(&base) {
                        let total_w = cur_sig.width;
                        let mut cur = cur_sig.resize(total_w);
                        let piece = val.resize(w);
                        for i in 0..w {
                            cur.set_bit((off + i) as usize, piece.get_bit(i as usize));
                        }
                        let changed = cur != cur_sig;
                        self.set_local_or_signal(&base, cur);
                        return changed;
                    }
                }
                // LRM §25.10 — `vif_arr[i].member = ...` (indexed
                // virtual-iface property write). Index{ Ident(vif_arr), idx }
                // base shape; look up `(this_h, "vif_arr[idx]")` binding.
                if let ExprKind::Index { expr: base, index } = &expr.kind {
                    if let ExprKind::Ident(hier) = &base.kind {
                        if hier.path.len() == 1 {
                            let prop_base = hier.path[0].name.name.to_string();
                            let this_h_opt = self.this_stack.last().and_then(|t| *t);
                            if let Some(this_h) = this_h_opt {
                                let cls_name = self
                                    .heap
                                    .get(this_h)
                                    .and_then(|o| o.as_ref().map(|i| i.class_name.clone()));
                                let is_vif = cls_name.as_ref().is_some_and(|cn| {
                                    self.class_vif_decl(cn, &prop_base).is_some()
                                });
                                if is_vif {
                                    let idx = self.eval_expr(index).to_u64().unwrap_or(0);
                                    let key_prop = format!("{}[{}]", prop_base, idx);
                                    if let Some((bound_name, _modport)) = self
                                        .virtual_iface_bindings
                                        .get(&(this_h, key_prop))
                                        .cloned()
                                    {
                                        let target = format!("{}.{}", bound_name, member.name);
                                        let prev = self.get_signal_value_by_name(&target);
                                        let w = prev.as_ref().map(|v| v.width).unwrap_or(val.width);
                                        let resized = val.resize(w);
                                        let changed = prev.as_ref() != Some(&resized);
                                        self.set_signal_value_by_name(&target, resized);
                                        return changed;
                                    }
                                }
                            }
                        }
                    }
                }
                // Associative-array element struct-member write:
                // `assoc[key].field = val` where the element is an (unpacked)
                // struct (an assoc keyed by a handle/key, element = struct). Stored
                // as the dotted signal `assoc[keystr].field`, plus a presence
                // marker `assoc[keystr]` so `exists(key)` sees the slot. Packed-
                // struct arrays are handled by the dedicated walk below (this is
                // gated on `is_associative_array`, which they are not).
                if let ExprKind::Index { .. } = &expr.kind {
                    // A member of a struct ELEMENT of a class collection:
                    // `assoc[key].field = v` (1-D) or `m[k1][k2][k3].field = v`
                    // (multidim). Stored as the dotted cell `<elem>.<field>`
                    // plus a presence marker `<elem>` so `exists(key)` sees the
                    // slot. The generalised resolver handles any index arity via
                    // the compound key `nested_index_name`.
                    if let Some((elem, su)) = self.coll_assoc_struct_elem(expr) {
                        if self.coll_field_width(&su, &member.name).is_some() {
                            let target = format!("{}.{}", elem, member.name);
                            let prev = self.get_signal_value_by_name(&target);
                            let w = prev.as_ref().map(|v| v.width).unwrap_or(val.width);
                            let resized = val.resize(w);
                            let changed = prev.as_ref() != Some(&resized);
                            self.set_signal_value_by_name(&target, resized);
                            if self.get_signal_value_by_name(&elem).is_none() {
                                self.set_signal_value_by_name(&elem, Value::zero(1));
                            }
                            return changed;
                        }
                    }
                }
                // LRM §25.8 — `vif.member = ...` where `vif` is a virtual-
                // interface property of the current `this`'s class. Look
                // up the binding, redirect the write to
                // `<bound>.<member>`.
                //
                // `this.vif.member = ...` is the same write with an explicit
                // `this.` and must resolve identically: it parses as
                // MemberAccess(MemberAccess(This, vif), member), so match the
                // inner `this.vif` here too. Without this the write missed the
                // redirect entirely and landed on a phantom signal, leaving the
                // real interface member reading x.
                let vif_prop_name: Option<&str> = match &expr.kind {
                    ExprKind::Ident(hier) if hier.path.len() == 1 => {
                        Some(hier.path[0].name.name.as_str())
                    }
                    ExprKind::MemberAccess {
                        expr: inner,
                        member: vprop,
                    } if matches!(&inner.kind, ExprKind::This) => Some(vprop.name.as_str()),
                    _ => None,
                };
                if let Some(prop) = vif_prop_name {
                    {
                        if let Some(Some(this_h)) = self.this_stack.last() {
                            let cls_name = if let Some(Some(inst)) = self.heap.get(*this_h) {
                                Some(inst.class_name.clone())
                            } else {
                                None
                            };
                            let bound = cls_name
                                .as_ref()
                                .and_then(|cn| self.class_vif_decl(cn, prop).map(|_| ()))
                                .and(
                                    self.virtual_iface_bindings
                                        .get(&(*this_h, prop.to_string()))
                                        .cloned(),
                                );
                            if let Some((bound_name, modport_opt)) = bound {
                                // LRM §25.4 — warn (don't reject) when
                                // writing to a member declared `input` by
                                // the property's modport view. We do not
                                // block the write because the user may have
                                // a deliberate test pattern; downstream the
                                // member will still drive whatever signal
                                // `<bound>.<member>` names.
                                if let Some(mp_name) = &modport_opt {
                                    // Locate the iface type from the class's
                                    // virtual_iface_properties record.
                                    let iface_t = cls_name
                                        .as_ref()
                                        .and_then(|cn| self.class_vif_decl(cn, prop))
                                        .map(|(_, (t, _))| t.clone());
                                    if let Some(iface_t) = iface_t {
                                        if let Some(dirs) = self
                                            .module
                                            .modport_member_dirs
                                            .get(&(iface_t.clone(), mp_name.clone()))
                                        {
                                            if matches!(
                                                dirs.get(&member.name),
                                                Some(crate::ast::types::PortDirection::Input)
                                            ) {
                                                eprintln!(
                                                    "[modport] virtual {}.{} write to input member '{}' (LRM §25.4)",
                                                    iface_t, mp_name, member.name
                                                );
                                            }
                                        }
                                    }
                                }
                                let target = format!("{}.{}", bound_name, member.name);
                                let prev = self.get_signal_value_by_name(&target);
                                let w = prev.as_ref().map(|v| v.width).unwrap_or(val.width);
                                let resized = val.resize(w);
                                let changed = prev.as_ref() != Some(&resized);
                                self.set_signal_value_by_name(&target, resized);
                                return changed;
                            }
                        }
                    }
                }
                // `ClassName#(params)::static_prop = ...` — a STATIC scalar /
                // handle property write through a class SPECIALIZATION
                // receiver (the READ path resolves it per-spec via
                // class_static_get; the write must land in the SAME per-spec
                // cell). Without this `C#(int)::n = 5` fell through to a
                // bare-name store the later read never consulted.
                if let ExprKind::Specialization { .. } = &expr.kind {
                    if let Some((base, sig)) = self
                        .resolve_call_spec_params(Self::extract_call_spec(expr), &self.current_spec)
                    {
                        if self.module.classes.contains_key(&base)
                            && !self.signal_name_to_id.contains_key(base.as_str())
                        {
                            let saved = self.current_spec.take();
                            self.current_spec = Some((base.clone(), sig));
                            let ok = self.class_static_set(&base, &member.name, val.clone());
                            self.current_spec = saved;
                            if ok {
                                return true;
                            }
                        }
                    }
                }
                // `ClassName::static_prop = ...` — explicit static write.
                if let ExprKind::Ident(hier) = &expr.kind {
                    if hier.path.len() == 1 {
                        let cls = hier.path[0].name.name.clone();
                        if self.module.classes.contains_key(&cls)
                            && !self
                                .local_stack
                                .last()
                                .is_some_and(|m| m.contains_key(&cls))
                            && !self.signal_name_to_id.contains_key(cls.as_str())
                            && !self.signals.contains_key(&cls)
                            && self.class_static_set(&cls, &member.name, val.clone())
                        {
                            return true;
                        }
                        // §8.25/§6.18: through a typedef of a specialization.
                        if !self.module.classes.contains_key(&cls)
                            && !self
                                .local_stack
                                .last()
                                .is_some_and(|m| m.contains_key(&cls))
                            && !self.signal_name_to_id.contains_key(cls.as_str())
                            && !self.signals.contains_key(&cls)
                            && let Some((base, sig)) = self.scope_prefix_spec(&cls)
                        {
                            let saved = self.current_spec.replace((base.clone(), sig));
                            let ok = self.class_static_set(&base, &member.name, val.clone());
                            self.current_spec = saved;
                            if ok {
                                return true;
                            }
                        }
                    }
                }
                // A procedural-local unpacked struct whose members were
                // materialised individually — by the dotted `Ident([s,a])`
                // lvalue arm above, or by foreach/randomize element
                // construction — lives in the runtime `signals` map as
                // standalone dotted keys (`s.a`, `s.b`), NOT as a single
                // packed whole. The whole-struct splice arm below reads the
                // (empty) whole value, splices the field into it, and writes
                // a phantom whole `s` that no member read ever consults — so
                // the actual `s.a` leaf stays at its pre-call value.
                //
                // When the dotted leaf already exists as its own key, write
                // it directly. A struct stored whole (case 1: `output`
                // formal whose actual was never pre-touched) has no such key
                // and correctly falls through to the whole-struct arm.
                if let Some(flat) = self.flat_member_name(lhs) {
                    if flat.contains('.') && self.signals.contains_key(flat.as_str()) {
                        let prev = self.signals.get(flat.as_str()).cloned();
                        let fitted = match prev.as_ref() {
                            Some(p) if p.is_real && !val.is_real => Value::from_f64(val.to_f64()),
                            Some(p) if !p.is_real && !val.is_real => val.resize(p.width),
                            _ => val.clone(),
                        };
                        let changed = prev.as_ref() != Some(&fitted);
                        self.signals.insert(flat, fitted);
                        return changed;
                    }
                }
                // Struct variable member write: `result.field1 = ...`
                // (BUT NOT a member of a CLASS handle: a forward-declared-
                // class formal like UVM's `uvm_reg_item rw` is recorded in
                // `var_class_types`, yet a stale same-named struct local from
                // a CALLER scope (the predictor's `uvm_reg_bus_op rw`) stays
                // in the flat `var_typedef_types`, so this arm misread the
                // class handle as a struct and spliced `status` over its low
                // bits, zeroing the heap handle inside `uvm_reg::do_predict`.
                // Never struct-splice a class variable — its members live in
                // the heap, not a packed local.
                if let ExprKind::Ident(hier) = &expr.kind {
                    if hier.path.len() == 1 {
                        let base_name = hier.path[0].name.name.as_str();
                        if self.class_of_var(base_name).is_none() {
                            let type_name = self
                                .var_typedef_types
                                .get(base_name)
                                .cloned()
                                .or_else(|| self.get_expr_type_name(expr));
                            let dt: Option<DataType> = type_name
                                .as_deref()
                                .and_then(|tn| self.module.typedef_types.get(tn).cloned());
                            if let Some(dt) = dt {
                                let resolved =
                                    Self::resolve_type_ref(&dt, &self.module.typedef_types);
                                if let Some(fields) = Self::struct_field_layout(&resolved) {
                                    if let Some((_, off, w, _real)) = fields
                                        .iter()
                                        .find(|(m, _, _, _)| m == &member.name)
                                        .cloned()
                                    {
                                        let mut cur = self
                                            .get_local_or_signal(base_name)
                                            .unwrap_or_else(|| Value::zero(off + w));
                                        if (off + w) as usize > cur.width as usize {
                                            cur = cur.resize(off + w);
                                        }
                                        let piece = val.resize(w);
                                        for i in 0..w {
                                            let bit = piece.get_bit(i as usize);
                                            cur.set_bit((off + i) as usize, bit);
                                        }
                                        self.set_local_or_signal(base_name, cur);
                                        return true;
                                    }
                                }
                            }
                        }
                    }
                }
                // packed_struct_fields for signal-based structs
                if let ExprKind::Ident(hier) = &expr.kind {
                    let name = self.resolve_hier_name(hier);
                    if let Some(fields) = self.module.packed_struct_fields.get(&*name).cloned() {
                        if let Some((_, off, w)) =
                            fields.iter().find(|(m, _, _)| m == &member.name).cloned()
                        {
                            if let Some(cur_sig) = self.get_signal_value_by_name(&name) {
                                let total_w = cur_sig.width;
                                let mut cur = cur_sig.resize(total_w);
                                let piece = val.resize(w);
                                for i in 0..w {
                                    let bit = piece.get_bit(i as usize);
                                    cur.set_bit((off + i) as usize, bit);
                                }
                                let prev = self.get_signal_value_by_name(&name);
                                let changed = prev.as_ref() != Some(&cur);
                                self.set_signal_value_by_name(&name, cur);
                                return changed;
                            }
                        }
                    }
                }
                // `arr[idx...].outer.inner...field = val` — N-dim arrays of
                // packed struct with chained MemberAccess. Walks both the
                // MemberAccess chain (collecting dotted field-path:
                // `outer.inner...field`) AND the Index chain (collecting
                // indices into `arr[i][j]...[k]`). Then looks up the dotted
                // field path in `packed_struct_fields[arr_name]` — the
                // elaborator's `flatten_subfields` already registers nested
                // fields under their full dotted path.
                {
                    let mut field_path: Vec<String> = vec![member.name.clone()];
                    let mut cur_expr: &Expression = expr;
                    // Walk inward through MemberAccess nodes; the LAST one
                    // reaches either Index (array form) or Ident (plain
                    // struct, handled by the Ident branch below).
                    loop {
                        match &cur_expr.kind {
                            ExprKind::MemberAccess { expr: b, member: m } => {
                                field_path.push(m.name.clone());
                                cur_expr = b.as_ref();
                            }
                            _ => break,
                        }
                    }
                    field_path.reverse();
                    let mut indices: Vec<u64> = Vec::new();
                    let root_name: Option<String> = loop {
                        match &cur_expr.kind {
                            ExprKind::Index { expr: b, index } => {
                                indices.push(self.eval_expr(index).to_u64().unwrap_or(0));
                                cur_expr = b.as_ref();
                            }
                            // A procedural LOCAL registers its layout under
                            // its BARE name, but inside a submodule process
                            // `resolve_hier_name` scopes a bare ident
                            // (`u.a`) — so `a[0].p = v` on a task-local
                            // packed array of structs found no layout and
                            // the element read back x, while the same code
                            // in the top module (empty scope) worked.
                            ExprKind::Ident(h) => {
                                let scoped = self.resolve_hier_name(h);
                                if !self.module.packed_struct_fields.contains_key(&*scoped)
                                    && h.path.len() == 1
                                    && h.path[0].selects.is_empty()
                                    && self
                                        .module
                                        .packed_struct_fields
                                        .contains_key(&h.path[0].name.name)
                                {
                                    break Some(h.path[0].name.name.clone());
                                }
                                break Some(scoped.to_string());
                            }
                            // §7.4.2: root may itself be a struct-member path
                            // (`main.sub_list[0].f`) — build the dotted name so
                            // the `packed_struct_fields["main.sub_list"]` layout
                            // registered at elaboration resolves it.
                            ExprKind::MemberAccess { .. } => {
                                let mut segs: Vec<String> = Vec::new();
                                let mut c = cur_expr;
                                while let ExprKind::MemberAccess { expr: b, member: m } = &c.kind {
                                    segs.push(m.name.clone());
                                    c = b.as_ref();
                                }
                                if let ExprKind::Ident(h) = &c.kind {
                                    segs.push(self.resolve_hier_name(h).into_owned());
                                    segs.reverse();
                                    break Some(segs.join("."));
                                }
                                break None;
                            }
                            _ => break None,
                        }
                    };
                    if let Some(arr_name) = root_name {
                        if !indices.is_empty() {
                            indices.reverse();
                            if let Some(fields) =
                                self.module.packed_struct_fields.get(&arr_name).cloned()
                            {
                                let field_key = field_path.join(".");
                                if let Some((_, off, w)) =
                                    fields.iter().find(|(m, _, _)| m == &field_key).cloned()
                                {
                                    let mut elem_name = arr_name.clone();
                                    for i in &indices {
                                        elem_name.push_str(&format!("[{}]", i));
                                    }
                                    if let Some(cur_sig) = self.get_signal_value_by_name(&elem_name)
                                    {
                                        let total_w = cur_sig.width;
                                        let mut cur = cur_sig.resize(total_w);
                                        let piece = val.resize(w);
                                        for i in 0..w {
                                            let bit = piece.get_bit(i as usize);
                                            cur.set_bit((off + i) as usize, bit);
                                        }
                                        let prev = self.get_signal_value_by_name(&elem_name);
                                        let changed = prev.as_ref() != Some(&cur);
                                        self.set_signal_value_by_name(&elem_name, cur);
                                        return changed;
                                    }
                                    // §7.4.2: PACKED array of packed struct —
                                    // `foo[i]` is NOT a separate signal but a bit
                                    // slice of the single backing vector. Splice
                                    // the field at `i*elem_w + off`.
                                    {
                                        // Struct width from the field layout, and
                                        // the slot flattened over ALL declared
                                        // dims — the old form handled a single
                                        // index with `packed_signal_elem_widths`,
                                        // which is the OUTERMOST element width, so
                                        // `t [0:0][1:0]` (two dims) both failed the
                                        // arity check and would have used 2x the
                                        // struct width had it passed.
                                        let struct_w =
                                            fields.iter().map(|(_, o, w)| o + w).max().unwrap_or(0);
                                        let dims =
                                            self.module.packed_full_dims.get(&arr_name).cloned();
                                        if let Some(slot) =
                                            Self::flatten_packed_slot(&indices, dims.as_deref())
                                        {
                                            // Frame-aware: a task/function local
                                            // lives in the call frame, not the
                                            // signal table (`get_local_or_signal`
                                            // covers both).
                                            if let Some(cur_sig) =
                                                self.get_local_or_signal(&arr_name)
                                            {
                                                let total_w = cur_sig.width;
                                                let mut cur = cur_sig.resize(total_w);
                                                let abs_off = slot * struct_w + off;
                                                let piece = val.resize(w);
                                                for i in 0..w {
                                                    cur.set_bit(
                                                        (abs_off + i) as usize,
                                                        piece.get_bit(i as usize),
                                                    );
                                                }
                                                let changed = cur != cur_sig;
                                                self.set_local_or_signal(&arr_name, cur);
                                                return changed;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                // Mirror writes to the unpacked-struct member signal
                // (e.g. `t_unpacked_struct.val_i`) so VPI-forced values
                // and SV reads stay in sync. Both the heap property and the
                // separate signal are written; the read path prefers the
                // separate signal.
                if let ExprKind::Ident(hier) = &expr.kind {
                    if hier.path.len() == 1 {
                        let dotted = format!("{}.{}", hier.path[0].name.name, member.name);
                        if let Some(id) = self.signal_name_to_id.get(dotted.as_str()).copied() {
                            let width = self.signal_widths[id];
                            let mut resized = if self.signal_real[id] {
                                if val.is_real {
                                    val.clone()
                                } else {
                                    Value::from_f64(val.to_f64())
                                }
                            } else {
                                if val.is_real {
                                    Self::real_to_int(val.to_f64(), width)
                                } else {
                                    val.resize(width)
                                }
                            };
                            resized.is_signed = self.signal_signed[id];
                            self.set_signal_value_by_name(&dotted, resized);
                            // Fall through to also update the heap so existing
                            // read paths that look there stay consistent.
                        }
                    }
                }
                // id-collision-in-heap: this last-resort dereference may
                // reinterpret the receiver's integral VALUE as a handle only
                // when the receiver statically names a class-object lvalue
                // (`obj.prop`, `trk.cfg.mode`, `ch[i].prop`). A struct
                // variable, a struct-element collection, a hierarchical ref,
                // or a `ref` formal of struct type splices plain data (§7.8) —
                // `r.delay_counter = v` on such a receiver used to land in
                // `heap[1].delay_counter` instead. Exotic receivers (method
                // results) keep the legacy value-based attempt, which still
                // requires a live object declaring the property.
                if self.receiver_may_be_handle(expr) {
                    let base = self.eval_expr(expr);
                    let handle = base.to_u64().unwrap_or(0) as usize;
                    if handle != 0 && handle < self.heap.len() && self.heap[handle].is_some() {
                        // §8.9: a static reached through a handle chain
                        // (`a.b.s = v`) is the class's shared cell.
                        if let Some(ok) = self.static_write_via_handle(
                            handle,
                            None,
                            None,
                            &member.name,
                            val.clone(),
                        ) {
                            return ok;
                        }
                        // §8.x: a class property assignment truncates/sign-
                        // extends the rvalue to the property's declared type
                        // (`byte a; a = 'hfff;` ⇒ 8-bit 0xff, read -1 when
                        // signed). fit_class_prop clamps the width;
                        // set_prop_if_changed stamps the declared signedness.
                        let fitted = self.fit_class_prop(handle, &member.name, val);
                        return self.set_prop_if_changed(handle, &member.name, fitted);
                    }
                }
                false
            }
            _ => false,
        }
    }
}
