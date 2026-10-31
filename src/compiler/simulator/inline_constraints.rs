//! Simulator inline constraints.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    pub(super) fn randomize_check_items(
        &mut self,
        handle: usize,
        inline: &[crate::ast::decl::ConstraintItem],
    ) -> Value {
        let cons = self.active_constraints(handle);
        for con in &cons {
            for item in &con.items {
                // Constructs the checker has no view of (foreach/solve/unique)
                // are skipped rather than reported as violations.
                if Self::constraint_unmodeled(item) {
                    continue;
                }
                if !self.check_constraint_item(handle, item) {
                    return Value::zero(32);
                }
            }
        }
        for item in inline {
            if Self::constraint_unmodeled(item) {
                continue;
            }
            if !self.check_constraint_item(handle, item) {
                return Value::zero(32);
            }
        }
        Value::from_u64(1, 32)
    }

    /// Bare single-segment identifier name of `e`, if that is what it is.
    pub(super) fn plain_ident_name(e: &Expression) -> Option<String> {
        match &e.kind {
            ExprKind::Ident(h) if h.path.len() == 1 && h.root.is_none() => {
                Some(h.path[0].name.name.clone())
            }
            _ => None,
        }
    }

    /// The element names of the unpacked collection `name` in STREAM order
    /// (§11.4.14 / §6.24.3: the leftmost element first — index 0 of a
    /// dynamic array or queue, the left bound of each fixed dimension, rows
    /// before columns) and the element width. None for anything else.
    pub(super) fn stream_elem_order(&self, name: &str) -> Option<(Vec<String>, u32)> {
        let ew_of = |sim: &Self, first: &str, fallback: u32| {
            sim.lookup_signal_width(first).unwrap_or(fallback).max(1)
        };
        if self.module.dynamic_arrays.contains(name) {
            let n = self.get_queue_size(name) as usize;
            let fallback = self.module.arrays.get(name).map(|t| t.2).unwrap_or(8);
            let ew = ew_of(self, &format!("{}[0]", name), fallback);
            return Some(((0..n).map(|i| format!("{}[{}]", name, i)).collect(), ew));
        }
        let (dims, ew): (Vec<(i64, i64)>, u32) =
            if let Some(&(lo, hi, ew)) = self.module.arrays.get(name) {
                let d = match self.module.unpacked_decl_dims.get(name) {
                    Some(d) if d.len() == 1 => d[0],
                    _ if self.module.descending_arrays.contains(name) => (hi, lo),
                    _ => (lo, hi),
                };
                (vec![d], ew)
            } else if let Some(&(r0, r1, ew)) = self.module.arrays_2d.get(name) {
                match self.module.unpacked_decl_dims.get(name) {
                    Some(d) if d.len() == 2 => (d.clone(), ew),
                    _ => (vec![r0, r1], ew),
                }
            } else if let Some((shape, ew)) = self.module.arrays_nd.get(name) {
                match self.module.unpacked_decl_dims.get(name) {
                    Some(d) if d.len() == shape.len() => (d.clone(), *ew),
                    _ => (shape.clone(), *ew),
                }
            } else {
                return None;
            };
        let total: i64 = dims.iter().map(|(l, r)| (l - r).abs() + 1).product();
        if total <= 0 || total > 1 << 20 {
            return None;
        }
        let mut names = vec![name.to_string()];
        for &(l, r) in &dims {
            let idx: Vec<i64> = if l <= r {
                (l..=r).collect()
            } else {
                (r..=l).rev().collect()
            };
            names = names
                .iter()
                .flat_map(|p| idx.iter().map(move |i| format!("{}[{}]", p, i)))
                .collect();
        }
        let ew = names
            .first()
            .map(|f| ew_of(self, f, ew))
            .unwrap_or(ew.max(1));
        Some((names, ew))
    }

    /// §6.24.3: the operand of a cast to an integral type may be an
    /// UNPACKED array or queue (`int'(barr)`, `24'(q)`): its elements are
    /// packed left to right, which is `{>>{operand}}`. The operand read the
    /// array's container signal instead (0 / stale). None when the cast is
    /// not such a cast.
    #[cold]
    #[inline(never)]
    pub(super) fn cast_of_unpacked_operand(
        &mut self,
        expr: &Expression,
        ctx_width: u32,
        name: &String,
        args: &[Expression],
    ) -> Option<Value> {
        if !matches!(
            name.as_str(),
            "$__xz_named_cast" | "$__xz_size_cast" | "$__xz_type_cast"
        ) {
            return None;
        }
        let a = args.get(1)?;
        if !self.cast_operand_is_unpacked(a) {
            return None;
        }
        let mut packed_args = args.to_vec();
        packed_args[1] = Expression::new(
            ExprKind::StreamOp {
                left_to_right: false,
                slice_size: None,
                exprs: vec![a.clone()],
            },
            a.span,
        );
        Some(self.eval_expr_system_call(expr, ctx_width, name, &packed_args))
    }

    /// A cast operand that is a whole unpacked collection (or a slice of a
    /// queue / dynamic array), whose bit stream is its packed elements.
    pub(super) fn cast_operand_is_unpacked(&mut self, e: &Expression) -> bool {
        let e = match &e.kind {
            ExprKind::Paren(i) => i.as_ref(),
            ExprKind::RangeSelect { expr, .. } => match &expr.kind {
                ExprKind::Ident(_) => expr.as_ref(),
                _ => return false,
            },
            _ => e,
        };
        let ExprKind::Ident(h) = &e.kind else {
            return false;
        };
        if h.path.iter().any(|s| !s.selects.is_empty()) || self.ident_shadowed_by_non_collection(h)
        {
            return false;
        }
        let nm = self.resolve_hier_name(h);
        self.module.arrays.contains_key(&*nm)
            || self.module.dynamic_arrays.contains(&*nm)
            || self.module.arrays_2d.contains_key(&*nm)
            || self.module.arrays_nd.contains_key(&*nm)
    }

    /// Storage name of an ASSOCIATIVE-array operand (`a`, `this.a`,
    /// `obj.a`), or None for any other expression.
    pub(super) fn assoc_operand_name(&mut self, expr: &Expression) -> Option<String> {
        if let ExprKind::Ident(h) = &expr.kind
            && !self.ident_shadowed_by_non_collection(h)
        {
            let nm = self.resolve_hier_name(h);
            let nm = self
                .instance_assoc_member(&nm)
                .unwrap_or_else(|| nm.to_string());
            if self.is_associative_array(&nm) {
                return Some(nm);
            }
        }
        self.expr_assoc_name(expr)
            .filter(|nm| self.is_associative_array(nm))
    }

    /// If `expr` names a queue/array (possibly an instance-scoped member),
    /// return its resolved storage name — used for `inside {arr}` membership.
    /// True when `h` cannot name a module- or package-scope collection under
    /// `resolve_hier_name(h)` because it is either:
    ///   - a single-segment name shadowed by a non-collection local/formal/loop
    ///     variable in the current frame or a scalar/handle property of `this`
    ///     (§6.21, §8.10), or
    ///   - a multi-segment path rooted at `this`/`super`, a class-handle
    ///     variable, or a frame local (`c.fa`), whose collection members live
    ///     under `<handle>#member` and whose last-resort `resolve_hier_name`
    ///     fallback (`"fa"`) would otherwise collide with a same-named module
    ///     collection.
    pub(super) fn ident_shadowed_by_non_collection(&self, h: &HierarchicalIdentifier) -> bool {
        if h.path.is_empty() {
            return false;
        }
        let first = h.path[0].name.name.as_str();
        if h.path.len() == 1 {
            if !h.path[0].selects.is_empty() {
                return false;
            }
            if self.dyn_name_lookup(first).is_some() || self.instance_assoc_member(first).is_some()
            {
                return false;
            }
            if self
                .local_stack
                .last()
                .is_some_and(|f| f.contains_key(first))
            {
                return true;
            }
            if let Some(Some(th)) = self.this_stack.last()
                && self
                    .heap
                    .get(*th)
                    .and_then(|o| o.as_ref())
                    .is_some_and(|inst| inst.properties.contains_key(first))
            {
                return true;
            }
            false
        } else {
            matches!(first, "this" | "super")
                || (!self.module.classes.contains_key(first) && self.class_of_var(first).is_some())
                || self
                    .local_stack
                    .last()
                    .is_some_and(|f| f.contains_key(first))
        }
    }

    /// §11.4.13: an unpacked array in an inside set contributes its values,
    /// including associative arrays with sparse or string keys.
    pub(super) fn inside_operand_keys(&mut self, expr: &Expression) -> Option<Vec<String>> {
        if let Some(name) = self.expr_assoc_name(expr)
            && self.is_associative_array(&name)
        {
            return Some(
                self.assoc_top_level_keys(&name)
                    .into_iter()
                    .map(|key| format!("{}[{}]", name, key))
                    .collect(),
            );
        }
        self.array_operand_name(expr).map(|name| {
            (0..self.get_queue_size(&name))
                .map(|i| format!("{}[{}]", name, i))
                .collect()
        })
    }

    pub(super) fn array_operand_name(&mut self, expr: &Expression) -> Option<String> {
        if let ExprKind::Ident(h) = &expr.kind
            && !self.ident_shadowed_by_non_collection(h)
        {
            let mut nm = self.resolve_hier_name(h);
            if let Some(s) = self.instance_assoc_member(&nm) {
                nm = std::borrow::Cow::Owned(s);
            }
            if self.module.arrays.contains_key(&*nm) || self.module.dynamic_arrays.contains(&*nm) {
                return Some(nm.to_string());
            }
        }
        // `obj.q` / `arr[i].q` — an instance-scoped queue/array member.
        if let Some(nm) = self.expr_assoc_name(expr) {
            if self.module.arrays.contains_key(&nm) || self.module.dynamic_arrays.contains(&nm) {
                return Some(nm);
            }
        }
        None
    }

    /// For `assoc[key]` where `assoc` is an associative array whose elements are
    /// themselves collections (queue/array — e.g. riscv-dv's
    /// `instr_category[category][$]`), return the compound storage base
    /// `assoc[<keyval>]`. The nested collection then lives under that name and
    /// reuses the ordinary queue/array machinery (`<base>.size`, `<base>[i]`).
    /// Declared index bounds of `name`, outermost dimension first.
    /// Dimensions a constraint `foreach (a[v1, v2, ...])` iterates for its
    /// `nvars` loop variables: the unpacked shape first, then the packed
    /// dimensions, each normalised to (low, high); a dynamic/queue outer
    /// dimension takes its current size. Fewer dimensions than variables
    /// leaves the trailing variables unbound, as before.
    /// Name of a constraint-foreach operand that `foreach_check_dims` can
    /// iterate: unpacked arrays of any rank, queues/dynamic arrays,
    /// associative arrays, and packed multi-dimensional variables.
    pub(super) fn foreach_check_operand_name(&mut self, expr: &Expression) -> Option<String> {
        if let Some(n) = self.array_operand_name(expr) {
            return Some(n);
        }
        let ExprKind::Ident(h) = &expr.kind else {
            return None;
        };
        let mut nm = self.resolve_hier_name(h).into_owned();
        if let Some(s) = self.instance_assoc_member(&nm) {
            nm = s;
        }
        if self.module.arrays_2d.contains_key(&nm)
            || self.module.arrays_nd.contains_key(&nm)
            || self.is_associative_array(&nm)
            || self
                .module
                .packed_full_dims
                .get(&nm)
                .is_some_and(|d| !d.is_empty())
        {
            return Some(nm);
        }
        None
    }

    pub(super) fn foreach_check_dims(&self, name: &str, nvars: usize) -> Vec<(i64, i64)> {
        let mut dims: Vec<(i64, i64)> = match self.foreach_dims(name) {
            Some(d) => d.into_iter().map(|(l, r)| (l.min(r), l.max(r))).collect(),
            None if self.module.dynamic_arrays.contains(name)
                || self.is_associative_array(name) =>
            {
                vec![(0, self.get_queue_size(name) as i64 - 1)]
            }
            None => Vec::new(),
        };
        if self.module.dynamic_arrays.contains(name) && !dims.is_empty() {
            dims[0] = (0, self.get_queue_size(name) as i64 - 1);
        }
        if dims.len() < nvars {
            if let Some(pdims) = self.module.packed_full_dims.get(name) {
                for &(l, r) in pdims.iter() {
                    if dims.len() >= nvars {
                        break;
                    }
                    dims.push((l.min(r), l.max(r)));
                }
            }
        }
        dims.truncate(nvars);
        dims
    }

    pub(super) fn foreach_dims(&self, name: &str) -> Option<Vec<(i64, i64)>> {
        if let Some((shape, _)) = self.module.arrays_nd.get(name) {
            return Some(shape.clone());
        }
        if let Some(&(d1, d2, _)) = self.module.arrays_2d.get(name) {
            return Some(vec![d1, d2]);
        }
        self.module
            .arrays
            .get(name)
            .map(|&(lo, hi, _)| vec![(lo, hi)])
    }

    /// §20.7: the array query functions also accept a TYPE as their operand
    /// (`$size(int)`, `$left(my_vec_t)`, `$size(logic [7:0])`). A bare type
    /// keyword or a typedef name in an argument list parses as a plain Ident
    /// (see `parse_call_args`), so it has to be recognised here. Returns None
    /// unless the operand really names a type — a signal of the same name is
    /// the ordinary array-query operand and keeps the signal path.
    pub(super) fn type_query_operand(
        &self,
        arg: &Expression,
    ) -> Option<super::elaborate::TypeQueryDims> {
        if let ExprKind::TypeLiteral(dt) = &arg.kind {
            return Some(super::elaborate::type_query_dims_of(
                dt,
                Some(&self.module.parameters),
                Some(&self.module.typedefs),
                Some(&self.module.typedef_types),
            ));
        }
        let ExprKind::Ident(hier) = &arg.kind else {
            return None;
        };
        let seg = hier.path.last()?;
        if !seg.selects.is_empty() {
            return None;
        }
        let resolved = self.resolve_hier_name(hier);
        if self.signal_name_to_id.contains_key(resolved.as_ref())
            || self.module.arrays.contains_key(&*resolved)
            || self.module.arrays_2d.contains_key(&*resolved)
            || self.module.arrays_nd.contains_key(&*resolved)
            || self.module.dynamic_arrays.contains(&*resolved)
            || self.module.associative_arrays.contains_key(&*resolved)
            || self.lookup_signal_width(&resolved).is_some()
            || self.get_signal_value_by_name(&seg.name.name).is_some()
        {
            return None;
        }
        super::elaborate::type_query_dims_by_name(
            &seg.name.name,
            Some(&self.module.parameters),
            Some(&self.module.typedefs),
            Some(&self.module.typedef_types),
            Some(&self.module.typedef_unpacked_dims),
        )
    }

    /// A `foreach` index variable is implicitly `int` (signed, §12.7.3), so a
    /// negative declared bound (`foreach (a[i])` on `a[-2:-4]`) must read back
    /// as a negative number — not `from_u64`'s unsigned 4294967294. Masks to 32
    /// bits and marks the value signed so `%0d`, arithmetic, and negative array
    /// indexing (`a[i]`) all behave.
    #[inline]
    pub(super) fn signed_loop_val(i: i64) -> Value {
        let mut v = Value::from_u64(i as u64, 32);
        v.is_signed = true;
        v
    }

    /// The result of a built-in COUNT method: `size()` on a queue or dynamic
    /// array (IEEE 1800-2023 §7.10.2.1, §7.5.2), `num()`/`size()` on an
    /// associative array (§7.9.1), `len()` on a string (§6.16.1), `num()` on
    /// an enum (§6.19.5.3) or a mailbox (§15.4.3). Each is declared
    /// `function int`, so the value is SIGNED 32-bit: `q.size() - 2` on an
    /// empty queue is -2, and `i < q.size() - 1` does not wrap to ~4e9.
    #[inline]
    pub(super) fn int_count(n: u64) -> Value {
        Self::signed_loop_val(n as i64)
    }

    /// Run `body` once per index tuple, last dimension varying fastest.
    /// IEEE 1800-2017 §12.7.3: a dimension whose loop variable is OMITTED
    /// (`foreach (m[, j])`) is not traversed at all — only the named ones are.
    pub(super) fn exec_foreach_nested(
        &mut self,
        all_dims: &[(i64, i64)],
        vars: &[Option<crate::ast::Identifier>],
        body: &Statement,
        scope: Option<&str>,
        iterations: &mut u64,
    ) {
        self.exec_foreach_nested_dir(all_dims, vars, body, scope, None, iterations)
    }

    /// §12.7.3: the loop variable ranges LEFT bound → RIGHT bound. `desc`
    /// (aligned to `all_dims`, dims still normalized (lo,hi)) marks
    /// dimensions whose declared left bound is the HIGH one — those iterate
    /// hi→lo (packed `[3:0]` visits 3,2,1,0).
    pub(super) fn exec_foreach_nested_dir(
        &mut self,
        all_dims: &[(i64, i64)],
        vars: &[Option<crate::ast::Identifier>],
        body: &Statement,
        scope: Option<&str>,
        desc: Option<&[bool]>,
        iterations: &mut u64,
    ) {
        let iterated: Vec<((i64, i64), bool, &crate::ast::Identifier)> = all_dims
            .iter()
            .enumerate()
            .zip(vars)
            .filter_map(|((k, d), v)| {
                v.as_ref().map(|id| {
                    (
                        *d,
                        desc.and_then(|m| m.get(k)).copied().unwrap_or(false),
                        id,
                    )
                })
            })
            .collect();
        if iterated.is_empty() || iterated.iter().any(|((lo, hi), _, _)| hi < lo) {
            return;
        }
        let dims: Vec<(i64, i64)> = iterated.iter().map(|(d, _, _)| *d).collect();
        let descs: Vec<bool> = iterated.iter().map(|(_, dsc, _)| *dsc).collect();
        for (_, _, id) in &iterated {
            self.widths.insert(id.name.clone(), 32);
        }
        let n = dims.len();
        let mut idx: Vec<i64> = dims
            .iter()
            .zip(&descs)
            .map(|(d, &dsc)| if dsc { d.1 } else { d.0 })
            .collect();
        // Resolve the BODY's bare names under the array's instance too. The
        // loop bounds are found by scoping the array name, but the body then
        // wrote through an unscoped `m[i][j]` — in a submodule that resolved to
        // nothing and every write was dropped while the loop itself ran fine.
        // Only installed when the array is instance-scoped; restored on exit.
        let saved_fe_hint = self.name_resolve_hint.borrow().clone();
        if let Some(sc) = scope.filter(|s| !s.is_empty()) {
            *self.name_resolve_hint.borrow_mut() = Some(sc.to_string());
        }
        macro_rules! fe_done {
            () => {{
                *self.name_resolve_hint.borrow_mut() = saved_fe_hint.clone();
            }};
        }
        loop {
            if self.finished {
                fe_done!();
                return;
            }
            for (k, (_, _, id)) in iterated.iter().enumerate() {
                self.set_loop_var_aliased(scope, &id.name, Self::signed_loop_val(idx[k]));
            }
            self.continue_flag = false;
            *iterations = iterations.saturating_add(1);
            self.exec_statement(body);
            if self.break_flag {
                fe_done!();
                return;
            }
            self.continue_flag = false;
            // Odometer: advance the innermost dimension first.
            let mut k = n;
            loop {
                if k == 0 {
                    fe_done!();
                    return;
                }
                k -= 1;
                if descs[k] {
                    idx[k] -= 1;
                    if idx[k] >= dims[k].0 {
                        break;
                    }
                    idx[k] = dims[k].1;
                } else {
                    idx[k] += 1;
                    if idx[k] <= dims[k].1 {
                        break;
                    }
                    idx[k] = dims[k].0;
                }
            }
        }
    }

    /// `q[i]` / `q[i][j]` naming an ELEMENT of an unpacked array of queues —
    /// the element is itself a queue stored under that flat name (§7.4.5).
    pub(super) fn indexed_queue_base(&mut self, base: &Expression) -> Option<String> {
        if !matches!(base.kind, ExprKind::Index { .. }) {
            return None;
        }
        let n = self.flat_member_name(base)?;
        if self.module.dynamic_arrays.contains(&n) {
            Some(n)
        } else {
            None
        }
    }

    /// `a[i]` naming an ELEMENT of an unpacked array of ASSOCIATIVE arrays
    /// (`int a[2][u8_t]`): the element is itself an associative array.
    pub(super) fn indexed_assoc_base(&mut self, base: &Expression) -> Option<String> {
        if !matches!(base.kind, ExprKind::Index { .. }) {
            return None;
        }
        let n = self.flat_member_name(base)?;
        if self.module.associative_arrays.contains_key(&n) {
            Some(n)
        } else {
            None
        }
    }

    /// Flat storage name of `<assoc>[key]` for such an element.
    pub(super) fn indexed_assoc_elem(
        &mut self,
        base: &Expression,
        index: &Expression,
    ) -> Option<String> {
        let an = self.indexed_assoc_base(base)?;
        let kv = self.eval_expr(index);
        let key = self.assoc_key_str(&an, &kv);
        Some(format!("{}[{}]", an, key))
    }

    /// The text of key `kv` selecting into `level` of the nested associative
    /// array rooted at `root`: a row registered as an associative array of
    /// its own (`ensure_assoc_row`) uses its declared key type, a queue or
    /// dynamic row takes an element index, and an unregistered level (a row
    /// a whole copy made) is keyed like the root. §7.4.4: a declared level
    /// is keyed as declared, whether or not it is registered yet.
    fn nested_level_key(&self, level: &str, root: &str, kv: &Value) -> String {
        match self.declared_dim(level) {
            Some(Dim::Assoc(_)) => self.assoc_key_str(level, kv),
            Some(_) => kv.to_i64().unwrap_or(0).to_string(),
            // No declared shape (a level past a row's size): keyed as
            // registered, else as the root it hangs from.
            None if level != root && self.module.dynamic_arrays.contains(level) => {
                kv.to_i64().unwrap_or(0).to_string()
            }
            None if self.module.associative_arrays.contains_key(level) => {
                self.assoc_key_str(level, kv)
            }
            None => self.assoc_key_str(root, kv),
        }
    }

    pub(super) fn nested_index_name(&mut self, expr: &Expression) -> Option<String> {
        // Peel every Index level: `expr` is `Index(...Index(base, k1),...,kn)`.
        // Collect indices leaf-first, then resolve the root `Ident` to its
        // associative-array storage name and build the compound key
        // `base[k1][k2]...[kn]` using `assoc_key_str` for every segment — the
        // SAME format the write/read path (`multidim_assoc_elem`) uses, so a
        // method call like `m[k1][k2].exists(k3)` resolves to the stored
        // `m[K1][K2]` storage name. Without the recursion a depth-2 receiver
        // (`Index(Index(..),..)`) has a non-`Ident` base and returned `None`,
        // so `.exists()`/`.num()`/... never dispatched for 3D+ assoc arrays
        // (IEEE 1800-2023 §7.8.1) — breaking UVM's cycle detector
        // `m_recur_states[obj][obj][policy].exists(...)`.
        let mut rev: Vec<&Expression> = Vec::new();
        let mut cur = expr;
        loop {
            if let ExprKind::Index { expr: base, index } = &cur.kind {
                rev.push(index.as_ref());
                cur = base.as_ref();
            } else {
                break;
            }
        }
        if rev.is_empty() {
            return None; // not an Index expression
        }
        // The prefix with one key outside it (§7.4.5 fallback below).
        let ExprKind::Index { expr: start, .. } = &expr.kind else {
            return None;
        };
        // The root must be a plain Ident naming an associative array, or
        // (§7.4.5) a prefix of the chain names one: `v[0]["A"]` of
        // `int v[2][string][string]`, or `a[0].m["A"]` of a struct member.
        let bh = if let ExprKind::Ident(bh) = &cur.kind {
            bh
        } else {
            return self
                .inner_assoc_compound_key(start, &rev, 1)
                .map(|(_, k)| k);
        };
        let mut bn = self.resolve_hier_name(bh);
        if let Some(s) = self.instance_assoc_member(&bn) {
            bn = std::borrow::Cow::Owned(s);
        } else if bh.path.len() >= 2 {
            // `obj.member` accessed from module/lower scope where the first
            // path segment is an OBJECT handle and the member is an
            // associative property of that object's class. resolve_hier_name
            // collapses to the bare `member` (losing the per-instance scope),
            // so is_associative_array(bare) is false and the multidim helper
            // bails. Re-derive the per-instance storage `<h>#<member>`.
            let objnm = &bh.path[0].name.name;
            let member = &bh.path[bh.path.len() - 1].name.name;
            if let Some(h) = self.eval_ident_handle(objnm) {
                if h != 0 {
                    let cand = format!("{h}#{member}");
                    if self.is_associative_array(&cand) {
                        bn = std::borrow::Cow::Owned(cand);
                    }
                }
            }
        }
        if !self.is_associative_array(&bn) {
            return self
                .inner_assoc_compound_key(start, &rev, 1)
                .map(|(_, k)| k);
        }
        let mut name = bn.to_string();
        // Build root→leaf so the string reads `base[K1][K2]...[Kn]`. Each key
        // takes the key type of the array it selects into: a row registered
        // as an associative array of its own (`ensure_assoc_row`) has its
        // declared one; an unregistered level (a row a whole copy made, say)
        // is keyed like the root.
        for idx in rev.iter().rev() {
            let kv = self.eval_expr(idx);
            let ks = self.nested_level_key(&name, &bn, &kv);
            name = format!("{}[{}]", name, ks);
        }
        Some(name)
    }

    /// Resolve a base expression to its associative-array STORAGE name if it
    /// is one — a module-level signal name, or a per-instance `<handle>#member`
    /// (a bare class member accessed inside a method). Returns None for a
    /// non-assoc base (a fixed/dynamic/queue array, a non-array signal).
    pub(super) fn assoc_array_storage_name(&self, e: &Expression) -> Option<String> {
        if let ExprKind::Ident(h) = &e.kind {
            if h.path.len() == 1 && h.path[0].selects.is_empty() {
                let nm = self.resolve_hier_name(h);
                if self.is_associative_array(&nm) {
                    return Some(nm.to_string());
                }
                if let Some(scoped) = self.instance_assoc_member(&nm) {
                    if self.is_associative_array(&scoped) {
                        return Some(scoped);
                    }
                }
            }
        }
        None
    }

    /// A multiply-indexed ASSOCIATIVE-array element `base[k1][k2]...[kn]`
    /// (n ≥ 2) whose `base` is a registered associative array AND whose
    /// first-level element is NOT itself a queue/dynamic collection (which
    /// would make it an assoc-of-collection, handled by the queue/indexed
    /// paths). Returns `(assoc_base_name, flat_compound_key)` where the
    /// compound key is `base[K1][K2]...[Kn]` with each `Ki` rendered as an
    /// assoc key string via `assoc_key_str` — so the read and write paths
    /// (which both call this) stay byte-for-byte consistent regardless of
    /// key type (handle / int / signed / string). 1D indexes are left to the
    /// existing single-index assoc handlers.
    ///
    /// Multidimensional associative arrays (`int m[K1][K2]`, IEEE 1800-2023 §7.8.1)
    /// store each
    /// element under the flat compound key, mirroring how the 1D assoc
    /// machinery keys off `<name>[<key>]`. `m.exists(k)` scans the
    /// `<base>[k][` prefix; `m[k1].exists(k2)` checks `<base>[k1][k2]`.
    pub(super) fn multidim_assoc_elem(
        &mut self,
        base: &Expression,
        outer_index: &Expression,
    ) -> Option<(String, String)> {
        // The full expression is `base[outer_index]`; `base` may itself be a
        // nested Index (`m[k1][k2]` parses as Index(Index(m,k1),k2)). Unwrap
        // the Index levels of `base`, then treat `outer_index` as k_n.
        let mut rev: Vec<&Expression> = vec![outer_index];
        let mut cur = base;
        let mut depth = 1;
        while let ExprKind::Index { expr: b, index } = &cur.kind {
            rev.push(index.as_ref());
            cur = b.as_ref();
            depth += 1;
        }
        if depth < 2 {
            return None; // 1D — existing handlers
        }
        // A bare name, or a class member reached through a handle (`h.m`).
        let base_name = match self.assoc_array_storage_name(cur) {
            Some(n) => n,
            None if matches!(&cur.kind, ExprKind::MemberAccess { .. })
                || matches!(&cur.kind, ExprKind::Ident(h) if h.path.len() >= 2) =>
            {
                match self.handle_assoc_storage_name(cur) {
                    Some(n) => n,
                    None => return self.inner_assoc_compound_key(base, &rev, 2),
                }
            }
            // §7.4.5 / §7.8: the root is NOT associative but an element of it
            // is (`int v[2][string][string]`, `q[$][string][string]`, a
            // class property of that shape), or it is an associative struct
            // member (`a[0].m`): the compound key starts at the prefix naming
            // that associative array, `v[0]`.
            None => return self.inner_assoc_compound_key(base, &rev, 2),
        };
        // Distinguish a true multidim assoc from an assoc-of-queue/
        // dynamic-array (`m[k1][i]`): if the first-level element `m[K1]` is a
        // registered queue/dynamic collection, defer to the collection paths.
        let k0 = self.eval_expr(rev[depth - 1]);
        let k0s = self.assoc_key_str(&base_name, &k0);
        let first_elem = format!("{}[{}]", base_name, k0s);
        if self.module.dynamic_arrays.contains(&first_elem)
            || self.module.arrays.contains_key(&first_elem)
        {
            return None;
        }
        // Build the compound key inner→outer so it reads base[K1][K2]...[Kn],
        // each key in the key type of the array it selects into (see
        // `nested_index_name`; an unregistered level is keyed like the
        // root). Below a queue row (`q["x"][5][0]` of
        // `int q[string][int][$]`) the last key is the element index, and
        // the compound key is that row's element name.
        let mut key = base_name.clone();
        for idx in rev.iter().rev() {
            let kv = self.eval_expr(idx);
            let ks = self.nested_level_key(&key, &base_name, &kv);
            key = format!("{}[{}]", key, ks);
        }
        Some((base_name, key))
    }

    /// The associative array a multiply-indexed `e[k1]..[kn]` selects into
    /// when its root is not one (`multidim_assoc_elem`, `nested_index_name`):
    /// the shallowest prefix (`start` with `j` Index levels peeled, so `j + 1`
    /// keys outside it, at least `min_outside`; `j == rev.len() - 1` is the
    /// root itself) whose storage is a registered associative array, as
    /// `(that array, compound key)` with the outer keys (`rev[..=j]`,
    /// innermost last) rendered in its key format. None when no prefix is one, or when the element right below it
    /// is a queue / dynamic array (an associative array of queues, handled
    /// by the collection paths).
    #[inline(never)]
    pub(super) fn inner_assoc_compound_key(
        &mut self,
        start: &Expression,
        rev: &[&Expression],
        min_outside: usize,
    ) -> Option<(String, String)> {
        if self.module.associative_arrays.is_empty() {
            return None;
        }
        for j in (min_outside.max(1) - 1..rev.len()).rev() {
            let mut p = start;
            for _ in 0..j {
                let ExprKind::Index { expr, .. } = &p.kind else {
                    return None;
                };
                p = expr;
            }
            let name = match &p.kind {
                ExprKind::Index { .. } => self
                    .coll_elem_expr_key(p)
                    .filter(|n| self.module.associative_arrays.contains_key(n))
                    .or_else(|| self.flat_member_name(p)),
                // A struct member root (`s.m`, `a[0].m`), registered on first
                // sight like any struct member collection.
                ExprKind::MemberAccess { .. } => self.flat_member_name(p),
                ExprKind::Ident(h) if h.path.len() >= 2 => self.flat_member_name(p),
                _ => None,
            };
            if let Some(n) = name.as_deref().filter(|n| n.contains('.')) {
                self.ensure_struct_member_coll(n);
            }
            let Some(name) = name.filter(|n| self.is_associative_array(n)) else {
                continue;
            };
            let mut key = name.clone();
            for (n, idx) in rev[..=j].iter().rev().enumerate() {
                let kv = self.eval_expr(idx);
                let declared_tail = if self.dynamic_assoc_dims.is_empty() {
                    None
                } else {
                    self.dynamic_assoc_dims.get(&key).cloned()
                };
                let ks =
                    self.assoc_key_str(if declared_tail.is_some() { &key } else { &name }, &kv);
                let prior = declared_tail.as_ref().map(|_| key.clone());
                key.push('[');
                key.push_str(&ks);
                key.push(']');
                if let Some(tail) = declared_tail.filter(|tail| tail.len() > 1) {
                    if let Some(dt) = self
                        .module
                        .var_decl_types
                        .get(prior.as_deref().unwrap())
                        .cloned()
                    {
                        self.register_dynamic_assoc_tail(&key, &dt, &tail[1..]);
                    }
                }
                if n == 0
                    && j > 0
                    && (self.module.dynamic_arrays.contains(&key)
                        || self.module.arrays.contains_key(&key))
                {
                    return None;
                }
            }
            return Some((name, key));
        }
        None
    }

    /// All randomizable property names across an object's class hierarchy.
    pub(super) fn object_rand_set(&self, handle: usize) -> HashSet<String> {
        let mut out = HashSet::default();
        let mut cur = self
            .heap
            .get(handle)
            .and_then(|x| x.as_ref())
            .map(|i| i.class_name.clone());
        while let Some(cn) = cur {
            if let Some(cd) = self.module.classes.get(&cn) {
                for p in &cd.random_properties {
                    out.insert(p.clone());
                }
                cur = cd.extends.clone();
            } else {
                break;
            }
        }
        out
    }

    /// Apply one inline `std::randomize` constraint to free target variables.
    /// Handles `inside` (pick from the set, incl. queue membership), equality
    /// (pin), and conditional (if/else, implication) constraints.
    pub(super) fn apply_inline_constraint(
        &mut self,
        item: &crate::ast::decl::ConstraintItem,
        targets: &[(String, Expression)],
    ) {
        use crate::ast::decl::ConstraintItem as CI;
        match item {
            CI::Expr(e) => match &e.kind {
                ExprKind::Binary {
                    op: BinaryOp::Eq,
                    left,
                    right,
                } => {
                    // §18.5.9: `arr.size() == N` on a dynamic-array target
                    // SIZES the array (`new[N]` semantics, elements random).
                    // Nothing handled it, so the call reported success with
                    // the array left at its old size — the ubiquitous
                    // `std::randomize(list) with { list.size() == N; }` idiom
                    // silently produced an empty list.
                    // A size from a not yet constrained variable can be
                    // anything; one past the size phase's bound is left to
                    // the retries rather than allocated.
                    if let Some(arr) = self.size_call_on_target(left, targets) {
                        let n = self.eval_expr(right).to_i64().unwrap_or(0).max(0) as u64;
                        if n <= 1 << 20 {
                            self.resize_random_dyn_target(&arr, n);
                        }
                        return;
                    }
                    if let Some(arr) = self.size_call_on_target(right, targets) {
                        let n = self.eval_expr(left).to_i64().unwrap_or(0).max(0) as u64;
                        if n <= 1 << 20 {
                            self.resize_random_dyn_target(&arr, n);
                        }
                        return;
                    }
                    if let Some(lv) = self.target_for(left, targets) {
                        let v = self.eval_expr(right);
                        self.assign_value(&lv, &v);
                    } else if self.is_select_of_target(left, targets) {
                        // §18.3/§11.5.1: the constrained term may be a BIT or
                        // PART SELECT of a target (`scalar_addr[1:0] == 2'b0`,
                        // a word-alignment constraint). Solve it by writing the
                        // value straight through the select — leaving it to a
                        // lucky reseed made the whole call flaky.
                        let v = self.eval_expr(right);
                        self.assign_value(left, &v);
                    } else if self.is_select_of_target(right, targets) {
                        let v = self.eval_expr(left);
                        self.assign_value(right, &v);
                    }
                }
                ExprKind::Inside {
                    expr: inner,
                    ranges,
                } => {
                    if let Some(lv) = self.target_for(inner, targets) {
                        let target = self.eval_expr(inner);
                        if let Some(v) = self.pick_inside_value(ranges, &target) {
                            self.assign_value(&lv, &v);
                        }
                    }
                }
                _ => {}
            },
            CI::Inside {
                expr,
                range,
                is_dist,
                dist_weights,
                ..
            } => {
                // §18.3/§11.5.1: the constrained term may be a BIT or PART
                // SELECT of a target (`pattern[i] dist {0 := 98, 1 := 2}`,
                // the per-bit shape a foreach over a packed vector produces).
                // `target_for` matches whole variables only, so these picked
                // nothing and the dist/inside was silently dropped.
                if self.target_for(expr, targets).is_none()
                    && self.is_select_of_target(expr, targets)
                {
                    let picked = if *is_dist && !dist_weights.is_empty() {
                        self.pick_dist_value(None, range, dist_weights)
                    } else {
                        let rs: Vec<Expression> = range
                            .iter()
                            .map(|r| match r {
                                ConstraintRange::Value(e) => e.clone(),
                                ConstraintRange::Range { lo, hi } => Expression::new(
                                    ExprKind::Range(
                                        Box::new((*lo).clone()),
                                        Box::new((*hi).clone()),
                                    ),
                                    lo.span,
                                ),
                            })
                            .collect();
                        {
                            let target = self.eval_expr(expr);
                            self.pick_inside_value(&rs, &target)
                        }
                    };
                    if let Some(v) = picked {
                        self.assign_value(expr, &v);
                    }
                    return;
                }
                if let Some(lv) = self.target_for(expr, targets) {
                    let picked = if *is_dist && !dist_weights.is_empty() {
                        // §18.12 `std::randomize(v) with { v dist {...} }` — no
                        // object, so key the dist schedule on the variable name.
                        let key = Self::plain_ident_name(expr).map(|n| (usize::MAX, n));
                        self.pick_dist_value(key, range, dist_weights)
                    } else {
                        let rs: Vec<Expression> = range
                            .iter()
                            .map(|r| match r {
                                ConstraintRange::Value(e) => e.clone(),
                                ConstraintRange::Range { lo, hi } => Expression::new(
                                    ExprKind::Range(
                                        Box::new((*lo).clone()),
                                        Box::new((*hi).clone()),
                                    ),
                                    lo.span,
                                ),
                            })
                            .collect();
                        {
                            let target = self.eval_expr(expr);
                            self.pick_inside_value(&rs, &target)
                        }
                    };
                    if let Some(v) = picked {
                        self.assign_value(&lv, &v);
                    }
                }
            }
            CI::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                if self.eval_expr(condition).is_true() {
                    self.apply_inline_constraint(then_item, targets);
                } else if let Some(ei) = else_item {
                    self.apply_inline_constraint(ei, targets);
                }
            }
            CI::Implication {
                condition,
                constraint,
                ..
            } if self.eval_expr(condition).is_true() => {
                self.apply_inline_constraint(constraint, targets);
            }
            CI::Block(items) => {
                for it in items {
                    self.apply_inline_constraint(it, targets);
                }
            }
            CI::Soft(inner) => self.apply_inline_constraint(inner, targets),
            // IEEE 1800-2017 §18.5.7 iterative (foreach) constraints:
            // `foreach (arr[i]) <body>` constrains every element of the rand
            // array, with the loop variable `i` bound to each valid index.
            // The body is solved per element: equality pins (issue #28's
            // `arr[i] == i + 5`), `inside`/`dist` picks (riscv-dv's
            // `branch_idx[i] inside {[1:max_branch_step]}`), relational
            // narrowing, and conditional shapes.
            CI::Foreach {
                array, vars, item, ..
            } => {
                // `foreach (arr[i])` carries the loop vars separately, so
                // `array` is normally a bare identifier; unwrap an Index
                // shape too for robustness.
                let base = match &array.kind {
                    ExprKind::Index { expr: b, .. } => b.as_ref(),
                    _ => array,
                };
                // A multi-dimensional operand (2-D/N-D unpacked array, packed
                // multi-dimensional variable) is repaired element by element
                // with every loop variable bound; the 1-D path below keeps
                // its own solver.
                if vars.len() >= 2 || self.array_operand_name(base).is_none() {
                    if let Some(arr_name) = self.foreach_check_operand_name(base) {
                        let dims = self.foreach_check_dims(&arr_name, vars.len());
                        if dims.len() == vars.len() && !dims.is_empty() {
                            self.solve_inline_foreach_multidim(item, base, vars, &dims);
                            return;
                        }
                    }
                }
                if let Some(arr_name) = self.array_operand_name(base) {
                    // Index range: declared bounds for fixed arrays,
                    // 0..size-1 for queues / dynamic arrays.
                    let (lo, hi) = self
                        .foreach_dims(&arr_name)
                        .and_then(|d| d.first().copied())
                        // A DYNAMIC array registers the sentinel bounds
                        // (0, -1), which made this loop EMPTY — the whole
                        // foreach body silently applied to nothing, so
                        // `std::randomize(list) with { foreach ... }` left
                        // every element unconstrained. Use the live size.
                        .filter(|&(l, h)| h >= l)
                        .unwrap_or_else(|| (0, self.get_queue_size(&arr_name) as i64 - 1));
                    let elem_w = self
                        .module
                        .arrays
                        .get(&arr_name)
                        .map(|t| t.2)
                        .unwrap_or(32)
                        .max(1);
                    let idx_var: Option<String> = vars
                        .first()
                        .and_then(|v| v.as_ref().map(|id| id.name.clone()));
                    for i in lo..=hi {
                        // §18.5.7.1: bind the index variable so body
                        // expressions (`arr[i]`, `i + 5`) see the concrete
                        // index value.
                        let binds: Vec<(String, Value)> = idx_var
                            .iter()
                            .map(|iv| (iv.clone(), Self::signed_loop_val(i)))
                            .collect();
                        let saved = self.bind_loop_vars(binds);
                        self.solve_inline_foreach_elem(item, &arr_name, i, elem_w);
                        self.unbind_loop_vars(saved);
                    }
                } else if let Some(w) = Self::plain_ident_name(base)
                    .and_then(|n| self.lookup_signal_width(&n))
                    .filter(|&w| w > 1)
                {
                    // §18.5.7 foreach over a PACKED vector's bits
                    // (`foreach (possible[i]) …`) — often a STATE vector whose
                    // bits GUARD constraints on bit-selects of the target
                    // (`if (possible[i]) pattern[i] dist …; else pattern[i]==0;`).
                    // There is no unpacked array to solve elements of, so bind
                    // the index and apply the body as ordinary inline
                    // constraints — the select-aware equality/dist arms write
                    // through `target[i]`.
                    let idx_var: Option<String> = vars
                        .first()
                        .and_then(|v| v.as_ref().map(|id| id.name.clone()));
                    for i in 0..w as i64 {
                        let binds: Vec<(String, Value)> = idx_var
                            .iter()
                            .map(|iv| (iv.clone(), Self::signed_loop_val(i)))
                            .collect();
                        let saved = self.bind_loop_vars(binds);
                        self.apply_inline_constraint(item, targets);
                        self.unbind_loop_vars(saved);
                    }
                }
            }
            _ => {}
        }
    }

    /// `Some(array_name)` when `e` is `<target>.size()` / `<target>.len()` on a
    /// dynamic-array/queue scope-randomize target.
    pub(super) fn size_call_on_target(
        &mut self,
        e: &Expression,
        targets: &[(String, Expression)],
    ) -> Option<String> {
        let ExprKind::Call { func, args } = &e.kind else {
            return None;
        };
        if !args.is_empty() {
            return None;
        }
        let ExprKind::MemberAccess { expr, member } = &func.kind else {
            return None;
        };
        if member.name != "size" && member.name != "len" {
            return None;
        }
        let lv = self.target_for(expr, targets)?;
        let ExprKind::Ident(h) = &lv.kind else {
            return None;
        };
        let nm = self.resolve_hier_name(h);
        if self.module.dynamic_arrays.contains(&*nm) || self.module.arrays.contains_key(&*nm) {
            Some(nm.to_string())
        } else {
            None
        }
    }

    /// Bind `foreach` index variables for a `std::randomize` constraint walk.
    /// They go into the CURRENT local frame (a fresh frame would hide the
    /// caller's automatic locals — lookups see only the top frame — so a
    /// `foreach (mask[i]) if (mask[i]) …` over a task-local `mask` read x,
    /// and writes to a task-local target were lost when the frame popped).
    /// Returns what to restore; outside any frame one is pushed.
    pub(super) fn bind_loop_vars(
        &mut self,
        binds: Vec<(String, Value)>,
    ) -> Option<Vec<(String, Option<Value>)>> {
        let Some(top) = self.local_stack.last_mut() else {
            self.push_local_frame(binds.into_iter().collect());
            return None;
        };
        Some(
            binds
                .into_iter()
                .map(|(k, v)| {
                    let old = top.insert(k.clone(), v);
                    (k, old)
                })
                .collect(),
        )
    }

    pub(super) fn unbind_loop_vars(&mut self, saved: Option<Vec<(String, Option<Value>)>>) {
        let Some(saved) = saved else {
            self.pop_local_frame();
            return;
        };
        if let Some(top) = self.local_stack.last_mut() {
            for (k, old) in saved.into_iter().rev() {
                match old {
                    Some(v) => {
                        top.insert(k, v);
                    }
                    None => {
                        top.remove(&k);
                    }
                }
            }
        }
    }

    /// `std::randomize` sizing for dynamic-array targets constrained only by
    /// bounds on `size()`: collect `size() REL e` / `e REL size()` (with `e`
    /// reading no target) and `size() inside {…}` over the top-level items,
    /// and draw a size inside them. An array with an `==` on its size, or no
    /// bound at all, is left alone.
    pub(super) fn size_bounded_dyn_targets(
        &mut self,
        constraints: &[crate::ast::decl::ConstraintItem],
        targets: &[(String, Expression)],
    ) {
        use crate::ast::decl::ConstraintItem as CI;
        use rand::Rng;
        let roots: HashSet<String> = targets
            .iter()
            .filter_map(|(_, lv)| Self::randomize_target_root(lv))
            .collect();
        let mut flat: Vec<&CI> = Vec::new();
        let mut stack: Vec<&CI> = constraints.iter().collect();
        while let Some(it) = stack.pop() {
            match it {
                CI::Block(xs) => stack.extend(xs.iter()),
                other => flat.push(other),
            }
        }
        // array -> (lo, hi, pinned by ==)
        let mut bounds: HashMap<String, (i64, i64, bool)> = HashMap::default();
        // array -> the values a `size() inside {…}` allows
        let mut sets: HashMap<String, Vec<(i64, i64)>> = HashMap::default();
        for it in flat {
            match it {
                // §11.4.13: `size() inside {…}` written as an expression.
                CI::Expr(e) if matches!(e.kind, ExprKind::Inside { .. }) => {
                    let ExprKind::Inside { expr, ranges } = &e.kind else {
                        continue;
                    };
                    let Some(arr) = self.size_call_on_target(expr, targets) else {
                        continue;
                    };
                    let mut set: Option<Vec<(i64, i64)>> = Some(Vec::new());
                    for r in ranges {
                        let iv = match &r.kind {
                            ExprKind::Range(lo, hi) => {
                                self.eval_expr(lo).to_i64().zip(self.eval_expr(hi).to_i64())
                            }
                            _ => self.eval_expr(r).to_i64().map(|v| (v, v)),
                        };
                        match (iv, set.as_mut()) {
                            (Some((l, h)), Some(s)) => s.push((l.min(h), l.max(h))),
                            _ => set = None,
                        }
                    }
                    let b = bounds
                        .entry(arr.clone())
                        .or_insert((0, i32::MAX as i64, false));
                    match set {
                        Some(s) if !s.is_empty() && !sets.contains_key(&arr) => {
                            sets.insert(arr, s);
                        }
                        _ => b.2 = true,
                    }
                }
                CI::Expr(e) => {
                    let ExprKind::Binary { op, left, right } = &e.kind else {
                        continue;
                    };
                    let (arr, other, flip) =
                        if let Some(a) = self.size_call_on_target(left, targets) {
                            (a, right, false)
                        } else if let Some(a) = self.size_call_on_target(right, targets) {
                            (a, left, true)
                        } else {
                            continue;
                        };
                    let b = bounds.entry(arr).or_insert((0, i32::MAX as i64, false));
                    if *op == BinaryOp::Eq {
                        b.2 = true;
                        continue;
                    }
                    let mut reads = false;
                    Self::walk_operands(other, &mut |x| {
                        if let ExprKind::Ident(h) = &x.kind {
                            if h.path.first().is_some_and(|f| roots.contains(&f.name.name)) {
                                reads = true;
                            }
                        }
                    });
                    if reads {
                        b.2 = true;
                        continue;
                    }
                    let Some(k) = self.eval_expr(other).to_i64() else {
                        continue;
                    };
                    // Read as `size OP k`.
                    let op = if flip {
                        match op {
                            BinaryOp::Lt => BinaryOp::Gt,
                            BinaryOp::Leq => BinaryOp::Geq,
                            BinaryOp::Gt => BinaryOp::Lt,
                            BinaryOp::Geq => BinaryOp::Leq,
                            o => *o,
                        }
                    } else {
                        *op
                    };
                    match op {
                        BinaryOp::Gt => b.0 = b.0.max(k.saturating_add(1)),
                        BinaryOp::Geq => b.0 = b.0.max(k),
                        BinaryOp::Lt => b.1 = b.1.min(k.saturating_sub(1)),
                        BinaryOp::Leq => b.1 = b.1.min(k),
                        _ => {}
                    }
                }
                CI::Inside { expr, range, .. } => {
                    if let Some(arr) = self.size_call_on_target(expr, targets) {
                        // A set membership pins the choice to the set.
                        let b = bounds.entry(arr).or_insert((0, i32::MAX as i64, false));
                        if range.len() == 1 {
                            if let crate::ast::decl::ConstraintRange::Range { lo, hi } = &range[0] {
                                if let (Some(l), Some(h)) =
                                    (self.eval_expr(lo).to_i64(), self.eval_expr(hi).to_i64())
                                {
                                    b.0 = b.0.max(l);
                                    b.1 = b.1.min(h);
                                    continue;
                                }
                            }
                        }
                        b.2 = true;
                    }
                }
                _ => {}
            }
        }
        for (arr, (lo, hi, pinned)) in bounds {
            if pinned || lo > hi || (lo == 0 && hi == i32::MAX as i64 && !sets.contains_key(&arr)) {
                continue;
            }
            // A set: a value of it inside the bounds, every value alike.
            if let Some(set) = sets.get(&arr) {
                let parts: Vec<(i64, i64)> = set
                    .iter()
                    .map(|&(l, h)| (l.max(lo).max(0), h.min(hi).min(lo.max(0) + (1 << 20))))
                    .filter(|(l, h)| l <= h)
                    .collect();
                let total: i64 = parts.iter().map(|(l, h)| h - l + 1).sum();
                if total == 0 {
                    continue;
                }
                let mut k = self.cur_rng().gen_range(0..total);
                for (l, h) in parts {
                    if k <= h - l {
                        self.resize_random_dyn_target(&arr, (l + k) as u64);
                        break;
                    }
                    k -= h - l + 1;
                }
                continue;
            }
            // An open upper bound stays near the lower one: an unbounded
            // draw would build an enormous array.
            let hi = hi.min(lo.saturating_add(1024));
            let n = self.cur_rng().gen_range(lo.max(0)..=hi.max(lo.max(0)));
            self.resize_random_dyn_target(&arr, n as u64);
        }
    }

    /// §7.2.1/§7.4.5: a struct member that is a dynamic collection is its
    /// own collection object. Elaboration registers it for a plain struct
    /// variable with a one-dimension member (`s.q` for `int q[$]`), but an
    /// element of an ARRAY of structs (`sa[1].q`) was left out of
    /// `dynamic_arrays` — `push_back` was dropped and `size()` read the
    /// storage placeholder — and a member that is an ARRAY of queues
    /// (`rg_t q[7:0][$]`) was registered as a 2-D array of scalars. Register
    /// both the way a standalone `int q[$]` / `int aq[7:0][$]` is.
    pub(super) fn register_struct_member_collections(&mut self) {
        use crate::ast::types::UnpackedDimension as UD;
        // A root is a declared variable: a bare name, or one scoped by an
        // instance path (`top2.h` when several top modules run) — not a
        // member path below another declared variable.
        fn is_root(vdt: &HashMap<String, DataType>, k: &str, extra: &[char]) -> bool {
            !k.contains(['[', ':'])
                && !k.contains(extra)
                && k.rsplit_once('.').is_none_or(|(parent, _)| {
                    !vdt.contains_key(parent) && !parent.contains(['[', ':', '#'])
                })
        }
        let roots: Vec<(String, DataType)> = self
            .module
            .var_decl_types
            .iter()
            .filter(|(k, _)| is_root(&self.module.var_decl_types, k, &[]))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let params = self.module.parameters.clone();
        let fixed = |d: &UD| -> Option<(i64, i64)> {
            match d {
                UD::Range { left, right, .. } => {
                    let l = super::elaborate::const_eval_i64_with_params(left, Some(&params))?;
                    let r = super::elaborate::const_eval_i64_with_params(right, Some(&params))?;
                    Some((l.min(r), l.max(r)))
                }
                UD::Expression { expr, .. } => {
                    let n = super::elaborate::const_eval_i64_with_params(expr, Some(&params))?;
                    (n > 0).then_some((0, n - 1))
                }
                _ => None,
            }
        };
        let mut new_dyn: Vec<(String, u32, bool)> = Vec::new();
        let mut new_outer: Vec<(String, (i64, i64), u32)> = Vec::new();
        for (root, dt) in roots {
            let resolved =
                super::elaborate::resolve_typedef_chain(&dt, &self.module.typedef_types).clone();
            let DataType::Struct(su) = resolved else {
                continue;
            };
            if su.packed {
                continue;
            }
            // The struct variables: the root itself, or each element of a
            // 1-D array of structs.
            let elem_array = self
                .module
                .arrays
                .get(&root)
                .copied()
                .filter(|_| !self.module.dynamic_arrays.contains(&root));
            let prefixes: Vec<String> = match elem_array {
                Some((lo, hi, _)) => (lo.min(hi)..=lo.max(hi))
                    .map(|i| format!("{}[{}]", root, i))
                    .collect(),
                None => vec![root.clone()],
            };
            for m in &su.members {
                let mw = super::elaborate::resolve_type_width(
                    &m.data_type,
                    Some(&self.module.parameters),
                    Some(&self.module.typedefs),
                )
                .max(1);
                for d in &m.declarators {
                    let dyn_kind = |x: &UD| match x {
                        UD::Queue { .. } => Some(true),
                        UD::Unsized(_) => Some(false),
                        _ => None,
                    };
                    match d.dimensions.as_slice() {
                        [one] if elem_array.is_some() => {
                            if let Some(is_q) = dyn_kind(one) {
                                for p in &prefixes {
                                    new_dyn.push((format!("{}.{}", p, d.name.name), mw, is_q));
                                }
                            }
                        }
                        [outer, last] => {
                            let (Some(range), Some(is_q)) = (fixed(outer), dyn_kind(last)) else {
                                continue;
                            };
                            for p in &prefixes {
                                let base = format!("{}.{}", p, d.name.name);
                                for i in range.0..=range.1 {
                                    new_dyn.push((format!("{}[{}]", base, i), mw, is_q));
                                }
                                new_outer.push((base, range, mw));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        for (base, (lo, hi), w) in new_outer {
            self.module.arrays_2d.remove(&base);
            self.module.arrays.insert(base, (lo, hi, w));
        }
        for (name, w, is_q) in new_dyn {
            if self.module.dynamic_arrays.contains(&name) {
                continue;
            }
            self.module.dynamic_arrays.insert(name.clone());
            if is_q {
                self.module.queue_vars.insert(name.clone());
            }
            self.module.arrays.insert(name.clone(), (0, 63, w));
            self.set_queue_size(&name, 0);
        }
        // Every other statically named dynamic member: one inside a NESTED
        // struct (`s.in.q`), an associative member (`s.aa`), or one below
        // a fixed member array of structs (`s.rows[1].q`). Each registers
        // exactly as its runtime classification would.
        let mut names: Vec<String> = Vec::new();
        let mut budget = 1usize << 16;
        let roots2: Vec<(String, DataType)> = self
            .module
            .var_decl_types
            .iter()
            .filter(|(k, _)| is_root(&self.module.var_decl_types, k, &['#']))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (root, dt) in roots2.iter() {
            let Some(su) = self.unpacked_struct_of(dt) else {
                continue;
            };
            let prefixes: Vec<String> = match self.module.arrays.get(root.as_str()) {
                Some(_) if self.module.dynamic_arrays.contains(root.as_str()) => continue,
                Some(&(lo, hi, _)) if hi.abs_diff(lo) < 4096 => (lo.min(hi)..=lo.max(hi))
                    .map(|i| format!("{}[{}]", root, i))
                    .collect(),
                Some(_) => continue,
                None if self.is_associative_array(root) => continue,
                None if self.module.arrays_2d.contains_key(root.as_str())
                    || self.module.arrays_nd.contains_key(root.as_str()) =>
                {
                    continue;
                }
                None => vec![root.clone()],
            };
            for p in prefixes {
                self.collect_struct_member_colls(&p, &su, 0, &mut budget, &mut names);
            }
        }
        for n in names {
            self.ensure_struct_member_coll(&n);
        }
    }

    /// The statically named dynamic members below the struct `su` rooted at
    /// `base` (see `register_struct_member_collections`).
    pub(super) fn collect_struct_member_colls(
        &self,
        base: &str,
        su: &crate::ast::types::StructUnionType,
        depth: usize,
        budget: &mut usize,
        out: &mut Vec<String>,
    ) {
        if depth > 8 {
            return;
        }
        for m in &su.members {
            for d in &m.declarators {
                if *budget == 0 {
                    return;
                }
                *budget -= 1;
                let target = format!("{}.{}", base, d.name.name);
                if d.dimensions.is_empty() {
                    if let Some(inner) = self.unpacked_struct_of(&m.data_type) {
                        self.collect_struct_member_colls(&target, &inner, depth + 1, budget, out);
                    }
                } else if d.dimensions.len() == 1 && Self::coll_kind_of(&d.dimensions).is_some() {
                    if !self.module.dynamic_arrays.contains(&target)
                        && !self.module.associative_arrays.contains_key(&target)
                    {
                        out.push(target);
                    }
                } else if let Some(inner) = self.unpacked_struct_of(&m.data_type) {
                    if let Some(tups) = self.member_elem_indices(&d.dimensions) {
                        for t in tups {
                            let et = format!("{}{}", target, Self::index_suffix(&t));
                            self.collect_struct_member_colls(&et, &inner, depth + 1, budget, out);
                        }
                    }
                }
            }
        }
    }

    /// `new[n]`-style resize of a dynamic-array randomize target: set the size
    /// and draw a fresh random value for every element (they are rand and
    /// otherwise unconstrained; later constraint items may overwrite).
    pub(super) fn resize_random_dyn_target(&mut self, arr: &str, n: u64) {
        use rand::Rng;
        let w = self
            .module
            .arrays
            .get(arr)
            .map(|t| t.2)
            .unwrap_or(32)
            .max(1);
        self.set_queue_size(arr, n);
        for i in 0..n {
            let r: u64 = self.cur_rng().r#gen();
            let masked = if w >= 64 { r } else { r & ((1u64 << w) - 1) };
            self.set_signal_value_by_name(&format!("{}[{}]", arr, i), Value::from_u64(masked, w));
        }
    }

    /// §18.3: true when `e` is a BIT or PART select of one of the scope-randomize
    /// targets (`scalar_addr[1:0]`, `data[7:4]`). Such a term is solvable — the
    /// value is written straight through the select — but it is not a target in
    /// its own right, so `target_for` (which matches whole variables) misses it.
    pub(super) fn is_select_of_target(
        &mut self,
        e: &Expression,
        targets: &[(String, Expression)],
    ) -> bool {
        let base = match &e.kind {
            ExprKind::Index { expr, .. } => expr,
            ExprKind::RangeSelect { expr, .. } => expr,
            _ => return false,
        };
        // An ARRAY element is not a select of a scalar target — its own
        // per-element machinery (foreach / unique) owns it.
        if self.array_operand_name(base).is_some() {
            return false;
        }
        match self.constraint_operand_name(base) {
            Some(n) => targets.iter().any(|(tn, _)| *tn == n),
            None => false,
        }
    }

    /// §18.5.5 `unique { arr }` over scope-randomize targets: make the elements
    /// of each listed array/queue pairwise distinct. A duplicate is re-picked
    /// from the element's OWN allowed domain — the `foreach (arr[i]) arr[i]
    /// inside {…}` item in the same constraint set — so the repair can never
    /// break the range constraint it shares the block with. Without this the
    /// scope path enforced nothing and simply hoped the random draw came out
    /// distinct.
    pub(super) fn enforce_inline_unique(
        &mut self,
        constraints: &[crate::ast::decl::ConstraintItem],
    ) {
        for e in self.collect_unique_operands(constraints) {
            let Some(arr) = self.array_operand_name(&e) else {
                continue;
            };
            let size = self.get_queue_size(&arr);
            if size == 0 {
                continue;
            }
            let w = self
                .module
                .arrays
                .get(&arr)
                .map(|t| t.2)
                .unwrap_or(32)
                .max(1);
            let ranges = self.inline_elem_ranges(constraints, &arr);
            let mut used: HashSet<u64> = HashSet::default();
            for i in 0..size {
                let key = format!("{}[{}]", arr, i);
                let cur = self.get_signal_value_by_name(&key).and_then(|v| v.to_u64());
                let in_domain = |v: u64| {
                    ranges.is_empty() || ranges.iter().any(|(lo, hi)| v >= *lo && v <= *hi)
                };
                if let Some(c) = cur {
                    if !used.contains(&c) && in_domain(c) {
                        used.insert(c);
                        continue;
                    }
                }
                if let Some(v) = self.pick_unused_in_ranges(&ranges, w, &used) {
                    used.insert(v);
                    self.set_signal_value_by_name(&key, Value::from_u64(v, w));
                }
            }
        }
    }

    /// The array operands of every `unique {…}` item in a constraint set.
    pub(super) fn collect_unique_operands(
        &self,
        constraints: &[crate::ast::decl::ConstraintItem],
    ) -> Vec<Expression> {
        use crate::ast::decl::ConstraintItem as CI;
        fn walk(item: &CI, out: &mut Vec<Expression>) {
            match item {
                CI::Unique { exprs, .. } => out.extend(exprs.iter().cloned()),
                CI::Block(items) => {
                    for it in items {
                        walk(it, out);
                    }
                }
                CI::Soft(inner) => walk(inner, out),
                _ => {}
            }
        }
        let mut out = Vec::new();
        for c in constraints {
            walk(c, &mut out);
        }
        out
    }

    /// §18.5.7: the `(lo, hi)` value ranges a constraint set imposes on the
    /// ELEMENTS of `arr` — the `inside` list inside its `foreach` body. Empty
    /// when the elements are unconstrained.
    pub(super) fn inline_elem_ranges(
        &mut self,
        constraints: &[crate::ast::decl::ConstraintItem],
        arr: &str,
    ) -> Vec<(u64, u64)> {
        let mut out = Vec::new();
        for c in constraints {
            self.collect_elem_ranges(c, arr, &mut out);
        }
        out
    }

    pub(super) fn collect_elem_ranges(
        &mut self,
        item: &crate::ast::decl::ConstraintItem,
        arr: &str,
        out: &mut Vec<(u64, u64)>,
    ) {
        use crate::ast::decl::ConstraintItem as CI;
        match item {
            CI::Expr(e) => {
                if let ExprKind::Inside { expr, ranges } = &e.kind {
                    if self.is_elem_of(expr, arr) {
                        for r in ranges {
                            if let ExprKind::Range(lo, hi) = &r.kind {
                                let l = self.eval_expr(lo).to_u64().unwrap_or(0);
                                let h = self.eval_expr(hi).to_u64().unwrap_or(0);
                                out.push((l, h));
                            } else {
                                let v = self.eval_expr(r).to_u64().unwrap_or(0);
                                out.push((v, v));
                            }
                        }
                    }
                }
            }
            CI::Inside { expr, range, .. } if self.is_elem_of(expr, arr) => {
                for r in range {
                    match r {
                        ConstraintRange::Range { lo, hi } => {
                            let l = self.eval_expr(lo).to_u64().unwrap_or(0);
                            let h = self.eval_expr(hi).to_u64().unwrap_or(0);
                            out.push((l, h));
                        }
                        ConstraintRange::Value(v) => {
                            let v = self.eval_expr(v).to_u64().unwrap_or(0);
                            out.push((v, v));
                        }
                    }
                }
            }
            CI::Foreach { item, .. } => self.collect_elem_ranges(item, arr, out),
            CI::Block(items) => {
                for it in items {
                    self.collect_elem_ranges(it, arr, out);
                }
            }
            CI::Soft(inner) => self.collect_elem_ranges(inner, arr, out),
            _ => {}
        }
    }

    /// True when `e` indexes into `arr` (`arr[i]`), whatever the index is.
    pub(super) fn is_elem_of(&mut self, e: &Expression, arr: &str) -> bool {
        let e = match &e.kind {
            ExprKind::Paren(inner) => inner.as_ref(),
            _ => e,
        };
        match &e.kind {
            ExprKind::Index { expr: base, .. } => {
                self.array_operand_name(base).as_deref() == Some(arr)
            }
            _ => false,
        }
    }

    /// Pick a value inside `ranges` (or the full `w`-bit domain when empty) that
    /// is not already in `used`. Random draws first, then an exhaustive scan so a
    /// tight domain still resolves deterministically.
    pub(super) fn pick_unused_in_ranges(
        &mut self,
        ranges: &[(u64, u64)],
        w: u32,
        used: &HashSet<u64>,
    ) -> Option<u64> {
        use rand::Rng;
        let full: Vec<(u64, u64)> = if ranges.is_empty() {
            let max = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
            vec![(0, max)]
        } else {
            ranges.to_vec()
        };
        for _ in 0..64 {
            let (lo, hi) = full[self.rng.gen_range(0..full.len())];
            if hi < lo {
                continue;
            }
            let v = if lo == hi {
                lo
            } else {
                self.rng.gen_range(lo..=hi)
            };
            if !used.contains(&v) {
                return Some(v);
            }
        }
        for (lo, hi) in &full {
            let span = hi.saturating_sub(*lo);
            if span > (1 << 20) {
                continue;
            }
            for v in *lo..=*hi {
                if !used.contains(&v) {
                    return Some(v);
                }
            }
        }
        None
    }

    /// Narrow the [lo, hi] interval for randomize target `name` using the
    /// relational items (`<`, `<=`, `>`, `>=`) in one inline-constraint item.
    /// Only `target REL bound` shapes where `bound` references no other
    /// randomize target contribute (inter-target relations are left to the
    /// equality/fixpoint pass). Sets `any` when at least one bound applied.
    pub(super) fn narrow_relational_bounds(
        &mut self,
        item: &crate::ast::decl::ConstraintItem,
        name: &str,
        targets: &[(String, Expression)],
        lo: &mut i64,
        hi: &mut i64,
        any: &mut bool,
    ) {
        use crate::ast::decl::ConstraintItem as CI;
        match item {
            CI::Expr(e) => {
                if let ExprKind::Binary { op, left, right } = &e.kind {
                    if !matches!(
                        op,
                        BinaryOp::Lt | BinaryOp::Leq | BinaryOp::Gt | BinaryOp::Geq
                    ) {
                        return;
                    }
                    // §18.3: decompose each side as an affine function of the
                    // target (`ls*t + lc REL rs*t + rc`), seeing through
                    // §6.24.1 size casts / type casts. This subsumes the
                    // plain `t REL bound` shape (scale 1 / scale 0) and also
                    // solves cast-wrapped compounds like `32'(A * B) < 1000`
                    // (issue #29: A known 8-bit, B the target → B <= 999/A).
                    let Some((ls, lc)) = self.affine_in_target(left, name, targets) else {
                        return;
                    };
                    let Some((rs, rc)) = self.affine_in_target(right, name, targets) else {
                        return;
                    };
                    // Rearrange to `s*t REL c`.
                    let (Some(s), Some(c)) = (ls.checked_sub(rs), rc.checked_sub(lc)) else {
                        return;
                    };
                    if s == 0 {
                        return; // target cancelled out — nothing to narrow
                    }
                    // Normalize to positive scale (dividing an inequality by
                    // a negative flips it).
                    let (s, c, eff) = if s < 0 {
                        let flipped = match op {
                            BinaryOp::Lt => BinaryOp::Gt,
                            BinaryOp::Gt => BinaryOp::Lt,
                            BinaryOp::Leq => BinaryOp::Geq,
                            BinaryOp::Geq => BinaryOp::Leq,
                            other => *other,
                        };
                        (-s, c.checked_neg().unwrap_or(i64::MAX), flipped)
                    } else {
                        (s, c, *op)
                    };
                    // Integer bounds on t for `s*t REL c`, s > 0 (floor/ceil
                    // division keeps them exact for non-unit scales).
                    let div_floor = |a: i64, b: i64| a.div_euclid(b);
                    let div_ceil = |a: i64, b: i64| a.checked_add(b - 1).map(|x| x.div_euclid(b));
                    match eff {
                        BinaryOp::Gt => {
                            // s*t > c  →  t >= floor(c/s) + 1
                            *lo = (*lo).max(div_floor(c, s).saturating_add(1));
                            *any = true;
                        }
                        BinaryOp::Geq => {
                            // s*t >= c →  t >= ceil(c/s)
                            let Some(b) = div_ceil(c, s) else { return };
                            *lo = (*lo).max(b);
                            *any = true;
                        }
                        BinaryOp::Lt => {
                            // s*t < c  →  t <= floor((c-1)/s)
                            *hi = (*hi).min(div_floor(c.saturating_sub(1), s));
                            *any = true;
                        }
                        BinaryOp::Leq => {
                            // s*t <= c →  t <= floor(c/s)
                            *hi = (*hi).min(div_floor(c, s));
                            *any = true;
                        }
                        _ => {}
                    }
                }
            }
            CI::Block(items) => {
                for it in items {
                    self.narrow_relational_bounds(it, name, targets, lo, hi, any);
                }
            }
            CI::Soft(inner) => self.narrow_relational_bounds(inner, name, targets, lo, hi, any),
            _ => {}
        }
    }

    /// True when `e` names element `idx` of array `arr` — i.e. an inline
    /// `foreach` body operand `arr[i]` with the loop var currently bound so
    /// its index expression evaluates to `idx` (§18.5.7).
    pub(super) fn inline_foreach_elem_ref(&mut self, e: &Expression, arr: &str, idx: i64) -> bool {
        let e = match &e.kind {
            // §6.24: a type cast lowered to a pass-through Paren wrapper.
            ExprKind::Paren(inner) => inner.as_ref(),
            _ => e,
        };
        if let ExprKind::Index { expr: base, index } = &e.kind {
            if self.array_operand_name(base).as_deref() == Some(arr) {
                return self.eval_expr(index).to_i64() == Some(idx);
            }
        }
        false
    }

    /// §18.5.7: solve one inline-`foreach` body item for element `arr[idx]`.
    /// The loop index variable is already bound in the top local_stack frame,
    /// so index expressions (`i`, `i+1`) and value expressions (`i + 5`)
    /// evaluate with the concrete index.
    ///
    /// Two-phase per element (mirrors the scalar-target flow in
    /// `eval_randomize_with`): first ACCUMULATE all relational bounds across
    /// the whole body into one [lo, hi] interval — item-at-a-time re-picking
    /// would ping-pong between `arr[i] > 100` and `arr[i] <= 110` — then
    /// apply pins (equality / `inside` / `dist`), and finally re-pick inside
    /// the interval only when the current value violates it, so
    /// cross-element relations (`arr[i] > arr[i-1]`) converge across the
    /// outer apply sweeps.
    /// Per-element repair for a constraint `foreach (a[v1, v2, ...]) body`
    /// over a multi-dimensional operand. For every index tuple the loop
    /// variables are bound in a frame and each `Expr` item in the body is
    /// applied to the element `a[v1][v2]...` (read and written through the
    /// ordinary expression paths, so packed slices and unpacked elements
    /// behave alike): relational operators narrow the element's range and
    /// re-draw it inside; `elem == e` pins it; `$countones(x[...]) == elem`
    /// (either way round) draws `x[...]` with exactly that many ones.
    pub(super) fn solve_inline_foreach_multidim(
        &mut self,
        body: &crate::ast::decl::ConstraintItem,
        base: &Expression,
        vars: &[Option<crate::ast::Identifier>],
        dims: &[(i64, i64)],
    ) {
        use crate::ast::decl::ConstraintItem as CI;
        use rand::Rng;
        let names: Vec<Option<String>> = vars
            .iter()
            .map(|v| v.as_ref().map(|i| i.name.clone()))
            .collect();
        let items: Vec<&CI> = match body {
            CI::Block(items) => items.iter().collect(),
            other => vec![other],
        };
        let mut elem = base.clone();
        for v in vars {
            let Some(v) = v else { return };
            let idx = Expression::new(
                ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                    root: None,
                    path: vec![crate::ast::expr::HierPathSegment {
                        name: v.clone(),
                        selects: Vec::new(),
                    }],
                    span: crate::ast::Span::dummy(),
                    cached_signal_id: std::cell::Cell::new(None),
                    cached_resolved_name: std::cell::OnceCell::new(),
                }),
                crate::ast::Span::dummy(),
            );
            elem = Expression::new(
                ExprKind::Index {
                    expr: Box::new(elem),
                    index: Box::new(idx),
                },
                crate::ast::Span::dummy(),
            );
        }
        let mut idx: Vec<i64> = dims.iter().map(|&(lo, _)| lo).collect();
        if dims.iter().any(|&(lo, hi)| hi < lo) {
            return;
        }
        loop {
            let mut binds: Vec<(String, Value)> = Vec::new();
            for (k, n) in names.iter().enumerate() {
                if let Some(n) = n {
                    binds.push((n.clone(), Self::signed_loop_val(idx[k])));
                }
            }
            let saved = self.bind_loop_vars(binds);
            let cur = self.eval_expr(&elem);
            let w = cur.width.max(1);
            let (mut lo, mut hi): (i64, i64) =
                (0, if w >= 63 { i64::MAX } else { (1i64 << w) - 1 });
            let mut any_rel = false;
            let mut pinned = false;
            let mut popcount_targets: Vec<Expression> = Vec::new();
            for it in &items {
                let CI::Expr(e) = it else { continue };
                let ExprKind::Binary { op, left, right } = &e.kind else {
                    continue;
                };
                let on_l = Self::expr_same_ignoring_span(left, &elem);
                let on_r = Self::expr_same_ignoring_span(right, &elem);
                if on_l == on_r {
                    continue;
                }
                let (other, eff) = if on_l {
                    (right.as_ref(), *op)
                } else {
                    let m = match op {
                        BinaryOp::Lt => BinaryOp::Gt,
                        BinaryOp::Gt => BinaryOp::Lt,
                        BinaryOp::Leq => BinaryOp::Geq,
                        BinaryOp::Geq => BinaryOp::Leq,
                        o => *o,
                    };
                    (left.as_ref(), m)
                };
                if let ExprKind::SystemCall { name, args } = &other.kind {
                    if name == "$countones" && matches!(eff, BinaryOp::Eq) {
                        if let Some(t) = args.first() {
                            popcount_targets.push(t.clone());
                        }
                        continue;
                    }
                }
                let b = self.eval_expr(other).to_i64().unwrap_or(0);
                match eff {
                    BinaryOp::Lt => {
                        hi = hi.min(b.saturating_sub(1));
                        any_rel = true;
                    }
                    BinaryOp::Leq => {
                        hi = hi.min(b);
                        any_rel = true;
                    }
                    BinaryOp::Gt => {
                        lo = lo.max(b.saturating_add(1));
                        any_rel = true;
                    }
                    BinaryOp::Geq => {
                        lo = lo.max(b);
                        any_rel = true;
                    }
                    BinaryOp::Eq => {
                        let v = self.eval_expr(other).resize(w);
                        let e2 = elem.clone();
                        self.assign_value(&e2, &v);
                        pinned = true;
                    }
                    _ => {}
                }
            }
            if !pinned && any_rel && lo <= hi {
                let curv = cur.to_i64().unwrap_or(0);
                if curv < lo || curv > hi {
                    let p = if lo == hi {
                        lo
                    } else {
                        self.cur_rng().gen_range(lo..=hi)
                    };
                    let e2 = elem.clone();
                    self.assign_value(&e2, &Value::from_u64(p as u64, w));
                }
            }
            if !popcount_targets.is_empty() {
                let want = self.eval_expr(&elem).to_i64().unwrap_or(0).max(0) as usize;
                for t in popcount_targets {
                    let tw = self.eval_expr(&t).width.max(1) as usize;
                    let ones = want.min(tw);
                    let mut bits: Vec<usize> = (0..tw).collect();
                    for i in (1..bits.len()).rev() {
                        let j = self.cur_rng().gen_range(0..=i);
                        bits.swap(i, j);
                    }
                    let mut v = Value::zero(tw as u32);
                    for &b in &bits[..ones] {
                        v.set_bit(b, LogicBit::One);
                    }
                    self.assign_value(&t, &v);
                }
            }
            self.unbind_loop_vars(saved);
            let mut k = idx.len();
            loop {
                if k == 0 {
                    return;
                }
                k -= 1;
                if idx[k] < dims[k].1 {
                    idx[k] += 1;
                    break;
                }
                idx[k] = dims[k].0;
            }
        }
    }

    /// Structural equality of `Ident` / `Index` / `Paren` expression shapes,
    /// ignoring source spans (a synthesized element expression never shares
    /// the constraint's spans).
    pub(super) fn expr_same_ignoring_span(a: &Expression, b: &Expression) -> bool {
        match (&a.kind, &b.kind) {
            (ExprKind::Paren(x), _) => Self::expr_same_ignoring_span(x, b),
            (_, ExprKind::Paren(y)) => Self::expr_same_ignoring_span(a, y),
            (ExprKind::Ident(x), ExprKind::Ident(y)) => {
                x.path.len() == y.path.len()
                    && x.path.iter().zip(y.path.iter()).all(|(p, q)| {
                        p.name.name == q.name.name && p.selects.is_empty() && q.selects.is_empty()
                    })
            }
            (
                ExprKind::Index {
                    expr: xb,
                    index: xi,
                },
                ExprKind::Index {
                    expr: yb,
                    index: yi,
                },
            ) => Self::expr_same_ignoring_span(xb, yb) && Self::expr_same_ignoring_span(xi, yi),
            _ => false,
        }
    }

    pub(super) fn solve_inline_foreach_elem(
        &mut self,
        item: &crate::ast::decl::ConstraintItem,
        arr: &str,
        idx: i64,
        w: u32,
    ) {
        use rand::Rng;
        let elem_name = format!("{}[{}]", arr, idx);
        // Phase 1: gather relational bounds (unsigned element domain).
        let (mut lo, mut hi) = (0i64, if w >= 63 { i64::MAX } else { (1i64 << w) - 1 });
        let mut any_rel = false;
        let mut excl: Vec<i64> = Vec::new();
        self.narrow_inline_foreach_bounds(
            item,
            arr,
            idx,
            &mut lo,
            &mut hi,
            &mut any_rel,
            &mut excl,
        );
        // Phase 2: equality / inside / dist pins.
        let pinned = self.apply_inline_foreach_pins(item, arr, idx, w);
        // Phase 3: repair against the accumulated interval / exclusions.
        if pinned || !(any_rel || !excl.is_empty()) || lo > hi {
            return;
        }
        let cur = self
            .get_signal_value_by_name(&elem_name)
            .and_then(|v| v.to_i64())
            .unwrap_or(0);
        if cur >= lo && cur <= hi && !excl.contains(&cur) {
            return; // already satisfied
        }
        for _ in 0..16 {
            let p = if lo == hi {
                lo
            } else {
                self.cur_rng().gen_range(lo..=hi)
            };
            if !excl.contains(&p) {
                self.set_signal_value_by_name(&elem_name, Value::from_u64(p as u64, w));
                return;
            }
        }
    }

    /// Phase-1 walker for `solve_inline_foreach_elem`: accumulate the
    /// relational bounds (`<`, `<=`, `>`, `>=`) and `!=` exclusions the body
    /// places on element `arr[idx]`. Bound sides are evaluated with the loop
    /// var bound, so index-dependent (`i * 2`) and cross-element
    /// (`arr[i-1] + 1`) bounds use the current solve state.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn narrow_inline_foreach_bounds(
        &mut self,
        item: &crate::ast::decl::ConstraintItem,
        arr: &str,
        idx: i64,
        lo: &mut i64,
        hi: &mut i64,
        any: &mut bool,
        excl: &mut Vec<i64>,
    ) {
        use crate::ast::decl::ConstraintItem as CI;
        match item {
            CI::Expr(e) => {
                if let ExprKind::Binary { op, left, right } = &e.kind {
                    if !matches!(
                        op,
                        BinaryOp::Lt | BinaryOp::Leq | BinaryOp::Gt | BinaryOp::Geq | BinaryOp::Neq
                    ) {
                        return;
                    }
                    let on_l = self.inline_foreach_elem_ref(left, arr, idx);
                    let on_r = self.inline_foreach_elem_ref(right, arr, idx);
                    if on_l == on_r {
                        return;
                    }
                    // Normalize to `arr[i] OP bound` (mirror the op when the
                    // element sits on the right: `5 < arr[i]` ≡ `arr[i] > 5`).
                    let (bound_side, eff) = if on_l {
                        (right, *op)
                    } else {
                        let m = match op {
                            BinaryOp::Lt => BinaryOp::Gt,
                            BinaryOp::Gt => BinaryOp::Lt,
                            BinaryOp::Leq => BinaryOp::Geq,
                            BinaryOp::Geq => BinaryOp::Leq,
                            other => *other,
                        };
                        (left, m)
                    };
                    let b = self.eval_expr(bound_side).to_i64().unwrap_or(0);
                    match eff {
                        BinaryOp::Lt => {
                            *hi = (*hi).min(b.saturating_sub(1));
                            *any = true;
                        }
                        BinaryOp::Leq => {
                            *hi = (*hi).min(b);
                            *any = true;
                        }
                        BinaryOp::Gt => {
                            *lo = (*lo).max(b.saturating_add(1));
                            *any = true;
                        }
                        BinaryOp::Geq => {
                            *lo = (*lo).max(b);
                            *any = true;
                        }
                        BinaryOp::Neq => excl.push(b),
                        _ => {}
                    }
                }
            }
            CI::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                if self.eval_expr(condition).is_true() {
                    self.narrow_inline_foreach_bounds(then_item, arr, idx, lo, hi, any, excl);
                } else if let Some(ei) = else_item {
                    self.narrow_inline_foreach_bounds(ei, arr, idx, lo, hi, any, excl);
                }
            }
            CI::Implication {
                condition,
                constraint,
                ..
            } if self.eval_expr(condition).is_true() => {
                self.narrow_inline_foreach_bounds(constraint, arr, idx, lo, hi, any, excl);
            }
            CI::Block(items) => {
                for it in items {
                    self.narrow_inline_foreach_bounds(it, arr, idx, lo, hi, any, excl);
                }
            }
            CI::Soft(inner) => {
                self.narrow_inline_foreach_bounds(inner, arr, idx, lo, hi, any, excl)
            }
            _ => {}
        }
    }

    /// Phase-2 walker for `solve_inline_foreach_elem`: apply value PINS to
    /// element `arr[idx]` — equality (`arr[i] == i + 5`, issue #28's shape)
    /// and `inside` / `dist` picks. Returns true when a pin was applied (the
    /// relational repair phase then leaves the element alone).
    pub(super) fn apply_inline_foreach_pins(
        &mut self,
        item: &crate::ast::decl::ConstraintItem,
        arr: &str,
        idx: i64,
        w: u32,
    ) -> bool {
        use crate::ast::decl::ConstraintItem as CI;
        let elem_name = format!("{}[{}]", arr, idx);
        match item {
            CI::Expr(e) => match &e.kind {
                ExprKind::Binary {
                    op: BinaryOp::Eq,
                    left,
                    right,
                } => {
                    let on_l = self.inline_foreach_elem_ref(left, arr, idx);
                    let on_r = self.inline_foreach_elem_ref(right, arr, idx);
                    if on_l == on_r {
                        return false;
                    }
                    let bound_side = if on_l { right } else { left };
                    let v = self.eval_expr(bound_side).resize(w);
                    self.set_signal_value_by_name(&elem_name, v);
                    true
                }
                ExprKind::Inside {
                    expr: inner,
                    ranges,
                } => {
                    if self.inline_foreach_elem_ref(inner, arr, idx) {
                        let target = self
                            .get_signal_value_by_name(&elem_name)
                            .unwrap_or_else(|| Value::zero(w));
                        if let Some(v) = self.pick_inside_value(ranges, &target) {
                            self.set_signal_value_by_name(&elem_name, v);
                            return true;
                        }
                    }
                    false
                }
                _ => false,
            },
            CI::Inside {
                expr,
                range,
                is_dist,
                dist_weights,
                ..
            } => {
                if self.inline_foreach_elem_ref(expr, arr, idx) {
                    let picked = if *is_dist && !dist_weights.is_empty() {
                        // Per-element target: no stable key, draw independently.
                        self.pick_dist_value(None, range, dist_weights)
                    } else {
                        let rs: Vec<Expression> = range
                            .iter()
                            .map(|r| match r {
                                ConstraintRange::Value(e) => e.clone(),
                                ConstraintRange::Range { lo, hi } => Expression::new(
                                    ExprKind::Range(
                                        Box::new((*lo).clone()),
                                        Box::new((*hi).clone()),
                                    ),
                                    lo.span,
                                ),
                            })
                            .collect();
                        {
                            let target = self.eval_expr(expr);
                            self.pick_inside_value(&rs, &target)
                        }
                    };
                    if let Some(v) = picked {
                        self.set_signal_value_by_name(&elem_name, v);
                        return true;
                    }
                }
                false
            }
            CI::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                if self.eval_expr(condition).is_true() {
                    self.apply_inline_foreach_pins(then_item, arr, idx, w)
                } else if let Some(ei) = else_item {
                    self.apply_inline_foreach_pins(ei, arr, idx, w)
                } else {
                    false
                }
            }
            CI::Implication {
                condition,
                constraint,
                ..
            } if self.eval_expr(condition).is_true() => {
                self.apply_inline_foreach_pins(constraint, arr, idx, w)
            }
            CI::Block(items) => {
                let mut pinned = false;
                for it in items {
                    if self.apply_inline_foreach_pins(it, arr, idx, w) {
                        pinned = true;
                    }
                }
                pinned
            }
            CI::Soft(inner) => self.apply_inline_foreach_pins(inner, arr, idx, w),
            _ => false,
        }
    }

    /// Post-solve satisfaction check for a `std::randomize(...) with {…}`
    /// constraint set (§18.5.7 / §18.3): true when every MODELED item holds
    /// under the current variable assignment. Soft items (§18.5.14) and
    /// structurally unmodeled shapes never force a retry. Foreach items are
    /// checked per element with the loop var bound — the piece the generic
    /// class-path checker skips via `constraint_unmodeled`.
    pub(super) fn inline_constraints_satisfied(
        &mut self,
        constraints: &[crate::ast::decl::ConstraintItem],
    ) -> bool {
        for c in constraints {
            if !self.inline_constraint_item_ok(c) {
                return false;
            }
        }
        true
    }

    pub(super) fn inline_constraint_item_ok(
        &mut self,
        item: &crate::ast::decl::ConstraintItem,
    ) -> bool {
        use crate::ast::decl::ConstraintItem as CI;
        match item {
            CI::Soft(_) => true, // §18.5.14: soft constraints never fail the check
            // §18.5.5: the scope path DOES model `unique {arr}` (see
            // `enforce_inline_unique`), so check it rather than treating it as
            // unmodeled — otherwise a residual collision never triggered a
            // retry and the call silently returned a non-unique array.
            CI::Unique { exprs, .. } => {
                for e in exprs {
                    let Some(arr) = self.array_operand_name(e) else {
                        continue;
                    };
                    let size = self.get_queue_size(&arr);
                    let mut seen: HashSet<u64> = HashSet::default();
                    for i in 0..size {
                        let Some(v) = self
                            .get_signal_value_by_name(&format!("{}[{}]", arr, i))
                            .and_then(|v| v.to_u64())
                        else {
                            continue;
                        };
                        if !seen.insert(v) {
                            return false;
                        }
                    }
                }
                true
            }
            CI::Foreach {
                array,
                vars,
                item: body,
                ..
            } => {
                // §18.12: the scope's collections are already sized, so a
                // `size()`/`sum()`/`min()`… term is an ordinary value here —
                // only `solve…before` carries no condition.
                if matches!(**body, CI::Solve { .. }) {
                    return true;
                }
                let base = match &array.kind {
                    ExprKind::Index { expr: b, .. } => b.as_ref(),
                    _ => array,
                };
                // Any iterable shape, not only the 1-D unpacked arrays
                // `array_operand_name` knows: a 2-D/N-D unpacked array and a
                // packed multi-dimensional VARIABLE used to fall out here as
                // "not an array" and the whole foreach passed vacuously.
                let Some(arr_name) = self.foreach_check_operand_name(base) else {
                    return true;
                };
                // Bind EVERY loop variable, over unpacked dimensions first
                // and then the packed ones (§12.7.3). Binding only the first
                // used to leave `j` in `d[i][j]` unbound (x) for a 2-D
                // target, so the body compared against x and the constraint
                // passed vacuously: `std::randomize(d) with { foreach
                // (d[i, j]) d[i][j] < 100; }` returned 1 with every element 0.
                let dims = self.foreach_check_dims(&arr_name, vars.len());
                let idx_vars: Vec<Option<String>> = vars
                    .iter()
                    .map(|v| v.as_ref().map(|id| id.name.clone()))
                    .collect();
                let mut idx: Vec<i64> = dims.iter().map(|&(lo, _)| lo).collect();
                if dims.iter().any(|&(lo, hi)| hi < lo) {
                    return true;
                }
                loop {
                    let mut binds: Vec<(String, Value)> = Vec::new();
                    for (k, iv) in idx_vars.iter().enumerate() {
                        if let (Some(iv), Some(&i)) = (iv, idx.get(k)) {
                            binds.push((iv.clone(), Self::signed_loop_val(i)));
                        }
                    }
                    let saved = self.bind_loop_vars(binds);
                    // The body may itself be a `foreach` over the next
                    // dimension (`foreach (a[i]) foreach (a[i][j]) …`, #519)
                    // or a block of them: check it through this function so
                    // its loop variables are bound too; the member-foreach
                    // check has no object here and passed it vacuously.
                    let ok = self.inline_constraint_item_ok(body);
                    self.unbind_loop_vars(saved);
                    if !ok {
                        return false;
                    }
                    // odometer advance over the bound dimensions
                    let mut k = idx.len();
                    loop {
                        if k == 0 {
                            return true;
                        }
                        k -= 1;
                        if idx[k] < dims[k].1 {
                            idx[k] += 1;
                            break;
                        }
                        idx[k] = dims[k].0;
                    }
                }
            }
            // Recurse so nested foreach items get the per-element check.
            CI::Block(items) => {
                for it in items {
                    if !self.inline_constraint_item_ok(it) {
                        return false;
                    }
                }
                true
            }
            CI::Solve { .. } => true,
            _ => self.check_constraint_item_impl(item),
        }
    }

    /// §18.3 constraint expressions may wrap operands in casts. Decompose a
    /// constraint operand as an AFFINE function `scale * <name> + offset` of
    /// the randomize target `name`, seeing through §6.24.1 size casts
    /// (`32'(e)`, lowered by the parser to `$__xz_size_cast`), §6.24 type
    /// casts (lowered to pass-through `Paren`) and `$signed`/`$unsigned`.
    /// Modeling the cast context in i64 arithmetic matches §11.6.1: inside
    /// `32'(A * B)` the operands are expanded to the cast size BEFORE the
    /// multiply, so the solver must reason in the widened domain instead of
    /// giving up on the compound operand (issue #29).
    ///
    /// Subexpressions referencing NO randomize target evaluate to constants;
    /// returns None when the shape isn't affine in `name` (or pulls in a
    /// different, still-unsolved target).
    pub(super) fn affine_in_target(
        &mut self,
        e: &Expression,
        name: &str,
        targets: &[(String, Expression)],
    ) -> Option<(i64, i64)> {
        if self.constraint_operand_name(e).as_deref() == Some(name) {
            return Some((1, 0));
        }
        // A subexpression with no target references is a plain constant.
        if let Some(c) = self.affine_const_if_no_targets(e, targets) {
            return Some(c);
        }
        match &e.kind {
            ExprKind::Paren(inner) => self.affine_in_target(inner, name, targets),
            // §6.24.1 size cast / sign casts — transparent for solving (the
            // cast context WIDENS the operands per §11.6.1; i64 math models
            // exactly that).
            ExprKind::SystemCall { name: fname, args } => match fname.as_str() {
                "$__xz_size_cast" if args.len() == 2 => {
                    self.affine_in_target(&args[1], name, targets)
                }
                "$signed" | "$unsigned" if args.len() == 1 => {
                    self.affine_in_target(&args[0], name, targets)
                }
                _ => None,
            },
            ExprKind::Binary { op, left, right }
                if matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul) =>
            {
                let (ls, lc) = self.affine_in_target(left, name, targets)?;
                let (rs, rc) = self.affine_in_target(right, name, targets)?;
                match op {
                    BinaryOp::Add => Some((ls.checked_add(rs)?, lc.checked_add(rc)?)),
                    BinaryOp::Sub => Some((ls.checked_sub(rs)?, lc.checked_sub(rc)?)),
                    BinaryOp::Mul => {
                        if ls == 0 {
                            Some((lc.checked_mul(rs)?, lc.checked_mul(rc)?))
                        } else if rs == 0 {
                            Some((rc.checked_mul(ls)?, rc.checked_mul(lc)?))
                        } else {
                            None // quadratic in the target — not affine
                        }
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Constant leg of `affine_in_target`: if `e` references none of the
    /// randomize targets, evaluate it and return `(0, value)`.
    pub(super) fn affine_const_if_no_targets(
        &mut self,
        e: &Expression,
        targets: &[(String, Expression)],
    ) -> Option<(i64, i64)> {
        let mut ids = HashSet::default();
        self.collect_expr_idents(e, &mut ids);
        if targets.iter().any(|(n, _)| ids.contains(n)) {
            return None;
        }
        let v = self.eval_expr(e);
        // §11.8.1: in an unsigned comparison context read the bound by its
        // unsigned value (`to_i64` sign-extends, turning e.g. 0xFFFFFFFA into
        // -6). Only take the unsigned reading when it stays representable in the
        // i64 bound domain; otherwise fall back to the signed reading.
        if self.constraint_cmp_unsigned {
            if let Some(u) = v.to_u64() {
                if u <= i64::MAX as u64 {
                    return Some((0, u as i64));
                }
            }
        }
        Some((0, v.to_i64()?))
    }

    // -----------------------------------------------------------------------
    // §18.5.12 COUPLED multi-variable affine constraint solve (issue #4).
    //
    // `narrow_relational_bounds` narrows each rand variable's [lo,hi] interval
    // INDEPENDENTLY: a constraint that references two targets (`A + B < 1000`)
    // narrows NOTHING, so both are drawn full-range and the coupled constraint
    // is essentially never satisfied. The routines below add a SEQUENTIAL /
    // propagation solve: order the coupled variables, assign them one at a
    // time treating already-assigned ones as constants and still-unassigned
    // ones at the domain extreme that keeps a feasible completion, and draw
    // each uniformly from its narrowed range. This handles `A+B<K`, `A<B`,
    // `A+B==C`, `A-B==0` and any `sum(ci*Vi) REL K`. Nonlinear (`A*B`) and
    // non-affine terms are left to the existing best-effort path.
    // -----------------------------------------------------------------------

    /// §18.3 flip a relational operator (used when dividing an inequality by a
    /// negative coefficient).
    pub(super) fn flip_op(op: BinaryOp) -> BinaryOp {
        match op {
            BinaryOp::Lt => BinaryOp::Gt,
            BinaryOp::Gt => BinaryOp::Lt,
            BinaryOp::Leq => BinaryOp::Geq,
            BinaryOp::Geq => BinaryOp::Leq,
            o => o,
        }
    }

    /// Integer bounds on `vi` for `a*vi OP R` with `a > 0`, where `R` ranges
    /// over `[rmin, rmax]`. We keep `vi` feasible for the BEST completion (the
    /// most permissive endpoint of R), so a later variable can always realise
    /// the required R. Returns `(lo, hi)` narrowings (either may be `None`).
    pub(super) fn coupled_narrow(
        a: i128,
        op: BinaryOp,
        rmin: i128,
        rmax: i128,
    ) -> (Option<i128>, Option<i128>) {
        let floor_div = |x: i128, d: i128| x.div_euclid(d);
        let ceil_div = |x: i128, d: i128| -((-x).div_euclid(d));
        match op {
            // a*vi < R  → feasible if vi < rmax  → vi <= floor((rmax-1)/a)
            BinaryOp::Lt => (None, Some(floor_div(rmax - 1, a))),
            // a*vi <= R → vi <= floor(rmax/a)
            BinaryOp::Leq => (None, Some(floor_div(rmax, a))),
            // a*vi > R  → vi > rmin  → vi >= floor(rmin/a)+1
            BinaryOp::Gt => (Some(floor_div(rmin, a) + 1), None),
            // a*vi >= R → vi >= ceil(rmin/a)
            BinaryOp::Geq => (Some(ceil_div(rmin, a)), None),
            // a*vi == R → vi ∈ [ceil(rmin/a), floor(rmax/a)]
            BinaryOp::Eq => (Some(ceil_div(rmin, a)), Some(floor_div(rmax, a))),
            // != cannot narrow an interval — the acceptance check enforces it.
            _ => (None, None),
        }
    }

    /// Decompose `e` into `(coeffs, const)` where `coeffs[i]` is the integer
    /// coefficient of target column `i` (`names[last_segment] = i`) and `const`
    /// is the target-free remainder. `None` when `e` is not affine in the
    /// targets (a product of two targets, an unmodeled call, …). Mirrors
    /// `affine_in_target` but tracks ALL target columns at once.
    pub(super) fn affine_cols(
        &mut self,
        e: &Expression,
        names: &HashMap<String, usize>,
        ncols: usize,
        members: bool,
    ) -> Option<(Vec<i128>, i128)> {
        // A bare target leaf (matched by last path segment, as the constraint
        // solver matches rand props / scope-randomize targets everywhere).
        if let ExprKind::Ident(h) = &e.kind {
            if let Some(seg) = h.path.last() {
                if let Some(&idx) = names.get(&seg.name.name) {
                    let mut c = vec![0i128; ncols];
                    c[idx] = 1;
                    return Some((c, 0));
                }
            }
        }
        // A scope target named through a handle (`std::randomize(obj.m)`,
        // `members`), keyed by `constraint_operand_name` like the targets.
        if members && matches!(e.kind, ExprKind::MemberAccess { .. }) {
            if let Some(&idx) = self.constraint_operand_name(e).and_then(|n| names.get(&n)) {
                let mut c = vec![0i128; ncols];
                c[idx] = 1;
                return Some((c, 0));
            }
        }
        // A subexpression naming no target is a plain constant.
        let mut ids = HashSet::default();
        self.collect_expr_idents(e, &mut ids);
        if members {
            self.collect_member_operand_names(e, &mut ids);
        }
        if !names.keys().any(|n| ids.contains(n)) {
            let v = self.eval_expr(e);
            let k: i128 = if self.constraint_cmp_unsigned {
                v.to_u128() as i128
            } else {
                v.to_i64()? as i128
            };
            return Some((vec![0i128; ncols], k));
        }
        match &e.kind {
            ExprKind::Paren(inner) => self.affine_cols(inner, names, ncols, members),
            ExprKind::SystemCall { name, args } => match name.as_str() {
                "$__xz_size_cast" if args.len() == 2 => {
                    self.affine_cols(&args[1], names, ncols, members)
                }
                "$signed" | "$unsigned" if args.len() == 1 => {
                    self.affine_cols(&args[0], names, ncols, members)
                }
                _ => None,
            },
            ExprKind::Binary { op, left, right }
                if matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul) =>
            {
                let (lc, lk) = self.affine_cols(left, names, ncols, members)?;
                let (rc, rk) = self.affine_cols(right, names, ncols, members)?;
                let mut c = vec![0i128; ncols];
                match op {
                    BinaryOp::Add => {
                        for i in 0..ncols {
                            c[i] = lc[i].checked_add(rc[i])?;
                        }
                        Some((c, lk.checked_add(rk)?))
                    }
                    BinaryOp::Sub => {
                        for i in 0..ncols {
                            c[i] = lc[i].checked_sub(rc[i])?;
                        }
                        Some((c, lk.checked_sub(rk)?))
                    }
                    BinaryOp::Mul => {
                        let l_const = lc.iter().all(|&x| x == 0);
                        let r_const = rc.iter().all(|&x| x == 0);
                        if r_const {
                            for i in 0..ncols {
                                c[i] = lc[i].checked_mul(rk)?;
                            }
                            Some((c, lk.checked_mul(rk)?))
                        } else if l_const {
                            for i in 0..ncols {
                                c[i] = rc[i].checked_mul(lk)?;
                            }
                            Some((c, lk.checked_mul(rk)?))
                        } else {
                            None // quadratic in the targets — not affine
                        }
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// A top-level relational/equality expression as a normalized affine
    /// constraint `sum(coeffs·V) OP c`. `None` when either side is non-affine
    /// or the targets cancel out entirely.
    pub(super) fn affine_binary_con(
        &mut self,
        e: &Expression,
        names: &HashMap<String, usize>,
        ncols: usize,
        obj_handle: Option<usize>,
    ) -> Option<(Vec<i128>, i128, BinaryOp)> {
        let ExprKind::Binary { op, left, right } = &e.kind else {
            return None;
        };
        if !matches!(
            op,
            BinaryOp::Lt
                | BinaryOp::Leq
                | BinaryOp::Gt
                | BinaryOp::Geq
                | BinaryOp::Eq
                | BinaryOp::Neq
        ) {
            return None;
        }
        // §18.5: a rand dynamic array's `.size()` is not a settled numeric
        // operand during the coupled scalar solve (the array is only sized
        // later) — `arr.size() == N` would pin `N` to the current empty size
        // (0). The array-sizing solver owns that coupling. Prefer the
        // randomized object's handle (from the caller) over the this-frame
        // top, which in a UVM run points at the TEST harness (uvm_root), not
        // the payload object, so the array lookup would fail and the guard
        // would let `arr.size()==N` become a spurious `N == 0` row that
        // contradicts `N == <non-zero>`.
        let h = obj_handle.or_else(|| self.this_stack.iter().flatten().next().copied());
        if let Some(h) = h {
            if self.is_rand_dyn_size_expr(h, left) || self.is_rand_dyn_size_expr(h, right) {
                return None;
            }
        }
        // Scope randomization (no object) may name targets through handles.
        let members = obj_handle.is_none();
        let (lc, lk) = self.affine_cols(left, names, ncols, members)?;
        let (rc, rk) = self.affine_cols(right, names, ncols, members)?;
        let mut coeffs = vec![0i128; ncols];
        let mut any = false;
        for i in 0..ncols {
            coeffs[i] = lc[i].checked_sub(rc[i])?;
            if coeffs[i] != 0 {
                any = true;
            }
        }
        if !any {
            return None; // no target term after cancellation
        }
        let c = rk.checked_sub(lk)?;
        Some((coeffs, c, *op))
    }

    /// Collect every affine relational/equality constraint reachable in `item`
    /// (through `Block`). `soft` items are skipped — they are best-effort and
    /// must never over-constrain the coupled solve.
    pub(super) fn collect_affine_cons(
        &mut self,
        item: &crate::ast::decl::ConstraintItem,
        names: &HashMap<String, usize>,
        ncols: usize,
        obj_handle: Option<usize>,
        out: &mut Vec<(Vec<i128>, i128, BinaryOp)>,
    ) {
        use crate::ast::decl::ConstraintItem as CI;
        match item {
            CI::Expr(e) => {
                if let Some(con) = self.affine_binary_con(e, names, ncols, obj_handle) {
                    out.push(con);
                }
            }
            CI::Block(items) => {
                for it in items {
                    self.collect_affine_cons(it, names, ncols, obj_handle, out);
                }
            }
            _ => {}
        }
    }

    /// Every identifier (by last path segment) referenced anywhere in a
    /// constraint item, including conditions, ranges and nested items.
    pub(super) fn collect_item_idents(
        &self,
        item: &crate::ast::decl::ConstraintItem,
        out: &mut HashSet<String>,
    ) {
        use crate::ast::decl::ConstraintItem as CI;
        use crate::ast::decl::ConstraintRange as CR;
        match item {
            CI::Expr(e) => self.collect_expr_idents(e, out),
            CI::Inside { expr, range, .. } => {
                self.collect_expr_idents(expr, out);
                for r in range {
                    match r {
                        CR::Value(e) => self.collect_expr_idents(e, out),
                        CR::Range { lo, hi } => {
                            self.collect_expr_idents(lo, out);
                            self.collect_expr_idents(hi, out);
                        }
                    }
                }
            }
            CI::Implication {
                condition,
                constraint,
                ..
            } => {
                self.collect_expr_idents(condition, out);
                self.collect_item_idents(constraint, out);
            }
            CI::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                self.collect_expr_idents(condition, out);
                self.collect_item_idents(then_item, out);
                if let Some(ei) = else_item {
                    self.collect_item_idents(ei, out);
                }
            }
            CI::Foreach { array, item, .. } => {
                self.collect_expr_idents(array, out);
                self.collect_item_idents(item, out);
            }
            CI::Soft(inner) => self.collect_item_idents(inner, out),
            CI::Block(items) => {
                for it in items {
                    self.collect_item_idents(it, out);
                }
            }
            CI::Unique { exprs, .. } => {
                for e in exprs {
                    self.collect_expr_idents(e, out);
                }
            }
            CI::Solve { .. } => {}
        }
    }

    /// True when `item` is safe for the coupled solve to own the referenced
    /// targets — i.e. it does NOT constrain an owned variable through a
    /// construct the affine solve cannot see (`inside`, `dist`, implication,
    /// `foreach`, a non-affine `==`, …). If it does, we abort the coupled
    /// solve and let the existing best-effort pipeline handle the whole set,
    /// so we never regress a case that already worked.
    pub(super) fn item_ok_for_coupled(
        &mut self,
        item: &crate::ast::decl::ConstraintItem,
        names: &HashMap<String, usize>,
        ncols: usize,
        owned: &HashSet<usize>,
        obj_handle: Option<usize>,
    ) -> bool {
        use crate::ast::decl::ConstraintItem as CI;
        let owned_count = |me: &Self, it: &CI| -> usize {
            let mut ids = HashSet::default();
            me.collect_item_idents(it, &mut ids);
            ids.iter()
                .filter(|n| names.get(n.as_str()).is_some_and(|i| owned.contains(i)))
                .count()
        };
        let touches_owned = |me: &Self, it: &CI| -> bool { owned_count(me, it) > 0 };
        match item {
            CI::Expr(e) => {
                // A `.size()` coupling to a rand DYNAMIC array (`arr.size() ==
                // N`) is sized to the array's length separately; it must not
                // block the coupled scalar solve even though it touches an
                // owned variable through the equality.
                let h = obj_handle.or_else(|| self.this_stack.iter().flatten().next().copied());
                if let Some(h) = h {
                    if self.expr_involves_dyn_size(h, e) {
                        return true;
                    }
                }
                if self
                    .affine_binary_con(e, names, ncols, obj_handle)
                    .is_some()
                {
                    return true; // fully modeled affine constraint
                }
                // A non-affine predicate that constrains at most ONE owned
                // variable (`m % 4 == 0`, `m inside {…}`, a single-var
                // inequality) cannot take part in the affine coupling, but it
                // need not abort the whole solve: the assigned value is still
                // enforced by the trial acceptance check, which retries a
                // fresh draw if violated. Requiring it to pass here would
                // abandon coupling for the whole set whenever any scalar has a
                // modulus/inside guard.
                owned_count(self, item) <= 1
            }
            CI::Block(items) => items
                .iter()
                .all(|it| self.item_ok_for_coupled(it, names, ncols, owned, obj_handle)),
            // A soft constraint can always be dropped, and `solve` is only an
            // ordering hint — neither blocks the coupled solve.
            CI::Soft(_) | CI::Solve { .. } => true,
            other => !touches_owned(self, other),
        }
    }

    /// §18.5.12 — sequential coupled affine solve. `var_names[i]` /
    /// `var_dom[i]` describe target column `i` (name matched by last path
    /// segment; `[lo,hi]` its declared integer domain). Returns `(col, value)`
    /// pairs for the COUPLED columns it assigned, or `None` when there is no
    /// coupling (backward-compatible: caller uses its existing path), when an
    /// owned variable is also constrained by an unmodeled construct, or when
    /// the chosen order dead-ends (caller retries with a fresh seed/order).
    pub(super) fn solve_coupled_affine(
        &mut self,
        items: &[crate::ast::decl::ConstraintItem],
        var_names: &[String],
        var_dom: &[(i128, i128)],
        attempt: usize,
        obj_handle: Option<usize>,
    ) -> Option<Vec<(usize, i128)>> {
        use rand::Rng;
        let ncols = var_names.len();
        if ncols < 2 {
            return None;
        }
        let mut names: HashMap<String, usize> = HashMap::default();
        for (i, n) in var_names.iter().enumerate() {
            names.insert(n.clone(), i);
        }
        // Extract the affine relational/equality constraints.
        let mut cons: Vec<(Vec<i128>, i128, BinaryOp)> = Vec::new();
        for it in items {
            self.collect_affine_cons(it, &names, ncols, obj_handle, &mut cons);
        }
        // A variable is COUPLED if it shares a constraint with another target.
        let mut owned: HashSet<usize> = HashSet::default();
        for (coeffs, _, _) in &cons {
            let nz: Vec<usize> = (0..ncols).filter(|&i| coeffs[i] != 0).collect();
            if nz.len() >= 2 {
                for i in nz {
                    owned.insert(i);
                }
            }
        }
        if owned.is_empty() {
            return None; // no coupling — nothing to do here
        }
        // Never take ownership of a variable that an unmodeled construct also
        // constrains (preserve existing behavior for such mixed sets).
        for it in items {
            if !self.item_ok_for_coupled(it, &names, ncols, &owned, obj_handle) {
                return None;
            }
        }
        // Keep only constraints wholly over owned columns (single-variable
        // bounds on an owned var are included and tighten its domain).
        cons.retain(|(coeffs, _, _)| (0..ncols).all(|i| coeffs[i] == 0 || owned.contains(&i)));

        // Pre-narrow each owned var by its SINGLE-variable bounds first, so the
        // extreme used for a still-unassigned var already respects that var's
        // own constraints (makes the solve order far less brittle).
        let mut dom0 = var_dom.to_vec();
        for &col in owned.iter() {
            let (mut lo, mut hi) = dom0[col];
            for (coeffs, c, op) in &cons {
                if coeffs[col] == 0 {
                    continue;
                }
                if (0..ncols).any(|i| i != col && coeffs[i] != 0) {
                    continue; // not a single-variable bound
                }
                let ci = coeffs[col];
                let (a, eop, rmn, rmx) = if ci < 0 {
                    (-ci, Self::flip_op(*op), -*c, -*c)
                } else {
                    (ci, *op, *c, *c)
                };
                let (ol, oh) = Self::coupled_narrow(a, eop, rmn, rmx);
                if let Some(l) = ol {
                    lo = lo.max(l);
                }
                if let Some(h) = oh {
                    hi = hi.min(h);
                }
            }
            dom0[col] = (lo, hi);
        }

        // Solve order: declaration order first; reshuffle on retries so a
        // dead-ending order is escaped with a fresh seed.
        let mut order: Vec<usize> = {
            let mut v: Vec<usize> = owned.iter().copied().collect();
            v.sort_unstable();
            v
        };
        if attempt > 0 && order.len() > 1 {
            for i in (1..order.len()).rev() {
                let j = self.cur_rng().gen_range(0..=i);
                order.swap(i, j);
            }
        }

        // Sequential assignment.
        let mut assigned: HashMap<usize, i128> = HashMap::default();
        for &col in &order {
            let (mut lo, mut hi) = dom0[col];
            for (coeffs, c, op) in &cons {
                let ci = coeffs[col];
                if ci == 0 {
                    continue;
                }
                let mut fixed = 0i128;
                let mut sum_min = 0i128; // min of sum(cj*vj) over unassigned j!=col
                let mut sum_max = 0i128;
                for j in 0..ncols {
                    let cj = coeffs[j];
                    if j == col || cj == 0 {
                        continue;
                    }
                    if let Some(&vj) = assigned.get(&j) {
                        fixed += cj * vj;
                    } else {
                        let (lj, hj) = dom0[j];
                        if cj > 0 {
                            sum_min += cj * lj;
                            sum_max += cj * hj;
                        } else {
                            sum_min += cj * hj;
                            sum_max += cj * lj;
                        }
                    }
                }
                let rmax = *c - fixed - sum_min;
                let rmin = *c - fixed - sum_max;
                let (a, eop, rmn, rmx) = if ci < 0 {
                    (-ci, Self::flip_op(*op), -rmax, -rmin)
                } else {
                    (ci, *op, rmin, rmax)
                };
                let (ol, oh) = Self::coupled_narrow(a, eop, rmn, rmx);
                if let Some(l) = ol {
                    lo = lo.max(l);
                }
                if let Some(h) = oh {
                    hi = hi.min(h);
                }
            }
            if lo > hi {
                return None; // this order dead-ended — caller retries
            }
            let pick = if lo == hi {
                lo
            } else {
                self.cur_rng().gen_range(lo..=hi)
            };
            assigned.insert(col, pick);
        }
        Some(assigned.into_iter().collect())
    }

    /// Match `expr` (a constraint operand) to one of the randomize targets,
    /// returning its lvalue expression.
    /// Canonical name of a constraint operand that is a scalar variable (`x`)
    /// or a struct member (`pkt.field`), matching how scope-randomize field
    /// targets are named (see `expand_struct_target`). A struct member parses
    /// as a `MemberAccess`, so without this an inline constraint like
    /// `pkt.data < 100` would never bind to the `pkt.data` field target.
    /// Returns `None` for anything else.
    pub(super) fn constraint_operand_name(&self, e: &Expression) -> Option<String> {
        match &e.kind {
            ExprKind::Ident(h) => Some(self.resolve_hier_name(h).into_owned()),
            ExprKind::MemberAccess { expr, member } => {
                if let ExprKind::Ident(bh) = &expr.kind {
                    if bh.path.len() == 1 {
                        let mut fh = bh.clone();
                        fh.path.push(crate::ast::expr::HierPathSegment {
                            name: crate::ast::Identifier {
                                name: member.name.clone(),
                                span: member.span,
                            },
                            selects: Vec::new(),
                        });
                        fh.cached_signal_id = std::cell::Cell::new(None);
                        fh.cached_resolved_name = std::cell::OnceCell::new();
                        return Some(self.resolve_hier_name(&fh).into_owned());
                    }
                }
                // A deeper select chain (`o.f.value`): the name of the same
                // path written as one dotted identifier.
                if matches!(expr.kind, ExprKind::MemberAccess { .. }) {
                    let segs = Self::flatten_member_path(e)?;
                    let fh = crate::ast::expr::HierarchicalIdentifier {
                        root: None,
                        path: segs
                            .into_iter()
                            .map(|n| crate::ast::expr::HierPathSegment {
                                name: crate::ast::Identifier {
                                    name: n,
                                    span: member.span,
                                },
                                selects: Vec::new(),
                            })
                            .collect(),
                        span: e.span,
                        cached_signal_id: std::cell::Cell::new(None),
                        cached_resolved_name: std::cell::OnceCell::new(),
                    };
                    return Some(self.resolve_hier_name(&fh).into_owned());
                }
                None
            }
            _ => None,
        }
    }

    pub(super) fn target_for(
        &mut self,
        expr: &Expression,
        targets: &[(String, Expression)],
    ) -> Option<Expression> {
        let n = self.constraint_operand_name(expr)?;
        for (name, lv) in targets {
            if *name == n {
                return Some(lv.clone());
            }
        }
        None
    }

    /// LRM §18.5.4 weighted distribution picker.
    ///
    /// Each `(range[i], weights[i])` pair is one dist ITEM with a probability
    /// mass:
    /// - `Each(w)` (the `:=` operator) — every individual value the item
    ///   covers gets weight `w`, so a `[lo:hi]` item's TOTAL mass is
    ///   `w * (hi-lo+1)` (LRM: "`[100:102] := 1, 200 := 2` ⇒ ratio
    ///   1-1-1-2"). A bare value is one outcome of weight `w`.
    /// - `Total(w)` (the `:/` operator) — `w` is the weight of the item as a
    ///   WHOLE, so each of its `n` values gets `w / n` and the item's total
    ///   mass is just `w`.
    /// - `None` — defaults to `Each(1)` per the LRM.
    /// - A weight of ZERO means the item can never be chosen.
    ///
    /// Under both operators the values WITHIN one item are equiprobable, so a
    /// draw is: choose an item in proportion to its mass, then sample
    /// uniformly inside it.
    ///
    /// The item choice is DEALT from a per-(object, variable) schedule rather
    /// than sampled independently: the schedule holds each item exactly as
    /// often as its share of the total mass and spreads the copies evenly
    /// (smooth weighted round-robin, then locally jittered). Every value keeps
    /// exactly the marginal probability §18.5.4 gives it, but the empirical
    /// histogram tracks the declared distribution over tens of calls instead of
    /// needing thousands to average out sampling noise — which is what
    /// distribution checks in testbenches actually measure. §18.10: the weights
    /// are expressions re-evaluated on every call, and each set of weights
    /// keeps a schedule of its own. Weights too fine for a schedule
    /// (`[0:99999] := 1, 100000 := 100`) are drawn independently.
    pub(super) fn pick_dist_value(
        &mut self,
        key: Option<(usize, String)>,
        ranges: &[ConstraintRange],
        weights: &[Option<crate::ast::decl::DistWeight>],
    ) -> Option<Value> {
        use crate::ast::decl::DistWeight;
        use rand::Rng;
        // Fixed-point scaler: keeps `Total(w)` masses exact for small w.
        const SCALER: u128 = 1024;

        #[derive(Clone)]
        enum DistItem {
            One(Value),
            /// lo, hi, width
            Span(i64, i64, u32),
        }
        let mut items: Vec<(DistItem, u128)> = Vec::new();
        for (i, r) in ranges.iter().enumerate() {
            let w_kind = weights.get(i).and_then(|w| w.as_ref());
            let w = match w_kind {
                Some(DistWeight::Each(e)) | Some(DistWeight::Total(e)) => {
                    self.eval_expr(e).to_u64().unwrap_or(1)
                }
                None => 1,
            };
            match r {
                ConstraintRange::Value(e) => {
                    let v = self.eval_expr(e);
                    items.push((DistItem::One(v), w as u128 * SCALER));
                }
                ConstraintRange::Range { lo, hi } => {
                    let lov = self.eval_expr(lo);
                    let hiv = self.eval_expr(hi);
                    // An item must be as wide as its bounds: `[64'h1_0000_0000 :
                    // 64'h1_0000_00FF]` truncated to 32 bits and produced 0.
                    let bw = lov.width.max(hiv.width).max(32);
                    let l = lov.to_i64().unwrap_or(0);
                    let h = hiv.to_i64().unwrap_or(l);
                    if h < l {
                        continue;
                    }
                    let span = (h as i128 - l as i128 + 1) as u128;
                    let mass = match w_kind {
                        // `:/` — w is the weight of the whole item.
                        Some(DistWeight::Total(_)) => w as u128 * SCALER,
                        // `:=` (and the default) — w is the weight of EVERY
                        // value in the item.
                        _ => (w as u128 * SCALER).saturating_mul(span),
                    };
                    items.push((DistItem::Span(l, h, bw), mass));
                }
            }
        }
        if items.is_empty() {
            return None;
        }
        let masses: Vec<u128> = items.iter().map(|(_, m)| *m).collect();
        let total: u128 = masses.iter().fold(0u128, |a, m| a.saturating_add(*m));
        // Every weight is 0 — the constraint admits no value; keep the first
        // item so the caller still makes progress.
        if total == 0 {
            return Some(match &items[0].0 {
                DistItem::One(v) => v.clone(),
                DistItem::Span(l, _, bw) => Value::from_u64(*l as u64, *bw),
            });
        }

        let dealt = match key {
            Some(k) => self.deal_dist_item(k, &masses),
            None => None,
        };
        let idx = match dealt {
            Some(i) => i,
            None => {
                // No stable identity for the target (a foreach element, say)
                // or weights too fine to schedule: an independent draw.
                let mut draw = match u64::try_from(total) {
                    Ok(t) => (self.cur_rng().r#gen::<u64>() % t) as u128,
                    Err(_) => self.cur_rng().gen_range(0..total),
                };
                let mut pick = masses.len() - 1;
                for (i, m) in masses.iter().enumerate() {
                    if draw < *m {
                        pick = i;
                        break;
                    }
                    draw -= *m;
                }
                pick
            }
        };

        match items[idx].0.clone() {
            DistItem::One(v) => Some(v),
            DistItem::Span(l, h, bw) => {
                // §18.5.4: the values inside one item are equiprobable under
                // both `:=` and `:/`.
                let v = if h > l {
                    self.cur_rng().gen_range(l..=h)
                } else {
                    l
                };
                Some(Value::from_u64(v as u64, bw))
            }
        }
    }

    /// §18.5.4/§18.10 — deal the next dist ITEM index for `key` from its
    /// proportional schedule, rebuilding the schedule when it is exhausted.
    /// Each set of resolved weights keeps its own schedule: the weights are
    /// re-evaluated per call (§18.10 dynamic weights), and a variable whose
    /// dist differs per `if` branch must not restart one branch's schedule
    /// every time the other branch deals — a fresh schedule always opens on
    /// its heaviest item. See `pick_dist_value` for why the item choice is
    /// scheduled rather than drawn independently.
    pub(super) fn deal_dist_item(
        &mut self,
        key: (usize, String),
        masses: &[u128],
    ) -> Option<usize> {
        /// Weight sets remembered per variable.
        const MAX_DECKS: usize = 8;
        let mut sig: u64 = 0xcbf2_9ce4_8422_2325;
        for m in masses {
            sig ^= *m as u64 ^ (*m >> 64) as u64;
            sig = sig.wrapping_mul(0x0000_0100_0000_01b3);
        }
        let live = self
            .dist_decks
            .get(&key)
            .and_then(|ds| ds.iter().find(|d| d.0 == sig))
            .is_some_and(|(_, deck, pos)| *pos < deck.len());
        if !live {
            let deck = self.build_dist_deck(masses)?;
            let decks = self.dist_decks.entry(key.clone()).or_default();
            decks.retain(|d| d.0 != sig);
            if decks.len() >= MAX_DECKS {
                decks.remove(0);
            }
            decks.push((sig, deck, 0));
        }
        let decks = self.dist_decks.get_mut(&key).unwrap();
        let (_, deck, pos) = decks.iter_mut().find(|d| d.0 == sig).unwrap();
        if deck.is_empty() {
            return Some(0);
        }
        let idx = deck[*pos];
        *pos += 1;
        Some(idx)
    }

    /// §18.5.4 — build a schedule holding each dist item in proportion to its
    /// mass, with the copies spread out (smooth weighted round-robin: repeatedly
    /// hand the slot to the item with the largest accumulated credit). Adjacent
    /// entries are then randomly transposed so the dealt sequence is not a rigid
    /// repeating pattern while any window of it stays proportional.
    pub(super) fn build_dist_deck(&mut self, masses: &[u128]) -> Option<Vec<usize>> {
        use rand::Rng;
        /// Longest schedule dealt; finer weights are drawn independently.
        const MAX_DECK: u128 = 4096;
        fn gcd(a: u128, b: u128) -> u128 {
            if b == 0 { a } else { gcd(b, a % b) }
        }
        let g = masses
            .iter()
            .copied()
            .filter(|m| *m > 0)
            .fold(0u128, gcd)
            .max(1);
        let counts: Vec<u128> = masses.iter().map(|m| m / g).collect();
        let len = counts.iter().fold(0u128, |a, c| a.saturating_add(*c));
        if len > MAX_DECK {
            return None;
        }
        if len == 0 {
            return Some(Vec::new());
        }
        let n = counts.len();
        let mut credit: Vec<i64> = vec![0; n];
        let mut deck: Vec<usize> = Vec::with_capacity(len as usize);
        for _ in 0..len {
            let mut best = usize::MAX;
            let mut best_credit = i64::MIN;
            for i in 0..n {
                if counts[i] == 0 {
                    continue; // weight 0 — never scheduled
                }
                credit[i] += counts[i] as i64;
                if credit[i] > best_credit {
                    best_credit = credit[i];
                    best = i;
                }
            }
            if best == usize::MAX {
                break;
            }
            credit[best] -= len as i64;
            deck.push(best);
        }
        for i in 1..deck.len() {
            if self.cur_rng().r#gen::<bool>() {
                deck.swap(i - 1, i);
            }
        }
        Some(deck)
    }

    pub(super) fn pick_inside_value(
        &mut self,
        ranges: &[Expression],
        target: &Value,
    ) -> Option<Value> {
        use rand::Rng;
        // Numeric scope/foreach constraints share the class solver's joint
        // comparison and union sampler. Keep the legacy non-integral path
        // for strings, reals, X/Z and expressions outside the exact model.
        let string_item = ranges.iter().any(|e| match &e.kind {
            ExprKind::Range(lo, hi) => {
                self.expr_is_string_valued(lo) || self.expr_is_string_valued(hi)
            }
            _ => self.expr_is_string_valued(e),
        });
        if !target.is_real && !string_item {
            let cr: Vec<_> = ranges
                .iter()
                .map(|r| match &r.kind {
                    ExprKind::Range(lo, hi) => ConstraintRange::Range {
                        lo: (**lo).clone(),
                        hi: (**hi).clone(),
                    },
                    _ => ConstraintRange::Value(r.clone()),
                })
                .collect();
            let (choices, ws) = self.cons_ranges_i128_typed(&cr, target.width, target.is_signed);
            if ws.is_some() {
                let choices =
                    Self::fit_cons_ranges_for_target(choices, ws, target.width, target.is_signed);
                return self.pick_i128_range(&choices, target.width, target.is_signed);
            }
        }
        let mut candidates: Vec<Value> = Vec::new();
        for r in ranges {
            match &r.kind {
                ExprKind::Range(lo, hi) => {
                    let lov = self.eval_expr(lo);
                    let hiv = self.eval_expr(hi);
                    let l = lov.to_i64().unwrap_or(0);
                    let h = hiv.to_i64().unwrap_or(l);
                    if h >= l {
                        // Pick uniformly over the WHOLE range. This is one
                        // modulo, not an enumeration: a cap here would silently
                        // shrink the legal value set (`[32'he000_0000 :
                        // 32'he000_2000]` could never yield anything above
                        // 32'he000_0fff).
                        let span = (h as i128) - (l as i128) + 1;
                        let pick = (l as i128) + (self.cur_rng().r#gen::<u64>() as i128) % span;
                        let w = lov.width.max(hiv.width).max(32);
                        candidates.push(Value::from_u64(pick as u64, w));
                    }
                }
                ExprKind::Ident(h) => {
                    let mut name = self.resolve_hier_name(h);
                    if let Some(s) = self.instance_assoc_member(&name) {
                        name = std::borrow::Cow::Owned(s);
                    }
                    if self.module.arrays.contains_key(&*name)
                        || self.module.dynamic_arrays.contains(&*name)
                    {
                        let sz = self.get_queue_size(&name);
                        for i in 0..sz {
                            if let Some(v) =
                                self.get_signal_value_by_name(&format!("{}[{}]", name, i))
                            {
                                candidates.push(v);
                            }
                        }
                    } else {
                        candidates.push(self.eval_expr(r));
                    }
                }
                _ => candidates.push(self.eval_expr(r)),
            }
        }
        if candidates.is_empty() {
            None
        } else {
            Some(candidates[self.cur_rng().r#gen::<usize>() % candidates.len()].clone())
        }
    }

    /// Class type of a variable, if it is a class-handle variable. Guards `%p`
    /// against treating a plain integer as an object handle.
    /// Declared type NAME of a variable, whatever its storage. Unlike
    /// `class_of_var` this does not require the name to be a known class, so an
    /// enum-typed destination can be recognised too.
    pub(super) fn type_name_of_var(&self, vname: &str) -> Option<String> {
        if let Some(c) = self.var_class_types.get(vname) {
            return Some(c.clone());
        }
        if let Some(sig) = self.module.signals.get(vname) {
            if let Some(tn) = &sig.type_name {
                return Some(tn.clone());
            }
        }
        if let Some(&id) = self.signal_name_to_id.get(vname) {
            if let Some(tn) = self.signal_type_names.get(&id) {
                return Some(tn.clone());
            }
        }
        None
    }

    /// Resolve a declared `DataType` to the ENUM typedef name it names, if it
    /// is (or aliases to) an enum — following typedef chains. Used by `$cast`
    /// to range-check a value against an enum destination whose variable is a
    /// class-member / method-local (not tracked by `type_name_of_var`).
    pub(super) fn data_type_enum_name(&self, dt: &crate::ast::types::DataType) -> Option<String> {
        use crate::ast::types::DataType;
        let mut cur = dt.clone();
        let mut seen: Vec<String> = Vec::new();
        loop {
            match &cur {
                DataType::TypeReference { name, .. } => {
                    let n = name.name.name.clone();
                    if self.module.enum_members.contains_key(&n) {
                        return Some(n);
                    }
                    if seen.contains(&n) {
                        return None; // typedef cycle
                    }
                    seen.push(n.clone());
                    // Follow an aliasing typedef to its underlying type.
                    match self.module.typedef_types.get(&n) {
                        Some(next) => cur = next.clone(),
                        None => return None,
                    }
                }
                _ => return None,
            }
        }
    }

    /// Resolve a variable to its ENUM typedef name, if any, for `$cast` range
    /// validation. Tries every type registry in order — module signal / local
    /// (`type_name_of_var`), a runtime-recorded method-local declaration
    /// (`var_decl_types`), and a property of the current `this` object (a
    /// class member like `uvm_report_handler::l_verbosity`).
    pub(super) fn enum_typename_of_var(&self, vname: &str) -> Option<String> {
        if let Some(tn) = self.type_name_of_var(vname) {
            if self.module.enum_members.contains_key(&tn) {
                return Some(tn);
            }
        }
        if let Some(dt) = self.module.var_decl_types.get(vname).cloned() {
            if let Some(tn) = self.data_type_enum_name(&dt) {
                return Some(tn);
            }
        }
        if let Some(h) = self.this_stack.last().copied().flatten() {
            if let Some(cls) = self
                .heap
                .get(h)
                .and_then(|o| o.as_ref())
                .map(|i| i.class_name.clone())
            {
                if let Some(tn) = self.class_prop_type_named(&cls, vname) {
                    if self.module.enum_members.contains_key(&tn) {
                        return Some(tn);
                    }
                }
            }
        }
        None
    }

    /// Build the VPI object for one `$systf` argument.
    ///
    /// A signal-backed argument gets a real signal handle, so a systf can
    /// `vpi_put_value` into it — that is how an `output` argument works.
    /// Anything else (a literal, a concatenation, a computed expression) is
    /// evaluated once and handed over as a read-only `vpiConstant`.
    pub(super) fn vpi_arg_handle(&mut self, e: &Expression) -> VpiHandle {
        if let ExprKind::Ident(h) = &e.kind {
            let name = self.resolve_hier_name(h);
            if let Some(&id) = self.signal_name_to_id.get(name.as_ref()) {
                let ty = vpi_type_of(self, &name, id);
                let leaf = name.rsplit('.').next().unwrap_or(&name).to_string();
                let full = vpi_full_name(self, &name);
                return VpiHandle::signal(id, ty, &leaf, &full);
            }
        }
        VpiHandle::constant(self.eval_expr(e))
    }

    /// Declared width of property `prop` on `class_name` or any ancestor.
    ///
    /// A class property is stored as a bare `Value` in `ClassInstance`, which
    /// carries no declared width of its own. Every write therefore has to
    /// consult the class definition, or a wide value lands in a narrow
    /// property intact: `u2_t v; v = 32'hDEADBEEF;` kept all 32 bits.
    /// `None` for reals, class handles and strings, which must not be resized.
    /// Module definition the current process belongs to, resolved from its
    /// instance-scope hint. The top module's own definition when there is no
    /// hint (the process runs in the top scope).
    pub(super) fn current_module_def(&self) -> &str {
        // An edge block executes outside any scheduled process, so
        // `current_pid` is whatever ran last — its module's timescale would
        // leak into this block's $time/%t (issue seen with --module-timescale
        // on a hierarchical design: the first clock edge in a scaled
        // submodule printed the TOP-scaled time). The dispatcher installs the
        // block's own instance scope here for the duration.
        match self
            .timescale_scope_override
            .as_ref()
            .or_else(|| {
                // §3.14.1: a delay inside an INLINED hierarchical/interface
                // task body quantizes at the DECLARING instance's timescale,
                // not the calling process's. The task frame carries that
                // instance (same source the name-resolve hint uses on
                // resume); without this, `#0.002` in a 1ns/1ns BFM module
                // called from a 1ps-precision testbench kept a real 2ps
                // duration instead of rounding to zero — and a delay-paced
                // BFM loop that hits one such call is phase-shifted off the
                // clock grid for the rest of the run.
                self.task_cleanup
                    .last()
                    .and_then(|c| c.frame_scope_hint.as_ref())
            })
            .or_else(|| self.process_scope_hint.get(&self.current_pid))
        {
            Some(s) if !s.is_empty() => {
                let rel = s
                    .strip_prefix(self.module.name.as_str())
                    .and_then(|rest| rest.strip_prefix('.'))
                    .unwrap_or(s);
                self.module
                    .instances
                    .iter()
                    .find(|i| i.path == rel || i.path == *s)
                    .map(|i| i.def_name.as_str())
                    .unwrap_or(self.module.name.as_str())
            }
            _ => self.module.name.as_str(),
        }
    }

    /// Timeunit / time-precision exponents (powers of 10 in seconds) of the
    /// current process's module (§20.3). Falls back to the design defaults.
    pub(super) fn current_timescale_exp(&self) -> (i32, i32) {
        let def = self.current_module_def();
        self.module
            .module_timescale_exp
            .get(def)
            .copied()
            .unwrap_or((self.module.timeunit_exp, self.module.timeprecision_exp))
    }

    /// §3.14.2 / §20.4 — the timescale a module REPORTS via `$printtimescale`.
    /// A module with no `timescale directive (and none preceding it in
    /// compilation order) runs on the tool default `1ns / 1ns` for both delays
    /// and `$time`/`$realtime`; elaboration records that default in
    /// `module_timescale_exp`, so the fallback here only covers definitions the
    /// map never saw. A directive-less module must not display another
    /// module's timescale.
    pub(super) fn reported_timescale_exp(&self, def: &str) -> (i32, i32) {
        self.module
            .module_timescale_exp
            .get(def)
            .copied()
            .unwrap_or((-9, -9))
    }

    /// `--dump-timescales`: print every module definition's timescale before the
    /// run. Shows the REPORTED unit/precision (`$printtimescale` semantics: an
    /// explicit/inherited `timescale, or the 1ns/1ns tool default when a
    /// module has none — flagged, so a mixed design shows which modules count
    /// nanoseconds by default). No source `$printtimescale` calls are needed.
    pub(super) fn dump_module_timescales(&self) {
        // Every module definition reachable in the design: the top plus every
        // instantiated def_name (deduplicated, sorted for stable output).
        // The multi-top wrapper is not a module of the design.
        let mut defs: Vec<String> = if self.root_is_multi_top() {
            Vec::new()
        } else {
            vec![self.module.name.clone()]
        };
        let mut seen: std::collections::HashSet<String> =
            std::iter::once(self.module.name.clone()).collect();
        for inst in &self.module.instances {
            if seen.insert(inst.def_name.clone()) {
                defs.push(inst.def_name.clone());
            }
        }
        defs.sort();
        println!("=== module timescales ({} modules) ===", defs.len());
        for d in &defs {
            let (u, p) = self.reported_timescale_exp(d);
            let defaulted = self.module.modules_without_timescale.iter().any(|m| m == d);
            let note = if defaulted {
                "   (no `timescale — 1ns/1ns default)"
            } else {
                ""
            };
            println!(
                "  {:<28} {} / {}{}",
                d,
                Self::time_exp_to_str(u),
                Self::time_exp_to_str(p),
                note
            );
        }
        println!("======================================");
    }

    /// Current simulation time in the current module's time unit, as
    /// `$time`/`$realtime` require (§20.3). For a default 1 ns design this is
    /// just the tick count.
    pub(super) fn time_in_current_unit(&self) -> f64 {
        // Exponent-difference division keeps the result exact where it can
        // be: 1600 ticks(1ps) in a 1ns unit is 1600.0 / 1000.0 == literal
        // 1.6, so testbench `$realtime != 1.6` compares hold. Multiplying by
        // tick seconds first (1e-12 is inexact) drifted by an ulp.
        let (unit_exp, _) = self.current_timescale_exp();
        let diff = unit_exp - Self::secs_to_exp(self.tick_s);
        if diff >= 0 {
            self.time as f64 / 10f64.powi(diff)
        } else {
            self.time as f64 * 10f64.powi(-diff)
        }
    }

    pub(super) fn secs_to_exp(s: f64) -> i32 {
        if s <= 0.0 {
            -9
        } else {
            s.log10().round() as i32
        }
    }

    /// A power-of-ten seconds exponent as a time literal string:
    /// -6 -> "1us", -8 -> "10ns", -7 -> "100ns".
    pub(super) fn time_exp_to_str(exp: i32) -> String {
        let unit_exp = (exp as f64 / 3.0).floor() as i32 * 3;
        let mant = 10i64.pow((exp - unit_exp).max(0) as u32);
        let unit = match unit_exp {
            0 => "s",
            -3 => "ms",
            -6 => "us",
            -9 => "ns",
            -12 => "ps",
            -15 => "fs",
            _ => "s",
        };
        format!("{}{}", mant, unit)
    }

    /// Format a %t field (§21.2.1.5). `value` is the time argument in the
    /// current scope's time unit ($time/$realtime already are); it is scaled to
    /// the $timeformat unit, printed with the configured fraction digits and
    /// suffix, and right-justified in the minimum field width.
    /// `%t` body (unpadded) plus the minimum field width in effect —
    /// `$timeformat`'s if called, else the §21.3.5 default of 20.
    pub(super) fn format_time_parts(&self, value: f64) -> (String, usize) {
        let (scope_unit_exp, _) = self.current_timescale_exp();
        let (tf_units, tf_prec, tf_suffix, tf_width) = self
            .time_format
            .clone()
            .unwrap_or_else(|| (Self::secs_to_exp(self.tick_s), 0, String::new(), 20));
        let display = value * 10f64.powi(scope_unit_exp - tf_units);
        let body = format!("{:.*}{}", tf_prec, display, tf_suffix);
        (body, tf_width)
    }
}
