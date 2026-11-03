//! IEEE 1800-2023 §10.9: assignment patterns in a packed type, executed from
//! their slice plan.

use super::*;
use crate::ast::types::{IntegerVectorType, PackedDimension, TypeName};
use crate::compiler::packed_assignment_pattern_plan::{
    self, PackedPlan, PatternTypeResolver, SliceSrc,
};

impl PatternTypeResolver for Simulator {
    fn typedef_target(&mut self, name: &TypeName) -> Option<DataType> {
        self.pattern_typedef_target(name)
    }

    fn const_i64(&mut self, e: &Expression) -> Option<i64> {
        super::elaborate::const_eval_i64_with_params(e, Some(&self.module.parameters))
            .or_else(|| self.eval_expr(e).to_i64())
    }

    fn is_type_name(&mut self, name: &str) -> bool {
        !self.module.parameters.contains_key(name) && self.lookup_typedef_target(name).is_some()
    }

    fn expr_type(&mut self, e: &Expression) -> Option<DataType> {
        if let Some(t) = packed_assignment_pattern_plan::named_type_of(e) {
            return Some(t);
        }
        match &e.kind {
            ExprKind::Paren(inner) => self.expr_type(inner),
            ExprKind::Call { func, .. } => self.call_return_type(func),
            // A parameter's declared type first (§6.20.2), then a variable's.
            ExprKind::Ident(_) | ExprKind::MemberAccess { .. } => self
                .param_decl_type(e)
                .or_else(|| self.lvalue_pattern_type(e)),
            ExprKind::Index { .. } => self.lvalue_pattern_type(e),
            _ => None,
        }
    }
}

impl Simulator {
    /// The declared type of the parameter `e` names (§6.20.2).
    fn param_decl_type(&self, e: &Expression) -> Option<DataType> {
        let types = &self.module.param_decl_types;
        if let ExprKind::Ident(h) = &e.kind
            && let Some(dt) = types.get(self.resolve_hier_name(h).as_ref())
        {
            return Some(dt.clone());
        }
        types
            .get(&packed_assignment_pattern_plan::scoped_name_key(e)?)
            .cloned()
    }

    /// Give `compiler` the tables a pattern plan resolves its context type
    /// from: declared variable and parameter types, and typedef unpacked
    /// dimensions.
    pub(super) fn set_pattern_type_tables<'a>(
        module: &'a ElaboratedModule,
        compiler: &mut super::bytecode::BytecodeCompiler<'a>,
    ) {
        compiler.set_var_decl_types(&module.var_decl_types);
        compiler.set_typedef_unpacked_dims(&module.typedef_unpacked_dims);
        compiler.set_param_decl_types(&module.param_decl_types);
        compiler.set_port_aliases(&module.port_aliases);
    }

    /// `e` evaluated as the source of an assignment to `dt` (§10.8), when
    /// `e` holds a pattern and `dt` is a packed aggregate. None otherwise, so
    /// the caller keeps its own evaluation.
    pub(super) fn eval_in_packed_type(&mut self, e: &Expression, dt: &DataType) -> Option<Value> {
        if !has_pattern(e) {
            return None;
        }
        let plan = packed_assignment_pattern_plan::packed_plan(e, dt, self)?;
        if plan.uncovered
            && self
                .warned_uncovered_patterns
                .insert((e.span.start, e.span.end))
        {
            eprintln!(
                "[xezim][warning] assignment pattern leaves an element or member uncovered; \
                 it is set to 0 (IEEE 1800-2023 §10.9.1, §10.9.2)"
            );
        }
        Some(self.exec_packed_assignment_pattern_plan(&plan))
    }

    /// The value the type and default keys of `items` give a member of the
    /// packed type `dt` no other key sets (§10.9.1, §10.9.2).
    pub(super) fn eval_default_in_packed_type(
        &mut self,
        items: &[AssignmentPatternItem],
        dt: &DataType,
    ) -> Option<Value> {
        let plan = packed_assignment_pattern_plan::default_plan(items, dt, self)?;
        Some(self.exec_packed_assignment_pattern_plan(&plan))
    }

    /// `T'{...}`: the value a variable of type `T` holds when initialized
    /// with the pattern (§10.9).
    pub(super) fn eval_typed_pattern(&mut self, e: &Expression) -> Value {
        let ExprKind::TypedPattern { ty, pattern } = &e.kind else {
            return self.eval_expr(e);
        };
        if let Some(v) = self.eval_in_packed_type(pattern, ty) {
            return v;
        }
        // An unpacked struct value travels packed, as a struct return does.
        match self.typed_struct_pattern_value(e) {
            Some(v) => v,
            None => self.eval_expr(pattern),
        }
    }

    /// Run a plan: each slice takes its item as an assignment to the
    /// slice's type (§10.7, §6.12.2), over zeros.
    pub(super) fn exec_packed_assignment_pattern_plan(&mut self, plan: &PackedPlan<'_>) -> Value {
        let mut acc = Value::zero(plan.width);
        for s in &plan.slices {
            let v = match &s.src {
                SliceSrc::Expr(e) => {
                    let raw = self.eval_expr_ctx(e, s.width);
                    let v = self.fit_pattern_item_value(raw, s.width);
                    if s.two_state && v.has_xz() {
                        v.to_two_state()
                    } else {
                        v
                    }
                }
                SliceSrc::Const(v) => v.clone(),
                SliceSrc::Repeat { expr, elem, count } => {
                    let raw = self.eval_expr_ctx(expr, *elem);
                    let v = self.fit_pattern_item_value(raw, *elem);
                    let v = if s.two_state && v.has_xz() {
                        v.to_two_state()
                    } else {
                        v
                    };
                    v.replicate(*count as usize)
                }
                SliceSrc::Cond { cond, then, els } => {
                    let c = self.eval_expr(cond);
                    if c.has_unknown() {
                        let t = self.exec_packed_assignment_pattern_plan(then);
                        let e = self.exec_packed_assignment_pattern_plan(els);
                        t.merge_unknown(&e)
                    } else if c.is_true() {
                        self.exec_packed_assignment_pattern_plan(then)
                    } else {
                        self.exec_packed_assignment_pattern_plan(els)
                    }
                }
            };
            acc.copy_bits_from(s.offset as usize, &v, 0, s.width as usize);
        }
        acc.is_signed = plan.signed;
        acc
    }

    /// The target of a named packed type: a class-scope typedef first, then
    /// the design's tables, then a bound type parameter. None for a typedef
    /// with unpacked dimensions (§7.4).
    fn pattern_typedef_target(&self, name: &TypeName) -> Option<DataType> {
        let unpacked = |key: &str| {
            self.typedef_unpacked_dims_of(key)
                .is_some_and(|d| !d.is_empty())
        };
        if name.has_scope()
            && let Some(dt) = super::elaborate::lookup_scoped_type(name, &self.module.typedef_types)
        {
            return (!unpacked(&name.qualified())).then(|| dt.clone());
        }
        if unpacked(&name.name.name) {
            return None;
        }
        self.lookup_typedef_target(&name.name.name).or_else(|| {
            let bound = self.resolve_type_param_binding(&name.name.name)?;
            let bound = bound.trim();
            if unpacked(bound) {
                return None;
            }
            self.module.typedef_types.get(bound).cloned()
        })
    }

    /// Whether a pattern can take the return type `dt`: a packed vector
    /// with dimensions, a packed struct or union, or a named type.
    pub(super) fn ret_may_take_pattern(dt: &DataType) -> bool {
        match dt {
            DataType::IntegerVector { dimensions, .. } | DataType::Implicit { dimensions, .. } => {
                !dimensions.is_empty()
            }
            DataType::Struct(su) => su.packed,
            DataType::Enum(e) => !e.dimensions.is_empty(),
            DataType::TypeReference { .. } => true,
            _ => false,
        }
    }

    /// The `fn_ret_frames` entry of a call of `fd`.
    pub(super) fn return_pattern_type(&mut self, fd: &FunctionDeclaration) -> ReturnPatternType {
        let dt = &fd.return_type;
        if !Self::ret_may_take_pattern(dt) {
            return ReturnPatternType::NotPacked;
        }
        let span = match dt {
            DataType::IntegerVector { span, .. }
            | DataType::Implicit { span, .. }
            | DataType::TypeReference { span, .. } => *span,
            DataType::Struct(su) => su.span,
            DataType::Enum(e) => e.span,
            _ => return ReturnPatternType::NotPacked,
        };
        let name = fd.name.name.name.as_str();
        let entries = self.fn_ret_types.entry((span.start, span.end)).or_default();
        if let Some((_, t)) = entries.iter().find(|(n, _)| n == name) {
            return ReturnPatternType::Func(t.clone());
        }
        let t = std::rc::Rc::new(dt.clone());
        entries.push((name.to_string(), t.clone()));
        ReturnPatternType::Func(t)
    }

    /// The value `return e` yields when `e` holds a pattern (§13.4.1).
    pub(super) fn ret_pattern_value(&mut self, e: &Expression) -> Option<Value> {
        if !has_pattern(e) {
            return None;
        }
        match self.fn_ret_frames.last()?.clone() {
            ReturnPatternType::NotPacked => None,
            ReturnPatternType::Func(dt) => self.eval_in_packed_type(e, &dt),
            ReturnPatternType::Method(m) => match &m.kind {
                crate::ast::decl::ClassMethodKind::Function(f) => {
                    self.eval_in_packed_type(e, &f.return_type)
                }
                _ => None,
            },
        }
    }

    /// The value `e` assigns to the formal `port` when `e` holds a pattern
    /// (§13.5, §10.8).
    pub(super) fn formal_pattern_value(
        &mut self,
        port: &crate::ast::decl::FunctionPort,
        e: &Expression,
    ) -> Option<Value> {
        if !port.dimensions.is_empty() || !has_pattern(e) {
            return None;
        }
        self.eval_in_packed_type(e, &port.data_type)
    }

    /// The declared type of an assignment target, which a pattern assigned to
    /// it takes (§10.8).
    pub(super) fn lvalue_pattern_type(&mut self, lvalue: &Expression) -> Option<DataType> {
        if let Some(access) = self.declared_packed_access(lvalue)
            && access.unpacked == 0
        {
            return Some(self.packed_access_type(access.element_type, &access.dimensions));
        }
        if let Some(dt) = self.member_path_type(lvalue) {
            return Some(dt);
        }
        if let Some(dt) = self.registered_vector_type(lvalue) {
            return Some(dt);
        }
        if let Some(dt) = self.registered_struct_type(lvalue) {
            return Some(dt);
        }
        let (handle, prop) = self.class_prop_pattern_target(lvalue)?;
        // A redeclared base property is stored as `<Class>::<name>` (§8.14).
        let (class, name) = match prop.split_once("::") {
            Some((c, n)) => (c.to_string(), n.to_string()),
            None => (
                self.heap.get(handle)?.as_ref()?.class_name.clone(),
                prop.clone(),
            ),
        };
        if let Some((dt, unpacked)) = self.class_property_decl_type(&class, &name)
            && unpacked.is_empty()
        {
            return Some(dt);
        }
        self.class_prop_struct(handle, &prop)
            .filter(|su| su.packed)
            .map(DataType::Struct)
    }

    /// The value of `rvalue` assigned to `lvalue` when `rvalue` holds a
    /// pattern and `lvalue` has a packed aggregate type (§10.8).
    pub(super) fn eval_pattern_for_lvalue(
        &mut self,
        lvalue: &Expression,
        rvalue: &Expression,
    ) -> Option<Value> {
        if !has_pattern(rvalue) {
            return None;
        }
        let ty = self.lvalue_pattern_type(lvalue)?;
        self.eval_in_packed_type(rvalue, &ty)
    }

    /// The packed vector type of a signal (or an element of an unpacked
    /// array of them) with no declaration record, from its registered
    /// packed dimensions: a plain vector is an array of bits (§10.9.1).
    fn registered_vector_type(&mut self, lvalue: &Expression) -> Option<DataType> {
        let (name, is_elem) = match &lvalue.kind {
            ExprKind::Ident(h) => (self.resolve_hier_name(h).into_owned(), false),
            ExprKind::Index { expr, .. } => match &expr.kind {
                ExprKind::Ident(h) => (self.resolve_hier_name(h).into_owned(), true),
                _ => return None,
            },
            _ => return None,
        };
        let m = &self.module;
        if m.var_decl_types.contains_key(&name)
            || m.packed_struct_fields.contains_key(&name)
            || m.associative_arrays.contains_key(&name)
            || m.dynamic_arrays.contains(&name)
            || m.arrays_2d.contains_key(&name)
            || m.arrays_nd.contains_key(&name)
            || m.string_signals.contains(&name)
            || self.real_signals.contains(&name)
            || m.arrays.contains_key(&name) != is_elem
        {
            return None;
        }
        let total = if is_elem {
            m.arrays.get(&name)?.2
        } else {
            self.lookup_signal_width(&name)?
        };
        let dims = match self.module.packed_full_dims.get(&name) {
            Some(d) if !d.is_empty() => d.clone(),
            _ => vec![(i64::from(total) - 1, 0)],
        };
        let bits: i64 = dims
            .iter()
            .map(|&(l, r)| l.abs_diff(r) as i64 + 1)
            .product();
        if total <= 1 || bits != i64::from(total) {
            return None;
        }
        let logic = DataType::IntegerVector {
            kind: IntegerVectorType::Logic,
            signing: None,
            dimensions: Vec::new(),
            span: crate::ast::Span::dummy(),
        };
        Some(self.packed_access_type(logic, &dims))
    }

    /// The packed struct type of a signal (or a member of one) with no
    /// declaration record, rebuilt from its registered member layout.
    fn registered_struct_type(&mut self, lvalue: &Expression) -> Option<DataType> {
        let flat = self.flat_member_name(lvalue)?;
        if flat.contains('[') || self.module.var_decl_types.contains_key(&flat) {
            return None;
        }
        let mut split = flat.len();
        loop {
            let base = &flat[..split];
            if let Some(layout) = self.module.packed_struct_fields.get(base) {
                let prefix = flat.get(split + 1..).unwrap_or("");
                let full = &self.module.packed_full_dims;
                return packed_assignment_pattern_plan::layout_struct_type(layout, prefix, &|p| {
                    full.get(&format!("{base}.{p}")).cloned()
                });
            }
            split = flat[..split].rfind('.')?;
        }
    }

    /// The packed type of a `declared_packed_access` result: `dims` over
    /// the element type (a struct, or bits of an integral type).
    fn packed_access_type(&self, element: DataType, dims: &[(i64, i64)]) -> DataType {
        let int = |v: i64| Box::new(Self::int_lit_expr(v, crate::ast::Span::dummy()));
        let packed: Vec<PackedDimension> = dims
            .iter()
            .map(|&(l, r)| PackedDimension::Range {
                left: int(l),
                right: int(r),
                span: crate::ast::Span::dummy(),
            })
            .collect();
        match element {
            DataType::Struct(mut su) => {
                su.dimensions = packed;
                DataType::Struct(su)
            }
            DataType::IntegerVector { .. }
            | DataType::Implicit { .. }
            | DataType::IntegerAtom { .. }
            | DataType::Enum(_)
                if !packed.is_empty() =>
            {
                let two_state = super::elaborate::is_type_two_state_resolved(
                    &element,
                    &self.module.typedef_types,
                );
                DataType::IntegerVector {
                    kind: if two_state {
                        IntegerVectorType::Bit
                    } else {
                        IntegerVectorType::Logic
                    },
                    signing: None,
                    dimensions: packed,
                    span: crate::ast::Span::dummy(),
                }
            }
            other => other,
        }
    }

    /// The type of a `base.member...` target whose base is a declared
    /// variable (`s.inner`, `u_inst.s`).
    fn member_path_type(&mut self, lvalue: &Expression) -> Option<DataType> {
        let flat = self.flat_member_name(lvalue)?;
        if flat.contains('[') {
            return None;
        }
        let mut split = flat.len();
        loop {
            split = flat[..split].rfind('.')?;
            if let Some(dt) = packed_assignment_pattern_plan::declared_type(
                &flat[..split],
                &self.module.var_decl_types,
                Some(&self.module.port_aliases),
            )
            .cloned()
            {
                return packed_assignment_pattern_plan::member_path_type(
                    &dt,
                    &flat[split + 1..],
                    self,
                );
            }
        }
    }
}
