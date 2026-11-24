//! Plain identifier and member reads, plain compares, receiver-call memoization and the
//! hierarchical parent handle.

use super::*;

impl Simulator {
    /// §23.6: whether `h` is `$root.top.a[i].m`, a name that selects a member
    /// after an element select, whose flat key names no storage. A generate
    /// block or instance-array element (`$root.top.g[1].x`) is a scope whose
    /// constant index is part of that key, and keeps the name resolver's path.
    /// Nothing is evaluated or built.
    pub(in crate::compiler::simulator) fn root_member_select_shape(
        &self,
        h: &HierarchicalIdentifier,
    ) -> bool {
        h.path.len() >= 3
            && h.path[0].name.name == "$root"
            && h.path[..h.path.len() - 1]
                .iter()
                .any(|s| !s.selects.is_empty())
            && !self.unit_scope_decl_exists(&self.resolve_hier_name(h))
    }

    /// Whether any select of `h` has a side effect (`i++`, a call).
    pub(in crate::compiler::simulator) fn hier_has_effect_select(
        h: &HierarchicalIdentifier,
    ) -> bool {
        h.path.iter().any(|s| {
            s.selects
                .iter()
                .any(|e| !Self::cond_expr_is_effect_free(e, false))
        })
    }

    /// The dotted names of `h` (without `$root`) up to its first select as
    /// one identifier, that segment's selects, and the segments after it.
    pub(in crate::compiler::simulator) fn root_member_parts(
        h: &HierarchicalIdentifier,
    ) -> Option<(
        HierarchicalIdentifier,
        Vec<Expression>,
        Vec<crate::ast::expr::HierPathSegment>,
    )> {
        let first = h.path.iter().position(|s| !s.selects.is_empty())?;
        let mut base = h.clone();
        let rest = base.path.split_off(first + 1);
        base.path.remove(0);
        let selects = std::mem::take(&mut base.path.last_mut()?.selects);
        base.cached_signal_id = crate::ast::cache::SignalCache::new(None);
        base.cached_resolved_name = crate::ast::cache::NameCache::new();
        base.bound = crate::ast::binding::Bound::UNSET;
        Some((base, selects, rest))
    }

    /// A name of `root_member_select_shape` in the shape its relative
    /// spelling `top.a[i].m` has after elaboration: the dotted names up to
    /// the first select as one identifier (`top.a`), then the selects and the
    /// later members (`MemberAccess { Index { Ident, i }, m }`). The member
    /// and select arms read a class property through a queue or array element
    /// (§7.10, §8.4) and a field of a packed-struct element (§7.2.1) from
    /// that shape only; as one identifier its flat key named no storage and
    /// the read was 0.
    ///
    /// The chain's readers may look its selects up more than once. For a
    /// `read`, a select with a side effect (`objs[i++]`, `objs[next()]`) is
    /// made a literal of its value, evaluated here once, left to right
    /// (§11.4.2, §13.4.1). For sizing it is not evaluated: the first select
    /// of the collection becomes `probe`, any other 0.
    #[cold]
    #[inline(never)]
    pub(in crate::compiler::simulator) fn root_member_select_chain(
        &mut self,
        h: &HierarchicalIdentifier,
        span: crate::ast::Span,
        read: bool,
        probe: i64,
    ) -> Option<Expression> {
        if !self.root_member_select_shape(h) {
            return None;
        }
        let (base, selects, rest) = Self::root_member_parts(h)?;
        let mut chain = Expression::new(ExprKind::Ident(base), span);
        let index = |me: &mut Self, chain: Expression, sel: &Expression, probe: i64| {
            let sel = if Self::cond_expr_is_effect_free(sel, false) {
                sel.clone()
            } else if read {
                me.capture_index_literal(sel, &chain)
            } else {
                Expression::new(
                    ExprKind::Number(NumberLiteral::Integer {
                        size: None,
                        signed: true,
                        base: NumberBase::Decimal,
                        value: probe.to_string(),
                        cached_val: crate::ast::cache::LiteralCache::new(None),
                    }),
                    sel.span,
                )
            };
            Expression::new(
                ExprKind::Index {
                    expr: Box::new(chain),
                    index: Box::new(sel),
                },
                span,
            )
        };
        for (k, sel) in selects.iter().enumerate() {
            chain = index(self, chain, sel, if k == 0 { probe } else { 0 });
        }
        for seg in &rest {
            chain = Expression::new(
                ExprKind::MemberAccess {
                    expr: Box::new(chain),
                    member: seg.name.clone(),
                },
                span,
            );
            for sel in &seg.selects {
                chain = index(self, chain, sel, 0);
            }
        }
        Some(chain)
    }

    /// What the width probes size a name of `root_member_select_shape` by,
    /// and whether that width holds for every evaluation of the name, so it
    /// may be kept on the node. A name whose selects are free of side effects
    /// is sized as its chain reads, from the element it selects. For one with
    /// a side-effect select nothing is evaluated: the expression is a literal
    /// as wide as the member's declared type (`root_member_declared_width`),
    /// which holds; else the chain through the collection's first element
    /// that holds an object (`root_member_probe_index`). The width of that
    /// object's own property holds too, since every element has the declared
    /// type; a member deeper than that is reached through handles it may
    /// hold null, so its width does not.
    #[cold]
    #[inline(never)]
    pub(in crate::compiler::simulator) fn root_member_size(
        &mut self,
        h: &HierarchicalIdentifier,
        span: crate::ast::Span,
    ) -> Option<(Expression, bool)> {
        if !self.root_member_select_shape(h) {
            return None;
        }
        if !Self::hier_has_effect_select(h) {
            return Some((self.root_member_select_chain(h, span, false, 0)?, false));
        }
        let (base, _, rest) = Self::root_member_parts(h)?;
        if let Some(w) = self.root_member_declared_width(&base, &rest) {
            let lit = Expression::new(
                ExprKind::Number(NumberLiteral::Integer {
                    size: Some(w),
                    signed: false,
                    base: NumberBase::Decimal,
                    value: "0".to_string(),
                    cached_val: crate::ast::cache::LiteralCache::new(None),
                }),
                span,
            );
            return Some((lit, true));
        }
        let probe = self.root_member_probe_index(&base);
        let chain = self.root_member_select_chain(h, span, false, probe.unwrap_or(0))?;
        // The probed object's own property sizes the member; a deeper member
        // is reached through handles that object may hold null, so its
        // width is not kept.
        Some((chain, probe.is_some() && rest.len() == 1))
    }

    /// The first index of the collection `base` names whose element is a
    /// live object, when its elements are class handles: the element the
    /// width probes read for a select they must not evaluate. `None` when
    /// there is none, or the elements are not handles.
    pub(in crate::compiler::simulator) fn root_member_probe_index(
        &self,
        base: &HierarchicalIdentifier,
    ) -> Option<i64> {
        let name = self.resolve_hier_name(base);
        let name = name.as_ref();
        let holds_objects = self.module.var_decl_types.get(name).is_some_and(|dt| {
            matches!(self.resolve_dt_ref(dt),
                DataType::TypeReference { name: t, .. } if self.module.classes.contains_key(&t.name.name))
        });
        if !holds_objects {
            return None;
        }
        let (lo, n) = if self.module.dynamic_arrays.contains(name) {
            (0, self.get_queue_size(name) as i64)
        } else if let Some(&(l, r, _)) = self.module.arrays.get(name) {
            (l.min(r), l.abs_diff(r) as i64 + 1)
        } else {
            return None;
        };
        (lo..lo + n).find(|k| {
            self.get_signal_value_by_name(&format!("{name}[{k}]"))
                .and_then(|v| v.to_u64())
                .is_some_and(|h| h != 0)
        })
    }

    /// The declared width of the member `rest` names below an element of the
    /// collection `base` (`objs` of `$root.tb.objs[k].v`, with `rest` the
    /// segments from `v`), walked from the collection's declared element type
    /// through struct members and class properties (§7.2, §8.4). No element
    /// is read, so a null handle at any level does not hide the width.
    ///
    /// A member declared in a parameterized class may be sized by the
    /// object's own parameters (§8.25): there only a type whose ranges are
    /// literals is accepted, and a type-parameter property ends the walk.
    /// Elsewhere a range must fold with the design's parameters, since
    /// `resolve_type_width` skips one it cannot fold and would hand back 1.
    /// `None` when a later segment selects or the walk meets another type.
    pub(in crate::compiler::simulator) fn root_member_declared_width(
        &self,
        base: &HierarchicalIdentifier,
        rest: &[crate::ast::expr::HierPathSegment],
    ) -> Option<u32> {
        if rest.iter().any(|s| !s.selects.is_empty()) {
            return None;
        }
        let name = self.resolve_hier_name(base);
        let mut dt = self.module.var_decl_types.get(name.as_ref())?.clone();
        // Whether the current type was declared in a parameterized class.
        let mut in_param_class = false;
        for seg in rest {
            let member = seg.name.name.as_str();
            dt = match self.resolve_dt_ref(&dt) {
                DataType::Struct(su) => su
                    .members
                    .iter()
                    .find(|m| m.declarators.iter().any(|d| d.name.name == member))?
                    .data_type
                    .clone(),
                DataType::TypeReference { name, .. }
                    if self.module.classes.contains_key(&name.name.name) =>
                {
                    let owners = self.prop_owners(&name.name.name);
                    let (owner, cd) = owners.get(member)?;
                    let pt = cd.property_types.get(member)?.clone();
                    if let DataType::TypeReference { name: t, .. } = &pt
                        && cd.type_param_names.contains(&t.name.name)
                    {
                        return None;
                    }
                    in_param_class = self.class_is_parameterized(owner)
                        || self.class_is_parameterized(&name.name.name);
                    pt
                }
                _ => return None,
            };
        }
        let params = (!in_param_class).then_some(&self.module.parameters);
        let folds = |dims: &[crate::ast::types::PackedDimension]| {
            dims.iter().all(|d| match d {
                crate::ast::types::PackedDimension::Range { left, right, .. } => {
                    crate::elaborate::const_eval_i64_with_params(left, params).is_some()
                        && crate::elaborate::const_eval_i64_with_params(right, params).is_some()
                }
                crate::ast::types::PackedDimension::Unsized(_) => false,
            })
        };
        let sized = match &dt {
            DataType::IntegerVector { dimensions, .. } | DataType::Implicit { dimensions, .. } => {
                folds(dimensions)
            }
            DataType::IntegerAtom { .. } => true,
            // A named type in a parameterized class may be one of its own
            // typedefs over its parameters.
            DataType::TypeReference { dimensions, .. } => !in_param_class && folds(dimensions),
            _ => !in_param_class,
        };
        if !sized {
            return None;
        }
        let w = super::elaborate::resolve_type_width(&dt, params, Some(&self.module.typedefs));
        (w > 0).then_some(w)
    }

    /// Read fast path for a select-free single-segment identifier: the frame
    /// local of that name, else the plain property of `this`. Everything the
    /// full path checks before those two reads is keyed on multi-segment or
    /// selected shapes, on an active `ref` redirect, on a static local, or
    /// on the names in `ident_slow` / `shadowed_prop_names`; any of those
    /// (or a miss) returns `None` and the full path runs.
    #[inline]
    pub(in crate::compiler::simulator) fn plain_ident_read(
        &self,
        h: &HierarchicalIdentifier,
    ) -> Option<Value> {
        if h.path.len() != 1 || h.root.is_some() || self.ref_redirect_hot || self.name_stats_on {
            return None;
        }
        let seg = &h.path[0];
        if !seg.selects.is_empty() {
            return None;
        }
        let name = seg.name.name.as_str();
        let names = self.class_member_names();
        let slow_first = names.ident_slow_first.get_or_init(|| {
            let mut first = [false; 256];
            for name in &names.ident_slow {
                if let Some(&byte) = name.as_bytes().first() {
                    first[byte as usize] = true;
                }
            }
            first
        });
        if name.as_bytes().first().is_none_or(|&b| slow_first[b as usize])
            && names.ident_slow.contains(name)
        {
            return None;
        }
        if !self.in_const_param_eval
            && !self.static_local_names.is_empty()
            && name
                .as_bytes()
                .first()
                .is_some_and(|&b| self.static_local_first[b as usize])
            && self.static_local_names.contains(name)
        {
            return None;
        }
        if let Some(locals) = self.local_stack.last() {
            if let Some(v) = locals.get(name) {
                return Some(v.clone());
            }
        }
        if !self.shadowed_prop_names.is_empty() && self.shadowed_prop_names.contains(name) {
            return None;
        }
        let handle = self.this_stack.last().copied().flatten()?;
        self.heap
            .get(handle)?
            .as_ref()?
            .properties
            .get(name)
            .cloned()
    }

    fn cached_known_integer(expr: &Expression) -> Option<Value> {
        let ExprKind::Number(NumberLiteral::Integer {
            size,
            signed,
            cached_val,
            ..
        }) = &expr.kind
        else {
            return None;
        };
        let (bits, 0, width) = cached_val.get()? else {
            return None;
        };
        if size.is_some_and(|size| size != width) {
            return None;
        }
        let mut value = Value::from_inline(bits, 0, width);
        value.is_signed = *signed;
        Some(value)
    }

    /// §11.6.1/§11.8.2: a plain integer read and a known integer literal
    /// need only their inferred widths and joint signedness for + or -.
    pub(in crate::compiler::simulator) fn plain_literal_add_sub(
        &mut self,
        op: BinaryOp,
        left: &Expression,
        right: &Expression,
        ctx_width: u32,
        unsigned_context: bool,
    ) -> Option<Value> {
        let ExprKind::Ident(h) = &left.kind else {
            return None;
        };
        let mut r = Self::cached_known_integer(right)?;
        let mut l = self.plain_ident_read(h)?;
        if l.is_real || l.is_fill {
            return None;
        }
        let lw = self.infer_width(left);
        let rw = right.cached_width.get().unwrap_or(r.width);
        // The general arithmetic path checks declared real types at width 64.
        if lw == 64 || rw == 64 {
            return None;
        }
        let signed = !unsigned_context && l.is_signed && r.is_signed;
        l.is_signed = signed;
        r.is_signed = signed;
        let width = lw.max(rw).max(l.width).max(r.width).max(ctx_width);
        if l.width < width {
            l = l.resize(width);
        }
        if r.width < width {
            r = r.resize(width);
        }
        self.shift_free_operand = 0;
        Some(if op == BinaryOp::Add { l.add(&r) } else { l.sub(&r) })
    }

    /// Read fast path for `h.p`: `h` a name `plain_ident_read` resolves to a
    /// live object, `p` an instance property of it. The member read's other
    /// arms key on names these two miss (vif, struct-capable, static,
    /// parameter, class, package, union-tag, associative), on an interface
    /// alias or clocking signal of that spelling, on a registered layout for
    /// `h`, or on a flat `h.p` frame key or signal (probed, also under the
    /// scope hint); any of those returns `None`.
    pub(in crate::compiler::simulator) fn plain_member_read(
        &self,
        base: &Expression,
        member: &crate::ast::Identifier,
    ) -> Option<Value> {
        let ExprKind::Ident(h) = &base.kind else {
            return None;
        };
        if h.path.len() != 1 || h.root.is_some() || !h.path[0].selects.is_empty() {
            return None;
        }
        let n = h.path[0].name.name.as_str();
        let m = member.name.as_str();
        if matches!(m, "size" | "num" | "triggered" | "status")
            || Self::is_parenless_zero_arg_builtin(m)
            || matches!(n, "super" | "this" | "process")
            || self.name_stats_on
            || self.rand_receiver.is_some()
            || self.item_alias.is_some()
            || self.iface_alias_for(n).is_some()
        {
            return None;
        }
        let names = self.class_member_names();
        if names.vif_props.contains(n)
            || names.params.contains(n)
            || names.static_or_param.contains(m)
            || self.struct_prop_name_possible(n)
            || self.struct_prop_name_possible(m)
        {
            return None;
        }
        if n == self.module.name
            || n.starts_with("genblk")
            || m.starts_with("genblk")
            || self.module.classes.contains_key(n)
            || self.module.packages.contains(n)
            || self.module.package_enum_members.contains_key(n)
            || self.module.packed_struct_fields.contains_key(n)
            || self.module.associative_arrays.contains_key(n)
            || self.active_union_tag.contains_key(n)
            || self.dyn_name_lookup(n).is_some()
            || (self.clocking_read_possible(base)
                && self.try_clocking_signal_read(&[n, m]).is_some())
        {
            return None;
        }
        if self.resolve_hier_name(h) != n {
            return None;
        }
        // The struct-layout projection: a typedef'd name whose type is a
        // struct, a module signal, or a typedef name.
        match self.var_typedef_types.get(n) {
            Some(tn) => {
                if let Some(dt) = self.module.typedef_types.get(tn.as_str()) {
                    let resolved = Self::resolve_type_ref(dt, &self.module.typedef_types);
                    if self.struct_field_layout(&resolved).is_some() {
                        return None;
                    }
                }
            }
            None => {
                if self.signal_name_to_id.contains_key(n)
                    || self.module.typedef_types.contains_key(n)
                {
                    return None;
                }
            }
        }
        let recv = self.plain_ident_read(h)?;
        {
            let mut key = self.hint_key_scratch.borrow_mut();
            key.clear();
            key.push_str(n);
            key.push('.');
            if self.any_struct_formal_markers
                && self
                    .local_stack
                    .last()
                    .is_some_and(|l| l.contains_key(key.as_str()))
            {
                return None;
            }
            key.push_str(m);
            if self
                .local_stack
                .last()
                .is_some_and(|l| l.contains_key(key.as_str()))
                || self.signal_name_to_id.contains_key(key.as_str())
                || self.signals.contains_key(key.as_str())
            {
                return None;
            }
            if let Some(hint) = self.name_resolve_hint.borrow().as_deref() {
                let mut scope = hint;
                let mut scoped = String::new();
                loop {
                    scoped.clear();
                    scoped.push_str(scope);
                    scoped.push('.');
                    scoped.push_str(&key);
                    if self.signal_name_to_id.contains_key(scoped.as_str())
                        || self.signals.contains_key(&scoped)
                    {
                        return None;
                    }
                    match scope.rsplit_once('.') {
                        Some((p, _)) => scope = p,
                        None => break,
                    }
                }
            }
        }
        let handle = recv.to_u64()? as usize;
        if handle == 0 || (self.shadowed_prop_mask != 0 && self.prop_maybe_shadowed(m)) {
            return None;
        }
        self.heap.get(handle)?.as_ref()?.properties.get(m).cloned()
    }

    /// `h.m(args)`: `h` a name `plain_ident_read` resolves to a live object
    /// and `m` a method some class declares, none declares static, and no
    /// interceptor of the call dispatch names (the builtin collection, enum,
    /// string, mailbox, randomization, process, factory and coverage
    /// methods). The dispatch's other arms key on such a method name or on a
    /// receiver that is a package, class, type parameter, typedef,
    /// collection, interface alias or subroutine scope; `h` is none of those,
    /// so the call is the dispatch's own tail: the non-virtual binding
    /// through `h`'s declared class, else the virtual method call.
    pub(in crate::compiler::simulator) fn plain_member_call(
        &mut self,
        func: &Expression,
        args: &[Expression],
    ) -> Option<Value> {
        let ExprKind::MemberAccess { expr: recv, member } = &func.kind else {
            return None;
        };
        let ExprKind::Ident(h) = &recv.kind else {
            return None;
        };
        if h.path.len() != 1 || h.root.is_some() || !h.path[0].selects.is_empty() {
            return None;
        }
        let n = h.path[0].name.name.as_str();
        let m = member.name.as_str();
        if matches!(n, "super" | "this" | "std" | "process")
            || BuiltinM::classify(m) != BuiltinM::Other
            || Self::is_array_builtin_method(m)
            || matches!(
                m,
                "new"
                    | "rand_mode"
                    | "constraint_mode"
                    | "randomize"
                    | "srandom"
                    | "get_randstate"
                    | "set_randstate"
                    | "name"
                    | "tolower"
                    | "toupper"
                    | "putc"
                    | "compare"
                    | "icompare"
                    | "itoa"
                    | "hextoa"
                    | "octtoa"
                    | "bintoa"
                    | "realtoa"
                    | "atoreal"
                    | "atoi"
                    | "atohex"
                    | "atooct"
                    | "atobin"
                    | "put"
                    | "get"
                    | "try_get"
                    | "try_put"
                    | "peek"
                    | "try_peek"
                    | "get_next_item"
                    | "create"
                    | "create_component_by_name"
                    | "create_object_by_name"
                    | "get_coverage"
                    | "get_inst_coverage"
                    | "start"
                    | "stop"
                    | "self"
                    | "status"
                    | "kill"
                    | "await"
                    | "suspend"
                    | "resume"
            )
        {
            return None;
        }
        {
            let names = self.class_member_names();
            if !names.methods.contains(m)
                || names.static_methods.contains(m)
                || names.vif_props.contains(n)
                || names.params.contains(n)
            {
                return None;
            }
            let mut key = self.hint_key_scratch.borrow_mut();
            key.clear();
            key.push_str(n);
            key.push('.');
            key.push_str(m);
            if names.subroutine_suffixes.contains(key.as_str()) {
                return None;
            }
        }
        if self.module.packages.contains(n)
            || self.module.classes.contains_key(n)
            || self.module.dynamic_arrays.contains(n)
            || self.module.associative_arrays.contains_key(n)
            || self.module.arrays.contains_key(n)
            || self.coll_member_possible(n)
            || self.iface_alias_for(n).is_some()
            || self.dyn_name_lookup(n).is_some()
        {
            return None;
        }
        // A receiver that is no frame local and no signal is first tried as
        // a class-scope name (a typedef alias of a class).
        if !self.local_stack.last().is_some_and(|l| l.contains_key(n)) {
            if self.resolve_hier_name(h) != n
                || self.signal_name_to_id.contains_key(n)
                || self.signals.contains_key(n)
                || self.lookup_typedef_target(n).is_some()
            {
                return None;
            }
        }
        let handle = self.plain_ident_read(h)?.to_u64()? as usize;
        if handle == 0 || !matches!(self.heap.get(handle), Some(Some(_))) {
            return None;
        }
        if self.cg_index(handle).is_some() {
            return None;
        }
        if let Some(decl_cls) = self.class_of_var(n) {
            let decl_base = decl_cls.split('#').next().unwrap_or(&decl_cls).to_string();
            if let Some(target) = self.nonvirtual_target_class(&decl_base, m) {
                let runtime = self
                    .heap
                    .get(handle)
                    .and_then(|o| o.as_ref())
                    .map(|i| i.class_name.clone());
                let target_on_chain = runtime
                    .as_deref()
                    .is_some_and(|rt| target == rt || self.class_is_a(rt, &target));
                if target_on_chain && runtime.as_deref() != Some(target.as_str()) {
                    return Some(self.exec_method_in_class_hierarchy(handle, &target, m, args));
                }
            }
        }
        Some(self.exec_method_call(handle, m, args))
    }

    /// A comparison operand the binary arm's aggregate probes cannot claim:
    /// a literal, `null`, `this` or a negated/inverted literal. The virtual
    /// interface, struct, array, queue, event and string probes all need a
    /// named (or selected) operand, or a named one on BOTH sides.
    pub(in crate::compiler::simulator) fn cmp_literal_like(e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Number(_) | ExprKind::Null | ExprKind::This => true,
            ExprKind::Unary { op, operand } => {
                !matches!(
                    op,
                    UnaryOp::PreIncr | UnaryOp::PreDecr | UnaryOp::PostIncr | UnaryOp::PostDecr
                ) && matches!(operand.kind, ExprKind::Number(_))
            }
            _ => false,
        }
    }

    /// The binary arm's per-operand probes come back empty for the named
    /// comparison operand `e` (compared with `other`): no virtual-interface
    /// binding can name it, it is no event compared with `null`, and it is
    /// not string-valued. A plain name, a call, or `h.p` / `this.p`.
    pub(in crate::compiler::simulator) fn cmp_named_ok(
        &mut self,
        e: &Expression,
        other: &Expression,
    ) -> bool {
        match &e.kind {
            ExprKind::Ident(h) => {
                if h.path.len() != 1 || h.root.is_some() || !h.path[0].selects.is_empty() {
                    return false;
                }
                let n = h.path[0].name.name.as_str();
                if self.class_member_names().vif_props.contains(n)
                    || self.vif_local_names.contains(n)
                    || self.is_interface_instance(n)
                    || self.iface_alias_for(n).is_some()
                {
                    return false;
                }
                // `ev == null` compares event identities.
                if matches!(other.kind, ExprKind::Null) && self.module.events.contains(n) {
                    return false;
                }
            }
            // A call result: no probe but the string one names it.
            ExprKind::Call { .. } => {}
            // `h.p` / `this.p`: the virtual-interface probe keys on a
            // declared vif property, the struct one on a struct-capable one.
            ExprKind::MemberAccess { expr: b, member } => {
                let plain_base = match &b.kind {
                    ExprKind::This => true,
                    ExprKind::Ident(bh) => bh.path.len() == 1 && bh.path[0].selects.is_empty(),
                    _ => false,
                };
                if !plain_base
                    || self
                        .class_member_names()
                        .vif_props
                        .contains(member.name.as_str())
                    || self.struct_prop_name_possible(&member.name)
                {
                    return false;
                }
            }
            _ => return false,
        }
        !self.expr_is_string_valued(e)
    }

    /// A select-free one-segment name the binary arm's two-sided probes
    /// cannot take for an aggregate: no struct-capable or collection member
    /// name, no queue, event or renamed local, no declared struct type and
    /// no fixed, 2-D or N-D array under its resolved name.
    pub(in crate::compiler::simulator) fn cmp_name_scalar(
        &mut self,
        h: &HierarchicalIdentifier,
    ) -> bool {
        if h.path.len() != 1 || h.root.is_some() || !h.path[0].selects.is_empty() {
            return false;
        }
        let n = h.path[0].name.name.as_str();
        if self.struct_prop_name_possible(n)
            || self.coll_member_possible(n)
            || self.module.dynamic_arrays.contains(n)
            || self.module.events.contains(n)
            || self.dyn_name_lookup(n).is_some()
            || self
                .module
                .var_decl_types
                .get(n)
                .is_some_and(|dt| matches!(self.resolve_dt_ref(dt), DataType::Struct(_)))
        {
            return false;
        }
        let r = self.resolve_hier_name(h);
        !(self.module.arrays.contains_key(&*r)
            || self.module.arrays_2d.contains_key(&*r)
            || self.module.arrays_nd.contains_key(&*r)
            || self.module.associative_arrays.contains_key(&*r))
    }

    /// Whether the comparison operand `e` may be an unpacked aggregate: not
    /// a literal and not a plain scalar name (`cmp_name_scalar`).
    pub(in crate::compiler::simulator) fn may_be_aggregate(&mut self, e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Ident(h) => !self.cmp_name_scalar(h),
            _ => !Self::cmp_literal_like(e),
        }
    }

    /// A relational or (in)equality comparison of literal-like operands,
    /// named operands (`cmp_named_ok`) and at most one named per side. With
    /// one side literal-like the binary arm's struct, array and queue
    /// comparisons (which need both sides aggregates) cannot engage; two
    /// named sides must be plain scalar names (`cmp_name_scalar`). The rest
    /// is the arm's own sizing, evaluation order, real promotion and
    /// integral compare.
    /// §5.9 / §6.16: a literal is integral until a string-typed operand
    /// supplies a string context. Parentheses do not supply that context.
    pub(in crate::compiler::simulator) fn operand_has_string_type(
        &mut self,
        e: &Expression,
    ) -> bool {
        match &e.kind {
            ExprKind::StringLiteral(_) => false,
            ExprKind::Paren(inner) => self.operand_has_string_type(inner),
            ExprKind::Concatenation(parts) => parts.iter().any(|e| self.operand_has_string_type(e)),
            _ => self.operand_is_string(e),
        }
    }

    pub(in crate::compiler::simulator) fn plain_compare(
        &mut self,
        op: BinaryOp,
        left: &Expression,
        right: &Expression,
        ctx_width: u32,
    ) -> Option<Value> {
        if !matches!(
            op,
            BinaryOp::Eq
                | BinaryOp::Neq
                | BinaryOp::Lt
                | BinaryOp::Leq
                | BinaryOp::Gt
                | BinaryOp::Geq
        ) {
            return None;
        }
        match (Self::cmp_literal_like(left), Self::cmp_literal_like(right)) {
            // `null == null` compares event identities.
            (true, true) => {
                if matches!(left.kind, ExprKind::Null) && matches!(right.kind, ExprKind::Null) {
                    return None;
                }
            }
            (true, false) => {
                if !self.cmp_named_ok(right, left) {
                    return None;
                }
            }
            (false, true) => {
                if !self.cmp_named_ok(left, right) {
                    return None;
                }
            }
            // Two plain names: the aggregate comparisons need both named.
            (false, false) => {
                let (ExprKind::Ident(lh), ExprKind::Ident(rh)) = (&left.kind, &right.kind) else {
                    return None;
                };
                if !self.cmp_name_scalar(lh)
                    || !self.cmp_name_scalar(rh)
                    || !self.cmp_named_ok(left, right)
                    || !self.cmp_named_ok(right, left)
                {
                    return None;
                }
            }
        }
        // §11.6.1: these two reads produce their own widths even when the
        // evaluator supplies a comparison context. Reuse the normal value
        // comparison below, including its real and X/Z behavior.
        let (l, r) = if let ExprKind::Ident(h) = &left.kind
            && let Some(r) = Self::cached_known_integer(right)
            && let Some(l) = self.plain_ident_read(h)
        {
            (l, r)
        } else {
            let sw = match (self.lrm_self_width(left), self.lrm_self_width(right)) {
                (Some(lw), Some(rw)) => lw.max(rw),
                _ => ctx_width,
            };
            (self.eval_expr_ctx(left, sw), self.eval_expr_ctx(right, sw))
        };
        // §11.8.2: against a real operand the plain name or literal on the
        // other side is converted from its own value — the comparison
        // operators below convert it.
        Some(match op {
            BinaryOp::Eq => l.is_equal(&r),
            BinaryOp::Neq => l.is_not_equal(&r),
            BinaryOp::Lt => l.less_than(&r),
            BinaryOp::Leq => l.leq(&r),
            BinaryOp::Gt => l.greater_than(&r),
            _ => l.geq(&r),
        })
    }

    /// Whether the member/select chain `e` reads a member (`objs[k].v`,
    /// `objs[k].arr[j]`), not only selects of a variable.
    fn chain_reads_member(e: &Expression) -> bool {
        let mut cur = e;
        loop {
            match &cur.kind {
                ExprKind::MemberAccess { .. } => return true,
                ExprKind::Index { expr, .. }
                | ExprKind::RangeSelect { expr, .. }
                | ExprKind::Paren(expr) => cur = expr,
                _ => return false,
            }
        }
    }

    /// Whether `e` reads a member through a select whose index has a side
    /// effect (`objs[i++].v`, `qs[next()].a`, `objs[i++].arr[j++]`), in a
    /// chain not rooted at a call. A call-rooted chain (`f().objs[i++].v`)
    /// keeps its own path: its statement may hold the call's value by node
    /// (`recv_call_memo`), which a copy of the chain would not find.
    pub(in crate::compiler::simulator) fn is_effect_member_read(e: &Expression) -> bool {
        Self::chain_reads_member(e)
            && Self::lvalue_has_incdec_index(e)
            && Self::recv_chain_call(e).is_none()
    }

    /// §11.4.2, §13.4: a member read of `is_effect_member_read`'s shape
    /// evaluates each side-effect index once. The member and select arms
    /// resolve the element several times as they try each storage shape (a
    /// handle, a packed or an unpacked struct element), so the index ran once
    /// per try and the read returned another element's member. As for a
    /// method call's receiver (`eval_call_inner`), each such index becomes the
    /// literal of its value, evaluated once, left to right.
    pub(in crate::compiler::simulator) fn freeze_member_read_selects(
        &mut self,
        e: &Expression,
    ) -> Option<Expression> {
        if !Self::is_effect_member_read(e) {
            return None;
        }
        self.rewrite_lvalue_index_side_effects(e)
    }

    /// Whether `e` is a select chain with a side-effect select (`a[k++]`,
    /// `q[next()][1]`, `objs[i++].v`), not rooted at a call (see
    /// `is_effect_member_read`).
    pub(in crate::compiler::simulator) fn is_effect_select_read(e: &Expression) -> bool {
        matches!(
            e.kind,
            ExprKind::Index { .. }
                | ExprKind::RangeSelect { .. }
                | ExprKind::MemberAccess { .. }
                | ExprKind::Paren(_)
        ) && Self::lvalue_has_incdec_index(e)
            && Self::recv_chain_call(e).is_none()
    }

    /// §11.3.2, §11.4.2, §13.4: an operand of a binary operator is evaluated
    /// once. The operator's arm tries an operand several ways before it
    /// reads its value (the struct, array and pattern comparisons, the width
    /// probes of `*`), so a side-effect select in an operand (`a[k++] ==
    /// a[1]`) ran once per try, and the operator read another element. Each
    /// such select becomes the literal of its value, evaluated once, the left
    /// operand's first. `None` when no operand has one, or when the right
    /// one has and the left has some other side effect (a call), whose order
    /// against the select a frozen copy would change.
    pub(in crate::compiler::simulator) fn freeze_binary_effect_selects(
        &mut self,
        expr: &Expression,
        op: BinaryOp,
        left: &Expression,
        right: &Expression,
    ) -> Option<Expression> {
        let l = Self::is_effect_select_read(left);
        let r = Self::is_effect_select_read(right);
        if !l && !r {
            return None;
        }
        if !l && !Self::cond_expr_is_effect_free(left, false) {
            return None;
        }
        let left = if l {
            self.rewrite_lvalue_index_side_effects(left)
        } else {
            None
        }
        .unwrap_or_else(|| left.clone());
        let right = if r {
            self.rewrite_lvalue_index_side_effects(right)
        } else {
            None
        }
        .unwrap_or_else(|| right.clone());
        Some(Expression::new(
            ExprKind::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            },
            expr.span,
        ))
    }

    /// What the width probes size a read of `is_effect_select_read`'s
    /// shape by, without running its side effects (§11.4.2, §13.4): a copy in
    /// which each side-effect index is the literal of an element the select
    /// may name (`probe_select`). Every element has the collection's
    /// declared type, so the copy is as wide as the read, and evaluating it
    /// evaluates no side effect. Sized by evaluating the read itself, an
    /// operand of a comparison or of an arithmetic operator ran its index
    /// once more for each probe.
    pub(in crate::compiler::simulator) fn member_read_width_probe(
        &mut self,
        e: &Expression,
    ) -> Expression {
        let kind = match &e.kind {
            ExprKind::Index { expr, index } => {
                let base = self.member_read_width_probe(expr);
                let index = if Self::cond_expr_is_effect_free(index, false) {
                    (**index).clone()
                } else {
                    self.probe_select(&base, index.span)
                };
                ExprKind::Index {
                    expr: Box::new(base),
                    index: Box::new(index),
                }
            }
            ExprKind::RangeSelect {
                expr,
                left,
                right,
                kind,
            } => {
                let base = self.member_read_width_probe(expr);
                let bound = |me: &mut Self, b: &Expression| {
                    if Self::cond_expr_is_effect_free(b, false) {
                        b.clone()
                    } else {
                        me.probe_select(&base, b.span)
                    }
                };
                let (left, right) = (bound(self, left), bound(self, right));
                ExprKind::RangeSelect {
                    expr: Box::new(base),
                    left: Box::new(left),
                    right: Box::new(right),
                    kind: *kind,
                }
            }
            ExprKind::MemberAccess { expr, member } => ExprKind::MemberAccess {
                expr: Box::new(self.member_read_width_probe(expr)),
                member: member.clone(),
            },
            ExprKind::Paren(inner) => {
                ExprKind::Paren(Box::new(self.member_read_width_probe(inner)))
            }
            _ => return e.clone(),
        };
        Expression::new(kind, e.span)
    }

    /// A select the width probes may read through `base` in place of a
    /// side-effect select: the first index or key of the collection `base`
    /// names. When its elements are class handles, the first that holds an
    /// object instead, however far back: a null element has no object to
    /// size a property by, and sized through one a byte property read as 32
    /// bits. With no object anywhere, the read itself is through a null
    /// handle. `base` has no side-effect select left, so naming it evaluates
    /// none.
    fn probe_select(&mut self, base: &Expression, span: crate::ast::Span) -> Expression {
        let zero = || Self::probe_index_literal(0, span);
        // An object's collection property (`h.items`, `items` in a method)
        // is stored as `<handle>#items`.
        let Some(name) = self
            .expr_assoc_name(base)
            .or_else(|| self.flat_member_name(base))
        else {
            return zero();
        };
        let handles = self.probe_coll_holds_handles(&name);
        let live = |me: &Self, elem: &str| {
            me.get_signal_value_by_name(elem)
                .and_then(|v| v.to_u64())
                .is_some_and(|h| h != 0 && me.heap.get(h as usize).is_some_and(|o| o.is_some()))
        };
        if self.is_associative_array(&name) {
            let keys = self.assoc_top_level_keys_shared(&name);
            let found = handles
                .then(|| keys.iter().find(|k| live(self, &format!("{name}[{k}]"))))
                .flatten();
            let Some(key) = found.or(keys.first()) else {
                return zero();
            };
            if self.is_string_keyed_array(&name) {
                let key = decode_name_key(key).to_string();
                return Expression::new(ExprKind::StringLiteral(key), span);
            }
            return key
                .parse::<i64>()
                .map_or_else(|_| zero(), |k| Self::probe_index_literal(k, span));
        }
        // A queue or dynamic array by its live size: the fixed-array table
        // can hold a placeholder shape for it.
        let (lo, n) = match self.module.arrays.get(&name) {
            Some(&(l, r, _)) if !self.module.dynamic_arrays.contains(&name) => {
                (l.min(r), l.abs_diff(r) + 1)
            }
            _ => (0, self.get_queue_size(&name)),
        };
        let k = handles
            .then(|| (lo..lo + n as i64).find(|k| live(self, &format!("{name}[{k}]"))))
            .flatten()
            .unwrap_or(lo);
        Self::probe_index_literal(k, span)
    }

    /// Whether the elements of the collection stored as `name` are class
    /// handles: a design collection (`coll_elem_is_class_handle`), or an
    /// object's property (`<h>#items`) declared with a class element type.
    fn probe_coll_holds_handles(&self, name: &str) -> bool {
        if self.coll_elem_is_class_handle(name) {
            return true;
        }
        let Some((h, prop)) = name.split_once('#') else {
            return false;
        };
        let Some(class) = h
            .parse::<usize>()
            .ok()
            .and_then(|h| self.heap.get(h)?.as_ref())
            .map(|o| o.class_name.clone())
        else {
            return false;
        };
        let class = class.split('#').next().unwrap_or(&class).trim();
        let owners = self.prop_owners(class);
        owners
            .get(prop)
            .and_then(|(_, cd)| cd.properties.get(prop))
            .and_then(|sig| sig.type_name.as_deref())
            .is_some_and(|t| self.resolve_typeref_class_name_str(t.trim()).is_some())
    }

    fn probe_index_literal(k: i64, span: crate::ast::Span) -> Expression {
        let lit = Expression::new(
            ExprKind::Number(NumberLiteral::Integer {
                size: None,
                signed: true,
                base: NumberBase::Decimal,
                value: k.unsigned_abs().to_string(),
                cached_val: crate::ast::cache::LiteralCache::new(None),
            }),
            span,
        );
        if k >= 0 {
            return lit;
        }
        Expression::new(
            ExprKind::Unary {
                op: UnaryOp::Minus,
                operand: Box::new(lit),
            },
            span,
        )
    }

    /// The outermost CALL in receiver position of the member/select chain
    /// `e` — `me()` in `me().o.x`, `q.pop_front()` in `q.pop_front().addr`,
    /// `a().b()` in `a().b().c` — never `e`'s own callee.
    pub(in crate::compiler::simulator) fn recv_chain_call(e: &Expression) -> Option<&Expression> {
        let mut cur: &Expression = match &e.kind {
            ExprKind::MemberAccess { expr, .. }
            | ExprKind::Index { expr, .. }
            | ExprKind::RangeSelect { expr, .. } => expr,
            ExprKind::Call { func, .. } => match &func.kind {
                ExprKind::MemberAccess { expr, .. } => expr,
                _ => return None,
            },
            _ => return None,
        };
        loop {
            match &cur.kind {
                ExprKind::Call { .. } => return Some(cur),
                ExprKind::MemberAccess { expr, .. }
                | ExprKind::Index { expr, .. }
                | ExprKind::RangeSelect { expr, .. } => cur = expr,
                ExprKind::Paren(inner) => cur = inner,
                _ => return None,
            }
        }
    }

    /// `e`'s receiver call when no enclosing evaluation at this call depth
    /// already holds its value.
    pub(in crate::compiler::simulator) fn recv_call_to_memoize<'e>(
        &self,
        e: &'e Expression,
    ) -> Option<&'e Expression> {
        let call = Self::recv_chain_call(e)?;
        let key = call as *const Expression as usize;
        let depth = self.local_stack.len();
        (!self
            .recv_call_memo
            .iter()
            .any(|&(k, d, _)| k == key && d == depth))
        .then_some(call)
    }

    /// The held value of receiver call `e` (see `recv_call_memo`).
    pub(in crate::compiler::simulator) fn recv_call_memo_hit(
        &self,
        e: &Expression,
    ) -> Option<Value> {
        let key = e as *const Expression as usize;
        let depth = self.local_stack.len();
        self.recv_call_memo
            .iter()
            .rev()
            .find(|&&(k, d, _)| k == key && d == depth)
            .map(|(_, _, v)| v.clone())
    }

    /// §13.4/§8.5: a member or select applied to a CALL result — `me().x`,
    /// `q.pop_front().addr`, `get_cfg().vif.data`, `me().o.get()` — runs the
    /// call ONCE. The member paths below re-evaluate their receiver as they
    /// try each storage shape (a handle, a packed or unpacked struct value, a
    /// struct leaf), so a chain rooted at a call ran it up to a dozen times:
    /// every side effect repeated, and `q.pop_front().addr` drained the whole
    /// queue. Evaluate the call first, hold its value for the duration of
    /// this evaluation (the paths keep the call's AST for its type), and
    /// resolve the chain as before.
    #[inline(never)]
    pub(in crate::compiler::simulator) fn recv_call_once_eval(
        &mut self,
        expr: &Expression,
        ctx_width: u32,
    ) -> Option<Value> {
        let call = self.recv_call_to_memoize(expr)?;
        let v = self.eval_expr(call);
        let key = call as *const Expression as usize;
        self.recv_call_memo.push((key, self.local_stack.len(), v));
        let r = self.eval_expr_ctx(expr, ctx_width);
        self.recv_call_memo.pop();
        Some(r)
    }

    /// The write-side twin of [`recv_call_once_eval`]: `me().x = v` runs
    /// `me()` once.
    #[inline(never)]
    pub(in crate::compiler::simulator) fn recv_call_once_assign(
        &mut self,
        lhs: &Expression,
        val: &Value,
    ) -> Option<bool> {
        let call = self.recv_call_to_memoize(lhs)?;
        let v = self.eval_expr(call);
        let key = call as *const Expression as usize;
        self.recv_call_memo.push((key, self.local_stack.len(), v));
        let r = self.assign_value(lhs, val);
        self.recv_call_memo.pop();
        Some(r)
    }

    /// Statement-wide twin of [`recv_call_once_eval`]: run the receiver call
    /// of each of a statement's own expressions (RHS first) and hold the
    /// value until the statement ends — its probes (collection storage,
    /// fixed-array shape, lvalue width) evaluate those expressions on their
    /// own, outside any one evaluation. Returns the memo length the caller
    /// restores when the statement completes; `None` when no expression has
    /// a receiver call left to run.
    #[inline(never)]
    pub(in crate::compiler::simulator) fn pin_stmt_recv_calls(
        &mut self,
        exprs: &[&Expression],
    ) -> Option<usize> {
        let base = self.recv_call_memo.len();
        for e in exprs {
            if let Some(call) = self.recv_call_to_memoize(e) {
                let v = self.eval_expr(call);
                let key = call as *const Expression as usize;
                self.recv_call_memo.push((key, self.local_stack.len(), v));
            }
        }
        (self.recv_call_memo.len() > base).then_some(base)
    }

    /// §8.4/§23.7: resolve the declared class boundary before following
    /// object properties. Integral values and unrelated leaf names cannot
    /// establish a class boundary.
    pub(in crate::compiler::simulator) fn hier_parent_handle(
        &self,
        hier: &crate::ast::expr::HierarchicalIdentifier,
    ) -> Option<usize> {
        if self.no_class_objects()
            || hier.path.len() < 3
            || hier.path.iter().any(|s| !s.selects.is_empty())
            || self.bare_name_is_class_channel(&hier.path[0].name.name)
        {
            return None;
        }
        let segs: Vec<&str> = hier.path.iter().map(|s| s.name.name.as_str()).collect();
        for split in (1..segs.len()).rev() {
            let dotted = segs[..split].join(".");
            let name = dotted
                .strip_prefix(&format!("{}.", self.module.name))
                .unwrap_or(&dotted);
            let class = self
                .module
                .var_decl_types
                .get(name)
                .and_then(|dt| self.declared_typeref_name(dt))
                .or_else(|| {
                    self.module
                        .signals
                        .get(name)
                        .and_then(|s| s.type_name.as_deref().cloned())
                })
                .or_else(|| {
                    self.signal_name_to_id
                        .get(name)
                        .and_then(|id| self.signal_type_names.get(id).cloned())
                })
                .and_then(|tn| self.resolve_typeref_class_name_str(&tn));
            let mut handle = if split == 1 && self.bare_name_is_class_channel(segs[0]) {
                self.eval_ident_handle(segs[0])
            } else if class.is_some() {
                self.get_signal_value_by_name(name)
                    .and_then(|v| v.to_u64())
                    .map(|v| v as usize)
            } else {
                None
            };
            let mids = &segs[split..segs.len() - 1];
            let mut i = 0;
            while i < mids.len() {
                let Some(h) = handle else {
                    break;
                };
                // §7.2, §8.4: a class-handle member of an unpacked-struct
                // property (`o.r.h.k`): the struct's members are cells of
                // `o`, and the handle member's cell names the next object.
                if let Some((next, used)) = self.struct_prop_handle_walk(h, &mids[i..]) {
                    handle = Some(next);
                    i += used;
                    continue;
                }
                let member = mids[i];
                handle = self
                    .class_prop_type_name(h, member)
                    .and_then(|tn| self.resolve_typeref_class_name_str(&tn))
                    .and_then(|_| self.member_handle(h, member));
                i += 1;
            }
            if let Some(h) =
                handle.filter(|&h| h != 0 && self.heap.get(h).is_some_and(|o| o.is_some()))
            {
                return Some(h);
            }
        }
        None
    }
}
