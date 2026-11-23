//! Member access, member-element selects and signedness hints of members.

use super::*;

impl Simulator {
    /// §7.2.1: the signedness of a packed-struct member read (`m.s`,
    /// `p.s` and `o.p.s` of a struct class property): its declared type's.
    /// Unknown, an unsigned sibling (`p.s + 1 < l.s + b.u`) did not make the
    /// relation unsigned, and the signed side was sign-extended (§11.8.2).
    pub(super) fn member_signed_hint(&mut self, expr: &Expression) -> Option<bool> {
        if let Some(ClassAggRef::Packed { signed, .. }) = self.class_agg_member(expr) {
            return Some(signed);
        }
        let ExprKind::Ident(h) = &expr.kind else {
            return None;
        };
        if h.path.iter().any(|s| !s.selects.is_empty()) {
            return None;
        }
        let name = self.resolve_hier_name(h).into_owned();
        let (base, ..) = self.packed_leaf_of_hier(&name)?;
        let field = name.strip_prefix(&base)?.strip_prefix('.')?;
        Some(self.struct_member_signed(&base, field))
    }

    /// `base[index]` where the index chain ends in a member of a packed
    /// struct that is a call result: the element its packed dimensions
    /// (the member's declared ones, through its typedef) address, x for an
    /// x/z or out-of-range index (§11.5.1). None for any other shape.
    pub(super) fn call_member_elem_select(
        &mut self,
        base: &Expression,
        index: &Expression,
    ) -> Option<Value> {
        let mut idx: Vec<&Expression> = vec![index];
        let mut cur = base;
        while let ExprKind::Index { expr: b, index: i2 } = &cur.kind {
            idx.push(i2);
            cur = b;
        }
        let ExprKind::MemberAccess { expr: sb, member } = &cur.kind else {
            return None;
        };
        let su = self.chain_base_packed_struct(sb)?;
        let m_dt = su
            .members
            .iter()
            .find(|m| m.declarators.iter().any(|d| d.name.name == member.name))?
            .data_type
            .clone();
        let params = &self.module.parameters;
        let dims = super::elaborate::packed_member_meta(
            &m_dt,
            params,
            &self.module.typedefs,
            &self.module.typedef_types,
        )
        .1
        .or_else(|| self.packed_dim_bounds(&m_dt, params))?;
        if dims.len() < idx.len() {
            return None;
        }
        let base_v = self.eval_expr(cur);
        let mut w = base_v.width as u64;
        let mut lsb = 0u64;
        let mut known = true;
        for (k, ie) in idx.iter().rev().enumerate() {
            let (l, r) = dims[k];
            let count = (l - r).unsigned_abs() + 1;
            if w % count != 0 {
                return None;
            }
            w /= count;
            let iv = self.eval_expr(ie);
            let off = (!iv.has_xz())
                .then(|| iv.to_i64())
                .flatten()
                .map(|i| i - l.min(r))
                .filter(|&o| o >= 0 && (o as u64) < count);
            match off {
                Some(o) if known => {
                    let slot = if l >= r {
                        o as u64
                    } else {
                        count - 1 - o as u64
                    };
                    lsb += slot * w;
                }
                _ => known = false,
            }
        }
        if !known {
            return Some(Value::new(w.max(1) as u32));
        }
        Some(base_v.range_select((lsb + w - 1) as usize, lsb as usize))
    }

    /// Outlined from `eval_expr_ctx` (see `exec_stmt_blocking_assign`): keeps
    /// the dispatcher's stack frame small.
    #[inline(never)]
    pub(in crate::compiler::simulator) fn eval_expr_member_access(
        &mut self,
        expr: &Box<Expression>,
        member: &crate::ast::Identifier,
        ctx_width: u32,
    ) -> Value {
        // #1108: a whole array member of an instance-array element
        // (`lane[0].payload`).
        if let Some(folded) = self.inst_array_member_canon_parts(expr, member) {
            return self.eval_expr_ctx(&folded, ctx_width);
        }
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
        // §26.3: a struct parameter reached through a package must read
        // the package's field, including when an aggregate copy synthesizes
        // the final member access from a `pkg::PARAM` operand.
        if let Some(pkg) = Self::member_path_head(expr)
            && self.module.packages.contains(pkg)
            && !self.signal_name_to_id.contains_key(pkg)
            && !self.signals.contains_key(pkg)
            && !self.local_stack.last().is_some_and(|frame| frame.contains_key(pkg))
            && let Some(path) = Self::flatten_member_path(expr)
            && path.len() >= 2
        {
            let key = format!("{pkg}::{}", path[1]);
            if self.module.packed_struct_fields.contains_key(&key) {
                let mut field = key;
                for part in path[2..].iter().map(String::as_str).chain([member.name.as_str()]) {
                    field.push('.');
                    field.push_str(part);
                }
                if let Some(value) = self.module.parameters.get(&field) {
                    return value.clone();
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
        // §27.6: through an indexed scope, `gen[1].cb.a` reads the block of
        // copy 1 (`u[0].cb.a`, of an instance-array element).
        if matches!(expr.kind, ExprKind::MemberAccess { .. })
            && let Some(flat) = self.clocking_indexed_term_as_ident(expr)
            && let ExprKind::Ident(h) = &flat.kind
        {
            let mut refs: Vec<&str> = h.path.iter().map(|s| s.name.name.as_str()).collect();
            refs.push(member.name.as_str());
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
                        if w == 64 && self.union_real_member(&base_flat, &member.name) {
                            return Self::union_real_read(out);
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
                        if let Some(fs) = self.struct_field_layout(&resolved) {
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
                                self.struct_field_layout(&resolved)
                            } else {
                                None
                            }
                        } else if let Some(type_name) = self.var_typedef_types.get(&*base_name) {
                            // Local variable/parameter with typedef type
                            let dt = self.module.typedef_types.get(type_name.as_str())?;
                            let resolved = Self::resolve_type_ref(dt, &self.module.typedef_types);
                            self.struct_field_layout(&resolved)
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
                                self.struct_field_layout(&resolved)
                            } else {
                                // Try resolving base_name as a typedef
                                let dt = self.module.typedef_types.get(&*base_name)?;
                                let resolved =
                                    Self::resolve_type_ref(dt, &self.module.typedef_types);
                                self.struct_field_layout(&resolved)
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
                        if w == 64 && self.union_real_member(&name, &member.name) {
                            return Self::union_real_read(v);
                        }
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
                ExprKind::This => (Some("this".to_string()), None),
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
}
