//! Statement compilation.

use super::*;

impl<'a> BytecodeCompiler<'a> {
    /// Compile a statement. Returns true on success.
    /// When `allow_ast_fallback` is set, any nested failure rolls back and
    /// emits a single `StmtFallback` for the whole statement.
    /// Count code coverage counter `c` first (an instrumented continuous
    /// assignment's entry).
    pub fn emit_cov_hit(&mut self, c: u32) {
        self.emit(Insn::CovHit(c));
    }

    /// `$__xz_covc(<n>, cond)`: the condition's register, with counter `n`
    /// counted when it is true and `n + 1` when it is false (neither for
    /// x/z).
    pub(super) fn compile_cov_cond(&mut self, args: &[Expression]) -> Option<RegId> {
        let c = args
            .first()
            .and_then(crate::compiler::simulator::cov_marker_id)?;
        let r = self.compile_expr(args.get(1)?, 0)?;
        let br = self.insns.len();
        self.emit(Insn::BranchIfFalse(r, 0));
        self.emit(Insn::CovHit(c));
        let jump = self.insns.len();
        self.emit(Insn::Jump(0));
        let not_true = self.insns.len() as u32;
        self.insns[br] = Insn::BranchIfFalse(r, not_true);
        let bz = self.insns.len();
        self.emit(Insn::BranchUnlessZero(r, 0));
        self.emit(Insn::CovHit(c + 1));
        let end = self.insns.len() as u32;
        self.insns[jump] = Insn::Jump(end);
        self.insns[bz] = Insn::BranchUnlessZero(r, end);
        Some(r)
    }

    pub fn compile_stmt(&mut self, stmt: &Statement) -> bool {
        // §6.21: a block-local declaration that SHADOWS a module signal needs
        // the whole enclosing block interpreted as one unit — the AST path
        // pushes a shadow frame for the block's duration, which per-statement
        // StmtFallback insns cannot reproduce (the local would clobber the
        // module variable). Failing WITHOUT fallback here makes the enclosing
        // SeqBlock's own wrapper roll back and emit a single whole-block
        // StmtFallback instead.
        if let StatementKind::VarDecl { declarators, .. }
        | StatementKind::ParamDecl { declarators, .. } = &stmt.kind
        {
            if declarators.iter().any(|d| {
                self.signal_name_to_id.contains_key(d.name.name.as_str())
                    || self.process_local_names.contains(d.name.name.as_str())
            }) {
                self.bail("VarDecl_shadows_signal");
                return false;
            }
        }
        // §7.4.1, §11.5.1: a block local declared with a packed range other
        // than `[N-1:0]` is selected by its labels, which a compiled select
        // does not place (`t[0:3]` of `reg [0:7] t` read the low nibble).
        // Failing without a fallback keeps the whole block on the
        // interpreter, which runs the declaration. An inlined subroutine's
        // locals were checked when it was admitted (`selects_offset_range_var`).
        if self.inlining_stack.is_empty()
            && let StatementKind::VarDecl {
                data_type,
                declarators,
                ..
            } = &stmt.kind
            && declarators.iter().any(|d| d.dimensions.is_empty())
            && self.decl_has_offset_range(data_type)
        {
            self.bail("VarDecl_offset_range");
            return false;
        }
        let start = self.insns.len();
        let start_reg = self.next_reg;
        let saved_reason = self.bail_reason;
        let saved_overflow = self.register_overflow;
        let loop_depth = (
            self.loop_break_patches.len(),
            self.loop_continue_patches.len(),
        );
        self.bail_reason = None;
        self.register_overflow = false;
        let strict_ok = self.compile_stmt_strict(stmt);
        if strict_ok && !self.register_overflow {
            self.bail_reason = saved_reason;
            self.register_overflow = saved_overflow;
            return true;
        }
        // A loop that bailed part-way may leave its own patch frames pushed.
        self.loop_break_patches.truncate(loop_depth.0);
        self.loop_continue_patches.truncate(loop_depth.1);
        if self.register_overflow {
            self.bail("bytecode_register_limit");
        }
        // Same guard as emit_fallback: inside a loop whose counter lives in
        // a VM register, a per-statement fallback reads the loop var as a
        // (non-existent) SIGNAL — `wr[i] = req.vld` inside `for (int i;…)`
        // silently indexed with x and wrote NOTHING, 16 times per fire,
        // while the entry reported success. Fail the statement instead so
        // the whole loop (or block) rolls back to one AST-interpreted unit
        // where the loop var is a real interpreter local.
        if self.allow_ast_fallback {
            let reason = self
                .bail_reason
                .unwrap_or_else(|| Self::stmt_kind_label(stmt));
            self.truncate_insns(start);
            self.next_reg = start_reg;
            let locals = if self.fb_live() {
                self.fb_stmt_locals(stmt)
            } else {
                Ok(FbLocals::default())
            };
            match locals {
                Ok(locals) => {
                    self.trace_fallback_site_carrying(reason, stmt.span, "stmt-whole", &locals);
                    self.push_stmt_fallback(stmt, reason, locals);
                    self.bail_reason = saved_reason;
                    self.register_overflow = saved_overflow;
                    return true;
                }
                Err(why) => {
                    self.trace_fallback_site(why, stmt.span, "block-bail(local-regs)");
                }
            }
        }
        self.register_overflow = saved_overflow;
        false
    }

    /// §7.2: `a = b` / `a <= b` between WHOLE unpacked structs. Each member
    /// is its own signal and the container only carries the width, so a
    /// plain signal copy moved a container nobody reads and left every
    /// member of `a` unchanged (`always_comb l = t;` kept `l` at its
    /// default while the identical `initial` statement worked). Copies
    /// member by member when both sides are struct variables with the same
    /// members. Some(false) for any other source, or when a name may be a
    /// same-named struct of an inner scope, so the statement takes the
    /// interpreter, which already copies member-wise; None when `a` is not a
    /// struct variable.
    /// True when `name` is a register-bound local, a formal or a local
    /// collection of the code being compiled, or a member of one (`name.m`).
    fn names_local(&self, name: &str) -> bool {
        let member = format!("{name}.");
        self.local_var_regs
            .keys()
            .any(|k| k == name || k.starts_with(&member))
            || self.local_array_regs.contains_key(name)
            || self.local_coll_names.contains(name)
            || self.decl_local_regs.contains(name)
            || self.process_local_names.contains(name)
    }

    fn compile_unpacked_struct_copy(&mut self, stmt: &Statement) -> Option<bool> {
        let map = self.unpacked_structs?;
        let (lvalue, rvalue, nb_delay) = match &stmt.kind {
            StatementKind::BlockingAssign { lvalue, rvalue } => (lvalue, rvalue, None),
            StatementKind::NonblockingAssign {
                lvalue,
                delay,
                rvalue,
            } => (lvalue, rvalue, Some(delay)),
            _ => return None,
        };
        // §10.4.2: parentheses preserve the struct value and its leaves.
        let mut rvalue = rvalue;
        while let ExprKind::Paren(inner) = &rvalue.kind {
            rvalue = inner;
        }
        // `lookup_signal_id` falls back from `<scope>.<n>` to outer names. A
        // struct declared in an inner scope may have member leaves but no
        // container signal, so an outer hit can be an unrelated struct of
        // the same name: give up on it when an inner scope has the member
        // leaf (`Some(None)`).
        let id_of = |e: &Expression| match &e.kind {
            ExprKind::Ident(h) if h.path.iter().all(|s| s.selects.is_empty()) => {
                let id = self.lookup_signal_id(h)?;
                // §13.4.2, §23.9: a local or formal of an inlined subroutine
                // (its members bound as `r.a` registers) hides a module
                // struct of the same name, which the lookup found.
                if map.contains_key(&id) && self.names_local(&h.path[0].name.name) {
                    return Some(None);
                }
                if let (Some(scope), Some((path, leaf))) = (
                    self.scope_hint.as_deref(),
                    map.get(&id).and_then(|v| v.first()),
                ) {
                    let raw = Self::hier_raw_cow(h);
                    let shadowed = scope
                        .match_indices('.')
                        .map(|(i, _)| i)
                        .chain([scope.len()])
                        .any(|end| {
                            let key = format!("{}.{raw}.{path}", &scope[..end]);
                            key != *leaf && self.signal_name_to_id.get(key.as_str()).is_some()
                        });
                    if shadowed {
                        return Some(None);
                    }
                }
                Some(Some(id))
            }
            _ => None,
        };
        let Some(dst) = id_of(lvalue)? else {
            return Some(false);
        };
        let dst = map.get(&dst)?;
        // §7.2 / §13.5: project an automatic scalar-member constructor to
        // registers, then scatter. The destination may also be an actual.
        if nb_delay.is_none()
            && let ExprKind::Call { func, args } = &rvalue.kind
        {
            let ExprKind::Ident(h) = &func.kind else { return Some(false); };
            let [segment] = h.path.as_slice() else { return Some(false); };
            let constructor = self.scoped_callee_name(segment.name.name.clone())
                .and_then(|name| self.functions?.get(&name));
            if !constructor.is_some_and(|f| super::struct_sra::projectable_constructor(f, args)) {
                return Some(false);
            }
            let leaves: Option<Vec<_>> = dst.iter().map(|(member, leaf)| {
                let id = self.signal_name_to_id.get(leaf.as_str()).copied()?;
                Some((member.clone(), id, *self.signal_widths.get(id)?))
            }).collect();
            let Some(leaves) = leaves else { return Some(false); };
            let mut writes = Vec::with_capacity(leaves.len());
            for (member, id, width) in leaves {
                let Some(value) = self.compile_struct_member_call(func, args, &member) else {
                    return Some(false);
                };
                writes.push(Insn::BlockingAssign(as_sig_id(id), value, width));
            }
            for insn in writes {
                self.emit(insn);
            }
            return Some(true);
        }
        let Some(src) = id_of(rvalue).flatten().and_then(|id| map.get(&id)) else {
            return Some(false);
        };
        if dst.is_empty() || dst.len() != src.len() || dst.iter().zip(src).any(|(d, s)| d.0 != s.0)
        {
            return Some(false);
        }
        let leaf = |name: &str, span| {
            Expression::new(
                ExprKind::Ident(HierarchicalIdentifier {
                    root: None,
                    path: vec![HierPathSegment {
                        name: crate::ast::Identifier {
                            name: name.to_string(),
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
        };
        let pairs: Vec<(String, String)> = dst
            .iter()
            .zip(src)
            .map(|(d, s)| (d.1.clone(), s.1.clone()))
            .collect();
        for (d, s) in pairs {
            let (l, r) = (leaf(&d, lvalue.span), leaf(&s, rvalue.span));
            let kind = match nb_delay {
                None => StatementKind::BlockingAssign {
                    lvalue: l,
                    rvalue: r,
                },
                Some(delay) => StatementKind::NonblockingAssign {
                    lvalue: l,
                    delay: delay.clone(),
                    rvalue: r,
                },
            };
            if !self.compile_stmt_strict(&Statement::new(kind, stmt.span)) {
                return Some(false);
            }
        }
        Some(true)
    }

    /// An assignment or increment whose target select holds `$`.
    fn writes_through_dollar(stmt: &Statement) -> bool {
        use crate::compiler::simulator::Simulator;
        let target = match &stmt.kind {
            StatementKind::BlockingAssign { lvalue, .. }
            | StatementKind::NonblockingAssign { lvalue, .. } => lvalue,
            StatementKind::Expr(e) => match &e.kind {
                ExprKind::AssignExpr { lvalue, .. } => lvalue,
                ExprKind::Unary {
                    op: UnaryOp::PreIncr | UnaryOp::PostIncr | UnaryOp::PreDecr | UnaryOp::PostDecr,
                    operand,
                } => operand,
                _ => return false,
            },
            _ => return false,
        };
        Simulator::select_mentions_dollar(target)
    }

    fn compile_stmt_strict(&mut self, stmt: &Statement) -> bool {
        // §7.10: a target whose select holds `$` goes to the interpreter
        // whole; `assign_value` binds each `$` to its own base before it
        // stores.
        if Self::writes_through_dollar(stmt) {
            self.bail("dollar_lvalue");
            return false;
        }
        if let Some(done) = self.compile_unpacked_struct_copy(stmt) {
            if !done {
                self.bail("Stmt_unpacked_struct_whole");
            }
            return done;
        }
        match &stmt.kind {
            // Process-FSM mode: a statement-level timing control becomes a
            // wait insn followed by its guarded statement. Star (`@*`) and
            // intra-assignment forms never reach here (gated by the caller /
            // canonicalized into marker calls that fail compile_expr).
            StatementKind::TimingControl {
                control,
                stmt: inner,
            } if self.allow_waits => {
                match control {
                    crate::ast::stmt::TimingControl::Delay(d) => {
                        // §3.14.3: `1step` is one global tick, not a value the
                        // wait rounds to the scope's precision; the
                        // interpreter's delay path handles it (#615).
                        if matches!(&d.kind, ExprKind::SystemCall { name, .. }
                            if name == crate::compiler::elaborate::ONE_STEP_DELAY)
                        {
                            return false;
                        }
                        let Some(r) = self.compile_expr(d, 0) else {
                            return false;
                        };
                        self.emit(Insn::WaitDelayReg(r));
                    }
                    crate::ast::stmt::TimingControl::Event(ev) => {
                        if matches!(
                            ev,
                            crate::ast::stmt::EventControl::Star
                                | crate::ast::stmt::EventControl::ParenStar
                        ) {
                            return false;
                        }
                        self.wait_specs.push(ev.clone());
                        self.emit(Insn::WaitEdge((self.wait_specs.len() - 1) as u32));
                    }
                }
                return self.compile_stmt(inner);
            }
            // Process-FSM mode: `forever <body-with-waits>` is the FSM's
            // native shape — body then an unconditional back-jump. The ≥1
            // wait gate at registration guarantees each iteration suspends.
            StatementKind::Forever { body } if self.allow_waits && Self::stmt_is_blocking(body) => {
                let top = self.insns.len() as u32;
                if !self.compile_stmt(body) {
                    return false;
                }
                self.emit(Insn::Jump(top));
                return true;
            }
            // Process-FSM mode: `repeat (N) <body-with-waits>` compiles to a
            // counted loop (count evaluated ONCE, §12.7.2) so the classic
            // `repeat (n) @(posedge clk);` cycle-wait works. Wait-free
            // repeats keep the existing unroll paths below.
            StatementKind::Repeat { count, body }
                if self.allow_waits && Self::stmt_is_blocking(body) =>
            {
                // A real count rounds to an integer (§12.7.2); the counted
                // loop below would decrement the real itself. Interpret it.
                if self.expr_may_be_real(count) {
                    self.bail("Repeat_real_count");
                    return false;
                }
                let Some(cnt) = self.compile_expr(count, 0) else {
                    return false;
                };
                let ctr = self.alloc_reg();
                self.emit(Insn::Move(ctr, cnt));
                // §12.7.2/§6.12.2: a signed-negative count (e.g. `repeat(-1)`)
                // means ZERO iterations — mirrors the interpreter's
                // `repeat_count`, which clamps signed values with `.max(0)`.
                // Without this guard, `repeat(-1)` would read back as
                // 0xFFFFFFFF (non-zero, non-XZ) and spin ~2^32 times.
                let neg_test = self.alloc_reg();
                let signed_zero = self.alloc_reg();
                let mut zv = Value::from_u64(0, 32);
                zv.is_signed = true;
                self.emit(Insn::LoadConst(signed_zero, Box::new(zv)));
                // Signed compare (both operands need is_signed for `Lt` to
                // use signed semantics): neg = (signed ctr < 0).
                self.emit(Insn::Lt(neg_test, ctr, signed_zero));
                let neg_jump = self.insns.len();
                // Skip the counted loop when the count is definitely negative
                // (`BranchUnlessZero` jumps on a definite 1; an X count falls
                // through to the BranchIfFalse below, which exits on X/Z).
                self.emit(Insn::BranchUnlessZero(neg_test, 0));
                let top = self.insns.len() as u32;
                let branch_idx = self.insns.len();
                // Exits when the counter is no longer definitely non-zero
                // (X/Z counts as zero, matching the interpreter's repeat).
                self.emit(Insn::BranchIfFalse(ctr, 0));
                if !self.compile_stmt(body) {
                    return false;
                }
                let one = self.alloc_reg();
                self.emit(Insn::LoadConst(one, Box::new(Value::from_u64(1, 32))));
                self.emit(Insn::Sub(ctr, ctr, one));
                self.emit(Insn::Jump(top));
                let end = self.insns.len() as u32;
                self.insns[branch_idx] = Insn::BranchIfFalse(ctr, end);
                self.insns[neg_jump] = Insn::BranchUnlessZero(neg_test, end);
                return true;
            }

            StatementKind::Null => true,
            // §13.4.1 early return inside an INLINED body: move the value
            // into the result register and jump to the body end (patched by
            // the inliner). Outside an inline there is nothing to return to.
            // class-perf method-mode: an explicit `return e;` in a compiled
            // class method writes e into the designated return register and
            // jumps to the method's common exit (the prologue reads that
            // register's value and returns it to the caller). Trailing/jump
            // targets are back-patched like `inline_ret_jumps` below.
            // §13.4.1: a nested inline returns to its caller inside this
            // method, using inline_ret below, rather than ending the method.
            StatementKind::Return(e) if self.method_mode && self.inline_ret.is_none() => {
                // class-perf collection-return: only `return '{…}` literals
                // are admitted — each element lands in a contiguous arg slot
                // and one `RetCollection` hands the range to the interpreter's
                // snapshot store (mirrors the AST path's
                // `populate_queue_from_init(RET_SNAP, e)` + pending hand-off).
                // Any other return expression form bails — the interpreter's
                // conversion/concat paths for queue-typed values are not
                // reproduced.
                if self.method_ret_is_collection {
                    let Some(items) = e else {
                        self.bail("Return_collection_empty");
                        return false;
                    };
                    let crate::ast::expr::ExprKind::AssignmentPattern(items) = &items.kind else {
                        self.bail("Return_collection_not_literal");
                        return false;
                    };
                    let element = self
                        .method_return_type
                        .and_then(|dt| self.local_kind_of(dt, None));
                    let n = items.len() as u32;
                    let arg_start = self.alloc_reg();
                    for _ in 1..items.len() {
                        self.alloc_reg();
                    }
                    if self.register_overflow {
                        return false;
                    }
                    for (i, item) in items.iter().enumerate() {
                        let crate::ast::expr::AssignmentPatternItem::Ordered(item) = item else {
                            self.bail("Return_collection_named_item");
                            return false;
                        };
                        // Other element types and replicated pattern items use the interpreter.
                        let Some(kind @ LocalKind::Int { width, .. }) = element else {
                            self.bail("Return_collection_element_type");
                            return false;
                        };
                        if matches!(
                            item.kind,
                            ExprKind::Replication { .. } | ExprKind::AssignmentPattern(_)
                        ) {
                            self.bail("Return_collection_nested_pattern");
                            return false;
                        }
                        let Some(value) = self.compile_expr(item, width) else {
                            return false;
                        };
                        let slot = arg_start + i as RegId;
                        self.emit(Insn::Move(slot, value));
                        self.emit_real_to_int_fit(slot, item, width);
                        self.emit(Insn::Resize(slot, width));
                        self.local_kinds.insert(slot, kind);
                        self.emit_local_fit(slot);
                    }
                    self.emit(Insn::RetCollection(arg_start as u32, n));
                    let j = self.insns.len();
                    self.emit(Insn::Jump(0));
                    self.method_ret_jumps.push(j);
                    return true;
                }
                if let Some(e) = e {
                    let Some(rv) = self.method_return_val_reg else {
                        self.bail("Return_value_in_void");
                        return false;
                    };
                    // Class-handle returns pass the slot value through UNTOUCHED
                    // (no Resize), mirroring the interpreter, which never
                    // resizes/stamps a non-integral (handle) return. The result
                    // width is therefore allowed to be absent when
                    // `method_result_is_class` is set.
                    let w = if let Some(w) = self
                        .method_result_reg
                        .and_then(|_| self.method_result_width)
                    {
                        w
                    } else if self.method_result_is_class || self.method_result_is_string {
                        0
                    } else {
                        self.bail("Return_unknown_width");
                        return false;
                    };
                    self.pattern_ctx = self.method_return_type.cloned();
                    let Some(v) = self.compile_expr(e, w) else {
                        return false;
                    };
                    self.emit(Insn::Move(rv, v));
                    if w > 0 {
                        // §13.4.1, §6.12.1: an integral result rounds a real.
                        if matches!(self.local_kinds.get(&rv), Some(LocalKind::Int { .. })) {
                            self.emit_real_to_int_fit(rv, e, w);
                        }
                        self.emit(Insn::Resize(rv, w));
                        self.emit_local_fit(rv);
                    }
                }
                let j = self.insns.len();
                self.emit(Insn::Jump(0));
                self.method_ret_jumps.push(j);
                true
            }
            StatementKind::Return(e) => {
                let Some((slot, w)) = self.inline_ret else {
                    self.bail("Return_outside_inline");
                    return false;
                };
                if let Some(e) = e {
                    let Some(slot) = slot else {
                        self.bail("Return_value_in_void");
                        return false;
                    };
                    self.pattern_ctx = self.inline_ret_ty.clone();
                    let Some(v) = self.compile_expr(e, w) else {
                        return false;
                    };
                    self.emit(Insn::Move(slot, v));
                    // §13.4.1: the returned value converts to the declared
                    // type. A real return converts an integral value; an
                    // integral return rounds a real (`emit_real_to_int_fit`
                    // for the 64-bit width `Resize` keeps a real at).
                    if self.inline_ret_real {
                        if w > 0 {
                            self.emit(Insn::Resize(slot, w));
                        }
                        if !self.expr_is_real_static(e) {
                            self.emit_to_real(slot);
                        }
                    } else if w > 0 {
                        self.emit_real_to_int_fit(slot, e, w);
                        self.emit(Insn::Resize(slot, w));
                        self.emit_local_fit(slot);
                    }
                }
                let j = self.insns.len();
                self.emit(Insn::Jump(0));
                self.inline_ret_jumps.push(j);
                true
            }
            StatementKind::VarDecl {
                data_type,
                lifetime,
                declarators,
            } => {
                if self.method_mode && self.class_alias_dynamic_width(data_type) {
                    self.bail("class_alias_runtime_width");
                    return false;
                }
                for decl in declarators {
                    if !self.compile_stmt_decl_item(stmt.span, data_type, lifetime, decl) {
                        return false;
                    }
                }
                true
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
                for decl in declarators {
                    let eff_dt = if untyped
                        && let crate::ast::types::DataType::Implicit { signing, .. } = data_type
                    {
                        Some(self.untyped_param_effective_dt(
                            signing,
                            decl.init.as_deref(),
                            stmt.span,
                        ))
                    } else {
                        None
                    };
                    let dt = eff_dt.as_ref().unwrap_or(data_type);
                    if !self.compile_stmt_decl_item(stmt.span, dt, &None, decl) {
                        return false;
                    }
                }
                true
            }
            StatementKind::NonblockingAssign { lvalue, rvalue, .. } => {
                // §9.4.5 / §14.16 intra-assignment timing markers
                // (`lhs <= #d rhs`, `<= @(e) rhs`, `<= ##n rhs`) must reach
                // the interpreter's NBA arm, which schedules the update d
                // ticks out. Compiled as a plain expression the marker is an
                // unknown system call that yields its RHS with the delay
                // silently dropped (issue #160).
                if Self::is_intra_timing_marker(rvalue) {
                    self.bail("nba_intra_timing");
                    return self.emit_fallback(stmt);
                }
                // Unpacked copies snapshot the RHS and schedule one update
                // per element. The scalar NBA instructions cannot do that.
                if self.names_unpacked_collection(lvalue)
                    || self.names_unpacked_collection(rvalue)
                {
                    self.bail("nba_unpacked_collection");
                    return self.emit_fallback(stmt);
                }
                let width = self.infer_lhs_width(lvalue);
                let start = self.insns.len();
                let start_reg = self.next_reg;
                let rctx = self.assign_rhs_ctx(lvalue, width);
                self.pattern_ctx = self.lvalue_pattern_ctx(lvalue, rvalue);
                let compiled = self.compile_expr(rvalue, rctx);
                self.pattern_ctx = None;
                if let Some(mut val_reg) = compiled {
                    if self.lvalue_is_real(lvalue) && !self.expr_is_real_static(rvalue) {
                        let t = self.alloc_reg();
                        self.emit(Insn::Move(t, val_reg));
                        self.emit_real_target_fit(t, rvalue);
                        val_reg = t;
                    }
                    // Note: NbaAssign itself performs §10.7 assignment-padding resize,
                    // so we don't emit a generic (zero-extending) Resize here — that
                    // would strip X/Z from the MSB before the assignment could X/Z-extend.
                    if self.compile_nba_target(lvalue, val_reg, width) {
                        return true;
                    }
                    self.bail("nba_target");
                } else {
                    self.bail("nba_rvalue");
                }
                // Roll back partial work and emit fallback if allowed.
                self.truncate_insns(start);
                self.next_reg = start_reg;
                self.emit_fallback(stmt)
            }
            StatementKind::BlockingAssign { lvalue, rvalue } => {
                // Same intra-assignment timing markers on a blocking
                // assignment (`lhs = #d rhs`): the interpreter arm blocks
                // the process; a compiled expression drops the delay.
                if Self::is_intra_timing_marker(rvalue) {
                    self.bail("blocking_intra_timing");
                    return self.emit_fallback(stmt);
                }
                // class-perf Step 9b-ii: member-collection ELEMENT store
                // (`q[i] = v`, `aa[k] = v`, dotted forms). Must hook BEFORE
                // the `infer_lhs_width` Resize below: its Index arm guesses
                // 1 bit for a member collection (no element-width signal
                // entry) and would destroy the value before the runtime
                // store sees it — the interpreter fits the element width
                // from the live store inside `assign_value_index`. The
                // rvalue compiles at its RAW width; the exec arm delegates
                // to the interpreter's §10.7 fit. Admission is the same
                // receiver matrix as the read; a rejection falls through to
                // the ordinary AST path (all-or-nothing statement).
                if self.method_mode
                    && let ExprKind::Index { expr, index } = &lvalue.kind
                    && self.member_coll_elem_target(expr).is_some()
                    // The interpreter's statement path returns EARLY (before
                    // its member-element arm) for an aggregate-pattern RHS
                    // (`aa[k] = '{...}` spreads) and for a `new` RHS
                    // (`mb[i] = new(...)` allocates / constructs) — neither
                    // is a plain element store. Keep both shapes AST.
                    && !match &rvalue.kind {
                        ExprKind::AssignmentPattern(..) | ExprKind::TypedPattern { .. } => true,
                        ExprKind::Call { func, .. } => matches!(
                            &func.kind,
                            ExprKind::Ident(h)
                                if h.path.len() == 1 && h.path[0].name.name == "new"
                        ),
                        ExprKind::Ident(h) => {
                            h.path.len() == 1 && h.path[0].name.name == "new"
                        }
                        _ => false,
                    }
                {
                    let start = self.insns.len();
                    let start_reg = self.next_reg;
                    // §11.6.1: the element's declared width is the context
                    // (`d[i] = d[i] & ((1 << w) - 1)` on `bit [1023:0] d[]`).
                    let ctx = self.member_coll_elem_width(expr).unwrap_or(0);
                    if let Some(val_reg) = self.compile_expr(rvalue, ctx)
                        && self.compile_member_coll_elem_store(expr, index, val_reg)
                    {
                        return true;
                    }
                    self.truncate_insns(start);
                    self.next_reg = start_reg;
                    // Fall through to the AST fallback below.
                    self.bail("coll_elem_store");
                    return self.emit_fallback(stmt);
                }
                let width = self.infer_lhs_width(lvalue);
                let start = self.insns.len();
                let start_reg = self.next_reg;
                // Step 9g: `lhs = new` / `lhs = new(args)` — the
                // constructed class is the LHS's DECLARED type (§8.8); a
                // handle needs no Resize regardless of the inferred width.
                let new_cls = if Self::is_new_shape(rvalue) {
                    self.new_target_for(lvalue)
                } else {
                    None
                };
                let via_new = new_cls.is_some();
                let compiled = if let Some(cls) = new_cls {
                    self.compile_new_construct(rvalue, &cls)
                } else {
                    let rctx = self.assign_rhs_ctx(lvalue, width);
                    self.pattern_ctx = self.lvalue_pattern_ctx(lvalue, rvalue);
                    self.compile_expr(rvalue, rctx)
                };
                self.pattern_ctx = None;
                if let Some(val_reg) = compiled {
                    // class-perf collection-return store (§13.4):
                    // `q_local = coll_fn(..)`. The callee hands a queue /
                    // array result through the `pending_ret_collection` side
                    // channel, which the call's exec arm records in
                    // `vm_ret_coll`; AssignPendingColl copies it whole into
                    // the local (or assigns the scalar result when nothing was
                    // handed back). Only when the value register is the
                    // result of the LAST call instruction emitted for the
                    // RHS, so the recorded collection is this call's.
                    if self.method_mode
                        && matches!(rvalue.kind, ExprKind::Call { .. })
                        && let Some(key) = self.local_coll_lhs_key(lvalue)
                        && self.insns[start..]
                            .iter()
                            .rev()
                            .find_map(Self::call_insn_dest)
                            == Some(val_reg)
                    {
                        self.emit(Insn::AssignPendingColl(key.into_boxed_str(), val_reg));
                        return true;
                    }
                    if width > 0 && !via_new && !self.expr_is_string_static(rvalue) {
                        // §10.7, §6.12.1: a real stored into an integral
                        // variable is rounded (`l = r;` with `longint l`).
                        if !self.value_may_be_real(lvalue) {
                            self.emit_real_to_int_fit(val_reg, rvalue, width);
                        } else if self.lvalue_is_real(lvalue) {
                            self.emit_real_target_fit(val_reg, rvalue);
                        }
                        self.emit(Insn::Resize(val_reg, width));
                    }
                    // class-perf P2: STRING member store (`m_leaf_name = name`)
                    // whose RHS is from the string-valued expr universe. The
                    // heap round-trip is exact for string Values — the store
                    // executor's fit_class_prop has no declared integral
                    // width for a string slot and clones the Value verbatim,
                    // same as the AST store. Bare this-relative and dotted
                    // chain targets only; shadow-gated like the integral path.
                    if self.method_mode
                        && width == 0
                        && !via_new
                        && self.expr_is_string_static(rvalue)
                        && self.method_string_store_target(lvalue, val_reg)
                    {
                        return true;
                    }
                    if self.compile_blocking_target(lvalue, val_reg, width) {
                        return true;
                    }
                    self.bail("blocking_target");
                } else {
                    self.bail("blocking_rvalue");
                }
                self.truncate_insns(start);
                self.next_reg = start_reg;
                self.emit_fallback(stmt)
            }
            StatementKind::If {
                condition,
                then_stmt,
                else_stmt,
                ..
            } => {
                // §12.6 `if (e matches p)` binds the pattern's `.name`s for the
                // then-branch. That needs the AST interpreter — compiling it to
                // a conditional jump would evaluate the match but drop the
                // bindings, so the branch ran with `n` unset.
                if matches!(condition.kind, ExprKind::Matches { .. }) {
                    return false;
                }
                if let Some(cond_reg) = self.compile_expr(condition, 0) {
                    let branch_idx = self.insns.len();
                    self.emit(Insn::BranchIfFalse(cond_reg, 0)); // placeholder target
                    if !self.compile_stmt(then_stmt) {
                        return false;
                    }
                    if let Some(el) = else_stmt {
                        let jump_idx = self.insns.len();
                        self.emit(Insn::Jump(0)); // placeholder
                        let else_start = self.insns.len() as u32;
                        self.insns[branch_idx] = Insn::BranchIfFalse(cond_reg, else_start);
                        if !self.compile_stmt(el) {
                            return false;
                        }
                        let end = self.insns.len() as u32;
                        self.insns[jump_idx] = Insn::Jump(end);
                    } else {
                        let end = self.insns.len() as u32;
                        self.insns[branch_idx] = Insn::BranchIfFalse(cond_reg, end);
                    }
                    true
                } else {
                    false
                }
            }
            StatementKind::Case {
                kind, expr, items, ..
            } => {
                // §12.5.4: `case inside` matches each item with the `inside`
                // operator: a range by `>=`/`<=`, a value by `==?` (the
                // item's x/z bits are wildcards) and a real by `==`
                // (§11.4.13). The compare chain below matches by `===`, which
                // agrees with that only for an integral selector and x/z-free
                // integral constant items; anything else leaves the statement
                // to the interpreter's matcher.
                let inside_kind = matches!(kind, crate::ast::stmt::CaseKind::CaseInside);
                // §12.5: a real selector or item compares numerically, not
                // bit for bit (`case (r) 1:` matches `r = 1.0`); the
                // interpreter's matcher does that.
                if matches!(kind, crate::ast::stmt::CaseKind::Case)
                    && (self.expr_is_real_static(expr)
                        || items
                            .iter()
                            .flat_map(|it| it.patterns.iter())
                            .any(|p| self.expr_is_real_static(p)))
                {
                    self.bail("Stmt_Case_real");
                    return self.emit_fallback(stmt);
                }
                // The masked compare reads the selector at its own width, so an
                // operation selector (sized with the items, below) keeps `===`.
                let wild_sel_w = if inside_kind
                    && !matches!(
                        expr.kind,
                        ExprKind::Binary { .. }
                            | ExprKind::Unary { .. }
                            | ExprKind::Conditional { .. }
                    )
                    && self.expr_signedness(expr) == Some(false)
                {
                    Some(self.expr_max_width(expr)).filter(|w| (1..=64).contains(w))
                } else {
                    None
                };
                if inside_kind && !self.case_inside_matches_by_case_eq(expr, items, wild_sel_w) {
                    self.bail("Stmt_CaseInside");
                    return self.emit_fallback(stmt);
                }
                // §12.5: the selector and the items are sized to the widest
                // of them. An operation selector (`a8 + 8'd1`) narrower than
                // an item is computed at that width, in the compare chain.
                let operation = |e: &Expression| {
                    matches!(
                        e.kind,
                        ExprKind::Binary { .. }
                            | ExprKind::Unary { .. }
                            | ExprKind::Conditional { .. }
                    )
                };
                let patterns = items.iter().flat_map(|it| &it.patterns);
                let op_selector = operation(expr);
                let op_item = !inside_kind && patterns.clone().any(operation);
                let common_w = if op_selector || op_item {
                    let own = self.lrm_self_width(expr);
                    patterns
                        .clone()
                        .filter(|p| !matches!(p.kind, ExprKind::Range(..)))
                        .fold(own, |w, p| w.max(self.lrm_self_width(p)))
                } else {
                    0
                };
                let sel_w =
                    (op_selector && common_w > self.lrm_self_width(expr)).then_some(common_w);
                let case_unsigned = !inside_kind
                    && (self.expr_signedness(expr) == Some(false)
                        || patterns
                            .clone()
                            .any(|p| self.expr_signedness(p) == Some(false)));
                // ROM shape: plain `case` whose every arm assigns a CONSTANT
                // to the SAME variable, constant patterns throughout. One
                // table lookup replaces the whole compare chain. `===`
                // semantics hold exactly: the patterns are fully defined, so
                // a selector with any x/z bit matches none of them → default.
                if sel_w.is_none() && matches!(kind, crate::ast::stmt::CaseKind::Case) {
                    if case_unsigned
                        && let Some((lhs, table, default, res_w)) = self.case_lut_shape(items)
                    {
                        if let Some(sel) = self.compile_expr(expr, 0) {
                            let dst = self.alloc_reg();
                            self.emit(Insn::CaseLut(
                                dst,
                                sel,
                                Box::new(CaseLutData { table, default }),
                            ));
                            if self.compile_blocking_target(&lhs, dst, res_w) {
                                return true;
                            }
                        }
                        // fall through to the generic chain on any failure
                    }
                    if self.compile_case_jump(expr, items) {
                        return true;
                    }
                }
                if sel_w.is_none()
                    && matches!(
                        kind,
                        crate::ast::stmt::CaseKind::Casez | crate::ast::stmt::CaseKind::Casex
                    )
                    && self.compile_case_mask_jump(kind, expr, items)
                {
                    return true;
                }
                if let Some(val_reg) =
                    self.compile_expr_typed(expr, sel_w.unwrap_or(0), case_unsigned)
                {
                    let mut end_jumps: Vec<usize> = Vec::new();
                    let mut default_item: Option<&Statement> = None;
                    // Every pattern arm is mutually exclusive and jumps to the
                    // case end after its body. Reuse its temporary registers so
                    // a large constant map does not exhaust the register id
                    // space merely by having many alternatives.
                    let arm_reg_start = self.next_reg;
                    let mut peak_reg = arm_reg_start;
                    for item in items {
                        if item.is_default {
                            default_item = Some(&item.stmt);
                            continue;
                        }
                        // Compile pattern match: val === pattern (or casez/casex
                        // wildcard match per CaseKind).
                        for pat in &item.patterns {
                            // §12.5.4: a wildcard `case inside` item compares
                            // the masked selector with its known bits.
                            let wild =
                                wild_sel_w.and_then(|w| self.case_inside_wildcard_item(pat, w));
                            let cmp_reg = if let Some((mask, known)) = wild {
                                let mask_reg = self.alloc_reg();
                                self.emit(Insn::LoadConst(mask_reg, Box::new(mask)));
                                let masked = self.alloc_reg();
                                self.emit(Insn::BitAnd(masked, val_reg, mask_reg));
                                let known_reg = self.alloc_reg();
                                self.emit(Insn::LoadConst(known_reg, Box::new(known)));
                                let cmp_reg = self.alloc_reg();
                                self.emit(Insn::CaseEq(cmp_reg, masked, known_reg));
                                Some(cmp_reg)
                            } else if let Some(pat_reg) =
                                self.compile_expr_typed(pat, common_w, case_unsigned)
                            {
                                let cmp_reg = self.alloc_reg();
                                self.emit(match kind {
                                    crate::ast::stmt::CaseKind::Casez => {
                                        Insn::CasezEq(cmp_reg, val_reg, pat_reg)
                                    }
                                    crate::ast::stmt::CaseKind::Casex => {
                                        Insn::CasexEq(cmp_reg, val_reg, pat_reg)
                                    }
                                    _ => Insn::CaseEq(cmp_reg, val_reg, pat_reg),
                                });
                                Some(cmp_reg)
                            } else {
                                None
                            };
                            if let Some(cmp_reg) = cmp_reg {
                                let branch_idx = self.insns.len();
                                self.emit(Insn::BranchIfFalse(cmp_reg, 0));
                                if !self.compile_stmt(&item.stmt) {
                                    return false;
                                }
                                end_jumps.push(self.insns.len());
                                self.emit(Insn::Jump(0));
                                let next = self.insns.len() as u32;
                                self.insns[branch_idx] = Insn::BranchIfFalse(cmp_reg, next);
                                peak_reg = peak_reg.max(self.next_reg);
                                self.next_reg = arm_reg_start;
                            } else {
                                return false;
                            }
                        }
                    }
                    // Default case
                    if let Some(def_stmt) = default_item {
                        if !self.compile_stmt(def_stmt) {
                            return false;
                        }
                        peak_reg = peak_reg.max(self.next_reg);
                    }
                    self.next_reg = peak_reg;
                    let end = self.insns.len() as u32;
                    for idx in end_jumps {
                        self.insns[idx] = Insn::Jump(end);
                    }
                    true
                } else {
                    false
                }
            }
            // §9.3.2: a fork's children are CONCURRENT PROCESSES. Compiling
            // a ParBlock like a SeqBlock ran them sequentially INLINE — for
            // `fork ... join_none` with a delaying child that meant the edge
            // block itself executed the child's infinite `#1` loop on its own
            // stack: the design froze (no other always block could ever fire
            // again, so even $finish became unreachable) and the run had to
            // be SIGKILLed. Always leave fork/join to the AST interpreter's
            // ParBlock arm, which spawns real child processes.
            StatementKind::ParBlock { .. } => {
                self.bail("Stmt_ParBlock");
                self.emit_fallback(stmt)
            }
            StatementKind::SeqBlock { name, stmts } => {
                // §6.21: a static local of a process block keeps its value
                // between activations, which a register seeded at the
                // declaration cannot do. Leave the block to the interpreter
                // when such a local can be observed doing so.
                if !self.method_mode
                    && self.inlining_stack.is_empty()
                    && Self::block_has_carried_static(stmts, Some(self))
                {
                    self.bail("static_block_local");
                    return false;
                }
                let saved_locals = self.local_var_regs.clone();
                let saved_decl_locals = self.decl_local_regs.clone();
                // A block's declarations re-type their names only inside it.
                let saved_reals = self.local_var_is_real.clone();
                let saved_strings = self.local_var_is_string.clone();
                let m_depth = self.m_labels.len();
                let prev_block_patches = if let Some(n) = name {
                    self.m_labels.push(n.clone());
                    self.named_block_patches.insert(n.name.clone(), Vec::new())
                } else {
                    None
                };
                let mut ok = true;
                for s in stmts {
                    if !self.compile_stmt(s) {
                        ok = false;
                        break;
                    }
                }
                if let Some(n) = name {
                    if let Some(patches) = self.named_block_patches.remove(&n.name) {
                        let end = self.insns.len() as u32;
                        for idx in patches {
                            self.insns[idx] = Insn::Jump(end);
                        }
                    }
                    if let Some(prev) = prev_block_patches {
                        self.named_block_patches.insert(n.name.clone(), prev);
                    }
                }
                self.m_labels.truncate(m_depth);
                self.local_var_regs = saved_locals;
                self.decl_local_regs = saved_decl_locals;
                self.local_var_is_real = saved_reals;
                self.local_var_is_string = saved_strings;
                ok
            }
            // Bail out on anything else (timing controls, loops, system tasks, etc.)
            StatementKind::Expr(e)
                if matches!(&e.kind, ExprKind::SystemCall { name, .. }
                    if name == crate::compiler::simulator::COV_HIT_TASK) =>
            {
                let ExprKind::SystemCall { args, .. } = &e.kind else {
                    return false;
                };
                match args
                    .first()
                    .and_then(crate::compiler::simulator::cov_marker_id)
                {
                    Some(c) => {
                        self.emit(Insn::CovHit(c));
                        true
                    }
                    None => false,
                }
            }
            StatementKind::Expr(e) => {
                // §6.24.1: `void'(expr)` lowers to `Paren(expr)` (the cast is
                // a pure discard). The old `Paren(_) => no-op` arm below then
                // swallowed the whole statement, so `void'(q.pop_front())`
                // inside a compiled always block never popped — while the
                // bare `q.pop_front();` form fell through to the AST fallback
                // and worked, which is exactly how the difference hid. Peel
                // the wrappers so the inner expression's own arm decides.
                let mut e = e;
                while let ExprKind::Paren(inner) = &e.kind {
                    e = inner;
                }
                match &e.kind {
                    // Bare identifier as statement: side-effect-free read, compile as no-op
                    // — BUT only if it actually resolves to a signal. A bare ident that
                    // doesn't resolve is typically a task-enable (`task_name;`) whose
                    // dispatch must happen in the AST interpreter's `exec_expr_stmt`.
                    ExprKind::Ident(hier) if hier.path.len() == 1 => {
                        if self.lookup_signal_id(hier).is_some() {
                            return true;
                        }
                        let name = hier.path[0].name.name.clone();
                        if self.try_inline_task(&name) {
                            return true;
                        }
                        self.bail("Expr_TaskEnable");
                        return self.emit_fallback(stmt);
                    }
                    ExprKind::Ident(hier) if hier.path.len() > 1 => {
                        let mname = hier.path.last().unwrap().name.name.as_str();
                        if matches!(
                            mname,
                            "delete"
                                | "sort"
                                | "rsort"
                                | "reverse"
                                | "unique"
                                | "unique_index"
                                | "pop_front"
                                | "pop_back"
                        ) {
                            return self
                                .emit_fallback(&Statement::new(stmt.kind.clone(), stmt.span));
                        }
                        if self.lookup_signal_id(hier).is_some() {
                            return true;
                        }
                        let leaf = hier.path.last().unwrap().name.name.clone();
                        if self.try_inline_task(&leaf) {
                            return true;
                        }
                        self.bail("Expr_TaskEnable");
                        return self.emit_fallback(stmt);
                    }
                    // A literal as a statement is genuinely side-effect-free.
                    // (`Paren` can no longer appear here — peeled above.)
                    ExprKind::Number(_) => {
                        return true;
                    }
                    // Pre/post increment/decrement have side effects — compile them
                    ExprKind::Unary {
                        op: UnaryOp::PreIncr,
                        operand,
                    }
                    | ExprKind::Unary {
                        op: UnaryOp::PostIncr,
                        operand,
                    } => {
                        if let Some(sig_id) = self.expr_to_signal_id(operand) {
                            let r = self.alloc_reg();
                            self.emit(Insn::LoadSignal(r, as_sig_id(sig_id)));
                            let one = self.alloc_reg();
                            let w = self.signal_widths[sig_id];
                            self.emit(Insn::LoadConst(one, Box::new(Value::from_u64(1, w))));
                            let result = self.alloc_reg();
                            self.emit(Insn::Add(result, r, one));
                            self.emit(Insn::Resize(result, w));
                            self.emit(Insn::BlockingAssign(as_sig_id(sig_id), result, w));
                            return true;
                        }
                        // class-perf P2: statement-position ++ on a register-
                        // backed method target (class member via any handle
                        // chain, block local, formal) — read, +1, store
                        // through the ordinary assignment machinery. The
                        // POST forms discard the value in statement
                        // position, so one lowering serves all four ops.
                        if self.method_mode {
                            // Width 0 = a self-determined member/static store
                            // (the heap truncates at the declared width —
                            // `++cnt` on a 4-bit member wraps exactly like
                            // `cnt = cnt + 1`, verified against the AST
                            // interpreter); Resize only for width>0 targets.
                            let width = self.infer_lhs_width(operand);
                            let start = self.insns.len();
                            let start_reg = self.next_reg;
                            let lowered = (|| {
                                let r = self.compile_expr(operand, width)?;
                                let one = self.alloc_reg();
                                self.emit(Insn::LoadConst(
                                    one,
                                    Box::new(Value::from_u64(
                                        1,
                                        if width > 0 { width } else { 32 },
                                    )),
                                ));
                                let result = self.alloc_reg();
                                self.emit(Insn::Add(result, r, one));
                                if width > 0 {
                                    self.emit(Insn::Resize(result, width));
                                }
                                self.compile_blocking_target(operand, result, width)
                                    .then_some(())
                            })();
                            if lowered == Some(()) {
                                return true;
                            }
                            self.truncate_insns(start);
                            self.next_reg = start_reg;
                        }
                        self.bail("Expr_PreIncr");
                        return self.emit_fallback(stmt);
                    }
                    ExprKind::Unary {
                        op: UnaryOp::PreDecr,
                        operand,
                    }
                    | ExprKind::Unary {
                        op: UnaryOp::PostDecr,
                        operand,
                    } => {
                        if let Some(sig_id) = self.expr_to_signal_id(operand) {
                            let r = self.alloc_reg();
                            self.emit(Insn::LoadSignal(r, as_sig_id(sig_id)));
                            let one = self.alloc_reg();
                            let w = self.signal_widths[sig_id];
                            self.emit(Insn::LoadConst(one, Box::new(Value::from_u64(1, w))));
                            let result = self.alloc_reg();
                            self.emit(Insn::Sub(result, r, one));
                            self.emit(Insn::Resize(result, w));
                            self.emit(Insn::BlockingAssign(as_sig_id(sig_id), result, w));
                            return true;
                        }
                        // class-perf P2: the -- mirror of the ++ lowering above
                        // (statement position, register-backed targets).
                        if self.method_mode {
                            // Width 0 = a self-determined member/static store
                            // (the heap truncates at the declared width —
                            // `++cnt` on a 4-bit member wraps exactly like
                            // `cnt = cnt + 1`, verified against the AST
                            // interpreter); Resize only for width>0 targets.
                            let width = self.infer_lhs_width(operand);
                            let start = self.insns.len();
                            let start_reg = self.next_reg;
                            let lowered = (|| {
                                let r = self.compile_expr(operand, width)?;
                                let one = self.alloc_reg();
                                self.emit(Insn::LoadConst(
                                    one,
                                    Box::new(Value::from_u64(
                                        1,
                                        if width > 0 { width } else { 32 },
                                    )),
                                ));
                                let result = self.alloc_reg();
                                self.emit(Insn::Sub(result, r, one));
                                if width > 0 {
                                    self.emit(Insn::Resize(result, width));
                                }
                                self.compile_blocking_target(operand, result, width)
                                    .then_some(())
                            })();
                            if lowered == Some(()) {
                                return true;
                            }
                            self.truncate_insns(start);
                            self.next_reg = start_reg;
                        }
                        self.bail("Expr_PreDecr");
                        return self.emit_fallback(stmt);
                    }
                    // Call-as-statement: a task enable with arguments, or a
                    // void function. Tasks first (they are not in the function
                    // table); a void function call compiles through the
                    // ordinary expression path below, discarding its (empty)
                    // result register.
                    ExprKind::Call { func, args } => {
                        // §6.16.4/§6.16.10 in-place string mutators as
                        // statements: compute the modified text natively,
                        // store it back to the receiver.
                        if let Some((recv, method)) = self.string_method_shape(func, e.span) {
                            let kind = match (method, args.len()) {
                                ("putc", 2) => Some(StrOpKind::PutC),
                                ("itoa", 1) => Some(StrOpKind::IToA),
                                ("hextoa", 1) => Some(StrOpKind::HexToA),
                                ("octtoa", 1) => Some(StrOpKind::OctToA),
                                ("bintoa", 1) => Some(StrOpKind::BinToA),
                                _ => None,
                            };
                            if let Some(kind) = kind {
                                let start = self.insns.len();
                                let start_reg = self.next_reg;
                                let mut regs: Vec<RegId> = Vec::new();
                                let mut ok = true;
                                // PutC reads the current text; the *toa
                                // family overwrites it wholesale.
                                if kind == StrOpKind::PutC {
                                    match self.compile_expr(&recv, 0) {
                                        Some(r) => regs.push(r),
                                        None => ok = false,
                                    }
                                }
                                if ok {
                                    for a in args {
                                        match self.compile_expr(a, 0) {
                                            Some(r) => regs.push(r),
                                            None => {
                                                ok = false;
                                                break;
                                            }
                                        }
                                    }
                                }
                                if ok {
                                    let dst = self.alloc_reg();
                                    self.emit(Insn::StrOp(dst, kind, Box::new(regs)));
                                    if self.compile_blocking_target(&recv, dst, 0) {
                                        return true;
                                    }
                                }
                                self.truncate_insns(start);
                                self.next_reg = start_reg;
                                // fall through to the task/AST paths
                            }
                        }
                        if let ExprKind::Ident(h) = &func.kind {
                            let raw = Self::hier_raw_name(h);
                            // Step 9e: a bare name that is a CLASS method of
                            // the enclosing chain must not inline a
                            // same-named module task — class scope shadows
                            // module scope. Fall through to the expression
                            // path's CallMethod lowering.
                            let this_bounded = self.method_mode
                                && h.path.len() == 1
                                && h.path[0].selects.is_empty()
                                && self.class_method_names.contains(raw.as_str());
                            if !this_bounded && self.try_inline_task_args(&raw, args) {
                                return true;
                            }
                            if let Some(leaf) = raw.rsplit('.').next() {
                                if !this_bounded
                                    && leaf != raw
                                    && self.try_inline_task_args(leaf, args)
                                {
                                    return true;
                                }
                            }
                        }
                        let start = self.insns.len();
                        let start_reg = self.next_reg;
                        if self.compile_expr(e, 0).is_some() {
                            return true;
                        }
                        // A call can be emitted before its write-back
                        // declines; the fallback below runs it again.
                        self.truncate_insns(start);
                        self.next_reg = start_reg;
                        self.bail("Expr_Call_stmt");
                        return self.emit_fallback(stmt);
                    }
                    // class-perf Step 9c: `$cast(dest, src);` in statement
                    // position — the §8.16 task form (failure prints a
                    // runtime error, unlike the silent function form). Same
                    // insn as the expression form, `stmt_form` set.
                    ExprKind::SystemCall { name, args } if name == "$cast" && self.method_mode => {
                        if self.compile_cast_call(args, 0, true).is_some() {
                            return true;
                        }
                        self.bail("Expr_cast_stmt");
                        return self.emit_fallback(stmt);
                    }
                    _ => {}
                }
                // Void diagnostic system tasks ($display family, $finish,
                // $stop): they never produce a value, so route them through
                // the AST-fallback insn — the runtime interpreter prints
                // byte-identical output. Method bodies only (their
                // allow_ast_fallback is off, deliberately not consulted here:
                // these are leaf statements with no control-flow effect);
                // module code keeps `emit_fallback` below.
                if self.method_mode
                    && let ExprKind::SystemCall { name, .. } = &e.kind
                    && matches!(
                        name.as_str(),
                        "$display" | "$write" | "$strobe" | "$monitor" | "$finish" | "$stop"
                    )
                {
                    let locals = if self.fb_live() {
                        match self.fb_stmt_locals(stmt) {
                            Ok(l) => l,
                            Err(why) => {
                                self.bail(why);
                                return false;
                            }
                        }
                    } else {
                        FbLocals::default()
                    };
                    self.push_stmt_fallback(stmt, "Expr_syscall_void", locals);
                    return true;
                }
                let n: &'static str = match &e.kind {
                    ExprKind::SystemCall { name, .. } => match name.as_str() {
                        "$display" => "Expr_display",
                        "$write" => "Expr_write",
                        "$strobe" => "Expr_strobe",
                        "$monitor" => "Expr_monitor",
                        "$finish" => "Expr_finish",
                        "$stop" => "Expr_stop",
                        _ => "Expr_syscall_other",
                    },
                    ExprKind::Call { .. } => "Expr_Call",
                    ExprKind::Binary { .. } => "Expr_Binary",
                    ExprKind::Concatenation(_) => "Expr_Concat",
                    ExprKind::Replication { .. } => "Expr_Replication",
                    ExprKind::MemberAccess { .. } => "Expr_MemberAccess",
                    ExprKind::AssignmentPattern(_) => "Expr_AsgnPat",
                    ExprKind::TypedPattern { .. } => "Expr_TypedPat",
                    ExprKind::Index { .. } => "Expr_Index",
                    ExprKind::RangeSelect { .. } => "Expr_RangeSelect",
                    ExprKind::Conditional { .. } => "Expr_Conditional",
                    _ => "Expr_other",
                };
                self.bail(n);
                self.emit_fallback(stmt)
            }
            StatementKind::For {
                init,
                condition,
                step,
                body,
            } => {
                if self.method_mode
                    && init.iter().any(|i| {
                        matches!(i,
                    ForInit::VarDecl { data_type, .. } if self.class_alias_dynamic_width(data_type))
                    })
                {
                    self.bail("class_alias_runtime_width");
                    return false;
                }
                if self.try_reduce_loop(init, condition.as_ref(), step, body) {
                    return true;
                }
                let (vector_plans, vectorized_body) =
                    match self.full_range_nba_copy_plan(init, condition.as_ref(), step, body) {
                        Some((plans, pruned)) => (plans, Some(pruned)),
                        None => (Vec::new(), None),
                    };
                let body_to_compile = vectorized_body.as_ref().unwrap_or(body);
                // LRM §12.7 — `break`/`continue` are now compiled to direct
                // jumps; we push fresh patch lists on entry and apply them
                // once we know the step-start and loop-end addresses.
                self.loop_break_patches.push(Vec::new());
                self.loop_continue_patches.push(Vec::new());
                // Save outer for-loop overrides so nested loops don't leak.
                let saved_for_vars = std::mem::take(&mut self.for_loop_var_ids);
                let saved_locals = self.local_var_regs.clone();
                let mut reg_vars_registered: u32 = 0;
                // Inherit the outer overrides too — a nested loop's body
                // can still reference the outer counter.
                self.for_loop_var_ids = saved_for_vars.clone();
                for fi in init {
                    match fi {
                        ForInit::Assign { lvalue, rvalue } => {
                            let width = self.infer_lhs_width(lvalue);
                            let val_reg = match self.compile_expr(rvalue, width) {
                                Some(r) => r,
                                None => {
                                    self.bail("For_init_rvalue");
                                    return false;
                                }
                            };
                            if width > 0 && !self.expr_is_string_static(rvalue) {
                                self.emit(Insn::Resize(val_reg, width));
                            }
                            if !self.compile_blocking_target(lvalue, val_reg, width) {
                                self.bail("For_init_target");
                                return false;
                            }
                            // Capture init's lvalue signal_id keyed by leaf
                            // name. The for-loop's step / body expressions
                            // often re-parse bare-ident references that the
                            // elaborator did not scope-qualify (only init's
                            // lvalue gets qualified through an elaboration
                            // path). Without this, a bare `i` in step
                            // `i = i+1` collides with an unrelated top-level
                            // signal of the same name and resolves to the
                            // wrong signal_id. On c910 the always-block
                            // counter was clobbering the testbench's
                            // top-level `integer i` (signal_id 9), and the
                            // actual counter never advanced — the loop ran
                            // forever (10M+ insns per call, hung the sim
                            // in iter 1 of the event loop).
                            // Capture init's resolved signal_id keyed by the
                            // *leaf* of the lvalue's hier path. The
                            // elaborator may have rewritten init's lvalue
                            // from bare `i` to a multi-segment `module.i`
                            // form (which is why init resolves correctly
                            // to the module-local id), while leaving the
                            // for-step's bare `i` untouched. Capturing by
                            // leaf bridges that asymmetry: bare `i` in step
                            // gets re-routed to init's resolved id.
                            if let ExprKind::Ident(hier) = &lvalue.kind {
                                let leaf = if hier.path.len() == 1
                                    && hier.path[0].name.name.contains('.')
                                {
                                    // Parser flattened a hier path into one segment with dots.
                                    hier.path[0]
                                        .name
                                        .name
                                        .rsplit('.')
                                        .next()
                                        .unwrap_or("")
                                        .to_string()
                                } else {
                                    hier.path
                                        .last()
                                        .map(|s| s.name.name.clone())
                                        .unwrap_or_default()
                                };
                                if !leaf.is_empty() && !leaf.contains('.') {
                                    if let Some(id) = self.lookup_signal_id(hier) {
                                        self.for_loop_var_ids.insert(leaf, id);
                                    }
                                }
                            }
                        }
                        // NOTE the ORDER: the register-backed path (below)
                        // keeps every loop the specialized fast paths
                        // (vectorized packed-copy, fused array NBA) know how
                        // to handle; the unroller is the CATCH-ALL for bodies
                        // they cannot compile at all — task/function calls,
                        // register-bank locals, const-index folding.
                        ForInit::VarDecl {
                            data_type,
                            name,
                            init,
                        } if self.for_body_is_simple(body) && !self.stmt_touches_reg_bank(body) => {
                            // §12.7.1: `for (int i = ...)` — the loop var
                            // lives in a VM REGISTER (it has no signal).
                            // Body/step reads resolve through local_var_regs,
                            // which compile_expr consults BEFORE the signal
                            // tables, so a same-named outer signal can never
                            // capture. Array indexing through the register is
                            // safe: the register path (Nba/BlockingAssignArray)
                            // takes a RegId, and the SigId-fusion peepholes
                            // pattern-match LoadSignal, which a register-backed
                            // index never emits. This bail was 83% of a
                            // customer run's wall time (For_init_vardecl,
                            // 204µs per AST execution of a lane-copy loop).
                            let w = self.decl_width(data_type);
                            let slot = self.alloc_reg();
                            let Some(v) = self.compile_expr(init, w) else {
                                self.for_loop_var_ids = saved_for_vars;
                                self.local_var_regs = saved_locals;
                                self.bail("For_init_vardecl_rvalue");
                                return false;
                            };
                            self.emit(Insn::Move(slot, v));
                            // §6.12.1: an integral loop variable rounds a
                            // real initial value.
                            if !self.decl_is_real(data_type, None) {
                                self.emit_real_to_int_fit(slot, init, w);
                            }
                            if w > 0 {
                                self.emit(Insn::Resize(slot, w));
                            }
                            // §6.11: int/byte/shortint/longint/integer are
                            // SIGNED by default — the init literal may not be
                            // (`for (int i = 4'hF; ...)`), and an unsigned
                            // slot makes `i >= 0` never terminate / negative
                            // comparisons go unsigned.
                            let decl_signed = crate::compiler::elaborate::is_type_signed(
                                self.resolved_decl_type(data_type, None),
                            );
                            if decl_signed {
                                self.emit(Insn::SetSigned(slot));
                            } else {
                                self.emit(Insn::ClearSigned(slot));
                            }
                            if let Some(k) = self.local_kind_of(data_type, None) {
                                self.local_kinds.insert(slot, k);
                                self.drop_xz_new_two_state_local(slot);
                            }
                            self.local_var_regs.insert(name.name.clone(), (slot, w));
                            self.reg_var_loop_depth += 1;
                            reg_vars_registered += 1;
                        }
                        // Catch-all: bodies the specialized paths above
                        // decline (task/function calls, register-bank locals,
                        // const-index folding) unroll with a const-bound loop
                        // variable.
                        #[allow(unreachable_patterns)]
                        ForInit::VarDecl {
                            name,
                            init,
                            data_type,
                        } if self.try_unroll_for(name, data_type, init, condition, step, body) => {
                            self.for_loop_var_ids = saved_for_vars;
                            self.local_var_regs = saved_locals;
                            return true;
                        }
                        #[allow(unreachable_patterns)]
                        ForInit::VarDecl {
                            data_type,
                            name,
                            init,
                        } if self.method_mode => {
                            // General loop-scoped declaration (METHOD bodies
                            // only): bind the induction variable to a plain
                            // register (no reg-bank fast path, no unrolling)
                            // and let the ordinary loop skeleton below drive
                            // it. Module-process loops keep the AST path —
                            // exec_statement's vectorized collapses (packed
                            // fill / copy, fused array NBA) fire there and a
                            // compiled loop would bypass them.
                            let w = self.decl_width(data_type);
                            let slot = self.alloc_reg();
                            let Some(v) = self.compile_expr(init, w) else {
                                self.for_loop_var_ids = saved_for_vars;
                                self.local_var_regs = saved_locals;
                                self.bail("For_init_vardecl_rvalue");
                                return false;
                            };
                            self.emit(Insn::Move(slot, v));
                            // §6.12.1: an integral loop variable rounds a
                            // real initial value.
                            if !self.decl_is_real(data_type, None) {
                                self.emit_real_to_int_fit(slot, init, w);
                            }
                            if w > 0 {
                                self.emit(Insn::Resize(slot, w));
                            }
                            let decl_signed = crate::compiler::elaborate::is_type_signed(
                                self.resolved_decl_type(data_type, None),
                            );
                            if decl_signed {
                                self.emit(Insn::SetSigned(slot));
                            } else {
                                self.emit(Insn::ClearSigned(slot));
                            }
                            if let Some(k) = self.local_kind_of(data_type, None) {
                                self.local_kinds.insert(slot, k);
                                self.drop_xz_new_two_state_local(slot);
                            }
                            self.local_var_regs.insert(name.name.clone(), (slot, w));
                        }
                        #[allow(unreachable_patterns)]
                        ForInit::VarDecl { .. } => {
                            self.for_loop_var_ids = saved_for_vars;
                            self.local_var_regs = saved_locals;
                            self.bail("For_init_vardecl");
                            return false;
                        }
                    }
                }
                // The plan proves this canonical loop executes its complete
                // packed range. Queue each identity copy once at the point
                // where its first iteration would sample the source.
                for &(dst, src, width) in &vector_plans {
                    let r = self.alloc_reg();
                    self.emit(Insn::LoadSignal(r, as_sig_id(src)));
                    self.emit(Insn::NbaAssign(as_sig_id(dst), r, width));
                }
                let loop_start = self.insns.len() as u32;
                let cond_branch_idx = if let Some(c) = condition {
                    let cond_reg = match self.compile_expr(c, 0) {
                        Some(r) => r,
                        None => {
                            self.bail("For_condition");
                            self.for_loop_var_ids = saved_for_vars;
                            self.local_var_regs = saved_locals;
                            self.reg_var_loop_depth -=
                                reg_vars_registered.min(self.reg_var_loop_depth);
                            return false;
                        }
                    };
                    let idx = self.insns.len();
                    self.emit(Insn::BranchIfFalse(cond_reg, 0));
                    Some(idx)
                } else {
                    None
                };
                let bounds = self.proven_loop_index(init, condition.as_ref(), step, body_to_compile);
                if let Some((reg, range)) = bounds {
                    self.loop_index_bounds.insert(reg, range);
                }
                let compiled = self.compile_stmt(body_to_compile);
                if let Some((reg, _)) = bounds {
                    self.loop_index_bounds.remove(&reg);
                }
                if !compiled {
                    // Bail path — pop patches so they don't leak.
                    self.loop_break_patches.pop();
                    self.loop_continue_patches.pop();
                    self.for_loop_var_ids = saved_for_vars;
                    self.local_var_regs = saved_locals;
                    self.reg_var_loop_depth -= reg_vars_registered.min(self.reg_var_loop_depth);
                    return false;
                }
                let step_start = self.insns.len() as u32;
                // `continue` jumps to the step (or to loop_start if there is
                // no step) — patch now.
                let cont_targ = if step.is_empty() {
                    loop_start
                } else {
                    step_start
                };
                if let Some(patches) = self.loop_continue_patches.pop() {
                    for idx in patches {
                        self.insns[idx] = Insn::Jump(cont_targ);
                    }
                }
                for s in step {
                    // For-loop step can be either the legacy `Binary{Assign,…}`
                    // shape or the newer `AssignExpr { lvalue, rvalue }` emitted
                    // by the parser for `i = i+1` / `i += 2` / etc. after
                    // xezim-core 8b9c88c (ibex parsing). Both collapse to a
                    // blocking assign.
                    // `i++` / `++i` / `i--` / `--i` on a REGISTER-backed block
                    // local: increment in place. (The signal-backed case is
                    // handled by the generic assign shapes below.)
                    if let ExprKind::Unary { op, operand } = &s.kind {
                        let delta: i64 = match op {
                            UnaryOp::PostIncr | UnaryOp::PreIncr => 1,
                            UnaryOp::PostDecr | UnaryOp::PreDecr => -1,
                            _ => 0,
                        };
                        if delta != 0 {
                            if let ExprKind::Ident(h) = &operand.kind {
                                if let Some((slot, dw)) = self.local_var_reg_of(h) {
                                    let one = self.alloc_reg();
                                    let w = if dw > 0 { dw } else { 32 };
                                    // SIGNED one: signed+signed stays signed
                                    // (an unsigned 1 silently stripped the
                                    // loop var's sign on the first step, so
                                    // `i >= -2` compared unsigned);
                                    // signed+unsigned still yields unsigned,
                                    // so unsigned loop vars are unaffected.
                                    let mut one_v = Value::from_u64(1, w);
                                    one_v.is_signed = true;
                                    self.emit(Insn::LoadConst(one, Box::new(one_v)));
                                    let dst = self.alloc_reg();
                                    self.emit(Insn::Move(dst, slot));
                                    if delta > 0 {
                                        self.emit(Insn::Add(dst, dst, one));
                                    } else {
                                        self.emit(Insn::Sub(dst, dst, one));
                                    }
                                    if w > 0 {
                                        self.emit(Insn::Resize(dst, w));
                                    }
                                    self.emit(Insn::Move(slot, dst));
                                    continue;
                                }
                                // SIGNAL-backed loop counter (`int i;` at
                                // module/block scope): load, ±1, store.
                                // Previously bailed the whole loop to the AST
                                // path ("For_step_other") — ~30µs per edge.
                                if let Some(id) = self
                                    .lookup_signal_id(h)
                                    .filter(|_| self.for_body_is_simple(body))
                                {
                                    let w =
                                        self.signal_widths.get(id).copied().unwrap_or(32).max(1);
                                    let cur = self.alloc_reg();
                                    self.emit(Insn::LoadSignal(cur, id as u32));
                                    let one = self.alloc_reg();
                                    let mut one_v = Value::from_u64(1, w);
                                    one_v.is_signed = true; // see register arm
                                    self.emit(Insn::LoadConst(one, Box::new(one_v)));
                                    if delta > 0 {
                                        self.emit(Insn::Add(cur, cur, one));
                                    } else {
                                        self.emit(Insn::Sub(cur, cur, one));
                                    }
                                    self.emit(Insn::Resize(cur, w));
                                    self.emit(Insn::BlockingAssign(id as u32, cur, w));
                                    continue;
                                }
                            }
                        }
                    }
                    let (lhs, rhs) = match &s.kind {
                        ExprKind::Binary {
                            op: BinaryOp::Assign,
                            left,
                            right,
                        } => (&**left, &**right),
                        ExprKind::AssignExpr { lvalue, rvalue } => (&**lvalue, &**rvalue),
                        _ => {
                            self.bail("For_step_other");
                            return false;
                        }
                    };
                    let width = self.infer_lhs_width(lhs);
                    let val_reg = match self.compile_expr(rhs, width) {
                        Some(r) => r,
                        None => {
                            self.bail("For_step_rvalue");
                            return false;
                        }
                    };
                    if width > 0 && !self.expr_is_string_static(rhs) {
                        self.emit(Insn::Resize(val_reg, width));
                    }
                    if !self.compile_blocking_target(lhs, val_reg, width) {
                        self.bail("For_step_target");
                        return false;
                    }
                }
                self.emit(Insn::Jump(loop_start));
                let end = self.insns.len() as u32;
                if let Some(idx) = cond_branch_idx {
                    if let Insn::BranchIfFalse(reg, _) = self.insns[idx] {
                        self.insns[idx] = Insn::BranchIfFalse(reg, end);
                    }
                }
                // `break` jumps to the loop-exit address.
                if let Some(patches) = self.loop_break_patches.pop() {
                    for idx in patches {
                        self.insns[idx] = Insn::Jump(end);
                    }
                }
                // Restore outer for-loop's override map and block locals.
                self.for_loop_var_ids = saved_for_vars;
                self.local_var_regs = saved_locals;
                self.reg_var_loop_depth -= reg_vars_registered.min(self.reg_var_loop_depth);
                if !vector_plans.is_empty() {
                    PACKED_LOOP_NBA_COPIES.fetch_add(
                        vector_plans.len() as u64,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                }
                true
            }
            StatementKind::Disable(target) => {
                // LRM §9.6.2: disable <named_block> terminates execution of that block.
                // If the block is an active enclosing named block in this compiled unit,
                // emit a forward Jump(0) and record the patch.
                let name = crate::ast::stmt::disable_name(target);
                // A path (`g1.blk`) may name another scope's block of the same
                // label: the interpreter resolves it (§9.6.2).
                if target.path.len() == 1 && self.named_block_patches.contains_key(name) {
                    let idx = self.insns.len();
                    self.emit(Insn::Jump(0));
                    self.named_block_patches.get_mut(name).unwrap().push(idx);
                    true
                } else {
                    self.bail("Disable");
                    self.emit_fallback(stmt)
                }
            }
            StatementKind::Break => {
                // LRM §12.7 — exits innermost enclosing loop. Compiled as a
                // forward Jump(0) patched after the loop body+step finish.
                // Outside a loop in the compiled scope: bail so the AST path
                // can produce the right diagnostic.
                if self.loop_break_patches.last().is_some() {
                    let idx = self.insns.len();
                    self.emit(Insn::Jump(0));
                    self.loop_break_patches.last_mut().unwrap().push(idx);
                    true
                } else {
                    self.bail("Break_outside_loop");
                    self.emit_fallback(stmt)
                }
            }
            StatementKind::Continue => {
                // LRM §12.7 — restart innermost enclosing loop at its step.
                if self.loop_continue_patches.last().is_some() {
                    let idx = self.insns.len();
                    self.emit(Insn::Jump(0));
                    self.loop_continue_patches.last_mut().unwrap().push(idx);
                    true
                } else {
                    self.bail("Continue_outside_loop");
                    self.emit_fallback(stmt)
                }
            }
            StatementKind::While { condition, body } => {
                // §12.7.2: a while is a For with no init and no step. The
                // condition is re-evaluated at the loop head each iteration;
                // `continue` jumps back to the head, `break` to the end.
                // Compiling it (rather than bailing "Stmt_While") is what
                // lets a pure while-loop helper inline (issue #146) — the
                // purity arm alone would only have moved the bail here.
                self.loop_break_patches.push(Vec::new());
                self.loop_continue_patches.push(Vec::new());
                let top = self.insns.len() as u32;
                let Some(c) = self.compile_expr(condition, 0) else {
                    self.loop_break_patches.pop();
                    self.loop_continue_patches.pop();
                    self.bail("While_cond");
                    return false;
                };
                let br = self.insns.len();
                self.emit(Insn::BranchIfFalse(c, 0));
                if !self.compile_stmt(body) {
                    self.loop_break_patches.pop();
                    self.loop_continue_patches.pop();
                    return false;
                }
                self.emit(Insn::Jump(top));
                let end = self.insns.len() as u32;
                if let Insn::BranchIfFalse(reg, _) = self.insns[br] {
                    self.insns[br] = Insn::BranchIfFalse(reg, end);
                }
                if let Some(patches) = self.loop_continue_patches.pop() {
                    for idx in patches {
                        self.insns[idx] = Insn::Jump(top);
                    }
                }
                if let Some(patches) = self.loop_break_patches.pop() {
                    for idx in patches {
                        self.insns[idx] = Insn::Jump(end);
                    }
                }
                true
            }
            StatementKind::DoWhile { body, condition } => {
                // Body first, then the condition; `continue` re-tests the
                // condition (§12.7.4), `break` exits.
                self.loop_break_patches.push(Vec::new());
                self.loop_continue_patches.push(Vec::new());
                let top = self.insns.len() as u32;
                if !self.compile_stmt(body) {
                    self.loop_break_patches.pop();
                    self.loop_continue_patches.pop();
                    return false;
                }
                let cond_at = self.insns.len() as u32;
                let Some(c) = self.compile_expr(condition, 0) else {
                    self.loop_break_patches.pop();
                    self.loop_continue_patches.pop();
                    self.bail("DoWhile_cond");
                    return false;
                };
                self.emit(Insn::BranchUnlessZero(c, top));
                let end = self.insns.len() as u32;
                if let Some(patches) = self.loop_continue_patches.pop() {
                    for idx in patches {
                        self.insns[idx] = Insn::Jump(cond_at);
                    }
                }
                if let Some(patches) = self.loop_break_patches.pop() {
                    for idx in patches {
                        self.insns[idx] = Insn::Jump(end);
                    }
                }
                true
            }
            StatementKind::Repeat { count, body } => {
                // §12.7.5: repeat (count) body
                // Lowered as an internal countdown loop to 0.
                // §12.7.2: a real count rounds to an integer (ties away from
                // zero). compile_expr(count,32) would ceiling the real to an
                // int (10.4 -> 11), so let the interpreter's `repeat_count`
                // handle real counts — mirrors the Process-FSM path's
                // `bail("Repeat_real_count")` above.
                if self.expr_may_be_real(count) {
                    self.bail("Repeat_real_count");
                    return false;
                }
                let Some(cnt_reg) = self.compile_expr(count, 32) else {
                    self.bail("Repeat_count");
                    return false;
                };
                let iter_reg = self.alloc_reg();
                self.emit(Insn::Move(iter_reg, cnt_reg));
                let zero_reg = self.alloc_reg();
                self.emit(Insn::LoadConst(zero_reg, Box::new(Value::from_u64(0, 32))));
                let one_reg = self.alloc_reg();
                self.emit(Insn::LoadConst(one_reg, Box::new(Value::from_u64(1, 32))));

                // §12.7.2/§6.12.2: a signed-negative count (e.g. `repeat(-1)`)
                // means ZERO iterations — mirrors the interpreter's
                // `repeat_count`, which clamps signed values with `.max(0)`.
                // Without this, `repeat(-1)` reads back as 0xFFFFFFFF
                // (non-zero) and the Gt-based loop below spins ~2^32 times.
                let neg_rs = self.alloc_reg();
                let signed_zero_reg = self.alloc_reg();
                let mut zsv = Value::from_u64(0, 32);
                zsv.is_signed = true;
                self.emit(Insn::LoadConst(signed_zero_reg, Box::new(zsv)));
                // Signed compare: `Lt` is signed only when BOTH operands are.
                self.emit(Insn::Lt(neg_rs, iter_reg, signed_zero_reg));
                let neg_jmp = self.insns.len();
                // `BranchUnlessZero` jumps on a definite 1; an X count falls
                // through to the Gt condition below, which already exits on X.
                self.emit(Insn::BranchUnlessZero(neg_rs, 0));

                self.loop_break_patches.push(Vec::new());
                self.loop_continue_patches.push(Vec::new());

                let top = self.insns.len() as u32;
                let cond_reg = self.alloc_reg();
                self.emit(Insn::Gt(cond_reg, iter_reg, zero_reg));
                let br = self.insns.len();
                self.emit(Insn::BranchIfFalse(cond_reg, 0));

                if !self.compile_stmt(body) {
                    self.loop_break_patches.pop();
                    self.loop_continue_patches.pop();
                    return false;
                }

                let step_top = self.insns.len() as u32;
                self.emit(Insn::Sub(iter_reg, iter_reg, one_reg));
                self.emit(Insn::Jump(top));

                let end = self.insns.len() as u32;
                if let Insn::BranchIfFalse(reg, _) = self.insns[br] {
                    self.insns[br] = Insn::BranchIfFalse(reg, end);
                }
                self.insns[neg_jmp] = Insn::BranchUnlessZero(neg_rs, end);
                if let Some(patches) = self.loop_continue_patches.pop() {
                    for idx in patches {
                        self.insns[idx] = Insn::Jump(step_top);
                    }
                }
                if let Some(patches) = self.loop_break_patches.pop() {
                    for idx in patches {
                        self.insns[idx] = Insn::Jump(end);
                    }
                }
                true
            }
            StatementKind::Forever { body } => {
                // §12.7.1: forever body
                // Infinite loop: jumps to top at the end; break exits, continue loops.
                self.loop_break_patches.push(Vec::new());
                self.loop_continue_patches.push(Vec::new());
                let top = self.insns.len() as u32;
                if !self.compile_stmt(body) {
                    self.loop_break_patches.pop();
                    self.loop_continue_patches.pop();
                    return false;
                }
                self.emit(Insn::Jump(top));
                let end = self.insns.len() as u32;
                if let Some(patches) = self.loop_continue_patches.pop() {
                    for idx in patches {
                        self.insns[idx] = Insn::Jump(top);
                    }
                }
                if let Some(patches) = self.loop_break_patches.pop() {
                    for idx in patches {
                        self.insns[idx] = Insn::Jump(end);
                    }
                }
                true
            }
            StatementKind::Foreach { array, vars, body } => {
                // class-perf Step 9d FIRST: `foreach` over a class MEMBER
                // collection (queue / dynamic / associative) inside a
                // compiled method — a REAL bytecode loop over a runtime
                // key snapshot. Admission reuses the 9b-ii element-access
                // probe (`member_coll_elem_target`): bare receivers must
                // be in the collision-excluded `coll_elem_member_names`
                // set and unshadowed; dotted receivers must pass
                // `method_handle_chain_ok` with the member in
                // `coll_member_names`. Queue/dynamic-array FORMALS and
                // locals land in `local_var_regs`, so they decline here
                // and keep the AST path. Everything else (multi-var,
                // packed, `$`-shaped, local collections) falls through to
                // the existing arms below.
                if let Some(outcome) = self.try_compile_member_foreach(array, vars, body) {
                    return outcome;
                }
                // §12.7.3 foreach over a register-bound local array — an
                // inlined resolver's dynamic-array formal or a #129 local
                // buffer. The element count is a compile-time constant, so
                // the loop UNROLLS with the loop variable bound both as a
                // register (value uses) and as a constant (index uses fold
                // to a direct element register via const_var_binds).
                let arr_name = match &array.kind {
                    ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                        h.path[0].name.name.clone()
                    }
                    _ => {
                        self.bail("Stmt_Foreach");
                        return false;
                    }
                };
                let Some(ab) = self.local_var_array.get(&arr_name).cloned() else {
                    self.bail("Stmt_Foreach");
                    return false;
                };
                let var = match vars.as_slice() {
                    [Some(v)] => v.name.clone(),
                    _ => {
                        self.bail("Stmt_Foreach_vars");
                        return false;
                    }
                };
                if ab.lo < 0 || Self::stmt_has_break_or_continue(body) {
                    // §12.7 break/continue jump-patch lists belong to REAL
                    // compiled loops; an unrolled iteration has no loop to
                    // exit. Rolls back to one AST-interpreted unit instead.
                    self.bail("Stmt_Foreach_break");
                    return false;
                }
                let var_reg = self.alloc_reg();
                let saved_local = self.local_var_regs.insert(var.clone(), (var_reg, 32));
                // §12.7.3: a fixed-size array's loop variable is an `int`. A
                // fallback in the body carries it (`fb_carry`), read-only —
                // the rest of the iteration folds it as a constant.
                self.local_kinds.insert(
                    var_reg,
                    LocalKind::Int {
                        width: 32,
                        signed: true,
                        two_state: true,
                    },
                );
                let saved_const = self.const_var_binds.get(&var).copied();
                let mut ok = true;
                for k in 0..ab.regs.len() {
                    let idx = (ab.lo + k as i64) as u64;
                    self.emit(Insn::LoadConst(var_reg, Box::new(Value::from_u64(idx, 32))));
                    self.const_var_binds.insert(var.clone(), idx);
                    if !self.compile_stmt(body) {
                        ok = false;
                        break;
                    }
                }
                match saved_const {
                    Some(v) => {
                        self.const_var_binds.insert(var.clone(), v);
                    }
                    None => {
                        self.const_var_binds.remove(&var);
                    }
                }
                match saved_local {
                    Some(b) => {
                        self.local_var_regs.insert(var, b);
                    }
                    None => {
                        self.local_var_regs.remove(&var);
                    }
                }
                if !ok {
                    self.bail("Stmt_Foreach_body");
                    return false;
                }
                true
            }
            other => {
                let name: &'static str = match other {
                    StatementKind::Expr(_) => "Expr",
                    StatementKind::For { .. } => "For",
                    StatementKind::Foreach { .. } => "Foreach",
                    StatementKind::While { .. } => "While",
                    StatementKind::DoWhile { .. } => "DoWhile",
                    StatementKind::Repeat { .. } => "Repeat",
                    StatementKind::Forever { .. } => "Forever",
                    StatementKind::TimingControl { .. } => "TimingControl",
                    StatementKind::EventTrigger { .. } => "EventTrigger",
                    StatementKind::Wait { .. } => "Wait",
                    StatementKind::WaitFork => "WaitFork",
                    StatementKind::Disable(_) => "Disable",
                    StatementKind::Return(_) => "Return",
                    StatementKind::Break => "Break",
                    StatementKind::Continue => "Continue",
                    StatementKind::Assertion(_) => "Assertion",
                    StatementKind::ProceduralContinuous(_) => "ProceduralContinuous",
                    StatementKind::VarDecl { .. } => "VarDecl",
                    StatementKind::ParamDecl { .. } => "ParamDecl",
                    StatementKind::Coverpoint { .. } => "Coverpoint",
                    StatementKind::Cross { .. } => "Cross",
                    _ => "Other",
                };
                self.bail_reason = Some(name);
                self.emit_fallback(stmt)
            }
        }
    }

    /// Compile an expression, returning the register holding the result.
    /// Returns None if the expression can't be compiled to bytecode.
    /// class-perf method-mode: does `expr` (single-segment, rootless Ident)
    /// name a CLASS-SCOPE entity the compiled path cannot model (shadowed
    /// member, static, localparam, array/struct member)? Locals and loadable
    /// bare members never match.
    pub(super) fn method_bares_to_shadowed(&self, expr: &Expression) -> bool {
        if !self.method_mode {
            return false;
        }
        let ExprKind::Ident(h) = &expr.kind else {
            return false;
        };
        if h.root.is_some() || h.path.len() != 1 {
            return false;
        }
        let bare = h.path[0].name.name.as_str();
        !self.local_var_regs.contains_key(bare)
            && !self.local_array_regs.contains_key(bare)
            && self.class_shadow_names.contains(bare)
    }

    /// Step 9g: is `e` a `new` construction shape — bare `new` or a
    /// call whose func is the unqualified Ident `new`?
    pub(super) fn is_new_shape(e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Ident(h) => {
                h.root.is_none() && h.path.len() == 1 && h.path[0].name.name == "new"
            }
            ExprKind::Call { func, .. } => matches!(
                &func.kind,
                ExprKind::Ident(h)
                    if h.root.is_none()
                        && h.path.len() == 1
                        && h.path[0].name.name == "new"
            ),
            _ => false,
        }
    }

    /// Step 9g: the DECLARED class type of a bare lvalue name (§8.8 —
    /// `lhs = new` constructs the lhs's class), when admitted.
    fn new_target_for(&self, lhs: &Expression) -> Option<String> {
        if !self.method_mode {
            return None;
        }
        if let ExprKind::Ident(h) = &lhs.kind
            && h.root.is_none()
            && h.path.len() == 1
            && h.path[0].selects.is_empty()
        {
            return self.new_type_map.get(h.path[0].name.name.as_str()).cloned();
        }
        None
    }

    /// Step 9g: lower `new` / `new(args)` for a KNOWN target class into
    /// ConstructObject (allocation + ctor chain stay in instantiate_class).
    pub(super) fn compile_new_construct(
        &mut self,
        expr: &Expression,
        class: &str,
    ) -> Option<RegId> {
        let args: Vec<&Expression> = match &expr.kind {
            ExprKind::Ident(..) => Vec::new(),
            ExprKind::Call { args, .. } => args.iter().collect(),
            _ => return None,
        };
        // §8.7, §13.5.1: an unpacked-array formal of the constructor.
        if self.coll_formal_actual(Some(class), "new", args.iter().copied()) {
            self.bail("method_coll_literal_actual");
            return None;
        }
        let call_start = self.insns.len();
        let call_next = self.next_reg;
        let mut arg_values: Vec<RegId> = Vec::with_capacity(args.len());
        // §13.5.3: an empty actual (`new(a, , c)`) takes its default; the
        // positions travel with the class name (`class\0mask`).
        let mut empty_mask: u64 = 0;
        for (i, a) in args.iter().enumerate() {
            if matches!(a.kind, ExprKind::Empty) {
                if i >= 64 {
                    self.truncate_insns(call_start);
                    self.next_reg = call_next;
                    self.bail("Expr_New_args");
                    return None;
                }
                empty_mask |= 1 << i;
                let r = self.alloc_reg();
                self.emit(Insn::LoadConst(r, Box::new(Value::zero(1))));
                arg_values.push(r);
                continue;
            }
            match self.compile_expr(a, 0) {
                Some(r) => arg_values.push(r),
                None => {
                    self.truncate_insns(call_start);
                    self.next_reg = call_next;
                    self.bail("Expr_New_args");
                    return None;
                }
            }
        }
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
        let name = if empty_mask == 0 {
            class.to_string()
        } else {
            format!("{class}\u{0}{empty_mask}")
        };
        self.emit(Insn::ConstructObject(
            dest,
            Box::new(Name(name.into_boxed_str())),
            arg_start,
            n,
        ));
        Some(dest)
    }
}
