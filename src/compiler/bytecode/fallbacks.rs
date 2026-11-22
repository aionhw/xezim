//! Interpreter fallbacks of expressions and statements.

use super::*;

impl<'a> BytecodeCompiler<'a> {
    /// Can `stmt` print a scope name anywhere (`stmt_names_scope`, ignoring
    /// the `disable` that makes that one give up)?
    pub(super) fn stmt_prints_scope(stmt: &Statement) -> bool {
        let expr = Self::expr_names_scope;
        match &stmt.kind {
            StatementKind::Expr(e) => expr(e),
            StatementKind::BlockingAssign { rvalue, .. }
            | StatementKind::NonblockingAssign { rvalue, .. } => expr(rvalue),
            StatementKind::Assertion(_) => true,
            StatementKind::If {
                condition,
                then_stmt,
                else_stmt,
                ..
            } => {
                expr(condition)
                    || Self::stmt_prints_scope(then_stmt)
                    || else_stmt.as_deref().is_some_and(Self::stmt_prints_scope)
            }
            StatementKind::Case { items, .. } => {
                items.iter().any(|it| Self::stmt_prints_scope(&it.stmt))
            }
            StatementKind::SeqBlock { stmts, .. } => stmts.iter().any(Self::stmt_prints_scope),
            StatementKind::For { body, .. }
            | StatementKind::Foreach { body, .. }
            | StatementKind::While { body, .. }
            | StatementKind::DoWhile { body, .. }
            | StatementKind::Repeat { body, .. }
            | StatementKind::Forever { body }
            | StatementKind::TimingControl { stmt: body, .. }
            | StatementKind::Wait { stmt: body, .. } => Self::stmt_prints_scope(body),
            _ => false,
        }
    }

    /// Expression-level escape hatch (see Insn::EvalExprFallback). Returns
    /// None when forbidden: no ast-fallback, or it names a register-backed
    /// local the interpreter cannot be handed (see `fb_carry`).
    pub(super) fn emit_expr_fallback(
        &mut self,
        e: &Expression,
        ctx_width: u32,
        reason: &'static str,
    ) -> Option<RegId> {
        // A math expression in a stepped process can use the existing
        // frame-aware evaluator, including formals of an inlined function.
        let stepped_math = self.allow_waits && matches!(&e.kind,
            ExprKind::SystemCall { name, .. } if stepped_math_function(name));
        if !stepped_math && (!self.allow_ast_fallback || !self.allow_expr_fallback) {
            return None;
        }
        // Sampled-value functions ($past/$rose/...) take their clock from the
        // ENCLOSING block's inferred clocking — an isolated expression eval
        // has no block context, so the whole statement must fall back.
        if Self::expr_has_sampled_value_call(e) {
            return None;
        }
        let locals = if self.fb_live() {
            let scan = self.fb_scan_with(|s| s.expr(e));
            if !self.inlining_stack.is_empty() && Self::expr_names_scope(e) {
                return None;
            }
            match self.fb_carry(&scan) {
                Ok(l) => l,
                Err(why) => {
                    self.trace_fallback_site(why, e.span, "expr-refused");
                    return None;
                }
            }
        } else {
            FbLocals::default()
        };
        self.trace_fallback_site_carrying(reason, e.span, "expr", &locals);
        let r = self.alloc_reg();
        self.emit(Insn::EvalExprFallback(
            Box::new((Arc::new(e.clone()), Arc::from(reason), locals)),
            r,
            ctx_width,
        ));
        Some(r)
    }

    /// `XEZIM_FALLBACK_SITES=1`: report every construct the compiler hands
    /// to the AST interpreter, as `reason`, source byte span and enclosing
    /// scope. A fallback inside a loop costs microseconds per execution and
    /// forces the whole loop onto the interpreter, so on a slow design this
    /// names the statements worth compiling — the `[PROF] fallback_reason`
    /// counters say how much they cost, this says where they are.
    pub(super) fn trace_fallback_site(&mut self, reason: &str, span: crate::ast::Span, what: &str) {
        self.trace_fallback_site_carrying(reason, span, what, &[]);
    }

    /// `trace_fallback_site`, naming the register-backed locals the fallback
    /// carries (`carries=a,b`) when there are any.
    pub(super) fn trace_fallback_site_carrying(
        &mut self,
        reason: &str,
        span: crate::ast::Span,
        what: &str,
        locals: &[FbLocal],
    ) {
        use std::sync::OnceLock;
        static ON: OnceLock<bool> = OnceLock::new();
        let on = *ON.get_or_init(|| {
            std::env::var("XEZIM_FALLBACK_SITES")
                .map(|v| v != "0" && !v.is_empty())
                .unwrap_or(false)
        });
        if !on {
            return;
        }
        let carries = if locals.is_empty() {
            String::new()
        } else {
            let names: Vec<&str> = locals.iter().map(|l| l.name.as_ref()).collect();
            format!(" carries={}", names.join(","))
        };
        eprintln!(
            "[FALLBACK] {} reason={} bytes={}..{} scope={}{}",
            what,
            reason,
            span.start,
            span.end,
            self.scope_hint.as_deref().unwrap_or("-"),
            carries
        );
    }

    pub(super) fn emit_fallback(&mut self, stmt: &Statement) -> bool {
        if !self.allow_ast_fallback {
            return false;
        }
        let locals = if self.fb_live() {
            match self.fb_stmt_locals(stmt) {
                Ok(l) => l,
                Err(why) => {
                    // The interpreter cannot be handed the register-backed
                    // locals this statement needs; fail it so the enclosing
                    // unit (loop, block, inlined call) rolls back to one
                    // interpreted piece. Reported because one statement can
                    // take a whole block with it.
                    self.trace_fallback_site(why, stmt.span, "block-bail(local-regs)");
                    return false;
                }
            }
        } else {
            FbLocals::default()
        };
        let reason = self
            .bail_reason
            .unwrap_or_else(|| Self::stmt_kind_label(stmt));
        self.trace_fallback_site_carrying(reason, stmt.span, "stmt", &locals);
        self.push_stmt_fallback(stmt, reason, locals);
        true
    }

    /// Emit a `StmtFallback` for `stmt`. §21.2.1.7: the interpreter pushes a
    /// named block onto the `%m` chain only while it runs the block, so a
    /// scope-printing statement lifted out of one is re-wrapped in its names
    /// (inside an inlined body the first name is the task's own).
    pub(super) fn push_stmt_fallback(
        &mut self,
        stmt: &Statement,
        reason: &'static str,
        locals: FbLocals,
    ) {
        let mut body = stmt.clone();
        if !self.m_labels.is_empty() && Self::stmt_names_scope(stmt) == Some(true) {
            for n in self.m_labels.iter().rev() {
                body = Statement::new(
                    StatementKind::SeqBlock {
                        name: Some(n.clone()),
                        stmts: vec![body].into(),
                    },
                    stmt.span,
                );
            }
        }
        self.emit(Insn::StmtFallback(Box::new((
            Arc::new(body),
            Arc::from(reason),
            locals,
        ))));
    }

    /// Can evaluating `e` print a scope name (`%m` in a format string, a
    /// severity task)?
    fn expr_names_scope(e: &Expression) -> bool {
        match &e.kind {
            ExprKind::StringLiteral(s) => s.contains("%m") || s.contains("%M"),
            ExprKind::SystemCall { name, args } => {
                matches!(name.as_str(), "$info" | "$warning" | "$error" | "$fatal")
                    || args.iter().any(Self::expr_names_scope)
            }
            ExprKind::Call { args, .. } => args.iter().any(Self::expr_names_scope),
            ExprKind::Unary { operand, .. } => Self::expr_names_scope(operand),
            ExprKind::Binary { left, right, .. } => {
                Self::expr_names_scope(left) || Self::expr_names_scope(right)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::expr_names_scope(condition)
                    || Self::expr_names_scope(then_expr)
                    || Self::expr_names_scope(else_expr)
            }
            ExprKind::Concatenation(parts) => parts.iter().any(Self::expr_names_scope),
            ExprKind::Paren(inner) => Self::expr_names_scope(inner),
            _ => false,
        }
    }

    /// Can `stmt` print its own scope — a `%m`, a severity task's scope line
    /// or an assertion report? `None` when it contains a `disable`, which a
    /// synthetic named block around it could intercept.
    pub(super) fn stmt_names_scope(stmt: &Statement) -> Option<bool> {
        let expr = Self::expr_names_scope;
        let any = |stmts: &mut dyn Iterator<Item = &Statement>| -> Option<bool> {
            let mut found = false;
            for st in stmts {
                found |= BytecodeCompiler::stmt_names_scope(st)?;
            }
            Some(found)
        };
        match &stmt.kind {
            StatementKind::Disable(_) => None,
            StatementKind::Expr(e) => Some(expr(e)),
            StatementKind::BlockingAssign { rvalue, .. }
            | StatementKind::NonblockingAssign { rvalue, .. } => Some(expr(rvalue)),
            StatementKind::Assertion(_) => Some(true),
            StatementKind::If {
                condition,
                then_stmt,
                else_stmt,
                ..
            } => Some(
                expr(condition)
                    | any(&mut std::iter::once(&**then_stmt).chain(else_stmt.as_deref()))?,
            ),
            StatementKind::Case { items, .. } => any(&mut items.iter().map(|it| &it.stmt)),
            StatementKind::SeqBlock { stmts, .. } => any(&mut stmts.iter()),
            StatementKind::For { body, .. }
            | StatementKind::Foreach { body, .. }
            | StatementKind::While { body, .. }
            | StatementKind::DoWhile { body, .. }
            | StatementKind::Repeat { body, .. }
            | StatementKind::Forever { body }
            | StatementKind::TimingControl { stmt: body, .. }
            | StatementKind::Wait { stmt: body, .. } => BytecodeCompiler::stmt_names_scope(body),
            _ => Some(false),
        }
    }

    /// True when `rvalue` is one of the pre-parse intra-assignment timing
    /// markers (see `crate::intra_delay`): the assignment carries a delay,
    /// event, or cycle control that only the interpreter's assignment arms
    /// honour.
    pub(super) fn is_intra_timing_marker(rvalue: &Expression) -> bool {
        matches!(
            &rvalue.kind,
            ExprKind::SystemCall { name, .. }
                if name == crate::intra_delay::INTRA_DELAY_MARKER
                    || name == crate::intra_delay::INTRA_EVENT_MARKER
                    || name == crate::intra_delay::INTRA_CYCLE_MARKER
        )
    }
}
