//! Expression compilation.

use super::*;

impl<'a> BytecodeCompiler<'a> {
    pub(super) fn compile_expr(&mut self, expr: &Expression, ctx_width: u32) -> Option<RegId> {
        // §8.8: a bare typed constructor `C::new` constructs, as `C::new()`
        // does, rather than reading a class-scope name.
        if !matches!(expr.kind, ExprKind::Call { .. })
            && let Some((prefix, _)) = crate::compiler::simulator::Simulator::typed_ctor_call(expr)
            && self.scope_static_receivers.contains(prefix)
        {
            let call = Expression::new(
                ExprKind::Call {
                    func: Box::new(expr.clone()),
                    args: Vec::new(),
                },
                expr.span,
            );
            return self.compile_expr_typed(&call, ctx_width, false);
        }
        self.compile_expr_typed(expr, ctx_width, false)
    }

    /// §11.8.2: an unsigned expression changes operand type before any
    /// context widening or nested operation, rather than after it finishes.
    pub(super) fn compile_expr_typed(
        &mut self,
        expr: &Expression,
        ctx_width: u32,
        unsigned_context: bool,
    ) -> Option<RegId> {
        if let Some(q) = self.own_pkg_qualified(expr) {
            return self.compile_expr_typed(&q, ctx_width, unsigned_context);
        }
        // §10.8: only a pattern, or a parenthesised or conditional operand
        // holding one, takes the context type; other operands start without.
        let pattern_ctx = self.pattern_ctx.take();
        if matches!(expr.kind, ExprKind::TypedPattern { .. })
            || (pattern_ctx.is_some()
                && crate::compiler::packed_assignment_pattern_plan::has_pattern(expr))
        {
            return self.compile_pattern(expr, ctx_width, pattern_ctx);
        }
        // §13.5.2: a call lowering can emit its call before a write-back of
        // an output or inout actual declines it. Whatever then runs the
        // expression in the interpreter (a statement fallback, `void'(...)`)
        // makes the call itself, so the compiled one must not stay.
        let call_start =
            matches!(expr.kind, ExprKind::Call { .. }).then(|| (self.insns.len(), self.next_reg));
        let Some(result) = self.compile_expr_inner(expr, ctx_width, unsigned_context) else {
            if let Some((start, first_reg)) = call_start {
                self.truncate_insns(start);
                self.next_reg = first_reg;
            }
            return None;
        };
        // §7.4.1: an element of a packed array of a named signed type
        // (`lane_t [0:8] x; x[1]`) is signed; the slice that read it is not.
        if matches!(expr.kind, ExprKind::Index { .. }) && self.packed_named_select_signed(expr) {
            self.emit(Insn::SetSigned(result));
        }
        let propagates = matches!(
            expr.kind,
            ExprKind::Binary { .. }
                | ExprKind::Conditional { .. }
                | ExprKind::Paren(_)
                | ExprKind::Unary {
                    op: UnaryOp::Plus | UnaryOp::Minus | UnaryOp::BitNot,
                    ..
                }
        );
        if unsigned_context && !propagates && self.expr_signedness(expr) != Some(false) {
            self.emit(Insn::ClearSigned(result));
        }
        Some(result)
    }

    fn compile_expr_inner(
        &mut self,
        expr: &Expression,
        ctx_width: u32,
        unsigned_context: bool,
    ) -> Option<RegId> {
        // §7.10: `$` is the last index of the queue its select indexes. The
        // interpreter evaluates a select chain holding one whole, binding each
        // `$` to its own base.
        if matches!(
            expr.kind,
            ExprKind::Index { .. } | ExprKind::RangeSelect { .. } | ExprKind::MemberAccess { .. }
        ) && self.inst_array_multidim_elem(expr)
        {
            // #1108: an element of a multidimensional array member of an
            // instance-array element (`lane[0].nest[1][2]`) is resolved by
            // the interpreter, from the array's stored name.
            return self.emit_expr_fallback(expr, ctx_width, "inst_array_member");
        }
        if matches!(
            expr.kind,
            ExprKind::Index { .. } | ExprKind::RangeSelect { .. } | ExprKind::MemberAccess { .. }
        ) && crate::compiler::simulator::Simulator::select_mentions_dollar(expr)
        {
            let dest = self.emit_expr_fallback(expr, ctx_width, "dollar_select");
            if dest.is_none() {
                self.bail("dollar_select");
            }
            return dest;
        }
        if let Some(id) = self.const_multi_dim_array_elem_signal_id(expr) {
            let dest = self.alloc_reg();
            if self.signal_signed[id] {
                self.emit(Insn::LoadSignalSigned(dest, as_sig_id(id)));
            } else {
                self.emit(Insn::LoadSignal(dest, as_sig_id(id)));
            }
            return Some(dest);
        }
        let whole = expr;
        match &expr.kind {
            ExprKind::Number(num) => {
                let val = self.eval_number_static(num)?;
                let r = self.alloc_reg();
                self.emit(Insn::LoadConst(r, Box::new(val)));
                Some(r)
            }
            ExprKind::Null => {
                // §6.15: null is the default (invalid) handle — the
                // interpreter evaluates it as a 32-bit zero.
                let r = self.alloc_reg();
                self.emit(Insn::LoadConst(r, Box::new(Value::zero(32))));
                Some(r)
            }
            ExprKind::This => {
                // Step 6 (method mode): `this` evaluates to the receiving
                // object's heap handle — a register Move from the method's
                // this-register. Covers `sched = this`, `return this`, and
                // method calls on `this`.
                if !self.method_mode {
                    self.bail("Expr_This");
                    return None;
                }
                let this_reg = self.method_this_reg?;
                let r = self.alloc_reg();
                self.emit(Insn::Move(r, this_reg));
                Some(r)
            }
            ExprKind::Ident(hier) => {
                // An UNROLLED loop variable is a compile-time constant and
                // shadows everything else.
                if hier.root.is_none() && hier.path.len() == 1 && hier.path[0].selects.is_empty() {
                    if let Some(v) = self.local_const_vars.get(&hier.path[0].name.name).cloned() {
                        let r = self.alloc_reg();
                        self.emit(Insn::LoadConst(r, Box::new(v)));
                        return Some(r);
                    }
                }
                // §26.3: a package constant two packages both declare has a
                // shared bare signal twin (last package registered wins);
                // the compiled unit's import decides which one it names.
                // §26.3: `pkg.member` — the inliner's spelling of an enum
                // member two packages both declare — is a constant of THAT
                // package; resolved by name it landed on the bare signal twin.
                if hier.path.len() == 2
                    && let Some(v) = self.pkg_enum_const_of(hier)
                {
                    let r = self.alloc_reg();
                    self.emit(Insn::LoadConst(r, Box::new(v)));
                    return Some(r);
                }
                // A register-backed block local (a for-loop variable) shadows
                // any same-named signal for the duration of its loop.
                if let Some((src, _)) = self.local_var_reg_of(hier) {
                    let r = self.alloc_reg();
                    self.emit(Insn::Move(r, src));
                    return Some(r);
                }
                if let Some(r) = self.compile_formal_struct_member(expr) {
                    return Some(r);
                }
                // Element of a register-bound LOCAL ARRAY (`row[n]` parsed as
                // an Ident with one select).
                if !self.local_var_array.is_empty()
                    && hier.path.len() == 1
                    && hier.path[0].selects.len() == 1
                    && self
                        .local_var_array
                        .contains_key(hier.path[0].name.name.as_str())
                {
                    let name = hier.path[0].name.name.clone();
                    let idx = hier.path[0].selects[0].clone();
                    if let Some(r) = self.compile_local_array_read(&name, &idx) {
                        return Some(r);
                    }
                    self.bail("local_array_read");
                    return None;
                }
                // Dynamic element of a 2-D unpacked array, Ident-with-selects
                // shape (`TBL[i][j]` inside an inlined body).
                if hier.path.len() == 1 && hier.path[0].selects.len() == 2 {
                    let i_e = hier.path[0].selects[0].clone();
                    let j_e = hier.path[0].selects[1].clone();
                    let mut bare = hier.clone();
                    bare.path[0].selects.clear();
                    if let Some(r) = self.compile_2d_array_read(&bare, &i_e, &j_e) {
                        return Some(r);
                    }
                }
                // A REAL-valued parameter wins over its signal-table twin: a
                // header parameter gets a placeholder signal entry whose
                // stored Value is integral/x, so loading the signal compared
                // raw bits against a real literal and `r1 != 5.0` on
                // `parameter real r1 = 5.0` came out true. The parameter
                // table is authoritative and carries the f64. Integral
                // parameters keep the historical signal-first order.
                if let Some(v) = self.lookup_param_value(hier) {
                    if v.is_real {
                        let r = self.alloc_reg();
                        self.emit(Insn::LoadConst(r, Box::new(v)));
                        return Some(r);
                    }
                }
                // class-perf P2: a PACKAGE-scope enum constant
                // (`uvm_pkg::UVM_NONE` in a report-macro argument) folds
                // to a literal — the same table the interpreter's
                // `package_enum_member` reads.
                if self.method_mode
                    && hier.root.is_none()
                    && hier.path.len() == 2
                    && hier.path.iter().all(|seg| seg.selects.is_empty())
                {
                    let key = format!("{}::{}", hier.path[0].name.name, hier.path[1].name.name);
                    if let Some(&(val, width)) = self.pkg_enum_consts.get(&key) {
                        let r = self.alloc_reg();
                        self.emit(Insn::LoadConst(
                            r,
                            Box::new(Value::from_u64(val, width.max(1))),
                        ));
                        return Some(r);
                    }
                }
                // In method mode, a BARE Ident that names a member/static of
                // the method's class or an enclosing class must resolve to
                // class scope, NOT to a same-named module-level signal. A
                // NON-shadowed instance member of `this` resolves to
                // `this.<member>` (a heap access); everything else (shadowed
                // members, class statics, enclosing-class members) the
                // compiled path cannot model, so it bails (AST) rather than
                // read the wrong (module) storage.
                if self.method_mode && hier.root.is_none() && hier.path.len() == 1 {
                    let bare = hier.path[0].name.name.as_str();
                    if !self.local_var_regs.contains_key(bare) {
                        if hier.path[0].selects.is_empty()
                            && (self.bare_member_names.contains(bare)
                                // Step 9a: a bare STRING member reads from the
                                // heap slot verbatim, exactly like an integral
                                // one — `fit_class_prop` never resizes a string
                                // member (`class_prop_width_impl` returns None),
                                // so the loaded Value round-trips. The member is
                                // NOT in `bare_member_names` (nonloadable), so
                                // test the string set separately.
                                || self.string_member_names.contains(bare))
                        {
                            // Resolve `return parent;` to `this.parent`.
                            let dest = self.alloc_reg();
                            self.emit(Insn::LoadClassMember(
                                dest,
                                self.method_this_reg.expect("this in method_mode"),
                                bare.to_string().into_boxed_str(),
                            ));
                            return Some(dest);
                        }
                        if let Some(value) = self.method_enum_constant(hier) {
                            let dest = self.alloc_reg();
                            self.emit(Insn::LoadConst(dest, Box::new(value)));
                            return Some(dest);
                        }
                        // Step 9g: a bare STATIC property (or class localparam)
                        // of the enclosing chain — the AST resolves it
                        // through class_static_get starting at the lexical
                        // class; the executor does exactly that with the
                        // baked defining class.
                        if self.static_member_names.contains(bare)
                            || self.class_const_names.contains(bare)
                        {
                            let dest = self.alloc_reg();
                            self.emit(Insn::LoadClassStatic(
                                dest,
                                Box::new(NamePair(
                                    self.method_class
                                        .clone()
                                        .unwrap_or_default()
                                        .into_boxed_str(),
                                    bare.to_string().into_boxed_str(),
                                )),
                            ));
                            return Some(dest);
                        }
                        if self.class_shadow_names.contains(bare) {
                            // A class-scope name the compiled path cannot
                            // model: shadowed members, statics/localparams,
                            // arrays (also as `g[i][j]` element selects —
                            // those reach HERE, selects non-empty, and used
                            // to fall through to a bogus module-signal read).
                            self.bail("method_class_shadow");
                        }
                        // Step 9g: a bare `new` Ident that reached here is
                        // a §8.8 handle construct outside the assign-site
                        // lowering (target class = lhs declared type).
                        if bare == "new" {
                            self.bail("Expr_New");
                            return None;
                        }
                        // class-perf free-function tier: a free (package/
                        // module-scope) function frame has NO class scope —
                        // every bare name that is not a formal/local/return
                        // cell resolves, in the interpreter, to a
                        // package-level VARIABLE (assoc arrays, queues,
                        // state objects: uvm_globals' `m_uvm_core_state`
                        // and friends). Those live outside the signal
                        // table (or with unrelated storage), and a compiled
                        // block reading them as module signals returned
                        // garbage that corrupted the whole UVM init
                        // sequence (uvm_complete_test: run_test silently
                        // produced nothing). Decline the body unless every
                        // such name was explicitly admitted.
                        if self.free_function_mode
                            && !self.method_handle_names.contains(bare)
                            && !self.string_formal_names.contains(bare)
                        {
                            self.bail("free_fn_pkg_variable");
                            return None;
                        }
                    }
                }
                if let Some(id) = self.lookup_signal_id(hier) {
                    // An integral parameter's signal-table twin: fold to the
                    // parameter's constant instead of a runtime load. On
                    // c906 `tmp_out[n*DATA +: DATA]` read DATA as a signal,
                    // so its bounds were register-computed, the entry's
                    // dependency on `tmp_out` was whole-signal, and one bit
                    // change re-triggered all 64 chain entries (the loop
                    // census saw a 64-entry cycle in a plain chain).
                    if let Some(v) = self.param_twin_value(hier, id) {
                        let r = self.alloc_reg();
                        self.emit(Insn::LoadConst(r, Box::new(v)));
                        return Some(r);
                    }
                    let r = self.alloc_reg();
                    if self.signal_signed[id] {
                        self.emit(Insn::LoadSignalSigned(r, as_sig_id(id)));
                    } else {
                        self.emit(Insn::LoadSignal(r, as_sig_id(id)));
                    }
                    return Some(r);
                }
                if let Some(v) = self.lookup_param_value(hier) {
                    let r = self.alloc_reg();
                    self.emit(Insn::LoadConst(r, Box::new(v)));
                    return Some(r);
                }
                if let Some(r) = self.compile_packed_member_read(hier) {
                    return Some(r);
                }
                if std::env::var_os("XEZIM_PROBE_IDENT").is_some() {
                    if let ExprKind::Ident(h) = &expr.kind {
                        eprintln!("[IDENT_FALLBACK] {}", Self::hier_raw_name(h));
                    }
                }
                if let Some(r) = self.emit_expr_fallback(expr, ctx_width, "ident_lookup") {
                    return Some(r);
                }
                self.bail("ident_lookup");
                None
            }
            ExprKind::StringLiteral(s) => {
                let mut v = Value::from_string(s);
                if ctx_width > 0 {
                    v = v.resize(ctx_width);
                }
                let r = self.alloc_reg();
                self.emit(Insn::LoadConst(r, Box::new(v)));
                Some(r)
            }
            ExprKind::Unary { op, operand } => {
                // class-perf P2: EXPRESSION-position ++/-- (uvm_object::new's
                // `m_inst_id = m_inst_count++`) — read, ±1, store through
                // the assignment machinery; yield the OLD (post) or NEW
                // (pre) value. Width discipline identical to the statement
                // form: 0 = self-determined member store (heap truncates
                // at the declared width), Resize only for width>0 targets.
                if self.method_mode
                    && matches!(
                        op,
                        UnaryOp::PreIncr | UnaryOp::PostIncr | UnaryOp::PreDecr | UnaryOp::PostDecr
                    )
                {
                    let width = self.infer_lhs_width(operand);
                    let start = self.insns.len();
                    let start_reg = self.next_reg;
                    let is_incr = matches!(op, UnaryOp::PreIncr | UnaryOp::PostIncr);
                    let is_post = matches!(op, UnaryOp::PostIncr | UnaryOp::PostDecr);
                    let lowered = (|| {
                        let r = self.compile_expr(operand, width)?;
                        let one = self.alloc_reg();
                        self.emit(Insn::LoadConst(
                            one,
                            Box::new(Value::from_u64(1, if width > 0 { width } else { 32 })),
                        ));
                        let result = self.alloc_reg();
                        if is_incr {
                            self.emit(Insn::Add(result, r, one));
                        } else {
                            self.emit(Insn::Sub(result, r, one));
                        }
                        if width > 0 {
                            self.emit(Insn::Resize(result, width));
                        }
                        if !self.compile_blocking_target(operand, result, width) {
                            return None;
                        }
                        Some(if is_post { r } else { result })
                    })();
                    if let Some(out) = lowered {
                        return Some(out);
                    }
                    self.truncate_insns(start);
                    self.next_reg = start_reg;
                    self.bail("Expr_IncrExpr");
                    return None;
                }
                // Reduction (&a, |a, ^a, ~&a, ~|a, ~^a) and logical-NOT (!a)
                // are SELF-DETERMINED: operand keeps its natural width, the
                // unary produces 1 bit. Passing parent ctx_width here would
                // resize the operand and corrupt the reduction
                // (e.g. zero-extending a 32-bit value to 64 makes &a = 0
                // even when the 32-bit value was all 1s).
                // Step 8 (string formals): `!s`/`~s` on a string value would
                // bit-bang bytes the interpreter string-paths never form —
                // bail.
                if self.string_operand_unsafe(operand) {
                    self.bail("string_formal_unary");
                    return None;
                }
                let operand_ctx = if matches!(
                    op,
                    UnaryOp::BitAnd
                        | UnaryOp::BitNand
                        | UnaryOp::BitOr
                        | UnaryOp::BitNor
                        | UnaryOp::BitXor
                        | UnaryOp::BitXnor
                        | UnaryOp::LogNot
                ) {
                    0
                } else {
                    ctx_width
                };
                let src = self.compile_expr_typed(
                    operand,
                    operand_ctx,
                    unsigned_context
                        && matches!(op, UnaryOp::Plus | UnaryOp::Minus | UnaryOp::BitNot),
                )?;
                // §11.6.1: `~` and unary `-` are CONTEXT-determined — the
                // operand is extended to the context width BEFORE the
                // operation, not after. Passing `operand_ctx` down is not
                // enough on its own: a plain signal load returns its declared
                // width, so `logic [31:0] r = ~a;` with an 8-bit `a` computed
                // ~a in 8 bits and zero-extended, giving 0000004b where
                // ffffff4b is required (and 0000004c for `-a`). Resize
                // explicitly; the value carries its own signedness, so a
                // signed operand still sign-extends.
                let src = if operand_ctx > 0 && matches!(op, UnaryOp::Minus | UnaryOp::BitNot) {
                    self.emit(Insn::Resize(src, operand_ctx));
                    src
                } else {
                    src
                };
                let dest = self.alloc_reg();
                match op {
                    UnaryOp::Plus => return Some(src),
                    UnaryOp::Minus => self.emit(Insn::Negate(dest, src)),
                    UnaryOp::LogNot => self.emit(Insn::LogNot(dest, src)),
                    UnaryOp::BitNot => self.emit(Insn::BitNot(dest, src)),
                    UnaryOp::BitAnd => self.emit(Insn::ReduceAnd(dest, src)),
                    UnaryOp::BitNand => {
                        self.emit(Insn::ReduceAnd(dest, src));
                        self.emit(Insn::BitNot(dest, dest));
                    }
                    UnaryOp::BitOr => self.emit(Insn::ReduceOr(dest, src)),
                    UnaryOp::BitNor => {
                        self.emit(Insn::ReduceOr(dest, src));
                        self.emit(Insn::BitNot(dest, dest));
                    }
                    UnaryOp::BitXor => self.emit(Insn::ReduceXor(dest, src)),
                    UnaryOp::BitXnor => {
                        self.emit(Insn::ReduceXor(dest, src));
                        self.emit(Insn::BitNot(dest, dest));
                    }
                    _ => {
                        self.bail("UnaryOp_other");
                        return None;
                    }
                }
                Some(dest)
            }
            ExprKind::Binary { op, left, right } => {
                // §11.4.7: `&&` / `||` do not evaluate the right operand once
                // the left decides the result. Matters when it has an effect
                // (`h != null && h.f()` must not call through null).
                if self.method_mode
                    && matches!(op, BinaryOp::LogAnd | BinaryOp::LogOr)
                    && Self::expr_has_effect(right)
                {
                    return self.compile_short_circuit(*op, left, right);
                }
                // Step 8 (string formals): only equality/case-equality is
                // byte-for-byte safe on string values — for pure-byte
                // operands (no X/Z, byte-multiple widths, which is all a
                // string formal/literal/call-return can hold) the packed
                // compare's zero-extension of the shorter side matches the
                // interpreter's string path (leading-NUL trim, §4.2 of
                // value.rs). Relational and arithmetic string ops route
                // through the interpreter's string_paths / would zero-extend
                // wrongly — bail them.
                if !matches!(
                    op,
                    BinaryOp::Eq | BinaryOp::Neq | BinaryOp::CaseEq | BinaryOp::CaseNeq
                ) && (self.string_operand_unsafe(left) || self.string_operand_unsafe(right))
                {
                    self.bail("string_formal_non_eq_binary");
                    return None;
                }
                // §11.8.2: an operator with a REAL operand follows the real
                // rules, not the integral width/sign propagation below.
                if let Some(r) = self.compile_real_operator(*op, left, right) {
                    return r;
                }
                // §11.6.1: in a self-determined position (a concatenation
                // operand, no context width) an arithmetic or bitwise
                // operator is its own context: the wider operand's width,
                // propagated down to both operands, so the narrower one is
                // extended by its own sign before the operator. Without it
                // the operands met at unequal widths and the two execution
                // tiers extended them differently.
                let ctx_width = if ctx_width == 0
                    && matches!(
                        op,
                        BinaryOp::Add
                            | BinaryOp::Sub
                            | BinaryOp::Mul
                            | BinaryOp::BitAnd
                            | BinaryOp::BitOr
                            | BinaryOp::BitXor
                            | BinaryOp::BitXnor
                    ) {
                    // Only with both operands' widths (the two width oracles
                    // agreeing) and signs known: anything else keeps the
                    // run-time operator's own extension.
                    // Equal widths need no extension.
                    let (lw, rw) = (self.lrm_self_width(left), self.lrm_self_width(right));
                    let known = lw > 0
                        && rw > 0
                        && lw != rw
                        && lw == self.expr_max_width(left)
                        && rw == self.expr_max_width(right)
                        && self.expr_signedness(left).is_some()
                        && self.expr_signedness(right).is_some();
                    if known { lw.max(rw) } else { 0 }
                } else {
                    ctx_width
                };
                // Verilog operand-width rules: comparison and logical ops
                // (==, !=, <, <=, >, >=, &&, ||, ===, !==, case-eq) are
                // self-determined — their operands' widths are max(L,R) of
                // the operands themselves, NOT the surrounding context.
                // Propagating the (often narrow, e.g. 1-bit LHS) ctx_width
                // into them silently truncates wide sub-expressions like
                // `(addr[31:20] & mask[11:0]) == base[11:0]` where the
                // 12-bit BitAnd would get resized to 1 bit, producing
                // wrong results on any high-order bits. (Bug seen on E902
                // cr_bmu_dbus_if iahbl_hit cont-assign at cyc 14: addr
                // 0x20000000 → 0x200, AND'd with 0xe00 should be 0x200,
                // but resized to 1 bit gives 0, so == 0 returns 1 instead
                // of 0.)
                let is_self_determined = matches!(
                    op,
                    BinaryOp::Eq
                        | BinaryOp::Neq
                        | BinaryOp::CaseEq
                        | BinaryOp::CaseNeq
                        | BinaryOp::WildcardEq
                        | BinaryOp::WildcardNeq
                        | BinaryOp::Lt
                        | BinaryOp::Leq
                        | BinaryOp::Gt
                        | BinaryOp::Geq
                        | BinaryOp::LogAnd
                        | BinaryOp::LogOr
                        | BinaryOp::LogImplies
                        | BinaryOp::LogEquiv
                );
                // §11.6.1: for the operators whose operands are
                // CONTEXT-determined, the context width is the MAXIMUM of the
                // surrounding context and the operands' own widths — it must
                // never NARROW an operand. Propagating a narrow LHS width down
                // truncated the left operand before the operation, which is
                // observably wrong wherever the low bits are not preserved:
                // `logic [4:0] r; r <= (1 << s) >> 3;` computed `1 << 5` at 5
                // bits (0) instead of 32 bits (32), so r read 0 instead of 4.
                // (For +/-/*/&/|/^ the low bits are the same either way, which
                // is why only the shift/divide family showed it.)
                let widens_operands = matches!(
                    op,
                    BinaryOp::ShiftLeft
                        | BinaryOp::ShiftRight
                        | BinaryOp::ArithShiftLeft
                        | BinaryOp::ArithShiftRight
                        | BinaryOp::Div
                        | BinaryOp::Mod
                        | BinaryOp::Power
                );
                // §6.16 Table 6-9: a comparison of two STRING operands is
                // textual (the `StrOp` arms below); a string has no fixed
                // width, so neither operand takes a context — a literal
                // compiled at the nominal width of a string-queue element
                // (`q[0] == "e"`) was cut to one bit.
                let string_cmp = matches!(
                    op,
                    BinaryOp::Eq
                        | BinaryOp::Neq
                        | BinaryOp::Lt
                        | BinaryOp::Leq
                        | BinaryOp::Gt
                        | BinaryOp::Geq
                ) && self.expr_is_string_static(left)
                    && self.expr_is_string_static(right)
                    && (self.expr_has_string_type_static(left)
                        || self.expr_has_string_type_static(right));
                let sub_ctx = if string_cmp {
                    0
                } else if matches!(
                    op,
                    BinaryOp::LogAnd | BinaryOp::LogOr | BinaryOp::LogImplies | BinaryOp::LogEquiv
                ) {
                    // §11.6.1 Table 11-21: each operand of `&&`, `||`, `->`
                    // and `<->` is self-determined on its own. Sizing both to
                    // the wider one (an 8-bit call argument counted as the
                    // call's width) made `~f(x)` of a 1-bit `f` 8'hfe, true.
                    0
                } else if is_self_determined {
                    let lw = self.expr_max_width(left);
                    let rw = self.expr_max_width(right);
                    lw.max(rw)
                } else if matches!(op, BinaryOp::Div | BinaryOp::Mod) {
                    // §11.6.1 Table 11-21: `/` and `%` size BOTH operands to
                    // max(L(i), L(j)) and the context — the right operand's
                    // width counts too. Taking only the left's let a narrow
                    // dividend narrow the divisor: `x % (n < 32 ? n : 32)`
                    // with a 5-bit `x` compiled the ternary at 5 bits, so the
                    // divisor 32 became 0 and the index read x.
                    ctx_width
                        .max(self.lrm_self_width(left))
                        .max(self.lrm_self_width(right))
                } else if widens_operands {
                    ctx_width.max(self.expr_max_width(left))
                } else {
                    ctx_width
                };
                let is_shift = matches!(
                    op,
                    BinaryOp::ShiftLeft
                        | BinaryOp::ShiftRight
                        | BinaryOp::ArithShiftLeft
                        | BinaryOp::ArithShiftRight
                );
                let self_determined_operands = matches!(
                    op,
                    BinaryOp::LogAnd | BinaryOp::LogOr | BinaryOp::LogImplies | BinaryOp::LogEquiv
                );
                let joint_unsigned = !string_cmp
                    && !self_determined_operands
                    && ((!is_self_determined && unsigned_context)
                        || self.expr_signedness(left) == Some(false)
                        || (!is_shift
                            && !matches!(op, BinaryOp::Power)
                            && self.expr_signedness(right) == Some(false)));
                let sub_ctx = if joint_unsigned
                    && !is_self_determined
                    && !is_shift
                    && !matches!(op, BinaryOp::Power)
                {
                    sub_ctx
                        .max(self.lrm_self_width(left))
                        .max(self.lrm_self_width(right))
                } else {
                    sub_ctx
                };
                // §11.8.2: comparison operands determine each other's type
                // before evaluation, including nested arithmetic, not only shifts.
                let left_unsigned = joint_unsigned;
                let right_unsigned = joint_unsigned;
                let l_sub_ctx = if matches!(
                    op,
                    BinaryOp::Div
                        | BinaryOp::Mod
                        | BinaryOp::ShiftLeft
                        | BinaryOp::ShiftRight
                        | BinaryOp::ArithShiftLeft
                        | BinaryOp::ArithShiftRight
                        | BinaryOp::Power
                ) && self.lrm_self_width(left) == 0
                {
                    0
                } else {
                    sub_ctx
                };
                let r_sub_ctx = if matches!(op, BinaryOp::Div | BinaryOp::Mod)
                    && self.lrm_self_width(right) == 0
                {
                    0
                } else {
                    sub_ctx
                };
                let l = self.compile_expr_typed(left, l_sub_ctx, left_unsigned)?;
                // §11.4.10: a shift's RIGHT operand is SELF-DETERMINED — its
                // width never affects the result, so it keeps its own.
                let is_shift = matches!(
                    op,
                    BinaryOp::ShiftLeft
                        | BinaryOp::ShiftRight
                        | BinaryOp::ArithShiftLeft
                        | BinaryOp::ArithShiftRight
                );
                let r = if is_shift || matches!(op, BinaryOp::Power) {
                    self.compile_expr(right, self.expr_max_width(right))?
                } else {
                    self.compile_expr_typed(right, r_sub_ctx, right_unsigned)?
                };
                // Context width resizing for arithmetic / bitwise ops only.
                // For self-determined comparisons we must NOT resize to
                // ctx_width — that would clobber the operands.
                if !is_self_determined
                    && ctx_width > 0
                    && matches!(
                        op,
                        BinaryOp::Add
                            | BinaryOp::Sub
                            | BinaryOp::Mul
                            | BinaryOp::BitAnd
                            | BinaryOp::BitOr
                            | BinaryOp::BitXor
                            | BinaryOp::BitXnor
                    )
                {
                    // §11.8.1: the expression is UNSIGNED if ANY operand is
                    // unsigned — widening must then ZERO-extend both. The
                    // runtime Resize extends by each VALUE's own signed flag,
                    // so a signed operand in a mixed expression sign-extended:
                    // `sa + b` in a 32-bit context read fffffff9 instead of
                    // 000000f9 (the $display path was already correct).
                    // Unknown signedness keeps the historical behavior.
                    let ls = self.expr_signedness(left);
                    let rs = self.expr_signedness(right);
                    if ls == Some(false) || rs == Some(false) {
                        if !left_unsigned && !self.operand_scrub_is_noop(left) {
                            self.emit(Insn::ClearSigned(l));
                        }
                        if !right_unsigned && !self.operand_scrub_is_noop(right) {
                            self.emit(Insn::ClearSigned(r));
                        }
                    }
                    self.emit(Insn::Resize(l, ctx_width));
                    self.emit(Insn::Resize(r, ctx_width));
                }
                let dest = self.alloc_reg();
                match op {
                    BinaryOp::Add => self.emit(Insn::Add(dest, l, r)),
                    BinaryOp::Sub => self.emit(Insn::Sub(dest, l, r)),
                    BinaryOp::Mul => self.emit(Insn::Mul(dest, l, r)),
                    BinaryOp::Div | BinaryOp::Mod => {
                        // §11.6.1 Table 11-21: BOTH operands are context-
                        // determined. The registers kept their declared
                        // widths, so `smin / sm1` divided at 8 bits (wrapping
                        // -128/-1) and a divide-by-zero produced x at 8 bits.
                        // §11.8.1 signedness applies exactly as for +/-.
                        // When either operand's static width is unknown (0),
                        // do not resize before the runtime Div/Mod — `opw`
                        // could be narrower than the operand's runtime width
                        // and truncate high bits before the division/modulo,
                        // whereas `Value::div`/`Value::modulo` already widens
                        // both operands to `max(l.width, r.width)` at runtime.
                        let lw = self.lrm_self_width(left);
                        let rw = self.lrm_self_width(right);
                        let opw = ctx_width.max(lw).max(rw);
                        let ls = self.expr_signedness(left);
                        let rs = self.expr_signedness(right);
                        if ls == Some(false) || rs == Some(false) {
                            if !left_unsigned && !self.operand_scrub_is_noop(left) {
                                self.emit(Insn::ClearSigned(l));
                            }
                            if !right_unsigned && !self.operand_scrub_is_noop(right) {
                                self.emit(Insn::ClearSigned(r));
                            }
                        }
                        if lw > 0 && rw > 0 && opw > 0 {
                            self.emit(Insn::Resize(l, opw));
                            self.emit(Insn::Resize(r, opw));
                        }
                        if matches!(op, BinaryOp::Div) {
                            self.emit(Insn::Div(dest, l, r));
                        } else {
                            self.emit(Insn::Mod(dest, l, r));
                        }
                    }
                    BinaryOp::BitAnd => self.emit(Insn::BitAnd(dest, l, r)),
                    BinaryOp::BitOr => self.emit(Insn::BitOr(dest, l, r)),
                    BinaryOp::BitXor => self.emit(Insn::BitXor(dest, l, r)),
                    BinaryOp::BitXnor => self.emit(Insn::BitXnor(dest, l, r)),
                    BinaryOp::LogAnd => self.emit(Insn::LogAnd(dest, l, r)),
                    BinaryOp::LogOr => self.emit(Insn::LogOr(dest, l, r)),
                    // a -> b  ==  !a || b   (IEEE 1800-2023 §11.4.7)
                    BinaryOp::LogImplies => {
                        self.emit(Insn::LogNot(dest, l));
                        self.emit(Insn::LogOr(dest, dest, r));
                    }
                    // a <-> b  ==  (!a || b) && (!b || a)
                    BinaryOp::LogEquiv => {
                        let nl = self.alloc_reg();
                        let nr = self.alloc_reg();
                        let t1 = self.alloc_reg();
                        self.emit(Insn::LogNot(nl, l));
                        self.emit(Insn::LogNot(nr, r));
                        self.emit(Insn::LogOr(t1, nl, r));
                        self.emit(Insn::LogOr(dest, nr, l));
                        self.emit(Insn::LogAnd(dest, t1, dest));
                    }
                    // §6.16 / Table 6-9: `==`/`!=` on STRING operands are
                    // 2-STATE textual compares. The integral Eq on the
                    // 1024-bit packed storage returned X whenever either
                    // side's unused capacity was X padding — an empty-string
                    // compare always was. Same StrOp routing as the
                    // relational arm below.
                    BinaryOp::Eq | BinaryOp::Neq if string_cmp => {
                        let cmp = self.alloc_reg();
                        self.emit(Insn::StrOp(cmp, StrOpKind::Order, Box::new(vec![l, r])));
                        let z = self.alloc_reg();
                        let mut zero = Value::from_u64(0, 32);
                        zero.is_signed = true;
                        self.emit(Insn::LoadConst(z, Box::new(zero)));
                        self.emit(if matches!(op, BinaryOp::Eq) {
                            Insn::Eq(dest, cmp, z)
                        } else {
                            Insn::Neq(dest, cmp, z)
                        });
                    }
                    BinaryOp::Eq => self.emit(Insn::Eq(dest, l, r)),
                    BinaryOp::Neq => self.emit(Insn::Neq(dest, l, r)),
                    BinaryOp::CaseEq => self.emit(Insn::CaseEq(dest, l, r)),
                    // LRM §11.4.5: `!==` is the bit-exact negation of `===`.
                    // No dedicated Insn; compose CaseEq → LogNot. (Previously
                    // this hit the catch-all and bailed to the AST interp.)
                    BinaryOp::CaseNeq => {
                        self.emit(Insn::CaseEq(dest, l, r));
                        self.emit(Insn::LogNot(dest, dest));
                    }
                    // §6.16.6: relational operators on STRING operands are
                    // lexicographic. The numeric Lt/Gt insns compare packed
                    // magnitudes, which diverges once lengths differ
                    // ("abc" < "b" is TRUE lexicographically, false
                    // numerically) — route through the native strcmp and
                    // compare its signed difference against zero.
                    BinaryOp::Lt | BinaryOp::Leq | BinaryOp::Gt | BinaryOp::Geq if string_cmp => {
                        let cmp = self.alloc_reg();
                        self.emit(Insn::StrOp(cmp, StrOpKind::Order, Box::new(vec![l, r])));
                        let z = self.alloc_reg();
                        let mut zero = Value::from_u64(0, 32);
                        zero.is_signed = true;
                        self.emit(Insn::LoadConst(z, Box::new(zero)));
                        self.emit(match op {
                            BinaryOp::Lt => Insn::Lt(dest, cmp, z),
                            BinaryOp::Leq => Insn::Leq(dest, cmp, z),
                            BinaryOp::Gt => Insn::Gt(dest, cmp, z),
                            _ => Insn::Geq(dest, cmp, z),
                        });
                    }
                    BinaryOp::Lt => self.emit(Insn::Lt(dest, l, r)),
                    BinaryOp::Leq => self.emit(Insn::Leq(dest, l, r)),
                    BinaryOp::Gt => self.emit(Insn::Gt(dest, l, r)),
                    BinaryOp::Geq => self.emit(Insn::Geq(dest, l, r)),
                    BinaryOp::ShiftLeft | BinaryOp::ArithShiftLeft => {
                        // §11.4.10/§11.6.1: the LEFT operand takes the LRM
                        // operation width — ctx joined with the operand's own
                        // LRM width (never the carry-aware estimate, which
                        // shifted dropped carries back into range).
                        // An operand of unknown width (0) keeps the width its
                        // load produced so a narrower target context cannot
                        // truncate bits before the shift.
                        let lw = self.lrm_self_width(left);
                        let opw = ctx_width.max(lw);
                        if lw > 0 && opw > 0 {
                            self.emit(Insn::Resize(l, opw));
                        }
                        self.emit(Insn::Shl(dest, l, r));
                    }
                    BinaryOp::ShiftRight | BinaryOp::ArithShiftRight => {
                        // Same rule for right shifts — previously the operand
                        // register kept whatever width its sub-expression
                        // produced: a signed 8-bit value in a 32-bit context
                        // shifted at 8 bits then zero-extended (00000013 for
                        // 1ffffff3), and `(a+a) >> 1` shifted the carry back
                        // in (0xa3 for 0x23). When `lw == 0`, keep the loaded
                        // width so a narrower target cannot truncate high bits
                        // before shifting them into range.
                        let lw = self.lrm_self_width(left);
                        let opw = ctx_width.max(lw);
                        if lw > 0 && opw > 0 {
                            self.emit(Insn::Resize(l, opw));
                        }
                        if matches!(op, BinaryOp::ShiftRight) {
                            self.emit(Insn::Shr(dest, l, r));
                        } else {
                            self.emit(Insn::AShr(dest, l, r));
                        }
                    }
                    // LRM §11.4.3 power. There is no runtime Pow instruction;
                    // every `**` seen in RTL has constant operands (`2**level`
                    // after genvar substitution, `2**N` parameters), so fold
                    // it to a constant here. Without this arm `**` hit the
                    // catch-all `bail` below — which, for a `**` inside an
                    // array-element LHS index like `mem[2**lvl-1+k]`, dropped
                    // the whole continuous assign to the AST interpreter and
                    // mis-evaluated the RHS. A genuinely non-constant `a**b`
                    // still bails (rare; preserves prior behavior).
                    BinaryOp::Power => {
                        // Fold `**` to a constant (no runtime Pow insn). Compute
                        // the result in u64 and load it at the expression's
                        // natural width: `eval_const_expr` truncates to u32 and
                        // the old `from_u64(v, 32)` truncated again, so 2**N for
                        // N>=32 collapsed to 0 (e.g. 2**51 -> 0). (pr2865563)
                        if let (Some(base), Some(exp)) =
                            (self.eval_const_expr(left), self.eval_const_expr(right))
                        {
                            let mut result: u64 = 1;
                            for _ in 0..(exp as u64).min(64) {
                                result = result.wrapping_mul(base as u64);
                            }
                            let w = self.expr_max_width(expr).max(ctx_width).max(1);
                            self.emit(Insn::LoadConst(dest, Box::new(Value::from_u64(result, w))));
                        } else {
                            // Non-constant base: a REAL Pow insn. The left
                            // operand is context-determined (§11.6.1) — a
                            // load returns its declared width, so resize it
                            // to the operation width first; `a ** 2` in a
                            // 32-bit context computed at 8 bits (0x90) and
                            // then bailed the whole block to the interpreter.
                            let lw = self.expr_max_width(left);
                            let opw = sub_ctx.max(lw);
                            if lw > 0 && opw > 0 {
                                self.emit(Insn::Resize(l, opw));
                            }
                            self.emit(Insn::Pow(dest, l, r));
                        }
                    }
                    _ => {
                        self.bail("BinaryOp_other");
                        return None;
                    }
                }
                Some(dest)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                // §11.4.11/§11.8.2: with a REAL arm the result is real and no
                // context width applies; see `compile_real_arm`.
                if self.expr_is_real_static(then_expr) || self.expr_is_real_static(else_expr) {
                    let cond = self.compile_expr(condition, 0)?;
                    let then_reg = self.compile_real_arm(then_expr)?;
                    let else_reg = self.compile_real_arm(else_expr)?;
                    let dest = self.alloc_reg();
                    self.emit(Insn::Select(dest, cond, then_reg, else_reg));
                    return Some(dest);
                }
                // Evaluate both branches unconditionally so Select can do a
                // per-bit merge when the condition has X/Z (IEEE 1800 §11.4.11).
                let cond = self.compile_expr(condition, 0)?;
                let arms_unsigned = unsigned_context
                    || self.expr_signedness(then_expr) == Some(false)
                    || self.expr_signedness(else_expr) == Some(false);
                let arm_ctx = if arms_unsigned {
                    ctx_width
                        .max(self.lrm_self_width(then_expr))
                        .max(self.lrm_self_width(else_expr))
                } else {
                    ctx_width
                };
                // §11.4.11: a known condition evaluates only the selected
                // arm, which matters when an arm has an effect (`h == null ?
                // "" : h.name()` must not call through null).
                let short = self.method_mode
                    && (Self::expr_has_effect(then_expr) || Self::expr_has_effect(else_expr));
                // Each arm is lowered twice below, so nested ones double per
                // level; a deep nest stays interpreted.
                if short && Self::ternary_depth(then_expr).max(Self::ternary_depth(else_expr)) > 6 {
                    return None;
                }
                let mut pick: Option<(usize, usize, RegId)> = None;
                if short {
                    let red = self.alloc_reg();
                    self.emit(Insn::ReduceOr(red, cond));
                    let branch_on = |c: &mut Self, v: u64| {
                        let k = c.alloc_reg();
                        c.emit(Insn::LoadConst(k, Box::new(Value::from_u64(v, 1))));
                        let is = c.alloc_reg();
                        c.emit(Insn::CaseEq(is, red, k));
                        let not = c.alloc_reg();
                        c.emit(Insn::LogNot(not, is));
                        let at = c.insns.len();
                        c.emit(Insn::BranchIfFalse(not, 0));
                        at
                    };
                    let to_then = branch_on(self, 1);
                    let to_else = branch_on(self, 0);
                    pick = Some((to_then, to_else, self.alloc_reg()));
                }
                let then_reg = self.compile_expr_typed(then_expr, arm_ctx, arms_unsigned)?;
                let else_reg = self.compile_expr_typed(else_expr, arm_ctx, arms_unsigned)?;
                // §11.8.1: a ternary with ANY unsigned arm is unsigned — both
                // arms then ZERO-extend (a signed arm sign-extended, so
                // `c ? sa : b` in a 32-bit context read ffffff9c for
                // 0000009c). And §11.4.11's x-condition per-bit merge must
                // happen at the CONTEXT width — merging at arm width and
                // zero-extending after produced 000000XX for xxxxxxXX.
                if ctx_width > 0 {
                    let ts = self.expr_signedness(then_expr);
                    let es = self.expr_signedness(else_expr);
                    if ts == Some(false) || es == Some(false) {
                        if !arms_unsigned && !self.operand_scrub_is_noop(then_expr) {
                            self.emit(Insn::ClearSigned(then_reg));
                        }
                        if !arms_unsigned && !self.operand_scrub_is_noop(else_expr) {
                            self.emit(Insn::ClearSigned(else_reg));
                        }
                    }
                    self.emit(Insn::Resize(then_reg, ctx_width));
                    self.emit(Insn::Resize(else_reg, ctx_width));
                }
                let Some((to_then, to_else, dest)) = pick else {
                    let dest = self.alloc_reg();
                    self.emit(Insn::Select(dest, cond, then_reg, else_reg));
                    return Some(dest);
                };
                // X condition: both arms above, merged. A known one jumps to
                // its own arm below, given the same fit as the merged one.
                self.emit(Insn::Select(dest, cond, then_reg, else_reg));
                let mut ends = vec![self.insns.len()];
                self.emit(Insn::Jump(0));
                let ts = self.expr_signedness(then_expr);
                let es = self.expr_signedness(else_expr);
                let mixed = ts == Some(false) || es == Some(false);
                for (at, arm) in [(to_then, then_expr), (to_else, else_expr)] {
                    let here = self.insns.len() as u32;
                    if let Insn::BranchIfFalse(reg, _) = self.insns[at] {
                        self.insns[at] = Insn::BranchIfFalse(reg, here);
                    }
                    let r = self.compile_expr_typed(arm, arm_ctx, arms_unsigned)?;
                    if ctx_width > 0 {
                        if mixed && !arms_unsigned && !self.operand_scrub_is_noop(arm) {
                            self.emit(Insn::ClearSigned(r));
                        }
                        self.emit(Insn::Resize(r, ctx_width));
                    }
                    self.emit(Insn::Move(dest, r));
                    ends.push(self.insns.len());
                    self.emit(Insn::Jump(0));
                }
                let end = self.insns.len() as u32;
                for at in ends {
                    self.insns[at] = Insn::Jump(end);
                }
                Some(dest)
            }
            ExprKind::Paren(inner) => self.compile_expr_typed(inner, ctx_width, unsigned_context),
            ExprKind::Index { expr, index } => {
                // A register-bound formal shadows every other reading of
                // the name (see `compile_formal_member_elem`).
                if let Some(r) = self.compile_formal_member_elem(whole) {
                    if r.is_none() {
                        self.bail("formal_member_elem");
                    }
                    return r;
                }
                // §13.4.1, §7.4.1: two or more indices into a packed-array
                // member of a call result (`mk().f0[i][j]`) have no shape
                // here: past the first, each was a bit select of the
                // element. The AST path takes the member's dimensions.
                if matches!(&expr.kind, ExprKind::Index { .. }) && chain_has_call_base(expr) {
                    self.bail("call_member_index_chain");
                    return None;
                }
                // Step 8 (string formals): a select on a string formal takes
                // the interpreter's char-select path (`string_signals`
                // registry, out-of-range → NUL byte, not X) — bail. An index
                // EXPRESSION mentioning a string formal is equally unsafe
                // (non-integral index coercion differs).
                // Step 9b-ii EXCEPTion: a WHOLE string formal used as an
                // INDEX (`aa[name]` — uvm_component::get_child's
                // `m_children[name]`) is a plain read of the formal's
                // register: the seeded byte-vector Value is the exact value
                // the interpreter's local-frame read sees, and
                // `assoc_key_str`'s §6.16 narrowing runs identically at
                // runtime. The relaxation COMMITS to the member-element
                // read below: if that declines, the whole expression bails
                // rather than lowering the formal index through any other
                // arm.
                let relaxed_formal_index = self.method_mode
                    && !self.touches_string_formal(expr)
                    && self.index_is_whole_string_formal(index);
                // §6.16: `s[i]` READ on a bare string formal is byte i of the
                // bound text, 0 out of range — exactly a string local's
                // `GetC` (the formal's register holds the text the
                // interpreter's frame binds).
                if self.method_mode
                    && self.string_formal_len_only(index)
                    && matches!(&expr.kind, ExprKind::Ident(h)
                        if h.root.is_none() && h.path.len() == 1 && h.path[0].selects.is_empty()
                            && self.string_formal_names.contains(h.path[0].name.name.as_str()))
                    && let Some(src) = self.string_local_reg(expr)
                {
                    let i = self.compile_expr(index, 32)?;
                    let dst = self.alloc_reg();
                    self.emit(Insn::StrOp(dst, StrOpKind::GetC, Box::new(vec![src, i])));
                    return Some(dst);
                }
                if !relaxed_formal_index
                    && (self.touches_string_formal(expr) || self.touches_string_formal(index))
                {
                    self.bail("string_formal_index");
                    return None;
                }
                // §6.16: `s[i]` on a string local reads byte i, 0 when out of
                // range, as `s.getc(i)`. The generic tail below bit-selected
                // the register, so `s[0] != "["` compared bit 0 of the text.
                if let Some(src) = self.string_local_reg(expr) {
                    let i = self.compile_expr(index, 32)?;
                    let dst = self.alloc_reg();
                    self.emit(Insn::StrOp(dst, StrOpKind::GetC, Box::new(vec![src, i])));
                    return Some(dst);
                }
                // §7.4.5: an unpacked row occupies multiple storage cells.
                // LoadCollElem reads a single cell; a second index would then
                // select a bit of that placeholder rather than a row element.
                if self.method_mode
                    && let Some((name, _, receiver)) = self.member_coll_elem_target(expr)
                    && let Some(owner) = self.coll_owner_class(receiver.as_ref())
                    && self.method_admission.as_ref()
                        .and_then(|facts| facts.instance_colls.get(&owner))
                        .is_some_and(|colls| colls.row_members.contains(&name))
                {
                    self.bail("unpacked_member_collection_row");
                    return None;
                }
                // class-perf Step 9b-ii: element READ of a member collection
                // (`q[i]`, `aa[k]`, `this.aa[k]`, `obj.aa[k]`) inside a
                // method body — lower to LoadCollElem (delegated exec). Sit
                // BEFORE the assoc-elem bail (which would keep the read on
                // the AST path) and before every module-scope arm, which
                // cannot resolve a member collection receiver.
                // §11.8.2: in an unsigned context a signed element is
                // converted BEFORE it is widened, so the read must not
                // sign-extend it to the context itself.
                let typed = self.method_mode
                    && unsigned_context
                    && ctx_width > 0
                    && self.expr_signedness(whole).is_some();
                if self.method_mode
                    && let Some(dest) = self.compile_member_coll_elem_read(
                        expr,
                        index,
                        if typed { 0 } else { ctx_width },
                    )
                {
                    if typed {
                        self.emit(Insn::ClearSigned(dest));
                        self.emit(Insn::Resize(dest, ctx_width));
                    }
                    return Some(dest);
                }
                // issue-301: a select INTO a member-collection element
                // (`arr[j][2]` on `bit [8:0][7:0] arr[3]`). The base is an
                // Index node, so the element-read arm above declined and
                // the generic tail below would BIT-select one bit of the
                // loaded element; for a multi-dimensional packed element
                // the select names a packed DIMENSION — an ew-bit slice.
                if self.method_mode
                    && let Some(dest) =
                        self.compile_member_coll_elem_packed_select(expr, index, ctx_width)
                {
                    return Some(dest);
                }
                if relaxed_formal_index {
                    // The relaxed index only ever flows here when the
                    // member-element read declined — nothing else may lower
                    // a string-formal index (Step 8).
                    self.bail("string_formal_index");
                    return None;
                }
                // class-perf method-mode: element selects on a CLASS-SCOPE
                // name (e.g. `g[i][j]` on an array member) must bail — arrays
                // live in their own storage, and falling through used to read
                // a bogus module-signal element (regression:
                // chained_handle_multidim). Locals are unaffected.
                if self.method_bares_to_shadowed(expr) {
                    self.bail("method_class_shadow");
                    return None;
                }
                // Register-bank local array with a CONSTANT (possibly folded
                // through an unrolled loop var) index: a plain register move.
                if let ExprKind::Ident(h) = &expr.kind {
                    if h.root.is_none() && h.path.len() == 1 && h.path[0].selects.is_empty() {
                        if let Some(&LocalArrayBank {
                            base,
                            width: ew,
                            len,
                            lo,
                            ..
                        }) = self.local_array_regs.get(&h.path[0].name.name)
                        {
                            let Some(iv) = self.fold_const(index).and_then(|v| v.to_u64()) else {
                                // Dynamic index into registers is impossible.
                                self.bail("local_array_dyn_index");
                                return None;
                            };
                            let slot = (iv as i64) - lo;
                            if slot < 0 || slot as usize >= len {
                                self.bail("local_array_oob");
                                return None;
                            }
                            let r = self.alloc_reg();
                            self.emit(Insn::Move(r, base + slot as RegId));
                            let _ = ew;
                            return Some(r);
                        }
                    }
                }
                // A packed-array field embedded in a packed struct has no
                // standalone signal id. Slice it directly from its container;
                // the generic non-Ident index path below would otherwise take
                // one bit instead of one packed element.
                let member_start = self.insns.len();
                let member_reg = self.next_reg;
                if let Some(dest) = self.compile_typed_packed_chain(expr, index) {
                    return Some(dest);
                }
                self.truncate_insns(member_start);
                self.next_reg = member_reg;
                if let Some(dest) = self.compile_packed_member_index(expr, index) {
                    return Some(dest);
                }
                self.truncate_insns(member_start);
                self.next_reg = member_reg;
                if let Some(dest) = self.compile_packed_member_index_chain(expr, index) {
                    return Some(dest);
                }
                self.truncate_insns(member_start);
                self.next_reg = member_reg;
                // §7.4.1 CHAINED packed element select — `v[i][j][k]` on
                // `logic [0:0][1:0][1:0]`. Only the innermost `Index` has an
                // Ident base, so the outer ones fell through to the bit select
                // below: `v[0]` gave a 4-bit slice, `[0]` then took ONE BIT of
                // it, and `[1]` ran off the end and produced x. The AST
                // interpreter walks the whole chain, so `$display` printed the
                // right bit while the same expression in an `assign` or an `if`
                // condition read x — a guard that never fired while the value
                // still looked correct in a print.
                //
                // Must precede the Ident-base branch, which by construction
                // only ever sees one level. Constant indices only: that is the
                // shape RTL uses (and what a genvar unrolls to), and it leaves
                // the single-level dynamic path untouched.
                // §7.4.1, §7.4.5: indices past an unpacked array's dimensions
                // select into the element's packed dimensions — `d[1][0]` on
                // `logic [1:0][7:0] d [2]` is a byte of element `d[1]`. The
                // packed chain below would take `[1]` as a packed index of the
                // root, and the bit select further down would take `[0]` as a
                // bit. An element that does not compile keeps the read on the
                // AST path, which walks the chain itself.
                // Without the declared range labels the select would be
                // placed by guess; the AST path reads it from the element's
                // declaration.
                if let Some((depth, (dims, labelled))) =
                    self.unpacked_elem_select_depth(expr, index)
                {
                    let r = if labelled {
                        self.compile_unpacked_elem_packed_select(expr, index, depth, &dims)
                    } else {
                        None
                    };
                    if r.is_none() {
                        self.bail("unpacked_elem_packed_select");
                    }
                    return r;
                }
                if let Some((lo, w)) = self.chained_packed_slice(expr, index) {
                    let base = self.compile_expr_root_of(expr)?;
                    let dest = self.alloc_reg();
                    self.emit(Insn::RangeSelectConst(dest, base, lo + w - 1, lo));
                    return Some(dest);
                }
                // Same chain with a DYNAMIC index somewhere — `a[i][j][3]`.
                if let Some(dest) = self.emit_chained_packed_slice_dyn(expr, index) {
                    return Some(dest);
                }
                // §11.5.1: `(X[a:b])[i]` on a packed ARRAY selects ELEMENT i
                // (labels pass through a constant part-select). This shape
                // comes from port inlining of `.p(arr[15:0])`-style
                // connections; the bit-select compilation below would read
                // bit i of a bit-slice. Bail to the AST interpreter, which
                // normalizes it to a plain element select.
                if let ExprKind::RangeSelect { expr: rs_base, .. } = &expr.kind {
                    if let ExprKind::Ident(h) = &rs_base.kind {
                        let nm = Self::hier_raw_name(h);
                        let elemish = self
                            .packed_elem_widths
                            .is_some_and(|m| m.get(&nm).is_some_and(|&ew| ew > 1))
                            || self
                                .packed_full_dims
                                .is_some_and(|m| m.get(&nm).is_some_and(|d| d.len() > 1));
                        if elemish {
                            self.bail("Index_of_ranged_packed_array");
                            return None;
                        }
                    }
                }
                // Packed element of a REGISTER-BACKED local (`x[i]` on a
                // `u8_vec16_t x` formal/local inside an inlined function):
                // slice [i*ew +: ew] straight out of the register. Assumes a
                // [N-1:0] outer range (what a packed-of-packed typedef
                // declares); locals with exotic outer bounds simply have no
                // elem entry and keep the old path.
                if let ExprKind::Ident(h) = &expr.kind {
                    let raw = Self::hier_raw_name(h);
                    if let Some(&ew) = self.local_var_elem.get(&raw) {
                        if let Some((src, _w)) = self.local_var_reg_of(h) {
                            let idx_reg = self.compile_expr(index, 0)?;
                            let ew_reg = self.alloc_reg();
                            self.emit(Insn::LoadConst(
                                ew_reg,
                                Box::new(Value::from_u64(ew as u64, 32)),
                            ));
                            let lo_reg = self.alloc_reg();
                            self.emit(Insn::Mul(lo_reg, idx_reg, ew_reg));
                            self.emit(Insn::Resize(lo_reg, 32));
                            let ewm1 = self.alloc_reg();
                            self.emit(Insn::LoadConst(
                                ewm1,
                                Box::new(Value::from_u64((ew - 1) as u64, 32)),
                            ));
                            let hi_reg = self.alloc_reg();
                            self.emit(Insn::Add(hi_reg, lo_reg, ewm1));
                            self.emit(Insn::Resize(hi_reg, 32));
                            let dest = self.alloc_reg();
                            self.emit(Insn::RangeSelect(dest, src, hi_reg, lo_reg));
                            return Some(dest);
                        }
                    }
                }
                // Element of a MULTI-dimensional unpacked array (`grid[i][j]`).
                // The base of the outer Index is itself an Index, so none of
                // the arms below match and the whole thing fell through to the
                // plain BIT-SELECT path: `grid[1][2]` compiled to bit 2 of bit
                // 1 of the array's base signal, and the read came back x.
                // Elements are stored under their flat name, so with constant
                // indices the element resolves directly. A dynamic index has no
                // flat name — leave those to the AST fallback, which handles
                // them.
                if let Some(id) = self.multi_dim_elem_signal_id(expr, index) {
                    let dest = self.alloc_reg();
                    self.emit(Insn::LoadSignal(dest, as_sig_id(id)));
                    return Some(dest);
                }
                // §7.4.1, §7.4.5: the AST fallback the note above relies on
                // was never reached for three or more dimensions: no arm
                // below matches, and the bit select at the end took
                // `a3[i][j][k]` as bits of the array's placeholder signal
                // (`000X`), and `a3[i][j][k][b]` as one of them.
                if self.nd_chain_names_no_element(expr, index) {
                    self.bail("nd_unpacked_dyn_index");
                    return None;
                }
                // Array element access
                if let ExprKind::Ident(hier) = &expr.kind {
                    // An ASSOCIATIVE array element has no dense storage and no
                    // bit offset: falling through compiled `aa["bob"]` as a
                    // BIT-SELECT of the array's placeholder signal, with the
                    // packed string as the bit index. Fail the compile so the
                    // read stays on the AST path.
                    if self.is_assoc_target(hier) {
                        self.bail("assoc_elem_read");
                        return None;
                    }
                    if let Some(name) = self.lookup_array_name(hier) {
                        let idx_reg = self.compile_expr(index, 0)?;
                        let dest = self.alloc_reg();
                        // §7.5 / §7.10: a dynamic array or queue reads its
                        // live collection (see `Insn::LoadDynElem`).
                        if self.is_dynamic_collection(&name) {
                            let cells = self
                                .array_first_id
                                .and_then(|m| m.get(name.as_str()).copied());
                            let size_key = format!("{name}.size").into_boxed_str();
                            self.emit(Insn::LoadDynElem(
                                dest,
                                Box::new(DynElemOp {
                                    name: name.into_boxed_str(),
                                    size_key,
                                    cells,
                                    ctx_width,
                                }),
                                idx_reg,
                            ));
                            return Some(dest);
                        }
                        let (array, idx_reg) = self.array_operand_index(name, idx_reg)?;
                        self.emit(Insn::LoadArrayElem(dest, array, idx_reg));
                        return Some(dest);
                    }
                    // Packed multi-D READ: `mem_q[i]` for `logic [N-1:0][W-1:0]`
                    // must extract a W-bit slice at `i*W +: W`, not a single
                    // bit. Mirror the LHS variable-index slice path so reads
                    // and writes stay symmetric.
                    let elem_w = self.packed_elem_width_of(hier);
                    if let Some(elem_w) = elem_w {
                        // Constant index (the common case — genvar-unrolled
                        // `idx_nodes[n] = idx_lut[k]` in rr_arb_tree/lzc, and
                        // any literal `b[4]`): emit a CONSTANT-range slice.
                        // The dynamic RangeSelect below produces a result whose
                        // width is only known at runtime; feeding that into a
                        // packed-2D element LHS write (BlockingAssignRangeDyn)
                        // mis-places the bits and the target reads back X. A
                        // RangeSelectConst carries a static width, so the LHS
                        // write lands correctly. (This was the FlooNOC "router
                        // never forwards" root cause: the arbiter's selected
                        // index came out X.)
                        // §11.5.1: an x/z literal index reads x; it folds to
                        // 0 here, so the run-time path below takes it.
                        if !self.is_xz_literal(index)
                            && let Some(idx) = self.eval_const_expr(index)
                        {
                            let base = self.compile_expr(expr, 0)?;
                            // §11.5.1: a constant index outside the declared
                            // range reads x. `eval_const_expr` folds in u32, so
                            // a `-1` (`conf_addr_i[i-1]` at genvar 0) arrived as
                            // 4294967295 and `lo + elem_w` overflowed.
                            let dim = self.packed_outer_dim(hier);
                            let in_range = match dim {
                                Some((l, r)) => (idx as i64) >= l.min(r) && (idx as i64) <= l.max(r),
                                None => (idx as u64).saturating_mul(elem_w as u64) < u32::MAX as u64,
                            };
                            if !in_range {
                                let dest = self.alloc_reg();
                                self.emit(Insn::LoadConst(dest, Box::new(Value::new(elem_w))));
                                return Some(dest);
                            }
                            let lo = Self::packed_elem_lsb(dim, idx as i64, elem_w).max(0) as u32;
                            let hi = lo + elem_w - 1;
                            let dest = self.alloc_reg();
                            self.emit(Insn::RangeSelectConst(dest, base, hi, lo));
                            return Some(dest);
                        }
                        // The element sits at a runtime offset inside the
                        // signal: slice it in place rather than copying the
                        // whole signal (a memory) into a register first.
                        let sig = self.lookup_signal_id(hier);
                        let idx_reg = self.compile_expr(index, 0)?;
                        // §7.4.1: normalize a DYNAMIC index against the
                        // declared outer range, exactly like the constant
                        // branch above and the write path. Without this,
                        // `src[i]` on a `[N:1]` outer dimension read slice
                        // i+1 — and the top element read out of range,
                        // injecting X into whatever it fed (a lane-expander's
                        // per-lane status flops all went X on the first
                        // multi-lane advance).
                        let idx_reg =
                            self.emit_packed_slot_index(self.packed_outer_dim(hier), idx_reg);
                        let elem_w_reg = self.alloc_reg();
                        self.emit(Insn::LoadConst(
                            elem_w_reg,
                            Box::new(Value::from_u64(elem_w as u64, 32)),
                        ));
                        let lo_reg = self.alloc_reg();
                        self.emit(Insn::Mul(lo_reg, idx_reg, elem_w_reg));
                        if let Some(id) = sig {
                            let dest = self.alloc_reg();
                            self.emit(Insn::LoadSignalRangeDyn(
                                dest,
                                as_sig_id(id),
                                lo_reg,
                                elem_w,
                            ));
                            return Some(dest);
                        }
                        let base = self.compile_expr(expr, 0)?;
                        // §11.5.1: an x/z index reads x at the element's width.
                        let dest = self.alloc_reg();
                        self.emit(Insn::RangeSelectW(dest, base, lo_reg, elem_w, true));
                        return Some(dest);
                    }
                }
                // Bit select
                //
                // §7.4.1: a non-zero-based vector stores its declared low bit
                // at PHYSICAL offset 0 — `logic [3:1] w` keeps declared bit 1
                // at offset 0, and `logic [1:1] h` is one bit at offset 0.
                // Both `Insn::BitSelect*` index raw physical bits, so the
                // declared index has to be rebased first. The WRITE path
                // already does this via `emit_rebased_index`; the read path
                // did not, so `h[1]` selected physical bit 1 of a one-bit
                // signal and evaluated to x. `$display("%b", h[1])` was
                // correct throughout because it goes through the AST
                // interpreter, which rebases — so the bug only showed in
                // assign / always_comb / always_ff, which compile to bytecode.
                //
                // Dynamic element of a 2-D unpacked array, nested-Index
                // shape (`TBL[i][j]`).
                if let ExprKind::Index {
                    expr: inner_e,
                    index: i_idx,
                } = &expr.kind
                {
                    if let ExprKind::Ident(h) = &inner_e.kind {
                        if h.path.len() == 1 && h.path[0].selects.is_empty() {
                            let (i_e, j_e, h2) =
                                (i_idx.as_ref().clone(), index.as_ref().clone(), h.clone());
                            if let Some(r) = self.compile_2d_array_read(&h2, &i_e, &j_e) {
                                return Some(r);
                            }
                        }
                    }
                }
                // Element of a register-bound LOCAL ARRAY (`row[n]` parsed
                // as Index over a bare Ident).
                if !self.local_var_array.is_empty() {
                    if let ExprKind::Ident(h) = &expr.kind {
                        if h.path.len() == 1
                            && h.path[0].selects.is_empty()
                            && self
                                .local_var_array
                                .contains_key(h.path[0].name.name.as_str())
                        {
                            let name = h.path[0].name.name.clone();
                            if let Some(r) = self.compile_local_array_read(&name, index) {
                                return Some(r);
                            }
                            self.bail("local_array_read");
                            return None;
                        }
                    }
                }
                // §7.4.1: a base whose labels need mapping compiles only when
                // that map is known here; the rest keep the AST path, which
                // applies the mapping itself.
                if self.sel_base_needs_ast(expr) && self.sel_base_dim(expr).is_none() {
                    self.bail("bit_sel_base_maps");
                    return None;
                }
                let base_start = self.insns.len();
                let base_reg = self.next_reg;
                let base = self.compile_expr(expr, 0)?;
                // An identifier the compiler could not resolve came back from
                // the interpreter as ONE value. Selecting from that is only
                // right when it really is a scalar, so hand the interpreter
                // the whole select instead.
                if self.sel_base_is_interpreted(base_start, expr) {
                    self.truncate_insns(base_start);
                    self.next_reg = base_reg;
                    if let Some(r) =
                        self.emit_expr_fallback(whole, ctx_width, "sel_base_interpreted")
                    {
                        return Some(r);
                    }
                    self.bail("sel_base_interpreted");
                    return None;
                }
                // Map the declared label to a physical bit: this covers an
                // ascending range and a dimension that does not start at zero.
                let sel_dim = self.sel_base_dim(expr);
                let base_lo = match &expr.kind {
                    ExprKind::Ident(h) => self.declared_low_bound(h),
                    _ => 0,
                };
                if let Some(idx) = self.eval_const_expr(index) {
                    let dest = self.alloc_reg();
                    // Saturate rather than wrap: an out-of-range declared index
                    // is already x-valued, and a negative operand would read as
                    // a huge unsigned bit position.
                    let phys = match sel_dim {
                        Some(d) => Self::label_to_phys(d, idx as i64),
                        None => idx as i64 - base_lo,
                    };
                    if phys < 0 {
                        // §11.5.1: below the declared low bound reads x.
                        self.emit(Insn::LoadConst(dest, Box::new(Value::new(1))));
                        return Some(dest);
                    }
                    self.emit(Insn::BitSelectConst(dest, base, phys as u32));
                    return Some(dest);
                }
                let idx = self.compile_expr(index, 0)?;
                let idx = match sel_dim {
                    Some(d) => self.emit_packed_slot_index(Some(d), idx),
                    None => idx,
                };
                let dest = self.alloc_reg();
                self.emit(Insn::BitSelect(dest, base, idx));
                Some(dest)
            }
            // §7.4.1: a part-select of a packed-array member of a struct
            // formal above its last dimension selects elements; only the
            // bits of a last-dimension element are compiled.
            ExprKind::RangeSelect { expr, .. } if self.formal_member_elem_range(expr) => {
                self.bail("formal_member_elem_range");
                None
            }
            ExprKind::RangeSelect {
                expr,
                left,
                right,
                kind,
                ..
            } => match kind {
                RangeKind::Constant => {
                    if self.sel_base_needs_ast(expr) && self.sel_base_dim(expr).is_none() {
                        self.bail("range_sel_base_maps");
                        return None;
                    }
                    let base_start = self.insns.len();
                    let base_reg = self.next_reg;
                    let base = self.compile_expr(expr, 0)?;
                    // An identifier the compiler could not resolve came back from
                    // the interpreter as ONE value. Selecting from that is only
                    // right when it really is a scalar, so hand the interpreter
                    // the whole select instead.
                    if self.sel_base_is_interpreted(base_start, expr) {
                        self.truncate_insns(base_start);
                        self.next_reg = base_reg;
                        if let Some(r) =
                            self.emit_expr_fallback(whole, ctx_width, "sel_base_interpreted")
                        {
                            return Some(r);
                        }
                        self.bail("sel_base_interpreted");
                        return None;
                    }
                    if let (Some(l), Some(r)) =
                        (self.eval_const_bound(left), self.eval_const_bound(right))
                    {
                        // §7.4.1: on a packed MULTI-D base (`logic [1:0][63:0]`
                        // or a packed array of a struct typedef), a constant
                        // range selects ELEMENTS — `pv[1:0]` is BOTH 64-bit
                        // slices (128 bits), not bits 1..0. Scale the bounds by
                        // the registered element width; a plain vector has no
                        // entry and keeps the historical bit-range meaning.
                        if let ExprKind::Ident(h) = &expr.kind {
                            if let Some(ew) = self.packed_elem_width_of(h).filter(|&w| w > 1) {
                                let dim = self.packed_outer_dim(h);
                                let lsb_l = Self::packed_elem_lsb(dim, l as i64, ew);
                                let lsb_r = Self::packed_elem_lsb(dim, r as i64, ew);
                                let lo = lsb_l.min(lsb_r).max(0) as u32;
                                let hi = (lsb_l.max(lsb_r) + ew as i64 - 1).max(0) as u32;
                                let dest = self.alloc_reg();
                                self.emit(Insn::RangeSelectConst(dest, base, hi, lo));
                                return Some(dest);
                            }
                        }
                        // §7.4.1: labels to physical bits, for an ascending
                        // range or an element dimension that does not start
                        // at zero as much as for the plain descending case.
                        // §7.4.3: after an index into a multi-D packed value
                        // the labels count ELEMENTS of the next dimension
                        // (`a[0][5:0]` on `logic [3:0][5:0][7:0] a` is 48
                        // bits), so the mapped slots scale by their width.
                        let elem = match &expr.kind {
                            ExprKind::Ident(_) => None,
                            _ => self.part_select_elem(expr),
                        };
                        let (mut phys_l, mut phys_r) = (l as i64, r as i64);
                        if let Some(d) = elem.map(|(d, _)| d).or_else(|| self.sel_base_dim(expr)) {
                            phys_l = Self::label_to_phys(d, phys_l);
                            phys_r = Self::label_to_phys(d, phys_r);
                            if phys_l < phys_r {
                                std::mem::swap(&mut phys_l, &mut phys_r);
                            }
                        }
                        if let Some((_, ew)) = elem {
                            let ew = i64::from(ew);
                            phys_l = phys_l * ew + ew - 1;
                            phys_r *= ew;
                        }
                        let dest = self.alloc_reg();
                        if phys_l.min(phys_r) < 0 {
                            // A bound below the declared low bound: the
                            // out-of-range positions read x (§11.5.1), which
                            // the signed dynamic form does per bit.
                            let lr = self.alloc_reg();
                            let rr = self.alloc_reg();
                            self.emit(Insn::LoadConst(
                                lr,
                                Box::new(Value::from_u64(phys_l as u32 as u64, 32)),
                            ));
                            self.emit(Insn::LoadConst(
                                rr,
                                Box::new(Value::from_u64(phys_r as u32 as u64, 32)),
                            ));
                            self.emit(Insn::RangeSelect(dest, base, lr, rr));
                            return Some(dest);
                        }
                        self.emit(Insn::RangeSelectConst(
                            dest,
                            base,
                            phys_l as u32,
                            phys_r as u32,
                        ));
                        return Some(dest);
                    }
                    // §7.4.1, §11.5.1: bounds the compiler cannot fold are
                    // still labels of the declared range. A descending range
                    // that does not start at 0 (`input [11:2] a`) shifts them
                    // to physical bits; an ascending one also mirrors their
                    // order, which only the interpreter's select does.
                    // Only for a plain vector: on a packed multi-D base the
                    // bounds select whole elements (`a[$clog2(2+2)-1:0]` on
                    // `logic [3:0][7:0] a` is two bytes), which the bit-range
                    // `RangeSelect` below does not scale, so that select is
                    // the interpreter's too.
                    let elems = matches!(&expr.kind, ExprKind::Ident(h)
                        if self.packed_elem_width_of(h).is_some_and(|w| w > 1));
                    let plain =
                        !elems && matches!(&expr.kind, ExprKind::Ident(_) | ExprKind::Index { .. });
                    let dim = self.sel_base_dim(expr).filter(|_| plain);
                    let interpreted = if elems {
                        Some("range_sel_dyn_elements")
                    } else if dim.is_some_and(|(l, r)| l < r) {
                        Some("range_sel_dyn_ascending")
                    } else {
                        None
                    };
                    if let Some(site) = interpreted {
                        self.truncate_insns(base_start);
                        self.next_reg = base_reg;
                        if let Some(r) = self.emit_expr_fallback(whole, ctx_width, site) {
                            return Some(r);
                        }
                        self.bail(site);
                        return None;
                    }
                    let mut l = self.compile_expr(left, 0)?;
                    let mut r = self.compile_expr(right, 0)?;
                    if let Some((_, lo)) = dim.filter(|&(_, lo)| lo != 0) {
                        let off = self.alloc_reg();
                        self.emit(Insn::LoadConst(
                            off,
                            Box::new(Value::from_u64(lo as u64, 32)),
                        ));
                        let (pl, pr) = (self.alloc_reg(), self.alloc_reg());
                        self.emit(Insn::Sub(pl, l, off));
                        self.emit(Insn::Sub(pr, r, off));
                        (l, r) = (pl, pr);
                    }
                    let dest = self.alloc_reg();
                    self.emit(Insn::RangeSelect(dest, base, l, r));
                    Some(dest)
                }
                RangeKind::IndexedUp | RangeKind::IndexedDown => {
                    if self.sel_base_needs_ast(expr) && self.sel_base_dim(expr).is_none() {
                        self.bail("range_sel_base_maps");
                        return None;
                    }
                    // `sig[idx +: W]` / `sig[idx -: W]` — W must be constant.
                    // Emit idx register, then compute hi/lo via Add/Sub with a
                    // const (W-1), and reuse existing RangeSelect insn.
                    let width = match self.eval_const_expr(right) {
                        Some(w) if w > 0 => w,
                        _ => {
                            self.bail("RangeSelect_width_nonconst");
                            return None;
                        }
                    };
                    let base_start = self.insns.len();
                    let base_reg = self.next_reg;
                    let base = self.compile_expr(expr, 0)?;
                    // An identifier the compiler could not resolve came back
                    // from the interpreter as ONE value. Selecting from that
                    // is only right when it really is a scalar, so hand the
                    // interpreter the whole select instead.
                    if self.sel_base_is_interpreted(base_start, expr) {
                        self.truncate_insns(base_start);
                        self.next_reg = base_reg;
                        if let Some(r) =
                            self.emit_expr_fallback(whole, ctx_width, "sel_base_interpreted")
                        {
                            return Some(r);
                        }
                        self.bail("sel_base_interpreted");
                        return None;
                    }
                    // §7.4.3: `[i +: W]` on a multi-D packed value counts W
                    // ELEMENTS of the selected dimension, each `ew` bits.
                    let elem = self.part_select_elem(expr);
                    // A constant base index (genvar-unrolled `x[n*W +: W]`)
                    // gives static bounds: select the range directly instead
                    // of computing `idx*W + W-1` in registers on every eval.
                    if let Some(c) = self.fold_const(left).and_then(|v| v.to_i64()) {
                        let (l, r) = if *kind == RangeKind::IndexedUp {
                            (c + width as i64 - 1, c)
                        } else {
                            (c, c - width as i64 + 1)
                        };
                        let plain = match &expr.kind {
                            ExprKind::Ident(h) => {
                                self.packed_elem_width_of(h).filter(|&w| w > 1).is_none()
                            }
                            // An index has already selected an element; what
                            // remains is a vector of bits or of elements.
                            ExprKind::Index { .. } => true,
                            _ => false,
                        };
                        if (plain || elem.is_some()) && r >= 0 {
                            // Both label bounds map, then order them: on an
                            // ascending base `+:` runs DOWN the physical bits.
                            let (mut phys_l, mut phys_r) = (l, r);
                            if let Some(d) =
                                elem.map(|(d, _)| d).or_else(|| self.sel_base_dim(expr))
                            {
                                phys_l = Self::label_to_phys(d, phys_l);
                                phys_r = Self::label_to_phys(d, phys_r);
                                if phys_l < phys_r {
                                    std::mem::swap(&mut phys_l, &mut phys_r);
                                }
                            }
                            if let Some((_, ew)) = elem {
                                let ew = i64::from(ew);
                                phys_l = phys_l * ew + ew - 1;
                                phys_r *= ew;
                            }
                            if phys_r >= 0 {
                                let dest = self.alloc_reg();
                                self.emit(Insn::RangeSelectConst(
                                    dest,
                                    base,
                                    phys_l as u32,
                                    phys_r as u32,
                                ));
                                return Some(dest);
                            }
                        }
                    }
                    let idx = self.compile_expr(left, 0)?;
                    let idx = match elem {
                        Some(_) => self.widen_part_select_base(left, idx),
                        None => idx,
                    };
                    // §7.4.6/§11.5.1: the base index is a DECLARED index, but
                    // `RangeSelect` takes physical bit offsets — rebase it for a
                    // non-zero-based vector exactly as the plain bit select
                    // does. Without this `w[1 +: 2]` on a `logic [3:1] w` read
                    // physical 2:1 (declared 3:2) instead of declared 2:1, and
                    // `w[3 -: 2]` ran off the top of the signal and returned x.
                    let sel_dim = elem.map(|(d, _)| d).or_else(|| self.sel_base_dim(expr));
                    let idx = match sel_dim {
                        Some(d) => self.emit_packed_slot_index(Some(d), idx),
                        None => idx,
                    };
                    // On an ascending base the mapped index counts the other
                    // way, so `+:` becomes a downward physical range.
                    let ascending = sel_dim.is_some_and(|d| d.0 < d.1);
                    let up = (*kind == RangeKind::IndexedUp) != ascending;
                    // Element slot to bit offset: the first bit of the slot
                    // for an upward range, its last bit for a downward one.
                    let (idx, width) = match elem {
                        Some((_, ew)) => (
                            self.emit_slot_bit(idx, ew, if up { 0 } else { ew - 1 }),
                            width.saturating_mul(ew),
                        ),
                        None => (idx, width),
                    };
                    let dest = self.alloc_reg();
                    self.emit(Insn::RangeSelectW(dest, base, idx, width, up));
                    Some(dest)
                }
            },
            ExprKind::Replication { count, exprs } => {
                let n = match self.eval_const_expr(count) {
                    Some(val) => val,
                    _ => {
                        self.bail("Replication_nonconst_count");
                        return None;
                    }
                };
                if n == 0 {
                    let dest = self.alloc_reg();
                    self.emit(Insn::LoadConst(dest, Box::new(Value::zero(0))));
                    return Some(dest);
                }
                if n > 10000 {
                    self.bail("Replication_excessive_count");
                    return None;
                }

                // Optimization: use Insn::Replicate if possible
                if exprs.len() == 1 {
                    let r = self.compile_expr(&exprs[0], 0)?;
                    let dest = self.alloc_reg();
                    self.emit(Insn::Replicate(dest, r, n));
                    return Some(dest);
                }

                let mut regs = Vec::with_capacity((exprs.len() * n as usize).max(1));
                for _ in 0..n {
                    for e in exprs {
                        let r = self.compile_expr(e, 0)?;
                        regs.push(r);
                    }
                }
                let dest = self.alloc_reg();
                self.emit(Insn::Concat(dest, Box::new(regs)));
                Some(dest)
            }
            ExprKind::Concatenation(parts) => {
                // LRM §11.4.12 — when any operand is a `string`, `{a, b, …}`
                // is a string concat (byte-level), not a bit-vector concat.
                // The bytecode `Concat` insn bit-concatenates and would
                // shift the bytes (e.g. a 5-char "hello" gets sized to 40
                // bits and aligned wrong), so for any string-valued operand
                // we bail to the AST interpreter which has the special
                // case at `eval_expr_ctx::Concatenation`.
                if parts.iter().any(|p| self.expr_is_string_concat_operand(p)) {
                    // §11.4.12: with every operand statically string-valued,
                    // this is a byte-level join — one native op. A MIXED
                    // concat (some operands of unknown type) keeps the AST
                    // path, whose type knowledge is authoritative.
                    if parts.iter().all(|p| self.expr_is_string_static(p)) {
                        let start = self.insns.len();
                        let start_reg = self.next_reg;
                        let mut regs: Vec<RegId> = Vec::with_capacity(parts.len());
                        let mut ok = true;
                        for p in parts {
                            match self.compile_expr(p, 0) {
                                Some(r) => regs.push(r),
                                None => {
                                    ok = false;
                                    break;
                                }
                            }
                        }
                        if ok {
                            let dst = self.alloc_reg();
                            self.emit(Insn::StrOp(dst, StrOpKind::Concat, Box::new(regs)));
                            return Some(dst);
                        }
                        self.truncate_insns(start);
                        self.next_reg = start_reg;
                    }
                    self.bail("Concat_string");
                    return None;
                }
                let mut regs = Vec::new();
                for p in parts {
                    let r = self.compile_expr(p, 0)?;
                    regs.push(r);
                }
                let dest = self.alloc_reg();
                self.emit(Insn::Concat(dest, Box::new(regs)));
                Some(dest)
            }
            // §11.4.14 streaming concatenation. `{>>{…}}` is exactly the
            // concatenation; `{<<N{…}}` additionally reverses the order of
            // N-bit slices, which with a constant N and a known total width is
            // a FIXED bit permutation — a concat of constant range selects.
            // Previously the whole expression fell to the AST interpreter,
            // measured at ~0.45us per evaluation (a byte-swap written as
            // `{<<8{x}}` ran ~32% slower than the same swap written out by
            // hand).
            ExprKind::StreamOp {
                left_to_right,
                slice_size,
                exprs,
            } => {
                // Widths must be known to place the slices; a part whose width
                // the compiler cannot size keeps the AST path.
                // The LRM self-determined width, cross-checked against
                // `expr_max_width`: the latter is unreliable for an index
                // select (it reports 1 for an element of a packed-struct
                // typedef array), and a wrong total silently permutes the
                // wrong bits. Disagreement means the width is not established
                // well enough to place slices — keep the AST path.
                // Placing the slices needs each part's width to be exactly
                // right — a wrong total silently permutes the wrong bits, and
                // the general width oracles are not trustworthy enough here:
                // BOTH `lrm_self_width` and `expr_max_width` report 1 for an
                // element of a packed-struct typedef array in a submodule, so
                // cross-checking them does not catch it. Accept only shapes
                // whose width is unambiguous here and leave every other
                // spelling on the (correct) AST path.
                let mut widths: Vec<u32> = Vec::with_capacity(exprs.len());
                for e in exprs {
                    let mut inner = e;
                    while let ExprKind::Paren(i) = &inner.kind {
                        inner = i;
                    }
                    let w = match &inner.kind {
                        // A whole signal: the signal table is authoritative.
                        ExprKind::Ident(h) if h.path.iter().all(|s| s.selects.is_empty()) => self
                            .lookup_signal_id(h)
                            .and_then(|id| self.signal_widths.get(id).copied())
                            .unwrap_or(0),
                        // `x[hi:lo]` with constant bounds: hi - lo + 1
                        // ELEMENTS of `x`'s outer packed dimension, each as
                        // wide as one element (§7.4.3, §11.4.14): a slice of
                        // `lane_t [-9:9] x` is eleven bits per element.
                        ExprKind::RangeSelect {
                            expr: base,
                            left,
                            right,
                            kind: RangeKind::Constant,
                        } => match (
                            self.eval_const_bound(left),
                            self.eval_const_bound(right),
                            self.packed_part_select_elem_width(base),
                        ) {
                            (Some(hi), Some(lo), Some(ew)) => {
                                ((hi - lo).unsigned_abs() as u32 + 1).saturating_mul(ew)
                            }
                            _ => 0,
                        },
                        _ => 0,
                    };
                    if w == 0 {
                        self.bail("Stream_operand_shape");
                        return None;
                    }
                    widths.push(w);
                }
                let total_w: u32 = widths.iter().sum();
                let mut regs = Vec::with_capacity(exprs.len());
                for e in exprs {
                    regs.push(self.compile_expr(e, 0)?);
                }
                let src = self.alloc_reg();
                self.emit(Insn::Concat(src, Box::new(regs)));
                if !*left_to_right {
                    return Some(src);
                }
                // `{<<N{…}}`: N must be a constant to know the chunking.
                let slice = match slice_size {
                    None => 1u32,
                    Some(e) => match self.fold_const(e).and_then(|v| v.to_u64()) {
                        Some(n) if n > 0 && n <= u32::MAX as u64 => n as u32,
                        _ => {
                            self.bail("Stream_slice_nonconst");
                            return None;
                        }
                    },
                };
                if slice >= total_w {
                    // One chunk (plus nothing to reverse): identity.
                    return Some(src);
                }
                let full = total_w / slice;
                let rem = total_w - full * slice;
                // Output MSB-first is chunk0, chunk1, … chunk(full-1), then the
                // leftover high bits of the source. Chunk k occupies source
                // bits [k*slice + slice-1 : k*slice].
                let mut parts: Vec<RegId> = Vec::with_capacity(full as usize + 1);
                let range = |me: &mut Self, hi: u32, lo: u32| -> RegId {
                    let hr = me.alloc_reg();
                    me.emit(Insn::LoadConst(
                        hr,
                        Box::new(Value::from_u64(hi as u64, 32)),
                    ));
                    let lr = me.alloc_reg();
                    me.emit(Insn::LoadConst(
                        lr,
                        Box::new(Value::from_u64(lo as u64, 32)),
                    ));
                    let d = me.alloc_reg();
                    me.emit(Insn::RangeSelect(d, src, hr, lr));
                    d
                };
                for k in 0..full {
                    let lo = k * slice;
                    parts.push(range(self, lo + slice - 1, lo));
                }
                if rem > 0 {
                    parts.push(range(self, total_w - 1, full * slice));
                }
                let dst = self.alloc_reg();
                self.emit(Insn::Concat(dst, Box::new(parts)));
                Some(dst)
            }
            ExprKind::SystemCall { name, args } => match name.as_str() {
                // §6.24.3: a cast whose operand is an UNPACKED array/queue
                // packs the operand's elements (`int'(barr)`); the register
                // path would read the container signal. Interpreted.
                "$__xz_named_cast" | "$__xz_size_cast" | "$__xz_type_cast"
                    if args
                        .get(1)
                        .is_some_and(|a| self.names_unpacked_collection(a)) =>
                {
                    if let Some(r) = self.emit_expr_fallback(expr, ctx_width, "cast_unpacked") {
                        return Some(r);
                    }
                    self.bail("cast_unpacked");
                    None
                }
                crate::compiler::simulator::COV_COND_FN => self.compile_cov_cond(args),
                // §20.8.1: `$clog2` of a constant (a parameter, a literal) is
                // that constant; anything else keeps the interpreter call.
                "$clog2" if args.len() == 1 && self.fold_const(expr).is_some() => {
                    let v = self.fold_const(expr)?;
                    let dest = self.alloc_reg();
                    self.emit(Insn::LoadConst(dest, Box::new(v)));
                    Some(dest)
                }
                // §21.3.3 `$sformatf` with a LITERAL template and specs the
                // native filler covers exactly — parsed once here, filled
                // from register Values at exec. Anything else (non-literal
                // fmt, %t/%p/%m/…, arg-count mismatch) keeps the AST path.
                "$sformatf" | "$psprintf" => {
                    let mut native: Option<(Vec<FmtSeg>, Vec<RegId>)> = None;
                    if let Some(ExprKind::StringLiteral(fmt)) = args.first().map(|a| &a.kind) {
                        if let Some((segs, nargs)) = Self::parse_format_template(fmt) {
                            if nargs == args.len() - 1 {
                                let start = self.insns.len();
                                let start_reg = self.next_reg;
                                let mut arg_regs: Vec<RegId> = Vec::with_capacity(nargs);
                                let mut ok = true;
                                for a in &args[1..] {
                                    match self.compile_expr(a, 0) {
                                        Some(r) => arg_regs.push(r),
                                        None => {
                                            ok = false;
                                            break;
                                        }
                                    }
                                }
                                if ok {
                                    native = Some((segs, arg_regs));
                                } else {
                                    self.truncate_insns(start);
                                    self.next_reg = start_reg;
                                }
                            }
                        }
                    }
                    let Some((mut segs, arg_regs)) = native else {
                        // Same escape hatch as the `other` arm: one
                        // expression-level fallback, not a whole-stmt bail.
                        if let Some(r) =
                            self.emit_expr_fallback(expr, ctx_width, "SystemCall_sformatf")
                        {
                            return Some(r);
                        }
                        self.bail("SystemCall_sformatf");
                        return None;
                    };
                    let mut ai = 0usize;
                    for seg in segs.iter_mut() {
                        if let FmtSeg::Spec {
                            spec, str_valued, ..
                        } = seg
                        {
                            if *spec == 's' {
                                *str_valued = self.expr_is_string_static(&args[1 + ai]);
                            }
                            ai += 1;
                        }
                    }
                    let dst = self.alloc_reg();
                    self.emit(Insn::Format(
                        dst,
                        Box::new(FormatData {
                            segs,
                            args: arg_regs,
                        }),
                    ));
                    Some(dst)
                }
                "$signed" | "$unsigned" => {
                    // §6.24.1: the sign conversion consumes a literal's fill.
                    let arg = args.first()?;
                    let signed = name == "$signed";
                    let mut operand = arg;
                    while let ExprKind::Paren(inner) = &operand.kind {
                        operand = inner;
                    }
                    if let ExprKind::Number(n) = &operand.kind {
                        let mut v = self.eval_number_static(n)?;
                        v.is_fill = false;
                        v.is_signed = signed;
                        let dst = self.alloc_reg();
                        self.emit(Insn::LoadConst(dst, Box::new(v)));
                        return Some(dst);
                    }
                    let r = self.compile_expr(arg, 0)?;
                    self.emit(if signed { Insn::SetSigned(r) } else { Insn::ClearSigned(r) });
                    Some(r)
                }
                "$__xz_size_cast" => {
                    // §6.24.1 `N'(x)`: evaluate x in context width N,
                    // then resize. N is a constant expression, folded here.
                    let n = args.first().and_then(|a| self.eval_const_expr(a));
                    let Some(n) = n.filter(|&n| n > 0) else {
                        self.bail("SystemCall_size_cast_width");
                        return None;
                    };
                    let r = self.compile_expr(args.get(1)?, n)?;
                    self.emit(Insn::Resize(r, n));
                    Some(r)
                }
                // §20.6.2: `$bits` of a statically-known operand is a
                // COMPILE-TIME constant. Restricted to the shapes the
                // tables answer exactly: a name in `cast_widths` (typedef
                // or enum — the `$bits(ibex_mubi_t)` form ibex's unused-
                // signal reductions use ~4x per cycle) or a plain SIGNAL
                // (declared width). Anything else — strings, class
                // handles, unpacked aggregates — keeps the interpreter.
                "$bits" => {
                    let w: Option<u32> = args.first().and_then(|a| match &a.kind {
                        ExprKind::Ident(h)
                            if h.root.is_none()
                                && h.path.len() == 1
                                && h.path[0].selects.is_empty() =>
                        {
                            let nm = &h.path[0].name.name;
                            self.cast_widths
                                .and_then(|m| m.get(nm).map(|&(w, _)| w))
                                .or_else(|| {
                                    self.lookup_signal_id(h).map(|id| self.signal_widths[id])
                                })
                        }
                        _ => None,
                    });
                    if let Some(w) = w.filter(|&w| w > 0) {
                        // §20.6.2: `$bits` returns an `integer` — signed,
                        // so `$bits(a) - 200` is negative, not 2**32 - 72.
                        let mut c = Value::from_u64(w as u64, 32);
                        c.is_signed = true;
                        let r = self.alloc_reg();
                        self.emit(Insn::LoadConst(r, Box::new(c)));
                        Some(r)
                    } else {
                        if let Some(r) = self.emit_expr_fallback(expr, ctx_width, "SystemCall_bits")
                        {
                            return Some(r);
                        }
                        self.bail("SystemCall_bits");
                        None
                    }
                }
                // §6.24.1 named cast, statically resolvable target. The
                // cast type is the CONTEXT for its operand, so the operand
                // compiles at the target width, then Resize + sign mark.
                // A Call operand keeps the interpreter path (it may return
                // a collection the runtime packs — see the AST handler),
                // as does a target that is a runtime signal or a real type.
                // §6.24.1 type cast (`real'(x)`, `int'(x)`) — mirror
                // the interpreter: operand in the target's context, then
                // convert. A REAL target converts numerically (`emit_to_real`);
                // an integral target resizes and takes the type's
                // signedness (a real operand rounds per §10.7 inside
                // `Value::resize`). Stream operands and exotic targets
                // keep the AST path.
                "$__xz_type_cast" => {
                    let dt = match args.first().map(|a| &a.kind) {
                        Some(ExprKind::TypeLiteral(dt)) => dt.clone(),
                        _ => {
                            self.bail("type_cast_shape");
                            return None;
                        }
                    };
                    let inner = args.get(1)?;
                    fn is_stream(e: &Expression) -> bool {
                        match &e.kind {
                            ExprKind::StreamOp { .. } => true,
                            ExprKind::Paren(i) => is_stream(i),
                            _ => false,
                        }
                    }
                    if self.decl_is_shortreal(&dt, None) {
                        return self.emit_expr_fallback(expr, ctx_width, "shortreal_cast");
                    }
                    if is_stream(inner) {
                        if let Some(r) =
                            self.emit_expr_fallback(expr, ctx_width, "type_cast_stream")
                        {
                            return Some(r);
                        }
                        self.bail("type_cast_stream");
                        return None;
                    }
                    let is_real = self.decl_is_real(&dt, None);
                    let w = crate::compiler::elaborate::resolve_type_width(&dt, self.params, None)
                        .max(1);
                    // §6.24.1: an integral target is the operand's context
                    // width (`int'(a + b)` sums 8-bit operands at 32 bits);
                    // `string'` and real targets leave it self-determined.
                    let integral = matches!(
                        dt.as_ref(),
                        crate::ast::types::DataType::IntegerAtom { .. }
                            | crate::ast::types::DataType::IntegerVector { .. }
                    );
                    let src = self.compile_expr(inner, if integral { w } else { 0 })?;
                    // §6.16/§6.24.3: a `string` target CONVERTS the operand
                    // to a string value — every NUL byte removed. Resizing to
                    // the 1024-bit string placeholder instead let each
                    // character appended by {out, string'(c)} carry 127 NUL
                    // bytes into the concat (#391).
                    if self.decl_is_string(&dt, None) {
                        let r = self.alloc_reg();
                        self.emit(Insn::StrOp(r, StrOpKind::ToStr, Box::new(vec![src])));
                        return Some(r);
                    }
                    let r = self.alloc_reg();
                    self.emit(Insn::Move(r, src));
                    if is_real {
                        self.emit_to_real(r);
                        return Some(r);
                    }
                    // §6.24.1, §6.12.1: `longint'(r)` rounds a real operand.
                    self.emit_real_to_int_fit(r, inner, w);
                    self.emit(Insn::Resize(r, w));
                    // §6.24.1: the cast value is what a variable of the
                    // target type holds, so a 2-state target (`int'`,
                    // `bit [7:0]'`) maps x/z bits to 0.
                    if crate::compiler::elaborate::is_type_two_state(&dt) {
                        self.emit(Insn::DropXz(r));
                    }
                    if crate::compiler::elaborate::is_type_signed(&dt) {
                        self.emit(Insn::SetSigned(r));
                    } else {
                        self.emit(Insn::ClearSigned(r));
                    }
                    Some(r)
                }
                "$__xz_named_cast" => {
                    if self.cast_arg_targets_string(args) {
                        let src = self.compile_expr(args.get(1)?, 0)?;
                        let result = self.alloc_reg();
                        self.emit(Insn::StrOp(result, StrOpKind::ToStr, Box::new(vec![src])));
                        return Some(result);
                    }
                    // `pkg::T'(x)` prefers the qualified `pkg::T` entry.
                    let target = args.first().and_then(|a| match &a.kind {
                        // §8.23: a scoped `cls::N` identifier path prefers
                        // its qualified entry.
                        ExprKind::Ident(h) if h.path.len() >= 2 => {
                            let n = h.path.len();
                            let key =
                                format!("{}::{}", h.path[n - 2].name.name, h.path[n - 1].name.name);
                            let known = self.cast_widths.is_some_and(|m| m.contains_key(&key))
                                || self.params.is_some_and(|p| p.contains_key(&key));
                            Some(if known {
                                key
                            } else {
                                h.path[n - 1].name.name.clone()
                            })
                        }
                        ExprKind::Ident(h) => h.path.last().map(|s| s.name.name.clone()),
                        ExprKind::MemberAccess { expr, member } => match &expr.kind {
                            ExprKind::Ident(h) => {
                                let key = format!("{}::{}", h.path.last()?.name.name, member.name);
                                let known = self.cast_widths.is_some_and(|m| m.contains_key(&key))
                                    || self.params.is_some_and(|p| p.contains_key(&key));
                                Some(if known { key } else { member.name.clone() })
                            }
                            _ => None,
                        },
                        _ => None,
                    });
                    // `8'(x)` — the size is a literal, no name lookup.
                    let literal_w: Option<u32> = args.first().and_then(|a| {
                        if let ExprKind::Number(n) = &a.kind {
                            self.eval_number_static(n)
                                .and_then(|v| v.to_u64())
                                .map(|n| (n as u32).max(1))
                        } else {
                            None
                        }
                    });
                    let inner_is_call =
                        matches!(args.get(1).map(|a| &a.kind), Some(ExprKind::Call { .. }));
                    // §6.24.1: a cast whose operand is a STREAMING concat yields its
                    // elements — route the natural-width (no element-collapse)
                    // interpretation to the AST, which knows the target's packed
                    // vs unpacked shape (see the named-cast arm there). This is the
                    // shape uvm_reg_map's byte-shift relies on:
                    //   `p = {<< 8 {bit_q_t'({<< {bits}}) }}`
                    {
                        fn is_stream(e: &Expression) -> bool {
                            match &e.kind {
                                ExprKind::StreamOp { .. } => true,
                                ExprKind::Paren(i) => is_stream(i),
                                _ => false,
                            }
                        }
                        if args.get(1).is_some_and(is_stream) {
                            if let Some(r) =
                                self.emit_expr_fallback(expr, ctx_width, "named_cast_stream")
                            {
                                return Some(r);
                            }
                            self.bail("named_cast_stream");
                            return None;
                        }
                    }
                    // (width, signedness): a SIZE cast (`8'(x)`, `N'(x)`)
                    // keeps the operand's signedness (§6.24.1), a type
                    // cast takes the type's.
                    // A struct-parameter field names no type, so its leaf
                    // must not be looked up as one below.
                    let member_w = args.first().and_then(|a| self.member_cast_size(a));
                    let target = if member_w.is_some() { None } else { target };
                    let known = literal_w.or(member_w).map(|w| (w, None)).or_else(|| {
                        target.as_ref().and_then(|nm| {
                            self.cast_widths
                                .and_then(|m| m.get(nm).copied())
                                .map(|(w, signed)| (w, Some(signed)))
                                .or_else(|| {
                                    // Parameter-valued SIZE cast: `N'(x)` with
                                    // N a constant parameter.
                                    self.params
                                        .and_then(|p| p.get(nm))
                                        .and_then(|v| v.to_u64())
                                        .map(|n| ((n as u32).max(1), None))
                                })
                        })
                    });
                    if let (Some((w, signed)), false) = (known, inner_is_call) {
                        // §6.24.1: the target width is the operand's
                        // context — `my_int_t'(a + b)` over 8-bit operands
                        // sums at 32 bits (a narrowing cast is unaffected:
                        // the context never drops below the operand's own
                        // width). Mirrors the interpreter.
                        let src = self.compile_expr(args.get(1)?, w)?;
                        // NEVER resize `src` in place: for a bare local
                        // (a loop variable, say) compile_expr hands back
                        // the variable's OWN register, and an in-place
                        // Resize would truncate the variable itself —
                        // `NumBitsDeviceSel'(device)` inside ibex's bus
                        // arbiter loop corrupted `device` for the rest of
                        // the loop exactly this way.
                        let r = self.alloc_reg();
                        self.emit(Insn::Move(r, src));
                        // §6.24.1, §6.12.1: a cast to a 64-bit integral type
                        // (`xlen_t'(r)`) rounds a real operand.
                        if signed.is_some() && !self.named_cast_is_real(args) {
                            self.emit_real_to_int_fit(r, &args[1], w);
                        }
                        self.emit(Insn::Resize(r, w));
                        // §6.24.1: a named two-state type also converts X/Z.
                        if let Some(types) = self.typedef_types {
                            if target
                                .as_ref()
                                .and_then(|name| types.get(name))
                                .is_some_and(|dt| {
                                    crate::compiler::elaborate::is_type_two_state_resolved(
                                        dt, types,
                                    )
                                })
                            {
                                self.emit(Insn::DropXz(r));
                            }
                        }
                        match signed {
                            Some(true) => self.emit(Insn::SetSigned(r)),
                            Some(false) => self.emit(Insn::ClearSigned(r)),
                            None => {}
                        }
                        Some(r)
                    } else {
                        if let Some(r) =
                            self.emit_expr_fallback(expr, ctx_width, "SystemCall_named_cast")
                        {
                            return Some(r);
                        }
                        self.bail("SystemCall_named_cast");
                        None
                    }
                }
                // class-perf Step 9c: `$cast(dest, src)` in a compiled
                // method body. Method mode only — the module-level
                // (always-block) path keeps its existing fallback
                // behavior via the `other` arm below.
                "$cast" if self.method_mode => {
                    match self.compile_cast_call(args, ctx_width, false) {
                        Some(r) => Some(r),
                        None => {
                            self.bail("SystemCall_cast");
                            None
                        }
                    }
                }
                other => {
                    let _ = other;
                    if std::env::var_os("XEZIM_PROBE_SYSCALL").is_some() {
                        eprintln!("[SYSCALL_FALLBACK] {}", name);
                    }
                    if let Some(r) = self.emit_expr_fallback(expr, ctx_width, "SystemCall_other") {
                        return Some(r);
                    }
                    self.bail("SystemCall_other");
                    None
                }
            },
            ExprKind::MemberAccess { expr: base, member } => {
                // §6.6.7/§7.2: a member of a pure function's unpacked-struct
                // result (`res('{...}).V`, every struct nettype's resolved
                // value) inlines over scalars.
                if let ExprKind::Call { func, args } = &base.kind
                    && let Some(r) = self.compile_struct_member_call(func, args, &member.name)
                {
                    return Some(r);
                }
                if let Some(dest) = self.compile_formal_struct_member(expr) {
                    return Some(dest);
                }
                // §9.7: `p.status` / `p.status()` on a process handle
                if member.name == "status"
                    && self
                        .method_handle_chain_class(base)
                        .is_some_and(|cls| cls == "process")
                {
                    let handle_reg = self.compile_expr(base, 0)?;
                    let dest = self.alloc_reg();
                    self.emit(Insn::LoadProcessStatus(dest, handle_reg));
                    return Some(dest);
                }
                // class-perf method-mode: `this.<member>`/`<classlocal>.<member>`
                // is an OBJECT-field read off the heap, not a hierarchical
                // signal. Only the literal `this` keyword or a method-local
                // bound as a CLASS HANDLE qualifies; anything else keeps the
                // pre-existing (signal/struct) paths below.
                if let Some(v) = self.class_scoped_enum(base, &member.name) {
                    let dest = self.alloc_reg();
                    self.emit(Insn::LoadConst(dest, Box::new(v)));
                    return Some(dest);
                }
                if self.method_mode {
                    // class-perf: a member read rooted at an ELEMENT of a
                    // class-handle member collection (`m[k].data`,
                    // `this.aa[k].field`). Checked before the handle-chain
                    // path and the signal/struct arms below, which cannot
                    // resolve an indexed collection receiver.
                    if let Some(dest) = self.compile_member_handle_elem_member(base, &member.name) {
                        return Some(dest);
                    }
                    // Step 6: the base may be a whole chain (`a.b`,
                    // `this.m_parent`, a bare member) — lower it to its
                    // handle register first, then read `member` off that
                    // handle. `method_member_handle_reg` EMITS the base load
                    // for a bare member (`m.f` reads this.m first).
                    let base_reg = if self.method_handle_chain_ok(base) {
                        match self.method_member_handle_reg(base) {
                            Some(r) => Some(r),
                            None => self.compile_expr(base, 0),
                        }
                    } else {
                        None
                    };
                    if let Some(handle_reg) = base_reg {
                        if !self.method_instance_field_ok(base, &member.name) {
                            if let Some(class) = self.method_handle_chain_class(base) {
                                if self.static_member_names.contains(member.name.as_str()) {
                                    let dest = self.alloc_reg();
                                    self.emit(Insn::LoadClassStatic(
                                        dest,
                                        Box::new(NamePair(
                                            class.into_boxed_str(),
                                            member.name.clone().into_boxed_str(),
                                        )),
                                    ));
                                    return Some(dest);
                                }
                            }
                            self.bail("method_receiver_field");
                            return None;
                        }
                        // The interpreter's dotted read uses the instance's
                        // runtime key (`properties["m"]`) — the same key
                        // LoadClassMember reads — so an extends-chain INSTANCE
                        // property is faithful even when shadowed (Step 6,
                        // `member_safe_names`). Shadowed names NOT in that set
                        // (statics/localparams, enclosing-class members,
                        // non-loadable types) keep the AST path.
                        if self.class_shadow_names.contains(member.name.as_str())
                            && !self.member_safe_names.contains(member.name.as_str())
                            // Step 9a: a string member's heap slot holds the
                            // byte-vector Value verbatim (no width-fitting),
                            // so a dotted read is as faithful as an integral
                            // one. It is in neither `member_safe_names` nor
                            // `bare_member_names` (nonloadable), so admit it
                            // explicitly.
                            && !(self.string_member_names.contains(member.name.as_str())
                                && !self.member_safe_names.contains(member.name.as_str()))
                        {
                            self.bail("method_member_noload");
                            return None;
                        }
                        let dest = self.alloc_reg();
                        self.emit(Insn::LoadClassMember(
                            dest,
                            handle_reg,
                            member.name.clone().into_boxed_str(),
                        ));
                        return Some(dest);
                    }
                }
                let member_start = self.insns.len();
                let member_reg = self.next_reg;
                if let Some(dest) = self.compile_indexed_packed_member(base, &member.name) {
                    return Some(dest);
                }
                self.truncate_insns(member_start);
                self.next_reg = member_reg;
                // `b.descriptors[0].region` parses with the element select on
                // the LAST path segment (`b.descriptors[0]` is one identifier):
                // peel that segment into member + index.
                if let ExprKind::Ident(h) = &base.kind
                    && h.root.is_none()
                    && h.path.len() >= 2
                    && h.path.last().is_some_and(|s| s.selects.len() == 1)
                    && h.path[..h.path.len() - 1]
                        .iter()
                        .all(|s| s.selects.len() <= 1)
                {
                    let mut root_hier = h.clone();
                    let last = root_hier.path.pop().unwrap();
                    let mname = last.name.name.clone();
                    let index = last.selects[0].clone();
                    if let Some(dest) =
                        self.compile_indexed_folded_member(&root_hier, &mname, &index, &member.name)
                    {
                        return Some(dest);
                    }
                }
                self.truncate_insns(member_start);
                self.next_reg = member_reg;

                // Direct packed member (`container.field`). Nested field paths
                // are already flattened in the layout table.
                let direct_start = self.insns.len();
                let direct_reg = self.next_reg;
                if let Some((root, key, hier, fields)) = self.compile_packed_struct_value(base)
                    && let Some((_, off, width)) =
                        fields.iter().find(|(name, _, _)| name == &member.name)
                {
                    let dest = self.alloc_reg();
                    self.emit(Insn::RangeSelectConst(dest, root, *off + *width - 1, *off));
                    // §7.2.1: the member keeps its declared type's sign.
                    let var = Self::hier_raw_name(&hier);
                    if self
                        .packed_member_signed(&key, &member.name)
                        .or_else(|| self.packed_member_signed(&var, &member.name))
                        == Some(true)
                    {
                        self.emit(Insn::SetSigned(dest));
                    }
                    return Some(dest);
                }
                self.truncate_insns(direct_start);
                self.next_reg = direct_reg;
                // class-perf P2: a PACKAGE-scope enum constant parsed as
                // MemberAccess (`pk::LOW` in an argument). Same fold as the
                // 2-segment Ident form; a 3D base (queue element
                // member-of-member) must NOT be caught by this (base is a
                // lone Ident there only when it's a name, which a package
                // never is — hence root.is_none + single bare segment).
                if self.method_mode
                    && let ExprKind::Ident(h) = &base.kind
                    && h.root.is_none()
                    && h.path.len() == 1
                    && h.path[0].selects.is_empty()
                {
                    let key = format!("{}::{}", h.path[0].name.name, member.name);
                    if let Some(&(val, width)) = self.pkg_enum_consts.get(&key) {
                        let r = self.alloc_reg();
                        self.emit(Insn::LoadConst(
                            r,
                            Box::new(Value::from_u64(val, width.max(1))),
                        ));
                        return Some(r);
                    }
                }
                if let Some(r) = self.emit_expr_fallback(expr, ctx_width, "Expr_MemberAccess") {
                    return Some(r);
                }
                self.bail("Expr_MemberAccess");
                None
            }
            // §13.4: inline a PURE function call — one whose body is a single
            // assignment to the function name (or a single `return`) over input
            // formals. That is the overwhelmingly common combinational-helper
            // shape in RTL (`lfsr32(s)`, `mix(a,b)`), and leaving it to the AST
            // interpreter dragged the whole enclosing block out of bytecode.
            // An assignment pattern outside an assignment-like context has
            // no type (§10.9); `compile_expr_typed` compiles typed and
            // context-typed ones.
            ExprKind::AssignmentPattern(_) | ExprKind::TypedPattern { .. } => {
                self.bail("Expr_AssignmentPattern");
                None
            }
            // §11.4.13 set membership, restricted to what makes `==?`
            // degenerate to `==`: every member a compile-time constant with
            // no x/z bits. That is the enum-list shape ibex's decoder and CSR
            // logic use (`csr_op inside {CSR_OP_WRITE, ...}`) — ~2.6 such
            // evaluations per cycle ran interpreted. An x in the OPERAND
            // still propagates exactly per LRM: Eq yields x, and x|1 = 1,
            // x|0 = x. Ranges and wildcard members keep the interpreter.
            ExprKind::Inside { expr: e, ranges } => {
                // Step 8 (string formals): `inside` on a string formal takes
                // the packed wildcard match; interpreter string semantics
                // (leading-NUL trim) differ — bail.
                if self.touches_string_formal(e) {
                    self.bail("string_formal_inside");
                    return None;
                }
                let mut members: Vec<Value> = Vec::with_capacity(ranges.len());
                let mut ok = true;
                for m in ranges {
                    match self.inside_member_const(m) {
                        Some(v) => members.push(v),
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                // §11.4.13, §11.4.5: a real operand is compared with `==`,
                // converting each member to real, so the members must keep
                // their own sign; the unsigned compare below is integral only.
                if ok && self.inside_needs_numeric_eq(e, &members) {
                    ok = false;
                }
                if !ok || members.is_empty() {
                    if let Some(r) = self.emit_expr_fallback(expr, ctx_width, "Expr_Inside") {
                        return Some(r);
                    }
                    self.bail("Expr_Inside");
                    return None;
                }
                let wmax = members
                    .iter()
                    .map(|v| v.width)
                    .fold(self.lrm_self_width(e), u32::max)
                    .max(1);
                let src = self.compile_expr(e, wmax)?;
                // Fresh register: Resize/ClearSigned must not mutate a shared
                // one (same rule as the named cast above).
                let er = self.alloc_reg();
                self.emit(Insn::Move(er, src));
                self.emit(Insn::Resize(er, wmax));
                self.emit(Insn::ClearSigned(er));
                let mut acc: Option<RegId> = None;
                for v in members {
                    let mut c = v.resize(wmax);
                    c.is_signed = false;
                    let cr = self.alloc_reg();
                    self.emit(Insn::LoadConst(cr, Box::new(c)));
                    let t = self.alloc_reg();
                    self.emit(Insn::Eq(t, er, cr));
                    acc = Some(match acc {
                        None => t,
                        Some(a) => {
                            let o = self.alloc_reg();
                            self.emit(Insn::BitOr(o, a, t));
                            o
                        }
                    });
                }
                acc
            }
            ExprKind::Call { func, args } => {
                // §18.8 (#846): mode builtins address a variable/constraint
                // through its receiver expression. CallMethod carries only
                // a handle value, losing `sub` versus `this.sub` and bypassing
                // the builtin dispatcher. Preserve that expression and the
                // register-backed locals needed to evaluate it.
                let mode_builtin = match &func.kind {
                    ExprKind::MemberAccess { member, .. } => {
                        matches!(member.name.as_str(), "rand_mode" | "constraint_mode")
                    }
                    ExprKind::Ident(h) if h.path.len() > 1 => h.path.last().is_some_and(|s| {
                        matches!(s.name.name.as_str(), "rand_mode" | "constraint_mode")
                    }),
                    _ => false,
                };
                if self.method_mode && mode_builtin {
                    if let Some(r) =
                        self.emit_expr_fallback(expr, ctx_width, "Expr_Call_random_mode")
                    {
                        return Some(r);
                    }
                    self.bail("Expr_Call_random_mode");
                    return None;
                }
                // A register holds one value, so `{"a", "b"}` cannot reach an
                // unpacked collection formal as separate elements.
                if self.method_mode && self.call_has_coll_formal_actual(func, args) {
                    self.bail("method_coll_literal_actual");
                    return None;
                }
                // class-perf method-mode: `this.m(a,..)` / `<classhandle>.m(a,..)`
                // where the receiver is a class handle lowers to a CallMethod
                // instruction. Runtime re-enters the interpreter's method
                // dispatcher (full virtual dispatch / defaults / refs / strings
                // in the callee), so the callee keeps all AST semantics. The
                // ALL-OR-NOTHING contract holds on the ARGS: any actual that
                // cannot become a register keeps the whole body on the AST
                // interpreter (rollback below).
                // Step 8 (string formals): `s.f()` on a string formal (len,
                // putc, itoa...) dispatches through the string_signals
                // registry at runtime — unlowerable here. Bail on a receiver
                // that mentions a string formal. (ARGS are pass-through and
                // stay allowed — `m.exists(key)` is the whole point.)
                if let ExprKind::MemberAccess { expr: base, .. } = &func.kind {
                    // A PURE string method (.len/.getc/.substr/...) lowers
                    // below this call path to a StrOp, so the string-formal
                    // receiver is NOT passed to the interpreter's dispatcher —
                    // don't bail for it here. (compile_string_method handles
                    // the bare MemberAccess form and the dotted-Ident form.)
                    if !self.string_method_shape(func, func.span).is_some()
                        && self.touches_string_formal(base)
                    {
                        self.bail("string_formal_method_receiver");
                        return None;
                    }
                } else if self.touches_string_formal(func) {
                    self.bail("string_formal_as_callee");
                    return None;
                }
                // class-perf Step 9f: `super.m(args)` / `super.new(args)` —
                // §8.15 static dispatch to the parent-of-the-defining-class
                // method. Admission mirrors the interpreter's `routed` gate:
                // the method must exist in the PARENT chain (or be `new`);
                // `super.m` where only the enclosing class itself defines
                // `m` does NOT route statically — it falls through to
                // virtual dispatch on `this`, which only the AST path
                // reproduces. The receiver is always `this` (a static frame
                // has none — method_this_reg is None there, so statics with
                // super calls decline here).
                if self.method_mode
                    && let ExprKind::MemberAccess { expr: base, member } = &func.kind
                    && matches!(&base.kind, ExprKind::Ident(h)
                        if h.path.len() == 1
                            && h.path[0].selects.is_empty()
                            && h.path[0].name.name == "super")
                    && self.super_method_names.contains(member.name.as_str())
                {
                    let Some(this_reg) = self.method_this_reg else {
                        self.bail("super_no_this");
                        return None;
                    };
                    // Baked start class: the parent of the DEFINING class
                    // (§8.15 — compile-time-fixed, identical to the
                    // interpreter's class-context resolution for a
                    // compiled frame).
                    let Some(parent) = self.method_class_parent.clone() else {
                        self.bail("super_no_parent");
                        return None;
                    };
                    // §13.5.1: the actual is context-determined by the
                    // parent's declared formal, as for the bare call below.
                    // `super.<m>` holds the parent chain's widths, which an
                    // `m` the defining class hides does not shadow (see
                    // `class_method_formal_widths`).
                    let callee_widths: Option<Vec<u32>> = self
                        .method_formal_widths
                        .get(format!("super.{}", member.name).as_str())
                        .cloned();
                    let call_start = self.insns.len();
                    let call_next = self.next_reg;
                    let mut ok = true;
                    let mut arg_values: Vec<RegId> = Vec::with_capacity(args.len());
                    for (ai, a) in args.iter().enumerate() {
                        let ctx = callee_widths
                            .as_ref()
                            .and_then(|w| w.get(ai).copied())
                            .unwrap_or(0);
                        match self.compile_expr(a, ctx) {
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
                        // Contiguous arg slots (same reservation discipline
                        // as the receiver form above).
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
                        self.emit(Insn::CallScopedMethod(
                            dest,
                            this_reg,
                            Box::new(NamePair(
                                parent.clone().into_boxed_str(),
                                member.name.clone().into_boxed_str(),
                            )),
                            arg_start,
                            n,
                        ));
                        self.emit_arg_slot_writeback(
                            args,
                            arg_start,
                            Some((parent.as_str(), member.name.as_str())),
                        )?;
                        return Some(dest);
                    }
                    self.truncate_insns(call_start);
                    self.next_reg = call_next;
                    self.bail("Expr_Call_super");
                    return None;
                }
                // class-perf Step 9b: a builtin-collection call on a member
                // collection — `m_children.exists(n)`, `q.size()`,
                // `m_total_count[obj] += count`'s read — lowers to a
                // CallCollMethod (see compile_coll_method_call). Sits BEFORE
                // the CallMethod branch: collection members are nonloadable,
                // so `method_handle_chain_ok(base)` is false for them and
                // that branch would bail. Not admitted (method/arity/receiver
                // gate) => falls through to the pre-existing paths unchanged.
                if self.method_mode
                    && let ExprKind::MemberAccess { expr: base, member } = &func.kind
                    && let Some(r) = self.compile_coll_method_call(base, member.name.as_str(), args)
                {
                    return Some(r);
                }
                if self.method_mode
                    && let ExprKind::MemberAccess { expr: base, member } = &func.kind
                    // TYPED chain (Step 9e): a foreign-class collection
                    // member must never become a dispatch "handle".
                    && self.method_handle_chain_class(base).is_some()
                {
                    // Step 6: the receiver may be a chain (`sched.m_parent.get_
                    // phase_type()`) or a bare member (`m.f()` dispatches on
                    // `this.m`, loaded here) — lower the base to its handle
                    // register.
                    let handle_reg = match self.method_member_handle_reg(base) {
                        Some(r) => r,
                        None => self.compile_expr(base, 0)?,
                    };
                    let call_start = self.insns.len();
                    let call_next = self.next_reg;
                    let mut ok = true;
                    let mut arg_values: Vec<RegId> = Vec::with_capacity(args.len());
                    for a in args {
                        match self.compile_expr(a, 0) {
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
                        // Reserve ALL n contiguous arg slots. The old code
                        // allocated only the FIRST (`arg_start`) while the
                        // moves and the runtime read (`base + i`) use slots
                        // arg_start..arg_start+n-1 — registers next_reg never
                        // counted. Latent until a block's register file was
                        // sized exactly (vm_regs.resize(block.num_regs)), then
                        // any 2+-arg call indexed past the end (found on a
                        // 3-argument class-method wrapper).
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
                        // §8.20: a non-virtual method binds to the
                        // receiver's DECLARED class; CallMethod dispatches on
                        // the object's dynamic class, which differs only when
                        // a subclass redeclares the method. Those calls start
                        // the hierarchy walk at the statically bound class.
                        let rc_decl = self.method_handle_chain_class(base);
                        // §8.23: a STATIC method never dereferences the
                        // receiver, so `h.m(...)` with `h == null` is legal and
                        // resolves on the DECLARED class. CallMethod has only
                        // the handle, so it dispatched on a live object's
                        // class; with a null handle the runtime returned a
                        // silent zero. Bake the declared class the same way a
                        // class-scope static call does.
                        let static_decl = rc_decl
                            .as_deref()
                            .and_then(|rc| self.declared_static_target(rc, member.name.as_str()));
                        let static_target = rc_decl.as_deref().and_then(|rc| {
                            self.method_admission.as_ref().and_then(|a| {
                                a.nonvirtual_targets
                                    .get(rc.split('#').next().unwrap_or(rc))
                                    .and_then(|m| m.get(member.name.as_str()))
                                    .cloned()
                            })
                        });
                        if let Some(target) = static_decl {
                            // The receiver register rides along so a LIVE
                            // handle still dispatches through the object (its
                            // runtime class carries the specialization); a
                            // NULL one falls back to the baked declared class.
                            self.emit(Insn::CallScopedMethod(
                                dest,
                                handle_reg,
                                Box::new(NamePair(
                                    target.clone().into_boxed_str(),
                                    member.name.clone().into_boxed_str(),
                                )),
                                arg_start,
                                n,
                            ));
                            let wb_target = rc_decl.as_deref().unwrap_or(target.as_str());
                            self.emit_arg_slot_writeback(
                                args,
                                arg_start,
                                Some((wb_target, member.name.as_str())),
                            )?;
                        } else if let Some(target) = static_target {
                            self.emit(Insn::CallScopedMethod(
                                dest,
                                handle_reg,
                                Box::new(NamePair(
                                    target.clone().into_boxed_str(),
                                    member.name.clone().into_boxed_str(),
                                )),
                                arg_start,
                                n,
                            ));
                            self.emit_arg_slot_writeback(
                                args,
                                arg_start,
                                Some((target.as_str(), member.name.as_str())),
                            )?;
                        } else {
                            self.emit(Insn::CallMethod(
                                dest,
                                handle_reg,
                                Box::new(Name(member.name.clone().into_boxed_str())),
                                arg_start,
                                n,
                            ));
                            self.emit_arg_slot_writeback(
                                args,
                                arg_start,
                                rc_decl.as_deref().map(|rc| (rc, member.name.as_str())),
                            )?;
                        }
                        return Some(dest);
                    }
                    self.truncate_insns(call_start);
                    self.next_reg = call_next;
                    self.bail("Expr_Call_class_method");
                    return None;
                }
                // Step 9e: a BARE this-bounded call — `helper(2)`,
                // `early(cnt)` — where the callee name is a method of the
                // enclosing class chain (class scope shadows module
                // functions per §8.23). Identical to the receiver form but
                // with `this` as the receiver register.
                // Step 9g: `new(args)` OUTSIDE an assignment target's
                // typed lowering — the constructed class is the LHS's
                // DECLARED type (§8.8), unknowable here. Keep the whole
                // method on the AST (the assign site below intercepts the
                // `lhs = new(...)` shape with a known lhs type).
                if self.method_mode
                    && let ExprKind::Ident(h) = &func.kind
                    && h.root.is_none()
                    && h.path.len() == 1
                    && h.path[0].selects.is_empty()
                    && h.path[0].name.name == "new"
                {
                    self.bail("Expr_Call_new");
                    return None;
                }
                if self.method_mode
                    && let ExprKind::Ident(h) = &func.kind
                    && h.path.len() == 1
                    && h.path[0].selects.is_empty()
                    && self
                        .class_method_names
                        .contains(h.path[0].name.name.as_str())
                {
                    let this_reg = if let Some(tr) = self.method_this_reg {
                        tr
                    } else if let Some(target) = self.static_bare_targets.get(h.path[0].name.name.as_str()).cloned() {
                        // Static method in a static method body (where this_reg is None):
                        // emit CallStaticScoped with this class target.
                        let call_start = self.insns.len();
                        let call_next = self.next_reg;
                        let mut ok = true;
                        let mut arg_values: Vec<RegId> = Vec::with_capacity(args.len());
                        for a in args {
                            match self.compile_expr(a, 0) {
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
                            self.emit(Insn::CallStaticScoped(
                                dest,
                                Box::new(NamePair(
                                    target.into_boxed_str(),
                                    h.path[0].name.name.clone().into_boxed_str(),
                                )),
                                arg_start,
                                n,
                            ));
                            let wb_target = self.method_class.clone();
                            self.emit_arg_slot_writeback(
                                args,
                                arg_start,
                                wb_target.as_deref().map(|c| (c, h.path[0].name.name.as_str())),
                            )?;
                            return Some(dest);
                        }
                        self.truncate_insns(call_start);
                        self.next_reg = call_next;
                        self.bail("Expr_Call_bare_static");
                        return None;
                    } else {
                        self.bail("bare_call_no_this");
                        return None;
                    };
                    // LRM 11.6/11.8.2: a call ACTUAL is context-determined
                    // by the callee's DECLARED formal, exactly like the
                    // operand of a comparison.  `wide9(a + b)` against
                    // `function bit wide9(logic [8:0] v)` must evaluate
                    // `a + b` at 9 bits (300); compiling it self-determined
                    // yields 8-bit 44 and the callee compares 44 !== 300.
                    // Mirrors the inline path, which binds with
                    // `port_effective_width`.  Unknown callee / non-scalar
                    // port = width 0 = the previous self-determined behavior.
                    let callee_widths: Option<Vec<u32>> = self
                        .method_formal_widths
                        .get(h.path[0].name.name.as_str())
                        .cloned();
                    let call_start = self.insns.len();
                    let call_next = self.next_reg;
                    let mut ok = true;
                    let mut arg_values: Vec<RegId> = Vec::with_capacity(args.len());
                    for (ai, a) in args.iter().enumerate() {
                        let ctx = callee_widths
                            .as_ref()
                            .and_then(|w| w.get(ai).copied())
                            .unwrap_or(0);
                        match self.compile_expr(a, ctx) {
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
                        // Contiguous arg slots (same reservation discipline
                        // as the receiver form above).
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
                        // §8.20: a NON-VIRTUAL method binds statically to
                        // the first defining class of the lexical chain —
                        // the derived override must not run (the AST
                        // fallback's `nonvirtual_target_class`). Virtual
                        // (and static) methods keep the virtual
                        // CallMethod dispatch.
                        if let Some(target) = self
                            .static_bare_targets
                            .get(h.path[0].name.name.as_str())
                            .cloned()
                        {
                            self.emit(Insn::CallScopedMethod(
                                dest,
                                this_reg,
                                Box::new(NamePair(
                                    target.clone().into_boxed_str(),
                                    h.path[0].name.name.clone().into_boxed_str(),
                                )),
                                arg_start,
                                n,
                            ));
                            self.emit_arg_slot_writeback(
                                args,
                                arg_start,
                                Some((target.as_str(), h.path[0].name.name.as_str())),
                            )?;
                            return Some(dest);
                        }
                        let this_cls = self.method_class.clone();
                        self.emit(Insn::CallMethod(
                            dest,
                            this_reg,
                            Box::new(Name(h.path[0].name.name.clone().into_boxed_str())),
                            arg_start,
                            n,
                        ));
                        self.emit_arg_slot_writeback(
                            args,
                            arg_start,
                            this_cls
                                .as_deref()
                                .map(|c| (c, h.path[0].name.name.as_str())),
                        )?;
                        return Some(dest);
                    }
                    self.truncate_insns(call_start);
                    self.next_reg = call_next;
                    self.bail("Expr_Call_bare_method");
                    return None;
                }
                // class-perf P2: a FREE-FUNCTION call — bare callee in the
                // function table that is NOT a class method (§8.23: class
                // scope shadows, and that branch above already declined),
                // not shadowed by a local/member read, with positional
                // args and `input`-only formals. The value round-trip
                // cannot write back, so any potentially-assignable formal
                // direction declines (same discipline as CallStaticScoped).
                if self.method_mode
                    && let ExprKind::Ident(h) = &func.kind
                    && h.root.is_none()
                    && h.path.len() == 1
                    && h.path[0].selects.is_empty()
                    && let Some(name) = self.scoped_callee_name(h.path[0].name.name.clone())
                    && let Some(fd) = self.functions.and_then(|f| f.get(name.as_str()))
                    && fd.ports.len() == args.len()
                    && fd
                        .ports
                        .iter()
                        .all(|p| matches!(p.direction, crate::ast::types::PortDirection::Input))
                    && !args
                        .iter()
                        .any(|a| matches!(a.kind, ExprKind::NamedArg { .. }))
                {
                    let call_start = self.insns.len();
                    let call_next = self.next_reg;
                    let mut ok = true;
                    let mut arg_values: Vec<RegId> = Vec::with_capacity(args.len());
                    // §13.5.1: a scalar input formal is the actual's context
                    // width (the inline path binds the same way); unpacked
                    // formals keep the self-determined compile.
                    let fn_scope = name.rsplit_once('.').map(|(s, _)| s.to_string());
                    for (ai, a) in args.iter().enumerate() {
                        let ctx = if fd.ports[ai].dimensions.is_empty() {
                            self.port_effective_width(&fd.ports, ai, fn_scope.as_deref())
                        } else {
                            0
                        };
                        match self.compile_expr(a, ctx) {
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
                        // Contiguous arg slots (same reservation discipline
                        // as the bare-call form above).
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
                        return Some(dest);
                    }
                    self.truncate_insns(call_start);
                    self.next_reg = call_next;
                }
                // class-perf Step 9h: a CLASS-SCOPE static call —
                // `Cls.m(args)` / `alias.m(args)` / `Tparam.m(args)` where
                // the receiver Ident names a class-scope entity admitted by
                // the compile site (class name, chain typedef alias, or type
                // parameter) and is NOT any variable-like name. The receiver
                // name is baked RAW; resolution (type-param binding, embedded
                // `base#sig`, plain class, typedef spec, simple typedef) is
                // executed by the interpreter at runtime via the shared
                // `exec_class_scope_static_call` — identical to the AST path,
                // including per-specialization static seeding.
                // §13.5.2: the shared resolution binds actuals the same way
                // the AST member-call path does — a ref/output formal writes
                // back through its ACTUAL EXPRESSION. The compiled call
                // marshals register VALUES as constant actuals, so a
                // write-back would be a no-op and the caller's lvalue would
                // silently keep its pre-call value (uvm_config_db::get's
                // inout `value` arg zeroed every config read).
                // Register-backed locals are writable via emit_arg_slot_writeback,
                // but other assignable actuals (heap members, static members,
                // arrays) decline the compiled static method call.
                let arg_could_be_ref_bound = |a: &Expression| -> bool {
                    match &a.kind {
                        ExprKind::Ident(h)
                            if h.root.is_none()
                                && h.path.len() == 1
                                && h.path[0].selects.is_empty() =>
                        {
                            let n = h.path[0].name.name.as_str();
                            if n == "this" {
                                return false;
                            }
                            if self.local_var_reg_of(h).is_some() {
                                return false;
                            }
                            self.local_array_regs.contains_key(n)
                                || self.bare_member_names.contains(n)
                                || self.static_member_names.contains(n)
                                || self.string_member_names.contains(n)
                                || self.class_shadow_names.contains(n)
                        }
                        // A member train or element select is assignable in
                        // principle (this.m, arr[i]); a nested call result,
                        // literal, or operator expression is not.
                        ExprKind::MemberAccess { .. } | ExprKind::Index { .. } => true,
                        _ => false,
                    }
                };
                if self.method_mode
                    && let ExprKind::MemberAccess { expr: rx, member } = &func.kind
                    && let ExprKind::Ident(h) = &rx.kind
                    && h.root.is_none()
                    && h.path.len() == 1
                    && h.path[0].selects.is_empty()
                    && !self
                        .local_var_regs
                        .contains_key(h.path[0].name.name.as_str())
                    && !self
                        .local_array_regs
                        .contains_key(h.path[0].name.name.as_str())
                    && !self
                        .bare_member_names
                        .contains(h.path[0].name.name.as_str())
                    // A free-function body has no `this` and no defining
                    // class context, so a class-scope static call re-enters
                    // the dispatcher with whatever frame ran last as the
                    // caller's context. A CONCRETE class receiver is safe:
                    // `CallStaticScoped` bakes the class name and
                    // `exec_class_scope_static_call` resolves it through
                    // `module.classes` without the stale `current_spec`. A
                    // type parameter or typedef alias resolves against that
                    // stale spec (the `uvm_report_server::get_server()`
                    // string corruption), so for a free function the
                    // simulator admits only elaborated class names here.
                    && self
                        .scope_static_receivers
                        .contains(h.path[0].name.name.as_str())
                    && !args.iter().any(arg_could_be_ref_bound)
                {
                    let call_start = self.insns.len();
                    let call_next = self.next_reg;
                    let mut ok = true;
                    let mut arg_values: Vec<RegId> = Vec::with_capacity(args.len());
                    for a in args {
                        match self.compile_expr(a, 0) {
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
                        // Contiguous arg slots (same reservation discipline
                        // as the bare-call form above).
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
                        self.emit(Insn::CallStaticScoped(
                            dest,
                            Box::new(NamePair(
                                h.path[0].name.name.clone().into_boxed_str(),
                                member.name.clone().into_boxed_str(),
                            )),
                            arg_start,
                            n,
                        ));
                        self.emit_arg_slot_writeback(
                            args,
                            arg_start,
                            Some((h.path[0].name.name.as_str(), member.name.as_str())),
                        )?;
                        return Some(dest);
                    }
                    self.truncate_insns(call_start);
                    self.next_reg = call_next;
                    self.bail("Expr_Call_static_scope");
                    return None;
                }
                if let Some(r) = self.compile_string_method(func, args, expr.span) {
                    return Some(r);
                }
                // Pure-call inlining is speculative. A body can compile a
                // prefix (including placeholder branches) before an
                // unsupported local lvalue makes it fall back. Do not leave
                // that incomplete prefix in the caller's instruction stream.
                let start = self.insns.len();
                let first_reg = self.next_reg;
                if let Some(r) = self.compile_pure_call(func, args) {
                    return Some(r);
                }
                self.truncate_insns(start);
                self.next_reg = first_reg;
                self.emit_expr_fallback(expr, ctx_width, "Expr_Call_impure")
            }
            // `$` outside a select has no queue bound here (§7.10).
            ExprKind::Dollar => {
                self.bail("dollar_outside_select");
                None
            }
            other => {
                let n: &'static str = match other {
                    ExprKind::StringLiteral(_) => "Expr_StringLiteral",
                    ExprKind::Replication { .. } => "Expr_Replication",
                    ExprKind::AssignmentPattern(_) => "Expr_AssignmentPattern",
                    ExprKind::TypedPattern { .. } => "Expr_TypedPattern",
                    ExprKind::Call { .. } => "Expr_Call",
                    ExprKind::Inside { .. } => "Expr_Inside",
                    ExprKind::MemberAccess { expr, member } => {
                        let _ = expr;
                        let _ = member;
                        "Expr_MemberAccess"
                    }
                    ExprKind::Range(..) => "Expr_Range",
                    ExprKind::NamedArg { .. } => "Expr_NamedArg",
                    _ => "Expr_other",
                };
                // Assignment patterns (and named args inside them) spread
                // member-wise at the STATEMENT level on the AST path;
                // evaluating one here to a packed value changes NBA
                // semantics on unpacked structs. Let the statement bail.
                let pattern_like = matches!(
                    other,
                    ExprKind::AssignmentPattern(_) | ExprKind::NamedArg { .. }
                );
                if !pattern_like {
                    if let Some(r) = self.emit_expr_fallback(expr, ctx_width, n) {
                        return Some(r);
                    }
                }
                self.bail(n);
                None
            }
        }
    }
}
