//! Signedness of selects and expressions.

use super::*;

impl Simulator {
    /// §7.4.1: whether `expr` (an index chain on a plain packed variable,
    /// not a local or an unpacked array) selects an element of the named
    /// signed type the variable is a packed array of.
    #[inline(never)]
    pub(in crate::compiler::simulator) fn packed_named_select_signed(
        &mut self,
        expr: &Expression,
    ) -> bool {
        let mut root = expr;
        let mut selects = 0usize;
        while let ExprKind::Index { expr: base, .. } = &root.kind {
            selects += 1;
            root = base;
        }
        // Rebase a virtual interface receiver before asking for the signal's
        // declared element type. A selected generate scope likewise names
        // independently stored signals.
        let physical = match &root.kind {
            ExprKind::MemberAccess { .. } => self.vif_rebase_expr(root)
                .or_else(|| self.fold_indexed_hier_path(root)),
            ExprKind::Ident(h) if h.path.len() > 1 || h.path.iter().any(|s| !s.selects.is_empty()) =>
                self.vif_rebase_expr(root).or_else(|| {
                    h.path.iter().any(|s| !s.selects.is_empty())
                        .then(|| self.fold_indexed_hier_path(root)).flatten()
                }),
            _ => None,
        };
        if let Some(ref physical) = physical {
            root = physical;
        }
        // A class property (`h.m_amb[1]`, or `m_amb[1]` in a method): its
        // declared type is its class's.
        if !self.packed_named_signed_props.is_empty() {
            let prop = match &root.kind {
                ExprKind::MemberAccess { member, .. } => Some(member.name.as_str()),
                ExprKind::Ident(h) if h.root.is_none() => h
                    .path
                    .last()
                    .filter(|s| s.selects.is_empty())
                    .map(|s| s.name.name.as_str()),
                _ => None,
            };
            if prop.is_some_and(|p| self.packed_named_signed_props.get(p) == Some(&selects))
                && self.class_prop_named_signed_depth(root) == Some(selects)
            {
                return true;
            }
        }
        let ExprKind::Ident(h) = &root.kind else {
            // Rebasing intentionally leaves an identity-bound interface
            // unchanged. Its source node can still be a member access while
            // its physical signal declaration uses the flat instance path.
            if let ExprKind::MemberAccess { expr: receiver, member } = &root.kind
                && let Some((bound, _)) = self.vif_recv_target(receiver)
            {
                let name = format!("{bound}.{}", member.name);
                return self.declared_signed_select_depth(&name) == Some(selects);
            }
            // §7.2, §7.4.1: a member written as a member access
            // (`sa[1].m[1]`, `f().m[1]`, `h.st.m[1]`).
            return matches!(&root.kind, ExprKind::MemberAccess { member, .. }
                if self.packed_named_signed_members.contains(member.name.as_str()))
                && self.lvalue_static_type(root).is_some_and(|dt| {
                    super::bytecode::signed_named_packed_elem_depth(&dt, &self.module.typedef_types)
                        == Some(selects)
                });
        };
        if h.root.is_some() {
            // Virtual-interface rebasing marks the physical instance rooted.
            if h.root.as_deref() != Some("$root")
                || h.path.iter().any(|seg| !seg.selects.is_empty())
            {
                return false;
            }
            let name = self.resolve_hier_name(h);
            let flat = name.strip_prefix(&self.module.name)
                .and_then(|tail| tail.strip_prefix('.')).unwrap_or(&name);
            return self.declared_signed_select_depth(flat).or_else(|| {
                let dt = super::bytecode::declared_type_of(
                    flat, &self.module.var_decl_types, &self.module.typedef_types)?;
                super::bytecode::signed_named_packed_elem_depth(dt, &self.module.typedef_types)
            }) == Some(selects);
        }
        if h.path.iter().any(|s| !s.selects.is_empty()) {
            // A variable of an indexed scope (`g_loop[0].gv[1]`).
            if h.path.last().is_some_and(|s| s.selects.is_empty()) {
                let name = self.resolve_hier_name(h);
                let name = name.as_ref();
                if self.packed_named_signed.get(name) == Some(&selects)
                    && !self.module.arrays.contains_key(name)
                {
                    return true;
                }
            }
            // An element of a packed array of structures on the way
            // (`sa[1].m` as one path): the member's declared type.
            return h.path.len() >= 2
                && h.path.last().is_some_and(|s| {
                    s.selects.is_empty()
                        && self
                            .packed_named_signed_members
                            .contains(s.name.name.as_str())
                })
                && self.lvalue_static_type(root).is_some_and(|dt| {
                    super::bytecode::signed_named_packed_elem_depth(&dt, &self.module.typedef_types)
                        == Some(selects)
                });
        }
        // §7.4.1, §13.3: a formal or a local of the running subroutine or
        // block. Its declaration is recorded under the bare name while the
        // frame is live, so the element type comes from there; the table
        // built at start-up only holds the design's own variables.
        if let [seg] = h.path.as_slice()
            && selects > 1
            && let Some(store) = self.dyn_name_lookup(&seg.name.name)
            && let Some(depth) = self.declared_signed_select_depth(store)
        {
            return depth == selects;
        }
        if let Some(n) = self.frame_bound_bare_name(h) {
            return self.frame_named_signed_depth(n) == Some(selects);
        }
        let name = self.resolve_hier_name(h);
        let name = name.as_ref();
        let depth = self.declared_signed_select_depth(name);
        if depth == Some(selects) {
            return true;
        }
        // §7.2, §7.4.1: a packed-array member reached through structure
        // members, elements of a packed array of structures or an instance
        // (`nst.i.m[1]`, `sa[1].m[1]`, `u.st.m[1]`) has its declared type.
        depth.is_none()
            && h.path.len() >= 2
            && h.path.last().is_some_and(|s| {
                self.packed_named_signed_members
                    .contains(s.name.name.as_str())
            })
            && self.lvalue_static_type(root).is_some_and(|dt| {
                super::bytecode::signed_named_packed_elem_depth(&dt, &self.module.typedef_types)
                    == Some(selects)
            })
    }

    fn declared_signed_select_depth(&self, name: &str) -> Option<usize> {
        let packed = self.packed_named_signed.get(name).copied().or_else(|| {
            if name.starts_with('@') {
                let dt = self.p_elem_type_ref(name)?;
                super::bytecode::signed_named_packed_elem_depth(&dt, &self.module.typedef_types)
            } else {
                let dt = self.module.var_decl_types.get(name)?;
                super::bytecode::signed_named_packed_elem_depth(dt, &self.module.typedef_types)
            }
        })?;
        let unpacked = self.unpacked_dims_of(name).max(usize::from(
            self.module.dynamic_arrays.contains(name) || self.module.queue_vars.contains(name)
                || self.is_associative_array(name),
        ));
        Some(unpacked + packed)
    }

    /// §7.4.1: whether `dt` is a packed array of a named type, written with
    /// its own dimensions (`lane_t [0:1]`) or carried by a typedef
    /// (`typedef lane_t [0:1] arr_t;`).
    pub(in crate::compiler::simulator) fn names_packed_array_of_named(
        &self,
        dt: &DataType,
    ) -> bool {
        let mut cur = dt;
        for _ in 0..16 {
            let DataType::TypeReference {
                name, dimensions, ..
            } = cur
            else {
                return false;
            };
            if !dimensions.is_empty() {
                return true;
            }
            match super::elaborate::lookup_scoped_type(name, &self.module.typedef_types) {
                Some(next) => cur = next,
                None => return false,
            }
        }
        false
    }

    /// §7.4.1: the select depth at which the formal or local `n` of the
    /// running frame, a packed array, reaches a named signed element type.
    /// None for any other variable, an unpacked array of such arrays
    /// included: its first selects are unpacked, and its declaration here
    /// is that of one element.
    fn frame_named_signed_depth(&self, n: &str) -> Option<usize> {
        let dt = self.module.var_decl_types.get(n)?;
        let depth =
            super::bytecode::signed_named_packed_elem_depth(dt, &self.module.typedef_types)?;
        let frame = self.local_stack.last()?;
        let whole = frame.get(n)?;
        let width = super::elaborate::resolve_type_width(
            dt,
            Some(&self.module.parameters),
            Some(&self.module.typedefs),
        );
        (whole.width == width
            && !frame.contains_key(format!("\u{1}array_formal:{n}").as_str())
            && !self.module.arrays.contains_key(n)
            && !self.module.arrays_2d.contains_key(n)
            && !self.module.arrays_nd.contains_key(n)
            && !self.module.dynamic_arrays.contains(n)
            && !self.module.queue_vars.contains(n))
        .then_some(depth)
    }

    /// §7.4.1: the select depth at which the class property `e` names
    /// reaches a named signed element type, from the property's declaration
    /// in its class.
    fn class_prop_named_signed_depth(&self, e: &Expression) -> Option<usize> {
        let dt = match &e.kind {
            // A bare property of the running method's object.
            ExprKind::Ident(h) if h.path.len() == 1 => {
                let name = h.path[0].name.name.as_str();
                if self
                    .local_stack
                    .last()
                    .is_some_and(|f| f.contains_key(name))
                {
                    return None;
                }
                let th = self.this_stack.last().copied().flatten()?;
                let class = &self.heap.get(th)?.as_ref()?.class_name;
                let base = class.split('#').next().unwrap_or(class).trim();
                let owners = self.prop_owners(base);
                let (_, cd) = owners.get(name)?;
                cd.property_types.get(name)?.clone()
            }
            _ => self.lvalue_static_type(e)?,
        };
        super::bytecode::signed_named_packed_elem_depth(&dt, &self.module.typedef_types)
    }

    /// Declared expression signedness without executing conditional arms or
    /// calls. Context-determined operands need this type before evaluation.
    #[inline(never)]
    pub(in crate::compiler::simulator) fn expression_signed_hint(
        &mut self,
        expr: &Expression,
    ) -> Option<bool> {
        fn joint(left: Option<bool>, right: Option<bool>) -> Option<bool> {
            if left == Some(false) || right == Some(false) {
                Some(false)
            } else if left == Some(true) && right == Some(true) {
                Some(true)
            } else {
                None
            }
        }
        match &expr.kind {
            ExprKind::Number(n) => Some(self.eval_number(n).is_signed),
            ExprKind::Ident(h) => {
                let hint = self.plain_ident_read(h).map(|v| v.is_signed).or_else(|| {
                    let name = self.resolve_hier_name(h);
                    self.signal_name_to_id
                        .get(name.as_ref())
                        .map(|&id| self.signal_signed[id])
                        .or_else(|| {
                            self.module.var_decl_types.get(name.as_ref()).map(|dt| {
                                super::elaborate::is_type_signed_resolved(
                                    dt,
                                    &self.module.typedef_types,
                                )
                            })
                        })
                        .or_else(|| {
                            self.get_expr_type_name(expr)
                                .and_then(|name| self.concrete_type_signed(&name))
                        })
                });
                if hint.is_none() && h.path.len() >= 2 {
                    return self.member_signed_hint(expr);
                }
                hint
            }
            ExprKind::MemberAccess { .. } => self.member_signed_hint(expr).or_else(|| {
                self.get_expr_type_name(expr)
                    .and_then(|name| self.concrete_type_signed(&name))
            }),
            ExprKind::Paren(e) => self.expression_signed_hint(e),
            ExprKind::Index { .. } => {
                let mut root = expr;
                let mut indices = 0;
                loop {
                    match &root.kind {
                        ExprKind::Index { expr, .. } => {
                            indices += 1;
                            root = expr;
                        }
                        ExprKind::Paren(inner) => root = inner,
                        _ => break,
                    }
                }
                let ExprKind::Ident(h) = &root.kind else {
                    return None;
                };
                if let Some(n) = self.frame_bound_bare_name(h) {
                    // §7.4.1: an element of a named signed type.
                    if self.packed_named_signed_types
                        && self.frame_named_signed_depth(n) == Some(indices)
                    {
                        return Some(true);
                    }
                    return self
                        .get_expr_type_name(expr)
                        .and_then(|name| self.concrete_type_signed(&name));
                }
                let name = self.resolve_hier_name(h);
                let unpacked = if let Some((dims, _)) = self.module.arrays_nd.get(name.as_ref()) {
                    dims.len()
                } else if self.module.arrays_2d.contains_key(name.as_ref()) {
                    2
                } else {
                    usize::from(
                        self.module.arrays.contains_key(name.as_ref())
                            || self.module.dynamic_arrays.contains(name.as_ref())
                            || self.module.queue_vars.contains(name.as_ref())
                            || self.is_associative_array(name.as_ref()),
                    )
                };
                if self.declared_signed_select_depth(name.as_ref()) == Some(indices) {
                    // §7.4.1: an element of a named signed type.
                    return Some(true);
                }
                if indices > unpacked {
                    // More indices can enter a nested queue or associative
                    // row. Only a known scalar signal proves a packed select;
                    // unknown class/local storage must retain its own type.
                    return (unpacked == 0 && self.module.signals.contains_key(name.as_ref()))
                        .then_some(false);
                }
                self.module.var_decl_types.get(name.as_ref()).map(|dt| {
                    super::elaborate::is_type_signed_resolved(dt, &self.module.typedef_types)
                })
            }
            ExprKind::Unary {
                op:
                    UnaryOp::Plus
                    | UnaryOp::Minus
                    | UnaryOp::BitNot
                    | UnaryOp::PreIncr
                    | UnaryOp::PreDecr
                    | UnaryOp::PostIncr
                    | UnaryOp::PostDecr,
                operand,
            } => self.expression_signed_hint(operand),
            ExprKind::Unary { .. } => Some(false),
            ExprKind::Binary {
                op:
                    BinaryOp::ShiftLeft
                    | BinaryOp::ShiftRight
                    | BinaryOp::ArithShiftLeft
                    | BinaryOp::ArithShiftRight
                    | BinaryOp::Power,
                left,
                ..
            } => self.expression_signed_hint(left),
            ExprKind::Binary {
                op:
                    BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Mod
                    | BinaryOp::BitAnd
                    | BinaryOp::BitOr
                    | BinaryOp::BitXor
                    | BinaryOp::BitXnor,
                left,
                right,
            } => {
                let l = self.expression_signed_hint(left);
                let r = self.expression_signed_hint(right);
                joint(l, r)
            }
            ExprKind::Binary { .. } => Some(false),
            ExprKind::Conditional {
                then_expr,
                else_expr,
                ..
            } => {
                let t = self.expression_signed_hint(then_expr);
                let e = self.expression_signed_hint(else_expr);
                joint(t, e)
            }
            ExprKind::Concatenation(_)
            | ExprKind::Replication { .. }
            | ExprKind::RangeSelect { .. }
            | ExprKind::StringLiteral(_) => Some(false),
            ExprKind::Call { func, .. } => self.call_return_type(func).map(|dt| {
                super::elaborate::is_type_signed_resolved(&dt, &self.module.typedef_types)
            }),
            ExprKind::SystemCall { name, .. } if name == "$signed" => Some(true),
            ExprKind::SystemCall { name, .. } if name == "$unsigned" => Some(false),
            _ => self
                .get_expr_type_name(expr)
                .and_then(|name| self.concrete_type_signed(&name)),
        }
    }
}
