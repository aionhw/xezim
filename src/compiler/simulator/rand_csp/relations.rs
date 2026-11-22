//! Dynamic selects and domains, shifts, concatenations, relations, `inside` and members.

use super::*;

impl Simulator {
    /// §7.4/§11.5.1: a relation reading an element of a solver array, or a
    /// bit of a solver variable, at a position that itself depends on solver
    /// variables (`rights[i*16 + id[i]] == 4'b1011`, `mask[id] == 0`). Per
    /// position `k` the select names one variable, so the relation holds as
    /// `idx == k -> rel[k]` for every `k`: each implication propagates (a
    /// decided index pins its element; a refuted element removes the
    /// position from the index). The original relation is also judged as a
    /// whole, which keeps an out-of-range index exact. None when no such
    /// select is found or the expansion would be too large.
    pub(super) fn csp_dyn_expand(
        &mut self,
        csp: &mut Csp,
        e: &Expression,
        env: &Env,
        neg: bool,
    ) -> Option<Node> {
        let depth = CSP_DYN_DEPTH.with(|c| c.get());
        if depth >= 2 || (csp.arrays.is_empty() && csp.segs.is_empty()) {
            return None;
        }
        let ph = format!("__xz_dyn_ix{depth}");
        let mut body = e.clone();
        let (idx, dom) = self.csp_take_dyn_select(csp, &mut body, env, &ph)?;
        if dom.is_empty() || dom.len() > 256 {
            return None;
        }
        CSP_DYN_DEPTH.with(|c| c.set(depth + 1));
        let out = self.csp_dyn_cases(csp, e, &body, idx, &dom, &ph, env, neg);
        CSP_DYN_DEPTH.with(|c| c.set(depth));
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn csp_dyn_cases(
        &mut self,
        csp: &mut Csp,
        whole: &Expression,
        body: &Expression,
        idx: Expression,
        dom: &[i64],
        ph: &str,
        env: &Env,
        neg: bool,
    ) -> Option<Node> {
        let cond = Expression::new(
            ExprKind::Binary {
                op: BinaryOp::Eq,
                left: Box::new(idx),
                right: Box::new(Self::csp_ph_ident(ph, whole.span)),
            },
            whole.span,
        );
        // The declared domains, to drop positions the index never reaches.
        let st0 = St {
            d: csp.dom0.clone(),
            trail: Vec::new(),
            dirty: Vec::new(),
            work: 0,
            est_work: 0,
            cut: false,
            level_tries: 0,
            level_cut: false,
            probing: false,
        };
        let mut out = Vec::with_capacity(dom.len() + 1);
        for &k in dom {
            let mut ek = env.clone();
            ek.binds.push((ph.to_string(), Self::signed_loop_val(k)));
            let c = self.csp_bool(csp, &cond, &ek, false)?;
            let never = match &c {
                Node::False => true,
                Node::Lin {
                    lin,
                    rel: Rel::Eq,
                    fits,
                    ..
                } if st0.fits_hold(fits) => {
                    let (lo, hi) = st0.lin_bounds(lin);
                    lo > 0 || hi < 0
                }
                _ => false,
            };
            if never {
                continue;
            }
            let then = self.csp_bool(csp, body, &ek.guarded(&c), neg)?;
            if matches!(c, Node::True) {
                out.push(then);
                continue;
            }
            if matches!(then, Node::True) {
                continue;
            }
            let nc = self.csp_bool(csp, &cond, &ek, true)?;
            out.push(Node::If {
                cond: Box::new(c),
                ncond: Box::new(nc),
                then: Box::new(then),
                els: Box::new(Node::True),
            });
        }
        // The relation as a whole, for an index outside every case. The
        // cases already decide every position, so a select of a wide
        // variable here may wait for all of its segments.
        let prev = CSP_DYN_WHOLE.with(|c| c.replace(true));
        let guard = self.csp_eval_node(csp, SrcItem::Expr(whole.clone()), env, neg);
        CSP_DYN_WHOLE.with(|c| c.set(prev));
        out.push(guard?);
        Some(Self::csp_and(out))
    }

    fn csp_ph_ident(ph: &str, span: crate::ast::Span) -> Expression {
        Expression::new(
            ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                root: None,
                path: vec![crate::ast::expr::HierPathSegment {
                    name: crate::ast::Identifier {
                        name: ph.to_string(),
                        span,
                    },
                    selects: Vec::new(),
                }],
                span,
                cached_signal_id: crate::ast::cache::SignalCache::new(None),
                cached_resolved_name: crate::ast::cache::NameCache::new(),
                bound: crate::ast::binding::Bound::UNSET,
            }),
            span,
        )
    }

    /// The first select in `e` (outermost first) whose index reads solver
    /// variables and names an element of a solver array or a bit of a
    /// solver variable; its index is replaced in place by the identifier
    /// `ph`. Returns that index and the positions the select can name.
    fn csp_take_dyn_select(
        &mut self,
        csp: &Csp,
        e: &mut Expression,
        env: &Env,
        ph: &str,
    ) -> Option<(Expression, Vec<i64>)> {
        let hit = match &e.kind {
            ExprKind::Index { expr, index } if !self.csp_free(csp, index, env) => {
                self.csp_dyn_domain(csp, expr, env)
            }
            ExprKind::Ident(h)
                if h.root.is_none()
                    && h.path.last().is_some_and(|s| s.selects.len() == 1)
                    && h.path[..h.path.len() - 1]
                        .iter()
                        .all(|s| s.selects.is_empty())
                    && !self.csp_free(csp, &h.path.last().unwrap().selects[0], env) =>
            {
                let mut bh = h.clone();
                bh.path.last_mut().unwrap().selects.clear();
                let base = Expression::new(ExprKind::Ident(bh), e.span);
                let arr = self
                    .csp_member(csp, &base)
                    .and_then(|n| csp.arrays.get(&n))
                    .map(|a| a.elems.iter().map(|x| x.0).collect());
                // §7.4.5: or the elements of a packed-array field.
                if arr.is_none() && !csp.field_elems.is_empty() {
                    let mut konst = |me: &mut Self, x: &Expression| -> Option<i64> {
                        if !me.csp_free(csp, x, env) {
                            return None;
                        }
                        me.csp_const(x, env)?.to_i64()
                    };
                    self.csp_field_elem_all(csp, &base, &mut konst)
                        .map(|(_, all)| all.iter().map(|x| x.0).collect())
                } else {
                    arr
                }
            }
            _ => None,
        };
        if let Some(dom) = hit {
            let ph = Self::csp_ph_ident(ph, e.span);
            let idx = match &mut e.kind {
                ExprKind::Index { index, .. } => std::mem::replace(&mut **index, ph),
                ExprKind::Ident(h) => {
                    h.cached_signal_id.set(None);
                    h.cached_resolved_name = crate::ast::cache::NameCache::new();
                    std::mem::replace(&mut h.path.last_mut().unwrap().selects[0], ph)
                }
                _ => return None,
            };
            e.cached_width.set(None);
            return Some((idx, dom));
        }
        let found = match &mut e.kind {
            ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                self.csp_take_dyn_select(csp, operand, env, ph)
            }
            ExprKind::Binary { left, right, .. } | ExprKind::Range(left, right) => self
                .csp_take_dyn_select(csp, left, env, ph)
                .or_else(|| self.csp_take_dyn_select(csp, right, env, ph)),
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => self
                .csp_take_dyn_select(csp, condition, env, ph)
                .or_else(|| self.csp_take_dyn_select(csp, then_expr, env, ph))
                .or_else(|| self.csp_take_dyn_select(csp, else_expr, env, ph)),
            ExprKind::Concatenation(xs) | ExprKind::SystemCall { args: xs, .. } => {
                let mut r = None;
                for x in xs {
                    r = self.csp_take_dyn_select(csp, x, env, ph);
                    if r.is_some() {
                        break;
                    }
                }
                r
            }
            ExprKind::Inside { expr, ranges } => {
                let mut r = self.csp_take_dyn_select(csp, expr, env, ph);
                for x in ranges {
                    if r.is_some() {
                        break;
                    }
                    r = self.csp_take_dyn_select(csp, x, env, ph);
                }
                r
            }
            ExprKind::Index { expr, index } => self
                .csp_take_dyn_select(csp, expr, env, ph)
                .or_else(|| self.csp_take_dyn_select(csp, index, env, ph)),
            ExprKind::RangeSelect { expr, .. } | ExprKind::MemberAccess { expr, .. } => {
                self.csp_take_dyn_select(csp, expr, env, ph)
            }
            _ => None,
        };
        if found.is_some() {
            e.cached_width.set(None);
        }
        found
    }

    /// The positions a select of `base` can name: the indices of a solver
    /// array, or the declared bit indices of a solver variable or field.
    fn csp_dyn_domain(&mut self, csp: &Csp, base: &Expression, env: &Env) -> Option<Vec<i64>> {
        if let Some(a) = self.csp_member(csp, base).and_then(|n| csp.arrays.get(&n)) {
            return Some(a.elems.iter().map(|x| x.0).collect());
        }
        let mut konst = |me: &mut Self, x: &Expression| -> Option<i64> {
            if !me.csp_free(csp, x, env) {
                return None;
            }
            me.csp_const(x, env)?.to_i64()
        };
        // §7.4.1: the labels of the dimension a select of a
        // multi-dimensional packed variable indexes.
        if let Some(p) = self.csp_pdim_ref(csp, base, &mut konst) {
            let p = p?;
            let &(l, r) = csp.pdims.get(&p.v)?.get(p.depth)?;
            return Some((l.min(r)..=l.max(r)).collect());
        }
        if let Some((_, all)) = self.csp_field_elem_all(csp, base, &mut konst) {
            return Some(all.iter().map(|x| x.0).collect());
        }
        let (v, _, w, whole) = self.csp_slice_base_k(csp, base, &mut konst)?;
        let w = i64::from(w);
        if !whole {
            return Some((0..w).collect());
        }
        let l = csp.lsb.get(&v).copied().unwrap_or(0);
        Some(if csp.ascending.contains(&v) {
            (l - w + 1..=l).collect()
        } else {
            (l..l + w).collect()
        })
    }

    /// `(a >> k) REL c` or `(a / d) REL c` -> `a inside [lo:hi]`,
    /// for a constant shift amount or positive divisor (§11.4.3) and
    /// a constant right-hand side. Returns None when the shape does not apply,
    /// leaving the caller's normal affine path untouched.
    ///
    /// Restricted to UNSIGNED `a` and the logical `>>`. For a signed value
    /// `>>` still zero-fills (only `>>>` sign-fills), so the
    /// `a >> k == floor(a / 2**k)` identity this relies on does not hold once
    /// `a` can be negative.
    #[allow(clippy::too_many_arguments)]
    fn csp_shr_rel(
        &mut self,
        csp: &mut Csp,
        whole: &Expression,
        op: BinaryOp,
        left: &Expression,
        right: &Expression,
        env: &Env,
        neg: bool,
    ) -> Option<Node> {
        // Shift on one side, constant on the other; flip the relation when the
        // shift is on the right so `c REL (a >> k)` reads the same way.
        let (shift, other, op) = match &Self::unparen(left).kind {
            ExprKind::Binary {
                op: BinaryOp::ShiftRight | BinaryOp::Div,
                ..
            } => (left, right, op),
            _ => match &Self::unparen(right).kind {
                ExprKind::Binary {
                    op: BinaryOp::ShiftRight | BinaryOp::Div,
                    ..
                } => (
                    right,
                    left,
                    match op {
                        BinaryOp::Lt => BinaryOp::Gt,
                        BinaryOp::Leq => BinaryOp::Geq,
                        BinaryOp::Gt => BinaryOp::Lt,
                        BinaryOp::Geq => BinaryOp::Leq,
                        o => o,
                    },
                ),
                _ => return None,
            },
        };
        let ExprKind::Binary {
            op: operation,
            left: base,
            right: amt,
        } = &Self::unparen(shift).kind
        else {
            return None;
        };
        if !self.csp_free(csp, amt, env) || !self.csp_free(csp, other, env) {
            return None;
        }
        let amount = self.csp_const(amt, env)?;
        let m = if *operation == BinaryOp::Div {
            if amount.is_signed && amount.to_i64()? <= 0 {
                return None;
            }
            let divisor = amount.to_u64()?;
            if divisor == 0 {
                return None;
            }
            i128::from(divisor)
        } else {
            let k = amount.to_u64()?;
            if k >= 64 {
                return None;
            }
            1i128 << k
        };
        let c = i128::from(self.csp_const(other, env)?.to_i64()?);
        if c < 0 {
            return None;
        }
        let a = self.csp_ae(csp, base, env)?;
        let (w, sg) = Self::csp_ws(&a);
        if sg || w == 0 || w > 64 {
            return None;
        }
        let (_, amax) = ws_range(w, false);
        // Each relation as one inclusive interval on `a`, before clamping.
        let (mut lo, mut hi) = match op {
            BinaryOp::Eq | BinaryOp::CaseEq | BinaryOp::Neq | BinaryOp::CaseNeq => {
                (c * m, c * m + m - 1)
            }
            BinaryOp::Lt => (0, c * m - 1),
            BinaryOp::Leq => (0, c * m + m - 1),
            BinaryOp::Gt => (c * m + m, amax),
            BinaryOp::Geq => (c * m, amax),
            _ => return None,
        };
        lo = lo.max(0);
        hi = hi.min(amax);
        // `!=` is the complement of the `==` interval, which Node::In's own
        // `neg` expresses; fold it together with the caller's negation.
        let inv = matches!(op, BinaryOp::Neq | BinaryOp::CaseNeq) != neg;
        let mut fits = Vec::new();
        let mut cx = LinCx::new(csp);
        let lin = Self::csp_lin(&a, w, false, &mut fits, &mut cx)?;
        let carries = cx.take();
        self.csp_add_carries(csp, carries);
        fits.push(Fit {
            lin: lin.clone(),
            lo: 0,
            hi: amax,
            modw: 0,
        });
        let set: Dom = if lo > hi { Vec::new() } else { vec![(lo, hi)] };
        let mut deps: Vec<usize> = lin.t.iter().map(|t| t.0).collect();
        for f in &fits {
            deps.extend(f.lin.t.iter().map(|t| t.0));
        }
        deps.sort_unstable();
        deps.dedup();
        let src = self.csp_src(csp, SrcItem::Expr(whole.clone()), env, deps);
        Some(Node::In {
            lin,
            set,
            neg: inv,
            fits,
            src,
            wrap: None,
        })
    }

    /// `LHS REL (expr << v)` (either way round) with `v` a solver variable
    /// of a small domain: a shift by a variable amount is not affine, so
    /// the relation would drop to a `Fun` node that only narrows its
    /// variable once every variable the shift reads is fixed — a pinned
    /// left side (`xfer_byte_cnt == (req.len + 1) << req.size` with
    /// `req.len <= 255` and `req.size <= 4`) then defeats the search,
    /// which cannot refute the relation within its budget although
    /// reasoning over the cases proves it instantly. Per value `k` of `v`
    /// the shift is by a constant, so the case is the affine
    /// `LHS REL expr * 2**k` (§11.4.10, `csp_ae`); the guard `v == k`
    /// makes it hold only at that value, and a refuted case refutes its
    /// guard, so propagation alone settles the relation. The cases cover
    /// the whole declared domain of `v`, so the split is exact.
    #[allow(clippy::too_many_arguments)]
    fn csp_shl_split(
        &mut self,
        csp: &mut Csp,
        whole: &Expression,
        op: BinaryOp,
        left: &Expression,
        right: &Expression,
        env: &Env,
        neg: bool,
    ) -> Option<Node> {
        // The shift on one side; the relation keeps its direction, since
        // each case rebuilds the same comparison with the amount replaced.
        let (shift, shift_left) = match &Self::unparen(left).kind {
            ExprKind::Binary {
                op: BinaryOp::ShiftLeft | BinaryOp::ArithShiftLeft,
                ..
            } => (left, true),
            _ => match &Self::unparen(right).kind {
                ExprKind::Binary {
                    op: BinaryOp::ShiftLeft | BinaryOp::ArithShiftLeft,
                    ..
                } => (right, false),
                _ => return None,
            },
        };
        let ExprKind::Binary {
            op: shift_op,
            left: base,
            right: amt,
        } = &Self::unparen(shift).kind
        else {
            return None;
        };
        // A variable amount: a constant one is already affine (`csp_ae`).
        if self.csp_free(csp, amt, env) {
            return None;
        }
        // The amount must be one solver variable of a small domain of
        // non-negative values below 63: the cases enumerate it, and the
        // constant arm reads `a << k` as `a * 2**k` only for such a k.
        let Ae::Var(v, ..) = self.csp_ae(csp, amt, env)? else {
            return None;
        };
        let dom = csp.dom0[v].clone();
        if dom_size(&dom) > 64 || dom.iter().any(|&(l, h)| l < 0 || h >= 63) {
            return None;
        }
        let ph = "__xz_shl_ix";
        let amt_ph = Self::csp_ph_ident(ph, whole.span);
        // `v == k`: the guard of the case for `k`.
        let cond = Expression::new(
            ExprKind::Binary {
                op: BinaryOp::Eq,
                left: amt.clone(),
                right: Box::new(amt_ph.clone()),
            },
            whole.span,
        );
        // The relation with the amount replaced by the placeholder, which
        // the case binds to its value.
        let body = |amt_ph: Expression| {
            let shifted = Expression::new(
                ExprKind::Binary {
                    op: *shift_op,
                    left: base.clone(),
                    right: Box::new(amt_ph),
                },
                whole.span,
            );
            let (l, r) = if shift_left {
                (shifted, right.clone())
            } else {
                (left.clone(), shifted)
            };
            Expression::new(
                ExprKind::Binary {
                    op,
                    left: Box::new(l),
                    right: Box::new(r),
                },
                whole.span,
            )
        };
        let mut out = Vec::new();
        for &(l, h) in &dom {
            for k in l..=h {
                let mut ek = env.clone();
                ek.binds
                    .push((ph.to_string(), Self::signed_loop_val(k as i64)));
                let c = self.csp_bool(csp, &cond, &ek, false)?;
                if matches!(c, Node::False) {
                    continue;
                }
                let then = self.csp_bool(csp, &body(amt_ph.clone()), &ek.guarded(&c), neg)?;
                if matches!(then, Node::True) {
                    continue;
                }
                if matches!(c, Node::True) {
                    out.push(then);
                    continue;
                }
                let nc = self.csp_bool(csp, &cond, &ek, true)?;
                out.push(Node::If {
                    cond: Box::new(c),
                    ncond: Box::new(nc),
                    then: Box::new(then),
                    els: Box::new(Node::True),
                });
            }
        }
        Some(Self::csp_and(out))
    }

    /// §11.4.12/§11.6: split aligned equality into fields to avoid sparse search.
    /// Singleton concatenations preserve unsigned bits and self-determined widths.
    fn csp_concat_eq(
        &mut self,
        csp: &mut Csp,
        left: &Expression,
        right: &Expression,
        env: &Env,
    ) -> Option<Node> {
        fn fields<'a>(e: &'a Expression, out: &mut Vec<&'a Expression>) {
            let e = Simulator::unparen(e);
            if let ExprKind::Concatenation(parts) = &e.kind {
                for part in parts {
                    fields(part, out);
                }
            } else {
                out.push(e);
            }
        }
        if !matches!(Self::unparen(left).kind, ExprKind::Concatenation(_))
            || !matches!(Self::unparen(right).kind, ExprKind::Concatenation(_))
        {
            return None;
        }
        let (mut ls, mut rs) = (Vec::new(), Vec::new());
        fields(left, &mut ls);
        fields(right, &mut rs);
        // A singleton also terminates recursive translation of the fields.
        if ls.len() < 2 || ls.len() != rs.len() {
            return None;
        }
        for (&l, &r) in ls.iter().zip(&rs) {
            let (wl, _) = Self::csp_ws(&self.csp_ae(csp, l, env)?);
            let (wr, _) = Self::csp_ws(&self.csp_ae(csp, r, env)?);
            if wl == 0 || wl > 64 || wl != wr {
                return None;
            }
        }
        let mut nodes = Vec::with_capacity(ls.len());
        for (l, r) in ls.into_iter().zip(rs) {
            let l = Expression::new(ExprKind::Concatenation(vec![l.clone()]), l.span);
            let r = Expression::new(ExprKind::Concatenation(vec![r.clone()]), r.span);
            let whole = Expression::new(
                ExprKind::Binary {
                    op: BinaryOp::Eq,
                    left: Box::new(l.clone()),
                    right: Box::new(r.clone()),
                },
                left.span,
            );
            nodes.push(self.csp_rel(csp, &whole, BinaryOp::Eq, &l, &r, env, false)?);
        }
        Some(Self::csp_and(nodes))
    }

    /// `left op right` (§11.6.1: both sides extended to the context width
    /// and signedness).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn csp_rel(
        &mut self,
        csp: &mut Csp,
        whole: &Expression,
        op: BinaryOp,
        left: &Expression,
        right: &Expression,
        env: &Env,
        neg: bool,
    ) -> Option<Node> {
        if matches!(op, BinaryOp::Eq | BinaryOp::CaseEq)
            && !neg
            && let Some(n) = self.csp_concat_eq(csp, left, right, env)
        {
            return Some(n);
        }
        // A relation on a wide variable is modelled over its segments; one
        // that only reads single segments (a field, an aligned select) also
        // fits the forms below, where the segment is an ordinary variable.
        let wide = !csp.wide.is_empty()
            && (self.csp_wide_touch(csp, left, env) || self.csp_wide_touch(csp, right, env));
        if wide && let Some(n) = self.csp_wide_rel(csp, op, left, right, env, neg) {
            return Some(n);
        }
        if wide
            && matches!(op, BinaryOp::Eq | BinaryOp::CaseEq)
            && !neg
            && let Some(n) = self
                .csp_wide_fun(csp, whole, left, right, env)
                .or_else(|| self.csp_wide_fun(csp, whole, right, left, env))
        {
            return Some(n);
        }
        let fallback = |me: &mut Self, csp: &mut Csp| match me
            .csp_rel_fun(csp, whole, op, left, right, env, neg)
        {
            Some(n) => Some(n),
            None => me.csp_eval_node(csp, SrcItem::Expr(whole.clone()), env, neg),
        };
        // `(a >> k) REL c` with a constant k and an unsigned `a` is a RANGE on
        // `a`: the shift is floor division by 2**k, so each relation maps onto
        // one interval. Without this the relation is not affine, csp_ae fails
        // and the whole constraint drops to the trial loop — where
        // `(x >> 8) == 0` on a 32-bit rand has probability 2**-24 and
        // randomize() reports failure (issue #229) even though `x < 256`,
        // its exact equivalent, solves instantly.
        if !wide && let Some(n) = self.csp_shr_rel(csp, whole, op, left, right, env, neg) {
            return Some(n);
        }
        // A shift by a VARIABLE amount on one side: split it over the
        // amount's values (`csp_shl_split`) before the affine attempt
        // below fails on it and the relation drops to a late `Fun` node
        // that cannot refute a pinned web.
        if !wide && let Some(n) = self.csp_shl_split(csp, whole, op, left, right, env, neg) {
            return Some(n);
        }
        if matches!(op, BinaryOp::Eq | BinaryOp::CaseEq) && !neg && !wide {
            if let Some(n) = self
                .csp_masked_eq(csp, whole, left, right, env)
                .or_else(|| self.csp_masked_eq(csp, whole, right, left, env))
            {
                return Some(n);
            }
            self.csp_note_mask(csp, whole, left, right, env);
            self.csp_note_mask(csp, whole, right, left, env);
        }
        let (Some(a), Some(b)) = (self.csp_ae(csp, left, env), self.csp_ae(csp, right, env)) else {
            return fallback(self, csp);
        };
        let ((wl, sl), (wr, sr)) = (Self::csp_ws(&a), Self::csp_ws(&b));
        let w = wl.max(wr).max(1);
        if w > 64 {
            return fallback(self, csp);
        }
        // §5.7.1: an unsized `'0`/`'1` fills the width of its context; as a
        // 1-bit constant, `y == '1` solved as `y == 1`.
        let fill = |e: &Expression, ae: Ae| match &Self::unparen(e).kind {
            ExprKind::Number(crate::ast::expr::NumberLiteral::UnbasedUnsized('1')) => {
                Ae::Const(Value::from_u64(u64::MAX >> (64 - w), w))
            }
            ExprKind::Number(crate::ast::expr::NumberLiteral::UnbasedUnsized('0')) => {
                Ae::Const(Value::zero(w))
            }
            _ => ae,
        };
        let (a, b) = (fill(left, a), fill(right, b));
        let s = sl && sr;
        let mut fits = Vec::new();
        let mut cx = LinCx::new(csp);
        let (Some(la), Some(lb)) = (
            Self::csp_lin(&a, w, s, &mut fits, &mut cx),
            Self::csp_lin(&b, w, s, &mut fits, &mut cx),
        ) else {
            return fallback(self, csp);
        };
        let carries = cx.take();
        let (rlo, rhi) = ws_range(w, s);
        let nleaf = fits.len();
        fits.push(Fit {
            lin: la.clone(),
            lo: rlo,
            hi: rhi,
            modw: 0,
        });
        fits.push(Fit {
            lin: lb.clone(),
            lo: rlo,
            hi: rhi,
            modw: 0,
        });
        let Some(d) = la.plus(&lb, -1) else {
            return fallback(self, csp);
        };
        let op = if neg {
            match op {
                BinaryOp::Eq | BinaryOp::CaseEq => BinaryOp::Neq,
                BinaryOp::Neq | BinaryOp::CaseNeq => BinaryOp::Eq,
                BinaryOp::Lt => BinaryOp::Geq,
                BinaryOp::Leq => BinaryOp::Gt,
                BinaryOp::Gt => BinaryOp::Leq,
                _ => BinaryOp::Lt,
            }
        } else {
            op
        };
        let (lin, rel) = match op {
            BinaryOp::Eq | BinaryOp::CaseEq => (Some(d), Rel::Eq),
            BinaryOp::Neq | BinaryOp::CaseNeq => (Some(d), Rel::Ne),
            BinaryOp::Lt => (d.shift(1), Rel::Le),
            BinaryOp::Leq => (Some(d), Rel::Le),
            BinaryOp::Gt => (d.scale(-1).and_then(|x| x.shift(1)), Rel::Le),
            _ => (d.scale(-1), Rel::Le),
        };
        let Some(lin) = lin else {
            return fallback(self, csp);
        };
        self.csp_add_carries(csp, carries);
        let mut deps: Vec<usize> = lin.t.iter().map(|t| t.0).collect();
        for f in &fits {
            deps.extend(f.lin.t.iter().map(|t| t.0));
        }
        deps.sort_unstable();
        deps.dedup();
        let src = self.csp_src(csp, SrcItem::Expr(whole.clone()), env, deps);
        let sg = if matches!(op, BinaryOp::Gt | BinaryOp::Geq) {
            -1
        } else {
            1
        };
        let wrap = Some(Wrap { w, s, n: nleaf, sg });
        Some(Node::Lin {
            lin,
            rel,
            fits,
            src,
            sneg: neg,
            wrap,
        })
    }

    /// `x OP e` (either way round) with `x` a solver variable and `e` an
    /// expression of OTHER variables the linear model cannot express: a
    /// `Fun` node (see there). None when no side qualifies.
    #[allow(clippy::too_many_arguments)]
    fn csp_rel_fun(
        &mut self,
        csp: &mut Csp,
        whole: &Expression,
        op: BinaryOp,
        left: &Expression,
        right: &Expression,
        env: &Env,
        neg: bool,
    ) -> Option<Node> {
        if !matches!(
            op,
            BinaryOp::Eq
                | BinaryOp::CaseEq
                | BinaryOp::Neq
                | BinaryOp::CaseNeq
                | BinaryOp::Lt
                | BinaryOp::Leq
                | BinaryOp::Gt
                | BinaryOp::Geq
        ) {
            return None;
        }
        let op = if neg {
            match op {
                BinaryOp::Eq | BinaryOp::CaseEq => BinaryOp::Neq,
                BinaryOp::Neq | BinaryOp::CaseNeq => BinaryOp::Eq,
                BinaryOp::Lt => BinaryOp::Geq,
                BinaryOp::Leq => BinaryOp::Gt,
                BinaryOp::Gt => BinaryOp::Leq,
                _ => BinaryOp::Lt,
            }
        } else {
            op
        };
        let mirror = |o: BinaryOp| match o {
            BinaryOp::Lt => BinaryOp::Gt,
            BinaryOp::Leq => BinaryOp::Geq,
            BinaryOp::Gt => BinaryOp::Lt,
            BinaryOp::Geq => BinaryOp::Leq,
            o => o,
        };
        let plain_var = |me: &mut Self, csp: &Csp, e: &Expression| match me.csp_ae(csp, e, env) {
            Some(Ae::Var(v, ..)) => Some(v),
            _ => None,
        };
        let reads = |me: &mut Self, csp: &Csp, e: &Expression| -> Option<Vec<usize>> {
            let mut d = Vec::new();
            // A field of a wide variable reads only its own segments.
            if !csp.wide.is_empty() && me.csp_wide_deps(csp, e, env, &mut d) {
                return Some(d);
            }
            d.clear();
            me.csp_refs(csp, e, env, &mut d).then_some(d)
        };
        let mut pick = None;
        if let Some(v) = plain_var(self, csp, left) {
            if let Some(d) = reads(self, csp, right).filter(|d| !d.is_empty() && !d.contains(&v)) {
                pick = Some((v, right, op, d));
            }
        }
        if pick.is_none() {
            if let Some(v) = plain_var(self, csp, right) {
                if let Some(d) = reads(self, csp, left).filter(|d| !d.is_empty() && !d.contains(&v))
                {
                    pick = Some((v, left, mirror(op), d));
                }
            }
        }
        let (var, other, op, mut deps) = pick?;
        deps.sort_unstable();
        deps.dedup();
        let other_src = self.csp_src(csp, SrcItem::Expr(other.clone()), env, deps.clone());
        deps.push(var);
        deps.sort_unstable();
        deps.dedup();
        let whole_src = self.csp_src(csp, SrcItem::Expr(whole.clone()), env, deps);
        Some(Node::Fun {
            var,
            op,
            other: other_src,
            whole: whole_src,
            neg,
        })
    }

    /// `expr inside {ranges}` with constant ranges; judged like the checker
    /// judges it (`value_in_ranges` on the operand's own value).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn csp_inside(
        &mut self,
        csp: &mut Csp,
        expr: &Expression,
        ranges: &[ConstraintRange],
        env: &Env,
        neg: bool,
        whole: SrcItem,
    ) -> Option<Node> {
        if !csp.wide.is_empty()
            && self.csp_wide_touch(csp, expr, env)
            && let Some(n) = self.csp_wide_inside(csp, expr, ranges, env, neg)
        {
            return Some(n);
        }
        let ranges_free = ranges.iter().all(|r| match r {
            ConstraintRange::Value(e) => self.csp_free(csp, e, env),
            ConstraintRange::Range { lo, hi } => {
                self.csp_free(csp, lo, env) && self.csp_free(csp, hi, env)
            }
        });
        if !ranges_free && !neg {
            // §11.4.13: a bound that reads a rand variable (`dst inside
            // {[BASE : TOP - size*4]}`) makes each range the pair of
            // relations `lo <= expr <= hi` (a value: `expr == v`), and the
            // set their disjunction — propagated rather than left to the
            // evaluator, which only judges it once every operand is fixed.
            let rel = |op: BinaryOp, l: &Expression, r: &Expression| {
                Expression::new(
                    ExprKind::Binary {
                        op,
                        left: Box::new(l.clone()),
                        right: Box::new(r.clone()),
                    },
                    expr.span,
                )
            };
            let mut alts = Vec::with_capacity(ranges.len());
            for r in ranges {
                let n = match r {
                    ConstraintRange::Value(v) => {
                        // a rand array operand is set membership, not `==`
                        if self
                            .csp_member(csp, v)
                            .is_some_and(|n| csp.arrays.contains_key(&n))
                        {
                            return self.csp_eval_node(csp, whole, env, neg);
                        }
                        self.csp_bool(csp, &rel(BinaryOp::Eq, expr, v), env, false)?
                    }
                    ConstraintRange::Range { lo, hi } => {
                        if matches!(lo.kind, ExprKind::Dollar)
                            || matches!(hi.kind, ExprKind::Dollar)
                        {
                            return self.csp_eval_node(csp, whole, env, neg);
                        }
                        let a = self.csp_bool(csp, &rel(BinaryOp::Leq, lo, expr), env, false)?;
                        let b = self.csp_bool(csp, &rel(BinaryOp::Leq, expr, hi), env, false)?;
                        Node::And(vec![a, b])
                    }
                };
                alts.push(n);
            }
            return Some(match alts.len() {
                1 => alts.pop().unwrap(),
                _ => Node::Or(alts),
            });
        }
        let a = if ranges_free {
            self.csp_ae(csp, expr, env)
        } else {
            None
        };
        let Some(a) = a else {
            return self.csp_eval_node(csp, whole, env, neg);
        };
        let (w, s) = Self::csp_ws(&a);
        // §18.5.9: the items are constants the evaluator reads below, so an
        // element of an object array (`a[k].ranges.size()`, state of a
        // sub-object of the solve) is spelled as the evaluator spells it.
        // Under the solver's name it read as an unknown name, and the set
        // built from that refuted a problem that has solutions.
        let real_ranges: Vec<ConstraintRange>;
        let ranges: &[ConstraintRange] = if ranges.iter().any(|r| match r {
            ConstraintRange::Value(e) => Self::csp_obj_real(e).is_some(),
            ConstraintRange::Range { lo, hi } => {
                Self::csp_obj_real(lo).is_some() || Self::csp_obj_real(hi).is_some()
            }
        }) {
            let real = |e: &Expression| Self::csp_obj_real(e).unwrap_or_else(|| e.clone());
            real_ranges = ranges
                .iter()
                .map(|r| match r {
                    ConstraintRange::Value(e) => ConstraintRange::Value(real(e)),
                    ConstraintRange::Range { lo, hi } => ConstraintRange::Range {
                        lo: real(lo),
                        hi: real(hi),
                    },
                })
                .collect();
            &real_ranges
        } else {
            ranges
        };
        // §11.4.13: the operand and the items are compared like `==`, at the
        // width of all of them and unsigned unless all are signed: `a8 +
        // 8'd1 inside {[256:256]}` adds at 32 bits.
        let frame = self.csp_frame(&env.binds);
        self.push_local_frame(frame);
        let joint = self.inside_items_ws(ranges);
        self.pop_local_frame();
        let Some((iw, is)) = joint else {
            return self.csp_eval_node(csp, whole, env, neg);
        };
        // A signed operand against an unsigned item: the comparison is
        // unsigned, of the operand extended by its own sign (as
        // `value_in_ranges` judges it). Keep the operand signed and map the
        // items' unsigned bit patterns to the signed values they are.
        let mixed = s && !is;
        let (w, s) = (w.max(iw), s && (is || mixed));
        let mut fits = Vec::new();
        let mut cx = LinCx::new(csp);
        let lin = if w <= 64 {
            Self::csp_lin(&a, w, s, &mut fits, &mut cx)
        } else {
            None
        };
        let carries = cx.take();
        let Some(lin) = lin else {
            return self.csp_eval_node(csp, whole, env, neg);
        };
        self.csp_add_carries(csp, carries);
        let frame = self.csp_frame(&env.binds);
        self.push_local_frame(frame);
        let mut set = dom_norm(self.cons_ranges_i128_typed(ranges, w, s && !mixed).0);
        self.pop_local_frame();
        if mixed {
            let half = 1i128 << (w - 1);
            let mut signed = Vec::with_capacity(set.len() + 1);
            for &(l, h) in set.iter() {
                let (l, h) = (Self::bits_at(l, w) as i128, Self::bits_at(h, w) as i128);
                if l > h {
                    continue;
                }
                if l < half {
                    signed.push((l, h.min(half - 1)));
                }
                if h >= half {
                    signed.push((l.max(half) - (half << 1), h - (half << 1)));
                }
            }
            set = dom_norm(signed);
        }
        // A negative endpoint makes the checker read an unsigned value as
        // signed; leave that reading to the checker itself.
        if !s && set.iter().any(|r| r.0 < 0) {
            return self.csp_eval_node(csp, whole, env, neg);
        }
        let (rlo, rhi) = ws_range(w, s);
        let nleaf = fits.len();
        fits.push(Fit {
            lin: lin.clone(),
            lo: rlo,
            hi: rhi,
            modw: 0,
        });
        let mut deps: Vec<usize> = lin.t.iter().map(|t| t.0).collect();
        for f in &fits {
            deps.extend(f.lin.t.iter().map(|t| t.0));
        }
        deps.sort_unstable();
        deps.dedup();
        let src = self.csp_src(csp, whole, env, deps);
        Some(Node::In {
            lin,
            set,
            neg,
            fits,
            src,
            wrap: Some(Wrap {
                w,
                s,
                n: nleaf,
                sg: 1,
            }),
        })
    }

    /// The joint (width, signed) of an `inside` set's items (§11.4.13): the
    /// widest item, signed only when every item is. A `$` bound counts as
    /// neither; an unpacked-array item counts its elements. None when an
    /// item cannot be sized exactly.
    pub(in crate::compiler::simulator) fn inside_items_ws(
        &mut self,
        ranges: &[ConstraintRange],
    ) -> Option<(u32, bool)> {
        let (mut w, mut s) = (1u32, true);
        let mut one = |me: &mut Self, e: &Expression, item: bool| -> Option<()> {
            if matches!(e.kind, ExprKind::Dollar) {
                return Some(());
            }
            if item && let Some(nm) = me.array_operand_name(e) {
                for i in 0..me.get_queue_size(&nm) {
                    let ev = me.get_signal_value_by_name(&format!("{}[{}]", nm, i))?;
                    w = w.max(ev.width);
                    s &= ev.is_signed;
                }
                return Some(());
            }
            let (_, ew, es) = me.exact_int(e)?;
            w = w.max(ew);
            s &= es;
            Some(())
        };
        for r in ranges {
            match r {
                ConstraintRange::Value(e) => one(self, e, true)?,
                ConstraintRange::Range { lo, hi } => {
                    one(self, lo, false)?;
                    one(self, hi, false)?;
                }
            }
        }
        Some((w, s))
    }

    /// The object's own member named by `e` (`x`, `this.x`, `obj.x` for the
    /// inline-constraint receiver), if it is a solver scalar or array.
    pub(super) fn csp_member(&self, csp: &Csp, e: &Expression) -> Option<String> {
        let known = |n: &str| {
            csp.scalars.contains_key(n) || csp.arrays.contains_key(n) || csp.nd_dims.contains_key(n)
        };
        let recv_ok = |me: &Self, r: &str| r == "this" || me.rand_receiver.as_deref() == Some(r);
        // A rand sub-object's member (`a.x`, `this.a.x`, `t.a.x`): a
        // `prop.name` variable when the sub-object joined the solve.
        // (Also `s.x`, a member of an unpacked struct member.)
        if (csp.has_subs || csp.dotted)
            && let Some(mut path) = Self::csp_member_path(e)
        {
            if path.len() >= 3 && (path[0] == "this" || recv_ok(self, &path[0])) {
                path.remove(0);
            }
            // `a.x`, or `a.b.x` for a sub-object of a sub-object.
            if path.len() >= 2 {
                let n = path.join(".");
                if known(&n) {
                    return Some(n);
                }
                if let Some(n) = self.csp_member_through_handles(csp, &path) {
                    return Some(n);
                }
            }
        }
        let name = match &e.kind {
            ExprKind::Ident(h) if h.path.iter().all(|s| s.selects.is_empty()) => match h.path.len()
            {
                1 => h.path[0].name.name.clone(),
                2 if recv_ok(self, &h.path[0].name.name) => h.path[1].name.name.clone(),
                _ => return None,
            },
            ExprKind::MemberAccess { expr, member } => match &expr.kind {
                ExprKind::This => member.name.clone(),
                ExprKind::Ident(h) if h.path.len() == 1 && recv_ok(self, &h.path[0].name.name) => {
                    member.name.clone()
                }
                _ => return None,
            },
            _ => return None,
        };
        known(&name).then_some(name)
    }

    /// §18.5.9: a member path through handle members (`sub.p.x`, `p` a
    /// sub-object's handle back to the solved object) names that object's
    /// solver variable or array.
    pub(super) fn csp_member_through_handles(&self, csp: &Csp, path: &[String]) -> Option<String> {
        let (leaf, hops) = path.split_last()?;
        let h = hops.iter().try_fold(csp.handle, |h, m| {
            // `a[k]`: an element of an object array of the solve.
            if let Some((arr, k)) = m.split_once('[') {
                let k = k.strip_suffix(']')?;
                return self
                    .read_coll_elem(&format!("{h}#{arr}[{k}]"))
                    .and_then(|v| v.to_u64())
                    .map(|v| v as usize)
                    .filter(|&h| h != 0);
            }
            self.member_handle(h, m).filter(|&h| h != 0)
        })?;
        let has_member = |n: &str| {
            csp.scalars.contains_key(n)
                || csp.arrays.contains_key(n)
                || csp.nd_dims.contains_key(n)
                || csp.sizes.contains_key(n)
        };
        if h == csp.handle && has_member(leaf) {
            return Some(leaf.clone());
        }
        if let Some(prefix) = csp.sub_prefixes.get(&h) {
            let cand = format!("{prefix}.{leaf}");
            if has_member(&cand) {
                return Some(cand);
            }
        }
        csp.scalars.iter().find_map(|(n, &v)| {
            let hit = match &csp.vars[v].key {
                VarKey::Prop(p) => h == csp.handle && p == leaf,
                VarKey::Sub(s, p) => *s == h && p == leaf,
                _ => false,
            };
            hit.then(|| n.clone())
        })
    }

    /// The names of a plain member path (`a.b`, `this.a.b`), `this` kept.
    /// The segments of a non-wide variable split by `csp_split_slices` that
    /// a field path (`cmd.base`, `this.cmd.base`) spans. None for anything
    /// else, including nested fields.
    pub(super) fn csp_field_segs(&self, csp: &Csp, e: &Expression) -> Option<Vec<usize>> {
        let mut path = Self::csp_member_path(e)?;
        if path.len() > 2
            && (path[0] == "this" || self.rand_receiver.as_deref() == Some(path[0].as_str()))
        {
            path.remove(0);
        }
        let [var, field] = path.as_slice() else {
            return None;
        };
        let &v = csp.scalars.get(var)?;
        if csp.wide.contains(&v) {
            return None;
        }
        let segs = csp.segs.get(&v)?;
        let &(_, lo, w) = csp.fields.get(&v)?.iter().find(|f| &f.0 == field)?;
        Some(
            segs.iter()
                .filter(|s| s.0 < lo + w && s.0 + s.1 > lo)
                .map(|s| s.2)
                .collect(),
        )
    }

    pub(super) fn csp_member_path(e: &Expression) -> Option<Vec<String>> {
        match &e.kind {
            ExprKind::This => Some(vec!["this".to_string()]),
            ExprKind::Ident(h) => {
                if h.root.is_some() || h.path.iter().any(|s| !s.selects.is_empty()) {
                    return None;
                }
                Some(h.path.iter().map(|s| s.name.name.clone()).collect())
            }
            ExprKind::MemberAccess { expr, member } => {
                let mut v = Self::csp_member_path(expr)?;
                v.push(member.name.clone());
                Some(v)
            }
            // §18.4: `rg.item[0]`, an element of a fixed array member of an
            // unpacked struct, is a solver variable of its own.
            ExprKind::Index { expr, index } => {
                let mut path = Self::csp_member_path(expr)?;
                let i = super::elaborate::const_eval_i64_with_params(index, None)?;
                path.last_mut()?.push_str(&format!("[{i}]"));
                Some(path)
            }
            _ => None,
        }
    }
}
