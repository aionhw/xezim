//! Typedefs, declared types, declaration widths and casts.

use super::*;

impl<'a> BytecodeCompiler<'a> {
    /// §6.18/§26.3: resolve a concrete alias in its declaring scope. A
    /// specialized class scope needs runtime type bindings and is deliberately
    /// left to the existing class/type-parameter path.
    fn runtime_type_param(&self, name: &crate::ast::types::TypeName, scope: Option<&str>) -> bool {
        if name.has_scope() || !self.unresolved_type_names.contains(&name.name.name) {
            return false;
        }
        // An inlined module/package callee uses its own declaration scope,
        // even inside a method whose class has a same-spelled type parameter.
        let context = scope.or_else(|| self.inlining_stack.last().map(String::as_str));
        context.is_none_or(|s| {
            self.method_class.as_deref().is_some_and(|class| {
                s.strip_prefix(class)
                    .is_some_and(|tail| tail.starts_with("::") || tail.starts_with('.'))
            })
        })
    }

    pub(super) fn typedef_entry<'b, T>(
        &self,
        table: &'b HashMap<String, T>,
        name: &crate::ast::types::TypeName,
        scope: Option<&str>,
    ) -> Option<(&'b String, &'b T)> {
        self.typedef_entry_in(table, None, name, scope)
    }

    fn typedef_entry_in<'b, T>(
        &self,
        table: &'b HashMap<String, T>,
        other: Option<&'b HashMap<String, T>>,
        name: &crate::ast::types::TypeName,
        scope: Option<&str>,
    ) -> Option<(&'b String, &'b T)> {
        let get = |key: &str| {
            table
                .get_key_value(key)
                .or_else(|| other.and_then(|t| t.get_key_value(key)))
        };
        if name.has_scope() {
            return get(&name.qualified());
        }
        let bare = name.name.name.as_str();
        if self.runtime_type_param(name, scope) {
            return None;
        }
        let in_scope = |s: &str| {
            // Scope may name an instance, a subroutine, or a package member.
            let mut prefix = s;
            loop {
                if let Some(hit) = get(&format!("{prefix}::{bare}")) {
                    return Some(hit);
                }
                if let Some(hit) = get(&format!("{prefix}.{bare}")) {
                    return Some(hit);
                }
                if let Some((p, _)) = prefix.rsplit_once("::") {
                    prefix = p;
                } else if let Some((p, _)) = prefix.rsplit_once('.') {
                    prefix = p;
                } else {
                    return None;
                }
            }
        };
        if let Some(owner) = scope.or_else(|| self.inlining_stack.last().map(String::as_str)) {
            // A callee/alias belongs to this scope, not its caller's class.
            return in_scope(owner)
                .or_else(|| other.and_then(|t| t.get_key_value(bare)))
                .or_else(|| table.get_key_value(bare));
        }
        self.method_class
            .as_deref()
            .and_then(|s| in_scope(s.strip_prefix("__free_fn__").unwrap_or(s)))
            .or_else(|| self.method_class_parent.as_deref().and_then(in_scope))
            .or_else(|| self.scope_hint.as_deref().and_then(in_scope))
            .or_else(|| get(bare))
    }

    pub(super) fn typedef_type_entry(
        &self,
        name: &crate::ast::types::TypeName,
        scope: Option<&str>,
    ) -> Option<(&String, &crate::ast::types::DataType)> {
        self.typedef_entry_in(&self.local_typedef_types, self.typedef_types, name, scope)
    }

    /// Class aliases with parameter-dependent packed bounds need the live
    /// specialization. Do not newly resolve them using module defaults (or
    /// the unresolved-range width of one) in a compiled local declaration.
    /// §8.25: neither may a method's own `bit [N-1:0]` whose bound names a
    /// class value parameter the compiler cannot fold; its width would fall
    /// to the 32-bit default instead of the receiver's specialization.
    pub(super) fn class_alias_dynamic_width(&self, dt: &crate::ast::types::DataType) -> bool {
        use crate::ast::types::{DataType as D, PackedDimension as P};
        let mut cur = dt;
        let mut owner = None;
        let mut class_alias = false;
        for _ in 0..64 {
            match cur {
                D::TypeReference {
                    name, type_args, ..
                } => {
                    if !type_args.is_empty() || name.has_specialized_scope() {
                        return false;
                    }
                    let Some((key, next)) = self.typedef_type_entry(name, owner) else {
                        return false;
                    };
                    class_alias |= self.local_typedef_types.contains_key(key);
                    cur = next;
                    owner = Some(key);
                }
                D::IntegerVector { dimensions, .. } | D::Implicit { dimensions, .. } => {
                    if !class_alias {
                        return self.method_mode
                            && owner.is_none()
                            && dimensions.iter().any(|dim| match dim {
                                P::Range { left, right, .. } => {
                                    self.eval_const_expr(left).is_none()
                                        || self.eval_const_expr(right).is_none()
                                }
                                P::Unsized(_) => false,
                            });
                    }
                    return dimensions.iter().any(|dim| match dim {
                        P::Range { left, right, .. } => {
                            !matches!(left.kind, ExprKind::Number(_))
                                || !matches!(right.kind, ExprKind::Number(_))
                        }
                        P::Unsized(_) => true,
                    });
                }
                _ => return false,
            }
        }
        false
    }

    pub(super) fn resolved_decl_type<'b>(
        &'b self,
        dt: &'b crate::ast::types::DataType,
        scope: Option<&str>,
    ) -> &'b crate::ast::types::DataType {
        use crate::ast::types::DataType;
        let mut cur = dt;
        let mut owner = scope;
        for _ in 0..64 {
            let DataType::TypeReference {
                name, type_args, ..
            } = cur
            else {
                return cur;
            };
            if !type_args.is_empty() || name.has_specialized_scope() {
                return dt;
            }
            let Some((key, next)) = self.typedef_type_entry(name, owner) else {
                // Preserve unresolved class handles and type parameters.
                return dt;
            };
            cur = next;
            owner = Some(key);
        }
        dt // A cyclic alias must not acquire a concrete local kind.
    }

    /// §6.24.1: whether the named cast `T'(x)` (`pkg::T'(x)`) targets a
    /// typedef of a real type, so the cast is a real operand of its context.
    pub(super) fn named_cast_is_real(&self, args: &[Expression]) -> bool {
        let (scope, leaf) = match args.first().map(|a| &a.kind) {
            Some(ExprKind::Ident(h)) if h.path.len() >= 2 => {
                let n = h.path.len();
                (
                    Some(h.path[n - 2].name.name.as_str()),
                    h.path[n - 1].name.name.as_str(),
                )
            }
            Some(ExprKind::Ident(h)) if h.path.len() == 1 => (None, h.path[0].name.name.as_str()),
            Some(ExprKind::MemberAccess { expr, member }) => match &expr.kind {
                ExprKind::Ident(h) => (
                    h.path.last().map(|s| s.name.name.as_str()),
                    member.name.as_str(),
                ),
                _ => return false,
            },
            _ => return false,
        };
        let get = |key: &str| {
            self.local_typedef_types
                .get(key)
                .or_else(|| self.typedef_types.and_then(|t| t.get(key)))
        };
        scope
            .and_then(|s| get(&format!("{s}::{leaf}")))
            .or_else(|| get(leaf))
            .is_some_and(|dt| self.decl_is_real(dt, None))
    }

    pub(super) fn decl_is_shortreal(
        &self,
        dt: &crate::ast::types::DataType,
        scope: Option<&str>,
    ) -> bool {
        matches!(self.resolved_decl_type(dt, scope), crate::ast::types::DataType::Real {
            kind: crate::ast::types::RealType::ShortReal, ..
        })
    }

    pub(super) fn decl_is_real(
        &self,
        dt: &crate::ast::types::DataType,
        scope: Option<&str>,
    ) -> bool {
        crate::compiler::elaborate::is_type_real(self.resolved_decl_type(dt, scope))
    }

    pub(super) fn decl_is_string(
        &self,
        dt: &crate::ast::types::DataType,
        scope: Option<&str>,
    ) -> bool {
        matches!(
            self.resolved_decl_type(dt, scope),
            crate::ast::types::DataType::Simple {
                kind: crate::ast::types::SimpleType::String,
                ..
            }
        )
    }

    /// §6.24.3: keyword and named casts to a scalar string use the same
    /// conversion, including aliases in the callee's lexical scope.
    pub(super) fn cast_arg_targets_string(&self, args: &[Expression]) -> bool {
        use crate::ast::types::{TypeName, TypeScope};
        let Some(arg) = args.first() else {
            return false;
        };
        if let ExprKind::TypeLiteral(dt) = &arg.kind {
            return self.decl_is_string(dt, None);
        }
        fn names(e: &Expression, out: &mut Vec<crate::ast::Identifier>) -> bool {
            match &e.kind {
                ExprKind::Ident(h)
                    if h.root.is_none() && h.path.iter().all(|p| p.selects.is_empty()) =>
                {
                    out.extend(h.path.iter().map(|p| p.name.clone()));
                    true
                }
                ExprKind::MemberAccess { expr, member } => {
                    if !names(expr, out) {
                        return false;
                    }
                    out.push(member.clone());
                    true
                }
                ExprKind::Paren(inner) => names(inner, out),
                _ => false,
            }
        }
        let mut parts = Vec::new();
        if !names(arg, &mut parts) {
            return false;
        }
        let Some(name) = parts.pop() else {
            return false;
        };
        let name = TypeName {
            scopes: parts
                .into_iter()
                .map(|name| TypeScope {
                    name,
                    type_args: Vec::new(),
                })
                .collect(),
            name,
            span: arg.span,
            bound: Default::default(),
        };
        if self
            .typedef_entry(&self.typedef_coll_dims, &name, None)
            .is_some_and(|(_, dims)| !dims.is_empty())
        {
            return false;
        }
        self.typedef_type_entry(&name, None)
            .is_some_and(|(key, dt)| self.decl_is_string(dt, Some(key)))
    }

    /// §13.4.1 / §6.8: the initial value of a variable of `dt` — x for a
    /// 4-state type, 0 for a 2-state one. Getting this wrong for an inlined
    /// function's return variable makes a partially-assigned function return 0
    /// where it must return x.
    pub(super) fn type_default_value(&self, dt: &crate::ast::types::DataType, w: u32) -> Value {
        self.type_default_value_in(dt, w, None)
    }

    pub(super) fn type_default_value_in(
        &self,
        dt: &crate::ast::types::DataType,
        w: u32,
        scope: Option<&str>,
    ) -> Value {
        let dt = self.resolved_decl_type(dt, scope);
        if matches!(
            dt,
            crate::ast::types::DataType::Simple {
                kind: crate::ast::types::SimpleType::String,
                ..
            }
        ) {
            // §6.16: a string variable initializes to "" — and its Value
            // must never be resized to the 1024-bit placeholder width.
            return Value::from_string("");
        }
        let w = if w > 0 { w } else { 32 };
        let mut value = if crate::compiler::elaborate::is_type_two_state(dt) {
            Value::zero(w)
        } else {
            Value::new(w)
        };
        value.is_signed = crate::compiler::elaborate::is_type_signed(dt);
        value
    }

    /// Declared width of a block-local variable's data type, resolved against
    /// the module's parameters/typedefs (0 when unknown, meaning "leave as-is").
    /// §13.5.2: a formal that omits its type inherits the PREVIOUS formal's.
    /// The parser leaves such a port's data_type Implicit, and sizing it
    /// directly gives 1 bit — `input logic [7:0] a0, a1` bound a1 one bit
    /// wide and truncated every argument passed through it.
    pub(super) fn port_effective_type(
        ports: &[crate::ast::decl::FunctionPort],
        i: usize,
    ) -> &crate::ast::types::DataType {
        for k in (0..=i).rev() {
            let dt = &ports[k].data_type;
            if !matches!(dt, crate::ast::types::DataType::Implicit { dimensions, .. } if dimensions.is_empty())
            {
                return dt;
            }
        }
        &ports[i].data_type
    }

    /// The context width of the actual of formal `i` of a call the compiled
    /// code delegates to the interpreter (an import, an impure function):
    /// the formal's width where the interpreter binds at it
    /// (`Simulator::actual_ctx_fixed`; for an import also a typedef of a
    /// fixed-width integral type, `Simulator::dpi_typedef_ctx_type`), else 0
    /// (self-determined).
    pub(super) fn actual_ctx_width_of(
        &self,
        ports: &[crate::ast::decl::FunctionPort],
        i: usize,
        callee: &str,
        import: bool,
    ) -> u32 {
        use crate::compiler::simulator::Simulator;
        let Some(p) = ports.get(i) else {
            return 0;
        };
        if Simulator::actual_ctx_fixed(p) {
            return self.port_effective_width(ports, i, Some(callee));
        }
        match self.typedef_types {
            Some(tt) if import => Simulator::dpi_typedef_ctx_type(p, tt)
                .map_or(0, |dt| self.decl_width_in(&dt, Some(callee))),
            _ => 0,
        }
    }

    pub(super) fn port_effective_width(
        &self,
        ports: &[crate::ast::decl::FunctionPort],
        i: usize,
        scope: Option<&str>,
    ) -> u32 {
        for k in (0..=i).rev() {
            let dt = &ports[k].data_type;
            if !matches!(dt, crate::ast::types::DataType::Implicit { dimensions, .. } if dimensions.is_empty())
            {
                return self.decl_width_in(dt, scope);
            }
        }
        self.decl_width_in(&ports[i].data_type, scope)
    }

    pub(super) fn decl_width(&self, dt: &crate::ast::types::DataType) -> u32 {
        self.decl_width_in(dt, None)
    }

    /// Declared width with INSTANCE-SCOPED typedef fallback. Elaboration keys
    /// a submodule's typedef as `<inst>.<name>` while a declaration inside an
    /// inlined body still spells the bare name; the bare miss used to fall to
    /// the 32-bit default, truncating every wide typedef'd local. `scope` is
    /// the inlined subroutine's qualified name (`d.conv`) when the caller has
    /// it; enclosing inline frames are tried otherwise.
    pub(super) fn decl_width_in(
        &self,
        dt: &crate::ast::types::DataType,
        scope: Option<&str>,
    ) -> u32 {
        use crate::ast::types::DataType;
        let inst_scope = self.scope_hint.as_deref().or(scope);
        let mut cur = dt;
        let mut owner = scope;
        let mut factor = 1u32;
        for _ in 0..64 {
            let DataType::TypeReference {
                name,
                dimensions,
                type_args,
                span,
            } = cur
            else {
                return factor.saturating_mul(self.decl_width_with_arrays(cur, inst_scope));
            };
            if !type_args.is_empty() || name.has_specialized_scope() {
                break;
            }
            // The width table already includes the target alias's dimensions;
            // only the dimensions attached to this reference are additional.
            let dims = DataType::Implicit {
                signing: None,
                dimensions: dimensions.clone(),
                span: *span,
            };
            factor = factor.saturating_mul(self.decl_width_with_arrays(&dims, inst_scope));
            if self.runtime_type_param(name, owner) {
                return crate::compiler::elaborate::resolve_type_width(dt, self.params, None);
            }
            if let Some((key, next)) = self.typedef_type_entry(name, owner) {
                if let Some(&width) = self.typedefs.and_then(|t| t.get(key)) {
                    return factor.saturating_mul(width);
                }
                cur = next;
                owner = Some(key);
            } else if let Some((_, &width)) = self
                .typedefs
                .and_then(|t| self.typedef_entry(t, name, owner))
            {
                return factor.saturating_mul(width);
            } else {
                break;
            }
        }
        self.decl_width_with_arrays(dt, inst_scope)
    }

    fn decl_width_with_arrays(&self, dt: &crate::ast::types::DataType, scope: Option<&str>) -> u32 {
        use crate::compiler::scoped_arrays;
        let mut view = HashMap::default();
        for arg in scoped_arrays::query_operands(dt) {
            let Some(name) = scoped_arrays::operand_name(arg) else {
                continue;
            };
            let shape = if let Some(local) = self.local_var_array.get(&name) {
                Some((
                    local.lo,
                    local.lo + local.regs.len() as i64 - 1,
                    local.elem_w,
                ))
            } else if let Some(&LocalArrayBank { width, len, lo, .. }) =
                self.local_array_regs.get(&name)
            {
                Some((lo, lo + len as i64 - 1, width))
            } else {
                scope
                    .and_then(|scope| self.arrays.get(&cat_dot(scope, &name)))
                    .or_else(|| self.arrays.get(&name))
                    .or_else(|| self.arrays.get(&name.replace('.', "::")))
                    .copied()
            };
            if let Some(shape) = shape {
                view.insert(name, shape);
            }
        }
        scoped_arrays::resolve_type_width(dt, self.params, self.typedefs, &view)
    }

    /// §7.4.1: is the outermost packed dimension of `dt` other than the
    /// normalized `[N-1:0]` (`u8_t [0:3]`, `logic [1:4][7:0]`)? A
    /// register-backed local's element select slices `[i*ew +: ew]`, which
    /// addresses only that shape: an ascending local came back reversed.
    pub(super) fn decl_outer_dim_unnormalized(&self, dt: &crate::ast::types::DataType) -> bool {
        let (Some(p), Some(tt)) = (self.params, self.typedef_types) else {
            return false;
        };
        crate::compiler::elaborate::packed_full_dims_of(dt, p)
            .or_else(|| crate::compiler::elaborate::packed_full_dims_chained(dt, p, tt))
            .and_then(|d| d.first().copied())
            .is_some_and(|(l, r)| l < r || l.min(r) != 0)
    }

    /// Packed ELEMENT width of a declared type, for register-backed locals:
    /// direct packed-of-packed declarations resolve structurally; a bare
    /// typedef name resolves through the elaborator's typedef elem table.
    pub(super) fn decl_elem_width(&self, dt: &crate::ast::types::DataType) -> Option<u32> {
        self.decl_elem_width_in(dt, None)
    }

    pub(super) fn decl_elem_width_in(
        &self,
        dt: &crate::ast::types::DataType,
        scope: Option<&str>,
    ) -> Option<u32> {
        if let (Some(p), Some(t)) = (self.params, self.typedefs) {
            if let Some(ew) = crate::compiler::elaborate::packed_inner_elem_width(dt, p, t) {
                return Some(ew);
            }
        }
        if let crate::ast::types::DataType::TypeReference {
            name, dimensions, ..
        } = dt
        {
            if dimensions.is_empty() {
                let m = self.typedef_elems?;
                return self.typedef_entry(m, name, scope).map(|(_, &width)| width);
            }
        }
        None
    }

    /// class-perf Step 9c: lower `$cast(dest, src)` — the function form at
    /// expression position, or the §8.16 task form at statement position
    /// when `stmt_form` (the task form prints a runtime error on failure;
    /// the function form is silent). Admission — dest must be one of:
    ///   * a bare register-backed formal / top-level local
    ///     (`cast_reg_ok_locals`) — assigned into its VM register with the
    ///     frame arm's fit;
    ///   * a dotted class member off a handle chain
    ///     (`method_handle_chain_ok`) — `StoreClassMember` semantics;
    ///   * any other BARE name (module-scope variable, `this`-member,
    ///     static cell) — `assign_value` on the carried AST.
    /// Everything else (indexed dests, nested-block locals, compound
    /// expressions) declines; in method mode a decline bails the whole
    /// method (all-or-nothing). Returns the result register (1 = success,
    /// 0 = failure, 32-bit before the context Resize).
    pub(super) fn compile_cast_call(
        &mut self,
        args: &[Expression],
        ctx_width: u32,
        stmt_form: bool,
    ) -> Option<RegId> {
        if args.len() != 2 {
            return None;
        }
        let mut decl_class: Option<Box<str>> = None;
        let dest = match &args[0].kind {
            ExprKind::Ident(h)
                if h.root.is_none() && h.path.len() == 1 && h.path[0].selects.is_empty() =>
            {
                let name = h.path[0].name.name.as_str();
                if let Some(&(r, _)) = self.local_var_regs.get(name) {
                    if !self.cast_reg_ok_locals.contains(name) {
                        // A NESTED-block local: register-backed, but its
                        // declared type was never recorded in the runtime
                        // overlay (only formals and top-level decls are).
                        // class-perf P2: T1's typed-local map now knows the
                        // declared class at any nesting depth — thread it
                        // through CastInsn so `cast_type_ok` uses it instead
                        // of the overlay lookup. Without a known class the
                        // old refusal stands (stale cross-frame risk).
                        match self.method_handle_local_types.get(name) {
                            Some(c) if !c.contains('#') => {
                                decl_class = Some(c.as_str().to_owned().into_boxed_str())
                            }
                            _ => return None,
                        }
                    }
                    CastDest::Reg(r)
                } else {
                    // Not register-backed: `assign_value` resolves bare
                    // names through the live funnel at exec — module
                    // tables, then the frame (absent here), then the
                    // `this` object's property map, then statics.
                    CastDest::Scope
                }
            }
            ExprKind::MemberAccess { expr: base, member } if self.method_handle_chain_ok(base) => {
                if !self.method_instance_field_ok(base, &member.name) {
                    self.bail("cast_receiver_field");
                    return None;
                }
                // Same store gate as the BlockingAssign member arm: a
                // member the heap cannot round-trip refuses.
                if self.class_shadow_names.contains(member.name.as_str())
                    && !self.member_safe_names.contains(member.name.as_str())
                {
                    self.bail("cast_member_shadow");
                    return None;
                }
                let handle_reg = match self.method_member_handle_reg(base) {
                    Some(r) => r,
                    None => match self.compile_expr(base, 0) {
                        Some(r) => r,
                        None => return None,
                    },
                };
                CastDest::Member(handle_reg, member.name.clone().into_boxed_str())
            }
            _ => return None,
        };
        let src_reg = self.compile_expr(&args[1], 0)?;
        let out = self.alloc_reg();
        self.emit(Insn::Cast(
            Box::new(CastInsn {
                dest,
                expr: Box::new(args[0].clone()),
                stmt_form,
                decl_class,
            }),
            src_reg,
            out,
        ));
        if ctx_width > 0 {
            self.emit(Insn::Resize(out, ctx_width));
        }
        Some(out)
    }
}
