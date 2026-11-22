//! Real operands and real contexts, with the constant, effect and short-circuit checks they use.

use super::*;

impl<'a> BytecodeCompiler<'a> {
    /// §13.3.1 numeric conversion to real, in place. Emitted as
    /// `reg = reg * 1.0(real)`: `Value::mul` with a real operand converts the
    /// other side via `to_f64`, which is EXACTLY `Value::from_f64(v.to_f64())`
    /// — so no new opcode (and the JIT/AOT handle it for free). A no-op for a
    /// value that is already real.
    pub(super) fn emit_to_real(&mut self, reg: RegId) {
        let one = self.alloc_reg();
        self.emit(Insn::LoadConst(one, Box::new(Value::from_f64(1.0))));
        self.emit(Insn::Mul(reg, reg, one));
    }

    /// §11.8.2: true when `e` is REAL-typed by declaration — a real literal, a
    /// real variable, a real-valued system function, a cast to a real type,
    /// or an arithmetic/conditional operator over such an operand. Exact,
    /// unlike `expr_may_be_real`: it decides how the OTHER operand of a real
    /// operator is evaluated, so a shape it cannot type answers false and
    /// keeps the integral rules.
    pub(super) fn expr_is_real_static(&self, e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Number(n) => matches!(n, NumberLiteral::Real(_) | NumberLiteral::Time(_)),
            ExprKind::Ident(h) => {
                self.local_var_is_real.contains(&Self::hier_raw_name(h))
                    || self.lookup_signal_id(h).is_some_and(|id| {
                        self.signal_real
                            .is_some_and(|r| r.get(id).copied().unwrap_or(false))
                    })
            }
            // §11.3.1: an element of a register-bound local `real` array (an
            // inlined function's `input real d[]`) is real too. Without this
            // `d[k] / x` divided as integers and read 0.
            ExprKind::Index { expr: a, .. } => matches!(
                &a.kind,
                ExprKind::Ident(h) if self
                    .local_var_array
                    .get(&Self::hier_raw_name(h))
                    .is_some_and(|ab| ab.is_real)
            ),
            ExprKind::Paren(x) => self.expr_is_real_static(x),
            ExprKind::Unary {
                op: UnaryOp::Plus | UnaryOp::Minus,
                operand,
            } => self.expr_is_real_static(operand),
            ExprKind::Binary {
                op: BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Power,
                left,
                right,
            } => self.expr_is_real_static(left) || self.expr_is_real_static(right),
            ExprKind::Conditional {
                then_expr,
                else_expr,
                ..
            } => self.expr_is_real_static(then_expr) || self.expr_is_real_static(else_expr),
            ExprKind::SystemCall { name, args } => match name.as_str() {
                "$__xz_type_cast" => matches!(
                    args.first().map(|a| &a.kind),
                    Some(ExprKind::TypeLiteral(dt)) if self.decl_is_real(dt, None)
                ),
                "$__xz_named_cast" => self.named_cast_is_real(args),
                n => system_function_is_real(n),
            },
            ExprKind::Call { func, .. } => self.call_returns_real(func),
            _ => false,
        }
    }

    /// §13.4.1: `func` names a function (or DPI import) declared to return
    /// a real type, so a call of it is real-typed: `fr(x) / 2` divides as
    /// reals instead of truncating the result to an integer first.
    fn call_returns_real(&self, func: &Expression) -> bool {
        let ExprKind::Ident(h) = &func.kind else {
            return false;
        };
        if h.path.len() != 1 || !h.path[0].selects.is_empty() {
            return false;
        }
        let Some(name) = self.scoped_callee_name(h.path[0].name.name.clone()) else {
            return false;
        };
        let real = |fd: &FunctionDeclaration| self.decl_is_real(&fd.return_type, Some(&name));
        self.functions.and_then(|f| f.get(&name)).is_some_and(real)
            || self
                .dpi_import_fds
                .as_deref()
                .and_then(|m| m.get(&name))
                .is_some_and(real)
    }

    /// §10.7/§11.8.2: the context width an assignment gives its right-hand
    /// side. A REAL target has no bit width to lend: an integral RHS is
    /// evaluated self-determined and then converted (`r = u - 2` with
    /// `int unsigned u = 1` is 4294967295.0, `r = x + 1` wraps at 32 bits).
    pub(super) fn assign_rhs_ctx(&self, lvalue: &Expression, width: u32) -> u32 {
        if matches!(lvalue.kind, ExprKind::Ident(_)) && self.expr_is_real_static(lvalue) {
            0
        } else {
            width
        }
    }

    /// A constant expression by its shape — literals, parameters, and
    /// operators, casts and real conversions over them.
    fn expr_is_constant(&self, e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Number(_) => true,
            ExprKind::Ident(h) => self.lookup_param_value(h).is_some(),
            ExprKind::Paren(x) | ExprKind::Unary { operand: x, .. } => self.expr_is_constant(x),
            ExprKind::Binary { left, right, .. } => {
                self.expr_is_constant(left) && self.expr_is_constant(right)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.expr_is_constant(condition)
                    && self.expr_is_constant(then_expr)
                    && self.expr_is_constant(else_expr)
            }
            ExprKind::SystemCall { name, args } => match name.as_str() {
                "$__xz_type_cast" => args.get(1).is_some_and(|a| self.expr_is_constant(a)),
                // A time read changes with the simulation time.
                n if n != "$realtime" && n != "$__xz_realtime_at" && system_function_is_real(n) => {
                    args.iter().all(|a| self.expr_is_constant(a))
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// §11.8.2: integral operands of real arithmetic and comparisons are
    /// self-determined before conversion, including constant expressions.
    pub(super) fn compile_real_operator(
        &mut self,
        op: BinaryOp,
        left: &Expression,
        right: &Expression,
    ) -> Option<Option<RegId>> {
        let arith = matches!(
            op,
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Power
        );
        let cmp = matches!(
            op,
            BinaryOp::Eq
                | BinaryOp::Neq
                | BinaryOp::Lt
                | BinaryOp::Leq
                | BinaryOp::Gt
                | BinaryOp::Geq
        );
        if !arith && !cmp {
            return None;
        }
        let lr = self.expr_is_real_static(left);
        let rr = self.expr_is_real_static(right);
        if !lr && !rr {
            return None;
        }
        let Some(l) = self.compile_real_operand(left, lr) else {
            return Some(None);
        };
        let Some(r) = self.compile_real_operand(right, rr) else {
            return Some(None);
        };
        Some(Some(self.emit_real_binop(op, l, r)))
    }

    /// An arm of a `?:` whose result is real: a real arm as itself, a
    /// constant integral arm in real context, any other integral arm
    /// converted from its self-determined value (as the reference simulator
    /// does at run time).
    pub(super) fn compile_real_arm(&mut self, e: &Expression) -> Option<RegId> {
        if self.expr_is_real_static(e) {
            self.compile_expr(e, 0)
        } else if self.expr_is_constant(e) {
            self.compile_in_real_context(e)
        } else {
            self.compile_real_operand(e, false)
        }
    }

    /// One real-operator instruction over two real registers.
    fn emit_real_binop(&mut self, op: BinaryOp, l: RegId, r: RegId) -> RegId {
        let dest = self.alloc_reg();
        self.emit(match op {
            BinaryOp::Add => Insn::Add(dest, l, r),
            BinaryOp::Sub => Insn::Sub(dest, l, r),
            BinaryOp::Mul => Insn::Mul(dest, l, r),
            BinaryOp::Div => Insn::Div(dest, l, r),
            BinaryOp::Power => Insn::Pow(dest, l, r),
            BinaryOp::Eq => Insn::Eq(dest, l, r),
            BinaryOp::Neq => Insn::Neq(dest, l, r),
            BinaryOp::Lt => Insn::Lt(dest, l, r),
            BinaryOp::Leq => Insn::Leq(dest, l, r),
            BinaryOp::Gt => Insn::Gt(dest, l, r),
            _ => Insn::Geq(dest, l, r),
        });
        dest
    }

    /// An operand of a real operator: a real operand as itself, an integral
    /// one SELF-determined (§11.8.2) and converted to real (§6.12.2: x and z
    /// bits read as 0). Converted in a fresh register — the operand's own
    /// may be a local variable's.
    fn compile_real_operand(&mut self, e: &Expression, is_real: bool) -> Option<RegId> {
        let v = self.compile_expr(e, 0)?;
        if is_real {
            return Some(v);
        }
        let t = self.alloc_reg();
        self.emit(Insn::Move(t, v));
        self.emit_to_real(t);
        Some(t)
    }

    /// §11.8.2: an integral operand that takes REAL type from its context (the
    /// other side of a relational operator, the other arm of `?:`). The real
    /// type reaches the operator at its top — an arithmetic one computes in
    /// real — whose own integral operands are then self-determined and
    /// converted. Anything else is self-determined and converted whole.
    fn compile_in_real_context(&mut self, e: &Expression) -> Option<RegId> {
        match &e.kind {
            ExprKind::Paren(x) => self.compile_in_real_context(x),
            ExprKind::Binary {
                op:
                    op @ (BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Power),
                left,
                right,
            } => {
                let lr = self.expr_is_real_static(left);
                let l = self.compile_real_operand(left, lr)?;
                let rr = self.expr_is_real_static(right);
                let r = self.compile_real_operand(right, rr)?;
                Some(self.emit_real_binop(*op, l, r))
            }
            ExprKind::Unary {
                op: op @ (UnaryOp::Plus | UnaryOp::Minus),
                operand,
            } => {
                let real = self.expr_is_real_static(operand);
                let o = self.compile_real_operand(operand, real)?;
                if matches!(op, UnaryOp::Plus) {
                    return Some(o);
                }
                let dest = self.alloc_reg();
                self.emit(Insn::Negate(dest, o));
                Some(dest)
            }
            _ => {
                let real = self.expr_is_real_static(e);
                self.compile_real_operand(e, real)
            }
        }
    }

    /// May evaluating `e` have an effect beyond its value — a subroutine or
    /// method call, an increment, an assignment, a randomizing system call?
    /// How deeply `?:` nests in `e`, through the operators around it.
    pub(super) fn ternary_depth(e: &Expression) -> usize {
        match &e.kind {
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                1 + Self::ternary_depth(condition)
                    .max(Self::ternary_depth(then_expr))
                    .max(Self::ternary_depth(else_expr))
            }
            ExprKind::Paren(x) | ExprKind::Unary { operand: x, .. } => Self::ternary_depth(x),
            ExprKind::Binary { left, right, .. } => {
                Self::ternary_depth(left).max(Self::ternary_depth(right))
            }
            _ => 0,
        }
    }

    pub(super) fn expr_has_effect(e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Call { .. } | ExprKind::AssignExpr { .. } => true,
            ExprKind::Unary { op, operand } => {
                matches!(
                    op,
                    UnaryOp::PreIncr | UnaryOp::PreDecr | UnaryOp::PostIncr | UnaryOp::PostDecr
                ) || Self::expr_has_effect(operand)
            }
            // Seeded generators advance their seed; the others write an
            // argument or move a file position.
            ExprKind::SystemCall { name, args } => {
                matches!(
                    name.as_str(),
                    "$urandom"
                        | "$random"
                        | "$urandom_range"
                        | "$cast"
                        | "$sscanf"
                        | "$fscanf"
                        | "$fgets"
                        | "$fgetc"
                        | "$ungetc"
                        | "$fread"
                        | "$value$plusargs"
                        | "$fopen"
                        | "$system"
                        | "$dist_uniform"
                        | "$dist_normal"
                        | "$dist_exponential"
                        | "$dist_poisson"
                        | "$dist_chi_square"
                        | "$dist_t"
                        | "$dist_erlang"
                ) || args.iter().any(Self::expr_has_effect)
            }
            ExprKind::Paren(x) | ExprKind::MemberAccess { expr: x, .. } => Self::expr_has_effect(x),
            ExprKind::Binary { left, right, .. } => {
                Self::expr_has_effect(left) || Self::expr_has_effect(right)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::expr_has_effect(condition)
                    || Self::expr_has_effect(then_expr)
                    || Self::expr_has_effect(else_expr)
            }
            ExprKind::Index { expr, index } => {
                Self::expr_has_effect(expr) || Self::expr_has_effect(index)
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                Self::expr_has_effect(expr)
                    || Self::expr_has_effect(left)
                    || Self::expr_has_effect(right)
            }
            ExprKind::Concatenation(parts) => parts.iter().any(Self::expr_has_effect),
            ExprKind::Replication { count, exprs } => {
                Self::expr_has_effect(count) || exprs.iter().any(Self::expr_has_effect)
            }
            ExprKind::Inside { expr, ranges } => {
                Self::expr_has_effect(expr) || ranges.iter().any(Self::expr_has_effect)
            }
            _ => false,
        }
    }

    /// §11.4.7: `left && right` / `left || right` with the right operand
    /// evaluated only when the left does not decide the result: a left that
    /// is exactly 0 (`&&`) or has a 1 bit (`||`) skips it. Otherwise the
    /// result is computed exactly as the unconditional lowering does, X
    /// operands included.
    pub(super) fn compile_short_circuit(
        &mut self,
        op: BinaryOp,
        left: &Expression,
        right: &Expression,
    ) -> Option<RegId> {
        let l = self.compile_expr(left, 0)?;
        let dest = self.alloc_reg();
        let (decides, init) = match op {
            BinaryOp::LogAnd => (0, 0),
            _ => (1, 1),
        };
        self.emit(Insn::LoadConst(dest, Box::new(Value::from_u64(init, 1))));
        let red = self.alloc_reg();
        self.emit(Insn::ReduceOr(red, l));
        let k = self.alloc_reg();
        self.emit(Insn::LoadConst(k, Box::new(Value::from_u64(decides, 1))));
        let decided = self.alloc_reg();
        self.emit(Insn::CaseEq(decided, red, k));
        let undecided = self.alloc_reg();
        self.emit(Insn::LogNot(undecided, decided));
        let br = self.insns.len();
        self.emit(Insn::BranchIfFalse(undecided, 0));
        let r = self.compile_expr(right, 0)?;
        match op {
            BinaryOp::LogAnd => self.emit(Insn::LogAnd(dest, l, r)),
            _ => self.emit(Insn::LogOr(dest, l, r)),
        }
        let end = self.insns.len() as u32;
        if let Insn::BranchIfFalse(reg, _) = self.insns[br] {
            self.insns[br] = Insn::BranchIfFalse(reg, end);
        }
        Some(dest)
    }

    /// Conservative: true when `e` may evaluate to a real — a real literal,
    /// a real signal or local, or a real-valued system function.
    pub(super) fn expr_may_be_real(&self, e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Number(n) => matches!(n, NumberLiteral::Real(_) | NumberLiteral::Time(_)),
            ExprKind::Ident(h) => {
                self.local_var_is_real.contains(&Self::hier_raw_name(h))
                    || self.lookup_signal_id(h).is_some_and(|id| {
                        self.signal_real
                            .is_some_and(|r| r.get(id).copied().unwrap_or(false))
                    })
            }
            ExprKind::SystemCall { name, args } => {
                matches!(
                    name.as_str(),
                    "$realtime"
                        | "$__xz_realtime_at"
                        | "$itor"
                        | "$bitstoreal"
                        | "$shortrealtobits"
                        | "$sqrt"
                        | "$ln"
                        | "$log10"
                        | "$exp"
                        | "$pow"
                        | "$floor"
                        | "$ceil"
                ) || args.iter().any(|a| self.expr_may_be_real(a))
            }
            ExprKind::Paren(x) | ExprKind::Unary { operand: x, .. } => self.expr_may_be_real(x),
            ExprKind::Binary { left, right, .. } => {
                self.expr_may_be_real(left) || self.expr_may_be_real(right)
            }
            ExprKind::Conditional {
                then_expr,
                else_expr,
                ..
            } => self.expr_may_be_real(then_expr) || self.expr_may_be_real(else_expr),
            ExprKind::Call { func, .. } => self.call_returns_real(func),
            _ => false,
        }
    }

    /// Conservative the other way from `expr_may_be_real`: false only when
    /// `e` provably evaluates to a value that is not real (an integral,
    /// string or handle value), true for a real and for any shape not
    /// understood here. Decides whether a value fitted to a 64-bit integral
    /// type needs `RealToInt`, and (applied to an assignment target) whether
    /// that target is integral.
    pub(super) fn value_may_be_real(&self, e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Number(n) => matches!(n, NumberLiteral::Real(_) | NumberLiteral::Time(_)),
            ExprKind::StringLiteral(_) => false,
            ExprKind::Paren(x) => self.value_may_be_real(x),
            ExprKind::Ident(h) => self.ident_may_be_real(h),
            // §11.5.1: a part-select is integral (a real has none).
            ExprKind::RangeSelect { .. } => false,
            ExprKind::Index { expr: base, .. } => match &base.kind {
                ExprKind::Ident(h) if h.path.iter().all(|s| s.selects.is_empty()) => {
                    let raw = Self::hier_raw_name(h);
                    if let Some(ab) = self.local_var_array.get(&raw) {
                        return ab.is_real;
                    }
                    if let Some(name) = self.lookup_array_name(h) {
                        // §7.4: the element type of an unpacked array.
                        let lo = self.arrays.get(&name).map_or(0, |&(lo, _, _)| lo);
                        return self
                            .lookup_signal_id_by_name(&format!("{name}[{lo}]"))
                            .is_none_or(|id| self.signal_is_real(id));
                    }
                    if self.is_assoc_target(h)
                        || self.is_dynamic_collection(&raw)
                        || self.arrays.contains_key(&raw)
                        || self
                            .multi_dim_arrays
                            .is_some_and(|m| m.contains(raw.as_str()))
                    {
                        return true;
                    }
                    // A select of a packed vector or structure (§7.4.1).
                    !(self.local_var_reg_of(h).is_some_and(|(r, _)| {
                        matches!(self.local_kinds.get(&r), Some(LocalKind::Int { .. }))
                    }) || self.lookup_signal_id(h).is_some_and(|id| {
                        self.signal_real.is_some()
                            && !self.signal_is_real(id)
                            && self.signal_widths.get(id).is_some_and(|&w| w > 0)
                    }))
                }
                // An element of a two-dimensional unpacked array.
                ExprKind::Index { expr: root, .. } => match &root.kind {
                    ExprKind::Ident(h) if h.path.iter().all(|s| s.selects.is_empty()) => self
                        .arrays_2d_entry(h)
                        .is_none_or(|(key, ((a, b), (c, d), _))| {
                            self.lookup_signal_id_by_name(&format!(
                                "{key}[{}][{}]",
                                a.min(b),
                                c.min(d)
                            ))
                            .is_none_or(|id| self.signal_is_real(id))
                        }),
                    _ => true,
                },
                _ => true,
            },
            ExprKind::MemberAccess { expr: base, member } => match &base.kind {
                ExprKind::Ident(h) if h.path.iter().all(|s| s.selects.is_empty()) => {
                    let mut h = h.clone();
                    h.path.push(HierPathSegment {
                        name: member.clone(),
                        selects: Vec::new(),
                    });
                    self.ident_may_be_real(&h)
                }
                _ => true,
            },
            ExprKind::Unary { op, operand } => match op {
                UnaryOp::Plus
                | UnaryOp::Minus
                | UnaryOp::PreIncr
                | UnaryOp::PreDecr
                | UnaryOp::PostIncr
                | UnaryOp::PostDecr => self.value_may_be_real(operand),
                // Logical, bitwise and reduction operators are integral.
                UnaryOp::LogNot
                | UnaryOp::BitNot
                | UnaryOp::BitAnd
                | UnaryOp::BitNand
                | UnaryOp::BitOr
                | UnaryOp::BitNor
                | UnaryOp::BitXor
                | UnaryOp::BitXnor => false,
                _ => true,
            },
            ExprKind::Binary { op, left, right } => match op {
                // §11.3.1: arithmetic is real when an operand is.
                BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::Mod
                | BinaryOp::Power => self.value_may_be_real(left) || self.value_may_be_real(right),
                // Relational, equality, logical, bitwise and shift operators
                // give an integral result (§11.4.4-§11.4.10).
                BinaryOp::Eq
                | BinaryOp::Neq
                | BinaryOp::CaseEq
                | BinaryOp::CaseNeq
                | BinaryOp::WildcardEq
                | BinaryOp::WildcardNeq
                | BinaryOp::LogAnd
                | BinaryOp::LogOr
                | BinaryOp::LogImplies
                | BinaryOp::LogEquiv
                | BinaryOp::Lt
                | BinaryOp::Leq
                | BinaryOp::Gt
                | BinaryOp::Geq
                | BinaryOp::BitAnd
                | BinaryOp::BitOr
                | BinaryOp::BitXor
                | BinaryOp::BitXnor
                | BinaryOp::ShiftLeft
                | BinaryOp::ShiftRight
                | BinaryOp::ArithShiftLeft
                | BinaryOp::ArithShiftRight => false,
                _ => true,
            },
            ExprKind::Conditional {
                then_expr,
                else_expr,
                ..
            } => self.value_may_be_real(then_expr) || self.value_may_be_real(else_expr),
            // §11.4.12, §11.4.13: concatenations and `inside` are integral.
            ExprKind::Concatenation(_)
            | ExprKind::Replication { .. }
            | ExprKind::Inside { .. }
            | ExprKind::StreamOp { .. } => false,
            ExprKind::SystemCall { name, args } => match name.as_str() {
                "$__xz_type_cast" => match args.first().map(|a| &a.kind) {
                    Some(ExprKind::TypeLiteral(dt)) => self.decl_is_real(dt, None),
                    _ => true,
                },
                "$__xz_named_cast" => {
                    self.named_cast_is_real(args)
                        || !(self.cast_arg_targets_string(args)
                            || matches!(args.first().map(|a| &a.kind), Some(ExprKind::Number(_)))
                            || args.first().is_some_and(|a| {
                                let leaf = match &a.kind {
                                    ExprKind::Ident(h) => h.path.last().map(|s| &s.name.name),
                                    ExprKind::MemberAccess { member, .. } => Some(&member.name),
                                    _ => None,
                                };
                                leaf.is_some_and(|n| {
                                    self.cast_widths.is_some_and(|m| m.contains_key(n))
                                        || self.params.is_some_and(|p| p.contains_key(n))
                                })
                            }))
                }
                "$__xz_size_cast" => false,
                "$signed" | "$unsigned" | "$sampled" | "$past" => {
                    args.first().is_none_or(|a| self.value_may_be_real(a))
                }
                "$time"
                | "$stime"
                | "$random"
                | "$urandom"
                | "$urandom_range"
                | "$clog2"
                | "$bits"
                | "$countones"
                | "$countbits"
                | "$onehot"
                | "$onehot0"
                | "$isunknown"
                | "$size"
                | "$high"
                | "$low"
                | "$left"
                | "$right"
                | "$increment"
                | "$dimensions"
                | "$unpacked_dimensions"
                | "$rtoi"
                | "$realtobits"
                | "$shortrealtobits"
                | "$rose"
                | "$fell"
                | "$stable"
                | "$changed"
                | "$test$plusargs"
                | "$fopen"
                | "$feof"
                | "$fgetc" => false,
                _ => true,
            },
            ExprKind::Call { func, .. } => {
                let ExprKind::Ident(h) = &func.kind else {
                    return true;
                };
                if h.path.len() != 1 || !h.path[0].selects.is_empty() {
                    return true;
                }
                let Some(name) = self.scoped_callee_name(h.path[0].name.name.clone()) else {
                    return true;
                };
                // §13.4.1: a call has its declared return type.
                let fd = self
                    .functions
                    .and_then(|f| f.get(&name))
                    .or_else(|| self.dpi_import_fds.as_deref().and_then(|m| m.get(&name)));
                fd.is_none_or(|fd| self.decl_is_real(&fd.return_type, Some(&name)))
            }
            _ => true,
        }
    }

    /// `value_may_be_real` for a name.
    fn ident_may_be_real(&self, h: &HierarchicalIdentifier) -> bool {
        let simple = h.path.iter().all(|s| s.selects.is_empty());
        if h.root.is_none() && h.path.len() == 1 && simple {
            if let Some(v) = self.local_const_vars.get(&h.path[0].name.name) {
                return v.is_real;
            }
        }
        let raw = Self::hier_raw_name(h);
        if self.local_var_is_real.contains(&raw) {
            return true;
        }
        if let Some((r, _)) = self.local_var_reg_of(h) {
            return !matches!(
                self.local_kinds.get(&r),
                Some(LocalKind::Int { .. } | LocalKind::Str | LocalKind::Handle)
            );
        }
        if !simple {
            return true;
        }
        if let Some(v) = self.lookup_param_value(h) {
            return v.is_real;
        }
        if self.signal_real.is_none() {
            return true;
        }
        if let Some(id) = self.lookup_signal_id(h) {
            return self.signal_is_real(id);
        }
        // §7.2.1: a member of a packed structure is integral.
        self.packed_struct_member_target(h).is_none()
    }

    pub(super) fn signal_is_real(&self, id: usize) -> bool {
        self.signal_real
            .is_none_or(|r| r.get(id).copied().unwrap_or(true))
    }

    /// §6.12.1, §6.24.1: `reg` holds the value of `e`, about to be fitted to a
    /// `width`-bit integral type. `Resize` rounds a real to every width but
    /// 64, where it keeps the real (real storage is 64 bits wide), so a value
    /// that may be real at that width is rounded first.
    pub(super) fn emit_real_to_int_fit(&mut self, reg: RegId, e: &Expression, width: u32) {
        if width == 64 && self.value_may_be_real(e) {
            self.emit(Insn::RealToInt(reg));
        }
    }

    /// The assignment target `lv` is known to be real: a real variable or
    /// net, or an element of a real unpacked array.
    pub(super) fn lvalue_is_real(&self, lv: &Expression) -> bool {
        match &lv.kind {
            ExprKind::Ident(_) => self.expr_is_real_static(lv),
            ExprKind::Index { expr: base, .. } => match &base.kind {
                ExprKind::Ident(h) if h.path.iter().all(|s| s.selects.is_empty()) => {
                    if let Some(ab) = self.local_var_array.get(&Self::hier_raw_name(h)) {
                        return ab.is_real;
                    }
                    self.signal_real.is_some()
                        && self.lookup_array_name(h).is_some_and(|name| {
                            let lo = self.arrays.get(&name).map_or(0, |&(lo, _, _)| lo);
                            self.lookup_signal_id_by_name(&format!("{name}[{lo}]"))
                                .is_some_and(|id| self.signal_is_real(id))
                        })
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// §10.7, §6.12.1: a value stored into a real target converts to real
    /// first, at its own width. Fitting it to the target's 64-bit storage
    /// first would widen an unbased unsized `'1` to 2**64-1 (§5.7.1: it has
    /// one bit where the context gives no width) and truncate an integral
    /// value wider than 64 bits.
    pub(super) fn emit_real_target_fit(&mut self, reg: RegId, rhs: &Expression) {
        if !self.expr_is_real_static(rhs) {
            self.emit_to_real(reg);
        }
    }
}
