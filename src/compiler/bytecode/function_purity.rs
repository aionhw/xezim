//! Function purity checks for bytecode inlining.

use super::*;

impl BytecodeCompiler<'_> {
    /// True when `fd`'s body reads only its formals, its own declared locals
    /// and compile-time constants — i.e. its result depends on nothing but the
    /// arguments. Conservative: any construct not understood here says "no".
    /// `prefix` is the instance scope the function was registered under
    /// (`u0` for a key `u0.onehot`). Elaboration rewrites an instantiated
    /// module's function body to instance-qualified names, so its OWN
    /// formals and result come back as `u0.c` / `u0.onehot`. Judging those
    /// as free names made every function inside an instantiated module look
    /// impure — i.e. inlinable only at the top level, which is nowhere in
    /// real RTL. Stripping the function's own prefix restores the intended
    /// test: is every name a formal, a local, or a constant?
    pub(super) fn fn_is_pure_in(&self, fd: &FunctionDeclaration, prefix: Option<&str>) -> bool {
        self.fn_is_pure_in_ext(fd, prefix, false)
    }

    /// `allow_ext_reads`: additionally accept free names as module-state
    /// READS (the caller has established they resolve unambiguously — see
    /// `compile_pure_call`). Assignment TARGETS are always held to the strict
    /// rule: an inlined body must not write module state.
    pub(super) fn fn_is_pure_in_ext(
        &self,
        fd: &FunctionDeclaration,
        prefix: Option<&str>,
        allow_ext_reads: bool,
    ) -> bool {
        const MAX_PURITY_DEPTH: u32 = 8;
        if self.purity_depth.get() >= MAX_PURITY_DEPTH {
            self.purity_cut.set(true);
            return false;
        }
        let facts = self.method_admission.as_ref().map(|a| &a.fn_facts);
        let key = (
            fd.name.name.name.clone(),
            fd.span.start,
            fd.span.end,
            prefix.map(str::to_string),
            allow_ext_reads,
        );
        if let Some(hit) = facts.and_then(|f| f.pure.borrow().get(&key).copied()) {
            return hit;
        }
        let outer_cut = self.purity_cut.replace(false);
        self.purity_depth.set(self.purity_depth.get() + 1);
        let ok = self.fn_is_pure_in_inner(fd, prefix, allow_ext_reads);
        self.purity_depth.set(self.purity_depth.get() - 1);
        let cut = self.purity_cut.get();
        // `true` holds at any depth; `false` only when no nested check was
        // cut off by the depth limit (asked again higher up, it could pass).
        if (ok || !cut)
            && let Some(f) = facts
        {
            f.pure.borrow_mut().insert(key, ok);
        }
        self.purity_cut.set(outer_cut || cut);
        ok
    }

    /// `compile_pure_call`'s external-read admission: a callee with no
    /// instance-qualified (`<inst>.name`) twin in the function table.
    pub(super) fn callee_ext_reads_ok(&self, name: &str) -> bool {
        if name.contains('.') {
            return true;
        }
        let facts = self.method_admission.as_ref().map(|a| &a.fn_facts);
        if let Some(hit) = facts.and_then(|f| f.ext_reads_ok.borrow().get(name).copied()) {
            return hit;
        }
        let ok = self.functions.is_some_and(|f| {
            if let Some(index) = self.function_scope_index {
                return !index.qualified_leaves.contains(name);
            }
            let suffix = format!(".{}", name);
            !f.keys().any(|k| k.ends_with(suffix.as_str()))
        });
        if let Some(f) = facts {
            f.ext_reads_ok.borrow_mut().insert(name.to_string(), ok);
        }
        ok
    }

    fn fn_is_pure_in_inner(
        &self,
        fd: &FunctionDeclaration,
        prefix: Option<&str>,
        allow_ext_reads: bool,
    ) -> bool {
        // A DPI IMPORT function (`import "DPI-C" function string f(...);`)
        // has NO body: it is a foreign call the compiled path cannot inline
        // or const-fold. An empty body used to look "vacuously pure", so a
        // wrapper like uvm_dpi_get_next_arg inlined it to a constant and
        // every call returned garbage/empty strings (uvm_complete_test:
        // the +UVM_TESTNAME cmdline scan read an empty string and run_test
        // never saw the test name). Treat a body-less function as impure —
        // the call stays on the interpreter, which performs the DPI call.
        if self
            .dpi_import_fds
            .as_deref()
            .is_some_and(|m| m.contains_key(&fd.name.name.name))
        {
            return false;
        }
        let mut bound: HashSet<String> = HashSet::default();
        bound.insert(fd.name.name.name.clone());
        for p in &fd.ports {
            bound.insert(p.name.name.clone());
        }
        // Accept the qualified spellings of exactly those same names.
        if let Some(pfx) = prefix {
            for n in bound.clone() {
                bound.insert(format!("{pfx}.{n}"));
            }
        }
        fn expr_ok(
            e: &Expression,
            bound: &HashSet<String>,
            me: &BytecodeCompiler,
            ext: bool,
        ) -> bool {
            match &e.kind {
                ExprKind::Ident(h) => {
                    if h.path.len() != 1 {
                        // `Cfg.VLEN`: a member of a packed-struct parameter
                        // is a constant (§6.20.2), not module state.
                        if h.path.iter().all(|seg| seg.selects.is_empty())
                            && me.lookup_param_value(h).is_some()
                        {
                            return true;
                        }
                        // `fname.member` (or a nested `c.p.hi`) may arrive as
                        // segments; it is function-local when the head is
                        // bound (the return variable or a formal).
                        return bound.contains(&h.path[0].name.name)
                            && h.path
                                .iter()
                                .all(|seg| seg.selects.iter().all(|x| expr_ok(x, bound, me, ext)));
                    }
                    let n = &h.path[0].name.name;
                    // A dotted name is only acceptable when it is one of the
                    // qualified spellings inserted above; anything else
                    // reaching outside the function stays impure — unless the
                    // caller allows module-state READS (`ext`), in which case
                    // a free name (dotted or bare) is a signal read the body
                    // compiler resolves (or bails on) itself.
                    let head_bound = n
                        .split_once('.')
                        .is_some_and(|(head, _)| bound.contains(head));
                    // §6.20: a parameter is a constant, not module state, in
                    // either spelling; an instance's function body names its
                    // module's `localparam` instance-qualified (`u0.W`).
                    let param = me.params.is_some_and(|p| p.contains_key(n));
                    if n.contains('.') && !bound.contains(n) && !head_bound && !ext && !param {
                        return false;
                    }
                    let known = ext || head_bound || bound.contains(n) || param;
                    known
                        && h.path[0]
                            .selects
                            .iter()
                            .all(|sel| expr_ok(sel, bound, me, ext))
                }
                ExprKind::Number(_) | ExprKind::StringLiteral(_) => true,
                ExprKind::Paren(i) => expr_ok(i, bound, me, ext),
                ExprKind::AssignExpr { lvalue, rvalue } => {
                    // §11.4.1: an assignment expression writes its target
                    // just as a blocking assignment statement does.
                    expr_ok(lvalue, bound, me, false) && expr_ok(rvalue, bound, me, ext)
                }
                ExprKind::Unary {
                    op: UnaryOp::PreIncr | UnaryOp::PostIncr | UnaryOp::PreDecr | UnaryOp::PostDecr,
                    operand,
                } => expr_ok(operand, bound, me, false),
                ExprKind::Unary { operand, .. } => expr_ok(operand, bound, me, ext),
                ExprKind::Binary { left, right, .. } => {
                    expr_ok(left, bound, me, ext) && expr_ok(right, bound, me, ext)
                }
                ExprKind::Conditional {
                    condition,
                    then_expr,
                    else_expr,
                } => {
                    expr_ok(condition, bound, me, ext)
                        && expr_ok(then_expr, bound, me, ext)
                        && expr_ok(else_expr, bound, me, ext)
                }
                ExprKind::Concatenation(parts) => parts.iter().all(|p| expr_ok(p, bound, me, ext)),
                ExprKind::Replication { count, exprs } => {
                    expr_ok(count, bound, me, ext)
                        && exprs.iter().all(|p| expr_ok(p, bound, me, ext))
                }
                ExprKind::Index { expr, index } => {
                    expr_ok(expr, bound, me, ext) && expr_ok(index, bound, me, ext)
                }
                ExprKind::RangeSelect {
                    expr, left, right, ..
                } => {
                    expr_ok(expr, bound, me, ext)
                        && expr_ok(left, bound, me, ext)
                        && expr_ok(right, bound, me, ext)
                }
                // A nested call keeps the function pure iff the CALLEE is
                // itself pure-in-arguments and every argument is. AES's
                // `mix_column` calls `xtime(a ^ b)`; without this arm the
                // whole helper — and the task enclosing it — was branded
                // impure and stayed on the interpreter.
                // Pure formatting/interpretation builtins: value depends
                // only on the (checked) arguments. `$sformatf` here is what
                // lets the string decode helpers (get_csr_name-style: a big
                // case of returns with a formatted default) inline at all.
                ExprKind::SystemCall { name, args } => {
                    // Casts desugar to these; the FIRST argument is the
                    // target type/size NAME (not a data read), so only the
                    // operand arguments gate purity — `u8v_t'(128'(x))` kept
                    // branding its whole function impure.
                    if matches!(
                        name.as_str(),
                        "$__xz_named_cast" | "$__xz_size_cast" | "$__xz_type_cast"
                    ) {
                        return args.iter().skip(1).all(|a| expr_ok(a, bound, me, ext));
                    }
                    (matches!(name.as_str(), "$sformatf" | "$psprintf" | "$signed" | "$unsigned")
                        || (me.allow_waits && stepped_math_function(name)))
                        && args.iter().all(|a| expr_ok(a, bound, me, ext))
                }
                ExprKind::Call { func, args } => {
                    let ExprKind::Ident(h) = &func.kind else {
                        return false;
                    };
                    if h.root.is_some() || h.path.iter().any(|s| !s.selects.is_empty()) {
                        return false;
                    }
                    let name = BytecodeCompiler::hier_raw_name(h);
                    if let Some((key, fd2)) = me.functions.and_then(|f| {
                        f.get_key_value(&name).or_else(|| {
                            name.rsplit('.')
                                .next()
                                .and_then(|leaf| f.get_key_value(leaf))
                        })
                    }) {
                        // Input-only callees here (an output arg would write a
                        // caller name this walker cannot track). A callee that
                        // is not itself inlinable may still be DELEGATED to the
                        // interpreter via CallFreeFunction (impure body, or a
                        // `string` formal the inline shape rejects): scalar
                        // inputs with arity ≤ ports is delegatable, so the
                        // caller stays compilable. §13.5.4 named actuals are
                        // normalized to positional order first.
                        let owned;
                        let args = if args
                            .iter()
                            .any(|a| matches!(a.kind, ExprKind::NamedArg { .. } | ExprKind::Empty))
                        {
                            let Some(v) =
                                crate::compiler::simulator::Simulator::normalize_call_args(
                                    &fd2.ports, args,
                                )
                            else {
                                return false;
                            };
                            owned = v;
                            owned.as_slice()
                        } else {
                            args
                        };
                        let input_only = fd2
                            .ports
                            .iter()
                            .all(|p| matches!(p.direction, PortDirection::Input));
                        let scalar_ports = fd2.ports.iter().all(|p| p.dimensions.is_empty())
                            && me.call_passes_values(fd2, key.rsplit_once('.').map(|(p, _)| p));
                        let arity_le = fd2.ports.len() >= args.len();
                        let args_ok = args.iter().all(|a| expr_ok(a, bound, me, ext));
                        // The callee is judged in the scope it was registered
                        // under, as a direct call judges it: an instance's
                        // function body names its own result and its
                        // module's parameters instance-qualified (`u0.f`,
                        // `u0.W`), which the bare walk took for module state.
                        let pfx2 = key.rsplit_once('.').map(|(p, _)| p);
                        // Edge blocks can delegate integral helpers under the
                        // same conditions as the bytecode call site.
                        let delegatable = scalar_ports
                            && arity_le
                            && (me.method_mode
                                || (me.edge_delegate
                                    && fd2.ports.iter().all(|p| {
                                        me.edge_call_formal_ok(&p.data_type, Some(key))
                                    })));
                        return input_only && args_ok && (delegatable || me.fn_is_pure_in(fd2, pfx2));
                    }
                    // Not a declared function — a DPI IMPORT is still a
                    // legitimate callee: the interpreter executes it through
                    // exec_dpi_import_call and the compiled body lowers it to
                    // a CallFreeFunction whose exec arm falls back to the DPI
                    // dispatch. Treat the import as a leaf: input-only ports,
                    // argument-contained actuals; an `output` formal only
                    // when its actual is a plain assignable lvalue (the
                    // copyback target).
                    let Some(idf) = me
                        .dpi_import_fds
                        .as_deref()
                        .and_then(|m| m.get(&name))
                        .cloned()
                    else {
                        return false;
                    };
                    let normalized = crate::compiler::simulator::Simulator::normalize_call_args(
                        &idf.ports, args,
                    );
                    let args = normalized.as_deref().unwrap_or(args);
                    idf.ports.len() == args.len()
                        && idf.ports.iter().zip(args).all(|(p, a)| match p.direction {
                            PortDirection::Input => expr_ok(a, bound, me, ext),
                            PortDirection::Output => matches!(
                                &a.kind,
                                ExprKind::Ident(h)
                                    if h.root.is_none()
                                        && h.path.len() == 1
                                        && h.path[0].selects.is_empty()
                            ),
                            _ => false,
                        })
                }
                // §10.9.2: a pattern builds its value from nothing but the
                // expressions inside it, so it is exactly as pure as they are.
                // Falling through to `false` here made every function whose
                // body was `return '{...}` — the ordinary way to build a
                // struct result — impure, and so never inlined.
                // `fname.member` / `formal.member`: as pure as its base, which
                // is the function's own return variable or a formal -- both
                // bound, so this stays function-local. Inside a function body
                // the member access is NOT collapsed to a dotted Ident.
                ExprKind::MemberAccess { expr, .. } => expr_ok(expr, bound, me, ext),
                ExprKind::AssignmentPattern(items) => {
                    use crate::ast::expr::AssignmentPatternItem as It;
                    items.iter().all(|it| match it {
                        It::Named(_, e) | It::Ordered(e) | It::Default(e) => {
                            expr_ok(e, bound, me, ext)
                        }
                        _ => false,
                    })
                }
                ExprKind::TypedPattern { pattern, .. } => expr_ok(pattern, bound, me, ext),
                _ => false,
            }
        }
        fn stmt_ok(
            st: &Statement,
            bound: &mut HashSet<String>,
            me: &BytecodeCompiler,
            ext: bool,
        ) -> bool {
            match &st.kind {
                StatementKind::Null => true,
                // §6.21: a `static` local keeps its value from one call to
                // the next; an inlined copy would start from scratch.
                StatementKind::VarDecl {
                    lifetime: Some(crate::ast::types::Lifetime::Static),
                    ..
                } => false,
                StatementKind::VarDecl { declarators, .. }
                | StatementKind::ParamDecl { declarators, .. } => {
                    for d in declarators {
                        if let Some(e) = &d.init {
                            if !expr_ok(e, bound, me, ext) {
                                return false;
                            }
                        }
                        bound.insert(d.name.name.clone());
                    }
                    true
                }
                StatementKind::BlockingAssign { lvalue, rvalue } => {
                    // The TARGET is held strict regardless of `ext`: an
                    // inlined body must not write module state. A local-array
                    // element target (`row[n] = …`) is a bound name with a
                    // select, which the strict Ident arm accepts; only the
                    // INDEX may use ext reads.
                    expr_ok(lvalue, bound, me, false) && expr_ok(rvalue, bound, me, ext)
                }
                StatementKind::Return(e) => e.as_ref().is_none_or(|e| expr_ok(e, bound, me, ext)),
                // A void call in statement position (`uvm_report_error(...)`)
                // is as pure as the expression walker says: an inlinable,
                // delegatable (scalar-input) or DPI-import callee is a leaf.
                // Method bodies and edge blocks may delegate these calls.
                StatementKind::Expr(e) if me.method_mode || me.edge_delegate => {
                    let mut e = e;
                    while let ExprKind::Paren(inner) = &e.kind {
                        e = inner;
                    }
                    expr_ok(e, bound, me, ext)
                }
                StatementKind::While { condition, body } => {
                    // Same gap as the Foreach arm (issue #146): no arm meant
                    // `_ => false`, branding a pure while-loop helper
                    // (popcnt-style) impure.
                    expr_ok(condition, bound, me, ext) && stmt_ok(body, &mut bound.clone(), me, ext)
                }
                StatementKind::DoWhile { body, condition } => {
                    expr_ok(condition, bound, me, ext) && stmt_ok(body, &mut bound.clone(), me, ext)
                }
                StatementKind::Repeat { count, body } => {
                    expr_ok(count, bound, me, ext) && stmt_ok(body, &mut bound.clone(), me, ext)
                }
                // §12.7: control flow only; reads nothing, writes nothing.
                StatementKind::Break | StatementKind::Continue => true,
                StatementKind::Foreach { array, vars, body } => {
                    // §12.7.3: the loop variables are implicitly DECLARED by
                    // the foreach for its body — without this arm they read
                    // as free module names and every resolver body using
                    // `foreach (drivers[i])` was branded impure (the
                    // Expr_Call_impure half of issue #137).
                    if !expr_ok(array, bound, me, ext) {
                        return false;
                    }
                    let mut inner = bound.clone();
                    for v in vars.iter().flatten() {
                        inner.insert(v.name.clone());
                    }
                    stmt_ok(body, &mut inner, me, ext)
                }
                StatementKind::SeqBlock { stmts, .. } => {
                    // A block's declarations are visible to the statements that
                    // FOLLOW them, so thread one scope through the sequence.
                    let mut inner = bound.clone();
                    stmts.iter().all(|s| stmt_ok(s, &mut inner, me, ext))
                }
                StatementKind::If {
                    condition,
                    then_stmt,
                    else_stmt,
                    ..
                } => {
                    expr_ok(condition, bound, me, ext)
                        && stmt_ok(then_stmt, &mut bound.clone(), me, ext)
                        && else_stmt
                            .as_ref()
                            .is_none_or(|e| stmt_ok(e, &mut bound.clone(), me, ext))
                }
                StatementKind::For {
                    init,
                    condition,
                    step,
                    body,
                } => {
                    let mut inner = bound.clone();
                    for fi in init {
                        match fi {
                            ForInit::VarDecl { name, init, .. } => {
                                if !expr_ok(init, &inner, me, ext) {
                                    return false;
                                }
                                inner.insert(name.name.clone());
                            }
                            ForInit::Assign { lvalue, rvalue } => {
                                if !expr_ok(lvalue, &inner, me, false)
                                    || !expr_ok(rvalue, &inner, me, ext)
                                {
                                    return false;
                                }
                            }
                        }
                    }
                    condition
                        .as_ref()
                        .is_none_or(|c| expr_ok(c, &inner, me, ext))
                        && step.iter().all(|e| expr_ok(e, &inner, me, ext))
                        && stmt_ok(body, &mut inner, me, ext)
                }
                // case/casez/casex — the shape of virtually every decode
                // helper (`assign x = f(op)` over a big casez). Leaving this
                // arm out branded ALL of them impure, which kept the whole
                // enclosing assign on the AST interpreter. The `matches`
                // pattern/guard form stays conservative.
                StatementKind::Case { expr, items, .. } => {
                    expr_ok(expr, bound, me, ext)
                        && items.iter().all(|it| {
                            it.pattern.is_none()
                                && it.guard.is_none()
                                && it.patterns.iter().all(|p| expr_ok(p, bound, me, ext))
                                && stmt_ok(&it.stmt, &mut bound.clone(), me, ext)
                        })
                }
                _ => false,
            }
        }
        let materialized = (!fd.name.name.name.is_empty())
            .then(|| {
                self.functions
                    .and_then(|fs| fs.get(&fd.name.name.name))
                    .filter(|raw| Arc::ptr_eq(fd.items.as_arc(), raw.items.as_arc()))
                    .and_then(|raw| self.materialize_deferred_fn(&fd.name.name.name, raw))
            })
            .flatten();
        let items = materialized.as_ref().unwrap_or(&fd.items);
        let mut scope = bound;
        items
            .iter()
            .all(|st| stmt_ok(st, &mut scope, self, allow_ext_reads))
    }
}
