//! Typed locals, carried static locals and local scans for fallbacks.

use super::*;

impl<'a> BytecodeCompiler<'a> {
    /// The interpreter-visible type of a register-backed local declared with
    /// `dt` (see `LocalKind`), or None when the interpreter would need
    /// metadata the compiled local never registered.
    pub(super) fn local_kind_of(
        &mut self,
        dt: &crate::ast::types::DataType,
        scope: Option<&str>,
    ) -> Option<LocalKind> {
        use crate::ast::types::{
            DataType as D, IntegerAtomType as A, PackedDimension as P, Signing as Sg,
            SimpleType as St,
        };
        // An additional packed range on an alias needs element/range
        // metadata, just like a direct multi-dimensional packed declaration.
        let mut cur = dt;
        let mut owner = scope;
        for _ in 0..64 {
            let D::TypeReference {
                name, dimensions, ..
            } = cur
            else {
                break;
            };
            if !dimensions.is_empty() {
                return None;
            }
            let Some((key, next)) = self.typedef_type_entry(name, owner) else {
                break;
            };
            cur = next;
            owner = Some(key);
        }
        let resolved = self.resolved_decl_type(dt, scope);
        match resolved {
            D::Real { .. } => Some(LocalKind::Real),
            D::Simple {
                kind: St::String, ..
            } => Some(LocalKind::Str),
            D::IntegerAtom { kind, signing, .. } => {
                let width = self.decl_width_in(dt, scope);
                // §6.11: the atoms are signed by default, `time` is not.
                let signed = match signing {
                    Some(Sg::Signed) => true,
                    Some(Sg::Unsigned) => false,
                    None => !matches!(kind, A::Time),
                };
                // §6.11: `integer` and `time` are the 4-state atoms.
                let two_state = !matches!(kind, A::Integer | A::Time);
                Some(LocalKind::Int {
                    width,
                    signed,
                    two_state,
                })
            }
            D::IntegerVector {
                signing,
                dimensions,
                ..
            }
            | D::Implicit {
                signing,
                dimensions,
                ..
            } => {
                let two_state = matches!(
                    resolved,
                    D::IntegerVector {
                        kind: crate::ast::types::IntegerVectorType::Bit,
                        ..
                    }
                );
                let signed = matches!(signing, Some(Sg::Signed));
                // One `[N-1:0]` range at most: the interpreter selects bits
                // of a frame value from bit 0, with no declared range.
                match dimensions.as_slice() {
                    [] => {}
                    [P::Range { right, .. }] => {
                        let right = right.clone();
                        if self.fold_const(&right)?.to_u64()? != 0 {
                            return None;
                        }
                    }
                    _ => return None,
                }
                let width = self.decl_width_in(dt, scope);
                if width == 0 {
                    return None;
                }
                Some(LocalKind::Int {
                    width,
                    signed,
                    two_state,
                })
            }
            _ => None,
        }
    }

    /// §6.8/§10.7: a write to an integral local takes the local's declared
    /// signedness, whatever the right-hand side's was. Width is the caller's
    /// `Resize`; this stamps the flag the register-backed value would
    /// otherwise inherit from its source (`int s; s = u;` with an unsigned
    /// `u` left `s < 0` false).
    pub(super) fn emit_local_fit(&mut self, reg: RegId) {
        if let Some(LocalKind::Int {
            signed, two_state, ..
        }) = self.local_kinds.get(&reg).copied()
        {
            // §6.11.1: a 2-state local drops X/Z — unless the value just
            // stored provably has none (see `stored_value_two_state`).
            let known = two_state && self.stored_value_two_state(reg);
            self.emit(if signed {
                Insn::SetSigned(reg)
            } else {
                Insn::ClearSigned(reg)
            });
            if two_state && !known {
                self.emit(Insn::DropXz(reg));
            }
        }
    }

    /// §6.11.1: `slot` was just bound as a register-backed local and typed
    /// in `local_kinds`; a 2-state one drops any X/Z its initial value
    /// carries, so every 2-state local register holds a 2-state value (the
    /// invariant `stored_value_two_state` relies on).
    pub(super) fn drop_xz_new_two_state_local(&mut self, slot: RegId) {
        if matches!(
            self.local_kinds.get(&slot),
            Some(LocalKind::Int {
                two_state: true,
                ..
            })
        ) && !self.stored_value_two_state(slot)
        {
            self.emit(Insn::DropXz(slot));
        }
    }

    /// Is the value just stored into the 2-state local `reg` (the trailing
    /// `Move`/`LoadConst` into it, before its `Resize`) free of X/Z by
    /// construction? Every store into a 2-state local passes through
    /// `emit_local_fit`, so a register typed as one (`local_kinds`) always
    /// holds a 2-state value; a known constant has none, and an arithmetic,
    /// bitwise or compare op over such operands yields none. Only the
    /// straight-line instructions of the store's own expression are
    /// inspected (no branch can land between them: conditionals compile to
    /// `Select`), and anything not recognised keeps the `DropXz`. A loop
    /// counter's `i = i + 1` / `i++` therefore costs nothing extra.
    fn stored_value_two_state(&self, reg: RegId) -> bool {
        let insns = &self.insns;
        let mut end = insns.len();
        while end > 0
            && matches!(
                insns[end - 1],
                Insn::Resize(r, _) | Insn::SetSigned(r) | Insn::ClearSigned(r) if r == reg
            )
        {
            end -= 1;
        }
        let home = |r: RegId| {
            matches!(
                self.local_kinds.get(&r),
                Some(LocalKind::Int {
                    two_state: true,
                    ..
                })
            )
        };
        // `r` as defined by the instructions before index `upto` (a short
        // window of the same expression).
        let operand = |r: RegId, upto: usize| -> bool {
            if home(r) {
                return true;
            }
            insns[upto.saturating_sub(4)..upto]
                .iter()
                .rev()
                .find_map(|i| match i {
                    Insn::LoadConst(d, v) if *d == r => Some(!v.has_xz() && !v.is_real),
                    i if Self::dest_reg(i) == Some(r)
                        || Self::in_place_reg(i) == Some(r)
                        || matches!(
                            i,
                            Insn::EvalExprFallback(_, d, _) | Insn::QueuePop(d, ..) if *d == r
                        )
                        || matches!(i, Insn::StmtFallback(..))
                        || matches!(i, Insn::InitLocalColl(..)) =>
                    {
                        Some(false)
                    }
                    _ => None,
                })
                .unwrap_or(false)
        };
        let def_known = |at: usize, d: RegId| -> bool {
            match &insns[at] {
                Insn::LoadConst(x, v) if *x == d => !v.has_xz() && !v.is_real,
                Insn::Add(x, a, b)
                | Insn::Sub(x, a, b)
                | Insn::Mul(x, a, b)
                | Insn::BitAnd(x, a, b)
                | Insn::BitOr(x, a, b)
                | Insn::BitXor(x, a, b)
                | Insn::Shl(x, a, b)
                | Insn::Shr(x, a, b)
                | Insn::Eq(x, a, b)
                | Insn::Neq(x, a, b)
                | Insn::Lt(x, a, b)
                | Insn::Leq(x, a, b)
                | Insn::Gt(x, a, b)
                | Insn::Geq(x, a, b)
                    if *x == d =>
                {
                    operand(*a, at) && operand(*b, at)
                }
                Insn::BitNot(x, a) if *x == d => operand(*a, at),
                _ => false,
            }
        };
        let Some(last) = end.checked_sub(1) else {
            return false;
        };
        match &insns[last] {
            Insn::Move(d, src) if *d == reg => {
                home(*src) || (last > 0 && def_known(last - 1, *src))
            }
            _ => def_known(last, reg),
        }
    }

    /// Is any register-backed name in scope, or are we inside an inlined
    /// body? Only then can a fallback need to carry locals (or be unable
    /// to): everywhere else fallbacks are emitted exactly as they always were.
    pub(super) fn fb_live(&self) -> bool {
        // A method body always has register-backed state (`this`, formals,
        // the result cell), and a fallback that could `return` must be
        // refused even where it names none of it.
        self.method_mode
            || !self.local_var_regs.is_empty()
            || !self.local_const_vars.is_empty()
            || !self.local_array_regs.is_empty()
            || !self.local_var_array.is_empty()
            || !self.inlining_stack.is_empty()
    }

    /// The register-backed locals a fallback over the scanned fragment must
    /// carry, or why it cannot be emitted at all. Unrolled loop constants
    /// are loaded into fresh registers here, so call this only once the
    /// fallback is certain to be emitted next.
    pub(super) fn fb_carry(
        &mut self,
        scan: &super::fb_scan::FbScan,
    ) -> Result<FbLocals, &'static str> {
        use super::fb_scan::{DEFER, HIER, NBA, SEL, WRITE};
        if let Some(why) = scan.bad {
            return Err(why);
        }
        // A process-frame import already lives in the interpreter's frame; a
        // carried frame on top would hide it.
        if !self.process_local_names.is_empty() {
            return Err("fb_process_locals");
        }
        if self.inline_foreign > 0 {
            return Err("fb_foreign_inline");
        }
        let is_local = |c: &Self, n: &str| {
            c.local_var_regs.contains_key(n)
                || c.local_const_vars.contains_key(n)
                || c.local_array_regs.contains_key(n)
                || c.local_var_array.contains_key(n)
        };
        // A declaration inside the fragment would land in the carried frame
        // and be copied back over the outer local of the same name.
        if scan.decls.iter().any(|d| is_local(self, d)) {
            return Err("fb_decl_shadows_local");
        }
        let mut consts: Vec<(Arc<str>, Value, LocalKind)> = Vec::new();
        let mut out: Vec<FbLocal> = Vec::new();
        for (name, flags) in &scan.names {
            let flags = *flags;
            if !is_local(self, name) {
                continue;
            }
            let unkinded = self
                .local_var_regs
                .get(name)
                .is_some_and(|(r, _)| !self.local_kinds.contains_key(r));
            let bound = self.method_bound_names.contains(name);
            // The activation records the outer declaration's class, not a
            // nested block's redeclaration of the name.
            if bound
                && self
                    .local_var_regs
                    .get(name)
                    .is_some_and(|(r, _)| self.method_shadowing_regs.contains(r))
            {
                return Err("fb_shadowed_handle");
            }
            // A nested-block handle carries its declared class instead.
            let nested_class = (!bound)
                .then(|| self.method_handle_local_types.get(name))
                .flatten()
                .filter(|c| !c.contains('#'))
                .map(|c| Arc::<str>::from(c.as_str()));
            let handle_local = self.method_mode
                && self.method_handle_names.contains(name)
                && (bound || nested_class.is_some())
                && unkinded;
            // A carried handle is the base of its member accesses: the
            // interpreter reads it from the carried frame.
            if flags & NBA != 0 || (flags & HIER != 0 && !handle_local) {
                return Err("fb_local_by_path");
            }
            if flags & DEFER != 0 {
                return Err("fb_local_deferred");
            }
            // Same precedence as `compile_expr`: an unrolled constant first.
            if let Some(v) = self.local_const_vars.get(name) {
                if flags & WRITE != 0 {
                    return Err("fb_const_local_write");
                }
                let Some(&kind) = self.local_const_kinds.get(name) else {
                    return Err("fb_local_untyped");
                };
                if flags & SEL != 0 && !matches!(kind, LocalKind::Int { .. }) {
                    return Err("fb_local_select");
                }
                consts.push((Arc::from(name.as_str()), v.clone(), kind));
                continue;
            }
            let Some(&(reg, _)) = self.local_var_regs.get(name) else {
                return Err("fb_local_array");
            };
            let kind = match self.local_kinds.get(&reg) {
                Some(&kind) => kind,
                None if handle_local => LocalKind::Handle,
                None => return Err("fb_local_untyped"),
            };
            if flags & SEL != 0
                && (!matches!(kind, LocalKind::Int { .. })
                    || self.local_var_elem.contains_key(name))
            {
                return Err("fb_local_select");
            }
            // An unrolled `foreach` index is folded as a constant elsewhere
            // in the iteration.
            if flags & WRITE != 0 && self.const_var_binds.contains_key(name) {
                return Err("fb_const_local_write");
            }
            out.push(FbLocal {
                name: Arc::from(name.as_str()),
                reg,
                kind,
                via_binding: self.method_mode && self.method_writeback_names.contains(name),
                class: if handle_local { nested_class } else { None },
            });
        }
        for (name, v, kind) in consts {
            let r = self.alloc_reg();
            self.emit(Insn::LoadConst(r, Box::new(v)));
            out.push(FbLocal {
                name,
                reg: r,
                kind,
                via_binding: false,
                class: None,
            });
        }
        Ok(out.into_boxed_slice())
    }

    /// Per-argument "binds an input formal" for a call to the user
    /// subroutine `name` (`FbScan::callee_inputs`): None unless every
    /// declaration the interpreter could resolve the name to — the one in
    /// the block's own scope and the bare one — agrees.
    fn callee_input_dirs(
        tasks: Option<&HashMap<String, TaskDeclaration>>,
        functions: Option<&HashMap<String, FunctionDeclaration>>,
        scope: Option<&str>,
        name: &str,
    ) -> Option<Vec<bool>> {
        let dirs = |key: &str| -> Option<Vec<bool>> {
            let ports = tasks
                .and_then(|t| t.get(key))
                .map(|td| &td.ports)
                .or_else(|| functions.and_then(|f| f.get(key)).map(|fd| &fd.ports))?;
            Some(
                ports
                    .iter()
                    .map(|p| matches!(p.direction, PortDirection::Input))
                    .collect(),
            )
        };
        let scoped = scope.and_then(|sc| dirs(&format!("{sc}.{name}")));
        let bare = dirs(name);
        match (scoped, bare) {
            (Some(a), Some(b)) => (a == b).then_some(a),
            (Some(a), None) | (None, Some(a)) => Some(a),
            (None, None) => None,
        }
    }

    /// Scan a fallback fragment (see `FbScan`), resolving the formal
    /// directions of the subroutines it calls.
    pub(super) fn fb_scan_with<F: FnOnce(&mut super::fb_scan::FbScan<'_>)>(
        &self,
        f: F,
    ) -> super::fb_scan::FbScan<'static> {
        let (tasks, functions) = (self.tasks, self.functions);
        let scope = self.scope_hint.clone();
        let look = move |n: &str| Self::callee_input_dirs(tasks, functions, scope.as_deref(), n);
        let mut scan = super::fb_scan::FbScan::new(Some(&look));
        f(&mut scan);
        scan.detach()
    }

    /// `fb_carry` for a statement fallback.
    pub(super) fn fb_stmt_locals(&mut self, stmt: &Statement) -> Result<FbLocals, &'static str> {
        // A declaration run by the interpreter would live in the carried
        // frame and vanish with it, while the statements after it still
        // need the variable.
        if matches!(
            stmt.kind,
            StatementKind::VarDecl { .. } | StatementKind::ParamDecl { .. }
        ) {
            return Err("fb_decl_escapes");
        }
        let scan = self.fb_scan_with(|s| s.stmt(stmt));
        // Inside an inlined body `%m` must name the task: a scope-printing
        // fallback is wrapped in the task's name (see `m_labels`), which a
        // `disable` in the same statement rules out.
        if !self.inlining_stack.is_empty()
            && Self::stmt_names_scope(stmt).is_none()
            && Self::stmt_prints_scope(stmt)
        {
            return Err("fb_inline_scope_name");
        }
        self.fb_carry(&scan)
    }

    /// §6.21, §13.3.1: does a static subroutine body, or any block nested in
    /// it, declare a variable whose value can carry from one call to the next?
    pub(super) fn stmts_carry_static(stmts: &[Statement]) -> bool {
        fn nested(s: &Statement) -> bool {
            match &s.kind {
                StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                    BytecodeCompiler::stmts_carry_static(stmts)
                }
                StatementKind::If {
                    then_stmt,
                    else_stmt,
                    ..
                } => nested(then_stmt) || else_stmt.as_ref().is_some_and(|e| nested(e)),
                StatementKind::Case { items, .. } => items.iter().any(|it| nested(&it.stmt)),
                StatementKind::For { body, .. }
                | StatementKind::Foreach { body, .. }
                | StatementKind::While { body, .. }
                | StatementKind::DoWhile { body, .. }
                | StatementKind::Repeat { body, .. }
                | StatementKind::Forever { body }
                | StatementKind::TimingControl { stmt: body, .. }
                | StatementKind::Wait { stmt: body, .. } => nested(body),
                _ => false,
            }
        }
        Self::block_has_carried_static(stmts, None) || stmts.iter().any(nested)
    }

    /// §6.21: does a process block declare a static local whose value can
    /// carry from one activation to the next (see `static_local_carries`)?
    pub(super) fn block_has_carried_static(stmts: &[Statement], caller: Option<&Self>) -> bool {
        stmts.iter().enumerate().any(|(i, s)| match &s.kind {
            StatementKind::VarDecl {
                lifetime,
                declarators,
                data_type,
            } if !matches!(lifetime, Some(crate::ast::types::Lifetime::Automatic)) => {
                // An initializer without an explicit `static` runs at every
                // entry (see the interpreter's VarDecl).
                let explicit = matches!(lifetime, Some(crate::ast::types::Lifetime::Static));
                declarators.iter().any(|d| match &d.init {
                    Some(_) => explicit,
                    None => {
                        let rest = &stmts[i + 1..];
                        !caller.is_some_and(|c| c.first_call_copies_output(&d.name.name, rest))
                            && Self::static_local_carries(&d.name.name, Some(data_type), rest)
                    }
                })
            }
            _ => false,
        })
    }

    /// §13.5.1: a function copies its output back before the next statement.
    /// A whole local used solely as that output cannot carry an old value
    /// into the rest of the block. Only simple positional actuals prove it.
    fn first_call_copies_output(&self, local: &str, rest: &[Statement]) -> bool {
        let Some(Statement {
            kind: StatementKind::Expr(call),
            ..
        }) = rest.first()
        else {
            return false;
        };
        let ExprKind::Call { func, args } = &call.kind else {
            return false;
        };
        let ExprKind::Ident(h) = &func.kind else {
            return false;
        };
        if h.root.is_some() || h.path.iter().any(|seg| !seg.selects.is_empty()) {
            return false;
        }
        let name = Self::hier_raw_name(h);
        let Some(fd) = self.functions.and_then(|f| f.get(&name)) else {
            return false;
        };
        if fd.ports.len() != args.len()
            || !matches!(fd.return_type, crate::ast::types::DataType::Void(_))
        {
            return false;
        }
        let mut written = false;
        for (port, actual) in fd.ports.iter().zip(args) {
            match &actual.kind {
                ExprKind::Ident(h)
                    if h.root.is_none() && h.path.len() == 1 && h.path[0].selects.is_empty() =>
                {
                    if h.path[0].name.name == local {
                        if port.direction != PortDirection::Output {
                            return false;
                        }
                        written = true;
                    }
                }
                ExprKind::Number(_) => {}
                _ => return false,
            }
        }
        written
    }

    /// §6.21: does a subroutine body declare, at any depth, a static
    /// variable whose value can carry from one call to the next? `auto` is
    /// the subroutine's lifetime: in an automatic one only a variable
    /// declared `static` is static.
    pub(super) fn body_has_carried_static(stmts: &[Statement], auto: bool) -> bool {
        use crate::ast::types::Lifetime;
        fn nested(s: &Statement, auto: bool) -> bool {
            match &s.kind {
                StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                    BytecodeCompiler::body_has_carried_static(stmts, auto)
                }
                StatementKind::If {
                    then_stmt,
                    else_stmt,
                    ..
                } => nested(then_stmt, auto) || else_stmt.as_ref().is_some_and(|e| nested(e, auto)),
                StatementKind::Case { items, .. } => items.iter().any(|it| nested(&it.stmt, auto)),
                StatementKind::For { body, .. }
                | StatementKind::Foreach { body, .. }
                | StatementKind::While { body, .. }
                | StatementKind::DoWhile { body, .. }
                | StatementKind::Repeat { body, .. }
                | StatementKind::Forever { body }
                | StatementKind::TimingControl { stmt: body, .. }
                | StatementKind::Wait { stmt: body, .. } => nested(body, auto),
                _ => false,
            }
        }
        stmts.iter().enumerate().any(|(i, s)| match &s.kind {
            StatementKind::VarDecl {
                lifetime,
                declarators,
                data_type,
            } => {
                let explicit = matches!(lifetime, Some(Lifetime::Static));
                let is_static = explicit || (!auto && lifetime.is_none());
                is_static
                    && declarators.iter().any(|d| {
                        // Hoisted static cells are hierarchically observable
                        // even when this call writes them before reading.
                        (explicit && d.name.name.contains('.'))
                            || match &d.init {
                                Some(_) => explicit,
                                None => Self::static_local_carries(
                                    &d.name.name,
                                    Some(data_type),
                                    &stmts[i + 1..],
                                ),
                            }
                    })
            }
            _ => nested(s, auto),
        })
    }

    /// §6.21: can a STATIC variable `name`, declared just before `rest`,
    /// carry a value from one execution of its declaration to the next?
    /// Not when every path writes it whole (`t = e`, `for (t = e; ...)`,
    /// both arms of an `if`, every arm of a `case` with a default) before
    /// anything reads it — a loop body or a branch that writes it first
    /// only may write it — giving it a fresh value on every entry is then
    /// unobservable. So are constant selects that, one after another and
    /// none of them reading it, together write every bit of a vector `dt`
    /// declares with constant bounds (`n[0] = a[7]; ... n[7] = a[0];`).
    /// Conservative: any other first use carries.
    pub(crate) fn static_local_carries(
        name: &str,
        dt: Option<&crate::ast::types::DataType>,
        rest: &[Statement],
    ) -> bool {
        /// The labels of a vector not yet written by the constant selects
        /// met so far, on the straight-line path from the declaration.
        struct Cover {
            /// Lowest and highest label.
            lo: i64,
            hi: i64,
            /// One flag per label from `lo`: written yet.
            done: Vec<bool>,
            /// Labels still to write.
            left: usize,
        }
        impl Cover {
            /// §7.4.1: the labels of a vector declared with one packed
            /// dimension of constant bounds, or of an integer atom.
            fn of(dt: &crate::ast::types::DataType) -> Option<Cover> {
                use crate::ast::types::{DataType, PackedDimension};
                let (lo, hi) = match dt {
                    DataType::IntegerVector { dimensions, .. }
                    | DataType::Implicit { dimensions, .. } => {
                        let [PackedDimension::Range { left, right, .. }] = dimensions.as_slice()
                        else {
                            return None;
                        };
                        let l = crate::compiler::elaborate::const_eval_i64_with_params(left, None)?;
                        let r =
                            crate::compiler::elaborate::const_eval_i64_with_params(right, None)?;
                        (l.min(r), l.max(r))
                    }
                    DataType::IntegerAtom { .. } => {
                        let w = crate::compiler::elaborate::resolve_type_width(dt, None, None);
                        (0, i64::from(w) - 1)
                    }
                    _ => return None,
                };
                // A bound in the millions is no hand-written selects' work.
                let n = usize::try_from(hi.checked_sub(lo)?.checked_add(1)?)
                    .ok()
                    .filter(|&n| (1..=1 << 16).contains(&n))?;
                Some(Cover {
                    lo,
                    hi,
                    done: vec![false; n],
                    left: n,
                })
            }
            /// Note labels `a..=b` written (clipped to the vector); true
            /// once every label is.
            fn write(&mut self, a: i64, b: i64) -> bool {
                for l in a.min(b).max(self.lo)..=a.max(b).min(self.hi) {
                    let slot = &mut self.done[(l - self.lo) as usize];
                    if !*slot {
                        *slot = true;
                        self.left -= 1;
                    }
                }
                self.left == 0
            }
        }
        /// §11.5.1: the labels a select of `name` with constant bounds
        /// writes (`n[3]`, `n[7:4]`, `n[4 +: 4]`, `n[7 -: 4]`).
        fn const_select(name: &str, lv: &Expression) -> Option<(i64, i64)> {
            let k =
                |e: &Expression| crate::compiler::elaborate::const_eval_i64_with_params(e, None);
            let (base, a, b) = match &lv.kind {
                ExprKind::Index { expr, index } => {
                    let i = k(index)?;
                    (expr, i, i)
                }
                ExprKind::RangeSelect {
                    expr,
                    kind,
                    left,
                    right,
                } => {
                    let (a, n) = (k(left)?, k(right)?);
                    match kind {
                        RangeKind::Constant => (expr, a, n),
                        RangeKind::IndexedUp if n > 0 => (expr, a, a.checked_add(n - 1)?),
                        RangeKind::IndexedDown if n > 0 => (expr, a.checked_sub(n - 1)?, a),
                        _ => return None,
                    }
                }
                _ => return None,
            };
            whole(name, base).then_some((a, b))
        }
        fn names(name: &str, f: &dyn Fn(&mut super::fb_scan::FbScan)) -> bool {
            let mut sc = super::fb_scan::FbScan::new(None);
            f(&mut sc);
            sc.bad.is_some() || sc.names.iter().any(|(n, _)| n == name)
        }
        fn whole(name: &str, lv: &Expression) -> bool {
            matches!(&lv.kind, ExprKind::Ident(h)
                if h.root.is_none()
                    && h.path.len() == 1
                    && h.path[0].selects.is_empty()
                    && h.path[0].name.name == name)
        }
        /// How a statement first uses the variable.
        #[derive(Clone, Copy, PartialEq)]
        enum Use {
            /// Not at all.
            None,
            /// Writes it whole before anything reads it.
            Write,
            /// May write it (a loop body, one branch), never reads it first.
            Maybe,
            /// May read it (or write part of it) first.
            Read,
        }
        fn branches(us: &[Use], all_paths: bool) -> Use {
            if us.contains(&Use::Read) {
                Use::Read
            } else if all_paths && us.iter().all(|u| *u == Use::Write) {
                Use::Write
            } else if us.iter().all(|u| *u == Use::None) {
                Use::None
            } else {
                Use::Maybe
            }
        }
        /// A loop body, run zero or more times after `control` is read.
        fn looped(name: &str, control_reads: bool, body: &Statement) -> Use {
            if control_reads {
                return Use::Read;
            }
            match first_use(name, body, None) {
                Use::Read => Use::Read,
                _ => Use::Maybe,
            }
        }
        /// `cover`: the labels left to write, for a statement on the
        /// straight-line path (None inside a branch or a loop, whose writes
        /// may not run).
        fn first_use(name: &str, s: &Statement, cover: Option<&mut Cover>) -> Use {
            let reads = |e: &Expression| names(name, &|sc| sc.expr(e));
            // Control transfers the scan refuses on their own.
            match &s.kind {
                StatementKind::Break | StatementKind::Continue | StatementKind::Return(None) => {
                    return Use::None;
                }
                StatementKind::Return(Some(e)) => {
                    return if reads(e) { Use::Read } else { Use::None };
                }
                _ => {}
            }
            // A wait (refused by the scan, as it suspends): its control,
            // then its statement.
            let control_reads = |c: &crate::ast::stmt::TimingControl| -> bool {
                use crate::ast::stmt::{EventControl as EC, TimingControl as TC};
                match c {
                    TC::Delay(e) => reads(e),
                    TC::Event(EC::Star | EC::ParenStar) => true,
                    TC::Event(EC::Identifier(id)) => id.name == name,
                    TC::Event(EC::HierIdentifier(e)) => reads(e),
                    TC::Event(EC::EventExpr(es)) => es
                        .iter()
                        .any(|ee| reads(&ee.expr) || ee.iff.as_ref().is_some_and(|i| reads(i))),
                }
            };
            match &s.kind {
                StatementKind::TimingControl { control, stmt } => {
                    return if control_reads(control) {
                        Use::Read
                    } else {
                        first_use(name, stmt, cover)
                    };
                }
                StatementKind::Wait { condition, stmt } => {
                    return if reads(condition) {
                        Use::Read
                    } else {
                        first_use(name, stmt, cover)
                    };
                }
                StatementKind::Null => return Use::None,
                _ => {}
            }
            if !names(name, &|sc| sc.stmt(s)) {
                return Use::None;
            }
            match &s.kind {
                StatementKind::BlockingAssign { lvalue, rvalue } => {
                    if whole(name, lvalue) && !reads(rvalue) {
                        Use::Write
                    } else if let Some(cover) = cover
                        && !reads(rvalue)
                        && let Some((a, b)) = const_select(name, lvalue)
                    {
                        // Part of it, read by nothing yet: the whole of it
                        // once every label is written.
                        if cover.write(a, b) {
                            Use::Write
                        } else {
                            Use::None
                        }
                    } else {
                        Use::Read
                    }
                }
                StatementKind::SeqBlock { stmts, .. } => {
                    // A block redeclaring the name is another variable:
                    // judge conservatively.
                    let redeclares = stmts.iter().any(|c| {
                        matches!(&c.kind, StatementKind::VarDecl { declarators, .. } | StatementKind::ParamDecl { declarators, .. }
                            if declarators.iter().any(|d| d.name.name == name))
                    });
                    if redeclares {
                        Use::Read
                    } else {
                        seq(name, stmts, cover)
                    }
                }
                StatementKind::If {
                    condition,
                    then_stmt,
                    else_stmt,
                    ..
                } => {
                    if reads(condition) {
                        return Use::Read;
                    }
                    let t = first_use(name, then_stmt, None);
                    match else_stmt {
                        Some(el) => branches(&[t, first_use(name, el, None)], true),
                        None => branches(&[t, Use::None], false),
                    }
                }
                StatementKind::Case { expr, items, .. } => {
                    if reads(expr)
                        || items
                            .iter()
                            .any(|it| it.pattern.is_some() || it.patterns.iter().any(|p| reads(p)))
                    {
                        return Use::Read;
                    }
                    let us: Vec<Use> = items
                        .iter()
                        .map(|it| first_use(name, &it.stmt, None))
                        .collect();
                    branches(&us, items.iter().any(|it| it.is_default))
                }
                StatementKind::For {
                    init,
                    condition,
                    step,
                    body,
                } => {
                    if let Some(crate::ast::stmt::ForInit::Assign { lvalue, rvalue }) = init.first()
                        && whole(name, lvalue)
                    {
                        return if reads(rvalue) { Use::Read } else { Use::Write };
                    }
                    let control = init.iter().any(|fi| match fi {
                        crate::ast::stmt::ForInit::Assign { lvalue, rvalue } => {
                            reads(lvalue) || reads(rvalue)
                        }
                        crate::ast::stmt::ForInit::VarDecl { name: n, init, .. } => {
                            n.name == name || reads(init)
                        }
                    }) || condition.as_ref().is_some_and(|c| reads(c))
                        || step.iter().any(|e| reads(e));
                    looped(name, control, body)
                }
                StatementKind::While { condition, body } => looped(name, reads(condition), body),
                StatementKind::Repeat { count, body } => looped(name, reads(count), body),
                StatementKind::Foreach { array, vars, body } => looped(
                    name,
                    reads(array) || vars.iter().flatten().any(|v| v.name == name),
                    body,
                ),
                StatementKind::DoWhile { body, condition } => match first_use(name, body, None) {
                    Use::Write => Use::Write,
                    Use::Read => Use::Read,
                    _ if reads(condition) => Use::Read,
                    _ => Use::Maybe,
                },
                _ => Use::Read,
            }
        }
        fn seq(name: &str, stmts: &[Statement], mut cover: Option<&mut Cover>) -> Use {
            let mut acc = Use::None;
            for s in stmts {
                match first_use(name, s, cover.as_deref_mut()) {
                    Use::None => {}
                    Use::Maybe => acc = Use::Maybe,
                    u => return u,
                }
            }
            acc
        }
        let mut cover = dt.and_then(Cover::of);
        seq(name, rest, cover.as_mut()) == Use::Read
    }
}
