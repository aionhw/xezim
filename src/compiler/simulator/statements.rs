//! Simulator statements.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    /// §10.10/§11.4.12: the elements an unpacked-array concatenation's
    /// operands contribute, in order — a queue / array operand (bare, an
    /// object member, a struct member reached through any selects, an
    /// associative element that is itself a collection) or a slice of one
    /// contributes all of its elements; anything else is one element.
    #[inline(never)]
    pub(super) fn unpacked_concat_operand_values(&mut self, items: Vec<&Expression>) -> Vec<Value> {
        let mut all_vals: Vec<Value> = Vec::new();
        for expr in items {
            // A queue/array operand — bare (`q`), an instance-
            // scoped member (`obj.q`, `arr[i].q`), or a nested
            // assoc-of-queue element (`assoc[key]`) —
            // contributes each of its elements in order.
            let qn = self.array_operand_name(expr).or_else(|| {
                self.nested_index_name(expr)
                    .filter(|cn| self.module.dynamic_arrays.contains(cn))
            });
            if let Some(ename) = qn {
                let esize = self.get_queue_size(&ename) as usize;
                for j in 0..esize {
                    if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", ename, j)) {
                        all_vals.push(v);
                    }
                }
                continue;
            }
            // Slice of a queue: q[a:b]
            if let ExprKind::RangeSelect {
                expr: rbase,
                left,
                right,
                ..
            } = &expr.kind
            {
                if let ExprKind::Ident(rhier) = &rbase.kind {
                    let mut rname = self.resolve_hier_name(rhier);
                    // Instance-scope a member-queue slice
                    // operand inside a concat (`{q[0:i], …}`).
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
                        if l <= r {
                            let lo = l.max(r_lo_a);
                            let hi = r.min(r_upper);
                            if hi >= lo {
                                for idx in lo..=hi {
                                    if let Some(v) = self
                                        .get_signal_value_by_name(&format!("{}[{}]", rname, idx))
                                    {
                                        all_vals.push(v);
                                    }
                                }
                            }
                        }
                        continue;
                    }
                }
            }
            all_vals.push(self.eval_expr(expr));
        }
        all_vals
    }

    /// The size and optional source of a §7.5.1 dynamic-array constructor:
    /// `new[n]` parses as `Call{Ident(new), [n]}` and `new[n](src)` as
    /// `Call{Call{Ident(new), [n]}, [src]}`. `None` for any other expression.
    pub(super) fn sized_new_parts(
        rvalue: &Expression,
    ) -> Option<(&Expression, Option<&Expression>)> {
        let is_new = |e: &Expression| {
            matches!(&e.kind,
                ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].name.name == "new")
        };
        match &rvalue.kind {
            ExprKind::Call { func, args } if is_new(func) && !args.is_empty() => {
                Some((&args[0], None))
            }
            ExprKind::Call { func, args } => match &func.kind {
                ExprKind::Call {
                    func: inner,
                    args: iargs,
                } if is_new(inner) && !iargs.is_empty() => Some((&iargs[0], args.first())),
                _ => None,
            },
            _ => None,
        }
    }

    /// Leaf width of the class member `<h>#d` declared with dynamic or
    /// queue rows (`T d[][]`, `T d[][$]`), when `name` is that member.
    pub(super) fn class_dyn_row_width(&self, name: &str) -> Option<u32> {
        let (h, prop) = name.split_once('#')?;
        if prop.contains('[') || prop.contains('.') {
            return None;
        }
        let handle = h.parse::<usize>().ok()?;
        let cd = self.class_prop_declarer(handle, prop)?;
        let tail = cd.dynamic_tail_properties.get(prop)?;
        matches!(
            tail.first(),
            Some(
                crate::ast::types::UnpackedDimension::Unsized(_)
                    | crate::ast::types::UnpackedDimension::Queue { .. }
            )
        )
        .then(|| {
            self.spec_member_elem_width(&cd, handle, prop)
                .or_else(|| cd.queue_properties.get(prop).map(|&(w, _)| w))
                .unwrap_or(32)
                .max(1)
        })
    }

    /// Drop every element of the row collection `row` (`<h>#d[i]`).
    pub(super) fn clear_dyn_row(&mut self, row: &str) {
        for key in self.signals.keys_with_elem_prefix(&format!("{}[", row)) {
            self.signals.remove(&key);
        }
    }

    /// The declared associative tail below a dynamic outer array, scoped to
    /// a local, object property, or unpacked-struct member (§7.4.5).
    #[cold]
    pub(super) fn dynamic_assoc_decl(
        &self,
        name: &str,
    ) -> Option<(DataType, Vec<crate::ast::types::UnpackedDimension>)> {
        if let Some(tail) = self.dynamic_assoc_dims.get(name) {
            let dt = self
                .module
                .var_decl_types
                .get(name)
                .cloned()
                .or_else(|| self.class_member_coll_type(name).cloned())?;
            return Some((dt, tail.clone()));
        }
        if let Some((h, prop)) = name.split_once('#')
            && let Ok(handle) = h.parse::<usize>()
            && let Some(cd) = self.class_prop_declarer(handle, prop)
            && let Some(tail) = cd.dynamic_assoc_properties.get(prop)
        {
            return Some((cd.property_types.get(prop)?.clone(), tail.clone()));
        }
        if !name.contains('.') {
            return None;
        }
        let (dt, dims, selected) = self.struct_member_path_decl(name)?;
        (selected == 0
            && matches!(
                dims.first(),
                Some(crate::ast::types::UnpackedDimension::Unsized(_))
            )
            && dims.len() > 1
            && dims[1..]
                .iter()
                .all(|d| matches!(d, crate::ast::types::UnpackedDimension::Associative { .. })))
        .then(|| (dt, dims[1..].to_vec()))
    }

    /// Register a row's declared key type; metadata alone creates no AA entry.
    #[cold]
    pub(super) fn register_dynamic_assoc_tail(
        &mut self,
        name: &str,
        dt: &DataType,
        tail: &[crate::ast::types::UnpackedDimension],
    ) {
        let Some(crate::ast::types::UnpackedDimension::Associative { data_type, .. }) =
            tail.first()
        else {
            return;
        };
        let w = resolve_type_width(
            dt,
            Some(&self.module.parameters),
            Some(&self.module.typedefs),
        )
        .max(1);
        self.register_coll_name(
            name,
            StructCollKind::Assoc,
            w,
            self.string_keyed_dim(&tail[0]),
        );
        let (index, key_type) = super::elaborate::assoc_key_type_info(
            data_type.as_deref(),
            &self.module.parameters,
            &self.module.typedefs,
            &self.module.typedef_types,
        );
        if let Some(index) = index {
            self.module
                .assoc_index_widths
                .insert(name.to_string(), index);
        }
        if let Some(key_type) = key_type {
            self.module
                .assoc_key_type_names
                .insert(name.to_string(), key_type);
        }
        self.module
            .var_decl_types
            .insert(name.to_string(), dt.clone());
        self.dynamic_assoc_dims
            .insert(name.to_string(), tail.to_vec());
        if let Some(levels) = super::elaborate::assoc_inner_string_levels(tail) {
            self.module
                .assoc_inner_string_keys
                .insert(name.to_string(), levels);
        }
    }

    /// Discard rows outside the retained prefix, including every nested key
    /// and its runtime metadata. Reallocation must not resurrect old entries.
    #[cold]
    pub(super) fn clear_dynamic_assoc_rows(&mut self, root: &str, keep: u64) {
        let prefix = format!("{}[", root);
        let discard = |key: &str| {
            key.strip_prefix(&prefix)
                .and_then(|rest| rest.split(']').next())
                .and_then(|i| i.parse::<u64>().ok())
                .is_some_and(|i| i >= keep)
        };
        for key in self.signals.keys_with_elem_prefix(&prefix) {
            if discard(&key) {
                // AA leaves are created in the runtime map, never allocated
                // compact-table signal IDs; deleting the map entry is final.
                debug_assert!(!self.signal_name_to_id.contains_key(key.as_str()));
                self.signals.remove(&key);
                self.widths.remove(&key);
            }
        }
        self.module
            .associative_arrays
            .retain(|key, _| !discard(key));
        self.module
            .assoc_index_widths
            .retain(|key, _| !discard(key));
        self.module
            .assoc_key_type_names
            .retain(|key, _| !discard(key));
        self.module
            .assoc_inner_string_keys
            .retain(|key, _| !discard(key));
        self.module.assoc_elem_widths.retain(|key, _| !discard(key));
        self.module.var_decl_types.retain(|key, _| !discard(key));
        self.dynamic_assoc_dims.retain(|key, _| !discard(key));
    }

    /// §7.5.1: size the dynamic array stored at `name` to `new[n_expr]`,
    /// copying min(n, src.size()) elements from `src_expr` (`new[n](src)`)
    /// and giving the rest the element type's default.
    #[inline(never)]
    pub(super) fn fill_sized_new(
        &mut self,
        name: &str,
        n_expr: &Expression,
        src_expr: Option<&Expression>,
    ) {
        let n = self.eval_expr(n_expr).to_u64().unwrap_or(0);
        // The source may be any array or queue. A self-copy
        // (`d = new[n](d)`) is index-preserving, so no
        // snapshot is needed.
        let src = src_expr.and_then(|e| match &e.kind {
            ExprKind::Ident(h) => {
                // Resolve flattened object members before the
                // hierarchy resolver adds a module/local prefix.
                if h.path.len() > 1
                    && let Some(storage) = self.expr_assoc_name(e)
                {
                    return Some(storage);
                }
                let n = self.resolve_hier_name(h);
                // A class-member source (`new[t+1](slices)`
                // in a method) lives at `<h>#slices` — the
                // bare name read size 0, so the copy-init
                // silently dropped every existing element.
                Some(self.resolve_locator_storage(&n))
            }
            // `obj.member` and indexed/call-bearing owners are
            // collection operands, not scalar broadcast values.
            ExprKind::MemberAccess { .. } => self.expr_assoc_name(e),
            _ => None,
        });
        if let Some((dt, tail)) = self.dynamic_assoc_decl(name) {
            self.module
                .var_decl_types
                .insert(name.to_string(), dt.clone());
            self.dynamic_assoc_dims
                .insert(name.to_string(), tail.clone());
            let keep = src
                .as_deref()
                .map_or(0, |sn| self.get_queue_size(sn).min(n));
            // A self-resize keeps its surviving rows. Every other constructor
            // replaces the destination, including nested associative entries.
            self.clear_dynamic_assoc_rows(
                name,
                if src.as_deref() == Some(name) {
                    keep
                } else {
                    0
                },
            );
            for i in 0..n {
                let row = format!("{}[{}]", name, i);
                self.register_dynamic_assoc_tail(&row, &dt, &tail);
                if i < keep && src.as_deref() != Some(name) {
                    self.queue_copy_elem(src.as_deref().unwrap(), i, name, i);
                }
            }
            self.set_queue_size(name, n);
            return;
        }
        // §7.4.5: a class member `T d[][]` (dynamic rows): each element is
        // a collection of its own, empty until `d[i] = new[m]` sizes it.
        if let Some(w) = self.class_dyn_row_width(name) {
            let self_copy = src.as_deref() == Some(name);
            let keep = src
                .as_deref()
                .map_or(0, |sn| self.get_queue_size(sn).min(n));
            for i in if self_copy { keep } else { 0 }..n {
                let row = format!("{}[{}]", name, i);
                self.clear_dyn_row(&row);
                self.module.dynamic_arrays.insert(row.clone());
                self.module.arrays.insert(row.clone(), (0, 63, w));
                self.set_queue_size(&row, 0);
            }
            // §7.5.1: each kept row of another array is copied with its size
            // and elements; every row was cleared (#595).
            if let Some(sn) = src.as_deref().filter(|_| !self_copy) {
                for i in 0..keep {
                    let (drow, srow) = (format!("{name}[{i}]"), format!("{sn}[{i}]"));
                    self.copy_dyn_tree(&drow, &srow);
                }
            }
            self.set_queue_size(name, n);
            return;
        }
        let keep = match &src {
            Some(sn) => self.get_queue_size(sn).min(n),
            None => 0,
        };
        if let Some(sn) = &src {
            for i in 0..keep {
                // §7.5.1: a row that is itself a dynamic array or queue
                // (`int d[][]`) is copied with its size and elements; read as
                // one value it was x (#595).
                let srow = format!("{sn}[{i}]");
                if sn.as_str() != name && self.module.dynamic_arrays.contains(&srow) {
                    let drow = format!("{name}[{i}]");
                    self.register_dyn_row_like(&drow, &srow);
                    self.copy_dyn_tree(&drow, &srow);
                } else {
                    self.queue_copy_elem(sn, i, &name, i);
                }
            }
        }
        // `T a[][16]`: each slot is itself a fixed array
        // whose elements live at `a[i][j]` — default-init
        // those, not a phantom scalar `a[i]`.
        let inner: Option<(Vec<(i64, i64)>, u32)> = self
            .module
            .arrays_2d
            .get(&*name)
            .map(|&(_, d2, w)| (vec![d2], w))
            .or_else(|| {
                self.module
                    .arrays_nd
                    .get(&*name)
                    .map(|(sh, w)| (sh[1..].to_vec(), *w))
            });
        if let Some((shape, w)) = inner {
            let mut suffixes: Vec<String> = vec![String::new()];
            for &(lo, hi) in &shape {
                let mut next = Vec::new();
                for s in &suffixes {
                    for k in lo..=hi {
                        next.push(format!("{}[{}]", s, k));
                    }
                }
                suffixes = next;
            }
            // §7.5.1: the element type's default (x when 4-state).
            let four_state = self.coll_elem_starts_x(name);
            for i in keep..n {
                for s in &suffixes {
                    let v = if four_state {
                        Value::new(w)
                    } else {
                        Value::zero(w)
                    };
                    self.set_signal_value_by_name(&format!("{}[{}]{}", name, i, s), v);
                }
            }
        } else {
            // §7.5.1: new elements take the element
            // type's DEFAULT — x for a 4-state type
            // (logic/reg/integer/time), 0 otherwise.
            // Everything x-init'd as 0 before.
            let w = self
                .module
                .arrays
                .get(&*name)
                .map(|&(_, _, w)| w)
                .unwrap_or(32);
            // A class member (`<handle>#da`) has its type on the class.
            let four_state = self.coll_elem_starts_x(name);
            let fill = if four_state {
                Value::new(w)
            } else {
                Value::zero(w)
            };
            // A STRUCT-element array keeps each member in
            // its own leaf (`arr[i].path`) — seeding a
            // packed container instead made later member
            // reads slice that zero container while
            // member writes went to the leaves, so
            // `slices[t].path` read back blank (UVM's
            // uvm_hdl_path_concat::add_path).
            let elem_su = self.queue_elem_struct(&name);
            for i in keep..n {
                let elem = format!("{}[{}]", name, i);
                match &elem_su {
                    Some(su) => {
                        for (k, lw, is_real) in self.unpacked_struct_leaf_keys(&elem, su) {
                            let sv = if is_real {
                                Value::from_f64(0.0)
                            } else {
                                Value::zero(lw)
                            };
                            self.signals.insert(k, sv);
                        }
                        // §6.8: a 4-state member leaf starts at x.
                        self.seed_struct_elem_cells(&elem, su);
                    }
                    None => {
                        self.set_signal_value_by_name(&elem, fill.clone());
                    }
                }
            }
        }
        // §7.5.1: `new[n]('{...})` — the argument may be an
        // assignment pattern (array literal), not just a
        // named source array. Populate elements from it,
        // mirroring a plain `arr = '{...}` pattern assign.
        if let Some(se) = src_expr
            && src.is_none()
        {
            match &se.kind {
                ExprKind::AssignmentPattern(items) => {
                    for (i, item) in items.iter().enumerate() {
                        if (i as u64) >= n {
                            break;
                        }
                        let v = self.eval_expr(item.expr());
                        self.set_signal_value_by_name(&format!("{}[{}]", name, i), v);
                    }
                }
                // A non-array scalar argument (`new[n](3.0)`,
                // `new[n]("A")`) broadcasts to every element
                // (matches a reference simulator). Ident sources were already
                // element-copied above.
                ExprKind::Ident(_) => {}
                _ => {
                    let v = self.eval_expr(se);
                    for i in 0..n {
                        self.set_signal_value_by_name(&format!("{}[{}]", name, i), v.clone());
                    }
                }
            }
        }
        self.set_queue_size(&name, n);
    }

    /// Outlined from `exec_statement` so the dispatcher's stack frame
    /// stays small: this arm's locals inflated every call's frame (the
    /// dispatcher recursed with a 7.8 KB frame per level).
    #[inline(never)]
    /// A statement-coverage marker inserted by `--code-coverage`.
    fn is_cov_marker(s: &Statement) -> bool {
        matches!(&s.kind, StatementKind::Expr(e)
            if matches!(&e.kind, ExprKind::SystemCall { name, .. } if name == super::COV_HIT_TASK))
    }

    pub(super) fn exec_stmt_foreach(
        &mut self,
        stmt: &Statement,
        array: &Expression,
        vars: &[Option<crate::ast::Identifier>],
        body: &Statement,
    ) {
        // Count executed bodies, including continue/break/return iterations
        // (§12.7.3, §12.8), across every collection shape. Publish heat once
        // after the inner runner has restored loop and method scopes.
        let mut iterations = 0;
        self.exec_stmt_foreach_inner(stmt, array, vars, body, &mut iterations);
        self.record_ast_loop_heat(iterations);
    }

    /// A selected row of a fixed array of collections has its own store.
    /// Resolve the declaration before evaluating selectors, so a failed
    /// probe cannot repeat an index expression's side effects.
    #[cold]
    #[inline(never)]
    fn foreach_selected_collection(&mut self, array: &Expression) -> Option<String> {
        let selected = match &array.kind {
            ExprKind::Ident(h) if h.path.iter().any(|p| !p.selects.is_empty()) => {
                std::borrow::Cow::Owned(Self::hier_select_chain(h, array.span))
            }
            ExprKind::Index { .. } => std::borrow::Cow::Borrowed(array),
            _ => return None,
        };
        if let Some(name) = self.class_array_of_coll_key(&selected) {
            return Some(name);
        }
        let mut indices = Vec::new();
        let mut base = selected.as_ref();
        while let ExprKind::Index { expr, index } = &base.kind {
            indices.push(index.as_ref());
            base = expr;
        }
        let ExprKind::Ident(h) = &base.kind else {
            return None;
        };
        let mut name = self.resolve_hier_name(h).into_owned();
        if let Some(local) = self.dyn_name_lookup(&name) {
            name = local.to_string();
        }
        // §7.8/§12.7.3: `foreach (aq[k][j])` over a row of an associative
        // array of collections walks that row (`aq[k]`), not `aq`'s keys.
        if indices.len() == 1 && self.is_associative_array(&name) {
            let kv = self.eval_expr(indices[0]);
            let row = format!("{}[{}]", name, self.assoc_key_str(&name, &kv));
            return (self.module.dynamic_arrays.contains(&row) || self.is_associative_array(&row))
                .then_some(row);
        }
        if self.foreach_dims(&name).is_none()
            && let Some(scoped) = self.hint_scoped(&name)
        {
            name = scoped;
        }
        let dims = self.foreach_dims(&name)?;
        if dims.len() != indices.len() {
            return None;
        }
        let mut first = name.clone();
        for &(lo, _) in &dims {
            first.push_str(&format!("[{lo}]"));
        }
        if !self.module.associative_arrays.contains_key(&first)
            && !self.module.dynamic_arrays.contains(&first)
        {
            return None;
        }
        for index in indices.into_iter().rev() {
            let value = self.eval_expr(index).to_i64().unwrap_or(0);
            name.push_str(&format!("[{value}]"));
        }
        Some(name)
    }

    /// §12.7.3: a loop variable runs from its dimension's LEFT bound to its
    /// RIGHT bound. The per-dimension "declared descending" flags of the
    /// fixed array `name` (`int d [1:2][3:1]` gives `[false, true]`); the
    /// shape tables normalize every range to (lo, hi).
    pub(super) fn foreach_unpacked_descs(&self, name: &str, ndims: usize) -> Vec<bool> {
        let mut descs: Vec<bool> = self
            .module
            .unpacked_decl_dims
            .get(name)
            .map(|d| d.iter().map(|&(l, r)| l > r).collect())
            .unwrap_or_default();
        descs.resize(ndims, false);
        descs
    }

    /// The iteration shape of `foreach (a[i]...[k][vars])` over a FIXED
    /// unpacked array `a`: the dimensions after the `k` selected ones, then
    /// the element's packed dimensions. None for any other collection, a
    /// range select, or a row that is itself a queue, dynamic or associative
    /// array (those have their own stores; see `foreach_selected_collection`).
    fn foreach_fixed_subarray_dims(
        &self,
        array: &Expression,
        nvars: usize,
    ) -> Option<(Vec<(i64, i64)>, Vec<bool>)> {
        let is_range =
            |e: &Expression| matches!(e.kind, ExprKind::Range(..) | ExprKind::RangeSelect { .. });
        let mut base = array;
        let mut k = 0usize;
        while let ExprKind::Index { expr, index } = &base.kind {
            if is_range(index) {
                return None;
            }
            k += 1;
            base = expr;
        }
        let ExprKind::Ident(h) = &base.kind else {
            return None;
        };
        if h.path.len() != 1 || h.path[0].selects.iter().any(is_range) {
            return None;
        }
        k += h.path[0].selects.len();
        if k == 0 || nvars == 0 {
            return None;
        }
        let mut name = h.path[0].name.name.clone();
        if let Some(local) = self.dyn_name_lookup(&name) {
            name = local.to_string();
        }
        if self.foreach_dims(&name).is_none()
            && let Some(scoped) = self.hint_scoped(&name)
        {
            name = scoped;
        }
        if self.module.dynamic_arrays.contains(&name) || self.is_associative_array(&name) {
            return None;
        }
        let dims = self.foreach_dims(&name)?;
        if dims.len() < k {
            return None;
        }
        // A row (or element) that is itself a collection.
        let mut first = name.clone();
        for &(lo, _) in &dims {
            first.push_str(&format!("[{lo}]"));
        }
        if self.module.dynamic_arrays.contains(&first)
            || self.module.associative_arrays.contains_key(&first)
            || self.signals.contains_key(&format!("{first}.size"))
        {
            return None;
        }
        let mut rest: Vec<(i64, i64)> = dims[k..].to_vec();
        let mut descs = self.foreach_unpacked_descs(&name, dims.len())[k..].to_vec();
        if rest.len() < nvars {
            if let Some(pdims) = self.module.packed_full_dims.get(&name) {
                for &(l, r) in pdims {
                    if rest.len() >= nvars {
                        break;
                    }
                    rest.push((l.min(r), l.max(r)));
                    descs.push(l > r);
                }
            } else if rest.len() + 1 == nvars {
                // A lone packed dimension the packed tables do not record
                // (`logic [3:0] b [2]`): the element width, `[w-1:0]` unless
                // declared ascending.
                let ew = self
                    .module
                    .arrays
                    .get(&name)
                    .map(|&(_, _, w)| w)
                    .or_else(|| self.module.arrays_2d.get(&name).map(|&(_, _, w)| w))
                    .or_else(|| self.module.arrays_nd.get(&name).map(|(_, w)| *w));
                if let Some(w) = ew.filter(|&w| w > 1) {
                    rest.push((0, w as i64 - 1));
                    descs.push(!self.module.ascending_packed.contains_key(&name));
                }
            }
        }
        if rest.len() < nvars {
            return None;
        }
        rest.truncate(nvars);
        descs.truncate(nvars);
        Some((rest, descs))
    }

    pub(super) fn exec_stmt_foreach_inner(
        &mut self,
        _stmt: &Statement,
        array: &Expression,
        vars: &[Option<crate::ast::Identifier>],
        body: &Statement,
        iterations: &mut u64,
    ) {
        // foreach index variables are loop-scoped (SV): save the outer
        // values and restore them when the loop exits so a same-named
        // local survives unchanged.
        let fe_names: Vec<String> = vars
            .iter()
            .filter_map(|v| v.as_ref().map(|id| id.name.clone()))
            .collect();
        let fe_saved = self.snapshot_loop_vars(&fe_names);
        // §6.19.6: when the collection is an associative array whose KEY
        // is a named type (`int cnt [sev_e];`), the index variable HAS
        // that type — `foreach (cnt[s]) s.name()` must resolve against
        // that enum. Without the binding the hint is absent and
        // the enum-name fallback scans every enum and picks the largest, so
        // the key printed as a member of an unrelated enum (UVM's report
        // summary listed `UVM_NORADIX` where a severity belonged).
        // Registered for the loop's duration; the loop-var restore below
        // is by value, so drop the type binding on the way out too.
        let fe_key_type_var: Option<String> = (|| {
            let arr_name = Self::foreach_array_root_name(array)?;
            // A subroutine-local array's key type lives in its frame's
            // overlay (see the local associative-array declaration); ""
            // there means an unnamed key type shadowing an outer array.
            let local_kt = self.local_typedef_type_of(&format!("{}[]", arr_name));
            if local_kt.as_deref() == Some("") {
                return None;
            }
            // Module/package-scope collections record the key type in
            // the module map; a CLASS PROPERTY records it on its class
            // (§3b): walk `this`'s hierarchy when the module map
            // misses — UVM's `severity_count[uvm_severity]` lives on
            // uvm_report_server, and without this the summary printed
            // members of an unrelated enum for every severity.
            let kt = local_kt
                .or_else(|| self.module.assoc_key_type_names.get(&arr_name).cloned())
                .or_else(|| {
                    // `foreach (c.data[k])`: the property's class is the
                    // receiver's, not `this`'s.
                    let recv = match &array.kind {
                        ExprKind::Index { expr: b, .. } => &**b,
                        _ => array,
                    };
                    let recv_handle = match &recv.kind {
                        ExprKind::MemberAccess { expr: inner, .. } => self.eval_handle_expr(inner),
                        ExprKind::Ident(h) if h.path.len() >= 2 => {
                            self.eval_ident_handle(&h.path[0].name.name)
                        }
                        _ => None,
                    };
                    let handle = match recv_handle {
                        Some(h) => h,
                        None => self.this_stack.last().copied().flatten()?,
                    };
                    let mut cur = self
                        .heap
                        .get(handle)
                        .and_then(|o| o.as_ref())
                        .map(|i| i.class_name.clone());
                    let mut seen: HashSet<String> = HashSet::default();
                    while let Some(cn) = cur {
                        if !seen.insert(cn.clone()) {
                            break;
                        }
                        let cd = self.module.classes.get(&cn)?;
                        if let Some(kt) = cd.assoc_key_types.get(&arr_name) {
                            return Some(kt.clone());
                        }
                        cur = cd.extends.clone();
                    }
                    None
                })?;
            let v = vars.first()?.as_ref()?.name.clone();
            self.meta_note(&v);
            // §7.8.2: a packed-struct key gives the index variable that
            // struct's members (`foreach (m[k]) k.a`), as a local declared
            // with the type would get (`exec_stmt_var_decl`). The layout is
            // scoped to the loop by `restore_loop_vars`.
            if let Some(dt) = self.module.typedef_types.get(&kt).cloned() {
                if let Some(fields) = super::elaborate::packed_struct_field_layout(
                    &dt,
                    &self.module.parameters,
                    &self.module.typedefs,
                    &self.module.typedef_types,
                )
                .filter(|f| !f.is_empty())
                {
                    self.module.packed_struct_fields.insert(v.clone(), fields);
                }
            }
            self.var_typedef_types.insert(v.clone(), kt);
            // The loop var is a FRESH declaration (§12.7.3) — its type
            // binding shadows class properties and stale flat-map
            // entries for the loop's duration (see fe_trusted_types).
            self.fe_trusted_types.insert(v.clone());
            Some(v)
        })();
        let _fe_key_guard = &fe_key_type_var;
        // §12.7.3: a foreach loop variable over a CLASS-handle collection has
        // the collection's element class as its declared type
        // (`foreach (m_successors[succ])` — `succ` is a uvm_phase). Record it
        // in the frame overlay so member accesses through the loop variable
        // classify as class-object accesses (id-collision-in-heap receiver
        // gating); the overlay pops with the frame, so no cleanup is needed.
        if vars.len() == 1 {
            if let Some(v) = vars[0].as_ref() {
                if let Some(arr_name) = Self::foreach_array_root_name(array) {
                    // An ASSOCIATIVE collection's loop variable is its KEY
                    // (§12.7.3) — try the key's class before the element
                    // class: `bit m_successors[uvm_phase]` has an element
                    // class of `bit` (None) while the key names `uvm_phase`.
                    // Key first matters when BOTH are classes: the loop var
                    // still binds the key, not the value type.
                    if let Some(cls) = self
                        .assoc_key_class_of_foreach(&arr_name, array)
                        .or_else(|| self.elem_class_of_arr(&arr_name))
                    {
                        if self.module.classes.contains_key(&cls) {
                            self.record_local_class_type(&v.name, &cls);
                        }
                    }
                }
            }
        }
        // foreach index vars are automatic (§12.7.3); like for-init
        // vars, record them so a `fork … join_none/join_any` in the
        // body captures the per-iteration value by value (§9.3.2).
        // Truncated on every exit path of this arm.
        let fe_auto_len = self.auto_loop_vars.len();
        if self.local_stack.last().is_none() {
            for nm in &fe_names {
                self.auto_loop_vars.push(nm.clone());
            }
        }
        // §8.3.2/§6.20.2 (issue #388): `foreach (pkg::Cls::T[k])` —
        // `foreach (Cls::T[k])` from a foreign class, bare `foreach (T[k])`
        // in a method of the declaring class — iterates a class-scope ARRAY
        // constant (localparam array, header value-parameter array, static
        // fixed-array property). The constant has no module-scope
        // collection cell, so the generic arms below found no rows and the
        // body never ran.
        if vars.len() == 1 {
            if let Some(c) = self.resolve_class_scope_array_const(array) {
                let iv = vars[0].as_ref().map(|v| v.name.clone());
                if let Some(n) = &iv {
                    self.widths.insert(n.clone(), 32);
                }
                // §12.7.3: visit the indices in DECLARED order — a
                // descending `[5:4]` constant iterates 5 then 4, not the
                // normalized ascending pair.
                let order: Vec<i64> = if c.left > c.right {
                    (c.right..=c.left).rev().collect()
                } else {
                    (c.left..=c.right).collect()
                };
                'csc: for i in order {
                    if self.finished {
                        break 'csc;
                    }
                    if let Some(n) = &iv {
                        self.set_loop_var(n, Self::signed_loop_val(i));
                    }
                    self.continue_flag = false;
                    *iterations = iterations.saturating_add(1);
                    self.exec_statement(body);
                    if self.break_flag {
                        break 'csc;
                    }
                    self.continue_flag = false;
                }
                self.auto_loop_vars.truncate(fe_auto_len);
                self.restore_loop_vars(&fe_saved);
                if !self.return_flag && self.disable_target.is_none() {
                    self.break_flag = false;
                }
                return;
            }
        }
        // §12.7.3/§7.4.5: `foreach (a[i, j])` over a fixed array of queues
        // walks each row's own size.
        if vars.len() == 2
            && (self.indexed_coll_mask.get() != 0 || self.indexed_coll_roots.get().is_none())
            && !self.indexed_coll_roots().is_empty()
            && let Some((an, rows)) = self.fixed_rows_of_colls(array)
        {
            let iv = vars[0].as_ref().map(|v| v.name.clone());
            let jv = vars[1].as_ref().map(|v| v.name.clone());
            for n in iv.iter().chain(jv.iter()) {
                self.widths.insert(n.clone(), 32);
            }
            'rows: for i in rows {
                let inner = self.get_queue_size(&Self::name_with_index(&an, i));
                for j in 0..inner {
                    if self.finished {
                        break 'rows;
                    }
                    if let Some(n) = &iv {
                        self.set_loop_var(n, Self::signed_loop_val(i));
                    }
                    if let Some(n) = &jv {
                        self.set_loop_var(n, Self::signed_loop_val(j as i64));
                    }
                    self.continue_flag = false;
                    *iterations = iterations.saturating_add(1);
                    self.exec_statement(body);
                    if self.break_flag {
                        break 'rows;
                    }
                    self.continue_flag = false;
                }
            }
            self.auto_loop_vars.truncate(fe_auto_len);
            self.restore_loop_vars(&fe_saved);
            if !self.return_flag && self.disable_target.is_none() {
                self.break_flag = false;
            }
            return;
        }
        // §12.7.3/§7.4.5: `foreach (a[i, j, k])` over fixed dimensions
        // that end in a collection (`int a[3][3][$]`, `int a[2][2][int]`,
        // a class property too): the last variable walks each element's
        // own size or keys, not the element's packed bits.
        // §12.7.3: an omitted variable skips that dimension. Before a
        // collection dimension that leaves the element's size undefined
        // (every row has its own), so the loop is rejected rather than
        // walking the element's packed bits (it iterated 32 times over an
        // `int` row).
        if vars.len() >= 2
            && self.indexed_colls_live()
            && let Some((an, dims)) = self.fixed_dims_of_colls(array)
            && vars.len() == dims.len() + 1
            && vars[..dims.len()].iter().any(|v| v.is_none())
            && vars.last().is_some_and(Option::is_some)
        {
            let m = format!(
                "[xezim][error] foreach ({}[...]): a loop variable before a queue, dynamic or \
                 associative dimension cannot be omitted (§12.7.3: the collection's size \
                 depends on that index)",
                an
            );
            self.record_output(m.clone());
            self.stdout_writeln(&m);
            self.abort_run();
            return;
        }
        if vars.len() >= 2
            && self.indexed_colls_live()
            && let Some((an, dims)) = self.fixed_dims_of_colls(array)
            && vars.len() == dims.len() + 1
            && vars[..dims.len()].iter().all(|v| v.is_some())
            && vars.last().is_some_and(Option::is_some)
        {
            let names: Vec<Option<String>> = vars
                .iter()
                .map(|v| v.as_ref().map(|v| v.name.clone()))
                .collect();
            for n in names.iter().flatten() {
                self.widths.insert(n.clone(), 32);
            }
            let nd = dims.len();
            let mut idx: Vec<i64> = dims.iter().map(|d| d.0).collect();
            'elems: loop {
                if self.finished || self.return_flag || self.disable_target.is_some() {
                    break;
                }
                let en = idx
                    .iter()
                    .fold(an.clone(), |n, &i| Self::name_with_index(&n, i));
                let keys: Vec<Value> = if self.is_associative_array(&en) {
                    let is_str = self.is_string_keyed_array(&en);
                    let (kw, signed) = self.assoc_index_width_for(&en).unwrap_or((32, false));
                    if let (false, Some(n)) = (is_str, &names[nd]) {
                        self.widths.insert(n.clone(), kw);
                    }
                    self.assoc_top_level_keys(&en)
                        .iter()
                        .map(|k| {
                            if is_str {
                                Value::from_string(k)
                            } else if kw > 64 {
                                let mut v = Value::from_str_radix(k, 16, kw);
                                v.is_signed = signed;
                                v
                            } else {
                                let mut v =
                                    Value::from_u64(k.parse::<i64>().unwrap_or(0) as u64, kw);
                                v.is_signed = signed;
                                v
                            }
                        })
                        .collect()
                } else {
                    (0..self.get_queue_size(&en))
                        .map(|j| Self::signed_loop_val(j as i64))
                        .collect()
                };
                for kv in keys {
                    if self.finished || self.return_flag || self.disable_target.is_some() {
                        break 'elems;
                    }
                    for (d, n) in names[..nd].iter().enumerate() {
                        if let Some(n) = n {
                            self.set_loop_var(n, Self::signed_loop_val(idx[d]));
                        }
                    }
                    if let Some(n) = &names[nd] {
                        self.set_loop_var(n, kv);
                    }
                    self.continue_flag = false;
                    *iterations = iterations.saturating_add(1);
                    self.exec_statement(body);
                    if self.break_flag {
                        break 'elems;
                    }
                    self.continue_flag = false;
                }
                // Next element, last outer dimension fastest.
                let mut d = nd;
                loop {
                    if d == 0 {
                        break 'elems;
                    }
                    d -= 1;
                    if idx[d] != dims[d].1 {
                        idx[d] += if dims[d].0 <= dims[d].1 { 1 } else { -1 };
                        break;
                    }
                    idx[d] = dims[d].0;
                }
            }
            self.auto_loop_vars.truncate(fe_auto_len);
            self.restore_loop_vars(&fe_saved);
            if !self.return_flag && self.disable_target.is_none() {
                self.break_flag = false;
            }
            return;
        }
        // §12.7.3: `foreach (a[i][j])` inside `foreach (a[i])` iterates the
        // SELECTED sub-array: its loop variables map onto the dimensions after
        // the selected ones. The selects were dropped, so `j` walked the first
        // dimension again (2 of 3 columns of `int a[2][3]`).
        if let Some((dims, descs)) = self.foreach_fixed_subarray_dims(array, vars.len()) {
            self.exec_foreach_nested_dir(&dims, vars, body, None, Some(&descs), iterations);
            self.auto_loop_vars.truncate(fe_auto_len);
            self.restore_loop_vars(&fe_saved);
            if !self.return_flag && self.disable_target.is_none() {
                self.break_flag = false;
            }
            return;
        }
        // `foreach (obj.assoc_member[k])` — iterate that object's
        // instance-scoped associative array (base may be a 2-seg
        // Ident `b.edges` or a MemberAccess).
        {
            let selected = match &array.kind {
                ExprKind::Index { .. } => self.foreach_selected_collection(array),
                ExprKind::Ident(h) if h.path.iter().any(|p| !p.selects.is_empty()) => {
                    self.foreach_selected_collection(array)
                }
                _ => None,
            };
            if let Some(an) = selected.or_else(|| self.expr_assoc_name(array)) {
                // §12.7.3 `foreach (m[i, j])` over a DYNAMIC ARRAY OF
                // (`<h>#m[i].size`), so the two dimensions cannot be
                // iterated as a rectangle.
                if vars.len() >= 2
                    && self.signals.contains_key(&format!("{}.size", an))
                    && self.module.dynamic_arrays.contains(&format!("{}[0]", an))
                {
                    let outer = self.get_queue_size(&an);
                    let iv = vars[0].as_ref().map(|v| v.name.clone());
                    let jv = vars[1].as_ref().map(|v| v.name.clone());
                    if let Some(n) = &iv {
                        self.widths.insert(n.clone(), 32);
                    }
                    if let Some(n) = &jv {
                        self.widths.insert(n.clone(), 32);
                    }
                    'rows: for i in 0..outer {
                        let inner = self.get_queue_size(&format!("{}[{}]", an, i));
                        for j in 0..inner {
                            if self.finished {
                                break 'rows;
                            }
                            if let Some(n) = &iv {
                                self.set_loop_var(n, Self::signed_loop_val(i as i64));
                            }
                            if let Some(n) = &jv {
                                self.set_loop_var(n, Self::signed_loop_val(j as i64));
                            }
                            self.continue_flag = false;
                            *iterations = iterations.saturating_add(1);
                            self.exec_statement(body);
                            if self.break_flag {
                                break 'rows;
                            }
                            self.continue_flag = false;
                        }
                    }
                    self.auto_loop_vars.truncate(fe_auto_len);
                    self.restore_loop_vars(&fe_saved);
                    if !self.return_flag && self.disable_target.is_none() {
                        self.break_flag = false;
                    }
                    return;
                }
                // Multi-var foreach over a FIXED multi-dim class member
                // (`foreach (foo[ia,ib])` on `test_t foo[0:3][0:7]`):
                // its per-instance shape is registered in
                // arrays_2d/arrays_nd — iterate the rectangle. The
                // single-var key-scan below would leave the inner
                // index X and mis-parse `i][j` composite keys.
                if vars.len() >= 2 {
                    if let Some(mut dims) = self.foreach_dims(&an) {
                        // A dynamic outer dimension (`T a[][N]`) iterates
                        // its current size, not the 64-slot buffer.
                        if self.module.dynamic_arrays.contains(&an) {
                            dims[0] = (0, self.get_queue_size(&an) as i64 - 1);
                        }
                        if dims.len() >= vars.len() {
                            self.exec_foreach_nested(
                                &dims[..vars.len()],
                                vars,
                                body,
                                None,
                                iterations,
                            );
                            self.auto_loop_vars.truncate(fe_auto_len);
                            self.restore_loop_vars(&fe_saved);
                            return;
                        }
                    }
                }
                // §12.7.3: `foreach (m[i])` with FEWER index vars than
                // dims iterates the first dimension fully — the
                // single-var fallbacks below sized it wrong (one
                // iteration on a 2-D fixed array).
                if vars.len() == 1 {
                    if let Some(dims) = self.foreach_dims(&an) {
                        if dims.len() >= 2 {
                            let descs = self.foreach_unpacked_descs(&an, 1);
                            self.exec_foreach_nested_dir(
                                &dims[..1],
                                vars,
                                body,
                                None,
                                Some(&descs),
                                iterations,
                            );
                            self.auto_loop_vars.truncate(fe_auto_len);
                            self.restore_loop_vars(&fe_saved);
                            if !self.return_flag && self.disable_target.is_none() {
                                self.break_flag = false;
                            }
                            return;
                        }
                    }
                }
                // §7.9.1/§7.10: `foreach (all[outer, inner])` over an
                // ASSOCIATIVE ARRAY OF QUEUES (`T all[key][$]`). The
                // outer index walks the (sparse) assoc keys, the inner
                // index walks each key's queue. Unlike the
                // dynamic-array-of-dynamic-arrays branch above, the
                // keys are not a dense [0,N) range, so iterate the
                // collected top-level keys and each key's `.size`.
                if vars.len() >= 2 {
                    let akeys = self.assoc_top_level_keys(&an);
                    let is_assoc_of_queue = !akeys.is_empty()
                        && akeys.iter().all(|k| {
                            let qn = format!("{}[{}]", an, k);
                            self.module.dynamic_arrays.contains(&qn)
                                || self.signals.contains_key(&format!("{}.size", qn))
                        });
                    if is_assoc_of_queue {
                        let ov = vars[0].as_ref().map(|v| v.name.clone());
                        let iv = vars[1].as_ref().map(|v| v.name.clone());
                        if let Some(n) = &ov {
                            self.widths.insert(n.clone(), 32);
                        }
                        if let Some(n) = &iv {
                            self.widths.insert(n.clone(), 32);
                        }
                        'akeys: for key in &akeys {
                            let qn = format!("{}[{}]", an, key);
                            let inner = self.get_queue_size(&qn);
                            for j in 0..inner {
                                if self.finished {
                                    break 'akeys;
                                }
                                if let Some(n) = &ov {
                                    let kv = if let Ok(v) = key.parse::<i64>() {
                                        Value::from_u64(v as u64, 32)
                                    } else {
                                        Value::from_string(key)
                                    };
                                    self.set_loop_var(n, kv);
                                }
                                if let Some(n) = &iv {
                                    self.set_loop_var(n, Self::signed_loop_val(j as i64));
                                }
                                self.continue_flag = false;
                                *iterations = iterations.saturating_add(1);
                                self.exec_statement(body);
                                if self.break_flag {
                                    break 'akeys;
                                }
                                self.continue_flag = false;
                            }
                        }
                        self.auto_loop_vars.truncate(fe_auto_len);
                        self.restore_loop_vars(&fe_saved);
                        if !self.return_flag && self.disable_target.is_none() {
                            self.break_flag = false;
                        }
                        return;
                    }
                }
                // §7.8, §12.7.3: `foreach (m[i, j])` over an associative
                // array of associative arrays (`int m[int][string]`) walks
                // each level's own keys. The single-var key scan below
                // visited the outer keys only and left `j` unset.
                if vars.len() >= 2 && self.is_assoc_of_assoc(&an) {
                    self.foreach_assoc_levels(&an, &an, vars, 0, body, iterations);
                    self.auto_loop_vars.truncate(fe_auto_len);
                    self.restore_loop_vars(&fe_saved);
                    if !self.return_flag && self.disable_target.is_none() {
                        self.break_flag = false;
                    }
                    return;
                }
                if let Some(var) = vars.first().and_then(|v| v.as_ref()) {
                    // A queue / dynamic array carries an authoritative
                    // `<name>.size` shadow and is DENSE: iterate the
                    // logical range [0, size). Scanning the raw signal
                    // keys instead would visit stale elements left
                    // behind when the queue shrank (e.g. a reassigned
                    // `obj.q = {…}` that dropped its tail) — those keys
                    // linger in the `signals` map past the logical size
                    // and produce phantom iterations / off-by-one index
                    // counts. Only TRUE associative arrays (sparse, no
                    // `.size` shadow) fall through to the key-scan.
                    let keys: Vec<String>;
                    let is_str;
                    if let Some(&(lo, hi, _)) =
                        self.module.arrays.get(&an).filter(|&&(lo, hi, _)| {
                            hi >= lo
                                && !self.module.dynamic_arrays.contains(&an)
                                && !self.module.associative_arrays.contains_key(&an)
                        })
                    {
                        // §12.7.3: a FIXED class-member array iterates its
                        // declared range, left bound first (`[3:1]`: 3, 2, 1),
                        // whatever stray `.size` a whole-array write left.
                        let desc = self.module.descending_arrays.contains(&an);
                        keys = (0..=(hi - lo))
                            .map(|k| if desc { hi - k } else { lo + k }.to_string())
                            .collect();
                        is_str = false;
                    } else if let Some(sz) = self
                        .signals
                        .get(&format!("{}.size", an))
                        .and_then(|v| v.to_u64())
                    {
                        keys = (0..sz).map(|i| i.to_string()).collect();
                        is_str = false;
                    } else {
                        let prefix = format!("{}[", an);
                        let mut struct_elem: Option<bool> = None;
                        // An assoc-of-collections member (e.g.
                        // `bq_t all[int]`) stores elements as
                        // `all[5][0]` etc. — extract the key up to
                        // the FIRST `]` and dedup so multiple
                        // sub-elements don't each produce a phantom key.
                        let mut ks: Vec<String> = self
                            .signals
                            .elem_keys_with_prefix(&prefix)
                            .filter_map(|k| {
                                let rest = k.strip_prefix(prefix.as_str())?;
                                let se = Self::key_has_member_suffix(rest)
                                    && *struct_elem.get_or_insert_with(|| {
                                        self.queue_elem_struct(&an).is_some()
                                    });
                                Self::assoc_elem_key_seg(rest, se).map(|s| s.to_string())
                            })
                            .collect();
                        ks.sort();
                        ks.dedup();
                        is_str = self
                            .module
                            .associative_arrays
                            .get(&an)
                            .copied()
                            .unwrap_or(false)
                            || self.is_string_keyed_array(&an);
                        // Integer-keyed members iterate in NUMERIC order;
                        // lexicographic sort would visit "10" before "2".
                        // Use i64 so negative (signed) keys keep order.
                        if is_str {
                            ks.sort();
                        } else {
                            // §7.9: int keys traverse in NUMERIC order
                            // including NEGATIVE keys — u64 parsing
                            // mangled "-7" to 0.
                            ks.sort_by_key(|k| k.parse::<i64>().unwrap_or(0));
                        }
                        keys = ks;
                    }
                    self.widths.insert(var.name.clone(), 32);
                    // §7.8.2: the loop variable takes the AA's index
                    // type (width + signedness) so a signed/narrowed
                    // key (e.g. shortint -3) keeps its value instead
                    // of a failed u64 parse clobbering it to 0.
                    let (kw, ks_sign) = self.assoc_index_width_for(&an).unwrap_or((32, false));
                    self.widths.insert(var.name.clone(), kw);
                    let var_scope = (!an.contains('#'))
                        .then(|| an.rsplit_once('.').map(|(scope, _)| scope.to_string()))
                        .flatten();
                    for key in keys {
                        if self.finished {
                            break;
                        }
                        let kv = if is_str {
                            Value::from_string(&key)
                        } else if kw > 64 {
                            // Wide numeric keys are keyed by hex digits.
                            Value::from_str_radix(&key, 16, kw)
                        } else if ks_sign {
                            let mut v = Value::from_u64(key.parse::<i64>().unwrap_or(0) as u64, kw);
                            v.is_signed = true;
                            v
                        } else {
                            // §7.9 + §7.8.2: SIGNED parse (negative keys
                            // must not collapse to 0) at the DECLARED
                            // index width/signedness.
                            let mut v = Value::from_u64(key.parse::<i64>().unwrap_or(0) as u64, kw);
                            v.is_signed = ks_sign;
                            v
                        };
                        self.set_loop_var_aliased(var_scope.as_deref(), &var.name, kv);
                        self.continue_flag = false;
                        *iterations = iterations.saturating_add(1);
                        self.exec_statement(body);
                        // `continue` is per-iteration (clear it); a
                        // `break`/`return` (both set break_flag) exits
                        // the loop and propagates — do NOT clear it.
                        if self.break_flag {
                            break;
                        }
                        self.continue_flag = false;
                    }
                }
                self.auto_loop_vars.truncate(fe_auto_len);
                self.restore_loop_vars(&fe_saved);
                // A `break` inside the loop consumed the loop exit.
                // Only a `return` (return_flag) should propagate to
                // the enclosing function body (IEEE 1800-2023 §12.8) —
                // and a `disable` (§9.6.2), which unwinds to the block or
                // task it names: consuming it as a `break` finished the
                // disabled task's remaining statements and left the target
                // set for an unrelated later loop to trip over.
                if !self.return_flag && self.disable_target.is_none() {
                    self.break_flag = false;
                }
                return;
            }
        }
        if let ExprKind::Ident(hier) = &array.kind {
            let mut name = self.resolve_hier_name(hier);
            // Packed multi-dimensional CLASS property (`foreach (c.d[i, j])`
            // over `bit [6:0] [4:0][1:0] d`): the geometry lives on the
            // class type, not in the module-level packed tables.
            let mut prop_packed_dims: Option<Vec<(i64, i64)>> = None;
            // A subroutine-LOCAL queue/dyn array lives under its
            // per-process rename — and shadows any same-named class
            // member (§8.10). Without this, `foreach (pp[j])` over a
            // local queue inside a class method iterated ZERO times
            // (UVM's get_full_hdl_path parent_paths loop, so every
            // backdoor access lost its hdl path).
            if let Some(rn) = self.dyn_name_lookup(&name) {
                name = std::borrow::Cow::Owned(rn.to_string());
            } else {
                let spec_key = self.spec_static_coll_key(&name);
                if let std::borrow::Cow::Owned(k) = spec_key {
                    name = std::borrow::Cow::Owned(k);
                } else if let Some(scoped) = self.instance_assoc_member(&name) {
                    name = std::borrow::Cow::Owned(scoped);
                }
            }
            // §7.8, §12.7.3: a multi-variable `foreach` over an
            // associative array of associative arrays (see the member arm).
            if vars.len() >= 2 && self.is_associative_array(&name) && self.is_assoc_of_assoc(&name)
            {
                let an = name.into_owned();
                self.foreach_assoc_levels(&an, &an, vars, 0, body, iterations);
                self.auto_loop_vars.truncate(fe_auto_len);
                self.restore_loop_vars(&fe_saved);
                if !self.return_flag && self.disable_target.is_none() {
                    self.break_flag = false;
                }
                return;
            }
            // `foreach (obj.member[i, j])` over a FIXED array
            // property: the hier name resolves under the VARIABLE
            // scope ("o.g2"), but the per-instance shape tables key
            // the storage `<handle>#g2` — so every dims lookup below
            // missed, none of the multi-var arms ran, and the loop
            // collapsed to a SINGLE iteration (a 1-D member survived
            // only via a later element-key scan).
            if hier.path.len() >= 2
                && hier.path.iter().all(|s| s.selects.is_empty())
                && !self.module.arrays.contains_key(&*name)
                && !self.module.arrays_2d.contains_key(&*name)
                && !self.module.arrays_nd.contains_key(&*name)
                && !self.module.dynamic_arrays.contains(&*name)
                && !self.is_associative_array(&name)
            {
                let obj = hier.path[0].name.name.clone();
                let mut h = if obj == "this" || obj == "super" {
                    self.this_stack.last().copied().flatten()
                } else {
                    self.eval_ident_handle(&obj)
                };
                // Middle segments are handle-valued properties
                // (`o.sub.g` — follow `sub` on o's heap instance).
                for seg in &hier.path[1..hier.path.len() - 1] {
                    h = h.filter(|&x| x != 0).and_then(|x| {
                        self.heap
                            .get(x)
                            .and_then(|o| o.as_ref())
                            .and_then(|i| i.properties.get(&seg.name.name))
                            .and_then(|v| v.to_u64())
                            .map(|v| v as usize)
                    });
                }
                if let Some(h) = h.filter(|&h| h != 0) {
                    let scoped = format!(
                        "{}#{}",
                        h,
                        hier.path.last().map(|s| s.name.name.as_str()).unwrap_or("")
                    );
                    if self.module.arrays.contains_key(&scoped)
                        || self.module.arrays_2d.contains_key(&scoped)
                        || self.module.arrays_nd.contains_key(&scoped)
                    {
                        name = std::borrow::Cow::Owned(scoped);
                    } else if let Some(pd) = self.class_prop_packed_dims(
                        h,
                        hier.path.last().map(|s| s.name.name.as_str()).unwrap_or(""),
                    ) {
                        if pd.len() >= vars.len() {
                            prop_packed_dims = Some(pd);
                        }
                    }
                }
            }
            // Same geometry for a bare property name inside a method
            // (`foreach (d[i, j])` with `this` implied).
            if prop_packed_dims.is_none()
                && hier.path.len() == 1
                && hier.path[0].selects.is_empty()
                && self.foreach_dims(&name).is_none()
                && !self.module.packed_full_dims.contains_key(&*name)
            {
                if let Some(h) = self
                    .this_stack
                    .last()
                    .copied()
                    .flatten()
                    .filter(|&h| h != 0)
                {
                    if let Some(pd) = self.class_prop_packed_dims(h, &hier.path[0].name.name) {
                        if pd.len() >= vars.len() {
                            prop_packed_dims = Some(pd);
                        }
                    }
                }
            }
            // A bare array name inside a SUBMODULE process resolves
            // under the process's instance scope ("u_s.mem"), like
            // reads/writes already do — otherwise the collection
            // lookups all miss and the foreach body never runs.
            if !self.module.arrays.contains_key(&*name)
                && !self.module.arrays_2d.contains_key(&*name)
                && !self.module.arrays_nd.contains_key(&*name)
                && !self.module.dynamic_arrays.contains(&*name)
                && !self.is_associative_array(&name)
            {
                if let Some(scoped) = self.hint_scoped(&name) {
                    if self.module.arrays.contains_key(&scoped)
                        || self.module.arrays_2d.contains_key(&scoped)
                        || self.module.arrays_nd.contains_key(&scoped)
                        || self.module.dynamic_arrays.contains(&scoped)
                        || self.is_associative_array(&scoped)
                    {
                        name = std::borrow::Cow::Owned(scoped);
                    }
                }
            }
            // The hint above is only installed while a PROCESS runs. A
            // `foreach` inside an `always_comb` is evaluated from the
            // settle path with NO hint, so a bare name in a submodule
            // stayed unscoped, `foreach_dims` returned None and the loop
            // body never ran — the array kept its X. Fall back to a
            // UNIQUE suffix match: accept `<scope>.m` only when exactly
            // one registered array ends that way, so this can never pick
            // between same-named arrays in sibling instances.
            if !self.module.arrays.contains_key(&*name)
                && !self.module.arrays_2d.contains_key(&*name)
                && !self.module.arrays_nd.contains_key(&*name)
                && !self.module.dynamic_arrays.contains(&*name)
                && !self.is_associative_array(&name)
                && !name.contains('.')
            {
                let suffix = format!(".{}", name);
                let mut hit: Option<String> = None;
                let mut many = false;
                for k in self
                    .module
                    .arrays
                    .keys()
                    .chain(self.module.arrays_2d.keys())
                    .chain(self.module.arrays_nd.keys())
                {
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
            // The array's instance prefix ("u_s" from "u_s.mem") —
            // loop vars are ALSO bound under it (see
            // set_loop_var_aliased).
            let var_scope: Option<String> = name.rsplit_once('.').map(|(p, _)| p.to_string());
            // §12.7.3 over an unpacked-struct MEMBER array with no
            // registered shape — a member of a subroutine-local or formal
            // struct (`foreach (r.mm[i, j])` in a function). Its dimensions
            // come from the member declaration, iterated left bound to
            // right bound.
            if name.contains('.')
                && self.struct_root_possible(name.split(['.', '[']).next().unwrap_or(""))
                && self.foreach_dims(&name).is_none()
                && !self.module.dynamic_arrays.contains(&*name)
                && !self.is_associative_array(&name)
            {
                if let Some((_, Some(lists))) = self.flat_path_type(&name) {
                    if vars.len() <= lists.len() {
                        let n = vars.len();
                        let dims: Vec<(i64, i64)> = lists[..n]
                            .iter()
                            .map(|l| {
                                let (a, b) = (l[0], l[l.len() - 1]);
                                (a.min(b), a.max(b))
                            })
                            .collect();
                        let descs: Vec<bool> =
                            lists[..n].iter().map(|l| l[0] > l[l.len() - 1]).collect();
                        self.exec_foreach_nested_dir(
                            &dims,
                            vars,
                            body,
                            None,
                            Some(&descs),
                            iterations,
                        );
                        self.auto_loop_vars.truncate(fe_auto_len);
                        self.restore_loop_vars(&fe_saved);
                        if !self.return_flag && self.disable_target.is_none() {
                            self.break_flag = false;
                        }
                        return;
                    }
                }
            }
            // IEEE 1800-2017 §12.7.3: one loop variable per leading
            // dimension — `foreach (m[i, j])`. A null entry (`m[, j]`)
            // skips that dimension. Only `vars[0]` was ever bound, so
            // `j` stayed X and the loop ran over the first dim alone.
            // §12.7.3: `foreach (m[i])` with FEWER index vars than
            // dims iterates the FIRST dimension fully — the 1-var
            // fallbacks below sized a 2-D fixed array wrong (one
            // iteration).
            if vars.len() == 1
                && !self.module.dynamic_arrays.contains(&*name)
                && !self.is_associative_array(&name)
            {
                if let Some(dims) = self.foreach_dims(&name) {
                    if dims.len() >= 2 {
                        let descs = self.foreach_unpacked_descs(&name, 1);
                        self.exec_foreach_nested_dir(
                            &dims[..1],
                            vars,
                            body,
                            None,
                            Some(&descs),
                            iterations,
                        );
                        self.auto_loop_vars.truncate(fe_auto_len);
                        self.restore_loop_vars(&fe_saved);
                        if !self.return_flag && self.disable_target.is_none() {
                            self.break_flag = false;
                        }
                        return;
                    }
                }
            }
            if vars.len() >= 2 {
                // §12.7.3: the loop variables map onto the array's
                // dimensions left to right, unpacked first, then
                // packed. A PURELY packed array (`u7_t [4:0][1:0] a`,
                // `bit [6:0][4:0][1:0] b`) has no unpacked shape, so
                // `foreach_dims` is None; that used to fall through to
                // the single-dimension path and iterate one packed
                // dimension with every other variable x. Start from an
                // empty shape and let the packed dimensions supply
                // every variable.
                let purely_packed = prop_packed_dims.is_some()
                    || (self.foreach_dims(&name).is_none()
                        && !self.module.dynamic_arrays.contains(&*name)
                        && !self.is_associative_array(&name)
                        && self
                            .module
                            .packed_full_dims
                            .get(&*name)
                            .is_some_and(|d| d.len() >= vars.len()));
                let start_dims = if purely_packed {
                    Some(Vec::new())
                } else {
                    self.foreach_dims(&name)
                };
                if let Some(mut dims) = start_dims {
                    // A dynamic outer dimension (`T a[][16]`) is backed
                    // by a fixed 64-slot buffer; iterate only the
                    // CURRENT size, like the 1-var dynamic path below.
                    if self.module.dynamic_arrays.contains(&*name) {
                        let sz = self.get_queue_size(&name) as i64;
                        dims[0] = (0, sz - 1);
                    }
                    // §12.7.3: loop vars BEYOND the unpacked dimensions
                    // iterate the element's PACKED dimensions — `foreach
                    // (array[i,j])` on `logic [1:0][7:0] array[0:3]`
                    // iterates j over the [1:0] packed dim. Use the
                    // packed dimension STRUCTURE (`packed_full_dims`,
                    // outermost first) rather than the flattened element
                    // width, so multi-packed-dim elements and non-zero /
                    // negative / descending packed ranges iterate the
                    // correct index set (each dim normalized to
                    // ascending for the odometer). Fall back to the
                    // flat width only for a lone packed dim we did not
                    // record (e.g. a bare `reg [W-1:0]` element).
                    let mut descs: Vec<bool> = if purely_packed {
                        Vec::new()
                    } else {
                        self.foreach_unpacked_descs(&name, dims.len())
                    };
                    if dims.len() < vars.len() {
                        if let Some(pdims) = prop_packed_dims
                            .as_ref()
                            .or_else(|| self.module.packed_full_dims.get(&*name))
                        {
                            for &(l, r) in pdims.iter() {
                                if dims.len() >= vars.len() {
                                    break;
                                }
                                // §12.7.3: left→right — a packed
                                // [3:0] dim iterates 3,2,1,0.
                                dims.push((l.min(r), l.max(r)));
                                descs.push(l > r);
                            }
                        } else if dims.len() + 1 == vars.len() {
                            let ew = self
                                .module
                                .arrays
                                .get(&*name)
                                .map(|&(_, _, w)| w)
                                .or_else(|| self.module.arrays_2d.get(&*name).map(|&(_, _, w)| w))
                                .or_else(|| self.module.arrays_nd.get(&*name).map(|(_, w)| *w));
                            if let Some(w) = ew.filter(|&w| w > 1) {
                                dims.push((0, w as i64 - 1));
                                // §12.7.3 left→right: an unrecorded
                                // lone packed dim is `[w-1:0]` unless
                                // the ascending map says otherwise.
                                descs.push(!self.module.ascending_packed.contains_key(&*name));
                            }
                        }
                    }
                    if dims.len() >= vars.len() {
                        descs.resize(vars.len().max(descs.len()), false);
                        self.exec_foreach_nested_dir(
                            &dims[..vars.len()],
                            vars,
                            body,
                            var_scope.as_deref(),
                            Some(&descs[..vars.len()]),
                            iterations,
                        );
                        self.auto_loop_vars.truncate(fe_auto_len);
                        self.restore_loop_vars(&fe_saved);
                        if !self.return_flag && self.disable_target.is_none() {
                            self.break_flag = false;
                        }
                        return;
                    }
                }
                // §12.7.3 / §7.9.1: `foreach (all[outer, inner])` over
                // an ASSOCIATIVE ARRAY OF QUEUES (`T all[key][$]`, e.g.
                // array precedence sorting). The outer
                // var walks the sparse assoc keys (ascending for an
                // integer-keyed array), the inner var walks each key's
                // queue [0, size); higher cardinality (inner) changes
                // fastest. `foreach_dims` returns None for a sparse
                // assoc, so without this the 2-var form fell through to
                // the 1-var scan below and only ever bound `outer` —
                // `inner` stayed at its last value and the body ran
                // once per key (losing the queue elements).
                if self.is_associative_array(&name) {
                    let akeys = self.assoc_top_level_keys(&name);
                    let is_assoc_of_queue = !akeys.is_empty()
                        && akeys.iter().all(|k| {
                            let qn = format!("{}[{}]", name, k);
                            self.module.dynamic_arrays.contains(&qn)
                                || self.signals.contains_key(&format!("{}.size", qn))
                        });
                    if is_assoc_of_queue {
                        let ov = vars[0].as_ref().map(|v| v.name.clone());
                        let iv = vars[1].as_ref().map(|v| v.name.clone());
                        if let Some(n) = &ov {
                            self.widths.insert(n.clone(), 32);
                        }
                        if let Some(n) = &iv {
                            self.widths.insert(n.clone(), 32);
                        }
                        let is_str = self
                            .module
                            .associative_arrays
                            .get(&*name)
                            .copied()
                            .unwrap_or(false);
                        'akeys: for key in &akeys {
                            let qn = format!("{}[{}]", name, key);
                            let inner = self.get_queue_size(&qn);
                            for j in 0..inner {
                                if self.finished {
                                    break 'akeys;
                                }
                                if let Some(n) = &ov {
                                    let kv = if is_str {
                                        Value::from_string(key)
                                    } else {
                                        let mut v = Value::from_u64(
                                            key.parse::<i64>().unwrap_or(0) as u64,
                                            32,
                                        );
                                        v.is_signed = true;
                                        v
                                    };
                                    self.set_loop_var_aliased(var_scope.as_deref(), n, kv);
                                }
                                if let Some(n) = &iv {
                                    self.set_loop_var_aliased(
                                        var_scope.as_deref(),
                                        n,
                                        Self::signed_loop_val(j as i64),
                                    );
                                }
                                self.continue_flag = false;
                                *iterations = iterations.saturating_add(1);
                                self.exec_statement(body);
                                if self.break_flag {
                                    break 'akeys;
                                }
                                self.continue_flag = false;
                            }
                        }
                        self.auto_loop_vars.truncate(fe_auto_len);
                        self.restore_loop_vars(&fe_saved);
                        if !self.return_flag && self.disable_target.is_none() {
                            self.break_flag = false;
                        }
                        return;
                    }
                }
            }
            if let Some(var) = vars.first().and_then(|v| v.as_ref()) {
                self.widths.insert(var.name.clone(), 32);
                if self.string_signals.contains(&*name)
                    // A QUEUE/dyn array OF STRINGS is in
                    // `string_signals` too (so its elements render as
                    // strings) — but foreach over it walks ELEMENTS,
                    // not characters. Without this exclusion a
                    // `string pp[$]` in a class method iterated zero
                    // times (UVM get_full_hdl_path's parent_paths
                    // loop — every backdoor access lost its path).
                    && !self.module.dynamic_arrays.contains(&*name)
                    && !self.is_associative_array(&name)
                    && !self.module.arrays.contains_key(&*name)
                    && !self.module.arrays_2d.contains_key(&*name)
                    && !self.module.arrays_nd.contains_key(&*name)
                {
                    // foreach over a STRING iterates its characters
                    // [0..len) by CONTENT length, and must be checked
                    // BEFORE any collection-array arm: a string name can
                    // carry a stale collection-name registration (width
                    // 2, e.g. module.arrays[value] == (0,-1,1)) that
                    // would otherwise send it through the fixed-array
                    // arm iterating 2 instead of the true character
                    // count, truncating UVM's `pack_string` by 8 bytes.
                    //
                    // A QUEUE / DYNAMIC / ASSOC ARRAY OF STRINGS
                    // (`string q[$]`, `string m[int]`) is NOT a scalar
                    // string: `string_signals` holds the collection name
                    // but the loop must iterate its ELEMENTS, not its
                    // characters — treating it as a string here left
                    // `foreach (name_order[k])` over uvm's string queue
                    // of field names with zero iterations, so config was
                    // never applied (`string_signals` won because the
                    // collection-carried registration; the char count
                    // of the unbuilt queue was 0).
                    let len = self.eval_expr(array).sv_string_bytes().len() as u64;
                    for i in 0..len {
                        if self.finished {
                            break;
                        }
                        self.set_loop_var_aliased(
                            var_scope.as_deref(),
                            &var.name,
                            Self::signed_loop_val(i as i64),
                        );
                        self.continue_flag = false;
                        *iterations = iterations.saturating_add(1);
                        self.exec_statement(body);
                        if self.break_flag {
                            break;
                        }
                        self.continue_flag = false;
                    }
                } else if self.is_associative_array(&name) {
                    // §7.8.2: the loop variable takes the AA's index
                    // type (width + signedness). A signed shortint key
                    // like -11823 must stay signed, not be re-parsed as
                    // u64 (which fails and yields 0) or widened to 32.
                    let (kw, ks_sign) = self.assoc_index_width_for(&name).unwrap_or((32, false));
                    self.widths.insert(var.name.clone(), kw);
                    let prefix = format!("{}[", name);
                    let mut struct_elem: Option<bool> = None;
                    // An assoc-of-collections (e.g. `bq_t all[int]`
                    // where `bq_t = Base[$]`) stores elements as
                    // `all[5][0]` etc. — extract the key up to the
                    // FIRST `]` and dedup so multiple sub-elements
                    // don't each produce a phantom key.
                    let mut keys: Vec<String> = self
                        .signals
                        .elem_keys_with_prefix(&prefix)
                        .filter_map(|k| {
                            let rest = k.strip_prefix(prefix.as_str())?;
                            let se = Self::key_has_member_suffix(rest)
                                && *struct_elem
                                    .get_or_insert_with(|| self.queue_elem_struct(&name).is_some());
                            Self::assoc_elem_key_seg(rest, se).map(|s| s.to_string())
                        })
                        .collect();
                    keys.sort();
                    keys.dedup();
                    let is_str = self
                        .module
                        .associative_arrays
                        .get(&*name)
                        .copied()
                        .unwrap_or(false)
                        || self.is_string_keyed_array(&name);
                    // Numeric key order for integer-keyed arrays (see
                    // note above); lexicographic would scramble order.
                    if is_str {
                        keys.sort();
                    } else {
                        keys.sort_by_key(|k| k.parse::<i64>().unwrap_or(0));
                    }
                    for key in keys {
                        if self.finished {
                            break;
                        }
                        let kv = if is_str {
                            Value::from_string(&key)
                        } else if kw > 64 {
                            Value::from_str_radix(&key, 16, kw)
                        } else {
                            // §7.9 + §7.8.2: SIGNED parse (negative keys
                            // must not collapse to 0) at the DECLARED
                            // index width/signedness.
                            let mut v = Value::from_u64(key.parse::<i64>().unwrap_or(0) as u64, kw);
                            v.is_signed = ks_sign;
                            v
                        };
                        self.set_loop_var_aliased(var_scope.as_deref(), &var.name, kv);
                        self.continue_flag = false;
                        *iterations = iterations.saturating_add(1);
                        self.exec_statement(body);
                        // `continue` is per-iteration (clear it); a
                        // `break`/`return` (both set break_flag) exits
                        // the loop and propagates — do NOT clear it.
                        if self.break_flag {
                            break;
                        }
                        self.continue_flag = false;
                    }
                } else if self.module.dynamic_arrays.contains(&*name)
                    || self.is_per_spec_dynamic_static(&name)
                {
                    // Queue / dynamic array: iterate 0..current size.
                    let size = self.get_queue_size(&name);
                    for i in 0..size {
                        if self.finished {
                            break;
                        }
                        self.set_loop_var_aliased(
                            var_scope.as_deref(),
                            &var.name,
                            Self::signed_loop_val(i as i64),
                        );
                        self.continue_flag = false;
                        *iterations = iterations.saturating_add(1);
                        self.exec_statement(body);
                        // `continue` is per-iteration (clear it); a
                        // `break`/`return` (both set break_flag) exits
                        // the loop and propagates — do NOT clear it.
                        if self.break_flag {
                            break;
                        }
                        self.continue_flag = false;
                    }
                } else if let Some(&(lo, hi, _)) = self.module.arrays.get(&*name) {
                    // Fixed unpacked array: iterate its declared index range.
                    let descending = self.module.descending_arrays.contains(&*name);
                    let mut idx = lo;
                    while idx <= hi {
                        if self.finished {
                            break;
                        }
                        let v = if descending { hi - (idx - lo) } else { idx };
                        self.set_loop_var_aliased(
                            var_scope.as_deref(),
                            &var.name,
                            Self::signed_loop_val(v),
                        );
                        self.continue_flag = false;
                        *iterations = iterations.saturating_add(1);
                        self.exec_statement(body);
                        // `continue` is per-iteration (clear it); a
                        // `break`/`return` (both set break_flag) exits
                        // the loop and propagates — do NOT clear it.
                        if self.break_flag {
                            break;
                        }
                        self.continue_flag = false;
                        idx += 1;
                    }
                } else if self.string_signals.contains(&*name) {
                    // foreach over a string iterates its characters
                    // [0..len). Use the CONTENT length: a string is held
                    // in a fixed-width container (e.g. 1024 bits), so
                    // `width/8` is the container size (128), not the
                    // string length — the loop ran 128 times over "hi".
                    let len = self.eval_expr(array).sv_string_bytes().len() as u64;
                    for i in 0..len {
                        if self.finished {
                            break;
                        }
                        self.set_loop_var_aliased(
                            var_scope.as_deref(),
                            &var.name,
                            Self::signed_loop_val(i as i64),
                        );
                        self.continue_flag = false;
                        *iterations = iterations.saturating_add(1);
                        self.exec_statement(body);
                        if self.break_flag {
                            break;
                        }
                        self.continue_flag = false;
                    }
                } else if let Some(&(l, r)) = self
                    .module
                    .packed_full_dims
                    .get(&*name)
                    .filter(|d| d.len() >= 2)
                    .and_then(|d| d.first())
                {
                    // §12.7.3 (measured): a MULTI-packed-dim vector
                    // (`logic [3:0][7:0] w`) iterates its OUTERMOST
                    // packed dimension (4 iterations, not 32 bits),
                    // left→right — [3:0] visits 3,2,1,0.
                    let n = (l - r).abs() + 1;
                    let step: i64 = if l >= r { -1 } else { 1 };
                    let mut idx = l;
                    for _ in 0..n {
                        if self.finished {
                            break;
                        }
                        self.set_loop_var_aliased(
                            var_scope.as_deref(),
                            &var.name,
                            Self::signed_loop_val(idx),
                        );
                        self.continue_flag = false;
                        *iterations = iterations.saturating_add(1);
                        self.exec_statement(body);
                        if self.break_flag {
                            break;
                        }
                        self.continue_flag = false;
                        idx += step;
                    }
                } else {
                    // Bit-by-bit foreach over a packed vector.
                    let size = self.lookup_signal_width(&name).unwrap_or(1) as u64;
                    for i in 0..size {
                        if self.finished {
                            break;
                        }
                        self.set_loop_var_aliased(
                            var_scope.as_deref(),
                            &var.name,
                            Self::signed_loop_val(i as i64),
                        );
                        self.continue_flag = false;
                        *iterations = iterations.saturating_add(1);
                        self.exec_statement(body);
                        // `continue` is per-iteration (clear it); a
                        // `break`/`return` (both set break_flag) exits
                        // the loop and propagates — do NOT clear it.
                        if self.break_flag {
                            break;
                        }
                        self.continue_flag = false;
                    }
                }
            }
        }
        self.auto_loop_vars.truncate(fe_auto_len);
        self.restore_loop_vars(&fe_saved);
        if !self.return_flag && self.disable_target.is_none() {
            self.break_flag = false;
        }
    }

    /// Outlined from `exec_statement` so the dispatcher's stack frame
    /// stays small: this arm's locals inflated every call's frame (the
    /// dispatcher recursed with a 7.8 KB frame per level).
    #[inline(never)]
    /// §6.8/§10.7: a procedural local's declaration initializer is an
    /// assignment to the declared type — it takes the type's width (an
    /// unbased-unsized fill replicates), signedness and 2-state-ness, and a
    /// real converts. `byte unsigned v = 200` read -56 and `bit [3:0] b =
    /// 4'b1x0z` kept its x/z because the raw initializer value was stored.
    /// A `string` keeps its text unresized: resizing to the placeholder
    /// width padded it with NULs that later concatenations copied.
    pub(super) fn fit_local_decl_init(
        raw: Value,
        w: u32,
        data_type: &DataType,
        two_state: bool,
        is_real_type: bool,
        signed: bool,
    ) -> Value {
        if is_real_type {
            return if raw.is_real {
                raw
            } else {
                Value::from_f64(raw.to_f64())
            };
        }
        if matches!(
            data_type,
            crate::ast::types::DataType::Simple {
                kind: crate::ast::types::SimpleType::String,
                ..
            }
        ) {
            return raw;
        }
        let mut v = if raw.is_real {
            Self::real_to_int(raw.to_f64(), w)
        } else {
            raw.resize_for_assign(w)
        };
        if two_state && v.has_xz() {
            v = v.to_two_state();
        }
        v.is_signed = signed;
        v
    }

    /// Outlined from `exec_statement` so the dispatcher's stack frame
    /// stays small: this arm's locals inflated every call's frame (the
    /// dispatcher recursed with a 7.8 KB frame per level).
    #[inline(never)]
    pub(super) fn exec_stmt_var_decl(
        &mut self,
        _stmt: &Statement,
        data_type: &DataType,
        lifetime: &Option<crate::ast::types::Lifetime>,
        declarators: &[VarDeclarator],
    ) {
        // §6.20.2 / §8.25: a local typed by a TYPE PARAMETER of the method's
        // class (`T tmp;`) has the type bound in this specialization.
        let tp_param = self.local_type_param_concrete(data_type);
        // §8.25.1: a class-scope typedef reached through a specialization.
        let tp_scoped = if tp_param.is_none() {
            self.scoped_typedef_concrete(data_type)
        } else {
            None
        };
        let tp_concrete = tp_param.or_else(|| tp_scoped.clone());
        let tp_width = tp_concrete
            .as_deref()
            .and_then(|c| self.concrete_type_width(c));
        // §8.25: a local sized by a class parameter or localparam
        // (`bit [N-1:0] x;` in a method of `C #(N)`) takes the executing
        // specialization's value; the module's parameter table has no `N`
        // (#681).
        let w0 = match tp_width
            .or_else(|| self.bound_agg_width(data_type))
            .or_else(|| self.class_param_sized_width(data_type))
        {
            Some(w) => w,
            None => self.resolve_type_width_in_process_scope(data_type),
        };
        // A class-handle-typed variable holds a 32-bit handle, not
        // the class's content layout (which `resolve_type_width`
        // may report as 0 for a class with only methods/statics).
        // A 0 width is never valid for a variable.
        let w = if w0 == 0 { 32 } else { w0 };
        // A built-in scalar or a plain class type: the packed-layout,
        // struct, enum, typedef and type-parameter probes below all come
        // back empty for it (a plain class resolves to itself).
        let class_ref = self.plain_class_ref(data_type);
        let plain_class = class_ref.is_some();
        let plain = plain_class || Self::type_is_plain_scalar(data_type);
        // §6.8/§6.18: resolve typedef aliases before deciding 2- vs
        // 4-state — `uvm_reg_data_t value_adjust;` (typedef of `bit
        // [63:0]`) must initialize to 0, not x (UVM's field
        // read-modify-write ORs into it, so an x seed poisoned the
        // whole register write). A class handle defaults to null either
        // way.
        let two_state = !plain_class
            && match tp_concrete.as_deref().filter(|_| tp_width.is_some()) {
                Some(c) => self.concrete_type_two_state(c),
                None => super::elaborate::is_type_two_state_resolved(
                    data_type,
                    &self.module.typedef_types,
                ),
            };
        // LRM §8.4: an uninitialized class handle defaults to
        // `null`. Treat class-handle types and `chandle` as
        // two-state-zero so `if (h == null)` works without an
        // explicit `= null`. We detect class handles by
        // resolving the type name against `module.classes`.
        let is_class_handle = match data_type {
            crate::ast::types::DataType::TypeReference { .. } => {
                plain_class || self.type_ref_is_class_handle(data_type)
            }
            crate::ast::types::DataType::Simple {
                kind: crate::ast::types::SimpleType::Chandle,
                ..
            } => true,
            _ => false,
        };
        // LRM §8.4: a class handle defaults to `null`. A class-handle
        // type whose name is a typedef/parameterized alias (e.g.
        // `resource_types::rsrc_q_t` → `queue#(...)`) isn't
        // found directly in `module.classes`, so `is_class_handle` is
        // false — but its width resolves to 0 (no data layout). Treat a
        // 0-width type as a handle and default it to zero, so an
        // uninitialized `rsrc_q_t rq; if (rq == null) rq = new();` works
        // inside a method (else `rq` reads X, `rq==null` is X, the
        // `new()` is skipped, and X is stored — breaking resource
        // pool population).
        // A class-scoped typedef (e.g.
        // `resource_types::rsrc_q_t` → `queue#(...)`, declared
        // INSIDE a class) is registered in neither `typedefs` (the width
        // map) nor `typedef_types`, so `resolve_type_width` fell back to
        // 32 and `is_class_handle` is false. A bare TypeReference naming
        // neither a known width-typedef nor a known class is almost
        // certainly such a handle alias — default it to null so the
        // ubiquitous `rsrc_q_t rq; if (rq == null) rq = new();` idiom
        // works inside a method (else `rq` reads X, `rq==null` is X, the
        // `new()` is skipped, and X is stored — which silently broke
        // resource pool population).
        let unknown_typeref_handle = !two_state
            && !is_class_handle
            && match data_type {
                crate::ast::types::DataType::TypeReference {
                    name, dimensions, ..
                } => {
                    dimensions.is_empty()
                        && !self.module.typedefs.contains_key(&name.name.name)
                        && !self.module.typedef_types.contains_key(&name.name.name)
                }
                _ => false,
            };
        // §6.4: `real`/`shortreal` variables default to 0.0.
        let is_real_type = super::elaborate::is_type_real(data_type);
        // §6.11.3: the declared signedness every declarator's value and
        // later reads carry.
        let decl_signed = !plain_class
            && match tp_scoped.as_deref() {
                Some(c) => self.concrete_type_signed(c) == Some(true),
                None => self.type_is_signed_concrete(data_type),
            };
        // §25.9 / IEEE 1800-2023 Table 6-7: a virtual-interface handle
        // defaults to `null`, also when its type is a typedef alias
        // (`vif_t v;`) or an explicit `virtual bus_if v` declaration.
        let is_vif_handle = !is_class_handle && Self::dt_is_virtual_iface(data_type, &self.module);
        let mut default_v = if is_real_type {
            Value::from_f64(0.0)
        } else if two_state || is_class_handle || w0 == 0 || unknown_typeref_handle || is_vif_handle
        {
            Value::zero(w)
        } else {
            Value::new(w)
        };
        // §6.11.3: the initial cell carries its declared signing before its
        // first assignment. Frame-local stores use this stable type stamp.
        default_v.is_signed = decl_signed;
        // §6.21: a local of an `initial`/`always`/`final` block is static
        // unless declared `automatic` — one variable, initialized once (its
        // initializer is not re-run when the block is entered again). Such a
        // block has no call frame; a subroutine or class method has one.
        let static_block = !matches!(lifetime, Some(crate::ast::types::Lifetime::Automatic))
            && !self.in_const_param_eval
            && self.current_static_task.is_none()
            && self.current_auto_task.is_none()
            && !matches!(self.class_context_stack.last(), Some(Some(_)))
            && self.in_process_block_scope();
        // An initializer without an explicit `static` is an error (the
        // variable is implicitly static, §6.21); such a declaration keeps
        // running each time, as an automatic one would.
        let explicit_static = matches!(lifetime, Some(crate::ast::types::Lifetime::Static));
        for d in declarators {
            // Set below for an unpacked collection; never carried over.
            self.pending_local_shape = None;
            if explicit_static
                && !self.static_local_syncs.is_empty()
                && self.hoisted_sub_static_names.contains(&d.name.name)
            {
                continue;
            }
            if static_block && (explicit_static || d.init.is_none()) {
                // Done already? (Allocation-free once it is: the activation
                // after the first.)
                let names = self
                    .static_block_decls
                    .entry((d.name.span.start, d.name.span.end))
                    .or_default();
                if names.iter().any(|n| *n == d.name.name) {
                    continue;
                }
                names.push(d.name.name.clone());
            }
            // Every metadata key this declaration writes is its name or
            // `name.member` (or a `@` per-call key, never a formal's name).
            self.meta_note(&d.name.name);
            // A fresh local declaration shadows any prior global-scoped
            // collection registration left over from another frame's
            // same-named local (subroutine locals use bare names). Reset
            // the array/queue/assoc bookkeeping so e.g. a scalar `string
            // str` after a previous `string str[$]` isn't still treated
            // as a queue.
            {
                let nm = d.name.name.as_str();
                if !self.decl_shadow_log.is_empty() {
                    self.note_decl_shadow(nm, data_type);
                }
                // If this local shares its bare name with a caller-frame
                // queue, preserve the caller's queue storage so it is
                // restored when this subroutine returns (queue locals are
                // global-by-bare-name; see `queue_frame_saves`).
                let saved = self.save_queue_local(nm, true);
                if !saved {
                    self.module.dynamic_arrays.remove(nm);
                    self.module.arrays.remove(nm);
                    self.module.descending_arrays.remove(nm);
                    self.module.queue_max_sizes.remove(nm);
                }
                self.module.associative_arrays.remove(nm);
                self.module.packed_signal_elem_widths.remove(nm);
                let mut size_key = std::mem::take(&mut self.queue_key_scratch);
                size_key.clear();
                size_key.push_str(nm);
                size_key.push_str(".size");
                self.signals.remove(&size_key);
                self.queue_key_scratch = size_key;
            }
            // Register a *packed*-struct local's field layout so member
            // access (`p.field`) aliases into the whole variable, as
            // module-level packed-struct signals already do. Without
            // this a local `pkt_t p;` keeps `p` and `p.field` in
            // separate storage (a whole write doesn't reach members and
            // vice-versa). Unpacked structs are intentionally excluded.
            // §6.17: an `event` local becomes a real event —
            // `->e` / `@(e)` resolve through module.events. Arrays
            // and queues of events register their BASE name so
            // element refs (`ev_arr[i]`) are recognized as events.
            if matches!(
                data_type,
                crate::ast::types::DataType::Simple {
                    kind: crate::ast::types::SimpleType::Event,
                    ..
                }
            ) {
                self.module.events.insert(d.name.name.clone());
            }
            // Declared ELEMENT type of a local. `var_decl_types` holds
            // the element type for a collection, so this is the same
            // `data_type` either way — the unpacked dims live on the
            // declarator. Registering it only for SCALAR locals meant a
            // local QUEUE OF STRUCTS had no element type, so
            // `queue_elem_struct` returned None and `push_back` fell
            // back to a packed scalar copy: an unpacked struct has no
            // container signal, so every member was lost and the queue
            // yielded zeros. Module-scope queues were unaffected because
            // elaboration registers them. (A recurring name keeps its key:
            // only the value is replaced.)
            match self.module.var_decl_types.get_mut(d.name.name.as_str()) {
                Some(slot) => *slot = data_type.clone(),
                None => {
                    self.module
                        .var_decl_types
                        .insert(d.name.name.clone(), data_type.clone());
                }
            }
            // §7.4.1: a packed-of-packed local (`u8_vec16_t y;`)
            // needs its ELEMENT width registered, or `y[i]` reads
            // and writes degrade to single-BIT selects. This was the
            // long-standing AST bug behind self-checking TBs whose
            // reference functions silently agreed with equally-broken
            // DUT paths (pixel-conversion battery).
            if d.dimensions.is_empty() && !plain {
                if let Some(ew) = self.ast_decl_elem_width(data_type) {
                    if ew > 0 && w > ew && w % ew == 0 {
                        self.module
                            .packed_signal_elem_widths
                            .insert(d.name.name.clone(), ew);
                    }
                }
            }
            if d.dimensions.is_empty() && plain {
                // No element width (removed above) and no struct or enum
                // layout; only a vector's own non-normalized range is kept.
                match super::elaborate::packed_full_dims_of(data_type, &self.module.parameters) {
                    Some(fdims) => {
                        self.module
                            .packed_full_dims
                            .insert(d.name.name.clone(), fdims);
                    }
                    None => {
                        self.module.packed_full_dims.remove(&d.name.name);
                    }
                }
            } else if d.dimensions.is_empty() {
                // Packed multi-D local (`logic [1:0][3:0] m;`): record
                // the per-element width so `m[i]` is an element slice,
                // not a bit-select — module-scope decls already do.
                // The packed dims may live in the TYPEDEF rather than on
                // the declaration (`typedef T1 [3:0] T2; T2 v;`) — see
                // `typedef_one_step`. Without this the element width was
                // never recorded and `v[i]` degraded to a BIT select.
                let dt1_local = self.typedef_one_step(data_type);
                if let Some(elem_w) = super::elaborate::packed_inner_elem_width(
                    data_type,
                    &self.module.parameters,
                    &self.module.typedefs,
                )
                .or_else(|| {
                    super::elaborate::packed_inner_elem_width(
                        &dt1_local,
                        &self.module.parameters,
                        &self.module.typedefs,
                    )
                }) {
                    self.module
                        .packed_signal_elem_widths
                        .insert(d.name.name.clone(), elem_w);
                } else {
                    self.module.packed_signal_elem_widths.remove(&d.name.name);
                }
                if let Some(fdims) =
                    super::elaborate::packed_full_dims_of(data_type, &self.module.parameters)
                        .or_else(|| {
                            super::elaborate::packed_full_dims_of(
                                &dt1_local,
                                &self.module.parameters,
                            )
                        })
                        .or_else(|| {
                            super::elaborate::packed_full_dims_chained(
                                data_type,
                                &self.module.parameters,
                                &self.module.typedef_types,
                            )
                        })
                        .or_else(|| self.struct_array_full_dims(data_type))
                {
                    self.module
                        .packed_full_dims
                        .insert(d.name.name.clone(), fdims);
                } else {
                    self.module.packed_full_dims.remove(&d.name.name);
                }
                // Per-field element widths (`u.n[i]` where the member
                // is itself a packed array) — mirrors the module-scope
                // registration keyed "var.field". A member whose own type
                // is a struct carries aggregate members of its own:
                // `u.inner.arr[i]` needs `inner.arr`'s element stride to
                // place the trailing select, so the walk recurses (bounded)
                // exactly like `register_struct_member_packed_dims` does
                // for signals. Depth-1-only registration left that stride
                // unknown and the depth-2 select degraded to a 1-BIT read.
                if let crate::ast::types::DataType::Struct(su) = self.resolve_dt(data_type) {
                    let mut pending: Vec<(String, crate::ast::types::StructUnionType)> =
                        vec![(d.name.name.clone(), su)];
                    let mut budget = 4096usize;
                    while let Some((prefix, su)) = pending.pop() {
                        if budget == 0 {
                            break;
                        }
                        budget -= 1;
                        for m in &su.members {
                            let ew = super::elaborate::packed_inner_elem_width(
                                &m.data_type,
                                &self.module.parameters,
                                &self.module.typedefs,
                            );
                            let fd = super::elaborate::packed_full_dims_of(
                                &m.data_type,
                                &self.module.parameters,
                            );
                            for mdecl in &m.declarators {
                                let key = format!("{}.{}", prefix, mdecl.name.name);
                                // The member keys are formal metadata too
                                // (see `meta_note`).
                                self.meta_note(&key);
                                // Re-registration REPLACES: a later local
                                // reusing the same `var.field` name must not
                                // keep the earlier declaration's geometry
                                // (two block-local structs whose same-named
                                // member has different packed dims).
                                if let Some(ew) = ew {
                                    self.note_elem_width_key(&key);
                                    self.module
                                        .packed_signal_elem_widths
                                        .insert(key.clone(), ew);
                                } else {
                                    self.module.packed_signal_elem_widths.remove(&key);
                                }
                                if let Some(fd) = &fd {
                                    self.module.packed_full_dims.insert(key.clone(), fd.clone());
                                } else {
                                    self.module.packed_full_dims.remove(&key);
                                }
                                if let crate::ast::types::DataType::Struct(nested) =
                                    self.resolve_dt(&m.data_type)
                                {
                                    pending.push((key, nested));
                                }
                            }
                        }
                    }
                }
                // §6.19.6: an inline-enum LOCAL (a typedef'd enum
                // local arrives here already folded to its enum
                // type) keys its member list by its own name.
                if matches!(data_type, crate::ast::types::DataType::Enum(_)) {
                    if let Some(m) = super::elaborate::anon_enum_members_ordered(
                        data_type,
                        &self.module.parameters,
                    ) {
                        self.type_tables_gen += 1;
                        self.module.enum_members.insert(d.name.name.clone(), m);
                        if let Some(set) = self.plain_class_types.borrow_mut().as_mut() {
                            set.remove(&d.name.name);
                        }
                    }
                }
                if let Some(fields) = super::elaborate::packed_struct_field_layout(
                    data_type,
                    &self.module.parameters,
                    &self.module.typedefs,
                    &self.module.typedef_types,
                ) {
                    if !fields.is_empty() {
                        if env_set_cached!("XEZIM_PSDBG") {
                            eprintln!("[PSDBG] register packed local {}", d.name.name);
                        }
                        self.module
                            .packed_struct_fields
                            .insert(d.name.name.clone(), fields);
                    }
                }
                // Declaration-order member names for `%p` (§21.2.1.7).
                if let Some(names) =
                    super::elaborate::struct_member_names(data_type, &self.module.typedef_types)
                {
                    if !names.is_empty() {
                        self.module
                            .struct_members
                            .insert(d.name.name.clone(), names);
                    }
                }
            } else {
                // §7.2.1/§7.4: an ARRAY local of a packed struct element
                // (`s_t a [];` in a task or function) gets the element's
                // field layout, as a module-scope array does in
                // elaboration: `a[i].f` slices the element. Without it a
                // field write was dropped and a field read gave 0.
                self.local_elem_layout = None;
                if let Some(fields) = super::elaborate::packed_struct_field_layout(
                    data_type,
                    &self.module.parameters,
                    &self.module.typedefs,
                    &self.module.typedef_types,
                )
                .filter(|f| !f.is_empty())
                {
                    let prev = self
                        .module
                        .packed_struct_fields
                        .insert(d.name.name.clone(), fields.clone());
                    // An array local to an automatic task lives under a
                    // per-call key (`declare_local_dyn`).
                    self.local_elem_layout = Some((fields, d.name.name.clone(), prev));
                }
                // §7.4/§20.7: an ARRAY local of a packed multi-dimensional
                // element (`logic [7:0][3:0] pk [2][3];`) keeps its packed
                // geometry, as a module-scope array does: the element width
                // makes `pk[i][j][k]` a lane select and the full dims answer
                // `$size(pk, 3)` / `$dimensions(pk)`. Only the flat element
                // width was kept, so the packed dims read as one 32-bit one.
                match super::elaborate::packed_full_dims_of(data_type, &self.module.parameters)
                    .or_else(|| {
                        super::elaborate::packed_full_dims_chained(
                            data_type,
                            &self.module.parameters,
                            &self.module.typedef_types,
                        )
                    }) {
                    Some(fdims) if fdims.len() >= 2 => {
                        if let Some(ew) = super::elaborate::packed_inner_elem_width(
                            data_type,
                            &self.module.parameters,
                            &self.module.typedefs,
                        )
                        .or_else(|| self.ast_decl_elem_width(data_type))
                        {
                            self.module
                                .packed_signal_elem_widths
                                .insert(d.name.name.clone(), ew);
                        }
                        self.module
                            .packed_full_dims
                            .insert(d.name.name.clone(), fdims);
                        self.array_local_packed_dims.insert(d.name.name.clone());
                    }
                    // Only a name this arm registered can hold a stale
                    // geometry: the common array local pays no lookup.
                    _ if !self.array_local_packed_dims.is_empty()
                        && self.array_local_packed_dims.remove(&d.name.name) =>
                    {
                        self.module.packed_full_dims.remove(&d.name.name);
                    }
                    _ => {}
                }
            }
            // §6.18/§6.20.3: a typedef'd (or type-param-bound) local
            // carries its unpacked dims on the TYPE — `my_array_t a;`
            // declared inside a task elaborated as a scalar.
            let typedef_dims: Vec<crate::ast::types::UnpackedDimension> =
                if d.dimensions.is_empty() && !plain_class {
                    if let crate::ast::types::DataType::TypeReference { name, .. } = data_type {
                        let tn = &name.name.name;
                        let concrete = self
                            .resolve_type_param_binding(tn)
                            .unwrap_or_else(|| tn.clone());
                        self.module
                            .typedef_unpacked_dims
                            .get(&concrete)
                            .cloned()
                            .or_else(|| {
                                // CLASS-LOCAL typedef: an explicit
                                // `C::t_t v;` scope, else the class
                                // whose method is executing — a
                                // `Ph::d_t dd;` local bound as a
                                // scalar, so a later `ref` call saw
                                // no collection to write back to.
                                let start = name
                                    .scopes
                                    .first()
                                    .map(|s| s.name.name.clone())
                                    .or_else(|| self.class_context_stack.last().cloned().flatten())
                                    .or_else(|| {
                                        self.this_stack.last().copied().flatten().and_then(|h| {
                                            self.heap
                                                .get(h)
                                                .and_then(|x| x.as_ref())
                                                .map(|i| i.class_name.clone())
                                        })
                                    })?;
                                Self::typedef_dims_via_tables(&self.module, &start, &concrete)
                            })
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                };
            let dims: &Vec<crate::ast::types::UnpackedDimension> = if typedef_dims.is_empty() {
                &d.dimensions
            } else {
                &typedef_dims
            };
            // §7.4.2 / §8.23: a dimension sized by a constant of the
            // enclosing scope (`l[N]`, `l[N * 2]`, `l[cls::N]`) — the
            // context-free evaluation below cannot resolve it.
            let scope_sized: Option<Vec<crate::ast::types::UnpackedDimension>> =
                if dims.iter().any(Self::local_dim_needs_scope) {
                    self.scope_sized_local_dims(dims)
                } else {
                    None
                };
            let dims = scope_sized.as_ref().unwrap_or(dims);
            // §7.4.5: its declared shape, for a per-call storage key. A
            // block-scope local keeps its bare name, recorded at elaboration.
            self.pending_local_shape = CollShape::of(dims, |kdt| {
                super::elaborate::coll_key_kind(
                    kdt,
                    Some(&self.module.parameters),
                    Some(&self.module.typedefs),
                    Some(&self.module.typedef_types),
                )
            });
            // §7.4.5 / §7.8: a local unpacked array whose ELEMENT is a
            // collection (`int q[3][$]`, `int v[2][string][string]`), as a
            // module-scope one is registered in elaboration: the fixed outer
            // shape, and every element its own queue / associative array.
            // Without it the trailing dimensions were dropped, so
            // `v[0]["A"] = 7` and `q[1].push_back(4)` were lost.
            if let Some(k) = super::elaborate::coll_elem_split(dims)
                && self.declare_local_coll_array(d, data_type, dims, k, w, lifetime.clone())
            {
                continue;
            }
            // A local MULTI-dimensional array was registered from its
            // first dimension only, so `int n[2][2]` became a 1-D array
            // of double-width elements: `n[1][0]` read 0 and the
            // declaration initializer could not be spread.
            if dims.len() >= 2 {
                use crate::ast::types::UnpackedDimension as UD;
                let shape: Option<Vec<(i64, i64)>> = dims
                    .iter()
                    .map(|dim| match dim {
                        UD::Range { left, right, .. } => {
                            let l = super::elaborate::const_eval_i64_with_params(left, None)?;
                            let r = super::elaborate::const_eval_i64_with_params(right, None)?;
                            Some((l.min(r), l.max(r)))
                        }
                        UD::Expression { expr, .. } => {
                            let n = super::elaborate::const_eval_i64_with_params(expr, None)?;
                            if n > 0 { Some((0, n - 1)) } else { None }
                        }
                        _ => None,
                    })
                    .collect();
                if let Some(shape) = shape.filter(|sh| {
                    sh.iter()
                        .map(|&(lo, hi)| (hi - lo + 1).max(0))
                        .product::<i64>()
                        <= (1 << 20)
                }) {
                    // §6.21: automatic like the 1-D fixed local below — a
                    // per-invocation key in a class method or automatic task.
                    let bare = d.name.name.clone();
                    let in_method = matches!(self.class_context_stack.last(), Some(Some(_)));
                    let name = if (in_method || self.current_auto_task.is_some())
                        && self.current_static_task.is_none()
                        && !matches!(lifetime, Some(crate::ast::types::Lifetime::Static))
                    {
                        self.declare_local_dyn(&bare)
                    } else {
                        bare
                    };
                    self.adopt_local_shape(&name);
                    match shape.len() {
                        2 => {
                            self.module
                                .arrays_2d
                                .insert(name.clone(), (shape[0], shape[1], w));
                        }
                        _ => {
                            self.module
                                .arrays_nd
                                .insert(name.clone(), (shape.clone(), w));
                        }
                    }
                    // Every element, row-major.
                    let mut suffixes: Vec<String> = vec![String::new()];
                    for &(lo, hi) in &shape {
                        let mut next = Vec::new();
                        for pre in &suffixes {
                            for i in lo..=hi {
                                next.push(format!("{}[{}]", pre, i));
                            }
                        }
                        suffixes = next;
                    }
                    let is_string_elem = matches!(
                        data_type,
                        crate::ast::types::DataType::Simple {
                            kind: crate::ast::types::SimpleType::String,
                            ..
                        }
                    );
                    if is_string_elem {
                        self.string_signals.insert(name.clone());
                    }
                    for sfx in suffixes {
                        let elem = format!("{}{}", name, sfx);
                        let seed = if is_string_elem {
                            Value::from_string("")
                        } else {
                            default_v.clone()
                        };
                        self.signals.insert(elem.clone(), seed);
                        self.widths.insert(elem, w);
                    }
                    self.widths.insert(name.clone(), w);
                    self.module
                        .var_decl_types
                        .insert(name.clone(), data_type.clone());
                    if let Some(init) = &d.init {
                        let lvalue = crate::ast::expr::Expression::new(
                            crate::ast::expr::ExprKind::Ident(
                                crate::ast::expr::HierarchicalIdentifier {
                                    root: None,
                                    path: vec![crate::ast::expr::HierPathSegment {
                                        name: crate::ast::Identifier {
                                            name: name.clone(),
                                            span: d.name.span,
                                        },
                                        selects: Vec::new(),
                                    }],
                                    span: d.name.span,
                                    cached_signal_id: std::cell::Cell::new(None),
                                    cached_resolved_name: std::cell::OnceCell::new(),
                                },
                            ),
                            d.name.span,
                        );
                        let assign = crate::ast::stmt::Statement::new(
                            crate::ast::stmt::StatementKind::BlockingAssign {
                                lvalue,
                                rvalue: init.clone(),
                            },
                            d.name.span,
                        );
                        self.exec_statement(&assign);
                    }
                    continue;
                }
            }
            let mut range: Option<(i64, i64)> = None;
            let mut descending = false;
            if let Some(first) = dims.first() {
                use crate::ast::types::UnpackedDimension;
                match first {
                    UnpackedDimension::Range { left, right, .. } => {
                        let l =
                            super::elaborate::const_eval_i64_with_params(left, None).unwrap_or(0);
                        let r =
                            super::elaborate::const_eval_i64_with_params(right, None).unwrap_or(0);
                        range = Some((l.min(r), l.max(r)));
                        if l > r {
                            descending = true;
                        }
                    }
                    UnpackedDimension::Expression { expr, .. } => {
                        let n =
                            super::elaborate::const_eval_i64_with_params(expr, None).unwrap_or(0);
                        if n > 0 {
                            range = Some((0, n - 1));
                        }
                    }
                    UnpackedDimension::Unsized(_) | UnpackedDimension::Queue { .. } => {
                        // Register as dynamic array / queue (initially empty).
                        // §6.21: in a class method it is automatic — each
                        // invocation owns its storage, under the per-frame
                        // key the associative and fixed-size locals use.
                        // Keyed by its bare name, two concurrent calls
                        // shared one queue and each declaration emptied the
                        // other's.
                        let bare = d.name.name.clone();
                        let in_method = matches!(self.class_context_stack.last(), Some(Some(_)));
                        let name = if (in_method || self.current_auto_task.is_some())
                            && self.current_static_task.is_none()
                            && !matches!(lifetime, Some(crate::ast::types::Lifetime::Static))
                        {
                            self.declare_local_dyn(&bare)
                        } else {
                            bare.clone()
                        };
                        self.adopt_local_shape(&name);
                        self.module.arrays.insert(name.clone(), (0, -1, w));
                        self.module.dynamic_arrays.insert(name.clone());
                        if name != bare {
                            // The element type, under the per-call key too
                            // (a queue of structs spreads its members by it).
                            self.module
                                .var_decl_types
                                .insert(name.clone(), data_type.clone());
                            if let crate::ast::types::DataType::TypeReference { name: tn, .. } =
                                data_type
                            {
                                if self.module.classes.contains_key(&tn.name.name) {
                                    self.module
                                        .array_elem_class
                                        .insert(name.clone(), tn.name.name.clone());
                                }
                            }
                        }
                        // A QUEUE-typed local (`int a[$];`, possibly with
                        // a bound `[$:N]`) keeps the queue semantics -
                        // index-at-size appends, size tracking on
                        // `q[i] = v` back-fill - not the discard-out-of-
                        // range rule of an unallocated dynamic array.
                        // Mirror the module-scope classification.
                        if matches!(first, UnpackedDimension::Queue { .. }) {
                            self.module.queue_vars.insert(name.clone());
                        }
                        self.widths.insert(name.clone(), w);
                        self.set_queue_size(&name, 0);
                        // §7.4.5: a LOCAL `T a[][16]` / `a[$][16]` —
                        // dynamic outer with fixed trailing dims —
                        // registers the 2-D/N-D shape like a module-
                        // scope one, else the trailing dims are
                        // dropped and `a[i][j]` misaddresses.
                        if dims.len() >= 2
                            && dims[1..].iter().all(|dm| {
                                matches!(
                                    dm,
                                    UnpackedDimension::Range { .. }
                                        | UnpackedDimension::Expression { .. }
                                )
                            })
                        {
                            let inner: Option<Vec<(i64, i64)>> = dims[1..]
                                .iter()
                                .map(|dm| {
                                    super::elaborate::extract_array_range(
                                        std::slice::from_ref(dm),
                                        &self.module.parameters,
                                    )
                                })
                                .collect();
                            if let Some(inner) = inner
                                .filter(|sh| sh.iter().all(|&(lo, hi)| hi >= lo && hi - lo < 4096))
                            {
                                self.module.arrays.remove(&name);
                                if inner.len() == 1 {
                                    self.module
                                        .arrays_2d
                                        .insert(name.clone(), ((0, 63), inner[0], w));
                                } else {
                                    let mut shape = vec![(0i64, 63i64)];
                                    shape.extend(inner.iter().cloned());
                                    self.module.arrays_nd.insert(name.clone(), (shape, w));
                                }
                                if let crate::ast::types::DataType::TypeReference {
                                    name: tn, ..
                                } = data_type
                                {
                                    self.module
                                        .array_elem_class
                                        .insert(name.clone(), tn.name.name.clone());
                                }
                            }
                        }
                        // A `string q[$]` queue: mark the name so its
                        // elements are treated as strings (`%p`, `%s`,
                        // concat) — the element-string check keys off
                        // the queue name being in `string_signals`. A
                        // non-string decl must UNmark: the set is keyed
                        // by bare name, so a stale entry from another
                        // frame's same-named `string q[$]` would give
                        // this queue string treatment.
                        if matches!(
                            data_type,
                            crate::ast::types::DataType::Simple {
                                kind: crate::ast::types::SimpleType::String,
                                ..
                            }
                        ) {
                            self.string_signals.insert(name.clone());
                        } else {
                            self.forget_string_flag(&name);
                        }
                        // LRM §7.10 / §7.12.1 — if an initializer
                        // is provided (e.g. `int idx[$] =
                        // q.find_first_index with (…);`),
                        // synthesise a BlockingAssign so the queue
                        // is populated via the same path as a
                        // standalone assignment. Without this the
                        // init is silently dropped and the queue
                        // stays empty.
                        if let Some(init) = &d.init {
                            let lvalue = crate::ast::expr::Expression::new(
                                crate::ast::expr::ExprKind::Ident(
                                    crate::ast::expr::HierarchicalIdentifier {
                                        root: None,
                                        path: vec![crate::ast::expr::HierPathSegment {
                                            name: crate::ast::Identifier {
                                                name: bare.clone(),
                                                span: d.name.span,
                                            },
                                            selects: Vec::new(),
                                        }],
                                        span: d.name.span,
                                        cached_signal_id: std::cell::Cell::new(None),
                                        cached_resolved_name: std::cell::OnceCell::new(),
                                    },
                                ),
                                d.name.span,
                            );
                            let assign = crate::ast::stmt::Statement::new(
                                crate::ast::stmt::StatementKind::BlockingAssign {
                                    lvalue,
                                    rvalue: init.clone(),
                                },
                                d.name.span,
                            );
                            self.exec_statement(&assign);
                        }
                        continue;
                    }
                    UnpackedDimension::Associative {
                        data_type: key_dt, ..
                    } => {
                        // Function-local associative array (`int m[int]`,
                        // `int m[string]`). Register so indexed writes/reads
                        // resolve to the signal-keyed assoc storage instead
                        // of being mistaken for a scalar.
                        let bare = d.name.name.clone();
                        let name = if matches!(lifetime, Some(crate::ast::types::Lifetime::Static))
                        {
                            bare.clone()
                        } else {
                            self.declare_local_dyn(&bare)
                        };
                        self.adopt_local_shape(&name);
                        let is_string_key = key_dt.as_ref().is_some_and(|dt| {
                            matches!(
                                dt.as_ref(),
                                crate::ast::types::DataType::Simple {
                                    kind: crate::ast::types::SimpleType::String,
                                    ..
                                }
                            )
                        });
                        // Drop any stale elements left by a prior frame's
                        // same-named local before this fresh declaration —
                        // struct-element member leaves (`m[k].x`) included.
                        let pre = format!("{}[", name);
                        let stale: Vec<String> = self.signals.keys_with_elem_prefix(&pre);
                        for k in stale {
                            self.signals.remove(&k);
                        }
                        self.module
                            .associative_arrays
                            .insert(name.clone(), is_string_key);
                        if let Some(lv) = super::elaborate::assoc_inner_string_levels(dims) {
                            self.module.assoc_inner_string_keys.insert(name.clone(), lv);
                        }
                        self.widths.insert(name.clone(), w);
                        // §7.8.2 / §7.9: the key's width and signedness, as
                        // elaboration records them for a module-scope array.
                        // Without them a `foreach` over a local `int a[int]`
                        // bound the negative key -5 as 4294967291, and the
                        // element read through it missed.
                        // §8.25: a key sized by a class parameter
                        // (`bit seen[bit [N-1:0]]` in a method of `C #(N)`)
                        // takes the executing specialization's width (#681).
                        let class_key = key_dt.as_deref().and_then(|k| {
                            let w = self.class_param_sized_width(k)?;
                            let signed = matches!(
                                k,
                                crate::ast::types::DataType::IntegerVector {
                                    signing: Some(crate::ast::types::Signing::Signed),
                                    ..
                                }
                            );
                            Some((w, signed))
                        });
                        match class_key.or_else(|| {
                            super::elaborate::assoc_key_type_info(
                                key_dt.as_deref(),
                                &self.module.parameters,
                                &self.module.typedefs,
                                &self.module.typedef_types,
                            )
                            .0
                        }) {
                            Some(iw) => {
                                self.module.assoc_index_widths.insert(name.clone(), iw);
                            }
                            None => {
                                self.module.assoc_index_widths.remove(&name);
                            }
                        }
                        // §6.19.6/§7.8: a NAMED key type (`int aa[e_t];`)
                        // gives a `foreach (aa[k])` index variable that type,
                        // so `k.last`/`k.next`/`k.name` resolve against the
                        // enum. Elaboration records it for module-scope
                        // arrays only. Inside a subroutine frame it goes to
                        // the frame's overlay under `aa[]` (no variable can
                        // have that name), so a same-named array elsewhere
                        // keeps its own key type; an unnamed key type records
                        // "" there to shadow an outer entry.
                        let key_tn = key_dt
                            .as_ref()
                            .and_then(|dt| match dt.as_ref() {
                                crate::ast::types::DataType::TypeReference { name: tn, .. } => {
                                    Some(tn.name.name.clone())
                                }
                                _ => None,
                            })
                            .filter(|tn| {
                                self.module.enum_members.contains_key(tn)
                                    || self.module.typedef_types.contains_key(tn)
                            });
                        if self.local_type_stack.is_empty() {
                            match key_tn {
                                Some(tn) => {
                                    self.module.assoc_key_type_names.insert(bare.clone(), tn);
                                }
                                None => {
                                    self.module.assoc_key_type_names.remove(&bare);
                                }
                            }
                        } else {
                            let slot = format!("{}[]", bare);
                            self.record_local_typedef_type(&slot, key_tn.as_deref().unwrap_or(""));
                        }
                        // Record the ELEMENT type under the renamed
                        // storage key so `all[k] = new` can resolve
                        // the element class (incl. parameterized
                        // typedefs — UVM's sort_by_precedence local
                        // `rsrc_q_t all[int]`; issue #113).
                        self.module
                            .var_decl_types
                            .insert(name.clone(), data_type.clone());
                        if let crate::ast::types::DataType::TypeReference { name: tn, .. } =
                            data_type
                        {
                            self.module
                                .array_elem_class
                                .entry(name.clone())
                                .or_insert_with(|| tn.name.name.clone());
                        }
                        // A `string m[...]` (string-VALUED assoc): mark the
                        // name so its elements render as quoted strings under
                        // `%p` (the element-string check keys off the array
                        // name being in `string_signals`). This mirrors the
                        // string-queue marking above.
                        if matches!(
                            data_type,
                            crate::ast::types::DataType::Simple {
                                kind: crate::ast::types::SimpleType::String,
                                ..
                            }
                        ) {
                            self.string_signals.insert(name.clone());
                        } else {
                            self.forget_string_flag(&name);
                        }
                        // If the element type is a known class, record it
                        // so `assoc[k] = new()` constructs the right class
                        // (ranged arrays get this at elaborate time; assoc
                        // arrays were missing it).
                        if let crate::ast::types::DataType::TypeReference { name: en, .. } =
                            data_type
                        {
                            let ecn = en.name.name.clone();
                            if self.module.classes.contains_key(&ecn) {
                                self.module.array_elem_class.insert(name.clone(), ecn);
                            }
                        }
                        // A local ASSOCIATIVE array's declaration
                        // initializer (`uvm_object_wrapper m[string] =
                        // '{"a": w, default: d};` inside a method) was
                        // silently dropped — the branch registered the
                        // array then `continue`d, so every element read
                        // back null/X even for EXPLICIT keys. Route it
                        // through a BlockingAssign (as the queue and
                        // fixed-array branches do) so the pattern-
                        // aggregate spread applies write the keyed
                        // elements and record the default fallback.
                        if let Some(init) = &d.init {
                            let lvalue = crate::ast::expr::Expression::new(
                                crate::ast::expr::ExprKind::Ident(
                                    crate::ast::expr::HierarchicalIdentifier {
                                        root: None,
                                        path: vec![crate::ast::expr::HierPathSegment {
                                            name: crate::ast::Identifier {
                                                name: bare,
                                                span: d.name.span,
                                            },
                                            selects: Vec::new(),
                                        }],
                                        span: d.name.span,
                                        cached_signal_id: std::cell::Cell::new(None),
                                        cached_resolved_name: std::cell::OnceCell::new(),
                                    },
                                ),
                                d.name.span,
                            );
                            let assign = crate::ast::stmt::Statement::new(
                                crate::ast::stmt::StatementKind::BlockingAssign {
                                    lvalue,
                                    rvalue: init.clone(),
                                },
                                d.name.span,
                            );
                            self.exec_statement(&assign);
                        }
                        continue;
                    }
                }
            }
            if let Some((lo, hi)) = range {
                // §6.21: a fixed array local to a class method is automatic —
                // each invocation owns its storage. Keyed by its bare name it
                // was one array shared by every concurrent call (two
                // sequences' `item req_c[10]` overwrote each other) and
                // re-seeded under the other's feet. Give it the per-frame
                // key the associative locals already use.
                let bare = d.name.name.clone();
                let in_method = matches!(self.class_context_stack.last(), Some(Some(_)));
                let name = if (in_method || self.current_auto_task.is_some())
                    && self.current_static_task.is_none()
                    && !matches!(lifetime, Some(crate::ast::types::Lifetime::Static))
                {
                    self.declare_local_dyn(&bare)
                } else {
                    bare.clone()
                };
                self.adopt_local_shape(&name);
                if name == bare
                    && let Some(frame) = self.local_dyn.last_mut()
                    && !frame.iter().any(|(local, _)| local == &bare)
                {
                    frame.push((bare.clone(), name.clone()));
                }
                // §7.4.4 (#519): a fixed MULTI-dimensional local keeps its
                // full shape — it was registered from its first dimension
                // as a 1-D array, and the elaborated shape of a same-named
                // local of another subroutine stayed in the other table, so
                // a later `std::randomize` read whichever shape it met first.
                // Each declaration now owns the name in exactly one table.
                let nd_shape: Option<Vec<(i64, i64)>> = if dims.len() >= 2 {
                    dims.iter()
                        .map(|dm| {
                            super::elaborate::extract_array_range(
                                std::slice::from_ref(dm),
                                &self.module.parameters,
                            )
                        })
                        .collect::<Option<Vec<_>>>()
                        .filter(|sh| sh.iter().all(|&(l, h)| h >= l && h - l < 4096))
                } else {
                    None
                };
                if let Some(shape) = nd_shape {
                    self.module.arrays.remove(&name);
                    if shape.len() == 2 {
                        self.module
                            .arrays_2d
                            .insert(name.clone(), (shape[0], shape[1], w));
                        self.module.arrays_nd.remove(&name);
                    } else {
                        self.module.arrays_nd.insert(name.clone(), (shape, w));
                        self.module.arrays_2d.remove(&name);
                    }
                    continue;
                }
                self.module.arrays.insert(name.clone(), (lo, hi, w));
                self.module.arrays_2d.remove(&name);
                self.module.arrays_nd.remove(&name);
                if descending {
                    self.module.descending_arrays.insert(name.clone());
                }
                // A local FIXED array of STRINGS: mark the array name
                // (the element-string check keys off it, like the
                // string-queue branch) and seed every element as the
                // EMPTY string. Seeded as zero bits, an unset element
                // rendered under `%s` as a run of blanks, and the
                // declaration's `'{…}` pattern had no string-typed
                // element to land in — `string nm[3] = '{"a","b","c"}`
                // in a block printed nothing but padding.
                let is_string_elem = matches!(
                    data_type,
                    crate::ast::types::DataType::Simple {
                        kind: crate::ast::types::SimpleType::String,
                        ..
                    }
                );
                if is_string_elem {
                    self.string_signals.insert(name.clone());
                } else {
                    self.forget_string_flag(&name);
                }
                // §7.2/§7.4: an element of a local array of UNPACKED structs
                // is stored as its member leaves, as a module-scope one is —
                // every element of every member dimension (`a[1].mm[i][j]`).
                // A packed container per element had no leaf for a member
                // element select to land on.
                let elem_su = self.unpacked_struct_of(data_type);
                if elem_su.is_some() {
                    self.note_struct_root(&bare);
                    self.note_struct_root(&name);
                    self.note_struct_type(data_type);
                }
                for idx in lo..=hi {
                    let elem = format!("{}[{}]", name, idx);
                    if let Some(su) = &elem_su {
                        let mut leaves = Vec::new();
                        self.unpacked_struct_leaf_defaults(&elem, su, 0, &mut leaves);
                        for (k, dv, mdt) in leaves {
                            self.register_packed_leaf_layout(&k, &mdt);
                            self.widths.insert(k.clone(), dv.width);
                            self.signals.insert(k, dv);
                        }
                        continue;
                    }
                    let seed = if is_string_elem {
                        Value::from_string("")
                    } else {
                        default_v.clone()
                    };
                    self.signals.insert(elem.clone(), seed);
                    self.widths.insert(elem, w);
                }
                self.widths.insert(name.clone(), w);
                // Element type of a LOCAL array: the pattern spread and
                // the type-directed `%p` renderer both need it, and only
                // module-scope declarations recorded one.
                self.module
                    .var_decl_types
                    .insert(name.clone(), data_type.clone());
                // A local FIXED array's declaration initializer was
                // dropped, so `int a[4] = '{10,20,30,40};` inside a block
                // read back all zeros. Route it through an assignment,
                // as the queue branch above already does — the pattern
                // spread then applies every §10.9.1 form.
                if let Some(init) = &d.init {
                    let lvalue = crate::ast::expr::Expression::new(
                        crate::ast::expr::ExprKind::Ident(
                            crate::ast::expr::HierarchicalIdentifier {
                                root: None,
                                path: vec![crate::ast::expr::HierPathSegment {
                                    name: crate::ast::Identifier {
                                        name: bare.clone(),
                                        span: d.name.span,
                                    },
                                    selects: Vec::new(),
                                }],
                                span: d.name.span,
                                cached_signal_id: std::cell::Cell::new(None),
                                cached_resolved_name: std::cell::OnceCell::new(),
                            },
                        ),
                        d.name.span,
                    );
                    let assign = crate::ast::stmt::Statement::new(
                        crate::ast::stmt::StatementKind::BlockingAssign {
                            lvalue,
                            rvalue: init.clone(),
                        },
                        d.name.span,
                    );
                    self.exec_statement(&assign);
                }
            } else {
                if !matches!(lifetime, Some(crate::ast::types::Lifetime::Automatic))
                    && let Some(task_name) = self.current_static_task.clone()
                {
                    let key = format!("{}.{}", task_name, d.name.name);
                    if !self.static_task_init.insert(key) {
                        continue;
                    }
                }
                let class_name =
                    if let (true, crate::ast::types::DataType::TypeReference { name, .. }) =
                        (plain_class, data_type)
                    {
                        Some(name.name.name.clone())
                    } else if let crate::ast::types::DataType::TypeReference { name, .. } =
                        data_type
                    {
                        // §8.25: the declared type may be a TYPE
                        // PARAMETER of the enclosing parameterized class
                        // (e.g. `T obj = new(...)` inside
                        // `class C #(type T=int);`). Resolve it through
                        // the active specialization FIRST so the
                        // constructor form constructs the bound class,
                        // not a broken instance of the literal "T".
                        if let Some(bound) = self.resolve_type_param_binding(&name.name.name) {
                            Some(bound)
                        } else {
                            // Resolve a typedef/scoped alias (e.g.
                            // `uvm_resource_types::rsrc_q_t`) to its
                            // concrete class so `name = new()`
                            // constructs it; fall back to the bare
                            // name otherwise.
                            Some(
                                self.resolve_typeref_class_name(name)
                                    .unwrap_or_else(|| name.name.name.clone()),
                            )
                        }
                    } else {
                        None
                    };
                // §6.8/§7.2: an unpacked-struct local is initialized member-
                // wise below (`init_unpacked_struct_local`), which runs the
                // initializer itself. Evaluating it here too ran it twice:
                // `receipt_t r = q.pop_front();` popped two elements.
                let struct_init_later = d.init.is_some()
                    && d.dimensions.is_empty()
                    && !plain
                    && matches!(
                        self.resolve_dt(data_type),
                        crate::ast::types::DataType::Struct(su) if Self::spreads_member_wise(&su)
                    );
                let v = if let Some(init_expr) = d.init.as_ref().filter(|_| !struct_init_later) {
                    let mut produced: Option<Value> = None;
                    if let Some(cn) = &class_name {
                        // `T x = new(...);` / `T x = new;` — detect the
                        // constructor form once; consumed by both the
                        // class and covergroup branches below.
                        let is_new = match &init_expr.kind {
                            ExprKind::Call { func, args } => {
                                if let ExprKind::Ident(h) = &func.kind {
                                    if h.path.last().is_some_and(|s| s.name.name == "new") {
                                        Some(args.clone())
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            }
                            ExprKind::Ident(h) => {
                                if h.path.last().is_some_and(|s| s.name.name == "new") {
                                    Some(vec![])
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        };
                        // §8.25: the declared type resolved to a
                        // SPECIALIZED class name `Base#(args)` (a type
                        // param bound to a nested specialization, e.g.
                        // `T tmp = new;` with T = `queue#(string)` —
                        // UVM's uvm_object_string_pool::get building
                        // the hdl-paths queue). The plain-name lookup
                        // below misses it, so the init fell to generic
                        // eval and the local read x. Mirror the
                        // assignment arm: extract the spec, carry it
                        // through `current_spec`, construct the base.
                        // §15.3/§15.4: `mailbox mb = new(2);` /
                        // `semaphore s = new(1);` as a block-local
                        // declaration — the built-in containers are not in
                        // `module.classes`, so the constructor was skipped
                        // and the local stayed null.
                        if produced.is_none() && !self.module.classes.contains_key(cn) {
                            if let (Some(kind), Some(call_args)) =
                                (Self::container_base(cn), is_new.as_ref())
                            {
                                let handle = self.alloc_builtin_container(kind, call_args);
                                produced = Some(Value::from_u64(handle as u64, 32));
                            }
                        }
                        if produced.is_none()
                            && is_new.is_some()
                            && !self.module.classes.contains_key(cn)
                        {
                            if let Some((base, sig)) = self.extract_spec_from_string(cn) {
                                if let Some(class_def) = self.module.classes.get(&base).cloned() {
                                    let call_args = is_new.clone().unwrap();
                                    self.ensure_spec_statics(&base, &sig);
                                    let saved_spec = self.current_spec.clone();
                                    self.current_spec = Some((base.clone(), sig));
                                    produced = Some(self.instantiate_class(&class_def, &call_args));
                                    self.current_spec = saved_spec;
                                }
                            }
                        }
                        if let Some(class_def) = self.module.classes.get(cn).cloned() {
                            if let Some(call_args) = is_new {
                                // §8.12 copy constructor: `T x = new src;`
                                // with a single CLASS-HANDLE argument
                                // shallow-copies `src`, it does NOT run
                                // the constructor. The assignment path
                                // handles this; the decl-init path used
                                // to fall straight through to construction
                                // (passing the handle as an ignored ctor
                                // arg), so `T x = new src;` silently ran
                                // `new()` and produced a fresh object.
                                if call_args.len() == 1
                                    && matches!(
                                        &call_args[0].kind,
                                        ExprKind::Ident(_)
                                            | ExprKind::MemberAccess { .. }
                                            | ExprKind::Index { .. }
                                            // `new this` in a clone
                                            // method copies the
                                            // CURRENT object (§8.12)
                                            | ExprKind::This
                                    )
                                    && self.expr_is_class_handle(&call_args[0])
                                {
                                    let src_h = self.eval_expr(&call_args[0]).to_u64().unwrap_or(0)
                                        as usize;
                                    // §8.12: `T x = new src;` shallow-copies
                                    // `src` ONLY when src is of type T (or a
                                    // subclass of T). A single class-handle
                                    // arg of a DIFFERENT, unrelated class is
                                    // a constructor ARGUMENT, not a copy
                                    // source — e.g. `factory f = new(b)` where
                                    // b is a base_cls must call
                                    // factory::new(b), not copy-construct b.
                                    // Without this type check, the loose
                                    // `expr_is_class_handle` test treated any
                                    // class-typed arg as a copy source,
                                    // silently skipping the constructor body
                                    // (so field assignments like
                                    // `reg_type = t` never ran).
                                    let is_copy_src = src_h != 0
                                        && matches!(self.heap.get(src_h), Some(Some(src_inst))
                                            if self.class_extends(&src_inst.class_name, cn));
                                    if is_copy_src {
                                        produced = Some(self.copy_construct(src_h));
                                    }
                                }
                                if produced.is_none() {
                                    // Pass the declared type's `#(...)` args
                                    // (e.g. `resource#(int) r = new()`) so
                                    // the instance records its type bindings
                                    // (`T -> int`) — needed for per-spec static
                                    // member resolution (get_type vs
                                    // get_type_handle).
                                    // §8.25.1: a typedef ALIAS of a
                                    // specialization (`typedef
                                    // Box#(byte,8) ByteBox; ByteBox b
                                    // = new;`) carries no args on the
                                    // declared type — pull them from
                                    // the typedef's target, else the
                                    // instance silently gets the
                                    // class DEFAULTS.
                                    let td_args: Option<Vec<Expression>> = match data_type {
                                        crate::ast::types::DataType::TypeReference {
                                            name,
                                            type_args,
                                            ..
                                        } if type_args.is_empty() => {
                                            match self.lookup_typedef_target(&name.name.name) {
                                                Some(
                                                    crate::ast::types::DataType::TypeReference {
                                                        type_args: t2,
                                                        ..
                                                    },
                                                ) if !t2.is_empty() => Some(t2.clone()),
                                                _ => None,
                                            }
                                        }
                                        _ => None,
                                    };
                                    let ta: Option<&[Expression]> = match data_type {
                                        crate::ast::types::DataType::TypeReference {
                                            type_args,
                                            ..
                                        } if !type_args.is_empty() => Some(type_args.as_slice()),
                                        _ => td_args.as_deref(),
                                    };
                                    produced = Some(self.instantiate_class_with_type_args(
                                        &class_def, &call_args, ta,
                                    ));
                                }
                            }
                        } else if let Some(cg_def) = self.covergroup_def_for(cn) {
                            // IEEE 1800-2017 §19.8: a covergroup-typed
                            // LOCAL `cg c = new();` (declared inside a
                            // procedural block) must allocate a real
                            // covergroup instance. This branch only
                            // handled classes, so the initializer fell
                            // through to generic eval, `c` never got a
                            // cg_heap handle, and sample() /
                            // get_inst_coverage() (and the @(event)
                            // clocking registration done inside
                            // instantiate_covergroup) were silent
                            // no-ops — coverage always read 0.
                            if let Some(call_args) = is_new {
                                produced = Some(self.instantiate_covergroup(&cg_def, &call_args));
                            }
                        }
                    }
                    match produced {
                        Some(p) => p,
                        None => {
                            let raw = self.eval_expr_ctx(init_expr, w);
                            Self::fit_local_decl_init(
                                raw,
                                w,
                                data_type,
                                two_state,
                                is_real_type,
                                decl_signed,
                            )
                        }
                    }
                } else {
                    default_v.clone()
                };
                match self.widths.get_mut(d.name.name.as_str()) {
                    Some(slot) => *slot = w,
                    None => {
                        self.widths.insert(d.name.name.clone(), w);
                    }
                }
                // A local variable inside a method (which has a pushed
                // call frame) must live in THAT frame, not the global
                // signal table — otherwise a callee's local shadows and
                // CLOBBERS a same-named caller local that lives as a
                // signal (initial/always blocks have no frame). This avoids
                // a local `resource_base r` nulling an outer `r`. Reads
                // already consult `local_stack.last()` before signals, so
                // the method still sees its own local. No frame → module/
                // initial scope → signal table as before.
                // §7.2/§13.4.2: an UNPACKED-struct local declared inside
                // a subroutine keeps each member in the frame under
                // `<name>.<member>` — the convention formal binding
                // already uses. Nothing seeded those keys for a local,
                // so its member references escaped to the module signal
                // table and landed on an unrelated same-named
                // variable's member: the callee read the CALLER's value
                // back and its own writes vanished. Seeding also makes
                // each call's local FRESH, as `automatic` requires.
                let mut unpacked_struct_local = false;
                // A static process-block local lives in the signal table even
                // under a loop's variable frame, which its block outlives.
                let in_frame = !self.local_stack.is_empty() && !static_block;
                if in_frame && d.dimensions.is_empty() && !plain {
                    if let crate::ast::types::DataType::Struct(su) = self.resolve_dt(data_type) {
                        if Self::spreads_member_wise(&su) {
                            unpacked_struct_local = true;
                            let base = d.name.name.clone();
                            // §6.8 defaults per leaf (a 2-state member reads
                            // 0), and packed members alias their fields.
                            let mut leaves = Vec::new();
                            self.unpacked_struct_leaf_defaults(&base, &su, 0, &mut leaves);
                            for (k, _, mdt) in &leaves {
                                self.register_packed_leaf_layout(k, mdt);
                            }
                            // The `<base>.` marker, as `unpacked_struct_leaf_keys`.
                            self.any_struct_formal_markers = true;
                            if let Some(frame) = self.local_stack.last_mut() {
                                frame.insert(format!("{}.", base), Value::new(0));
                                for (k, dv, _) in leaves {
                                    frame.insert(k, dv);
                                }
                            }
                        }
                    }
                }
                if unpacked_struct_local {
                    // An unpacked struct has no container of its own —
                    // its leaves ARE the storage. Seeding one anyway
                    // shadowed them: reading the bare name returned the
                    // container's x instead of the packed members, so
                    // `return s;` from a struct-returning function
                    // yielded x even though every member was set.
                    self.local_stack
                        .last_mut()
                        .map(|frame| frame.remove(&d.name.name));
                    // Record the declared type so type-directed
                    // consumers (`%s` string-member detection, `%p`)
                    // can resolve `p.member` through the typedef.
                    self.module
                        .var_decl_types
                        .insert(d.name.name.clone(), data_type.clone());
                    self.note_struct_root(&d.name.name);
                    self.note_struct_type(data_type);
                    // §6.21: each entry gets FRESH (empty) dynamic members.
                    if self.struct_colls_possible()
                        && let Some(su) = self.unpacked_struct_of(data_type)
                    {
                        self.fresh_struct_colls(&d.name.name, &su);
                    }
                    // A decl-init whose SOURCE is an unpacked-struct
                    // CLASS property (`pair_t p = o.orig;`, incl. the
                    // ternary form) must write the leaves member-wise —
                    // the whole-value `v` computed above has nothing to
                    // assemble from and left every member x.
                    if let Some(init_expr) = d.init.clone() {
                        self.init_unpacked_struct_local(d, data_type, &init_expr);
                    }
                } else if let Some(frame) = self.local_stack.last_mut().filter(|_| in_frame) {
                    frame.insert(d.name.name.clone(), v);
                } else {
                    self.signals.insert(d.name.name.clone(), v);
                    // §7.2: an unpacked-struct local of an `initial`/`always`
                    // block (no call frame) gets its member leaves as
                    // signals, as module-scope structs do; a member write
                    // through a selected member (`t.rows[0].level = 2`) found
                    // no leaf to land on and vanished.
                    if d.dimensions.is_empty() && !plain {
                        if let crate::ast::types::DataType::Struct(su) = self.resolve_dt(data_type)
                        {
                            if Self::spreads_member_wise(&su) {
                                self.note_struct_root(&d.name.name);
                                self.note_struct_type(data_type);
                                let mut leaves = Vec::new();
                                self.unpacked_struct_leaf_defaults(
                                    &d.name.name,
                                    &su,
                                    0,
                                    &mut leaves,
                                );
                                for (k, dv, mdt) in leaves {
                                    self.register_packed_leaf_layout(&k, &mdt);
                                    self.widths.insert(k.clone(), dv.width);
                                    self.signals.insert(k, dv);
                                }
                                if self.struct_colls_possible() {
                                    self.fresh_struct_colls(&d.name.name, &su);
                                }
                                if let Some(init_expr) = d.init.clone() {
                                    self.init_unpacked_struct_local(d, data_type, &init_expr);
                                }
                            }
                        }
                    }
                    // §6.21/§9.3.2: an explicitly `automatic` block
                    // local is a fresh variable per block entry, and a
                    // fork child must capture ITS entry's value. With
                    // no call frame it lives in the shared signal map,
                    // so record it for `inherit_fork_child_context` to
                    // copy by value at spawn time — exactly like a
                    // for-init loop variable. The enclosing SeqBlock
                    // pops it on scope exit.
                    if matches!(lifetime, Some(crate::ast::types::Lifetime::Automatic)) {
                        self.auto_loop_vars.push(d.name.name.clone());
                    }
                }
                // §6.8/§25.9/§7.3.2: a virtual-interface or tagged-union
                // local's initializer is the assignment `name = init` — the
                // assignment arm binds the interface instance and builds the
                // tag + member value; evaluated as a plain expression the
                // vif read x and the union matched no tag.
                if let Some(init_expr) = d.init.as_ref().filter(|_| !plain) {
                    // Followed through the typedef table by reference: this
                    // runs for every non-scalar declaration with an
                    // initializer, so no resolved-type clone.
                    let via_assign = match self.typedef_chain_end(data_type) {
                        crate::ast::types::DataType::Interface { .. } => true,
                        crate::ast::types::DataType::Struct(su) => su.tagged,
                        _ => false,
                    };
                    if via_assign {
                        let lvalue = crate::ast::expr::Expression::new(
                            crate::ast::expr::ExprKind::Ident(
                                crate::ast::expr::HierarchicalIdentifier {
                                    root: None,
                                    path: vec![crate::ast::expr::HierPathSegment {
                                        name: d.name.clone(),
                                        selects: Vec::new(),
                                    }],
                                    span: d.name.span,
                                    cached_signal_id: std::cell::Cell::new(None),
                                    cached_resolved_name: std::cell::OnceCell::new(),
                                },
                            ),
                            d.name.span,
                        );
                        let assign = crate::ast::stmt::Statement::new(
                            crate::ast::stmt::StatementKind::BlockingAssign {
                                lvalue,
                                rvalue: init_expr.clone(),
                            },
                            d.name.span,
                        );
                        self.exec_statement(&assign);
                    }
                }
                // Track `signed` scalars so reads carry `is_signed`
                // (needed for signed division/modulo, comparison, and
                // `%0d` display). A fresh decl also clears a stale
                // same-named signed flag from another frame.
                if decl_signed {
                    if !self.signed_signals.contains(d.name.name.as_str()) {
                        self.signed_signals.insert(d.name.name.clone());
                    }
                } else {
                    self.signed_signals.remove(&d.name.name);
                }
                // Track `real` locals so assignments coerce to real —
                // without this a block-local `real r; r = 3.14159;`
                // rounded through real_to_int and read back 3.0
                // (issue #23). A non-real decl clears a stale flag.
                if is_real_type {
                    self.real_signals.insert(d.name.name.clone());
                } else {
                    self.real_signals.remove(&d.name.name);
                }
                // Track `string`-typed locals so `{a, b}` concatenations
                // involving them are evaluated as string concat. A
                // non-string decl clears a stale same-named flag from
                // another frame (else `s[1]` on an `int s` would take
                // the string char-index path) — same pattern as
                // `signed_signals` above.
                if matches!(
                    data_type,
                    crate::ast::types::DataType::Simple {
                        kind: crate::ast::types::SimpleType::String,
                        ..
                    }
                ) {
                    self.string_signals.insert(d.name.name.clone());
                } else {
                    self.forget_string_flag(&d.name.name);
                }
                // §6.21: another process may declare a same-named string
                // while this frame is parked. Keep this declaration's kind
                // with the frame, independently of the global name tables.
                let is_string = self.declared_name_is_string(&d.name.name);
                self.record_frame_string_kind(&d.name.name, is_string);
                // Record a class-typed local's type so a later
                // separate `name = new();` knows what to construct.
                // Covergroup-typed locals (§19.8) are recorded too:
                // get_expr_type_name consults this map, and both the
                // bare `c = new;` and `c = new();` assignment paths
                // already route a covergroup type name to
                // instantiate_covergroup — they just never saw it
                // for a procedural local.
                if let Some(cn) = &class_name {
                    // §8.25: `class_name` may be a resolved
                    // VALUE-PARAMETERIZED specialization (e.g. `T me;`
                    // where T → `base#(1)`), which is NOT a bare key in
                    // `module.classes`. Accept it when its base class
                    // is real, so the var's type is recorded and
                    // `$cast`/`new` resolve the specialization later.
                    let cn_is_class = self.module.classes.contains_key(cn)
                        || self.module.covergroups.contains_key(cn)
                        || (cn.contains('#')
                            && self
                                .module
                                .classes
                                .contains_key(cn.split('#').next().unwrap_or(cn)));
                    if cn_is_class {
                        self.record_local_class_type(&d.name.name, cn);
                        if self.var_class_types.get(d.name.name.as_str()) != Some(cn) {
                            self.var_class_types.insert(d.name.name.clone(), cn.clone());
                        }
                        // A typedef'd local (e.g. `table_q_t rq` where
                        // `table_q_t = shared#(Foo[$])`) resolves to
                        // `cn` but hides the type_args inside the
                        // typedef chain. Record them so a separate
                        // `rq = new()` constructs the right
                        // specialization with type_bindings populated
                        // (§8.25 — otherwise `T` stays unbound).
                        if let crate::ast::types::DataType::TypeReference { name, .. } = data_type {
                            if plain_class {
                                // A plain class has no typedef to carry arguments.
                            } else if let Some((_, Some(ta))) =
                                self.resolve_typeref_class_with_type_args(name)
                            {
                                self.var_type_args.insert(d.name.name.clone(), ta);
                            }
                        }
                    } else if self.resolve_type_param_binding(cn).is_some() {
                        // `cn` is a class TYPE parameter (e.g. `T obj;`
                        // inside a parameterized-class method). Record
                        // the param name so a separate `obj = new()`
                        // resolves it to the concrete class.
                        //
                        // `resolve_type_param_binding` covers BOTH the
                        // instance path (a method's `this` instance
                        // `type_bindings`) AND the static path (the
                        // active specialization `current_spec`). The
                        // latter matters for STATIC methods of a
                        // parameterized class — e.g.
                        // `callbacks#(T,CB)::get_first` declares
                        // `CB cb;` with no `this`. Without the static
                        // branch, `cb` was never registered, so a bare
                        // `$cast(cb, ...)` assignment and any later
                        // `new()` could not resolve CB, and the return
                        // value came back null to the caller.
                        self.record_local_class_type(&d.name.name, cn);
                        self.var_class_types.insert(d.name.name.clone(), cn.clone());
                    }
                }
                // Record a parameterized local's declared `#(...)` type
                // args so a SEPARATE `name = new(...)` constructs the right
                // specialization (`resource#(T) r; r=new`).
                // A typedef'd local (e.g. `table_q_t rq` where `table_q_t =
                // shared#(Foo[$])`) has EMPTY explicit type_args but
                // the args are hidden in the typedef chain — resolve
                // them so the new() populates type_bindings (§8.25).
                // Clear a stale same-named entry when this decl has none.
                let resolved_ta = match data_type {
                    crate::ast::types::DataType::TypeReference { name, .. } if !plain_class => self
                        .resolve_typeref_class_with_type_args(name)
                        .and_then(|(_, ta)| ta),
                    _ => None,
                };
                if let crate::ast::types::DataType::TypeReference { type_args, .. } = data_type {
                    if !type_args.is_empty() {
                        self.var_type_args
                            .insert(d.name.name.clone(), type_args.clone());
                    } else if let Some(ta) = resolved_ta {
                        self.var_type_args.insert(d.name.name.clone(), ta);
                    } else {
                        self.var_type_args.remove(&d.name.name);
                    }
                } else {
                    self.var_type_args.remove(&d.name.name);
                }
                // §15.3/§15.4: record a mailbox/semaphore-typed local so
                // a later separate `name = new(bound)` allocates the right
                // container. Clear any stale same-named entry from another
                // frame (var_container_types is a flat map like
                // var_class_types) when this decl isn't a container.
                match class_name.as_deref().and_then(Self::container_base) {
                    Some(k) => {
                        self.var_container_types
                            .insert(d.name.name.clone(), k.to_string());
                    }
                    None => {
                        self.var_container_types.remove(&d.name.name);
                    }
                }
                // LRM §6.19.6: track typedef-typed locals so
                // `local.next()`/`.first()`/etc. find their
                // enum-member list. We extract the bare type
                // name from a TypeReference data_type. (A plain class
                // is neither, nor a type parameter; a forward-declared one
                // is registered as its typedef here.)
                if let (false, crate::ast::types::DataType::TypeReference { name, .. }) =
                    (class_ref == Some(false), data_type)
                {
                    let tn = name.name.name.clone();
                    // A local declared with a class TYPE PARAMETER
                    // (`T e;` in a parameterized class) isn't in the
                    // flat typedef/enum tables under `T`. Resolve the
                    // active specialization's binding (T -> e_t) so
                    // `e.first()/.next()/.last()/.prev()/.num()/`
                    // names find the CONCRETE enum-member list
                    // (else they fall through to a generic 0).
                    let tn = if self.module.enum_members.contains_key(&tn)
                        || self.module.typedefs.contains_key(&tn)
                        || self.module.typedef_types.contains_key(&tn)
                    {
                        tn
                    } else if self.current_spec.is_some() {
                        self.resolve_type_param_binding(&tn).unwrap_or_else(|| tn)
                    } else {
                        tn
                    };
                    if self.module.enum_members.contains_key(&tn)
                        || self.module.typedefs.contains_key(&tn)
                        || self.module.typedef_types.contains_key(&tn)
                    {
                        if self.var_typedef_types.get(d.name.name.as_str()) != Some(&tn) {
                            self.var_typedef_types
                                .insert(d.name.name.clone(), tn.clone());
                        }
                        // Record the frame-scoped typedef too so a
                        // method-local enum type is resolvable without
                        // the flat `var_class_types`/`var_typedef_types`
                        // collision across unrelated frames (a same-
                        // named formal `VISITOR v` in another method
                        // must not shadow this `uvm_verbosity v;
                        // local). `local_typedef_type_of` reads the
                        // innermost active frame. §6.19.6.
                        self.record_local_typedef_type(&d.name.name, &tn);
                    }
                }
            }
        }
        // §6.21 / §13.3.1: a `static` local declared inside a subroutine
        // keeps ONE persistent storage across all runtime calls, and its
        // initializer runs only once. On the FIRST call we persist the
        // just-initialized value; on later calls we restore the persisted
        // value (undoing this call's re-initialization). The final value
        // is written back on return by `sync_static_locals`. Only scalar
        // integral/real statics are tracked — arrays/queues keep per-call
        // storage.
        if matches!(lifetime, Some(crate::ast::types::Lifetime::Static))
            && !self.static_local_syncs.is_empty()
            && !self.in_const_param_eval
        {
            let sub_name = self
                .static_local_syncs
                .last()
                .map(|(n, _)| n.clone())
                .unwrap_or_default();
            for d in declarators {
                let nm = &d.name.name;
                // Hoisted declarations already use their shared global cell;
                // never create a second frame-backed scalar mirror.
                if self.hoisted_sub_static_names.contains(nm) {
                    continue;
                }
                // Skip aggregates: arrays/queues/assoc are not persisted.
                if !d.dimensions.is_empty()
                    || self.module.arrays.contains_key(nm)
                    || self.module.dynamic_arrays.contains(nm)
                    || self.module.associative_arrays.contains_key(nm)
                {
                    continue;
                }
                // Build the persistent key. A free function keeps
                // `"<subroutine>::<var>"` (its class context is None).
                // A METHOD of a parameterized class appends the active
                // specialization: `static this_type m_inst;` inside
                // `C#(P)::get()` must be one shared cell per
                // specialization (shared across calls of the SAME spec,
                // distinct across DIFFERENT specs) — IEEE 1800-2023
                // §8.25/§6.21. The class prefix also keeps two classes'
                // same-named method locals from colliding.
                let key = match (
                    self.class_context_stack.last().and_then(|c| c.as_ref()),
                    self.current_spec.as_ref(),
                ) {
                    (Some(cn), Some((spec_base, sig))) if spec_base == cn => {
                        format!("{}#{}::{}::{}", cn, sig, sub_name, nm)
                    }
                    (Some(cn), _) => format!("{}::{}::{}", cn, sub_name, nm),
                    _ => format!("{}::{}", sub_name, nm),
                };
                if let Some(pv) = self.static_local_vars.get(&key).cloned() {
                    if let Some(l) = self.local_stack.last_mut() {
                        l.insert(nm.clone(), pv);
                    }
                } else {
                    let cur = self
                        .local_stack
                        .last()
                        .and_then(|l| l.get(nm))
                        .cloned()
                        .unwrap_or_else(|| Value::zero(w));
                    self.static_local_vars.insert(key.clone(), cur);
                }
                if let Some((_n, syncs)) = self.static_local_syncs.last_mut() {
                    syncs.push((nm.clone(), key.clone()));
                }
                // Route to the live shared cell only for scalar
                // integral/real statics (the re-entrant/recursive
                // case this fixes). Strings and other aggregates keep
                // their existing per-call storage semantics and must
                // not be intercepted.
                if !self.string_signals.contains(nm.as_str())
                    && !self.module.arrays.contains_key(nm.as_str())
                {
                    if let Some(&b0) = nm.as_bytes().first() {
                        self.static_local_first[b0 as usize] = true;
                    }
                    self.static_local_names.insert(nm.clone());
                    self.static_local_keys.insert(key);
                }
            }
        }
    }

    pub fn exec_statement(&mut self, stmt: &Statement) {
        if self.finished
            || self.time > self.max_time
            || self.break_flag
            || self.continue_flag
            || self.return_flag
        {
            return;
        }
        // §10.4.2 / §25.9: evaluate receiver calls once while capturing an
        // NBA target, before rebasing it to the interface instance. Re-enter
        // with those calls pinned so callbacks still fire once below.
        if let StatementKind::NonblockingAssign { lvalue, rvalue, delay } = &stmt.kind
            && (Self::recv_chain_call(rvalue).is_some() || Self::recv_chain_call(lvalue).is_some())
            && let Some(base) = self.pin_stmt_recv_calls(&[rvalue, lvalue])
        {
            if self.is_unpacked_aggregate_operand(lvalue) || self.is_unpacked_aggregate_operand(rvalue) {
                let rvalue = self.freeze_conditional_path(rvalue, true);
                let lvalue = self.freeze_conditional_path(lvalue, true);
                self.exec_statement(&Statement::new(StatementKind::NonblockingAssign {
                    lvalue, rvalue, delay: delay.clone(),
                }, stmt.span));
            } else {
                self.exec_statement(stmt);
            }
            self.recv_call_memo.truncate(base);
            return;
        }
        // §38.36.1 cbStmt, before the statement runs (interpreted code only).
        if let StatementKind::NonblockingAssign {
            lvalue,
            delay,
            rvalue,
        } = &stmt.kind
            && let Some(selected) = self.selected_aggregate_rhs(Some(lvalue), rvalue)
        {
            self.exec_statement(&Statement::new(
                StatementKind::NonblockingAssign {
                    lvalue: lvalue.clone(),
                    delay: delay.clone(),
                    rvalue: selected,
                },
                stmt.span,
            ));
            return;
        }
        if self.vpi_cb_mask & vpi_cb::m(vpi_cb::CB_STMT) != 0 {
            self.vpi_stmt_hook(stmt, false);
        }
        match &stmt.kind {
            StatementKind::Null => {}
            StatementKind::ScopePop => {
                if let Some(c) = self.task_cleanup.pop() {
                    self.unwind_task_frame(c);
                }
            }
            // INTERNAL continuation sentinel — only meaningful in the
            // suspend-aware `run_process_stmts` stream. If reached here
            // (synchronous exec), it is a no-op.
            StatementKind::ForeachTail { .. } => {}
            StatementKind::ForeverTail { .. } => {}
            // Same: the synchronous `for` runs its own step, so the barrier
            // has nothing to do here. (It cannot even be reached with a
            // pending `continue` — the guard above returns first, which is
            // precisely why `run_process_stmts` handles it itself.)
            StatementKind::LoopStep => {}
            StatementKind::Expr(expr) => self.exec_expr_stmt(expr),
            StatementKind::BlockingAssign { lvalue, rvalue } => {
                self.exec_stmt_blocking_assign(stmt, lvalue, rvalue)
            }
            StatementKind::NonblockingAssign {
                lvalue,
                delay,
                rvalue,
            } => {
                // §25.8/§25.9: an lvalue whose receiver is a bound virtual
                // interface (`cfg.vif.data`, `d.a.cfg.vif.cb.data`) targets
                // the bound instance. Resolved NOW, while the owner handles
                // and `this` are live: the commit runs later in a context
                // where neither is.
                let lvalue_vif;
                let lvalue: &Expression = match self.vif_rebase_expr(lvalue) {
                    Some(r) => {
                        lvalue_vif = r;
                        &lvalue_vif
                    }
                    None => lvalue,
                };
                // §11.4.1: an lvalue index with a side effect
                // (`v[i++ +: 18] <= ...`) is evaluated once. Fold it to the
                // literal it evaluates to now, as the blocking path does, so
                // the target's width and the captured target below read the
                // same index. Before, `i` advanced more than once per
                // assignment and the value landed in the wrong slice. The
                // statement carries on with the folded target, so its
                // statement callbacks fire once.
                let lvalue_folded;
                let lvalue: &Expression = match Self::lvalue_has_incdec_index(lvalue)
                    .then(|| self.rewrite_lvalue_index_side_effects(lvalue))
                    .flatten()
                {
                    Some(f) => {
                        lvalue_folded = f;
                        &lvalue_folded
                    }
                    None => lvalue,
                };
                // §14.4: normalize a plain dotted-chain lvalue (`vif.cb.data`
                // inside a method) to the flat Ident the clocking-drive check
                // below expects — the chain shape skipped the check and the
                // drive landed on a phantom name instead of the clocking
                // output net.
                // Keep the member tree for declaration-based packed geometry;
                // the flat spelling below is only needed by clocking drives.
                let packed_lvalue = lvalue;
                let lvalue_norm;
                let lvalue: &Expression = match Self::member_chain_as_flat_ident(lvalue) {
                    Some(flat) => {
                        lvalue_norm = flat;
                        &lvalue_norm
                    }
                    None => lvalue,
                };
                // §9.4.5 intra-assignment delay on an NBA (`lhs <= #d rhs`,
                // canonicalized to `$__xz_intra_delay(d, rhs)`): the RHS is
                // evaluated now; the update is scheduled d ticks out.
                let mut intra_d: Option<u64> = None;
                // §14.16 `cb.out <= ##N rhs`: strip the cycle marker and keep
                // N; the clocking-drive path below defers by N clock events.
                let mut intra_cycles: Option<u64> = None;
                let rvalue: &Expression = match Self::intra_cycle_marker(rvalue) {
                    Some((n_expr, rhs)) => {
                        intra_cycles = Some(self.eval_expr(n_expr).to_u64().unwrap_or(0));
                        rhs
                    }
                    None => rvalue,
                };
                let rvalue: &Expression = match Self::intra_delay_marker(rvalue) {
                    Some((d_expr, rhs)) => {
                        intra_d = Some(self.eval_delay_ticks(d_expr));
                        rhs
                    }
                    None => rvalue,
                };
                // §9.4.5 intra-assignment EVENT control on an NBA
                // (`lhs <= [repeat(n)] @(edge sig) rhs`, canonicalized to
                // `$__xz_intra_ev(n, edge, sig, rhs)`). Only the BLOCKING arm
                // handled the marker, so this rvalue fell through to plain
                // eval — an unknown system call, i.e. ZERO — and the NBA
                // posted 0 immediately, clobbering the variable instead of
                // updating it at the n-th edge. Capture the RHS NOW, spawn a
                // one-shot process that waits the events and posts the saved
                // value as an ordinary NBA; the CURRENT process continues
                // without blocking (that is what makes it nonblocking).
                if let ExprKind::SystemCall { name, args } = &rvalue.kind {
                    if name == crate::intra_delay::INTRA_EVENT_MARKER && args.len() == 4 {
                        let n_val = self.eval_expr(&args[0]).to_u64().unwrap_or(1) as i64;
                        let edge_code = self.eval_expr(&args[1]).to_u64().unwrap_or(0);
                        let val = self.eval_target_rhs(packed_lvalue, &args[3]);
                        let saved = self.make_intra_saved_expr(val, rvalue.span);
                        let edge = match edge_code {
                            1 => Some(Edge::Posedge),
                            2 => Some(Edge::Negedge),
                            _ => None,
                        };
                        let mut cont: Vec<Statement> = Vec::new();
                        for _ in 0..n_val.max(0) {
                            cont.push(Statement::new(
                                StatementKind::TimingControl {
                                    control: TimingControl::Event(EventControl::EventExpr(vec![
                                        EventExpr {
                                            edge,
                                            expr: args[2].clone(),
                                            iff: None,
                                            span: stmt.span,
                                        },
                                    ])),
                                    stmt: Box::new(Statement::new(StatementKind::Null, stmt.span)),
                                },
                                stmt.span,
                            ));
                        }
                        cont.push(Statement::new(
                            StatementKind::NonblockingAssign {
                                lvalue: lvalue.clone(),
                                delay: None,
                                rvalue: saved,
                            },
                            stmt.span,
                        ));
                        let child = self.next_pid;
                        self.next_pid += 1;
                        self.process_origin
                            .insert(child, (stmt.span, "intra-assignment event NBA"));
                        self.event_queue.schedule(self.time, child, cont.into());
                        return;
                    }
                }
                // A named/ordered/default pattern driven onto a PACKED struct
                // (or sub-member) must be laid out by member offset, not
                // blind-concatenated — see try_assign_packed_struct_pattern.
                // A pattern onto a PACKED ARRAY lvalue likewise expands per
                // ELEMENT (`cdts <= '{default: v}` on logic [1:0][9:0] filled
                // only element 0 through the generic eval) — same route the
                // CA and blocking paths already take.
                // §10.9.2/§10.10: a pattern written to an aggregate whose
                // leaves are SEPARATE signals (unpacked struct, unpacked array
                // of them, associative array) must be spread member-wise, not
                // collapsed into one packed value. Only the blocking path did
                // this, so `s <= '{a:8'h77, b:8'h88};` on an unpacked struct
                // left every member x while the identical blocking assignment
                // was correct.
                if let ExprKind::AssignmentPattern(items) = &rvalue.kind {
                    if self.packed_pattern_for_lhs(lvalue, items).is_none()
                        && self.eval_packed_struct_pattern(lvalue, items).is_none()
                    {
                        let spread = if matches!(&lvalue.kind, ExprKind::Ident(_)) {
                            self.pattern_aggregate_names(lvalue)
                                .into_iter()
                                .any(|n| self.assign_pattern_aggregate(&n, items))
                        } else {
                            self.assign_pattern_element(lvalue, rvalue)
                                || match self.flat_member_name(lvalue) {
                                    Some(flat) => self.assign_pattern_flat(&flat, items),
                                    None => false,
                                }
                        };
                        if spread {
                            return;
                        }
                    }
                }
                // §10.4.2: a whole UNPACKED struct has no storage of its own —
                // its member leaves are the storage — so it is updated as one
                // nonblocking assignment per leaf, every source leaf read now.
                if let Some(leaf_nbas) = self
                    .unpacked_array_nba_leaves(lvalue, rvalue)
                    .or_else(|| self.unpacked_struct_nba_leaves(lvalue, rvalue))
                {
                    let d = match intra_d {
                        Some(d) => d,
                        None => delay
                            .as_ref()
                            .map(|de| self.eval_expr(de).to_u64().unwrap_or(0))
                            .unwrap_or(0),
                    };
                    let delay_expr = (d > 0).then(|| {
                        Expression::new(
                            ExprKind::Number(NumberLiteral::Integer {
                                size: None,
                                signed: false,
                                base: NumberBase::Decimal,
                                value: d.to_string(),
                                cached_val: Cell::new(None),
                            }),
                            stmt.span,
                        )
                    });
                    for (leaf_lvalue, v) in leaf_nbas {
                        let saved = self.make_intra_saved_expr(v, rvalue.span);
                        self.exec_statement(&Statement::new(
                            StatementKind::NonblockingAssign {
                                lvalue: leaf_lvalue,
                                delay: delay_expr.clone(),
                                rvalue: saved,
                            },
                            stmt.span,
                        ));
                    }
                    return;
                }
                let val = match &rvalue.kind {
                    ExprKind::AssignmentPattern(items) => self
                        .packed_pattern_for_lhs(lvalue, items)
                        .or_else(|| self.eval_packed_struct_pattern(lvalue, items))
                        .unwrap_or_else(|| self.eval_expr(rvalue)),
                    _ => self.eval_target_rhs(packed_lvalue, rvalue),
                };
                // LRM §14.4: clocking-block output drive
                // `cb.<out_sig> <= val`. Defer to the cb's next clock
                // edge (output skew) instead of driving now. Handles both a
                // module-scoped `cb.sig` and an interface-scoped
                // `iface_inst.cb.sig` (≥3 segments): the block key is every
                // segment but the last, and the queued signal is the RESOLVED
                // output net (matched from the block's signal list by exact
                // name or `.sig` suffix) so `tick_clocking_blocks` drives it.
                // A drive of a select (`cb.v[2]`, `vif.cb.u[1]`, `cb.x[3:2]`)
                // is recognized through its clockvar and drives only the
                // selected part.
                let mut cb_sels: Vec<&Expression> = Vec::new();
                let mut cb_base = lvalue;
                while let ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } =
                    &cb_base.kind
                {
                    cb_sels.push(cb_base);
                    cb_base = expr;
                }
                let cb_base_flat;
                if !cb_sels.is_empty()
                    && let Some(f) = Self::member_chain_as_flat_ident(cb_base)
                {
                    cb_base_flat = f;
                    cb_base = &cb_base_flat;
                }
                if let ExprKind::Ident(hier) = &cb_base.kind {
                    if hier.path.len() >= 2 {
                        let cb_segs: Vec<&str> = hier
                            .path
                            .iter()
                            .take(hier.path.len() - 1)
                            .map(|s| s.name.name.as_str())
                            .collect();
                        let cb = self
                            .resolve_clocking_key(&cb_segs)
                            .unwrap_or_else(|| cb_segs.join("."));
                        let raw_sig = hier.path.last().unwrap().name.name.as_str();
                        let sig: &str = self
                            .clocking_sig_alias
                            .get(&cb)
                            .and_then(|m| m.get(raw_sig))
                            .map(|s| s.as_str())
                            .unwrap_or(raw_sig);
                        if let Some((_, sigs)) = self.clocking_meta.get(&cb) {
                            // §14.16.2: an `inout` clockvar is driven like an
                            // output (it is registered as an input for sampling).
                            let inouts = self.clocking_inouts.get(&cb);
                            let out_net = sigs.iter().find(|(n, is_in)| {
                                (!*is_in || inouts.is_some_and(|s| s.contains(n)))
                                    && (n == sig || n.rsplit('.').next() == Some(sig))
                            });
                            if let Some((net, _)) = out_net {
                                let net = net.clone();
                                let last_sels = &hier.path[hier.path.len() - 1].selects;
                                let target = if cb_sels.is_empty() && last_sels.is_empty() {
                                    None
                                } else {
                                    let mut t = Expression::new(
                                        ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                                            root: None,
                                            path: vec![crate::ast::expr::HierPathSegment {
                                                name: crate::ast::Identifier {
                                                    name: net.clone(),
                                                    span: lvalue.span,
                                                },
                                                selects: last_sels.clone(),
                                            }],
                                            span: lvalue.span,
                                            cached_signal_id: std::cell::Cell::new(None),
                                            cached_resolved_name: std::cell::OnceCell::new(),
                                        }),
                                        lvalue.span,
                                    );
                                    for layer in cb_sels.iter().rev() {
                                        let mut l = (*layer).clone();
                                        if let ExprKind::Index { expr, .. }
                                        | ExprKind::RangeSelect { expr, .. } = &mut l.kind
                                        {
                                            *expr = Box::new(t);
                                        }
                                        t = l;
                                    }
                                    Some(self.freeze_lvalue_indices(&t))
                                };
                                // §14.16 `##N`: wait N further clocking events.
                                if let Some(n) = intra_cycles {
                                    if n > 0 {
                                        self.clocking_cycle_pending
                                            .entry(cb)
                                            .or_default()
                                            .push((n, net, val, target));
                                        return;
                                    }
                                }
                                // §14.16.1: a drive executed AT the clocking
                                // event matures at edge + OUTPUT SKEW (per-sig
                                // override, else block default, else #0 =
                                // Re-NBA of this timestep). Only a drive issued
                                // BETWEEN events waits for the next edge.
                                let skew = self
                                    .clocking_sig_out_skew
                                    .get(&cb)
                                    .and_then(|m| m.get(raw_sig).or_else(|| m.get(&net)))
                                    .copied()
                                    .or_else(|| self.clocking_out_skew.get(&cb).copied())
                                    .unwrap_or(0);
                                if self.clocking_last_edge.get(&cb).copied() == Some(self.time) {
                                    if let Some(t) = &target {
                                        self.drive_clocking_select(t, val, skew);
                                        return;
                                    }
                                    if let Some(&id) = self.signal_name_to_id.get(net.as_str()) {
                                        let w = self.signal_widths[id];
                                        self.schedule_delayed_with_delay(id, val.resize(w), skew);
                                        return;
                                    }
                                }
                                self.clocking_output_pending
                                    .entry(cb)
                                    .or_default()
                                    .push((net, val, target));
                                return;
                            }
                        }
                    }
                }
                // SV-2023: NBA to a ref formal redirects to the caller's
                // actual lvalue (IEEE 1800-2023 §13.5.2).
                let written = lvalue;
                let lvalue_owned;
                let lvalue: &Expression = if let ExprKind::Ident(hier) = &lvalue.kind {
                    if hier.path.len() == 1 && hier.path[0].selects.is_empty() {
                        let formal = &hier.path[0].name.name;
                        if let Some(actual) = self
                            .ref_binding_stack
                            .last()
                            .and_then(|m| m.get(formal))
                            .cloned()
                        {
                            lvalue_owned = actual;
                            &lvalue_owned
                        } else {
                            lvalue
                        }
                    } else {
                        lvalue
                    }
                } else {
                    lvalue
                };
                // The caller's actual of a `ref` formal names the caller's
                // variables, not this frame's.
                let rebound = !std::ptr::eq(lvalue, written);
                let w = self.infer_lhs_width(lvalue);
                let d = match intra_d {
                    Some(d) => d,
                    None => delay
                        .as_ref()
                        .map(|de| self.eval_expr(de).to_u64().unwrap_or(0))
                        .unwrap_or(0),
                };
                // §10.4.2 / §6.21: capture persistent storage before the
                // call frame is restored. Every write still commits in NBA.
                if let Some(key) = self.nba_static_key_for_lvalue(lvalue) {
                    self.enqueue_context_nba(
                        NbaEntry {
                            lhs: None,
                            value: val.resize_for_assign(w),
                            resolved_id: None,
                            static_key: Some(key),
                            static_slice: None,
                            this_handle: None,
                            root_frame: self.nba_root_frame(lvalue),
                        },
                        d,
                    );
                    return;
                }
                if !rebound && self.try_schedule_static_packed_nba(packed_lvalue, &val, d) {
                    return;
                }
                if d == 0 {
                    let id_opt = self.resolve_nba_target(lvalue);
                    if let Some(id) = id_opt {
                        // Track via index map so subsequent partial-NBAs
                        // (NbaAssignRange / NbaAssignBitDyn from the
                        // bytecode VM) targeting the same signal merge into
                        // this entry rather than re-seeding from
                        // signal_table — same invariant as the bytecode
                        // NbaAssign Insn.
                        self.nba_fast_index.insert(id, self.nba_fast.len());
                        self.nba_fast.push(NbaFast {
                            block_index: 0,
                            signal_id: id,
                            value: val.resize_for_assign(w),
                        });
                    } else if self.try_schedule_packed_index_nba(lvalue, &val) {
                        // The packed slice has already been merged into the
                        // pending whole-signal value.
                    } else if matches!(&lvalue.kind, ExprKind::Ident(h)
                        if h.path.len() == 1
                            && self.local_stack.last().is_some_and(|m| m.contains_key(&h.path[0].name.name)))
                    {
                        // Preserve the existing whole-frame fallback.
                        // Persistent local declarations were captured above.
                        self.assign_value(lvalue, &val.resize_for_assign(w));
                    } else {
                        let frozen = self.freeze_lvalue_indices(lvalue);
                        let qval = if self.nba_target_is_assoc_elem(lvalue) {
                            val.clone()
                        } else {
                            val.resize_for_assign(w)
                        };
                        let static_key = self.nba_static_key_for_lvalue(lvalue);
                        let this_handle = self.this_stack.last().copied().flatten();
                        self.nba_queue.push(NbaEntry {
                            lhs: Some(frozen),
                            value: qval,
                            resolved_id: None,
                            static_key,
                            static_slice: None,
                            this_handle,
                            root_frame: self.nba_root_frame(lvalue),
                        });
                    }
                } else if let Some(id) = self.resolve_nba_target(lvalue) {
                    // Delayed NBA is transport, not inertial: push directly
                    // so pending same-signal updates are NOT cancelled.
                    // §4.9.4: matures in the target slot's NBA REGION — the
                    // separate queue commits inside apply_nba, so Active
                    // reads at the same time still see the old value.
                    //
                    // Reference-parity (#145): a delayed NBA whose captured
                    // value equals the CURRENT value is elided at SCHEDULE
                    // time (`q = 0; q <= #5 0; #1 q = 1;` leaves q at 1 —
                    // the reference never lands the update). Elide only when
                    // no delayed NBA is already pending for the id: a
                    // pending entry, not the register, is the truth then
                    // (the #142 rule).
                    let mut v = val.resize_for_assign(w);
                    // Normalize signedness to the TARGET before comparing —
                    // the apply site does the same at maturity, and without
                    // it the elision compare saw identical bits as unequal
                    // Values and always pushed.
                    v.is_signed = self.signal_signed[id];
                    let pending_here = self.delayed_nba.iter().any(|(_, sid, _)| *sid == id);
                    if pending_here || self.signal_table[id] != v {
                        self.delayed_nba.push((self.time + d, id, v));
                    }
                } else {
                    let frozen = self.freeze_lvalue_indices(lvalue);
                    let qval = if self.nba_target_is_assoc_elem(lvalue) {
                        val.clone()
                    } else {
                        val.resize_for_assign(w)
                    };
                    let static_key = self.nba_static_key_for_lvalue(lvalue);
                    let this_handle = self.this_stack.last().copied().flatten();
                    self.enqueue_context_nba(
                        NbaEntry {
                            lhs: Some(frozen),
                            value: qval,
                            resolved_id: None,
                            static_key,
                            static_slice: None,
                            this_handle,
                            root_frame: self.nba_root_frame(lvalue),
                        },
                        d,
                    );
                }
            }
            StatementKind::If {
                condition,
                then_stmt,
                else_stmt,
                ..
            } => {
                // §12.6: `if (e matches p) ...` — the pattern's `.name` bindings
                // are visible in the then-branch. The parser used to discard the
                // pattern and yield a literal 0, so the branch never ran.
                if let ExprKind::Matches { expr, pattern } = &condition.kind {
                    let mut binds: Vec<(String, Value)> = Vec::new();
                    if self.match_pattern(expr, pattern, &mut binds) {
                        let st = self.push_pattern_bindings(&binds);
                        self.exec_statement(then_stmt);
                        self.pop_pattern_bindings(st);
                    } else if let Some(el) = else_stmt {
                        self.exec_statement(el);
                    }
                    return;
                }
                if self.eval_expr(condition).is_true() {
                    self.exec_statement(then_stmt);
                } else if let Some(el) = else_stmt {
                    self.exec_statement(el);
                }
            }
            StatementKind::Case {
                unique_priority,
                kind,
                expr,
                items,
            } => {
                // IEEE 1800-2017 §12.6.1 pattern case:
                //   case (expr) matches <pattern> [&&& guard] : stmt ... endcase
                // Distinguished by items carrying a `pattern`.
                if items.iter().any(|i| i.pattern.is_some()) {
                    self.exec_case_matches(expr, items);
                    return;
                }
                let val = self.case_selector_value(kind, expr, items);
                // SV-2023: under unique/unique0/priority, pre-pass the items
                // and emit a violation message if multiple match or (for
                // unique/priority) none match without a default.
                if sv_parser::is_sv2023()
                    && unique_priority.is_some()
                    && self.unique_check_enabled(unique_priority.as_ref())
                {
                    let mut match_count = 0usize;
                    let mut has_default = false;
                    for item in items.iter() {
                        if item.is_default {
                            has_default = true;
                            continue;
                        }
                        for pat in &item.patterns {
                            let hit = match kind {
                                CaseKind::CaseInside => self.case_inside_match(&val, pat),
                                CaseKind::Casez => {
                                    let pv = self.eval_expr(pat);
                                    val.casez_eq(&pv).is_true()
                                }
                                CaseKind::Casex => {
                                    let pv = self.eval_expr(pat);
                                    val.casex_eq(&pv).is_true()
                                }
                                _ => {
                                    let pv = self.eval_expr(pat);
                                    val.case_eq(&pv).is_true()
                                }
                            };
                            if hit {
                                match_count += 1;
                                break;
                            }
                        }
                    }
                    let qual = match unique_priority {
                        Some(UniquePriority::Unique) => "unique",
                        Some(UniquePriority::Unique0) => "unique0",
                        Some(UniquePriority::Priority) => "priority",
                        None => "",
                    };
                    if match_count > 1 {
                        self.record_output(format!(
                            "[SV-2023 §12.5.3] {} case: multiple items matched ({})",
                            qual, match_count
                        ));
                    } else if match_count == 0
                        && !has_default
                        && matches!(
                            unique_priority,
                            Some(UniquePriority::Unique) | Some(UniquePriority::Priority)
                        )
                    {
                        self.record_output(format!(
                            "[SV-2023 §12.5.3] {} case: no item matched and no default",
                            qual
                        ));
                    }
                }
                let mut matched = false;
                for item in items.iter() {
                    if item.is_default {
                        continue;
                    }
                    for pat in &item.patterns {
                        let hit = match kind {
                            CaseKind::CaseInside => self.case_inside_match(&val, pat),
                            CaseKind::Casez => {
                                let pv = self.eval_expr(pat);
                                val.casez_eq(&pv).is_true()
                            }
                            CaseKind::Casex => {
                                let pv = self.eval_expr(pat);
                                val.casex_eq(&pv).is_true()
                            }
                            _ => {
                                let pv = self.eval_expr(pat);
                                val.case_eq(&pv).is_true()
                            }
                        };
                        if hit {
                            self.exec_statement(&item.stmt);
                            matched = true;
                            break;
                        }
                    }
                    if matched {
                        break;
                    }
                }
                if !matched {
                    for item in items {
                        if item.is_default {
                            self.exec_statement(&item.stmt);
                            break;
                        }
                    }
                }
            }
            StatementKind::For {
                init,
                condition,
                step,
                body,
            } => {
                if self.try_exec_full_packed_fill(init, condition.as_ref(), step, body) {
                    return;
                }
                // §12.7 / §6.21: a for-loop-declared variable is automatic and
                // scoped to the loop — it must SHADOW any same-named outer
                // signal, not clobber it. Without a local frame the loop var
                // would land on the outer signal (wrong width + destroyed
                // value: `logic idx` outer vs `for (int idx...)` spun forever on
                // a 1-bit counter). Push a throwaway frame so the loop var gets
                // its own declared-width storage; pop it at the arm's end.
                // §12.7.1 / §6.21: for-init-declared variables are scoped to
                // the loop — like foreach index vars, save the outer value and
                // restore it on exit. Without this, a comb block's
                // `for (int i...)` executed by a settle triggered MID-ITERATION
                // of another block's same-named loop (blocking assigns settle
                // synchronously) clobbered the interrupted loop's counter, and
                // that loop silently exited after one iteration.
                let fv_names: Vec<String> = init
                    .iter()
                    .filter_map(|fi| match fi {
                        ForInit::VarDecl { name, .. } => Some(name.name.clone()),
                        _ => None,
                    })
                    .collect();
                let fv_saved = self.snapshot_loop_vars(&fv_names);
                // An init-declared loop variable is automatic and loop-scoped
                // (§12.7/§6.21): it always gets its own frame when the
                // process has none. Probing for same-named outer signals only
                // caught the module-scope / process-scope shapes (u.i) — an
                // INLINED child instance's same-named leaf is invisible to the
                // probe, and the leaf-name fallback in name resolution then
                // bound the bare reads to the child's storage while the writes
                // landed on a bare signal: two different cells (IEEE 1800
                // §23.6 — an unqualified name must not resolve downward
                // into an instance). Pushing unconditionally is cheap and
                // closes every downward-collision shape.
                let shadow_frame = self.local_stack.last().is_none()
                    && init.iter().any(|fi| matches!(fi, ForInit::VarDecl { .. }));
                if shadow_frame {
                    self.push_loop_frame();
                }
                // Init-declared loop vars are automatic. With the frame push
                // above they always live in a local frame, never in the bare
                // signal table, so a fork in the body captures the
                // per-iteration value from the frame copy that
                // inherit_fork_child_context gives each child (LRM §9.3.2).
                for fi in init {
                    match fi {
                        ForInit::VarDecl {
                            data_type,
                            name,
                            init: e,
                        } => {
                            let rv = self.declare_for_init_var(data_type, &name.name, e);
                            // Declare the loop variable in the current call
                            // frame's local scope (highest read/write
                            // precedence) so its init, the step, and condition
                            // reads all agree — and so it properly *shadows* any
                            // same-named class member (riscv-dv reuses `hart`
                            // both as a field and as a loop variable; without
                            // shadowing the field's stale value made the second
                            // `for (int hart...)` never iterate).
                            if let Some(frame) = self.local_stack.last_mut() {
                                frame.insert(name.name.clone(), rv);
                            } else {
                                self.set_signal_value_by_name(&name.name, rv);
                            }
                        }
                        ForInit::Assign { lvalue, rvalue } => {
                            let v = self.eval_expr(rvalue);
                            self.assign_value(lvalue, &v);
                        }
                    }
                }
                // Loop vars declared signed (`for (int i = ...)`): the step
                // (`--i`, `i = i-1`) can write back an unsigned result, which
                // would make `i >= 0` always-true and spin a descending loop.
                // Re-assert their signedness after each step (cheap, only for
                // signed-typed for-decl vars).
                let signed_loop_vars: Vec<String> = init
                    .iter()
                    .filter_map(|fi| match fi {
                        ForInit::VarDecl {
                            data_type, name, ..
                        } if self.type_is_signed_concrete(data_type) => Some(name.name.clone()),
                        _ => None,
                    })
                    .collect();
                static LOOP_LIMIT_ENV: std::sync::OnceLock<Option<u64>> =
                    std::sync::OnceLock::new();
                let loop_limit: u64 = (*LOOP_LIMIT_ENV.get_or_init(|| {
                    std::env::var("XEZIM_LOOP_LIMIT")
                        .ok()
                        .and_then(|s| s.parse().ok())
                }))
                .unwrap_or(10_000_000);
                let between: Vec<&Expression> = condition.iter().chain(step.iter()).collect();
                let body_scope = self.loop_scope_open(body, &between, &fv_names);
                let mut iters: u64 = 0;
                loop {
                    if iters > loop_limit || self.finished {
                        break;
                    }
                    iters += 1;
                    if iters & LOOP_INTERRUPT_POLL_MASK == 0 && loop_stop_requested() {
                        self.take_interrupt();
                        break;
                    }
                    if let Some(c) = condition {
                        self.reset_hint_to_process_scope();
                        if !self.eval_expr(c).is_true() {
                            break;
                        }
                    }
                    self.break_flag = false;
                    self.continue_flag = false;
                    self.exec_statement(body);
                    if self.break_flag {
                        // Keep the flag set while a `disable` unwinds to an
                        // enclosing named block.
                        if self.disable_target.is_none() {
                            self.break_flag = false;
                        }
                        break;
                    }
                    self.continue_flag = false;
                    for s in step {
                        self.exec_expr_stmt(s);
                    }
                    for nm in &signed_loop_vars {
                        if let Some(frame) = self.local_stack.last_mut() {
                            if let Some(v) = frame.get_mut(nm) {
                                v.is_signed = true;
                                continue;
                            }
                        }
                        if let Some(v) = self.signals.get_mut(nm) {
                            v.is_signed = true;
                        }
                    }
                }
                self.loop_scope_close(body, body_scope);
                if shadow_frame {
                    self.pop_loop_frame();
                }
                self.restore_loop_vars(&fv_saved);
                // class-perf loop-heat boost (pass C): one call per loop exit.
                self.record_ast_loop_heat(iters);
            }
            // A foreach index variable is automatic and loop-scoped (§12.7.3):
            // like a for-init declaration it always gets its own frame when
            // the process has none. The old same-named-signal probe only
            // caught module-scope / process-scope shapes (u.i); an INLINED
            // child instance's same-named leaf was invisible to it, and the
            // leaf-name fallback then bound the bare index reads to the
            // child's storage while set_loop_var wrote a bare signal — two
            // different cells (IEEE 1800 §23.6). The re-dispatch below
            // sees a local frame and takes the normal path, so every exit of
            // that path pops nothing extra.
            StatementKind::Foreach { vars, .. }
                if self.local_stack.last().is_none() && vars.iter().any(|v| v.is_some()) =>
            {
                self.push_loop_frame();
                self.exec_statement(stmt);
                self.pop_loop_frame();
            }
            StatementKind::Foreach { array, vars, body } => {
                let names: Vec<String> = vars
                    .iter()
                    .filter_map(|v| v.as_ref().map(|id| id.name.clone()))
                    .collect();
                let body_scope = self.loop_scope_open(body, &[array], &names);
                self.exec_stmt_foreach(stmt, array, vars, body);
                self.loop_scope_close(body, body_scope);
            }
            StatementKind::While { condition, body } => {
                static LOOP_LIMIT_ENV: std::sync::OnceLock<Option<u64>> =
                    std::sync::OnceLock::new();
                let loop_limit: u64 = (*LOOP_LIMIT_ENV.get_or_init(|| {
                    std::env::var("XEZIM_LOOP_LIMIT")
                        .ok()
                        .and_then(|s| s.parse().ok())
                }))
                .unwrap_or(10_000_000);
                let body_scope = self.loop_scope_open(body, &[condition], &[]);
                let mut i: u64 = 0;
                loop {
                    if i > loop_limit || self.finished {
                        break;
                    }
                    i += 1;
                    if i & LOOP_INTERRUPT_POLL_MASK == 0 && loop_stop_requested() {
                        self.take_interrupt();
                        break;
                    }
                    self.reset_hint_to_process_scope();
                    if !self.eval_expr(condition).is_true() {
                        // §12.7.2: a `continue` on the FINAL iteration reaches
                        // this exit with the flag still set (the `for` arm
                        // clears it after the body; `while` re-tests the
                        // condition first). Leaked, it skips every statement
                        // after the loop — a function ends without executing
                        // its `return` and the caller reads a stale slot.
                        self.continue_flag = false;
                        break;
                    }
                    self.break_flag = false;
                    // §12.7.2: clear `continue` before each iteration, as the
                    // `for` loop already did. Left set, the body's very first
                    // statement saw the stale flag and skipped the whole block
                    // every time — the loop variable never advanced and a
                    // `while` containing a `continue` spun forever.
                    self.continue_flag = false;
                    self.exec_statement(body);
                    if self.break_flag {
                        // Keep the flag set while a `disable` unwinds to an
                        // enclosing named block.
                        if self.disable_target.is_none() {
                            self.break_flag = false;
                        }
                        break;
                    }
                }
                self.loop_scope_close(body, body_scope);
                // class-perf loop-heat boost (pass C): one call per loop exit.
                self.record_ast_loop_heat(i);
            }
            StatementKind::DoWhile { body, condition } => {
                static LOOP_LIMIT_ENV: std::sync::OnceLock<Option<u64>> =
                    std::sync::OnceLock::new();
                let loop_limit: u64 = (*LOOP_LIMIT_ENV.get_or_init(|| {
                    std::env::var("XEZIM_LOOP_LIMIT")
                        .ok()
                        .and_then(|s| s.parse().ok())
                }))
                .unwrap_or(10_000_000);
                let body_scope = self.loop_scope_open(body, &[condition], &[]);
                let mut i: u64 = 0;
                loop {
                    if i > loop_limit || self.finished {
                        break;
                    }
                    i += 1;
                    if i & LOOP_INTERRUPT_POLL_MASK == 0 && loop_stop_requested() {
                        self.take_interrupt();
                        break;
                    }
                    self.break_flag = false;
                    self.continue_flag = false;
                    self.exec_statement(body);
                    if self.break_flag {
                        // Keep the flag set while a `disable` unwinds to an
                        // enclosing named block.
                        if self.disable_target.is_none() {
                            self.break_flag = false;
                        }
                        break;
                    }
                    self.continue_flag = false;
                    if !self.eval_expr(condition).is_true() {
                        break;
                    }
                }
                self.loop_scope_close(body, body_scope);
                // class-perf loop-heat boost (pass C): one call per loop exit.
                self.record_ast_loop_heat(i);
            }
            StatementKind::Repeat { count, body } => {
                // §12.7.2: full count — the old 10000 cap silently truncated
                // large loops (and negatives-as-unsigned hit it constantly).
                let n = self.repeat_count(count);
                let body_scope = self.loop_scope_open(body, &[], &[]);
                let mut rp_it: u64 = 0;
                for k in 0..n {
                    if self.finished {
                        break;
                    }
                    if k & LOOP_INTERRUPT_POLL_MASK == LOOP_INTERRUPT_POLL_MASK
                        && loop_stop_requested()
                    {
                        self.take_interrupt();
                        break;
                    }
                    self.break_flag = false;
                    self.continue_flag = false;
                    rp_it += 1;
                    self.exec_statement(body);
                    if self.break_flag {
                        // Keep the flag set while a `disable` unwinds to an
                        // enclosing named block.
                        if self.disable_target.is_none() {
                            self.break_flag = false;
                        }
                        break;
                    }
                    self.continue_flag = false;
                }
                self.loop_scope_close(body, body_scope);
                // class-perf loop-heat boost (pass C): one call per loop exit.
                self.record_ast_loop_heat(rp_it);
            }
            StatementKind::Forever { body } => {
                let mut i: u64 = 0;
                loop {
                    if i > 100000 || self.finished || self.time > self.max_time {
                        break;
                    }
                    i += 1;
                    if i & LOOP_INTERRUPT_POLL_MASK == 0 && loop_stop_requested() {
                        self.take_interrupt();
                        break;
                    }
                    // §9.3.3 / §12.7.2 loop control, exactly as `while` does
                    // above. This arm honoured NO flag: a `break` left the
                    // body no-opping for the rest of the cap, and — the real
                    // damage — the flag SURVIVED the loop, so every statement
                    // after the `forever` (including `$finish`) was skipped
                    // and the run looked like a hang. Only reachable when the
                    // `forever` is nested inside a block, which is why a
                    // top-level one always worked.
                    self.break_flag = false;
                    self.continue_flag = false;
                    self.exec_statement(body);
                    if self.break_flag {
                        // Keep the flag set while a `disable` unwinds to an
                        // enclosing named block.
                        if self.disable_target.is_none() {
                            self.break_flag = false;
                        }
                        break;
                    }
                }
                // class-perf loop-heat boost (pass C): one call per loop exit.
                self.record_ast_loop_heat(i);
            }
            StatementKind::SeqBlock { stmts, name } => {
                // §9.3.1: a labelled procedural assertion is this block. A
                // `--code-coverage` statement marker beside it is not a
                // statement of the design: counted, it dropped the label of
                // every instrumented one (an `initial` block's, #586).
                if let Some(n) = name {
                    let mut real = stmts.iter().filter(|s| !Self::is_cov_marker(s));
                    if matches!(real.next(), Some(s) if matches!(s.kind, StatementKind::Assertion(_)))
                        && real.next().is_none()
                    {
                        self.assert_wrap_label = Some(n.name.clone());
                    }
                }
                // §6.21: a block-local declaration SHADOWS a same-named module
                // signal — without a frame the frameless VarDecl exec lands on
                // the signal itself and the block's writes clobber the module
                // variable (`begin logic [7:0] v; v = 8'hF0; end` overwrote
                // module `v`). Push a throwaway frame for the block's duration,
                // exactly like the for-loop shadow frame. Only when a direct
                // child declaration actually collides, so the common case pays
                // one scan and no allocation.
                let shadow_frame = self.local_stack.last().is_none()
                    && stmts.iter().any(|s| match &s.kind {
                        StatementKind::VarDecl { declarators, .. } => declarators
                            .iter()
                            .any(|d| self.signal_name_to_id.contains_key(d.name.name.as_str())),
                        _ => false,
                    });
                // With a call frame, the block's declarations land in it:
                // open the block's scope so they end with it (§6.21), unless
                // the loop running this body holds it open.
                let scope = if self.hoisted_scope.0 == stmt as *const Statement as usize
                    && self.local_gen_stack.last().copied() == Some(self.hoisted_scope.1)
                {
                    None
                } else {
                    self.block_scope_enter(stmts)
                };
                if shadow_frame {
                    self.push_local_frame(HashMap::default());
                }
                // `automatic` locals declared in this block (see the VarDecl
                // arm) are fork-capturable only while the block is in scope
                // (§6.21): each entry — e.g. each loop iteration — is a fresh
                // variable, so pop them on exit.
                let seq_auto_len = self.auto_loop_vars.len();
                // §21.2.1.7: a NAMED block adds its name to the `%m` hierarchy.
                let m_pushed = if let Some(n) = name {
                    self.m_scope_stack.push(n.name.clone());
                    true
                } else {
                    false
                };
                for s in stmts {
                    if self.finished || self.break_flag || self.continue_flag {
                        break;
                    }
                    self.exec_statement(s);
                }
                if m_pushed {
                    self.m_scope_stack.pop();
                }
                if shadow_frame {
                    self.pop_local_frame();
                }
                if let Some(sv) = scope {
                    self.block_scope_exit(stmts, sv);
                }
                self.auto_loop_vars.truncate(seq_auto_len);
                // A `disable` naming THIS block ends here; execution resumes
                // after it (§9.6.2).
                if let (Some(target), Some(n)) = (self.disable_target.as_deref(), name.as_ref()) {
                    if Self::block_name_matches(target, &n.name) {
                        self.disable_target = None;
                        self.disable_check_pending = false;
                        self.break_flag = false;
                        self.continue_flag = false;
                    }
                }
            }
            StatementKind::ParBlock {
                stmts,
                join_type,
                name: block_name,
                ..
            } => {
                // Mirror the suspend-aware path (run_process_stmts): children are
                // parented to the CURRENT pid (not 0) and inherit a *snapshot* of
                // the forking process's context. The snapshot captures automatic
                // variables (e.g. a loop index) by value at fork time, so the
                // canonical `for (int i…) fork … join_none` idiom captures the
                // per-iteration `i` instead of the parent's post-loop value
                // (LRM §9.3.2 automatic-variable semantics).
                // As in run_process_stmts: fork-block declarations are not
                // processes; run them here so each child snapshots them.
                let (spawnable, saved_auto_len) = self.exec_fork_block_decls(stmts);
                let mut child_set = HashSet::default();
                for s in spawnable {
                    let pid = self.next_pid;
                    self.next_pid += 1;
                    self.process_parents.insert(pid, self.current_pid);
                    // Same scope-hint inheritance as the run_process_stmts
                    // ParBlock arm — §9.3.2, additive.
                    if let Some(h) = self.process_scope_hint.get(&self.current_pid).cloned() {
                        self.process_scope_hint.insert(pid, h);
                    }
                    self.process_origin.insert(pid, (s.span, "fork child"));
                    self.inherit_fork_child_context(pid);
                    self.inherit_fork_m_scope(pid, self.current_pid, block_name.as_ref());
                    let frame = self.fork_child_frame(s);
                    self.event_queue
                        .schedule(self.time, pid, ProcCont::from_frame(frame));
                    child_set.insert(pid);
                }
                self.auto_loop_vars.truncate(saved_auto_len);
                if let Some(nm) = block_name {
                    self.fork_block_children
                        .insert(nm.name.clone(), child_set.clone());
                }
                match join_type {
                    // `join` waits for all, `join_any` for the first; both suspend
                    // the forking process via a JoinWaiter. `join_none` continues.
                    // An empty fork has no children to wait for, so it must NOT
                    // suspend — a childless waiter is never re-checked.
                    // A task entered from C (DPI export) cannot park: run the
                    // scheduler until the join is satisfied, then carry on with
                    // the statements after the fork (#204).
                    JoinType::Join | JoinType::JoinAny
                        if !child_set.is_empty() && self.dpi_export_depth > 0 =>
                    {
                        let (id, wpid, wake) = self.sync_wait_target();
                        self.join_waiters.push(JoinWaiter {
                            parent_pid: wpid,
                            child_pids: child_set,
                            join_type: *join_type,
                            continuation: wake,
                            finished_children: HashSet::default(),
                            wait_fork: false,
                        });
                        self.sync_wait(id);
                    }
                    JoinType::Join | JoinType::JoinAny if !child_set.is_empty() => {
                        self.join_waiters.push(JoinWaiter {
                            parent_pid: self.current_pid,
                            child_pids: child_set,
                            join_type: *join_type,
                            continuation: Vec::new().into(),
                            finished_children: HashSet::default(),
                            wait_fork: false,
                        });
                        self.break_flag = true; // Suspend current execution
                    }
                    _ => {}
                }
            }
            StatementKind::TimingControl { control, stmt } => {
                match control {
                    // §35.5.2: a delay inside an imported task running on its
                    // own stack parks the calling process (dpi_task.rs).
                    TimingControl::Delay(d) if self.dpi_cur_task.is_some() => {
                        self.dpi_wait_delay(d);
                        self.exec_statement(stmt);
                    }
                    TimingControl::Delay(d) => {
                        // The nested work below (run_events_until, settle,
                        // edge cascades) executes OTHER processes and comb
                        // entries whose dotted-name resolutions leave a sticky
                        // `name_resolve_hint` behind. That residue must not
                        // leak back into THIS process's bare-name resolution:
                        // a leaked sibling-instance hint once made a
                        // testbench's loop variable `c` shadow-resolve to the
                        // DUT instance's `c` (then the per-node cache froze
                        // it and the loop never terminated). Save/restore the
                        // hint across the whole nested-delay window.
                        let saved_hint_nested = self.name_resolve_hint.borrow().clone();
                        let delay = self.eval_delay_ticks(d);
                        // §4.4/§4.5: the NBA region of the CURRENT timestamp
                        // runs after every active-region process at that
                        // timestamp has evaluated its RHS. Flushing here is
                        // premature when this delay is inside an EDGE block:
                        // the remaining blocks on the same edge have not
                        // sampled yet, and committing now lets them read the
                        // post-edge value. A `always @(posedge clk) begin #1;
                        // … end` observer anywhere in a testbench therefore
                        // broke every OTHER clocked block that sampled a
                        // signal written by a sibling block on that edge —
                        // `d <= dut.if.sig;` recorded the new value, so a
                        // one-cycle pipeline check saw no delay at all.
                        // Outside an edge block (a plain initial/task `#d`)
                        // keep flushing: the active region there is just this
                        // process, and `run_events_until` below re-flushes at
                        // the proper point either way.
                        if !self.in_edge_block {
                            self.apply_nba();
                        }
                        self.settle_combinatorial();
                        self.check_edges();
                        let _ = self.drain_edge_cascade(self.cascade_limit);
                        self.snapshot_edge_signals();
                        let target = self.time + delay;
                        // Run scheduled continuations (other processes,
                        // clock toggles, delayed updates) whose fire time
                        // falls inside this delay window so concurrent
                        // blocks advance while we're sleeping in a task.
                        self.run_events_until(target);
                        if self.time < target {
                            self.time = target;
                            publish_sim_time(target);
                        }
                        // Full active→inactive→NBA traversal at the new
                        // time, per LRM §4.4: a delayed update or clocked
                        // block scheduled AT target itself must fire before
                        // the sleeping process resumes.
                        if self.apply_delayed_updates() {
                            self.dirty_any = true;
                        }
                        if self
                            .clock_generators
                            .iter()
                            .any(|c| c.next_toggle_time == self.time)
                        {
                            self.fire_clock_generators();
                            self.dirty_any = true;
                        }
                        if !self.nba_fast.is_empty()
            || !self.nba_queue.is_empty()
            // §15.5.2: a bare `->>` (deferred trigger) must flush with the NBA
            // region even when no NBA DATA is queued.
            || !self.pending_nba_triggers.is_empty()
                || !self.packed_nba.is_empty()
            || !self.pending_nba_instance_triggers.is_empty()
                        {
                            self.apply_nba();
                        }
                        self.settle_combinatorial();
                        self.check_edges();
                        let _ = self.drain_edge_cascade(self.cascade_limit);
                        self.snapshot_edge_signals();
                        // The whole postponed region, not just `$monitor`. This
                        // slot is reached by jumping `self.time` to the delay's
                        // target, and the dump was left out — so the monitor
                        // reported the slot and the waveform did not, and the
                        // changes surfaced later at whatever tick next wrote the
                        // dump. That is why a monitor and a VCD of the same
                        // event disagreed with each other as well as with the
                        // change itself.
                        self.run_postponed_region();
                        *self.name_resolve_hint.borrow_mut() = saved_hint_nested;
                    }
                    // A task entered from C (DPI export) cannot park: register
                    // the waiter on a wake marker and run the scheduler until
                    // the event fires; the body then runs below (#204).
                    TimingControl::Event(e) if self.dpi_export_depth > 0 => {
                        let (id, wpid, wake) = self.sync_wait_target();
                        let mut key = None;
                        if let Some(fname) = self.event_control_field_name(e) {
                            key = self.resolve_this_event_field(&fname);
                        }
                        if key.is_none() {
                            if let Some(expr) = Self::event_control_single_expr(e) {
                                key = self.expr_handle_event_field(&expr);
                                if key.is_none() {
                                    key = self.expr_instance_event_field_general(&expr);
                                }
                            }
                        }
                        if let Some(key) = key {
                            self.instance_event_waiters.push(InstanceEventWaiter {
                                key: InstanceEventKeys::One(key),
                                pid: wpid,
                                continuation: wake,
                            });
                        } else {
                            let sens = self.event_to_sens(e);
                            let is_clk_ev = self.is_clocking_event(e);
                            let w = self.make_event_waiter_kind(wpid, sens, wake, is_clk_ev);
                            self.event_waiters.push(w);
                        }
                        self.sync_wait(id);
                    }
                    TimingControl::Event(e) => {
                        // Class-field named event parked from the synchronous
                        // executor (a `fork ... join_none` body executing
                        // inline, e.g. a heartbeat start function): route to the
                        // per-instance waiter list so `->m_event` can wake it.
                        // Same rationale as the run_process_stmts arm.
                        if let Some(fname) = self.event_control_field_name(e) {
                            if let Some(key) = self.resolve_this_event_field(&fname) {
                                let cont = vec![*stmt.clone()];
                                let pid = self.current_pid;
                                self.instance_event_waiters.push(InstanceEventWaiter {
                                    key: InstanceEventKeys::One(key),
                                    pid,
                                    continuation: cont.into(),
                                });
                                self.break_flag = true;
                                return;
                            }
                        }
                        // Member-reached event property (`@(h.ce)`,
                        // `@(m_events[obj].all_dropped)`) — same routing as the
                        // run_process_stmts arm; without it these fell into the
                        // generic waiter below with a sensitivity that never
                        // matches an instance event.
                        if let Some(expr) = Self::event_control_single_expr(e) {
                            if let Some(key) = self
                                .expr_handle_event_field(&expr)
                                .or_else(|| self.expr_instance_event_field_general(&expr))
                            {
                                let cont = vec![*stmt.clone()];
                                let pid = self.current_pid;
                                self.instance_event_waiters.push(InstanceEventWaiter {
                                    key: InstanceEventKeys::One(key),
                                    pid,
                                    continuation: cont.into(),
                                });
                                self.break_flag = true;
                                return;
                            }
                        }
                        if let Some(cond) = self.class_prop_value_change_cond(e) {
                            let pid = self.current_pid;
                            let wait = Self::vc_wait_stmt(cond.clone(), stmt, stmt.span);
                            self.park_condition_waiter(pid, vec![wait].into(), &cond);
                            self.break_flag = true;
                            return;
                        }
                        let sens = self.event_to_sens(e);
                        let is_clk_ev = self.is_clocking_event(e);
                        sim_dbg_eprintln!(
                            "[DEBUG] process {} waiting for event {:?} at time {}",
                            self.current_pid,
                            sens,
                            self.time
                        );
                        // Suspension is handled by run_process_stmts for top-level timing controls.
                        // If we are here, it's a nested timing control which we don't fully support yet.
                        let cont = vec![*stmt.clone()];
                        let pid = self.cg_this.unwrap_or(0); // placeholder
                        {
                            let w = self.make_event_waiter_kind(pid, sens, cont.into(), is_clk_ev);
                            self.event_waiters.push(w);
                        }
                        self.break_flag = true;
                        return;
                    }
                }
                self.exec_statement(stmt);
                // After body executes, check for edges (e.g., clk toggled)
                // and drain any cascade — see drain_edge_cascade.
                self.settle_combinatorial();
                self.check_edges();
                let _ = self.drain_edge_cascade(self.cascade_limit);
            }
            StatementKind::Break => {
                self.break_flag = true;
            }
            StatementKind::Continue => {
                self.continue_flag = true;
            }
            StatementKind::Return(expr) => {
                if let Some(rhs) = expr
                    && let Some(selected) = self.selected_aggregate_rhs(None, rhs)
                {
                    self.exec_statement(&Statement::new(
                        StatementKind::Return(Some(selected)),
                        stmt.span,
                    ));
                    return;
                }

                if let Some(e) = expr {
                    // Parentheses do not change an unpacked return's shape.
                    // Recognize the collection before falling back to a
                    // scalar Value (which would turn an empty queue into {0}).
                    let e = Self::unparen(e);
                    // One call for a receiver call in the returned chain
                    // (`return me().o.x;`), shared by the probes below.
                    let pinned = Self::recv_chain_call(e)
                        .is_some()
                        .then(|| self.pin_stmt_recv_calls(&[e]))
                        .flatten();
                    // An operand no virtual-interface source can name: both
                    // vif lookups below come back empty for it.
                    let vif_free = !self.vif_rhs_possible(e);
                    // §25.9: `return val` where val carries a vif binding —
                    // record it so the caller's `x = f()` re-binds x
                    // (uvm_resource#(virtual if)::read; issue #113).
                    if let Some(nm) = (!vif_free)
                        .then(|| self.resolve_vif_rhs_name_strict(e))
                        .flatten()
                    {
                        self.signals
                            .insert("__vif_return__".to_string(), Value::from_string(&nm));
                        self.vif_return_pending = true;
                    }
                    // §13.4: `return <collection>` — record the collection's
                    // storage name so the caller's assign can copy elements
                    // (a packed Value cannot carry them). Values a compiled
                    // callee left in `vm_ret_vals` belong to an earlier call.
                    self.pending_ret_collection = None;
                    self.vm_ret_vals = None;
                    // `instance_assoc_member` of a one-segment name, when
                    // computed below: it is also `expr_assoc_name`'s answer.
                    let mut bare_member: Option<Option<String>> = None;
                    if let ExprKind::Ident(h) = &e.kind {
                        let bare = h.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
                        // A queue / dynamic-array member lives at `<handle>#member`,
                        // not under the bare name that `resolve_hier_name`
                        // returns (its per-node cache doesn't consult
                        // `instance_assoc_member`). Resolve the instance-scoped
                        // name explicitly so `pending_ret_collection` captures
                        // the real storage.
                        // A subroutine-LOCAL dyn array/queue lives under its
                        // process-unique rename (`declare_local_dyn`), so
                        // `return tmp;` must resolve through the frame's
                        // rename first — the bare name isn't in
                        // `dynamic_arrays` (ivtest sv_darray_function).
                        //
                        // §8.9/§10 scope: an unqualified name in a method
                        // body resolves to (1) a local of the CURRENT frame,
                        // (2) a member of THIS class, then (3) an enclosing
                        // frame's local (inlined blocking calls share the
                        // caller's scope). `dyn_name_lookup` walks every
                        // frame, so a caller's same-named local could steal
                        // the slot from the class member — e.g.
                        // `uvm_report_message::do_print`'s local
                        // `elements[$]` shadowing the container's `elements`
                        // member that `get_elements` returns, recording the
                        // wrong (empty) storage under `pending_ret_collection`
                        // and dropping every message element.
                        let n = self
                            .dyn_name_lookup_innermost(bare)
                            .map(|s| std::borrow::Cow::Owned(s.to_string()))
                            .or_else(|| {
                                let m = self.instance_assoc_member(bare);
                                if h.path.len() == 1 {
                                    bare_member = Some(m.clone());
                                }
                                m.map(std::borrow::Cow::Owned)
                            })
                            .or_else(|| {
                                self.dyn_name_lookup_enclosing(bare)
                                    .map(|s| std::borrow::Cow::Owned(s.to_string()))
                            })
                            .unwrap_or_else(|| self.resolve_hier_name(h));
                        if self.module.dynamic_arrays.contains(&*n) {
                            // Frame teardown wipes a subroutine-LOCAL
                            // collection before the caller's assignment
                            // consumes it (the recorded name read back size
                            // 0) — snapshot elements+size under a reserved
                            // name that survives the pop, unless the element
                            // type is an unpacked struct (that copy path is
                            // keyed by the ORIGINAL name; module-scope
                            // collections aren't wiped, so keep them live).
                            if self.queue_elem_struct(&n).is_none() {
                                const RET_SNAP: &str = "__xz_ret_coll__";
                                self.copy_whole_queue(RET_SNAP, &n);
                                self.pending_ret_collection = Some(RET_SNAP.to_string());
                            } else {
                                self.pending_ret_collection = Some(n.to_string());
                            }
                        } else if self
                            .fn_ret_collection_stack
                            .last()
                            .copied()
                            .unwrap_or(false)
                            && h.path.iter().all(|seg| seg.selects.is_empty())
                        {
                            // A scalar local can shadow a module's fixed
                            // array; only an unpacked return consumes it whole.
                            self.snapshot_ret_fixed(&n);
                        }
                    }
                    if self.pending_ret_collection.is_none() {
                        let an = match bare_member {
                            Some(m) => m,
                            None => self.expr_assoc_name(e),
                        };
                        if let Some(an) = an {
                            if !self.is_associative_array(&an) {
                                self.pending_ret_collection = Some(an);
                            }
                        }
                    }
                    // §13.4, §7.9.9: `return aa;` from a function whose return
                    // type is an associative array. The frame teardown wipes
                    // a local array before the caller's assignment reads it,
                    // so snapshot its entries under a reserved name.
                    if self.pending_ret_collection.is_none()
                        && self.fn_ret_assoc_stack.last().copied().unwrap_or(false)
                    {
                        if let Some(src) = self.ret_assoc_operand_name(e) {
                            self.snapshot_ret_assoc(&src);
                        }
                    }
                    // §13.4: `return '{}`/`return '{...}` — a whole-collECTION
                    // LITERAL returned directly (queue / unsized dynamic array).
                    // The Ident path above cannot fire (there is no storage
                    // name), and `eval_expr` alone collapses a queue to a
                    // single packed scalar, so the caller's `q = f()` lost
                    // elements — an empty `'{}` came back as size 1 (one
                    // phantom element), and `'{a,b}` kept only `a`. When the
                    // CURRENT function returns a collection, snapshot the
                    // literal's elements into the reserved ret name and hand
                    // it through `pending_ret_collection` exactly as the
                    // Ident-return path does.
                    if self.pending_ret_collection.is_none()
                        && self
                            .fn_ret_collection_stack
                            .last()
                            .copied()
                            .unwrap_or(false)
                    {
                        if matches!(
                            e.kind,
                            ExprKind::AssignmentPattern(_) | ExprKind::Concatenation(_)
                        ) {
                            const RET_SNAP: &str = "__xz_ret_coll__";
                            self.populate_queue_from_init(RET_SNAP, e);
                            self.pending_ret_collection = Some(RET_SNAP.to_string());
                        }
                    }
                    // §13.4.1: an unpacked struct that is an ELEMENT of a
                    // class-property collection (`return q[i];`) or a
                    // subroutine local (`return r;`) has no container value;
                    // assemble it from its leaves.
                    let v = if self.pending_ret_collection.is_some() {
                        // §13.4.1: an unpacked return has already been read
                        // into its collection snapshot. Evaluating its bare
                        // function-name variable as a scalar would call the
                        // function again, recursively, because it has no
                        // packed container value.
                        Value::zero(32)
                    } else {
                        match self.struct_elem_type(e) {
                            Some(su) => self.pack_struct_elem_ref(e, &su),
                            None => match self.frame_struct_local(e) {
                                Some((name, su)) => self
                                    .pack_unpacked_struct(&name, &su)
                                    .unwrap_or_else(|| self.eval_expr(e)),
                                None => self.eval_expr(e),
                            },
                        }
                    };
                    if self.struct_colls_possible()
                        && matches!(
                            e.kind,
                            ExprKind::Ident(_)
                                | ExprKind::MemberAccess { .. }
                                | ExprKind::Index { .. }
                        )
                        && self.root_may_hold_struct(e)
                    {
                        self.stage_ret_struct_colls(e);
                    }
                    self.return_value = Some(v);
                    // §25.9: return the interface's handle token as well as
                    // its binding, so a memoized `getter().member` receiver
                    // can resolve the instance without repeating the call.
                    let vif_free = !self.vif_rhs_possible(e);
                    if let Some(bound) = (!vif_free).then(|| self.resolve_vif_rhs_name(e)).flatten()
                    {
                        if self.is_interface_instance(&bound) {
                            self.return_value =
                                Some(Value::from_u64(Self::iface_handle_token(&bound), 32));
                            self.last_vif_return = Some(bound);
                        }
                    }
                    if let Some(base) = pinned {
                        self.recv_call_memo.truncate(base);
                    }
                }
                self.break_flag = true;
                self.return_flag = true;
            }
            StatementKind::Disable(name) => {
                // §38.36.1 cbDisable, reported once the disable took effect.
                if self.vpi_cb_mask & vpi_cb::m(vpi_cb::CB_DISABLE) != 0 {
                    self.vpi_note_disable(&name.name);
                }
                // IEEE 1800-2017 §9.6.2. `disable_target` was set but never
                // read: the only effect was `break_flag`, a GLOBAL flag. So a
                // `disable` broke the innermost loop instead of terminating the
                // named block, truncated the disabling process, and leaked into
                // unrelated processes.
                //
                // A named `fork : blk ... join*` block: terminate the processes
                // it spawned (and their descendants), and CONTINUE. A join_none
                // fork block has already returned, so trying to unwind to it
                // below would run off the end of the disabling process and drop
                // its continuation — which is exactly what used to happen.
                if let Some(children) = self.fork_block_children.get(&name.name).cloned() {
                    let mut to_kill: HashSet<usize> = HashSet::default();
                    for &c in &children {
                        to_kill.insert(c);
                        to_kill.extend(self.collect_fork_descendants(c));
                    }
                    // The disabling process may itself be one of them (a child
                    // or deeper descendant disabling its own fork block): it
                    // is terminated with its siblings and unwinds to its end.
                    let self_doomed = to_kill.contains(&self.current_pid);
                    {
                        for &pid in &to_kill {
                            self.killed_pids.insert(pid);
                            self.release_killed_process_state(pid);
                            self.vc_cancel_process(pid);
                            self.process_parents.remove(&pid);
                            self.process_contexts.remove(&pid);
                            // §9.6.2: purge the killed pid's future-time
                            // #delay too, or the wheel keeps reporting
                            // activity and the idle break never fires
                            // (UVM's run-phase watchdog timer).
                            self.event_queue.remove_pid(pid);
                        }
                        self.event_waiters.retain(|w| !to_kill.contains(&w.pid));
                        self.instance_event_waiters
                            .retain(|w| !to_kill.contains(&w.pid));
                        for q in self.mailbox_get_waiters.values_mut() {
                            q.retain(|w| !to_kill.contains(&w.pid));
                        }
                        for q in self.semaphore_get_waiters.values_mut() {
                            q.retain(|w| !to_kill.contains(&w.pid));
                        }
                        self.await_waiters
                            .retain(|w| !to_kill.contains(&w.waiter_pid));
                        self.release_killed_from_join_waiters(&to_kill);
                        self.wake_await_waiters(&to_kill);
                        if self_doomed {
                            self.disable_target = Some(name.name.clone());
                            self.break_flag = true;
                        }
                        return;
                    }
                }
                // A label naming ANOTHER process's top-level block terminates
                // that process; the disabling process carries on.
                if let Some(&pid) = self.disable_labels.get(&name.name) {
                    if pid != self.current_pid {
                        self.killed_pids.insert(pid);
                        self.release_killed_process_state(pid);
                        self.vc_cancel_process(pid);
                        self.process_parents.remove(&pid);
                        self.process_contexts.remove(&pid);
                        self.event_queue.remove_pid(pid);
                        self.event_waiters.retain(|w| w.pid != pid);
                        self.instance_event_waiters.retain(|w| w.pid != pid);
                        for q in self.mailbox_get_waiters.values_mut() {
                            q.retain(|w| w.pid != pid);
                        }
                        for q in self.semaphore_get_waiters.values_mut() {
                            q.retain(|w| w.pid != pid);
                        }
                        self.await_waiters.retain(|w| w.waiter_pid != pid);
                        let mut tset = HashSet::default();
                        tset.insert(pid);
                        self.wake_await_waiters(&tset);
                        let mut killed: HashSet<usize> = HashSet::default();
                        killed.insert(pid);
                        self.release_killed_from_join_waiters(&killed);
                        return;
                    }
                }
                // §9.6.2: `disable <task_name>` where OTHER processes are
                // currently executing an invocation of that task terminates
                // those invocations. (Approximation: the whole executing
                // process is terminated — exact for the common `fork
                // task_a; task_b; join` + `disable task_b` shape.)
                if let Some(pids) = self.active_task_pids.get(&name.name).cloned() {
                    // An ANCESTOR executing the task (the disabling process was
                    // forked inside it) is unwound out of the call instead —
                    // see `disable_remote_block`.
                    let mut to_kill: HashSet<usize> = pids
                        .into_iter()
                        .filter(|&p| {
                            p != self.current_pid
                                && !self.killed_pids.contains(&p)
                                && !self.is_fork_ancestor(p, self.current_pid)
                        })
                        .collect();
                    // §35.9: an EXPORTED task that is the target returns 0 to
                    // its C caller, and the process carries on in the C code.
                    let handled = !self.dpi_tasks.is_empty()
                        && self.dpi_disable_exported_task(&name.name, &mut to_kill);
                    if to_kill.is_empty() && handled {
                        return;
                    }
                    if !to_kill.is_empty() {
                        for &pid in &to_kill {
                            self.killed_pids.insert(pid);
                            self.release_killed_process_state(pid);
                            self.vc_cancel_process(pid);
                            self.process_parents.remove(&pid);
                            self.process_contexts.remove(&pid);
                            // §9.6.2: purge the killed pid's future-time
                            // #delay too, or the wheel keeps reporting
                            // activity and the idle break never fires
                            // (UVM's run-phase watchdog timer).
                            self.event_queue.remove_pid(pid);
                        }
                        self.event_waiters.retain(|w| !to_kill.contains(&w.pid));
                        self.instance_event_waiters
                            .retain(|w| !to_kill.contains(&w.pid));
                        for q in self.mailbox_get_waiters.values_mut() {
                            q.retain(|w| !to_kill.contains(&w.pid));
                        }
                        for q in self.semaphore_get_waiters.values_mut() {
                            q.retain(|w| !to_kill.contains(&w.pid));
                        }
                        self.release_killed_from_join_waiters(&to_kill);
                        for set in self.active_task_pids.values_mut() {
                            for p in &to_kill {
                                set.remove(p);
                            }
                        }
                        return;
                    }
                }
                // Otherwise unwind this process to the end of the named block
                // (or the named task): the SeqBlock / task-call arm clears the
                // flags when it sees its own name, so execution resumes after
                // it. For a loop-body block that is `continue`; for a block
                // enclosing the loop it is `break`.
                self.disable_target = Some(name.name.clone());
                self.break_flag = true;
                // Not yet known to be this process's own block: the process
                // loop checks, and hands it to `disable_remote_block` if not.
                self.disable_check_pending = true;
            }
            StatementKind::DisableFork => {
                // LRM §9.6.2: terminate all active descendant processes of the
                // calling process (children AND their descendants), then return.
                let cur = self.current_pid;
                let to_kill = self.collect_fork_descendants(cur);
                for &pid in &to_kill {
                    // Mark killed so any already-queued continuation is dropped
                    // at dispatch, and purge every place the process can be
                    // parked so it never resumes and never satisfies a join.
                    self.killed_pids.insert(pid);
                    self.release_killed_process_state(pid);
                    self.vc_cancel_process(pid);
                    self.process_parents.remove(&pid);
                    self.process_contexts.remove(&pid);
                    // §9.6.2: incl. its future-time #delay (see above).
                    self.event_queue.remove_pid(pid);
                }
                self.event_waiters.retain(|w| !to_kill.contains(&w.pid));
                self.instance_event_waiters
                    .retain(|w| !to_kill.contains(&w.pid));
                for q in self.mailbox_get_waiters.values_mut() {
                    q.retain(|w| !to_kill.contains(&w.pid));
                }
                for q in self.semaphore_get_waiters.values_mut() {
                    q.retain(|w| !to_kill.contains(&w.pid));
                }
                self.await_waiters
                    .retain(|w| !to_kill.contains(&w.waiter_pid));
                // A killed process must no longer hold back a join waiter: drop
                // it from every waiter's expected children, and wake any waiter
                // whose remaining children are now all accounted for.
                self.release_killed_from_join_waiters(&to_kill);
                self.wake_await_waiters(&to_kill);
            }
            StatementKind::WaitFork => {
                // §9.6.1 in a task entered from C (DPI export): the task cannot
                // park, so wait here until every child of this process is done
                // (#204). Elsewhere the suspend-aware runner handles it.
                if self.dpi_cur_task.is_some() {
                    self.dpi_wait_fork();
                } else if self.dpi_export_depth > 0 {
                    let me = self.current_pid;
                    self.run_nested_until(|sim| !sim.process_parents.values().any(|&p| p == me));
                }
            }
            StatementKind::RsReturn => {
                self.rs_return_flag = true;
                self.break_flag = true;
            }
            StatementKind::RsAction { body } => {
                let prev = self.rs_return_flag;
                self.rs_return_flag = false;
                self.exec_statement(body);
                let triggered = self.rs_return_flag;
                self.rs_return_flag = prev;
                if triggered {
                    // Consume the break we raised for our RsReturn so the
                    // enclosing sequence continues with the next production.
                    self.break_flag = false;
                }
            }
            StatementKind::Wait { condition, stmt } => {
                if self.wait_condition_true(condition) {
                    self.exec_statement(stmt);
                } else if self.dpi_cur_task.is_some() {
                    // §35.5.2: inside an imported task running on its own
                    // stack, the calling process parks until the condition
                    // holds (dpi_task.rs).
                    self.dpi_wait_condition(condition);
                    if !self.finished {
                        self.exec_statement(stmt);
                    }
                } else if self.dpi_export_depth > 0 {
                    // A task entered from C (DPI export, §35.8) cannot park:
                    // wait here, running the scheduler until the condition
                    // holds (#204). §9.7.4: the condition is level-sensitive,
                    // so it is evaluated here, in the task's own frame, after
                    // every time slot the scheduler runs. It used to be parked
                    // as a condition waiter of a fresh process instead, which
                    // the nested loop never re-checked unless a recognised
                    // write named one of its operands: an event's triggered
                    // state or a pending clock left it parked until the run
                    // ended, and the fresh process could not see the task's
                    // automatic variables at all (#279).
                    self.run_nested_until(|sim| sim.wait_condition_true(condition));
                    if !self.finished {
                        self.exec_statement(stmt);
                    }
                } else {
                    // IEEE 1800-2023 §9.7.4: `wait(cond)` must block when the
                    // condition is false. `exec_statement` is the synchronous
                    // path (used for loop bodies), so it normally can't park.
                    // But `run_process_stmts` may have set `exec_park_cont` to
                    // the full continuation. If so, park the process and set
                    // return_flag to unwind all the way back to run_process_stmts.
                    if let Some(cont) = self.exec_park_cont.take() {
                        self.park_condition_waiter(self.current_pid, cont, condition);
                        self.parked_from_exec = true;
                        self.return_flag = true;
                        self.break_flag = true;
                    }
                }
            }
            StatementKind::Assertion(a) => {
                use crate::ast::stmt::AssertionKind;
                // The site's label: a module item's own (§16.5), or the name
                // of the block the parser wrapped a labelled procedural
                // statement in (§9.3.1).
                let wrap_label = self.assert_wrap_label.take();
                let site_label: Option<String> =
                    a.label.as_ref().map(|l| l.name.clone()).or(wrap_label);
                // LRM §16.5 — concurrent (`assert property (…)`) defers
                // evaluation to the observed region. We queue an
                // `ObservedProbe`; the drain in `drain_observed_region`
                // re-evaluates the predicate against the post-NBA settled
                // state and folds the outcome into assertion_stats. The
                // action / else_action are not run for concurrent variants
                // today — they'd need to fire in the reactive region,
                // which is its own follow-up.
                if a.is_property {
                    // §16.15 `default disable iff <guard>;` (a marker item
                    // the parser emits in the declaring scope).
                    if let ExprKind::SystemCall { name, args } = &a.expr.kind
                        && name == "$sva_default_disable"
                    {
                        if let Some(g) = args.first() {
                            let mut g = g.clone();
                            self.sva_resolve_signals(&mut g);
                            // Keyed by the declaring scope's full path (a
                            // generate block included).
                            let scope = self.m_path();
                            self.sva_default_disable.insert(scope, g);
                            // It governs the whole scope, so assertions
                            // registered before it are covered too.
                            for i in 0..self.sva_sites.len() {
                                if !self.sva_sites[i].explicit_disable {
                                    let key = self.sva_sites[i].disable_scope.clone();
                                    self.sva_sites[i].disable = self.sva_default_disable_for(&key);
                                }
                            }
                        }
                        return;
                    }
                    // LRM §16.6: `assert property (p_name);` —
                    // resolve a bare property-name reference to its
                    // stored body before dispatching to SVA. The
                    // property's body was captured at parse time
                    // (common `@(clk) <expr>` shape) and stored in
                    // `module.property_decls`.
                    let resolved_expr: Expression = match &a.expr.kind {
                        ExprKind::Ident(h) if h.path.len() == 1 => {
                            let name = &h.path[0].name.name;
                            if let Some(body) = self.module.property_decls.get(name).cloned() {
                                body
                            } else {
                                a.expr.clone()
                            }
                        }
                        // §16.6: `assert property (p(actuals));` — an
                        // INSTANTIATION of a named property with a port
                        // list. Resolve the body and substitute each formal
                        // for its positional actual (the substitution walker
                        // descends into SvaClocked, so a formal used as the
                        // CLOCK — `p(clk, ...)` — binds too). Previously
                        // this shape fell through unresolved and the assert
                        // was silently dropped: it never passed, never
                        // failed, and read as always-true.
                        ExprKind::Call { func, args } => {
                            let pname: Option<String> = match &func.kind {
                                ExprKind::Ident(h)
                                    if !h.path.is_empty()
                                        && h.path.iter().all(|s| s.selects.is_empty()) =>
                                {
                                    Some(h.path.last().unwrap().name.name.clone())
                                }
                                _ => None,
                            };
                            let resolved = pname.and_then(|n| {
                                let body = self.module.property_decls.get(&n).cloned()?;
                                let formals = self
                                    .module
                                    .property_params
                                    .get(&n)
                                    .cloned()
                                    .unwrap_or_default();
                                let mut map: HashMap<String, Expression> = HashMap::default();
                                for (i, f) in formals.iter().enumerate() {
                                    if let Some(actual) = args.get(i) {
                                        map.insert(f.clone(), actual.clone());
                                    }
                                }
                                Some(super::elaborate::rewrite_expr(
                                    &body,
                                    "",
                                    &map,
                                    &std::collections::HashSet::new(),
                                    &HashMap::default(),
                                ))
                            });
                            resolved.unwrap_or_else(|| a.expr.clone())
                        }
                        _ => a.expr.clone(),
                    };
                    // LRM §16.5: a `SvaClocked{events, body}` body
                    // registers a clocked site instead of going through
                    // the per-iteration observed-probe path. Dedup by
                    // span_key so re-execution (e.g. inside an always
                    // block) doesn't grow the site list unbounded.
                    // A clockless `assert property` samples on the default
                    // clocking (§14.12).
                    let clocked: Option<(Vec<EventExpr>, Expression)> = match &resolved_expr.kind {
                        ExprKind::SvaClocked { events, body } => {
                            Some((events.clone(), (**body).clone()))
                        }
                        _ if a.inferred_clock.is_some() => Some((
                            vec![a.inferred_clock.clone().unwrap()],
                            resolved_expr.clone(),
                        )),
                        _ => match self.clocking_meta.get("__xz_default_clocking") {
                            Some((clk, _)) => {
                                let edge = match self.clocking_edge.get("__xz_default_clocking") {
                                    Some(EdgeKind::Negedge) => Edge::Negedge,
                                    _ => Edge::Posedge,
                                };
                                let event = EventExpr {
                                    expr: Self::hier_ident_expr(
                                        vec![HierPathSegment {
                                            name: crate::ast::Identifier {
                                                name: clk.clone(),
                                                span: a.span,
                                            },
                                            selects: Vec::new(),
                                        }],
                                        a.span,
                                    ),
                                    edge: Some(edge),
                                    iff: None,
                                    span: a.span,
                                };
                                Some((vec![event], resolved_expr.clone()))
                            }
                            None => None,
                        },
                    };
                    if let Some((events, body)) = clocked {
                        let span_key = a.span.start;
                        let existing = self.sva_sites.iter().position(|s| {
                            s.span_key == span_key && s.dedup_scope == self.current_scope
                        });
                        let idx = match existing {
                            Some(i) => i,
                            None => {
                                self.register_sva_site(a, events, body, site_label.as_deref());
                                self.sva_sites.len() - 1
                            }
                        };
                        // §16.14.6.2: each execution of a procedural
                        // concurrent assertion queues one instance, with the
                        // automatic variables it reads captured now.
                        if self.sva_sites[idx].procedural {
                            let mut env = self.sva_sites[idx].lv_init.clone();
                            for (k, name) in &self.sva_sites[idx].captured {
                                if let Some(v) =
                                    self.local_stack.last().and_then(|m| m.get(name.as_str()))
                                {
                                    env[*k] = v.clone();
                                }
                            }
                            let pid = if self.settling {
                                usize::MAX
                            } else {
                                self.current_pid
                            };
                            self.sva_proc_pending.push((idx, pid, env));
                        }
                        return;
                    }
                    // The unclocked property's outcomes land on the site's
                    // own entry (file, line, kind and label), not on a
                    // locationless one keyed by a synthetic name.
                    let src = self.current_src_file();
                    let stat_key = Self::assert_stat_key(a.span.start, src);
                    self.note_assert_label(stat_key, site_label.as_deref());
                    let kind: u8 = match a.kind {
                        AssertionKind::Assert => 0,
                        AssertionKind::Assume => 1,
                        AssertionKind::Cover => 2,
                    };
                    let this_handle = self.this_stack.last().and_then(|t| *t);
                    self.pending_observed.push(ObservedProbe {
                        stat_key,
                        loc: (src, a.span.start),
                        kind,
                        predicate: resolved_expr,
                        this_handle,
                    });
                    return;
                }
                // §20.12: an assertion that is off is not evaluated.
                let ctl = if self.assert_ctl_on {
                    let st = self.assert_ctl_state(&self.imm_assert_ident(a));
                    if !st.enabled() {
                        return;
                    }
                    st
                } else {
                    Default::default()
                };
                let true_branch = self.eval_expr(&a.expr).is_true();
                if let Some(kind) = a.deferred {
                    self.queue_deferred_assert(a, kind, true_branch, site_label.as_deref());
                    return;
                }
                // Track per-site tallies for end-of-sim summary + JSON DB.
                let kind_tag: u8 = match a.kind {
                    AssertionKind::Assert => 0,
                    AssertionKind::Assume => 1,
                    AssertionKind::Cover => 2,
                };
                let src = self.current_src_file();
                let stat_key = Self::assert_stat_key(a.span.start, src);
                self.note_assert_label(stat_key, site_label.as_deref());
                let entry = self
                    .assertion_stats
                    .entry(stat_key)
                    .or_insert(AssertionStat {
                        kind: kind_tag,
                        loc: Some((src, a.span.start)),
                        pass_count: 0,
                        fail_count: 0,
                    });
                if true_branch {
                    entry.pass_count += 1;
                } else {
                    entry.fail_count += 1;
                }
                if !true_branch {
                    if !ctl.fail_action() {
                    } else if let Some(ea) = &a.else_action {
                        self.exec_statement(ea);
                    } else if kind_tag != 2 {
                        // §16.3: with no `else` clause a failure calls
                        // `$error` (a cover has no failure action).
                        self.emit_severity_text("Error", ASSERT_DEFAULT_ERROR);
                    }
                } else if let Some(ac) = &a.action {
                    if ctl.pass_action(false) {
                        self.exec_statement(ac);
                    }
                }
            }
            StatementKind::ProceduralContinuous(pc) => match pc {
                ProceduralContinuous::Assign { lvalue, rvalue }
                | ProceduralContinuous::Force { lvalue, rvalue } => {
                    // LRM §10.6.2 `force` / §10.6.1 procedural continuous
                    // `assign`: the override supersedes ordinary procedural
                    // assignments and continuous drivers until the matching
                    // release/deassign. We register the target in
                    // `forced_signals` (compact-table id) or `forced_names`
                    // (runtime-map storage); every central write path
                    // (`write_sig!`, the comb-settle direct-copy items, the
                    // name-keyed fallbacks) drops writes to registered
                    // targets.
                    let v = self.eval_expr(rvalue);
                    let partial_sel = if matches!(pc, ProceduralContinuous::Force { .. })
                        && matches!(
                            lvalue.kind,
                            ExprKind::Index { .. } | ExprKind::RangeSelect { .. }
                        ) {
                        self.force_select_range(lvalue)
                    } else {
                        None
                    };
                    if let Some(sel) = partial_sel {
                        self.force_partial(sel, rvalue);
                        return;
                    }
                    let target = self.force_target(lvalue);
                    let vpi_target = target.as_ref().map(|(_, id)| *id);
                    // A second force/assign on an already-overridden target
                    // REPLACES the previous override (§10.6.1/§10.6.2), so
                    // lift the old one first — the guarded write paths would
                    // otherwise drop this very write — then re-arm below.
                    if let Some((ref name, id)) = target {
                        if let Some(id) = id {
                            self.unforce_cell(id);
                            self.partial_forces.remove(&id);
                        }
                        self.forced_names.remove(name);
                    }
                    self.assign_value(lvalue, &v);
                    // A net's gate must see the override (and, for a shape
                    // degraded to a plain write, overwrite it on the next
                    // input change exactly as before).
                    self.ctl_note_deposit(lvalue);
                    match target {
                        Some((ref name, Some(id))) => {
                            self.force_cell(id, v);
                            self.arm_force_expr(name.clone(), lvalue, rvalue);
                        }
                        Some((name, None)) => {
                            self.forced_names.insert(name.clone());
                            self.arm_force_expr(name, lvalue, rvalue);
                        }
                        // Unsupported lvalue shape (part-select, concat, …):
                        // degrade to a plain write, as before.
                        None => {}
                    }
                    // §38.36.1 cbForce / cbAssign, after the override took hold.
                    if let Some(sig) = vpi_target {
                        if self.vpi_cb_mask != 0 {
                            let reason = match pc {
                                ProceduralContinuous::Force { .. } => vpi_cb::CB_FORCE,
                                _ => vpi_cb::CB_ASSIGN,
                            };
                            self.vpi_notify_override(reason, sig);
                        }
                    }
                }
                ProceduralContinuous::Release(lvalue) | ProceduralContinuous::Deassign(lvalue) => {
                    // LRM §10.6.2 `release`: a VARIABLE keeps the forced
                    // value until the next procedural assignment — so we
                    // only lift the override and leave the stored value
                    // untouched. A NET returns to its continuous drivers —
                    // mark the source signals of every comb entry driving it
                    // dirty so the next settle re-evaluates the drivers.
                    // §10.6.1 `deassign` likewise retains the last value.
                    if matches!(pc, ProceduralContinuous::Release(_))
                        && matches!(
                            lvalue.kind,
                            ExprKind::Index { .. } | ExprKind::RangeSelect { .. }
                        )
                    {
                        if let Some(sel) = self.force_select_range(lvalue) {
                            self.release_partial(sel);
                            return;
                        }
                    }
                    let target = self.force_target(lvalue);
                    self.release_override(target.as_ref().map(|(n, id)| (n.as_str(), *id)));
                    // §38.36.1 cbRelease / cbDeassign, reported once the
                    // released net has been re-driven.
                    if let Some((_, sig)) = target {
                        if self.vpi_cb_mask != 0 {
                            let reason = match pc {
                                ProceduralContinuous::Release(_) => vpi_cb::CB_RELEASE,
                                _ => vpi_cb::CB_DEASSIGN,
                            };
                            self.vpi_queue_release(reason, sig);
                        }
                    }
                }
            },
            StatementKind::RandCase { items } => {
                if let Some(i) = self.randcase_pick(items) {
                    let stmt = items[i].1.clone();
                    self.exec_statement(&stmt);
                }
            }
            StatementKind::Typedef(td) => {
                // §6.18: a block-local typedef registers its width and
                // underlying type so later declarations in this process
                // resolve it — it was parsed and DISCARDED before, so a
                // local `typedef struct packed {...} t; t v;` lost the
                // member layout. Enum members become named constants.
                let w = super::elaborate::resolve_type_width(
                    &td.data_type,
                    Some(&self.module.parameters),
                    Some(&self.module.typedefs),
                );
                self.type_tables_gen += 1;
                self.module.typedefs.insert(td.name.name.clone(), w);
                self.module
                    .typedef_types
                    .insert(td.name.name.clone(), td.data_type.clone());
                self.typedef_layouts_by_width = None;
                if let Some(set) = self.plain_class_types.borrow_mut().as_mut() {
                    set.remove(&td.name.name);
                }
                *self.coll_member_names.borrow_mut() = None;
                if !td.dimensions.is_empty() {
                    self.module
                        .typedef_unpacked_dims
                        .insert(td.name.name.clone(), td.dimensions.clone());
                }
                if let crate::ast::types::DataType::Enum(et) = &td.data_type {
                    let base_w = et
                        .base_type
                        .as_ref()
                        .map(|bt| {
                            super::elaborate::resolve_type_width(
                                bt,
                                Some(&self.module.parameters),
                                Some(&self.module.typedefs),
                            )
                        })
                        .unwrap_or(32);
                    let mut next: u64 = 0;
                    let mut members: Vec<(String, u64)> = Vec::new();
                    for m in &et.members {
                        let v = match &m.init {
                            Some(e) => self.eval_expr(e).to_u64().unwrap_or(next),
                            None => next,
                        };
                        next = v.wrapping_add(1);
                        self.module
                            .parameters
                            .insert(m.name.name.clone(), Value::from_u64(v, base_w));
                        members.push((m.name.name.clone(), v));
                    }
                    self.module
                        .enum_members
                        .insert(td.name.name.clone(), members);
                    // The member values were evaluated after the first bump.
                    self.type_tables_gen += 1;
                }
            }
            StatementKind::VarDecl {
                data_type,
                lifetime,
                declarators,
            } => self.exec_stmt_var_decl(stmt, data_type, lifetime, declarators),
            StatementKind::EventTrigger {
                name,
                nonblocking,
                target,
                ..
            } => {
                // §15.5: `-> h.evs[i]` — an element of a class event array.
                if let Some(key) = target
                    .as_deref()
                    .and_then(|t| self.expr_instance_event_elem(t))
                {
                    if *nonblocking {
                        self.pending_nba_instance_triggers.push(key);
                    } else {
                        self.fire_instance_event(key);
                    }
                    return;
                }
                if *nonblocking {
                    // §15.5.2: defer to the NBA region of the current slot.
                    // Resolve a class-field instance key NOW (while `this` is
                    // live); the NBA drain cannot reconstruct it later.
                    if let Some(key) = self.resolve_this_event_field(&name.name) {
                        self.pending_nba_instance_triggers.push(key);
                    } else if let Some(key) =
                        self.static_class_event_trigger_key(&name.name, target.as_deref())
                    {
                        self.pending_nba_instance_triggers.push(key);
                    } else if let Some(key) = target
                        .as_ref()
                        .and_then(|t| self.expr_instance_event_field_general(t))
                    {
                        self.pending_nba_instance_triggers.push(key);
                    } else {
                        let key = target
                            .as_ref()
                            .and_then(|t| self.event_ref_key(t))
                            .unwrap_or_else(|| self.lexical_event_name(&name.name));
                        self.pending_nba_triggers.push(key);
                    }
                    return;
                }
                if let Some(key) = self.resolve_this_event_field(&name.name) {
                    self.fire_instance_event(key);
                    return;
                }
                if let Some(key) =
                    self.static_class_event_trigger_key(&name.name, target.as_deref())
                {
                    self.fire_instance_event(key);
                    return;
                }
                // `-> h.ce` / `-> this.ce`: a class-property event reached
                // through a handle (the `->` parser flattens it to a dotted
                // name).
                if name.name.contains('.') {
                    if let Some(key) = self.resolve_handle_event_field(&name.name) {
                        self.fire_instance_event(key);
                        return;
                    }
                }
                // Receivers the flattened string cannot express — runtime
                // selects (`-> m_events[obj].all_dropped`, the UVM objection
                // drop path, #109) and chained handles — resolve from the
                // carried target EXPRESSION.
                if let Some(key) = target
                    .as_ref()
                    .and_then(|t| self.expr_instance_event_field_general(t))
                {
                    self.fire_instance_event(key);
                    return;
                }
                // An event bound to a `ref` formal fires the CALLER's event.
                // §15.5: the rewritten target retains an event-array select
                // and its instance scope when the flattened name does not.
                if let Some(key) = target.as_ref().and_then(|t| self.event_ref_key(t)) {
                    self.fire_named_event(&key);
                    return;
                }
                if !self.module.events.contains(name.name.as_str()) {
                    if let Some(k) = self.ref_bound_event_key(&name.name) {
                        self.fire_named_event(&k);
                        return;
                    }
                }
                // §23.4 lexical home: a bare name fired from a cross-scope
                // call targets the callee's own event, not the caller's
                // same-named one.
                let lex = self.lexical_event_name(&name.name);
                self.fire_named_event(&lex);
            }
            StatementKind::Coverpoint { .. } | StatementKind::Cross { .. } => {}
            StatementKind::WaitOrder { .. } => {
                // The synchronous executor cannot suspend; wait_order is
                // handled on the process path (run_process_stmts). Reaching
                // here means the construct sat in a context that never
                // inlines — warn rather than silently drop.
                eprintln!(
                    "[xezim][warning] wait_order reached the non-suspending execution path and was skipped (IEEE 1800-2017 §15.5.3)"
                );
            }
        }
    }

    /// Register a local array of collections (see the caller): `dims[..k]`
    /// fixed, `dims[k]` the element's collection kind. Returns false (nothing
    /// registered) when the outer shape is not a fixed one.
    #[cold]
    #[inline(never)]
    pub(super) fn declare_local_coll_array(
        &mut self,
        d: &VarDeclarator,
        data_type: &DataType,
        dims: &[crate::ast::types::UnpackedDimension],
        k: usize,
        w: u32,
        lifetime: Option<crate::ast::types::Lifetime>,
    ) -> bool {
        use crate::ast::types::UnpackedDimension as UD;
        if matches!(dims.first(), Some(UD::Unsized(_)))
            && dims.len() > 1
            && dims[1..]
                .iter()
                .all(|d| matches!(d, UD::Associative { .. }))
        {
            let name = if matches!(lifetime, Some(crate::ast::types::Lifetime::Static)) {
                d.name.name.clone()
            } else {
                self.declare_local_dyn(&d.name.name)
            };
            self.register_coll_name(&name, StructCollKind::Dynamic, w, false);
            self.module
                .var_decl_types
                .insert(name.clone(), data_type.clone());
            self.dynamic_assoc_dims
                .insert(name.clone(), dims[1..].to_vec());
            self.clear_dynamic_assoc_rows(&name, 0);
            self.set_queue_size(&name, 0);
            if let Some(init) = &d.init
                && let Some((n, src)) = Self::sized_new_parts(init)
            {
                self.fill_sized_new(&name, n, src);
            }
            return true;
        }
        let shape: Option<Vec<(i64, i64)>> = dims[..k]
            .iter()
            .map(|dim| match dim {
                UD::Range { left, right, .. } => {
                    let l = super::elaborate::const_eval_i64_with_params(left, None)?;
                    let r = super::elaborate::const_eval_i64_with_params(right, None)?;
                    Some((l.min(r), l.max(r)))
                }
                UD::Expression { expr, .. } => {
                    let n = super::elaborate::const_eval_i64_with_params(expr, None)?;
                    (n > 0).then_some((0, n - 1))
                }
                _ => None,
            })
            .collect();
        let Some(shape) = shape.filter(|sh| {
            sh.iter()
                .map(|&(lo, hi)| (hi - lo + 1).max(0))
                .product::<i64>()
                <= 4096
        }) else {
            return false;
        };
        let bare = d.name.name.clone();
        let name = if matches!(lifetime, Some(crate::ast::types::Lifetime::Static)) {
            bare
        } else {
            self.declare_local_dyn(&bare)
        };
        self.adopt_local_shape(&name);
        match shape.len() {
            1 => {
                self.module
                    .arrays
                    .insert(name.clone(), (shape[0].0, shape[0].1, w));
            }
            2 => {
                self.module
                    .arrays_2d
                    .insert(name.clone(), (shape[0], shape[1], w));
            }
            _ => {
                self.module
                    .arrays_nd
                    .insert(name.clone(), (shape.clone(), w));
            }
        }
        let assoc_key = match &dims[k] {
            UD::Associative { data_type: kdt, .. } => Some(kdt.as_ref().is_some_and(|dt| {
                matches!(
                    super::elaborate::resolve_typedef_chain(dt, &self.module.typedef_types),
                    DataType::Simple {
                        kind: crate::ast::types::SimpleType::String,
                        ..
                    }
                )
            })),
            _ => None,
        };
        let key_info = match &dims[k] {
            UD::Associative { data_type, .. } => super::elaborate::assoc_key_type_info(
                data_type.as_deref(),
                &self.module.parameters,
                &self.module.typedefs,
                &self.module.typedef_types,
            ),
            _ => (None, None),
        };
        let mut elems = Vec::new();
        for sfx in Self::nd_elem_suffixes(&shape) {
            let en = format!("{}{}", name, sfx);
            elems.push(en.clone());
            // A fresh declaration: drop what a prior frame's same-named
            // local left behind.
            for key in self.signals.keys_with_elem_prefix(&format!("{}[", en)) {
                self.signals.remove(&key);
            }
            match assoc_key {
                Some(is_str) => {
                    if let Some(index) = key_info.0 {
                        self.module.assoc_index_widths.insert(en.clone(), index);
                    }
                    if let Some(key_type) = &key_info.1 {
                        self.module
                            .assoc_key_type_names
                            .insert(en.clone(), key_type.clone());
                    }
                    self.module.assoc_elem_widths.insert(en.clone(), w);
                    self.module.associative_arrays.insert(en, is_str);
                }
                None => {
                    self.module.dynamic_arrays.insert(en.clone());
                    self.module.arrays.insert(en.clone(), (0, 63, w));
                    self.set_queue_size(&en, 0);
                }
            }
        }
        if name.starts_with('@') {
            self.local_coll_elems.insert(name.clone(), elems);
        }
        self.module.var_decl_types.insert(name, data_type.clone());
        true
    }

    /// Whether a local's unpacked dimension needs the enclosing scope to be
    /// sized: a size or bound that does not fold without parameters, or a
    /// `[NAME]` that parses as an associative key type but may name a
    /// constant.
    pub(super) fn local_dim_needs_scope(dim: &crate::ast::types::UnpackedDimension) -> bool {
        use super::elaborate::const_eval_i64_with_params as ce;
        use crate::ast::types::UnpackedDimension as UD;
        match dim {
            UD::Expression { expr, .. } => ce(expr, None).is_none(),
            UD::Range { left, right, .. } => ce(left, None).is_none() || ce(right, None).is_none(),
            UD::Associative {
                data_type: Some(dt),
                ..
            } => matches!(
                dt.as_ref(),
                crate::ast::types::DataType::TypeReference { dimensions, .. } if dimensions.is_empty()
            ),
            _ => false,
        }
    }

    /// §7.4.2 / §8.23: fold a local's scope-sized dimensions to literals,
    /// evaluated in the executing scope — `[N]` whose `N` is a constant and
    /// not a type, `[cls::N]`, and size/bound expressions over parameters or
    /// class constants. A dimension that does not resolve is left as is.
    #[cold]
    #[inline(never)]
    pub(super) fn scope_sized_local_dims(
        &mut self,
        dims: &[crate::ast::types::UnpackedDimension],
    ) -> Option<Vec<crate::ast::types::UnpackedDimension>> {
        use crate::ast::expr::{ExprKind, Expression, NumberBase, NumberLiteral};
        use crate::ast::types::UnpackedDimension as UD;
        fn lit(n: i64, span: crate::ast::Span) -> Box<Expression> {
            Box::new(Expression::new(
                ExprKind::Number(NumberLiteral::Integer {
                    size: None,
                    signed: true,
                    base: NumberBase::Decimal,
                    value: n.to_string(),
                    cached_val: Default::default(),
                }),
                span,
            ))
        }
        let normalized = super::elaborate::normalize_unpacked_dims(
            dims,
            &self.module.parameters,
            &self.module.typedef_types,
        );
        let mut changed = false;
        let mut out = Vec::with_capacity(normalized.len());
        for (orig, dim) in dims.iter().zip(normalized) {
            let folded = match &dim {
                UD::Expression { expr, span }
                    if super::elaborate::const_eval_i64_with_params(expr, None).is_none() =>
                {
                    self.eval_expr(expr)
                        .to_i64()
                        .filter(|&n| n > 0)
                        .map(|n| UD::Expression {
                            expr: lit(n, expr.span),
                            span: *span,
                        })
                }
                UD::Range { left, right, span } => {
                    let l = self.eval_expr(left).to_i64();
                    let r = self.eval_expr(right).to_i64();
                    match (l, r) {
                        (Some(l), Some(r)) => Some(UD::Range {
                            left: lit(l, left.span),
                            right: lit(r, right.span),
                            span: *span,
                        }),
                        _ => None,
                    }
                }
                // `[N]` still associative: a constant of the executing
                // scope (not a type) sizes it.
                UD::Associative {
                    data_type: Some(dt),
                    span,
                } => match dt.as_ref() {
                    crate::ast::types::DataType::TypeReference { name, .. }
                        if name.scopes.is_empty()
                            && !self.module.typedef_types.contains_key(&name.name.name)
                            && !self.module.classes.contains_key(&name.name.name)
                            && self.resolve_type_param_binding(&name.name.name).is_none() =>
                    {
                        let id = Expression::new(
                            ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                                root: None,
                                path: vec![crate::ast::expr::HierPathSegment {
                                    name: name.name.clone(),
                                    selects: Vec::new(),
                                }],
                                span: name.span,
                                cached_signal_id: std::cell::Cell::new(None),
                                cached_resolved_name: std::cell::OnceCell::new(),
                            }),
                            name.span,
                        );
                        let v = self.eval_expr(&id);
                        (!v.is_real && v.width > 0)
                            .then(|| v.to_i64())
                            .flatten()
                            .filter(|&n| n > 0)
                            .map(|n| UD::Expression {
                                expr: lit(n, name.span),
                                span: *span,
                            })
                    }
                    _ => None,
                },
                _ => None,
            };
            // `normalize_unpacked_dims` turning `[N]` into a size counts too.
            changed |= folded.is_some()
                || matches!(
                    (orig, &dim),
                    (UD::Associative { .. }, UD::Expression { .. })
                );
            out.push(folded.unwrap_or(dim));
        }
        changed.then_some(out)
    }

    /// Member-wise declaration initializer of an unpacked-struct local
    /// (`row_t r = '{...};`, `op_s x = accesses[i];`, `pair_t p = o.orig;`).
    pub(super) fn init_unpacked_struct_local(
        &mut self,
        d: &VarDeclarator,
        data_type: &DataType,
        init_expr: &Expression,
    ) {
        let init_expr = init_expr.clone();
        let lv = crate::ast::expr::Expression::new(
            crate::ast::expr::ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                root: None,
                path: vec![crate::ast::expr::HierPathSegment {
                    name: crate::ast::Identifier {
                        name: d.name.name.clone(),
                        span: d.name.span,
                    },
                    selects: Vec::new(),
                }],
                span: d.name.span,
                cached_signal_id: std::cell::Cell::new(None),
                cached_resolved_name: std::cell::OnceCell::new(),
            }),
            d.name.span,
        );
        if self
            .try_decompose_struct_class_prop_read_assign(&lv, &init_expr)
            .is_none()
        {
            // §7.2: decl-init from any OTHER struct
            // source — another local, a queue/array
            // element (`op_s x = accesses[i];`, UVM's
            // reg-map bus access loop) — copies member
            // -wise exactly like the assignment arm.
            // The packed `v` stored above has no
            // leaves, so without this every member of
            // the fresh local stayed x.
            if let crate::ast::types::DataType::Struct(su) = self.resolve_dt(data_type) {
                let src = self.flat_member_name(&init_expr);
                match src {
                    Some(src) if self.struct_storage_exists(&src, &su) => {
                        self.copy_unpacked_struct(&d.name.name, &src, &su);
                    }
                    // An element of a class-property
                    // collection (UVM's `row_info row =
                    // m_rows[i];`) keeps its leaves under
                    // the instance, not the flat name.
                    _ if Self::is_struct_elem_ref(&init_expr) => {
                        self.assign_struct_memberwise(&lv, &init_expr, &su, 0);
                    }
                    // §5.10/§10.9.2: any other source (an assignment
                    // pattern, a call, a conditional) is exactly the
                    // assignment `name = init`, which gives the pattern
                    // the struct's member layout as its context. Evaluated
                    // self-determined, `'{a:5, b:6}` packed each item at
                    // 32 bits and the spread mis-placed every member
                    // (`a` read 5 << 24).
                    _ => {
                        let assign = crate::ast::stmt::Statement::new(
                            crate::ast::stmt::StatementKind::BlockingAssign {
                                lvalue: lv,
                                rvalue: init_expr,
                            },
                            d.name.span,
                        );
                        self.exec_statement(&assign);
                    }
                }
            }
        }
    }
}
