//! Constraint items: prefixes, sources, translation, `foreach`, `dist`, conditionals and
//! booleans.

use super::*;

impl Simulator {
    /// Visit `it` and every item nested in it.

    pub(in crate::compiler::simulator) fn csp_item_walk(
        it: &ConstraintItem,
        f: &mut dyn FnMut(&ConstraintItem),
    ) {
        f(it);
        match it {
            ConstraintItem::Implication { constraint, .. } => Self::csp_item_walk(constraint, f),
            ConstraintItem::IfElse {
                then_item,
                else_item,
                ..
            } => {
                Self::csp_item_walk(then_item, f);
                if let Some(e) = else_item {
                    Self::csp_item_walk(e, f);
                }
            }
            ConstraintItem::Foreach { item, .. } | ConstraintItem::Soft(item) => {
                Self::csp_item_walk(item, f)
            }
            ConstraintItem::Block(items) => items.iter().for_each(|i| Self::csp_item_walk(i, f)),
            _ => {}
        }
    }

    /// Rewrite a sub-object's constraint item so its member references go
    /// through the enclosing object's member `prop` (`x` becomes `prop.x`).
    /// False when the item holds something that cannot be rewritten so.
    pub(super) fn csp_prefix_item(
        it: &mut ConstraintItem,
        prop: &str,
        members: &HashSet<String>,
        bound: &mut Vec<String>,
    ) -> bool {
        use crate::ast::decl::ConstraintRange;
        match it {
            ConstraintItem::Expr(e) => Self::csp_prefix_expr(e, prop, members, bound),
            ConstraintItem::Inside { expr, range, .. } => {
                Self::csp_prefix_expr(expr, prop, members, bound)
                    && range.iter_mut().all(|r| match r {
                        ConstraintRange::Value(v) => Self::csp_prefix_expr(v, prop, members, bound),
                        ConstraintRange::Range { lo, hi } => {
                            Self::csp_prefix_expr(lo, prop, members, bound)
                                && Self::csp_prefix_expr(hi, prop, members, bound)
                        }
                    })
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => {
                Self::csp_prefix_expr(condition, prop, members, bound)
                    && Self::csp_prefix_item(constraint, prop, members, bound)
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                Self::csp_prefix_expr(condition, prop, members, bound)
                    && Self::csp_prefix_item(then_item, prop, members, bound)
                    && else_item
                        .as_mut()
                        .is_none_or(|e| Self::csp_prefix_item(e, prop, members, bound))
            }
            ConstraintItem::Foreach {
                array, vars, item, ..
            } => {
                if !Self::csp_prefix_expr(array, prop, members, bound) {
                    return false;
                }
                let mark = bound.len();
                bound.extend(vars.iter().flatten().map(|v| v.name.clone()));
                let ok = Self::csp_prefix_item(item, prop, members, bound);
                bound.truncate(mark);
                ok
            }
            ConstraintItem::Unique { exprs, .. } => exprs
                .iter_mut()
                .all(|e| Self::csp_prefix_expr(e, prop, members, bound)),
            ConstraintItem::Soft(inner) => Self::csp_prefix_item(inner, prop, members, bound),
            ConstraintItem::Block(items) => items
                .iter_mut()
                .all(|i| Self::csp_prefix_item(i, prop, members, bound)),
            // §18.5.10: an ordering among the sub-object's own members.
            ConstraintItem::Solve {
                before,
                after,
                before_paths,
                after_paths,
                ..
            } => {
                if before_paths.len() != before.len() || after_paths.len() != after.len() {
                    return false;
                }
                before_paths
                    .iter_mut()
                    .chain(after_paths.iter_mut())
                    .all(|e| Self::csp_prefix_expr(e, prop, members, bound))
            }
        }
    }

    fn csp_prefix_expr(
        e: &mut Expression,
        prop: &str,
        members: &HashSet<String>,
        bound: &[String],
    ) -> bool {
        // An element of an object array (`arr[k]`) is named by a path
        // segment of its own (see `obj`).
        let elem = prop.contains('[');
        let seg = |name: &str, span: crate::ast::Span| crate::ast::expr::HierPathSegment {
            name: crate::ast::Identifier {
                name: name.to_string(),
                span,
            },
            selects: Vec::new(),
        };
        let through = |name: &str, span: crate::ast::Span| {
            if elem {
                // `a[k]`, or `a[k].q[j]` for an element of an object array
                // that element holds: one segment per object.
                let mut path: Vec<_> = prop.split('.').map(|p| seg(p, span)).collect();
                path.push(seg(name, span));
                return Expression::new(
                    ExprKind::Ident(HierarchicalIdentifier {
                        root: None,
                        path,
                        span,
                        cached_signal_id: crate::ast::cache::SignalCache::new(None),
                        cached_resolved_name: crate::ast::cache::NameCache::new(),
                        bound: crate::ast::binding::Bound::UNSET,
                    }),
                    span,
                );
            }
            // `a.b` (a sub-object of a sub-object) is a member chain.
            let mut base = Expression::new(
                ExprKind::Ident(HierarchicalIdentifier {
                    root: None,
                    path: vec![seg(prop.split('.').next().unwrap_or(prop), span)],
                    span,
                    cached_signal_id: crate::ast::cache::SignalCache::new(None),
                    cached_resolved_name: crate::ast::cache::NameCache::new(),
                    bound: crate::ast::binding::Bound::UNSET,
                }),
                span,
            );
            for part in prop.split('.').skip(1) {
                base = Expression::new(
                    ExprKind::MemberAccess {
                        expr: Box::new(base),
                        member: crate::ast::Identifier {
                            name: part.to_string(),
                            span,
                        },
                    },
                    span,
                );
            }
            Expression::new(
                ExprKind::MemberAccess {
                    expr: Box::new(base),
                    member: crate::ast::Identifier {
                        name: name.to_string(),
                        span,
                    },
                },
                span,
            )
        };
        match &mut e.kind {
            ExprKind::Ident(h) => {
                if h.root.is_some() {
                    return false;
                }
                if h.path.len() == 1 && h.path[0].selects.is_empty() {
                    let n = h.path[0].name.name.clone();
                    if !bound.contains(&n) && members.contains(&n) {
                        *e = through(&n, e.span);
                    }
                    return true;
                }
                // §7.2.1/§11.5.1: a field or select of a member (`m.f`,
                // `m[i]`, `this.m.f[k]`), the selects rewritten too.
                if h.path[0].name.name == "this" && h.path.len() >= 2 {
                    h.path.remove(0);
                }
                for s in &mut h.path {
                    for x in &mut s.selects {
                        if !Self::csp_prefix_expr(x, prop, members, bound) {
                            return false;
                        }
                    }
                }
                if bound.contains(&h.path[0].name.name) || !members.contains(&h.path[0].name.name) {
                    return false;
                }
                let span = h.span;
                {
                    for (k, part) in prop.split('.').enumerate() {
                        h.path.insert(k, seg(part, span));
                    }
                }
                h.cached_signal_id.set(None);
                h.cached_resolved_name = crate::ast::cache::NameCache::new();
                e.cached_width.set(None);
                true
            }
            ExprKind::MemberAccess { expr, member } => {
                if matches!(expr.kind, ExprKind::This) && members.contains(&member.name) {
                    let n = member.name.clone();
                    *e = through(&n, e.span);
                    return true;
                }
                if matches!(expr.kind, ExprKind::This) {
                    return false;
                }
                Self::csp_prefix_expr(expr, prop, members, bound)
            }
            ExprKind::Number(_) | ExprKind::StringLiteral(_) | ExprKind::TypeLiteral(_) => true,
            ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                Self::csp_prefix_expr(operand, prop, members, bound)
            }
            ExprKind::Binary { left, right, .. }
            | ExprKind::Range(left, right)
            | ExprKind::Index {
                expr: left,
                index: right,
            } => {
                Self::csp_prefix_expr(left, prop, members, bound)
                    && Self::csp_prefix_expr(right, prop, members, bound)
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                Self::csp_prefix_expr(expr, prop, members, bound)
                    && Self::csp_prefix_expr(left, prop, members, bound)
                    && Self::csp_prefix_expr(right, prop, members, bound)
            }
            ExprKind::SystemCall { args, .. } => args
                .iter_mut()
                .all(|a| Self::csp_prefix_expr(a, prop, members, bound)),
            ExprKind::Call { func, args }
                if matches!(
                    &func.kind,
                    ExprKind::MemberAccess { member, .. }
                        if matches!(
                            member.name.as_str(),
                            "size" | "num" | "sum" | "and" | "or" | "xor" | "product" | "min" | "max"
                        )
                ) || matches!(
                    &func.kind,
                    ExprKind::Ident(h)
                        if h.path.len() >= 2
                            && h.path.last().is_some_and(|s| matches!(
                                s.name.name.as_str(),
                                "size" | "num" | "sum" | "and" | "or" | "xor" | "product" | "min" | "max"
                            ))
                ) =>
            {
                Self::csp_prefix_expr(func, prop, members, bound)
                    && args
                        .iter_mut()
                        .all(|a| Self::csp_prefix_expr(a, prop, members, bound))
            }
            ExprKind::Replication { count, exprs } => {
                Self::csp_prefix_expr(count, prop, members, bound)
                    && exprs
                        .iter_mut()
                        .all(|p| Self::csp_prefix_expr(p, prop, members, bound))
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::csp_prefix_expr(condition, prop, members, bound)
                    && Self::csp_prefix_expr(then_expr, prop, members, bound)
                    && Self::csp_prefix_expr(else_expr, prop, members, bound)
            }
            ExprKind::Inside { expr, ranges } => {
                Self::csp_prefix_expr(expr, prop, members, bound)
                    && ranges
                        .iter_mut()
                        .all(|r| Self::csp_prefix_expr(r, prop, members, bound))
            }
            ExprKind::Concatenation(parts) => parts
                .iter_mut()
                .all(|p| Self::csp_prefix_expr(p, prop, members, bound)),
            _ => false,
        }
    }

    // ---------------------------------------------------------------------
    // Translation
    // ---------------------------------------------------------------------

    pub(super) fn csp_src(
        &self,
        csp: &mut Csp,
        item: SrcItem,
        env: &Env,
        mut deps: Vec<usize>,
    ) -> usize {
        // The evaluator reads a counted variable itself, not its count.
        if !csp.pop_rev.is_empty() {
            let xs: Vec<usize> = deps
                .iter()
                .filter_map(|d| csp.pop_rev.get(d).copied())
                .collect();
            if !xs.is_empty() {
                deps.extend(xs);
                deps.sort_unstable();
                deps.dedup();
            }
        }
        // The evaluator reads a sum's terms, not its carry variables.
        if !csp.carries.is_empty() {
            let ts: Vec<usize> = csp
                .carries
                .iter()
                .filter(|c| deps.contains(&c.0) || deps.contains(&c.1))
                .flat_map(|c| c.2.iter().copied())
                .collect();
            if !ts.is_empty() {
                deps.extend(ts);
                deps.sort_unstable();
                deps.dedup();
            }
        }
        // §18.5.9: the evaluator reads an object-array element through its
        // select (`arr[k]`), not by the solver's one-segment name.
        let item = match item {
            SrcItem::Expr(e) => SrcItem::Expr(Self::csp_obj_real(&e).unwrap_or(e)),
            SrcItem::Item(it) => SrcItem::Item(Self::csp_obj_real_item(&it)),
        };
        let stale = csp.size_phase && Self::csp_src_sized(csp, &item);
        csp.srcs.push(Src {
            item,
            binds: env.binds.clone(),
            deps,
            stale,
        });
        csp.srcs.len() - 1
    }

    /// A node judged by the evaluator alone. None when its variables cannot
    /// be determined.
    pub(super) fn csp_eval_node(
        &mut self,
        csp: &mut Csp,
        item: SrcItem,
        env: &Env,
        neg: bool,
    ) -> Option<Node> {
        let mut deps = Vec::new();
        let complete = match &item {
            SrcItem::Expr(e) => self.csp_refs(csp, e, env, &mut deps),
            SrcItem::Item(it) => self.csp_item_refs(csp, it, env, &mut deps),
        };
        let mut complete = complete;
        if !csp.wide.is_empty() && self.csp_may_read_wide(csp, &item, env) {
            // Left to the evaluator, a relation on a whole wide variable
            // would wait for every segment to be guessed: not modelled. One
            // that reads fields or selects of it depends on their segments.
            let SrcItem::Expr(e) = &item else {
                return None;
            };
            deps.clear();
            if !self.csp_wide_deps(csp, e, env, &mut deps) {
                return None;
            }
            complete = true;
        }
        if !complete {
            // An opaque call may read any rand member.
            deps = (0..csp.vars.len()).collect();
        }
        if matches!(
            item,
            SrcItem::Item(ConstraintItem::Foreach { .. } | ConstraintItem::Unique { .. })
        ) {
            csp.opaque = true;
        }
        deps.sort_unstable();
        deps.dedup();
        let src = self.csp_src(csp, item, env, deps);
        Some(Node::Eval { src, neg })
    }

    pub(super) fn csp_item(
        &mut self,
        csp: &mut Csp,
        item: &ConstraintItem,
        env: &Env,
    ) -> Option<Node> {
        let r = self.csp_item_inner(csp, item, env);
        // The size phase solves a relaxation: what it cannot model (an
        // element of an array not sized yet) constrains nothing there.
        if r.is_none() && csp.size_phase {
            return Some(Node::True);
        }
        r
    }

    fn csp_item_inner(&mut self, csp: &mut Csp, item: &ConstraintItem, env: &Env) -> Option<Node> {
        match item {
            ConstraintItem::Expr(e) => self.csp_bool(csp, e, env, false),
            ConstraintItem::Inside {
                expr,
                range,
                is_dist,
                dist_weights,
                ..
            } => {
                // The weights of a wide operand are modelled only on one
                // segment (a field, an aligned select). Over a whole wide
                // variable the dist still constrains it to its items
                // (§18.5.4: the value is in the set of a nonzero weight);
                // the value is then drawn uniformly over them.
                let unweighted = *is_dist
                    && !csp.wide.is_empty()
                    && self.csp_wide_touch(csp, expr, env)
                    && !matches!(self.csp_ae(csp, expr, env), Some(Ae::Var(..)));
                let set: Vec<ConstraintRange> = if *is_dist {
                    // §18.5.4: a zero weight removes the value.
                    let mut kept = Vec::new();
                    for (k, r) in range.iter().enumerate() {
                        let w = match dist_weights.get(k).and_then(|w| w.as_ref()) {
                            Some(DistWeight::Each(e)) | Some(DistWeight::Total(e)) => {
                                self.csp_const(e, env).and_then(|v| v.to_u64())
                            }
                            None => Some(1),
                        };
                        if w != Some(0) {
                            kept.push(r.clone());
                        }
                    }
                    kept
                } else {
                    range.clone()
                };
                if unweighted
                    && let Some(n) = self.csp_wide_dist(csp, expr, range, dist_weights, env)
                {
                    return Some(n);
                }
                let n =
                    self.csp_inside(csp, expr, &set, env, false, SrcItem::Item(item.clone()))?;
                if *is_dist && !unweighted {
                    self.csp_dist(csp, expr, &range, dist_weights, env);
                }
                Some(n)
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => {
                let c = self.csp_bool(csp, condition, &env.optional_mask(), false)?;
                let then = self.csp_item(csp, constraint, &env.guarded(&c))?;
                self.csp_if(csp, condition, then, Node::True, env)
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                let c = self.csp_bool(csp, condition, &env.optional_mask(), false)?;
                let then = self.csp_item(csp, then_item, &env.guarded(&c))?;
                let els = match else_item {
                    Some(e) => {
                        let nc = self.csp_bool(csp, condition, &env.optional_mask(), true)?;
                        self.csp_item(csp, e, &env.guarded(&nc))?
                    }
                    None => Node::True,
                };
                self.csp_if(csp, condition, then, els, env)
            }
            ConstraintItem::Foreach {
                array,
                vars,
                item: body,
                ..
            } => {
                // §7.4.1/§12.7.3: a loop over a multi-dimensional packed
                // variable (`foreach (a.w[i, j])`) walks the labels of its
                // dimensions, one loop variable per dimension.
                if let Some(p) = self.csp_pdim_ref(csp, array, &mut |_, _| None) {
                    let dims = match p {
                        Some(p) if p.depth == 0 => csp.pdims.get(&p.v)?.clone(),
                        _ => return None,
                    };
                    if vars.len() > dims.len() {
                        return None;
                    }
                    let loops: Vec<(String, i64, i64)> = vars
                        .iter()
                        .zip(&dims)
                        .filter_map(|(v, &(l, r))| {
                            v.as_ref().map(|v| (v.name.clone(), l.min(r), l.max(r)))
                        })
                        .collect();
                    let count = loops
                        .iter()
                        .try_fold(1u64, |n, l| n.checked_mul((l.2 - l.1 + 1) as u64))?;
                    if count > 4096 {
                        return None;
                    }
                    let mut idx: Vec<i64> = loops.iter().map(|l| l.1).collect();
                    let mut out = Vec::new();
                    loop {
                        let mut e2 = env.clone();
                        for (l, &i) in loops.iter().zip(&idx) {
                            e2.binds.push((l.0.clone(), Self::signed_loop_val(i)));
                        }
                        match self.csp_item(csp, body, &e2)? {
                            Node::True => {}
                            n => out.push(n),
                        }
                        let mut k = loops.len();
                        loop {
                            if k == 0 {
                                return Some(Self::csp_and(out));
                            }
                            k -= 1;
                            idx[k] += 1;
                            if idx[k] <= loops[k].2 {
                                break;
                            }
                            idx[k] = loops[k].1;
                        }
                    }
                }
                // §7.4.5: a loop over a packed-array field (`foreach
                // (a.s.f[i])`) walks the labels of its first dimension.
                if let [Some(iv)] = vars.as_slice()
                    && !csp.field_elems.is_empty()
                    && let Some((_, all)) = self.csp_field_elem_all(csp, array, &mut |_, _| None)
                {
                    let mut out = Vec::with_capacity(all.len());
                    for (i, ..) in all {
                        let mut e2 = env.clone();
                        e2.binds.push((iv.name.clone(), Self::signed_loop_val(i)));
                        match self.csp_item(csp, body, &e2)? {
                            Node::True => {}
                            n => out.push(n),
                        }
                    }
                    return Some(Self::csp_and(out));
                }
                // §18.4 / §12.7.3 (#519): a loop over a multi-dimensional
                // solver array (`foreach (a[i])`, `foreach (a[i][j])`) or over
                // one of its rows (`foreach (a[i][j])` with `i` bound): each
                // loop variable walks the declared bounds of its dimension.
                if !csp.nd_dims.is_empty() {
                    let mut konst = |me: &mut Self, x: &Expression| -> Option<i64> {
                        if !me.csp_free(csp, x, env) {
                            return None;
                        }
                        me.csp_const(x, env)?.to_i64()
                    };
                    if let Some((base, consts)) = self.csp_nd_prefix_k(csp, array, &mut konst)
                        && let Some(dims) = csp.nd_dims.get(&base).cloned()
                        && consts.len() + vars.len() <= dims.len()
                    {
                        // A dynamic dimension is `(0, size - 1)`: one that is
                        // empty gives the loop no iteration (§12.7.3), where
                        // sorting its bounds walked -1 and 0.
                        let ndyn = csp.nd_dyn.get(&base).copied().unwrap_or(0);
                        let loops: Vec<(Option<String>, i64, i64)> = vars
                            .iter()
                            .enumerate()
                            .map(|(j, v)| {
                                let d = consts.len() + j;
                                let (l, r) = dims[d];
                                let (lo, hi) = if d < ndyn {
                                    (0, r)
                                } else {
                                    (l.min(r), l.max(r))
                                };
                                (v.as_ref().map(|v| v.name.clone()), lo, hi)
                            })
                            .collect();
                        let count = loops
                            .iter()
                            .try_fold(1u64, |n, l| n.checked_mul((l.2 - l.1 + 1).max(0) as u64))?;
                        if count == 0 {
                            return Some(Node::True);
                        }
                        if count > 4096 {
                            return None;
                        }
                        let mut idx: Vec<i64> = loops.iter().map(|l| l.1).collect();
                        let mut out = Vec::new();
                        loop {
                            let mut e2 = env.clone();
                            for (l, &i) in loops.iter().zip(&idx) {
                                if let Some(n) = &l.0 {
                                    e2.binds.push((n.clone(), Self::signed_loop_val(i)));
                                }
                            }
                            match self.csp_item(csp, body, &e2)? {
                                Node::True => {}
                                n => out.push(n),
                            }
                            let mut k = loops.len();
                            loop {
                                if k == 0 {
                                    return Some(Self::csp_and(out));
                                }
                                k -= 1;
                                idx[k] += 1;
                                if idx[k] <= loops[k].2 {
                                    break;
                                }
                                idx[k] = loops[k].1;
                            }
                        }
                    }
                }
                let name = self.csp_member(csp, array).or_else(|| {
                    Self::foreach_plain_member(array)
                        .then(|| Self::foreach_base_name(array))
                        .flatten()
                });
                let arr = name.as_ref().and_then(|n| csp.arrays.get(n)).cloned();
                let idx_var = match vars.as_slice() {
                    [Some(v)] => Some(v.name.clone()),
                    _ => None,
                };
                let (Some(arr), Some(iv)) = (arr, idx_var.clone()) else {
                    // §18.5.8.1: a foreach over a fixed-shape member that is
                    // not a solver array (a state vector used as an
                    // iterator, `bit [0:3] it;`) or over the bits of a
                    // solver scalar: its indices are known, so the body is
                    // translated once per index like a solver array's.
                    if let (Some(iv), Some(idx)) = (
                        idx_var,
                        self.csp_foreach_indices(csp, array, name.as_deref()),
                    ) {
                        let mut out = Vec::with_capacity(idx.len());
                        for i in idx {
                            let mut e2 = env.clone();
                            e2.binds.push((iv.clone(), Self::signed_loop_val(i)));
                            match self.csp_item(csp, body, &e2)? {
                                Node::True => {}
                                n => out.push(n),
                            }
                        }
                        return Some(Self::csp_and(out));
                    }
                    return self.csp_eval_node(csp, SrcItem::Item(item.clone()), env, false);
                };
                let mut out = Vec::with_capacity(arr.elems.len());
                for (i, _) in &arr.elems {
                    let mut e2 = env.clone();
                    // Ordinary array indices are signed ints, including negative bounds.
                    e2.binds.push((iv.clone(), Self::signed_loop_val(*i)));
                    let n = self.csp_item(csp, body, &e2)?;
                    match n {
                        Node::True => {}
                        n => out.push(n),
                    }
                }
                Some(Self::csp_and(out))
            }
            ConstraintItem::Solve {
                before,
                after,
                before_paths,
                after_paths,
                ..
            } => {
                // §18.5.10: an operand is a variable, an array, or a member,
                // field or element path (`a.b.f`, `arr[i].f`, `s.f[3]`); a
                // path the solve does not hold falls back to its root name.
                let vars = |me: &mut Self,
                            names: &[crate::ast::Identifier],
                            paths: &[Expression]|
                 -> Vec<usize> {
                    let mut out = Vec::new();
                    for (k, id) in names.iter().enumerate() {
                        if let Some(p) = paths.get(k)
                            && let Some(vs) = me.csp_solve_path(csp, p, env)
                        {
                            out.extend(vs);
                            continue;
                        }
                        if let Some(&v) = csp.scalars.get(&id.name) {
                            out.push(v);
                        } else if let Some(a) = csp.arrays.get(&id.name) {
                            out.extend(a.elems.iter().map(|e| e.1));
                        }
                    }
                    // A wide variable is decided through its segments.
                    let mut i = 0;
                    while i < out.len() {
                        match csp.segs.get(&out[i]) {
                            Some(segs) if csp.wide.contains(&out[i]) => {
                                let segs: Vec<usize> = segs.iter().map(|x| x.2).collect();
                                out.splice(i..=i, segs.iter().copied());
                                i += segs.len();
                            }
                            _ => i += 1,
                        }
                    }
                    out.sort_unstable();
                    out.dedup();
                    out
                };
                let (b, a) = (
                    vars(self, before, before_paths),
                    vars(self, after, after_paths),
                );
                if !b.is_empty() && !a.is_empty() {
                    csp.order.push((b, a));
                }
                Some(Node::True)
            }
            ConstraintItem::Soft(inner) => {
                let id = csp.soft_priority.len();
                csp.soft_priority
                    .push((csp.soft_rank.0, csp.soft_rank.1, id));
                let enabled = csp
                    .soft_enabled
                    .as_ref()
                    .is_none_or(|set| set.contains(&id));
                let dist_len = csp.dists.len();
                let order_len = csp.order.len();
                let opaque = csp.opaque;
                let mask_env = if enabled {
                    std::borrow::Cow::Borrowed(env)
                } else {
                    env.optional_mask()
                };
                let node = self.csp_item(csp, inner, &mask_env)?;
                if enabled {
                    Some(node)
                } else {
                    csp.dists.truncate(dist_len);
                    csp.order.truncate(order_len);
                    csp.opaque = opaque;
                    Some(Node::True)
                }
            }
            ConstraintItem::Block(items) => {
                let mut out = Vec::with_capacity(items.len());
                for it in items {
                    match self.csp_item(csp, it, env)? {
                        Node::True => {}
                        n => out.push(n),
                    }
                }
                Some(Self::csp_and(out))
            }
            ConstraintItem::Unique { exprs, .. } => {
                let mut vs = Vec::new();
                for e in exprs {
                    let e = Self::unparen(e);
                    if let Some(a) = self.csp_member(csp, e).and_then(|n| csp.arrays.get(&n)) {
                        vs.extend(a.elems.iter().map(|x| x.1));
                        continue;
                    }
                    match self.csp_ae(csp, e, env) {
                        Some(Ae::Var(v, ..)) => vs.push(v),
                        _ => {
                            return self.csp_eval_node(
                                csp,
                                SrcItem::Item(item.clone()),
                                env,
                                false,
                            );
                        }
                    }
                }
                // §18.5.5 over elements wider than 64 bits: distinctness is
                // not modelled (the bases are segment-held), but the rest of
                // the set still is — giving the whole translation up here
                // left `a[0] < 96'h1_0000_0000_0000_0000` to blind draws
                // (#928). `enforce_unique_coll` separates the elements
                // before the solve and the trial checker judges the item.
                if vs.iter().any(|v| csp.wide.contains(v)) {
                    csp.opaque = true;
                    return Some(Node::True);
                }
                Some(Node::AllDiff(vs))
            }
        }
    }

    /// §18.5.10: the solver variables a `solve` operand path names: a
    /// variable or array, a member of a rand sub-object, or a field,
    /// element or constant select of a variable (the segments it spans).
    /// None when the path names nothing of the solve.
    fn csp_solve_path(&mut self, csp: &Csp, e: &Expression, env: &Env) -> Option<Vec<usize>> {
        if let Some(n) = self.csp_member(csp, e) {
            if let Some(&v) = csp.scalars.get(&n) {
                return Some(vec![v]);
            }
            if let Some(a) = csp.arrays.get(&n) {
                return Some(a.elems.iter().map(|x| x.1).collect());
            }
        }
        let mut konst = |me: &mut Self, x: &Expression| -> Option<i64> {
            if !me.csp_free(csp, x, env) {
                return None;
            }
            me.csp_const(x, env)?.to_i64()
        };
        let (v, lo, w) = match self.csp_select_ref(csp, e, &mut konst) {
            Some((v, Some((lo, w)))) => (v, lo, w),
            Some((v, None)) => (v, 0, csp.vars[v].width),
            None => {
                let (v, lo, w, _) = self.csp_slice_base_k(csp, e, &mut konst)?;
                (v, lo, w)
            }
        };
        let Some(segs) = csp.segs.get(&v) else {
            return Some(vec![v]);
        };
        if let Some(&x) = csp.slice_vars.get(&(v, lo, w)) {
            return Some(vec![x]);
        }
        let vs: Vec<usize> = segs
            .iter()
            .filter(|s| s.0 < lo + w && s.0 + s.1 > lo)
            .map(|s| s.2)
            .collect();
        Some(if vs.is_empty() { vec![v] } else { vs })
    }

    /// The first-dimension indices of `foreach (name[i])` over a plain
    /// member of a fixed shape (an unpacked fixed array or a packed vector)
    /// of the object being solved. None for anything else: a collection of
    /// handles, a dynamic shape, a path through another object.
    pub(super) fn csp_foreach_indices(
        &self,
        csp: &Csp,
        array: &Expression,
        name: Option<&str>,
    ) -> Option<Vec<i64>> {
        if let Some(name) = name
            && (csp.scope.is_some() || csp.arrays.contains_key(name))
        {
            return None;
        }
        if csp.scope.is_none()
            && !Self::foreach_plain_member(array)
            && let Some((owner, leaf)) = self.csp_resolve_member_owner(csp.handle, array)
        {
            if self.prop_class_type(owner, &leaf).is_some()
                && !self.is_associative_array(&format!("{owner}#{leaf}"))
                && !self.prop_is_rand(owner, &leaf)
                && let Some(keys) = self.rand_child_collection_keys(owner, &leaf)
            {
                let prefix = format!("{owner}#{leaf}[");
                return keys
                    .iter()
                    .map(|k| k.strip_prefix(&prefix)?.strip_suffix(']')?.parse().ok())
                    .collect();
            }
            let plain = super::vpi_model::ident_expr(&leaf);
            return self.csp_foreach_indices_of(owner, &plain, &leaf);
        }
        let name = name?;
        self.csp_foreach_indices_of(csp.handle, array, name)
    }

    /// The same for a member `name` of the object `handle`.
    pub(super) fn csp_foreach_indices_of(
        &self,
        handle: usize,
        array: &Expression,
        name: &str,
    ) -> Option<Vec<i64>> {
        let plain = match &array.kind {
            ExprKind::Ident(h) => {
                h.root.is_none()
                    && h.path.iter().all(|s| s.selects.is_empty())
                    && (h.path.len() == 1 || (h.path.len() == 2 && h.path[0].name.name == "this"))
            }
            ExprKind::MemberAccess { expr, .. } => matches!(expr.kind, ExprKind::This),
            _ => false,
        };
        // An associative member has no index range; its element type's
        // packed range (`bit [3:0] m[string]`) is not one either.
        if !plain
            || self.prop_class_type(handle, name).is_some()
            || self.is_associative_array(&format!("{handle}#{name}"))
        {
            return None;
        }
        let fixed = self.fixed_foreach_dims(handle, name);
        if fixed.is_none()
            && let Some(keys) = self.rand_child_collection_keys(handle, name)
        {
            // §7.4/§18.5.9 (#860): foreach indexes the unpacked collection,
            // not its element's packed dimensions. `bit [7:0] r[]` with
            // four elements has indices 0..3, never the eight bit indices.
            let prefix = format!("{handle}#{name}[");
            return keys
                .iter()
                .map(|key| key.strip_prefix(&prefix)?.strip_suffix(']')?.parse().ok())
                .collect();
        }
        let (lo, hi) = match fixed.or_else(|| self.packed_prop_foreach_dims(handle, name)) {
            Some((dims, _)) => *dims.first()?,
            // A state queue or dynamic array: its current size bounds the
            // loop for this solve (an empty one constrains nothing).
            None => match self.csp_vector_range(handle, name) {
                Some(r) => r,
                None => *self.state_foreach_dims(handle, array, name, 1)?.first()?,
            },
        };
        (hi - lo < 4096).then(|| (lo..=hi).collect())
    }

    /// The declared index range (low, high) of a one-dimensional packed
    /// vector member (`bit [0:3] it;`).
    fn csp_vector_range(&self, handle: usize, name: &str) -> Option<(i64, i64)> {
        let mut cur = self
            .heap
            .get(handle)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.clone());
        let mut dt = None;
        while let Some(cn) = cur {
            let cd = self.module.classes.get(&cn)?;
            if let Some(t) = self.class_prop_decl_type(cd, name) {
                dt = Some(t.clone());
                break;
            }
            cur = cd.extends.clone();
        }
        let dt =
            crate::compiler::elaborate::resolve_typedef_chain(&dt?, &self.module.typedef_types)
                .clone();
        let DataType::IntegerVector { dimensions, .. } = &dt else {
            return None;
        };
        let [crate::ast::types::PackedDimension::Range { left, right, .. }] = dimensions.as_slice()
        else {
            return None;
        };
        let p = Some(&self.module.parameters);
        let l = crate::compiler::elaborate::const_eval_i64_with_params(left, p)?;
        let r = crate::compiler::elaborate::const_eval_i64_with_params(right, p)?;
        Some((l.min(r), l.max(r)))
    }

    /// Record the weights of `expr dist {…}` when `expr` is one solver
    /// variable (§18.5.4): a value or range item weighs `w` per value with
    /// `:=` (the default) and `w` in all with `:/`; a zero weight or an empty
    /// (reversed) range contributes nothing, and `$` is the variable's own
    /// bound.
    fn csp_dist(
        &mut self,
        csp: &mut Csp,
        expr: &Expression,
        ranges: &[ConstraintRange],
        weights: &[Option<DistWeight>],
        env: &Env,
    ) {
        let Some(Ae::Var(v, w, s)) = self.csp_ae(csp, expr, env) else {
            return;
        };
        let (vlo, vhi) = ws_range(w, s);
        let frame = self.csp_frame(&env.binds);
        self.push_local_frame(frame);
        let mut items = Vec::with_capacity(ranges.len());
        let mut ok = true;
        for (k, r) in ranges.iter().enumerate() {
            let (wt, per_value) = match weights.get(k).and_then(|w| w.as_ref()) {
                Some(DistWeight::Each(e)) => (self.csp_const(e, env), true),
                Some(DistWeight::Total(e)) => (self.csp_const(e, env), false),
                None => (Some(Value::from_u64(1, 32)), true),
            };
            let Some(wt) = wt.and_then(|x| x.to_u64()) else {
                ok = false;
                break;
            };
            let bound = |me: &mut Self, e: &Expression, dollar: i128| -> Option<i128> {
                if matches!(e.kind, ExprKind::Dollar) {
                    return Some(dollar);
                }
                if !me.csp_free(csp, e, env) {
                    return None;
                }
                me.exact_int(e).map(|x| x.0)
            };
            let (lo, hi) = match r {
                ConstraintRange::Value(e) => match bound(self, e, vhi) {
                    Some(x) => (x, x),
                    None => {
                        ok = false;
                        break;
                    }
                },
                ConstraintRange::Range { lo, hi } => {
                    match (bound(self, lo, vlo), bound(self, hi, vhi)) {
                        (Some(l), Some(h)) => (l, h),
                        _ => {
                            ok = false;
                            break;
                        }
                    }
                }
            };
            let (lo, hi) = (lo.max(vlo), hi.min(vhi));
            if wt == 0 || lo > hi {
                continue;
            }
            let mass = if per_value {
                wt as f64 * (hi - lo + 1) as f64
            } else {
                wt as f64
            };
            items.push((lo, hi, mass));
        }
        self.pop_local_frame();
        if ok && !items.is_empty() {
            csp.dists.push(Dist {
                var: v,
                items,
                guards: env.guards.clone(),
            });
        }
    }

    /// §18.5.4: `x dist {…}` over a wide `x`. Its items are not values of
    /// one solver variable, so an auxiliary variable picks the item: it
    /// carries the dist (decided first, in proportion to each item's weight
    /// among the items still feasible), and `x` is constrained to the item
    /// it picks. None when an item or weight is not constant.
    fn csp_wide_dist(
        &mut self,
        csp: &mut Csp,
        expr: &Expression,
        ranges: &[ConstraintRange],
        weights: &[Option<DistWeight>],
        env: &Env,
    ) -> Option<Node> {
        // The number of values of `[lo:hi]`, as a float (wide bounds).
        fn span_count(lo: &Value, hi: &Value) -> f64 {
            let f = |v: &Value| {
                let mut x = 0.0f64;
                for i in (0..v.width as usize).rev() {
                    x = x * 2.0
                        + if v.get_bit(i) == LogicBit::One {
                            1.0
                        } else {
                            0.0
                        };
                }
                x
            };
            (f(hi) - f(lo) + 1.0).max(1.0)
        }
        let mut items: Vec<(ConstraintRange, f64)> = Vec::new();
        for (k, r) in ranges.iter().enumerate() {
            let (wt, per_value) = match weights.get(k).and_then(|w| w.as_ref()) {
                Some(DistWeight::Each(e)) => (self.csp_const(e, env)?.to_u64()?, true),
                Some(DistWeight::Total(e)) => (self.csp_const(e, env)?.to_u64()?, false),
                None => (1, true),
            };
            if wt == 0 {
                continue;
            }
            let mass = match r {
                ConstraintRange::Value(_) => wt as f64,
                ConstraintRange::Range { lo, hi } => {
                    if matches!(lo.kind, ExprKind::Dollar) || matches!(hi.kind, ExprKind::Dollar) {
                        return None;
                    }
                    if per_value {
                        let (l, h) = (self.csp_const_any(lo, env), self.csp_const_any(hi, env));
                        if l.has_xz() || h.has_xz() {
                            return None;
                        }
                        wt as f64 * span_count(&l, &h)
                    } else {
                        wt as f64
                    }
                }
            };
            items.push((r.clone(), mass));
        }
        if items.is_empty() || items.len() > 4096 {
            return None;
        }
        let mut alts = Vec::with_capacity(items.len());
        let c = csp.vars.len();
        csp.vars.push(CspVar {
            key: VarKey::Aux,
            width: 32,
            signed: false,
        });
        csp.dom0.push(vec![(0, items.len() as i128 - 1)]);
        let one = Self::literal_of(&Value::from_u64(1, 1), expr.span);
        for (i, (r, _)) in items.iter().enumerate() {
            let node = self.csp_wide_inside(csp, expr, std::slice::from_ref(r), env, false)?;
            let src = self.csp_src(csp, SrcItem::Expr(one.clone()), &Env::default(), vec![c]);
            let pick = Node::In {
                lin: Lin::var(c),
                set: vec![(i as i128, i as i128)],
                neg: false,
                fits: Vec::new(),
                src,
                wrap: None,
            };
            alts.push(Self::csp_wide_and(vec![pick, node]));
        }
        csp.dists.push(Dist {
            var: c,
            items: items
                .iter()
                .enumerate()
                .map(|(i, (_, m))| (i as i128, i as i128, *m))
                .collect(),
            guards: env.guards.clone(),
        });
        Some(Self::csp_wide_or(alts))
    }

    pub(super) fn csp_and(mut v: Vec<Node>) -> Node {
        match v.len() {
            0 => Node::True,
            1 => v.pop().unwrap(),
            _ => Node::And(v),
        }
    }

    fn csp_if(
        &mut self,
        csp: &mut Csp,
        cond: &Expression,
        then: Node,
        els: Node,
        env: &Env,
    ) -> Option<Node> {
        let env = &env.optional_mask();
        let c = self.csp_bool(csp, cond, env, false)?;
        match c {
            Node::True => return Some(then),
            Node::False => return Some(els),
            _ => {}
        }
        if matches!(then, Node::True) && matches!(els, Node::True) {
            return Some(Node::True);
        }
        let nc = self.csp_bool(csp, cond, env, true)?;
        Some(Node::If {
            cond: Box::new(c),
            ncond: Box::new(nc),
            then: Box::new(then),
            els: Box::new(els),
        })
    }

    /// A boolean constraint expression (negated with `neg`).
    pub(super) fn csp_bool(
        &mut self,
        csp: &mut Csp,
        e: &Expression,
        env: &Env,
        neg: bool,
    ) -> Option<Node> {
        let e = Self::unparen(e);
        if self.csp_free(csp, e, env) {
            let v = self.csp_const_any(e, env);
            return Some(if v.is_true() != neg {
                Node::True
            } else {
                Node::False
            });
        }
        match &e.kind {
            ExprKind::Unary {
                op: UnaryOp::LogNot,
                operand,
            } => self.csp_bool(csp, operand, env, !neg),
            ExprKind::Binary { op, left, right } => match op {
                BinaryOp::LogAnd | BinaryOp::LogOr => {
                    let e = if (*op == BinaryOp::LogOr) != neg {
                        env.optional_mask()
                    } else {
                        std::borrow::Cow::Borrowed(env)
                    };
                    let l = self.csp_bool(csp, left, &e, neg)?;
                    let r = self.csp_bool(csp, right, &e, neg)?;
                    // De Morgan: a negated && is an || of the negations
                    Some(if (*op == BinaryOp::LogAnd) != neg {
                        Node::And(vec![l, r])
                    } else {
                        Node::Or(vec![l, r])
                    })
                }
                // §11.4.5: on one-bit operands `&` is `&&` and `|` is `||`.
                // Decomposing them (rather than leaving the whole item to the
                // evaluator, which only judges a complete assignment) lets a
                // conjunct over a wide rand variable propagate; evaluated, the
                // conjunction is hit by sampling once in 2**32, so every trial
                // was rejected and randomize() reported "no solution" for e.g.
                // `with { s == A & data == 32'h1234; }`.
                BinaryOp::BitAnd | BinaryOp::BitOr
                    if Self::csp_one_bit(left) && Self::csp_one_bit(right) =>
                {
                    let e = if (*op == BinaryOp::BitOr) != neg {
                        env.optional_mask()
                    } else {
                        std::borrow::Cow::Borrowed(env)
                    };
                    let l = self.csp_bool(csp, left, &e, neg)?;
                    let r = self.csp_bool(csp, right, &e, neg)?;
                    Some(if (*op == BinaryOp::BitAnd) != neg {
                        Node::And(vec![l, r])
                    } else {
                        Node::Or(vec![l, r])
                    })
                }
                BinaryOp::LogImplies => {
                    if neg {
                        let l = self.csp_bool(csp, left, env, false)?;
                        let r = self.csp_bool(csp, right, env, true)?;
                        Some(Node::And(vec![l, r]))
                    } else {
                        let c = self.csp_bool(csp, left, &env.optional_mask(), false)?;
                        let then = self.csp_bool(csp, right, &env.guarded(&c), false)?;
                        self.csp_if(csp, left, then, Node::True, env)
                    }
                }
                BinaryOp::Eq
                | BinaryOp::Neq
                | BinaryOp::CaseEq
                | BinaryOp::CaseNeq
                | BinaryOp::Lt
                | BinaryOp::Leq
                | BinaryOp::Gt
                | BinaryOp::Geq => {
                    if let Some(n) = self.csp_dyn_expand(csp, e, env, neg) {
                        return Some(n);
                    }
                    self.csp_rel(csp, e, *op, left, right, env, neg)
                }
                _ => self
                    .csp_dyn_expand(csp, e, env, neg)
                    .or_else(|| self.csp_eval_node(csp, SrcItem::Expr(e.clone()), env, neg)),
            },
            ExprKind::Inside { expr, ranges } => {
                if let Some(n) = self.csp_dyn_expand(csp, e, env, neg) {
                    return Some(n);
                }
                let cr: Vec<ConstraintRange> = ranges
                    .iter()
                    .map(|r| match &r.kind {
                        ExprKind::Range(lo, hi) => ConstraintRange::Range {
                            lo: (**lo).clone(),
                            hi: (**hi).clone(),
                        },
                        _ => ConstraintRange::Value(r.clone()),
                    })
                    .collect();
                self.csp_inside(csp, expr, &cr, env, neg, SrcItem::Expr(e.clone()))
            }
            _ => self
                .csp_dyn_expand(csp, e, env, neg)
                .or_else(|| self.csp_eval_node(csp, SrcItem::Expr(e.clone()), env, neg)),
        }
    }

    /// True when `e` is one bit wide by construction: a comparison, an
    /// `inside`, a logical operator, or a bitwise operator whose operands are
    /// both one bit. `a & b` over such operands is exactly `a && b` (§11.4.5),
    /// so `csp_bool` can decompose it instead of leaving it to the evaluator.
    fn csp_one_bit(e: &Expression) -> bool {
        match &Self::unparen(e).kind {
            ExprKind::Binary { op, left, right } => match op {
                BinaryOp::Eq
                | BinaryOp::Neq
                | BinaryOp::CaseEq
                | BinaryOp::CaseNeq
                | BinaryOp::Lt
                | BinaryOp::Leq
                | BinaryOp::Gt
                | BinaryOp::Geq
                | BinaryOp::LogAnd
                | BinaryOp::LogOr
                | BinaryOp::LogImplies => true,
                BinaryOp::BitAnd | BinaryOp::BitOr => {
                    Self::csp_one_bit(left) && Self::csp_one_bit(right)
                }
                _ => false,
            },
            ExprKind::Unary {
                op: UnaryOp::LogNot,
                ..
            } => true,
            ExprKind::Inside { .. } => true,
            _ => false,
        }
    }
}
