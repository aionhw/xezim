//! §18.4/§18.5.9: the rand sub-objects of a joint solve held in arrays of
//! object handles (`rand C a[N]`, `rand C d[]`, `rand C q[$]`).
//!
//! Every non-null element joins the solve like a plain rand handle member:
//! its rand scalars become `a[k].<name>` variables and its constraints,
//! rewritten through `a[k]`, join the set. Inside the solver an element is
//! named by one identifier segment `a[k]` (no select), so the plain-handle
//! machinery (`prop.x`, its fields and selects) reads it unchanged; every
//! expression handed to the evaluator gets the select back (`csp_obj_real`).
//! A `foreach` whose body picks an element by its loop index (`foreach
//! (a[i]) a[i].x > s`, `foreach (it[i]) a[i].f == …`) is unrolled first, so
//! each element is named by a constant.
use super::*;

thread_local! {
    /// The object arrays whose elements take part in the running solve.
    static CSP_OBJ: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Restores the previous `CSP_OBJ` when a solve ends.
pub(super) struct ObjGuard(Vec<String>);

impl Drop for ObjGuard {
    fn drop(&mut self) {
        let prev = std::mem::take(&mut self.0);
        CSP_OBJ.with(|c| *c.borrow_mut() = prev);
    }
}

pub(super) fn csp_obj_enter(names: Vec<String>) -> ObjGuard {
    ObjGuard(CSP_OBJ.with(|c| std::mem::replace(&mut *c.borrow_mut(), names)))
}

fn csp_obj_active() -> bool {
    CSP_OBJ.with(|c| !c.borrow().is_empty())
}

/// `a[k]` -> ("a", k) for an object array `a` of the running solve.
fn csp_obj_split(name: &str) -> Option<(String, i64)> {
    let (arr, rest) = name.split_once('[')?;
    let k: i64 = rest.strip_suffix(']')?.parse().ok()?;
    CSP_OBJ
        .with(|c| c.borrow().iter().any(|a| a == arr))
        .then(|| (arr.to_string(), k))
}

fn ident_expr(path: Vec<crate::ast::expr::HierPathSegment>, span: crate::ast::Span) -> Expression {
    Expression::new(
        ExprKind::Ident(HierarchicalIdentifier {
            root: None,
            path,
            span,
            cached_signal_id: Cell::new(None),
            cached_resolved_name: std::cell::OnceCell::new(),
        }),
        span,
    )
}

fn seg(name: &str, span: crate::ast::Span) -> crate::ast::expr::HierPathSegment {
    crate::ast::expr::HierPathSegment {
        name: crate::ast::Identifier {
            name: name.to_string(),
            span,
        },
        selects: Vec::new(),
    }
}

impl Simulator {
    /// §18.5.9: the rand sub-objects of `handle` that join its joint solve,
    /// as (name, object): a plain rand handle member by its name, and each
    /// non-null element of a fixed, dynamic or queue array of rand handles
    /// as `arr[k]`. None keeps them out of the joint solve: a member-subset
    /// call, a null or self-referencing plain handle, an associative or
    /// multi-dimensional array of handles, a dynamic array whose size a
    /// constraint draws, or one object reached through two array slots.
    /// A member no constraint of the class reads shares no constraint with
    /// it and is left out: each trial draws it with its own solve first
    /// (`randomize_nested_with`), which gives the same solutions.
    pub(crate) fn rand_csp_subs(
        &self,
        handle: usize,
        class_name: &str,
        rand_obj_props: &[String],
        constraints: &[ClassConstraint],
    ) -> Option<Vec<(String, usize)>> {
        let cd = self.module.classes.get(class_name);
        let mut out: Vec<(String, usize)> = Vec::new();
        let mut seen: HashSet<usize> = HashSet::default();
        let mut elems_in = false;
        for p in rand_obj_props {
            if !Self::csp_constraints_read(p, constraints) {
                continue;
            }
            let scoped = format!("{}#{}", handle, p);
            if self.is_associative_array(&scoped)
                || cd.is_some_and(|cd| {
                    cd.assoc_properties.contains_key(p) || cd.array_nd_properties.contains_key(p)
                })
            {
                return None;
            }
            match self.rand_child_collection_keys(handle, p) {
                Some(keys) => {
                    if cd.is_some_and(|cd| cd.queue_properties.contains_key(p))
                        && Self::csp_obj_sized(p, constraints)
                    {
                        return None;
                    }
                    elems_in = true;
                    let prefix = format!("{}[", scoped);
                    for k in keys {
                        let Some(idx) = k
                            .strip_prefix(&prefix)
                            .and_then(|r| r.strip_suffix(']'))
                            .and_then(|r| r.parse::<i64>().ok())
                        else {
                            return None;
                        };
                        let sub = self
                            .read_coll_elem(&k)
                            .and_then(|v| v.to_u64())
                            .unwrap_or(0) as usize;
                        // §18.5.9: a null element holds no object to solve.
                        if sub == 0 || self.heap.get(sub).and_then(|o| o.as_ref()).is_none() {
                            continue;
                        }
                        if sub == handle || !seen.insert(sub) {
                            return None;
                        }
                        out.push((format!("{}[{}]", p, idx), sub));
                    }
                }
                None => {
                    let sub = self.member_handle(handle, p).unwrap_or(0);
                    if sub == 0 || sub == handle {
                        return None;
                    }
                    if !seen.insert(sub) && elems_in {
                        return None;
                    }
                    out.push((p.clone(), sub));
                    self.rand_csp_nested_subs(p, sub, handle, &mut seen, &mut out, 0)?;
                }
            }
        }
        // A plain handle that aliases an element seen later.
        if elems_in && seen.len() != out.len() {
            return None;
        }
        Some(out)
    }

    /// Whether some item of `constraints` may read member `prop`: a name or
    /// member access spelled `prop` anywhere in it. An expression form the
    /// walk does not look inside counts as a read.
    fn csp_constraints_read(prop: &str, constraints: &[ClassConstraint]) -> bool {
        let reads = |e: &Expression| {
            let mut hit = false;
            Self::walk_operands(e, &mut |x| match &x.kind {
                ExprKind::Ident(h) => hit |= h.path.iter().any(|s| s.name.name == prop),
                ExprKind::MemberAccess { member, .. } => hit |= member.name == prop,
                ExprKind::Number(_)
                | ExprKind::StringLiteral(_)
                | ExprKind::Unary { .. }
                | ExprKind::Binary { .. }
                | ExprKind::Conditional { .. }
                | ExprKind::Concatenation(_)
                | ExprKind::Replication { .. }
                | ExprKind::Call { .. }
                | ExprKind::SystemCall { .. }
                | ExprKind::Inside { .. }
                | ExprKind::Paren(_)
                | ExprKind::Index { .. }
                | ExprKind::RangeSelect { .. }
                | ExprKind::Range(..)
                | ExprKind::Dollar
                | ExprKind::Null
                | ExprKind::This
                | ExprKind::Empty => {}
                _ => hit = true,
            });
            hit
        };
        let mut hit = false;
        for c in constraints {
            for it in &c.items {
                Self::csp_item_walk(it, &mut |i| {
                    hit |= match i {
                        ConstraintItem::Expr(e) => reads(e),
                        ConstraintItem::Inside {
                            expr,
                            range,
                            dist_weights,
                            ..
                        } => {
                            reads(expr)
                                || range.iter().any(|r| match r {
                                    ConstraintRange::Value(v) => reads(v),
                                    ConstraintRange::Range { lo, hi } => reads(lo) || reads(hi),
                                })
                                || dist_weights.iter().flatten().any(|w| match w {
                                    DistWeight::Each(e) | DistWeight::Total(e) => reads(e),
                                })
                        }
                        ConstraintItem::Implication { condition, .. }
                        | ConstraintItem::IfElse { condition, .. } => reads(condition),
                        ConstraintItem::Foreach { array, .. } => reads(array),
                        ConstraintItem::Solve {
                            before,
                            after,
                            before_paths,
                            after_paths,
                            ..
                        } => {
                            before.iter().chain(after).any(|n| n.name == prop)
                                || before_paths.iter().chain(after_paths).any(reads)
                        }
                        ConstraintItem::Unique { exprs, .. } => exprs.iter().any(reads),
                        ConstraintItem::Soft(_) | ConstraintItem::Block(_) => false,
                    }
                });
            }
        }
        hit
    }

    /// §18.5.9: the rand sub-objects of sub-object `sub` (member path
    /// `prefix`), recursively, as `prefix.member`. A null member holds no
    /// object; an array of handles, a shared object or a cycle keeps the
    /// whole set out of the joint solve (None).
    fn rand_csp_nested_subs(
        &self,
        prefix: &str,
        sub: usize,
        root: usize,
        seen: &mut HashSet<usize>,
        out: &mut Vec<(String, usize)>,
        depth: u32,
    ) -> Option<()> {
        if depth > 8 {
            return None;
        }
        let rand_disabled = self
            .rand_mode_disabled
            .get(&sub)
            .cloned()
            .unwrap_or_default();
        let mut cur = self.heap.get(sub)?.as_ref().map(|i| i.class_name.clone());
        let mut members: Vec<String> = Vec::new();
        let mut hops = 0;
        while let Some(cn) = cur {
            hops += 1;
            if hops > 64 {
                break;
            }
            let cd = self.module.classes.get(&cn)?;
            for m in self.randomize_members(cd, &rand_disabled) {
                let Some(sig) = cd.properties.get(&m) else {
                    continue;
                };
                let is_obj = sig
                    .type_name
                    .as_ref()
                    .is_some_and(|tn| self.module.classes.contains_key(tn));
                if !is_obj {
                    continue;
                }
                if cd.array_properties.contains_key(&m)
                    || cd.array_nd_properties.contains_key(&m)
                    || cd.queue_properties.contains_key(&m)
                    || cd.assoc_properties.contains_key(&m)
                {
                    return None;
                }
                if !members.contains(&m) {
                    members.push(m);
                }
            }
            cur = cd.extends.clone();
        }
        for m in members {
            let h = self.member_handle(sub, &m).unwrap_or(0);
            if h == 0 || self.heap.get(h).and_then(|o| o.as_ref()).is_none() {
                continue;
            }
            if h == root || !seen.insert(h) {
                return None;
            }
            let path = format!("{}.{}", prefix, m);
            out.push((path.clone(), h));
            self.rand_csp_nested_subs(&path, h, root, seen, out, depth + 1)?;
        }
        Some(())
    }

    /// Some constraint reads the size of the collection `prop`.
    fn csp_obj_sized(prop: &str, constraints: &[ClassConstraint]) -> bool {
        let mut hit = false;
        for c in constraints {
            for it in &c.items {
                Self::csp_item_exprs(it, &mut |e| {
                    Self::walk_operands(e, &mut |x| match &x.kind {
                        ExprKind::Call { func, .. } => {
                            if let ExprKind::MemberAccess { expr, member } = &func.kind
                                && member.name == "size"
                                && Self::member_chain(expr)
                                    .is_some_and(|n| n.last() == Some(&prop.to_string()))
                            {
                                hit = true;
                            }
                            if let ExprKind::Ident(h) = &func.kind
                                && h.path.len() >= 2
                                && h.path.last().is_some_and(|s| s.name.name == "size")
                                && h.path[h.path.len() - 2].name.name == prop
                            {
                                hit = true;
                            }
                        }
                        ExprKind::SystemCall { name, args } if name == "$size" => {
                            if args
                                .first()
                                .and_then(Self::member_chain)
                                .is_some_and(|n| n.last() == Some(&prop.to_string()))
                            {
                                hit = true;
                            }
                        }
                        _ => {}
                    })
                });
            }
        }
        hit
    }

    /// Visit every expression an item holds (dist weights and `solve`
    /// operand paths included), nested items too.
    pub(super) fn csp_item_exprs(it: &ConstraintItem, f: &mut dyn FnMut(&Expression)) {
        match it {
            ConstraintItem::Expr(e) => f(e),
            ConstraintItem::Inside {
                expr,
                range,
                dist_weights,
                ..
            } => {
                f(expr);
                for r in range {
                    match r {
                        ConstraintRange::Value(v) => f(v),
                        ConstraintRange::Range { lo, hi } => {
                            f(lo);
                            f(hi);
                        }
                    }
                }
                for w in dist_weights.iter().flatten() {
                    match w {
                        DistWeight::Each(e) | DistWeight::Total(e) => f(e),
                    }
                }
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => {
                f(condition);
                Self::csp_item_exprs(constraint, f);
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                f(condition);
                Self::csp_item_exprs(then_item, f);
                if let Some(e) = else_item {
                    Self::csp_item_exprs(e, f);
                }
            }
            ConstraintItem::Foreach { array, item, .. } => {
                f(array);
                Self::csp_item_exprs(item, f);
            }
            ConstraintItem::Solve {
                before_paths,
                after_paths,
                ..
            } => {
                before_paths.iter().chain(after_paths).for_each(f);
            }
            ConstraintItem::Soft(i) => Self::csp_item_exprs(i, f),
            ConstraintItem::Block(items) => {
                for i in items {
                    Self::csp_item_exprs(i, f);
                }
            }
            ConstraintItem::Unique { exprs, .. } => exprs.iter().for_each(f),
        }
    }

    /// The same, mutably; false as soon as `f` refuses an expression.
    pub(super) fn csp_item_exprs_mut(
        it: &mut ConstraintItem,
        f: &mut dyn FnMut(&mut Expression) -> bool,
    ) -> bool {
        match it {
            ConstraintItem::Expr(e) => f(e),
            ConstraintItem::Inside {
                expr,
                range,
                dist_weights,
                ..
            } => {
                let mut ok = f(expr);
                for r in range.iter_mut() {
                    match r {
                        ConstraintRange::Value(v) => ok &= f(v),
                        ConstraintRange::Range { lo, hi } => {
                            ok &= f(lo);
                            ok &= f(hi);
                        }
                    }
                }
                for w in dist_weights.iter_mut().flatten() {
                    match w {
                        DistWeight::Each(e) | DistWeight::Total(e) => ok &= f(e),
                    }
                }
                ok
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => f(condition) && Self::csp_item_exprs_mut(constraint, f),
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                f(condition)
                    && Self::csp_item_exprs_mut(then_item, f)
                    && else_item
                        .as_mut()
                        .is_none_or(|e| Self::csp_item_exprs_mut(e, f))
            }
            ConstraintItem::Foreach { array, item, .. } => {
                f(array) && Self::csp_item_exprs_mut(item, f)
            }
            ConstraintItem::Solve {
                before_paths,
                after_paths,
                ..
            } => before_paths.iter_mut().chain(after_paths.iter_mut()).all(f),
            ConstraintItem::Soft(i) => Self::csp_item_exprs_mut(i, f),
            ConstraintItem::Block(items) => {
                items.iter_mut().all(|i| Self::csp_item_exprs_mut(i, f))
            }
            ConstraintItem::Unique { exprs, .. } => exprs.iter_mut().all(f),
        }
    }

    /// The direct sub-expressions of `e`, mutably (selects of identifier
    /// path segments included).
    pub(super) fn csp_children_mut(e: &mut Expression) -> Vec<&mut Expression> {
        let mut out: Vec<&mut Expression> = Vec::new();
        match &mut e.kind {
            ExprKind::Ident(h) => {
                for s in &mut h.path {
                    out.extend(s.selects.iter_mut());
                }
            }
            ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => out.push(operand),
            ExprKind::Binary { left, right, .. } | ExprKind::Range(left, right) => {
                out.push(left);
                out.push(right);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                out.push(condition);
                out.push(then_expr);
                out.push(else_expr);
            }
            ExprKind::Concatenation(xs) | ExprKind::SystemCall { args: xs, .. } => {
                out.extend(xs.iter_mut())
            }
            ExprKind::Replication { count, exprs } => {
                out.push(count);
                out.extend(exprs.iter_mut());
            }
            ExprKind::Call { func, args } => {
                out.push(func);
                out.extend(args.iter_mut());
            }
            ExprKind::Inside { expr, ranges } => {
                out.push(expr);
                out.extend(ranges.iter_mut());
            }
            ExprKind::MemberAccess { expr, .. } => out.push(expr),
            ExprKind::Index { expr, index } => {
                out.push(expr);
                out.push(index);
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                out.push(expr);
                out.push(left);
                out.push(right);
            }
            ExprKind::WithClause { expr, filter } => {
                out.push(expr);
                out.push(filter);
            }
            _ => {}
        }
        out
    }

    // -----------------------------------------------------------------
    // Element names
    // -----------------------------------------------------------------

    /// A select index known before the solve: a literal, or an expression
    /// of literals.
    fn csp_obj_index(&mut self, e: &Expression) -> Option<i64> {
        if let Some(k) = Self::try_const_u64(e) {
            return i64::try_from(k).ok();
        }
        let mut names = false;
        Self::walk_operands(e, &mut |x| {
            if matches!(
                x.kind,
                ExprKind::Ident(_) | ExprKind::Call { .. } | ExprKind::MemberAccess { .. }
            ) {
                names = true;
            }
        });
        if names {
            return None;
        }
        let v = self.eval_expr(e);
        if v.has_xz() || v.is_real || v.width == 0 || v.width > 64 {
            return None;
        }
        v.to_i64()
    }

    /// Name every element of an object array of the solve by one segment
    /// (`a[3].x` -> `a[3]` `.x`), bottom-up. False when an element is
    /// picked by an index the solve does not know beforehand.
    pub(super) fn csp_obj_canon(&mut self, e: &mut Expression, obj: &HashSet<String>) -> bool {
        for c in Self::csp_children_mut(e) {
            if !self.csp_obj_canon(c, obj) {
                return false;
            }
        }
        let span = e.span;
        match &mut e.kind {
            ExprKind::Ident(h) => {
                let mut changed = false;
                for j in 0..h.path.len().min(2) {
                    if j == 1 && h.path[0].name.name != "this" {
                        break;
                    }
                    if !obj.contains(&h.path[j].name.name) || h.path[j].selects.is_empty() {
                        continue;
                    }
                    if h.path[j].selects.len() != 1 {
                        return false;
                    }
                    let Some(k) = self.csp_obj_index(&h.path[j].selects[0]) else {
                        return false;
                    };
                    let s = &mut h.path[j];
                    s.name.name = format!("{}[{}]", s.name.name, k);
                    s.selects.clear();
                    changed = true;
                }
                if changed {
                    h.cached_signal_id.set(None);
                    h.cached_resolved_name = std::cell::OnceCell::new();
                    e.cached_width.set(None);
                }
                true
            }
            ExprKind::Index { expr, index } => {
                let ExprKind::Ident(h) = &expr.kind else {
                    return true;
                };
                let n = h.path.len();
                if h.root.is_some()
                    || h.path.iter().any(|s| !s.selects.is_empty())
                    || !(n == 1 || (n == 2 && h.path[0].name.name == "this"))
                    || !obj.contains(&h.path[n - 1].name.name)
                {
                    return true;
                }
                let Some(k) = self.csp_obj_index(index) else {
                    return false;
                };
                let mut path = h.path.clone();
                path[n - 1].name.name = format!("{}[{}]", path[n - 1].name.name, k);
                *e = ident_expr(path, span);
                true
            }
            ExprKind::MemberAccess { expr, member } => {
                // `a[k].x` held as a member access over the element:
                // one identifier path, as the parser shapes `a.b.x`.
                if let ExprKind::Ident(h) = &expr.kind
                    && h.root.is_none()
                    && h.path.iter().any(|s| csp_obj_split_set(&s.name.name, obj))
                    && h.path.last().is_some_and(|s| s.selects.is_empty())
                {
                    let mut path = h.path.clone();
                    path.push(seg(&member.name, member.span));
                    *e = ident_expr(path, span);
                }
                true
            }
            _ => true,
        }
    }

    /// The evaluator's spelling of a solver expression: every element name
    /// `a[k]` gets its select back. None when nothing changes.
    pub(super) fn csp_obj_real(e: &Expression) -> Option<Expression> {
        if !csp_obj_active() {
            return None;
        }
        let mut hit = false;
        Self::walk_operands(e, &mut |x| {
            if let ExprKind::Ident(h) = &x.kind
                && h.path.iter().any(|s| csp_obj_split(&s.name.name).is_some())
            {
                hit = true;
            }
        });
        if !hit {
            return None;
        }
        let mut out = e.clone();
        Self::csp_obj_real_mut(&mut out);
        Some(out)
    }

    fn csp_obj_real_mut(e: &mut Expression) {
        for c in Self::csp_children_mut(e) {
            Self::csp_obj_real_mut(c);
        }
        let span = e.span;
        let ExprKind::Ident(h) = &e.kind else {
            return;
        };
        let Some((j, (arr, k))) = h
            .path
            .iter()
            .enumerate()
            .find_map(|(j, s)| csp_obj_split(&s.name.name).map(|x| (j, x)))
        else {
            return;
        };
        // As the parser shapes `arr[k].a.b[i]`: an index over the array,
        // then member accesses, each select an index of its own.
        let mut head: Vec<crate::ast::expr::HierPathSegment> = h.path[..j].to_vec();
        head.push(seg(&arr, h.path[j].name.span));
        let mut out = Expression::new(
            ExprKind::Index {
                expr: Box::new(ident_expr(head, span)),
                index: Box::new(Self::literal_of(&Self::signed_loop_val(k), span)),
            },
            span,
        );
        for s in &h.path[j + 1..] {
            out = Expression::new(
                ExprKind::MemberAccess {
                    expr: Box::new(out),
                    member: s.name.clone(),
                },
                span,
            );
            for x in &s.selects {
                out = Expression::new(
                    ExprKind::Index {
                        expr: Box::new(out),
                        index: Box::new(x.clone()),
                    },
                    span,
                );
            }
        }
        *e = out;
    }

    /// `csp_obj_real` over a whole item.
    pub(super) fn csp_obj_real_item(it: &ConstraintItem) -> ConstraintItem {
        let mut out = it.clone();
        if csp_obj_active() {
            Self::csp_item_exprs_mut(&mut out, &mut |e| {
                if let Some(r) = Self::csp_obj_real(e) {
                    *e = r;
                }
                true
            });
        }
        out
    }

    // -----------------------------------------------------------------
    // Unrolling
    // -----------------------------------------------------------------

    /// The parent's constraint set as the joint solve reads it: each
    /// `foreach` that picks an element of an object array by its loop index
    /// unrolled, and every element named by one segment. None when an
    /// element is picked by a random index, or such a loop cannot be
    /// unrolled.
    pub(super) fn csp_obj_prepare(
        &mut self,
        csp: &Csp,
        constraints: &[ClassConstraint],
        obj: &HashSet<String>,
    ) -> Option<Vec<ClassConstraint>> {
        let mut out = Vec::with_capacity(constraints.len());
        for c in constraints {
            let mut items = Vec::with_capacity(c.items.len());
            for it in &c.items {
                self.csp_unroll_item(csp.handle, Some((csp, obj)), it, &mut items)?;
            }
            for it in &mut items {
                if !Self::csp_item_exprs_mut(it, &mut |e| self.csp_obj_canon(e, obj)) {
                    return None;
                }
            }
            let mut c2 = c.clone();
            c2.items = items;
            out.push(c2);
        }
        Some(out)
    }

    /// Unroll the `foreach` loops of `it` into `out`. With `parent` (the
    /// enclosing solve and its object arrays) only a loop whose body picks an
    /// element of an object array by a loop index is unrolled; without it
    /// (a sub-object's own items, to be rewritten through its handle) every
    /// loop is. The indices come from the loop's array: an object array, a
    /// solver array, or a fixed-shape member of `handle`.
    pub(super) fn csp_unroll_item(
        &mut self,
        handle: usize,
        parent: Option<(&Csp, &HashSet<String>)>,
        it: &ConstraintItem,
        out: &mut Vec<ConstraintItem>,
    ) -> Option<()> {
        let one = |me: &mut Self, x: &ConstraintItem| -> Option<Box<ConstraintItem>> {
            let mut v = Vec::new();
            me.csp_unroll_item(handle, parent, x, &mut v)?;
            Some(Box::new(if v.len() == 1 {
                v.pop().unwrap()
            } else {
                ConstraintItem::Block(v)
            }))
        };
        match it {
            ConstraintItem::Foreach {
                array,
                vars,
                item,
                span,
            } => {
                let names: Vec<String> = vars.iter().flatten().map(|v| v.name.clone()).collect();
                let unroll = match parent {
                    None => true,
                    Some((_, obj)) => Self::csp_obj_picked_by(item, &names, obj),
                };
                if !unroll {
                    out.push(ConstraintItem::Foreach {
                        array: array.clone(),
                        vars: vars.clone(),
                        item: one(self, item)?,
                        span: *span,
                    });
                    return Some(());
                }
                let [Some(iv)] = vars.as_slice() else {
                    return None;
                };
                let idx = self.csp_unroll_indices(handle, parent, array)?;
                for i in idx {
                    let mut body = (**item).clone();
                    let lit = Self::literal_of(&Self::signed_loop_val(i), iv.span);
                    Self::subst_ident_item(&mut body, &iv.name, &lit);
                    self.csp_unroll_item(handle, parent, &body, out)?;
                }
            }
            ConstraintItem::Block(items) => {
                let mut v = Vec::with_capacity(items.len());
                for x in items {
                    self.csp_unroll_item(handle, parent, x, &mut v)?;
                }
                out.push(ConstraintItem::Block(v));
            }
            ConstraintItem::Soft(x) => out.push(ConstraintItem::Soft(one(self, x)?)),
            ConstraintItem::Implication {
                condition,
                constraint,
                span,
            } => out.push(ConstraintItem::Implication {
                condition: condition.clone(),
                constraint: one(self, constraint)?,
                span: *span,
            }),
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                span,
            } => {
                let else_item = match else_item {
                    Some(e) => Some(one(self, e)?),
                    None => None,
                };
                out.push(ConstraintItem::IfElse {
                    condition: condition.clone(),
                    then_item: one(self, then_item)?,
                    else_item,
                    span: *span,
                });
            }
            _ => out.push(it.clone()),
        }
        Some(())
    }

    /// Some object-array element in `it` is picked by an index reading one
    /// of `vars`.
    fn csp_obj_picked_by(it: &ConstraintItem, vars: &[String], obj: &HashSet<String>) -> bool {
        let reads = |x: &Expression| {
            let mut hit = false;
            Self::walk_operands(x, &mut |y| {
                if let ExprKind::Ident(h) = &y.kind
                    && h.path.len() == 1
                    && vars.contains(&h.path[0].name.name)
                {
                    hit = true;
                }
            });
            hit
        };
        let mut hit = false;
        Self::csp_item_exprs(it, &mut |e| {
            Self::walk_operands(e, &mut |x| match &x.kind {
                ExprKind::Ident(h) => {
                    for s in h.path.iter().take(2) {
                        if obj.contains(&s.name.name) && s.selects.iter().any(reads) {
                            hit = true;
                        }
                    }
                }
                ExprKind::Index { expr, index } => {
                    if let ExprKind::Ident(h) = &expr.kind
                        && h.path.last().is_some_and(|s| obj.contains(&s.name.name))
                        && reads(index)
                    {
                        hit = true;
                    }
                }
                _ => {}
            })
        });
        hit
    }

    /// The first-dimension indices of `foreach (array[i])` for unrolling.
    fn csp_unroll_indices(
        &mut self,
        handle: usize,
        parent: Option<(&Csp, &HashSet<String>)>,
        array: &Expression,
    ) -> Option<Vec<i64>> {
        let name = Self::foreach_base_name(array)?;
        if let Some((csp, obj)) = parent {
            if obj.contains(&name) {
                let keys = self.rand_child_collection_keys(csp.handle, &name)?;
                let prefix = format!("{}#{}[", csp.handle, name);
                return keys
                    .iter()
                    .map(|k| {
                        k.strip_prefix(&prefix)
                            .and_then(|r| r.strip_suffix(']'))
                            .and_then(|r| r.parse::<i64>().ok())
                    })
                    .collect();
            }
            if let Some(a) = csp.arrays.get(&name) {
                return Some(a.elems.iter().map(|e| e.0).collect());
            }
        }
        self.csp_foreach_indices_of(handle, array, &name)
    }
}

fn csp_obj_split_set(name: &str, obj: &HashSet<String>) -> bool {
    name.split_once('[')
        .is_some_and(|(a, r)| obj.contains(a) && r.ends_with(']'))
}
