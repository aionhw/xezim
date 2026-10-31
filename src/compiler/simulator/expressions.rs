//! Simulator expressions.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    /// Outlined from `eval_expr_ctx` (see `exec_stmt_blocking_assign`): keeps
    /// the dispatcher's stack frame small.
    #[inline(never)]
    pub(super) fn eval_expr_member_access(
        &mut self,
        expr: &Box<Expression>,
        member: &crate::ast::Identifier,
        ctx_width: u32,
    ) -> Value {
        // §18.7 — inside `obj.randomize() with { obj.member … }` the
        // receiver `obj` is a caller-scope handle that no longer evals
        // in the randomized frame, so `obj.member` would read 0 and
        // every prefixed acceptance (`req.addr == start_addr`) failed.
        // When the receiver is the recorded `rand_receiver` of the
        // in-flight call and the randomized object has that member,
        // read the OBJECT's property (mirrors the forcing in
        // `prefixed_self_member`). Pure read — no scope mutation.
        if self.rand_receiver.is_some() {
            if let Some(rn) = Self::plain_ident_name(expr) {
                if self.active_rand_receiver() == Some(&rn) {
                    if let Some(&obj) = self.obj_rng_stack.last() {
                        let v = self
                            .heap
                            .get(obj)
                            .and_then(|o| o.as_ref())
                            .and_then(|i| i.properties.get(&member.name))
                            .cloned();
                        if let Some(v) = v {
                            return v;
                        }
                    }
                }
            }
        }
        // §8.25/§26.3: `pkg::Cls::static_prop` (what `pkg::Cls#(..)::prop`
        // reads once its specialization is active): the class package
        // `pkg` holds.
        if let ExprKind::MemberAccess {
            expr: pkg_e,
            member: cls_m,
        } = &expr.kind
            && let ExprKind::Ident(ph) = &pkg_e.kind
            && ph.path.len() == 1
            && ph.path[0].selects.is_empty()
            && self.module.packages.contains(&ph.path[0].name.name)
        {
            // The class by the name its specialization keys statics with
            // (`P`); the qualified entry only for a class renamed because
            // another package declares the same name.
            let q = format!("{}::{}", ph.path[0].name.name, cls_m.name);
            let spec_named = self
                .current_spec
                .as_ref()
                .is_some_and(|(b, _)| b == &cls_m.name);
            let cls = if (spec_named || !self.module.classes.contains_key(&q))
                && self.module.classes.contains_key(&cls_m.name)
            {
                cls_m.name.clone()
            } else {
                q
            };
            if self.module.classes.contains_key(&cls) {
                // §13.5.1: a no-argument static FUNCTION named without
                // parens (`pkg::C#(..)::type_name`) is a call.
                if self.is_static_method(&cls, &member.name)
                    && self.class_parameterless_function(&cls, &member.name)
                    && let Some(v) = self.exec_static_method(&cls, &member.name, &[])
                {
                    return v;
                }
                if let Some(v) = self.active_spec_param_value(&cls, &member.name) {
                    return v;
                }
                if let Some(v) = self.class_static_get(&cls, &member.name) {
                    return v;
                }
            }
        }
        // §13.4.2: a member of a subroutine-local / formal unpacked
        // struct is owned by the call frame; read it there before any
        // module-scope lookup, mirroring the write path. Otherwise a
        // callee saw an outer scope's same-named variable.
        // §11.5.1: member of an element selected THROUGH a ranged
        // port connection — `p[11].hi` over `.p(arr[15:0])` arrives
        // as MemberAccess{Index{RangeSelect{arr},11},hi}. Rebuild over
        // the plain element select so the flat member paths below
        // resolve (labels pass through a constant part-select).
        if let ExprKind::Index { expr: ib, index } = &expr.kind {
            if let ExprKind::RangeSelect { expr: rs_base, .. } = &ib.kind {
                if let ExprKind::Ident(h) = &rs_base.kind {
                    let nm = self.resolve_hier_name(h);
                    let elemish = self
                        .module
                        .packed_signal_elem_widths
                        .get(&*nm)
                        .is_some_and(|&ew| ew > 1)
                        || self
                            .module
                            .packed_full_dims
                            .get(&*nm)
                            .is_some_and(|d| d.len() > 1);
                    if elemish {
                        let norm = Expression::new(
                            ExprKind::MemberAccess {
                                expr: Box::new(Expression::new(
                                    ExprKind::Index {
                                        expr: rs_base.clone(),
                                        index: index.clone(),
                                    },
                                    expr.span,
                                )),
                                member: member.clone(),
                            },
                            expr.span,
                        );
                        return self.eval_expr_ctx(&norm, ctx_width);
                    }
                }
            }
        }
        // §14.3 clocking-block signal read. Inside a task/function the
        // parser yields MemberAccess for `cb.sig` (module scope yields
        // a flat hier Ident), so this arm needs the same handling.
        if self.clocking_read_possible(expr)
            && let Some(mut segs) = Self::flatten_member_path(expr)
        {
            segs.push(member.name.clone());
            let refs: Vec<&str> = segs.iter().map(|s| s.as_str()).collect();
            if let Some(v) = self.try_clocking_signal_read(&refs) {
                return v;
            }
        }
        // §25.10: member access through an indexed plain virtual-
        // interface array (`vifs[i].data`). Resolve the element alias
        // before generic flattened aggregate handling creates/reads a
        // phantom `vifs[i].data` value.
        if let Some(bound) = self.indexed_iface_alias_for(expr) {
            return self
                .get_signal_value_by_name(&format!("{}.{}", bound, member.name))
                .unwrap_or_else(|| Value::new(1));
        }
        // `ClassName::ENUM_CONST` / `ClassName::static_prop`: when the
        // parser represents a class-scoped name as
        // `MemberAccess(Ident([ClassName]), member)` (notably inside a
        // method body), resolve the member against the class scope
        // BEFORE the object-property paths below — a class name has no
        // runtime instance, so without this `Base::STARTED` reads 0.
        if let ExprKind::Ident(h) = &expr.kind {
            // §9.7: the built-in `process` class's state enum
            // (`process::RUNNING` … `process::KILLED`). `process` is
            // not a USER-declared class (it is without a body, so no
            // `module.classes` entry exists), so the flattened
            // hier-Ident form `process::KILLED` (used at module/initial
            // scope) resolves it through the `process` special-case in
            // the Ident arm, but inside a function/task/method body the
            // parser represents the reference as
            // `MemberAccess(Ident("process"), X)` and it fell through to
            // an object-property read that returned 0 — so
            // `p.status() == process::KILLED` was ALWAYS false and
            // killed (zombie) processes were never cleared. Mirror the
            // flat-Ident handling here.
            if h.path.len() == 1 && h.path[0].name.name == "process" && h.path[0].selects.is_empty()
            {
                if let Some(v) = match member.name.as_str() {
                    "FINISHED" => Some(0u64),
                    "RUNNING" => Some(1u64),
                    "WAITING" => Some(2u64),
                    "SUSPENDED" => Some(3u64),
                    "KILLED" => Some(4u64),
                    _ => None,
                } {
                    return Value::from_u64(v, 32);
                }
            }
            if h.path.len() == 1
                && h.path[0].selects.is_empty()
                && self.module.classes.contains_key(&h.path[0].name.name)
            {
                if let Some(v) = self.class_scoped_const(&h.path[0].name.name, &member.name) {
                    return v;
                }
            }
        }
        // §8.23: `outer_c::inner_c::I` in a subroutine body — a constant of a
        // NESTED class, registered under its scoped name.
        if let ExprKind::MemberAccess { .. } = &expr.kind
            && Self::member_chain_root(expr).is_some_and(|r| self.module.classes.contains_key(r))
            && let Some(cls) = Self::expr_scoped_chain_key(expr)
            && self.module.classes.contains_key(&cls)
            && let Some(v) = self.class_scoped_const(&cls, &member.name)
        {
            return v;
        }
        if let ExprKind::Ident(h) = &expr.kind {
            // §26.3: the same shape with a PACKAGE base — `pkg::MEMBER`.
            // The elaborator only collapses `MemberAccess` into a
            // two-segment `Ident` for always/initial/continuous-assign
            // code, so inside a function, task, or class method the
            // reference arrives here instead. Nothing below resolves a
            // package base, and the arm's terminal fallback treats it as
            // an object handle — so every package-qualified enum
            // constant read inside a subroutine returned 0.
            if h.path.len() == 1 && h.path[0].selects.is_empty() {
                if let Some(v) = self.package_enum_member(&h.path[0].name.name, &member.name) {
                    return v;
                }
                // §26.3: a package PARAMETER/localparam through the
                // same shape (`p1::step` in a program initial or a
                // subroutine body — ivtest sv_package2). Qualified
                // key first so same-named parameters in two packages
                // don't collide through the bare fallback.
                let base = &h.path[0].name.name;
                if self.module.packages.contains(base)
                    && !self.signal_name_to_id.contains_key(base.as_str())
                    && !self.signals.contains_key(base)
                {
                    let qual = format!("{}::{}", base, member.name);
                    if let Some(v) = self.module.parameters.get(&qual) {
                        return v.clone();
                    }
                    if let Some(v) = self.module.parameters.get(&member.name) {
                        return v.clone();
                    }
                    // §26.3: a package DATA declaration — a `const` or
                    // a plain variable — reached through `pkg::X`.
                    // These are registered under their BARE name as a
                    // signal (the flat-Ident path at module/initial
                    // scope resolves them that way), so neither the
                    // enum nor the parameter route above has an entry
                    // and the reference fell through to read 0 inside
                    // a subroutine body. Read the bare signal; because
                    // the base is a real package (not a caller local),
                    // the lookup honors the package qualification
                    // rather than a same-named subroutine local.
                    if let Some(v) = self.get_signal(&member.name) {
                        return v.clone();
                    }
                }
            }
        }
        // §18.4: member of a struct/union CLASS property — read through
        // the aggregate's storage (a bit slice for a packed struct/
        // union, so a union's members alias). Must precede the flat
        // signal paths below: a stale `<obj>.<prop>.<field>` leaf could
        // otherwise shadow the aliased view.
        if let Some(r) = self.class_agg_member_parts(expr, &member.name) {
            return self.read_class_agg(&r);
        }
        // §7.13: a member of a struct ELEMENT of a class collection
        // (`m[k1][k2].state` — UNPACKED-struct assoc/dyn/queue array).
        // The cells live as `<elemkey>.<member>` signals (the model
        // `assoc[key].field` uses), so read the leader out. Previously
        // this fell through to a phantom flat path that returned 0 —
        // which made UVM's recurrence guard `m_rec[l][r][p].state` read
        // NEVER→ the cycle never terminated and `compare` recursed to
        // exhaustion.
        if let Some((key, su)) = self.coll_assoc_struct_elem(expr) {
            let cell = format!("{}.{}", key, member.name);
            if let Some(v) = self.get_signal_value_by_name(&cell) {
                return v;
            }
            if let Some(w) = self.coll_field_width(&su, &member.name) {
                // §7.8.6: a missing struct element has its member types'
                // defaults; reading it must not insert an associative entry.
                let mut defaults = Vec::new();
                self.struct_leaf_plain_defaults("", &su, 0, &mut defaults);
                return defaults
                    .into_iter()
                    .find(|(n, _)| *n == format!(".{}", member.name))
                    .map(|(_, v)| v)
                    .unwrap_or_else(|| Value::zero(w));
            }
        }
        // Whole unpacked-struct class-property read in MemberAccess
        // shape (`rhs.first` inside a method): assemble the member
        // cells. class_agg_member_parts returns None for the whole
        // property (only field accesses match), so this catches it.
        if let Some(v) = self.try_read_whole_struct_class_prop(expr) {
            return v;
        }
        // §7.3.2 tagged-union member read (`t.Valid`).
        if let ExprKind::Ident(h) = &expr.kind {
            if h.path.len() == 1 && self.active_union_tag.contains_key(&h.path[0].name.name) {
                let (base, m) = (h.path[0].name.name.clone(), member.name.clone());
                if let Some(v) = self.tagged_union_member(&base, &m) {
                    return v;
                }
            }
        }
        // `item.field` / `i.field` / `item.inner.field` inside a `with`
        // filter over a struct queue. Fold the whole receiver chain,
        // then swap the `item` (or custom iterator) prefix for the
        // element's flat name.
        if let Some(alias) = self.item_alias.clone() {
            if let Some(base_flat) = self.flat_member_name(expr) {
                let head = self.locator_iter.as_str();
                if base_flat == head || base_flat.starts_with(&format!("{}.", head)) {
                    let rest = &base_flat[head.len()..];
                    let name = format!("{}{}.{}", alias, rest, member.name);
                    if let Some(v) = self.get_signal_value_by_name(&name) {
                        return v;
                    }
                }
            }
        }
        // Mirror of the unpacked-aggregate write path in `assign_value`:
        // a flattened leaf (`arr[i].m`, `c.nodes[i].m`) reads its own
        // signal; a nested PACKED member (`arr[i].tag.vlan`) slices its
        // parent's signal. Packed scalars keep the bit-slice paths below.
        if let Some(base_flat) = self.flat_member_name(expr) {
            // An element of an instance-scoped collection stores its
            // leaves under `<h>#slices[0].path`, not the flattened
            // `c.slices[0].path` — resolve the container through the
            // handle path (same rule as the write arm) before the
            // flat lookups, else the read fell to a phantom zero.
            let base_flat = match base_flat.rfind('[') {
                Some(cut) => {
                    let cont = base_flat[..cut].to_string();
                    // §8.10: inside a method a bare property name is the
                    // object's store even when a package variable of the
                    // same name is registered under the bare name.
                    let member_store = (!cont.is_empty()
                        && (self.module.dynamic_arrays.contains(&cont)
                            || self.module.arrays.contains_key(&cont)
                            || self.module.associative_arrays.contains_key(&cont))
                        && self.this_stack.last().copied().flatten().is_some())
                    .then(|| self.instance_assoc_member(&cont))
                    .flatten();
                    if let Some(sc) = member_store {
                        format!("{}{}", sc, &base_flat[cut..])
                    } else if !cont.is_empty()
                        && !self.module.dynamic_arrays.contains(&cont)
                        && !self.module.arrays.contains_key(&cont)
                        && !self.module.associative_arrays.contains_key(&cont)
                        && self
                            .get_signal_value_by_name(&format!("{}.{}", base_flat, member.name))
                            .is_none()
                    {
                        let sc = self.resolve_locator_storage(&cont);
                        if sc != cont {
                            format!("{}{}", sc, &base_flat[cut..])
                        } else {
                            // A struct ELEMENT of a member array inside a
                            // collection element (`q[0].rows[1].m`): the
                            // collection is the first select's base.
                            match base_flat.find('[').filter(|&f| f < cut) {
                                Some(f) => {
                                    let sc = self.resolve_locator_storage(&base_flat[..f]);
                                    if sc != base_flat[..f] && self.queue_elem_struct(&sc).is_some()
                                    {
                                        format!("{}{}", sc, &base_flat[f..])
                                    } else {
                                        base_flat
                                    }
                                }
                                None => base_flat,
                            }
                        }
                    } else {
                        base_flat
                    }
                }
                None => base_flat,
            };
            let flat = format!("{}.{}", base_flat, member.name);
            // A dotted read from a hinted scope (`core.seq` inside a
            // class method of an object built in `u_w`, issue #155):
            // `a.b` parses as a MemberAccess, so the identifier
            // resolver's hint walk never saw it and the flat name
            // missed. Try it under the hint and each parent scope.
            // The plain spelling is read once; after a miss only a hit under
            // the hint (or a parent scope) is read again.
            let hit = match self.get_signal_value_by_name(&flat) {
                Some(v) => Some(v),
                None => {
                    let mut found: Option<String> = None;
                    {
                        let hint = self.name_resolve_hint.borrow();
                        if let Some(hint) = hint.as_deref() {
                            let mut scope = hint;
                            let mut scoped = String::new();
                            loop {
                                scoped.clear();
                                scoped.push_str(scope);
                                scoped.push('.');
                                scoped.push_str(&flat);
                                if self.signal_name_to_id.contains_key(scoped.as_str())
                                    || self.signals.contains_key(&scoped)
                                {
                                    found = Some(scoped);
                                    break;
                                }
                                match scope.rsplit_once('.') {
                                    Some((p, _)) => scope = p,
                                    None => break,
                                }
                            }
                        }
                    }
                    found.and_then(|scoped| self.get_signal_value_by_name(&scoped))
                }
            };
            // A leaf may live in the compact table OR the runtime map (a
            // LOCAL unpacked-struct array element is only in the latter),
            // so consult both — `sa[1].a` read 0 while `%p` showed 2.
            if let Some(mut v) = hit {
                // §7.10.1: an unknown leaf may belong to a queue element
                // past the end.
                if v.has_xz()
                    && let Some(d) = self.missing_leaf_default(expr, &member.name, &base_flat)
                {
                    return d;
                }
                // §7.2.1: a 2-state member of a 4-state packed struct.
                if v.has_xz()
                    && !self.mixed_packed_vars.is_empty()
                    && self.packed_member_two_state(&base_flat, &member.name)
                {
                    v = v.to_two_state();
                }
                // §6.11.1: a collection element's member leaf carries no
                // declared metadata — stamp the member's declared
                // signedness so `q[0].latency <= 0` on an `integer`
                // member compares signed (it read -3 as 4294967293 and
                // a FIFO whose latency went negative never drained).
                if !v.is_real {
                    if let Some(cont) = base_flat
                        .rfind('[')
                        .map(|i| &base_flat[..i])
                        .filter(|c| !c.is_empty())
                    {
                        if let Some(su) = self.queue_elem_struct(cont) {
                            for m in &su.members {
                                if m.declarators.iter().any(|d| d.name.name == member.name) {
                                    v.is_signed = super::elaborate::is_type_signed(&m.data_type);
                                    break;
                                }
                            }
                        }
                    }
                }
                return v;
            }
            if let Some(fields) = self.module.packed_struct_fields.get(&base_flat).cloned() {
                if let Some((_, off, w)) =
                    fields.iter().find(|(m, _, _)| *m == member.name).cloned()
                {
                    if let Some(cur) = self.get_local_or_signal(&base_flat) {
                        let mut out = Value::zero(w.max(1));
                        for i in 0..w {
                            out.set_bit(i as usize, cur.get_bit((off + i) as usize));
                        }
                        // §7.2.1: a 2-state member of a 4-state struct.
                        if out.has_xz()
                            && !self.mixed_packed_vars.is_empty()
                            && self.packed_member_two_state(&base_flat, &member.name)
                        {
                            out = out.to_two_state();
                        }
                        return out;
                    }
                }
            }
        }
        // FALLBACK: check if `base.member` is a separate signal
        // (xezim's unpacked-struct member signal scheme for unpacked
        // structs) stored in `self.signals`. Catches dynamically named
        // leaves (e.g. queue/assoc members) not present in
        // `signal_name_to_id`. (From `nettype`.)
        {
            let base_name = match &expr.kind {
                ExprKind::Ident(h) => Some(self.resolve_hier_name(h).clone()),
                ExprKind::Index { expr: base, .. } => {
                    if let ExprKind::Ident(h) = &base.kind {
                        Some(self.resolve_hier_name(h).clone())
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(base) = base_name {
                let dotted = format!("{}.{}", base, member.name);
                if let Some(v) = self.signals.get(&dotted).cloned() {
                    return v;
                }
            }
        }
        // Handle MemberAccess where the base evaluates to a struct Value
        // (e.g., Tsum(...).field1 or driver[i].field1). We need to
        // extract the field from the returned value using the struct
        // layout (packed_struct_fields[base_name]). (From `nettype`:
        // covers struct values returned by calls / nettype resolver
        // functions, which have no flattened leaf signal.)
        // A nonexistent struct element reads its member defaults.
        if let Some(v) = self.missing_struct_elem_member(expr, &member.name) {
            return v;
        }
        {
            let base_val = self.eval_expr(expr);
            // §7.2.1/§8.4: an ELEMENT of a fixed rand/class array whose
            // element type is a PACKED struct (`o.arr[0].a`, `cfg[x][y].f`)
            // — the element is one integral value at `<h>#arr[i]`; the
            // field is a bit-slice of it. The Index arm below only serves
            // an Ident base, so a class-member array read returned 0.
            if let ExprKind::Index { .. } = &expr.kind {
                if let Some(su) = self.packed_struct_elem_type(expr) {
                    let (fields, total) = self.packed_agg_layout(&su);
                    if total == base_val.width {
                        if let Some((_, off, w)) =
                            fields.iter().find(|(m, _, _)| m == &member.name).cloned()
                        {
                            return base_val.range_select((off + w - 1) as usize, off as usize);
                        }
                    }
                }
            }
            // §8.24 / §7.2.1: member select on a CALL result —
            // directly (`make().tag`, `obj.read_payload().tag`) or
            // through nested members (`mk().n.hi`) — projects the
            // field from the callee's declared return type. The paths
            // below cannot serve this: `struct_field_layout` only
            // describes UNPACKED structs, and a method callee is a
            // MemberAccess the Ident-only lookups never match — so the
            // member read fell through and returned 0 even though the
            // call's whole value was correct. `base_val` is already
            // the inner chain's projected slice (this arm recursed to
            // produce it), so only the base's TYPE is resolved here.
            if matches!(
                &expr.kind,
                ExprKind::Call { .. } | ExprKind::MemberAccess { .. }
            ) {
                if let Some(su) = self.chain_base_packed_struct(expr) {
                    let (fields, total) = self.packed_agg_layout(&su);
                    if total == base_val.width {
                        if let Some((_, off, w)) =
                            fields.iter().find(|(m, _, _)| m == &member.name).cloned()
                        {
                            let mut v = base_val.range_select((off + w - 1) as usize, off as usize);
                            // §6.6.7 / §7.2.1: `packed_agg_layout` is a
                            // 3-tuple with no is_real, so a `real`
                            // member projected out of a struct VALUE
                            // (a call result — notably a nettype
                            // resolution function's return) came back
                            // as the raw f64 BIT PATTERN: 1.5 read as
                            // 4609434218613702656. Restore is_real
                            // from the member's declared type.
                            let mreal = su.members.iter().any(|m| {
                                m.declarators.iter().any(|d| d.name.name == member.name)
                                    && matches!(
                                        Self::resolve_type_ref(
                                            &m.data_type,
                                            &self.module.typedef_types
                                        ),
                                        DataType::Real { .. }
                                    )
                            });
                            if mreal {
                                v.is_real = true;
                            }
                            return v;
                        }
                    }
                }
            }
            // Try to find struct layout for the base.
            // For `Ident.field`: look up packed_struct_fields[Ident]
            // For `Index.field`: look up packed_struct_fields[base_arr_name]
            // For `Call.field`: try the function's return type layout
            let struct_base_name: Option<std::borrow::Cow<str>> = match &expr.kind {
                ExprKind::Ident(h) => Some(self.resolve_hier_name(h)),
                ExprKind::Index { expr: base, .. } => {
                    if let ExprKind::Ident(h) = &base.kind {
                        Some(self.resolve_hier_name(h))
                    } else {
                        None
                    }
                }
                // For Call base, try the function's return type name
                ExprKind::Call { func, .. } => {
                    if let ExprKind::Ident(h) = &func.kind {
                        // Look up function return type from packed_struct_fields
                        // (we register it there for nettype resolver functions)
                        let fn_name = h.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
                        Some(std::borrow::Cow::Owned(fn_name.to_string()))
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(base_name) = struct_base_name {
                // A variable that names a CLASS OBJECT is never a packed/
                // unpacked struct: its `base.member` is a heap property, not
                // a struct-field splice. Skip the layout path entirely even if
                // a stale flat `var_typedef_types` entry (a same-named struct
                // in an unrelated caller scope, e.g. the RAL predictor's
                // `uvm_reg_bus_op rw` vs the reg's `uvm_reg_item rw` formal)
                // poisoned the lookup. Without this, `rw.status` on a class
                // handle is read/spliced as the stale struct's field and a
                // class-handle member write is clobbered.
                let is_class_base = self.class_of_var(&*base_name).is_some();
                // First try packed_struct_fields directly (packed structs:
                // 3-tuple without is_real). Augment is_real by resolving
                // the base's struct typedef members. Then fall back to
                // `struct_field_layout` for unpacked structs.
                let real_fields: std::collections::HashMap<String, bool> = {
                    // Resolve the base's element/member type to a struct
                    // typedef and map field name → is_real.
                    let dt_opt = self
                        .module
                        .typedef_types
                        .get(&*base_name)
                        .cloned()
                        .or_else(|| {
                            self.var_typedef_types
                                .get(&*base_name)
                                .cloned()
                                .and_then(|tn| self.module.typedef_types.get(tn.as_str()).cloned())
                        })
                        .or_else(|| {
                            self.signal_name_to_id
                                .get(base_name.as_ref())
                                .and_then(|id| self.signal_type_names.get(id).cloned())
                                .and_then(|tn| self.module.typedef_types.get(tn.as_str()).cloned())
                        });
                    let mut m = std::collections::HashMap::new();
                    if let Some(dt) = dt_opt {
                        let resolved = Self::resolve_type_ref(&dt, &self.module.typedef_types);
                        if let Some(fs) = Self::struct_field_layout(&resolved) {
                            for (fn_, _, _, fr) in fs {
                                m.insert(fn_, fr);
                            }
                        }
                    }
                    m
                };
                let from_packed = self
                    .module
                    .packed_struct_fields
                    .get(&*base_name)
                    .cloned()
                    .map(|fs| {
                        fs.into_iter()
                            .map(|(n, o, w)| {
                                let fr = *real_fields.get(&n).unwrap_or(&false);
                                (n, o, w, fr)
                            })
                            .collect()
                    });
                let fields_opt: Option<Vec<(String, u32, u32, bool)>> = from_packed
                    .or_else(|| {
                        // For Call base: resolve the function's return type
                        if let ExprKind::Call { func, .. } = &expr.kind {
                            if let ExprKind::Ident(h) = &func.kind {
                                let fn_name = h.path.last()?.name.name.as_str();
                                let fd = self.module.functions.get(fn_name)?;
                                let resolved = Self::resolve_type_ref(
                                    &fd.return_type,
                                    &self.module.typedef_types,
                                );
                                Self::struct_field_layout(&resolved)
                            } else {
                                None
                            }
                        } else if let Some(type_name) = self.var_typedef_types.get(&*base_name) {
                            // Local variable/parameter with typedef type
                            let dt = self.module.typedef_types.get(type_name.as_str())?;
                            let resolved = Self::resolve_type_ref(dt, &self.module.typedef_types);
                            Self::struct_field_layout(&resolved)
                        } else {
                            // Module-level signal with a typedef'd struct
                            // type: resolve via signal_name_to_id →
                            // signal_type_names → typedef_types.
                            let sig_tn = self
                                .signal_name_to_id
                                .get(base_name.as_ref())
                                .and_then(|id| self.signal_type_names.get(id).cloned());
                            if let Some(tn) = sig_tn {
                                let dt = self.module.typedef_types.get(tn.as_str())?;
                                let resolved =
                                    Self::resolve_type_ref(dt, &self.module.typedef_types);
                                Self::struct_field_layout(&resolved)
                            } else {
                                // Try resolving base_name as a typedef
                                let dt = self.module.typedef_types.get(&*base_name)?;
                                let resolved =
                                    Self::resolve_type_ref(dt, &self.module.typedef_types);
                                Self::struct_field_layout(&resolved)
                            }
                        }
                    })
                    // A class-object variable is never struct-spliced: skip the
                    // stale-layout path even if a same-named struct in a caller
                    // scope poisoned `var_typedef_types` (see `is_class_base`).
                    .filter(|_| !is_class_base);
                if let Some(fields) = fields_opt {
                    // Only trust this layout if it actually describes
                    // `base_val` (total width matches). Otherwise we'd
                    // slice a field out of a value that doesn't carry it
                    // — e.g. an unpacked-aggregate element whose members
                    // live in separate flattened signals, for which
                    // `eval_expr(expr)` returns a narrow value. Let it
                    // fall through to the flat-signal / later handlers.
                    let layout_w: u32 = fields.iter().map(|(_, _, w, _)| *w).sum();
                    if layout_w == base_val.width {
                        if let Some((_, off, w, fr)) = fields
                            .iter()
                            .find(|(m, _, _, _)| m == &member.name)
                            .cloned()
                        {
                            let mut v = base_val.range_select((off + w - 1) as usize, off as usize);
                            if fr {
                                v.is_real = true;
                            }
                            return v;
                        }
                    }
                }
                // Last resort: scan all typedef_types for a struct layout
                // matching the base_val width (handles queue elements
                // where the element type isn't directly registered for
                // the queue variable name).
                let base_h = base_val.to_u64().unwrap_or(0) as usize;
                let is_heap_obj =
                    base_h != 0 && base_h < self.heap.len() && self.heap[base_h].is_some();
                if !is_heap_obj {
                    if let Some((off, w, fr)) =
                        self.typedef_layout_member(base_val.width, &member.name)
                    {
                        let mut v = base_val.range_select((off + w - 1) as usize, off as usize);
                        if fr {
                            v.is_real = true;
                        }
                        return v;
                    }
                }
            }
        }
        // LRM §15.5.3: `<event>.triggered` — returns 1 iff the
        // named event was triggered in the same simulation time
        // slot as this query. `EventTrigger` stamps
        // `event_triggered_time[name] = self.time`.
        if member.name == "triggered" {
            if let Some(fired) = self.event_triggered_now(expr) {
                return if fired {
                    Value::ones(1)
                } else {
                    Value::zero(1)
                };
            }
            if matches!(expr.kind, ExprKind::Ident(_)) {
                return Value::zero(1);
            }
        }
        // §9.7 `process::status()`: return the state enum. Previously
        // this was a stub that always returned RUNNING for non-heap
        // handles. Now delegates to `proc_status` which tracks the
        // real lifecycle state from the scheduler queues.
        if member.name == "status" {
            let h = self.eval_expr(expr).to_u64().unwrap_or(0);
            if let Some(pid) = Self::proc_handle_to_pid(h) {
                return Value::from_u64(self.proc_status(pid) as u64, 32);
            }
            // Not a process handle — fall through to property access.
        }
        // `ClassName::static_prop` — explicit static property read.
        if let ExprKind::Ident(hier) = &expr.kind {
            if hier.path.len() == 1 {
                // §8.25: the base may be a class TYPE PARAMETER
                // (e.g. `T.prop` resolved from `T::prop` inside a
                // parameterized-class method). Resolve it to the
                // concrete bound class first.
                let eff_cls: String = self
                    .resolve_type_param_binding(&hier.path[0].name.name)
                    .unwrap_or_else(|| hier.path[0].name.name.clone());
                let cls = &eff_cls;
                if self.module.classes.contains_key(cls)
                    && !self.local_stack.last().is_some_and(|m| m.contains_key(cls))
                    && !self.signal_name_to_id.contains_key(cls.as_str())
                    && !self.signals.contains_key(cls)
                {
                    // `Class::method` with NO parens invokes a
                    // parameterless STATIC function (LRM §13.4.1) —
                    // the MemberAccess twin of the flat-2-segment
                    // `Class::method` handling in the Ident arm. UVM's
                    // `TYPE::type_name` (in `uvm_misc`) parses as
                    // MemberAccess{`TYPE`, type_name}; the previous
                    // `class_static_get` only read static PROPERTIES,
                    // so the call came back empty instead of invoking
                    // `comp_b::type_name()`.
                    if self.is_static_method(cls, &member.name)
                        && self.class_parameterless_function(cls, &member.name)
                    {
                        if let Some(v) = self.exec_static_method(cls, &member.name, &[]) {
                            return v;
                        }
                    }
                    if let Some(v) = self.active_spec_param_value(cls, &member.name) {
                        return v;
                    }
                    if let Some(v) = self.class_static_get(cls, &member.name) {
                        return v;
                    }
                } else if !self.module.classes.contains_key(cls) {
                    // §13.5.1/§8.25: the prefix is a type parameter bound to
                    // a SPECIALIZATION or a typedef of one (UVM 2017's
                    // `Tregistry::type_name`); it names no class key, so
                    // resolve it with its specialization active.
                    let nm = &hier.path[0].name.name;
                    if self.class_member_names().scope_prefixes.contains(nm)
                        && !self.local_stack.last().is_some_and(|m| m.contains_key(nm))
                        && !self.signal_name_to_id.contains_key(nm.as_str())
                        && !self.signals.contains_key(nm)
                        && let Some(v) = self.class_scope_member_value(nm, &member.name)
                    {
                        return v;
                    }
                }
            }
        }
        // Local-stack dotted lookup: e.g. "item.index" inside a with-clause
        if let ExprKind::Ident(hier) = &expr.kind {
            if hier.path.len() == 1 {
                let base_name = &hier.path[0].name.name;
                let dotted = format!("{}.{}", base_name, member.name);
                if let Some(locals) = self.local_stack.last() {
                    if let Some(v) = locals.get(&dotted) {
                        return v.clone();
                    }
                }
            }
        }
        // Check if base_name is a local struct variable (read its value
        // via packed_struct_fields offsets).
        if let ExprKind::Ident(hier) = &expr.kind {
            if hier.path.len() == 1 {
                let base_name = &hier.path[0].name.name;
                if !self.local_stack.is_empty() {
                    if let Some(locals) = self.local_stack.last() {
                        if let Some(struct_val) = locals.get(base_name) {
                            if let Some(fields) =
                                self.module.packed_struct_fields.get(base_name).cloned()
                            {
                                if let Some((_, off, w)) =
                                    fields.iter().find(|(m, _, _)| m == &member.name).cloned()
                                {
                                    return struct_val
                                        .range_select((off + w - 1) as usize, off as usize);
                                }
                            }
                        }
                    }
                }
            }
        }
        // Associative-array element struct-member read:
        // `assoc[key].field` (element = unpacked struct). Mirror of the
        // assign-side handler — reads the dotted signal
        // `assoc[keystr].field`. Guarded by signal presence, so a
        // class-handle element (`rtab[nm].method`/`.prop`) where no such
        // signal exists correctly falls through to the handle path.
        if let ExprKind::Index {
            expr: idx_base,
            index,
        } = &expr.kind
        {
            if let ExprKind::Ident(hier) = &idx_base.kind {
                let mut an = self.resolve_hier_name(hier);
                if let Some(scoped) = self.instance_assoc_member(&an) {
                    an = std::borrow::Cow::Owned(scoped);
                } else if !self.is_associative_array(&an)
                    && !self.module.dynamic_arrays.contains(&*an)
                    && !self.module.arrays.contains_key(&*an)
                {
                    // Through an object handle (`c.slices[0].path`).
                    an = std::borrow::Cow::Owned(self.resolve_locator_storage(&an));
                }
                if self.is_associative_array(&an) {
                    let idx_val = self.eval_expr(index);
                    let key_str = self.assoc_key_str(&an, &idx_val);
                    let target = format!("{}[{}].{}", an, key_str, member.name);
                    if let Some(v) = self.get_signal_value_by_name(&target) {
                        return v;
                    }
                } else if (self.module.dynamic_arrays.contains(&*an)
                    || self.module.arrays.contains_key(&*an))
                    && self.queue_elem_struct(&an).is_some()
                {
                    // Same leaf read for a DYNAMIC/fixed collection of
                    // unpacked structs reached through an instance
                    // scope (`<h>#slices[0].path`) — the bare
                    // flattened name misses the storage, so a
                    // class-member `slices[t].path` read x while the
                    // write (fixed above) landed correctly.
                    let i = self.eval_expr(index).to_u64().unwrap_or(0);
                    let target = format!("{}[{}].{}", an, i, member.name);
                    if let Some(v) = self.get_signal_value_by_name(&target) {
                        return v;
                    }
                }
            }
        }
        // SV-2023: `arr[idx...].outer.inner...field` for N-dim
        // arrays of packed-struct elements with chained MemberAccess
        // — walks BOTH the field path (collecting dotted name) and
        // the index chain, then resolves via the dotted key in
        // `packed_struct_fields[arr_name]`. Mirror of assign_value.
        {
            let mut field_path: Vec<String> = vec![member.name.clone()];
            let mut cur_expr: &Expression = expr;
            loop {
                match &cur_expr.kind {
                    ExprKind::MemberAccess { expr: b, member: m } => {
                        field_path.push(m.name.clone());
                        cur_expr = b.as_ref();
                    }
                    _ => break,
                }
            }
            field_path.reverse();
            let mut indices: Vec<u64> = Vec::new();
            let root_name: Option<String> = loop {
                match &cur_expr.kind {
                    ExprKind::Index { expr: b, index } => {
                        indices.push(self.eval_expr(index).to_u64().unwrap_or(0));
                        cur_expr = b.as_ref();
                    }
                    // Same bare-name fallback as the write side: a
                    // procedural local's layout is keyed bare, and a
                    // submodule scope hint would otherwise turn
                    // `a[0].p` into an unknown `u.a`.
                    ExprKind::Ident(h) => {
                        let scoped = self.resolve_hier_name(h);
                        if !self.module.packed_struct_fields.contains_key(&*scoped)
                            && h.path.len() == 1
                            && h.path[0].selects.is_empty()
                            && self
                                .module
                                .packed_struct_fields
                                .contains_key(&h.path[0].name.name)
                        {
                            break Some(h.path[0].name.name.clone());
                        }
                        break Some(scoped.to_string());
                    }
                    // §7.4.2: root may itself be a struct-member path
                    // (`main.sub_list[0].f`) — mirror of assign_value.
                    ExprKind::MemberAccess { .. } => {
                        let mut segs: Vec<String> = Vec::new();
                        let mut c = cur_expr;
                        while let ExprKind::MemberAccess { expr: b, member: m } = &c.kind {
                            segs.push(m.name.clone());
                            c = b.as_ref();
                        }
                        if let ExprKind::Ident(h) = &c.kind {
                            segs.push(self.resolve_hier_name(h).into_owned());
                            segs.reverse();
                            break Some(segs.join("."));
                        }
                        break None;
                    }
                    _ => break None,
                }
            };
            if let Some(arr_name) = root_name {
                if !indices.is_empty() {
                    indices.reverse();
                    if let Some(fields) = self.module.packed_struct_fields.get(&arr_name).cloned() {
                        let field_key = field_path.join(".");
                        if let Some((_, off, w)) =
                            fields.iter().find(|(m, _, _)| m == &field_key).cloned()
                        {
                            let mut elem_name = arr_name.clone();
                            for i in &indices {
                                elem_name.push_str(&format!("[{}]", i));
                            }
                            if let Some(sig) = self.get_signal_value_by_name(&elem_name) {
                                return sig.range_select((off + w - 1) as usize, off as usize);
                            }
                            // §7.4.2: PACKED array of packed structs —
                            // `arr[i]` is a bit SLICE of one backing
                            // vector, not a signal of its own, so the
                            // `elem_name` lookup above finds nothing.
                            // The write path already had this fallback;
                            // without it here the read silently returned
                            // the default, so a correctly-stored value
                            // read back as 0 (and `$bits` as 32).
                            //
                            // `struct_w` comes from the field layout, NOT
                            // `packed_signal_elem_widths[arr_name]`: that
                            // is the width of an element of the OUTERMOST
                            // dimension, which for `t [0:0][1:0]` is the
                            // whole `[1:0]` sub-array rather than one
                            // struct.
                            let struct_w =
                                fields.iter().map(|(_, o, fw)| o + fw).max().unwrap_or(0);
                            let dims = self.module.packed_full_dims.get(&arr_name).cloned();
                            // A package parameter (`p::ENC[f].man_bits` in a
                            // class method reaches here as `p.ENC`) is no
                            // signal: read its value from the parameters.
                            if struct_w > 0
                                && let Some(slot) =
                                    Self::flatten_packed_slot(&indices, dims.as_deref())
                                && let Some(sig) =
                                    self.get_local_or_signal(&arr_name).or_else(|| {
                                        self.module
                                            .parameters
                                            .get(&arr_name)
                                            .or_else(|| {
                                                self.module
                                                    .parameters
                                                    .get(&arr_name.replacen('.', "::", 1))
                                            })
                                            .cloned()
                                    })
                            {
                                let lo = slot * struct_w + off;
                                if lo + w <= sig.width {
                                    return sig.range_select((lo + w - 1) as usize, lo as usize);
                                }
                            }
                        }
                    }
                }
            }
        }
        if let ExprKind::Ident(hier) = &expr.kind {
            let name = self.resolve_hier_name(hier);
            if let Some(fields) = self.module.packed_struct_fields.get(&*name).cloned() {
                if let Some((_, off, w)) =
                    fields.iter().find(|(m, _, _)| m == &member.name).cloned()
                {
                    if let Some(sig) = self.get_local_or_signal(&name) {
                        let mut v = sig.range_select((off + w - 1) as usize, off as usize);
                        if self.struct_member_signed(&name, &member.name) {
                            v.is_signed = true;
                        }
                        return v;
                    }
                }
            }
            if self.is_associative_array(&name) {
                let mname = member.name.as_str();
                if mname == "size" || mname == "num" {
                    let count = self.assoc_top_level_keys(&name).len();
                    return Self::int_count(count as u64);
                }
                if mname == "delete" {
                    let prefix = format!("{}[", name);
                    let keys: Vec<String> = self.signals.keys_with_elem_prefix(&prefix);
                    for k in keys {
                        self.signals.remove(&k);
                    }
                    return Value::zero(32);
                }
                let qualified = format!("{}.{}", name, mname);
                if let Some(v) = self.lookup_signal_value(&qualified) {
                    return v;
                }
                return Value::zero(32);
            }
            // `.size`/`.num` as a no-parens member access on a class
            // queue / dynamic-array / associative-array property — e.g.
            // `return q.size` inside a method, or a constraint operand
            // `q.size inside {…}`. `resolve_hier_name` yields the bare
            // member name for a class property, so the assoc block above
            // (and the module-level queue/dynamic-array checks) miss it;
            // resolve the instance-scoped `<h>#<name>` storage first.
            // §7.5.2 (dynamic), §7.9.1 (assoc `.num`/`.size`),
            // §7.10.2.1 (queue).
            {
                let mname = member.name.as_str();
                if mname == "size" || mname == "num" {
                    if let Some(scoped) = self.instance_assoc_member(&name) {
                        // Associative array: count live keys
                        // (`<h>#<name>[<key>` prefix).
                        if self.is_associative_array(&scoped) {
                            let count = self.assoc_top_level_keys(&scoped).len();
                            return Self::int_count(count as u64);
                        }
                        // Queue / dynamic array: read `.size` signal.
                        if self.module.dynamic_arrays.contains(&scoped)
                            || self.signals.contains_key(&format!("{}.size", scoped))
                        {
                            return Self::int_count(self.get_queue_size(&scoped));
                        }
                    }
                    // Module-level queue/dynamic/fixed array —
                    // unless the name is a class-typed receiver in
                    // the CURRENT scope (§8.10): then the user
                    // method must dispatch, not a same-named global.
                    if !self.bare_receiver_is_class_handle(&name)
                        && (self.module.arrays.contains_key(&*name)
                            || self.module.dynamic_arrays.contains(&*name)
                            || self.signals.contains_key(&format!("{}.size", name)))
                    {
                        return Self::int_count(self.get_queue_size(&name));
                    }
                }
            }
        }
        // LRM §25.8: virtual-interface member VALUE read `vif.member`
        // (MemberAccess shape, base is `this.vif`). eval(vif) returns the
        // binding-existence sentinel (1), not a heap handle, so the
        // properties path below misses and returns 0 — even though the
        // @(posedge vif.sig) EDGE path resolves the binding correctly
        // (via resolve_hier_name). Resolve the binding here and read the
        // bound interface signal directly (e.g. `vif.rst_n`).
        if let ExprKind::Ident(vh) = &expr.kind {
            if vh.path.len() == 1 {
                let seg0 = &vh.path[0].name.name;
                // (a) virtual-interface alias — a formal arg in this
                // call frame (LRM §25.9: `task f(virtual if av);
                // ...av.sig`) or a plain `virtual <iface>` variable.
                if let Some(bound) = self.iface_alias_for(seg0) {
                    let resolved = format!("{}.{}", bound, member.name);
                    if let Some(v) = self.lookup_signal_value(&resolved) {
                        return v;
                    }
                }
                // (b) `this.vif` class property binding.
                if let Some(Some(this_h)) = self.this_stack.last().copied() {
                    let is_vif = self
                        .heap
                        .get(this_h)
                        .and_then(|o| o.as_ref().map(|i| i.class_name.clone()))
                        .is_some_and(|cn| self.class_vif_decl(&cn, seg0).is_some());
                    if is_vif {
                        if let Some((bound, _mp)) = self
                            .virtual_iface_bindings
                            .get(&(this_h, seg0.clone()))
                            .cloned()
                        {
                            let resolved = format!("{}.{}", bound, member.name);
                            if let Some(v) = self.lookup_signal_value(&resolved) {
                                return v;
                            }
                        }
                    }
                }
            }
        }
        // Unpacked-struct member access where the member was pre-registered
        // as its own signal (e.g. `t_unpacked_struct.val_r`). Prefer
        // reading the member signal so VPI-forced values stay in
        // sync with the SV-visible value.
        if let ExprKind::Ident(hier) = &expr.kind {
            let base_name = self.resolve_hier_name(hier);
            let qualified = format!("{}.{}", base_name, member.name);
            if let Some(v) = self.lookup_signal_value(&qualified) {
                return v;
            }
        }
        // A member of an indexed PACKED aggregate (`n.wdata[0].amask`):
        // the paths above resolve a struct base only from an `Ident` or
        // `Index{Ident}`, so a receiver that is itself a nested select
        // fell through to the class-handle fallback and read zero(32).
        // Last, so every established lookup keeps priority.
        if let Some((sig, lsb, w)) = self.packed_member_slice(expr, &member.name) {
            if let Some(v) = self.get_signal_value_by_name(&sig) {
                return v.range_select((lsb + w - 1) as usize, lsb as usize);
            }
        }
        let base = self.eval_expr(expr);
        let handle = base.to_u64().unwrap_or(0) as usize;
        // §8.9: a STATIC property is shared and instance-independent —
        // it lives in `class_statics`, not per-instance storage, and is
        // readable even through a null handle (`obj.static_prop` before
        // `obj = new`). Route to the shared cell. class_static_get
        // returns None for a non-static member, so ordinary reads fall
        // through unchanged.
        let member_static = self
            .class_member_names()
            .statics
            .contains(member.name.as_str());
        if member_static {
            let (var, resolved) = match &expr.kind {
                ExprKind::Ident(h) if h.path.len() == 1 => (
                    Some(h.path[0].name.name.clone()),
                    Some(self.resolve_hier_name(h).into_owned()),
                ),
                ExprKind::MemberAccess { .. } | ExprKind::Ident(_) if handle == 0 => {
                    // §8.9: a static through a null property.
                    if let Some((o, p)) = self.handle_chain_owner(expr) {
                        if let Some((cn, spec)) =
                            self.null_property_static_target(o, &p, &member.name)
                        {
                            if let Some(v) = self.static_read_in(&cn, spec, &member.name) {
                                return v;
                            }
                        }
                    }
                    (None, None)
                }
                _ => (None, None),
            };
            let live = if handle < self.heap.len() { handle } else { 0 };
            if let Some(v) =
                self.static_read_via_handle(live, var.as_deref(), resolved.as_deref(), &member.name)
            {
                return v;
            }
        }
        // A class constant (localparam) through the reference; a static
        // property was resolved above, by the reference's declared type.
        let static_class: Option<String> = if member_static {
            None
        } else if handle != 0 && handle < self.heap.len() {
            self.heap[handle].as_ref().map(|i| i.class_name.clone())
        } else if let ExprKind::Ident(h) = &expr.kind {
            let n = self.resolve_hier_name(h);
            self.class_of_var(&n)
        } else {
            None
        };
        if let Some(cn) = static_class {
            // Header value parameters are per instance. They also sit in
            // `param_defaults`, so `class_static_get` answers them from the
            // elaborated default (`int D = H` is 32) and used to hide the
            // specialization stored on the object. Localparams and `static`
            // members are in `static_properties` and stay on the shared cell.
            if self.member_is_static_prop(&cn, &member.name) {
                if let Some(v) = self.class_static_get(&cn, &member.name) {
                    return v;
                }
            }
        }
        if handle == 0 || handle >= self.heap.len() {
            // §8.4: reading an instance property through a null handle
            // is a runtime FATAL (the reference simulator aborts) —
            // silently yielding 0/x lets a dead testbench keep "passing".
            if let ExprKind::Ident(h) = &expr.kind {
                let n = self.resolve_hier_name(h);
                if self.null_deref_fatal(&n, &member.name) {
                    return Value::new(32);
                }
            }
            Value::zero(32)
        } else {
            let prop = self.heap[handle]
                .as_ref()
                .and_then(|i| i.properties.get(&member.name).cloned())
                .or_else(|| {
                    let prefix = format!("{}.", member.name);
                    self.heap[handle]
                        .as_ref()
                        .and_then(|i| i.properties.get(&format!("{}m_type", prefix)).cloned())
                });
            if let Some(v) = prop {
                return v;
            }
            // SV §13.4.1: `obj.f` with no parens calls a
            // no-argument function `f`. The member matched no
            // property, so dispatch to a same-named parameterless
            // function if the class declares one (e.g. `if (port.size < 1)`).
            let cls = self.heap[handle].as_ref().map(|i| i.class_name.clone());
            match cls {
                Some(cn) if self.class_bare_method(&cn, &member.name) => {
                    let res = self.exec_method_call(handle, &member.name, &[]);
                    self.return_flag = false;
                    return res;
                }
                _ => {}
            }
            Value::zero(32)
        }
    }

    /// Read fast path for a select-free single-segment identifier: the frame
    /// local of that name, else the plain property of `this`. Everything the
    /// full path checks before those two reads is keyed on multi-segment or
    /// selected shapes, on an active `ref` redirect, on a static local, or
    /// on the names in `ident_slow` / `shadowed_prop_names`; any of those
    /// (or a miss) returns `None` and the full path runs.
    #[inline]
    pub(super) fn plain_ident_read(&self, h: &HierarchicalIdentifier) -> Option<Value> {
        if h.path.len() != 1 || h.root.is_some() || self.ref_redirect_hot || self.name_stats_on {
            return None;
        }
        let seg = &h.path[0];
        if !seg.selects.is_empty() {
            return None;
        }
        let name = seg.name.name.as_str();
        if self.class_member_names().ident_slow.contains(name) {
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

    /// Read fast path for `h.p`: `h` a name `plain_ident_read` resolves to a
    /// live object, `p` an instance property of it. The member read's other
    /// arms key on names these two miss (vif, struct-capable, static,
    /// parameter, class, package, union-tag, associative), on an interface
    /// alias or clocking signal of that spelling, on a registered layout for
    /// `h`, or on a flat `h.p` frame key or signal (probed, also under the
    /// scope hint); any of those returns `None`.
    pub(super) fn plain_member_read(
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
                    if Self::struct_field_layout(&resolved).is_some() {
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
    pub(super) fn plain_member_call(
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
    pub(super) fn cmp_literal_like(e: &Expression) -> bool {
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
    pub(super) fn cmp_named_ok(&mut self, e: &Expression, other: &Expression) -> bool {
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
    pub(super) fn cmp_name_scalar(&mut self, h: &HierarchicalIdentifier) -> bool {
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
            || self.module.arrays_nd.contains_key(&*r))
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
    pub(super) fn operand_has_string_type(&mut self, e: &Expression) -> bool {
        match &e.kind {
            ExprKind::StringLiteral(_) => false,
            ExprKind::Paren(inner) => self.operand_has_string_type(inner),
            ExprKind::Concatenation(parts) => parts.iter().any(|e| self.operand_has_string_type(e)),
            _ => self.operand_is_string(e),
        }
    }

    pub(super) fn plain_compare(
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
        let sw = match (self.lrm_self_width(left), self.lrm_self_width(right)) {
            (Some(lw), Some(rw)) => lw.max(rw),
            _ => ctx_width,
        };
        // §11.8.2: against a real operand the plain name or literal on the
        // other side is converted from its own value — the comparison
        // operators below convert it.
        let l = self.eval_expr_ctx(left, sw);
        let r = self.eval_expr_ctx(right, sw);
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
    pub(super) fn recv_chain_call(e: &Expression) -> Option<&Expression> {
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
    pub(super) fn recv_call_to_memoize<'e>(&self, e: &'e Expression) -> Option<&'e Expression> {
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
    pub(super) fn recv_call_memo_hit(&self, e: &Expression) -> Option<Value> {
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
    pub(super) fn recv_call_once_eval(
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
    pub(super) fn recv_call_once_assign(&mut self, lhs: &Expression, val: &Value) -> Option<bool> {
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
    pub(super) fn pin_stmt_recv_calls(&mut self, exprs: &[&Expression]) -> Option<usize> {
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
    pub(super) fn hier_parent_handle(
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
            for member in &segs[split..segs.len() - 1] {
                handle = handle.and_then(|h| {
                    let tn = self.class_prop_type_name(h, member)?;
                    self.resolve_typeref_class_name_str(&tn)?;
                    self.member_handle(h, member)
                });
            }
            if let Some(h) =
                handle.filter(|&h| h != 0 && self.heap.get(h).is_some_and(|o| o.is_some()))
            {
                return Some(h);
            }
        }
        None
    }

    /// Evaluate expression with a context width hint (for proper shift sizing).
    /// When ctx_width > 0, shift operators widen their left operand to ctx_width.
    pub fn eval_expr_ctx(&mut self, expr: &Expression, ctx_width: u32) -> Value {
        self.eval_expr_ctx_inner(expr, ctx_width, false)
    }

    pub(super) fn eval_expr_typed(
        &mut self,
        expr: &Expression,
        ctx_width: u32,
        unsigned: bool,
    ) -> Value {
        let mut value = self.eval_expr_ctx_inner(expr, ctx_width, unsigned);
        if unsigned && !value.is_real {
            value.is_signed = false;
        }
        value
    }

    /// Declared expression signedness without executing conditional arms or
    /// calls. Context-determined operands need this type before evaluation.
    #[inline(never)]
    pub(super) fn expression_signed_hint(&mut self, expr: &Expression) -> Option<bool> {
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
            ExprKind::Ident(h) => self.plain_ident_read(h).map(|v| v.is_signed).or_else(|| {
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
                if self.frame_bound_bare_name(h).is_some() {
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

    pub(super) fn eval_expr_ctx_inner(
        &mut self,
        expr: &Expression,
        ctx_width: u32,
        unsigned_context: bool,
    ) -> Value {
        // §18.7 receiver-prefixed reads inside `obj.randomize() with {…}`
        // (see `strip_rand_receiver_expr`).
        if self.rand_receiver.is_some() {
            if let Some(stripped) = self.strip_rand_receiver_expr(expr) {
                return self.eval_expr_ctx(&stripped, ctx_width);
            }
        }
        match &expr.kind {
            ExprKind::Ident(h) => {
                if let Some(v) = self.plain_ident_read(h) {
                    return v;
                }
                if h.path.len() >= 2
                    && !self.local_stack.is_empty()
                    && let Some(v) = self.frame_handle_path_read(expr)
                {
                    return v;
                }
            }
            ExprKind::SystemCall { name, args } if name == SVA_SIG_MARKER => {
                return self.sva_sig_read(args);
            }
            ExprKind::MemberAccess { expr: base, member } => {
                if let Some(v) = self.plain_member_read(base, member) {
                    return v;
                }
                if !self.local_stack.is_empty()
                    && let Some(v) = self.frame_handle_path_read(expr)
                {
                    return v;
                }
                if matches!(base.kind, ExprKind::Index { .. })
                    && let Some(v) = self.read_declared_packed_access(expr)
                {
                    return v;
                }
                if matches!(
                    base.kind,
                    ExprKind::Call { .. } | ExprKind::MemberAccess { .. }
                ) && let Some(v) = self.recv_call_once_eval(expr, ctx_width)
                {
                    return v;
                }
            }
            ExprKind::Call { func, args } => {
                if !self.recv_call_memo.is_empty()
                    && let Some(v) = self.recv_call_memo_hit(expr)
                {
                    return v;
                }
                if let Some(v) = self.plain_member_call(func, args) {
                    return v;
                }
                if matches!(&func.kind, ExprKind::MemberAccess { expr: r, .. }
                    if matches!(r.kind, ExprKind::Call { .. } | ExprKind::MemberAccess { .. }))
                    && let Some(v) = self.recv_call_once_eval(expr, ctx_width)
                {
                    return v;
                }
                if !self.module.lets.is_empty()
                    && let Some(v) = self.let_call_ctx(func, args, ctx_width)
                {
                    return v;
                }
            }
            ExprKind::Index { expr: base, .. } | ExprKind::RangeSelect { expr: base, .. } => {
                if matches!(
                    base.kind,
                    ExprKind::Call { .. } | ExprKind::MemberAccess { .. }
                ) && let Some(v) = self.recv_call_once_eval(expr, ctx_width)
                {
                    return v;
                }
                if (self.needs_declared_packed_access(expr)
                    || self.needs_static_packed_access(expr))
                    && let Some(v) = self.read_declared_packed_access(expr)
                {
                    return v;
                }
            }
            _ => {}
        }
        if let Some(v) = self.assoc_size_member(expr) {
            return v;
        }
        // `pkg::arr[i]` / `pkg::v[h:l]` as an rvalue: the indexed arms resolve
        // their base by name and only for a plain identifier, so under a
        // task/function frame a package-qualified base read 0. Strip the
        // prefix and re-enter, as the identifier spelling does below. Scalar
        // `pkg::x` reads keep their own arm (parameters of two packages must
        // not collide through the bare name).
        //
        // §6.20.2/§26.3 (issue #425): an UNPACKED-ARRAY package localparam
        // keeps its QUALIFIED registration only (`pkg::LIM` plus per-element
        // `pkg::LIM[i]`); stripping the head here resolved the element select
        // to a bare `LIM` that exists nowhere and read 0. Keep the qualified
        // spelling when it names a collection — the indexed arms below
        // resolve the base through `resolve_hier_name`, which returns the
        // qualified key (package variables still hoist bare as before).
        let pkg_qual_collection = match &expr.kind {
            ExprKind::Index { expr: base, .. } | ExprKind::RangeSelect { expr: base, .. } => {
                // §26.3 / §7.4: every select belongs to the qualified
                // collection, including the outer select of a nested index.
                let mut base = base.as_ref();
                while let ExprKind::Index { expr, .. }
                | ExprKind::RangeSelect { expr, .. }
                | ExprKind::Paren(expr) = &base.kind
                {
                    base = expr;
                }
                match &base.kind {
                    ExprKind::Ident(h)
                        if h.path.len() == 2
                            && h.path[0].selects.is_empty()
                            && self.module.packages.contains(&h.path[0].name.name) =>
                    {
                        self.pkg_qual_collection_key(&h.path[0].name.name, &h.path[1].name.name)
                            .is_some()
                    }
                    _ => false,
                }
            }
            _ => false,
        };
        if !self.module.packages.is_empty()
            && matches!(
                expr.kind,
                ExprKind::Index { .. } | ExprKind::RangeSelect { .. }
            )
            && !pkg_qual_collection
        {
            if let Some(stripped) = self.strip_package_lvalue(expr) {
                return self.eval_expr_ctx(&stripped, ctx_width);
            }
        }
        // §6.20.2/§26.3 (issue #425): `(pkg::LIM)[3]` — the Paren wrapper is
        // semantically transparent, but it defeats every base-shape probe
        // above and below (including the package strip), so the select fell
        // through to a bit-select of the whole-array read and returned 0.
        // Unwrap the wrapper for the package-collection shape and re-enter.
        if let ExprKind::Index { expr: base, index } = &expr.kind
            && let ExprKind::Paren(inner) = &base.kind
            && let ExprKind::Ident(h) = &inner.kind
            && h.path.len() == 2
            && h.path[0].selects.is_empty()
            && self.module.packages.contains(&h.path[0].name.name)
            && self
                .pkg_qual_collection_key(&h.path[0].name.name, &h.path[1].name.name)
                .is_some()
        {
            let rewritten = Expression::new(
                ExprKind::Index {
                    expr: inner.clone(),
                    index: index.clone(),
                },
                expr.span,
            );
            return self.eval_expr_ctx(&rewritten, ctx_width);
        }
        // §13.5.2: an indexed / part-selected read through a `ref` formal
        // reads the caller's actual (see `ref_formal_redirect_hier`).
        if matches!(
            expr.kind,
            ExprKind::Index { .. } | ExprKind::RangeSelect { .. }
        ) && self.ref_redirect_possible()
        {
            if let Some(rewritten) = self.ref_formal_redirect_inner(expr, false) {
                return self.eval_expr_ctx(&rewritten, ctx_width);
            }
        }
        // §7.2/§13.4.2: a leaf of an unpacked struct the innermost call frame
        // OWNS — `s.a`, `s.inner.x`, `s.arr[2]`. Read it from the frame, as the
        // write path does; otherwise the lookups below reach module scope and
        // return an unrelated same-named variable's member.
        if let Some(rewritten) = self.super_rewrite(expr) {
            return self.eval_expr_ctx(&rewritten, ctx_width);
        }
        // §25.8/§25.9: any chain rooted at a bound virtual interface reads as
        // a direct access to the bound instance.
        if let Some(rewritten) = self.vif_rebase_expr(expr) {
            return self.eval_expr_ctx(&rewritten, ctx_width);
        }
        if !self.local_stack.is_empty() {
            if let Some(key) = self.frame_leaf_key_of(expr) {
                if let Some(v) = self.frame_get(&key) {
                    return v.clone();
                }
            }
            // §7.4.1/§11.5.1: a nested select of a packed variable the frame
            // owns, placed as its write is (`frame_packed_select_write`).
            if Self::select_of_select(expr)
                && let Some(v) = self.frame_packed_select_read(expr)
            {
                return v;
            }
        }
        // §6.21: a `static` subroutine local is a LIVE cell shared across ALL
        // simultaneous activations — including re-entrant/recursive ones. Read
        // the committed value from `static_local_vars` rather than this
        // recursion window's snapshot frame, so mutations inside an inner
        // activation are instantly visible to the caller's frames (otherwise a
        // recursive phase re-entry / method would reset the static to its
        // per-window initialiser and livelock). Skipped during elaboration-time
        // constant-function evaluation, where §13.4.3 makes each such call
        // independent.
        if !self.in_const_param_eval && !self.static_local_names.is_empty() {
            if let ExprKind::Ident(h) = &expr.kind {
                if h.path.len() == 1 && h.path[0].selects.is_empty() {
                    let n = &h.path[0].name.name;
                    if n.as_bytes()
                        .first()
                        .is_some_and(|&b| self.static_local_first[b as usize])
                        && self.static_local_names.contains(n.as_str())
                        && let Some(k) = self.static_local_key_for(n)
                    {
                        if self.static_local_keys.contains(&k)
                            && let Some(v) = self.static_local_vars.get(&k).cloned()
                        {
                            return v;
                        }
                    }
                }
            }
        }
        // §7.2/§18.4: a leaf of an unpacked-struct CLASS PROPERTY at any
        // depth — `o.n.inner.a`, `o.s.arr[1]`, `o.arr[0].a`. The member-access
        // read arm resolves only ONE level below the property, and the ident
        // arm sees just the base of an index, so anything deeper fell through
        // to a bit-select or a stale module-scope leaf. A part-select
        // (`o.pkt.mem[1][255:248]`) rides along: the same select-aware walk
        // resolves the element first.
        if matches!(
            expr.kind,
            ExprKind::Index { .. } | ExprKind::MemberAccess { .. } | ExprKind::RangeSelect { .. }
        ) && !self.no_class_objects()
            && Self::chain_maybe_class_agg(
                expr,
                self.this_stack.last().copied().flatten().is_some(),
            )
        {
            if let Some(r) = self.class_agg_member(expr) {
                return self.read_class_agg(&r);
            }
        }
        // §7.2/§18.4: a WHOLE unpacked-struct class property read as a value —
        // `return s;` from a method, `v = obj.s;`. Its members are per-instance
        // cells with no container, so the lookups further down returned the
        // property's unused scalar cell (zero) instead of the members.
        if !self.no_class_objects() {
            if let ExprKind::Ident(h) = &expr.kind {
                if (h.path.len() >= 2 || self.this_stack.last().copied().flatten().is_some())
                    && h.path.iter().all(|p| p.selects.is_empty())
                    && h.path
                        .last()
                        .is_some_and(|p| self.struct_prop_name_possible(&p.name.name))
                {
                    if let Some((handle, prop)) = self.class_prop_receiver(expr) {
                        if let Some(su) = self.class_prop_struct(handle, &prop) {
                            if Self::spreads_member_wise(&su) {
                                if let Some(v) = self.pack_class_unpacked(handle, &prop, &su) {
                                    return v;
                                }
                            }
                        }
                    }
                }
            }
        }
        // §13.4.1: a member of a call result whose return type is an unpacked
        // struct. Gated on the chain actually ending in a call, so nothing else
        // pays for the walk.
        if matches!(
            expr.kind,
            ExprKind::MemberAccess { .. } | ExprKind::Index { .. }
        ) && Self::chain_has_call_base(expr)
        {
            if let Some(v) = self.eval_unpacked_call_member(expr) {
                return v;
            }
        }
        // §13.4.1: a nested class's bare static function uses exactly the
        // same dispatch as its parenthesized call, retaining the full scope
        // and specialization before the value-read path strips wrappers.
        if let ExprKind::MemberAccess { expr: recv, member } = &expr.kind
            && !matches!(&recv.kind, ExprKind::Ident(h) if h.path.len() == 1)
            && self
                .class_member_names()
                .static_methods
                .contains(&member.name)
            && let Some(root) = Self::class_scope_root(recv)
            && (self.module.classes.contains_key(root) || self.module.packages.contains(root))
            && !self
                .local_stack
                .last()
                .is_some_and(|f| f.contains_key(root))
            && !self.signal_name_to_id.contains_key(root)
            && let Some(scope) = Self::class_scope_chain(recv)
            && let Some((base, _)) = self.resolve_class_scope_prefix(&scope)
            && self.is_static_method(&base, &member.name)
            && self.class_parameterless_function(&base, &member.name)
        {
            return self.eval_call(expr, &[]);
        }
        // A parameterized-class specialization (`C#(...)`) in value context
        // evaluates as its base reference — strip the `Specialization`
        // wrapper(s) from the member/ident chain so resolution sees the same
        // shape as the pre-`Specialization` AST. The `#(...)` text only matters
        // for per-spec static keying (handled at the static-dispatch sites).
        // Keeps behavior identical to before.
        if Self::expr_has_specialization(expr) {
            // Set the active specialization (for per-spec static keying) while
            // resolving the stripped shape, e.g. `C#(params)::prop` reads the
            // per-spec static cell. Restore afterward.
            //
            // Keep `current_spec` set to the enclosing spec while resolving
            // the call's specialization: `resolve_call_spec_params` resolves
            // class-local typedef args (e.g. `special_comp_type`) through
            // `resolve_typedef_spec`, which reads `self.current_spec`.
            let saved = self.current_spec.clone();
            let lexical = self.lexical_spec();
            let extracted = self.resolve_call_spec_params(Self::extract_call_spec(expr), &lexical);
            if let Some((ref base, ref sig)) = extracted {
                self.ensure_spec_statics(base, sig);
            }
            self.current_spec = extracted.or(saved.clone());
            let v = self.eval_expr_ctx(&Self::strip_spec_shape(expr), ctx_width);
            self.current_spec = saved;
            return v;
        }
        match &expr.kind {
            ExprKind::Number(num) => {
                let v = self.eval_number(num);
                // §5.7.1: an UNSIZED based literal whose leftmost digit is x
                // or z extends with x/z (not 0) to the assignment context —
                // `reg [63:0] a = 'hx3x2x1x0` is xxxxxxxxx3x2x1x0 (ivtest
                // sv_packed_port2: the zero-extension made undriven struct
                // elements read 00 and drove ~00=ff where the reference
                // keeps x). Sized literals already extend inside
                // eval_number; only the context-driven widening is handled
                // here.
                if ctx_width > v.width {
                    if let crate::ast::expr::NumberLiteral::Integer {
                        size: None,
                        base:
                            crate::ast::expr::NumberBase::Hex
                            | crate::ast::expr::NumberBase::Octal
                            | crate::ast::expr::NumberBase::Binary,
                        value,
                        ..
                    } = num
                    {
                        let lead = value.chars().find(|c| *c != '_');
                        if let Some(c @ ('x' | 'X' | 'z' | 'Z' | '?')) = lead {
                            let fill = if matches!(c, 'x' | 'X') {
                                Value::all_x(ctx_width - v.width)
                            } else {
                                Value::all_z(ctx_width - v.width)
                            };
                            return Value::concat(&[fill, v]);
                        }
                    }
                }
                v
            }
            ExprKind::StringLiteral(s) => {
                // IEEE 1800-2017 §6.16: A string literal is a packed array of
                // bytes (8 bits per character).  An empty string "" has width
                // 0 (zero characters), *not* 8.  `Value::from_string` also
                // maps chars above 0x7F to their Latin-1 byte — one byte per
                // char, matching the §5.9.1 escape decoder — where the old
                // per-`s.bytes()` loop counted UTF-8 bytes, so `"\xf1"` was
                // 16 bits wide and truncated to the wrong byte (0xb1).
                Value::from_string(s)
            }
            ExprKind::Ident(hier) => {
                // §14.3: the flat spelling of `cb.sig[i]` (the select on the
                // last segment) on an unpacked-array clockvar.
                if !self.clocking_array_leaves.is_empty()
                    && hier.path.len() >= 2
                    && let Some((last, owner)) = hier.path.split_last()
                    && let [sel] = last.selects.as_slice()
                    && self.clocking_array_leaves.contains(last.name.name.as_str())
                    && owner.iter().all(|s| s.selects.is_empty())
                {
                    let mut base = hier.clone();
                    base.path.last_mut().unwrap().selects.clear();
                    base.cached_signal_id = std::cell::Cell::new(None);
                    base.cached_resolved_name = std::cell::OnceCell::new();
                    let base = Expression::new(ExprKind::Ident(base), expr.span);
                    if let Some(v) = self.clocking_array_elem_read(&base, sel) {
                        return v;
                    }
                }
                // §13.5.2: a selected read through a `ref` formal reads the
                // caller's actual (see `ref_formal_redirect_hier`).
                if self.ref_redirect_possible() {
                    if let Some(nh) = self.ref_formal_redirect_hier(hier, false) {
                        let rewritten = Expression::new(ExprKind::Ident(nh), expr.span);
                        return self.eval_expr_ctx(&rewritten, ctx_width);
                    }
                }
                if hier.path.len() == 3 && hier.path[1].name.name == "type_option" {
                    if let Some((cg, field)) = self.cg_type_option_path(hier) {
                        if let Some(v) = self.cg_type_option_get(&cg, &field) {
                            return v;
                        }
                    }
                }
                if hier.path.len() == 3 && hier.path[1].name.name == "option" {
                    if let Some(idx) = self.cg_option_target(hier) {
                        if let Some(v) = self.cg_option_read(idx, &hier.path[2].name.name) {
                            return v;
                        }
                    }
                }
                // §26.3: a PACKAGE-scoped reference (`P::s.x` = [P, s, x])
                // reads exactly like the bare `s.x` — the prefix carries no
                // hierarchy at runtime, but every shape-keyed branch below
                // (2-segment struct slices, enum members, …) would miss the
                // 3-segment form. Strip the prefix once and re-enter.
                if hier.path.len() >= 2
                    && hier.path[0].selects.is_empty()
                    && self.module.packages.contains(&hier.path[0].name.name)
                    && !self
                        .signal_name_to_id
                        .contains_key(hier.path[0].name.name.as_str())
                    && !self.signals.contains_key(&hier.path[0].name.name)
                    && !self.module.classes.contains_key(&hier.path[0].name.name)
                {
                    // §26.3: `P::x` (len 2) resolved through the BARE-name
                    // fallback, so two packages declaring the same parameter
                    // name collided and the reference read whichever package
                    // elaborated last (`p1::step` gave p2's value — ivtest
                    // sv_package2). Prefer the package's own qualified
                    // registration; the bare fallback still covers everything
                    // that has no qualified entry.
                    if hier.path.len() == 2 && hier.path[1].selects.is_empty() {
                        let qual =
                            format!("{}::{}", hier.path[0].name.name, hier.path[1].name.name);
                        if let Some(v) = self.module.parameters.get(&qual) {
                            return v.clone();
                        }
                    }
                    // Keep `P::x` (len 2) on the existing paths — those work
                    // via the bare-name fallbacks; only deeper paths need the
                    // reshape.
                    if hier.path.len() >= 3 {
                        let mut stripped = hier.clone();
                        stripped.path.remove(0);
                        // §26.3: a contested package variable's own storage.
                        if let Some(q) =
                            self.pkg_var_storage(&hier.path[0].name.name, &hier.path[1].name.name)
                        {
                            stripped.path[0].name.name = q;
                        }
                        stripped.cached_signal_id = std::cell::Cell::new(None);
                        stripped.cached_resolved_name = std::cell::OnceCell::new();
                        let e2 = Expression::new(ExprKind::Ident(stripped), expr.span);
                        return self.eval_expr_ctx(&e2, ctx_width);
                    }
                }
                // §13.4.1: `obj.method` with NO parens invokes a parameterless
                // function. The MemberAccess shape (`obj.method`) already
                // dispatches it; the FLATTENED 2-segment `Ident([obj, method])`
                // (how `t1.randomize & t2.randomize` parses) fell through to a
                // property read and returned 0 WITHOUT running the function —
                // so UVM's `assert(t1.randomize & t2.randomize)` never
                // randomized `t1`/`t2` (fields stayed 0). Mirror the
                // MemberAccess dispatch: if the base is a live class handle
                // whose type declares a parameterless function `method`, call
                // it and use its return. Guarded to the 2-segment flat form so
                // struct-member / enum / signal-path reads keep their own arms.
                if hier.path.len() == 2
                    && hier.path[1].selects.is_empty()
                    && hier.path[0].selects.is_empty()
                    && self.bare_name_is_class_channel(&hier.path[0].name.name)
                {
                    let base_name = hier.path[0].name.name.as_str();
                    let member = hier.path[1].name.name.clone();
                    if let Some(handle) = self.eval_ident_handle(base_name) {
                        if handle != 0 {
                            let cls = self
                                .heap
                                .get(handle)
                                .and_then(|o| o.as_ref())
                                .map(|i| i.class_name.clone());
                            if let Some(cn) = cls {
                                if self.class_bare_method(&cn, &member) {
                                    let r = self.exec_method_call(handle, &member, &[]);
                                    self.return_flag = false;
                                    return r;
                                }
                            }
                        }
                    }
                }
                // §18.4: `<obj>.<agg_prop>.<member>` / `<agg_prop>.<member>`
                // that parsed as a FLAT hier ident — same aggregate-class-
                // property storage as the `MemberAccess` shape (see there).
                if hier.path.len() >= 2 && !self.no_class_objects() {
                    if let Some(r) = self.class_agg_member(expr) {
                        return self.read_class_agg(&r);
                    }
                    // Whole unpacked-struct class-property read (`c.first`
                    // where `first` is a struct): assemble the member cells.
                    if let Some(v) = self.try_read_whole_struct_class_prop(expr) {
                        return v;
                    }
                }
                // §7.3.2 tagged-union member read (`t.Valid`).
                if hier.path.len() == 2 {
                    let base = hier.path[0].name.name.as_str();
                    if self.active_union_tag.contains_key(base) {
                        let (base, member) = (base.to_string(), hier.path[1].name.name.clone());
                        if let Some(v) = self.tagged_union_member(&base, &member) {
                            return v;
                        }
                    }
                }
                // `item.field` / `i.field` inside a `with` filter over a struct
                // queue. `item_alias` aliases the current element's flat name;
                // the head may be the literal `item` or a custom iterator name.
                if hier.path.len() >= 2 {
                    if let Some(alias) = self.item_alias.clone() {
                        if hier.path[0].name.name == "item"
                            || hier.path[0].name.name == self.locator_iter
                        {
                            let rest = hier.path[1..]
                                .iter()
                                .map(|s| s.name.name.as_str())
                                .collect::<Vec<_>>()
                                .join(".");
                            if let Some(v) =
                                self.get_signal_value_by_name(&format!("{}.{}", alias, rest))
                            {
                                return v;
                            }
                        }
                    }
                }
                // Unpacked-struct member path (e.g. `t_unpacked_struct.val_r`)
                // is pre-registered as a separate signal by elaboration.
                // Prefer the separate-signal read so VPI-forced values stay
                // in sync with the SV-visible value. Skip short idents and
                // the dotted name `Class::static_prop` (handled below).
                // §13.4.2: a member of a SUBROUTINE-LOCAL struct lives in the
                // call frame under its dotted name — that is exactly where the
                // write path puts it. The global lookup just below has no frame
                // check, so a callee whose local shares a bare name with any
                // process-block local elsewhere read back THAT variable's
                // member and its own write appeared to vanish: a function
                // writing `s.a = 8'h32` returned the caller's `s.a`. The frame
                // check further down never ran, because this returned first.
                if hier.path.len() > 1 && hier.path.iter().all(|s| s.selects.is_empty()) {
                    if let Some(locals) = self.local_stack.last() {
                        let dotted = hier
                            .path
                            .iter()
                            .map(|s| s.name.name.as_str())
                            .collect::<Vec<_>>()
                            .join(".");
                        if let Some(v) = locals.get(&dotted) {
                            return v.clone();
                        }
                    }
                }
                // §8.9: a static property through a handle, null or not,
                // names the shared cell of the handle's declared class.
                if hier.path.len() == 2
                    && hier.path[1].selects.is_empty()
                    && !self.module.classes.is_empty()
                    && self
                        .class_member_names()
                        .statics
                        .contains(hier.path[1].name.name.as_str())
                {
                    let mut head = hier.path[0].clone();
                    head.selects.clear();
                    let head_id = HierarchicalIdentifier {
                        root: hier.root.clone(),
                        path: vec![head],
                        span: hier.span,
                        cached_signal_id: std::cell::Cell::new(None),
                        cached_resolved_name: std::cell::OnceCell::new(),
                    };
                    let resolved = self.resolve_hier_name(&head_id).into_owned();
                    let obj = &hier.path[0].name.name;
                    let h = if hier.path[0].selects.is_empty() {
                        self.eval_ident_handle(obj)
                            .filter(|&h| h != 0 && h < self.heap.len())
                            .unwrap_or(0)
                    } else {
                        0
                    };
                    if let Some(v) = self.static_read_via_handle(
                        h,
                        Some(obj),
                        Some(&resolved),
                        &hier.path[1].name.name,
                    ) {
                        return v;
                    }
                }
                if hier.path.len() == 2 {
                    // A variable holding a live object shadows any signal or
                    // instance of the same name: `core.n` with a local `core`
                    // read x when a sibling instance was also called `core`,
                    // because the dotted signal lookup ran first and answered
                    // for the instance.
                    if hier.path[0].selects.is_empty()
                        && hier.path[1].selects.is_empty()
                        && !self.no_class_objects()
                        && self.bare_name_is_class_channel(&hier.path[0].name.name)
                    {
                        if let Some(hval) = self.eval_ident_handle(&hier.path[0].name.name) {
                            if hval != 0 {
                                if let Some(v) = self
                                    .heap
                                    .get(hval)
                                    .and_then(|o| o.as_ref())
                                    .and_then(|inst| inst.properties.get(&hier.path[1].name.name))
                                {
                                    return v.clone();
                                }
                            }
                        }
                    }
                    let dotted = format!("{}.{}", hier.path[0].name.name, hier.path[1].name.name);
                    if let Some(v) = self.get_signal_value_by_name(&dotted) {
                        return v;
                    }
                    // §4c: `h.prop` where `h` is a MODULE-SCOPE class-handle
                    // variable. No composed signal exists (the hit above wins
                    // for real instance paths), so resolve the head to its
                    // heap object and read the property — previously this
                    // fell through and returned x.
                    if hier.path[0].selects.is_empty()
                        && hier.path[1].selects.is_empty()
                        && self.bare_name_is_class_channel(hier.path[0].name.name.as_str())
                    {
                        let head = hier.path[0].name.name.as_str();
                        let field = hier.path[1].name.name.as_str();
                        if let Some(hval) = self.eval_ident_handle(head) {
                            if hval != 0 {
                                if let Some(v) = self
                                    .heap
                                    .get(hval)
                                    .and_then(|o| o.as_ref())
                                    .and_then(|inst| inst.properties.get(field))
                                {
                                    return v.clone();
                                }
                            }
                        }
                    }
                }
                // LRM §14.3 clocking-block input read — `cb.<sig>` returns
                // the snapshot taken at the block's most recent clock edge.
                // `w.r.c` with three or more segments: a handle chain. The
                // two-segment case above reads the property off the head
                // handle; longer chains had no such path and fell through to
                // hierarchical-name resolution, which inside a sub-instance
                // applied the instance scope hint and read x (the same chain
                // parenthesised, `(w.r).c`, or split in two steps was fine).
                // Walk the handles property by property; any miss falls
                // through to the existing resolution unchanged.
                if hier.path.len() >= 3
                    && hier.path.iter().all(|s| s.selects.is_empty())
                    && !self.no_class_objects()
                    && self.bare_name_is_class_channel(hier.path[0].name.name.as_str())
                {
                    let head = hier.path[0].name.name.as_str();
                    let head_handle = if head == "this" {
                        self.this_stack.last().copied().flatten()
                    } else {
                        self.eval_ident_handle(head)
                    };
                    if let Some(mut h) = head_handle {
                        let mut ok = h != 0;
                        let last = hier.path.len() - 1;
                        // (owner, property) when the final hop holds null.
                        let mut null_in: Option<(usize, &str)> = None;
                        for (k, seg) in hier.path[1..last].iter().enumerate() {
                            if !ok {
                                break;
                            }
                            if k == last - 2 {
                                null_in = Some((h, seg.name.name.as_str()));
                            }
                            match self
                                .heap
                                .get(h)
                                .and_then(|o| o.as_ref())
                                .and_then(|inst| inst.properties.get(&seg.name.name))
                                .and_then(|v| v.to_u64())
                            {
                                Some(next)
                                    if next != 0
                                        && self
                                            .heap
                                            .get(next as usize)
                                            .is_some_and(|o| o.is_some()) =>
                                {
                                    h = next as usize;
                                }
                                _ => ok = false,
                            }
                        }
                        if !ok {
                            // §8.9: a static through a null property is the
                            // property's declared class's shared cell.
                            if let Some((owner, prop)) = null_in {
                                let m = &hier.path[last].name.name;
                                if let Some((cn, spec)) =
                                    self.null_property_static_target(owner, prop, m)
                                {
                                    if let Some(v) = self.static_read_in(&cn, spec, m) {
                                        return v;
                                    }
                                }
                            }
                        }
                        if ok {
                            // §8.9: a static at the end of the chain is the
                            // class's shared cell.
                            if let Some(v) = self.static_read_via_handle(
                                h,
                                None,
                                None,
                                &hier.path[last].name.name,
                            ) {
                                return v;
                            }
                            if let Some(v) =
                                self.heap.get(h).and_then(|o| o.as_ref()).and_then(|inst| {
                                    inst.properties.get(&hier.path[last].name.name)
                                })
                            {
                                return v.clone();
                            }
                        }
                    }
                }
                if hier.path.len() >= 2 {
                    let segs: Vec<&str> = hier.path.iter().map(|s| s.name.name.as_str()).collect();
                    if let Some(v) = self.try_clocking_signal_read(&segs) {
                        return v;
                    }
                }
                // LRM §15.5.3: `e.triggered` on a named event.
                // `e.triggered` parses as Ident(path=[e, triggered]); match
                // here BEFORE generic signal lookup so we don't get back X.
                if hier.path.len() >= 2
                    && hier.path.last().map(|s| s.name.name.as_str()) == Some("triggered")
                {
                    let head = hier_prefix(hier);
                    let head_expr = Expression::new(ExprKind::Ident(head), expr.span);
                    let fired = self.event_triggered_now(&head_expr).unwrap_or(false);
                    return if fired {
                        Value::ones(1)
                    } else {
                        Value::zero(1)
                    };
                }
                // §6.19.6: enum methods written WITHOUT parentheses parse as a
                // hierarchical Ident, e.g. `color1.num` -> path=[color1, num].
                // Route to the enum-method evaluator when the prefix is an
                // enum-typed variable (guarded so a real hier signal whose leaf
                // happens to be one of these names is unaffected).
                if hier.path.len() >= 2
                    && hier
                        .path
                        .last()
                        .map(|s| s.selects.is_empty())
                        .unwrap_or(false)
                {
                    let mname = hier.path.last().unwrap().name.name.clone();
                    if matches!(
                        mname.as_str(),
                        "num" | "first" | "last" | "next" | "prev" | "name"
                    ) {
                        let head = hier_prefix(hier);
                        if head
                            .path
                            .last()
                            .map(|s| s.selects.is_empty())
                            .unwrap_or(false)
                        {
                            let bare = head.path.last().unwrap().name.name.clone();
                            let is_enum_var = self
                                .type_name_of_var(&bare)
                                .map(|tn| self.module.enum_members.contains_key(&tn))
                                .unwrap_or(false)
                                || self.module.enum_members.contains_key(&bare)
                                // A PROCEDURAL local's enum typedef is only in
                                // the typedef overlay/global —
                                // `type_name_of_var` reads var_class_types and
                                // the signal tables, none of which know it. So
                                // `e_t l; l.name` fell through to a
                                // hierarchical read (""), while the same
                                // declaration at module scope worked. Frame
                                // overlay first, as `local_class_type_of`
                                // orders it; this is the map
                                // `enum_typed_receiver` already keys on.
                                || self
                                    .local_typedef_type_of(&bare)
                                    .or_else(|| self.var_typedef_types.get(&bare).cloned())
                                    .is_some_and(|tn| {
                                        self.module.enum_members.contains_key(&tn)
                                    });
                            if is_enum_var {
                                if let Some(v) = self.eval_builtin_method(&bare, &mname, &[]) {
                                    return v;
                                }
                                // `name` has no `BuiltinM` arm: §6.19.5.7's
                                // enum-method evaluator lives on the CALL
                                // path, which already decomposes this exact
                                // multi-segment Ident. A parenless method is
                                // the zero-argument call §13.5.5 says it is,
                                // so dispatch it as one rather than reading
                                // the name as hierarchical storage — that read
                                // returned "" for `.name` at every scope.
                                return self.eval_call_inner(expr, &[]);
                            }
                        }
                    }
                }
                if hier.path.len() == 1 && hier.path[0].selects.is_empty() {
                    let name = &hier.path[0].name.name;
                    if name == "UVM_ACTIVE" {
                        return Value::from_u64(1, 32);
                    }
                    if name == "UVM_PASSIVE" {
                        return Value::from_u64(0, 32);
                    }
                    // §13.5.2: an ALIASED `ref` formal reads the caller's
                    // storage NOW — external writes made while this call was
                    // suspended must be visible (see `ref_alias_stack`). The
                    // storage is read BY NAME, not by re-evaluating the ident:
                    // a formal named like its actual (`task t(ref int foo);
                    // t(foo)`) would otherwise hit the callee frame's own
                    // copy (or recurse into this redirect).
                    if let Some(storage) = self
                        .ref_alias_stack
                        .last()
                        .and_then(|m| m.get(name.as_str()))
                        .cloned()
                    {
                        if let Some(v) = self.get_signal_value_by_name(&storage) {
                            return v;
                        }
                    }
                    if let Some(locals) = self.local_stack.last() {
                        if let Some(val) = locals.get(name) {
                            return val.clone();
                        }
                    }
                    // LRM §25.8: virtual-interface property read as a
                    // scalar value. Without an explicit member, the
                    // property's "value" is the binding existence —
                    // bound → non-zero (the bound instance's hashed
                    // sentinel handle), unbound → 0 (null). Drives the
                    // `vif == null` / `vif != null` idiom.
                    if let Some(Some(handle)) = self
                        .this_stack
                        .last()
                        .filter(|_| self.class_member_names().vif_props.contains(name.as_str()))
                    {
                        // Own or inherited (§8.13), instance or static.
                        let slot = self
                            .heap
                            .get(*handle)
                            .and_then(|o| o.as_ref())
                            .and_then(|i| self.vif_slot(&i.class_name, *handle, name));
                        if let Some((slot, _)) = slot {
                            return if self.virtual_iface_bindings.contains_key(&slot) {
                                Value::from_u64(1, 32)
                            } else {
                                Value::zero(32)
                            };
                        }
                    }
                    // A `static` class property must NOT be read from the
                    // instance's `properties` map: each specialization's
                    // singleton carries a stale construction-time copy that
                    // diverges from the shared static cell. Defer
                    // to the `class_static_get` fallback below. LRM §8.9:
                    // a static property has ONE cell shared across all
                    // instances.
                    // Borrowed walk + borrowed key: this is the bare-name
                    // property read, the most frequent operation in class
                    // code, and it used to allocate three times per read (the
                    // class-context name, one `extends` clone per ancestor
                    // level, and the property key).
                    let is_static_prop = match self.class_context_stack.last() {
                        Some(Some(_))
                            if !self.class_member_names().statics.contains(name.as_str()) =>
                        {
                            false
                        }
                        Some(Some(ctx)) => {
                            let mut cur: Option<&str> = Some(ctx.as_str());
                            let mut found = false;
                            while let Some(cn) = cur {
                                let Some(cd) = self.module.classes.get(cn) else {
                                    break;
                                };
                                if cd.static_properties.contains(name) {
                                    found = true;
                                    break;
                                }
                                cur = cd.extends.as_deref();
                            }
                            found
                        }
                        _ => false,
                    };
                    if !is_static_prop {
                        if let Some(Some(handle)) = self.this_stack.last().copied() {
                            // §8.10: a shadowed property resolves to the copy
                            // the EXECUTING method's class declares, not the
                            // leaf's.
                            let shadowed = self.shadowed_prop_key(name, false);
                            let key: &str = shadowed.as_deref().unwrap_or(name.as_str());
                            // An unpacked-struct property is stored member-wise
                            // (`<prop>.<member>` cells); any container cell is
                            // stale, so assemble the members on a whole read.
                            if let Some(v) = self.read_whole_struct_class_prop(handle, key) {
                                return v;
                            }
                            if self.name_stats_on {
                                self.name_stats[3].set(self.name_stats[3].get() + 1);
                            }
                            if let Some(Some(instance)) = self.heap.get(handle) {
                                if let Some(val) = instance.properties.get(key) {
                                    return val.clone();
                                }
                            }
                        }
                    }
                    // Class VALUE parameter of a specialized class, referenced
                    // bare inside a STATIC method (no instance → not in
                    // `properties`). Resolve from the active specialization
                    // BEFORE the static-property fallback, which would return
                    // the declared default instead (e.g.
                    // `object_registry#(T,"base_class")::get_type_name()`
                    // must return the string param `Tname`, not `<unknown>`).
                    if let Some(v) = self.resolve_value_param_from_spec(name) {
                        return v;
                    }
                    // §6.19: an enum member declared in THIS class (or an
                    // ancestor) is an inner-scope constant and must beat the
                    // design-wide fallback below. Enum members are also
                    // registered under their bare name in one flat namespace,
                    // so without this a same-named member of an unrelated
                    // class — whichever elaborated last — silently won.
                    if let Some(Some(ctx)) = self.class_context_stack.last().cloned() {
                        if let Some(v) = self.class_enum_member(&ctx, name) {
                            return v;
                        }
                    }
                    // Static class property referenced bare inside a method.
                    if let Some(Some(ctx)) = self.class_context_stack.last().cloned() {
                        if let Some(v) = self.class_static_get(&ctx, name) {
                            return v;
                        }
                    }
                    // §13.4.1: a bare PARAMETERLESS INSTANCE method name with
                    // NO parens inside a class method — `count`, `get_action`,
                    // `tag;`. `this` is bound, the name matched no property
                    // (read above) but the executing class's inheritance chain
                    // declares it as a no-argument, non-static function; invoke
                    // it on `this` and use its return. Property / vif / static
                    // reads above all win, so a name that IS a property never
                    // dispatches here.
                    if let Some(Some(this_h)) = self.this_stack.last().copied() {
                        if let Some(inst) = self.heap.get(this_h).and_then(|o| o.as_ref()) {
                            let cname = inst.class_name.clone();
                            if self.class_parameterless_function(&cname, name)
                                && !self.is_static_method(&cname, name)
                            {
                                let r = self.exec_method_call(this_h, name, &[]);
                                self.return_flag = false;
                                return r;
                            }
                        }
                    }
                    // §13.5.1/§8.10: a bare NO-ARGUMENT STATIC function of the
                    // executing class chain (`type_name` inherited from
                    // `holder #(int, "ext")`) is a call as well; it runs
                    // under the specialization the chain binds.
                    // Borrowed test first: this point is reached by every
                    // unresolved bare name in a method.
                    let static_fn = match self.class_context_stack.last() {
                        Some(Some(ctx))
                            if self.class_member_names().static_methods.contains(name) =>
                        {
                            self.is_static_method(ctx, name)
                                && self.class_parameterless_function(ctx, name)
                        }
                        _ => false,
                    };
                    if static_fn && let Some(Some(ctx)) = self.class_context_stack.last().cloned() {
                        let this_h = self.this_stack.last().copied().flatten().unwrap_or(0);
                        let r = if this_h != 0 {
                            self.exec_static_method_via_object(&ctx, name, &[], this_h)
                        } else {
                            self.exec_static_method(&ctx, name, &[])
                        };
                        if let Some(v) = r {
                            self.return_flag = false;
                            return v;
                        }
                    }
                } else if hier.path.len() > 1 {
                    // LRM §25.8: `<obj>.<vif>` external read — if `<obj>`
                    // is a class handle whose class declares `<vif>` as a
                    // virtual interface, return non-zero iff a binding
                    // exists. Drives the `if (d.vif == null)` idiom from
                    // outside the class.
                    if hier.path.len() == 2 {
                        let obj = &hier.path[0].name.name;
                        let prop = &hier.path[1].name.name;
                        let obj_handle = self
                            .local_stack
                            .last()
                            .and_then(|locals| locals.get(obj).cloned())
                            .or_else(|| self.get_signal_value_by_name(obj));
                        if let Some(v) = obj_handle {
                            let handle = v.to_u64().unwrap_or(0) as usize;
                            if handle != 0 && handle < self.heap.len() {
                                // Own or inherited (§8.13), instance or static.
                                let slot = self
                                    .heap
                                    .get(handle)
                                    .and_then(|o| o.as_ref())
                                    .and_then(|i| self.vif_slot(&i.class_name, handle, prop));
                                if let Some((slot, _)) = slot {
                                    return if self.virtual_iface_bindings.contains_key(&slot) {
                                        Value::from_u64(1, 32)
                                    } else {
                                        Value::zero(32)
                                    };
                                }
                            }
                        }
                    }
                    // `ClassName::static_prop` — explicit static property read.
                    // §9.7: the built-in `process` class's state enum
                    // (`process::RUNNING` …). Never user-declared, so the
                    // class lookup below finds nothing and the reference
                    // silently read 0 — `p.status() == process::WAITING`
                    // compared 2 == 0 and was ALWAYS false, even with the
                    // status itself correct.
                    if hier.path.len() == 2 && hier.path[0].name.name == "process" {
                        if let Some(v) = match hier.path[1].name.name.as_str() {
                            "FINISHED" => Some(0u64),
                            "RUNNING" => Some(1),
                            "WAITING" => Some(2),
                            "SUSPENDED" => Some(3),
                            "KILLED" => Some(4),
                            _ => None,
                        } {
                            return Value::from_u64(v, 32);
                        }
                    }
                    // §8.23: a constant of a NESTED class (`outer_c::inner_c::I`):
                    // the class is registered under its scoped name.
                    if hier.path.len() >= 3
                        && hier.path.iter().all(|s| s.selects.is_empty())
                        && self.module.classes.contains_key(&hier.path[0].name.name)
                    {
                        let n = hier.path.len();
                        let cls = hier.path[..n - 1]
                            .iter()
                            .map(|s| s.name.name.as_str())
                            .collect::<Vec<_>>()
                            .join("::");
                        if self.module.classes.contains_key(&cls)
                            && let Some(v) =
                                self.class_scoped_const(&cls, &hier.path[n - 1].name.name)
                        {
                            return v;
                        }
                    }
                    if hier.path.len() == 2 {
                        let prop = &hier.path[1].name.name;
                        // §8.25: the leading segment may be a class TYPE
                        // PARAMETER (e.g. `T::m_type_name` inside a
                        // parameterized-class method). Resolve it
                        // to the concrete bound class via the active
                        // specialization / `this` instance before the
                        // class/typedef lookups below. Without this, `T::prop`
                        // silently read as zero/empty because `T` is not in
                        // `module.classes`.
                        let eff_cls: String = self
                            .resolve_type_param_binding(&hier.path[0].name.name)
                            .unwrap_or_else(|| hier.path[0].name.name.clone());
                        let cls = &eff_cls;
                        // §8.25.1: typedef alias to a parameterized-class
                        // specialization (e.g. `typedef Reg#("foo") FooReg;`)
                        // — resolve the specialization and key the static
                        // cell per-spec. The same holds for a type
                        // parameter bound to a specialization. §13.5.1: a
                        // no-argument static FUNCTION named without parens
                        // is a call, so it runs under that specialization
                        // too.
                        if !self.module.classes.contains_key(cls)
                            && self
                                .class_member_names()
                                .scope_prefixes
                                .contains(&hier.path[0].name.name)
                            && let Some(v) =
                                self.class_scope_member_value(&hier.path[0].name.name, prop)
                        {
                            return v;
                        }
                        if self.module.classes.contains_key(cls)
                            && !self
                                .signal_name_to_id
                                .contains_key(self.resolve_hier_name(hier).as_ref())
                        {
                            // `Class::method` with NO parens invokes a
                            // parameterless STATIC function (LRM §13.4.1: a
                            // no-arg function may be called by name alone).
                            // e.g. UVM's `TYPE::type_name` in `uvm_misc`
                            // where `type_name` is `static function string
                            // type_name();`. Previously the flat 2-segment
                            // ident fell through `class_static_get` (which
                            // only reads static PROPERTIES), so the call
                            // returned garbage instead of invoking
                            // `comp_b::type_name()`. If it's a parameterless
                            // static function, invoke it and return its result.
                            if self.is_static_method(cls, prop)
                                && self.class_parameterless_function(cls, prop)
                            {
                                if let Some(v) = self.exec_static_method(cls, prop, &[]) {
                                    return v;
                                }
                            }
                            if let Some(v) = self.active_spec_param_value(cls, prop) {
                                return v;
                            }
                            if let Some(v) = self.class_static_get(cls, prop) {
                                return v;
                            }
                        }
                    }
                    // Check local stack for dotted names like "item.index"
                    if let Some(locals) = self.local_stack.last() {
                        let dotted = hier
                            .path
                            .iter()
                            .map(|s| s.name.name.as_str())
                            .collect::<Vec<_>>()
                            .join(".");
                        if let Some(val) = locals.get(&dotted) {
                            return val.clone();
                        }
                    }
                    // §8.4: the object's own storage wins over the signal table.
                    // `tb.u_leaf.obj.field` used to be looked up as the flat name
                    // "tb.u_leaf.obj.field", which fell through to the leaf and
                    // returned tb's own `field` signal — the object was never
                    // consulted.  Only taken when the parent really is a live
                    // object AND declares the member, so struct/net paths are
                    // untouched.
                    if let Some(h) = self.hier_parent_handle(hier) {
                        let member_name = hier.path[hier.path.len() - 1].name.name.as_str();
                        let via = hier.path[hier.path.len() - 2].name.name.as_str();
                        let mut mval = self.static_read_via_handle(h, Some(via), None, member_name);
                        if mval.is_none() {
                            mval = self.read_member_value(h, member_name);
                        }
                        if let Some(mval) = mval {
                            return mval;
                        }
                    }
                    // SV-2023: class-handle dereference. If a leading dotted
                    // prefix names a signal storing a class handle (e.g. a
                    // parameterized static like `registry#(byte)::singleton`),
                    // walk into heap[handle].properties for the suffix.
                    {
                        let segs: Vec<&str> =
                            hier.path.iter().map(|s| s.name.name.as_str()).collect();
                        for split in (1..segs.len()).rev() {
                            let dotted = segs[..split].join(".");
                            let bare = segs[split - 1].to_string();
                            let mut candidates: Vec<String> = Vec::new();
                            candidates.push(dotted);
                            if !candidates.contains(&bare) {
                                candidates.push(bare);
                            }
                            let mut handle_value: Option<Value> = None;
                            // id-collision-in-heap: a leading dotted prefix may
                            // steer the read into the heap only when it
                            // statically names a class object (class-typed
                            // root, frame-dotted handle member, class-typed
                            // composed signal, or handle-collection element).
                            // A struct/hierarchical prefix (`ov_r.inner`,
                            // `u2.sv_sub`, `q[0]`) holds integral data, not a
                            // handle — its value must not be read back as one.
                            if self.dotted_prefix_is_class_channel(&segs[..split]) {
                                for c in &candidates {
                                    if let Some(v) = self.get_signal_value_by_name(c) {
                                        handle_value = Some(v);
                                        break;
                                    }
                                }
                            }
                            // `Class::static_prop` prefix — the leading
                            // segment names a class and the prefix's last
                            // segment is one of its static properties.
                            if handle_value.is_none() && self.module.classes.contains_key(segs[0]) {
                                if let Some(v) = self.class_static_get(segs[0], segs[split - 1]) {
                                    handle_value = Some(v);
                                }
                            }
                            if let Some(v) = handle_value {
                                let h = v.to_u64().unwrap_or(0) as usize;
                                // §8.4: reading an instance property through a
                                // NULL declared class handle is a runtime
                                // fatal, not a silent x.
                                if h == 0 && split == segs.len() - 1 {
                                    let prefix = segs[..split].join(".");
                                    if self.null_deref_fatal(&prefix, segs[split]) {
                                        return Value::new(32);
                                    }
                                }
                                if h != 0 && h < self.heap.len() {
                                    if let Some(inst) = self.heap.get(h).and_then(|x| x.as_ref()) {
                                        let mut cur_props = &inst.properties;
                                        let mut cur_handle = h;
                                        let mut found: Option<Value> = None;
                                        let mut sub_handle: Option<usize> = None;
                                        let tail = &segs[split..];
                                        for (i, seg) in tail.iter().enumerate() {
                                            if let Some(val) = cur_props.get(*seg) {
                                                if i + 1 == tail.len() {
                                                    // The leaf may be a whole
                                                    // unpacked-struct property
                                                    // stored member-wise: any
                                                    // container cell is stale,
                                                    // so assemble the member
                                                    // cells from the CURRENT
                                                    // object handle (§10.6/§18.4).
                                                    if let Some(v) = self
                                                        .read_whole_struct_class_prop(
                                                            cur_handle, seg,
                                                        )
                                                    {
                                                        found = Some(v);
                                                    } else {
                                                        found = Some(val.clone());
                                                    }
                                                } else {
                                                    sub_handle =
                                                        Some(val.to_u64().unwrap_or(0) as usize);
                                                }
                                            } else {
                                                break;
                                            }
                                            if let Some(sh) = sub_handle.take() {
                                                if sh == 0 || sh >= self.heap.len() {
                                                    break;
                                                }
                                                if let Some(next_inst) =
                                                    self.heap.get(sh).and_then(|x| x.as_ref())
                                                {
                                                    cur_props = &next_inst.properties;
                                                    cur_handle = sh;
                                                } else {
                                                    break;
                                                }
                                            }
                                        }
                                        if let Some(v) = found {
                                            return v;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    let obj_name = &hier.path[0].name.name;
                    {
                        let sub = hier.path[1..]
                            .iter()
                            .map(|s| s.name.name.as_str())
                            .collect::<Vec<_>>()
                            .join(".");
                        if let Some(fields) =
                            self.module.packed_struct_fields.get(obj_name).cloned()
                        {
                            if let Some((_, off, w)) =
                                fields.iter().find(|(m, _, _)| m == &sub).cloned()
                            {
                                if let Some(sig) = self.get_signal_value_by_name(obj_name) {
                                    let mut v =
                                        sig.range_select((off + w - 1) as usize, off as usize);
                                    // §7.2.1: the member's DECLARED signedness
                                    // (the slice itself is an unsigned bit
                                    // pattern; `int s` printed as unsigned).
                                    if self.struct_member_signed(obj_name, &sub) {
                                        v.is_signed = true;
                                    }
                                    return v;
                                }
                            }
                        }
                    }
                    if hier.path.len() == 2 {
                        let mname = hier.path[1].name.name.as_str();
                        if self.is_associative_array(obj_name) {
                            if mname == "size" || mname == "num" {
                                let count = self.assoc_top_level_keys(obj_name).len();
                                return Self::int_count(count as u64);
                            }
                            if mname == "delete" {
                                let prefix = format!("{}[", obj_name);
                                let keys: Vec<String> = self.signals.keys_with_elem_prefix(&prefix);
                                for k in keys {
                                    self.signals.remove(&k);
                                }
                                return Value::zero(32);
                            }
                        }
                        if mname == "delete" && self.module.arrays.contains_key(obj_name) {
                            self.set_queue_size(obj_name, 0);
                            return Value::zero(32);
                        }
                        // §15.4.2: `mb[i].num()` on an ARRAY OF MAILBOXES. The
                        // ELEMENT holds the handle and the messages live in
                        // `self.mailboxes` keyed by it, but `obj_name` above is
                        // the BARE first segment — the index is dropped — so
                        // the array branch below matched the enclosing array
                        // and reported ITS element count instead of the
                        // mailbox's depth. `num()` then read 0 however many
                        // times a producer had put, making every put look like
                        // it vanished; a scalar mailbox in the same scope was
                        // fine, which is what made this look like a
                        // mailbox-in-instance bug rather than an indexing one.
                        // Resolve the element's own handle first and fall
                        // through only when it is not a mailbox.
                        if mname == "num"
                            && hier.path[0].selects.len() == 1
                            && let Some(idx) = self.eval_scalar_self(&hier.path[0].selects[0])
                            && let Some(hv) =
                                self.get_signal_value_by_name(&format!("{}[{}]", obj_name, idx))
                            && let Some(q) =
                                self.mailboxes.get(&(hv.to_u64().unwrap_or(0) as usize))
                        {
                            return Self::int_count(q.len() as u64);
                        }
                        if (mname == "size" || mname == "num")
                            && self.module.arrays.contains_key(obj_name)
                        {
                            return Self::int_count(self.get_queue_size(obj_name));
                        }
                        if mname == "pop_front" && self.module.arrays.contains_key(obj_name) {
                            return self
                                .eval_builtin_method(obj_name, "pop_front", &[])
                                .unwrap_or(Value::zero(32));
                        }
                        if mname == "pop_back" && self.module.arrays.contains_key(obj_name) {
                            return self
                                .eval_builtin_method(obj_name, "pop_back", &[])
                                .unwrap_or(Value::zero(32));
                        }
                        if matches!(
                            mname,
                            "sort"
                                | "rsort"
                                | "reverse"
                                | "sum"
                                | "product"
                                | "min"
                                | "max"
                                | "unique"
                                | "unique_index"
                                | "find"
                                | "find_first"
                                | "find_last"
                                | "find_index"
                                | "find_first_index"
                                | "find_last_index"
                                | "and"
                                | "or"
                                | "xor"
                        ) && self.module.arrays.contains_key(obj_name)
                        {
                            return self
                                .eval_builtin_method(obj_name, mname, &[])
                                .unwrap_or(Value::zero(32));
                        }
                    }
                    // Local struct variable field read: `t.field1` where `t` is a
                    // local var of unpacked-struct typedef type. Extract the field
                    // from the local's whole Value via the struct layout.
                    // Also handles signal-stored whole-value structs (initial-block
                    // locals and module signals without per-field member signals).
                    if hier.path.len() == 2 {
                        let member_name = hier.path[1].name.name.as_str();
                        let tn = self.var_typedef_types.get(obj_name).cloned();
                        let layout = tn
                            .as_ref()
                            .and_then(|t| self.module.typedef_types.get(t.as_str()))
                            .and_then(|dt| {
                                let resolved =
                                    Self::resolve_type_ref(dt, &self.module.typedef_types);
                                Self::struct_field_layout(&resolved)
                            })
                            .or_else(|| {
                                // Signal-stored struct: resolve type via
                                // signal_name_to_id → signal_type_names.
                                let sig_tn = self
                                    .signal_name_to_id
                                    .get(obj_name.as_str())
                                    .and_then(|id| self.signal_type_names.get(id).cloned());
                                sig_tn
                                    .as_ref()
                                    .and_then(|t| self.module.typedef_types.get(t.as_str()))
                                    .and_then(|dt| {
                                        let resolved =
                                            Self::resolve_type_ref(dt, &self.module.typedef_types);
                                        Self::struct_field_layout(&resolved)
                                    })
                            });
                        if let Some(fields) = layout {
                            // Only take this path if there is NO separate
                            // `<obj>.<member>` signal (the per-field scheme has
                            // its own signal and should be preferred).
                            let dotted = format!("{}.{}", obj_name, member_name);
                            let has_sep_sig = self.signal_name_to_id.contains_key(dotted.as_str())
                                || self.signals.contains_key(&dotted);
                            if !has_sep_sig {
                                let base_val = self
                                    .local_stack
                                    .last()
                                    .and_then(|f| f.get(obj_name).cloned())
                                    .or_else(|| self.get_signal_value_by_name(obj_name));
                                if let Some(base_val) = base_val {
                                    if let Some((_, off, w, fr)) =
                                        fields.iter().find(|(m, _, _, _)| m == member_name).cloned()
                                    {
                                        let mut v = base_val
                                            .range_select((off + w - 1) as usize, off as usize);
                                        if fr {
                                            v.is_real = true;
                                        }
                                        return v;
                                    }
                                }
                            }
                        }
                    }
                    // Handle hierarchical ident that might be class member access: obj.prop
                    // Unpacked-struct member paths (e.g. `t_unpacked_struct.val_r`)
                    // are pre-registered as separate signals by elaboration.
                    // Prefer the separate-signal read so VPI-forced values stay
                    // in sync with the SV-visible value.
                    {
                        let full_path = hier
                            .path
                            .iter()
                            .map(|s| s.name.name.as_str())
                            .collect::<Vec<_>>()
                            .join(".");
                        if let Some(v) = self.get_signal_value_by_name(&full_path) {
                            return v;
                        }
                    }
                    // Resolve the head to a handle from any storage: a
                    // static class property (e.g. `the_mid` inside a static
                    // method), a local, a `this` property, or a signal.
                    // `eval_ident_handle` covers local/this/static; signals
                    // need an explicit fallback.
                    let cur_handle = self
                        .eval_ident_handle(obj_name)
                        .filter(|&h| h != 0 && h < self.heap.len())
                        .or_else(|| {
                            self.local_stack
                                .last()
                                .and_then(|m| m.get(obj_name))
                                .and_then(|v| v.to_u64())
                                .map(|h| h as usize)
                                .filter(|&h| h != 0 && h < self.heap.len())
                        })
                        .or_else(|| {
                            self.get_signal_value_by_name(obj_name)
                                .and_then(|v| v.to_u64())
                                .map(|h| h as usize)
                                .filter(|&h| h != 0 && h < self.heap.len())
                        });
                    if let Some(h) = cur_handle {
                        for i in 1..hier.path.len() {
                            if h == 0 || h >= self.heap.len() {
                                break;
                            }
                            let member_name = &hier.path[i].name.name;
                            // §8.9: a static property is the class's shared
                            // cell, never the object's own storage.
                            let via = (i == 1).then_some(obj_name.as_str());
                            let mval = match self.static_read_via_handle(h, via, None, member_name)
                            {
                                Some(v) => Some(v),
                                None => self.read_member_value(h, member_name),
                            };
                            if let Some(mval) = mval {
                                if i == hier.path.len() - 1 {
                                    return mval;
                                }
                            } else {
                                break;
                            }
                        }
                    }
                }
                // §26.3: `pkg::MEMBER` reaching here is an enum member of a
                // package. `fast_signal_read` would discard the package scope
                // and look the leaf up flat, which silently reads a local
                // declaration that shadows the member. Checked last so any
                // genuinely hierarchical `pkg.member` signal still wins.
                if hier.path.len() == 2 && hier.path.iter().all(|s| s.selects.is_empty()) {
                    if let Some(v) =
                        self.package_enum_member(&hier.path[0].name.name, &hier.path[1].name.name)
                    {
                        return v;
                    }
                }
                // §7.2: a bare UNPACKED-struct name evaluated as a VALUE — the
                // struct has no container signal, only member leaves, so every
                // lookup above misses and the read came back x. That is what
                // `return s;` from a struct-returning function produced. Last
                // resort, so a real signal always wins.
                if hier.path.len() == 1
                    && hier.path[0].selects.is_empty()
                    && !self
                        .signal_name_to_id
                        .contains_key(hier.path[0].name.name.as_str())
                    && !self.signals.contains_key(&hier.path[0].name.name)
                    && self
                        .local_stack
                        .last()
                        .is_none_or(|l| !l.contains_key(&hier.path[0].name.name))
                {
                    let base = &hier.path[0].name.name;
                    if let Some(dt) = self.p_elem_type(base) {
                        if let Some(su) = self.unpacked_struct_of(&dt) {
                            if let Some(v) = self.pack_unpacked_struct(base, &su) {
                                return v;
                            }
                        }
                    }
                }
                if hier.path.iter().all(|seg| seg.selects.is_empty()) {
                    if hier.path.len() == 1 {
                        let name = hier.path[0].name.name.as_str();
                        if self.get_signal_value_by_name(name).is_none()
                            && (self.module.functions.contains_key(name)
                                || self.module.lets.contains_key(name))
                        {
                            return self.eval_call(expr, &[]);
                        }
                    } else {
                        let leaf = hier.path.last().unwrap().name.name.as_str();
                        let joined = hier
                            .path
                            .iter()
                            .map(|seg| seg.name.name.as_str())
                            .collect::<Vec<_>>()
                            .join(".");
                        let scoped = hier
                            .path
                            .iter()
                            .map(|seg| seg.name.name.as_str())
                            .collect::<Vec<_>>()
                            .join("::");
                        let package_head = self.module.packages.contains(&hier.path[0].name.name);
                        let package_fn = package_head
                            && self.get_signal_value_by_name(leaf).is_none()
                            && self.module.functions.contains_key(&scoped)
                            && self
                                .module
                                .pkg_subr_owner
                                .get(leaf)
                                .is_some_and(|owner| owner == &hier.path[0].name.name);
                        let instance_fn =
                            !package_head && self.module.functions.contains_key(&joined);
                        if package_fn || instance_fn {
                            return self.eval_call(expr, &[]);
                        }

                        if Self::is_parenless_zero_arg_builtin(leaf) {
                            let head = hier_prefix(hier);
                            let recv = Expression::new(ExprKind::Ident(head), expr.span);
                            let recv_name = self.expr_assoc_name(&recv).or_else(|| {
                                Self::plain_ident_name(&recv).filter(|name| {
                                    self.module.dynamic_arrays.contains(name)
                                        || self.module.associative_arrays.contains_key(name)
                                        || self.module.arrays.contains_key(name)
                                })
                            });
                            if self.expr_is_string_valued(&recv) || recv_name.is_some() {
                                return self.eval_call(expr, &[]);
                            }
                        }
                    }
                }
                self.fast_signal_read(hier)
            }
            ExprKind::Unary { op, operand } => {
                // §11.6.1: `~` and unary `-` are CONTEXT-determined — the
                // operand extends to the surrounding width BEFORE the
                // operation. The VM path got this fix earlier; this arm
                // evaluated at the operand's own width and zero-extended
                // after, so `~a + 32'd0` printed 0000005c for ffffff5c while
                // the identical assign was right. Reductions/! stay
                // self-determined.
                let v = if ctx_width > 0
                    && matches!(op, UnaryOp::Minus | UnaryOp::BitNot | UnaryOp::Plus)
                {
                    let v0 = self.eval_expr_typed(operand, ctx_width, unsigned_context);
                    if ctx_width > v0.width {
                        v0.resize(ctx_width)
                    } else {
                        v0
                    }
                } else {
                    self.eval_expr(operand)
                };
                match op {
                    UnaryOp::Plus => v,
                    UnaryOp::Minus => {
                        let mut r = Value::zero(v.width).sub(&v).resize(v.width);
                        // §11.8.1: unary minus preserves its operand type.
                        r.is_signed = v.is_signed;
                        r
                    }
                    UnaryOp::LogNot => v.logic_not(),
                    UnaryOp::BitNot => v.bitwise_not(),
                    UnaryOp::BitAnd => v.reduce_and(),
                    UnaryOp::BitOr => v.reduce_or(),
                    UnaryOp::BitXor => v.reduce_xor(),
                    UnaryOp::BitNand => v.reduce_and().logic_not(),
                    UnaryOp::BitNor => v.reduce_or().logic_not(),
                    UnaryOp::BitXnor => v.reduce_xor().logic_not(),
                    UnaryOp::PreIncr => {
                        let nv = v.add(&Self::inc_dec_step(&v));
                        self.assign_value(operand, &nv);
                        nv
                    }
                    UnaryOp::PostIncr => {
                        let nv = v.add(&Self::inc_dec_step(&v));
                        self.assign_value(operand, &nv);
                        v
                    }
                    UnaryOp::PreDecr => {
                        let nv = v.sub(&Self::inc_dec_step(&v));
                        self.assign_value(operand, &nv);
                        nv
                    }
                    UnaryOp::PostDecr => {
                        let nv = v.sub(&Self::inc_dec_step(&v));
                        self.assign_value(operand, &nv);
                        v
                    }
                    UnaryOp::HashHash => Value::zero(1),
                    // LRM §16.12.6 temporal operators outside their
                    // SVA context — evaluate operand as a boolean (the
                    // SVA executor handles the temporal semantics
                    // separately when these appear inside a property
                    // body).
                    UnaryOp::SEventually | UnaryOp::SAlways => v,
                }
            }
            ExprKind::Binary { op, left, right } => {
                if let Some(v) = self.plain_compare(*op, left, right, ctx_width) {
                    return v;
                }
                // §25.9: equality where a side is a VIRTUAL-INTERFACE
                // VARIABLE compares BINDINGS, not (x) values — `val == t` in
                // uvm_resource::write with t bound and val unbound must be
                // FALSE, or the guarded store never runs (issue #113). The
                // classifier is deliberately narrow (declared vif vars only,
                // never `iface.signal` selects) so ordinary comparisons keep
                // their value semantics.
                if matches!(op, BinaryOp::Eq | BinaryOp::Neq) {
                    let lv = self.vif_operand_eval(left);
                    let rv = self.vif_operand_eval(right);
                    let engaged = lv.is_some() || rv.is_some();
                    if engaged {
                        // A bare `null` on the other side of a vif operand is
                        // the unbound value.
                        let l = lv.or_else(|| matches!(&left.kind, ExprKind::Null).then_some(None));
                        let r =
                            rv.or_else(|| matches!(&right.kind, ExprKind::Null).then_some(None));
                        match (l, r) {
                            (Some(l), Some(r)) => {
                                let eq = l == r;
                                let bit = if matches!(op, BinaryOp::Eq) { eq } else { !eq };
                                return Value::from_u64(bit as u64, 1);
                            }
                            // A BOUND vif never equals an operand that is not
                            // itself vif-classifiable (its untracked storage
                            // reads x/0 — `val == t` in uvm_resource::write).
                            (Some(Some(_)), None) | (None, Some(Some(_))) => {
                                let bit = if matches!(op, BinaryOp::Eq) { 0 } else { 1 };
                                return Value::from_u64(bit, 1);
                            }
                            // Unbound vs unclassifiable: keep value semantics.
                            _ => {}
                        }
                    }
                }
                // §11.4.5 / §10.9: equality against an ASSIGNMENT PATTERN
                // (`a == '{9, 5, 5, 5}`) compares element- / member-wise.
                if matches!(
                    op,
                    BinaryOp::Eq | BinaryOp::Neq | BinaryOp::CaseEq | BinaryOp::CaseNeq
                ) && (matches!(left.kind, ExprKind::AssignmentPattern(_))
                    || matches!(right.kind, ExprKind::AssignmentPattern(_)))
                {
                    if let Some(v) = self.compare_with_pattern(*op, left, right) {
                        return v;
                    }
                }
                // §11.4.5: equality on unpacked structs is member-wise.
                if matches!(op, BinaryOp::Eq | BinaryOp::Neq) {
                    if let Some(v) = self.compare_unpacked_structs(left, right) {
                        return if matches!(op, BinaryOp::Eq) {
                            v
                        } else if v.has_unknown() {
                            v
                        } else if v.to_u64() == Some(1) {
                            Value::zero(1)
                        } else {
                            Value::ones(1)
                        };
                    }
                }
                // Short-circuit evaluation for logical operators (IEEE §11.3.5)
                if matches!(op, BinaryOp::LogAnd | BinaryOp::LogOr) {
                    let l = self.eval_expr_ctx(left, ctx_width);
                    match op {
                        BinaryOp::LogAnd => {
                            // §11.3.5: `&&` short-circuits only when the left is a
                            // KNOWN logic 0. An X/Z left is ambiguous — the right
                            // (with its side effects) must still be evaluated,
                            // because `x && 1` is x, not 0. `z.to_u64()` yields
                            // Some(0), so guard on has_xz too.
                            if !l.has_xz() && l.to_u64() == Some(0) {
                                return Value::from_u64(0, 1);
                            }
                            let r = self.eval_expr_ctx(right, ctx_width);
                            return l.logic_and(&r);
                        }
                        BinaryOp::LogOr => {
                            if l.to_u64().is_some_and(|v| v != 0) {
                                return Value::from_u64(1, 1);
                            }
                            let r = self.eval_expr_ctx(right, ctx_width);
                            return l.logic_or(&r);
                        }
                        _ => unreachable!(),
                    }
                }
                // Logical implication / equivalence (IEEE §11.4.7). The SVA
                // `iff` (§16.12.8), `property_expr1 iff property_expr2`, is
                // logical equivalence too: true iff both hold or both don't.
                if matches!(
                    op,
                    BinaryOp::LogImplies | BinaryOp::LogEquiv | BinaryOp::Iff
                ) {
                    let l = self.eval_expr_ctx(left, ctx_width);
                    let r = self.eval_expr_ctx(right, ctx_width);
                    return match op {
                        BinaryOp::LogImplies => l.logic_impl(&r),
                        BinaryOp::LogEquiv => l.logic_equiv(&r),
                        // §16.12.8: iff compares PROPERTY truth values. An
                        // unknown Boolean expression is a failed property.
                        BinaryOp::Iff => Value::from_u64(u64::from(l.is_true() == r.is_true()), 1),
                        _ => unreachable!(),
                    };
                }
                // LRM §16.12 — SVA implication operators `|->` (overlapping)
                // and `|=>` (non-overlapping). For our deferred-eval
                // observed-region framework (M9), evaluating both as plain
                // logical implication `!a || b` is the right minimum-viable
                // shape: the antecedent + consequent both already see the
                // post-NBA settled state. The temporal-cycle distinction
                // between `|->` and `|=>` would matter once we had a real
                // automaton (one cycle of delay for `|=>`) — that's L2 in
                // full and remains the larger follow-up.
                if matches!(op, BinaryOp::OrMinusArrow | BinaryOp::OrFatArrow) {
                    let l = self.eval_expr_ctx(left, ctx_width);
                    let r = self.eval_expr_ctx(right, ctx_width);
                    return l.logic_impl(&r);
                }
                // LRM §16.9 sequence operators — minimum-viable
                // single-cycle evaluation (the SVA executor handles
                // the temporal cases inside a property body; this is
                // the fallback for a bare overlap check):
                //   `a throughout b`  → a && b  (a must hold while b)
                //   `a within b`      → a && b  (a inside b)
                //   `a and b` (seq)   → a && b
                //   `a or b` (seq)    → a || b
                //   `a intersect b`   → a && b
                //   `a until b`       → a || b  (a holds until b)
                //   `a s_until b`     → a || b
                if matches!(
                    op,
                    BinaryOp::Throughout
                        | BinaryOp::Within
                        | BinaryOp::Intersect
                        | BinaryOp::SeqAnd
                ) {
                    let l = self.eval_expr_ctx(left, ctx_width);
                    let r = self.eval_expr_ctx(right, ctx_width);
                    return l.logic_and(&r);
                }
                if matches!(op, BinaryOp::SeqOr | BinaryOp::Until | BinaryOp::SUntil) {
                    let l = self.eval_expr_ctx(left, ctx_width);
                    let r = self.eval_expr_ctx(right, ctx_width);
                    return l.logic_or(&r);
                }
                // Unpacked array equality/inequality (§11.4.5) on fixed-size
                // arrays — module-level OR class-property, accessed as bare
                // `a`, `this.a`, or `obj.a`. `rhs_.sa == lhs.sa` in a class
                // method (`compare`/`do_compare`) is the common case.
                // Multi-dimensional unpacked arrays (or a row of one) compare
                // element-wise by suffix; the 1-D arm below sees no shape.
                if matches!(op, BinaryOp::Eq | BinaryOp::Neq)
                    && !(self.module.arrays_2d.is_empty() && self.module.arrays_nd.is_empty())
                {
                    if let (Some((lb, ls, l_nd)), Some((rb, rs, r_nd))) =
                        (self.nd_array_operand(left), self.nd_array_operand(right))
                    {
                        if l_nd || r_nd {
                            let mut equal = ls.len() == rs.len();
                            let ew = self
                                .coll_elem_width(&lb)
                                .or_else(|| self.coll_elem_width(&rb));
                            if equal {
                                for (a, b) in ls.iter().zip(rs.iter()) {
                                    let lv = self.get_signal_value_by_name(&format!("{}{}", lb, a));
                                    let rv = self.get_signal_value_by_name(&format!("{}{}", rb, b));
                                    if lv.is_none()
                                        || rv.is_none()
                                        || self.coll_values_opt_eq(&lv, &rv, ew) == false
                                    {
                                        equal = false;
                                        break;
                                    }
                                }
                            }
                            let bit = if matches!(op, BinaryOp::Eq) {
                                equal
                            } else {
                                !equal
                            };
                            return Value::from_u64(bit as u64, 1);
                        }
                    }
                }
                if matches!(op, BinaryOp::Eq | BinaryOp::Neq) {
                    if let (Some((ln, llo, lhi)), Some((rn, rlo, rhi))) = (
                        self.fixed_array_operand(left),
                        self.fixed_array_operand(right),
                    ) {
                        let lsize = (lhi - llo + 1) as usize;
                        let rsize = (rhi - rlo + 1) as usize;
                        if lsize != rsize {
                            return Value::from_u64(
                                if matches!(op, BinaryOp::Eq) { 0 } else { 1 },
                                1,
                            );
                        }
                        let l_desc = self.module.descending_arrays.contains(&ln);
                        let r_desc = self.module.descending_arrays.contains(&rn);
                        let mut equal = true;
                        for i in 0..lsize {
                            let lidx = if l_desc {
                                lhi - i as i64
                            } else {
                                llo + i as i64
                            };
                            let ridx = if r_desc {
                                rhi - i as i64
                            } else {
                                rlo + i as i64
                            };
                            let lv = self
                                .get_signal_value_by_name(&format!("{}[{}]", ln, lidx))
                                .unwrap_or(Value::zero(1));
                            let rv = self
                                .get_signal_value_by_name(&format!("{}[{}]", rn, ridx))
                                .unwrap_or(Value::zero(1));
                            if lv.has_unknown() || rv.has_unknown() {
                                // An x/z element makes the comparison neither
                                // equal nor unequal (both `==` and `!=` come
                                // out 0), as for dynamic arrays/queues.
                                return Value::from_u64(0, 1);
                            }
                            // Compare at the declared element width (differing
                            // storage slots must not decide the pair).
                            let ew = self
                                .coll_elem_width(&ln)
                                .or_else(|| self.coll_elem_width(&rn));
                            if !self.coll_values_eq(&lv, &rv, ew) {
                                equal = false;
                                break;
                            }
                        }
                        let r = if matches!(op, BinaryOp::Eq) {
                            equal
                        } else {
                            !equal
                        };
                        return Value::from_u64(if r { 1 } else { 0 }, 1);
                    }
                }
                // IEEE 1800-2017 §11.8.1: `==`/`!=` on two dynamic arrays /
                // queues compares element-by-element (after a length check),
                // not the container's packed scalar. UVM's
                // `uvm_resource#(T)::do_write` guards on `val == t` for a
                // `T`-typed queue, so a wrong scalar compare (X/garbage)
                // made the update early-return or was otherwise unreliable.
                // Associative arrays compare as equal key/value sets (§7.12) —
                // without this a class-property assoc `==` fell through to the
                // packed-scalar path and always yielded x.
                if matches!(op, BinaryOp::Eq | BinaryOp::Neq) {
                    let want_eq = matches!(op, BinaryOp::Eq);
                    if let Some(res) = self.compare_assoc_arrays(left, right, want_eq) {
                        return res;
                    }
                    if let Some(res) = self.compare_dyn_arrays(left, right, want_eq) {
                        return res;
                    }
                }
                let is_arith_or_bitwise = matches!(
                    op,
                    BinaryOp::Add
                        | BinaryOp::Sub
                        | BinaryOp::Mul
                        | BinaryOp::BitAnd
                        | BinaryOp::BitOr
                        | BinaryOp::BitXor
                        | BinaryOp::BitXnor
                );
                // IEEE §11.6.1: For context-determined operations, the width is
                // max(lhs_width, rhs_width, context_width), computed BEFORE evaluation
                // so that sub-expressions are widened to the full expression width.
                // §11.4.10 / §11.6.1: a shift's LEFT operand (and the operands
                // of / % **) is context-determined, but the context width is
                // the MAXIMUM of the surrounding context and the operand's own
                // width — never smaller. Propagating a narrow LHS width down
                // truncated the operand before the operation:
                // `logic [4:0] r; r <= (1 << s) >> 3;` evaluated `1 << 5` at
                // 5 bits (0) instead of 32 (32), so r read 0 instead of 4.
                // (+ - * & | ^ preserve the low bits either way, which is why
                // only this family showed it.) A shift's RIGHT operand stays
                // self-determined.
                let widens_left = matches!(
                    op,
                    BinaryOp::ShiftLeft
                        | BinaryOp::ShiftRight
                        | BinaryOp::ArithShiftLeft
                        | BinaryOp::ArithShiftRight
                        | BinaryOp::Div
                        | BinaryOp::Mod
                        | BinaryOp::Power
                );
                let is_comparison = matches!(
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
                );
                // Operands the width probe had to evaluate are kept and
                // reused below instead of being evaluated again.
                let has_shift = (is_arith_or_bitwise
                    || is_comparison
                    || matches!(op, BinaryOp::Div | BinaryOp::Mod))
                    && (super::bytecode::has_context_shift(left)
                        || super::bytecode::has_context_shift(right));
                let mut pre_l: Option<Value> = None;
                let mut pre_r: Option<Value> = None;
                let self_det_w = if is_arith_or_bitwise {
                    // A shift's count and carry-aware width estimate do not
                    // size its parent expression (§11.6.1). Keep fallback
                    // values for operands whose declared width is unavailable.
                    let (lw, lv) = if has_shift {
                        self.lrm_self_width(left)
                            .map(|w| (w, None))
                            .unwrap_or_else(|| self.width_probe(left))
                    } else {
                        self.width_probe(left)
                    };
                    let (rw, rv) = if has_shift {
                        self.lrm_self_width(right)
                            .map(|w| (w, None))
                            .unwrap_or_else(|| self.width_probe(right))
                    } else {
                        self.width_probe(right)
                    };
                    // §11.8.2: a real operand is 64 bits wide, so only then
                    // can `+ - *` have a real operand; each integral operand
                    // is then converted from its self-determined value (the
                    // probes above already evaluated some operands that way).
                    let (mut lv, mut rv) = (lv, rv);
                    if (lw == 64 || rw == 64)
                        && matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul)
                        && let Some(v) = self.real_arith_probed(*op, left, right, &mut lv, &mut rv)
                    {
                        return v;
                    }
                    pre_l = lv;
                    pre_r = rv;
                    let w = lw.max(rw).max(ctx_width);
                    // A narrow x/z value (a missing associative key reads as
                    // x) fills the whole context when evaluated there; keep
                    // that by letting such an operand evaluate again. A fill
                    // literal replicates on resize and needs no re-run.
                    let keep = |v: &Option<Value>| {
                        !v.as_ref()
                            .is_some_and(|v| v.width < w && v.has_xz() && !v.is_fill)
                    };
                    if !keep(&pre_l) {
                        pre_l = None;
                    }
                    if !keep(&pre_r) {
                        pre_r = None;
                    }
                    w
                } else if widens_left {
                    // The LRM self width, NOT the carry-aware infer_width:
                    // `(a<<4)>>2` in an 8-bit context otherwise evaluates the
                    // inner shift at 12 bits and the dropped carry returns.
                    // (Unknown shapes keep the historical infer_width.)
                    let llw = self.lrm_self_width(left);
                    // §11.6.1: `/` and `%` size from BOTH operands (only a
                    // shift amount or an exponent is self-determined), so
                    // `(a + b) / 3` over 8-bit a, b divides at the literal's
                    // 32 bits, as the compiled path already does.
                    let rlw = if matches!(op, BinaryOp::Div | BinaryOp::Mod | BinaryOp::Power) {
                        self.lrm_self_width(right)
                    } else {
                        Some(0)
                    };
                    // §11.8.2: `/` and `**` with a REAL operand (64 bits, or a
                    // literal the LRM width cannot size).
                    if matches!(op, BinaryOp::Div | BinaryOp::Power)
                        && (llw.is_none_or(|w| w == 64) || rlw.is_none_or(|w| w == 64))
                        && let Some(v) = self.eval_real_operator(*op, left, right)
                    {
                        return v;
                    }
                    let lw = llw.unwrap_or_else(|| self.infer_width(left));
                    let rw = if matches!(op, BinaryOp::Div | BinaryOp::Mod) {
                        rlw.unwrap_or(0)
                    } else {
                        0
                    };
                    lw.max(rw).max(ctx_width)
                } else if is_comparison {
                    // §11.6.1: a comparison's operands take the MAX OF EACH
                    // OTHER's widths — never the SURROUNDING context. The
                    // assignment width leaked in, so `t32 = (a + a) > 8'h50`
                    // compared at 32 bits (0x146 > 0x50 = 1) where the
                    // reference compares the joint 8-bit width (0x46 > 0x50 =
                    // 0). Reference-validated corner: an UNSIZED literal
                    // counts as 32, so `(8'hFF << 9) == 0` widens the shift
                    // to 32 bits and compares FALSE (0x1FE00 != 0).
                    // Any operand this cannot size without evaluating keeps
                    // the historical behavior (the outer context) — sizing
                    // through infer_width would EXECUTE method calls.
                    let (llw, rlw) = (self.lrm_self_width(left), self.lrm_self_width(right));
                    // §11.8.2: a REAL operand (64 bits, or a literal the LRM
                    // width cannot size) makes the comparison real.
                    if (llw.is_none_or(|w| w == 64) || rlw.is_none_or(|w| w == 64))
                        && let Some(v) = self.eval_real_operator(*op, left, right)
                    {
                        return v;
                    }
                    match (llw, rlw) {
                        (Some(lw), Some(rw)) => lw.max(rw),
                        // §7.5.1/§7.10.2: `size()` and `num()` return an int,
                        // so `a.size() == n + m` over 6-bit n, m adds at 32
                        // bits. Asked only when the other side is narrower.
                        (l, r)
                            if l.max(r).is_none_or(|w| w < 32)
                                && (self.count_method_call(left)
                                    || self.count_method_call(right)) =>
                        {
                            ctx_width.max(32)
                        }
                        _ => ctx_width,
                    }
                } else {
                    ctx_width
                };
                // §15.5.4: comparing event variables compares their IDENTITY
                // (which synchronization object they name), not a value. An
                // event's 1-bit backing signal holds an arbitrary toggle state,
                // so the integral path returned x — `e1 == e2` after `e1 = e2`
                // read x instead of 1. Null compares equal only to null.
                if matches!(op, BinaryOp::Eq | BinaryOp::Neq) {
                    let lk = self.event_operand_key(left);
                    let rk = self.event_operand_key(right);
                    if let (Some(lk), Some(rk)) = (lk, rk) {
                        let eq = lk == rk;
                        let res = if matches!(op, BinaryOp::Eq) { eq } else { !eq };
                        return Value::from_u64(res as u64, 1);
                    }
                }
                // §11.4.8 gated-call elision, RIGHT-hand mask. Handled BEFORE
                // the left operand is evaluated, because the left operand is
                // the call we are trying to avoid. Evaluating the mask first is
                // unobservable precisely because the call is pure — it has no
                // effects to reorder against.
                if matches!(op, BinaryOp::BitAnd | BinaryOp::BitOr)
                    && pre_l.is_none()
                    && pre_r.is_none()
                    && self.pure_call_name(left).is_some()
                {
                    let want_ones = matches!(op, BinaryOp::BitOr);
                    let r0 = self.eval_expr_ctx(right, self_det_w);
                    if Self::absorbs_operand(&r0, want_ones) {
                        let w = self_det_w.max(1);
                        return if want_ones {
                            Value::ones(w)
                        } else {
                            Value::zero(w)
                        };
                    }
                    pre_r = Some(r0);
                }
                // §11.8.2: propagate unsigned type before nested arithmetic
                // runs or its operands widen. Type probes never execute calls.
                let joint_unsigned = if is_arith_or_bitwise
                    || matches!(op, BinaryOp::Div | BinaryOp::Mod)
                    || is_comparison
                {
                    (!is_comparison && unsigned_context)
                        || (pre_l
                            .as_ref()
                            .map(|v| v.is_signed)
                            .or_else(|| self.expression_signed_hint(left))
                            == Some(false)
                            || pre_r
                                .as_ref()
                                .map(|v| v.is_signed)
                                .or_else(|| self.expression_signed_hint(right))
                                == Some(false))
                } else if widens_left {
                    unsigned_context
                } else {
                    false
                };
                let literal_comparison = matches!(
                    op,
                    BinaryOp::CaseEq
                        | BinaryOp::CaseNeq
                        | BinaryOp::WildcardEq
                        | BinaryOp::WildcardNeq
                );
                let left_unsigned =
                    joint_unsigned && !(literal_comparison && Self::expr_is_signed_literal(left));
                // §11.6.1: an exponent is self-determined and keeps its type.
                let right_unsigned = joint_unsigned
                    && !matches!(op, BinaryOp::Power)
                    && !(literal_comparison && Self::expr_is_signed_literal(right));
                let mut l = match pre_l.take() {
                    Some(mut v) => {
                        if left_unsigned && !v.is_real {
                            v.is_signed = false;
                        }
                        v
                    }
                    None => self.eval_expr_typed(left, self_det_w, left_unsigned),
                };
                // §11.4.8 zero-mask elision. `0 & anything` is 0 for EVERY
                // value the other side can take — `0 & x` and `0 & z` are both
                // 0 — so the result cannot depend on the call. Gating a
                // function result with a reset mask (`{N{rst_n}} & f(..)`) is
                // ordinary RTL, and evaluating `f` for a result that is thrown
                // away dominated such a design: on a 200k-cycle benchmark the
                // call cost the same during reset as out of it.
                //
                // The LRM does evaluate both operands of a bitwise `&` (unlike
                // `&&`), so this is only legal for a callee with no observable
                // effect — hence the purity check. `self_det_w` already carries
                // the max of both operand widths, computed by INFERENCE rather
                // than evaluation, so the skipped side still sizes the result.
                // Restricted to <=64 bits, where `raw_bits` sees every bit.
                if matches!(op, BinaryOp::BitAnd | BinaryOp::BitOr) && pre_r.is_none() {
                    let want_ones = matches!(op, BinaryOp::BitOr);
                    if Self::absorbs_operand(&l, want_ones) && self.pure_call_name(right).is_some()
                    {
                        let w = self_det_w.max(1);
                        return if want_ones {
                            Value::ones(w)
                        } else {
                            Value::zero(w)
                        };
                    }
                }
                let is_shift_op = matches!(
                    op,
                    BinaryOp::ShiftLeft
                        | BinaryOp::ShiftRight
                        | BinaryOp::ArithShiftLeft
                        | BinaryOp::ArithShiftRight
                );
                let mut r = if let Some(mut v) = pre_r {
                    if right_unsigned && !v.is_real {
                        v.is_signed = false;
                    }
                    v
                } else if matches!(op, BinaryOp::Power) {
                    // §11.6.1: the exponent keeps its own width as well as type.
                    self.eval_expr_ctx(right, 0)
                } else if is_shift_op {
                    let rw = self.infer_width(right);
                    self.eval_expr_ctx(right, rw)
                } else {
                    self.eval_expr_typed(right, self_det_w, right_unsigned)
                };
                // IEEE 1800-2023 §6.16 / Table 6-9: the `==`/`!=` operators
                // are 2-STATE when applied to the `string` data type — they
                // compare the textual content and always yield a definite
                // 0 or 1, never X/Z.  xezim stores a `string` value as a
                // fixed-width 1024-bit packed vector (128-char capacity)
                // whose unused high bits are X when the string is shorter
                // than the capacity; routing a string `==`/`!=` through
                // the integral 4-state equality path would therefore
                // wrongly return X whenever either side has X padding
                // (e.g. comparing an empty string to a non-empty one).
                // Apply the 2-state text comparison ONLY when BOTH
                // operands are string-typed (a string literal counts as
                // string type): a mixed `byte != "."` comparison is an
                // INTEGRAL comparison per §11.4 (the literal is treated
                // as a packed value), and forcing string semantics on it
                // corrupts code such as hierarchical scope-string walking.
                // Both string probes below ask the same questions of an
                // unchanged state; answer each at most once.
                let mut left_is_str: Option<bool> = None;
                let mut right_is_str: Option<bool> = None;
                if matches!(op, BinaryOp::Eq | BinaryOp::Neq)
                    && *left_is_str.get_or_insert_with(|| self.operand_is_string(left))
                    && *right_is_str.get_or_insert_with(|| self.operand_is_string(right))
                {
                    let ls = l.to_sv_string();
                    let rs = r.to_sv_string();
                    let res = match op {
                        BinaryOp::Eq => ls == rs,
                        BinaryOp::Neq => ls != rs,
                        _ => unreachable!(),
                    };
                    return Value::from_u64(res as u64, 1);
                }
                // IEEE 1800-2017 §11.8.1/§11.8.2: an expression is UNSIGNED as
                // soon as ANY operand is unsigned, and §11.8.2 step 2 converts
                // every operand to the EXPRESSION's signedness BEFORE it is
                // extended to the result width. So a signed operand sitting in
                // an unsigned expression is ZERO-extended, not sign-extended:
                //   `$signed(8'hFF) + 32'd1` == 32'd256, not 32'd0.
                // This is the "operands follow normal expression rules" clause
                // that §18.5.12 defers to for constraint expressions, so the
                // constraint evaluator (which routes through here) inherits it.
                // Div/Mod are context-determined in BOTH operands (§11.6.1
                // Table 11-21) and follow the same §11.8.1/§11.8.2 rules —
                // they were excluded from this widening, so `smin / sm1` in a
                // 32-bit context divided at 8 bits (wrapping -128/-1 to -128)
                // and a divide-by-zero produced x at 8 bits zero-extended
                // instead of x across the context.
                let widen_both = (is_arith_or_bitwise
                    || matches!(op, BinaryOp::Div | BinaryOp::Mod))
                    // §11.8.1 is an INTEGRAL rule: a real operand makes the
                    // expression real and sign-clearing an integer next to it
                    // turned `-1 / 1.0e-6` into 1.8e25.
                    && !l.is_real
                    && !r.is_real;
                if widen_both && !(l.is_signed && r.is_signed) {
                    l.is_signed = false;
                    r.is_signed = false;
                }
                let max_w = l.width.max(r.width).max(self_det_w);
                let wl = if widen_both && max_w > l.width {
                    l.resize(max_w)
                } else {
                    l
                };
                let wr = if widen_both && max_w > r.width {
                    r.resize(max_w)
                } else {
                    r
                };
                // §6.16.3: the relational operators on `string` operands compare
                // LEXICOGRAPHICALLY, not as zero-extended bit vectors. Comparing
                // the packed values made the comparison depend on LENGTH first:
                // "Jello" (40 bits) against "z" (8 bits) widened "z" to
                // 0x000000007A, so `"Jello" < "z"` came out FALSE even though
                // 'J' < 'z'. Equality was already correct — two byte strings
                // are equal iff their packed values are — so only the four
                // ordering operators are routed here.
                if matches!(
                    op,
                    BinaryOp::Lt
                        | BinaryOp::Leq
                        | BinaryOp::Gt
                        | BinaryOp::Geq
                        | BinaryOp::Eq
                        | BinaryOp::Neq
                ) && (self.operand_has_string_type(left) || self.operand_has_string_type(right))
                {
                    // §6.16: string comparison is by TEXT. Equality also goes
                    // through here because an out-of-bounds string-collection
                    // read stores X bits whose text is "" — a bit-level
                    // compare against "" then yielded X instead of 1 (§6.16
                    // strings are 2-state).
                    let (a, b) = (wl.to_sv_string(), wr.to_sv_string());
                    let t = match op {
                        BinaryOp::Lt => a < b,
                        BinaryOp::Leq => a <= b,
                        BinaryOp::Gt => a > b,
                        BinaryOp::Geq => a >= b,
                        BinaryOp::Eq => a == b,
                        _ => a != b,
                    };
                    return Value::from_u64(t as u64, 1);
                }
                // ===/!==/==?/!=? widening of a signed INTEGER LITERAL: the
                // commercial consensus sign-extends `-16` against a wider
                // unsigned operand (`u64 !== -16` is false for 64'hFF..F0)
                // while a signed VARIABLE in the same position zero-extends
                // per the propagated unsigned type. The distinction lives in
                // the EXPRESSION (literal vs variable), so it is applied here
                // rather than in Value::case_eq.
                if matches!(
                    op,
                    BinaryOp::CaseEq
                        | BinaryOp::CaseNeq
                        | BinaryOp::WildcardEq
                        | BinaryOp::WildcardNeq
                ) {
                    let (mut a, mut b) = (wl, wr);
                    if a.is_signed && a.width < b.width && Self::expr_is_signed_literal(left) {
                        a = a.resize(b.width);
                    }
                    if b.is_signed && b.width < a.width && Self::expr_is_signed_literal(right) {
                        b = b.resize(a.width);
                    }
                    return match op {
                        BinaryOp::CaseEq => a.case_eq(&b),
                        BinaryOp::CaseNeq => a.case_eq(&b).logic_not(),
                        BinaryOp::WildcardEq => a.wildcard_eq(&b),
                        _ => a.wildcard_ne(&b),
                    };
                }
                match op {
                    BinaryOp::Add => wl.add(&wr),
                    BinaryOp::Sub => wl.sub(&wr),
                    BinaryOp::Mul => wl.mul(&wr),
                    BinaryOp::Div => wl.div(&wr),
                    BinaryOp::Mod => wl.modulo(&wr),
                    // §11.6.1: `**`'s LEFT operand is context-determined;
                    // the load gave its declared width, so `a ** 2` in a
                    // 32-bit context computed at 8 bits and read 0x90 for
                    // 0x7e90. Widen to the operation width first (the right
                    // operand stays self-determined).
                    BinaryOp::Power => {
                        let opw = self_det_w.max(wl.width);
                        wl.resize(opw).power(&wr)
                    }
                    // §11.8.1: the integral result is signed when both
                    // operands are signed. The bit-plane helpers clear it.
                    BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor | BinaryOp::BitXnor => {
                        let mut value = match op {
                            BinaryOp::BitAnd => wl.bitwise_and(&wr),
                            BinaryOp::BitOr => wl.bitwise_or(&wr),
                            BinaryOp::BitXor => wl.bitwise_xor(&wr),
                            _ => wl.bitwise_xor(&wr).bitwise_not(),
                        };
                        value.is_signed = wl.is_signed && wr.is_signed;
                        value
                    }
                    BinaryOp::LogAnd => wl.logic_and(&wr),
                    BinaryOp::LogOr => wl.logic_or(&wr),
                    BinaryOp::LogImplies => wl.logic_impl(&wr),
                    BinaryOp::LogEquiv => wl.logic_equiv(&wr),
                    BinaryOp::Eq => wl.is_equal(&wr),
                    BinaryOp::Neq => wl.is_not_equal(&wr),
                    BinaryOp::CaseEq => wl.case_eq(&wr),
                    BinaryOp::CaseNeq => wl.case_eq(&wr).logic_not(),
                    BinaryOp::WildcardEq => wl.wildcard_eq(&wr),
                    BinaryOp::WildcardNeq => wl.wildcard_ne(&wr),
                    BinaryOp::Lt => wl.less_than(&wr),
                    BinaryOp::Leq => wl.leq(&wr),
                    BinaryOp::Gt => wl.greater_than(&wr),
                    BinaryOp::Geq => wl.geq(&wr),
                    BinaryOp::ShiftLeft | BinaryOp::ArithShiftLeft => {
                        // Widen left operand to context width before shifting
                        let wide_l = if self_det_w > wl.width {
                            wl.resize(self_det_w)
                        } else {
                            wl
                        };
                        wide_l.shift_left(&wr)
                    }
                    BinaryOp::ShiftRight | BinaryOp::ArithShiftRight => {
                        // §11.4.10: a shift's LEFT operand is
                        // context-determined, so it is resized to the
                        // expression's width BEFORE the shift — exactly as the
                        // left-shift arm above already does. Without it a
                        // signed operand shifted at its own narrow width and
                        // only then widened, so the vacated high bits came from
                        // the narrow result rather than the sign extension:
                        // `int i = 4'sb1100 >> 1` gave 6 instead of
                        // 32'h7FFF_FFFE. Only `>>` shows it — `>>>` and `<<`
                        // agree either way, which is why it survived.
                        let wide_l = if self_det_w > wl.width {
                            wl.resize(self_det_w)
                        } else {
                            wl
                        };
                        if matches!(op, BinaryOp::ShiftRight) {
                            wide_l.shift_right(&wr)
                        } else {
                            wide_l.arith_shift_right(&wr)
                        }
                    }
                    _ => Value::new(wl.width.max(wr.width)),
                }
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                // §11.4.11/§11.8.2: with a REAL arm the result is real; the
                // integral arm converts from its self-determined value
                // (`c ? (u - 2) : 1.5` is 4294967295.0 for `int unsigned u =
                // 1`). An x condition yields the else arm, as below.
                // (A node with a known non-64 static width is integral.)
                let maybe_real = |e: &Expression| !e.cached_width.get().is_some_and(|w| w != 64);
                if (maybe_real(then_expr) || maybe_real(else_expr))
                    && let Some(v) = self.eval_real_conditional(condition, then_expr, else_expr)
                {
                    return v;
                }
                let c = self.eval_expr(condition);
                let arms_unsigned = unsigned_context
                    || self.expression_signed_hint(then_expr) == Some(false)
                    || self.expression_signed_hint(else_expr) == Some(false);
                let arm_ctx = if arms_unsigned {
                    ctx_width
                        .max(self.lrm_self_width(then_expr).unwrap_or(0))
                        .max(self.lrm_self_width(else_expr).unwrap_or(0))
                } else {
                    ctx_width
                };
                if c.has_unknown() {
                    // IEEE 1800 §11.4.11 Table 11-21: per-bit merge — bit is known
                    // only where both branches agree; otherwise X.
                    let mut t = self.eval_expr_typed(then_expr, arm_ctx, arms_unsigned);
                    let mut e = self.eval_expr_typed(else_expr, arm_ctx, arms_unsigned);
                    // §5.7.1: a fill branch widens to the other branch before
                    // the per-bit merge (resize consumes the fill flag).
                    if t.is_fill && !e.is_fill {
                        t = t.resize(e.width);
                    } else if e.is_fill && !t.is_fill {
                        e = e.resize(t.width);
                    }
                    // A REAL result cannot hold X, so the bitwise merge would
                    // read the IEEE-754 bits of the operands as garbage (e.g. a
                    // real `1000.0` came out as 4.65e18). §11.3.1: the result is
                    // real when either branch is real — return a defined real
                    // value (x condition → the else/false branch) instead.
                    if t.is_real || e.is_real {
                        e
                    } else {
                        t.merge_unknown(&e)
                    }
                } else {
                    let (taken, other): (&Expression, &Expression) = if c.is_true() {
                        (then_expr, else_expr)
                    } else {
                        (else_expr, then_expr)
                    };
                    let mut v = self.eval_expr_typed(taken, arm_ctx, arms_unsigned);
                    // §11.4.11 + §11.8.1: BOTH branches determine the result
                    // type. If either is unsigned the result is unsigned, and
                    // the narrower branch is then extended by the RESULT's
                    // signedness — `x ? s : x` with `shortint s = -1` and
                    // `int unsigned x` is 65535, not -1. Only the taken branch
                    // is evaluated for its VALUE; the other is consulted for
                    // its type — and only when it is a bare identifier or
                    // literal. That keeps the probe side-effect-free AND cheap:
                    // probing an arbitrary expression re-evaluated it, which
                    // doubles the work in hot loops and goes EXPONENTIAL on
                    // nested ternaries (each level probes a branch that itself
                    // probes both of its branches — two ivtest cases went from
                    // pass to TIMEOUT before this was narrowed).
                    let other_is_cheap =
                        matches!(&other.kind, ExprKind::Ident(_) | ExprKind::Number(_));
                    if !v.is_real && other_is_cheap {
                        let o = self.eval_expr_ctx(other, ctx_width);
                        if !o.is_real {
                            if !o.is_signed {
                                v.is_signed = false;
                            }
                            if o.width > v.width {
                                v = v.resize(o.width);
                            }
                        }
                    }
                    v
                }
            }
            ExprKind::Concatenation(parts) => {
                // String concatenation (`{indent, s, ...}`) when any operand is
                // string-valued: join the operands' text. Bit-concat would
                // byte-misalign (shifting 0x20 spaces into 0x40 '@').
                if parts.iter().any(|p| self.expr_is_string_valued(p)) {
                    let mut s = String::new();
                    for p in parts {
                        s.push_str(&self.eval_expr(p).to_sv_string());
                    }
                    return Value::from_string(&s);
                }
                let mut r = Value::zero(0);
                for p in parts.iter().rev() {
                    r = self.eval_expr(p).concat_with(&r);
                }
                r
            }
            ExprKind::StreamOp {
                left_to_right,
                slice_size,
                exprs,
            } => {
                // Concat in source order: MSB = first expr. Build by concatenating from LSB (last expr).
                let mut concat = Value::zero(0);
                for p in exprs.iter().rev() {
                    // Special case: dynamic array/queue ident → concat all elements, idx0 at MSB.
                    // §11.4.14: an unpacked operand streams its elements
                    // leftmost first (`stream_elem_order`) — a descending
                    // `[2:0]` array starts at index 2, a 2-D array goes row
                    // by row. A queue / dynamic-array SLICE `q[a:b]` streams
                    // elements a..b.
                    // §11.4.14: resolve the collection's backing storage,
                    // including class members, before reading its elements.
                    let elem_list = if let Some(name) = self.array_operand_name(p) {
                        self.stream_elem_order(&name)
                    } else {
                        match &p.kind {
                            ExprKind::Ident(h) => {
                                let name = self.resolve_hier_name(h);
                                self.stream_elem_order(&name)
                            }
                            ExprKind::RangeSelect {
                                expr: base,
                                left,
                                right,
                                ..
                            } => {
                                if let Some(name) = self.array_operand_name(base)
                                    && self.module.dynamic_arrays.contains(&name)
                                {
                                    let left = self.eval_expr(left).to_i64();
                                    let right = self.eval_expr(right).to_i64();
                                    match (left, right) {
                                        (Some(left), Some(right))
                                            if left <= right && right - left < 1 << 20 =>
                                        {
                                            self.stream_elem_order(&name).map(|(_, width)| {
                                                (
                                                    (left..=right)
                                                        .map(|i| format!("{name}[{i}]"))
                                                        .collect(),
                                                    width,
                                                )
                                            })
                                        }
                                        _ => None,
                                    }
                                } else {
                                    None
                                }
                            }
                            ExprKind::Paren(inner) => self
                                .array_operand_name(inner)
                                .and_then(|name| self.stream_elem_order(&name)),
                            _ => None,
                        }
                    };
                    let piece = if let Some((names, ew)) = elem_list {
                        let mut acc = Value::zero(0);
                        for en in &names {
                            let ev = self.get_signal_value_by_name(en).unwrap_or(Value::zero(ew));
                            acc = acc.concat_with(&ev);
                        }
                        acc
                    } else {
                        self.eval_expr(p)
                    };
                    concat = piece.concat_with(&concat);
                }
                let total_w = concat.width as usize;
                let streamed = if !*left_to_right {
                    concat
                } else {
                    let slice = slice_size
                        .as_ref()
                        .map(|e| self.eval_expr(e).to_u64().unwrap_or(1) as usize)
                        .unwrap_or(1)
                        .max(1);
                    // Reverse successive slice-sized chunks starting from the MSB side.
                    let mut out = Value::zero(total_w as u32);
                    let full_chunks = total_w / slice;
                    let remainder = total_w - full_chunks * slice;
                    // Original bits [total_w-1 .. 0], where bit (total_w-1) is MSB.
                    // Source chunk k (0-indexed from MSB): bits [total_w-1-k*slice .. total_w-k*slice-slice].
                    // With left_to_right streaming, chunk order is preserved but bits *within* each chunk stay ordered.
                    // Actually `{<<slice {x}}` emits slice-sized groups MSB-first from x with each group's bits in LSB-first order inside the stream... The classic interpretation: slice the source into slice-sized chunks from LSB, then reverse the chunk order.
                    // Standard behavior: output = reverse the order of slice-sized chunks of the source.
                    for k in 0..full_chunks {
                        for b in 0..slice {
                            let src_bit = k * slice + b;
                            let dst_bit = (full_chunks - 1 - k) * slice + b + remainder;
                            if src_bit < total_w && dst_bit < total_w {
                                out.set_bit(dst_bit, concat.get_bit(src_bit));
                            }
                        }
                    }
                    // Leftover bits at the top of the source (fewer than slice) go to the LSB of output.
                    for b in 0..remainder {
                        let src_bit = full_chunks * slice + b;
                        let dst_bit = b;
                        if src_bit < total_w {
                            out.set_bit(dst_bit, concat.get_bit(src_bit));
                        }
                    }
                    out
                };
                // §11.4.14.2: a stream assigned to a WIDER integral target is
                // LEFT-justified — the LRM's own example is
                //   `bit [99:0] d = {>>{a, b, c}};  // d[3:0] = 0`
                // (a 96-bit stream landing in d[99:4]), and the companion
                // `int j = {>>{a, b, c}};` is called an ERROR precisely because
                // a stream may not be truncated. This is the opposite of an
                // ordinary assignment, which zero-extends on the LEFT, and the
                // rule holds for a cast too (`int'({<<8{16'hABCD}})` is
                // 32'hcdab_0000, not 32'h0000_cdab).
                //
                // `ctx_width` is the assignment/cast target width; it is 0
                // wherever no target width applies, so the self-determined
                // value is returned unchanged there. A NARROWER target is an
                // LRM error — left to the caller's ordinary fit rather than
                // made fatal here.
                if ctx_width > streamed.width {
                    let pad = Value::zero(ctx_width - streamed.width);
                    streamed.concat_with(&pad)
                } else {
                    streamed
                }
            }
            ExprKind::Replication { count, exprs } => {
                // Replication evaluation. Two correctness/perf fixes vs. the
                // old `for _ in 0..n { r = inner.concat_with(&r); }` loop:
                //
                // 1) **O(n × inner.width) instead of O(n²)**: pre-allocate
                //    the result `Value::zero(n × inner.width)` and copy
                //    `inner` into each slot at offset `k × inner.width`.
                //    Mirrors a reference simulator's `of_REPLICATE` (`vvp_vector4_t res
                //    (val.size() * rept, BIT4_X); for (idx) res.set_vec
                //    (idx * val.size(), val);`). The old loop allocated a
                //    growing `Value` each iteration and copied the
                //    accumulator — quadratic in `n`.
                //
                // 2) **Clamp the count**. `to_u64()` on a Value with X bits
                //    returns the masked value, which can be u64::MAX for
                //    pathological signals; the resulting `Value::zero(huge)`
                //    + `for _ in 0..huge` would either OOM or run forever.
                //    IEEE 1800 says replication with X count is undefined;
                //    treat anything above MAX_REPL as 0 and let downstream
                //    resize handle it.
                const MAX_REPL: u64 = 1 << 20;
                let count_val = self.eval_expr(count);
                let n_raw = count_val.to_u64().unwrap_or(1);
                let n = if n_raw > MAX_REPL { 0 } else { n_raw };
                let mut inner = Value::zero(0);
                for e in exprs.iter().rev() {
                    inner = self.eval_expr(e).concat_with(&inner);
                }
                let inner_w = inner.width as usize;
                let total_w = (inner_w as u64).saturating_mul(n) as u32;
                let mut r = Value::zero(total_w);
                if inner_w > 0 {
                    for k in 0..n as usize {
                        let off = k * inner_w;
                        for i in 0..inner_w {
                            r.set_bit(off + i, inner.get_bit(i));
                        }
                    }
                }
                r
            }
            ExprKind::Index { expr, index } => {
                if let Some(selected) = self.selected_aggregate_rhs(None, expr) {
                    let selected = Expression::new(
                        ExprKind::Index {
                            expr: Box::new(selected),
                            index: index.clone(),
                        },
                        expr.span,
                    );
                    return self.eval_expr_ctx(&selected, ctx_width);
                }

                // §14.3: `cb.sig[i]` on an unpacked-array clockvar reads the
                // element's sample (see `tick_clocking_blocks`).
                if let Some(v) = self.clocking_array_elem_read(expr, index) {
                    return v;
                }
                // §13.4/§7.4: an element of a function's or method's returned
                // collection (`f()[k]`, `o.build()[k]`). Evaluating the call
                // as a scalar and bit-selecting it read 0 or x.
                if let ExprKind::Call {
                    func: cf,
                    args: cargs,
                } = &expr.kind
                    && self.call_returns_collection(cf)
                {
                    self.pending_ret_collection = None;
                    let _ = self.eval_call(cf, cargs);
                    if let Some(src) = self.take_pending_ret_collection() {
                        let iv = self.eval_expr(index);
                        if self.is_associative_array(&src) {
                            let key = self.assoc_key_str(&src, &iv);
                            return self
                                .get_signal_value_by_name(&format!("{}[{}]", src, key))
                                .or_else(|| self.assoc_missing_elem_value(&src))
                                .unwrap_or_else(|| Value::new(32));
                        }
                        if let Some(i) = iv.to_i64().filter(|_| !iv.has_xz()) {
                            if i >= 0 && (i as u64) < self.get_queue_size(&src) {
                                if let Some(v) =
                                    self.get_signal_value_by_name(&format!("{}[{}]", src, i))
                                {
                                    return v;
                                }
                            }
                        }
                        // §7.4.6: out of range or x/z reads the element default.
                        return Value::new(32);
                    }
                }
                // §8.25: class constant arrays precede packed-property probes.
                // The declaration-name cache excludes ordinary indexed reads
                // before constructing or resolving a class-scope path.
                if !self.module.classes.is_empty()
                    && Self::receiver_prop_name(expr).is_some_and(|name| {
                        self.class_member_names().constant_arrays.contains(name)
                    })
                    && let Some(v) = self.class_param_array_element(expr, index)
                {
                    return v;
                }
                // Element select on a packed multi-dimensional CLASS property.
                if let Some((h, prop, idx_exprs)) = self.prop_index_chain_parts(expr, index) {
                    // §7.4: an element of an unpacked array property of
                    // multi-dimensional packed elements, then packed selects.
                    if idx_exprs.len() >= 2
                        && !self.class_prop_packed_dims(h, &prop).is_some()
                        && let Some((ev, pd, pidx, _)) =
                            self.class_packed_operand(h, &prop, &idx_exprs)
                        && !pidx.is_empty()
                        && pidx.len() <= pd.len()
                    {
                        return match Self::packed_chain_slice(ev.width as u64, &pd, &pidx) {
                            Some((lsb, w)) => ev.range_select((lsb + w - 1) as usize, lsb as usize),
                            None => Value::new(Self::packed_chain_width(
                                ev.width as u64,
                                &pd,
                                pidx.len(),
                            ) as u32),
                        };
                    }
                    if let Some(dims) = self.class_prop_packed_dims(h, &prop) {
                        if idx_exprs.len() <= dims.len() {
                            let idx: Vec<Value> =
                                idx_exprs.iter().map(|ie| self.eval_expr(ie)).collect();
                            if let Some(base_v) = self.read_member_value(h, &prop) {
                                return match Self::packed_chain_slice(
                                    base_v.width as u64,
                                    &dims,
                                    &idx,
                                ) {
                                    Some((lsb, w)) => {
                                        base_v.range_select((lsb + w - 1) as usize, lsb as usize)
                                    }
                                    None => Value::new(Self::packed_chain_width(
                                        base_v.width as u64,
                                        &dims,
                                        idx.len(),
                                    )
                                        as u32),
                                };
                            }
                        }
                    }
                }
                // §7.4.5: an element of a packed-array field of a packed
                // struct reached through a member chain (`h.r.t[k]`,
                // `a[0].r.t[k]`): the field's first dimension picks an
                // element, not a bit.
                if let ExprKind::MemberAccess { expr: sb, member } = &expr.kind
                    && matches!(sb.kind, ExprKind::MemberAccess { .. })
                    && let Some(v) = self.struct_field_elem_read(sb, member, index)
                {
                    return v;
                }
                // Every shape probe below used to re-resolve this same base
                // identifier from scratch — up to ten resolutions, each with
                // its own allocation, per indexed read. On an axi4Lite UVM run
                // that was ~24M of the 52.8M total resolutions. Resolve once.
                // Sound: for one AST node the result is idempotent — the
                // memoized path deliberately skips the scope-hint ratchet, so
                // the repeats were pure overhead, not a second decision.
                let base_nm: Option<std::borrow::Cow<str>> = match &expr.kind {
                    ExprKind::Ident(h) => Some(self.resolve_hier_name(h)),
                    _ => None,
                };
                // §8.9: `Cls::S[i]` of a static fixed array — the resolver
                // collapses the two-segment form to the bare leaf, which
                // named no array and read a bit of a scalar cell instead.
                if let ExprKind::Ident(h) = &expr.kind
                    && h.path.len() == 2
                    && base_nm
                        .as_deref()
                        .is_some_and(|n| !self.module.arrays.contains_key(n))
                    && let Some(k) = self.static_fixed_key_from_hier(h)
                {
                    let ew = self.module.arrays.get(&k).map(|t| t.2).unwrap_or(1).max(1);
                    let iv = self.eval_expr(index);
                    return match iv.to_i64().filter(|_| !iv.has_xz()) {
                        Some(i) => self
                            .get_signal_value_by_name(&format!("{}[{}]", k, i))
                            .unwrap_or_else(|| Value::new(ew)),
                        None => Value::new(ew),
                    };
                }
                // §7.4.6 / §11.5.1: an x/z index reads x at the ELEMENT width,
                // NOT element 0. `to_u64`/`to_i64` mask x bits to zero and
                // never fail, so every `unwrap_or` below silently addressed
                // element 0 and an unknown index returned real array data.
                // Only for a side-effect-free index, so the extra evaluation
                // cannot run a call twice.
                if let ExprKind::Ident(_h) = &expr.kind
                    && Self::cond_expr_is_effect_free(index, false)
                {
                    let nm = base_nm.as_deref().unwrap_or("");
                    if let Some(&(_, _, ew)) = self.module.arrays.get(nm) {
                        if self.eval_expr(index).has_xz() {
                            return Value::new(ew.max(1));
                        }
                    }
                }
                // §7.4.2: `arr[i]…[j].field[k]` over a PACKED array of packed
                // structs. `arr[i]` is a bit slice of one backing vector, so
                // there is no `arr[i]` signal to select from — resolve the
                // whole path to an offset instead. Shares the resolver with
                // the write side so the two cannot drift: reading and writing
                // the same expression must address the same bits.
                if let Some((sig, lsb, w)) = self.packed_struct_elem_slice(expr, index)
                    && let Some(v) = self.get_signal_value_by_name(&sig)
                    && lsb + w <= v.width
                {
                    return v.range_select((lsb + w - 1) as usize, lsb as usize);
                }
                // §11.5.1: a constant part-select of a packed ARRAY keeps the
                // original element labels, so `(X[15:0])[11]` selects ELEMENT
                // 11 of X — not bit 11 of a 16-bit slice. Port connections
                // like `.p(arr[15:0])` produce exactly this shape after
                // inlining grafts the port's select onto the connection; the
                // slice-of-bits reading returned a single widened bit (0/x)
                // and the consumer never saw the element. Normalize to a plain
                // element select of the underlying signal.
                if let ExprKind::RangeSelect { expr: rs_base, .. } = &expr.kind {
                    if let ExprKind::Ident(h) = &rs_base.kind {
                        let nm = self.resolve_hier_name(h);
                        let elemish = self
                            .module
                            .packed_signal_elem_widths
                            .get(&*nm)
                            .is_some_and(|&ew| ew > 1)
                            || self
                                .module
                                .packed_full_dims
                                .get(&*nm)
                                .is_some_and(|d| d.len() > 1);
                        if elemish {
                            let norm = Expression::new(
                                ExprKind::Index {
                                    expr: rs_base.clone(),
                                    index: index.clone(),
                                },
                                expr.span,
                            );
                            return self.eval_expr_ctx(&norm, ctx_width);
                        }
                    }
                }
                // §7.4.1: an ELEMENT select of a multi-dimensional PACKED
                // array (`logic [3:0][1:0] arr; arr[1]`) has a self-determined
                // width — the ELEMENT width. This context-width evaluator had
                // no case for it and fell through to a one-BIT read, so the
                // value came back X.
                //
                // Only an explicit size cast routes through here, which is why
                // it hid for so long: `arr[1]`, `{6'b0, arr[1]}` and a plain
                // widening assignment were all correct, while `8'(arr[1])` was
                // X. In real RTL the symptom was `1 << arr[i]` and
                // `arr[i] + 1` going X inside a submodule, poisoning
                // everything downstream while the array itself printed fine.
                //
                // Guarded on the base NOT being an unpacked array: for
                // `wire [1:0][7:0] w [0:1]`, `w[0]` indexes the UNPACKED
                // dimension first and must stay on the existing path.
                if let ExprKind::Ident(_h) = &expr.kind {
                    let nm = base_nm.as_deref().unwrap_or("");
                    if !self.module.arrays.contains_key(nm)
                        && (self.signal_name_to_id.contains_key(nm)
                            || self.module.packed_signal_elem_widths.contains_key(nm))
                    {
                        if let Some(&ew) = self.module.packed_signal_elem_widths.get(nm) {
                            // `ew == 1` is a real element width when the inner
                            // packed dimension collapses (`[N-1:0][0:0]`, which
                            // a parameterized design produces when
                            // `$clog2(2) - 1 == 0`); accept it only when the
                            // declared packed dims really are multi-dimensional.
                            let multi_packed = self
                                .module
                                .packed_full_dims
                                .get(nm)
                                .is_some_and(|d| d.len() > 1);
                            if ew > 1 || multi_packed {
                                // Extract here rather than delegating to
                                // `eval_expr`: that path routes multi-dim
                                // packed reads back through this function, so
                                // delegating recurses forever.
                                let base_v = self.eval_expr(expr);
                                if base_v.width as usize > ew as usize {
                                    // §11.5.1: an x/z or out-of-range element
                                    // index reads a whole-x element.
                                    return match self.eval_expr(index).to_index() {
                                        Some(i) => match self.packed_elem_lsb(nm, i, ew) {
                                            Some(lo)
                                                if lo + ew as usize <= base_v.width as usize =>
                                            {
                                                base_v.range_select(lo + ew as usize - 1, lo)
                                            }
                                            _ => {
                                                if self.packed_array_two_state(nm) {
                                                    Value::zero(ew)
                                                } else {
                                                    Value::new(ew)
                                                }
                                            }
                                        },
                                        None => {
                                            if self.packed_array_two_state(nm) {
                                                Value::zero(ew)
                                            } else {
                                                Value::new(ew)
                                            }
                                        }
                                    };
                                }
                            }
                        }
                    }
                }
                if env_set_cached!("XEZIM_EV_DBG") {
                    eprintln!(
                        "[EVDBG] idx-eval base_fmn={:?} qb={:?} ean={:?} nin={:?}",
                        self.flat_member_name(expr),
                        self.indexed_queue_base(expr),
                        self.expr_assoc_name(expr),
                        self.nested_index_name(expr)
                    );
                }
                // §7.4.1: `<obj>.<prop>[i]` on a multi-dimensional PACKED array
                // class property selects an ELEMENT, not a bit. Gated on the
                // receiver's shape so nothing else re-evaluates `expr`.
                if !self.no_class_objects()
                    && matches!(
                        &expr.kind,
                        ExprKind::MemberAccess { .. } | ExprKind::Ident(_) | ExprKind::This
                    )
                {
                    if let Some(r) = self.class_packed_elem_ref(expr, index) {
                        return self.read_class_agg(&r);
                    }
                }
                // §7.4.1 on a CALL result: `mk_vec()[1]` where the return type
                // is a multi-dim packed vector selects an ELEMENT. Without the
                // return type in hand this fell through to a bit-select.
                if let ExprKind::Call { func, .. } = &expr.kind {
                    let shape = self.call_return_type(func).and_then(|ret| {
                        let resolved = Self::resolve_type_ref(&ret, &self.module.typedef_types);
                        let dims = match &resolved {
                            DataType::IntegerVector { dimensions, .. } => dimensions,
                            DataType::Implicit { dimensions, .. } => dimensions,
                            _ => return None,
                        };
                        if dims.len() < 2 {
                            return None;
                        }
                        let b = |d: &crate::ast::types::PackedDimension| {
                            let crate::ast::types::PackedDimension::Range { left, right, .. } = d
                            else {
                                return None;
                            };
                            Some((
                                crate::elaborate::const_eval_i64_with_params(
                                    left,
                                    Some(&self.module.parameters),
                                )?,
                                crate::elaborate::const_eval_i64_with_params(
                                    right,
                                    Some(&self.module.parameters),
                                )?,
                            ))
                        };
                        let (l, r) = b(&dims[0])?;
                        let mut elem_w: u32 = 1;
                        for d in &dims[1..] {
                            let (il, ir) = b(d)?;
                            elem_w = elem_w.checked_mul(((il - ir).unsigned_abs() + 1) as u32)?;
                        }
                        Some((elem_w, l, r))
                    });
                    if let Some((elem_w, l, r)) = shape {
                        // Evaluate the call ONCE, then the index.
                        let base_val = self.eval_expr(expr);
                        if let Some(idx) = self.eval_expr(index).to_i64() {
                            if idx >= l.min(r) && idx <= l.max(r) {
                                let slot = if l >= r {
                                    (l - idx) as u32
                                } else {
                                    (idx - l) as u32
                                };
                                let n = ((l - r).unsigned_abs() + 1) as u32;
                                let off = (n - 1 - slot) * elem_w;
                                if off + elem_w <= base_val.width {
                                    return base_val
                                        .range_select((off + elem_w - 1) as usize, off as usize);
                                }
                            }
                        }
                    }
                }
                // §7.12.2: a locator method (`min`/`max`/`unique`/`find*`)
                // returns a QUEUE, so `a.max()[k]` / `(a.find with (..))[k]`
                // must index that result queue — not bit-select the scalar the
                // reduction path returns. Compute the selected indices and
                // return the k-th (for `_index` forms, the index value itself).
                if let Some((arr_name, mname, filter, iter_name)) = self.locator_call(expr) {
                    let idxs = self.locator_indices_named(
                        &arr_name,
                        &mname,
                        filter.as_ref(),
                        iter_name.as_deref(),
                    );
                    let k = self.eval_expr(index).to_i64().unwrap_or(0);
                    if k < 0 || k as usize >= idxs.len() {
                        return Value::new(32); // out-of-range read -> x
                    }
                    let src_i = idxs[k as usize];
                    if mname.ends_with("_index") {
                        return Value::from_u64(src_i as u64, 32);
                    }
                    return self
                        .get_signal_value_by_name(&format!("{}[{}]", arr_name, src_i))
                        .unwrap_or_else(|| Value::new(32));
                }
                // §7.4.2: an element of an N-D fixed-array CLASS property —
                // `obj.m[i][j]` — resolves to its per-instance cell.
                if !self.no_class_objects() {
                    if let Some(nm) = self.class_nd_elem_name(expr, index) {
                        if let Some(v) = self.get_signal_value_by_name(&nm) {
                            return v;
                        }
                    }
                }
                // Element of an unpacked ARRAY OF QUEUES: `q[i][k]` selects
                // element k of the queue `q[i]` (§7.4.5). Without this the base
                // resolves to a scalar and `[k]` becomes a bit-select.
                if let Some(qn) = self.indexed_queue_base(expr) {
                    // `$` is the last valid index of THIS queue.
                    self.dollar_bound.push(self.get_queue_size(&qn) as i64 - 1);
                    let k = self.eval_expr(index).to_i64().unwrap_or(0);
                    self.dollar_bound.pop();
                    return self
                        .get_signal_value_by_name(&format!("{}[{}]", qn, k))
                        .unwrap_or_else(|| Value::new(32));
                }
                // Element of an unpacked array of ASSOCIATIVE arrays:
                // `a[i][key]` looks the key up in the assoc array `a[i]`.
                if let Some(elem) = self.indexed_assoc_elem(expr, index) {
                    return self
                        .get_signal_value_by_name(&elem)
                        .unwrap_or_else(|| Value::new(32));
                }
                // Multidimensional associative-array read `m[k1][k2]...[kn]`:
                // look up the flat compound key `m[K1][K2]...[Kn]`. Mirrors
                // the write path (`multidim_assoc_elem`) so the two agree on
                // the key for every type (handle / int / signed / string).
                if let Some((base, key)) = self.multidim_assoc_elem(expr, index) {
                    if let Some(v) = self.get_signal_value_by_name(&key) {
                        return v;
                    }
                    // §7.8.6: a missing element reads the element type's
                    // default, at the element's width.
                    return self
                        .assoc_missing_elem_value(&base)
                        .unwrap_or_else(|| Value::new(ctx_width.max(1)));
                }
                // §7.4.2 element of a DYNAMIC ARRAY OF DYNAMIC ARRAYS reached
                // through a class member (`m[i][j]`, `obj.m[i][j]`): the base
                // `m[i]` is itself a dynamic array, stored flat under the
                // instance-scoped row name `<h>#m[i]`.
                if let Some(row) = self.coll_elem_expr_key(expr) {
                    if self.module.dynamic_arrays.contains(&row) {
                        let j = self.eval_expr(index).to_i64().unwrap_or(0);
                        return self
                            .get_signal_value_by_name(&format!("{}[{}]", row, j))
                            .unwrap_or_else(|| Value::zero(32));
                    }
                    // Associative element of an array-of-collections class
                    // property (`aa[0][key]`).
                    if self.is_associative_array(&row) {
                        let kv = self.eval_expr(index);
                        let key = self.assoc_key_str(&row, &kv);
                        return self
                            .get_signal_value_by_name(&format!("{}[{}]", row, key))
                            .unwrap_or_else(|| Value::new(ctx_width.max(1)));
                    }
                }
                // Ascending packed vector bit-select (`logic [0:7] pa; pa[i]`):
                // label i is the MSB end, so it reads internal bit (W-1)-i
                // (LRM §7.4.1, §11.5.1). Whole-value storage is normal.
                if let ExprKind::Ident(_h) = &expr.kind {
                    let nm = base_nm.as_deref().unwrap_or("");
                    if let Some((lo_l, hi_l)) = self.module.ascending_packed.get(nm).copied() {
                        // §11.5.1: unknown or out-of-range label reads x.
                        if let Some(i) = self.eval_expr(index).to_index() {
                            if i >= lo_l && i <= hi_l {
                                let pos = (hi_l - i) as usize;
                                return self.eval_expr(expr).range_select(pos, pos);
                            }
                        }
                        return Value::new(1);
                    }
                }
                // Nested-collection element read: `assoc_of_queue[key][i]`
                // (e.g. `instr_category[cat][i]`). The base `assoc[key]` maps to
                // a compound queue name; read its `[i]` element.
                if let Some(cn) = self.nested_index_name(expr) {
                    // §11.5.1: an x/z index reads x.
                    let Some(i) = self.eval_expr(index).to_index() else {
                        return Value::new(ctx_width.max(1));
                    };
                    return self
                        .get_signal_value_by_name(&format!("{}[{}]", cn, i))
                        .unwrap_or_else(|| Value::zero(32));
                }
                // Class associative-array / queue member element read — base
                // may be a bare member (`m[k]`) or another object's (`obj.m[k]`).
                if let Some(an) = self.expr_assoc_name(expr) {
                    // For a queue member, `$` in the index means last-index;
                    // make its bound available (e.g. `instr_list[$]`).
                    let is_dyn = self.module.dynamic_arrays.contains(&an);
                    if is_dyn {
                        self.dollar_bound
                            .push((self.get_queue_size(&an) as i64) - 1);
                    }
                    let idx_val = self.eval_expr(index);
                    if is_dyn {
                        self.dollar_bound.pop();
                    }
                    // §11.5.1: an x/z index into a queue or dynamic array
                    // reads an all-x element (an associative array's key may
                    // legitimately carry x bits, so it is left alone).
                    if !self.is_associative_array(&an) && idx_val.has_xz() && !idx_val.is_real {
                        let ew = self.assoc_elem_width(&an).unwrap_or(ctx_width.max(1));
                        return Value::new(ew.max(1));
                    }
                    let idx_str = self.assoc_key_str(&an, &idx_val);
                    let elem_name = Self::name_with_key(&an, &idx_str);
                    if let Some(ev) = self.signals.get(&elem_name) {
                        return ev.clone();
                    }
                    // A struct member queue of a FIXED array element
                    // (`fa[1].q[0]`) can have its cells in the signal table.
                    if is_dyn && let Some(ev) = self.table_elem_value(&elem_name) {
                        return ev;
                    }
                    // §7.9.11: a `default:` the array was assigned.
                    if !self.class_assoc_defaults.is_empty()
                        && let Some(v) = self.class_assoc_defaults.get(&an)
                    {
                        return v.clone();
                    }
                    // §7.8.6: a missing key reads the element type's default
                    // (x for `integer m[int]`, a signed 0 for `int`); §7.10.1:
                    // so does a queue or dynamic-array read past the end.
                    return self
                        .assoc_missing_elem_value(&an)
                        .unwrap_or_else(|| Value::zero(32));
                }
                // N-dimensional (N >= 3) unpacked array element access
                {
                    let mut cur = expr.as_ref();
                    let mut rev_idxs: Vec<&Expression> = vec![index.as_ref()];
                    while let ExprKind::Index {
                        expr: inner_e,
                        index: inner_i,
                    } = &cur.kind
                    {
                        rev_idxs.push(inner_i.as_ref());
                        cur = inner_e.as_ref();
                    }
                    if let ExprKind::Ident(hier) = &cur.kind {
                        let mut base_name = self.resolve_hier_name(hier);
                        // Bare class-member N-D array inside a method:
                        // resolve to its per-instance `<handle>#<member>`.
                        // A member shadows a module array of the same name (§8.10).
                        if !self.module.arrays_nd.contains_key(&*base_name)
                            || (hier.path.len() == 1
                                && hier.path[0].selects.is_empty()
                                && *base_name == *hier.path[0].name.name)
                        {
                            if let Some(s) = self.instance_assoc_member(&base_name) {
                                base_name = std::borrow::Cow::Owned(s);
                            }
                        }
                        // `o.m[i][j][k]` on an object's member: its store is
                        // `<handle>#m` (§7.4.5).
                        if !self.module.arrays_nd.contains_key(&*base_name) && hier.path.len() >= 2
                        {
                            if let Some(s) = self
                                .expr_assoc_name(cur)
                                .filter(|s| self.module.arrays_nd.contains_key(s))
                            {
                                base_name = std::borrow::Cow::Owned(s);
                            }
                        }
                        if let Some((shape, w)) = self.module.arrays_nd.get(&*base_name).cloned() {
                            if rev_idxs.len() == shape.len() {
                                let mut name = base_name.clone();
                                for i in (0..rev_idxs.len()).rev() {
                                    let v =
                                        self.eval_expr(rev_idxs[i]).to_u64().unwrap_or(0) as i64;
                                    name = std::borrow::Cow::Owned(format!("{}[{}]", name, v));
                                }
                                if let Some(&eid) = self.signal_name_to_id.get(name.as_ref()) {
                                    let mut v = self.signal_table[eid].clone();
                                    if self.signal_signed[eid] {
                                        v.is_signed = true;
                                    }
                                    return v;
                                }
                                if let Some(sv) = self.signals.get(&*name) {
                                    return sv.clone();
                                }
                                return Value::new(w);
                            }
                        }
                    }
                }
                // 2D array element access: mem[i][j]
                if let ExprKind::Index {
                    expr: inner_expr,
                    index: inner_idx,
                } = &expr.kind
                {
                    if let ExprKind::Ident(hier) = &inner_expr.kind {
                        let mut name = self.resolve_hier_name(hier);
                        // Bare class-member 2D array inside a method:
                        // resolve to its per-instance `<handle>#<member>`.
                        // A member shadows a module array of the same name (§8.10).
                        if !self.module.arrays_2d.contains_key(&*name)
                            || (hier.path.len() == 1
                                && hier.path[0].selects.is_empty()
                                && *name == *hier.path[0].name.name)
                        {
                            if let Some(s) = self.instance_assoc_member(&name) {
                                name = std::borrow::Cow::Owned(s);
                            }
                        }
                        // `o.m[i][j]` on an object's member: its store is
                        // `<handle>#m` (§7.4.5).
                        if !self.module.arrays_2d.contains_key(&*name) && hier.path.len() >= 2 {
                            if let Some(s) = self
                                .expr_assoc_name(inner_expr)
                                .filter(|s| self.module.arrays_2d.contains_key(s))
                            {
                                name = std::borrow::Cow::Owned(s);
                            }
                        }
                        if self.module.arrays_2d.contains_key(&*name) {
                            // §11.5.1: an x/z index reads an all-x element.
                            let (Some(i), Some(j)) = (
                                self.eval_expr(inner_idx).to_index(),
                                self.eval_expr(index).to_index(),
                            ) else {
                                let ew = self
                                    .module
                                    .arrays_2d
                                    .get(&*name)
                                    .map(|d| d.2)
                                    .unwrap_or(ctx_width.max(1));
                                return Value::new(ew.max(1));
                            };
                            let elem_name = format!("{}[{}][{}]", name, i, j);
                            if let Some(&eid) = self.signal_name_to_id.get(elem_name.as_str()) {
                                let mut v = self.signal_table[eid].clone();
                                if self.signal_signed[eid] {
                                    v.is_signed = true;
                                }
                                return v;
                            }
                            if let Some(sv) = self.signals.get(&elem_name) {
                                return sv.clone();
                            }
                            let w = self.module.arrays_2d.get(&*name).map(|t| t.2).unwrap_or(1);
                            return Value::new(w);
                        }
                    }
                }
                // §7.4.2: an element of a struct member with two or more
                // unpacked dimensions is its own leaf.
                if matches!(expr.kind, ExprKind::Index { .. })
                    && self.multidim_members != Some(false)
                {
                    if let Some(leaf) = self.struct_member_elem_leaf(expr, index) {
                        if let Some(v) = self.get_signal_value_by_name(&leaf) {
                            return v;
                        }
                    }
                }
                // Check if this is an array element access (memory[idx]) vs bit select
                // The base may be a flat/hier Ident OR a MemberAccess chain
                // (`u_h.mem[q]` from a parent scope) — mirror of the same
                // shape in assign_value.
                let idx_base_name: Option<std::borrow::Cow<str>> = match &expr.kind {
                    ExprKind::Ident(hier) => Some(self.resolve_hier_name(hier)),
                    ExprKind::MemberAccess { .. } => {
                        self.flat_member_name(expr).map(std::borrow::Cow::Owned)
                    }
                    _ => None,
                };
                if let Some(mut name) = idx_base_name {
                    // `ClassName.staticColl[i]` / `ClassName::staticColl[i]` —
                    // a static-collection element read reaching the Index
                    // handler as a qualified dotted name (the MemberAccess
                    // form `(ClassName).coll` that `flat_member_name` renders
                    // as `ClassName.coll`). The static collection itself is
                    // registered under the BARE collection name (module scope
                    // resolves the same `::` form to `q` via the hier collapse),
                    // so a dotted `Class.coll` never matches
                    // `dynamic_arrays`/`arrays`/`associative_arrays` here and
                    // falls through to a scalar bit-select — reading null.
                    if let Some((cls, coll)) = name.split_once('.') {
                        // §8.25: the leading segment may be a class-LOCAL
                        // TYPEDEF alias of a parameterized class (`this_type`
                        // -> `seqlib#(REQ,RSP)`), which the parser folds into
                        // a dotted member access `this_type.member`. Resolve it
                        // to the concrete per-spec class, so a STATIC-collection
                        // ELEMENT read `this_type::m[i]` keys under the per-spec
                        // cell (`seqlib#item,item::m`) that the ::-form size()
                        // and bare static writes share — NOT the literal
                        // `this_type.m` (which reads empty).
                        let real_cls = if self.module.classes.contains_key(cls) {
                            Some(cls.to_string())
                        } else {
                            self.resolve_typedef_spec(cls)
                                .map(|(b, _)| b)
                                .filter(|b| self.module.classes.contains_key(b))
                        };
                        if let Some(real_cls) = real_cls
                            && self.member_is_static_coll(&real_cls, coll)
                        {
                            // Sibling-collision storage is per-DECLARING class
                            // (`ClassName::coll` — e.g. the four
                            // `uvm_cmdline_set_*` classes each declaring
                            // `static … settings[$]`); a non-colliding static
                            // collection keeps the BARE name its accessors and
                            // store use. Match `spec_static_coll_key`.
                            name = if self.static_coll_name_collides(coll) {
                                std::borrow::Cow::Owned(
                                    self.static_prop_key(&real_cls, coll)
                                        .unwrap_or_else(|| coll.to_string()),
                                )
                            } else if self.class_is_parameterized(&real_cls) {
                                // PARAMETERIZED class: elements live
                                // per-specialization via the qualified form
                                // (§8.25) — see the matching receiver rewrite
                                // in the MemberAccess handler. For a TYPEDEF
                                // alias of a parameterized class (`this_type`)
                                // resolve its specialization explicitly, since
                                // the MemberAccess pre-rewrite only fires for a
                                // plain class receiver.
                                if let Some((b, sig)) = self.resolve_typedef_spec(cls) {
                                    std::borrow::Cow::Owned(format!(
                                        "{}#{}::{}",
                                        b,
                                        self.canonicalize_spec_sig(&b, &sig),
                                        coll
                                    ))
                                } else {
                                    name
                                }
                            } else {
                                std::borrow::Cow::Owned(coll.to_string())
                            };
                        }
                    }
                    if let Some(scoped) = self.instance_assoc_member(&name) {
                        name = std::borrow::Cow::Owned(scoped);
                    }
                    // A bare name inside a SUBMODULE process resolves under
                    // the process's instance scope (name_resolve_hint), like
                    // scalar reads/writes already do.
                    if !self.module.arrays.contains_key(&*name) && !self.is_associative_array(&name)
                    {
                        if let Some(scoped) = self.hint_scoped(&name) {
                            if self.module.arrays.contains_key(&scoped)
                                || self.is_associative_array(&scoped)
                            {
                                name = std::borrow::Cow::Owned(scoped);
                            }
                        }
                    }
                    if self.module.arrays.contains_key(&*name)
                        || self.module.dynamic_arrays.contains(&*name)
                        || self.is_associative_array(&name)
                        || self.is_per_spec_dynamic_static(&name)
                    {
                        // Check if `name` is a queue/dynamic array. This
                        // handles struct-field sub-paths (`info.addr`) and
                        // class-property paths (`c.addr`) that aren't in
                        // `arrays` or `associative_arrays` but ARE in
                        // `dynamic_arrays` (registered by assignment-pattern
                        // handling on first write). Without this check, an
                        // element read like `info.addr[0]` returns X because
                        // the enum arm exits before reaching the dynamic-array
                        // signal lookup below.
                        if self.module.dynamic_arrays.contains(&*name) {
                            // fall through to the element lookup below
                        }
                        // Per-spec storage key for STATIC collection properties
                        // of parameterized classes: membership stays on the
                        // bare `name` (registered globally), but element/size
                        // STORAGE must use the spec-aware key so each
                        // specialization gets its own cells (matching
                        // `eval_builtin_method`'s push_back/size).
                        let store_name = self.spec_static_coll_key(&name);
                        // Array element access: look up signal "name[idx]"
                        if self.module.dynamic_arrays.contains(&*name) {
                            self.dollar_bound
                                .push((self.get_queue_size(&store_name) as i64) - 1);
                        }
                        let idx_val = self.eval_expr(index);
                        if self.module.dynamic_arrays.contains(&*name) {
                            self.dollar_bound.pop();
                        }
                        // A side-effecting selector bypasses speculative
                        // probes; validate its single evaluated result here
                        // before either fixed-array storage lookup.
                        if !self.is_associative_array(&name) && idx_val.has_xz() {
                            if let Some(ew) = self.module.arrays.get(&*name).map(|a| a.2) {
                                return Value::new(ew.max(1));
                            }
                        }
                        // Fast path for 1-D arrays: compute signal_id directly
                        // via `array_first_id`. Critical for LARGE arrays
                        // (e.g. xezim_mem[0:4M]) where per-element names are
                        // NOT registered in `signal_name_to_id` — the
                        // string-keyed fallback below would return X for
                        // every element, silently corrupting reads.
                        if !self.is_associative_array(&name) {
                            if let Some(&(first_id, lo, hi)) =
                                self.array_first_id.get(name.as_ref())
                            {
                                // §7.4.6: SIGNED index — negative-lo arrays.
                                // §11.5.1: an x/z index reads an all-x element.
                                let Some(idx) = idx_val.to_index() else {
                                    let ew = self
                                        .module
                                        .arrays
                                        .get(&*name)
                                        .map(|a| a.2)
                                        .unwrap_or(ctx_width.max(1));
                                    return Value::new(ew.max(1));
                                };
                                if idx >= lo && idx <= hi {
                                    let eid = first_id + (idx - lo) as usize;
                                    // Packed-arena cell: no signal_table slot
                                    // (found by the #147 race test's AST-path
                                    // `$display("%h", mem[5])` — the clone
                                    // below panicked on the arena id).
                                    if is_packed_id(eid) {
                                        return self.packed.read(eid);
                                    }
                                    let mut v = self.signal_table[eid].clone();
                                    if self.signal_signed[eid] {
                                        v.is_signed = true;
                                    }
                                    return v;
                                }
                                // §7.4.6 / Table 7-1: an invalid fixed-array
                                // index yields the element type's default,
                                // fitted to its width and signedness.
                                if !self.module.dynamic_arrays.contains(&*name) {
                                    if let Some(ew) = self.module.arrays.get(&*name).map(|a| a.2) {
                                        return self
                                            .assoc_missing_elem_value(&name)
                                            .unwrap_or_else(|| Value::new(ew.max(1)));
                                    }
                                }
                            }
                        }
                        // §7.8.2: reading with an x/z index yields the element
                        // type's default rather than folding to another key.
                        if self.is_associative_array(&name) && idx_val.has_xz() {
                            self.warn_invalid_assoc_index(&name);
                            return Value::zero(ctx_width.max(1));
                        }
                        let idx_str = if self.is_associative_array(&name) {
                            self.assoc_key_str(&name, &idx_val)
                        } else {
                            // §7.4.6: match the declared (signed) element
                            // spelling — `nmem[-2]`, not `nmem[4294967294]`.
                            idx_val.to_i64().unwrap_or(0).to_string()
                        };
                        let elem_name = Self::name_with_key(&store_name, &idx_str);
                        if let Some(&eid) = self.signal_name_to_id.get(elem_name.as_str()) {
                            let mut v = self.signal_table[eid].clone();
                            if self.signal_signed[eid] {
                                v.is_signed = true;
                            }
                            return v;
                        }
                        let mut v = if let Some(sv) = self.signals.get(&elem_name) {
                            sv.clone()
                        } else if let Some(def_expr) =
                            self.module.assoc_defaults.get(&*name).cloned()
                        {
                            self.eval_expr(&def_expr)
                        } else {
                            // §7.8.6: reading a NONEXISTENT element yields the
                            // element type's default initial value — 0 for a
                            // 2-state type, x for a 4-state one — at the
                            // ELEMENT's width. A flat 1-bit x was wrong twice
                            // over: `int aa[int]; aa[7]` read x instead of 0,
                            // and even where x was right the 1-bit width made
                            // `%h` print a single `x` for what should be `xx`.
                            // (The read still must not CREATE the element —
                            // `num()` is unchanged, which is why this is a
                            // value-only fallback.) The declared type also
                            // gives a subroutine-local array its default (it
                            // read a 1-bit x) and the element's signedness,
                            // so `aa[k] -= 5` on a new `int` key is -5.
                            if let Some(v) = self.assoc_missing_elem_value(&name) {
                                v
                            } else {
                                let ew = self.assoc_elem_width(&name).unwrap_or(1).max(1);
                                if self.module.two_state_signals.contains(&*name)
                                    || name
                                        .rsplit('.')
                                        .next()
                                        .is_some_and(|l| self.module.two_state_signals.contains(l))
                                {
                                    Value::zero(ew)
                                } else {
                                    Value::new(ew)
                                }
                            }
                        };
                        if self.signed_signals.contains(&elem_name) {
                            v.is_signed = true;
                        }
                        return v;
                    }
                }
                // String character read: `s[i]` returns the i-th character
                // (byte), `s[0]` being the leftmost. Index the actual string
                // CONTENT, not `width/8`: a string variable is stored in a
                // fixed-width container (e.g. 1024 bits), so `width/8` is the
                // container size, not the string length — indexing that read
                // the empty leading bytes and always returned 0.
                if let ExprKind::Ident(_hier) = &expr.kind {
                    let nm = base_nm.as_deref().unwrap_or("");
                    // The running task frame decides for its own formals: a
                    // string flag another frame left on the name is not it.
                    if self.frame_name_is_string(nm) {
                        let content = self.eval_expr(expr).sv_string_bytes();
                        let i = self.eval_expr(index).to_u64().unwrap_or(0) as usize;
                        let byte = content.get(i).copied().unwrap_or(0);
                        return Self::sv_byte(byte);
                    }
                }
                // Nested packed multi-D select (`a[i][j]` on
                // `reg [1:0][15:0][7:0] a;`) — resolve the whole index chain
                // against the signal's full packed dims, normalizing indices
                // for ascending / non-zero-based / negative ranges (§7.4.1).
                if let Some((base, lo_opt, w)) = self.packed_nested_select(expr, index) {
                    // §13.5, §6.21: a formal or local of the running frame
                    // shadows a design signal of its name (an `initial`
                    // block's static `a` beside a function's `a`, #650).
                    let frame_v = self.frame_get(&base).cloned();
                    if let Some(base_v) = frame_v.or_else(|| self.get_signal_value_by_name(&base)) {
                        return match lo_opt {
                            Some(lo) => base_v.range_select(lo + w as usize - 1, lo),
                            None => {
                                if self.packed_array_two_state(&base) {
                                    Value::zero(w)
                                } else {
                                    Value::new(w)
                                }
                            }
                        };
                    }
                }
                // §7.4.1: MULTI-LEVEL packed element select — `x[i][j]` on
                // `T [0:0][1:0] x;` peels the whole index chain against the
                // DECLARED packed dims. The single-level arm below knows only
                // one element width, so the second level degraded to a BIT
                // select of the first slice ($bits(x[0][0]) read 1 and every
                // nested-element read returned one bit).
                {
                    let mut rev_idx_exprs: Vec<&Expression> = vec![index];
                    let mut cur: &Expression = expr;
                    while let ExprKind::Index { expr: b, index: i2 } = &cur.kind {
                        rev_idx_exprs.push(i2);
                        cur = b;
                    }
                    if rev_idx_exprs.len() >= 2 {
                        if let ExprKind::Ident(h) = &cur.kind {
                            let nm = self.resolve_hier_name(h);
                            if let Some(dims) = self.module.packed_full_dims.get(&*nm).cloned() {
                                if dims.len() >= rev_idx_exprs.len() {
                                    let base_v = self.eval_expr(cur);
                                    let total = base_v.width as u64;
                                    let mut w = total;
                                    let mut lsb: i64 = 0;
                                    let mut oob = false;
                                    let mut xz = false;
                                    for (k, ie) in rev_idx_exprs.iter().rev().enumerate() {
                                        let (dl, dr) = dims[k];
                                        let count = (dl - dr).unsigned_abs() + 1;
                                        if count == 0 || w % count != 0 {
                                            oob = true;
                                            break;
                                        }
                                        w /= count;
                                        let iv = self.eval_expr(ie);
                                        if iv.has_xz() {
                                            xz = true;
                                            break;
                                        }
                                        let i = iv.to_i64().unwrap_or(i64::MIN);
                                        let lo_b = dl.min(dr);
                                        let off = i - lo_b;
                                        if off < 0 || off as u64 >= count {
                                            oob = true;
                                            break;
                                        }
                                        let slot = if dl >= dr {
                                            off
                                        } else {
                                            (count as i64 - 1) - off
                                        };
                                        lsb += slot * w as i64;
                                    }
                                    if xz || oob {
                                        // §11.5.1: x/z or out-of-range index
                                        // reads all-x at the element width.
                                        // A TWO-STATE (bit/int) packed array
                                        // cannot hold x: relativize to a known
                                        // zero element (reference simulators).
                                        return if self.packed_array_two_state(&*nm) {
                                            Value::zero(w.max(1) as u32)
                                        } else {
                                            Value::new(w.max(1) as u32)
                                        };
                                    }
                                    return base_v
                                        .range_select((lsb as u64 + w - 1) as usize, lsb as usize);
                                }
                            }
                        }
                    }
                }
                // Packed multi-D signal element select (LRM §7.4.1): for
                // `logic [3:0][7:0] w;`, `w[i]` selects the i-th 8-bit slice
                // rather than the i-th bit. Triggered when the signal's name
                // (or `obj.field`) is in `packed_signal_elem_widths`.
                {
                    // Build candidate base names. For `obj.field[i]` the
                    // expression may parse as either MemberAccess or as a
                    // multi-segment Ident; try both forms.
                    let mut candidates: Vec<String> = Vec::new();
                    match &expr.kind {
                        ExprKind::Ident(h) => {
                            candidates.push(self.resolve_hier_name(h).into_owned());
                            if h.path.len() >= 2 {
                                let dotted: String = h
                                    .path
                                    .iter()
                                    .map(|s| s.name.name.as_str())
                                    .collect::<Vec<_>>()
                                    .join(".");
                                candidates.push(dotted);
                            }
                        }
                        ExprKind::MemberAccess { expr: e, member } => {
                            if let ExprKind::Ident(h) = &e.kind {
                                candidates.push(format!(
                                    "{}.{}",
                                    self.resolve_hier_name(h),
                                    member.name
                                ));
                            }
                            // `arr[i].field[j]` — the receiver is an ELEMENT of
                            // an unpacked array of structs, so the base parses
                            // as MemberAccess over an Index and neither arm
                            // above fires. Try the flattened element name
                            // (`arr[2].wdata`, the concrete signal) and the
                            // DECLARATION-keyed form with the element index
                            // stripped (`arr.wdata`) — the field's element
                            // width is the same for every element. Without
                            // these the select degraded to a 1-BIT read, so a
                            // reference model comparing `ref[i].wdata[j]`
                            // against a 64-bit lane checked one bit of it.
                            if let Some(flat) = self.flat_member_name(expr) {
                                if let (Some(bl), Some(br)) = (flat.find('['), flat.find(']')) {
                                    if bl < br {
                                        candidates.push(format!(
                                            "{}{}",
                                            &flat[..bl],
                                            &flat[br + 1..]
                                        ));
                                    }
                                }
                                candidates.push(flat);
                            }
                        }
                        _ => {}
                    };
                    // A hierarchical path through a PORT (`dut.result.member`)
                    // names the formal, but the port collapsed onto the
                    // parent's signal, so the width map is keyed by THAT
                    // (`r.member`). Rewrite the base through `port_aliases`.
                    // Without it the select degraded to a 1-BIT read, so a
                    // structural check comparing `dut.result.field[i]` against
                    // a 16-bit lane compared a single bit and always failed.
                    {
                        let mut aliased: Vec<String> = Vec::new();
                        for c in &candidates {
                            if let Some(dot) = c.rfind('.') {
                                let (base, member) = (&c[..dot], &c[dot..]);
                                if let Some(parent) = self.module.port_aliases.get(base) {
                                    aliased.push(format!("{}{}", parent, member));
                                }
                            }
                        }
                        candidates.extend(aliased);
                    }
                    if let Some((matched_key, &elem_w)) = candidates
                        .iter()
                        .find_map(|n| self.module.packed_signal_elem_widths.get_key_value(n))
                    {
                        let matched_key = matched_key.clone();
                        let idx = self.eval_expr(index).to_u64().unwrap_or(0) as usize;
                        // For `pp.words[i]`, the base eval reads the WHOLE
                        // struct, so we slice at struct-field-offset +
                        // i*elem_w. Detect 2-segment Ident form and route
                        // through packed_struct_fields.
                        if let ExprKind::Ident(h) = &expr.kind {
                            if h.path.len() == 2 {
                                let struct_var = h.path[0].name.name.as_str();
                                let field_name = h.path[1].name.name.as_str();
                                if let Some(fields) =
                                    self.module.packed_struct_fields.get(struct_var).cloned()
                                {
                                    if let Some((_, field_off, _field_w)) =
                                        fields.iter().find(|(m, _, _)| m == field_name).cloned()
                                    {
                                        if let Some(struct_v) =
                                            self.get_signal_value_by_name(struct_var)
                                        {
                                            let lo = (field_off as usize) + idx * (elem_w as usize);
                                            let hi = lo + (elem_w as usize) - 1;
                                            return struct_v.range_select(hi, lo);
                                        }
                                    }
                                }
                            }
                        }
                        let base_v = self.eval_expr(expr);
                        let Some(lo) = self.packed_elem_lsb(&matched_key, idx as i64, elem_w)
                        else {
                            if self.packed_array_two_state(&matched_key) {
                                return Value::zero(elem_w);
                            }
                            return Value::new(elem_w);
                        };
                        let hi = lo + (elem_w as usize) - 1;
                        return base_v.range_select(hi, lo);
                    }
                }
                // §6.16.3 `str[i]` is a CHARACTER select, not a bit select.
                // That works for a plain `string` variable, whose name is in
                // `string_signals` — but an ELEMENT of a string collection
                // (`q[0][3]`, `arr[i][0]`) has an Index base, so it missed the
                // name-keyed check and fell through to a 1-bit select, reading
                // empty even though `len()` reported the right length.
                if matches!(
                    expr.kind,
                    ExprKind::Index { .. } | ExprKind::MemberAccess { .. }
                ) {
                    if let Some(fname) = self.flat_member_name(expr) {
                        if self.is_string_collection(&fname) {
                            let text = self.eval_expr(expr).sv_string_bytes();
                            let i = self.eval_expr(index).to_u64().unwrap_or(0) as usize;
                            let b = text.get(i).copied().unwrap_or(0);
                            return Self::sv_byte(b);
                        }
                    }
                }
                // `s.arr[i]` READ where `arr` is an UNPACKED ARRAY MEMBER of
                // an unpacked struct — mirror of the write arm in
                // `assign_value`. The elements are individual signals and the
                // base is not a signal, so this otherwise bit-selected a
                // phantom scalar and always read x.
                {
                    // `s.arr[i]` where `s` may itself be an element
                    // (`q[0].arr[i]`), so the base is a MemberAccess over an
                    // Index rather than a plain Ident.
                    let base = match &expr.kind {
                        ExprKind::Ident(h) => Some(self.resolve_hier_name(h)),
                        ExprKind::MemberAccess { .. } => {
                            self.flat_member_name(expr).map(std::borrow::Cow::Owned)
                        }
                        _ => None,
                    };
                    if let Some(base) = base {
                        if !self.signal_name_to_id.contains_key(base.as_ref())
                            && !self.signals.contains_key(&*base)
                        {
                            // §11.5.1: an x/z index reads an all-x element,
                            // sized from the array's element width.
                            let Some(i) = self.eval_expr(index).to_index() else {
                                let ew = self
                                    .module
                                    .arrays
                                    .get(base.as_ref())
                                    .map(|a| a.2)
                                    .or_else(|| self.assoc_elem_width(base.as_ref()))
                                    .unwrap_or(ctx_width.max(1));
                                return Value::new(ew.max(1));
                            };
                            let elem = Self::name_with_index(&base, i);
                            if let Some(v) = self.get_signal_value_by_name(&elem) {
                                return v;
                            }
                            if let Some(elem) = self.class_coll_elem_leaf(&base, i) {
                                if let Some(v) = self.get_signal_value_by_name(&elem) {
                                    return v;
                                }
                            }
                            // §7.4.6: a declared array with no such element
                            // is an out-of-range read: x at the element
                            // width (the bit-select fallback below made it
                            // a 1-bit x that a store zero-extended).
                            if let Some(ew) = self.module.arrays.get(base.as_ref()).map(|a| a.2) {
                                return Value::new(ew.max(1));
                            }
                        }
                    }
                }
                // Fall back to bit select
                if let Some((dl, dr)) = self.class_prop_select_dim(expr) {
                    return match self.eval_expr(index).to_index() {
                        Some(i) if i >= dl.min(dr) && i <= dl.max(dr) => {
                            let physical = if dl < dr { dr - i } else { i - dr };
                            self.eval_expr(expr).bit_select(physical as usize)
                        }
                        _ => Value::new(1),
                    };
                }
                if let ExprKind::Ident(h) = &expr.kind {
                    let nm = self.resolve_hier_name(h);
                    if let Some(dims) = self.module.packed_full_dims.get(&*nm) {
                        if let Some(&(dl, dr)) = dims.first() {
                            let lo_b = dl.min(dr);
                            if lo_b != 0 {
                                // §11.5.1: below the declared low bound (or an
                                // x/z index) reads x, not the low bit.
                                return match self.eval_expr(index).to_index() {
                                    Some(i) if i >= lo_b => {
                                        self.eval_expr(expr).bit_select((i - lo_b) as usize)
                                    }
                                    _ => Value::new(1),
                                };
                            }
                        }
                    }
                }
                match self.eval_expr(index).to_index() {
                    Some(i) if i >= 0 => self.eval_expr(expr).bit_select(i as usize),
                    _ => Value::new(1),
                }
            }
            ExprKind::RangeSelect {
                expr,
                left,
                right,
                kind,
                ..
            } => {
                // A module-level plain name is never a class property.
                let plain = matches!(&expr.kind, ExprKind::Ident(h) if h.path.len() == 1);
                if (!plain || !self.this_stack.is_empty())
                    && let Some(v) = self.class_packed_part_select(expr, *kind, left, right)
                {
                    return v;
                }
                // Unpacked array slice: concatenate elements
                if let ExprKind::Ident(hier) = &expr.kind {
                    let name = self.resolve_hier_name(hier);
                    if let Some(&(arr_lo, arr_hi, elem_w)) = self.module.arrays.get(&*name) {
                        let is_dyn = self.module.dynamic_arrays.contains(&*name);
                        let upper_bound: i64 = if is_dyn {
                            (self.get_queue_size(&name) as i64) - 1
                        } else {
                            arr_hi
                        };
                        self.dollar_bound.push(upper_bound);
                        let l = self.eval_expr(left).to_i64().unwrap_or(0);
                        let r = self.eval_expr(right).to_i64().unwrap_or(0);
                        self.dollar_bound.pop();
                        // §7.10.1: `q[a:b]` with a > b is the EMPTY queue
                        // (`q[3:1]`, and `q[n:$]` on n elements) — not the
                        // swapped range.
                        if is_dyn && matches!(kind, RangeKind::Constant) && l > r {
                            return Value::zero(0);
                        }
                        let (lo, hi) = match kind {
                            RangeKind::Constant => (l.min(r), l.max(r)),
                            RangeKind::IndexedUp => (l, l + r - 1),
                            RangeKind::IndexedDown => (l - r + 1, l),
                        };
                        if hi < lo {
                            return Value::zero(0);
                        }
                        let lo = lo.max(arr_lo);
                        let hi = hi.min(if is_dyn { upper_bound } else { arr_hi });
                        if hi < lo {
                            return Value::zero(0);
                        }
                        let count = (hi - lo + 1) as usize;
                        let total_w = count as u32 * elem_w;
                        let mut acc = Value::zero(total_w);
                        for i in 0..count {
                            let idx = lo + i as i64;
                            if idx < arr_lo || idx > arr_hi {
                                continue;
                            }
                            let elem_name = format!("{}[{}]", name, idx);
                            let v = self
                                .get_signal_value_by_name(&elem_name)
                                .unwrap_or(Value::zero(elem_w));
                            for b in 0..elem_w as usize {
                                acc.set_bit(i * elem_w as usize + b, v.get_bit(b));
                            }
                        }
                        return acc;
                    }
                }
                // Packed multi-D ELEMENT range select (LRM §7.4.1): for
                // `logic [1:0][63:0] pv;` or a packed array of a struct typedef
                // (`bundle_t [1:0] arr;`), a constant `arr[1:0]` selects BOTH
                // elements — 2×elem_w bits — not bits 1..0. This mirrors the
                // single-element select above; without it a port actual written
                // as `.p(arr[1:0])` carried 2 bits into a 256-bit port.
                {
                    if let ExprKind::Ident(h) = &expr.kind {
                        let nm = self.resolve_hier_name(h);
                        if let Some(&elem_w) = self
                            .module
                            .packed_signal_elem_widths
                            .get(&*nm)
                            .filter(|&&w| w > 1)
                        {
                            let li = self.eval_expr(left).to_i64().unwrap_or(0);
                            let ri = self.eval_expr(right).to_i64().unwrap_or(0);
                            // Label bounds per select kind; `[i +: c]` covers
                            // elements i..i+c-1, `[i -: c]` covers i-c+1..i
                            // (§7.4.1 + §11.5.1 element semantics).
                            let (l, r): (i64, i64) = match kind {
                                RangeKind::Constant => (li, ri),
                                RangeKind::IndexedUp => (li, li + ri - 1),
                                RangeKind::IndexedDown => (li, li - (ri - 1)),
                            };
                            // LSB offset of a labeled element under the DECLARED
                            // outer range: `[N-1:0]` reduces to idx*elem_w, an
                            // ascending `[0:1]` reverses slot order.
                            let dim = self
                                .module
                                .packed_full_dims
                                .get(&*nm)
                                .and_then(|d| d.first())
                                .copied();
                            let lsb_of = |idx: i64| -> i64 {
                                match dim {
                                    None => idx * elem_w as i64,
                                    Some((dl, dr)) => {
                                        let (lo_b, hi_b) = (dl.min(dr), dl.max(dr));
                                        let count = hi_b - lo_b + 1;
                                        let off = idx - lo_b;
                                        let slot = if dl >= dr { off } else { count - 1 - off };
                                        slot * elem_w as i64
                                    }
                                }
                            };
                            let lsb_l = lsb_of(l);
                            let lsb_r = lsb_of(r);
                            let lo = lsb_l.min(lsb_r);
                            let hi = lsb_l.max(lsb_r) + elem_w as i64 - 1;
                            let base_v = self.eval_expr(expr);
                            return oob_range_select(&base_v, hi, lo);
                        }
                    }
                }
                // Ascending packed vector part-select (`logic [0:7] pa; pa[0:3]`):
                // labels index from the MSB end, so map [a:b] to internal
                // [(W-1)-a : (W-1)-b] (LRM §7.4.1, §11.5.1).
                if let ExprKind::Ident(h) = &expr.kind {
                    let nm = self.resolve_hier_name(h);
                    if let Some((_, hi_l)) = self.module.ascending_packed.get(&*nm).copied() {
                        // Label p is physical bit `high - p`. Every position
                        // outside the declared range reads x (§11.5.1), which
                        // `range_select_signed` does per bit; an x/z bound
                        // makes the whole select x.
                        let base = self.eval_expr(expr);
                        let (av, bv) = (self.eval_expr(left), self.eval_expr(right));
                        let (Some(a), Some(b)) = (av.to_index(), bv.to_index()) else {
                            let n = match kind {
                                RangeKind::Constant => 1,
                                _ => bv.to_u64().unwrap_or(1).max(1),
                            };
                            return Value::new(n as u32);
                        };
                        let (hi, lo) = match kind {
                            RangeKind::Constant => (hi_l - a.min(b), hi_l - a.max(b)),
                            RangeKind::IndexedUp => (hi_l - a, hi_l - a - (b - 1)),
                            RangeKind::IndexedDown => (hi_l - a + (b - 1), hi_l - a),
                        };
                        return oob_range_select(&base, hi, lo);
                        #[allow(unreachable_code)]
                        {
                            // §11.5.1: the INDEXED forms need the same mapping. Only
                            // the constant form was remapped, so `av[4 +: 4]` on a
                            // `logic [0:15]` read the bits a DESCENDING vector would
                            // have — the labels run from the MSB end, and a bit
                            // select on the same vector already honoured that, so
                            // the two disagreed about what `av[4]` meant.
                        }
                    }
                }
                let base = self.eval_expr(expr);
                // §11.5.1: a part-select bound may be NEGATIVE — those bits read
                // x, they are not dropped. Reading the bounds through `to_u64`
                // turned a negative base into a huge unsigned index, so
                // `w[-4 +: 8]` computed `l + r - 1` on 0xFFFF_FFFC and panicked
                // in the shift rather than returning `fx`. `to_i64` only
                // sign-extends a value whose type IS signed, so an ordinary
                // unsigned index is unaffected.
                //
                // Offset a part-select's bit indices by the declared lower
                // bound of a PLAIN packed vector (`logic [7:4] v; v[6:5]` ->
                // storage bits [2:1]), mirroring the WRITE path. In
                // `[base +: W]` / `[base -: W]` the right operand is a WIDTH,
                // not an index, so it is not shifted — only the constant
                // `[msb:lsb]` form has two indices (LRM §7.4.1 / §11.5.1).
                let (li_v, ri_v) = (self.eval_expr(left), self.eval_expr(right));
                let (Some(mut li), Some(mut ri)) = (li_v.to_index(), ri_v.to_index()) else {
                    // §11.5.1: an x/z bound selects nothing: the whole
                    // select reads x at the select's width.
                    let n = match kind {
                        RangeKind::Constant => 1,
                        _ => ri_v.to_u64().unwrap_or(1).max(1),
                    };
                    return Value::new(n as u32);
                };
                // Plain packed vector: the declared lower bound offsets the
                // labels (`logic [7:4] v; v[6:5]` → storage bits [2:1]),
                // mirroring the WRITE path. Ascending plain vectors were
                // intercepted by the ascending arm above.
                let plain_dim: Option<(i64, i64)> = match &expr.kind {
                    ExprKind::Ident(h) => {
                        let nm = self.resolve_hier_name(h);
                        self.module
                            .packed_full_dims
                            .get(&*nm)
                            .and_then(|d| d.first())
                            .copied()
                    }
                    _ => None,
                }
                .or_else(|| self.class_prop_select_dim(expr));
                if let Some((dl, dr)) = plain_dim.filter(|&(l, r)| l >= r) {
                    let _ = dl;
                    if dr != 0 {
                        li -= dr;
                        if matches!(kind, RangeKind::Constant) {
                            ri -= dr;
                        }
                    }
                } else if let Some((dl, dr)) = plain_dim.or_else(|| self.select_base_elem_dim(expr))
                {
                    // ELEMENT of an unpacked array/queue/assoc (or a packed
                    // element): `logic [31:8] q[$]; q[0][31:8]` selects the
                    // whole element (this read xxabcd — bits 23:8 plus
                    // out-of-range x — before). Descending offsets the labels
                    // by the low bound; an ASCENDING dim mirrors them
                    // (physical = right - label), handled by a direct select
                    // since the shared tail below is descending-only.
                    if dl < dr {
                        let phys_hi: i64;
                        let phys_lo: i64;
                        match kind {
                            RangeKind::Constant => {
                                phys_hi = dr - li.min(ri);
                                phys_lo = dr - li.max(ri);
                            }
                            RangeKind::IndexedUp => {
                                phys_hi = dr - li;
                                phys_lo = dr - li - (ri - 1);
                            }
                            RangeKind::IndexedDown => {
                                phys_hi = dr - li + (ri - 1);
                                phys_lo = dr - li;
                            }
                        }
                        return oob_range_select(&base, phys_hi, phys_lo);
                    }
                    let lo_b = dr.min(dl);
                    if lo_b != 0 {
                        li -= lo_b;
                        if matches!(kind, RangeKind::Constant) {
                            ri -= lo_b;
                        }
                    }
                }
                let l = li.max(0) as usize;
                let r = ri.max(0) as usize;

                match kind {
                    RangeKind::Constant => {
                        let _ = (l, r);
                        oob_range_select(&base, li.max(ri), li.min(ri))
                    }
                    RangeKind::IndexedUp => oob_range_select(&base, li + ri - 1, li),
                    RangeKind::IndexedDown => {
                        // §11.5.1: `[l -: r]` selects r bits down from l. When
                        // `l < r-1` the low bound is negative — those bits read
                        // x, and saturating to 0 would drop them AND shrink the
                        // width. Use the signed-bound select in that case.
                        let lo_signed = li - (ri - 1);
                        oob_range_select(&base, li, lo_signed)
                    }
                }
            }
            // §12.6 `e matches p` used as a plain boolean.
            ExprKind::Matches { expr, pattern } => {
                let mut binds: Vec<(String, Value)> = Vec::new();
                let ok = self.match_pattern(expr, pattern, &mut binds);
                if ok { Value::ones(1) } else { Value::zero(1) }
            }
            ExprKind::Inside { expr, ranges } => {
                // §11.4.13: the operands are compared like `==`, at the width
                // of ALL of them, so `(a8 + 8'd1) inside {256}` adds at 32
                // bits (256), not at the 8 bits of `a8`.
                let jw = self.inside_joint_width(expr, ranges);
                let val = match jw {
                    Some(w) => self.eval_expr_ctx(expr, w),
                    None => self.eval_expr(expr),
                };
                let item = |me: &mut Self, e: &Expression| match jw {
                    Some(w) => me.eval_expr_ctx(e, w),
                    None => me.eval_expr(e),
                };
                for r in ranges {
                    match &r.kind {
                        ExprKind::Range(lo, hi) => {
                            // §11.4.13: `$` bound = type extreme (see
                            // case_inside_match).
                            let lo_ok = matches!(lo.kind, ExprKind::Dollar)
                                || val.greater_equal(&item(self, lo)).is_true();
                            let hi_ok = matches!(hi.kind, ExprKind::Dollar)
                                || val.less_equal(&item(self, hi)).is_true();
                            if lo_ok && hi_ok {
                                return Value::from_u64(1, 1);
                            }
                        }
                        _ => {
                            // A queue/array operand means set membership: the
                            // value matches any element (e.g. `group inside
                            // {supported_isa}`). Scalars compare directly.
                            if let Some(keys) = self.inside_operand_keys(r) {
                                for key in keys {
                                    if let Some(ev) = self.get_signal_value_by_name(&key) {
                                        if val.wildcard_eq(&ev).is_true() {
                                            return Value::from_u64(1, 1);
                                        }
                                    }
                                }
                                continue;
                            }
                            // §8.3.2/§6.20.2 (issue #388): a class-scope ARRAY
                            // constant operand (`v inside {pkg::Cls::T}`,
                            // `{Cls::T}` from a foreign class, bare `{T}` in
                            // a method of the declaring class) is set
                            // membership over its elements, like any other
                            // array operand — the whole-array reference has
                            // no scalar value, so the direct compare below
                            // matched against a zero.
                            if let Some(c) = self.resolve_class_scope_array_const(r) {
                                for i in c.lo..=c.hi {
                                    if let Some(ev) = self.class_scope_array_const_elem(&c, i) {
                                        if val.wildcard_eq(&ev).is_true() {
                                            return Value::from_u64(1, 1);
                                        }
                                    }
                                }
                                continue;
                            }
                            // §11.4.13: a set element is compared with the
                            // tested expression using WILDCARD equality, so x/z
                            // bits in the element are don't-cares — exactly what
                            // `==?` does. Plain `==` missed `v inside {4'b10xx}`.
                            let ev = item(self, r);
                            if val.wildcard_eq(&ev).is_true() {
                                return Value::from_u64(1, 1);
                            }
                        }
                    }
                }
                Value::zero(1)
            }
            ExprKind::Range(lo, hi) => {
                // Standalone range shouldn't really happen except inside Inside, but handle it
                let l = self.eval_expr(lo);
                let h = self.eval_expr(hi);
                l.concat_with(&h) // Dummy representation
            }
            ExprKind::Paren(inner) => self.eval_expr_typed(inner, ctx_width, unsigned_context),
            // §8.10 (Syntax 8-2, footnote 23): the parentheseless `new
            // src` shallow-copy constructor. `<expr>` evaluates to an object
            // handle; a fresh object copies the source's fields (deep
            // cloning of instance-scoped collections per §8.13).
            ExprKind::ShallowCopy { source } => {
                let src_v = self.eval_expr(source);
                let src_h = src_v.to_u64().unwrap_or(0) as usize;
                if src_h != 0 && matches!(self.heap.get(src_h), Some(Some(_))) {
                    self.copy_construct(src_h)
                } else {
                    Value::zero(32)
                }
            }
            ExprKind::AssignExpr { lvalue, rvalue } => {
                let v = self.eval_expr(rvalue);
                self.assign_value(lvalue, &v);
                v
            }
            ExprKind::SystemCall { name, args } => {
                self.eval_expr_system_call(expr, ctx_width, name, args)
            }
            ExprKind::This => {
                if let Some(Some(handle)) = self.this_stack.last() {
                    Value::from_u64(*handle as u64, 32)
                } else {
                    Value::zero(32)
                }
            }
            ExprKind::MemberAccess { expr: recv, member } => {
                if Self::is_parenless_zero_arg_builtin(&member.name) {
                    let recv_name = self.expr_assoc_name(recv).or_else(|| {
                        Self::plain_ident_name(recv).filter(|name| {
                            self.module.dynamic_arrays.contains(name)
                                || self.module.associative_arrays.contains_key(name)
                                || self.module.arrays.contains_key(name)
                        })
                    });
                    if self.expr_is_string_valued(recv) || recv_name.is_some() {
                        return self.eval_call(expr, &[]);
                    }
                }

                // §13.5.5: inside a function/task/class-method/final body a
                // parenless zero-argument method parses as MemberAccess
                // (module/initial/always scope yields a flat hierarchical
                // Ident, handled in the Ident arm above). `eval_expr_member_access`
                // has no enum-method arm, so these fell through to the
                // object-property tail and read 0 — `last` never matched, and
                // a `while` loop exiting on `e == e.last` never terminated.
                // Dispatch it as the zero-argument call §13.5.5 says it is.
                if matches!(
                    member.name.as_str(),
                    "num" | "first" | "last" | "next" | "prev" | "name"
                ) {
                    if let Some(bare) = Self::plain_ident_name(recv) {
                        let is_enum = self
                            .local_typedef_type_of(&bare)
                            .or_else(|| self.var_typedef_types.get(&bare).cloned())
                            .is_some_and(|tn| self.module.enum_members.contains_key(&tn))
                            || self
                                .type_name_of_var(&bare)
                                .is_some_and(|tn| self.module.enum_members.contains_key(&tn));
                        if is_enum {
                            return self.eval_call_inner(expr, &[]);
                        }
                    }
                }
                // §13.5.5, the same shape for a PACKAGE function: inside a
                // subroutine body `vp::pkf` parses as MemberAccess on the
                // package name (the Ident arm handles the flat form). Read as
                // a member it returned 0, so `return vp::pkf;` lost the call.
                if let Some(pkg) = Self::plain_ident_name(recv) {
                    if self.module.packages.contains(&pkg)
                        && self
                            .module
                            .functions
                            .contains_key(&format!("{}::{}", pkg, member.name))
                        && self
                            .module
                            .pkg_subr_owner
                            .get(&member.name)
                            .is_some_and(|owner| owner == &pkg)
                    {
                        return self.eval_call(expr, &[]);
                    }
                }
                self.eval_expr_member_access(recv, member, ctx_width)
            }
            ExprKind::Call { func, args } => self.eval_call(func, args),
            ExprKind::Dollar => {
                if let Some(&b) = self.dollar_bound.last() {
                    let mut v = Value::from_u64(b as u64, 32);
                    v.is_signed = true;
                    v
                } else {
                    Value::from_u64(u64::MAX, 32)
                }
            }
            ExprKind::Null => Value::zero(32),
            ExprKind::Empty => Value::zero(1),
            // §20.6: a type used as an operand carries only its width.
            ExprKind::TypeLiteral(dt) => {
                let w = super::elaborate::resolve_type_width(
                    dt,
                    Some(&self.module.parameters),
                    Some(&self.module.typedefs),
                )
                .max(1);
                Value::zero(w)
            }
            ExprKind::RandomizeWith { call, constraints } => {
                self.eval_randomize_with(call, constraints)
            }
            ExprKind::WithClause { expr, filter } => {
                // Array reduction with a `with` clause, e.g.
                // `fld.sum() with (item.bit_width)`: evaluate `filter` for each
                // element (with `item` bound to it) and reduce. Without this the
                // reduction would sum the raw element/handle values.
                // Three parse shapes for `q.<method>`:
                //   Call{ MemberAccess{q, method}, [] }     (with parens)
                //   Call{ Ident(hier=[q, method]), [] }     (flat)
                //   Ident(hier=[q, method])                 (no parens — `q.sort` form)
                // §7.12: the call may DECLARE an iterator name —
                // `q.sort(x) with (x)` — which the filter references instead
                // of the default `item`. Silently ignoring it left the filter
                // unresolved (0 for every element): sort became a stable
                // no-op and reductions summed zeros.
                let iter_name: Option<String> = if let ExprKind::Call { args, .. } = &expr.kind {
                    args.first().and_then(|a| match &a.kind {
                        ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                            Some(h.path[0].name.name.clone())
                        }
                        _ => None,
                    })
                } else {
                    None
                };
                let parsed: Option<(String, String)> = if let ExprKind::Call { func, .. } =
                    &expr.kind
                {
                    match &func.kind {
                        ExprKind::MemberAccess {
                            expr: arr_e,
                            member,
                        } => {
                            if let ExprKind::Ident(h) = &arr_e.kind {
                                Some((self.resolve_hier_name(h).into_owned(), member.name.clone()))
                            } else {
                                None
                            }
                        }
                        ExprKind::Ident(h) if h.path.len() >= 2 => {
                            let mut head = h.clone();
                            let last = head.path.pop().unwrap().name.name;
                            Some((self.resolve_hier_name(&head).into_owned(), last))
                        }
                        _ => None,
                    }
                } else if let ExprKind::Ident(h) = &expr.kind {
                    if h.path.len() >= 2 {
                        let mut head = h.clone();
                        let last = head.path.pop().unwrap().name.name;
                        Some((self.resolve_hier_name(&head).into_owned(), last))
                    } else {
                        None
                    }
                } else if let ExprKind::MemberAccess {
                    expr: arr_e,
                    member,
                } = &expr.kind
                {
                    if let ExprKind::Ident(h) = &arr_e.kind {
                        Some((self.resolve_hier_name(h).into_owned(), member.name.clone()))
                    } else {
                        None
                    }
                } else {
                    None
                };
                // §7.12: a class-property receiver — `obj.arr`, `obj.in.arr`,
                // `this.arr`, `objs[i].arr`, `Cls::sarr` — lives at its
                // instance-scoped (`<handle>#<member>`) or class-qualified
                // storage. Left unresolved, the `with` clause was dropped and
                // the plain reduction came back.
                let parsed = match parsed {
                    Some((mut arr, method)) => {
                        if let Some(s) = self.instance_assoc_member(&arr) {
                            arr = s;
                        }
                        if !self.is_with_receiver_storage(&arr) && arr.contains('.') {
                            arr = self.resolve_locator_storage(&arr);
                        }
                        Some((arr, method))
                    }
                    None => None,
                };
                let parsed = match parsed {
                    Some((arr, method)) if self.is_with_receiver_storage(&arr) => {
                        Some((arr, method))
                    }
                    other => {
                        let recv = match &expr.kind {
                            ExprKind::Call { func, .. } => match &func.kind {
                                ExprKind::MemberAccess { expr: r, member } => Some((r, member)),
                                _ => None,
                            },
                            ExprKind::MemberAccess { expr: r, member } => Some((r, member)),
                            _ => None,
                        };
                        recv.and_then(|(r, m)| {
                            self.expr_assoc_name(r)
                                .filter(|st| self.is_with_receiver_storage(st))
                                .map(|st| (st, m.name.clone()))
                        })
                        .or(other)
                    }
                };
                {
                    if let Some((arr, method)) = parsed {
                        let is_arr = self.is_with_receiver_storage(&arr);
                        if is_arr {
                            if matches!(
                                method.as_str(),
                                "sum" | "product" | "min" | "max" | "and" | "or" | "xor"
                            ) {
                                return self.reduce_with(
                                    &arr,
                                    &method,
                                    filter,
                                    iter_name.as_deref(),
                                );
                            }
                            // LRM §7.12.2: `q.sort/.rsort/.unique with
                            // (expr)` — in-place mutators with a per-
                            // element filter expression (binds `item` or the
                            // declared iterator). sort/rsort use the filter's
                            // value as the sort key; unique dedups by it.
                            if matches!(method.as_str(), "sort" | "rsort" | "unique")
                                && !self.is_associative_array(&arr)
                            {
                                self.sort_with(&arr, &method, filter, iter_name.as_deref());
                                return Value::zero(32);
                            }
                        }
                    }
                }
                self.eval_expr(expr)
            }
            ExprKind::AssignmentPattern(parts) => {
                let mut r = Value::zero(0);
                for p in parts.iter().rev() {
                    r = self.eval_expr(p.expr()).concat_with(&r);
                }
                r
            }
            _ => Value::zero(32),
        }
    }
}
