//! References, arithmetic expressions, sums and linear forms.

use super::*;

impl Simulator {
    /// Variables `e` reads. False when it calls something whose reads are
    /// unknown (a user function).
    pub(super) fn csp_refs(
        &self,
        csp: &Csp,
        e: &Expression,
        env: &Env,
        out: &mut Vec<usize>,
    ) -> bool {
        // A field of a variable split into segments reads the segments it
        // spans, so a relation on `cmd.base` and `cmd.sz` is judged once
        // those are fixed rather than once all of `cmd` is.
        if matches!(e.kind, ExprKind::Ident(_) | ExprKind::MemberAccess { .. })
            && let Some(segs) = self.csp_field_segs(csp, e)
        {
            out.extend(segs);
            return true;
        }
        // A wide variable is read through its segments.
        let push_var = |v: usize, out: &mut Vec<usize>| match csp.segs.get(&v) {
            Some(segs) if csp.wide.contains(&v) => out.extend(segs.iter().map(|x| x.2)),
            _ => out.push(v),
        };
        let push_name = |n: &str, out: &mut Vec<usize>| {
            if let Some(&v) = csp.scalars.get(n) {
                push_var(v, out);
            } else if let Some(&sv) = csp.sizes.get(n) {
                // Its size is a variable of the size phase.
                out.push(sv);
            } else if let Some(a) = csp.arrays.get(n) {
                for x in &a.elems {
                    push_var(x.1, out);
                }
            } else if csp.nd_dims.contains_key(n) {
                // Every element of a multi-dimensional array (#519).
                let prefix = format!("{n}[");
                for (rn, a) in &csp.arrays {
                    if rn.starts_with(&prefix) {
                        for x in &a.elems {
                            push_var(x.1, out);
                        }
                    }
                }
            }
        };
        if let Some((n, idx)) = Self::csp_struct_elem(csp, e) {
            push_name(&n, out);
            return self.csp_refs(csp, &idx, env, out);
        }
        match &e.kind {
            ExprKind::Ident(h) => {
                if h.path.len() == 1 {
                    let n = &h.path[0].name.name;
                    if env.binds.iter().any(|b| &b.0 == n) {
                        return true;
                    }
                    if let Some((it, v, _)) = &env.it {
                        if it == n {
                            out.push(*v);
                            return true;
                        }
                    }
                }
                // §18.4 (#519): `a[i][j]` of a multi-dimensional solver
                // array reads that element of its row — every element of
                // the array when an index is not yet known.
                if !csp.nd_dims.is_empty()
                    && h.root.is_none()
                    && h.path.last().is_some_and(|s| !s.selects.is_empty())
                    && h.path[..h.path.len() - 1]
                        .iter()
                        .all(|s| s.selects.is_empty())
                {
                    let mut bh = h.clone();
                    let sels = std::mem::take(&mut bh.path.last_mut().unwrap().selects);
                    let base = Expression::new(ExprKind::Ident(bh), e.span);
                    if let Some(bn) = self.csp_member(csp, &base)
                        && csp.nd_dims.contains_key(&bn)
                    {
                        let consts: Option<Vec<i64>> =
                            sels.iter().map(|s| Self::nd_const_index(s, env)).collect();
                        match consts {
                            Some(cs) if !cs.is_empty() => {
                                let mut row = bn.clone();
                                for c in &cs[..cs.len() - 1] {
                                    row.push_str(&format!("[{c}]"));
                                }
                                if let Some(a) = csp.arrays.get(&row)
                                    && let Some(x) =
                                        a.elems.iter().find(|x| x.0 == cs[cs.len() - 1])
                                {
                                    push_var(x.1, out);
                                    return true;
                                }
                            }
                            _ => {}
                        }
                        let prefix = format!("{bn}[");
                        for (n, a) in &csp.arrays {
                            if n.starts_with(&prefix) {
                                for x in &a.elems {
                                    push_var(x.1, out);
                                }
                            }
                        }
                        return true;
                    }
                }
                if let Some(n) = self.csp_member(csp, e) {
                    push_name(&n, out);
                } else if let Some(seg) = h.path.last() {
                    // `a.b` on some other handle can only name a member of
                    // this object through an alias; stay conservative.
                    if h.path.len() == 1 {
                        push_name(&seg.name.name, out);
                    } else if csp.scalars.contains_key(&h.path[0].name.name) {
                        // A field of a packed-struct variable (`s.lo`) reads
                        // the variable.
                        push_name(&h.path[0].name.name, out);
                    } else if csp.has_subs || csp.dotted {
                        // A field or select of a sub-object's variable or
                        // back-referenced owner variable (`a.b.x.f`,
                        // `p[0].item.hdr.lo`, `p[0].item.arr[i].f`) reads
                        // that variable or array.
                        let p0 = &h.path[0].name.name;
                        let st = usize::from(
                            p0 == "this" || self.rand_receiver.as_deref() == Some(p0.as_str()),
                        );
                        let mut found = false;
                        for split in (st + 1..=h.path.len()).rev() {
                            if h.path[st..split - 1]
                                .iter()
                                .any(|s| !s.selects.is_empty())
                            {
                                continue;
                            }
                            let prefix: Vec<String> = h.path[st..split]
                                .iter()
                                .map(|s| s.name.name.clone())
                                .collect();
                            let joined = prefix.join(".");
                            let matched = if h.path[split - 1].selects.is_empty()
                                && (csp.scalars.contains_key(&joined)
                                    || csp.arrays.contains_key(&joined)
                                    || csp.nd_dims.contains_key(&joined))
                            {
                                Some(joined)
                            } else if !h.path[split - 1].selects.is_empty()
                                && (csp.arrays.contains_key(&joined)
                                    || csp.nd_dims.contains_key(&joined))
                            {
                                Some(joined)
                            } else if prefix.len() >= 2 {
                                self.csp_member_through_handles(csp, &prefix)
                            } else {
                                None
                            };
                            if let Some(n) = matched {
                                push_name(&n, out);
                                found = true;
                                break;
                            }
                        }
                        if !found && h.path.len() >= st + 2 {
                            let n =
                                format!("{}.{}", h.path[st].name.name, h.path[st + 1].name.name);
                            push_name(&n, out);
                        }
                    }
                }
                // The select indices read variables too (`f[id][7:0]`).
                let mut ok = true;
                for s in &h.path {
                    for x in &s.selects {
                        ok &= self.csp_refs(csp, x, env, out);
                    }
                }
                ok
            }
            ExprKind::MemberAccess { expr, member } => {
                if let Some(n) = self.csp_member(csp, e) {
                    push_name(&n, out);
                    return true;
                }
                // §8.11: `this.a…` names what the path `a…` names. Settle it
                // like the Ident arm — a lone member reads that member, a
                // field of a packed-struct variable reads the variable, and
                // anything else through a handle is state — rather than
                // descending to `this`, which reads as unanalyzable.
                if let Some(path) = Self::csp_member_path(e)
                    && path.len() >= 2
                    && path[0] == "this"
                {
                    let rest = &path[1..];
                    for split in (1..=rest.len()).rev() {
                        let prefix = &rest[..split];
                        let joined = prefix.join(".");
                        if csp.scalars.contains_key(&joined)
                            || csp.arrays.contains_key(&joined)
                            || csp.nd_dims.contains_key(&joined)
                        {
                            push_name(&joined, out);
                            return true;
                        }
                        if prefix.len() >= 2
                            && let Some(n) = self.csp_member_through_handles(csp, prefix)
                        {
                            push_name(&n, out);
                            return true;
                        }
                    }
                    push_name(&path[1], out);
                    return true;
                }
                if member.name == "index" {
                    if let ExprKind::Ident(h) = &expr.kind {
                        if let Some((it, _, _)) = &env.it {
                            if h.path.len() == 1 && &h.path[0].name.name == it {
                                return true;
                            }
                        }
                    }
                }
                self.csp_refs(csp, expr, env, out)
            }
            ExprKind::Call { func, args } => {
                let mut ok = match &func.kind {
                    ExprKind::MemberAccess { expr, .. } => self.csp_refs(csp, expr, env, out),
                    _ => false,
                };
                // A method call on a non-rand handle may still read members.
                if let ExprKind::MemberAccess { expr, .. } = &func.kind {
                    if self.csp_member(csp, expr).is_none()
                        && !matches!(&expr.kind, ExprKind::Ident(_))
                    {
                        ok = false;
                    }
                }
                for a in args {
                    ok &= self.csp_refs(csp, a, env, out);
                }
                ok
            }
            ExprKind::WithClause { expr, filter } => {
                let ok = self.csp_refs(csp, expr, env, out);
                // the filter reads `item`/`item.index`, covered by the array
                let mut tmp = Vec::new();
                let ok2 = self.csp_refs(csp, filter, env, &mut tmp);
                out.extend(tmp);
                ok && ok2
            }
            ExprKind::SystemCall { args, .. } => args
                .iter()
                .fold(true, |ok, a| self.csp_refs(csp, a, env, out) && ok),
            ExprKind::Unary { operand, .. } => self.csp_refs(csp, operand, env, out),
            ExprKind::Binary { left, right, .. } => {
                let a = self.csp_refs(csp, left, env, out);
                self.csp_refs(csp, right, env, out) && a
            }
            ExprKind::Paren(i) => self.csp_refs(csp, i, env, out),
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                let a = self.csp_refs(csp, condition, env, out);
                let b = self.csp_refs(csp, then_expr, env, out);
                self.csp_refs(csp, else_expr, env, out) && a && b
            }
            ExprKind::Inside { expr, ranges } => ranges
                .iter()
                .fold(self.csp_refs(csp, expr, env, out), |ok, r| {
                    self.csp_refs(csp, r, env, out) && ok
                }),
            ExprKind::Range(a, b) => {
                let x = self.csp_refs(csp, a, env, out);
                self.csp_refs(csp, b, env, out) && x
            }
            ExprKind::Index { expr, index } => {
                // §18.4 (#519): `a[i][j]` as nested indexes of a
                // multi-dimensional solver array reads one element when the
                // indices are known.
                if !csp.nd_dims.is_empty()
                    && let Some(v) = self.nd_chain_elem(csp, e, env)
                {
                    push_var(v, out);
                    return true;
                }
                // `a[i]` with `i` known reads that one element, so a relation
                // left to the evaluator (`(a[i] - a[i-1]) % 16 == 0` on signed
                // elements) is judged once its own elements are fixed, not
                // once every element of `a` is.
                if let Some(i) = Self::nd_const_index(index, env)
                    && let Some(a) = self.csp_member(csp, expr).and_then(|n| csp.arrays.get(&n))
                    && let Some(x) = a.elems.iter().find(|x| x.0 == i)
                {
                    push_var(x.1, out);
                    return true;
                }
                let x = self.csp_refs(csp, expr, env, out);
                self.csp_refs(csp, index, env, out) && x
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                let x = self.csp_refs(csp, expr, env, out);
                let y = self.csp_refs(csp, left, env, out);
                self.csp_refs(csp, right, env, out) && x && y
            }
            ExprKind::Concatenation(es) => es
                .iter()
                .fold(true, |ok, a| self.csp_refs(csp, a, env, out) && ok),
            ExprKind::Replication { count, exprs } => exprs
                .iter()
                .fold(self.csp_refs(csp, count, env, out), |ok, a| {
                    self.csp_refs(csp, a, env, out) && ok
                }),
            ExprKind::Number(_) | ExprKind::StringLiteral(_) | ExprKind::TypeLiteral(_) => true,
            _ => false,
        }
    }

    pub(super) fn csp_item_refs(
        &self,
        csp: &Csp,
        item: &ConstraintItem,
        env: &Env,
        out: &mut Vec<usize>,
    ) -> bool {
        match item {
            ConstraintItem::Expr(e) => self.csp_refs(csp, e, env, out),
            ConstraintItem::Inside { expr, range, .. } => {
                let mut ok = self.csp_refs(csp, expr, env, out);
                for r in range {
                    ok &= match r {
                        ConstraintRange::Value(e) => self.csp_refs(csp, e, env, out),
                        ConstraintRange::Range { lo, hi } => {
                            let a = self.csp_refs(csp, lo, env, out);
                            self.csp_refs(csp, hi, env, out) && a
                        }
                    };
                }
                ok
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => {
                let a = self.csp_refs(csp, condition, env, out);
                self.csp_item_refs(csp, constraint, env, out) && a
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                let a = self.csp_refs(csp, condition, env, out);
                let b = self.csp_item_refs(csp, then_item, env, out);
                let c = else_item
                    .as_ref()
                    .is_none_or(|e| self.csp_item_refs(csp, e, env, out));
                a && b && c
            }
            ConstraintItem::Foreach { array, item, .. } => {
                // the loop index is not a variable; the array's elements are
                let a = self.csp_refs(csp, array, env, out);
                let mut inner = Vec::new();
                let b = self.csp_item_refs(csp, item, env, &mut inner);
                out.extend(inner);
                a && b
            }
            ConstraintItem::Solve { .. } => true,
            ConstraintItem::Soft(i) => self.csp_item_refs(csp, i, env, out),
            ConstraintItem::Block(items) => items
                .iter()
                .fold(true, |ok, i| self.csp_item_refs(csp, i, env, out) && ok),
            ConstraintItem::Unique { exprs, .. } => exprs
                .iter()
                .fold(true, |ok, e| self.csp_refs(csp, e, env, out) && ok),
        }
    }

    /// Reads no solver variable (and calls nothing opaque).
    pub(super) fn csp_free(&self, csp: &Csp, e: &Expression, env: &Env) -> bool {
        let mut v = Vec::new();
        self.csp_refs(csp, e, env, &mut v) && v.is_empty()
    }

    /// Evaluate a variable-free expression with the foreach indices bound.
    pub(super) fn csp_const_any(&mut self, e: &Expression, env: &Env) -> Value {
        let real = Self::csp_obj_real(e);
        let e = real.as_ref().unwrap_or(e);
        let frame = self.csp_frame(&env.binds);
        self.push_local_frame(frame);
        let v = self.eval_expr(e);
        self.pop_local_frame();
        v
    }

    pub(super) fn csp_const(&mut self, e: &Expression, env: &Env) -> Option<Value> {
        let v = self.csp_const_any(e, env);
        (!v.has_xz() && !v.is_real && v.width > 0 && v.width <= 64).then_some(v)
    }

    /// Arithmetic operand tree of `e`, or None when it is not linear. A
    /// variable wider than 64 bits is no operand: it is only modelled
    /// through its segments (`csp_wide_rel`).
    pub(super) fn csp_ae(&mut self, csp: &Csp, e: &Expression, env: &Env) -> Option<Ae> {
        let a = self.csp_ae_raw(csp, e, env)?;
        if !csp.wide.is_empty() && matches!(a, Ae::Var(v, ..) if csp.wide.contains(&v)) {
            return None;
        }
        Some(a)
    }

    fn csp_ae_raw(&mut self, csp: &Csp, e: &Expression, env: &Env) -> Option<Ae> {
        let e = Self::unparen(e);
        // §18.4 (#519): an element of a multi-dimensional solver array
        // (`a[i][j]`, as nested indexes or as selects on the identifier)
        // is the row's element variable.
        if !csp.nd_dims.is_empty()
            && matches!(e.kind, ExprKind::Index { .. } | ExprKind::Ident(_))
            && let Some(v) = self.nd_chain_elem(csp, e, env)
        {
            return Some(Self::csp_var_ae(csp, v));
        }
        // the `with` iterator and its index
        if let Some((it, v, i)) = &env.it {
            match &e.kind {
                ExprKind::Ident(h) if h.path.len() == 1 && &h.path[0].name.name == it => {
                    return Some(Self::csp_var_ae(csp, *v));
                }
                ExprKind::MemberAccess { expr, member } if member.name == "index" => {
                    if let ExprKind::Ident(h) = &expr.kind {
                        if h.path.len() == 1 && &h.path[0].name.name == it {
                            // §7.12.4: `index` is an int
                            let mut ix = Value::from_u64(*i as u64, 32);
                            ix.is_signed = true;
                            return Some(Ae::Const(ix));
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some((n, idx)) = Self::csp_struct_elem(csp, e) {
            if !self.csp_free(csp, &idx, env) {
                return None;
            }
            let i = self.csp_const(&idx, env)?.to_i64()?;
            return csp.arrays[&n]
                .elems
                .iter()
                .find(|x| x.0 == i)
                .map(|x| Self::csp_var_ae(csp, x.1));
        }
        if matches!(
            e.kind,
            ExprKind::RangeSelect { .. } | ExprKind::Index { .. } | ExprKind::MemberAccess { .. }
        ) || (!csp.segs.is_empty() && matches!(e.kind, ExprKind::Ident(_)))
        {
            if let Some(a) = self.csp_slice_ae(csp, e, env) {
                return Some(a);
            }
        }
        if let Some(n) = self.csp_member(csp, e) {
            if !env.binds.iter().any(|b| b.0 == n) {
                return csp.scalars.get(&n).map(|&v| Self::csp_var_ae(csp, v));
            }
        }
        match &e.kind {
            // §11.4.12: concatenation operands retain their own widths
            // before their unsigned bit fields are joined.
            ExprKind::Concatenation(parts) => {
                if parts.len() == 2 && self.csp_free(csp, &parts[0], env) {
                    let k = self.csp_const(&parts[0], env)?;
                    if k.to_u64() == Some(0) && !k.is_signed {
                        let a = self.csp_ae(csp, &parts[1], env)?;
                        let (w, signed) = Self::csp_ws(&a);
                        if !signed {
                            return self.csp_cast(
                                csp,
                                &parts[1],
                                w.checked_add(k.width)?,
                                Some(false),
                                env,
                            );
                        }
                    }
                }
                if parts.is_empty() {
                    return None;
                }
                let mut terms = Vec::with_capacity(parts.len());
                let mut width = 0u32;
                for part in parts.iter().rev() {
                    let a = self.csp_ae(csp, part, env)?;
                    let (w, signed) = Self::csp_ws(&a);
                    if w == 0 || width.checked_add(w)? > 64 {
                        return None;
                    }
                    let a = Ae::Cast {
                        inner: Box::new(a),
                        iw: w,
                        is: signed,
                        w,
                        s: false,
                    };
                    terms.push(Ae::Shl(Box::new(a), u64::from(width)));
                    width += w;
                }
                return Some(Ae::Sum {
                    terms,
                    w: width,
                    s: false,
                });
            }
            ExprKind::Unary {
                op: UnaryOp::Plus,
                operand,
            } => return self.csp_ae(csp, operand, env),
            ExprKind::Unary {
                op: UnaryOp::Minus,
                operand,
            } => return Some(Ae::Neg(Box::new(self.csp_ae(csp, operand, env)?))),
            // Preserve the shift until comparison/cast context supplies its
            // width. A coefficient built at the left operand's own width
            // would truncate `1 << 48` to zero before a 64-bit comparison.
            ExprKind::Binary {
                op: BinaryOp::ShiftLeft | BinaryOp::ArithShiftLeft,
                left,
                right,
            } if self.csp_free(csp, right, env) => {
                if let Some(k) = self.csp_const(right, env).and_then(|v| v.to_u64()) {
                    if let Some(a) = self.csp_ae(csp, left, env) {
                        return Some(Ae::Shl(Box::new(a), k));
                    }
                }
            }
            ExprKind::Binary {
                op: BinaryOp::ShiftRight,
                left,
                right,
            } if self.csp_free(csp, right, env) => {
                if let Some(a) = self.csp_shr_slice(csp, left, right, env) {
                    return Some(a);
                }
            }
            ExprKind::Binary {
                op: op @ (BinaryOp::Div | BinaryOp::Mod),
                left,
                right,
            } if self.csp_free(csp, right, env) => {
                let divisor = self.csp_const(right, env)?;
                let a = self.csp_ae(csp, left, env)?;
                return Some(Ae::Quot(Box::new(a), divisor, *op == BinaryOp::Mod));
            }
            ExprKind::Binary {
                op: BinaryOp::Power,
                left,
                right,
            } if self.csp_free(csp, right, env) => {
                // §11.4.3: squaring has the same context width and modulo
                // behavior as multiplication of the base by itself.
                if self.csp_const(right, env)?.to_u64()? == 2 {
                    let a = self.csp_ae(csp, left, env)?;
                    return Some(Ae::Mul(Box::new(a.clone()), Box::new(a)));
                }
            }
            ExprKind::Binary {
                op: op @ (BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul),
                left,
                right,
            } => {
                let (l, r) = (self.csp_ae(csp, left, env)?, self.csp_ae(csp, right, env)?);
                let (l, r) = (Box::new(l), Box::new(r));
                return Some(match op {
                    BinaryOp::Add => Ae::Add(l, r),
                    BinaryOp::Sub => Ae::Sub(l, r),
                    _ => Ae::Mul(l, r),
                });
            }
            ExprKind::Index { expr, index } => {
                if let Some(a) = self.csp_member(csp, expr).and_then(|n| csp.arrays.get(&n)) {
                    if !self.csp_free(csp, index, env) {
                        return None;
                    }
                    let i = self.csp_const(index, env)?.to_i64()?;
                    return a
                        .elems
                        .iter()
                        .find(|x| x.0 == i)
                        .map(|x| Self::csp_var_ae(csp, x.1));
                }
            }
            ExprKind::SystemCall { name, args } => match (name.as_str(), args.as_slice()) {
                ("$__xz_type_cast", [t, inner]) => {
                    let ExprKind::TypeLiteral(dt) = &t.kind else {
                        return None;
                    };
                    if self.dt_is_real(dt) {
                        return None;
                    }
                    let w = resolve_type_width(
                        dt,
                        Some(&self.module.parameters),
                        Some(&self.module.typedefs),
                    );
                    let s = is_type_signed(dt);
                    return self.csp_cast(csp, inner, w, Some(s), env);
                }
                ("$__xz_size_cast", [n, inner]) if self.csp_free(csp, n, env) => {
                    let w = self.csp_const(n, env)?.to_u64()? as u32;
                    return self.csp_cast(csp, inner, w, None, env);
                }
                // `T'(e)` with `T` a typedef/enum, or a constant size.
                ("$__xz_named_cast", [t, inner]) => {
                    let nm = self.named_cast_key(t)?;
                    let nm = nm.as_str();
                    let (w, s) = if let Some(dt) = self.module.typedef_types.get(nm) {
                        if self.dt_is_real(dt) {
                            return None;
                        }
                        (self.cast_context_width(dt), Some(is_type_signed(dt)))
                    } else if let Some(&w) = self.module.typedefs.get(nm) {
                        (w, Some(false))
                    } else if self.csp_free(csp, t, env) {
                        (self.csp_const(t, env)?.to_u64()? as u32, None)
                    } else {
                        return None;
                    };
                    return self.csp_cast(csp, inner, w, s, env);
                }
                // §20.9: `$countones(x)` of a variable split into single
                // bits is the (int) sum of those bits.
                ("$countones", [x]) => {
                    if let Some(a) = self.csp_countones(csp, x, env) {
                        return Some(a);
                    }
                }
                ("$signed", [inner]) | ("$unsigned", [inner]) => {
                    let a = self.csp_ae(csp, inner, env)?;
                    let (iw, _) = Self::csp_ws(&a);
                    return self.csp_cast(csp, inner, iw, Some(name == "$signed"), env);
                }
                _ => {}
            },
            _ => {}
        }
        // §18.5.8.1: in the size phase an array's size is a variable.
        if !csp.sizes.is_empty()
            && let ExprKind::Call { func, args } = &e.kind
            && args.is_empty()
            && let ExprKind::MemberAccess { expr, member } = &func.kind
            && member.name == "size"
            && let Some(&sv) = self
                .csp_member(csp, expr)
                .or_else(|| self.csp_row_of(csp, expr, env))
                .and_then(|n| csp.sizes.get(&n))
        {
            return Some(Self::csp_var_ae(csp, sv));
        }
        // The size of a wide solver array is fixed while it is solved.
        if !csp.wide.is_empty()
            && let ExprKind::Call { func, args } = &e.kind
            && args.is_empty()
            && let ExprKind::MemberAccess { expr, member } = &func.kind
            && member.name == "size"
            && let Some(a) = self.csp_member(csp, expr).and_then(|n| csp.arrays.get(&n))
            && a.elems.first().is_some_and(|x| csp.wide.contains(&x.1))
        {
            let mut v = Value::from_u64(a.elems.len() as u64, 32);
            v.is_signed = true;
            return Some(Ae::Const(v));
        }
        if let Some(sum) = self.csp_sum(csp, e, env) {
            return sum;
        }
        // `arr.size()` of a solver array is a constant here
        if let ExprKind::Call { func, args } = &e.kind {
            if let ExprKind::MemberAccess { expr, member } = &func.kind {
                if args.is_empty() && member.name == "size" {
                    if let Some(a) = self.csp_member(csp, expr).and_then(|n| csp.arrays.get(&n)) {
                        let mut v = Value::from_u64(a.elems.len() as u64, 32);
                        v.is_signed = true;
                        return Some(Ae::Const(v));
                    }
                    // A collection reached through a state handle has a
                    // fixed size too. Do not treat its built-in size query
                    // as an opaque method reading every rand member.
                    if self.csp_free(csp, expr, env) {
                        // (Of a sub-object that is an array element, too:
                        // `a[k].ranges.size()`, as the evaluator spells it.)
                        let real = Self::csp_obj_real(expr);
                        let expr = real.as_ref().unwrap_or(expr);
                        let frame = self.csp_frame(&env.binds);
                        self.push_local_frame(frame);
                        let is_array = self.array_operand_name(expr).is_some()
                            || self.expr_assoc_name(expr).is_some();
                        self.pop_local_frame();
                        if is_array {
                            return self.csp_const(e, env).map(Ae::Const);
                        }
                    }
                    // §7.4.5: a row's size (`a[1].size()`), and the number
                    // of rows of a member over rows (`a.size()`).
                    let n = match self.csp_row_of(csp, expr, env) {
                        Some(row) => csp.arrays.get(&row).map(|a| a.elems.len() as u64),
                        None => self
                            .csp_member(csp, expr)
                            .filter(|n| !csp.arrays.contains_key(n))
                            .and_then(|n| csp.nd_dims.get(&n))
                            .and_then(|d| d.first())
                            .map(|&(lo, hi)| (hi - lo + 1).max(0) as u64),
                    };
                    if let Some(n) = n {
                        let mut v = Value::from_u64(n, 32);
                        v.is_signed = true;
                        return Some(Ae::Const(v));
                    }
                }
            }
        }
        if self.csp_free(csp, e, env) {
            return self.csp_const(e, env).map(Ae::Const);
        }
        None
    }

    /// §11.4.10: `x >> k` with a constant `k` fills with zeros, so it is
    /// the select `x[w-1:k]` zero-extended to `x`'s width. `csp_scan_expr`
    /// cut `x` there, so the select is a variable of its own and a relation
    /// like `(base >> 16) == (last >> 16)` is linear in two selects instead
    /// of judged only once both sides are fixed.
    fn csp_shr_slice(
        &mut self,
        csp: &Csp,
        x: &Expression,
        k: &Expression,
        env: &Env,
    ) -> Option<Ae> {
        let k = self.csp_const(k, env)?.to_u64()?;
        let mut konst = |me: &mut Self, y: &Expression| -> Option<i64> {
            if !me.csp_free(csp, y, env) {
                return None;
            }
            me.csp_const(y, env)?.to_i64()
        };
        let (v, lo, w, _) = self.csp_slice_base_k(csp, x, &mut konst)?;
        if k == 0 || k >= u64::from(w) {
            return None;
        }
        let (slo, sw) = (lo + k as u32, w - k as u32);
        let sel = csp
            .segs
            .get(&v)
            .and_then(|segs| segs.iter().find(|s| s.0 == slo && s.1 == sw).map(|s| s.2))
            .or_else(|| csp.slice_vars.get(&(v, slo, sw)).copied())?;
        let (xw, xs) = Self::csp_ws(&self.csp_ae(csp, x, env)?);
        if xw != w || xw > 64 {
            return None;
        }
        Some(Ae::Cast {
            inner: Box::new(Ae::Var(sel, sw, false)),
            iw: sw,
            is: false,
            w: xw,
            s: xs,
        })
    }

    fn csp_countones(&mut self, csp: &Csp, x: &Expression, env: &Env) -> Option<Ae> {
        // §20.9: `$countones({a, b, ...})` is the sum of the parts' counts.
        if let ExprKind::Concatenation(parts) = &Self::unparen(x).kind {
            let terms = parts
                .iter()
                .map(|p| self.csp_countones(csp, p, env))
                .collect::<Option<Vec<_>>>()?;
            return Some(Ae::Sum {
                terms,
                w: 32,
                s: true,
            });
        }
        let mut konst = |me: &mut Self, e: &Expression| -> Option<i64> {
            if !me.csp_free(csp, e, env) {
                return None;
            }
            me.csp_const(e, env)?.to_i64()
        };
        let (v, lo, w, whole) = self.csp_slice_base_k(csp, x, &mut konst)?;
        // A variable split into single bits anyway (a select at a variable
        // position) counts them; a whole variable otherwise has its count.
        let bits: Option<Vec<usize>> = csp.segs.get(&v).and_then(|segs| {
            segs.iter()
                .filter(|s| s.0 >= lo && s.0 < lo + w)
                .map(|s| (s.1 == 1).then_some(s.2))
                .collect::<Option<Vec<_>>>()
        });
        let bits = match bits {
            Some(b) if b.len() == w as usize => b,
            _ => {
                let &c = csp.pop.get(&v)?;
                return (whole && lo == 0 && w == csp.vars[v].width)
                    .then(|| Self::csp_var_ae(csp, c));
            }
        };
        let terms = bits
            .into_iter()
            .map(|b| Ae::Cast {
                inner: Box::new(Ae::Var(b, 1, false)),
                iw: 32,
                is: false,
                w: 32,
                s: true,
            })
            .collect();
        Some(Ae::Sum {
            terms,
            w: 32,
            s: true,
        })
    }

    fn csp_cast(
        &mut self,
        csp: &Csp,
        inner: &Expression,
        w: u32,
        s: Option<bool>,
        env: &Env,
    ) -> Option<Ae> {
        let a = self.csp_ae(csp, inner, env)?;
        // §6.24.1: the cast width is the operand's context, so the operand
        // is evaluated at no less than the target width.
        let (iw, is) = Self::csp_ws(&a);
        let iw = iw.max(w);
        if w == 0 || w > 64 || iw > 64 {
            return None;
        }
        Some(Ae::Cast {
            inner: Box::new(a),
            iw,
            is,
            w,
            s: s.unwrap_or(is),
        })
    }

    /// `arr.sum()` / `arr.sum() with (f)` / `arr.sum(it) with (f)` over a
    /// solver array. `Some(None)` for a reduction the model cannot express.
    fn csp_sum(&mut self, csp: &Csp, e: &Expression, env: &Env) -> Option<Option<Ae>> {
        let (call, filter) = match &e.kind {
            ExprKind::WithClause { expr, filter } => (expr.as_ref(), Some(filter.as_ref())),
            _ => (e, None),
        };
        let (recv, method, args): (&Expression, &str, &[Expression]) = match &call.kind {
            ExprKind::Call { func, args } => match &func.kind {
                ExprKind::MemberAccess { expr, member } => (expr, member.name.as_str(), args),
                _ => return None,
            },
            ExprKind::MemberAccess { expr, member } => (expr, member.name.as_str(), &[]),
            _ => return None,
        };
        // `arr.size()` is a constant of the solve (below `csp_sum`).
        if method == "size" {
            return None;
        }
        let name = self.csp_member(csp, recv)?;
        let arr = csp.arrays.get(&name)?.clone();
        if method != "sum" || csp.sizes.contains_key(&name) {
            return Some(None);
        }
        let it = match args {
            [] => "item".to_string(),
            [a] => match &Self::unparen(a).kind {
                ExprKind::Ident(h) if h.path.len() == 1 => h.path[0].name.name.clone(),
                _ => return Some(None),
            },
            _ => return Some(None),
        };
        let mut terms = Vec::with_capacity(arr.elems.len());
        for &(i, v) in &arr.elems {
            let t = match filter {
                None => Some(Self::csp_var_ae(csp, v)),
                Some(f) => {
                    let mut e2 = env.clone();
                    e2.it = Some((it.clone(), v, i));
                    self.csp_ae(csp, f, &e2)
                }
            };
            let Some(t) = t else {
                return Some(None);
            };
            terms.push(t);
        }
        let (w, s) = match (filter, terms.first()) {
            (Some(_), Some(t)) => Self::csp_ws(t),
            _ => (arr.width, arr.signed),
        };
        if terms.iter().any(|t| Self::csp_ws(t) != (w, s)) || w > 64 {
            return Some(None);
        }
        Some(Some(Ae::Sum { terms, w, s }))
    }

    pub(super) fn csp_var_ae(csp: &Csp, v: usize) -> Ae {
        Ae::Var(v, csp.vars[v].width, csp.vars[v].signed)
    }

    /// Self-determined (width, signed) of an operand tree.
    pub(super) fn csp_ws(a: &Ae) -> (u32, bool) {
        match a {
            Ae::Var(_, w, s) => (*w, *s),
            Ae::Const(v) => (v.width, v.is_signed),
            Ae::Neg(x) | Ae::Shl(x, _) => Self::csp_ws(x),
            Ae::Quot(l, r, _) => {
                let (w, s) = Self::csp_ws(l);
                (w.max(r.width), s && r.is_signed)
            }
            Ae::Add(l, r) | Ae::Sub(l, r) | Ae::Mul(l, r) => {
                let ((a, b), (c, d)) = (Self::csp_ws(l), Self::csp_ws(r));
                (a.max(c), b && d)
            }
            Ae::Sum { w, s, .. } | Ae::Cast { w, s, .. } => (*w, *s),
        }
    }

    /// Linear form of `a` in the context `(w, s)`, recording the fits that
    /// keep it exact.
    pub(super) fn csp_lin(
        a: &Ae,
        w: u32,
        s: bool,
        fits: &mut Vec<Fit>,
        cx: &mut LinCx,
    ) -> Option<Lin> {
        match a {
            Ae::Var(v, vw, vs) => {
                // an unsigned context zero-extends a signed operand; one that
                // already fills the context is only reinterpreted (§11.8.2)
                if *vs && !s {
                    // §11.8.2: the operand's value in that context is its
                    // own bits read unsigned, `v + 2^vw` when `v` is
                    // negative: a variable of its own (`LinCx::wrap`), so
                    // the relation propagates over it instead of waiting
                    // until `v` is fixed — `rb == w * 8` with an `int rb`
                    // and a `longint unsigned w` bounds `rb` at once (#994).
                    if let Some(u) = cx.wrap(*v, *vw, false) {
                        return Some(Lin::var(u));
                    }
                    fits.push(Fit {
                        lin: Lin::var(*v),
                        lo: 0,
                        hi: i128::MAX,
                        modw: if *vw >= w { *vw } else { 0 },
                    });
                }
                Some(Lin::var(*v))
            }
            Ae::Const(v) => {
                let raw = v.to_u64()?;
                Some(Lin::konst(Self::as_ctx(raw as i128, v.width, s)))
            }
            Ae::Neg(x) => Self::csp_lin(x, w, s, fits, cx)?.scale(-1),
            Ae::Shl(x, k) => {
                if *k >= u64::from(w) {
                    Some(Lin::konst(0))
                } else {
                    Self::csp_lin(x, w, s, fits, cx)?.scale(1i128 << *k)
                }
            }
            Ae::Add(l, r) => {
                Self::csp_lin(l, w, s, fits, cx)?.plus(&Self::csp_lin(r, w, s, fits, cx)?, 1)
            }
            Ae::Sub(l, r) => {
                Self::csp_lin(l, w, s, fits, cx)?.plus(&Self::csp_lin(r, w, s, fits, cx)?, -1)
            }
            Ae::Mul(l, r) => {
                let (x, y) = (
                    Self::csp_lin(l, w, s, fits, cx)?,
                    Self::csp_lin(r, w, s, fits, cx)?,
                );
                if x.is_const() {
                    y.scale(x.k)
                } else if y.is_const() {
                    x.scale(y.k)
                } else if let ([(a, 1)], [(b, 1)], 0, 0) = (&x.t[..], &y.t[..], x.k, y.k) {
                    // Two variables (`cnt * pitch`): their product is a
                    // variable of its own, exact as an integer; computed at
                    // `w` bits it wraps like any other operand, which the
                    // side fits of the relation see.
                    Some(Lin::var(cx.prod(*a, *b)?))
                } else {
                    None
                }
            }
            Ae::Quot(base, divisor, rem) => {
                let d = divisor.to_u64()? as i128;
                // Signed division truncates toward zero, not toward minus infinity.
                if s || divisor.is_signed && divisor.to_i64()? <= 0 {
                    return None;
                }
                let mut acc = Self::csp_lin(base, w, s, fits, cx)?;
                if let Some(v) = cx.carry(&acc, w, s) {
                    acc = Lin::var(v);
                }
                let (lo, hi) = cx.range(&acc)?;
                let (rlo, rhi) = ws_range(w, s);
                if lo < rlo || hi > rhi {
                    return None;
                }
                let (q, r) = cx.quotient(&acc, d)?;
                Some(Lin::var(if *rem { r } else { q }))
            }
            Ae::Sum {
                terms,
                w: sw,
                s: ss,
            } => {
                let mut acc = Lin::konst(0);
                let mark = fits.len();
                for t in terms {
                    acc = acc.plus(&Self::csp_lin(t, *sw, *ss, fits, cx)?, 1)?;
                }
                // §7.12.3: the sum is computed at its own width and wraps
                // there (`a.sum() == 3` over 16-bit elements also holds for
                // terms adding up to 3 + 2^16·k).
                if let Some(v) = cx.carry(&acc, *sw, *ss) {
                    // The carry identity holds modulo 2^sw, so a term fit
                    // that only guards a reinterpretation at sw bits or more
                    // (`int'(unsigned'(item))`) cannot change the sum's
                    // value: drop it, or it would stop the relation on the
                    // sum from propagating until every term is fixed.
                    let kept: Vec<Fit> = fits.drain(mark..).filter(|f| f.modw < *sw).collect();
                    fits.extend(kept);
                    return Self::csp_lin(&Ae::Var(v, *sw, *ss), w, s, fits, cx);
                }
                Self::csp_leaf_fit(&acc, *sw, *ss, *sw, *ss, w, s, fits);
                Some(acc)
            }
            Ae::Cast {
                inner,
                iw,
                is,
                w: cw,
                s: cs,
            } => {
                let l = Self::csp_lin(inner, *iw, *is, fits, cx)?;
                // §6.24.1, §11.8.2: the cast of one variable reads its low
                // `cw` bits, signed only when the context is signed too;
                // read wrapped (see `Ae::Var`) the relation propagates over
                // every value, not just those the cast leaves alone.
                if let ([(v, 1)], 0) = (&l.t[..], l.k)
                    && let Some(u) = cx.wrap(*v, *cw, *cs && s)
                {
                    return Some(Lin::var(u));
                }
                Self::csp_leaf_fit(&l, *iw, *is, *cw, *cs, w, s, fits);
                Some(l)
            }
        }
    }

    /// A sub-result computed at `(iw, is)`, converted to `(cw, cs)` and then
    /// extended into a context of signedness `ctx_s`, keeps its value when it
    /// lies in both ranges (and is non-negative when a signed value meets an
    /// unsigned context, which zero-extends).
    ///
    /// Outside that range the value differs from `l` by a multiple of
    /// `2^cw` (§6.24.1: it is evaluated at `iw >= cw` bits and truncated to
    /// `cw`), which an equality compared at `ctx_w <= cw` bits does not see —
    /// unless the value is extended into a wider context first.
    #[allow(clippy::too_many_arguments)]
    fn csp_leaf_fit(
        l: &Lin,
        iw: u32,
        is: bool,
        cw: u32,
        cs: bool,
        ctx_w: u32,
        ctx_s: bool,
        fits: &mut Vec<Fit>,
    ) {
        let (a, b) = ws_range(iw, is);
        let (c, d) = ws_range(cw, cs);
        let lo = if cs && !ctx_s {
            a.max(c).max(0)
        } else {
            a.max(c)
        };
        fits.push(Fit {
            lin: l.clone(),
            lo,
            hi: b.min(d),
            modw: if cw >= ctx_w { cw } else { 0 },
        });
    }
}
