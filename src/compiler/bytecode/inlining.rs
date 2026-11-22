//! Inlining of pure function calls and task calls.

use super::*;

impl<'a> BytecodeCompiler<'a> {
    pub(super) fn stmt_has_break_or_continue(stmt: &Statement) -> bool {
        match &stmt.kind {
            StatementKind::Break | StatementKind::Continue => true,
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                stmts.iter().any(Self::stmt_has_break_or_continue)
            }
            StatementKind::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                Self::stmt_has_break_or_continue(then_stmt)
                    || else_stmt
                        .as_ref()
                        .is_some_and(|e| Self::stmt_has_break_or_continue(e))
            }
            StatementKind::Case { items, .. } => items
                .iter()
                .any(|it| Self::stmt_has_break_or_continue(&it.stmt)),
            // Don't descend into nested loops — break/continue there target the
            // inner loop, not the enclosing one.
            _ => false,
        }
    }

    /// Does `stmt` declare a variable anywhere (a block-local `VarDecl`)?
    fn stmt_declares(stmt: &Statement) -> bool {
        match &stmt.kind {
            StatementKind::VarDecl { .. } | StatementKind::ParamDecl { .. } => true,
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                stmts.iter().any(Self::stmt_declares)
            }
            StatementKind::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                Self::stmt_declares(then_stmt)
                    || else_stmt.as_ref().is_some_and(|e| Self::stmt_declares(e))
            }
            StatementKind::Case { items, .. } => {
                items.iter().any(|it| Self::stmt_declares(&it.stmt))
            }
            StatementKind::For { body, .. }
            | StatementKind::Foreach { body, .. }
            | StatementKind::While { body, .. }
            | StatementKind::DoWhile { body, .. }
            | StatementKind::Repeat { body, .. }
            | StatementKind::Forever { body }
            | StatementKind::TimingControl { stmt: body, .. }
            | StatementKind::Wait { stmt: body, .. } => Self::stmt_declares(body),
            _ => false,
        }
    }

    /// §7.4.1: is `dt` a vector whose packed range is not `[N-1:0]` (an
    /// ascending range, or a low bound other than 0), directly or through a
    /// typedef? A bound that does not fold counts as one.
    pub(super) fn decl_has_offset_range(&self, dt: &crate::ast::types::DataType) -> bool {
        use crate::ast::types::{DataType, PackedDimension};
        // §7.4.1: so is a packed array of a named signed type. A select of
        // one of its elements is signed, which the interpreter reads from
        // the declaration of the formal or local.
        if self
            .typedef_types
            .is_some_and(|t| signed_named_packed_elem_depth(dt, t).is_some())
        {
            return true;
        }
        let offset = |dims: &[PackedDimension]| {
            dims.iter().any(|d| match d {
                PackedDimension::Range { left, right, .. } => {
                    let c =
                        |e| crate::compiler::elaborate::const_eval_i64_with_params(e, self.params);
                    c(right) != Some(0) || c(left).is_some_and(|l| l < 0)
                }
                PackedDimension::Unsized(_) => false,
            })
        };
        let mut cur = dt;
        for _ in 0..16 {
            match cur {
                DataType::IntegerVector { dimensions, .. }
                | DataType::Implicit { dimensions, .. } => {
                    return offset(dimensions);
                }
                DataType::TypeReference {
                    name, dimensions, ..
                } => {
                    if offset(dimensions) {
                        return true;
                    }
                    match self.typedef_types.and_then(|t| t.get(&name.name.name)) {
                        Some(next) => cur = next,
                        None => return false,
                    }
                }
                _ => return false,
            }
        }
        false
    }

    /// §7.4.1, §11.5.1: does a subroutine select (`v[i]`, `v[7:4]`,
    /// `v[i +: 2]`) one of its formals, locals or its result declared with a
    /// packed range other than `[N-1:0]`? Such a select names the bits its
    /// range labels, while an inlined body's select takes the label for a
    /// raw bit position: `n[0]` of `reg [0:7] n` read bit 0. The body is not
    /// inlined then; the interpreter places the label.
    fn selects_offset_range_var(
        &self,
        ports: &[crate::ast::decl::FunctionPort],
        ret: Option<(&str, &crate::ast::types::DataType)>,
        body: &[Statement],
    ) -> bool {
        fn decls<'s>(s: &'s Statement, out: &mut Vec<(&'s str, &'s crate::ast::types::DataType)>) {
            match &s.kind {
                StatementKind::VarDecl {
                    data_type,
                    declarators,
                    ..
                } => out.extend(
                    declarators
                        .iter()
                        .filter(|d| d.dimensions.is_empty())
                        .map(|d| (d.name.name.as_str(), data_type)),
                ),
                StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                    stmts.iter().for_each(|s| decls(s, out))
                }
                StatementKind::If {
                    then_stmt,
                    else_stmt,
                    ..
                } => {
                    decls(then_stmt, out);
                    if let Some(e) = else_stmt {
                        decls(e, out);
                    }
                }
                StatementKind::Case { items, .. } => {
                    items.iter().for_each(|it| decls(&it.stmt, out))
                }
                StatementKind::For { body, .. }
                | StatementKind::Foreach { body, .. }
                | StatementKind::While { body, .. }
                | StatementKind::DoWhile { body, .. }
                | StatementKind::Repeat { body, .. }
                | StatementKind::Forever { body } => decls(body, out),
                _ => {}
            }
        }
        let mut vars: Vec<(&str, &crate::ast::types::DataType)> = ports
            .iter()
            .filter(|p| p.dimensions.is_empty())
            .map(|p| (p.name.name.as_str(), &p.data_type))
            .chain(ret)
            .collect();
        body.iter().for_each(|s| decls(s, &mut vars));
        let names: Vec<&str> = vars
            .into_iter()
            .filter(|(_, dt)| self.decl_has_offset_range(dt))
            .map(|(n, _)| n)
            .collect();
        if names.is_empty() {
            return false;
        }
        let mut sc = super::fb_scan::FbScan::new(None);
        for s in body {
            sc.stmt(s);
        }
        // The body of an instantiated module's function names its own
        // variables instance-qualified too (`u0.n`).
        sc.names.iter().any(|(n, flags)| {
            flags & super::fb_scan::SEL != 0
                && names
                    .iter()
                    .any(|m| n == m || n.strip_suffix(m).is_some_and(|p| p.ends_with('.')))
        })
    }

    pub(super) fn stmt_is_blocking(stmt: &Statement) -> bool {
        match &stmt.kind {
            StatementKind::TimingControl { .. } => true,
            StatementKind::Wait { .. } => true,
            StatementKind::Forever { .. } => true,
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                stmts.iter().any(Self::stmt_is_blocking)
            }
            StatementKind::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                Self::stmt_is_blocking(then_stmt)
                    || else_stmt
                        .as_ref()
                        .is_some_and(|e| Self::stmt_is_blocking(e))
            }
            StatementKind::For { body, .. } | StatementKind::While { body, .. } => {
                Self::stmt_is_blocking(body)
            }
            _ => false,
        }
    }

    /// §26.3: the function-table key a BARE call names from the body being
    /// compiled. Inside a package function (free-function mode under
    /// `__free_fn__<pkg>`) a name declared in several scopes resolves in the
    /// enclosing package first, which the table keeps as `pkg::name` (the
    /// bare key holds whichever declaration registered last). A contested
    /// name the package does not declare itself, or one met inside an
    /// inlined callee (whose own package may differ), returns `None` so the
    /// body stays on the interpreter, which resolves it at run time.
    /// Uncontested names and every other compile mode keep the bare key.
    pub(super) fn scoped_callee_name(&self, bare: String) -> Option<String> {
        if !self.free_function_mode || bare.contains("::") {
            return Some(bare);
        }
        let Some(pkg) = self
            .method_class
            .as_deref()
            .and_then(|c| c.strip_prefix("__free_fn__"))
            .filter(|p| !p.is_empty())
        else {
            return Some(bare);
        };
        let Some(f) = self.functions else {
            return Some(bare);
        };
        let suffix = format!("::{bare}");
        if !f.keys().any(|k| k.ends_with(suffix.as_str())) {
            return Some(bare);
        }
        if !self.inlining_stack.is_empty() {
            return None;
        }
        let qualified = format!("{pkg}{suffix}");
        f.contains_key(&qualified).then_some(qualified)
    }

    /// `f(args).m` for a pure `f` returning an unpacked struct of scalars:
    /// inline the scalarized function for member `m` (`struct_sra`). None
    /// leaves the emitted code and the bail reason as they were.
    pub(super) fn compile_struct_member_call(
        &mut self,
        func: &Expression,
        args: &[Expression],
        member: &str,
    ) -> Option<RegId> {
        use crate::ast::types::{DataType, StructUnionKind};
        let ExprKind::Ident(h) = &func.kind else {
            return None;
        };
        if h.root.is_some() || h.path.len() != 1 || !h.path[0].selects.is_empty() {
            return None;
        }
        let name = self.scoped_callee_name(h.path[0].name.name.clone())?;
        let fd_ref = self.functions?.get(&name)?;
        let mut owned_fd;
        let fd = if let Some(items) = self.materialize_deferred_fn(&name, fd_ref) {
            owned_fd = fd_ref.clone();
            owned_fd.items = items;
            &owned_fd
        } else {
            fd_ref
        };
        let scope = Some(name.as_str());
        let members = |dt: &DataType| -> Option<super::struct_sra::Members> {
            let DataType::Struct(su) = self.resolved_decl_type(dt, scope) else {
                return None;
            };
            if su.packed || !matches!(su.kind, StructUnionKind::Struct) {
                return None;
            }
            let mut out = Vec::new();
            for m in &su.members {
                let scalar = !matches!(
                    self.resolved_decl_type(&m.data_type, scope),
                    DataType::Struct(_)
                ) && (self.decl_is_real(&m.data_type, scope)
                    || (1..=64).contains(&self.decl_width_in(&m.data_type, scope)));
                for d in &m.declarators {
                    // §7.2.2: a member default initializes the scalarized
                    // locals too; they are declared without one.
                    if !scalar || !d.dimensions.is_empty() || d.init.is_some() {
                        return None;
                    }
                    out.push((d.name.name.clone(), m.data_type.clone()));
                }
            }
            (!out.is_empty()).then_some(out)
        };
        // §7.4.2/§10.9.2: typedef array members keep the existing scalarizer
        // handling; only the new replication expansion is disabled for them.
        let replication_allowed = |ms: &super::struct_sra::Members| {
            !ms.iter().any(|(_, dt)| {
                let DataType::TypeReference { name, .. } = dt else {
                    return false;
                };
                self.typedef_unpacked_dims.is_some_and(|dims| {
                    self.typedef_entry(dims, name, scope)
                        .is_some_and(|(_, d)| !d.is_empty())
                })
            })
        };
        let (sfd, sargs) = super::struct_sra::scalarize_member_call(
            fd,
            args,
            member,
            &members,
            &replication_allowed,
        )?;
        let start = self.insns.len();
        let next = self.next_reg;
        let saved_reason = self.bail_reason;
        self.struct_call_fd = Some(sfd);
        let r = self.compile_pure_call(func, &sargs);
        self.struct_call_fd = None;
        if r.is_none() {
            self.truncate_insns(start);
            self.next_reg = next;
            self.bail_reason = saved_reason;
        }
        r
    }

    /// Try to inline a zero-arg, non-blocking user task's body at this
    /// call site. Returns true if successfully inlined.
    /// Inline a call to a pure combinational function, yielding the register
    /// holding its result. Accepts only argument-contained functions: input
    /// formals and directly writable `ref` actuals, with no external reads.
    pub(super) fn compile_pure_call(
        &mut self,
        func: &Expression,
        args: &[Expression],
    ) -> Option<RegId> {
        let name = match &func.kind {
            ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                h.path[0].name.name.clone()
            }
            _ => {
                self.bail("Expr_Call_func_not_single_ident");
                return None;
            }
        };
        let Some(name) = self.scoped_callee_name(name) else {
            self.bail("Expr_Call_pkg_scope");
            return None;
        };
        if self.inlining_stack.len() >= MAX_INLINE_DEPTH
            || self.inlining_stack.iter().any(|n| *n == name)
        {
            self.bail("Expr_Call_depth");
            return None;
        }
        // DPI import callee: not in the function table, but the interpreter
        // can execute it (exec_dpi_import_call). Emit a CallFreeFunction —
        // the exec arm falls back to the DPI dispatch when fn_decl_rc
        // misses. Admission mirrors the inlinable shape: input-only ports
        // (plus assignable `output` formals for functions like
        // uvm_re_compexecfree), no dimensions, arity matches after named-arg
        // normalization. Same register-file context as the existing
        // CallFreeFunction lowering, so method mode only. Structural only.
        if self.method_mode
            && let Some(idf) = self
                .dpi_import_fds
                .as_deref()
                .and_then(|m| m.get(&name))
                .cloned()
        {
            let args: Vec<Expression> =
                crate::compiler::simulator::Simulator::normalize_call_args(&idf.ports, args)
                    .unwrap_or_else(|| args.to_vec());
            if idf.ports.len() != args.len()
                || !idf
                    .ports
                    .iter()
                    .all(|p| matches!(p.direction, PortDirection::Input | PortDirection::Output))
                || idf.ports.iter().any(|p| !p.dimensions.is_empty())
            {
                self.bail("Expr_Call_ports");
                return None;
            }
            let mut ok = true;
            let call_start = self.insns.len();
            let call_next = self.next_reg;
            let mut arg_values: Vec<RegId> = Vec::with_capacity(args.len());
            for (i, a) in args.iter().enumerate() {
                // §13.5, §11.6.1: an integral input actual is evaluated at
                // its formal's width, as the interpreter's import call does.
                let w = self.actual_ctx_width_of(&idf.ports, i, &name, true);
                match self.compile_expr(a, w) {
                    Some(r) => arg_values.push(r),
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                let dest = self.alloc_reg();
                let n = arg_values.len() as u32;
                let arg_start = self.alloc_reg();
                for _ in 1..arg_values.len() {
                    self.alloc_reg();
                }
                for (i, &v) in arg_values.iter().enumerate() {
                    let slot = (arg_start as usize + i) as RegId;
                    if slot != v {
                        self.emit(Insn::Move(slot, v));
                    }
                }
                self.emit(Insn::CallFreeFunction(
                    dest,
                    Box::new(Name(name.into_boxed_str())),
                    arg_start,
                    n,
                ));
                // `output` formals of the import: the callee writes the
                // synthesized actuals in place; copy them back to the
                // caller's lvalues.
                for (i, p) in idf.ports.iter().enumerate() {
                    if matches!(p.direction, PortDirection::Output) {
                        if !self.compile_blocking_target(
                            &args[i],
                            (arg_start as usize + i) as RegId,
                            0,
                        ) {
                            ok = false;
                            break;
                        }
                    }
                }
                if ok {
                    return Some(dest);
                }
            }
            self.insns.truncate(call_start);
            self.next_reg = call_next;
            self.bail("Expr_Call_dpi");
            return None;
        }
        // A scalarized stand-in (`compile_struct_member_call`) is never
        // delegated: its formals are not the declared function's.
        let (mut fd, scalarized) = match self.struct_call_fd.take() {
            Some(fd) => (fd, true),
            None => {
                let Some(fd) = self.functions.and_then(|f| f.get(&name)).cloned() else {
                    self.bail("Expr_Call");
                    return None;
                };
                (fd, false)
            }
        };
        if self.decl_is_shortreal(&fd.return_type, Some(&name))
            || fd.ports.iter().any(|p| self.decl_is_shortreal(&p.data_type, Some(&name)))
        {
            self.bail("shortreal_call");
            return None;
        }
        if !scalarized
            && let Some(items) = self.materialize_deferred_fn(&name, &fd)
        {
            fd.items = items;
        }
        // §13.5.4 named actuals (`.re(expr)`) normalize to positional order
        // (with §13.5.3 defaults filled) so every path below — delegation and
        // inline — sees a plain positional actual list.
        let owned_args;
        let args: &[Expression] = if args
            .iter()
            .any(|a| matches!(a.kind, ExprKind::NamedArg { .. } | ExprKind::Empty))
        {
            // A failed normalization (nothing bindable) keeps the call on
            // the interpreter instead of compiling a zero-argument call.
            let Some(v) =
                crate::compiler::simulator::Simulator::normalize_call_args(&fd.ports, args)
            else {
                self.bail("Expr_Call_named_args");
                return None;
            };
            owned_args = v;
            &owned_args
        } else if args.len() < fd.ports.len()
            && fd.ports[args.len()..].iter().all(|p| {
                matches!(p.direction, PortDirection::Input)
                    && p.default
                        .as_ref()
                        .is_some_and(|d| matches!(d.kind, ExprKind::Number(_)))
            })
        {
            // §13.5.3: a literal default has the same meaning in the caller
            // and callee scopes. Bind it using the usual formal conversion.
            let mut full_args = args.to_vec();
            full_args.extend(
                fd.ports[args.len()..]
                    .iter()
                    .filter_map(|p| p.default.clone()),
            );
            owned_args = full_args;
            &owned_args
        } else {
            args
        };
        // Whether the callee may be inlined: decided at most once, for the
        // delegation shape below or the inline path after it.
        let mut fn_pure_memo: Option<bool> = None;
        let mut fn_pure = |me: &Self| -> bool {
            *fn_pure_memo.get_or_insert_with(|| {
                let ext = me.callee_ext_reads_ok(&name);
                me.fn_is_pure_in_ext(&fd, name.rsplit_once('.').map(|(p, _)| p), ext)
            })
        };
        // class-perf delegation: a callee that cannot be INLINED (impure, or
        // a `string` formal the inline shape rejects) but whose ports are
        // all scalar inputs with a matching arity is still callable from
        // compiled code: emit CallFreeFunction and let the exec arm
        // re-enter the interpreter (exec_function_call), exactly like the
        // class-method CallMethod lowering. This keeps the caller compiled
        // — the callee's side effects run with full interpreter semantics.
        {
            let input_only_scalar = fd
                .ports
                .iter()
                .all(|p| matches!(p.direction, PortDirection::Input) && p.dimensions.is_empty());
            let arity_le = args.len() <= fd.ports.len()
                && !args
                    .iter()
                    .any(|a| matches!(a.kind, ExprKind::NamedArg { .. } | ExprKind::Empty));
            let has_string_port = fd.ports.iter().any(|p| {
                matches!(
                    &p.data_type,
                    crate::ast::types::DataType::Simple {
                        kind: crate::ast::types::SimpleType::String,
                        ..
                    }
                )
            });
            // §13.4: a process body (`allow_waits`, or a timed body without
            // waits, `delegate_calls`) delegates too. A call there
            // runs the function once per execution, as the interpreter would,
            // so its writes to module variables and its system tasks need no
            // purity; only the name must reach this instance's own copy (a
            // dotted registration, or a bare one with no dotted twin). Edge
            // and comb blocks keep inlining pure callees alone: their
            // sensitivity follows the compiled reads.
            let delegable = self.method_mode
                || ((self.allow_waits || self.delegate_calls) && self.callee_ext_reads_ok(&name));
            if delegable
                && !scalarized
                && input_only_scalar
                && arity_le
                && (has_string_port || !fn_pure(self))
            {
                let call_start = self.insns.len();
                let call_next = self.next_reg;
                let mut ok = true;
                let mut arg_values: Vec<RegId> = Vec::with_capacity(args.len());
                for (i, a) in args.iter().enumerate() {
                    // §13.5, §11.6.1: as `exec_function_call` binds the
                    // actual, at its formal's width; the register reaches it
                    // as a value, too late to widen the operands.
                    let w = self.actual_ctx_width_of(&fd.ports, i, &name, false);
                    match self.compile_expr(a, w) {
                        Some(r) => arg_values.push(r),
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                if ok {
                    let dest = self.alloc_reg();
                    let n = arg_values.len() as u32;
                    let arg_start = self.alloc_reg();
                    for _ in 1..arg_values.len() {
                        self.alloc_reg();
                    }
                    for (i, &v) in arg_values.iter().enumerate() {
                        let slot = (arg_start as usize + i) as RegId;
                        if slot != v {
                            self.emit(Insn::Move(slot, v));
                        }
                    }
                    self.emit(Insn::CallFreeFunction(
                        dest,
                        Box::new(Name(name.clone().into_boxed_str())),
                        arg_start,
                        n,
                    ));
                    // The defaulted tail is bound by exec_function_call's
                    // own §13.5.3 normalization; outputs are excluded by
                    // input_only_scalar.
                    return Some(dest);
                }
                self.insns.truncate(call_start);
                self.next_reg = call_next;
                self.bail("Expr_Call_delegate");
                return None;
            }
        }

        // §6.6.7 resolver dispatch: a DYNAMIC-ARRAY formal (`input real
        // drivers[]`) whose actual is a FIXED assignment pattern is
        // monomorphic at this call site — the nettype resolution machinery
        // synthesizes one Ordered element per driver — so it binds as a
        // fixed local array of per-element registers (the #129 machinery).
        // This was the single largest RNM cost: every node re-ran the
        // resolver on the AST path each settle (issue #137, ~25µs/eval for
        // a 4-term multiply-accumulate).
        let dyn_array_formal = |p: &_, a: &Expression| -> bool {
            matches!(p, &crate::ast::decl::FunctionPort { .. })
                && matches!(
                    &a.kind,
                    ExprKind::AssignmentPattern(items)
                        if (1..=32).contains(&items.len())
                            && items.iter().all(|it| matches!(
                                it,
                                crate::ast::expr::AssignmentPatternItem::Ordered(_)
                            ))
                )
        };
        let is_unsized_dim = |p: &crate::ast::decl::FunctionPort| {
            matches!(
                p.dimensions.as_slice(),
                [crate::ast::types::UnpackedDimension::Unsized(_)]
            ) && matches!(p.direction, PortDirection::Input)
        };
        let mut array_output = |p: &crate::ast::decl::FunctionPort, a: &Expression| {
            use crate::ast::types::{Lifetime, UnpackedDimension};
            if p.direction != PortDirection::Output || fd.lifetime != Some(Lifetime::Automatic) {
                return None;
            }
            let ExprKind::Ident(h) = &a.kind else {
                return None;
            };
            if h.root.is_some() || h.path.len() != 1 || !h.path[0].selects.is_empty() {
                return None;
            }
            let bank = *self.local_array_regs.get(&h.path[0].name.name)?;
            let number = |e: &Expression| {
                if let ExprKind::Number(n) = &e.kind {
                    self.eval_number_static(n)?.to_u64()
                } else {
                    None
                }
            };
            let (left, right) = match p.dimensions.as_slice() {
                [UnpackedDimension::Range { left, right, .. }] => (number(left)?, number(right)?),
                [UnpackedDimension::Expression { expr, .. }] => (0, number(expr)?.checked_sub(1)?),
                _ => return None,
            };
            let element = self.local_kind_of(&p.data_type, Some(&name))?;
            if !matches!(element, LocalKind::Int { .. })
                || bank.element != Some(element)
                || left.max(right) > i32::MAX as u64
                || left.abs_diff(right).checked_add(1)? != bank.len as u64
            {
                return None;
            }
            Some((
                bank,
                i64::try_from(left.min(right)).ok()?,
                left > right,
                element,
            ))
        };
        let output_bank = fd
            .ports
            .iter()
            .position(|p| p.direction == PortDirection::Output && !p.dimensions.is_empty())
            .and_then(|index| {
                if fd
                    .ports
                    .iter()
                    .enumerate()
                    .any(|(i, p)| i != index && p.direction != PortDirection::Input)
                {
                    return None;
                }
                Some((index, array_output(&fd.ports[index], args.get(index)?)?))
            });
        if fd.ports.len() != args.len()
            || fd.ports.iter().zip(args).enumerate().any(|(i, (p, a))| {
                // §13.5.2 output formals ride the existing ref-writeback
                // machinery: the body sees a register, the caller's actual
                // is written on return. AES-style helpers
                // (`mix_column(c0.., r0..)`) are exactly this shape.
                (!matches!(
                    p.direction,
                    PortDirection::Input | PortDirection::Ref | PortDirection::Output
                ) || !p.dimensions.is_empty())
                    && !(is_unsized_dim(p) && dyn_array_formal(p, a))
                    && output_bank.is_none_or(|(index, _)| index != i)
            })
        {
            self.bail("Expr_Call_ports");
            return None;
        }
        // REAL-typed formals or return: register binding resizes through
        // integral widths, which destroys real semantics (an integral actual
        // must CONVERT to the real formal per §13.3.1, not bit-copy). The
        // AST call path does this correctly; stay on it.
        // REAL formals/return are admitted with §13.3.1 conversion at the
        // register bind (`emit_to_real`); `Value` arithmetic and the store
        // paths are real-aware, and `resize(64)` is identity on a real. Only
        // ref/output REAL formals stay on the AST path — their write-back
        // would need the reverse conversion against the actual's own type.
        let ret_is_real = self.decl_is_real(&fd.return_type, Some(&name));
        if fd.ports.iter().any(|p| {
            self.decl_is_real(&p.data_type, Some(&name))
                && !matches!(p.direction, PortDirection::Input)
        }) {
            self.bail("Expr_Call_real_ref");
            return None;
        }
        // Only inline a function that is PURE IN ITS ARGUMENTS: every name its
        // body reads must be a formal, one of its own locals, or a constant.
        // A function that reads module signals must NOT be inlined here — the
        // elaborator registers an instance's functions under BOTH the bare and
        // the instance-qualified name, so a bare-name lookup can pick the
        // un-rewritten copy whose free names belong to another scope. It also
        // keeps the AST path's sensitivity handling (which follows a callee's
        // reads) authoritative for such functions.
        // Module-signal READS are admissible when the free names resolve
        // unambiguously: a DOTTED registration's body was rewritten to
        // instance-qualified (absolute) names, and a bare registration with
        // no dotted twin can only be the top module's own copy (§13.4.2 —
        // the value may then depend on module state, and the CA dependency
        // machinery follows callee reads, so re-evaluation still triggers).
        // Writes to module state stay disqualifying either way.
        if !fn_pure(self) {
            if std::env::var_os("XEZIM_PROBE_INLINE").is_some() {
                eprintln!("[INLINE-FAIL] fn {} reason=impure", name);
            }
            self.bail("Expr_Call_impure");
            return None;
        }
        // Unwrap the body, through one level of begin/end.
        let items: Vec<Statement> = match fd.items.as_slice() {
            [one] => match &one.kind {
                StatementKind::SeqBlock { stmts, .. } => stmts.to_vec(),
                _ => vec![one.clone()],
            },
            other => other.to_vec(),
        };
        // Nothing that suspends, and no nested calls we cannot see through.
        if items.iter().any(Self::stmt_is_blocking) {
            self.bail("Expr_Call_blocking");
            return None;
        }
        if self.selects_offset_range_var(
            &fd.ports,
            Some((fd.name.name.name.as_str(), &fd.return_type)),
            &items,
        ) {
            self.bail("Expr_Call_offset_range_select");
            return None;
        }
        let ret_is_string = self.decl_is_string(&fd.return_type, Some(&name));
        let ret_w = if ret_is_string {
            0
        } else {
            // Resolve through the typedef table (and the instance scope): the
            // old bare `resolve_type_width(.., None)` call defaulted every
            // typedef'd return type to 32 bits, silently truncating a wide
            // result on `return`.
            self.decl_width_in(&fd.return_type, Some(&name))
        };
        // Evaluate the arguments in the CALLER's scope first, then bind them as
        // register-backed locals while compiling the body.
        let mut binds: Vec<(String, (RegId, u32))> = Vec::with_capacity(args.len());
        let mut elem_binds: Vec<(String, u32)> = Vec::new();
        let mut ref_writes: Vec<(Expression, RegId, u32)> = Vec::new();
        let mut string_formal_names: Vec<String> = Vec::new();
        let mut real_formal_names: Vec<String> = Vec::new();
        let mut array_binds: Vec<(String, LocalArrayBind)> = Vec::new();
        let mut struct_binds: Vec<(String, (RegId, Vec<(String, FormalMember)>))> = Vec::new();
        let mut output_elements = Vec::new();
        for (i, (p, a)) in fd.ports.iter().zip(args).enumerate() {
            if let Some((_, (bank, lo, descending, element))) =
                output_bank.filter(|(index, _)| *index == i)
            {
                // §13.5.3: initialize a private formal array and copy it back in
                // declaration order, including on an early return.
                let init = self.type_default_value_in(&p.data_type, bank.width, Some(&name));
                let mut regs = Vec::with_capacity(bank.len);
                for offset in 0..bank.len {
                    let slot = self.alloc_reg();
                    self.emit(Insn::LoadConst(slot, Box::new(init.clone())));
                    self.local_kinds.insert(slot, element);
                    let out = if descending == bank.descending {
                        offset
                    } else {
                        bank.len - 1 - offset
                    };
                    output_elements.push((bank.base + out as RegId, slot));
                    regs.push(slot);
                }
                let LocalKind::Int { signed, .. } = element else {
                    unreachable!()
                };
                array_binds.push((
                    p.name.name.clone(),
                    LocalArrayBind {
                        regs,
                        lo,
                        elem_w: bank.width,
                        is_real: false,
                        elem_signed: Some(signed),
                    },
                ));
                continue;
            }
            if is_unsized_dim(p) {
                // Dynamic-array formal from a fixed assignment pattern: one
                // register per element, evaluated in the CALLER's scope. The
                // gate above already proved the shape.
                let ExprKind::AssignmentPattern(items) = &a.kind else {
                    self.bail("Expr_Call_ports");
                    return None;
                };
                let is_real = self.decl_is_real(&p.data_type, Some(&name));
                let elem_w = if is_real {
                    64
                } else {
                    self.decl_width_in(&p.data_type, Some(&name))
                };
                if elem_w == 0 || elem_w > 64 {
                    self.bail("Expr_Call_ports");
                    return None;
                }
                // §13.5: each element fits the declared element type,
                // signing included, exactly like a scalar input formal; a
                // bare resize left `input int d[]` elements unsigned, so
                // `d[0] / 2` of -6 read 2147483645.
                let elem_kind = self.local_kind_of(&p.data_type, Some(&name));
                let mut regs = Vec::with_capacity(items.len());
                for it in items {
                    let crate::ast::expr::AssignmentPatternItem::Ordered(e) = it else {
                        self.bail("Expr_Call_ports");
                        return None;
                    };
                    let slot = self.alloc_reg();
                    let v = self.compile_expr(e, if is_real { 0 } else { elem_w })?;
                    self.emit(Insn::Move(slot, v));
                    if is_real {
                        // §13.3.1: each element CONVERTS to the real
                        // element type, exactly like a scalar real formal.
                        self.emit_to_real(slot);
                    } else {
                        self.emit(Insn::Resize(slot, elem_w));
                        if let Some(k) = elem_kind {
                            self.local_kinds.insert(slot, k);
                            self.emit_local_fit(slot);
                        }
                    }
                    regs.push(slot);
                }
                array_binds.push((
                    p.name.name.clone(),
                    LocalArrayBind {
                        regs,
                        lo: 0,
                        elem_w,
                        is_real,
                        elem_signed: match elem_kind {
                            Some(LocalKind::Int { signed, .. }) if !is_real => Some(signed),
                            _ => None,
                        },
                    },
                ));
                continue;
            }
            if matches!(p.direction, PortDirection::Ref)
                && (matches!(&a.kind, ExprKind::Ident(h) if self.local_var_reg_of(h).is_some())
                    || self.expr_to_signal_id(a).is_none())
            {
                self.bail("Expr_Call_ref_target");
                return None;
            }
            let formal_is_string = self.decl_is_string(&p.data_type, Some(&name));
            let w = if formal_is_string {
                0
            } else {
                self.port_effective_width(&fd.ports, i, Some(&name))
            };
            let formal_is_real = self.decl_is_real(&p.data_type, Some(&name));
            let slot = self.alloc_reg();
            self.record_local_packed_type(slot, &p.data_type);
            if matches!(p.direction, PortDirection::Output) {
                // §13.5.3: an output formal does NOT read the actual; it
                // starts at its type's default and is copied out on return.
                let init = self.type_default_value_in(&p.data_type, w, Some(&name));
                self.emit(Insn::LoadConst(slot, Box::new(init)));
            } else {
                if p.dimensions.is_empty() {
                    self.pattern_ctx = Some(p.data_type.clone());
                }
                let v = self.compile_expr(a, w)?;
                self.emit(Insn::Move(slot, v));
                // §13.5, §6.12.1: a real actual of an integral formal rounds.
                if !formal_is_real && !formal_is_string {
                    self.emit_real_to_int_fit(slot, a, w);
                }
            }
            if w > 0 && !formal_is_real {
                self.emit(Insn::Resize(slot, w));
            }
            if formal_is_real {
                // §13.3.1: an integral actual CONVERTS to the real formal.
                self.emit_to_real(slot);
            }
            if formal_is_string {
                string_formal_names.push(p.name.name.clone());
            }
            if formal_is_real {
                real_formal_names.push(p.name.name.clone());
            }
            if let Some(k) =
                self.local_kind_of(Self::port_effective_type(&fd.ports, i), Some(&name))
            {
                self.local_kinds.insert(slot, k);
                // §13.5: binding an input fits its declared type, including
                // signing before any wider use of the formal.
                self.emit_local_fit(slot);
            }
            binds.push((p.name.name.clone(), (slot, w)));
            if let Some(fields) =
                self.packed_formal_layout(Self::port_effective_type(&fd.ports, i), w)
            {
                struct_binds.push((p.name.name.clone(), (slot, fields)));
            }
            if let Some(ew) = self.decl_elem_width_in(&p.data_type, Some(&name)) {
                if ew > 0 && w > ew && w % ew == 0 {
                    if self.decl_outer_dim_unnormalized(&p.data_type) {
                        self.bail("Expr_Call_packed_outer_dim");
                        return None;
                    }
                    elem_binds.push((p.name.name.clone(), ew));
                }
            }
            if matches!(p.direction, PortDirection::Ref | PortDirection::Output) {
                ref_writes.push((a.clone(), slot, w));
            }
        }
        // Elaboration rewrites an instantiated module's function body to
        // instance-qualified names, so the body assigns `u0.onehot` and reads
        // `u0.c` while these bindings are keyed on the bare spelling. Bind
        // BOTH, or the inlined body's own result variable resolves to no
        // signal and the enclosing statement bails.
        let qpfx = name.rsplit_once('.').map(|(p, _)| p.to_string());
        let saved_locals = std::mem::take(&mut self.local_var_regs);
        let saved_local_strings = std::mem::take(&mut self.local_var_is_string);
        let saved_local_reals = std::mem::take(&mut self.local_var_is_real);
        let saved_local_arrays = std::mem::take(&mut self.local_var_array);
        let saved_local_elems = std::mem::take(&mut self.local_var_elem);
        let saved_local_structs = std::mem::take(&mut self.local_var_struct);
        // §13.4: the body never sees the caller's unrolled loop constants or
        // register banks, as for an inlined task. An unrolled loop variable
        // shadows every same-named name, so a callee's own `for (int s ...)`
        // tested the caller's `s` and never ended, and a module variable `s`
        // read the caller's loop value.
        let saved_banks = std::mem::take(&mut self.local_array_regs);
        let saved_consts = std::mem::take(&mut self.local_const_vars);
        let saved_const_kinds = std::mem::take(&mut self.local_const_kinds);
        let saved_const_binds = std::mem::take(&mut self.const_var_binds);
        let saved_decl_locals = std::mem::take(&mut self.decl_local_regs);
        for (n, sb) in struct_binds {
            if let Some(pfx) = &qpfx {
                self.local_var_struct
                    .insert(format!("{pfx}.{n}"), sb.clone());
            }
            self.local_var_struct.insert(n, sb);
        }
        for (n, ab) in array_binds {
            if let Some(pfx) = &qpfx {
                self.local_var_array
                    .insert(format!("{pfx}.{n}"), ab.clone());
            }
            self.local_var_array.insert(n, ab);
        }
        for (n, b) in binds {
            if let Some(pfx) = &qpfx {
                self.local_var_regs.insert(format!("{pfx}.{n}"), b);
            }
            self.local_var_regs.insert(n, b);
        }
        for (n, ew) in elem_binds {
            if let Some(pfx) = &qpfx {
                self.local_var_elem.insert(format!("{pfx}.{n}"), ew);
            }
            self.local_var_elem.insert(n, ew);
        }
        for n in &string_formal_names {
            if let Some(pfx) = &qpfx {
                self.local_var_is_string.insert(format!("{pfx}.{n}"));
            }
            self.local_var_is_string.insert(n.clone());
        }
        for n in &real_formal_names {
            if let Some(pfx) = &qpfx {
                self.local_var_is_real.insert(format!("{pfx}.{n}"));
            }
            self.local_var_is_real.insert(n.clone());
        }
        if ret_is_real {
            if let Some(pfx) = &qpfx {
                self.local_var_is_real
                    .insert(format!("{}.{}", pfx, fd.name.name.name));
            }
            self.local_var_is_real.insert(fd.name.name.name.clone());
        }
        if ret_is_string {
            if let Some(pfx) = &qpfx {
                self.local_var_is_string
                    .insert(format!("{}.{}", pfx, fd.name.name.name));
            }
            self.local_var_is_string.insert(fd.name.name.name.clone());
        }
        // The function's own name is its return variable (§13.4.1): give it a
        // register too, so a body that assigns it (possibly across several
        // statements) works exactly like the single-assignment form.
        let ret_slot = self.alloc_reg();
        let ret_init = self.type_default_value_in(&fd.return_type, ret_w, Some(&name));
        self.emit(Insn::LoadConst(ret_slot, Box::new(ret_init)));
        if let Some(kind) = self.local_kind_of(&fd.return_type, Some(&name)) {
            self.local_kinds.insert(ret_slot, kind);
        }
        if let Some(pfx) = &qpfx {
            self.local_var_regs
                .insert(format!("{}.{}", pfx, fd.name.name.name), (ret_slot, ret_w));
        }
        self.local_var_regs
            .insert(fd.name.name.name.clone(), (ret_slot, ret_w));
        let callee_pkg = self.inlined_callee_pkg(&name, fd.span, false);
        self.inline_pkg_stack.push(callee_pkg);
        self.inlining_stack.push(name);
        // AST fallback MUST be off inside an inlined body. `emit_fallback`
        // defers a statement to the AST interpreter, which resolves names
        // through the signal tables — but this body's locals (and the return
        // variable) live in REGISTERS that the interpreter cannot see. A
        // deferred statement therefore reads and writes the wrong storage and
        // its effect is silently lost: a pure helper whose accumulator is
        // updated in a `for` loop returned its initial value instead of the
        // sum, with no fallback counted and no diagnostic. If any statement
        // will not compile, the whole inline has to fail so the caller uses
        // the ordinary (correct) call path.
        let saved_fallback = self.allow_ast_fallback;
        self.allow_ast_fallback = false;
        let saved_ret = self.inline_ret;
        let saved_ret_jumps = std::mem::take(&mut self.inline_ret_jumps);
        // §12.8: break/continue belong to loops in this subroutine, never
        // the caller. A failed body can leave unfinished loop frames behind.
        let saved_break_patches = std::mem::take(&mut self.loop_break_patches);
        let saved_continue_patches = std::mem::take(&mut self.loop_continue_patches);
        let saved_named_block_patches = std::mem::take(&mut self.named_block_patches);
        self.inline_ret = Some((Some(ret_slot), ret_w));
        let saved_ret_real = std::mem::replace(&mut self.inline_ret_real, ret_is_real);
        let saved_ret_ty = self.inline_ret_ty.replace(fd.return_type.clone());
        let mut ok = self.compile_pure_body(&items);
        self.inline_ret_ty = saved_ret_ty;
        self.inline_ret_real = saved_ret_real;
        // Early returns land HERE — after the body, before the output-formal
        // copy-out, which §13.5.3 performs on every return path.
        let body_end = self.insns.len() as u32;
        for j in std::mem::take(&mut self.inline_ret_jumps) {
            self.insns[j] = Insn::Jump(body_end);
        }
        self.inline_ret = saved_ret;
        self.inline_ret_jumps = saved_ret_jumps;
        self.loop_break_patches = saved_break_patches;
        self.loop_continue_patches = saved_continue_patches;
        self.named_block_patches = saved_named_block_patches;
        self.allow_ast_fallback = saved_fallback;
        self.inlining_stack.pop();
        self.inline_pkg_stack.pop();
        self.local_var_regs = saved_locals;
        self.local_var_is_string = saved_local_strings;
        self.local_var_is_real = saved_local_reals;
        self.local_var_array = saved_local_arrays;
        self.local_var_elem = saved_local_elems;
        self.local_var_struct = saved_local_structs;
        self.local_array_regs = saved_banks;
        self.local_const_vars = saved_consts;
        self.local_const_kinds = saved_const_kinds;
        self.const_var_binds = saved_const_binds;
        self.decl_local_regs = saved_decl_locals;
        // §13.5.3: copy-out assigns the caller's lvalues after return. Restore
        // its locals and loop constants first, including for indexed targets.
        if ok {
            for (target, value) in output_elements {
                self.emit(Insn::Move(target, value));
            }
            for (target, value, width) in &ref_writes {
                if !self.compile_blocking_target(target, *value, *width) {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            if std::env::var_os("XEZIM_PROBE_INLINE").is_some() {
                eprintln!(
                    "[INLINE-FAIL] fn {} reason={}",
                    fd.name.name.name,
                    self.bail_reason.unwrap_or("unknown")
                );
            }
            return None;
        }
        if ret_w > 0 && !ret_is_real {
            self.emit(Insn::Resize(ret_slot, ret_w));
        }
        Some(ret_slot)
    }

    /// Compile the statements of an inlined pure function. Local `VarDecl`s
    /// become register-backed locals; a `return <expr>` assigns the return
    /// register (only valid as the final statement, which is the shape any
    /// combinational helper uses).
    pub(super) fn untyped_param_effective_dt(
        &self,
        signing: &Option<crate::ast::types::Signing>,
        init: Option<&Expression>,
        span: crate::ast::Span,
    ) -> crate::ast::types::DataType {
        use crate::ast::types::{
            DataType, IntegerVectorType, PackedDimension, RealType, Signing, SimpleType,
        };
        if let Some(e) = init {
            if self.expr_is_real_static(e) {
                return DataType::Real {
                    kind: RealType::Real,
                    span,
                };
            }
            if self.expr_is_string_static(e) {
                return DataType::Simple {
                    kind: SimpleType::String,
                    span,
                };
            }
        }
        let w = self
            .params
            .and_then(|p| init.and_then(|e| crate::compiler::elaborate::untyped_param_width(e, p)))
            .unwrap_or(32)
            .max(1);
        let eff_signing = match signing {
            Some(s) => Some(*s),
            None => match (self.params, init) {
                (Some(p), Some(e)) => Some(
                    if crate::compiler::elaborate::untyped_param_is_signed(e, p) {
                        Signing::Signed
                    } else {
                        Signing::Unsigned
                    },
                ),
                _ => Some(Signing::Signed),
            },
        };
        let int_lit = |val: String| {
            Expression::new(
                ExprKind::Number(crate::ast::expr::NumberLiteral::Integer {
                    size: Some(32),
                    signed: false,
                    base: crate::ast::expr::NumberBase::Decimal,
                    value: val,
                    cached_val: crate::ast::cache::LiteralCache::new(None),
                }),
                span,
            )
        };
        DataType::IntegerVector {
            kind: IntegerVectorType::Logic,
            signing: eff_signing,
            dimensions: vec![PackedDimension::Range {
                left: Box::new(int_lit((w - 1).to_string())),
                right: Box::new(int_lit("0".to_string())),
                span,
            }],
            span,
        }
    }

    pub(super) fn compile_stmt_decl_item(
        &mut self,
        stmt_span: crate::ast::Span,
        data_type: &crate::ast::types::DataType,
        lifetime: &Option<crate::ast::types::Lifetime>,
        decl: &crate::ast::stmt::VarDeclarator,
    ) -> bool {
        if self.decl_is_shortreal(data_type, None) {
            self.bail("shortreal_local");
            return false;
        }

        // §6.18: a local declared through a typedef that carries
        // unpacked dims is a collection too. A queue / unsized
        // array without an initializer takes the local-collection
        // path below; any other shape stays on the AST path.
        if decl.dimensions.is_empty()
            && let crate::ast::types::DataType::TypeReference { name: tn, .. } = data_type
            && let Some(dims) = self.typedef_coll_dims.get(&type_name_key(tn))
        {
            use crate::ast::types::UnpackedDimension as UD;
            if self.method_mode
                && decl.init.is_none()
                && dims.len() == 1
                && matches!(dims[0], UD::Queue { .. } | UD::Unsized(_))
            {
                let synth = Statement::new(
                    StatementKind::VarDecl {
                        data_type: data_type.clone(),
                        lifetime: lifetime.clone(),
                        declarators: vec![decl.clone()],
                    },
                    stmt_span,
                );
                self.emit(Insn::InitLocalColl(Box::new((
                    Arc::new(synth),
                    Arc::from("VarDecl_local_coll"),
                ))));
                self.local_coll_names.insert(decl.name.name.clone());
                // §6.16: `typedef string q_t[$]` elements are
                // strings, as a `string q[$]` local's are.
                if self.typedef_coll_strings.contains(&type_name_key(tn)) {
                    self.local_coll_strings.insert(decl.name.name.clone());
                } else {
                    self.local_coll_strings.remove(&decl.name.name);
                }
                return true;
            }
            self.bail("VarDecl_typedef_array");
            return false;
        }
        if !decl.dimensions.is_empty() {
            use crate::ast::types::UnpackedDimension as UD;
            if self.method_mode
                && decl.init.is_none()
                && decl.dimensions.len() == 1
                && matches!(decl.dimensions[0], UD::Queue { .. } | UD::Unsized(_))
            {
                let synth = Statement::new(
                    StatementKind::VarDecl {
                        data_type: data_type.clone(),
                        lifetime: lifetime.clone(),
                        declarators: vec![decl.clone()],
                    },
                    stmt_span,
                );
                self.emit(Insn::InitLocalColl(Box::new((
                    Arc::new(synth),
                    Arc::from("VarDecl_local_coll"),
                ))));
                self.local_coll_names.insert(decl.name.name.clone());
                if Self::dt_is_string(data_type) {
                    self.local_coll_strings.insert(decl.name.name.clone());
                } else {
                    self.local_coll_strings.remove(&decl.name.name);
                }
                return true;
            }
            // One constant-bounded unpacked dimension, small: a
            // register bank. (`logic [7:0] tmp [0:15]` in an
            // inlined AES shift_rows / rcon table.) Every access
            // must fold to a constant index or the enclosing
            // compile fails and rolls back.
            let bounds = if decl.dimensions.len() == 1 {
                match &decl.dimensions[0] {
                    UD::Range { left, right, .. } => {
                        let l = self.fold_const(left).and_then(|v| v.to_u64());
                        let r = self.fold_const(right).and_then(|v| v.to_u64());
                        match (l, r) {
                            (Some(l), Some(r)) => {
                                let (lo, hi) = (l.min(r) as i64, l.max(r) as i64);
                                Some((lo, (hi - lo + 1) as usize, l > r))
                            }
                            _ => None,
                        }
                    }
                    UD::Expression { expr, .. } => self
                        .fold_const(expr)
                        .and_then(|v| v.to_u64())
                        .filter(|&n| n > 0)
                        .map(|n| (0i64, n as usize, false)),
                    _ => None,
                }
            } else {
                None
            };
            let Some((lo, len, descending)) = bounds.filter(|&(_, n, _)| n <= 64) else {
                self.bail("VarDecl_array");
                return false;
            };
            if decl.init.is_some() {
                self.bail("VarDecl_array_init");
                return false;
            }
            let ew = self.decl_width(data_type);
            let base = self.next_reg;
            for _ in 0..len {
                let slot = self.alloc_reg();
                let init = self.type_default_value(data_type, ew);
                self.emit(Insn::LoadConst(slot, Box::new(init)));
            }
            let Ok(base) = RegId::try_from(base) else {
                self.bail("VarDecl_array");
                return false;
            };
            let element = self.local_kind_of(data_type, None);
            self.local_array_regs.insert(
                decl.name.name.clone(),
                LocalArrayBank {
                    base,
                    width: ew,
                    len,
                    lo,
                    descending,
                    element,
                },
            );
            return true;
        }
        // §6.16 string local: no declared width — bind at width
        // 0 so no Resize ever lands on it, and mark it for the
        // %s / string-assign paths.
        let is_string = self.decl_is_string(data_type, None);
        let decl_width = if is_string {
            0
        } else {
            self.decl_width(data_type)
        };
        // §6.21: a STATIC local persists across calls — its
        // declaration runs ONCE. A declarator WITH an initializer
        // has run-once semantics the block cannot express — bail
        // the whole method to the AST path.
        if self.method_mode
            && matches!(lifetime, Some(crate::ast::types::Lifetime::Static))
        {
            if decl.init.is_some() {
                self.bail("static_local_init");
                return false;
            }
            let slot = self.alloc_reg();
            let is_real = self.decl_is_real(data_type, None);
            let is_string = self.decl_is_string(data_type, None);
            let is_class_handle = self.class_local_names.contains(&decl.name.name);
            let (w, default) = if is_class_handle {
                (0, Value::zero(32))
            } else if is_real {
                (0, Value::from_f64(0.0))
            } else {
                let w = if is_string {
                    0
                } else {
                    self.decl_width(data_type)
                };
                (w, self.type_default_value(data_type, w))
            };
            if is_class_handle {
                self.method_handle_names.insert(decl.name.name.clone());
            } else {
                if is_real {
                    self.local_var_is_real.insert(decl.name.name.clone());
                }
                if is_string {
                    self.local_var_is_string.insert(decl.name.name.clone());
                }
                if let Some(k) = self.local_kind_of(data_type, None) {
                    self.local_kinds.insert(slot, k);
                }
            }
            self.local_var_regs
                .insert(decl.name.name.clone(), (slot, w));
            self.decl_local_regs.insert(decl.name.name.clone());
            self.method_static_locals
                .push((decl.name.name.clone(), slot, default));
            return true;
        }
        // Step 6: a CLASS-typed local holds a heap HANDLE in a register.
        if self.method_mode && self.class_local_names.contains(&decl.name.name) {
            let slot = self.alloc_reg();
            let new_cls = decl
                .init
                .as_ref()
                .filter(|e| Self::is_new_shape(e))
                .and_then(|_| self.new_type_map.get(&decl.name.name))
                .filter(|cls| {
                    matches!(data_type, crate::ast::types::DataType::TypeReference { name, .. }
                        if name.name.name.rsplit("::").next() == Some(cls.as_str()))
                })
                .cloned();
            match &decl.init {
                Some(expr) if new_cls.is_some() => {
                    let cls = new_cls.expect("checked");
                    let Some(value) = self.compile_new_construct(expr, &cls) else {
                        self.bail("VarDecl_init");
                        return false;
                    };
                    self.emit(Insn::Move(slot, value));
                }
                Some(expr) => {
                    let Some(value) = self.compile_expr(expr, 0) else {
                        self.bail("VarDecl_init");
                        return false;
                    };
                    self.emit(Insn::Move(slot, value));
                }
                None => {
                    self.emit(Insn::LoadConst(slot, Box::new(Value::zero(32))));
                }
            }
            if self.method_bound_names.contains(&decl.name.name)
                && self.local_var_regs.contains_key(&decl.name.name)
            {
                self.method_shadowing_regs.insert(slot);
            }
            self.local_var_regs
                .insert(decl.name.name.clone(), (slot, 0));
            self.method_handle_names.insert(decl.name.name.clone());
            self.decl_local_regs.insert(decl.name.name.clone());
            return true;
        }
        // §6.12: a `real` local holds a real, whatever its
        // initializer (or its default, 0.0) is.
        let is_real = self.decl_is_real(data_type, None);
        let kind = if is_string {
            Some(LocalKind::Str)
        } else {
            self.local_kind_of(data_type, None)
        };
        let slot = self.alloc_reg();
        match kind {
            Some(k) => {
                self.local_kinds.insert(slot, k);
            }
            None => {
                self.local_kinds.remove(&slot);
            }
        }
        let width = decl_width;
        match &decl.init {
            Some(expr) => {
                let Some(value) = self.compile_expr(expr, width) else {
                    self.bail("VarDecl_init");
                    return false;
                };
                self.emit(Insn::Move(slot, value));
                // §6.8, §6.12.1: `longint t = r;` rounds a real initializer.
                if matches!(kind, Some(LocalKind::Int { .. })) {
                    self.emit_real_to_int_fit(slot, expr, width);
                }
            }
            None => {
                let value = if is_real {
                    Value::from_f64(0.0)
                } else {
                    self.type_default_value(data_type, width)
                };
                self.emit(Insn::LoadConst(slot, Box::new(value)));
            }
        }
        let name = &decl.name.name;
        if is_real {
            self.emit_to_real(slot);
            self.local_var_is_real.insert(name.clone());
        } else {
            self.local_var_is_real.remove(name);
            if width > 0 {
                self.emit(Insn::Resize(slot, width));
                self.emit_local_fit(slot);
            }
        }
        if is_string {
            self.local_var_is_string.insert(name.clone());
        } else {
            self.local_var_is_string.remove(name);
        }
        self.local_var_regs.insert(name.clone(), (slot, width));
        self.record_local_packed_type(slot, data_type);
        self.decl_local_regs.insert(name.clone());
        true
    }

    fn compile_pure_decl(
        &mut self,
        data_type: &crate::ast::types::DataType,
        d: &crate::ast::stmt::VarDeclarator,
    ) -> bool {
        if self.decl_is_shortreal(data_type, None) {
            self.bail("shortreal_local");
            return false;
        }

        // Step 6: a CLASS-typed local holds a heap HANDLE in a
        // register — default null (0), no resize (handles
        // round-trip untouched), and the name joins
        // `method_handle_names` so `.member` is a heap access.
        if self.class_local_names.contains(&d.name.name) && d.dimensions.is_empty() {
            let slot = self.alloc_reg();
            match &d.init {
                Some(e) => {
                    let Some(v) = self.compile_expr(e, 0) else {
                        return false;
                    };
                    self.emit(Insn::Move(slot, v));
                }
                None => {
                    self.emit(Insn::LoadConst(slot, Box::new(Value::zero(32))));
                }
            }
            self.local_var_regs.insert(d.name.name.clone(), (slot, 0));
            self.method_handle_names.insert(d.name.name.clone());
            return true;
        }
        let is_real = self.decl_is_real(data_type, None);
        if !d.dimensions.is_empty() {
            // A SMALL fixed-shape local array (`real row
            // [0:3]`) — the working-buffer shape every
            // interpolation helper uses — becomes one
            // register per element; a dynamic index selects
            // by compare/branch chain (see
            // `compile_local_array_read`). Anything else
            // (queues, big or non-constant shapes) keeps the
            // AST path.
            if d.init.is_some()
                || !self.bind_local_array(&d.name.name, &d.dimensions, data_type, is_real)
            {
                self.bail("Expr_Call_local_array");
                return false;
            }
            return true;
        }
        let is_string = self.decl_is_string(data_type, None);
        let w = if is_string {
            0
        } else {
            self.decl_width(data_type)
        };
        let slot = self.alloc_reg();
        let kind = self.local_kind_of(data_type, None);
        match &d.init {
            Some(e) => {
                let Some(v) = self.compile_expr(e, w) else {
                    return false;
                };
                self.emit(Insn::Move(slot, v));
                // §6.8, §6.12.1: `longint t = r;` rounds a real initializer.
                if matches!(kind, Some(LocalKind::Int { .. })) {
                    self.emit_real_to_int_fit(slot, e, w);
                }
            }
            None => {
                let init = self.type_default_value(data_type, w);
                self.emit(Insn::LoadConst(slot, Box::new(init)));
            }
        }
        if is_real {
            self.local_var_is_real.insert(d.name.name.clone());
            self.emit_to_real(slot);
        } else if w > 0 {
            self.emit(Insn::Resize(slot, w));
        }
        if is_string {
            self.local_var_is_string.insert(d.name.name.clone());
        }
        if let Some(ew) = self.decl_elem_width(data_type) {
            if ew > 0 && w > ew && w % ew == 0 {
                if self.decl_outer_dim_unnormalized(data_type) {
                    self.bail("Expr_Call_packed_outer_dim");
                    return false;
                }
                self.local_var_elem.insert(d.name.name.clone(), ew);
            }
        }
        if let Some(k) = kind {
            self.local_kinds.insert(slot, k);
            self.emit_local_fit(slot);
        }
        self.local_var_regs.insert(d.name.name.clone(), (slot, w));
        self.record_local_packed_type(slot, data_type);
        true
    }

    fn compile_pure_body(&mut self, items: &[Statement]) -> bool {
        for st in items {
            match &st.kind {
                StatementKind::VarDecl {
                    data_type,
                    declarators,
                    ..
                } => {
                    if self.method_mode && self.class_alias_dynamic_width(data_type) {
                        self.bail("class_alias_runtime_width");
                        return false;
                    }
                    for d in declarators {
                        if !self.compile_pure_decl(data_type, d) {
                            return false;
                        }
                    }
                }
                StatementKind::ParamDecl {
                    data_type,
                    declarators,
                    ..
                } => {
                    if self.method_mode && self.class_alias_dynamic_width(data_type) {
                        self.bail("class_alias_runtime_width");
                        return false;
                    }
                    let untyped = matches!(
                        data_type,
                        crate::ast::types::DataType::Implicit { dimensions, .. }
                            if dimensions.is_empty()
                    );
                    for d in declarators {
                        let eff_dt = if untyped
                            && let crate::ast::types::DataType::Implicit { signing, .. } = data_type
                        {
                            Some(self.untyped_param_effective_dt(
                                signing,
                                d.init.as_deref(),
                                st.span,
                            ))
                        } else {
                            None
                        };
                        let dt = eff_dt.as_ref().unwrap_or(data_type);
                        if !self.compile_pure_decl(dt, d) {
                            return false;
                        }
                    }
                }
                // `return` — early or final — compiles via compile_stmt's
                // Return arm (result move + jump patched by the caller).
                StatementKind::Return(_) => {
                    if !self.compile_stmt(st) {
                        return false;
                    }
                }
                _ => {
                    if !self.compile_stmt(st) {
                        return false;
                    }
                }
            }
        }
        true
    }

    pub(super) fn try_inline_task(&mut self, task_name: &str) -> bool {
        self.try_inline_task_args(task_name, &[])
    }

    /// Inline a non-blocking task call, arguments included. Input formals
    /// bind the actual's value to a register-backed local; output formals get
    /// a default-initialized register and are copied back to the caller's
    /// lvalue on return (§13.5.3 — an output does not read its actual).
    /// Everything is rolled back on failure, so a task the compiler cannot
    /// handle costs nothing and keeps the AST path.
    ///
    /// This is what lets a task-structured FSM compile at all: an AES round
    /// (`sub_bytes(); shift_rows(); mix_columns(); add_round_key(round+1);`)
    /// was ONE interpreted 280µs block per clock because a single call with
    /// an argument bailed the whole always_ff.
    pub(super) fn try_inline_task_args(&mut self, task_name: &str, args: &[Expression]) -> bool {
        if self.inlining_stack.len() >= MAX_INLINE_DEPTH {
            return false;
        }
        if self.inlining_stack.iter().any(|n| n == task_name) {
            return false;
        }
        // §13.4.1: a VOID function called as a statement is a task enable in
        // all but name — inline it through the same machinery. (Non-void
        // returns would need a result variable; those stay on the AST.)
        let (ports, body, lifetime): (
            Vec<crate::ast::decl::FunctionPort>,
            crate::ast::shared_vec::SharedVec<Statement>,
            Option<crate::ast::types::Lifetime>,
        ) = if let Some(td) = self.tasks.and_then(|t| t.get(task_name)) {
            let body = self
                .materialize_deferred_task(task_name, td)
                .unwrap_or_else(|| td.items.clone());
            (td.ports.clone(), body, td.lifetime)
        } else if let Some(fd) = self.functions.and_then(|f| f.get(task_name)) {
            if !matches!(fd.return_type, crate::ast::types::DataType::Void(_)) {
                return false;
            }
            let body = self
                .materialize_deferred_fn(task_name, fd)
                .unwrap_or_else(|| fd.items.clone());
            (fd.ports.clone(), body, fd.lifetime)
        } else {
            return false;
        };
        if self.selects_offset_range_var(&ports, None, &body) {
            return false;
        }
        // §13.3.1: a STATIC subroutine's variables — its locals and output
        // formals — keep their values between calls; an inline gives each
        // call fresh registers. That differs only for a variable whose value
        // can carry from one call to the next (`static_local_carries`): one
        // written whole on every path before it is read behaves the same
        // either way, so the call still inlines. No written lifetime is
        // static; elaboration marks a subroutine of an `automatic` module
        // automatic (§13.3.1).
        if !matches!(lifetime, Some(crate::ast::types::Lifetime::Automatic))
            && (ports.iter().any(|p| {
                !matches!(p.direction, PortDirection::Input)
                    && Self::static_local_carries(&p.name.name, Some(&p.data_type), &body)
            }) || (body.iter().any(Self::stmt_declares) && Self::stmts_carry_static(&body)))
        {
            return false;
        }
        // §6.21: so does a static variable of any subroutine — a local of a
        // subroutine with the default (static) lifetime, or one declared
        // `static` in an automatic subroutine — when its value can be read
        // before the call writes it.
        if Self::body_has_carried_static(
            &body,
            matches!(lifetime, Some(crate::ast::types::Lifetime::Automatic)),
        ) {
            return false;
        }
        if ports.len() != args.len()
            || ports.iter().any(|p| {
                !matches!(p.direction, PortDirection::Input | PortDirection::Output)
                    || !p.dimensions.is_empty()
            })
        {
            return false;
        }
        // Process-FSM mode inlines BLOCKING task bodies too — their waits
        // compile to Wait insns like any other statement, and the register-
        // backed formals live in the process frame across suspensions.
        if !self.allow_waits && body.iter().any(Self::stmt_is_blocking) {
            return false;
        }
        let start = self.insns.len();
        let start_reg = self.next_reg;
        let qpfx = task_name.rsplit_once('.').map(|(p, _)| p.to_string());

        // Evaluate input actuals in the CALLER's scope, before any binding.
        let mut binds: Vec<(String, (RegId, u32))> = Vec::with_capacity(ports.len());
        let mut out_writes: Vec<(Expression, RegId, u32)> = Vec::new();
        let mut ok = true;
        let mut string_formals: Vec<String> = Vec::new();
        let mut real_formals: Vec<String> = Vec::new();
        for (i, (p, a)) in ports.iter().zip(args).enumerate() {
            // §13.5.2: a formal that omits its type inherits the previous
            // formal's (the parser leaves it `Implicit`).
            let eff_dt = Self::port_effective_type(&ports, i);
            // §6.16 string formal: no declared width, so no Resize — a
            // resize would truncate the front of the text.
            let is_string = self.decl_is_string(eff_dt, Some(task_name));
            let is_real = self.decl_is_real(eff_dt, Some(task_name));
            let w = if is_string {
                0
            } else {
                self.port_effective_width(&ports, i, Some(task_name))
            };
            let kind = if is_string {
                Some(LocalKind::Str)
            } else {
                self.local_kind_of(eff_dt, Some(task_name))
            };
            let slot = self.alloc_reg();
            match kind {
                Some(k) => {
                    self.local_kinds.insert(slot, k);
                }
                None => {
                    self.local_kinds.remove(&slot);
                }
            }
            if matches!(p.direction, PortDirection::Output) {
                if is_string {
                    ok = false;
                    break;
                }
                // §13.5.3: an output starts at its type's default.
                let init = if is_real {
                    Value::from_f64(0.0)
                } else {
                    self.type_default_value_in(eff_dt, w, Some(task_name))
                };
                self.emit(Insn::LoadConst(slot, Box::new(init)));
                out_writes.push((a.clone(), slot, w));
            } else {
                match self.compile_expr(a, w) {
                    Some(v) => self.emit(Insn::Move(slot, v)),
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if is_real {
                // §13.3.1: an integral actual CONVERTS to a real formal.
                self.emit_to_real(slot);
                real_formals.push(p.name.name.clone());
            } else if w > 0 {
                self.emit(Insn::Resize(slot, w));
                self.emit_local_fit(slot);
            }
            if is_string {
                string_formals.push(p.name.name.clone());
            }
            binds.push((p.name.name.clone(), (slot, w)));
        }

        if ok {
            // The body sees its own formals and locals and the module scope —
            // never the CALLER's locals, loop constants or register banks
            // (§13: a subroutine body is its own scope). Inheriting them let a
            // body's module-variable `cnt` resolve to a caller local `cnt`.
            let saved_locals = std::mem::take(&mut self.local_var_regs);
            let saved_local_strings = std::mem::take(&mut self.local_var_is_string);
            let saved_local_reals = std::mem::take(&mut self.local_var_is_real);
            let saved_local_elems = std::mem::take(&mut self.local_var_elem);
            let saved_local_arrays = std::mem::take(&mut self.local_var_array);
            let saved_banks = std::mem::take(&mut self.local_array_regs);
            let saved_consts = std::mem::take(&mut self.local_const_vars);
            let saved_const_kinds = std::mem::take(&mut self.local_const_kinds);
            let saved_const_binds = std::mem::take(&mut self.const_var_binds);
            let saved_decl_locals = std::mem::take(&mut self.decl_local_regs);
            let saved_loop_depth = std::mem::replace(&mut self.reg_var_loop_depth, 0);
            // `%m` inside the body names the task: a scope-printing fallback
            // is wrapped in a block carrying the task's own name.
            let leaf = task_name.rsplit('.').next().unwrap_or(task_name);
            let saved_labels = std::mem::replace(
                &mut self.m_labels,
                vec![crate::ast::Identifier {
                    name: leaf.to_string(),
                    span: crate::ast::Span::dummy(),
                }],
            );
            // A body declared in another scope prints that scope's `%m` and
            // runs on its timescale; no fallback may be emitted inside it.
            let foreign = qpfx.as_deref() != self.scope_hint.as_deref();
            self.inline_foreign += foreign as u32;
            // Formals shadow for the body only. The qualified spellings cover
            // elaboration's instance-rewritten bodies, same as pure calls.
            for (n, b) in &binds {
                if let Some(pfx) = &qpfx {
                    self.local_var_regs.insert(format!("{pfx}.{n}"), *b);
                }
                self.local_var_regs.insert(n.clone(), *b);
            }
            for n in &string_formals {
                if let Some(pfx) = &qpfx {
                    self.local_var_is_string.insert(format!("{pfx}.{n}"));
                }
                self.local_var_is_string.insert(n.clone());
            }
            for n in &real_formals {
                if let Some(pfx) = &qpfx {
                    self.local_var_is_real.insert(format!("{pfx}.{n}"));
                }
                self.local_var_is_real.insert(n.clone());
            }
            // A statement the compiler cannot handle falls back to the
            // interpreter carrying the formals and locals it names (see
            // `fb_carry`); one that cannot be carried fails the inline, and
            // the call keeps the ordinary interpreted path.
            let callee_pkg = self
                .tasks
                .and_then(|t| t.get(task_name))
                .and_then(|td| self.inlined_callee_pkg(task_name, td.span, true));
            self.inline_pkg_stack.push(callee_pkg);
            self.inlining_stack.push(task_name.to_string());
            let saved_ret = self.inline_ret;
            let saved_ret_jumps = std::mem::take(&mut self.inline_ret_jumps);
            // Keep the caller's loop fixups out of the task's body, including
            // when a failed lowering leaves unfinished loop frames behind.
            let saved_break_patches = std::mem::take(&mut self.loop_break_patches);
            let saved_continue_patches = std::mem::take(&mut self.loop_continue_patches);
            let saved_named_block_patches = std::mem::take(&mut self.named_block_patches);
            self.inline_ret = Some((None, 0));
            for st in &body {
                if !self.compile_stmt(st) {
                    ok = false;
                    break;
                }
            }
            // `return;` sites jump here, BEFORE the §13.5.3 output copy-out.
            let body_end = self.insns.len() as u32;
            for j in std::mem::take(&mut self.inline_ret_jumps) {
                self.insns[j] = Insn::Jump(body_end);
            }
            self.inline_ret = saved_ret;
            self.inline_ret_jumps = saved_ret_jumps;
            self.loop_break_patches = saved_break_patches;
            self.loop_continue_patches = saved_continue_patches;
            self.named_block_patches = saved_named_block_patches;
            self.inlining_stack.pop();
            self.inline_pkg_stack.pop();
            self.inline_foreign -= foreign as u32;
            self.m_labels = saved_labels;
            self.local_var_regs = saved_locals;
            self.local_var_is_string = saved_local_strings;
            self.local_var_is_real = saved_local_reals;
            self.local_var_elem = saved_local_elems;
            self.local_var_array = saved_local_arrays;
            self.local_array_regs = saved_banks;
            self.local_const_vars = saved_consts;
            self.local_const_kinds = saved_const_kinds;
            self.const_var_binds = saved_const_binds;
            self.decl_local_regs = saved_decl_locals;
            self.reg_var_loop_depth = saved_loop_depth;
            // The copy-out targets are the CALLER's lvalues, resolved in the
            // caller's scope: with the formals still bound, an actual named
            // like its formal (`t1(cyc, o)` into `output int o`) wrote the
            // formal's own register and the caller's `o` never changed.
            if ok {
                for (target, value, width) in &out_writes {
                    if !self.compile_blocking_target(target, *value, *width) {
                        ok = false;
                        break;
                    }
                }
            }
        }
        if !ok {
            if std::env::var_os("XEZIM_PROBE_INLINE").is_some() {
                eprintln!(
                    "[INLINE-FAIL] task {} reason={}",
                    task_name,
                    self.bail_reason.unwrap_or("unknown")
                );
            }
            // A half-emitted body writes REAL signals — roll everything back.
            self.truncate_insns(start);
            self.next_reg = start_reg;
            return false;
        }
        self.tasks_inlined += 1;
        true
    }

    /// Conservative structural test for loop bodies that the NEW for-loop
    /// compilation capabilities (register-backed `for (int i...)` vars and
    /// signal-backed `i++` steps) are allowed to handle. Nested indexing,
    /// member access, and non-assign statements go through addressing paths
    /// whose register/loop-var handling is not yet audited — those loops
    /// keep the old whole-loop AST fallback. Never regresses: these bodies
    /// always fell back before the new capabilities existed.
    /// Is this expression compilable inside a register-backed loop body?
    /// No member access anywhere; everything else resolves through
    /// `compile_expr`'s normal paths.
    fn expr_loop_simple(&self, e: &Expression) -> bool {
        match &e.kind {
            // Packed-struct member reads are lowered to bit slices. Unsupported
            // member shapes still fail strict compilation later, which rolls
            // the whole register-backed loop back to the interpreter.
            ExprKind::MemberAccess { expr, .. } => self.expr_loop_simple(expr),
            ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                self.expr_loop_simple(operand)
            }
            ExprKind::Binary { left, right, .. } => {
                self.expr_loop_simple(left) && self.expr_loop_simple(right)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.expr_loop_simple(condition)
                    && self.expr_loop_simple(then_expr)
                    && self.expr_loop_simple(else_expr)
            }
            ExprKind::Index { expr, index } => {
                self.expr_loop_simple(expr) && self.expr_loop_simple(index)
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                self.expr_loop_simple(expr)
                    && self.expr_loop_simple(left)
                    && self.expr_loop_simple(right)
            }
            ExprKind::SystemCall { args, .. } | ExprKind::Concatenation(args) => {
                args.iter().all(|a| self.expr_loop_simple(a))
            }
            // A call to a function `compile_pure_call` can INLINE is fine:
            // it becomes ordinary expression code with no call at all. This
            // gate used to reject every call, so one helper in a loop body
            // (`vec[i] <= onehot(code[i])` — a ubiquitous RTL shape) dragged
            // the whole loop, and every instance of the enclosing module,
            // onto the AST interpreter. Being optimistic is safe: a body that
            // fails to compile later bails the loop exactly as before,
            // because `StmtFallback` cannot be emitted while
            // `reg_var_loop_depth > 0`.
            ExprKind::Call { func, args } => {
                args.iter().all(|a| self.expr_loop_simple(a))
                    // A PURE string method (.len/.getc/.substr/... on a
                    // string object) lowers to a single StrOp with no call at
                    // all, exactly like an inlinable free function. Optimistic
                    // admission is safe for the same reason: a body that then
                    // fails to compile bails the whole loop, because
                    // StmtFallback cannot be emitted while reg_var_loop_depth
                    // > 0.
                    && (self.call_is_inlinable(func, args)
                        || self.string_method_shape(func, func.span).is_some())
            }
            _ => true,
        }
    }

    /// Would `compile_pure_call` accept this call? Mirrors its admission
    /// tests (single-segment callee, arity, supported scalar formals,
    /// argument-pure body, nothing that suspends) so the loop gate and the
    /// compiler agree on what "call-free after inlining" means.
    fn call_is_inlinable(&self, func: &Expression, args: &[Expression]) -> bool {
        let ExprKind::Ident(h) = &func.kind else {
            return false;
        };
        if h.path.len() != 1 || !h.path[0].selects.is_empty() {
            return false;
        }
        let Some(fd_ref) = self.functions.and_then(|f| f.get(&h.path[0].name.name)) else {
            return false;
        };
        let mut owned_fd;
        let fd = if let Some(items) = self.materialize_deferred_fn(&h.path[0].name.name, fd_ref) {
            owned_fd = fd_ref.clone();
            owned_fd.items = items;
            &owned_fd
        } else {
            fd_ref
        };
        if fd.ports.len() != args.len()
            || fd.ports.iter().any(|p| {
                !matches!(p.direction, PortDirection::Input | PortDirection::Ref)
                    || !p.dimensions.is_empty()
            })
        {
            return false;
        }
        let pfx = h.path[0].name.name.rsplit_once('.').map(|(p, _)| p);
        if !self.fn_is_pure_in(fd, pfx) {
            return false;
        }
        let items: Vec<Statement> = match fd.items.as_slice() {
            [one] => match &one.kind {
                StatementKind::SeqBlock { stmts, .. } => stmts.to_vec(),
                _ => vec![one.clone()],
            },
            other => other.to_vec(),
        };
        !items.iter().any(Self::stmt_is_blocking)
            && !self.selects_offset_range_var(
                &fd.ports,
                Some((fd.name.name.name.as_str(), &fd.return_type)),
                &items,
            )
    }

    /// `q.pop_front()` / `q.pop_back()` on a plain (possibly dotted) name,
    /// under any parentheses.
    fn is_queue_pop_call(e: &Expression) -> bool {
        let mut e = e;
        while let ExprKind::Paren(inner) = &e.kind {
            e = inner;
        }
        let ExprKind::Call { func, args } = &e.kind else {
            return false;
        };
        if !args.is_empty() {
            return false;
        }
        let plain = |h: &crate::ast::expr::HierarchicalIdentifier| {
            h.path.iter().all(|s| s.selects.is_empty())
        };
        let (method, recv_ok) = match &func.kind {
            ExprKind::Ident(h) if h.path.len() >= 2 => {
                (h.path.last().map(|s| s.name.name.as_str()), plain(h))
            }
            ExprKind::MemberAccess { expr, member } => (
                Some(member.name.as_str()),
                matches!(&expr.kind, ExprKind::Ident(h) if plain(h)),
            ),
            _ => (None, false),
        };
        recv_ok && matches!(method, Some("pop_front" | "pop_back"))
    }

    pub(super) fn for_body_is_simple(&self, stmt: &Statement) -> bool {
        let expr_simple = |e: &Expression| -> bool { self.expr_loop_simple(e) };
        let lv_simple = |e: &Expression| -> bool {
            match &e.kind {
                ExprKind::Ident(h) => h.path.iter().all(|s| s.selects.is_empty()),
                ExprKind::Index { expr, index } => {
                    (matches!(&expr.kind, ExprKind::Ident(h)
                        if h.path.iter().all(|s| s.selects.is_empty()))
                        // `a[i][j]` on a 2-D unpacked array — the store arm
                        // lowers it to the same row-major flat index the read
                        // uses.
                        || matches!(&expr.kind, ExprKind::Index { expr: b, index: bi }
                            if matches!(&b.kind, ExprKind::Ident(h)
                                if h.path.iter().all(|s| s.selects.is_empty()))
                                && expr_simple(bi)))
                        && expr_simple(index)
                }
                // `arr[i].m` / `s.m` on a packed struct: the assign arms
                // splice the member's static bit range, so the same two base
                // shapes above are what they can resolve.
                ExprKind::MemberAccess { expr, .. } => match &expr.kind {
                    ExprKind::Ident(h) => h.path.iter().all(|s| s.selects.is_empty()),
                    ExprKind::Index { expr: base, index } => {
                        matches!(&base.kind, ExprKind::Ident(h)
                            if h.path.iter().all(|s| s.selects.is_empty()))
                            && expr_simple(index)
                    }
                    _ => false,
                },
                _ => false,
            }
        };
        fn lv_base_name(e: &Expression) -> Option<&str> {
            match &e.kind {
                ExprKind::Index { expr, .. } => match &expr.kind {
                    ExprKind::Ident(h) => h.path.last().map(|s| s.name.name.as_str()),
                    // 2-D: peel the outer index and ask again, or widening
                    // lv_simple above smuggles `m[i][j] <= m[i][j]+1` past the
                    // alias guard.
                    ExprKind::Index { .. } => lv_base_name(expr),
                    _ => None,
                },
                // See through `.m` so `arr[i].m <= arr[i].m + 1` still reaches
                // the self-read audit below — widening lv_simple without this
                // would let exactly the aliasing case it guards slip through.
                ExprKind::MemberAccess { expr, .. } => lv_base_name(expr),
                _ => None,
            }
        }
        // Full dotted form of an Index lvalue's base ident.
        fn lv_base_full(e: &Expression) -> Option<String> {
            match &e.kind {
                ExprKind::Index { expr, .. } => match &expr.kind {
                    ExprKind::Ident(h) => Some(
                        h.path
                            .iter()
                            .map(|s| s.name.name.as_str())
                            .collect::<Vec<_>>()
                            .join("."),
                    ),
                    ExprKind::Index { .. } => lv_base_full(expr),
                    _ => None,
                },
                ExprKind::MemberAccess { expr, .. } => lv_base_full(expr),
                _ => None,
            }
        }
        // Does the rvalue read leaf `name` through a DIFFERENT dotted form
        // than `full`? Same-form self-reads (`cnt[b] <= cnt[b]+1`) resolve
        // through the identical lookup and cannot skew; the audited hazard
        // was an elaboration-rewritten dotted lvalue paired with a bare read.
        fn expr_reads_name_other_form(e: &Expression, name: &str, full: &str) -> bool {
            match &e.kind {
                ExprKind::Ident(h) => {
                    if h.path.last().is_some_and(|s| s.name.name == name) {
                        let f = h
                            .path
                            .iter()
                            .map(|s| s.name.name.as_str())
                            .collect::<Vec<_>>()
                            .join(".");
                        f != full
                    } else {
                        false
                    }
                }
                ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                    expr_reads_name_other_form(operand, name, full)
                }
                ExprKind::Binary { left, right, .. } => {
                    expr_reads_name_other_form(left, name, full)
                        || expr_reads_name_other_form(right, name, full)
                }
                ExprKind::Conditional {
                    condition,
                    then_expr,
                    else_expr,
                } => {
                    expr_reads_name_other_form(condition, name, full)
                        || expr_reads_name_other_form(then_expr, name, full)
                        || expr_reads_name_other_form(else_expr, name, full)
                }
                ExprKind::Index { expr, index } => {
                    expr_reads_name_other_form(expr, name, full)
                        || expr_reads_name_other_form(index, name, full)
                }
                ExprKind::RangeSelect {
                    expr, left, right, ..
                } => {
                    expr_reads_name_other_form(expr, name, full)
                        || expr_reads_name_other_form(left, name, full)
                        || expr_reads_name_other_form(right, name, full)
                }
                ExprKind::SystemCall { args, .. } | ExprKind::Concatenation(args) => args
                    .iter()
                    .any(|a| expr_reads_name_other_form(a, name, full)),
                _ => false,
            }
        }
        fn expr_reads_name(e: &Expression, name: &str) -> bool {
            match &e.kind {
                ExprKind::Ident(h) => h.path.last().is_some_and(|s| s.name.name == name),
                ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                    expr_reads_name(operand, name)
                }
                ExprKind::Binary { left, right, .. } => {
                    expr_reads_name(left, name) || expr_reads_name(right, name)
                }
                ExprKind::Conditional {
                    condition,
                    then_expr,
                    else_expr,
                } => {
                    expr_reads_name(condition, name)
                        || expr_reads_name(then_expr, name)
                        || expr_reads_name(else_expr, name)
                }
                ExprKind::Index { expr, index } => {
                    expr_reads_name(expr, name) || expr_reads_name(index, name)
                }
                ExprKind::RangeSelect {
                    expr, left, right, ..
                } => {
                    expr_reads_name(expr, name)
                        || expr_reads_name(left, name)
                        || expr_reads_name(right, name)
                }
                ExprKind::SystemCall { args, .. } | ExprKind::Concatenation(args) => {
                    args.iter().any(|a| expr_reads_name(a, name))
                }
                _ => false,
            }
        }
        match &stmt.kind {
            StatementKind::Null => true,
            StatementKind::VarDecl { declarators, .. }
            | StatementKind::ParamDecl { declarators, .. } => declarators
                .iter()
                .all(|decl| decl.init.as_deref().map(&expr_simple).unwrap_or(true)),
            StatementKind::NonblockingAssign { lvalue, rvalue, .. }
            | StatementKind::BlockingAssign { lvalue, rvalue } => {
                // A SELF-READING array update (`ptr[i] <= ptr[i] + 1`) is
                // excluded: in an inlined instance the compiled read and
                // write paths can resolve the array through different
                // aliases (port copy vs local), skewing the pre-edge read.
                // Keep such loops on the audited AST path.
                let self_read_skewed = match (lv_base_name(lvalue), lv_base_full(lvalue)) {
                    (Some(n), Some(f)) => expr_reads_name_other_form(rvalue, n, &f),
                    _ => false,
                };
                let _ = expr_reads_name; // superseded by the form-aware audit
                // §7.10.2: `v[i] = q.pop_front();` — the pop is evaluated by
                // the interpreter into a register (it names no loop
                // variable), the loop and the store stay compiled. Such a
                // drain loop used to run whole in the AST interpreter (#1210).
                let queue_pop = matches!(stmt.kind, StatementKind::BlockingAssign { .. })
                    && Self::is_queue_pop_call(rvalue);
                lv_simple(lvalue) && (expr_simple(rvalue) || queue_pop) && !self_read_skewed
            }
            StatementKind::SeqBlock { stmts, .. } => {
                stmts.iter().all(|st| self.for_body_is_simple(st))
            }
            StatementKind::If {
                condition,
                then_stmt,
                else_stmt,
                ..
            } => {
                expr_simple(condition)
                    && self.for_body_is_simple(then_stmt)
                    && else_stmt
                        .as_ref()
                        .map(|e| self.for_body_is_simple(e))
                        .unwrap_or(true)
            }
            // Case over a simple selector with simple arm bodies. The arb
            // datapath loops a customer profile flagged (86% of wall time in
            // the For_init_vardecl AST fallback, 364µs/execution) are exactly
            // banks×entries nests of case-inside-for; a mid-body construct
            // the compiler still can't handle bails the WHOLE loop as before
            // (StmtFallback emission is forbidden inside reg-var loops), so
            // widening this filter can't smuggle a half-compiled body.
            StatementKind::Case { expr, items, .. } => {
                expr_simple(expr)
                    && items.iter().all(|it| {
                        it.pattern.is_none()
                            && it.patterns.iter().all(expr_simple)
                            && self.for_body_is_simple(&it.stmt)
                    })
            }
            // Nested `for` — both the assign-init and VarDecl-init forms.
            // The per-assign audits (simple lvalue, no member access, no
            // self-reading array update) apply recursively through the body.
            StatementKind::For {
                init,
                condition,
                step,
                body,
            } => {
                let init_ok = init.iter().all(|fi| match fi {
                    crate::ast::stmt::ForInit::Assign { lvalue, rvalue } => {
                        lv_simple(lvalue) && expr_simple(rvalue)
                    }
                    crate::ast::stmt::ForInit::VarDecl { init, .. } => expr_simple(init),
                });
                let step_ok = step.iter().all(expr_simple);
                init_ok
                    && condition.as_ref().map(|c| expr_simple(c)).unwrap_or(true)
                    && step_ok
                    && self.for_body_is_simple(body)
            }
            // A bare side-effecting expression statement — `n++;` lower to
            // `StatementKind::Expr(Unary PostIncr n)` after elaboration, NOT
            // BlockingAssign, so without this arm every counter-increment
            // inside a register-backed `for` body (a ubiquitous
            // string/counting idiom) hit the default and bailed to AST.
            // Arithmetic/incr statements on a plain local/formal are
            // register-safe; the compile phase re-audits and bails to AST if
            // they are not (e.g. a member-array lvalue with a runtime index,
            // which needs StmtFallback, unavailable under reg_var_loop_depth).
            StatementKind::Expr(e) => expr_simple(e),
            _ => false,
        }
    }

    pub(super) fn expr_has_sampled_value_call(e: &Expression) -> bool {
        let sub = |x: &Expression| Self::expr_has_sampled_value_call(x);
        match &e.kind {
            ExprKind::SystemCall { name, args } => {
                matches!(
                    name.as_str(),
                    "$past" | "$rose" | "$fell" | "$stable" | "$changed" | "$sampled"
                ) || args.iter().any(sub)
            }
            ExprKind::Unary { operand, .. } => sub(operand),
            ExprKind::Binary { left, right, .. } => sub(left) || sub(right),
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => sub(condition) || sub(then_expr) || sub(else_expr),
            ExprKind::Paren(i) => sub(i),
            ExprKind::Concatenation(items) => items.iter().any(sub),
            ExprKind::Replication { count, exprs } => sub(count) || exprs.iter().any(sub),
            ExprKind::Call { args, .. } => args.iter().any(sub),
            _ => false,
        }
    }
}
