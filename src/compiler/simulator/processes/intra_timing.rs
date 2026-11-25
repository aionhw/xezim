//! Delay values and intra-assignment timing controls.

use super::*;
use crate::ast::types::{StructUnionType, UnpackedDimension};

impl Simulator {
    /// Evaluate a `#delay` expression to an integer number of simulator ticks.
    /// A real-valued delay (e.g. `#1.5`, or `#0.5` under a finer `\`timescale`)
    /// is rounded to the nearest tick — LRM §3.14.3 rounds delays to the time
    /// precision. Crucially this avoids reading a real Value's raw float *bits*
    /// as a huge integer (the old `to_u64()` did, scheduling the continuation
    /// astronomically far out so it never fired and the process was lost).
    pub(in crate::compiler::simulator) fn eval_delay_ticks(&mut self, d: &Expression) -> u64 {
        let (_, prec_exp) = self.current_timescale_exp();
        let precision_diff = prec_exp - Self::secs_to_exp(self.tick_s);
        self.eval_delay_ticks_with_precision(d, precision_diff)
    }

    pub(in crate::compiler::simulator) fn eval_delay_ticks_with_precision(
        &mut self,
        d: &Expression,
        precision_diff: i32,
    ) -> u64 {
        let v = self.eval_expr(d);
        self.delay_value_to_ticks(&v, d, precision_diff)
    }

    pub(in crate::compiler::simulator) fn delay_value_to_ticks(
        &mut self,
        v: &Value,
        d: &Expression,
        precision_diff: i32,
    ) -> u64 {
        // §3.14.3: `1step` is one tick of the global precision, whatever
        // the running scope's precision (#615).
        if matches!(&d.kind, ExprKind::SystemCall { name, .. }
            if name == super::elaborate::ONE_STEP_DELAY)
        {
            return 1;
        }
        let raw = if v.is_real {
            let f = v.to_f64();
            // Diagnose a pathological real delay: non-finite (NaN/inf) or
            // absurdly large. A huge `#delay` parks the process ~forever — which
            // reads as "the clock stopped" — and is almost always a bit-garbage
            // value (an IEEE-754 pattern read as a ~1e18 number) or a divide that
            // produced inf/NaN. Warn once per site so the offending delay names
            // itself in the log instead of silently freezing a clock.
            if (!f.is_finite() || f >= 1e15)
                && self.warned_delay_spikes.insert(d.span.start as usize)
            {
                let loc = self
                    .span_file_line_in(d.span, None)
                    .map(|l| format!(" at {}", l))
                    .unwrap_or_default();
                eprintln!(
                    "[xezim][warning] a #delay evaluated to a pathological real value {:.4e}{} \
                         at time {} — the process is parked far in the future, so a clock driven \
                         by it will appear to stop. This usually means the delay expression \
                         produced NaN/inf or IEEE-754 bit garbage.",
                    f, loc, self.time
                );
            }
            if f.is_finite() && f > 0.0 { f } else { 0.0 }
        } else {
            return Self::quantize_integral_delay_ticks(v.to_u64().unwrap_or(0), precision_diff);
        };
        // §3.14.3: a delay rounds to the CURRENT MODULE's time precision, not
        // the (finer) global tick — `#1.55` in a 1ns/100ps module is 1.6ns,
        // and `#0.04` collapses to 0. `raw` is already tick-denominated (the
        // elaborator pre-scales by the module unit), so quantize to the
        // module-precision-per-tick grid. The grid ratio comes from EXPONENT
        // arithmetic (10^n is exact in f64 for small n; dividing seconds
        // values is not), and `round_time_units` keeps a half-grid delay
        // that arrived as x.49999999999996 from rounding the wrong way.
        Self::quantize_delay_ticks(raw, precision_diff)
    }

    /// §3.14.1: `raw` global ticks rounded to the running scope's precision,
    /// 10^`precision_diff` ticks. A count of 2^52 or more is a whole number
    /// already: the relative nudge used before, `x * (1 + 1e-12)`, moved
    /// `#9200s` under a 1 ps precision with a 1 fs tick by 9.2 ns.
    #[inline]
    pub(in crate::compiler::simulator) fn quantize_delay_ticks(
        raw: f64,
        precision_diff: i32,
    ) -> u64 {
        use super::elaborate::{round_time_units, scale_pow10};
        if precision_diff > 0 {
            scale_pow10(
                round_time_units(scale_pow10(raw, -precision_diff)),
                precision_diff,
            ) as u64
        } else {
            raw.round() as u64
        }
    }

    /// §3.14.1, §9.4.1: an integral tick count rounded to the running scope's
    /// precision, 10^`precision_diff` ticks, ties away from zero, in integer
    /// arithmetic. Converting the count to binary64 first lost its low bits
    /// above 2^53 ticks (`#(64'd9007199254740993)` under 1 fs woke at
    /// ...992).
    #[inline]
    pub(in crate::compiler::simulator) fn quantize_integral_delay_ticks(
        raw: u64,
        precision_diff: i32,
    ) -> u64 {
        if precision_diff <= 0 {
            return raw;
        }
        match 10u64.checked_pow(precision_diff as u32) {
            Some(grid) => {
                let rem = raw % grid;
                (raw / grid + u64::from(rem >= grid - rem)).saturating_mul(grid)
            }
            // Half a grid step is more than any count: every count rounds to 0.
            None => 0,
        }
    }

    /// IEEE 1800-2017 §9.4.5 intra-assignment delay: `lhs = #d rhs` /
    /// `lhs <= #d rhs` is canonicalized at source level (see
    /// `crate::intra_delay`) into `lhs = $__xz_intra_delay(d, rhs)`.
    /// Returns `(delay_expr, rhs_expr)` when `rvalue` is that marker.
    /// §14.16 `cb.out <= ##N rhs` marker: returns (count_expr, rhs).
    pub(in crate::compiler::simulator) fn intra_cycle_marker(
        rvalue: &Expression,
    ) -> Option<(&Expression, &Expression)> {
        match &rvalue.kind {
            ExprKind::SystemCall { name, args }
                if name == crate::intra_delay::INTRA_CYCLE_MARKER && args.len() == 2 =>
            {
                Some((&args[0], &args[1]))
            }
            _ => None,
        }
    }

    /// A blocking assignment's rvalue carries an intra-assignment delay or
    /// event control (the pre-parse markers), so executing it suspends.
    pub(in crate::compiler::simulator) fn intra_timing_suspends(rvalue: &Expression) -> bool {
        Self::intra_delay_marker(rvalue).is_some()
            || matches!(&rvalue.kind, ExprKind::SystemCall { name, args }
                if name == crate::intra_delay::INTRA_EVENT_MARKER && args.len() == 4)
    }

    pub(in crate::compiler::simulator) fn intra_delay_marker(
        rvalue: &Expression,
    ) -> Option<(&Expression, &Expression)> {
        match &rvalue.kind {
            ExprKind::SystemCall { name, args }
                if name == crate::intra_delay::INTRA_DELAY_MARKER && args.len() == 2 =>
            {
                Some((&args[0], &args[1]))
            }
            _ => None,
        }
    }

    /// Freeze a selected RHS before trying the array and scalar capture paths.
    /// Each path then sees the same label even when its selector calls a function.
    pub(in crate::compiler::simulator) fn freeze_array_rhs_selectors(
        &mut self,
        rhs: &Expression,
    ) -> Option<Expression> {
        let rhs = Self::unparen(rhs);
        if (self.module.arrays_2d.is_empty() && self.module.arrays_nd.is_empty())
            || !matches!(rhs.kind, ExprKind::Index { .. })
            || Self::cond_expr_is_effect_free(rhs, false)
        {
            return None;
        }
        // Freeze just select chains on a name. Expressions that produce an
        // array by calling a function keep their own call-result handling.
        let mut base = rhs;
        let mut selects = 0;
        while let ExprKind::Index { expr, .. } = &base.kind {
            selects += 1;
            base = expr;
        }
        let name = match &base.kind {
            ExprKind::Ident(h) if h.path.iter().all(|s| s.selects.is_empty()) => self
                .expr_assoc_name(base)
                .unwrap_or_else(|| self.resolve_hier_name(h).into_owned()),
            ExprKind::MemberAccess { .. } if Self::operand_chain_effect_free(base) => {
                self.expr_assoc_name(base)?
            }
            _ => return None,
        };
        let rank = if self.module.arrays_2d.contains_key(&name) {
            2
        } else {
            self.module.arrays_nd.get(&name)?.0.len()
        };
        (rank > selects).then(|| self.freeze_conditional_path(rhs, true))
    }

    /// §10.10: capture a one-dimensional unpacked array in element order.
    pub(in crate::compiler::simulator) fn capture_array_rhs(
        &mut self,
        lhs: &Expression,
        rhs: &Expression,
    ) -> Option<Vec<Value>> {
        let dst = self.array_operand_name(lhs)?;
        if self.is_associative_array(&dst)
            || self.module.arrays_2d.contains_key(&dst)
            || self.module.arrays_nd.contains_key(&dst)
            || self.queue_elem_struct(&dst).is_some()
        {
            return None;
        }
        let mut rhs = rhs;
        while let ExprKind::Paren(inner) = &rhs.kind {
            rhs = inner;
        }
        // Existing slice capture retains indexed direction and declared bounds.
        if matches!(rhs.kind, ExprKind::RangeSelect { .. }) {
            return None;
        }
        let mut items = Vec::new();
        fn flatten<'a>(e: &'a Expression, out: &mut Vec<&'a Expression>) {
            match &e.kind {
                ExprKind::Concatenation(xs) => {
                    for x in xs {
                        flatten(x, out);
                    }
                }
                ExprKind::Paren(x) => flatten(x, out),
                _ => out.push(e),
            }
        }
        if let ExprKind::AssignmentPattern(xs) = &rhs.kind {
            // Positional patterns include already captured intra-assignment
            // operands. Keyed/default patterns retain their existing path.
            if !xs
                .iter()
                .all(|x| matches!(x, AssignmentPatternItem::Ordered(_)))
            {
                return None;
            }
            // §11.6.1: each item in the element type, as without the delay.
            let shape = self.intra_element_shape(&dst);
            let mut values = Vec::new();
            for x in xs {
                match &x.expr().kind {
                    // §10.9.1 `'{n{a, b}}`: the items, read once, repeated
                    // n times.
                    ExprKind::Replication { count, exprs } => {
                        let n = self.eval_expr(count).to_u64().unwrap_or(0) as usize;
                        let once: Vec<Value> = exprs
                            .iter()
                            .map(|e| self.intra_item_value(shape.as_ref(), e))
                            .collect();
                        for _ in 0..n {
                            values.extend(once.iter().cloned());
                        }
                    }
                    _ => values.push(self.intra_item_value(shape.as_ref(), x.expr())),
                }
            }
            return Some(values);
        }
        if !matches!(
            rhs.kind,
            ExprKind::Concatenation(_) | ExprKind::Call { .. }
        ) && self.array_operand_name(rhs).is_none()
        {
            return None;
        }
        flatten(rhs, &mut items);
        let expand_rows = self.concat_has_non_array_elements(&dst);
        Some(self.unpacked_concat_operand_values(items, expand_rows, None))
    }

    pub(in crate::compiler::simulator) fn saved_array_pattern(
        &mut self,
        values: Vec<Value>,
        span: crate::ast::Span,
    ) -> Expression {
        let items = values
            .into_iter()
            .map(|v| AssignmentPatternItem::Ordered(self.make_intra_saved_expr(v, span)))
            .collect();
        Expression::new(ExprKind::AssignmentPattern(items), span)
    }

    /// §9.4.5: capture the right-hand side of an intra-assignment timing
    /// control when the statement runs, against its target as an
    /// assignment without the timing control would evaluate it: at the
    /// target's width (§11.6.1), and a pattern shaped by a packed target
    /// (§10.9). A pattern for any other target takes its shape from the
    /// target only when it is stored, so each item's value is captured
    /// instead, keys and nesting kept, and the delayed store runs the
    /// pattern of captured values. Returns that right-hand side and the
    /// statement that frees the captured items after the store, if any.
    pub(in crate::compiler::simulator) fn capture_intra_rhs(
        &mut self,
        lvalue: &Expression,
        rhs: &Expression,
        span: crate::ast::Span,
    ) -> (Expression, Option<Statement>) {
        // §9.4.5: capture a computed receiver once before the array
        // probes. Leaf lookup can then use the held handle after the call.
        let rhs = Self::unparen(rhs);
        let frozen = if Self::recv_chain_call(rhs).is_some() {
            Some(self.freeze_conditional_path(rhs, true))
        } else {
            self.freeze_array_rhs_selectors(rhs)
        };
        let rhs = frozen.as_ref().unwrap_or(rhs);
        let mut inner = rhs;
        while let ExprKind::Paren(e) = &inner.kind {
            inner = e;
        }
        // §11.4.11: a conditional of unpacked aggregates (`c ? '{1, 2} :
        // '{3, 4}`) selects its operand now, and that operand is captured.
        // An operand read through a call, or both operands merged under an
        // unknown condition, are already a snapshot taken now.
        if matches!(inner.kind, ExprKind::Conditional { .. })
            && let Some(selected) = self.selected_aggregate_rhs(Some(lvalue), inner)
        {
            if matches!(&selected.kind, ExprKind::Ident(h)
                if h.path.len() == 1 && h.path[0].name.name.starts_with("__xz_cond_aggregate_"))
            {
                return (selected, None);
            }
            return self.capture_intra_rhs(lvalue, &selected, span);
        }
        if let Some(snapshot) = self.delayed_struct_collections_snapshot(inner, span) {
            return snapshot;
        }
        if let Some(values) = self.capture_array_rhs(lvalue, inner) {
            return (self.saved_array_pattern(values, span), None);
        }
        // §9.4.5, §7.6: any other unpacked array (a whole multidimensional
        // array, a subarray or slice of one, an array of unpacked structs, a
        // class property's row) is read element by element now. It was
        // captured as one scalar, so the delayed store wrote zeros.
        if let Some(snapshot) = self.delayed_struct_queue_snapshot(inner, span) {
            return snapshot;
        }
        if let Some(snapshot) = self.unpacked_array_snapshot(inner, span) {
            return snapshot;
        }
        if matches!(inner.kind, ExprKind::AssignmentPattern(_)) {
            // A packed target shapes the pattern now; its value is the one
            // evaluation of the items (a second would repeat their calls).
            if let Some(v) = self.eval_pattern_for_lvalue(lvalue, inner) {
                return (self.make_intra_saved_expr(v, span), None);
            }
            let mut slots = Vec::new();
            let shape = self.intra_target_shape(lvalue);
            let captured = self.capture_intra_pattern(inner, &mut slots, shape.as_ref());
            return (captured, Some(Self::intra_release_stmt(&slots, span)));
        }
        let val = self.eval_target_rhs(lvalue, rhs);
        (self.make_intra_saved_expr(val, span), None)
    }

    /// §7.2.1 and §9.4.5: a struct is sampled with its collection
    /// members, including their lengths, keys and defaults.
    fn delayed_struct_collections_snapshot(
        &mut self,
        expr: &Expression,
        span: crate::ast::Span,
    ) -> Option<(Expression, Option<Statement>)> {
        if !matches!(
            expr.kind,
            ExprKind::Ident(_)
                | ExprKind::MemberAccess { .. }
                | ExprKind::Index { .. }
                | ExprKind::RangeSelect { .. }
        ) || !Self::operand_chain_effect_free(expr)
        {
            return None;
        }
        let mut slots = Vec::new();
        let snapshot = if let Some((array, extents, _)) = self.fixed_array_snapshot_shape(expr) {
            let su = self.queue_elem_struct(&array.root_name)?;
            if !self.struct_has_collection_member(&su) || array.names.iter().any(Option::is_none) {
                return None;
            }
            self.snapshot_struct_rows(&array.names, &extents, &su, span, &mut slots)
        } else {
            let name = self
                .expr_assoc_name(expr)
                .or_else(|| self.class_struct_element_path(expr))
                .or_else(|| {
                    let (handle, prop) = self.class_prop_receiver(expr)?;
                    self.class_prop_struct(handle, &prop)?;
                    Some(format!("{handle}#{prop}"))
                })
                .or_else(|| self.flat_member_name(expr))?;
            let (storage, decl) = self.struct_copy_target(name.clone());
            let su = self
                .queue_elem_struct(&name)
                .or(decl)
                .or_else(|| self.struct_elem_type(expr))?;
            if !self.struct_has_collection_member(&su) {
                return None;
            }
            if let Some(size) = self.live_coll_size(&name) {
                if size > 1 << 20 {
                    return None;
                }
                let dim = UnpackedDimension::Unsized(span);
                return Some((
                    self.snapshot_struct_member(
                        &name,
                        &DataType::Struct(su),
                        &[dim],
                        span,
                        &mut slots,
                    ),
                    Some(Self::intra_release_stmt(&slots, span)),
                ));
            }
            if self.is_associative_array(&name) {
                return None;
            }
            self.snapshot_struct(&storage, &su, span, &mut slots)
        };
        Some((snapshot, Some(Self::intra_release_stmt(&slots, span))))
    }

    fn struct_has_collection_member(&self, su: &StructUnionType) -> bool {
        su.members.iter().any(|member| {
            member.declarators.iter().any(|decl| {
                self.member_dims(&member.data_type, &decl.dimensions)
                    .iter()
                    .any(|dim| Self::coll_kind_of(std::slice::from_ref(dim)).is_some())
            }) || self
                .unpacked_struct_of(&member.data_type)
                .is_some_and(|nested| self.struct_has_collection_member(&nested))
        })
    }

    fn snapshot_struct_rows(
        &mut self,
        names: &[Option<String>],
        extents: &[usize],
        su: &StructUnionType,
        span: crate::ast::Span,
        slots: &mut Vec<u64>,
    ) -> Expression {
        let Some((_, remaining)) = extents.split_first() else {
            return self.snapshot_struct(names[0].as_deref().unwrap(), su, span, slots);
        };
        let stride = remaining.iter().product::<usize>();
        let items = names
            .chunks(stride)
            .map(|row| {
                AssignmentPatternItem::Ordered(
                    self.snapshot_struct_rows(row, remaining, su, span, slots),
                )
            })
            .collect();
        Expression::new(ExprKind::AssignmentPattern(items), span)
    }

    fn snapshot_struct(
        &mut self,
        name: &str,
        su: &StructUnionType,
        span: crate::ast::Span,
        slots: &mut Vec<u64>,
    ) -> Expression {
        let mut items = Vec::new();
        for member in &su.members {
            for decl in &member.declarators {
                let path = format!("{name}.{}", decl.name.name);
                let dims = self.member_dims(&member.data_type, &decl.dimensions);
                let value =
                    self.snapshot_struct_member(&path, &member.data_type, &dims, span, slots);
                items.push(AssignmentPatternItem::Ordered(value));
            }
        }
        Expression::new(ExprKind::AssignmentPattern(items), span)
    }

    fn snapshot_struct_member(
        &mut self,
        name: &str,
        dt: &DataType,
        dims: &[UnpackedDimension],
        span: crate::ast::Span,
        slots: &mut Vec<u64>,
    ) -> Expression {
        let Some((dim, tail)) = dims.split_first() else {
            if let Some(su) = self.unpacked_struct_of(dt) {
                return self.snapshot_struct(name, &su, span, slots);
            }
            let v = self.struct_leaf_value(name).unwrap_or_else(|| {
                let width = super::elaborate::resolve_type_width(
                    dt,
                    Some(&self.module.parameters),
                    Some(&self.module.typedefs),
                );
                self.formal_type_default(dt, width.max(1))
            });
            return self.kept_item(v, span, slots);
        };
        let mut items = Vec::new();
        let dim = std::slice::from_ref(dim);
        match Self::coll_kind_of(dim) {
            Some(StructCollKind::Assoc) => {
                for key in self.assoc_top_level_keys(name) {
                    let key_value = if self.is_string_keyed_array(name) {
                        Expression::new(
                            ExprKind::StringLiteral(Self::name_key_value(&key).to_sv_string()),
                            span,
                        )
                    } else {
                        let key_value = self.assoc_foreach_key_value(name, &key);
                        self.value_method_arg_expr(&key_value)
                    };
                    let value = self.snapshot_struct_member(
                        &format!("{name}[{key}]"),
                        dt,
                        tail,
                        span,
                        slots,
                    );
                    items.push(AssignmentPatternItem::Keyed(key_value, value));
                }
                let default = self.class_assoc_defaults.get(name).cloned().or_else(|| {
                    self.module
                        .assoc_defaults
                        .get(name)
                        .cloned()
                        .map(|e| self.eval_expr(&e))
                });
                if let Some(default) = default {
                    items.push(AssignmentPatternItem::Default(
                        self.value_method_arg_expr(&default),
                    ));
                }
            }
            kind => {
                let indices = if kind.is_some() {
                    (0..self.get_queue_size(name) as i64).collect()
                } else {
                    self.member_dim_indices(dim)
                        .and_then(|mut dims| dims.pop())
                        .unwrap_or_default()
                };
                for index in indices {
                    let value = self.snapshot_struct_member(
                        &format!("{name}[{index}]"),
                        dt,
                        tail,
                        span,
                        slots,
                    );
                    items.push(AssignmentPatternItem::Ordered(value));
                }
            }
        }
        Expression::new(ExprKind::AssignmentPattern(items), span)
    }

    /// §7.6 and §9.4.5: a whole queue or dynamic array of unpacked
    /// structs needs both its live length and each element's field values.
    fn delayed_struct_queue_snapshot(
        &mut self,
        e: &Expression,
        span: crate::ast::Span,
    ) -> Option<(Expression, Option<Statement>)> {
        if !matches!(e.kind, ExprKind::Ident(_) | ExprKind::MemberAccess { .. })
            || !Self::operand_chain_effect_free(e)
        {
            return None;
        }
        let name = self.array_operand_name(e)?;
        let size = self.live_coll_size(&name)?;
        let su = self.queue_elem_struct(&name)?;
        if size == 0 {
            return Some((Expression::new(ExprKind::Concatenation(Vec::new()), span), None));
        }
        if size > 1 << 20 {
            return None;
        }
        let suffixes = self.unpacked_struct_leaf_keys("", &su);
        let mut rows = Vec::new();
        for i in 0..size {
            for (suffix, width, _) in &suffixes {
                if *width > 0 {
                    let value = self
                        .struct_leaf_value(&format!("{name}[{i}]{suffix}"))
                        .unwrap_or_else(|| Value::new(*width));
                    rows.push((format!("[{i}]{suffix}"), value));
                }
            }
        }
        let mut slots = Vec::new();
        let pattern = self.kept_element_pattern(Self::leaf_rows(rows), span, &mut slots);
        Some((pattern, Some(Self::intra_release_stmt(&slots, span))))
    }

    /// The statement that frees the captured pattern items `slots` once the
    /// store that reads them has run.
    fn intra_release_stmt(slots: &[u64], span: crate::ast::Span) -> Statement {
        let args = slots
            .iter()
            .map(|&idx| Self::intra_slot_literal(idx, span))
            .collect();
        Statement::new(
            StatementKind::Expr(Expression::new(
                ExprKind::SystemCall {
                    name: INTRA_RELEASE_MARKER.to_string(),
                    args,
                },
                span,
            )),
            span,
        )
    }

    /// §7.6, §9.4.5: the unpacked array `e` (see `unpacked_array_leaves`)
    /// read now, as an assignment pattern of the captured elements nested
    /// like the array, with the statement that frees them after the store.
    /// None, with nothing read, when `e` names no such array.
    pub(in crate::compiler::simulator) fn unpacked_array_snapshot(
        &mut self,
        e: &Expression,
        span: crate::ast::Span,
    ) -> Option<(Expression, Option<Statement>)> {
        let e = Self::unparen(e);
        if !matches!(
            e.kind,
            ExprKind::Ident(_)
                | ExprKind::Index { .. }
                | ExprKind::RangeSelect { .. }
                | ExprKind::MemberAccess { .. }
        ) {
            return None;
        }
        let leaves = self.unpacked_array_leaves(e)?;
        let rows = Self::leaf_rows(leaves);
        let mut slots = Vec::new();
        let captured = self.kept_element_pattern(rows, span, &mut slots);
        Some((captured, Some(Self::intra_release_stmt(&slots, span))))
    }

    /// Leaf paths (`[1][0]`, `[1].a`) split into one step per dimension or
    /// member, in declared order, so `kept_element_pattern` nests them.
    fn leaf_rows(leaves: Vec<(String, Value)>) -> Vec<(Vec<String>, Value)> {
        leaves
            .into_iter()
            .map(|(suffix, v)| {
                let path = suffix
                    .split(['[', '.'])
                    .filter(|c| !c.is_empty())
                    .map(|c| c.trim_end_matches(']').to_string())
                    .collect();
                (path, v)
            })
            .collect()
    }

    /// The pattern `e` with each item's value captured (`capture_intra_rhs`),
    /// each in the type of the element `shape` matches it to, if known.
    fn capture_intra_pattern(
        &mut self,
        e: &Expression,
        slots: &mut Vec<u64>,
        shape: Option<&super::intra_pattern_shape::ItemShape>,
    ) -> Expression {
        use crate::ast::expr::AssignmentPatternItem as Item;
        match &e.kind {
            ExprKind::Paren(inner) => self.capture_intra_pattern(inner, slots, shape),
            ExprKind::AssignmentPattern(items) => {
                if let Some(super::intra_pattern_shape::ItemShape::Struct(su)) = shape {
                    let members = su.members.iter().map(|m| m.declarators.len()).sum();
                    if let Some(expanded) = self.struct_replication_items(su, items, members) {
                        // §10.9.2: each replicated member takes its own type.
                        // Capturing the repeated expression in only the first
                        // member's type loses wider bits and four-state values.
                        let expanded = expanded
                            .into_iter()
                            .map(|e| Item::Ordered(e.clone()))
                            .collect();
                        let pattern = Expression::new(ExprKind::AssignmentPattern(expanded), e.span);
                        return self.capture_intra_pattern(&pattern, slots, shape);
                    }
                }
                let shapes = match shape {
                    Some(s) => self.intra_item_shapes(s, items),
                    None => Vec::new(),
                };
                let mut out = Vec::with_capacity(items.len());
                for (k, item) in items.iter().enumerate() {
                    let sh = shapes.get(k).and_then(Option::as_ref);
                    out.push(match item {
                        Item::Ordered(x) => Item::Ordered(self.capture_intra_pattern(x, slots, sh)),
                        Item::Named(n, x) => {
                            Item::Named(n.clone(), self.capture_intra_pattern(x, slots, sh))
                        }
                        Item::Typed(t, x) => {
                            Item::Typed(t.clone(), self.capture_intra_pattern(x, slots, sh))
                        }
                        Item::Default(x) => Item::Default(self.capture_intra_pattern(x, slots, sh)),
                        Item::Keyed(k, x) => {
                            Item::Keyed(k.clone(), self.capture_intra_pattern(x, slots, sh))
                        }
                    });
                }
                Expression::new(ExprKind::AssignmentPattern(out), e.span)
            }
            // `'{n{a, b}}`: the count stays, the items are captured, each
            // matched to an element as the replication is.
            ExprKind::Replication { count, exprs } => {
                let mut out = Vec::with_capacity(exprs.len());
                for x in exprs {
                    out.push(self.capture_intra_pattern(x, slots, shape));
                }
                Expression::new(
                    ExprKind::Replication {
                        count: count.clone(),
                        exprs: out,
                    },
                    e.span,
                )
            }
            _ => {
                // §10.9.1: an array matched to a sub-array (`'{wc, wc}`)
                // supplies its elements, read now.
                // A target whose shape is not known here (a class property,
                // a member path) takes an array item the same way: an
                // unpacked array is only ever matched to a sub-array.
                if !matches!(
                    shape,
                    Some(
                        super::intra_pattern_shape::ItemShape::Leaf(_)
                            | super::intra_pattern_shape::ItemShape::Struct(_)
                    )
                ) && let Some(elems) = self.array_item_elements(e)
                {
                    // `[1][0]` or, for an element that is an unpacked
                    // struct, `[1].a`: one path step per dimension or member,
                    // in declared order, so the steps nest positionally.
                    let rows = Self::leaf_rows(elems);
                    return self.kept_element_pattern(rows, e.span, slots);
                }
                // §7.2, §10.9.2: a whole unpacked struct matched to a struct
                // element (`'{sa[1], sa[0]}`) has no value of its own, only
                // member leaves: each is read now, nested like the members.
                // Captured as one value, it filled every member with it. A
                // class property's target has no known shape; an element of
                // its struct array (`'{ks[1], ks[0]}`) is recognized by itself.
                let elem_su = match shape {
                    Some(super::intra_pattern_shape::ItemShape::Struct(su)) => Some(su.clone()),
                    None if matches!(e.kind, ExprKind::Index { .. }) => self.struct_elem_type(e),
                    _ => None,
                };
                if let Some(su) = elem_su
                    && matches!(
                        e.kind,
                        ExprKind::Ident(_) | ExprKind::Index { .. } | ExprKind::MemberAccess { .. }
                    )
                    && Self::cond_expr_is_effect_free(e, false)
                    && !su.members.iter().any(|m| {
                        m.declarators.iter().any(|d| {
                            d.dimensions.iter().any(|dim| {
                                !matches!(
                                    dim,
                                    crate::ast::types::UnpackedDimension::Range { .. }
                                        | crate::ast::types::UnpackedDimension::Expression { .. }
                                )
                            })
                        })
                    })
                {
                    let mut leaves = Vec::new();
                    self.unpacked_struct_leaves("", e, &su, 0, &mut leaves);
                    if !leaves.is_empty() {
                        let values = leaves
                            .into_iter()
                            .map(|(key, leaf, _, _)| (key, self.eval_expr(&leaf)))
                            .collect();
                        return self.kept_element_pattern(Self::leaf_rows(values), e.span, slots);
                    }
                }
                let val = self.intra_item_value(shape, e);
                self.kept_item(val, e.span, slots)
            }
        }
    }

    /// A captured value as an item of the pattern the delayed store runs.
    fn kept_item(
        &mut self,
        val: Value,
        span: crate::ast::Span,
        slots: &mut Vec<u64>,
    ) -> Expression {
        let idx = self.intra_saved_next;
        self.intra_saved_next += 1;
        self.intra_saved.insert(idx, val);
        slots.push(idx);
        Expression::new(
            ExprKind::SystemCall {
                name: INTRA_KEPT_MARKER.to_string(),
                args: vec![Self::intra_slot_literal(idx, span)],
            },
            span,
        )
    }

    /// Captured array elements, each with its index path, as a pattern
    /// nested by dimension in declared order.
    fn kept_element_pattern(
        &mut self,
        rows: Vec<(Vec<String>, Value)>,
        span: crate::ast::Span,
        slots: &mut Vec<u64>,
    ) -> Expression {
        let mut items = Vec::new();
        let mut rows = rows.into_iter().peekable();
        while let Some((mut path, v)) = rows.next() {
            if path.len() <= 1 {
                items.push(AssignmentPatternItem::Ordered(
                    self.kept_item(v, span, slots),
                ));
                continue;
            }
            let head = path.remove(0);
            let mut group = vec![(path, v)];
            while rows.peek().is_some_and(|(p, _)| p.first() == Some(&head)) {
                let Some((mut p, v)) = rows.next() else {
                    break;
                };
                p.remove(0);
                group.push((p, v));
            }
            items.push(AssignmentPatternItem::Ordered(
                self.kept_element_pattern(group, span, slots),
            ));
        }
        Expression::new(ExprKind::AssignmentPattern(items), span)
    }

    pub(in crate::compiler::simulator) fn intra_slot_literal(
        idx: u64,
        span: crate::ast::Span,
    ) -> Expression {
        Expression::new(
            ExprKind::Number(NumberLiteral::Integer {
                size: None,
                signed: false,
                base: NumberBase::Decimal,
                value: idx.to_string(),
                cached_val: crate::ast::cache::LiteralCache::new(None),
            }),
            span,
        )
    }

    /// Stash a §9.4.5 RHS value evaluated at statement-execution time and
    /// synthesize the `$__xz_intra_saved(idx)` expression that retrieves it
    /// when the delayed assignment finally runs — the pre-computed Value
    /// survives process rescheduling this way.
    pub(in crate::compiler::simulator) fn make_intra_saved_expr(
        &mut self,
        val: Value,
        span: crate::ast::Span,
    ) -> Expression {
        let idx = self.intra_saved_next;
        self.intra_saved_next += 1;
        self.intra_saved.insert(idx, val);
        Expression::new(
            ExprKind::SystemCall {
                name: INTRA_SAVED_MARKER.to_string(),
                args: vec![Expression::new(
                    ExprKind::Number(NumberLiteral::Integer {
                        size: None,
                        signed: false,
                        base: NumberBase::Decimal,
                        value: idx.to_string(),
                        cached_val: crate::ast::cache::LiteralCache::new(None),
                    }),
                    span,
                )],
            },
            span,
        )
    }
}
