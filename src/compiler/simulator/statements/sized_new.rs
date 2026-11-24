//! Unpacked concatenation operands and sized `new` of dynamic arrays and their rows.

use super::*;

impl Simulator {
    /// §10.10/§11.4.12: the elements an unpacked-array concatenation's
    /// operands contribute, in order — a queue / array operand (bare, an
    /// object member, a struct member reached through any selects, an
    /// associative element that is itself a collection) or a slice contributes
    /// its elements. Expand a fixed row only for a non-array element type.
    /// Under §10.10.3 nested braces form one self-determined vector or string.
    #[inline(never)]
    pub(in crate::compiler::simulator) fn unpacked_concat_operand_values<'e>(
        &mut self,
        items: Vec<&'e Expression>,
        expand_rows: bool,
        mut first_row: Option<(&'e Expression, super::unpacked_slices::UnpackedLayout<'e>)>,
    ) -> Vec<Value> {
        let mut all_vals: Vec<Value> = Vec::new();
        for (position, expr) in items.into_iter().enumerate() {
            // §10.10, §13.4.1: a call returning an unpacked array contributes
            // the elements it returns, in order (the call runs once).
            if matches!(expr.kind, ExprKind::Call { .. })
                && let Some(src) = self.ret_collection_operand(expr)
            {
                if let Some(src) = src.filter(|s| !self.is_associative_array(s)) {
                    for j in 0..self.get_queue_size(&src) {
                        if let Some(v) = self.get_signal_value_by_name(&format!("{src}[{j}]")) {
                            all_vals.push(v);
                        }
                    }
                }
                continue;
            }
            if expand_rows
                && let Some(row) = if first_row
                    .as_ref()
                    .is_some_and(|(e, _)| std::ptr::eq(*e, expr))
                {
                    // A preceding operand can change an object's receiver.
                    first_row
                        .take()
                        .filter(|(_, row)| position == 0 || !row.object_member())
                        .map(|(_, row)| row)
                        .or_else(|| self.fixed_row_concat_shape(expr))
                } else {
                    self.fixed_row_concat_shape(expr)
                }
            {
                self.append_fixed_row_concat_values(row, &mut all_vals);
                continue;
            }
            // A queue/array operand — bare (`q`), an instance-
            // scoped member (`obj.q`, `arr[i].q`), or a nested
            // assoc-of-queue element (`assoc[key]`) —
            // contributes each of its elements in order.
            let qn = self.array_operand_name(expr).or_else(|| {
                self.nested_index_name(expr)
                    .filter(|cn| self.module.dynamic_arrays.contains(cn))
            });
            if let Some(ename) = qn {
                let lo = self.module.arrays.get(&ename).map_or(0, |a| a.0);
                let mut idx: Vec<i64> = (lo..lo + self.get_queue_size(&ename) as i64).collect();
                if self.module.descending_arrays.contains(&ename) {
                    idx.reverse();
                }
                for j in idx {
                    if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", ename, j)) {
                        all_vals.push(v);
                    }
                }
                continue;
            }
            if let ExprKind::RangeSelect { expr: base, .. } = &expr.kind {
                // A computed object receiver must be resolved only once.
                let computed = Self::recv_chain_call(base).is_some();
                if !computed && self.append_fixed_slice_concat_values(expr, &mut all_vals) {
                    continue;
                }
                if let Some((values, _)) = self.collection_slice_values(expr) {
                    all_vals.extend(values);
                    continue;
                }
            }
            // §10.10, §11.4.11, §6.24.1: a conditional choosing between two
            // array operands contributes the chosen operand's elements, and a
            // cast of an array operand to an unpacked array type
            // (`row_t'(src[1])`) contributes its operand's. An ambiguous
            // condition keeps the value path below.
            if let Some(inner) = self.unpacked_concat_inner_operand(expr) {
                let vals = self.unpacked_concat_operand_values(vec![inner], expand_rows, None);
                all_vals.extend(vals);
                continue;
            }
            if self.append_member_array_concat_values(expr, &mut all_vals) {
                continue;
            }
            all_vals.push(self.eval_expr(expr));
        }
        all_vals
    }

    /// The array operand that a conditional or an unpacked-array cast item
    /// of an unpacked concatenation stands for; `None` for any other item.
    /// Only the condition is evaluated.
    fn unpacked_concat_inner_operand<'e>(&mut self, e: &'e Expression) -> Option<&'e Expression> {
        match &e.kind {
            ExprKind::Paren(inner) => self.unpacked_concat_inner_operand(inner),
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } if self.concat_array_operand_like(then_expr)
                && self.concat_array_operand_like(else_expr) =>
            {
                match self.eval_expr(condition).is_nonzero()? {
                    true => Some(Self::unparen(then_expr)),
                    false => Some(Self::unparen(else_expr)),
                }
            }
            ExprKind::SystemCall { name, args }
                if name == "$__xz_named_cast"
                    && args.len() == 2
                    && matches!(&args[0].kind, ExprKind::Ident(h)
                    if h.root.is_none() && h.path.len() == 1 && h.path[0].selects.is_empty()
                        && {
                            let class = self
                                .class_context_stack
                                .last()
                                .and_then(|c| c.as_deref())
                                .unwrap_or("");
                            Self::typedef_dims_via_tables(&self.module, class, &h.path[0].name.name)
                                .is_some()
                        })
                    && self.concat_array_operand_like(&args[1]) =>
            {
                Some(Self::unparen(&args[1]))
            }
            _ => None,
        }
    }

    /// Whether `e` names an array operand of an unpacked concatenation (a
    /// collection, a fixed array or one of its rows), without evaluating it.
    fn concat_array_operand_like(&mut self, e: &Expression) -> bool {
        let e = Self::unparen(e);
        self.fixed_row_concat_shape(e).is_some()
            || self.array_operand_name(e).is_some()
            || self.unpacked_concat_inner_operand_shape(e)
    }

    /// A nested conditional or cast of array operands, checked without
    /// evaluating its condition.
    fn unpacked_concat_inner_operand_shape(&mut self, e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Conditional {
                then_expr,
                else_expr,
                ..
            } => {
                self.concat_array_operand_like(then_expr)
                    && self.concat_array_operand_like(else_expr)
            }
            _ => false,
        }
    }

    /// The size and optional source of a §7.5.1 dynamic-array constructor:
    /// `new[n]` parses as `Call{Ident(new), [n]}` and `new[n](src)` as
    /// `Call{Call{Ident(new), [n]}, [src]}`. `None` for any other expression.
    pub(in crate::compiler::simulator) fn sized_new_parts(
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
    pub(in crate::compiler::simulator) fn class_dyn_row_width(&self, name: &str) -> Option<u32> {
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
    pub(in crate::compiler::simulator) fn clear_dyn_row(&mut self, row: &str) {
        for key in self.signals.keys_with_elem_prefix(&format!("{}[", row)) {
            self.signals.remove(&key);
        }
    }

    /// Row key for `d[i] = new[n]` when the row's element type is not a class
    /// handle (§7.5.1). Class properties use [`class_array_of_coll_key`]; module
    /// or automatic locals use the same storage keys as reads.
    pub(in crate::compiler::simulator) fn sized_new_row_lvalue(
        &mut self,
        lvalue: &Expression,
    ) -> Option<String> {
        let ExprKind::Index { expr: base, index } = &lvalue.kind else {
            return None;
        };
        if self.lvalue_container_kind(lvalue).is_some() {
            return None;
        }
        let key = match &base.kind {
            ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                self.resolve_locator_storage(&self.resolve_hier_name(h))
            }
            _ => self.expr_assoc_name(base)?,
        };
        if !self.module.dynamic_arrays.contains(&key)
            && self.class_dyn_row_width(&key).is_none()
            && self
                .coll_shape_at(&key)
                .is_none_or(|(shape, _)| shape.dims.len() < 2)
        {
            return None;
        }
        let dt = self
            .class_member_coll_type(&key)
            .or_else(|| self.module.var_decl_types.get(&key))?;
        if matches!(
            super::elaborate::resolve_typedef_chain(dt, &self.module.typedef_types),
            DataType::TypeReference { .. }
        ) {
            return None;
        }
        // Associative outers (`int aa[string][]`) are sized by the caller's
        // collection `new[n]` store; the early row path only runs `fill_sized_new`
        // after `ensure_row`, which does not allocate a missing assoc entry.
        if self.is_associative_array(&key) {
            return None;
        }
        let i = self.eval_expr(index).to_i64()?;
        Some(format!("{}[{}]", key, i))
    }

    /// The declared associative tail below a dynamic outer array, scoped to
    /// a local, object property, or unpacked-struct member (§7.4.5).
    #[cold]
    pub(in crate::compiler::simulator) fn dynamic_assoc_decl(
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
    pub(in crate::compiler::simulator) fn register_dynamic_assoc_tail(
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
    }

    /// Discard rows outside the retained prefix, including every nested key
    /// and its runtime metadata. Reallocation must not resurrect old entries.
    #[cold]
    pub(in crate::compiler::simulator) fn clear_dynamic_assoc_rows(
        &mut self,
        root: &str,
        keep: u64,
    ) {
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
        self.module.assoc_elem_widths.retain(|key, _| !discard(key));
        self.module.var_decl_types.retain(|key, _| !discard(key));
        self.dynamic_assoc_dims.retain(|key, _| !discard(key));
    }

    /// §7.5.1: size the dynamic array stored at `name` to `new[n_expr]`,
    /// copying min(n, src.size()) elements from `src_expr` (`new[n](src)`)
    /// and giving the rest the element type's default.
    #[inline(never)]
    pub(in crate::compiler::simulator) fn fill_sized_new(
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
            // A ROW of an array of dynamic arrays (`d[i]`, `h.rows[i]`) is a
            // collection operand too; taken for a scalar, the copy kept
            // nothing (`d[1] = new[2](d[1])` zeroed the row).
            ExprKind::Index { .. } if Self::cond_expr_is_effect_free(e, false) => self
                .class_array_of_coll_key(e)
                .or_else(|| self.flat_member_name(e))
                .filter(|n| self.module.dynamic_arrays.contains(n.as_str())),
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
        // §7.5.1, §10.10: `new[n]({src[1], 5})` initializes from the elements
        // of an unpacked concatenation, read before the target is resized
        // (`d = new[3]({d, 8})`). Read as one packed value, it was broadcast.
        let concat_vals = match src_expr.map(|e| &e.kind) {
            Some(ExprKind::Concatenation(parts))
                if !self.module.arrays_2d.contains_key(name)
                    && !self.module.arrays_nd.contains_key(name) =>
            {
                let mut items = Vec::with_capacity(parts.len());
                let mut stack: Vec<&Expression> = parts.iter().rev().collect();
                while let Some(e) = stack.pop() {
                    match &e.kind {
                        ExprKind::Concatenation(inner) => stack.extend(inner.iter().rev()),
                        _ => items.push(e),
                    }
                }
                Some(self.unpacked_concat_operand_values(items, true, None))
            }
            _ => None,
        };
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
            // §7.5.1: the element type's default (x when 4-state, real 0.0
            // for real, §6.8).
            let four_state = self.coll_elem_starts_x(name);
            let real = self.coll_elem_is_real(name);
            for i in keep..n {
                for s in &suffixes {
                    let v = if real {
                        Value::from_f64(0.0)
                    } else if four_state {
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
            // §6.8, Table 6-7: a real/shortreal element starts at real 0.0.
            let four_state = self.coll_elem_starts_x(name);
            let fill = if self.coll_elem_is_real(name) {
                Value::from_f64(0.0)
            } else if four_state {
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
        if let Some(vals) = concat_vals {
            for (i, v) in vals.into_iter().take(n as usize).enumerate() {
                let v = self.fit_queue_push_elem(name, v);
                self.set_signal_value_by_name(&format!("{}[{}]", name, i), v);
            }
        } else if let Some(se) = src_expr
            && src.is_none()
        {
            match &self.unpacked_pattern(se).kind {
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
}
