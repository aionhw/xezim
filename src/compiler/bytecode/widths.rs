//! Expression widths and signedness.

use super::*;

impl<'a> BytecodeCompiler<'a> {
    pub fn infer_lhs_width_pub(&self, lhs: &Expression) -> u32 {
        self.infer_lhs_width(lhs)
    }

    pub(super) fn infer_lhs_width(&self, lhs: &Expression) -> u32 {
        match &lhs.kind {
            ExprKind::Ident(hier) => {
                // Register-backed locals FIRST — they shadow any same-named
                // signal, and they are absent from every signal table, so the
                // fallthrough guessed 32 bits: a 128-bit local accumulator in
                // an inlined task truncated on every assignment.
                if let Some((_, w)) = self.local_var_reg_of(hier) {
                    if w > 0 {
                        return w;
                    }
                    // A width-0 STRING local is deliberate: falling through
                    // to the 32-bit default truncated the text to 4 chars.
                    if self
                        .local_var_is_string
                        .contains(&Self::hier_raw_name(hier))
                    {
                        return 0;
                    }
                }
                // Register-bound LOCAL ARRAY element in Ident-with-select
                // shape (`row[0] = …` inside an inlined body).
                if hier.path.len() == 1 && hier.path[0].selects.len() == 1 {
                    if let Some(ab) = self.local_var_array.get(hier.path[0].name.name.as_str()) {
                        return ab.elem_w;
                    }
                }
                // Step 9e: in method mode, a BARE class member store
                // (`m_address = addr`) must evaluate its RHS self-determined
                // (0), exactly like the interpreter's heap store — the module
                // widths fallback said 32 and truncated
                // uvm_tlm_generic_payload::set_address's 64-bit `bit [63:0]`
                // actual to 32 (20extfunc printed 'h90abcdef). §11.6.1: a
                // field of fixed declared width is the context, as for
                // `this.m = rhs`, so `m = (1 << n) - 1` keeps the carry.
                if self.method_mode
                    && hier.root.is_none()
                    && hier.path.len() == 1
                    && hier.path[0].selects.is_empty()
                {
                    let bare = hier.path[0].name.name.as_str();
                    if !self.local_var_regs.contains_key(bare)
                        && self.bare_member_names.contains(bare)
                    {
                        return self.bare_member_width(hier).unwrap_or(0);
                    }
                    // Step 9g: a bare STATIC-property store evaluates its
                    // RHS self-determined — class_static_set stores the
                    // Value verbatim.
                    if !self.local_var_regs.contains_key(bare)
                        && self.static_member_names.contains(bare)
                    {
                        return 0;
                    }
                    // class-perf P2: a bare STRING member store/read is
                    // self-determined — a nonzero width would Resize and
                    // truncate the text from the front.
                    if !self.local_var_regs.contains_key(bare)
                        && self.string_member_names.contains(bare)
                    {
                        return 0;
                    }
                }
                if let Some(id) = self.lookup_signal_id(hier) {
                    self.signal_widths[id]
                } else if let Some((_, _, mw)) = self.packed_struct_member_target(hier) {
                    mw
                } else {
                    let raw = Self::hier_raw_name(hier);
                    self.widths.declared_width(&raw).unwrap_or(32)
                }
            }
            ExprKind::Index { expr, .. } => {
                if let ExprKind::Ident(hier) = &expr.kind {
                    // Register-bound LOCAL ARRAY element (inlined pure body):
                    // its declared element width. Missing this returned the
                    // 1-bit default, whose Resize destroyed the value (a REAL
                    // element resized-to-1 collapsed to 1.0).
                    if hier.path.len() == 1 && hier.path[0].selects.is_empty() {
                        if let Some(ab) = self.local_var_array.get(hier.path[0].name.name.as_str())
                        {
                            return ab.elem_w;
                        }
                    }
                    // Register-bank local array element: the declared element
                    // width. Missing this returned the 1-bit default and a
                    // bank store truncated every value to its LSB.
                    if hier.path.len() == 1 && hier.path[0].selects.is_empty() {
                        if let Some(&LocalArrayBank { width: ew, .. }) =
                            self.local_array_regs.get(&hier.path[0].name.name)
                        {
                            return ew;
                        }
                    }
                    // Register-backed PACKED-of-packed local (`u8_vec16_t y`
                    // in an inlined body): `y[i]` selects an ew-bit element,
                    // not one bit. Falling through compiled the RHS at width
                    // 1 and the splice wrote the value's LSB only.
                    if let Some(&ew) = self.local_var_elem.get(&Self::hier_raw_name(hier)) {
                        if ew > 1 {
                            return ew;
                        }
                    }
                    if let Some(name) = self.lookup_array_name(hier) {
                        if let Some((_, _, elem_w)) = self.arrays.get(&name) {
                            return *elem_w;
                        }
                    }
                    let raw = Self::hier_raw_name(hier);
                    if let Some((_, _, elem_w)) = self.arrays.get(&raw) {
                        return *elem_w;
                    }
                    // Packed multi-D vector: element is N bits, not 1.
                    if let Some(elem_w) = self.packed_elem_widths.and_then(|m| {
                        self.packed_shape_at(m, &raw)
                            .copied()
                            .or_else(|| hier.path.last().and_then(|s| m.get(s.name.name.as_str()).copied()))
                    }) {
                        if elem_w > 1 {
                            return elem_w;
                        }
                    }
                    // An ASSOCIATIVE array's element: its width lives in its
                    // own map (assoc elements have no signal-table entry, so
                    // `arrays` does not carry them).
                    if let Some(elem_w) = self.assoc_elem_widths.and_then(|m| {
                        m.get(raw.as_str()).copied().or_else(|| {
                            hier.path
                                .last()
                                .and_then(|s| m.get(s.name.name.as_str()).copied())
                        })
                    }) {
                        if elem_w > 0 {
                            return elem_w;
                        }
                    }
                    // Not an array — bit-select on a plain packed signal; width = 1.
                    1
                } else if self.plain_unpacked_elem(expr).is_some() {
                    // A bit of one unpacked-array element (`arr[i][j]`): the
                    // context width of its RHS is 1 (§11.6.1), not 32.
                    1
                } else if let Some(w) = self.chained_packed_select_width(lhs) {
                    // `d[i][j] = v` on a multi-D packed vector selects a whole
                    // sub-array (§7.4.1). The 32-bit default below truncated
                    // the RHS of a continuous assign or always_comb write to
                    // a wider element before the store ever saw it.
                    w
                } else {
                    // §7.4.5, §11.6.1: `a[i][j] <= v` on a multidimensional
                    // unpacked array. The 32-bit default below cut the value
                    // before the store, whatever the element's type.
                    self.multi_dim_unpacked_elem_width(lhs).unwrap_or(32)
                }
            }
            ExprKind::RangeSelect {
                expr: base,
                left,
                right,
                kind,
            } => {
                let count = match kind {
                    RangeKind::IndexedUp | RangeKind::IndexedDown => self.eval_const_expr(right),
                    RangeKind::Constant => self
                        .eval_const_bound(left)
                        .zip(self.eval_const_bound(right))
                        .map(|(l, r)| {
                            // Signed bounds: `x[4:-1]` is 6 bits wide, not the
                            // u32-wrapped 4294967292 that clamped to the cap.
                            let (hi, lo) = if l >= r { (l, r) } else { (r, l) };
                            (hi - lo + 1).clamp(1, u32::MAX as i64) as u32
                        }),
                };
                match count {
                    // §7.4.3: `count` whole elements of the selected
                    // dimension. Sizing the target by the label count wrote
                    // 6 of the 48 bits of `assign a[0][5:0] = b` on
                    // `logic [3:0][5:0][7:0] a`.
                    Some(n) => match self.part_select_elem(base) {
                        Some((_, ew)) => n.saturating_mul(ew),
                        None => n,
                    },
                    None => 32,
                }
            }
            ExprKind::Concatenation(parts) => parts.iter().map(|p| self.infer_lhs_width(p)).sum(),
            // §11.6.1: a heap-field store's context is the field's declared
            // width, for every admitted receiver, not just `this`, so
            // `d.f = a + b` keeps the carry. Fields without a fixed integral
            // width (strings, reals, parameter-sized) stay self-determined
            // (0) and StoreClassMember fits them; a guessed 32-bit context
            // truncated strings and wide values.
            ExprKind::MemberAccess { expr, member } if self.method_mode => {
                if self.method_handle_chain_ok(expr)
                    && self.method_instance_field_ok(expr, &member.name)
                {
                    self.method_handle_chain_class(expr)
                        .and_then(|class| {
                            self.method_admission
                                .as_ref()?
                                .instance_field_widths
                                .get(&class)?
                                .get(&member.name)
                                .copied()
                        })
                        .unwrap_or(0)
                } else {
                    32
                }
            }
            _ => 32,
        }
    }

    /// LRM §11.6.1 SELF-determined width — max-of-operands with NO carry
    /// headroom (expr_max_width deliberately over-reports so temporaries
    /// never truncate; a shift/divide OPERAND must take the LRM width or the
    /// dropped carry returns: `(a<<4)>>2` at 8 bits read 0x8c for 0x0c).
    pub(super) fn lrm_self_width(&mut self, e: &Expression) -> u32 {
        match &e.kind {
            ExprKind::Paren(i) => self.lrm_self_width(i),
            ExprKind::Number(NumberLiteral::Integer { size: Some(sz), .. }) => *sz,
            ExprKind::Number(n @ NumberLiteral::Integer { size: None, .. }) => {
                super::elaborate::number_literal_width(n)
            }
            ExprKind::Number(NumberLiteral::UnbasedUnsized(_)) => 1,
            ExprKind::Unary { op, operand } => match op {
                UnaryOp::Plus | UnaryOp::Minus | UnaryOp::BitNot => self.lrm_self_width(operand),
                _ => 1,
            },
            ExprKind::Binary { op, left, right } => match op {
                BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::Mod
                | BinaryOp::BitAnd
                | BinaryOp::BitOr
                | BinaryOp::BitXor
                | BinaryOp::BitXnor => self.lrm_self_width(left).max(self.lrm_self_width(right)),
                BinaryOp::ShiftLeft
                | BinaryOp::ShiftRight
                | BinaryOp::ArithShiftLeft
                | BinaryOp::ArithShiftRight
                | BinaryOp::Power => self.lrm_self_width(left),
                _ => 1,
            },
            ExprKind::Conditional {
                then_expr,
                else_expr,
                ..
            } => self
                .lrm_self_width(then_expr)
                .max(self.lrm_self_width(else_expr)),
            _ => self.expr_max_width(e),
        }
    }

    /// Static signedness of an expression operand (§11.8.1), where knowable.
    /// `Some(false)` is the only answer that changes codegen (it forces a
    /// zero-extending widen); anything uncertain returns `None` and keeps the
    /// historical sign-by-value-flag behavior.
    /// True when compiling `e` provably leaves an UNSIGNED value in its
    /// register at RUNTIME — stronger than §11.8.1's static judgment, which
    /// says what the flag SHOULD be, not what the exec arms produce. The
    /// §11.8.1 sites scrub such an operand with `ClearSigned`; when this
    /// returns true that scrub is a no-op and is elided (16.5% of executed
    /// insns on a comb-dense fabric were these scrubs).
    ///
    /// Proof obligations, per accepted shape:
    /// - bit/part select of a plain signal: `Value::bit_select` /
    ///   `range_select` construct `is_signed: false` on every path (fast,
    ///   slow, zext, Wide→Wide — verified in value.rs).
    /// - plain load of an unsigned signal: `LoadSignal` copies the table
    ///   Value, and every table write path re-stamps `is_signed` from
    ///   `signal_signed` before committing.
    /// Idents that name params (compiled as `LoadConst` of the param value)
    /// or arrays (element values keep their own flag) are REJECTED, as is
    /// everything this fn doesn't recognise — a false `false` only keeps a
    /// redundant scrub.
    pub(super) fn operand_scrub_is_noop(&self, e: &Expression) -> bool {
        let plain_unsigned_signal = |h: &crate::ast::expr::HierarchicalIdentifier| -> bool {
            if !h.path.iter().all(|s| s.selects.is_empty()) {
                return false;
            }
            let name = h.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
            if self.arrays.contains_key(name)
                || self.multi_dim_arrays.is_some_and(|m| m.contains(name))
                || self.assoc_arrays.is_some_and(|m| m.contains_key(name))
            {
                return false;
            }
            if self.lookup_param_value(h).is_some() {
                return false;
            }
            self.lookup_signal_id(h)
                .is_some_and(|id| !self.signal_signed[id])
        };
        // Root of a select/member chain. A select is compiled as a
        // value-select (unsigned result) UNLESS the chain roots in a
        // CONTAINER name — array / assoc / multi-dim — whose element LOAD
        // copies the stored element's own flag. A non-ident root (concat,
        // binary result, call, …) always selects a plain register value.
        fn chain_root(e: &Expression) -> Option<&crate::ast::expr::HierarchicalIdentifier> {
            match &e.kind {
                ExprKind::Paren(inner) => chain_root(inner),
                ExprKind::Index { expr, .. }
                | ExprKind::RangeSelect { expr, .. }
                | ExprKind::MemberAccess { expr, .. } => chain_root(expr),
                ExprKind::Ident(h) => Some(h),
                _ => None,
            }
        }
        let root_is_container = |e: &Expression| -> bool {
            match chain_root(e) {
                None => false,
                Some(h) => {
                    let name = h.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
                    // Path-segment selects mean the flattened key differs
                    // from `name` — refuse rather than mis-key the check.
                    !h.path.iter().all(|s| s.selects.is_empty())
                        || self.arrays.contains_key(name)
                        || self.multi_dim_arrays.is_some_and(|m| m.contains(name))
                        || self.assoc_arrays.is_some_and(|m| m.contains_key(name))
                        || self.string_signals.is_some_and(|m| m.contains(name))
                }
            }
        };
        match &e.kind {
            ExprKind::Paren(inner) => self.operand_scrub_is_noop(inner),
            ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => {
                // Selects are unsigned on every exec path (bit_select /
                // range_select construct is_signed:false) — regardless of
                // the base's own flag — unless the "select" is really a
                // container-element load.
                !root_is_container(expr)
                    && !(self.method_mode && self.member_coll_elem_target(expr).is_some())
            }
            ExprKind::Ident(h) => plain_unsigned_signal(h),
            _ => false,
        }
    }

    pub(super) fn expr_signedness(&mut self, e: &Expression) -> Option<bool> {
        if let Some((_, m)) = self.formal_struct_member(e) {
            return Some(m.signed);
        }
        if self.formal_member_chain(e).is_some() {
            return Some(false);
        }
        match &e.kind {
            ExprKind::Number(NumberLiteral::Integer { signed, .. }) => Some(*signed),
            ExprKind::Number(NumberLiteral::UnbasedUnsized(_)) => Some(false),
            ExprKind::Paren(i) => self.expr_signedness(i),
            // §20/§21: an `int`/`integer`-valued system function is SIGNED
            // (`$countones(x) - 8 < 0` is a signed compare); `bit`, `time`
            // and the unsigned-int ones are not; `$past`/`$sampled` carry
            // their operand's signedness.
            ExprKind::SystemCall { name, args }
                if matches!(name.as_str(), "$past" | "$sampled") =>
            {
                args.first().and_then(|a| self.expr_signedness(a))
            }
            ExprKind::SystemCall { name, .. } if system_function_result(name).is_some() => {
                system_function_result(name).map(|(_, signed)| signed)
            }
            ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                // A register-backed local of a known integral type carries its
                // declared signedness (`emit_local_fit` stamps it on every
                // write); an untyped one stays unknown.
                if let Some((reg, _)) = self.local_var_reg_of(h) {
                    return match self.local_kinds.get(&reg) {
                        Some(LocalKind::Int { signed, .. }) => Some(*signed),
                        _ => None,
                    };
                }
                // A bare instance member of the method's class.
                if self.bare_member_width(h).is_some() {
                    let class = self.method_class.as_deref()?;
                    return Some(self.class_field_signed(class, &h.path[0].name.name));
                }
                self.named_operand_signedness(h)
            }
            // §11.8.1: a package parameter (`pkg::P`), an interface variable
            // (`i0.u`), a hierarchical reference (`lf.u`) or a flattened
            // unpacked-structure member (`unp.u`) has its declared type's
            // signedness, so an unsigned one makes the expression unsigned.
            ExprKind::Ident(h)
                if h.path.len() > 1 && h.path.iter().all(|s| s.selects.is_empty()) =>
            {
                let name = Self::hier_raw_name(h);
                if let Some(&known) = self.dotted_ident_sign.get(&name) {
                    return known;
                }
                let sign = if self.lookup_signal_id(h).is_none()
                    && let Some((key, member)) = self.packed_member_of_hier(h)
                {
                    // §7.2.1: a packed-structure member has its declared type.
                    self.packed_member_signed(&key, &member)
                } else {
                    self.named_operand_signedness(h)
                };
                self.dotted_ident_sign.insert(name, sign);
                sign
            }
            // §11.5, §13.4.1: a function call has its return type's sign.
            ExprKind::Call { func, .. } => {
                if let Some((_, signed)) = self.class_method_return_type(func) {
                    return Some(signed);
                }
                let ExprKind::Ident(h) = &func.kind else {
                    return None;
                };
                if h.root.is_some() || h.path.len() != 1 || !h.path[0].selects.is_empty() {
                    return None;
                }
                let name = self.scoped_callee_name(h.path[0].name.name.clone())?;
                let fd = self.functions.and_then(|f| f.get(&name))?;
                self.decl_signedness(&fd.return_type, Some(name.as_str()))
            }
            ExprKind::MemberAccess { expr: base, member } if self.method_mode => {
                self.handle_member_width(base, &member.name)?;
                let class = self.method_handle_chain_class(base)?;
                Some(self.class_field_signed(&class, &member.name))
            }
            // §7.2.1: a member of a packed-structure element (`a[0].f`) has
            // its declared type's sign, as the direct member read gives it.
            ExprKind::MemberAccess { expr: base, member } => {
                let mut root = base.as_ref();
                while let ExprKind::Index { expr, .. } | ExprKind::Paren(expr) = &root.kind {
                    root = &**expr;
                }
                let ExprKind::Ident(hier) = &root.kind else {
                    return None;
                };
                let (key, fields) = self.packed_struct_layout_for_hier(hier)?;
                if !fields.iter().any(|(name, _, _)| name == &member.name) {
                    return None;
                }
                self.packed_member_signed(&key, &member.name)
                    .or_else(|| self.packed_member_signed(&Self::hier_raw_name(hier), &member.name))
            }
            // Part-selects, concatenations and replications are UNSIGNED
            // regardless of their operands (§11.8.1).
            ExprKind::Index { .. } if self.packed_named_select_signed(e) => Some(true),
            ExprKind::Index { expr, .. } => {
                // §7.4: an unpacked element retains its declared type; only
                // a packed bit-select is necessarily unsigned (§11.8.1).
                if let ExprKind::Ident(h) = &expr.kind
                    && let Some(signed) = self
                        .local_var_array
                        .get(&Self::hier_raw_name(h))
                        .and_then(|ab| ab.elem_signed)
                {
                    return Some(signed);
                }
                if let Some(id) = self.const_multi_dim_array_elem_signal_id(e) {
                    return self.signal_signed.get(id).copied();
                }
                if let ExprKind::Ident(h) = &expr.kind
                    && let Some(name) = self.lookup_array_name(h)
                {
                    let first = self.array_first_id?.get(name.as_str())?.0;
                    return self.signal_signed.get(first).copied();
                }
                // A class member collection element (`d[0]` of `int d[]`)
                // has the element type's sign (§7.4).
                if self.method_mode
                    && let Some((coll, _, recv)) = self.member_coll_elem_target(expr)
                {
                    let class = self.coll_owner_class(recv.as_ref())?;
                    let colls = self.method_admission.as_ref()?.instance_colls.get(&class)?;
                    return if colls.signed_elems.contains(&coll) {
                        Some(true)
                    } else if colls.unsigned_elems.contains(&coll) {
                        Some(false)
                    } else {
                        None
                    };
                }
                self.operand_scrub_is_noop(e).then_some(false)
            }
            ExprKind::RangeSelect { .. }
            | ExprKind::Concatenation(_)
            | ExprKind::Replication { .. } => Some(false),
            ExprKind::SystemCall { name, args } => match name.as_str() {
                "$signed" => Some(true),
                "$unsigned" => Some(false),
                // §6.24.1: a SIZE cast preserves the operand's signedness.
                "$__xz_size_cast" => args.get(1).and_then(|a| self.expr_signedness(a)),
                // §6.24.1: a type cast (`u8_t'(x)`) has the type's sign; a
                // size cast through a parameter (`N'(x)`) keeps the operand's.
                "$__xz_named_cast" => {
                    let key = self.named_cast_key(args.first()?)?;
                    if let Some(&(_, signed)) = self.cast_widths.and_then(|m| m.get(&key)) {
                        Some(signed)
                    } else if self.params.is_some_and(|p| p.contains_key(&key)) {
                        args.get(1).and_then(|a| self.expr_signedness(a))
                    } else {
                        None
                    }
                }
                _ => None,
            },
            ExprKind::Unary { op, operand } => match op {
                UnaryOp::Plus
                | UnaryOp::Minus
                | UnaryOp::BitNot
                | UnaryOp::PreIncr
                | UnaryOp::PreDecr
                | UnaryOp::PostIncr
                | UnaryOp::PostDecr => self.expr_signedness(operand),
                // Reductions and ! are 1-bit unsigned.
                _ => Some(false),
            },
            ExprKind::Binary { op, left, right } => match op {
                BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::Mod
                | BinaryOp::BitAnd
                | BinaryOp::BitOr
                | BinaryOp::BitXor
                | BinaryOp::BitXnor
                | BinaryOp::Power => {
                    match (self.expr_signedness(left), self.expr_signedness(right)) {
                        (Some(true), Some(true)) => Some(true),
                        (Some(false), _) | (_, Some(false)) => Some(false),
                        _ => None,
                    }
                }
                // Shifts take the LEFT operand's signedness.
                BinaryOp::ShiftLeft
                | BinaryOp::ShiftRight
                | BinaryOp::ArithShiftLeft
                | BinaryOp::ArithShiftRight => self.expr_signedness(left),
                // Comparisons / logical ops are 1-bit unsigned.
                _ => Some(false),
            },
            ExprKind::Conditional {
                then_expr,
                else_expr,
                ..
            } => {
                match (
                    self.expr_signedness(then_expr),
                    self.expr_signedness(else_expr),
                ) {
                    (Some(true), Some(true)) => Some(true),
                    (Some(false), _) | (_, Some(false)) => Some(false),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// §11.8.1: the signedness of a named operand with no select: a signal
    /// (module variable, interface variable, hierarchical reference or a
    /// flattened unpacked member) has its declared sign, a parameter its
    /// value's (§6.20.2). Unknown when the name resolves to neither.
    fn named_operand_signedness(&self, h: &HierarchicalIdentifier) -> Option<bool> {
        if let Some(id) = self.lookup_signal_id(h) {
            return self.signal_signed.get(id).copied();
        }
        self.lookup_param_value(h)
            .filter(|v| !v.is_real)
            .map(|v| v.is_signed)
    }

    /// §6.11.3: the signedness a declared integral type gives its values;
    /// None for a real, string or unresolved type.
    fn decl_signedness(
        &self,
        dt: &crate::ast::types::DataType,
        scope: Option<&str>,
    ) -> Option<bool> {
        use crate::ast::types::DataType as D;
        let resolved = self.resolved_decl_type(dt, scope);
        match resolved {
            D::IntegerAtom { .. } | D::IntegerVector { .. } | D::Enum(_) | D::Implicit { .. } => {
                Some(crate::compiler::elaborate::is_type_signed(resolved))
            }
            D::Struct(su) if su.packed => {
                Some(crate::compiler::elaborate::is_type_signed(resolved))
            }
            _ => None,
        }
    }

    /// Self-determined width of a system function's RESULT (IEEE 1800-2017
    /// §20/§21) — see `system_function_result`; `$signed`/`$unsigned`/
    /// `$past`/`$sampled` carry their argument's width. Real- and
    /// string-valued functions report None (no integral width to contribute
    /// to a context).
    fn system_function_width(&self, name: &str, args: &[Expression]) -> Option<u32> {
        if system_function_carries_arg(name) {
            return args.first().map(|a| self.expr_max_width(a));
        }
        system_function_result(name).map(|(w, _)| w)
    }

    /// The register of a bare string local (`s`, not `s[i]` or a member).
    pub(super) fn string_local_reg(&self, e: &Expression) -> Option<RegId> {
        let ExprKind::Ident(h) = &e.kind else {
            return None;
        };
        if h.root.is_some() || h.path.len() != 1 || !h.path[0].selects.is_empty() {
            return None;
        }
        let name = h.path[0].name.name.as_str();
        if !self.local_var_is_string.contains(name) {
            return None;
        }
        self.local_var_regs.get(name).map(|&(r, _)| r)
    }

    /// Method mode: the declared width of the bare instance member `hier`
    /// names (not a local or formal), when it is a fixed-width integral.
    fn bare_member_width(&self, hier: &HierarchicalIdentifier) -> Option<u32> {
        if !self.method_mode
            || hier.root.is_some()
            || hier.path.len() != 1
            || !hier.path[0].selects.is_empty()
        {
            return None;
        }
        let name = hier.path[0].name.name.as_str();
        if self.local_var_regs.contains_key(name) || !self.bare_member_names.contains(name) {
            return None;
        }
        self.this_field_widths
            .get(name)
            .copied()
            .or_else(|| self.class_field_width(self.method_class.as_deref()?, name))
    }

    /// Method mode: the declared width of `base.member` through an admitted
    /// handle chain, when it is a fixed-width integral.
    fn handle_member_width(&self, base: &Expression, member: &str) -> Option<u32> {
        if !self.method_handle_chain_ok(base) || !self.method_instance_field_ok(base, member) {
            return None;
        }
        if matches!(base.kind, ExprKind::This)
            && let Some(&w) = self.this_field_widths.get(member)
        {
            return Some(w);
        }
        self.class_field_width(&self.method_handle_chain_class(base)?, member)
    }

    fn class_field_signed(&self, class: &str, field: &str) -> bool {
        (self.method_class.as_deref() == Some(class) && self.this_field_signed.contains(field))
            || self.method_admission.as_ref().is_some_and(|admission| {
                admission
                    .instance_field_signed
                    .get(class)
                    .is_some_and(|fields| fields.contains(field))
            })
    }

    fn class_field_width(&self, class: &str, field: &str) -> Option<u32> {
        self.method_admission
            .as_ref()?
            .instance_field_widths
            .get(class)?
            .get(field)
            .copied()
    }

    fn class_method_return_type(&self, func: &Expression) -> Option<(u32, bool)> {
        if !self.method_mode {
            return None;
        }
        let admission = self.method_admission.as_ref()?;
        match &func.kind {
            ExprKind::Ident(h)
                if h.root.is_none()
                    && h.path.len() == 1
                    && h.path[0].selects.is_empty()
                    && self.class_method_names.contains(h.path[0].name.name.as_str()) =>
            {
                let class = self.method_class.as_ref()?;
                let base = class.split('#').next().unwrap_or(class.as_str());
                admission
                    .method_return_types
                    .get(&(class.clone(), h.path[0].name.name.clone()))
                    .or_else(|| {
                        admission
                            .method_return_types
                            .get(&(base.to_string(), h.path[0].name.name.clone()))
                    })
                    .copied()
            }
            ExprKind::MemberAccess { expr: base, member } => {
                let class = if matches!(&base.kind, ExprKind::Ident(h)
                    if h.path.len() == 1
                        && h.path[0].selects.is_empty()
                        && h.path[0].name.name == "super")
                {
                    self.method_class_parent.clone()?
                } else {
                    self.method_handle_chain_class(base)?
                };
                let base_cls = class.split('#').next().unwrap_or(class.as_str());
                admission
                    .method_return_types
                    .get(&(class.clone(), member.name.clone()))
                    .or_else(|| {
                        admission
                            .method_return_types
                            .get(&(base_cls.to_string(), member.name.clone()))
                    })
                    .copied()
            }
            _ => None,
        }
    }

    /// §6.24.1 / §11.6.1: a cast is self-determined at its target width.
    /// That width is what a parent `+` or `>` must see —
    /// `((9)'(cnt) + step) > 8'hff` adds at 9 bits, so 255+1 saturates
    /// instead of wrapping. The system-call fallback reported 0, the sum
    /// stayed 8 bits, and the compare saw 0. A named target (`W'(x)`,
    /// `(W)'(x)`) contributes the parameter value or the typedef width.
    fn cast_self_width(&self, name: &str, args: &[Expression]) -> Option<u32> {
        let literal = args.first().and_then(|a| self.literal_cast_size(a));
        match name {
            "$__xz_size_cast" => literal.or_else(|| {
                args.first()
                    .and_then(|a| self.eval_const_expr(a))
                    .map(|n| n.max(1))
            }),
            "$__xz_named_cast" => literal.or_else(|| {
                if let Some(w) = self.member_cast_size(args.first()?) {
                    return Some(w);
                }
                let key = self.named_cast_key(args.first()?)?;
                self.cast_widths
                    .and_then(|m| m.get(&key).map(|&(w, _)| w.max(1)))
                    .or_else(|| {
                        self.params
                            .and_then(|p| p.get(&key))
                            .and_then(|v| v.to_u64())
                            .map(|n| (n as u32).max(1))
                    })
            }),
            _ => None,
        }
    }

    /// §6.24.1: a cast whose casting type is a constant primary is a SIZE
    /// cast. A field of a struct parameter (`pt.DEPTH'(x)`, also written
    /// `(pt.DEPTH)'(x)`) parses like a scoped type name, so it reaches the
    /// named-cast arm; it names a value, never a type. Its folded value, or
    /// None when the target is not such a field.
    pub(super) fn member_cast_size(&self, target: &Expression) -> Option<u32> {
        let v = match &target.kind {
            ExprKind::Paren(inner) => return self.member_cast_size(inner),
            // An instance's names arrive with the instance folded into the
            // first segment (`u.pt` . `DEPTH`), or rooted and spelled out
            // (`$root.u.pt.DEPTH`). Elaboration keys each member of a struct
            // parameter by that whole path. The instance's own parameter
            // (`u.pt`) has no layout entry, so the layout walk below found
            // one only through a same-named top-level parameter, whose type
            // may differ.
            ExprKind::Ident(h) if h.path.len() >= 2 && matches!(h.root.as_deref(), None | Some("$root")) => {
                let mut flat = h.clone();
                flat.root = None;
                self.params
                    .and_then(|p| p.get(Self::hier_raw_name(&flat).as_str()).cloned())
                    .or_else(|| self.const_param_member_path(&flat))?
            }
            ExprKind::MemberAccess { expr, member } => {
                self.const_param_member(expr, &member.name)?
            }
            _ => return None,
        };
        if v.has_xz() || v.is_real {
            return None;
        }
        v.to_u64()
            .filter(|&n| n > 0)
            .and_then(|n| u32::try_from(n).ok())
    }

    /// Value of a cast-size literal (`9` in `9'(x)`), not the literal's
    /// own bit width.
    fn literal_cast_size(&self, e: &Expression) -> Option<u32> {
        let ExprKind::Number(n) = &e.kind else {
            return None;
        };
        self.eval_number_static(n)
            .and_then(|v| v.to_u64())
            .map(|bits| (bits as u32).max(1))
    }

    /// Name a `$__xz_named_cast` target resolves to. Same key the cast
    /// compiler uses, so the width reported here is the width it resizes to.
    fn named_cast_key(&self, target: &Expression) -> Option<String> {
        match &target.kind {
            ExprKind::Paren(inner) => self.named_cast_key(inner),
            ExprKind::Ident(h) if h.path.len() >= 2 => {
                let n = h.path.len();
                let key = format!("{}::{}", h.path[n - 2].name.name, h.path[n - 1].name.name);
                let known = self.cast_widths.is_some_and(|m| m.contains_key(&key))
                    || self.params.is_some_and(|p| p.contains_key(&key));
                Some(if known {
                    key
                } else {
                    h.path[n - 1].name.name.clone()
                })
            }
            ExprKind::Ident(h) => h.path.last().map(|s| s.name.name.clone()),
            ExprKind::MemberAccess { expr, member } => {
                let ExprKind::Ident(h) = &expr.kind else {
                    return None;
                };
                let key = format!("{}::{}", h.path.last()?.name.name, member.name);
                let known = self.cast_widths.is_some_and(|m| m.contains_key(&key))
                    || self.params.is_some_and(|p| p.contains_key(&key));
                Some(if known { key } else { member.name.clone() })
            }
            _ => None,
        }
    }

    pub(super) fn expr_max_width(&self, expr: &Expression) -> u32 {
        if let Some(q) = self.own_pkg_qualified(expr) {
            return self.expr_max_width(&q);
        }
        if let Some((_, m)) = self.formal_struct_member(expr) {
            return m.width;
        }
        if let Some((_, m, idx)) = self.formal_member_chain(expr)
            && let Some(dims) = m.elems.as_deref()
        {
            return Self::formal_member_chain_width_of(dims, idx.len());
        }
        match &expr.kind {
            ExprKind::Ident(hier) => self
                .lookup_signal_id(hier)
                .map(|id| self.signal_widths[id])
                // A packed-struct MEMBER read (`req.addr`) is not a signal of
                // its own, so the lookup above misses and the old fallback of
                // 0 made every SELF-DETERMINED use of it 1 bit wide: inside a
                // concatenation, `{(req.addr >> 5), lsbs}` resized the shift's
                // operand to 1 bit and the whole term evaluated to 0.
                .or_else(|| self.packed_struct_member_target(hier).map(|(_, _, mw)| mw))
                // A BLOCK-LOCAL declaration — a `for (int i = ...)` header
                // variable or a `begin`-block temp — is a REGISTER, not a
                // signal, so both lookups above miss. The old fallback of 0
                // then made every self-determined use of it 1 bit wide. In an
                // array INDEX (compiled at ctx_width 0) that collapsed a
                // shift's operand width to `max(0, 0).max(1)` = 1, so
                // `arr[(i << 6) + off]` truncated `i` to one bit and the shift
                // masked the result away: every iteration read `arr[off]`.
                // `local_var_regs` already carries the declared width.
                .or_else(|| self.local_var_reg_of(hier).map(|(_, w)| w))
                // A bare instance member in a method body has its declared
                // width too; 0 here sized `(burst << 3) != line` (members on
                // both sides) at one bit and truncated the shift.
                .or_else(|| self.bare_member_width(hier))
                .unwrap_or(0),
            ExprKind::MemberAccess { expr: base, member } if self.method_mode => self
                .handle_member_width(base, &member.name)
                .or_else(|| self.class_scoped_enum(base, &member.name).map(|v| v.width))
                .unwrap_or(0),
            ExprKind::Number(n) => self.eval_number_static(n).map(|v| v.width).unwrap_or(32),
            ExprKind::Binary { op, left, right } => {
                // Relational, equality, and logical operators always
                // produce a 1-bit result regardless of operand width.
                // Returning operand width here pollutes the ctx_width
                // passed into a sibling bitwise operand of `&&`/`||`,
                // causing it to be resized up and XNOR-then-NOT to
                // produce ~0 in the upper bits — manifests as
                // `(a ^~ b) && (c < d)` returning 1 instead of 0 when
                // a^~b should be 0. (c910 BJU branch_blt_taken bug.)
                if matches!(
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
                ) {
                    1
                } else if matches!(
                    op,
                    BinaryOp::ShiftLeft
                        | BinaryOp::ShiftRight
                        | BinaryOp::ArithShiftLeft
                        | BinaryOp::ArithShiftRight
                ) {
                    // §11.4.10: a shift result has the left operand's width;
                    // the self-determined right operand cannot widen it.
                    self.expr_max_width(left)
                } else {
                    self.expr_max_width(left).max(self.expr_max_width(right))
                }
            }
            ExprKind::Unary { op, operand } => {
                // Self-determined unary: reductions and logical NOT all
                // produce 1 bit regardless of operand width.
                if matches!(
                    op,
                    UnaryOp::BitAnd
                        | UnaryOp::BitNand
                        | UnaryOp::BitOr
                        | UnaryOp::BitNor
                        | UnaryOp::BitXor
                        | UnaryOp::BitXnor
                        | UnaryOp::LogNot
                ) {
                    1
                } else {
                    self.expr_max_width(operand)
                }
            }
            ExprKind::Paren(inner) => self.expr_max_width(inner),
            ExprKind::Call { func, args } => {
                // §11.6.1: a function call is as wide as its return type; its
                // arguments are sized by the formals, not by the call's
                // context (`f(16'(a))` with an 8-bit result is 8 bits).
                if let Some((w, _)) = self.class_method_return_type(func)
                    && w > 0
                {
                    return w;
                }
                if let ExprKind::Ident(h) = &func.kind
                    && h.root.is_none()
                    && h.path.len() == 1
                    && h.path[0].selects.is_empty()
                    && let Some(name) = self.scoped_callee_name(h.path[0].name.name.clone())
                    && let Some(fd) = self.functions.and_then(|f| f.get(&name))
                    && !matches!(fd.return_type, crate::ast::types::DataType::Void(_))
                {
                    let w = self.decl_width_in(&fd.return_type, Some(name.as_str()));
                    if w > 0 {
                        return w;
                    }
                }
                args.iter()
                    .map(|a| self.expr_max_width(a))
                    .max()
                    .unwrap_or(0)
            }
            // A system FUNCTION has the result type the LRM gives it, not the
            // width of its arguments — and certainly not 0, which the old
            // catch-all returned: `out_step <= $countones(be) >> 3` with a
            // 4-bit target then compiled the shift at the target's width and
            // truncated a count of 24 to 8 before shifting.
            ExprKind::SystemCall { name, args } => self
                .cast_self_width(name, args)
                .or_else(|| self.system_function_width(name, args))
                .unwrap_or(0),
            ExprKind::Conditional {
                then_expr,
                else_expr,
                ..
            } => {
                // Verilog: result of `cond ? then : else` is max(then, else).
                // Condition is self-determined (does NOT contribute to result width).
                self.expr_max_width(then_expr)
                    .max(self.expr_max_width(else_expr))
            }
            ExprKind::Concatenation(parts) => parts.iter().map(|p| self.expr_max_width(p)).sum(),
            ExprKind::RangeSelect {
                expr: base,
                left,
                right,
                kind,
                ..
            } => {
                // §7.4.3: a select of whole elements is that many elements wide.
                let ew = self.part_select_elem(base).map_or(1, |(_, w)| w);
                match kind {
                    RangeKind::Constant => {
                        if let (Some(l), Some(r)) =
                            (self.eval_const_bound(left), self.eval_const_bound(right))
                        {
                            (((l as i64 - r as i64).unsigned_abs() as u32) + 1).saturating_mul(ew)
                        } else {
                            // Fallback when bounds aren't const-evaluable:
                            // use the base signal's full width. Returning a
                            // tiny value here (the old `unwrap_or(0)` path)
                            // truncated bit-AND operands via ctx_width.
                            self.expr_max_width(base)
                        }
                    }
                    RangeKind::IndexedUp | RangeKind::IndexedDown => self
                        .eval_const_expr(right)
                        .map(|n| n.saturating_mul(ew))
                        .unwrap_or_else(|| self.expr_max_width(base)),
                }
            }
            // A bit-select of a plain vector is 1 bit — but an ELEMENT select
            // of a packed multi-dimensional array is the whole element
            // (`s[1]` on `logic [1:0][11:0] s` is 12 bits). Reporting 1 here
            // under-sized the shift/divide context above, whose whole job is
            // `ctx_width.max(expr_max_width(left))`: the left operand was then
            // compiled at the ASSIGNMENT's width and the source's high bits
            // were truncated BEFORE the shift, so
            // `d[2] = s[1] >> N` (d's element narrower than s's) lost N extra
            // high bits. The procedural interpreter got this right, so the bug
            // only showed inside `always_comb`/compiled blocks.
            // §6.16: a select on a string local is a byte.
            ExprKind::Index { expr: base, .. } if self.string_local_reg(base).is_some() => 8,
            ExprKind::Index { expr: base, .. } => match &base.kind {
                // An index select is ONE BIT only when the base is a plain
                // vector. A packed-array element has its element width — and
                // so does an UNPACKED-array element, which this arm used to
                // miss: `(addr_i[0] & mask[0]) == base[0]` sized the compare
                // operands at max(1,1), truncated both sides of the `&` to a
                // single bit, and ibex's bus decoder never matched an address
                // once its enclosing block compiled. The sibling of the
                // `expr_max_width returned 1 for every index select` bug fixed
                // in the simulator earlier — same disease, other table.
                ExprKind::Ident(hier) => self
                    .packed_elem_width_of(hier)
                    .or_else(|| {
                        self.lookup_array_name(hier)
                            .and_then(|n| self.arrays.get(&n).map(|&(_, _, w)| w))
                    })
                    .or_else(|| {
                        self.lookup_array_name(hier)
                            .and_then(|n| self.assoc_elem_widths.and_then(|m| m.get(&n).copied()))
                    })
                    .or_else(|| self.member_coll_elem_decl_width(hier))
                    // A member of the element-access plan set whose element
                    // is a plain vector (`int unsigned base[5]`, read as
                    // `base[i]`): the read lowers to `LoadCollElem`, but
                    // none of the tables above carries its element width
                    // (packed dims: none recorded; module array tables:
                    // module scope only), so the select sized at 1 bit —
                    // and a self-determined context (`(base[i] < base[j])
                    // ? …`) then resized both element reads to their LSBs,
                    // comparing 0 < 0. The plan's element-width fact
                    // (`coll_elem_widths`) is the missing table.
                    .or_else(|| self.member_elem_plan_width_of(hier))
                    .unwrap_or(1),
                // issue-301: a select INTO a member-collection element
                // (`arr[i][2]` on `bit [8:0][7:0] arr[3]`) — the width is
                // the first packed dim's ELEMENT width (ew bits), not 1.
                // Reporting 1 here narrowed the SELF-DETERMINED compare
                // context (`arr[i][2] != i` resized the 8-bit slice down
                // to one bit and misjudged the equality), so only the
                // store-backed shapes above were safe.
                ExprKind::Index { expr: root, .. } => {
                    self.member_coll_elem_index_width(root).unwrap_or(1)
                }
                // `this.base[i]`: the same width sources as the bare
                // spelling, in the same order — multi-dim packed dims
                // first, then the plan element-width fact.
                ExprKind::MemberAccess { expr: recv, member }
                    if matches!(recv.kind, ExprKind::This) =>
                {
                    self.coll_elem_packed_dims
                        .get(&member.name)
                        .map(|dims| dims.iter().map(|(l, r)| (l - r).abs() as u32 + 1).product())
                        .or_else(|| self.member_elem_plan_width(&member.name, true))
                        .unwrap_or(1)
                }
                _ => 1,
            },
            ExprKind::Replication { count, exprs } => {
                let n = self.eval_const_expr(count).unwrap_or(0);
                let inner: u32 = exprs.iter().map(|e| self.expr_max_width(e)).sum();
                n * inner
            }
            // `base.member` in MemberAccess spelling — the same packed-struct
            // member width as the dotted-Ident form above.
            ExprKind::MemberAccess { expr: base, member } => {
                let ExprKind::Ident(bh) = &base.kind else {
                    return 0;
                };
                let mut h = bh.clone();
                h.path.push(crate::ast::expr::HierPathSegment {
                    name: member.clone(),
                    selects: Vec::new(),
                });
                self.packed_struct_member_target(&h)
                    .map(|(_, _, mw)| mw)
                    .unwrap_or(0)
            }
            _ => 0,
        }
    }
}
