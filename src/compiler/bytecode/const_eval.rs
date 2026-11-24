//! Constant folding and constant evaluation.

use super::*;

impl<'a> BytecodeCompiler<'a> {
    /// Fold `e` to a compile-time constant, consulting unrolled loop vars.
    /// Conservative: 4-state-clean integers only.
    pub(super) fn fold_const(&mut self, e: &Expression) -> Option<Value> {
        let v = match &e.kind {
            ExprKind::Number(n) => self.eval_number_static(n)?,
            ExprKind::Paren(i) => return self.fold_const(i),
            ExprKind::Ident(h) => {
                if h.root.is_none() && h.path.len() == 1 && h.path[0].selects.is_empty() {
                    if let Some(v) = self.local_const_vars.get(&h.path[0].name.name) {
                        v.clone()
                    } else if self.local_var_reg_of(h).is_some() {
                        return None;
                    } else {
                        self.lookup_param_value(h)?
                    }
                } else {
                    self.lookup_param_value(h)
                        .or_else(|| self.const_param_member_path(h))?
                }
            }
            ExprKind::Unary { op, operand } => {
                let v = self.fold_const(operand)?;
                match op {
                    UnaryOp::Plus => v,
                    // §11.4.3/§11.8.1: negation keeps its operand's
                    // signedness — `-1` is a signed constant, so `i > -1`
                    // on a signed `i` compares signed.
                    UnaryOp::Minus => {
                        let mut r = Value::zero(v.width.max(32)).sub(&v);
                        r.is_signed = v.is_signed;
                        r
                    }
                    UnaryOp::BitNot => v.bitwise_not(),
                    UnaryOp::LogNot => v.logic_not(),
                    _ => return None,
                }
            }
            ExprKind::Binary { op, left, right } => {
                let l = self.fold_const(left)?;
                let r = self.fold_const(right)?;
                match op {
                    BinaryOp::Add => l.add(&r),
                    BinaryOp::Sub => l.sub(&r),
                    BinaryOp::Mul => l.mul(&r),
                    BinaryOp::Div => l.div(&r),
                    BinaryOp::Mod => l.modulo(&r),
                    BinaryOp::ShiftLeft | BinaryOp::ArithShiftLeft => l.shift_left(&r),
                    BinaryOp::ShiftRight => l.shift_right(&r),
                    BinaryOp::BitAnd => l.bitwise_and(&r),
                    BinaryOp::BitOr => l.bitwise_or(&r),
                    BinaryOp::BitXor => l.bitwise_xor(&r),
                    BinaryOp::Lt => l.less_than(&r),
                    BinaryOp::Leq => l.less_equal(&r),
                    BinaryOp::Gt => l.greater_than(&r),
                    BinaryOp::Geq => l.greater_equal(&r),
                    BinaryOp::Eq => l.is_equal(&r),
                    BinaryOp::Neq => l.is_not_equal(&r),
                    _ => return None,
                }
            }
            ExprKind::MemberAccess { expr, member } => {
                self.const_param_member(expr, &member.name)?
            }
            // §20.8.1: as the interpreter computes it, a 32-bit result, signed
            // as `system_function_result` types it (`$clog2(1) - 1 < 0`).
            ExprKind::SystemCall { name, args } if name == "$clog2" && args.len() == 1 => {
                let log = self.fold_clog2(&args[0])?;
                let mut r = Value::from_u64(u64::from(log), 32);
                r.is_signed = true;
                r
            }
            _ => return None,
        };
        (!v.has_xz() && !v.is_real).then_some(v)
    }

    /// §6.20.2, §7.2.1: a member of a struct-typed parameter (`Cfg.LW`) is a
    /// constant — the parameter's value sliced by its registered layout. None
    /// of the constant folders had a MemberAccess arm, so a part-select bound
    /// written through one (`q[Cfg.LW/Cfg.AW-1:1]` on a `[1:0][63:0]` array)
    /// counted as dynamic and took the flat BIT-range path: CVA6's AXI adapter
    /// returned its cache line shifted by 63 bits and the core never fetched a
    /// second instruction.
    pub(super) fn const_param_member(&self, base: &Expression, member: &str) -> Option<Value> {
        let mut base = base;
        while let ExprKind::Paren(inner) = &base.kind {
            base = inner;
        }
        let ExprKind::Ident(h) = &base.kind else {
            return None;
        };
        self.const_param_member_hier(h, member)
    }

    /// The inliner spells an instance's `Cfg.LW` as ONE hierarchical
    /// identifier whose last segment is the member (`u1.Cfg` . `LW`).
    pub(super) fn const_param_member_path(&self, h: &HierarchicalIdentifier) -> Option<Value> {
        if h.path.len() < 2 {
            return None;
        }
        let member = h.path.last()?.name.name.as_str();
        let base = HierarchicalIdentifier {
            root: h.root.clone(),
            path: h.path[..h.path.len() - 1].to_vec(),
            span: h.span,
            cached_signal_id: crate::ast::cache::SignalCache::new(None),
            cached_resolved_name: crate::ast::cache::NameCache::new(),
            bound: crate::ast::binding::Bound::UNSET,
        };
        self.const_param_member_hier(&base, member)
    }

    fn const_param_member_hier(&self, h: &HierarchicalIdentifier, member: &str) -> Option<Value> {
        if h.root.is_some() || h.path.iter().any(|s| !s.selects.is_empty()) {
            return None;
        }
        let pv = self.lookup_param_value(h)?;
        let (_, fields) = self.packed_struct_layout_for_hier(h)?;
        let &(_, off, w) = fields.iter().find(|(n, _, _)| n == member)?;
        if w == 0 || off.checked_add(w)? > pv.width {
            return None;
        }
        Some(pv.range_select((off + w - 1) as usize, off as usize))
    }

    /// `fold_const` as a signed 64-bit integer (x/z or too wide: None).
    pub(super) fn fold_const_i64(&mut self, e: &Expression) -> Option<i64> {
        let v = self.fold_const(e)?;
        if v.has_xz() || v.is_real || v.width == 0 || v.width > 64 {
            return None;
        }
        let u = v.to_u64()?;
        Some(
            if v.is_signed && v.width < 64 && (u >> (v.width - 1)) & 1 == 1 {
                (u | (u64::MAX << v.width)) as i64
            } else {
                u as i64
            },
        )
    }

    /// Signed constant for a part-select bound. `eval_const_expr` folds in
    /// u32, so a literal `-1`, `LO-1` or a negative parameter wrapped to
    /// 4294967295 and a write like `x[4:-1] = v` was dropped with a width
    /// of -4 (§11.5.1 keeps the in-range bits).
    pub(super) fn eval_const_bound(&self, e: &Expression) -> Option<i64> {
        match &e.kind {
            ExprKind::Paren(inner) => self.eval_const_bound(inner),
            ExprKind::Unary {
                op: UnaryOp::Minus,
                operand,
            } => Some(self.eval_const_bound(operand)?.wrapping_neg()),
            ExprKind::Unary {
                op: UnaryOp::Plus,
                operand,
            } => self.eval_const_bound(operand),
            ExprKind::Binary { op, left, right }
                if matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul) =>
            {
                let l = self.eval_const_bound(left)?;
                let r = self.eval_const_bound(right)?;
                Some(match op {
                    BinaryOp::Add => l.wrapping_add(r),
                    BinaryOp::Sub => l.wrapping_sub(r),
                    _ => l.wrapping_mul(r),
                })
            }
            ExprKind::Ident(hier) if hier.path.len() == 1 && hier.path[0].selects.is_empty() => {
                if let Some(&v) = self.const_var_binds.get(hier.path[0].name.name.as_str()) {
                    return Some(v as i64);
                }
                let pv = self.lookup_param_value(hier)?;
                if pv.is_signed {
                    pv.to_i64()
                } else {
                    pv.to_u64().map(|u| u as i64)
                }
            }
            // §20.8.1: `$clog2` of a constant is a constant (`a[$clog2(N)-1:3]`).
            ExprKind::SystemCall { name, args } if name == "$clog2" && args.len() == 1 => {
                Some(i64::from(self.fold_clog2(&args[0])?))
            }
            _ => self.eval_const_expr(e).map(|v| v as i64),
        }
    }

    /// §20.8.1: the operand of a `$clog2` that folds, as the unsigned value
    /// the interpreter's `$clog2` reads: a full-width literal, a parameter,
    /// or an unrolled loop's index, optionally in parentheses.
    /// An operator expression is self-determined (§11.6.1: `8'hff + 8'h01` is
    /// 0, `64'd1 << 33` keeps bit 33), which neither `eval_const_bound`
    /// (64-bit integers) nor `eval_const_expr` (32 bits) models, so it keeps
    /// the interpreter's call. So does a frame local that hides a parameter
    /// (§23.9), and an x/z value.
    fn fold_clog2(&self, e: &Expression) -> Option<u32> {
        let v = match &e.kind {
            ExprKind::Paren(inner) => return self.fold_clog2(inner),
            ExprKind::Number(n) => self.eval_number_static(n)?,
            ExprKind::Ident(h)
                if h.root.is_none() && h.path.len() == 1 && h.path[0].selects.is_empty() =>
            {
                let name = h.path[0].name.name.as_str();
                // A `foreach` index is an `int` (§12.7.3).
                if let Some(&k) = self.const_var_binds.get(name) {
                    return Some(Value::from_u64(u64::from(k as u32), 32).clog2_unsigned());
                }
                match self.local_const_vars.get(name) {
                    Some(v) => v.clone(),
                    None if self.frame_bound_target(e) => return None,
                    None => self.lookup_param_value(h)?,
                }
            }
            _ => return None,
        };
        if v.has_xz() || v.is_real {
            return None;
        }
        Some(v.clog2_unsigned())
    }

    pub(super) fn eval_const_expr(&self, e: &Expression) -> Option<u32> {
        match &e.kind {
            ExprKind::Number(n) => self.eval_number_static(n)?.to_u64().map(|v| v as u32),
            ExprKind::Paren(inner) => self.eval_const_expr(inner),
            ExprKind::Ident(hier) => {
                if hier.path.len() == 1 && hier.path[0].selects.is_empty() {
                    if let Some(&v) = self.const_var_binds.get(hier.path[0].name.name.as_str()) {
                        return Some(v as u32);
                    }
                }
                self.lookup_param_value(hier)
                    .or_else(|| self.const_param_member_path(hier))?
                    .to_u64()
                    .map(|u| u as u32)
            }
            ExprKind::MemberAccess { expr, member } => self
                .const_param_member(expr, &member.name)?
                .to_u64()
                .map(|u| u as u32),
            // Fold simple parameter arithmetic so slice bounds like
            // `[ENTRY_NUM-1:0]` resolve. Without this, expr_max_width on a
            // sliced range returned 1 (unwrap_or(0)), which then clobbered
            // bit-AND operand widths down to 1 via ctx_width propagation,
            // producing wrong results for `|(a[N-1:0] & b[N-1:0])`-shape
            // expressions. (Bug seen on c910 axi_fifo pop_req.)
            ExprKind::Binary { op, left, right } => {
                // LRM §11.4 operator set, evaluated in u64 (then truncated to
                // u32 for the slice-bound use-case). Logical && / || short-
                // circuit on the LHS to match §11.4.7.
                match op {
                    BinaryOp::LogAnd => {
                        let l = self.eval_const_expr(left)? as u64;
                        if l == 0 {
                            return Some(0);
                        }
                        let r = self.eval_const_expr(right)? as u64;
                        return Some(if r != 0 { 1 } else { 0 });
                    }
                    BinaryOp::LogOr => {
                        let l = self.eval_const_expr(left)? as u64;
                        if l != 0 {
                            return Some(1);
                        }
                        let r = self.eval_const_expr(right)? as u64;
                        return Some(if r != 0 { 1 } else { 0 });
                    }
                    _ => {}
                }
                let l = self.eval_const_expr(left)? as u64;
                let r = self.eval_const_expr(right)? as u64;
                let v: u64 = match op {
                    BinaryOp::Add => l.wrapping_add(r),
                    BinaryOp::Sub => l.wrapping_sub(r),
                    BinaryOp::Mul => l.wrapping_mul(r),
                    BinaryOp::Div => {
                        if r == 0 {
                            return None;
                        } else {
                            l / r
                        }
                    }
                    BinaryOp::Mod => {
                        if r == 0 {
                            return None;
                        } else {
                            l % r
                        }
                    }
                    // LRM §11.4.3 power — silently dropped before this fix.
                    BinaryOp::Power => {
                        let e = u32::try_from(r as i64).ok()?;
                        (l as i64).checked_pow(e)? as u64
                    }
                    BinaryOp::ShiftLeft | BinaryOp::ArithShiftLeft => l.checked_shl(r as u32)?,
                    BinaryOp::ShiftRight => l.checked_shr(r as u32)?,
                    BinaryOp::ArithShiftRight => ((l as i64).wrapping_shr(r as u32)) as u64,
                    BinaryOp::BitAnd => l & r,
                    BinaryOp::BitOr => l | r,
                    BinaryOp::BitXor => l ^ r,
                    BinaryOp::BitXnor => !(l ^ r),
                    BinaryOp::Eq | BinaryOp::CaseEq => {
                        if l == r {
                            1
                        } else {
                            0
                        }
                    }
                    BinaryOp::Neq | BinaryOp::CaseNeq => {
                        if l != r {
                            1
                        } else {
                            0
                        }
                    }
                    BinaryOp::Lt => {
                        if (l as i64) < (r as i64) {
                            1
                        } else {
                            0
                        }
                    }
                    BinaryOp::Leq => {
                        if (l as i64) <= (r as i64) {
                            1
                        } else {
                            0
                        }
                    }
                    BinaryOp::Gt => {
                        if (l as i64) > (r as i64) {
                            1
                        } else {
                            0
                        }
                    }
                    BinaryOp::Geq => {
                        if (l as i64) >= (r as i64) {
                            1
                        } else {
                            0
                        }
                    }
                    _ => return None,
                };
                Some(v as u32)
            }
            ExprKind::Unary { op, operand } => {
                let v = self.eval_const_expr(operand)? as u64;
                let r: u64 = match op {
                    UnaryOp::Plus => v,
                    UnaryOp::Minus => 0u64.wrapping_sub(v),
                    UnaryOp::BitNot => !v,
                    UnaryOp::LogNot => {
                        if v == 0 {
                            1
                        } else {
                            0
                        }
                    }
                    // LRM §11.4.9 reductions. The unknown bit-width is OK here
                    // since callers use this for sizing/indexing — `|MASK` only
                    // needs to be 1 if MASK has any set bits.
                    UnaryOp::BitAnd => {
                        if v == u64::MAX {
                            1
                        } else {
                            0
                        }
                    }
                    UnaryOp::BitNand => {
                        if v == u64::MAX {
                            0
                        } else {
                            1
                        }
                    }
                    UnaryOp::BitOr => {
                        if v != 0 {
                            1
                        } else {
                            0
                        }
                    }
                    UnaryOp::BitNor => {
                        if v != 0 {
                            0
                        } else {
                            1
                        }
                    }
                    UnaryOp::BitXor => (v.count_ones() & 1) as u64,
                    UnaryOp::BitXnor => 1 - ((v.count_ones() & 1) as u64),
                    _ => return None,
                };
                Some(r as u32)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                let c = self.eval_const_expr(condition)?;
                if c != 0 {
                    self.eval_const_expr(then_expr)
                } else {
                    self.eval_const_expr(else_expr)
                }
            }
            _ => None,
        }
    }

    pub(super) fn eval_number_static(&self, num: &NumberLiteral) -> Option<Value> {
        match num {
            NumberLiteral::Integer {
                size,
                signed,
                base,
                value,
                cached_val,
            } => {
                // §5.7.1 — see `Value::unsized_integer_width`.
                let w = match size {
                    Some(sz) => *sz,
                    None => Value::unsized_integer_width(
                        value,
                        match base {
                            NumberBase::Binary => 2,
                            NumberBase::Octal => 8,
                            NumberBase::Hex => 16,
                            NumberBase::Decimal => 10,
                        },
                        *signed,
                    ),
                };
                // §5.7.1: unsized all-x/all-z literal is a FILL (see
                // `Value::unsized_xz_fill_char`) — replicate to context.
                let xz_fill = size.is_none() && Value::unsized_xz_fill_char(value).is_some();
                if let Some((vb, xz, cw)) = cached_val.get() {
                    if cw == w {
                        let mut v = Value::from_inline(vb, xz, w);
                        v.is_signed = *signed;
                        v.is_fill = xz_fill;
                        return Some(v);
                    }
                }
                let r = match base {
                    NumberBase::Binary => 2,
                    NumberBase::Octal => 8,
                    NumberBase::Hex => 16,
                    NumberBase::Decimal => 10,
                };
                let mut v = Value::from_literal_text(value, r, w);
                // Same inline cache the AST evaluator keeps on this node: a
                // literal in a declared range is re-evaluated for every
                // reference to the signal.
                if w <= 64 {
                    if let Some((vb, xz)) = v.inline_bits() {
                        cached_val.set(Some((vb, xz, w)));
                    }
                }
                v.is_signed = *signed;
                v.is_fill = xz_fill;
                Some(v)
            }
            // A real literal must keep its fractional value as IEEE-754 bits so
            // the VM's real-aware arithmetic sees a real operand. The old
            // `*f as u64` truncated `4.4`→`4` and `5.5`→`5`, turning a comb/
            // cont-assign `(1.0/4.4)*1000.0` into integer `1/4*1000 = 0` (the
            // PLL clamp-mode `vcofbperiod` went to 0 → a #0 vclk livelock).
            NumberLiteral::Real(f) => Some(Value::from_f64(*f)),
            // A time literal's VALUE depends on the active scope's timescale
            // (§5.8: `30000ps` under 1ns/1ps is 30, not 30000 ns ticks) —
            // context this compiler does not carry. The old hardcoded ×1e9
            // silently mis-scaled every non-ns scope; decline instead so the
            // AST path (which scales per scope) stays authoritative.
            NumberLiteral::Time(_) => None,
            // §5.7.1: unbased-unsized literal — a 1-bit FILL value; the Value
            // binary ops and resize replicate it to the consuming context.
            NumberLiteral::UnbasedUnsized(c) => Some(Value::fill_of(*c)),
        }
    }
}
