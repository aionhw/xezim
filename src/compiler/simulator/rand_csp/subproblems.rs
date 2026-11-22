//! Sources, frames, reads and writes of the state, joined sub-problems and sub-constraints.

use super::*;

impl Simulator {
    pub(super) fn csp_src_fixed(&self, csp: &Csp, st: &St, src: usize) -> bool {
        !csp.srcs[src].stale && st.all_fixed(&csp.srcs[src].deps)
    }

    /// Whether `item` names an array of the size phase (`Csp::sizes`).
    pub(super) fn csp_src_sized(csp: &Csp, item: &SrcItem) -> bool {
        fn names(csp: &Csp, e: &Expression) -> bool {
            let mut hit = false;
            Simulator::walk_operands(e, &mut |x| match &x.kind {
                ExprKind::Ident(h)
                    if h.path.iter().any(|s| csp.sizes.contains_key(&s.name.name)) =>
                {
                    hit = true;
                }
                ExprKind::WithClause { expr, filter } => {
                    hit |= names(csp, expr) || names(csp, filter);
                }
                _ => {}
            });
            hit
        }
        let mut hit = false;
        let mut look = |e: &Expression| hit |= names(csp, e);
        match item {
            SrcItem::Expr(e) => look(e),
            SrcItem::Item(it) => Self::csp_item_walk(it, &mut |i| match i {
                ConstraintItem::Expr(e) => look(e),
                ConstraintItem::Inside { expr, range, .. } => {
                    look(expr);
                    for r in range {
                        match r {
                            ConstraintRange::Value(v) => look(v),
                            ConstraintRange::Range { lo, hi } => {
                                look(lo);
                                look(hi);
                            }
                        }
                    }
                }
                ConstraintItem::Implication { condition, .. }
                | ConstraintItem::IfElse { condition, .. } => look(condition),
                ConstraintItem::Foreach { array, .. } => look(array),
                ConstraintItem::Unique { exprs, .. } => exprs.iter().for_each(&mut look),
                _ => {}
            }),
        }
        hit
    }

    /// The local frame to evaluate under with `binds` (foreach indices)
    /// bound. A class solve runs in the object's method frame, which holds
    /// nothing else; a scope solve keeps the caller's locals visible.
    pub(super) fn csp_frame(&self, binds: &[(String, Value)]) -> HashMap<String, Value> {
        let mut frame: HashMap<String, Value> = if CSP_SCOPE.with(|c| c.get()) {
            self.local_stack.last().cloned().unwrap_or_default()
        } else {
            HashMap::default()
        };
        frame.extend(binds.iter().cloned());
        frame
    }

    /// Judge a source constraint with the ordinary evaluator: the fixed
    /// variables it reads are written to the object first.
    pub(super) fn csp_eval_src(&mut self, csp: &Csp, st: &St, src: usize) -> bool {
        let s = &csp.srcs[src];
        for &v in &s.deps {
            if let Some(x) = st.fixed(v) {
                self.csp_write(csp, v, x);
            }
        }
        let frame = self.csp_frame(&s.binds);
        self.push_local_frame(frame);
        let ok = match &s.item {
            SrcItem::Expr(e) => self.cons_expr_true(e),
            SrcItem::Item(it) => self.check_constraint_item_impl(it),
        };
        self.pop_local_frame();
        ok
    }

    /// The value of a source expression with its fixed variables written.
    pub(super) fn csp_eval_value(&mut self, csp: &Csp, st: &St, src: usize) -> Value {
        let s = &csp.srcs[src];
        for &v in &s.deps {
            if let Some(x) = st.fixed(v) {
                self.csp_write(csp, v, x);
            }
        }
        let frame = self.csp_frame(&s.binds);
        self.push_local_frame(frame);
        let v = match &s.item {
            SrcItem::Expr(e) => self.eval_expr(e),
            SrcItem::Item(_) => Value::new(1),
        };
        self.pop_local_frame();
        v
    }

    pub(super) fn csp_write(&mut self, csp: &Csp, v: usize, x: i128) {
        let var = &csp.vars[v];
        let mut val = Value::from_u64(Self::bits_at(x, var.width) as u64, var.width);
        val.is_signed = var.signed;
        if matches!(var.key, VarKey::Aux) {
            // A segment (a packed-struct field or a constant select,
            // §7.2.1/§11.5.1) is written into the bits of the variable it is
            // part of: an evaluator node that reads the field (`r.f ==
            // (K >> p)`) depends on the segment alone, so the base variable
            // is not yet fixed — and therefore not written — when the node is
            // judged.
            let Some(&(base, lo, w)) = csp.aux_base.get(&v) else {
                return;
            };
            let Some(mut cur) = self.csp_read(csp, base) else {
                return;
            };
            for k in 0..w {
                let bit = if (x >> k) & 1 == 1 {
                    LogicBit::One
                } else {
                    LogicBit::Zero
                };
                cur.set_bit((lo + k) as usize, bit);
            }
            self.csp_write_value(csp, base, cur);
            return;
        }
        self.csp_write_value(csp, v, val);
    }

    /// The current value of base variable `v` (not a segment).
    fn csp_read(&mut self, csp: &Csp, v: usize) -> Option<Value> {
        let var = &csp.vars[v];
        let cur = if let Some(lvals) = &csp.scope {
            match &var.key {
                VarKey::Prop(n) => {
                    let lv = lvals.get(n)?.clone();
                    Some(self.eval_expr(&lv))
                }
                VarKey::Elem(k) => self.get_signal_value_by_name(k),
                VarKey::Sub(..) | VarKey::Aux => None,
            }
        } else {
            match &var.key {
                VarKey::Prop(n) => {
                    let key = self.rand_var_key(csp.handle, n);
                    self.heap
                        .get(csp.handle)
                        .and_then(|o| o.as_ref())
                        .and_then(|i| i.properties.get(key.as_ref()).cloned())
                }
                VarKey::Elem(k) => self.read_coll_elem(k),
                VarKey::Sub(h, n) => self
                    .heap
                    .get(*h)
                    .and_then(|o| o.as_ref())
                    .and_then(|i| i.properties.get(n).cloned()),
                VarKey::Aux => None,
            }
        };
        let mut cur = cur.unwrap_or_else(|| Value::zero(var.width));
        if cur.width != var.width {
            cur = cur.resize(var.width);
        }
        cur.is_signed = var.signed;
        Some(cur)
    }

    fn csp_write_value(&mut self, csp: &Csp, v: usize, val: Value) {
        let var = &csp.vars[v];
        if let Some(lvals) = &csp.scope {
            match &var.key {
                VarKey::Prop(n) => {
                    if let Some(lv) = lvals.get(n) {
                        let lv = lv.clone();
                        self.assign_value(&lv, &val);
                    }
                }
                VarKey::Elem(k) => self.set_signal_value_by_name(k, val),
                VarKey::Sub(..) | VarKey::Aux => {}
            }
            return;
        }
        match &var.key {
            VarKey::Prop(n) => {
                let key = self.rand_var_key(csp.handle, n).into_owned();
                if let Some(Some(inst)) = self.heap.get_mut(csp.handle) {
                    inst.properties.insert(key, val);
                }
            }
            VarKey::Elem(k) => self.write_coll_elem(k, val),
            VarKey::Aux => {}
            VarKey::Sub(h, n) => {
                if let Some(Some(inst)) = self.heap.get_mut(*h) {
                    inst.properties.insert(n.clone(), val);
                }
            }
        }
    }

    /// Join the rand sub-objects in `subs` into `csp` and `constraints`
    /// (§18.5.9): prepare object-array indexing (`csp_obj_prepare`), register
    /// each sub-object's variables and rewritten constraints (`{prop}.*`), and
    /// optionally compute the evaluator-spelled `check` constraint list.
    ///
    /// When `hold_sub_sizes` is true (`csp_size_phase`), sub-object scalars
    /// read by a sub-object's own `.size()` constraints are pinned to their
    /// drawn trial values so already-sized sub-object arrays stay valid.
    pub(super) fn csp_join_subs<'a>(
        &mut self,
        csp: &mut Csp,
        constraints: &'a [ClassConstraint],
        subs: &[(String, usize)],
        hold_sub_sizes: bool,
        need_check: bool,
    ) -> Option<SubJoin<'a>> {
        if subs.is_empty() {
            return Some(SubJoin {
                _obj_guard: None,
                constraints: std::borrow::Cow::Borrowed(constraints),
                check: None,
                sub_dynamic: false,
                held_wide: Vec::new(),
            });
        }
        let obj_arrays: HashSet<String> = subs
            .iter()
            .flat_map(|(p, _)| p.split('.'))
            .filter_map(|seg| seg.split_once('[').map(|(a, _)| a.to_string()))
            .collect();
        let _obj_guard = (!obj_arrays.is_empty())
            .then(|| obj::csp_obj_enter(obj_arrays.iter().cloned().collect()));
        let mut joint = if obj_arrays.is_empty() {
            constraints.to_vec()
        } else {
            self.csp_obj_prepare(csp, constraints, &obj_arrays)?
        };
        let mut sub_dynamic = false;
        let mut held_wide = Vec::new();
        for (prop, sub) in subs {
            let (items, dynamic) =
                self.csp_add_sub(csp, prop, *sub, constraints, hold_sub_sizes, &mut held_wide)?;
            sub_dynamic |= dynamic;
            joint.push(ClassConstraint {
                is_static: false,
                is_extern: false,
                has_body: true,
                name: crate::ast::Identifier {
                    name: format!("{}.*", prop),
                    span: crate::ast::Span::dummy(),
                },
                items,
                span: crate::ast::Span::dummy(),
            });
        }
        let check = (need_check && !obj_arrays.is_empty()).then(|| {
            joint
                .iter()
                .map(|c| {
                    let mut c = c.clone();
                    c.items = c.items.iter().map(Self::csp_obj_real_item).collect();
                    c
                })
                .collect()
        });
        Some(SubJoin {
            _obj_guard,
            constraints: std::borrow::Cow::Owned(joint),
            check,
            sub_dynamic,
            held_wide,
        })
    }

    /// Pin a wide variable's split segments and slice vars to its drawn `val`.
    pub(super) fn csp_pin_wide_var(csp: &mut Csp, v: usize, val: &Value) {
        if val.has_xz() || val.is_real {
            return;
        }
        let slice_val = |lo: u32, w: u32| -> i128 {
            let mut x = 0i128;
            for k in 0..w {
                if val.get_bit((lo + k) as usize) == LogicBit::One {
                    x |= 1i128 << k;
                }
            }
            x
        };
        if let Some(segs) = csp.segs.get(&v) {
            for &(lo, w, sv) in segs {
                let x = slice_val(lo, w);
                csp.dom0[sv] = vec![(x, x)];
            }
        }
        for (&(bv, lo, w), &sv) in &csp.slice_vars {
            if bv == v {
                let x = slice_val(lo, w);
                csp.dom0[sv] = vec![(x, x)];
            }
        }
    }

    /// Whether any rand sub-object in `subs` owns a dynamic rand value
    /// collection (`CollKind::Dyn`, not handle arrays) whose size stayed fixed
    /// at its nested-randomize draw.
    pub(in crate::compiler::simulator) fn csp_subs_have_dyn(
        &self,
        subs: &[(String, usize)],
    ) -> bool {
        subs.iter().any(|&(_, sub)| {
            let disabled = self.rand_mode_disabled.get(&sub);
            let mut cur = self
                .heap
                .get(sub)
                .and_then(|o| o.as_ref())
                .map(|i| i.class_name.clone());
            let mut seen = HashSet::default();
            while let Some(cn) = cur {
                let Some(cd) = self.module.classes.get(&cn) else {
                    break;
                };
                for prop in &self.randomize_members(cd, disabled.unwrap_or(&HashSet::default())) {
                    if seen.insert(prop.clone())
                        && cd.queue_properties.contains_key(prop)
                        && self.prop_class_type(sub, prop).is_none()
                    {
                        return true;
                    }
                }
                cur = cd.extends.clone();
            }
            false
        })
    }

    fn sub_expr_root_member(e: &Expression) -> Option<&str> {
        match &Self::unparen(e).kind {
            ExprKind::Ident(h) if h.root.is_none() => {
                let start = usize::from(h.path.first().is_some_and(|s| s.name.name == "this"));
                h.path.get(start).map(|s| s.name.name.as_str())
            }
            ExprKind::MemberAccess { expr, member } => {
                if matches!(Self::unparen(expr).kind, ExprKind::This) {
                    Some(member.name.as_str())
                } else {
                    Self::sub_expr_root_member(expr)
                }
            }
            ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => {
                Self::sub_expr_root_member(expr)
            }
            _ => None,
        }
    }

    fn sub_expr_reads_dyn_size(e: &Expression, dyn_props: &HashSet<&str>) -> bool {
        let mut hit = false;
        Self::walk_operands(e, &mut |x| {
            let recv = match &x.kind {
                ExprKind::Call { func, args } if args.is_empty() => match &func.kind {
                    ExprKind::MemberAccess { expr, member } if member.name == "size" => {
                        Some(expr.as_ref())
                    }
                    _ => None,
                },
                ExprKind::MemberAccess { expr, member } if member.name == "size" => {
                    Some(expr.as_ref())
                }
                ExprKind::SystemCall { name, args } if name == "$size" => args.first(),
                _ => None,
            };
            if let Some(r) = recv
                && Self::sub_expr_root_member(r).is_some_and(|m| dyn_props.contains(m))
            {
                hit = true;
            }
            if let ExprKind::Ident(h) = &x.kind
                && h.root.is_none()
                && h.path.last().is_some_and(|s| s.name.name == "size")
            {
                let start = usize::from(h.path.first().is_some_and(|s| s.name.name == "this"));
                if h.path.len() >= start + 2 && dyn_props.contains(h.path[start].name.name.as_str())
                {
                    hit = true;
                }
            }
        });
        hit
    }

    fn sub_collect_expr_roots(e: &Expression, out: &mut HashSet<String>) {
        Self::walk_operands(e, &mut |x| {
            if let Some(m) = Self::sub_expr_root_member(x) {
                out.insert(m.to_string());
            }
        });
    }

    fn sub_collect_size_item_reads(
        item: &ConstraintItem,
        dyn_props: &HashSet<&str>,
        guards: &[&Expression],
        out: &mut HashSet<String>,
    ) {
        match item {
            ConstraintItem::Block(items) => {
                for it in items {
                    Self::sub_collect_size_item_reads(it, dyn_props, guards, out);
                }
            }
            ConstraintItem::Soft(inner) => {
                Self::sub_collect_size_item_reads(inner, dyn_props, guards, out);
            }
            ConstraintItem::Foreach { array, item, .. } => {
                let mut g = guards.to_vec();
                g.push(array);
                Self::sub_collect_size_item_reads(item, dyn_props, &g, out);
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => {
                let mut g = guards.to_vec();
                g.push(condition);
                Self::sub_collect_size_item_reads(constraint, dyn_props, &g, out);
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                let mut g = guards.to_vec();
                g.push(condition);
                Self::sub_collect_size_item_reads(then_item, dyn_props, &g, out);
                if let Some(e) = else_item {
                    Self::sub_collect_size_item_reads(e, dyn_props, &g, out);
                }
            }
            ConstraintItem::Expr(e) if Self::sub_expr_reads_dyn_size(e, dyn_props) => {
                Self::sub_collect_expr_roots(e, out);
                for g in guards {
                    Self::sub_collect_expr_roots(g, out);
                }
            }
            ConstraintItem::Inside { expr, range, .. } => {
                let has_sz = Self::sub_expr_reads_dyn_size(expr, dyn_props)
                    || range.iter().any(|r| match r {
                        ConstraintRange::Value(v) => Self::sub_expr_reads_dyn_size(v, dyn_props),
                        ConstraintRange::Range { lo, hi } => {
                            Self::sub_expr_reads_dyn_size(lo, dyn_props)
                                || Self::sub_expr_reads_dyn_size(hi, dyn_props)
                        }
                    });
                if has_sz {
                    Self::sub_collect_expr_roots(expr, out);
                    for r in range {
                        match r {
                            ConstraintRange::Value(v) => Self::sub_collect_expr_roots(v, out),
                            ConstraintRange::Range { lo, hi } => {
                                Self::sub_collect_expr_roots(lo, out);
                                Self::sub_collect_expr_roots(hi, out);
                            }
                        }
                    }
                    for g in guards {
                        Self::sub_collect_expr_roots(g, out);
                    }
                }
            }
            _ => {}
        }
    }

    /// Add the rand scalars of the rand sub-object `sub` (member `prop` of
    /// the object being randomized, or `arr[k]` for an element of an object
    /// array) as `prop.<name>` variables, and return its class constraints
    /// rewritten to name them through `prop`. A member wider than 64 bits (a
    /// wide packed struct) is solved through its segments like the parent's
    /// own (`csp_wide_split`), or left as drawn when no constraint of the
    /// solve (`parent`'s or the sub-object's) reads it. A `foreach` over a
    /// fixed-shape member of the sub-object is unrolled. None when the
    /// sub-object holds anything this solver does not model: a real,
    /// a randc, an unpacked struct, an unsupported collection shape, or a
    /// constraint that calls a method.
    fn csp_add_sub(
        &mut self,
        csp: &mut Csp,
        prop: &str,
        sub: usize,
        parent: &[ClassConstraint],
        hold_sub_sizes: bool,
        held_wide: &mut Vec<(usize, Value)>,
    ) -> Option<(Vec<ConstraintItem>, bool)> {
        let class_name = self.heap.get(sub)?.as_ref()?.class_name.clone();
        let rand_disabled = self
            .rand_mode_disabled
            .get(&sub)
            .cloned()
            .unwrap_or_default();
        let con_disabled = self
            .constraint_mode_disabled
            .get(&sub)
            .cloned()
            .unwrap_or_default();
        csp.has_subs = true;
        csp.sub_prefixes.insert(sub, prop.to_string());
        let mut members: HashSet<String> = HashSet::default();
        let mut items: Vec<ConstraintItem> = Vec::new();
        let mut seen_con: HashSet<String> = HashSet::default();
        let mut seen_var: HashSet<String> = HashSet::default();
        // (name, width, signed, domain; None for a wide member)
        let mut decls: Vec<(String, u32, bool, Option<Dom>)> = Vec::new();
        let mut cur = Some(class_name);
        while let Some(cn) = cur {
            let cd = self.module.classes.get(&cn)?.clone();
            if !cd.randc_properties.is_empty() {
                return None;
            }
            members.extend(cd.properties.keys().cloned());
            members.extend(cd.array_properties.keys().cloned());
            members.extend(cd.array_nd_properties.keys().cloned());
            members.extend(cd.queue_properties.keys().cloned());
            members.extend(cd.assoc_properties.keys().cloned());
            for name in self.randomize_members(&cd, &rand_disabled) {
                if !seen_var.insert(name.clone()) {
                    continue;
                }
                let sig = cd.properties.get(&name)?;
                // §8.25: a member typed by a type parameter bound to a class
                // is a handle too.
                let is_obj = self.prop_class_type(sub, &name).is_some();
                // §18.5.9: a rand object member joins the solve as a
                // sub-object of its own (`rand_csp_nested_subs`).
                if is_obj
                    && !cd.array_properties.contains_key(&name)
                    && !cd.array_nd_properties.contains_key(&name)
                    && !cd.queue_properties.contains_key(&name)
                    && !cd.assoc_properties.contains_key(&name)
                {
                    continue;
                }
                let is_coll = cd.array_properties.contains_key(&name)
                    || cd.array_nd_properties.contains_key(&name)
                    || cd.queue_properties.contains_key(&name)
                    || cd.assoc_properties.contains_key(&name);
                // §18.5.9 (#860): a child collection joins through its
                // elements below, rather than aborting the parent's solve.
                if is_coll && !is_obj {
                    if sig.is_real
                        || self
                            .class_prop_struct(sub, &name)
                            .is_some_and(|s| !s.packed)
                    {
                        return None;
                    }
                    continue;
                }
                // §18.5.9: the elements of a fixed, dynamic or queue array
                // of handles join as sub-objects of their own
                // (`rand_csp_nested_subs`).
                if is_obj
                    && !cd.array_nd_properties.contains_key(&name)
                    && !cd.assoc_properties.contains_key(&name)
                {
                    continue;
                }
                if is_obj || sig.is_real || sig.width == 0 || is_coll {
                    return None;
                }
                // §7.2.1: a packed struct is one integral value of its true
                // width; an unpacked one is not modelled.
                let width = match self.class_prop_struct(sub, &name) {
                    Some(su) if su.packed => {
                        let params = self.instance_param_scope(sub);
                        // §7.4.1: a packed ARRAY of structs holds every
                        // element; the layout alone is one element (#928).
                        let elem_w = self.packed_agg_layout_with(&su, &params).1;
                        Self::packed_struct_array_width(&su, elem_w, &params).unwrap_or(elem_w)
                    }
                    Some(_) => return None,
                    None => sig.width,
                };
                let enum_dom = sig
                    .type_name
                    .as_ref()
                    .and_then(|tn| self.module.enum_members.get(tn))
                    .filter(|m| !m.is_empty())
                    .map(|m| dom_norm(m.iter().map(|e| (e.1 as i128, e.1 as i128)).collect()));
                if width > 64 {
                    if enum_dom.is_some() {
                        return None;
                    }
                    decls.push((name, width, sig.is_signed, None));
                    continue;
                }
                let dom = enum_dom.unwrap_or_else(|| {
                    let (lo, hi) = ws_range(width, sig.is_signed);
                    vec![(lo, hi)]
                });
                decls.push((name, width, sig.is_signed, Some(dom)));
            }
            if !con_disabled.contains("*") {
                for (cname, con) in cd.constraints.iter() {
                    if !seen_con.insert(cname.clone()) || con_disabled.contains(cname) {
                        continue;
                    }
                    items.extend(con.items.iter().cloned());
                }
            }
            cur = cd.extends.clone();
        }
        let child_constraints = [ClassConstraint {
            is_static: false,
            is_extern: false,
            has_body: true,
            name: crate::ast::Identifier {
                name: "__child__".to_string(),
                span: crate::ast::Span::dummy(),
            },
            items: items.clone(),
            span: crate::ast::Span::dummy(),
        }];
        let colls = self.collect_rand_colls(sub, &rand_disabled, &child_constraints);
        // (An array of handles holds no values to size or draw: its
        // elements join as sub-objects.)
        let dynamic = colls
            .iter()
            .any(|c| c.kind == CollKind::Dyn && !c.is_object_elem);
        let held_for_sizes: HashSet<String> = if hold_sub_sizes && dynamic {
            let dyn_props: HashSet<&str> = colls
                .iter()
                .filter(|c| c.kind == CollKind::Dyn && !c.is_object_elem)
                .map(|c| c.prop.as_str())
                .collect();
            let mut held = HashSet::default();
            for it in &items {
                Self::sub_collect_size_item_reads(it, &dyn_props, &[], &mut held);
            }
            held
        } else {
            HashSet::default()
        };
        if !colls.is_empty() {
            let colls = self.csp_expand_rows(&colls);
            let enums: HashMap<String, String> = colls
                .iter()
                .filter_map(|c| {
                    if let Some(m) = &c.member {
                        return m.enum_type.clone().map(|tn| (c.prop.clone(), tn));
                    }
                    let p = c.nd_base.as_ref().map_or(c.prop.as_str(), |(p, _)| p);
                    let tn = self.class_prop_type_name(sub, p)?;
                    self.module
                        .enum_members
                        .contains_key(&tn)
                        .then(|| (c.prop.clone(), tn))
                })
                .collect();
            let child = self.csp_vars(
                sub,
                &[],
                &HashSet::default(),
                &HashMap::default(),
                &colls,
                &enums,
                &HashSet::default(),
                true,
                &HashSet::default(),
            )?;
            let offset = csp.vars.len();
            let child_wide = child.wide;
            csp.vars.extend(child.vars);
            csp.dom0.extend(child.dom0);
            csp.wide.extend(child_wide.iter().map(|&v| offset + v));
            for (name, mut array) in child.arrays {
                let pin_arr =
                    held_for_sizes.contains(name.split_once('[').map_or(name.as_str(), |(b, _)| b));
                for (_, v) in &mut array.elems {
                    *v += offset;
                    if pin_arr
                        && let VarKey::Elem(k) = &csp.vars[*v].key
                        && let Some(val) = self.read_coll_elem(k)
                        && !val.has_xz()
                        && !val.is_real
                    {
                        if child_wide.contains(&(*v - offset)) {
                            held_wide.push((*v, val));
                        } else {
                            let x = if csp.vars[*v].signed {
                                val.to_i64().map(i128::from)
                            } else {
                                val.to_u64().map(i128::from)
                            };
                            if let Some(x) = x {
                                csp.dom0[*v] = vec![(x, x)];
                            }
                        }
                    }
                }
                csp.arrays.insert(format!("{prop}.{name}"), array);
            }
            for (name, dims) in child.nd_dims {
                csp.nd_dims.insert(format!("{prop}.{name}"), dims);
            }
            for (name, n) in child.nd_dyn {
                csp.nd_dyn.insert(format!("{prop}.{name}"), n);
            }
        }
        // §18.5.8.1: a loop over a fixed-shape member of the sub-object has
        // known indices; it is unrolled so every item reads through `prop`.
        let has_foreach = items.iter().any(|it| {
            let mut hit = false;
            Self::csp_item_walk(it, &mut |x| {
                if matches!(x, ConstraintItem::Foreach { .. }) {
                    hit = true;
                }
            });
            hit
        });
        if has_foreach {
            let mut out = Vec::with_capacity(items.len());
            for it in &items {
                self.csp_unroll_item(sub, None, it, &mut out)?;
            }
            items = out;
        }
        // A wide member no constraint reads takes any value of its width.
        let wide: Vec<&str> = decls
            .iter()
            .filter(|d| d.3.is_none())
            .map(|d| d.0.as_str())
            .collect();
        let unread: HashSet<String> = if wide.is_empty() {
            HashSet::default()
        } else {
            let mut all: Vec<ClassConstraint> = parent.to_vec();
            all.push(ClassConstraint {
                is_static: false,
                is_extern: false,
                has_body: true,
                name: crate::ast::Identifier {
                    name: format!("{}.*", prop),
                    span: crate::ast::Span::dummy(),
                },
                items: items.clone(),
                span: crate::ast::Span::dummy(),
            });
            Self::csp_wide_unread(&all, &wide)
        };
        for (name, width, signed, dom) in decls {
            let hold = held_for_sizes.contains(&name);
            let drawn = if hold {
                self.heap
                    .get(sub)
                    .and_then(|o| o.as_ref())
                    .and_then(|i| i.properties.get(&name))
                    .cloned()
            } else {
                None
            };
            let dom = match dom {
                Some(d) => {
                    if let Some(ref val) = drawn
                        && !val.has_xz()
                        && !val.is_real
                    {
                        let x = if signed {
                            val.to_i64().map(i128::from)
                        } else {
                            val.to_u64().map(i128::from)
                        };
                        x.map_or(d, |x| vec![(x, x)])
                    } else {
                        d
                    }
                }
                None if unread.contains(&name) => continue,
                None => {
                    let v = csp.vars.len();
                    csp.wide.insert(v);
                    if let Some(val) = drawn {
                        held_wide.push((v, val));
                    }
                    vec![(0, 0)]
                }
            };
            csp.scalars
                .insert(format!("{}.{}", prop, name), csp.vars.len());
            csp.vars.push(CspVar {
                key: VarKey::Sub(sub, name),
                width,
                signed,
            });
            csp.dom0.push(dom);
        }
        let mut out = Vec::with_capacity(items.len());
        let mut bound = Vec::new();
        for mut it in items {
            if !Self::csp_prefix_item(&mut it, prop, &members, &mut bound) {
                return None;
            }
            out.push(it);
        }
        Some((out, dynamic))
    }
}
