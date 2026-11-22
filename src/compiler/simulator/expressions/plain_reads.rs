//! Plain identifier and member reads, plain compares, receiver-call memoization and the
//! hierarchical parent handle.

use super::*;

impl Simulator {
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
                        .and_then(|s| s.type_name.clone())
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
