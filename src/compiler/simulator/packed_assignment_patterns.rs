//! IEEE 1800-2023 §10.9: assignment patterns in a packed type, executed from
//! their slice plan.

use super::*;
use crate::ast::types::{IntegerVectorType, PackedDimension, TypeName};
use crate::compiler::packed_assignment_pattern_plan::{
    self, CachedPackedPlan, CachedSliceSrc, PackedPlan, PatternTypeResolver, SliceSrc,
};

#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) struct PackedPlanCacheKey {
    span_start: u32,
    span_end: u32,
    expr_fp_lo: u64,
    expr_fp_hi: u64,
    dt_fp_lo: u64,
    dt_fp_hi: u64,
    scope_hint: Option<Box<str>>,
    class_scope: Option<Box<str>>,
}

#[derive(Clone, Copy)]
struct DualFp(u64, u64);

impl std::hash::Hasher for DualFp {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut buf = [0u8; 8];
            buf[..chunk.len()].copy_from_slice(chunk);
            self.u64(u64::from_le_bytes(buf));
        }
    }
    #[inline]
    fn write_u64(&mut self, v: u64) {
        self.u64(v);
    }
}

impl DualFp {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325, 0x517c_c1b7_2722_0a95)
    }
    #[inline]
    fn u64(&mut self, v: u64) {
        self.0 = (self.0 ^ v).wrapping_mul(0x1000_0000_01b3);
        self.1 = (self.1.rotate_left(13) ^ v.wrapping_add(0x9e37_79b9_7f4a_7c15))
            .wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
    #[inline]
    fn disc<T>(&mut self, v: &T) {
        use std::hash::Hash;
        std::mem::discriminant(v).hash(self);
    }
    #[inline]
    fn str(&mut self, s: &str) {
        self.u64(s.len() as u64);
        for chunk in s.as_bytes().chunks(8) {
            let mut buf = [0u8; 8];
            buf[..chunk.len()].copy_from_slice(chunk);
            self.u64(u64::from_le_bytes(buf));
        }
    }
    fn expr(&mut self, e: &Expression) {
        self.u64(((e.span.start as u64) << 32) | (e.span.end as u64));
        self.disc(&e.kind);
        match &e.kind {
            ExprKind::AssignmentPattern(items) => {
                self.u64(items.len() as u64);
                for it in items {
                    self.disc(it);
                    match it {
                        AssignmentPatternItem::Ordered(x)
                        | AssignmentPatternItem::Default(x) => self.expr(x),
                        AssignmentPatternItem::Named(id, x) => {
                            self.str(&id.name);
                            self.expr(x);
                        }
                        AssignmentPatternItem::Keyed(k, x) => {
                            self.expr(k);
                            self.expr(x);
                        }
                        AssignmentPatternItem::Typed(t, x) => {
                            self.dt(t);
                            self.expr(x);
                        }
                    }
                }
            }
            ExprKind::TypedPattern { ty, pattern } => {
                self.dt(ty);
                self.expr(pattern);
            }
            ExprKind::Paren(inner) => self.expr(inner),
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.expr(condition);
                self.expr(then_expr);
                self.expr(else_expr);
            }
            ExprKind::Ident(h) => {
                self.u64(h.root.is_some() as u64);
                for s in &h.path {
                    self.str(&s.name.name);
                    self.u64(s.selects.len() as u64);
                }
            }
            ExprKind::Number(n) => {
                self.disc(n);
                match n {
                    NumberLiteral::Integer {
                        size,
                        signed,
                        base,
                        value,
                        ..
                    } => {
                        self.u64(size.unwrap_or(0) as u64);
                        self.u64(*signed as u64);
                        self.disc(base);
                        self.str(value);
                    }
                    NumberLiteral::Real(r) | NumberLiteral::Time(r) => self.u64(r.to_bits()),
                    NumberLiteral::UnbasedUnsized(c) => self.u64(*c as u64),
                }
            }
            ExprKind::StringLiteral(s) => self.str(s),
            ExprKind::TypeLiteral(dt) => self.dt(dt),
            ExprKind::Unary { op, operand } => {
                self.disc(op);
                self.expr(operand);
            }
            ExprKind::Binary { op, left, right } => {
                self.disc(op);
                self.expr(left);
                self.expr(right);
            }
            ExprKind::Concatenation(xs) => {
                self.u64(xs.len() as u64);
                for x in xs {
                    self.expr(x);
                }
            }
            ExprKind::Replication { count, exprs } => {
                self.expr(count);
                self.u64(exprs.len() as u64);
                for x in exprs {
                    self.expr(x);
                }
            }
            ExprKind::Call { func, args } => {
                self.expr(func);
                self.u64(args.len() as u64);
                for a in args {
                    self.expr(a);
                }
            }
            ExprKind::SystemCall { name, args } => {
                self.str(name);
                self.u64(args.len() as u64);
                for a in args {
                    self.expr(a);
                }
            }
            ExprKind::Index { expr, index } => {
                self.expr(expr);
                self.expr(index);
            }
            ExprKind::RangeSelect {
                expr,
                kind,
                left,
                right,
            } => {
                self.disc(kind);
                self.expr(expr);
                self.expr(left);
                self.expr(right);
            }
            ExprKind::NamedArg { name, expr } => {
                self.str(&name.name);
                if let Some(x) = expr {
                    self.expr(x);
                }
            }
            ExprKind::MemberAccess { expr, member } => {
                self.expr(expr);
                self.str(&member.name);
            }
            _ => {}
        }
    }
    fn dt(&mut self, dt: &DataType) {
        self.disc(dt);
        match dt {
            DataType::IntegerVector {
                kind,
                signing,
                dimensions,
                ..
            } => {
                self.disc(kind);
                if let Some(s) = signing {
                    self.disc(s);
                }
                self.dims(dimensions);
            }
            DataType::Implicit {
                signing,
                dimensions,
                ..
            } => {
                if let Some(s) = signing {
                    self.disc(s);
                }
                self.dims(dimensions);
            }
            DataType::IntegerAtom { kind, signing, .. } => {
                self.disc(kind);
                if let Some(s) = signing {
                    self.disc(s);
                }
            }
            DataType::Struct(su) => {
                self.disc(&su.kind);
                self.u64(u64::from(su.packed) | (u64::from(su.tagged) << 1) | (u64::from(su.soft) << 2));
                if let Some(s) = &su.signing {
                    self.disc(s);
                }
                self.dims(&su.dimensions);
                self.u64(su.members.len() as u64);
                for m in &su.members {
                    self.dt(&m.data_type);
                    for d in &m.declarators {
                        self.str(&d.name.name);
                        self.u64(d.dimensions.len() as u64);
                    }
                }
            }
            DataType::Enum(e) => {
                self.dims(&e.dimensions);
                self.u64(e.members.len() as u64);
                if let Some(bt) = &e.base_type {
                    self.dt(bt);
                }
            }
            DataType::TypeReference {
                name,
                dimensions,
                type_args,
                ..
            } => {
                for s in &name.scopes {
                    self.str(&s.name.name);
                }
                self.str(&name.name.name);
                self.dims(dimensions);
                self.u64(type_args.len() as u64);
            }
            DataType::Simple { kind, .. } => {
                self.disc(kind);
            }
            _ => {}
        }
    }
    fn dims(&mut self, dims: &[PackedDimension]) {
        self.u64(dims.len() as u64);
        for d in dims {
            self.disc(d);
            if let PackedDimension::Range { left, right, .. } = d {
                self.expr(left);
                self.expr(right);
            }
        }
    }
}

struct PlanBuildResolver<'a> {
    sim: &'a mut Simulator,
    dynamic: bool,
}

impl PatternTypeResolver for PlanBuildResolver<'_> {
    fn typedef_target(&mut self, name: &TypeName) -> Option<DataType> {
        let (dt, used_param) = self.sim.pattern_typedef_target_tracked(name);
        if used_param {
            self.dynamic = true;
        }
        dt
    }

    fn const_i64(&mut self, e: &Expression) -> Option<i64> {
        if let Some(v) =
            super::elaborate::const_eval_i64_with_params(e, Some(&self.sim.module.parameters))
        {
            return Some(v);
        }
        self.dynamic = true;
        self.sim.eval_expr(e).to_i64()
    }

    fn pattern_index(&mut self, e: &Expression, bounds: (i64, i64)) -> Option<i64> {
        super::elaborate::const_eval_typed_pattern_index(
            e,
            Some(&self.sim.module.parameters),
            bounds,
        )
        .or_else(|| self.const_i64(e))
    }

    fn is_type_name(&mut self, name: &str) -> bool {
        self.sim.resolve_type_key_name(name).is_some()
    }

    fn specialized_type_key(&mut self, e: &Expression) -> Option<DataType> {
        self.dynamic = true;
        self.sim.resolve_specialized_class_typedef_expr(e)
    }

    fn expr_type(&mut self, e: &Expression) -> Option<DataType> {
        self.sim.expr_type(e)
    }
}

impl PatternTypeResolver for Simulator {
    fn typedef_target(&mut self, name: &TypeName) -> Option<DataType> {
        self.pattern_typedef_target(name)
    }

    fn const_i64(&mut self, e: &Expression) -> Option<i64> {
        super::elaborate::const_eval_i64_with_params(e, Some(&self.module.parameters))
            .or_else(|| self.eval_expr(e).to_i64())
    }

    fn pattern_index(&mut self, e: &Expression, bounds: (i64, i64)) -> Option<i64> {
        super::elaborate::const_eval_typed_pattern_index(e, Some(&self.module.parameters), bounds)
            .or_else(|| self.const_i64(e))
    }

    fn is_type_name(&mut self, name: &str) -> bool {
        self.resolve_type_key_name(name).is_some()
    }

    fn specialized_type_key(&mut self, e: &Expression) -> Option<DataType> {
        self.resolve_specialized_class_typedef_expr(e)
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
        if let ExprKind::Ident(h) = &e.kind {
            let name = self.resolve_hier_name(h);
            if let Some(dt) = types.get(name.as_ref()) {
                return Some(dt.clone());
            }
            // A parameter of the running instance is keyed by its scope
            // (`u.N`), a package parameter imported bare by its package
            // (`pkg::P`); the pattern names both bare (§23.10, §26.3).
            if h.path.len() == 1 && h.root.is_none() {
                let bare = h.path[0].name.name.as_str();
                if let Some(hint) = self.name_resolve_hint.borrow().as_deref() {
                    let mut scope = hint;
                    loop {
                        if let Some(dt) = types.get(&format!("{scope}.{bare}")) {
                            return Some(dt.clone());
                        }
                        let Some((outer, _)) = scope.rsplit_once('.') else {
                            break;
                        };
                        scope = outer;
                    }
                }
                if self.module.parameters.contains_key(bare) {
                    let mut found = None;
                    for (k, dt) in types {
                        if k.rsplit_once("::").is_some_and(|(_, n)| n == bare) {
                            if found.is_some() {
                                return None;
                            }
                            found = Some(dt.clone());
                        }
                    }
                    return found;
                }
            }
        }
        // `pkg::P`, as a scoped identifier or a member-access chain.
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
        let warn_uncovered = |sim: &mut Simulator, e: &Expression, uncovered: bool| {
            if uncovered
                && sim
                    .warned_uncovered_patterns
                    .insert((e.span.start as usize, e.span.end as usize))
            {
                eprintln!(
                    "[xezim][warning] assignment pattern leaves an element or member uncovered; \
                     it is set to 0 (IEEE 1800-2023 §10.9.1, §10.9.2)"
                );
            }
        };
        if e.span.start < e.span.end {
            if self.packed_plan_cache_gen != self.type_tables_gen {
                self.packed_plan_cache.clear();
                self.packed_plan_cache_gen = self.type_tables_gen;
            }
            let mut efp = DualFp::new();
            efp.expr(e);
            let mut dfp = DualFp::new();
            dfp.dt(dt);
            let scope_hint = self
                .name_resolve_hint
                .borrow()
                .as_deref()
                .map(Box::<str>::from);
            let class_scope = self
                .class_context_stack
                .last()
                .and_then(|c| c.as_deref())
                .map(Box::<str>::from);
            let key = PackedPlanCacheKey {
                span_start: e.span.start,
                span_end: e.span.end,
                expr_fp_lo: efp.0,
                expr_fp_hi: efp.1,
                dt_fp_lo: dfp.0,
                dt_fp_hi: dfp.1,
                scope_hint,
                class_scope,
            };
            if let Some(hit) = self.packed_plan_cache.get(&key).cloned() {
                let plan = hit?;
                warn_uncovered(self, e, plan.uncovered);
                return Some(self.exec_cached_packed_plan(&plan));
            }
            let mut resolver = PlanBuildResolver {
                sim: self,
                dynamic: false,
            };
            let built = packed_assignment_pattern_plan::packed_plan(e, dt, &mut resolver);
            let is_dynamic = resolver.dynamic;
            if let Some(plan) = built {
                let uncovered = plan.uncovered;
                let cached_arc = (!is_dynamic).then(|| std::sync::Arc::new(plan.to_owned_plan()));
                let val = self.exec_packed_assignment_pattern_plan(&plan);
                if let Some(arc) = cached_arc {
                    if self.packed_plan_cache.len() >= 4096 {
                        self.packed_plan_cache.clear();
                    }
                    self.packed_plan_cache.insert(key, Some(arc));
                }
                warn_uncovered(self, e, uncovered);
                return Some(val);
            }
            if !is_dynamic {
                if self.packed_plan_cache.len() >= 4096 {
                    self.packed_plan_cache.clear();
                }
                self.packed_plan_cache.insert(key, None);
            }
            return None;
        }
        let plan = packed_assignment_pattern_plan::packed_plan(e, dt, self)?;
        let uncovered = plan.uncovered;
        warn_uncovered(self, e, uncovered);
        Some(self.exec_packed_assignment_pattern_plan(&plan))
    }

    /// The value the type and default keys of `items` give a member of the
    /// packed type `dt` no other key sets (§10.9.1, §10.9.2).
    pub(super) fn eval_default_in_packed_type(
        &mut self,
        items: &[AssignmentPatternItem],
        dt: &DataType,
        is_member: &dyn Fn(&str) -> bool,
    ) -> Option<Value> {
        let plan = packed_assignment_pattern_plan::default_plan(items, dt, self, is_member)?;
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

    fn exec_slice_expr(&mut self, e: &Expression, width: u32, two_state: bool) -> Value {
        let raw = self.eval_expr_ctx(e, width);
        let v = self.fit_pattern_item_value(raw, width);
        if two_state && v.has_xz() {
            v.to_two_state()
        } else {
            v
        }
    }

    /// Run a plan: each slice takes its item as an assignment to the
    /// slice's type (§10.7, §6.12.2), over zeros.
    pub(super) fn exec_packed_assignment_pattern_plan(&mut self, plan: &PackedPlan<'_>) -> Value {
        let mut acc = Value::zero(plan.width);
        for s in &plan.slices {
            let v = match &s.src {
                SliceSrc::Expr(e) => self.exec_slice_expr(e, s.width, s.two_state),
                SliceSrc::Const(v) => v.clone(),
                SliceSrc::Repeat { expr, elem, count } => {
                    self.exec_slice_expr(expr, *elem, s.two_state).replicate(*count as usize)
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

    pub(super) fn exec_cached_packed_plan(&mut self, plan: &CachedPackedPlan) -> Value {
        let mut acc = Value::zero(plan.width);
        for s in plan.slices.iter() {
            let v = match &s.src {
                CachedSliceSrc::Expr(e) => self.exec_slice_expr(e, s.width, s.two_state),
                CachedSliceSrc::Const(v) => v.clone(),
                CachedSliceSrc::Repeat { expr, elem, count } => {
                    self.exec_slice_expr(expr, *elem, s.two_state).replicate(*count as usize)
                }
                CachedSliceSrc::Cond { cond, then, els } => {
                    let c = self.eval_expr(cond);
                    if c.has_unknown() {
                        let t = self.exec_cached_packed_plan(then);
                        let e = self.exec_cached_packed_plan(els);
                        t.merge_unknown(&e)
                    } else if c.is_true() {
                        self.exec_cached_packed_plan(then)
                    } else {
                        self.exec_cached_packed_plan(els)
                    }
                }
            };
            acc.copy_bits_from(s.offset as usize, &v, 0, s.width as usize);
        }
        acc.is_signed = plan.signed;
        acc
    }

    fn pattern_typedef_target_tracked(&self, name: &TypeName) -> (Option<DataType>, bool) {
        let unpacked = |key: &str| {
            self.typedef_unpacked_dims_of(key)
                .is_some_and(|d| !d.is_empty())
        };
        if name.has_scope()
            && let Some(dt) = super::elaborate::lookup_scoped_type(name, &self.module.typedef_types)
        {
            return ((!unpacked(&name.qualified())).then(|| dt.clone()), false);
        }
        if unpacked(&name.name.name) {
            return (None, false);
        }
        if let Some((scoped_key, dt)) = self.lookup_hint_scoped_typedef(&name.name.name) {
            return ((!unpacked(&scoped_key)).then_some(dt), false);
        }
        if let Some(dt) = self.lookup_typedef_target(&name.name.name) {
            return (Some(dt), false);
        }
        let dt = self.resolve_type_param_binding(&name.name.name).and_then(|bound| {
            let bound = bound.trim();
            if unpacked(bound) {
                return None;
            }
            self.resolve_type_binding_dt(bound)
        });
        (dt, true)
    }

    /// The target of a named packed type: a class-scope typedef first, then
    /// the design's tables, then a bound type parameter. None for a typedef
    /// with unpacked dimensions (§7.4).
    fn pattern_typedef_target(&self, name: &TypeName) -> Option<DataType> {
        self.pattern_typedef_target_tracked(name).0
    }

    /// Whether a pattern can take the return type `dt`: a packed vector
    /// with dimensions, a packed struct or union, or a named type.
    pub(super) fn ret_may_take_pattern(dt: &DataType) -> bool {
        match dt {
            DataType::IntegerVector { dimensions, .. } | DataType::Implicit { dimensions, .. } => {
                !dimensions.is_empty()
            }
            DataType::Struct(su) => su.packed || su.tagged,
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
        let entries = self
            .fn_ret_types
            .entry((span.start as usize, span.end as usize))
            .or_default();
        if let Some((_, t)) = entries.iter().find(|(n, _)| n == name) {
            return ReturnPatternType::Func(t.clone());
        }
        let t = std::rc::Rc::new(dt.clone());
        entries.push((name.to_string(), t.clone()));
        ReturnPatternType::Func(t)
    }

    /// The value `return e` yields when `e` holds a pattern (§13.4.1).
    /// A packed return type takes the slice plan; an unpacked struct takes
    /// the pattern member by member, as a typed pattern does (§10.9) —
    /// without that route the generic evaluator concatenates the items
    /// into one flat value and the members are lost.
    pub(super) fn ret_pattern_value(&mut self, e: &Expression) -> Option<Value> {
        if !has_pattern(e) {
            return None;
        }
        let dt = self.return_frame_type()?;
        // An unpacked `T'{...}` places its items as the pattern does.
        let e = self.unpacked_pattern(e);
        self.eval_in_packed_type(e, &dt)
            .or_else(|| self.unpacked_struct_pattern_value(e, &dt))
    }

    /// The declared return type of the function or method being executed,
    /// when a pattern can take it (§13.4.1). A class type parameter is
    /// replaced by the type the specialization binds to it (§8.25).
    pub(super) fn return_frame_type(&self) -> Option<DataType> {
        let dt = match self.fn_ret_frames.last()? {
            ReturnPatternType::NotPacked => return None,
            ReturnPatternType::Func(dt) => (**dt).clone(),
            ReturnPatternType::Method(m) => match &m.kind {
                crate::ast::decl::ClassMethodKind::Function(f) => f.return_type.clone(),
                _ => return None,
            },
        };
        let mut dt = dt;
        if let Some(bound) = self.formal_type_param_binding(&dt)
            && self.module.typedef_types.contains_key(&bound)
            && let DataType::TypeReference { name, .. } = &mut dt
        {
            name.name.name = bound;
        }
        Some(dt)
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
        if let Some(dt) = self.decl_pattern_type(lvalue) {
            return Some(dt);
        }
        if let Some(access) = self.declared_packed_access(lvalue)
            && access.unpacked == 0
        {
            return Some(self.packed_access_type(&access.element_type, &access.dimensions));
        }
        if let Some(dt) = self.member_path_type(lvalue) {
            return Some(dt);
        }
        if let Some(dt) = self.aliased_decl_type(lvalue) {
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
            .map(|su| DataType::Struct((*su).clone()))
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

    /// The declared type of a bare name with no record of its own in the
    /// running scope: a port collapsed onto the net it connects (`u.o` on
    /// `port_aliases`, formal to actual) takes that net's declared type
    /// (§23.3.3). Tried under the instance scope first, then bare.
    fn aliased_decl_type(&mut self, lvalue: &Expression) -> Option<DataType> {
        let ExprKind::Ident(h) = &lvalue.kind else {
            return None;
        };
        if h.root.is_some() || h.path.len() != 1 || !h.path[0].selects.is_empty() {
            return None;
        }
        let bare = h.path[0].name.name.as_str();
        let hint = self.name_resolve_hint.borrow().clone();
        let mut names: Vec<String> = Vec::new();
        if let Some(hint) = hint.as_deref() {
            let mut scope = hint;
            loop {
                names.push(format!("{scope}.{bare}"));
                let Some((outer, _)) = scope.rsplit_once('.') else {
                    break;
                };
                scope = outer;
            }
        }
        names.push(bare.to_string());
        let m = &self.module;
        names.iter().find_map(|n| {
            // Only through an alias: a name with its own record was found
            // by `declared_packed_access` already.
            m.port_aliases.contains_key(n).then(|| {
                packed_assignment_pattern_plan::declared_type(
                    n,
                    &m.var_decl_types,
                    Some(&m.port_aliases),
                )
                .cloned()
            })?
        })
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
            Some(d) if !d.is_empty() => d.to_vec(),
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
        Some(self.packed_access_type(&logic, &dims))
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
                    full.get(&format!("{base}.{p}")).map(|d| d.to_vec())
                });
            }
            split = flat[..split].rfind('.')?;
        }
    }

    /// The packed type of a `declared_packed_access` result: `dims` over
    /// the element type (a struct, or bits of an integral type).
    fn packed_access_type(&self, element: &DataType, dims: &[(i64, i64)]) -> DataType {
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
            DataType::Struct(su) => {
                let mut su = su.clone();
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
                    element,
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
            other => other.clone(),
        }
    }

    /// Declared type of a variable (or indexed element/slice of one) when
    /// preserving typedef, signedness, and enum identity.
    fn decl_pattern_type(&mut self, lvalue: &Expression) -> Option<DataType> {
        let mut cur = lvalue;
        let mut index_count = 0usize;
        loop {
            match &cur.kind {
                ExprKind::Paren(inner) => cur = inner,
                ExprKind::Index { expr, .. } => {
                    index_count += 1;
                    cur = expr;
                }
                _ => break,
            }
        }
        let ExprKind::Ident(h) = &cur.kind else {
            return None;
        };
        if !h.path.iter().all(|s| s.selects.is_empty()) {
            return None;
        }
        if (h.path.len() == 1 && self.this_prop_hides(&h.path[0].name.name))
            || (h.path.len() > 1
                && (matches!(h.path[0].name.name.as_str(), "this" | "super")
                    || self.bare_name_is_class_channel(&h.path[0].name.name)
                    || self.hier_parent_handle(h).is_some()))
        {
            return None;
        }
        let name = self
            .frame_bound_bare_name(h)
            .map(str::to_owned)
            .unwrap_or_else(|| self.resolve_hier_name(h).into_owned());
        if self.is_associative_array(&name) {
            return None;
        }
        let dt = packed_assignment_pattern_plan::declared_type(
            &name,
            &self.module.var_decl_types,
            Some(&self.module.port_aliases),
        )?
        .clone();
        let unpacked = if let Some((dims, _)) = self.module.arrays_nd.get(&name) {
            dims.len()
        } else if self.module.arrays_2d.contains_key(&name) {
            2
        } else if self.module.arrays.contains_key(&name)
            || self.module.dynamic_arrays.contains(&name)
            || self.module.queue_vars.contains(&name)
        {
            1
        } else {
            0
        };
        let packed_selects = index_count.checked_sub(unpacked)?;
        let peeled = packed_assignment_pattern_plan::peel_packed_dims(&dt, packed_selects, self)?;
        packed_assignment_pattern_plan::is_packed(&peeled, self).then_some(peeled)
    }

    /// The type of a `base.member...` target whose base is a declared
    /// variable (`s.inner`, `u_inst.s`).
    fn member_path_type(&mut self, lvalue: &Expression) -> Option<DataType> {
        let mut base = lvalue;
        let mut index_count = 0usize;
        loop {
            match &base.kind {
                ExprKind::Paren(inner) => base = inner,
                ExprKind::Index { expr, .. } => {
                    index_count += 1;
                    base = expr;
                }
                _ => break,
            }
        }
        let flat = self.flat_member_name(base)?;
        if flat.contains('[') {
            return None;
        }
        let unpacked = if let Some((dims, _)) = self.module.arrays_nd.get(&flat) {
            dims.len()
        } else if self.module.arrays_2d.contains_key(&flat) {
            2
        } else if self.module.arrays.contains_key(&flat)
            || self.module.dynamic_arrays.contains(&flat)
            || self.module.queue_vars.contains(&flat)
        {
            1
        } else {
            0
        };
        let packed_selects = index_count.checked_sub(unpacked)?;
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
                let mty = packed_assignment_pattern_plan::member_path_type(
                    &dt,
                    &flat[split + 1..],
                    self,
                )?;
                return packed_assignment_pattern_plan::peel_packed_dims(
                    &mty,
                    packed_selects,
                    self,
                );
            }
        }
    }
}
