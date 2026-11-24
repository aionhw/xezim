//! Loop unrolling and reduction loops.

use super::*;

impl<'a> BytecodeCompiler<'a> {
    /// Bounds of a local `for (int i = a; i < b; i++)` counter that the body
    /// cannot modify. Calls may write a ref argument, so only the recognized
    /// expression casts are allowed.
    pub(super) fn proven_loop_index(
        &mut self,
        init: &[ForInit],
        condition: Option<&Expression>,
        step: &[Expression],
        body: &Statement,
    ) -> Option<(RegId, (i64, i64))> {
        if !self.for_body_is_simple(body) {
            return None;
        }
        let [ForInit::VarDecl { name, init, .. }] = init else {
            return None;
        };
        let name = name.name.as_str();
        let &(reg, width) = self.local_var_regs.get(name)?;
        if width != 32
            || !matches!(self.local_kinds.get(&reg), Some(LocalKind::Int { signed: true, .. }))
        {
            return None;
        }
        let ExprKind::Binary { op, left, right } = &condition?.kind else {
            return None;
        };
        let is_counter = |expr: &Expression| {
            matches!(&expr.kind, ExprKind::Ident(h) if h.root.is_none()
                && self.local_var_reg_of(h) == Some((reg, width)))
        };
        if !matches!(op, BinaryOp::Lt | BinaryOp::Leq) || !is_counter(left) {
            return None;
        }
        let [step] = step else { return None };
        let ExprKind::Unary {
            op: UnaryOp::PreIncr | UnaryOp::PostIncr,
            operand,
        } = &step.kind else {
            return None;
        };
        if !is_counter(operand) {
            return None;
        }
        let first = self.fold_const_i64(init)?;
        let last = self.fold_const_i64(right)?.checked_sub(i64::from(*op == BinaryOp::Lt))?;
        if first < 0 || last < first || last >= i64::from(i32::MAX) {
            return None;
        }

        fn may_target(expr: &Expression, name: &str) -> bool {
            match &expr.kind {
                ExprKind::Index { expr, .. }
                | ExprKind::RangeSelect { expr, .. }
                | ExprKind::MemberAccess { expr, .. }
                | ExprKind::Paren(expr) => may_target(expr, name),
                ExprKind::Concatenation(parts) => parts.iter().any(|e| may_target(e, name)),
                _ => BytecodeCompiler::plain_loop_ident(expr) == Some(name),
            }
        }
        let mut preserved = true;
        crate::compiler::elaborate::for_each_stmt_expr(body, &mut |expr| {
            // The walker also supplies assignment destinations as roots.
            preserved &= !may_target(expr, name);
            crate::type_lint::visit_expr(expr, &mut |part| {
                preserved &= match &part.kind {
                    ExprKind::AssignExpr { lvalue, .. } => !may_target(lvalue, name),
                    ExprKind::Unary {
                        op: UnaryOp::PreIncr | UnaryOp::PostIncr | UnaryOp::PreDecr | UnaryOp::PostDecr,
                        operand,
                    } => !may_target(operand, name),
                    ExprKind::Call { .. } => false,
                    ExprKind::SystemCall { name, .. } => matches!(
                        name.as_str(),
                        "$__xz_size_cast" | "$__xz_type_cast" | "$__xz_named_cast" | "$signed" | "$unsigned"
                    ),
                    _ => true,
                };
            });
        });
        preserved.then_some((reg, (first, last)))
    }

    /// Does this statement read or write a register-bank local array? Such a
    /// body needs CONSTANT indexes, i.e. the unroller — a register-backed
    /// runtime loop variable can never address the bank.
    pub(super) fn stmt_touches_reg_bank(&self, st: &Statement) -> bool {
        if self.local_array_regs.is_empty() {
            return false;
        }
        fn expr_hits(e: &Expression, banks: &HashMap<String, LocalArrayBank>) -> bool {
            match &e.kind {
                ExprKind::Ident(h) => h.path.len() == 1 && banks.contains_key(&h.path[0].name.name),
                ExprKind::Index { expr, index } => {
                    expr_hits(expr, banks) || expr_hits(index, banks)
                }
                ExprKind::Paren(i) | ExprKind::Unary { operand: i, .. } => expr_hits(i, banks),
                ExprKind::Binary { left, right, .. } => {
                    expr_hits(left, banks) || expr_hits(right, banks)
                }
                ExprKind::Conditional {
                    condition,
                    then_expr,
                    else_expr,
                } => {
                    expr_hits(condition, banks)
                        || expr_hits(then_expr, banks)
                        || expr_hits(else_expr, banks)
                }
                ExprKind::Concatenation(xs) => xs.iter().any(|x| expr_hits(x, banks)),
                ExprKind::Replication { count, exprs } => {
                    expr_hits(count, banks) || exprs.iter().any(|x| expr_hits(x, banks))
                }
                ExprKind::RangeSelect {
                    expr, left, right, ..
                } => expr_hits(expr, banks) || expr_hits(left, banks) || expr_hits(right, banks),
                ExprKind::Call { args, .. } | ExprKind::SystemCall { args, .. } => {
                    args.iter().any(|a| expr_hits(a, banks))
                }
                _ => false,
            }
        }
        let banks = &self.local_array_regs;
        fn walk(st: &Statement, banks: &HashMap<String, LocalArrayBank>) -> bool {
            match &st.kind {
                StatementKind::BlockingAssign { lvalue, rvalue }
                | StatementKind::NonblockingAssign { lvalue, rvalue, .. } => {
                    expr_hits(lvalue, banks) || expr_hits(rvalue, banks)
                }
                StatementKind::Expr(e) => expr_hits(e, banks),
                StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                    stmts.iter().any(|s| walk(s, banks))
                }
                StatementKind::If {
                    condition,
                    then_stmt,
                    else_stmt,
                    ..
                } => {
                    expr_hits(condition, banks)
                        || walk(then_stmt, banks)
                        || else_stmt.as_ref().is_some_and(|e| walk(e, banks))
                }
                StatementKind::Case { expr, items, .. } => {
                    expr_hits(expr, banks)
                        || items.iter().any(|it| {
                            it.patterns.iter().any(|p| expr_hits(p, banks)) || walk(&it.stmt, banks)
                        })
                }
                StatementKind::For {
                    init,
                    condition,
                    step,
                    body,
                } => {
                    init.iter().any(|fi| match fi {
                        ForInit::VarDecl { init, .. } => expr_hits(init, banks),
                        ForInit::Assign { lvalue, rvalue } => {
                            expr_hits(lvalue, banks) || expr_hits(rvalue, banks)
                        }
                    }) || condition.as_ref().is_some_and(|c| expr_hits(c, banks))
                        || step.iter().any(|e| expr_hits(e, banks))
                        || walk(body, banks)
                }
                _ => false,
            }
        }
        walk(st, banks)
    }

    /// Compile-time unroll of `for (int i = C; cond(i); i += C2)` with a
    /// small constant trip count. The loop variable becomes a per-iteration
    /// COMPILE-TIME constant, so array indexes fold (`state[col*4]`), local
    /// register-bank arrays become addressable, and call statements inline
    /// per iteration. All-or-nothing: any statement that fails rolls the
    /// whole unroll back and the loop takes the pre-existing paths.
    pub(super) fn try_unroll_for(
        &mut self,
        name: &crate::ast::Identifier,
        data_type: &crate::ast::types::DataType,
        init: &Expression,
        condition: &Option<Expression>,
        step: &[Expression],
        body: &Statement,
    ) -> bool {
        const MAX_TRIPS: usize = 64;
        let Some(cond) = condition else { return false };
        if step.len() != 1 {
            return false;
        }
        // A body that can SUSPEND must go one iteration at a time through the
        // scheduler — an unrolled copy would run synchronously through the
        // wait. And break/continue need per-iteration jump targets the unroll
        // does not lay down; leave those loops to the AST path.
        if Self::stmt_is_blocking(body) || Self::stmt_has_break_or_continue(body) {
            return false;
        }
        // Step: i++/++i/i--/--i, or (i = i + C) / (i += C) via AssignExpr.
        let vname = name.name.clone();
        let step_delta: i64 = match &step[0].kind {
            ExprKind::Unary { op, operand } => {
                let ExprKind::Ident(h) = &operand.kind else {
                    return false;
                };
                if Self::hier_raw_name(h) != vname {
                    return false;
                }
                match op {
                    UnaryOp::PostIncr | UnaryOp::PreIncr => 1,
                    UnaryOp::PostDecr | UnaryOp::PreDecr => -1,
                    _ => return false,
                }
            }
            ExprKind::AssignExpr { lvalue, rvalue } => {
                let ExprKind::Ident(h) = &lvalue.kind else {
                    return false;
                };
                if Self::hier_raw_name(h) != vname {
                    return false;
                }
                let ExprKind::Binary { op, left, right } = &rvalue.kind else {
                    return false;
                };
                let ExprKind::Ident(lh) = &left.kind else {
                    return false;
                };
                if Self::hier_raw_name(lh) != vname {
                    return false;
                }
                let Some(c) = self.fold_const(right).and_then(|v| v.to_i64()) else {
                    return false;
                };
                match op {
                    BinaryOp::Add => c,
                    BinaryOp::Sub => c.wrapping_neg(),
                    _ => return false,
                }
            }
            _ => return false,
        };
        if step_delta == 0 {
            return false;
        }
        // The loop variable is bound with its declared width and signedness
        // (§6.11, §6.8): the condition compares it as signed when both
        // operands are signed (§11.8.1), so `for (int i = 1; i > -1; i--)`
        // needs `i` signed, and the step wraps at the declared width.
        let kind = self.local_kind_of(data_type, None);
        let Some(LocalKind::Int { width, signed, .. }) = kind else {
            return false;
        };
        if width == 0 || width > 64 {
            return false;
        }
        let fit = |x: i64| -> i64 {
            if width == 64 {
                return x;
            }
            let mask = (1u64 << width) - 1;
            let u = (x as u64) & mask;
            if signed && (u >> (width - 1)) & 1 == 1 {
                (u | !mask) as i64
            } else {
                u as i64
            }
        };
        let Some(mut cur) = self
            .fold_const(init)
            .and_then(|v| if v.has_xz() { None } else { v.to_i64() })
            .map(fit)
        else {
            return false;
        };

        let start = self.insns.len();
        let start_reg = self.next_reg;
        let outer_const = self.local_const_vars.remove(&vname);
        // How a fallback in the body hands the constant to the interpreter.
        let outer_kind = self.local_const_kinds.insert(
            vname.clone(),
            LocalKind::Int {
                width,
                signed,
                two_state: matches!(
                    kind,
                    Some(LocalKind::Int {
                        two_state: true,
                        ..
                    })
                ),
            },
        );
        let mut ok = true;
        let mut trips = 0usize;
        loop {
            let mut bound = Value::from_u64(cur as u64, width);
            bound.is_signed = signed;
            self.local_const_vars.insert(vname.clone(), bound);
            let c = match self.fold_const(cond) {
                Some(v) => v.is_true(),
                None => {
                    ok = false;
                    break;
                }
            };
            if !c {
                break;
            }
            trips += 1;
            if trips > MAX_TRIPS {
                ok = false;
                break;
            }
            // A statement deferring to the AST interpreter inside the body
            // carries the const-bound loop variable in a register
            // (`fb_carry`), and may not write it; one naming a register bank
            // cannot fall back at all and rolls the whole unroll back.
            let body_ok = self.compile_stmt(body);
            if !body_ok {
                ok = false;
                break;
            }
            cur = fit(cur.wrapping_add(step_delta));
        }
        self.local_const_vars.remove(&vname);
        if let Some(v) = outer_const {
            self.local_const_vars.insert(vname.clone(), v);
        }
        match outer_kind {
            Some(k) => {
                self.local_const_kinds.insert(vname.clone(), k);
            }
            None => {
                self.local_const_kinds.remove(&vname);
            }
        }
        if !ok {
            self.truncate_insns(start);
            self.next_reg = start_reg;
            return false;
        }
        true
    }

    /// `for (k = K0; cond(k); k = k ± d) acc = acc | e(k);` (or `&`): the
    /// accumulate-over-a-range loop RTL writes as an OR (AND) tree over an
    /// unpacked array, `acc = acc | row_out[k]`. With a constant trip count
    /// it compiles straight-line: `k` a constant per trip (`row_out[k]` a
    /// plain load of the element), `acc` carried in one register, then one
    /// store of `acc` and the stores that leave `k` where the loop leaves
    /// it. The rolled loop paid two signal stores (each noting the write for
    /// propagation), a compare, a branch and an element-index check per trip
    /// (about 100 host instructions; the DRAM model's 128-row OR spent
    /// 12.7k per evaluation). `e` is compiled once and replayed per trip, so
    /// the compile cost stays a few instructions per trip.
    ///
    /// Exact. The loop is atomic (no timing control, call or fallback) and
    /// the emitted `e` reads neither `acc` nor `k`'s storage, so the only
    /// trace of the intermediate stores is their change marks. OR (AND)
    /// only sets (clears) bits along the way, so the union of those marks
    /// is `start ^ final`, exactly what the single store marks, and a
    /// two-state `acc`'s x-to-0 conversion per store commutes with the
    /// chain. `k` walks a constant sequence; its union of changes `u` is
    /// replayed by storing `K0`, `K0 ^ u`, then the exit value.
    pub(super) fn try_reduce_loop(
        &mut self,
        init: &[ForInit],
        condition: Option<&Expression>,
        step: &[Expression],
        body: &Statement,
    ) -> bool {
        const MAX_TRIPS: usize = 1024;
        let (Some(cond), [step]) = (condition, step) else {
            return false;
        };
        let mut stmt = body;
        while let StatementKind::SeqBlock { stmts, .. } = &stmt.kind {
            let [one] = stmts.as_slice() else {
                return false;
            };
            stmt = one;
        }
        let StatementKind::BlockingAssign { lvalue, rvalue } = &stmt.kind else {
            return false;
        };
        if Self::is_intra_timing_marker(rvalue) {
            return false;
        }
        let ExprKind::Ident(acc_h) = &lvalue.kind else {
            return false;
        };
        if acc_h.path.iter().any(|s| !s.selects.is_empty()) {
            return false;
        }
        let Some(acc_id) = self.lookup_signal_id(acc_h) else {
            return false;
        };
        fn strip(e: &Expression) -> &Expression {
            let mut e = e;
            while let ExprKind::Paren(i) = &e.kind {
                e = i;
            }
            e
        }
        fn leaf(h: &HierarchicalIdentifier) -> &str {
            let n = h.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
            n.rsplit('.').next().unwrap_or(n)
        }
        let ExprKind::Binary { op, left, right } = &strip(rvalue).kind else {
            return false;
        };
        if !matches!(op, BinaryOp::BitOr | BinaryOp::BitAnd) {
            return false;
        }
        let op = op.clone();
        let is_acc = |this: &Self, e: &Expression| {
            matches!(&strip(e).kind, ExprKind::Ident(h)
                if h.path.iter().all(|s| s.selects.is_empty())
                    && this.lookup_signal_id(h) == Some(acc_id))
        };
        let (acc_expr, e) = if is_acc(self, left) {
            (&**left, &**right)
        } else if is_acc(self, right) {
            (&**right, &**left)
        } else {
            return false;
        };
        // The loop variable: a signal (`integer k` at module scope) or a
        // `for (int k = ...)` local.
        let [fi] = init else {
            return false;
        };
        let (k_leaf, k_sig, k_init, k_w, k_signed): (
            String,
            Option<(usize, &Expression)>,
            _,
            u32,
            bool,
        ) = match fi {
            ForInit::Assign {
                lvalue: kl,
                rvalue: kr,
            } => {
                let ExprKind::Ident(kh) = &kl.kind else {
                    return false;
                };
                if kh.path.iter().any(|s| !s.selects.is_empty()) {
                    return false;
                }
                let Some(kid) = self.lookup_signal_id(kh) else {
                    return false;
                };
                let w = self.infer_lhs_width(kl);
                (
                    leaf(kh).to_string(),
                    Some((kid, kl)),
                    kr,
                    w,
                    self.signal_signed[kid],
                )
            }
            ForInit::VarDecl {
                data_type,
                name,
                init,
            } => {
                use crate::ast::types::{DataType as FDt, IntegerAtomType as FIat, Signing as FSg};
                let signed = match data_type {
                    FDt::IntegerAtom { kind, signing, .. } => {
                        !matches!(signing, Some(FSg::Unsigned)) && !matches!(kind, FIat::Time)
                    }
                    FDt::IntegerVector { signing, .. } => matches!(signing, Some(FSg::Signed)),
                    _ => return false,
                };
                (
                    name.name.clone(),
                    None,
                    init,
                    self.decl_width(data_type),
                    signed,
                )
            }
        };
        if k_leaf.is_empty() || k_w == 0 || k_w > 32 || k_sig.is_some_and(|(kid, _)| kid == acc_id)
        {
            return false;
        }
        let acc_leaf = leaf(acc_h).to_string();
        // Every spelling of `k` the loop uses, to bind as a constant; `e`
        // must be side-effect free and must not name `acc` at all.
        fn scan(
            e: &Expression,
            k_leaf: &str,
            acc_leaf: Option<&str>,
            names: &mut Vec<String>,
        ) -> bool {
            match &e.kind {
                ExprKind::Number(_) => true,
                ExprKind::Ident(h) => {
                    let lf = leaf(h);
                    if acc_leaf == Some(lf) {
                        return false;
                    }
                    if lf == k_leaf {
                        if h.root.is_some() || h.path.len() != 1 || !h.path[0].selects.is_empty() {
                            return false;
                        }
                        let n = &h.path[0].name.name;
                        if !names.contains(n) {
                            names.push(n.clone());
                        }
                    }
                    h.path
                        .iter()
                        .all(|s| s.selects.iter().all(|x| scan(x, k_leaf, acc_leaf, names)))
                }
                ExprKind::Paren(i) => scan(i, k_leaf, acc_leaf, names),
                ExprKind::Index { expr, index } => {
                    scan(expr, k_leaf, acc_leaf, names) && scan(index, k_leaf, acc_leaf, names)
                }
                ExprKind::RangeSelect {
                    expr, left, right, ..
                } => {
                    scan(expr, k_leaf, acc_leaf, names)
                        && scan(left, k_leaf, acc_leaf, names)
                        && scan(right, k_leaf, acc_leaf, names)
                }
                ExprKind::Unary { op, operand } => {
                    !matches!(
                        op,
                        UnaryOp::PreIncr | UnaryOp::PostIncr | UnaryOp::PreDecr | UnaryOp::PostDecr
                    ) && scan(operand, k_leaf, acc_leaf, names)
                }
                ExprKind::Binary { left, right, .. } => {
                    scan(left, k_leaf, acc_leaf, names) && scan(right, k_leaf, acc_leaf, names)
                }
                ExprKind::Conditional {
                    condition,
                    then_expr,
                    else_expr,
                } => {
                    scan(condition, k_leaf, acc_leaf, names)
                        && scan(then_expr, k_leaf, acc_leaf, names)
                        && scan(else_expr, k_leaf, acc_leaf, names)
                }
                ExprKind::Concatenation(v) => v.iter().all(|x| scan(x, k_leaf, acc_leaf, names)),
                ExprKind::Replication { count, exprs } => {
                    scan(count, k_leaf, acc_leaf, names)
                        && exprs.iter().all(|x| scan(x, k_leaf, acc_leaf, names))
                }
                _ => false,
            }
        }
        let mut names: Vec<String> = Vec::new();
        if !scan(e, &k_leaf, Some(&acc_leaf), &mut names) || !scan(cond, &k_leaf, None, &mut names)
        {
            return false;
        }
        // Step: `k++` / `k--` / `k = k ± C` (the parser's `k += C`).
        let delta: i64 = match &step.kind {
            ExprKind::Unary { op, operand } => {
                let ExprKind::Ident(h) = &operand.kind else {
                    return false;
                };
                if leaf(h) != k_leaf {
                    return false;
                }
                match op {
                    UnaryOp::PostIncr | UnaryOp::PreIncr => 1,
                    UnaryOp::PostDecr | UnaryOp::PreDecr => -1,
                    _ => return false,
                }
            }
            ExprKind::AssignExpr { lvalue, rvalue } => {
                let (ExprKind::Ident(lh), ExprKind::Binary { op, left, right }) =
                    (&lvalue.kind, &rvalue.kind)
                else {
                    return false;
                };
                let ExprKind::Ident(rh) = &left.kind else {
                    return false;
                };
                if leaf(lh) != k_leaf || leaf(rh) != k_leaf {
                    return false;
                }
                let Some(c) = self.fold_const(right).and_then(|v| v.to_u64()) else {
                    return false;
                };
                if c == 0 || c > u32::MAX as u64 {
                    return false;
                }
                match op {
                    BinaryOp::Add => c as i64,
                    BinaryOp::Sub => -(c as i64),
                    _ => return false,
                }
            }
            _ => return false,
        };
        // A value of `k` as its declared type holds it.
        let kmask: u64 = (1u64 << k_w) - 1;
        let norm = |v: i64| -> i64 {
            let u = (v as u64) & kmask;
            if k_signed && (u >> (k_w - 1)) & 1 == 1 {
                (u | !kmask) as i64
            } else {
                u as i64
            }
        };
        let kval = |v: i64| -> Value {
            let mut x = Value::from_u64((v as u64) & kmask, k_w);
            x.is_signed = k_signed;
            x
        };
        let Some(k0) = self
            .fold_const(k_init)
            .and_then(|v| if v.has_xz() { None } else { v.to_u64() })
        else {
            return false;
        };
        let saved: Vec<(String, Option<Value>)> = names
            .iter()
            .map(|n| (n.clone(), self.local_const_vars.get(n).cloned()))
            .collect();
        let restore = |this: &mut Self| {
            for (n, v) in &saved {
                match v {
                    Some(v) => this.local_const_vars.insert(n.clone(), v.clone()),
                    None => this.local_const_vars.remove(n),
                };
            }
        };
        let bind = |this: &mut Self, v: i64| {
            for n in &names {
                this.local_const_vars.insert(n.clone(), kval(v));
            }
        };
        // Trip values, folded exactly as the loop would test them.
        let mut vals: Vec<i64> = Vec::new();
        let mut cur = norm(k0 as i64);
        loop {
            bind(self, cur);
            let c = match self.fold_const(cond) {
                Some(v) if !v.has_xz() => v.is_true(),
                _ => {
                    restore(self);
                    return false;
                }
            };
            if !c {
                break;
            }
            vals.push(cur);
            if vals.len() > MAX_TRIPS {
                restore(self);
                return false;
            }
            cur = norm(cur.wrapping_add(delta));
        }
        restore(self);
        let start = self.insns.len();
        let start_reg = self.next_reg;
        let ok = self.emit_reduce_loop(
            lvalue, acc_expr, e, &op, acc_id, k_sig, k_init, k_w, &vals, cur, &names, &kval,
        );
        if !ok {
            self.truncate_insns(start);
            self.next_reg = start_reg;
        }
        ok
    }

    /// Emission half of `try_reduce_loop` (false: roll back).
    #[allow(clippy::too_many_arguments)]
    fn emit_reduce_loop(
        &mut self,
        lvalue: &Expression,
        acc_expr: &Expression,
        e: &Expression,
        op: &BinaryOp,
        acc_id: usize,
        k_sig: Option<(usize, &Expression)>,
        k_init: &Expression,
        k_w: u32,
        vals: &[i64],
        k_end: i64,
        names: &[String],
        kval: &dyn Fn(i64) -> Value,
    ) -> bool {
        // `k = K0`, compiled exactly as the loop's init.
        if let Some((_, kl)) = k_sig {
            let w = self.infer_lhs_width(kl);
            let Some(r) = self.compile_expr(k_init, w) else {
                return false;
            };
            if w > 0 {
                self.emit(Insn::Resize(r, w));
            }
            if !self.compile_blocking_target(kl, r, w) {
                return false;
            }
        }
        if !vals.is_empty() {
            // Per trip exactly the body's `acc OP e` (the Binary arm: both
            // operands scrubbed / resized to the statement width), with the
            // left operand the carried register instead of a reload. The
            // carried register's scrub and resize are idempotent after the
            // first trip (its width and signedness no longer change), so they
            // are emitted once.
            let width = self.infer_lhs_width(lvalue);
            if width == 0 {
                return false;
            }
            let ls = self.expr_signedness(acc_expr);
            let rs = self.expr_signedness(e);
            let unsigned = ls == Some(false) || rs == Some(false);
            let scrub_acc = unsigned && !self.operand_scrub_is_noop(acc_expr);
            let scrub_e = unsigned && !self.operand_scrub_is_noop(e);
            let Some(a) = self.compile_expr(acc_expr, width) else {
                return false;
            };
            if scrub_acc {
                self.emit(Insn::ClearSigned(a));
            }
            self.emit(Insn::Resize(a, width));
            // `e` is compiled ONCE, with the loop variable bound to a
            // register; each trip replays it with that register's reads
            // turned into the trip's constant (what compiling `e` with the
            // variable bound to the constant emits: one `LoadConst`), and an
            // element read through such a constant into the element's load.
            // (An outer unroll's constant binding of the same name would
            // shadow the register: lift it for the template.)
            let kreg = self.alloc_reg();
            let saved: Vec<(String, Option<(RegId, u32)>, Option<Value>)> = names
                .iter()
                .map(|n| {
                    (
                        n.clone(),
                        self.local_var_regs.insert(n.clone(), (kreg, k_w)),
                        self.local_const_vars.remove(n),
                    )
                })
                .collect();
            let t_start = self.insns.len();
            let r = self.compile_expr(e, width);
            for (n, v, c) in saved {
                match v {
                    Some(v) => self.local_var_regs.insert(n.clone(), v),
                    None => self.local_var_regs.remove(&n),
                };
                if let Some(c) = c {
                    self.local_const_vars.insert(n, c);
                }
            }
            let Some(r) = r else {
                return false;
            };
            let template: Vec<Insn> = self.insns.drain(t_start..).collect();
            let k_id = k_sig.map(|(kid, _)| kid);
            if !Self::reduce_operand_ok(&template, k_id, acc_id)
                || template.iter().any(|i| {
                    !matches!(i, Insn::Move(_, s) if *s == kreg) && Self::insn_reads_reg(i, kreg)
                })
                || template
                    .iter()
                    .any(|i| Self::dest_reg(i).or_else(|| Self::in_place_reg(i)) == Some(kreg))
            {
                return false;
            }
            // `Move(t, k); LoadArrayElem(d, arr, t)` with `t` read nowhere
            // else: per trip one load of the element signal.
            let mut elem_of: Vec<Option<(usize, i64, i64)>> = vec![None; template.len()];
            let mut skip = vec![false; template.len()];
            for (i, ins) in template.iter().enumerate() {
                let Insn::LoadArrayElem(_, arr, t) = ins else {
                    continue;
                };
                let ArrayOperand::Dense {
                    first_id, lo, hi, ..
                } = &**arr
                else {
                    continue;
                };
                let defs: Vec<usize> = (0..i)
                    .filter(|&j| {
                        let x = &template[j];
                        Self::dest_reg(x).or_else(|| Self::in_place_reg(x)) == Some(*t)
                    })
                    .collect();
                let [j] = defs.as_slice() else {
                    continue;
                };
                let reads = template
                    .iter()
                    .enumerate()
                    .filter(|&(m, x)| m != i && Self::insn_reads_reg(x, *t))
                    .count();
                if matches!(template[*j], Insn::Move(_, s) if s == kreg) && reads == 0 && hi >= lo {
                    elem_of[i] = Some((*first_id, *lo, *hi));
                    skip[*j] = true;
                }
            }
            for &v in vals {
                // The element index the trip's constant names, exactly as
                // `LoadArrayElem` resolves it (the register's bits as an
                // unsigned number).
                let kv = kval(v);
                let idx = kv.to_u64().map(|x| x as i64);
                let trip_start = self.insns.len();
                // The trip is one element load of the statement's width and
                // unsigned: the operand scrub and resize would be no-ops.
                let mut plain = false;
                for (n, i) in template.iter().enumerate() {
                    match (i, elem_of[n]) {
                        (Insn::LoadArrayElem(d, ..), Some((first, lo, hi)))
                            if idx.is_some_and(|x| x >= lo && x <= hi)
                                && first + ((hi - lo) as usize) < self.signal_widths.len() =>
                        {
                            let eid = first + (idx.unwrap() - lo) as usize;
                            self.emit(Insn::LoadSignal(*d, eid as SigId));
                            plain = *d == r
                                && self.signal_widths[eid] == width
                                && !self.signal_signed[eid];
                        }
                        (Insn::LoadArrayElem(_, _, t), Some(_)) => {
                            // Out of range: the generic read (x), as compiled.
                            self.emit(Insn::LoadConst(*t, Box::new(kv.clone())));
                            self.emit(i.clone());
                        }
                        (Insn::Move(d, s), _) if *s == kreg => {
                            if !skip[n] {
                                self.emit(Insn::LoadConst(*d, Box::new(kv.clone())));
                            }
                        }
                        _ => self.emit(i.clone()),
                    }
                }
                if !(plain && self.insns.len() == trip_start + 1) {
                    if scrub_e {
                        self.emit(Insn::ClearSigned(r));
                    }
                    self.emit(Insn::Resize(r, width));
                }
                self.emit(match op {
                    BinaryOp::BitOr => Insn::BitOr(a, a, r),
                    _ => Insn::BitAnd(a, a, r),
                });
            }
            self.emit(Insn::Resize(a, width));
            if self.register_overflow || !self.compile_blocking_target(lvalue, a, width) {
                return false;
            }
        }
        // Leave `k` at the exit value, marking the union of its changes.
        if let (Some((_, kl)), false) = (k_sig, vals.is_empty()) {
            let kmask: u64 = (1u64 << k_w) - 1;
            let mut u = 0u64;
            for (i, &v) in vals.iter().enumerate() {
                let next = vals.get(i + 1).copied().unwrap_or(k_end);
                u |= ((v ^ next) as u64) & kmask;
            }
            let k0 = (vals[0] as u64) & kmask;
            let kend = (k_end as u64) & kmask;
            let mut seq: Vec<i64> = Vec::new();
            if u != k0 ^ kend {
                seq.push((k0 ^ u) as i64);
            }
            seq.push(k_end);
            let w = self.infer_lhs_width(kl);
            for v in seq {
                let r = self.alloc_reg();
                self.emit(Insn::LoadConst(r, Box::new(kval(v))));
                if w > 0 {
                    self.emit(Insn::Resize(r, w));
                }
                if !self.compile_blocking_target(kl, r, w) {
                    return false;
                }
            }
        }
        !self.register_overflow
    }

    /// `try_reduce_loop`'s check on one trip's compiled operand: pure
    /// straight-line arithmetic that reads neither `acc` nor the loop
    /// variable's storage.
    fn reduce_operand_ok(insns: &[Insn], k_id: Option<usize>, acc_id: usize) -> bool {
        let hit = |s: SigId| s as usize == acc_id || Some(s as usize) == k_id;
        insns.iter().all(|i| match i {
            Insn::LoadSignal(_, s)
            | Insn::LoadSignalSigned(_, s)
            | Insn::LoadSignalRange(_, s, ..)
            | Insn::LoadSignalRangeDyn(_, s, ..)
            | Insn::LoadSignalBit(_, s, _) => !hit(*s),
            Insn::LoadConst(..)
            | Insn::LoadArrayElem(..)
            | Insn::Resize(..)
            | Insn::MoveResize(..)
            | Insn::Move(..)
            | Insn::SetSigned(..)
            | Insn::ClearSigned(..)
            | Insn::DropXz(..)
            | Insn::RealToInt(..)
            | Insn::Add(..)
            | Insn::Sub(..)
            | Insn::Mul(..)
            | Insn::BitAnd(..)
            | Insn::BitOr(..)
            | Insn::BitXor(..)
            | Insn::BitXnor(..)
            | Insn::LogAnd(..)
            | Insn::LogOr(..)
            | Insn::Eq(..)
            | Insn::Neq(..)
            | Insn::CaseEq(..)
            | Insn::Lt(..)
            | Insn::Leq(..)
            | Insn::Gt(..)
            | Insn::Geq(..)
            | Insn::Shl(..)
            | Insn::Shr(..)
            | Insn::AShr(..)
            | Insn::BitNot(..)
            | Insn::LogNot(..)
            | Insn::Negate(..)
            | Insn::ReduceAnd(..)
            | Insn::ReduceOr(..)
            | Insn::ReduceXor(..)
            | Insn::BitSelect(..)
            | Insn::BitSelectConst(..)
            | Insn::RangeSelect(..)
            | Insn::RangeSelectW(..)
            | Insn::RangeSelectConst(..)
            | Insn::Concat(..)
            | Insn::Replicate(..)
            | Insn::Select(..)
            | Insn::BinOpConst(..) => true,
            _ => false,
        })
    }
}
