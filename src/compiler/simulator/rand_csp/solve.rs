//! Solver entry points, row expansion, size phases and the solve cache.

use super::*;

impl Simulator {
    /// Solve the constraint set of `handle` jointly (see the module docs).
    /// `rand_props` are the scalar rand properties, `colls` the rand
    /// collections (dynamic ones already sized), `array_enums` the enum type
    /// of enum-typed fixed-array elements. `strict` gives up on a set the
    /// final check cannot fully verify.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::compiler::simulator) fn rand_csp_solve(
        &mut self,
        handle: usize,
        constraints: &[ClassConstraint],
        constraint_depth: &[usize],
        rand_props: &[(String, u32)],
        signed_props: &HashSet<String>,
        enum_props: &HashMap<String, String>,
        colls: &[RandColl],
        array_enums: &HashMap<String, String>,
        strict: bool,
        subs: &[(String, usize)],
    ) -> CspOutcome {
        // §18.5.8.1: the size of a dynamic array is solved before its
        // elements, jointly with the variables its size constraints tie it
        // to (`beats.size() == cmd.count + 1`). The sizes are a solve of
        // their own (`csp_size_phase`); the elements are then solved with
        // the arrays at those sizes. A size that leaves the elements without
        // a solution is drawn again a few times.
        // §7.4.5: the rows of a dynamic member over unpacked rows exist only
        // at its current size; they are expanded again whenever it changes,
        // and their own sizes (`a[i].size()`) are solved once they exist.
        let has_rows = colls.iter().any(|c| c.row_tail.is_some());
        let mut cur: Vec<RandColl> = if has_rows {
            self.csp_expand_rows(colls)
        } else {
            colls.to_vec()
        };
        let sized_of = |cs: &[RandColl]| -> HashSet<String> {
            cs.iter()
                .filter(|c| Self::csp_size_solved(c, constraints))
                .map(|c| c.prop.clone())
                .collect()
        };
        let mut sized = sized_of(&cur);
        if sized.is_empty() {
            return self.rand_csp_solve_sized(
                handle,
                constraints,
                constraint_depth,
                rand_props,
                signed_props,
                enum_props,
                &cur,
                array_enums,
                strict,
                subs,
            );
        }
        let mut rounds = 0;
        let mut solves = 0;
        while rounds < 8 && solves < 4 {
            rounds += 1;
            match self.csp_size_phase(
                handle,
                constraints,
                constraint_depth,
                rand_props,
                signed_props,
                enum_props,
                &cur,
                array_enums,
                &sized,
                subs,
            ) {
                Ok((sizes, csp, st)) => {
                    let sizes: Vec<(RandColl, u64)> =
                        sizes.into_iter().map(|(c, n)| (c.clone(), n)).collect();
                    for (c, n) in &sizes {
                        let n = *n;
                        if c.row_tail.is_some() {
                            // Its elements are rows, not values.
                            self.resize_row_owner(c, n);
                            self.module.dynamic_arrays.insert(c.scoped.clone());
                            continue;
                        }
                        self.resize_coll(&c.scoped, n);
                        self.module.dynamic_arrays.insert(c.scoped.clone());
                        let signed = self.class_prop_signed_of(handle, &c.prop);
                        for i in 0..n {
                            let key = format!("{}[{}]", c.scoped, i);
                            if self.read_coll_elem(&key).is_none() {
                                let mut v = if c.width > 64 {
                                    self.random_value_of_width(c.width)
                                } else {
                                    self.draw_elem(c.width, None)
                                };
                                v.is_signed = signed;
                                self.write_coll_elem(&key, v);
                            }
                        }
                    }

                    // Factorized solve: the size phase already found a valid
                    // assignment for all scalar properties and array sizes.
                    // Write back the scalars and solve the element constraints
                    // directly per-element.
                    let nvars = csp.vars.len();
                    for v in 0..nvars {
                        if csp.wide.contains(&v) {
                            continue;
                        }
                        let x = st.fixed(v).unwrap_or(0);
                        self.csp_write(&csp, v, x);
                    }
                    if csp.draw_free {
                        self.csp_draw_free(&csp);
                    }

                    let mut rand_set = HashSet::default();
                    for (n, _) in rand_props {
                        rand_set.insert(n.clone());
                    }
                    for c in &cur {
                        rand_set.insert(c.prop.clone());
                    }
                    for con in constraints {
                        for item in &con.items {
                            self.solve_forced(handle, item, &rand_set);
                        }
                    }

                    let mut fe_failed = false;
                    // Sub-objects need `rand_csp_solve_sized` so their own
                    // constraints, arrays and opaque checks are verified on
                    // the final values via `SubJoin::check`.
                    let accepted = subs.is_empty()
                        && csp.soft_priority.is_empty()
                        && array_enums.is_empty()
                        && self.rand_items_accept(handle, constraints, &cur, &mut fe_failed);
                    if accepted {
                        return CspOutcome::Sat;
                    }
                }
                // A sub-object dynamic array was held fixed at its trial draw;
                // refuting those sizes does not refute the next trial's draw.
                Err(SizePhaseErr::SubDynamicUnsat) => return CspOutcome::GaveUp,
                // A relaxation of the problem has no solution: neither has
                // the problem.
                Err(SizePhaseErr::Outcome(CspOutcome::Unsat)) => return CspOutcome::Unsat,
                // Not modelled: the sizes the caller drew stay, and the
                // outcome proves nothing about other sizes.
                Err(SizePhaseErr::Outcome(_)) => {
                    if has_rows {
                        cur = self.csp_expand_rows(colls);
                    }
                    return match self.rand_csp_solve_sized(
                        handle,
                        constraints,
                        constraint_depth,
                        rand_props,
                        signed_props,
                        enum_props,
                        &cur,
                        array_enums,
                        strict,
                        subs,
                    ) {
                        CspOutcome::Unsat => CspOutcome::GaveUp,
                        o => o,
                    };
                }
            }
            if has_rows {
                // New rows may have sizes of their own to solve first.
                let next = self.csp_expand_rows(colls);
                let next_sized = sized_of(&next);
                let fresh = next_sized.iter().any(|n| {
                    !sized.contains(n) && next.iter().any(|c| &c.prop == n && c.nd_base.is_some())
                });
                cur = next;
                sized = next_sized;
                if fresh {
                    continue;
                }
            }
            solves += 1;
            match self.rand_csp_solve_sized(
                handle,
                constraints,
                constraint_depth,
                rand_props,
                signed_props,
                enum_props,
                &cur,
                array_enums,
                strict,
                subs,
            ) {
                CspOutcome::Unsat | CspOutcome::GaveUp => {}
                o => return o,
            }
        }
        CspOutcome::GaveUp
    }

    /// `colls` with the rows of every member over unpacked rows
    /// (`RandColl::row_tail`) at its current size: a fixed row as a fixed
    /// array, a dynamic row as a dynamic array, each keyed under the
    /// member's multi-dimensional shape (`nd_base`).
    pub(in crate::compiler::simulator) fn csp_expand_rows(
        &self,
        colls: &[RandColl],
    ) -> Vec<RandColl> {
        let mut out = Vec::with_capacity(colls.len());
        for c in colls {
            out.push(c.clone());
            let Some(tail) = &c.row_tail else {
                continue;
            };
            let shape = self.csp_row_owner_shape(c);
            let n = self.get_queue_size(&c.scoped) as i64;
            match tail {
                RowTail::Fixed(inner) => {
                    let Some(&(lo, hi)) = inner.last() else {
                        continue;
                    };
                    let mut prefixes: Vec<String> = (0..n).map(|i| format!("[{}]", i)).collect();
                    for &(l, r) in &inner[..inner.len() - 1] {
                        prefixes = prefixes
                            .iter()
                            .flat_map(|p| (l..=r).map(move |j| format!("{}[{}]", p, j)))
                            .collect();
                    }
                    for p in prefixes {
                        out.push(RandColl {
                            prop: format!("{}{}", c.prop, p),
                            scoped: format!("{}{}", c.scoped, p),
                            kind: CollKind::Fixed,
                            width: c.width,
                            lo,
                            hi,
                            nested: false,
                            is_object_elem: c.is_object_elem,
                            nd_base: Some((c.prop.clone(), shape.clone())),
                            row_tail: None,
                            member: None,
                        });
                    }
                }
                RowTail::Dyn => {
                    for i in 0..n {
                        out.push(RandColl {
                            prop: format!("{}[{}]", c.prop, i),
                            scoped: format!("{}[{}]", c.scoped, i),
                            kind: CollKind::Dyn,
                            width: c.width,
                            lo: 0,
                            hi: 0,
                            nested: false,
                            is_object_elem: c.is_object_elem,
                            nd_base: Some((c.prop.clone(), shape.clone())),
                            row_tail: None,
                            member: None,
                        });
                    }
                }
            }
        }
        out
    }

    /// The multi-dimensional shape the joint solver walks for a member over
    /// unpacked rows: its current size, then a fixed row's dimensions or
    /// the longest dynamic row.
    pub(in crate::compiler::simulator) fn csp_row_owner_shape(
        &self,
        c: &RandColl,
    ) -> Vec<(i64, i64)> {
        let n = self.get_queue_size(&c.scoped) as i64;
        let mut shape = vec![(0, n - 1)];
        match &c.row_tail {
            Some(RowTail::Fixed(inner)) => shape.extend_from_slice(inner),
            Some(RowTail::Dyn) => {
                let m = (0..n)
                    .map(|i| self.get_queue_size(&format!("{}[{}]", c.scoped, i)) as i64)
                    .max()
                    .unwrap_or(0);
                shape.push((0, m - 1));
            }
            None => {}
        }
        shape
    }

    /// Whether the size phase decides the size of `c`: a dynamic array of
    /// values whose `size()` some constraint reads.
    pub(in crate::compiler::simulator) fn csp_size_solved(
        c: &RandColl,
        constraints: &[ClassConstraint],
    ) -> bool {
        if c.kind != CollKind::Dyn || (c.nested && c.row_tail.is_none()) || c.is_object_elem {
            return false;
        }
        // §7.4.5: a row (`a[1]` of `T a[][]`) is named by an index on its
        // base: a literal one equal to the row's, or a loop variable.
        let row_index: Option<(&str, i64)> = c.nd_base.as_ref().and_then(|(b, _)| {
            let i = c
                .prop
                .strip_prefix(b.as_str())?
                .strip_prefix('[')?
                .strip_suffix(']')?;
            Some((b.as_str(), i.parse().ok()?))
        });
        let names = |e: &Expression| match &Self::unparen(e).kind {
            ExprKind::Ident(h) if row_index.is_none() => {
                h.root.is_none()
                    && h.path.last().is_some_and(|s| s.name.name == c.prop)
                    && h.path.iter().all(|s| s.selects.is_empty())
                    && h.path.len() <= 2
            }
            ExprKind::MemberAccess { member, .. } if row_index.is_none() => member.name == c.prop,
            ExprKind::Index { expr, index } => row_index.is_some_and(|(b, i)| {
                let base_ok = match &Self::unparen(expr).kind {
                    ExprKind::Ident(h) => {
                        h.root.is_none()
                            && h.path.last().is_some_and(|s| s.name.name == b)
                            && h.path.iter().all(|s| s.selects.is_empty())
                            && h.path.len() <= 2
                    }
                    ExprKind::MemberAccess { member, .. } => member.name == b,
                    _ => false,
                };
                let idx_ok = match &Self::unparen(index).kind {
                    ExprKind::Number(_) => {
                        crate::compiler::elaborate::const_eval_i64_with_params(index, None)
                            .is_some_and(|k| k == i)
                    }
                    _ => true,
                };
                base_ok && idx_ok
            }),
            _ => false,
        };
        let mut hit = false;
        let mut look = |e: &Expression| {
            Self::walk_operands(e, &mut |x| {
                let recv = match &x.kind {
                    ExprKind::Call { func, args } if args.is_empty() => match &func.kind {
                        ExprKind::MemberAccess { expr, member } if member.name == "size" => {
                            Some(expr)
                        }
                        _ => None,
                    },
                    _ => None,
                };
                if recv.is_some_and(|r| names(r)) {
                    hit = true;
                }
            })
        };
        for con in constraints {
            for it in &con.items {
                Self::csp_item_walk(it, &mut |i| match i {
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
                    _ => {}
                });
            }
        }
        hit
    }

    /// §18.5.8.1: the sizes of the arrays in `sized`, solved with the other
    /// variables (including `subs`), every array of them a size variable
    /// without elements. What reads the elements of such an array is left out
    /// (a relaxation), so Unsat here is final unless a sub-object's own
    /// dynamic array kept a fixed trial size (`SubDynamicUnsat`).
    #[allow(clippy::too_many_arguments, clippy::type_complexity)]
    fn csp_size_phase<'c>(
        &mut self,
        handle: usize,
        constraints: &[ClassConstraint],
        constraint_depth: &[usize],
        rand_props: &[(String, u32)],
        signed_props: &HashSet<String>,
        enum_props: &HashMap<String, String>,
        colls: &'c [RandColl],
        array_enums: &HashMap<String, String>,
        sized: &HashSet<String>,
        subs: &[(String, usize)],
    ) -> Result<(Vec<(&'c RandColl, u64)>, Csp, St), SizePhaseErr> {
        let unread = self.csp_unread_with_subs(constraints, rand_props, colls, subs);
        let mut csp = self
            .csp_vars(
                handle,
                rand_props,
                signed_props,
                enum_props,
                colls,
                array_enums,
                &unread,
                !subs.is_empty(),
                sized,
            )
            .ok_or(SizePhaseErr::Outcome(CspOutcome::NotApplicable))?;
        csp.size_phase = true;
        let sub_join = self
            .csp_join_subs(&mut csp, constraints, subs, true, false)
            .ok_or(SizePhaseErr::Outcome(CspOutcome::NotApplicable))?;
        let constraints: &[ClassConstraint] = sub_join.constraints.as_ref();
        let sub_dynamic = sub_join.sub_dynamic;
        self.csp_split_slices(&mut csp, constraints);
        if csp.wide_fail {
            return Err(SizePhaseErr::Outcome(CspOutcome::NotApplicable));
        }
        for (v, val) in &sub_join.held_wide {
            Self::csp_pin_wide_var(&mut csp, *v, val);
        }
        self.csp_translate(&mut csp, constraints, constraint_depth)
            .ok_or(SizePhaseErr::Outcome(CspOutcome::NotApplicable))?;
        let st = match self.csp_find(&csp) {
            Ok(st) => st,
            // §18.5.14: a preference that conflicts with the hard
            // constraints is dropped; the sizes then follow the hard ones.
            Err(CspOutcome::Unsat) if !csp.soft_priority.is_empty() => {
                csp.soft_enabled = Some(HashSet::default());
                self.csp_translate(&mut csp, constraints, constraint_depth)
                    .ok_or(SizePhaseErr::Outcome(CspOutcome::NotApplicable))?;
                match self.csp_find(&csp) {
                    Ok(st) => st,
                    Err(CspOutcome::Unsat) if sub_dynamic => {
                        return Err(SizePhaseErr::SubDynamicUnsat);
                    }
                    Err(o) => return Err(SizePhaseErr::Outcome(o)),
                }
            }
            Err(CspOutcome::Unsat) if sub_dynamic => return Err(SizePhaseErr::SubDynamicUnsat),
            Err(o) => return Err(SizePhaseErr::Outcome(o)),
        };
        let mut out = Vec::new();
        for c in colls {
            if let Some(&sv) = csp.sizes.get(&c.prop) {
                let n = st
                    .fixed(sv)
                    .ok_or(SizePhaseErr::Outcome(CspOutcome::GaveUp))?;
                out.push((c, n.max(0) as u64));
            }
        }
        Ok((out, csp, st))
    }

    /// §18.4/§18.5.9: wide (>64-bit) rand variables not read by `constraints`
    /// or by any joined sub-object's active constraints (e.g., back-references).
    fn csp_unread_with_subs(
        &self,
        constraints: &[ClassConstraint],
        rand_props: &[(String, u32)],
        colls: &[RandColl],
        subs: &[(String, usize)],
    ) -> HashSet<String> {
        if subs.is_empty() {
            Self::csp_unread(constraints, rand_props, colls)
        } else {
            let mut all_cons: Vec<ClassConstraint> = constraints.to_vec();
            for &(_, sub) in subs {
                all_cons.extend(self.active_constraints(sub));
            }
            Self::csp_unread(&all_cons, rand_props, colls)
        }
    }

    /// §18.4: the rand variables wider than 64 bits that no constraint
    /// reads (or reads only through `size()`).
    fn csp_unread(
        constraints: &[ClassConstraint],
        rand_props: &[(String, u32)],
        colls: &[RandColl],
    ) -> HashSet<String> {
        if rand_props.iter().any(|(_, w)| *w > 64) || colls.iter().any(|c| c.width > 64) {
            let names: Vec<&str> = rand_props
                .iter()
                .filter(|(_, w)| *w > 64)
                .map(|(n, _)| n.as_str())
                .chain(
                    colls
                        .iter()
                        .filter(|c| c.width > 64)
                        .map(|c| c.prop.as_str()),
                )
                .collect();
            Self::csp_wide_unread(constraints, &names)
        } else {
            HashSet::default()
        }
    }

    /// Root member name of a solver/property reference: `x`, `this.x`,
    /// `s.f`, `a[i]`, `a[i].f`.
    fn csp_cache_root(e: &Expression) -> Option<&str> {
        match &Self::unparen(e).kind {
            ExprKind::Ident(h) if h.root.is_none() => {
                let segs = &h.path;
                let first = if segs.len() > 1 && segs[0].name.name == "this" {
                    &segs[1]
                } else {
                    segs.first()?
                };
                Some(first.name.name.as_str())
            }
            ExprKind::MemberAccess { expr, member } => {
                if matches!(Self::unparen(expr).kind, ExprKind::This) {
                    Some(member.name.as_str())
                } else {
                    Self::csp_cache_root(expr)
                }
            }
            ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => {
                Self::csp_cache_root(expr)
            }
            _ => None,
        }
    }

    /// System functions whose value depends only on their arguments.
    fn csp_cache_pure_sys(name: &str) -> bool {
        matches!(
            name,
            "$clog2"
                | "$bits"
                | "$signed"
                | "$unsigned"
                | "$countones"
                | "$countbits"
                | "$onehot"
                | "$onehot0"
                | "$isunknown"
                | "$size"
                | "$high"
                | "$low"
                | "$left"
                | "$right"
                | "$increment"
                | "$dimensions"
                | "$unpacked_dimensions"
                | "$__xz_type_cast"
        )
    }

    /// `e` reads no solver variable and no `foreach` index, and calls
    /// nothing whose value could depend on more than its arguments: its
    /// value can be evaluated on its own, before the translation.
    fn csp_cache_closed(e: &Expression, solver: &HashSet<String>, locals: &[String]) -> bool {
        let mut ok = true;
        crate::type_lint::visit_expr(e, &mut |x| match &x.kind {
            ExprKind::Call { .. }
            | ExprKind::WithClause { .. }
            | ExprKind::RandomizeWith { .. }
            | ExprKind::AssignExpr { .. }
            | ExprKind::Matches { .. }
            | ExprKind::StreamOp { .. }
            | ExprKind::AssignmentPattern(_)
            | ExprKind::TypedPattern { .. }
            | ExprKind::Tagged { .. }
            | ExprKind::ShallowCopy { .. }
            | ExprKind::SvaClocked { .. } => ok = false,
            ExprKind::SystemCall { name, .. } if !Self::csp_cache_pure_sys(name) => ok = false,
            ExprKind::Ident(_) | ExprKind::MemberAccess { .. } => {
                if let Some(r) = Self::csp_cache_root(x)
                    && (solver.contains(r) || locals.iter().any(|l| l == r))
                {
                    ok = false;
                }
            }
            _ => {}
        });
        ok
    }

    /// Classify every value `it` reads for the CSP cache: solver variables
    /// and `foreach` indices are the search's own; a bare own property is
    /// in the configuration snapshot; any other variable-free operand is
    /// pushed to `ext`, to be evaluated and compared on every call. False
    /// when `it` reads something neither covers.
    fn csp_cache_reads<'e>(
        it: &'e ConstraintItem,
        solver: &HashSet<String>,
        own: &HashSet<String>,
        consts: &dyn Fn(&str) -> bool,
        locals: &mut Vec<String>,
        ext: &mut Vec<&'e Expression>,
    ) -> bool {
        use crate::ast::decl::ConstraintRange;
        match it {
            ConstraintItem::Expr(e) => {
                Self::csp_cache_expr(e, false, solver, own, consts, locals, ext)
            }
            ConstraintItem::Inside {
                expr,
                range,
                dist_weights,
                ..
            } => {
                Self::csp_cache_expr(expr, false, solver, own, consts, locals, ext)
                    && range.iter().all(|r| match r {
                        ConstraintRange::Value(v) => {
                            Self::csp_cache_expr(v, true, solver, own, consts, locals, ext)
                        }
                        ConstraintRange::Range { lo, hi } => {
                            Self::csp_cache_expr(lo, false, solver, own, consts, locals, ext)
                                && Self::csp_cache_expr(hi, false, solver, own, consts, locals, ext)
                        }
                    })
                    && dist_weights.iter().flatten().all(|w| {
                        let (DistWeight::Each(w) | DistWeight::Total(w)) = w;
                        Self::csp_cache_expr(w, false, solver, own, consts, locals, ext)
                    })
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => {
                Self::csp_cache_expr(condition, false, solver, own, consts, locals, ext)
                    && Self::csp_cache_reads(constraint, solver, own, consts, locals, ext)
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                Self::csp_cache_expr(condition, false, solver, own, consts, locals, ext)
                    && Self::csp_cache_reads(then_item, solver, own, consts, locals, ext)
                    && else_item
                        .as_ref()
                        .is_none_or(|e| Self::csp_cache_reads(e, solver, own, consts, locals, ext))
            }
            ConstraintItem::Foreach {
                array, vars, item, ..
            } => {
                // Only a solver collection's shape is in the key.
                if !Self::csp_cache_root(array).is_some_and(|r| solver.contains(r)) {
                    return false;
                }
                let n = locals.len();
                locals.extend(vars.iter().flatten().map(|v| v.name.clone()));
                let ok = Self::csp_cache_reads(item, solver, own, consts, locals, ext);
                locals.truncate(n);
                ok
            }
            ConstraintItem::Solve { .. } => true,
            ConstraintItem::Soft(i) => Self::csp_cache_reads(i, solver, own, consts, locals, ext),
            ConstraintItem::Block(items) => items
                .iter()
                .all(|i| Self::csp_cache_reads(i, solver, own, consts, locals, ext)),
            ConstraintItem::Unique { exprs, .. } => exprs
                .iter()
                .all(|e| Self::csp_cache_expr(e, true, solver, own, consts, locals, ext)),
            #[allow(unreachable_patterns)]
            _ => false,
        }
    }

    /// `csp_cache_reads` for one expression. `set` marks a member of an
    /// `inside` / `unique` list, where a bare name may be a whole unpacked
    /// array (whose value is no single `Value`).
    fn csp_cache_expr<'e>(
        e: &'e Expression,
        set: bool,
        solver: &HashSet<String>,
        own: &HashSet<String>,
        consts: &dyn Fn(&str) -> bool,
        locals: &mut Vec<String>,
        ext: &mut Vec<&'e Expression>,
    ) -> bool {
        let e = Self::unparen(e);
        if matches!(
            e.kind,
            ExprKind::Number(_)
                | ExprKind::StringLiteral(_)
                | ExprKind::TypeLiteral(_)
                | ExprKind::Dollar
                | ExprKind::Null
                | ExprKind::Empty
        ) {
            return true;
        }
        let bare_own = match &e.kind {
            ExprKind::Ident(h) => {
                h.root.is_none()
                    && h.path.len() == 1
                    && h.path[0].selects.is_empty()
                    && own.contains(&h.path[0].name.name)
            }
            _ => false,
        };
        if Self::csp_cache_closed(e, solver, locals) {
            // A parameter or enum label is one constant value; any other
            // bare name in a set may be a whole unpacked array.
            let constant = matches!(&e.kind, ExprKind::Ident(h)
                if h.path.len() == 1 && h.path[0].selects.is_empty() && consts(&h.path[0].name.name));
            if set
                && !constant
                && matches!(e.kind, ExprKind::Ident(_) | ExprKind::MemberAccess { .. })
            {
                return false;
            }
            if bare_own {
                return true;
            }
            ext.push(e);
            return true;
        }
        let mut sub = |x: &'e Expression, set: bool, locals: &mut Vec<String>| {
            Self::csp_cache_expr(x, set, solver, own, consts, locals, ext)
        };
        let root = Self::csp_cache_root(e);
        let in_solver = root.is_some_and(|r| solver.contains(r));
        match &e.kind {
            ExprKind::Ident(h) => {
                if in_solver {
                    h.path
                        .iter()
                        .flat_map(|s| s.selects.iter())
                        .all(|x| sub(x, false, locals))
                } else {
                    // A bare `foreach` index.
                    h.root.is_none()
                        && h.path.len() == 1
                        && h.path[0].selects.is_empty()
                        && locals.iter().any(|l| *l == h.path[0].name.name)
                }
            }
            ExprKind::MemberAccess { expr, .. } => {
                in_solver
                    && (matches!(Self::unparen(expr).kind, ExprKind::This)
                        || sub(expr, false, locals))
            }
            // A select picked by a `foreach` index is only followed into a
            // solver variable: the element of any other array is no value
            // the snapshot holds.
            ExprKind::Index { expr, index } => {
                in_solver && sub(expr, false, locals) && sub(index, false, locals)
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                in_solver
                    && sub(expr, false, locals)
                    && sub(left, false, locals)
                    && sub(right, false, locals)
            }
            ExprKind::Unary { operand, .. } => sub(operand, false, locals),
            ExprKind::Binary { left, right, .. } => {
                sub(left, false, locals) && sub(right, false, locals)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                sub(condition, false, locals)
                    && sub(then_expr, false, locals)
                    && sub(else_expr, false, locals)
            }
            ExprKind::Concatenation(xs) => xs.iter().all(|x| sub(x, false, locals)),
            ExprKind::Replication { count, exprs } => {
                sub(count, false, locals) && exprs.iter().all(|x| sub(x, false, locals))
            }
            ExprKind::Range(a, b) => sub(a, false, locals) && sub(b, false, locals),
            ExprKind::Inside { expr, ranges } => {
                sub(expr, false, locals) && ranges.iter().all(|x| sub(x, true, locals))
            }
            ExprKind::SystemCall { name, args } if Self::csp_cache_pure_sys(name) => {
                args.iter().all(|x| sub(x, false, locals))
            }
            // A reduction or `size()` of a solver collection.
            ExprKind::Call { func, args } => match &Self::unparen(func).kind {
                ExprKind::MemberAccess { expr, member }
                    if matches!(
                        member.name.as_str(),
                        "size" | "sum" | "product" | "and" | "or" | "xor"
                    ) && Self::csp_cache_root(expr).is_some_and(|r| solver.contains(r)) =>
                {
                    sub(expr, false, locals) && args.iter().all(|x| sub(x, false, locals))
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// Structural + configuration fingerprint for the per-class CSP cache.
    /// Returns `None` when the solve is not safely cacheable: sub-objects
    /// (joint path re-builds the problem), or a random object with any
    /// non-rand, non-solver property that is not a plain scalar (an object
    /// handle / collection / unpacked aggregate stored as configuration —
    /// comparing its full `Value` each call would be too costly to be worth
    /// the cache, so we conservatively rebuild).
    /// The returned key is a deterministic string over the class name,
    /// rand properties (name/width/signed/enum), collection shapes and the
    /// constraint items' structural identity (source index + span). The
    /// value list is the live snapshot of the scalar non-rand configuration
    /// props the constraints fold in (`csp_const`), which must be
    /// byte-identical for a cached graph to be re-usable. Every other value
    /// the translation folds in (a module or package variable, a static
    /// property, another object's member, a parameter or an enum label) is
    /// evaluated now and appended to the snapshot (`csp_cache_reads`); a
    /// constraint that reads anything that cannot be captured so (a
    /// function call, a non-solver array element picked by a `foreach`
    /// index, a whole unpacked array) is not cached.
    #[allow(clippy::too_many_arguments)]
    fn rand_csp_cache_fp(
        &mut self,
        handle: usize,
        constraints: &[ClassConstraint],
        constraint_depth: &[usize],
        rand_props: &[(String, u32)],
        signed_props: &HashSet<String>,
        enum_props: &HashMap<String, String>,
        colls: &[RandColl],
        _array_enums: &HashMap<String, String>,
    ) -> Option<(String, Box<[(String, Value)]>)> {
        use std::fmt::Write;
        let inst = self.heap.get(handle)?.as_ref()?;
        let mut solver: HashSet<String> = HashSet::default();
        for (n, _) in rand_props {
            solver.insert(n.clone());
        }
        for c in colls {
            solver.insert(c.prop.clone());
        }
        // Non-rand, non-solver configuration props must all be plain scalars
        // for the cache to be engaged; anything else -> never cache.
        let mut config: Vec<(String, Value)> = Vec::new();
        for (n, v) in &inst.properties {
            if solver.contains(n) {
                continue;
            }
            if v.is_real || v.width > 64 {
                return None;
            }
            config.push((n.clone(), v.clone()));
        }
        config.sort_by(|a, b| a.0.cmp(&b.0));
        let mut key = String::new();
        let _ = write!(key, "cls{}", inst.class_name);
        // The specialization, type bindings and creation scope an object
        // carries all steer how its constraint names and types resolve.
        let mut binds: Vec<_> = inst.type_bindings.iter().collect();
        binds.sort();
        let _ = write!(
            key,
            "|s:{:?}|t:{:?}|cs:{}",
            inst.spec, binds, inst.creation_scope
        );
        let own: HashSet<String> = inst.properties.keys().cloned().collect();
        let mut ext: Vec<&Expression> = Vec::new();
        let mut locals: Vec<String> = Vec::new();
        let params = &self.module.parameters;
        let consts = |n: &str| params.contains_key(n);
        for con in constraints {
            for it in &con.items {
                if !Self::csp_cache_reads(it, &solver, &own, &consts, &mut locals, &mut ext) {
                    return None;
                }
            }
        }
        let env = Env::default();
        for (i, e) in ext.iter().enumerate() {
            let v = self.csp_const_any(e, &env);
            config.push((format!("@{i}"), v));
        }
        for (n, w) in rand_props {
            let s = signed_props.contains(n) as u8;
            let e = enum_props.get(n).cloned().unwrap_or_default();
            let _ = write!(key, "|r:{n}:{w}:{s}:{e}");
        }
        for c in colls {
            if matches!(c.kind, CollKind::Dyn | CollKind::Assoc) {
                // The current size / keys of a dynamic or associative array are
                // value-dependent for the translate-time graph and retaining a
                // large foreach/grow graph across solves costs more in memory
                // (GC pressure) than it saves; exclude to stay airtight and cheap.
                return None;
            }
            let _ = write!(
                key,
                "|c:{}:{:?}:{}:{}:{}:{}",
                c.prop, c.kind, c.width, c.lo, c.hi, c.nested
            );
        }
        for (i, con) in constraints.iter().enumerate() {
            // A dummy/zero span marks a SYNTHESIZED block (inline `with {…}`, a
            // pinned `__dealt__` item, or a sub-object join) whose contents vary
            // per call and are not captured by the block name+span. Such blocks
            // are not structurally stable, so the whole class is not cacheable.
            if con.span.start == 0 && con.span.end == 0 {
                return None;
            }
            let depth = constraint_depth.get(i).copied().unwrap_or(0);
            let _ = write!(
                key,
                "|x{}:{}:{}({})",
                con.name.name,
                con.span.start as usize,
                con.span.end - con.span.start,
                depth
            );
        }
        Some((key, config.into_boxed_slice()))
    }

    /// The joint solve with every dynamic array at its current size.
    #[allow(clippy::too_many_arguments)]
    fn rand_csp_solve_sized(
        &mut self,
        handle: usize,
        constraints: &[ClassConstraint],
        constraint_depth: &[usize],
        rand_props: &[(String, u32)],
        signed_props: &HashSet<String>,
        enum_props: &HashMap<String, String>,
        colls: &[RandColl],
        array_enums: &HashMap<String, String>,
        strict: bool,
        subs: &[(String, usize)],
    ) -> CspOutcome {
        // §18.4: a rand variable wider than 64 bits that no constraint reads
        // (or reads only through `size()`) takes any value of its width; it
        // stays out of the solve instead of keeping the whole class out.
        // §18.5.9: include active constraints of joined sub-objects so a wide
        // variable read only through a back-reference (`item.wide[95:64]`) is
        // kept in the solve.
        let unread = self.csp_unread_with_subs(constraints, rand_props, colls, subs);
        // Per-class memo cache: for a fixed class / constraint set / config
        // the whole `csp_vars` + `csp_split_slices` + `csp_translate` graph
        // is value-independent, so a cached `Csp` is reused read-only and
        // only the search (`csp_run`) re-runs against the current object.
        // Used only on the no-sub-object path; dynamic/associative arrays
        // and any non-scalar configuration make the fingerprint builder
        // return None (conservative fallback to the current full build).
        let cached: Option<(String, Box<[(String, Value)]>)> = if subs.is_empty() {
            self.rand_csp_cache_fp(
                handle,
                constraints,
                constraint_depth,
                rand_props,
                signed_props,
                enum_props,
                colls,
                array_enums,
            )
        } else {
            None
        };
        if let Some((key, live_config)) = &cached {
            let reuse = self
                .csp_cache
                .borrow()
                .get(key)
                .map(|m| {
                    m.config.len() == live_config.len()
                        && m.config.iter().zip(live_config.iter()).all(
                            |((a, va), (b, vb))| a == b && va == vb,
                        )
                })
                .unwrap_or(false);
            if reuse {
                let mut memo =
                    self.csp_cache.borrow_mut().remove(key).unwrap();
                memo.csp.handle = handle;
                memo.csp.draw_free = strict;
                let out = self.csp_run(&memo.csp, constraints, colls);
                let is_soft = !memo.csp.soft_priority.is_empty();
                self.csp_cache.borrow_mut().insert(key.clone(), memo);
                if is_soft {
                    return CspOutcome::GaveUp;
                }
                return out;
            }
        }
        let Some(mut csp) = self.csp_vars(
            handle,
            rand_props,
            signed_props,
            enum_props,
            colls,
            array_enums,
            &unread,
            !subs.is_empty(),
            &HashSet::default(),
        ) else {
            return CspOutcome::NotApplicable;
        };
        csp.draw_free = strict;
        let Some(sub_join) = self.csp_join_subs(&mut csp, constraints, subs, false, true) else {
            return CspOutcome::NotApplicable;
        };
        let sub_dynamic = sub_join.sub_dynamic;
        let constraints: &[ClassConstraint] = sub_join.constraints.as_ref();
        let check: &[ClassConstraint] = sub_join.check.as_deref().unwrap_or(constraints);
        self.csp_split_slices(&mut csp, constraints);
        if csp.wide_fail {
            return CspOutcome::NotApplicable;
        }
        if self
            .csp_translate(&mut csp, constraints, constraint_depth)
            .is_none()
        {
            return CspOutcome::NotApplicable;
        }
        if strict && csp.opaque {
            return CspOutcome::GaveUp;
        }
        let out = self.csp_run(&csp, check, colls);
        // The caller sized a sub-object's dynamic arrays in its own trial.
        // Refuting those particular sizes does not refute another size the
        // next trial may draw (§18.5.8.1).
        if sub_dynamic && matches!(out, CspOutcome::Unsat) {
            return CspOutcome::GaveUp;
        }
        if !matches!(out, CspOutcome::Unsat) || csp.soft_priority.is_empty() {
            // Freshly built graph: cache it for reuse on identical future
            // calls (only when cache-eligible, non-soft, and the solve ended
            // in a state the graph can be shared from). `csp_run` left `csp`
            // untouched, so the stored graph is the exact translated problem.
            if let Some((key, live_config)) = &cached {
                if csp.soft_priority.is_empty() {
                    let mut m = self.csp_cache.borrow_mut();
                    // Bound the table: a long run with many distinct rand
                    // classes must not grow it without limit. Past the cap,
                    // drop everything (an occasional rebuild is far cheaper
                    // than unbounded retained graphs).
                    if m.len() >= 512 {
                        m.clear();
                    }
                    m.insert(
                        key.clone(),
                        RandCspMemo {
                            csp,
                            config: live_config.clone(),
                        },
                    );
                    return out;
                }
            }
            return out;
        }
        // A preference is discarded only after proving a conflict with hard
        // constraints or already accepted, higher-priority preferences.
        let mut priority = csp.soft_priority.clone();
        priority.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(b.2.cmp(&a.2)));
        csp.soft_enabled = Some(HashSet::default());
        if self
            .csp_translate(&mut csp, constraints, constraint_depth)
            .is_none()
        {
            return CspOutcome::NotApplicable;
        }
        let hard = self.csp_run(&csp, check, colls);
        if !matches!(hard, CspOutcome::Sat) {
            return hard;
        }
        for (_, _, id) in priority {
            csp.soft_enabled.as_mut().unwrap().insert(id);
            if self
                .csp_translate(&mut csp, constraints, constraint_depth)
                .is_none()
            {
                return CspOutcome::NotApplicable;
            }
            match self.csp_run(&csp, check, colls) {
                CspOutcome::Sat => {}
                CspOutcome::Unsat => {
                    csp.soft_enabled.as_mut().unwrap().remove(&id);
                }
                other => return other,
            }
        }
        CspOutcome::Sat
    }

    /// §18.12: solve `std::randomize(…) with {items}` jointly. `scalars`
    /// are (name, lvalue, width, signed, enum values); `arrays` are (name
    /// as the constraints write it, storage name, element indices, width,
    /// signed, enum values) of collections already sized. The solution is written to the caller's variables and checked
    /// with the inline-constraint checker.
    #[allow(clippy::type_complexity)]
    pub(in crate::compiler::simulator) fn rand_csp_solve_scope(
        &mut self,
        items: &[ConstraintItem],
        scalars: &[(String, Expression, u32, bool, Option<Vec<u64>>)],
        arrays: &[(String, String, Vec<i64>, u32, bool, Option<Vec<u64>>)],
    ) -> CspOutcome {
        match self.csp_scope(items, scalars, arrays, &HashSet::default()) {
            Ok(_) => CspOutcome::Sat,
            Err(o) => o,
        }
    }

    /// §18.5.8.1: the sizes of the arrays in `sized` (named as the
    /// constraints write them) for `std::randomize`, solved before their
    /// elements (see `csp_size_phase`).
    #[allow(clippy::type_complexity)]
    pub(in crate::compiler::simulator) fn csp_scope_sizes(
        &mut self,
        items: &[ConstraintItem],
        scalars: &[(String, Expression, u32, bool, Option<Vec<u64>>)],
        arrays: &[(String, String, Vec<i64>, u32, bool, Option<Vec<u64>>)],
        sized: &HashSet<String>,
    ) -> Result<Vec<(String, u64)>, CspOutcome> {
        self.csp_scope(items, scalars, arrays, sized)
    }
}
