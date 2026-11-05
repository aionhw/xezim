//! IEEE 1800-2023 §11.4.11: conditional values retain their aggregate type.

use super::*;

#[derive(Clone, Copy)]
enum AggregateContext<'a> {
    Storage(&'a Expression),
    Declared(&'a DataType, &'a [crate::ast::types::UnpackedDimension]),
}

#[derive(Clone)]
struct AggregateValue {
    data_type: DataType,
    kind: AggregateKind,
}

#[derive(Clone)]
enum AggregateKind {
    Scalar {
        value: Value,
        default: Value,
    },
    Array {
        indices: Vec<i64>,
        values: Vec<AggregateValue>,
        dynamic: bool,
    },
    Struct(Vec<(String, AggregateValue)>),
}

impl AggregateValue {
    fn equivalent(&self, rhs: &Self) -> bool {
        match (&self.kind, &rhs.kind) {
            (AggregateKind::Scalar { value: a, .. }, AggregateKind::Scalar { value: b, .. }) => {
                a.is_equal(b).is_true()
            }
            (AggregateKind::Array { values: a, .. }, AggregateKind::Array { values: b, .. }) => {
                a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.equivalent(b))
            }
            (AggregateKind::Struct(a), AggregateKind::Struct(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b)
                        .all(|((an, a), (bn, b))| an == bn && a.equivalent(b))
            }
            _ => false,
        }
    }

    fn defaulted(mut self) -> Self {
        match &mut self.kind {
            AggregateKind::Scalar { value, default } => *value = default.clone(),
            AggregateKind::Array {
                values,
                indices,
                dynamic,
            } => {
                if *dynamic {
                    values.clear();
                    indices.clear();
                } else {
                    for v in values {
                        *v = v.clone().defaulted();
                    }
                }
            }
            AggregateKind::Struct(fields) => {
                for (_, v) in fields {
                    *v = v.clone().defaulted();
                }
            }
        }
        self
    }

    fn ambiguous(mut self, rhs: Self) -> Self {
        if self.equivalent(&rhs) {
            return self;
        }
        // §11.4.11: aggregate array elements match as WHOLE values; a
        // differing element gets its type's default, not an integral bit merge.
        if let (AggregateKind::Array { values: a, .. }, AggregateKind::Array { values: b, .. }) =
            (&mut self.kind, &rhs.kind)
            && a.len() == b.len()
        {
            for (a, b) in a.iter_mut().zip(b) {
                if !a.equivalent(b) {
                    *a = a.clone().defaulted();
                }
            }
            return self;
        }
        self.defaulted()
    }
}

impl Simulator {
    pub(super) fn is_unpacked_aggregate_operand(&mut self, e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Paren(inner) => return self.is_unpacked_aggregate_operand(inner),
            ExprKind::Conditional {
                then_expr,
                else_expr,
                ..
            } => {
                return self.is_unpacked_aggregate_operand(then_expr)
                    && self.is_unpacked_aggregate_operand(else_expr);
            }
            ExprKind::Call { func, .. } => {
                return self.call_return_type(func).is_some_and(|dt| {
                    self.type_is_unpacked_collection(&dt) || self.unpacked_struct_of(&dt).is_some()
                });
            }
            ExprKind::Ident(_) | ExprKind::MemberAccess { .. } | ExprKind::Index { .. } => {}
            _ => return false,
        }
        // Frame-local scalar/class names hide unrelated collection metadata.
        // Fixed formals have a marker; dynamic locals have a rename entry.
        if let ExprKind::Ident(h) = &e.kind
            && h.path.len() == 1
            && h.path[0].selects.is_empty()
        {
            let name = &h.path[0].name.name;
            let array_formal = format!("\u{1}array_formal:{name}");
            let local_array = self.dyn_name_lookup_innermost(name).is_some()
                || self
                    .local_stack
                    .last()
                    .is_some_and(|f| f.contains_key(&array_formal));
            if !local_array {
                if self.local_class_type_of(name).is_some() {
                    return false;
                }
                if self
                    .local_stack
                    .last()
                    .is_some_and(|f| f.contains_key(name))
                {
                    return self
                        .local_typedef_type_of(name)
                        .and_then(|t| self.module.typedef_types.get(&t))
                        .is_some_and(|dt| self.unpacked_struct_of(dt).is_some());
                }
            }
        }
        // Type probes must not execute selectors in an unselected arm.
        let probe = self.freeze_conditional_path(e, false);
        let e = &probe;
        // Type probes must not execute a receiver call in an unselected arm.
        if Self::recv_chain_call(e).is_some() {
            return false;
        }
        if self.queue_actual_storage(e).is_some() || self.fixed_array_operand(e).is_some() {
            return true;
        }
        if let Some(name) = self.flat_member_name(e) {
            if self.foreach_dims(&name).is_some()
                || self
                    .flat_path_type(&name)
                    .is_some_and(|(_, dims)| dims.is_some())
            {
                return true;
            }
            let (_, su) = self.struct_copy_target(name);
            if su.is_some_and(|s| Self::spreads_member_wise(&s)) {
                return true;
            }
        }
        self.class_prop_struct_of(e).is_some()
    }

    pub(super) fn aggregate_query_needs_value(&mut self, e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Paren(inner) => self.aggregate_query_needs_value(inner),
            ExprKind::Conditional {
                then_expr,
                else_expr,
                ..
            } => {
                self.aggregate_query_needs_value(then_expr)
                    || self.aggregate_query_needs_value(else_expr)
            }
            ExprKind::Call { func, .. } => self
                .call_return_type(func)
                .is_some_and(|dt| self.type_is_unpacked_collection(&dt)),
            _ => {
                let probe = self.freeze_conditional_path(e, false);
                self.queue_actual_storage(&probe)
                    .is_some_and(|n| self.module.dynamic_arrays.contains(&n))
            }
        }
    }

    pub(super) fn selected_aggregate_rhs(
        &mut self,
        target: Option<&Expression>,
        rhs: &Expression,
    ) -> Option<Expression> {
        self.select_aggregate_rhs(target.map(AggregateContext::Storage), rhs)
    }

    pub(super) fn selected_aggregate_formal_rhs(
        &mut self,
        data_type: &DataType,
        dims: &[crate::ast::types::UnpackedDimension],
        rhs: &Expression,
    ) -> Option<Expression> {
        self.select_aggregate_rhs(Some(AggregateContext::Declared(data_type, dims)), rhs)
    }

    fn select_aggregate_rhs(
        &mut self,
        context: Option<AggregateContext<'_>>,
        rhs: &Expression,
    ) -> Option<Expression> {
        let mut rhs = rhs;
        while let ExprKind::Paren(inner) = &rhs.kind {
            rhs = inner;
        }
        let ExprKind::Conditional {
            condition,
            then_expr,
            else_expr,
        } = &rhs.kind
        else {
            return None;
        };
        if !context.is_some_and(|c| match c {
            AggregateContext::Storage(e) => self.is_unpacked_aggregate_operand(e),
            AggregateContext::Declared(_, _) => true,
        }) && !(self.is_unpacked_aggregate_operand(then_expr)
            && self.is_unpacked_aggregate_operand(else_expr))
        {
            return None;
        }
        let selected = match self.eval_expr(condition).is_nonzero() {
            Some(true) => Some(&**then_expr),
            Some(false) => Some(&**else_expr),
            None => None,
        };
        let selected = selected.map(|mut e| {
            while let ExprKind::Paren(inner) = &e.kind {
                e = inner;
            }
            e
        });
        if let Some(selected) = selected
            && !matches!(selected.kind, ExprKind::Call { .. })
        {
            return Some(self.freeze_conditional_path(selected, true));
        }
        // Snapshot the arms in order: either one may call a function or
        // alias storage changed by the other. The predicate runs once.
        let snapshot = if let Some(selected) = selected {
            self.snapshot_aggregate_operand(selected, context)
        } else {
            let a = self.snapshot_aggregate_operand(then_expr, context);
            let b = self.snapshot_aggregate_operand(else_expr, context);
            a.zip(b).map(|(a, b)| a.ambiguous(b))
        };
        let Some(value) = snapshot else {
            // Never fall through after consuming an effectful predicate.
            self.emit_severity_text("Fatal", "Cannot resolve conditional aggregate storage");
            self.fatal_finish_number = Some(1);
            self.finished = true;
            return Some(self.make_intra_saved_expr(Value::new(1), rhs.span));
        };
        let name = format!("__xz_cond_aggregate_{}_{}__", rhs.span.start, rhs.span.end);
        self.store_conditional_aggregate(&name, &value);
        Some(Self::hier_ident_expr(
            vec![crate::ast::expr::HierPathSegment {
                name: crate::ast::Identifier {
                    name,
                    span: rhs.span,
                },
                selects: Vec::new(),
            }],
            rhs.span,
        ))
    }

    /// Freeze path selectors before querying storage or rebuilding member
    /// reads. An ambiguous condition reads each operand once (§11.4.11).
    pub(super) fn freeze_conditional_path(&mut self, e: &Expression, evaluate: bool) -> Expression {
        let literal = |value: Value, span| {
            Expression::new(
                ExprKind::Number(NumberLiteral::Integer {
                    size: Some(value.width.max(1)),
                    signed: value.is_signed,
                    base: NumberBase::Binary,
                    value: (0..value.width.max(1))
                        .rev()
                        .map(|i| match value.get_bit(i as usize) {
                            LogicBit::Zero => '0',
                            LogicBit::One => '1',
                            LogicBit::X => 'x',
                            LogicBit::Z => 'z',
                        })
                        .collect(),
                    cached_val: crate::ast::cache::LiteralCache::new(None),
                }),
                span,
            )
        };
        let kind = match &e.kind {
            ExprKind::Paren(inner) => return self.freeze_conditional_path(inner, evaluate),
            ExprKind::Ident(h) => {
                let mut h = h.clone();
                h.cached_signal_id = crate::ast::cache::SignalCache::new(None);
                h.cached_resolved_name = crate::ast::cache::NameCache::new();
                for seg in &mut h.path {
                    for sel in &mut seg.selects {
                        let value = if evaluate {
                            self.eval_expr(sel)
                        } else {
                            Value::zero(32)
                        };
                        *sel = literal(value, sel.span);
                    }
                }
                ExprKind::Ident(h)
            }
            ExprKind::Index { expr, index } => {
                let expr = self.freeze_conditional_path(expr, evaluate);
                let value = if evaluate {
                    self.eval_expr(index)
                } else {
                    Value::zero(32)
                };
                ExprKind::Index {
                    expr: Box::new(expr),
                    index: Box::new(literal(value, index.span)),
                }
            }
            ExprKind::MemberAccess { expr, member } => {
                let memo = matches!(expr.kind, ExprKind::Call { .. })
                    .then(|| self.recv_call_memo_hit(expr))
                    .flatten();
                let base = if let Some(value) = memo {
                    self.conditional_receiver_expr(value, expr.span)
                } else if matches!(expr.kind, ExprKind::Call { .. }) && evaluate {
                    let value = self.eval_expr(expr);
                    self.conditional_receiver_expr(value, expr.span)
                } else {
                    self.freeze_conditional_path(expr, evaluate)
                };
                ExprKind::MemberAccess {
                    expr: Box::new(base),
                    member: member.clone(),
                }
            }
            _ => return e.clone(),
        };
        Expression::new(kind, e.span)
    }

    fn conditional_receiver_expr(&mut self, value: Value, span: crate::ast::Span) -> Expression {
        // Receiver resolvers use handle variables. Keep the held result in
        // private storage so repeated type/leaf probes share one invocation.
        let handle = value.to_u64().unwrap_or(0) as usize;
        let name = format!("__xz_cond_receiver_{handle}__");
        if let Some(instance) = self.heap.get(handle).and_then(|o| o.as_ref()) {
            self.var_class_types
                .insert(name.clone(), instance.class_name.clone());
        }
        self.signals.insert(name.clone(), value);
        Self::hier_ident_expr(
            vec![crate::ast::expr::HierPathSegment {
                name: crate::ast::Identifier { name, span },
                selects: Vec::new(),
            }],
            span,
        )
    }

    fn snapshot_aggregate_operand(
        &mut self,
        e: &Expression,
        context: Option<AggregateContext<'_>>,
    ) -> Option<AggregateValue> {
        if let ExprKind::Paren(inner) = &e.kind {
            return self.snapshot_aggregate_operand(inner, context);
        }
        if let Some(selected) = self.select_aggregate_rhs(context, e) {
            return self.snapshot_aggregate_operand(&selected, context);
        }
        let frozen = self.freeze_conditional_path(e, true);
        self.snapshot_aggregate_operand_body(&frozen, context)
    }

    fn snapshot_aggregate_operand_body(
        &mut self,
        e: &Expression,
        context: Option<AggregateContext<'_>>,
    ) -> Option<AggregateValue> {
        if let ExprKind::Call { func, .. } = &e.kind {
            let dt = self.call_return_type(func)?;
            self.pending_ret_collection = None;
            let value = self.eval_expr(e);
            if let Some(name) = self.pending_ret_collection.take() {
                let ident = Self::hier_ident_expr(
                    vec![crate::ast::expr::HierPathSegment {
                        name: crate::ast::Identifier {
                            name: name.clone(),
                            span: e.span,
                        },
                        selects: Vec::new(),
                    }],
                    e.span,
                );
                let dims = vec![(0..self.get_queue_size(&name) as i64).collect()];
                return Some(self.snapshot_aggregate_shape(&ident, &dt, &dims, true));
            }
            // Unpacked struct functions use the existing packed transport.
            if let Some(su) = self.unpacked_struct_of(&dt) {
                let name = "__xz_cond_struct_call__";
                self.module.var_decl_types.insert(name.into(), dt.clone());
                self.spread_into_unpacked_struct(name, &su, &value);
                let ident = Self::hier_ident_expr(
                    vec![crate::ast::expr::HierPathSegment {
                        name: crate::ast::Identifier {
                            name: name.into(),
                            span: e.span,
                        },
                        selects: Vec::new(),
                    }],
                    e.span,
                );
                return Some(self.snapshot_aggregate_shape(&ident, &dt, &[], false));
            }
            return None;
        }
        if let ExprKind::AssignmentPattern(items) = &e.kind {
            let declared = match context {
                Some(AggregateContext::Declared(dt, dims)) => Some((dt.clone(), dims.to_vec())),
                Some(AggregateContext::Storage(_)) => None,
                None => None,
            };
            if let Some((dt, dims)) = declared {
                use crate::ast::types::UnpackedDimension as UD;
                let dynamic = matches!(dims.first(), Some(UD::Unsized(_) | UD::Queue { .. }));
                let indices = if dynamic {
                    vec![(0..self.pattern_ordered(items).len() as i64).collect()]
                } else {
                    self.member_dim_indices(&dims).unwrap_or_default()
                };
                return Some(self.snapshot_aggregate_shape(e, &dt, &indices, dynamic));
            }
        }
        // A pattern takes its type from the assignment context. Inspect a
        // harmless path of that type; the real lvalue is evaluated on write.
        let context_probe = context.and_then(|c| match c {
            AggregateContext::Storage(e) => Some(self.freeze_conditional_path(e, false)),
            AggregateContext::Declared(_, _) => None,
        });
        let probe = if matches!(e.kind, ExprKind::AssignmentPattern(_)) {
            context_probe.as_ref()?
        } else {
            e
        };
        let fixed = self.fixed_array_operand(probe);
        let name = self
            .queue_actual_storage(probe)
            .or_else(|| fixed.as_ref().map(|(n, _, _)| n.clone()))
            .or_else(|| self.flat_member_name(probe));
        let dt = name
            .as_deref()
            .and_then(|n| {
                self.p_elem_type(n)
                    .or_else(|| self.class_member_coll_type(n).cloned())
            })
            .or_else(|| self.class_prop_struct_of(probe).map(DataType::Struct))
            .or_else(|| self.struct_elem_type(probe).map(DataType::Struct))?;
        let dynamic = name
            .as_ref()
            .is_some_and(|n| self.module.dynamic_arrays.contains(n));
        let mut dims: Vec<Vec<i64>> = if dynamic {
            let n = if let ExprKind::AssignmentPattern(items) = &e.kind {
                self.pattern_ordered(items).len() as u64
            } else {
                self.get_queue_size(name.as_deref()?)
            };
            vec![(0..n as i64).collect()]
        } else if let Some(dims) = name.as_ref().and_then(|n| {
            self.module
                .unpacked_decl_dims
                .get(n)
                .cloned()
                .or_else(|| self.foreach_dims(n))
        }) {
            dims.into_iter()
                .map(|(l, r)| {
                    if l <= r {
                        (l..=r).collect()
                    } else {
                        (r..=l).rev().collect()
                    }
                })
                .collect()
        } else if let Some((_, lo, hi)) = fixed {
            vec![(lo..=hi).collect()]
        } else {
            Vec::new()
        };
        if dims.is_empty()
            && let Some(n) = &name
            && let Some(d) = self.partial_index_dims(n)
        {
            dims = d.into_iter().map(|(l, r)| (l..=r).collect()).collect();
        }
        Some(self.snapshot_aggregate_shape(e, &dt, &dims, dynamic))
    }

    fn snapshot_aggregate_shape(
        &mut self,
        e: &Expression,
        dt: &DataType,
        dims: &[Vec<i64>],
        dynamic: bool,
    ) -> AggregateValue {
        let kind = if let Some(indices) = dims.first() {
            let pattern = if let ExprKind::AssignmentPattern(items) = &e.kind {
                Some(self.pattern_elems(items, indices))
            } else {
                None
            };
            let mut values = Vec::with_capacity(indices.len());
            for (pos, idx) in indices.iter().enumerate() {
                let elem = match &pattern {
                    Some(items) => items[pos].cloned(),
                    None => Some(Self::index_expr(e, *idx)),
                };
                if let Some(elem) = elem {
                    values.push(self.snapshot_aggregate_shape(&elem, dt, &dims[1..], false));
                }
            }
            AggregateKind::Array {
                indices: indices.clone(),
                values,
                dynamic,
            }
        } else if let Some(su) = self.unpacked_struct_of(dt) {
            let mut fields = Vec::new();
            let bound = if let ExprKind::AssignmentPattern(items) = &e.kind {
                Some(self.bind_struct_pattern_items(&su, items))
            } else {
                None
            };
            let mut pos = 0;
            for member in &su.members {
                for decl in &member.declarators {
                    let child = match &bound {
                        Some(items) => items[pos].3.cloned(),
                        None => Some(Self::struct_member_lvalue(e, &decl.name.name)),
                    };
                    if let Some(child) = child {
                        let dims = self
                            .member_dim_indices(&decl.dimensions)
                            .unwrap_or_default();
                        fields.push((
                            decl.name.name.clone(),
                            self.snapshot_aggregate_shape(&child, &member.data_type, &dims, false),
                        ));
                    }
                    pos += 1;
                }
            }
            AggregateKind::Struct(fields)
        } else {
            let resolved = self.resolve_dt(dt);
            let value = match Self::ret_conv_of(&resolved, &self.module.typedef_types) {
                RetConv::Keep => self.eval_expr(e),
                RetConv::Real => {
                    let raw = self.eval_expr(e);
                    Self::ret_convert(raw, RetConv::Real, 64, None)
                }
                RetConv::Int { two_state } => {
                    let width = super::elaborate::resolve_type_width(
                        &resolved,
                        Some(&self.module.parameters),
                        Some(&self.module.typedefs),
                    )
                    .max(1);
                    let signed = super::elaborate::is_type_signed_resolved(
                        &resolved,
                        &self.module.typedef_types,
                    );
                    let raw = self.eval_expr_typed(e, width, !signed);
                    let value = self.fit_pattern_item_value(raw, width);
                    let mut value = if two_state {
                        value.to_two_state()
                    } else {
                        value
                    };
                    value.is_signed = super::elaborate::is_type_signed_resolved(
                        &resolved,
                        &self.module.typedef_types,
                    );
                    value
                }
            };
            let default = if matches!(
                &resolved,
                DataType::Simple {
                    kind: crate::ast::types::SimpleType::String,
                    ..
                }
            ) {
                Value::from_string("")
            } else if value.is_real {
                Value::from_f64(0.0)
            } else if super::elaborate::is_type_two_state_resolved(dt, &self.module.typedef_types)
                || matches!(&resolved, DataType::TypeReference { name, .. } if self.module.classes.contains_key(&name.name.name))
                || matches!(
                    &resolved,
                    DataType::Simple {
                        kind: crate::ast::types::SimpleType::Chandle,
                        ..
                    }
                )
            {
                Value::zero(value.width)
            } else {
                Value::new(value.width)
            };
            AggregateKind::Scalar { value, default }
        };
        AggregateValue {
            data_type: dt.clone(),
            kind,
        }
    }

    /// The pattern of the typed pattern `e`, `T'{...}`, when `T` is not
    /// packed. An unpacked target has an equivalent type (§6.22.2), so the
    /// items place as an untyped pattern's; a packed `T'{...}` is a value.
    pub(super) fn unpacked_typed_pattern<'e>(
        &mut self,
        e: &'e Expression,
    ) -> Option<&'e Expression> {
        match &e.kind {
            ExprKind::TypedPattern { ty, pattern } if !is_packed(ty, self) => Some(pattern),
            _ => None,
        }
    }

    /// `e`, or its pattern when it is an unpacked typed pattern.
    pub(super) fn unpacked_pattern<'e>(&mut self, e: &'e Expression) -> &'e Expression {
        self.unpacked_typed_pattern(e).unwrap_or(e)
    }

    /// The element type and unpacked dimensions of the unpacked type `ty`
    /// (an unpacked array typedef, or an unpacked struct).
    fn unpacked_type_shape(
        &mut self,
        ty: &DataType,
    ) -> Option<(DataType, Vec<crate::ast::types::UnpackedDimension>)> {
        let mut dims = Vec::new();
        let mut cur = ty.clone();
        for _ in 0..16 {
            let DataType::TypeReference { name, .. } = &cur else {
                break;
            };
            let key = if name.has_scope() {
                name.qualified()
            } else {
                name.name.name.clone()
            };
            let Some(d) = self
                .typedef_unpacked_dims_of(&key)
                .filter(|d| !d.is_empty())
            else {
                break;
            };
            dims.extend(d.iter().cloned());
            let base = if name.has_scope() {
                super::elaborate::lookup_scoped_type(name, &self.module.typedef_types).cloned()
            } else {
                self.lookup_typedef_target(&key)
            };
            cur = base?;
        }
        if dims.is_empty() && self.unpacked_struct_of(&cur).is_none() {
            return None;
        }
        Some((cur, dims))
    }

    /// §10.9: run `f` on storage holding the value of the typed pattern `e`
    /// of an unpacked type `T`, a variable of type `T` initialized with the
    /// pattern, then release it. None for any other expression.
    pub(super) fn with_typed_pattern_storage<R>(
        &mut self,
        e: &Expression,
        f: impl FnOnce(&mut Self, &Expression) -> R,
    ) -> Option<R> {
        let ExprKind::TypedPattern { ty, pattern } = &e.kind else {
            return None;
        };
        let (dt, dims) = self.unpacked_type_shape(ty)?;
        let value = self.snapshot_aggregate_operand_body(
            pattern,
            Some(AggregateContext::Declared(&dt, &dims)),
        )?;
        self.typed_pattern_seq += 1;
        let name = format!("__xz_typed_pattern_{}__", self.typed_pattern_seq);
        self.store_conditional_aggregate(&name, &value);
        let held = Self::hier_ident_expr(
            vec![crate::ast::expr::HierPathSegment {
                name: crate::ast::Identifier {
                    name: name.clone(),
                    span: e.span,
                },
                selects: Vec::new(),
            }],
            e.span,
        );
        let out = f(self, &held);
        self.release_conditional_aggregate(&name, &value);
        self.typed_pattern_seq -= 1;
        Some(out)
    }

    /// The packed transport value of the typed pattern `e` with an unpacked
    /// struct type, as an unpacked struct return carries it.
    pub(super) fn typed_struct_pattern_value(&mut self, e: &Expression) -> Option<Value> {
        let ExprKind::TypedPattern { ty, .. } = &e.kind else {
            return None;
        };
        let su = self.unpacked_struct_of(ty)?;
        self.with_typed_pattern_storage(e, |sim, held| {
            let ExprKind::Ident(h) = &held.kind else {
                return None;
            };
            sim.pack_unpacked_struct(&h.path[0].name.name, &su)
        })?
    }

    /// Remove what `store_conditional_aggregate` stored for `value`.
    fn release_conditional_aggregate(&mut self, name: &str, value: &AggregateValue) {
        self.module.var_decl_types.remove(name);
        self.module.arrays.remove(name);
        self.module.arrays_2d.remove(name);
        self.module.arrays_nd.remove(name);
        self.module.dynamic_arrays.remove(name);
        self.module.unpacked_decl_dims.remove(name);
        self.module.descending_arrays.remove(name);
        match &value.kind {
            AggregateKind::Scalar { .. } => {
                self.signals.remove(name);
            }
            AggregateKind::Struct(fields) => {
                for (field, value) in fields {
                    self.release_conditional_aggregate(&format!("{name}.{field}"), value);
                }
            }
            AggregateKind::Array {
                indices,
                values,
                dynamic,
            } => {
                if *dynamic {
                    self.signals.remove(&Self::name_with_suffix(name, ".size"));
                }
                for (i, child) in indices.iter().zip(values) {
                    self.release_conditional_aggregate(&format!("{name}[{i}]"), child);
                }
            }
        }
    }

    fn store_conditional_aggregate(&mut self, name: &str, value: &AggregateValue) {
        self.module
            .var_decl_types
            .insert(name.into(), value.data_type.clone());
        self.module.arrays.remove(name);
        self.module.arrays_2d.remove(name);
        self.module.arrays_nd.remove(name);
        self.module.dynamic_arrays.remove(name);
        self.module.unpacked_decl_dims.remove(name);
        self.module.descending_arrays.remove(name);
        match &value.kind {
            AggregateKind::Scalar { value, .. } => {
                self.signals.insert(name.into(), value.clone());
            }
            AggregateKind::Struct(fields) => {
                for (field, value) in fields {
                    self.store_conditional_aggregate(&format!("{name}.{field}"), value);
                }
            }
            AggregateKind::Array {
                indices,
                values,
                dynamic,
            } => {
                let width = super::elaborate::resolve_type_width(
                    &value.data_type,
                    Some(&self.module.parameters),
                    Some(&self.module.typedefs),
                )
                .max(1);
                let mut shape = vec![(
                    *indices.first().unwrap_or(&0),
                    *indices.last().unwrap_or(&-1),
                )];
                let mut child = values.first();
                while let Some(AggregateValue {
                    kind:
                        AggregateKind::Array {
                            indices,
                            values,
                            dynamic: false,
                        },
                    ..
                }) = child
                {
                    shape.push((
                        *indices.first().unwrap_or(&0),
                        *indices.last().unwrap_or(&-1),
                    ));
                    child = values.first();
                }
                self.module
                    .unpacked_decl_dims
                    .insert(name.into(), shape.clone());
                if shape[0].0 > shape[0].1 {
                    self.module.descending_arrays.insert(name.into());
                }
                let bounds = |(l, r): (i64, i64)| (l.min(r), l.max(r));
                if *dynamic || shape.len() == 1 {
                    let (lo, hi) = bounds(shape[0]);
                    self.module.arrays.insert(name.into(), (lo, hi, width));
                } else if shape.len() == 2 {
                    self.module
                        .arrays_2d
                        .insert(name.into(), (bounds(shape[0]), bounds(shape[1]), width));
                } else {
                    self.module.arrays_nd.insert(
                        name.into(),
                        (shape.iter().copied().map(bounds).collect(), width),
                    );
                }
                if *dynamic {
                    self.module.dynamic_arrays.insert(name.into());
                    self.set_queue_size(name, values.len() as u64);
                }
                for (i, child) in indices.iter().zip(values) {
                    self.store_conditional_aggregate(&format!("{name}[{i}]"), child);
                }
            }
        }
    }
}
