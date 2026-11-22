//! `foreach` loops.

use super::*;

impl Simulator {
    pub(in crate::compiler::simulator) fn exec_stmt_foreach(
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
        // #1108: `foreach (lane[0].payload[i])`.
        let folded = self.inst_array_member_canon(array);
        let array = folded.as_ref().unwrap_or(array);
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
    pub(in crate::compiler::simulator) fn foreach_unpacked_descs(
        &self,
        name: &str,
        ndims: usize,
    ) -> Vec<bool> {
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

    pub(in crate::compiler::simulator) fn exec_stmt_foreach_inner(
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
                                Self::name_key_value(k)
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
                                        Self::name_key_value(key)
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
                // §12.7.3: a declared collection of collections walks each
                // level as its declaration names it.
                if vars.len() >= 2 && self.foreach_shaped(&an, vars, body, iterations) {
                    self.auto_loop_vars.truncate(fe_auto_len);
                    self.restore_loop_vars(&fe_saved);
                    if !self.return_flag && self.disable_target.is_none() {
                        self.break_flag = false;
                    }
                    return;
                }
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
                                Self::assoc_first_key_seg(rest).map(|s| s.to_string())
                            })
                            .collect();
                        Self::sort_name_keys(&mut ks);
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
                            Self::sort_name_keys(&mut ks);
                        } else {
                            // §7.9: int keys traverse in NUMERIC order
                            // including NEGATIVE keys — u64 parsing
                            // mangled "-7" to 0.
                            let wide = self.assoc_index_width_for(&an).is_some_and(|(w, _)| w > 64);
                            Self::sort_int_keys(&mut ks, wide);
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
                            Self::name_key_value(&key)
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
            // §12.7.3: a declared collection of collections (see the
            // member arm).
            if vars.len() >= 2 && self.foreach_shaped(&name, vars, body, iterations) {
                self.auto_loop_vars.truncate(fe_auto_len);
                self.restore_loop_vars(&fe_saved);
                if !self.return_flag && self.disable_target.is_none() {
                    self.break_flag = false;
                }
                return;
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
            // §12.7.3, §7.2.1: `foreach (frame.lanes[k])` over a packed-array
            // member of a packed-struct class property iterates the member's
            // packed dimensions.
            if prop_packed_dims.is_none()
                && hier.path.len() >= 2
                && hier.path[..hier.path.len() - 1]
                    .iter()
                    .any(|s| self.struct_prop_name_possible(&s.name.name))
                && let Some(pd) = self.struct_member_packed_dims(array)
                && pd.len() >= vars.len()
            {
                prop_packed_dims = Some(pd);
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
                                        Self::name_key_value(key)
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
                            Self::assoc_first_key_seg(rest).map(|s| s.to_string())
                        })
                        .collect();
                    Self::sort_name_keys(&mut keys);
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
                        Self::sort_name_keys(&mut keys);
                    } else {
                        Self::sort_int_keys(&mut keys, kw > 64);
                    }
                    for key in keys {
                        if self.finished {
                            break;
                        }
                        let kv = if is_str {
                            Self::name_key_value(&key)
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
                } else if let Some(&(l, r)) = prop_packed_dims
                    .as_ref()
                    .and_then(|d| d.first())
                    .or_else(|| {
                        self.module
                            .packed_full_dims
                            .get(&*name)
                            .filter(|d| d.len() >= 2)
                            .and_then(|d| d.first())
                    })
                {
                    // §12.7.3 (measured): a MULTI-packed-dim vector
                    // (`logic [3:0][7:0] w`) iterates its OUTERMOST
                    // packed dimension (4 iterations, not 32 bits),
                    // left→right — [3:0] visits 3,2,1,0. A class
                    // property of that shape, or a packed array of
                    // packed structs (`cand_t [0:2] c`, §7.4.1), too:
                    // it has no module-scope dimension record (#928).
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
                    // §12.7.3: a formal's live value has the callee's width,
                    // including class-specialized bounds. It shadows any
                    // module signal or stale width entry with the same name.
                    let size = self
                        .local_stack
                        .iter()
                        .rev()
                        .find_map(|frame| frame.get(&*name).map(|v| v.width))
                        .or_else(|| self.lookup_signal_width(&name))
                        .unwrap_or(1) as u64;
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
}
