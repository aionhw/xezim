//! Simulator virtual interfaces.

use super::*;

impl Simulator {
    /// §8.5/§23.6: is `head` of `head.leaf` a variable (a class handle, a
    /// struct, `this`/`super`) rather than a scope? Then `head.leaf` selects
    /// a member and never names the interface instance `leaf`.
    pub(super) fn names_data_object(&self, head: &str) -> bool {
        matches!(head, "this" | "super") || self.eval_ident_handle(head).is_some()
    }

    /// The binding slot of a BARE virtual-interface name used inside a class
    /// method, when that name is a STATIC vif member of the executing class
    /// chain. A static property has ONE cell, keyed to the declaring class at
    /// handle 0, so neither the per-object binding table nor the object's own
    /// property map can reach it. `vif_slot` is the single place that knows
    /// which of the two cells a property uses; asking it with handle 0 yields
    /// a slot only for a static, so an instance property keeps taking the
    /// per-object path its caller already implements. The receiver class is
    /// the object's class when a `this` is live, else the class of the
    /// currently executing method, which is what a static method has.
    pub(super) fn static_vif_slot_of_bare(&self, name: &str) -> Option<(usize, String)> {
        // §6.21/§8.4: a LOCAL of the executing frame shadows the class static.
        // `virtual counter_if svif;` declared inside the method reads THAT
        // frame's (unbound) handle, not the class cell the static binding
        // lives in; without this filter a bound static leaked through an
        // uninitialised local and `svif == null` read false. Mirrors the local
        // filter `static_vif_key` applies on the write side.
        if self
            .local_stack
            .last()
            .is_some_and(|l| l.contains_key(name))
        {
            return None;
        }
        if !self.vif_prop_name_possible(name) {
            return None;
        }
        let class_name = self.static_vif_bare_class()?;
        let (slot, _) = self.vif_slot(&class_name, 0, name)?;
        (slot.0 == 0).then_some(slot)
    }

    /// The class whose STATIC virtual-interface property a bare name reads: the
    /// object's class when a `this` is live, else the class of the currently
    /// executing method, which is all a static method has.
    pub(super) fn static_vif_bare_class(&self) -> Option<String> {
        self.this_stack
            .last()
            .copied()
            .flatten()
            .filter(|&th| th != 0)
            .and_then(|th| {
                self.heap
                    .get(th)
                    .and_then(|o| o.as_ref())
                    .map(|i| i.class_name.clone())
            })
            .or_else(|| self.class_context_stack.last().cloned().flatten())
    }

    /// §8.26: the interface bound to the STATIC virtual-interface property
    /// `prop` of class scope `obj`, asked with the PARAMETERIZED-CLASS
    /// specialization preserved.
    ///
    /// A parameterized class keeps one static cell PER specialization, and the
    /// write side files it in `static_vif_bindings` under the canonical spec
    /// key (`C#(T)::p`) that `static_prop_key_spec` builds. `vif_slot`
    /// canonicalises through `vif_class_of_scope_name`, which drops the
    /// signature, so the slot it yields asks for `C::p` and misses the
    /// `C#(T)::p` cell the write filled — a bound static then classifies as
    /// null. The member-read path DOES resolve the spec, which is why
    /// `C#(T)::svif.count` reads back correctly while `C#(T)::svif == null`
    /// still reported null (§25.10).
    ///
    /// `None` means "no opinion": a plain class, an unbound property, or an
    /// instance property, all of which the caller resolves its own way.
    pub(super) fn static_vif_binding_of_scope(
        &self,
        obj: &str,
        prop: &str,
        spec_override: Option<&(String, String)>,
    ) -> Option<String> {
        let (base, spec) = self.resolve_class_scope_prefix(obj)?;
        let spec_pair = spec.map(|s| (base.clone(), s));
        let key = self.static_prop_key_spec(&base, prop, spec_pair.as_ref().or(spec_override))?;
        self.static_vif_bindings.get(&key).cloned()
    }

    /// Classify an equality operand as a VIRTUAL-INTERFACE VARIABLE:
    /// `None` — not one; `Some(Some(n))` — bound to instance `n`;
    /// `Some(None)` — a declared vif variable with no binding (null).
    /// Only declared vif storage engages — never `iface.signal` selects.
    pub(super) fn vif_operand(&self, e: &Expression) -> Option<Option<String>> {
        if !self.module.packages.is_empty()
            && let Some(stripped) = self.strip_package_lvalue(e)
        {
            return self.vif_operand(&stripped);
        }
        match &e.kind {
            ExprKind::Index { expr, index } => {
                if let ExprKind::Ident(h) = &expr.kind
                    && h.path.len() == 1
                    && self.vif_collection(&h.path[0].name.name)
                {
                    return Some(
                        self.vif_elem_by_value(None, &h.path[0].name.name, index)
                            .map(|v| v.0),
                    );
                }
                let property = match &expr.kind {
                    ExprKind::MemberAccess {
                        expr: owner,
                        member,
                    } => self
                        .vif_owner_of_expr(owner)
                        .map(|owner| (owner, member.name.as_str())),
                    ExprKind::Ident(h) if h.path.len() > 1 => {
                        let (last, owner) = h.path.split_last()?;
                        self.vif_owner_of_path(owner)
                            .map(|owner| (owner, last.name.name.as_str()))
                    }
                    _ => None,
                };
                if let Some((owner, name)) = property {
                    self.class_vif_decl(owner.0, name)?;
                    return Some(
                        self.vif_elem_by_value(Some(owner), name, index)
                            .map(|v| v.0),
                    );
                }
                self.nested_iface_operand(e)
            }
            ExprKind::Ident(h) if h.path.len() == 1 => {
                let raw = &h.path[0].name.name;
                if let Some(bound) = self.vif_frame_alias(raw) {
                    return Some(bound);
                }
                if !self.vif_name_possible(raw) {
                    return None;
                }
                if let Some(th) = self.this_stack.last().copied().flatten() {
                    if let Some((b, _)) = self.virtual_iface_bindings.get(&(th, raw.clone())) {
                        return Some(Some(b.clone()));
                    }
                    let hit = {
                        let mut k = self.vif_key_scratch.borrow_mut();
                        k.clear();
                        {
                            use std::fmt::Write as _;
                            let _ = write!(k, "__vif_local__{}#{}", th, raw);
                        }
                        self.signals.get(k.as_str()).map(|v| v.to_sv_string())
                    };
                    if let Some(v) = hit {
                        return Some(Some(v));
                    }
                }
                let hit = {
                    let mut k = self.vif_key_scratch.borrow_mut();
                    k.clear();
                    {
                        use std::fmt::Write as _;
                        let _ = write!(k, "__vif_local__{}", raw);
                    }
                    self.signals.get(k.as_str()).map(|v| v.to_sv_string())
                };
                if let Some(v) = hit {
                    if self.vif_flat_key_applies(raw) {
                        return Some(Some(v));
                    }
                }
                if let Some(b) = self.iface_alias_for(raw) {
                    if !self.data_name_shadows_iface(raw) {
                        return Some(Some(b));
                    }
                }
                if self.is_interface_instance(raw) && !self.data_name_shadows_iface(raw) {
                    return Some(Some(raw.clone()));
                }
                // A STATIC vif member read BARE inside a method. Its cell is
                // the class-scope one, which the per-object table above cannot
                // see, so the bound static missed; the "declared vif property
                // of `this`" fallback below then reported it as null. A static
                // method has no object at all and used to fall out of this arm
                // entirely, so the read went to `class_static_get` and a bound
                // static vif again compared equal to null (§25.10).
                if let Some(cls) = self.static_vif_bare_class() {
                    // §6.21/§8.4: a LOCAL of the executing frame shadows the
                    // class static, and this lookup answers straight from the
                    // class cell, so it has to defer to the frame exactly as
                    // `static_vif_slot_of_bare` below does. Without it a bound
                    // static leaked through an uninitialised local here.
                    if !self
                        .local_stack
                        .last()
                        .is_some_and(|l| l.contains_key(raw.as_str()))
                    {
                        // §8.26: inside a static method of a PARAMETERIZED class the
                        // executing specialization IS the context, and the write
                        // filed the binding under the spec key. The slot below is
                        // base-canonical and would miss it, reporting a bound
                        // static as null.
                        if let Some(bound) =
                            self.static_vif_binding_of_scope(&cls, raw, self.current_spec.as_ref())
                        {
                            return Some(Some(bound));
                        }
                    }
                }
                if self.static_vif_slot_of_bare(raw).is_some()
                    && let Some(cls) = self.static_vif_bare_class()
                {
                    return Some(self.static_vif_binding_of_scope(
                        &cls,
                        raw,
                        self.current_spec.as_ref(),
                    ));
                }
                // A DECLARED vif property of `this` with no binding: unbound.
                if let Some(th) = self
                    .this_stack
                    .last()
                    .copied()
                    .flatten()
                    .filter(|_| self.class_member_names().vif_props.contains(raw.as_str()))
                {
                    let mut cur = self
                        .heap
                        .get(th)
                        .and_then(|o| o.as_ref())
                        .map(|i| i.class_name.as_str());
                    while let Some(cn) = cur {
                        let Some(cd) = self.module.classes.get(cn) else {
                            break;
                        };
                        if cd.virtual_iface_properties.contains_key(raw) {
                            return Some(None);
                        }
                        cur = cd.extends.as_deref();
                    }
                }
                None
            }
            ExprKind::Ident(h) if h.path.len() == 2 => {
                // §23.6: a scope-qualified interface instance (`top.pif`) —
                // the LEAF names the instance; `vif_operand_obj_prop` would
                // reject it since no class declares `pif` as a vif property.
                // §8.5: a head naming a variable makes this a member select
                // (`t.irq`), never the same-named instance.
                if self.is_interface_instance(&h.path[1].name.name)
                    && !self.names_data_object(&h.path[0].name.name)
                {
                    return Some(Some(h.path[1].name.name.clone()));
                }
                self.vif_operand_obj_prop(&h.path[0].name.name, &h.path[1].name.name)
                    .or_else(|| self.nested_iface_operand(e))
            }
            ExprKind::Ident(h) if h.path.len() > 2 => {
                // `e.d.vif` — a NESTED receiver spelled as one hierarchical
                // path. The last segment is the vif property; the rest must
                // resolve (as a chain of instance-property handle reads) to
                // the owning object.
                let prop = h.path.last()?.name.name.clone();
                if self.class_member_names().vif_props.contains(prop.as_str())
                    && !self.is_interface_instance(&h.path[h.path.len() - 2].name.name)
                {
                    let base = {
                        let mut bh = h.clone();
                        bh.path.pop();
                        Expression::new(ExprKind::Ident(bh), e.span)
                    };
                    if let Some(oh) = self.chain_handle(&base) {
                        return self.vif_operand_handle_prop(oh, &prop);
                    }
                }
                self.nested_iface_operand(e)
            }
            ExprKind::MemberAccess { expr, member } => {
                // §8.26/§25.10: `C#(T)::svif` — a static virtual interface of a
                // SPECIALIZATION. The receiver is a `Specialization` node, so no
                // Ident/This arm below can name it, and the operand classified as
                // unclassifiable: `==`/`!=` fell through to value semantics, where
                // a bound static is an opaque placeholder that never matches null.
                // Ask the static map through the same key builder the write used;
                // `vif_recv_target` already does this for the member-read path,
                // which is why `C#(T)::svif.count` read back correctly while
                // `C#(T)::svif == null` did not.
                if self.vif_prop_name_possible(&member.name)
                    && let Some(key) = self.static_vif_key(e)
                    && let Some((owner, _)) = key.rsplit_once("::")
                    && let Some(decl) = owner.split('#').next()
                    && self.class_vif_decl(decl, &member.name).is_some()
                {
                    // Qualified, object and specialization spellings share
                    // the same identity; absence from this store means null.
                    return Some(self.static_vif_bindings.get(&key).cloned());
                }
                if let ExprKind::Ident(bh) = &expr.kind {
                    if bh.path.len() == 1 && !self.is_interface_instance(&bh.path[0].name.name) {
                        if self.is_interface_instance(&member.name)
                            && !self.names_data_object(&bh.path[0].name.name)
                        {
                            return Some(Some(member.name.clone()));
                        }
                        return self
                            .vif_operand_obj_prop(&bh.path[0].name.name, &member.name)
                            .or_else(|| self.nested_iface_operand(e));
                    }
                }
                // `this.svif` parses as `MemberAccess { expr: This, .. }`, not
                // as a one-segment ident, so neither the arm above nor
                // `chain_handle` (which only walks Ident chains) can name the
                // receiver — the live `this` IS the receiver, and
                // `vif_operand_handle_prop` resolves the property from the
                // object's class chain (§25.10).
                if matches!(expr.kind, ExprKind::This)
                    && self
                        .class_member_names()
                        .vif_props
                        .contains(member.name.as_str())
                    && let Some(th) = self.this_stack.last().copied().flatten()
                    && th != 0
                {
                    return self.vif_operand_handle_prop(th, &member.name);
                }
                // A NESTED receiver (`e.d.vif` — a vif property of a
                // handle-valued member, not of a bare variable): resolve
                // the receiver chain to its object handle and classify from
                // the receiver chain to its object handle and classify from
                // that object's class chain.
                if self
                    .class_member_names()
                    .vif_props
                    .contains(member.name.as_str())
                {
                    if let Some(oh) = self.chain_handle(expr) {
                        return self.vif_operand_handle_prop(oh, &member.name);
                    }
                }
                self.nested_iface_operand(e)
            }
            _ => None,
        }
    }

    /// `vif_operand` for a hierarchical name of a nested interface instance
    /// (`o.leaf`, `o.arr[1]`, #612): unless its head is a data object other
    /// than an interface instance, which makes it a member select.
    fn nested_iface_operand(&self, e: &Expression) -> Option<Option<String>> {
        let mut cur = e;
        let head = loop {
            match &cur.kind {
                ExprKind::Ident(h) => break h.path.first()?.name.name.as_str(),
                ExprKind::MemberAccess { expr, .. } | ExprKind::Index { expr, .. } => cur = expr,
                _ => return None,
            }
        };
        if !self.is_interface_instance(head) && self.names_data_object(head) {
            return None;
        }
        self.nested_iface_instance(e).map(Some)
    }

    /// §23.8: inside a subroutine of an interface or module instance,
    /// `parent_if.children[0]` names by its DEFINITION name the instance the
    /// subroutine runs in. Spell it the way the instance's own `children[0]`
    /// reaches the resolver (`parent.children`, under the same select); read as
    /// written it named no interface instance and bound nothing.
    pub(super) fn strip_own_definition_name(&self, e: &Expression) -> Option<Expression> {
        fn base(e: &Expression) -> Option<&HierarchicalIdentifier> {
            match &e.kind {
                ExprKind::Ident(h) => Some(h),
                ExprKind::MemberAccess { expr, .. }
                | ExprKind::Index { expr, .. }
                | ExprKind::RangeSelect { expr, .. } => base(expr),
                ExprKind::Call { func, .. } => base(func),
                _ => None,
            }
        }
        let h = base(e)?;
        // Only `defname.member...` names the instance: a bare `defname` is a
        // local, port or variable that happens to share the definition's name
        // (`int sub;` inside `module sub`).
        if h.root.is_some() || h.path.len() < 2 || !h.path[0].selects.is_empty() {
            return None;
        }
        let head = h.path[0].name.name.as_str();
        if self.local_stack.last().is_some_and(|l| l.contains_key(head)) {
            return None;
        }
        let scope_hint = self.name_resolve_hint.borrow();
        let scope = scope_hint.as_deref()?;
        if self.is_interface_instance(head)
            || !self
                .instance_by_path(scope)
                .is_some_and(|i| i.def_name == head)
        {
            return None;
        }
        let mut path = Self::iface_path_segs(scope, e.span);
        path.extend(h.path[1..].iter().cloned());
        let replacement = Self::resolved_iface_expr(path, e.span);
        fn replace_base(e: &mut Expression, replacement: Expression) {
            match &mut e.kind {
                ExprKind::MemberAccess { expr, .. }
                | ExprKind::Index { expr, .. }
                | ExprKind::RangeSelect { expr, .. } => replace_base(expr, replacement),
                ExprKind::Call { func, .. } => replace_base(func, replacement),
                _ => *e = replacement,
            }
        }
        let mut rewritten = e.clone();
        replace_base(&mut rewritten, replacement);
        Some(rewritten)
    }

    /// `vif_operand`, then a receiver with an element select
    /// (`cfg.kids[0].vif`, `arr[i].vif`), which the `&self` chain walk cannot
    /// index: those classified as nothing, so a bound vif compared equal to
    /// null by value (§25.9). Evaluate the receiver to its object handle and
    /// classify the property there. A receiver with a call is left alone so
    /// it is not evaluated twice.
    pub(super) fn vif_operand_eval(&mut self, e: &Expression) -> Option<Option<String>> {
        if let Some(c) = self.vif_operand(e) {
            return Some(c);
        }
        fn has_select(e: &Expression) -> bool {
            match &e.kind {
                ExprKind::Index { .. } => true,
                ExprKind::Ident(h) => h.path.iter().any(|s| !s.selects.is_empty()),
                ExprKind::MemberAccess { expr, .. } => has_select(expr),
                _ => false,
            }
        }
        let prop = match &e.kind {
            ExprKind::MemberAccess { member, .. } => member.name.as_str(),
            ExprKind::Ident(h) if h.path.len() >= 2 && h.path.last()?.selects.is_empty() => {
                h.path.last()?.name.name.as_str()
            }
            _ => return None,
        };
        if !self.class_member_names().vif_props.contains(prop) {
            return None;
        }
        let prop = prop.to_string();
        let recv = match &e.kind {
            ExprKind::MemberAccess { expr, .. } => (**expr).clone(),
            ExprKind::Ident(h) => {
                let mut bh = h.clone();
                bh.path.pop();
                Expression::new(ExprKind::Ident(bh), e.span)
            }
            _ => return None,
        };
        if !has_select(&recv) || Self::expr_contains_call(&recv) {
            return None;
        }
        let oh = self.eval_expr(&recv).to_u64()? as usize;
        if oh == 0 {
            return None;
        }
        self.vif_operand_handle_prop(oh, &prop)
    }

    /// `obj.prop` classification for `vif_operand` — engages only when the
    /// receiver's class DECLARES `prop` as a virtual interface.
    pub(super) fn vif_operand_obj_prop(&self, obj: &str, prop: &str) -> Option<Option<String>> {
        if !self.class_member_names().vif_props.contains(prop) {
            return None;
        }
        // A CLASS NAME on the left — `Holder::sa`, and equally its typedef
        // alias `HolderT::sa`, both reaching the simulator as
        // `Ident([Holder]).sa` — reads a STATIC virtual interface. Its binding
        // lives at the class scope (handle 0), never on an object, and
        // `eval_ident_handle` cannot resolve a class name at all. Without this
        // arm the operand classified as unclassifiable and `==`/`!=` fell
        // through to value semantics, where every bound static vif is the same
        // nonzero placeholder: two statics bound to two DIFFERENT interfaces
        // compared equal, and a static never matched the handle it holds
        // (§25.10). `vif_slot` yields None for a non-static prop at handle 0,
        // so instance properties still fall through to the object path.
        if let Some(cls) = self.vif_class_of_scope_name(obj) {
            self.vif_slot(&cls, 0, prop)?;
            return Some(self.vif_prop_target(obj, 0, prop).map(|(bound, _)| bound));
        }
        let oh = if obj == "this" {
            self.this_stack.last().copied().flatten().unwrap_or(0)
        } else {
            self.eval_ident_handle(obj).unwrap_or(0)
        };
        if oh == 0 {
            return None;
        }
        self.vif_operand_handle_prop(oh, prop)
    }

    /// The `vif_operand` tail for object `oh`: `Some(Some(instance))` when
    /// `oh`'s class chain declares `prop` as a virtual interface and a
    /// binding exists, `Some(None)` when declared but unbound (null).
    pub(super) fn vif_operand_handle_prop(&self, oh: usize, prop: &str) -> Option<Option<String>> {
        let class = self.heap.get(oh)?.as_ref()?.class_name.as_str();
        self.class_vif_decl(class, prop)?;
        Some(
            self.vif_prop_target(class, oh, prop)
                .map(|(bound, _)| bound),
        )
    }

    /// Resolve an expression to a class-object HANDLE without the eval
    /// machinery: a bare identifier, or a member chain whose links are all
    /// instance-property handle reads. Enough for vif classification of a
    /// nested receiver (`e.d.vif`); anything else yields None and the
    /// caller falls back to its non-vif path.
    pub(super) fn chain_handle(&self, expr: &Expression) -> Option<usize> {
        match &expr.kind {
            // §8.11: `this.cfg.vif` roots the chain at the live object; with
            // no handle the operand was unclassifiable and a bound vif
            // compared equal to null.
            ExprKind::This => self
                .this_stack
                .last()
                .copied()
                .flatten()
                .filter(|&h| h != 0),
            ExprKind::Ident(h) if h.path.len() == 1 => self.eval_ident_handle(&h.path[0].name.name),
            ExprKind::Ident(h) if h.path.len() > 1 => {
                // A packed multi-segment path (`e.d`): the head names the
                // object, each following segment an instance-property read.
                let mut oh = self.eval_ident_handle(&h.path[0].name.name)?;
                for seg in &h.path[1..] {
                    oh = self
                        .heap
                        .get(oh)?
                        .as_ref()?
                        .properties
                        .get(&seg.name.name)?
                        .to_u64()? as usize;
                }
                Some(oh)
            }
            ExprKind::MemberAccess { expr, member } => {
                let base = self.chain_handle(expr)?;
                self.heap
                    .get(base)?
                    .as_ref()?
                    .properties
                    .get(&member.name)
                    .and_then(|v| v.to_u64().map(|h| h as usize))
            }
            _ => None,
        }
    }

    /// Like `resolve_vif_rhs_name`, but the result must demonstrably NAME a
    /// live interface instance — bound names always bottom out at one, so
    /// anything else (an ordinary variable, an indexed storage name) is
    /// rejected. Keeps the propagation hooks from polluting bindings.
    /// Can a formal of this declared type be bound to a virtual interface?
    /// `virtual x_if` / bare interface types and typedef references (which
    /// may resolve to one) can; integral, real, string, struct, enum and
    /// class-less simple types cannot.
    #[inline]
    pub(super) fn formal_may_carry_vif(dt: &crate::ast::types::DataType) -> bool {
        matches!(
            dt,
            crate::ast::types::DataType::Interface { .. }
                | crate::ast::types::DataType::TypeReference { .. }
                | crate::ast::types::DataType::Implicit { .. }
        )
    }

    /// §25.9 `__vif_local__` keys on call entry: each formal takes its
    /// actual's interface binding, or none. The keys are flat by formal
    /// name, so the caller's values are returned for `vif_formals_exit` to
    /// restore — a nested call with a same-named formal must not clobber
    /// them.
    pub(super) fn vif_formals_enter(
        &mut self,
        formals: &[(&str, Option<&Expression>)],
    ) -> Vec<(String, Option<Value>)> {
        // Resolve every actual before any key changes: an actual may name
        // a caller formal that shares a key with an earlier callee formal.
        let bound: Vec<Option<String>> = formals
            .iter()
            .map(|(_, a)| a.and_then(|a| self.resolve_vif_rhs_name_strict(a)))
            .collect();
        let mut saved = Vec::with_capacity(formals.len());
        for ((name, _), nm) in formals.iter().zip(bound) {
            // No key was ever made for this name and none is made now: the
            // removal would find nothing (the store generation still moves,
            // as it did). An empty key marks the slot for the exit.
            if nm.is_none() && !self.vif_local_names.contains(*name) {
                bump_store_gen();
                saved.push((String::new(), None));
                continue;
            }
            let key = format!("__vif_local__{}", name);
            let prev = match nm {
                Some(nm) => {
                    self.vif_local_names.insert(name.to_string());
                    self.signals.insert(key.clone(), Value::from_string(&nm))
                }
                None => self.signals.remove(&key),
            };
            saved.push((key, prev));
        }
        saved
    }

    /// On call exit: the bindings the `outputs` formals ended with, keyed by
    /// formal name; then the caller's keys are put back.
    pub(super) fn vif_formals_exit(
        &mut self,
        saved: Vec<(String, Option<Value>)>,
        formals: &[(&str, Option<&Expression>)],
        outputs: &[&str],
    ) -> HashMap<String, String> {
        let mut ended: HashMap<String, String> = HashMap::default();
        for pn in outputs {
            if !self.vif_local_names.contains(*pn) {
                continue;
            }
            if let Some(v) = self.signals.get(&format!("__vif_local__{}", pn)) {
                ended.insert(pn.to_string(), v.to_sv_string());
            }
        }
        for ((key, prev), (name, _)) in saved.into_iter().zip(formals.iter()) {
            // Untouched on entry: the key exists only if the body made one.
            let key = if key.is_empty() {
                if !self.vif_local_names.contains(*name) {
                    bump_store_gen();
                    continue;
                }
                format!("__vif_local__{}", name)
            } else {
                key
            };
            match prev {
                Some(v) => {
                    self.signals.insert(key, v);
                }
                None => {
                    self.signals.remove(&key);
                }
            }
        }
        ended
    }

    /// Bind an out/inout actual to interface instance `inst`.
    pub(super) fn vif_bind_actual(&mut self, caller: &Expression, inst: &str) {
        let synth = Expression::new(
            ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                root: None,
                path: vec![crate::ast::expr::HierPathSegment {
                    name: crate::ast::Identifier {
                        name: inst.to_string(),
                        span: crate::ast::Span::dummy(),
                    },
                    selects: Vec::new(),
                }],
                span: crate::ast::Span::dummy(),
                cached_signal_id: std::cell::Cell::new(None),
                cached_resolved_name: std::cell::OnceCell::new(),
            }),
            crate::ast::Span::dummy(),
        );
        self.bind_static_vif(caller, &synth);
        let _ = self.try_bind_virtual_iface(caller, &synth);
    }

    /// §13.5.2 / §25.9: output writeback also replaces an old binding with
    /// null. A zero-valued non-vif actual retains ordinary value semantics.
    pub(super) fn vif_writeback(
        &mut self,
        caller: &Expression,
        value: &Value,
        bound: Option<&str>,
    ) {
        if bound == Some("")
            || (bound.is_none() && value.to_u64() == Some(0) && self.vif_operand(caller).is_some())
        {
            let null = Expression::new(ExprKind::Null, caller.span);
            self.bind_static_vif(caller, &null);
            self.try_bind_virtual_iface(caller, &null);
        } else if let Some(bound) = bound {
            self.vif_bind_actual(caller, bound);
        }
        self.assign_value(caller, value);
    }

    /// Every static property some class declares as a virtual interface or
    /// through a type parameter, collections excluded (built on first use).
    pub(super) fn vif_capable_static_set(&self) -> &HashSet<String> {
        self.vif_capable_statics.get_or_init(|| {
            let mut out: HashSet<String> = HashSet::default();
            for cd in self.module.classes.values() {
                for sp in cd.static_properties.iter() {
                    if Self::static_is_vif_capable(cd, sp) {
                        out.insert(sp.clone());
                    }
                }
            }
            out
        })
    }

    /// Whether static property `sp` of `cd` can hold a virtual interface:
    /// declared one, or typed by a type parameter (not a collection of it).
    pub(super) fn static_is_vif_capable(cd: &super::elaborate::ElaboratedClass, sp: &str) -> bool {
        if cd.static_collections.iter().any(|(n, ..)| n == sp)
            || cd.static_fixed_arrays.iter().any(|(n, ..)| n == sp)
        {
            return false;
        }
        cd.virtual_iface_properties.contains_key(sp)
            || matches!(cd.property_types.get(sp),
                Some(DataType::TypeReference { name, dimensions, .. })
                    if dimensions.is_empty() && cd.type_param_names.contains(&name.name.name))
    }

    /// Every static fixed array of virtual interfaces some class declares.
    pub(super) fn static_vif_array_name_set(&self) -> HashSet<String> {
        self.module
            .classes
            .values()
            .flat_map(|cd| {
                cd.static_fixed_arrays
                    .iter()
                    .filter(|(n, ..)| cd.virtual_iface_properties.contains_key(n))
                    .map(|(n, ..)| n.clone())
            })
            .collect()
    }

    /// The name an element select `a[i]` / `C::a[i]` / `o.a[i]` selects from.
    pub(super) fn selected_leaf(lv: &Expression) -> Option<&str> {
        match &lv.kind {
            ExprKind::Ident(h) => h
                .path
                .last()
                .filter(|s| s.selects.len() == 1)
                .map(|s| s.name.name.as_str()),
            ExprKind::Index { expr, .. } => match &expr.kind {
                ExprKind::Ident(h) => h.path.last().map(|s| s.name.name.as_str()),
                ExprKind::MemberAccess { member, .. } => Some(member.name.as_str()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Is `lv` one element of a static fixed array of virtual interfaces —
    /// `Cls::pa[i]`, or `pa[i]` in a method of its class?
    pub(super) fn static_vif_array_elem(&self, lv: &Expression) -> bool {
        // (class scope or None for a bare name, member, select count)
        let (scope, member, nsel): (Option<&str>, &str, usize) = match &lv.kind {
            ExprKind::Ident(h) => match h.path.as_slice() {
                [c, seg] if c.selects.is_empty() => (
                    Some(c.name.name.as_str()),
                    seg.name.name.as_str(),
                    seg.selects.len(),
                ),
                [seg] => (None, seg.name.name.as_str(), seg.selects.len()),
                _ => return false,
            },
            ExprKind::Index { expr, .. } => match &expr.kind {
                ExprKind::Ident(h) => match h.path.as_slice() {
                    [c, seg] if c.selects.is_empty() && seg.selects.is_empty() => {
                        (Some(c.name.name.as_str()), seg.name.name.as_str(), 1)
                    }
                    [seg] if seg.selects.is_empty() => (None, seg.name.name.as_str(), 1),
                    _ => return false,
                },
                ExprKind::MemberAccess { expr: o, member } => match &o.kind {
                    ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                        (Some(h.path[0].name.name.as_str()), member.name.as_str(), 1)
                    }
                    _ => return false,
                },
                _ => return false,
            },
            _ => return false,
        };
        if nsel != 1 {
            return false;
        }
        let cls = match scope {
            Some(c) if self.module.classes.contains_key(c) => c.to_string(),
            Some(_) => return false,
            None => {
                if self
                    .local_stack
                    .last()
                    .is_some_and(|l| l.contains_key(member))
                {
                    return false;
                }
                match self.class_context_stack.last() {
                    Some(Some(c)) => c.clone(),
                    _ => return false,
                }
            }
        };
        let Some((decl, _)) = self.class_vif_decl(&cls, member) else {
            return false;
        };
        self.module
            .classes
            .get(decl)
            .is_some_and(|cd| cd.static_fixed_arrays.iter().any(|(n, ..)| n == member))
    }

    /// Could the assignment target `lv` be a static property holding a
    /// virtual interface (see `bind_static_vif`)? A name test only.
    pub(super) fn static_vif_target_possible(&self, lv: &Expression) -> bool {
        let leaf = match &lv.kind {
            // `C::p` and `pkg::C::p` (§8.3.6-1, §26.3) both name a static
            // property; the leaf is the property either way.
            ExprKind::Ident(h) if h.path.len() <= 3 => h.path.last().map(|s| s.name.name.as_str()),
            ExprKind::MemberAccess { member, .. } => Some(member.name.as_str()),
            _ => None,
        };
        leaf.is_some_and(|l| self.vif_capable_static_set().contains(l))
    }

    /// §25.9/§8.9: `S::spv = pi`, `spv = x` in a method, `one = v` in
    /// `G#(virtual pif)`: a STATIC property holding a virtual interface binds
    /// per class / specialization (`static_vif_bindings`). The value
    /// assignment still runs.
    #[inline(always)]
    pub(super) fn bind_static_vif(&mut self, lvalue: &Expression, rvalue: &Expression) {
        // One byte on the per-assignment path: start-up clears it when no
        // class declares a static that can hold a virtual interface.
        if self.may_bind_static_vif {
            self.bind_static_vif_slow(lvalue, rvalue);
        }
    }

    #[cold]
    #[inline(never)]
    pub(super) fn bind_static_vif_slow(&mut self, lvalue: &Expression, rvalue: &Expression) {
        // §25.10/§8.9: an element of a STATIC fixed array of virtual
        // interfaces (`S::pa[1] = bus`, `pa[i] = bus` in a static method)
        // binds in its class cell; the value paths that store the element
        // run first and never reach the binding.
        if !self.static_vif_array_names.is_empty()
            && Self::selected_leaf(lvalue).is_some_and(|n| self.static_vif_array_names.contains(n))
            && self.static_vif_array_elem(lvalue)
        {
            let _ = self.try_bind_virtual_iface(lvalue, rvalue);
            return;
        }
        if self.vif_capable_static_set().is_empty() {
            return;
        }
        if !matches!(
            &rvalue.kind,
            ExprKind::Null
                | ExprKind::Ident(_)
                | ExprKind::MemberAccess { .. }
                | ExprKind::Index { .. }
        ) || !self.static_vif_target_possible(lvalue)
        {
            return;
        }
        let Some(key) = self.static_vif_key(lvalue) else {
            return;
        };
        if matches!(&rvalue.kind, ExprKind::Null) {
            self.static_vif_bindings.remove(&key);
        } else if let Some(nm) = self.resolve_vif_rhs_name_strict(rvalue) {
            // A bound formal may still have an opaque zero value. Its
            // interface identity takes priority over that representation.
            if let Some(leaf) = key.rsplit("::").next() {
                self.static_vif_names.insert(leaf.to_string());
            }
            self.static_vif_bindings.insert(key, nm);
        } else {
            let null_variable = match &rvalue.kind {
                ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                    let raw = h.path[0].name.name.as_str();
                    !self.is_interface_instance(raw)
                        && self
                            .module
                            .var_decl_types
                            .get(self.vif_variable_name(raw).as_ref())
                            .is_some_and(|dt| self.is_virtual_iface_type(dt))
                        && self.eval_scalar_self(rvalue) == Some(0)
                }
                _ => false,
            };
            if null_variable || self.vif_operand(rvalue) == Some(None) {
                self.static_vif_bindings.remove(&key);
            }
        }
    }

    /// §8.9: the static storage key of the static class property `e` names
    /// — bare inside a method of its class (no local, no instance property
    /// shadowing it), `C::p`, `C#(args)::p`, `alias_t::p`, or `obj.p` — or
    /// None. Gated on the property name: only a name some class declares
    /// static gets past the first check.
    pub(super) fn static_vif_key(&self, e: &Expression) -> Option<String> {
        let names = self.class_member_names();
        let static_name = |n: &str| names.static_or_param.contains(n);
        let class_head = |c: &str| -> Option<(String, Option<(String, String)>)> {
            if self.local_stack.last().is_some_and(|l| l.contains_key(c))
                || self.signal_name_to_id.contains_key(c)
            {
                return None;
            }
            if self.module.classes.contains_key(c) {
                return Some((c.to_string(), None));
            }
            // A type parameter or a typedef (§8.25.1, §6.18).
            if self.class_member_names().scope_prefixes.contains(c) {
                let (b, sig) = self.resolve_class_scope_prefix(c)?;
                return Some((b.clone(), sig.map(|s| (b, s))));
            }
            None
        };
        let keyed = |cls: &str, spec: Option<(String, String)>, p: &str| -> Option<String> {
            // §8.13: a nearer instance declaration hides an ancestor's static.
            let mut owner = Some(cls);
            while let Some(name) = owner {
                let cd = self.module.classes.get(name)?;
                if cd.properties.contains_key(p) || cd.virtual_iface_properties.contains_key(p) {
                    if !cd.static_properties.contains(p) {
                        return None;
                    }
                    break;
                }
                owner = cd.extends.as_deref();
            }
            let k = self.static_prop_key_spec(cls, p, spec.as_ref())?;
            // A class constant is no variable.
            let decl = k.rsplit_once("::")?.0.split('#').next()?;
            self.module
                .classes
                .get(decl)
                .is_some_and(|cd| cd.static_properties.contains(p))
                .then_some(k)
        };
        match &e.kind {
            ExprKind::Ident(h) if h.path.iter().all(|s| s.selects.is_empty()) => {
                match h.path.as_slice() {
                    [seg] => {
                        let n = seg.name.name.as_str();
                        if !static_name(n)
                            || self.local_stack.last().is_some_and(|l| l.contains_key(n))
                        {
                            return None;
                        }
                        let ctx = match self
                            .this_stack
                            .last()
                            .copied()
                            .flatten()
                            .filter(|&h| h != 0)
                        {
                            Some(th) => {
                                let inst = self.heap.get(th)?.as_ref()?;
                                if inst.properties.contains_key(n) {
                                    return None;
                                }
                                inst.class_name.clone()
                            }
                            None => self.class_context_stack.last().cloned().flatten()?,
                        };
                        keyed(&ctx, None, n)
                    }
                    [head, prop] => {
                        let p = prop.name.name.as_str();
                        if !static_name(p) {
                            return None;
                        }
                        if let Some((cls, spec)) = class_head(&head.name.name) {
                            return keyed(&cls, spec, p);
                        }
                        let oh = self.eval_ident_handle(&head.name.name)?;
                        let inst = self.heap.get(oh)?.as_ref()?;
                        keyed(&inst.class_name, inst.spec.clone(), p)
                    }
                    [_pkg, cls_seg, prop] => {
                        // §26.3: `pkg::C::p` — the parser keeps the package as
                        // a leading path segment, so the class is the MIDDLE
                        // segment and the property the last.
                        let p = prop.name.name.as_str();
                        if !static_name(p) {
                            return None;
                        }
                        let (cls, spec) = class_head(&cls_seg.name.name)?;
                        keyed(&cls, spec, p)
                    }
                    _ => None,
                }
            }
            ExprKind::MemberAccess { expr, member } => {
                let p = member.name.as_str();
                if !static_name(p) {
                    return None;
                }
                match &expr.kind {
                    ExprKind::Specialization { .. } => {
                        let spec = self.resolve_call_spec_params(
                            Self::extract_call_spec(expr),
                            &self.current_spec,
                        )?;
                        keyed(&spec.0.clone(), Some(spec), p)
                    }
                    ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                        if let Some((cls, spec)) = class_head(&h.path[0].name.name) {
                            return keyed(&cls, spec, p);
                        }
                        let oh = self.eval_ident_handle(&h.path[0].name.name)?;
                        let inst = self.heap.get(oh)?.as_ref()?;
                        keyed(&inst.class_name, inst.spec.clone(), p)
                    }
                    ExprKind::This => {
                        let th = self.this_stack.last().copied().flatten()?;
                        let inst = self.heap.get(th)?.as_ref()?;
                        keyed(&inst.class_name, inst.spec.clone(), p)
                    }
                    // §26.3: `pkg::C::p` — the class head is itself a member
                    // access whose OUTERMOST member names the class, the same
                    // head `extract_call_spec` reads for `pkg::C#(args)::p`.
                    // The READ side parses this way; the write side keeps the
                    // package as a leading Ident path segment (arm above).
                    ExprKind::MemberAccess { member: head, .. } => match class_head(&head.name) {
                        Some((cls, spec)) => keyed(&cls, spec, p),
                        None => None,
                    },
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The interface instance a static virtual-interface property `e` names
    /// is bound to (see `static_vif_bindings`).
    pub(super) fn static_vif_binding(&self, e: &Expression) -> Option<String> {
        if self.static_vif_bindings.is_empty() {
            return None;
        }
        let k = self.static_vif_key(e)?;
        self.static_vif_bindings.get(&k).cloned()
    }

    pub(super) fn resolve_vif_rhs_name_strict(&self, rvalue: &Expression) -> Option<String> {
        if !self.vif_rhs_possible(rvalue) {
            return None;
        }
        // `iface.member` where the dotted name is a real SIGNAL is a value
        // read, not a modport view — `dd1 = u.d` must not classify dd1 as a
        // vif (a modport view is never a signal).
        let dotted: Option<String> = match &rvalue.kind {
            ExprKind::Ident(h) if h.path.len() == 2 => {
                Some(format!("{}.{}", h.path[0].name.name, h.path[1].name.name))
            }
            ExprKind::MemberAccess { expr, member } => {
                if let ExprKind::Ident(bh) = &expr.kind {
                    if bh.path.len() == 1 {
                        Some(format!("{}.{}", bh.path[0].name.name, member.name))
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(d) = dotted {
            if self.signal_name_to_id.contains_key(d.as_str()) || self.signals.contains_key(&d) {
                return None;
            }
        }
        self.resolve_vif_rhs_name(rvalue)
            .filter(|n| self.is_interface_instance(n))
    }

    /// Object owning a virtual-interface property written as `obj.vif`: a
    /// local, else (inside a method) a property of `this` — a config_db
    /// output actual `cfg.vif` names one — else a module variable or a static.
    pub(super) fn vif_owner_handle(&self, obj: &str) -> usize {
        let v = self
            .local_stack
            .last()
            .and_then(|l| l.get(obj).cloned())
            .or_else(|| {
                let h = self.this_stack.last().copied().flatten()?;
                self.heap.get(h)?.as_ref()?.properties.get(obj).cloned()
            })
            .or_else(|| self.get_signal_value_by_name(obj));
        match v.and_then(|x| x.to_u64()) {
            Some(h) => h as usize,
            None => self.eval_ident_handle(obj).unwrap_or(0),
        }
    }

    pub(super) fn try_bind_virtual_iface_inner(
        &mut self,
        lvalue: &Expression,
        rvalue: &Expression,
    ) -> bool {
        // LRM §25.9: a PLAIN variable declared `virtual <iface>` (a
        // block-local `virtual bus_if vif;` or a module-scope one — not
        // a class property, which the (handle, prop) binding below
        // owns) assigned an interface instance is a name alias, not a
        // value copy: record `vif -> bus` in `viface_var_aliases` so
        // subsequent `vif.member` reads/writes and `vif.f()` calls
        // dispatch to the bound instance. `vif = null` clears it.
        if let ExprKind::Ident(h) = &lvalue.kind {
            if h.path.len() == 1 && h.path[0].selects.is_empty() {
                let storage = self.vif_variable_name(&h.path[0].name.name);
                let name = storage.as_ref();
                let is_viface_var = self
                    .module
                    .var_decl_types
                    .get(name)
                    .is_some_and(|dt| self.is_virtual_iface_type(dt))
                    || self.vif_frame_alias(name).is_some();
                // A same-named class vif PROPERTY written bare inside a
                // method takes the binding path below, not the var alias.
                let shadows_class_prop = is_viface_var
                    && !self
                        .local_stack
                        .last()
                        .is_some_and(|f| f.contains_key(name))
                    && self
                        .this_stack
                        .last()
                        .copied()
                        .flatten()
                        .and_then(|hd| self.heap.get(hd).and_then(|o| o.as_ref()))
                        .is_some_and(|i| self.class_vif_decl(&i.class_name, name).is_some());
                if is_viface_var && !shadows_class_prop {
                    // §25.10: whole-array binding (`vifs = interfaces`). Each
                    // virtual-interface element is an independent alias to the
                    // corresponding structural interface instance.
                    if let Some(&(lo, hi, _)) = self.module.arrays.get(name) {
                        if let ExprKind::Ident(rhs_h) = &rvalue.kind {
                            if rhs_h.path.len() == 1 && rhs_h.path[0].selects.is_empty() {
                                let rhs_base = &rhs_h.path[0].name.name;
                                let targets: Vec<String> = (lo..=hi)
                                    .map(|idx| format!("{}[{}]", rhs_base, idx))
                                    .collect();
                                if targets
                                    .iter()
                                    .all(|target| self.is_interface_instance(target))
                                {
                                    for (idx, target) in (lo..=hi).zip(targets) {
                                        self.vif_tokens_stored = true;
                                        self.set_signal_value_by_name(
                                            &format!("{}[{}]", name, idx),
                                            Value::from_u64(Self::iface_handle_token(&target), 32),
                                        );
                                    }
                                    return true;
                                }
                            }
                        }
                    }
                    // §7.6/§25.10: aggregate assignment copies the element
                    // stores; it must not take the scalar-null fallback.
                    if self.module.arrays.contains_key(name)
                        || self.module.dynamic_arrays.contains(name)
                        || self.module.associative_arrays.contains_key(name)
                    {
                        return false;
                    }
                    // A subroutine LOCAL `virtual ifc lv;` (the top frame owns
                    // the name) binds in the frame's alias map, which travels
                    // with the process context across a park. The global map
                    // is keyed by the bare name only: two class tasks each
                    // holding a local `lv` bound to different interfaces and
                    // interleaved on delays saw each other's binding (a1's
                    // `lv.id` read a2's interface after a2's task had run).
                    let frame_local = self
                        .local_stack
                        .last()
                        .is_some_and(|f| f.contains_key(name))
                        && !self.local_iface_aliases.is_empty();
                    if matches!(&rvalue.kind, ExprKind::Null) {
                        // Clear the alias, then FALL THROUGH so the normal
                        // value path stores the null handle — keeping a
                        // later `vif == null` true.
                        if frame_local {
                            if let Some(f) = self.local_iface_aliases.last_mut() {
                                f.insert(name.to_string(), String::new());
                            }
                        } else {
                            self.viface_var_aliases.remove(name);
                        }
                    } else if matches!(&rvalue.kind, ExprKind::Call { .. }) {
                        // `vd = mgr.get_vif();` — run the call; the callee's
                        // return recorded the bound instance (see the Return
                        // arm). Without this, accessor-style getters returned
                        // only the sentinel and the alias was lost.
                        self.last_vif_return = None;
                        let v = self.eval_expr(rvalue);
                        let returned = self.last_vif_return.take().or_else(|| {
                            v.to_u64()
                                .and_then(|token| self.iface_by_token(token))
                                .map(str::to_string)
                        });
                        if let Some(bound) = returned {
                            if frame_local {
                                if let Some(f) = self.local_iface_aliases.last_mut() {
                                    f.insert(name.to_string(), bound);
                                }
                            } else {
                                self.viface_var_aliases.insert(name.to_string(), bound);
                            }
                        } else if frame_local {
                            if let Some(f) = self.local_iface_aliases.last_mut() {
                                f.insert(name.to_string(), String::new());
                            }
                        } else {
                            self.viface_var_aliases.remove(name);
                        }
                        self.assign_value(lvalue, &v);
                        return true;
                    } else if let Some(bound) = self.resolve_vif_rhs_name(rvalue) {
                        // Only a resolved interface INSTANCE (directly, or
                        // through another vif's current binding) may bind.
                        if self.is_interface_instance(&bound) {
                            let name = name.to_string();
                            // Store the instance's handle token as the
                            // variable's VALUE too, so `vif != null`
                            // existence checks pass.
                            let token = Self::iface_handle_token(&bound);
                            if frame_local {
                                if let Some(f) = self.local_iface_aliases.last_mut() {
                                    f.insert(name.clone(), bound);
                                }
                            } else {
                                self.viface_var_aliases.insert(name, bound);
                            }
                            self.assign_value(lvalue, &Value::from_u64(token, 32));
                            return true;
                        }
                    }
                    // §25.9: copying an unbound handle replaces the previous
                    // binding with null, rather than leaving a stale alias.
                    if !matches!(rvalue.kind, ExprKind::Null) {
                        let value = if self.vif_operand(rvalue) == Some(None) {
                            Value::zero(32)
                        } else {
                            self.eval_expr(rvalue)
                        };
                        if frame_local {
                            if let Some(f) = self.local_iface_aliases.last_mut() {
                                f.insert(name.to_string(), String::new());
                            }
                        } else {
                            self.viface_var_aliases.remove(name);
                        }
                        self.assign_value(lvalue, &value);
                        return true;
                    }
                }
            }
        }
        // §25.10: ELEMENT-wise binding of a module/block-scope vif ARRAY —
        // `varr[0] = b0;`. Only the whole-array form was recorded, so the
        // per-element alias `indexed_iface_alias_for` looks up never existed:
        // the bind silently became a value copy and every later
        // `varr[0].data` access read x.
        if let ExprKind::Index { expr: base, .. } = &lvalue.kind {
            if let ExprKind::Ident(h) = &base.kind {
                if h.path.len() == 1 && h.path[0].selects.is_empty() {
                    let name = h.path[0].name.name.as_str();
                    if self
                        .module
                        .var_decl_types
                        .get(name)
                        .is_some_and(|dt| self.is_virtual_iface_type(dt))
                    {
                        // §25.10: the assignment evaluator owns the selector,
                        // so an index such as i++ runs exactly once.
                        if let Some(bound) = self.resolve_vif_rhs_name_strict(rvalue) {
                            self.vif_tokens_stored = true;
                            self.assign_value(
                                lvalue,
                                &Value::from_u64(Self::iface_handle_token(&bound), 32),
                            );
                            return true;
                        }
                        return false;
                    }
                }
            }
        }
        // Only a declared vif property takes a (handle, prop) binding; the
        // owner lookups of a name or member target are reads.
        let target_prop: Option<&str> = match &lvalue.kind {
            ExprKind::Ident(h) => h.path.last().map(|s| s.name.name.as_str()),
            ExprKind::MemberAccess { member, .. } => Some(member.name.as_str()),
            _ => None,
        };
        if target_prop.is_some_and(|p| !self.class_member_names().vif_props.contains(p)) {
            return false;
        }
        // (handle, prop) for the target. `prop` for arrays encodes the
        // index as `"vif_arr[<idx>]"` so existing single-key storage
        // can stash per-index bindings (LRM §25.10).
        // A STATIC vif property (§8.9) is reached through its class scope —
        // `Cls::svif = bus`, or `svif = bus` in a static method — with no
        // object; `static_scope` names that class.
        let mut static_scope: Option<String> = None;
        let (handle, prop) = match &lvalue.kind {
            // Bare `v = ...` inside a method — `v` is `this.<v>` (a class vif
            // property). The prop_info check below filters non-vif locals.
            ExprKind::Ident(h) if h.path.len() == 1 => {
                // §6.21/§8.4: a LOCAL of the executing frame shadows the class
                // static, and it must shadow the WRITE as well as the read.
                // A static method has no `this`, so `handle` is 0 and the name
                // below resolves to the CLASS cell; without this filter the
                // initializer of a frame-local declaration — `virtual
                // counter_if svif = null;`, whose assignment the declaration
                // statement synthesises — cleared the static binding for every
                // later reader, not just for its own frame. Mirrors the local
                // filters `static_vif_key` and `static_vif_slot_of_bare` apply.
                if self
                    .local_stack
                    .last()
                    .is_some_and(|l| l.contains_key(h.path[0].name.name.as_str()))
                {
                    return false;
                }
                let handle = self.this_stack.last().copied().flatten().unwrap_or(0);
                if handle == 0 {
                    static_scope = self.class_context_stack.last().cloned().flatten();
                }
                let Some(k) = self.vif_elem_key(&h.path[0], handle) else {
                    return false;
                };
                (handle, k)
            }
            // `c.vif = bus;` — `c` may be a local, a property of `this` (a
            // method writing `cfg.vif`, e.g. a config_db output actual) or a
            // module variable. `C::va[i] = bus` / `c.va[i] = bus` bind one
            // element of a vif array (§25.10).
            ExprKind::Ident(h) if h.path.len() == 2 && h.path[0].selects.is_empty() => {
                let obj = h.path[0].name.name.as_str();
                let oh = self.vif_owner_handle(obj);
                let Some(prop) = self.vif_elem_key(&h.path[1], oh) else {
                    return false;
                };
                if oh == 0 && self.vif_class_of_scope_name(obj).is_some() {
                    static_scope = Some(obj.to_string());
                }
                (oh, prop)
            }
            // `p.cfg.vif = bus;` — the vif property's OWNER is itself reached
            // through a handle chain. Walk the chain to the owning object;
            // without this arm the binding was silently dropped (handle 0).
            ExprKind::Ident(h) if h.path.len() >= 3 => {
                let n = h.path.len();
                let mut owner = h.clone();
                owner.path.truncate(n - 1);
                let owner_expr = Expression::new(ExprKind::Ident(owner), lvalue.span);
                let handle = self.eval_handle_expr(&owner_expr).unwrap_or(0);
                (handle, h.path[n - 1].name.name.clone())
            }
            ExprKind::MemberAccess { expr, member } => {
                let obj_handle = match &expr.kind {
                    ExprKind::Ident(h) if h.path.len() == 1 => {
                        let obj = h.path[0].name.name.as_str();
                        let oh = self.vif_owner_handle(obj);
                        if oh == 0 && self.vif_class_of_scope_name(obj).is_some() {
                            static_scope = Some(obj.to_string());
                        }
                        oh
                    }
                    // `this.vif = bus;` — the explicit-`this` spelling of the
                    // bare `vif = bus;` handled above. Resolving only an Ident
                    // object left the handle 0, so the binding was silently
                    // dropped: every later `vif.<member>` access (bare ones
                    // too) then missed the redirect and read x. This is the
                    // usual shape in a constructor that takes the interface as
                    // an argument.
                    ExprKind::This => self.this_stack.last().copied().flatten().unwrap_or(0),
                    // Nested receiver (`p.cfg.vif = bus` in MemberAccess
                    // form) — resolve the owner chain to its handle.
                    _ => self.eval_handle_expr(expr).unwrap_or(0),
                };
                (obj_handle, member.name.clone())
            }
            // LRM §25.10 virtual-interface array: `d.vif_arr[i] = bus`.
            // Parses as Index { expr: <obj.prop>, index: <idx_expr> }.
            // Encode the index into the binding's prop key.
            ExprKind::Index { expr: base, index } => {
                let index_value = self.eval_expr(index);
                let (obj_handle, prop_base) = match &base.kind {
                    // d.vif_arr  (MemberAccess on a class handle)
                    ExprKind::MemberAccess {
                        expr: outer,
                        member,
                    } => {
                        let h = match &outer.kind {
                            ExprKind::Ident(h) if h.path.len() == 1 => {
                                let obj = h.path[0].name.name.as_str();
                                let oh = self.vif_owner_handle(obj);
                                // §8.9: `Cls::va[i]` — a static vif array.
                                if oh == 0 && self.vif_class_of_scope_name(obj).is_some() {
                                    static_scope = Some(obj.to_string());
                                }
                                oh
                            }
                            // A nested owner (`a.cfg.vifs[i] = bus`).
                            _ => self.eval_handle_expr(outer).unwrap_or(0),
                        };
                        (h, member.name.clone())
                    }
                    // Bare `va[i] = bus;` inside a method — `va` is a class
                    // vif-array property of the implicit `this` (or, in a
                    // static method, a static one of the class). The bare
                    // spelling had no arm at all, so a driver binding its own
                    // array never recorded anything.
                    ExprKind::Ident(h) if h.path.len() == 1 => {
                        let hh = self.this_stack.last().copied().flatten().unwrap_or(0);
                        if hh == 0 {
                            static_scope = self.class_context_stack.last().cloned().flatten();
                        }
                        (hh, h.path[0].name.name.clone())
                    }
                    // d.vif_arr in hier-Ident form: Ident([d, vif_arr])
                    ExprKind::Ident(h) if h.path.len() == 2 => {
                        let obj = h.path[0].name.name.as_str();
                        let prop = h.path[1].name.name.clone();
                        let oh = self.vif_owner_handle(obj);
                        if oh == 0 && self.vif_class_of_scope_name(obj).is_some() {
                            static_scope = Some(obj.to_string());
                        }
                        (oh, prop)
                    }
                    // A nested owner in hier-Ident form: `d.cfg.vifs[i]`.
                    ExprKind::Ident(h) if h.path.len() >= 3 => {
                        let n = h.path.len();
                        let oh = self
                            .vif_owner_of_path(&h.path[..n - 1])
                            .map_or(0, |(_, oh)| oh);
                        (oh, h.path[n - 1].name.name.clone())
                    }
                    _ => return false,
                };
                // The index is evaluated (as it always was); only a declared
                // vif property can take the element binding.
                if !self
                    .class_member_names()
                    .vif_props
                    .contains(prop_base.as_str())
                {
                    return false;
                }
                let storage = if obj_handle == 0 {
                    prop_base.clone()
                } else {
                    format!("{}#{}", obj_handle, prop_base)
                };
                let idx = if self.is_string_keyed_array(&storage) {
                    index_value.to_sv_string()
                } else {
                    index_value.to_i64().unwrap_or(0).to_string()
                };
                (obj_handle, format!("{}[{}]", prop_base, idx))
            }
            _ => return false,
        };
        // The owner's class: the object's, else the class scope of a static.
        let class_name: String = match self.heap.get(handle).and_then(|o| o.as_ref()) {
            Some(inst) if handle != 0 => inst.class_name.clone(),
            _ => match static_scope {
                Some(c) => c,
                None => return false,
            },
        };
        // §7.5/§7.6: a whole collection takes allocation/copy semantics,
        // including class properties; only individual elements bind here.
        if !prop.contains('[')
            && (self.class_assoc_member(&class_name, &prop)
                || self.static_vif_collection(&format!("{}::{}", class_name, prop))
                || self.static_fixed_key_in(&class_name, &prop).is_some())
        {
            return false;
        }
        // The binding slot of a vif property the owner's class declares or
        // INHERITS (§8.13) — a static one keys by its class cell (§8.9).
        let Some((slot, modport)) = self.vif_slot(&class_name, handle, &prop) else {
            return false;
        };
        // §25.10: selected class properties store movable identity tokens
        // just like plain collections. Reuse the already evaluated index.
        let element_store = prop.split_once('[').map(|(base, suffix)| {
            let store = if slot.0 == 0 {
                self.static_fixed_key_in(&class_name, base)
                    .unwrap_or_else(|| self.spec_static_coll_key_in_class(base, &class_name))
            } else {
                format!("{}#{}", handle, base)
            };
            (store, base, suffix.strip_suffix(']').unwrap_or(suffix))
        });
        if let Some((collection, base, index)) = element_store {
            let value = if let Some(bound) = self.resolve_vif_rhs_name_strict(rvalue) {
                Value::from_u64(Self::iface_handle_token(&bound), 32)
            } else if self.vif_operand(rvalue) == Some(None) {
                Value::zero(32)
            } else {
                self.eval_expr(rvalue)
            };
            // §7.4.6/§7.10.1: evaluate the RHS even when a write is ignored;
            // only a queue's next index may append a new element.
            if !self.is_associative_array(&collection) {
                let Ok(index) = index.parse::<i64>() else {
                    return true;
                };
                if !self.vif_index_in_bounds(&collection, index) {
                    let Some((decl, _)) = self.class_vif_decl(&class_name, base) else {
                        return false;
                    };
                    let queue = self
                        .module
                        .classes
                        .get(decl)
                        .is_some_and(|cd| cd.queue_property_names.contains(base));
                    let size = self.get_queue_size(&collection);
                    let capacity = self
                        .module
                        .queue_max_sizes
                        .get(&collection)
                        .is_none_or(|&max| size < u64::from(max));
                    if !queue || index < 0 || index as u64 != size || !capacity {
                        return true;
                    }
                    self.set_queue_size(&collection, size + 1);
                }
            }
            self.virtual_iface_bindings.remove(&slot);
            self.vif_tokens_stored = true;
            self.set_signal_value_by_name(&format!("{}[{}]", collection, index), value);
            return true;
        }
        // RHS resolution (LRM §25.9):
        //   - `null` clears the binding so `vif == null` reads true again
        //     (a stale binding otherwise kept it non-null after re-assignment).
        //   - a direct interface-instance ident binds to that instance.
        //   - another virtual interface (`v2 = v` / `v2 = obj.v`) copies that
        //     vif's CURRENT bound interface, not the variable name (without
        //     this the copy bound to a name that resolves to nothing -> X).
        if matches!(&rvalue.kind, ExprKind::Null) {
            self.virtual_iface_bindings.remove(&slot);
            return true;
        }
        let rhs_name = self
            .resolve_vif_rhs_name(rvalue)
            .filter(|name| self.is_interface_instance(name));
        let rhs_name = match rhs_name {
            Some(name) => name,
            None => {
                let value = self.eval_expr(rvalue);
                if let Some(name) = value.to_u64().and_then(|v| self.iface_by_token(v)) {
                    name.to_string()
                } else {
                    self.virtual_iface_bindings.remove(&slot);
                    return true;
                }
            }
        };
        // Record the binding. Subsequent `obj.<prop>.<member>` accesses
        // will follow it; the modport (if any) is consulted at write
        // time to emit a direction warning.
        self.virtual_iface_bindings
            .insert(slot, (rhs_name, modport));
        true
    }

    /// The binding key of vif property segment `seg`: its name, or
    /// `name[i]` for one element of a vif array (§25.10). `None` for a
    /// multi-dimensional select.
    pub(super) fn vif_elem_key(
        &mut self,
        seg: &crate::ast::expr::HierPathSegment,
        owner: usize,
    ) -> Option<String> {
        match seg.selects.as_slice() {
            [] => Some(seg.name.name.clone()),
            [ix] => {
                let value = self.eval_expr(ix);
                let storage = if owner == 0 {
                    seg.name.name.clone()
                } else {
                    format!("{}#{}", owner, seg.name.name)
                };
                let i = if self.is_string_keyed_array(&storage) {
                    value.to_sv_string()
                } else {
                    value.to_i64()?.to_string()
                };
                Some(format!("{}[{}]", seg.name.name, i))
            }
            _ => None,
        }
    }

    /// Resolve a virtual-interface RHS expression to the bound interface
    /// instance name (LRM §25.9). A bare/member ident that is itself a bound
    /// virtual interface yields its current target (vif-to-vif copy); a plain
    /// interface-instance ident yields its own name. Returns None when the RHS
    /// is neither (so the caller falls back to a normal value assignment).

    pub(super) fn resolve_vif_rhs_name(&self, rvalue: &Expression) -> Option<String> {
        if !self.module.packages.is_empty()
            && let Some(stripped) = self.strip_package_lvalue(rvalue)
        {
            return self.resolve_vif_rhs_name(&stripped);
        }
        if let Some(local) = self.strip_own_definition_name(rvalue) {
            return self.resolve_vif_rhs_name(&local);
        }
        if let Some(b) = self.static_vif_binding(rvalue) {
            return Some(b);
        }
        // §25.9/§25.10: an ELEMENT of a vif array or collection held by an
        // object, a class scope or a frame (`o.vq[0]`, `S::pa[1]`, `vq[i]`)
        // names the instance that element is bound to. The arms below read
        // such a name with its select dropped, so the element bound nothing
        // and the target kept its previous interface.
        if Self::member_chain_has_index(rvalue)
            && let Some((b, _)) = self.vif_recv_target(rvalue)
        {
            return Some(b);
        }
        match &rvalue.kind {
            // `v` — a bound vif of the current `this`, else a vif
            // formal/local alias (LRM §25.9), else a direct instance.
            ExprKind::Ident(h) if h.path.len() == 1 => {
                let raw = h.path[0].name.name.clone();
                if let Some(bound) = self.vif_frame_alias(&raw) {
                    return bound;
                }
                if let Some(th) = self.this_stack.last().copied().flatten() {
                    if let Some((b, _)) = self.virtual_iface_bindings.get(&(th, raw.clone())) {
                        return Some(b.clone());
                    }
                }
                // Check for a recorded local/module-level vif binding: when
                // config_db GET lands into a variable with no class context, we
                // record the interface name in signals["__vif_local__<var>"] so a
                // later `obj.vif_prop = local_var` can propagate the binding.
                if let Some(th) = self.this_stack.last().copied().flatten() {
                    let hit = {
                        let mut k = self.vif_key_scratch.borrow_mut();
                        k.clear();
                        {
                            use std::fmt::Write as _;
                            let _ = write!(k, "__vif_local__{}#{}", th, raw);
                        }
                        self.signals.get(k.as_str()).map(|v| v.to_sv_string())
                    };
                    if let Some(s) = hit {
                        if !s.is_empty() {
                            return Some(s);
                        }
                    }
                }
                let hit = {
                    let mut k = self.vif_key_scratch.borrow_mut();
                    k.clear();
                    {
                        use std::fmt::Write as _;
                        let _ = write!(k, "__vif_local__{}", raw);
                    }
                    self.signals.get(k.as_str()).map(|v| v.to_sv_string())
                };
                if let Some(s) = hit {
                    if !s.is_empty() && self.vif_flat_key_applies(&raw) {
                        return Some(s);
                    }
                }
                // §23.9: a local or a property of `this` hides a same-named
                // interface instance (or vif alias) of an enclosing scope.
                if self.data_name_shadows_iface(&raw) {
                    return None;
                }
                if let Some(b) = self.iface_alias_for(&raw) {
                    return Some(b);
                }
                Some(raw)
            }
            // `obj.v` — a bound vif of another object.
            ExprKind::Ident(h) if h.path.len() == 2 => {
                // §25.9: `bus.modport` — a MODPORT VIEW of a structural
                // interface instance binds to the instance itself (a modport
                // restricts directions; it does not create new storage).
                // Without this, `rv = rb.driver;` bound nothing and every
                // write through `rv` landed on a phantom copy.
                let head = &h.path[0].name.name;
                // §23.6 / §25.9 (#570): `outer.leaf` names the interface
                // instance `leaf` nested in `outer`; an interface cannot
                // declare a modport and an instance of one name, so the
                // instance list decides before the modport-view reading.
                let nested = format!("{}.{}", head, h.path[1].name.name);
                if self.is_interface_instance(&nested) {
                    return Some(nested);
                }
                if self.is_interface_instance(head) {
                    return Some(head.clone());
                }
                // §25.9: `t.v` where `v` is a module-scope virtual-interface
                // VARIABLE (a name alias, not a class property) — e.g. an
                // omitted formal's default `x = t.v`. Resolve through the
                // module's variable aliases. Narrow to the current module's
                // own name so a same-named class property still wins.
                if head == &self.module.name
                    && let Some(b) = self
                        .viface_var_aliases
                        .get(&h.path[1].name.name)
                        .filter(|b| !b.is_empty())
                {
                    return Some(b.clone());
                }
                if let Some(p) = self.iface_instance_by_path(rvalue) {
                    return Some(p);
                }
                let oh = self.eval_ident_handle(head).unwrap_or(0);
                self.virtual_iface_bindings
                    .get(&(oh, h.path[1].name.name.clone()))
                    .map(|(b, _)| b.clone())
            }
            ExprKind::MemberAccess { expr, member } => {
                if let ExprKind::Ident(h) = &expr.kind {
                    if h.path.len() == 1 {
                        let head = &h.path[0].name.name;
                        // A nested interface instance first (#570), then the
                        // same §25.9 modport-view case in MemberAccess shape.
                        let nested = format!("{}.{}", head, member.name);
                        if self.is_interface_instance(&nested) {
                            return Some(nested);
                        }
                        if self.is_interface_instance(head) {
                            return Some(head.clone());
                        }
                        // §25.9: `t.v` where `v` is a module-scope
                        // virtual-interface VARIABLE alias (an omitted
                        // formal's default, e.g. `x = t.v`).
                        if head == &self.module.name
                            && let Some(b) = self
                                .viface_var_aliases
                                .get(&member.name)
                                .filter(|b| !b.is_empty())
                        {
                            return Some(b.clone());
                        }
                    }
                }
                if let Some(p) = self
                    .iface_instance_by_path(rvalue)
                    .or_else(|| self.nested_iface_instance(rvalue))
                {
                    return Some(p);
                }
                let oh = self.eval_handle_expr(expr).unwrap_or(0);
                self.virtual_iface_bindings
                    .get(&(oh, member.name.clone()))
                    .map(|(b, _)| b.clone())
            }
            // `a.b.d` / `g[0].a.d` — a nested interface instance.
            ExprKind::Ident(_) => self
                .iface_instance_by_path(rvalue)
                .or_else(|| self.nested_iface_instance(rvalue)),
            // `INT[i]` — an element of an interface-instance array
            // (`interrupt_if INT[N]()`). Bind to the element's name so
            // `vif.member` resolves to `INT[<i>].member` (interrupt examples set
            // the vif this way: `config_db#(virtual X)::set(..., INT[ii])`).
            ExprKind::Index { expr: base, index } => {
                if let ExprKind::Ident(h) = &base.kind {
                    if h.path.len() == 1 {
                        let idx = self.eval_scalar_self(index).unwrap_or(0);
                        return Some(format!("{}[{}]", h.path[0].name.name, idx));
                    }
                }
                // `o.arr[1]`: an element of an array nested in an instance.
                self.nested_iface_instance(rvalue)
            }
            _ => None,
        }
    }

    /// §23.6/§25.9: the instance path of an interface instance named by a
    /// HIERARCHICAL reference (`a.d`, `g[0].a.d`, `top.a.d`), searched upward
    /// from the executing instance scope; `None` when it names no interface
    /// instance. Only the head of a two-segment name was ever tried (as a
    /// modport view), so `px.vif = a.d` bound nothing and every call through
    /// the handle returned at once.
    pub(super) fn iface_instance_by_path(&self, e: &Expression) -> Option<String> {
        let segs = self.hier_segments(e)?;
        self.iface_instance_by_segments(segs)
    }

    /// §23.6/§25.9/§25.5 (#612): the interface instance a hierarchical name
    /// in a virtual-interface position names: an instance nested in another
    /// (`o.leaf`, `o.arr[1]`), or a modport view of one (`o.leaf.mp`), which
    /// binds the instance itself. Asked only when the name's last segment
    /// (or, for a modport view, the one before it) can end the path of an
    /// interface instance, so an ordinary hierarchical operand costs a set
    /// lookup.
    pub(super) fn nested_iface_instance(&self, e: &Expression) -> Option<String> {
        let (last_is, view_is) = self.nested_iface_gate(e);
        if !last_is && !view_is {
            return None;
        }
        let mut segs = self.hier_segments(e)?;
        if segs.len() < 2 {
            return None;
        }
        if last_is && let Some(p) = self.iface_instance_by_segments(segs.clone()) {
            return Some(p);
        }
        let mp = segs.pop()?;
        let inst = self.iface_instance_by_segments(segs.clone()).or_else(|| {
            (segs.len() == 1 && self.is_interface_instance(&segs[0])).then(|| segs[0].clone())
        })?;
        let def = &self
            .module
            .instances
            .iter()
            .find(|i| i.path == inst)?
            .def_name;
        self.module
            .modport_member_dirs
            .contains_key(&(def.clone(), mp))
            .then_some(inst)
    }

    /// Whether a multi-segment name's last segment, and the one before it,
    /// can end the path of an interface instance (`nested_iface_instance`).
    fn nested_iface_gate(&self, e: &Expression) -> (bool, bool) {
        let (tails, heads) = self.iface_path_parts();
        if tails.is_empty() {
            return (false, false);
        }
        // The last two names, without allocating: this runs for every
        // virtual-interface-capable operand and right-hand side.
        let mut names: [&str; 2] = [""; 2];
        let mut n = 0;
        let mut cur = e;
        let mut multi = false;
        while n < 2 {
            match &cur.kind {
                ExprKind::Ident(h) => {
                    multi |= h.path.len() > 1;
                    for s in h.path.iter().rev().take(2 - n) {
                        names[n] = s.name.name.as_str();
                        n += 1;
                    }
                    break;
                }
                ExprKind::MemberAccess { expr, member } => {
                    multi = true;
                    names[n] = member.name.as_str();
                    n += 1;
                    cur = expr;
                }
                ExprKind::Index { expr, .. } => cur = expr,
                _ => return (false, false),
            }
        }
        if !multi {
            return (false, false);
        }
        let ends = |s: &str| tails.contains(s) || heads.contains(s);
        let last_is = n > 0 && ends(names[0]);
        // A modport view (`o.leaf.mp`) only when the last name is a modport,
        // so `intf.sig` costs no instance search.
        let view_is = n > 1
            && ends(names[1])
            && self
                .module
                .modport_member_dirs
                .keys()
                .any(|(_, mp)| mp == names[0]);
        (last_is, view_is)
    }

    fn iface_instance_by_segments(&self, mut segs: Vec<String>) -> Option<String> {
        if segs.len() < 2 {
            return None;
        }
        if segs[0] == "$root" {
            segs.remove(0);
        }
        let path = segs.join(".");
        if segs.len() >= 2 && segs[0] == self.module.name {
            let rest = segs[1..].join(".");
            if self.is_interface_instance(&rest) {
                return Some(rest);
            }
        }
        // §23.8 upward name resolution from the executing instance.
        let scope = self.name_resolve_hint.borrow().clone();
        if let Some(scope) = scope {
            let mut sc = scope.as_str();
            loop {
                let cand = format!("{}.{}", sc, path);
                if self.is_interface_instance(&cand) {
                    return Some(cand);
                }
                match sc.rfind('.') {
                    Some(p) => sc = &sc[..p],
                    None => break,
                }
            }
        }
        self.is_interface_instance(&path).then_some(path)
    }

    /// Does a `a[i].b.c` receiver chain (MemberAccess/Index/Ident) select
    /// through an index anywhere?
    pub(super) fn member_chain_has_index(e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Index { .. } => true,
            ExprKind::Ident(h) => h.path.iter().any(|s| !s.selects.is_empty()),
            ExprKind::MemberAccess { expr, .. } => Self::member_chain_has_index(expr),
            _ => false,
        }
    }

    /// The `module.functions`/`module.tasks` key a dotted subroutine path
    /// names: the path itself (rooted at the top), else the path under the
    /// executing scope or one of its ancestors (§23.8 upward resolution).
    pub(super) fn scoped_subroutine_key(&self, path: &str) -> Option<String> {
        let explicit_root = path.starts_with("$root.");
        let path = path.strip_prefix("$root.").unwrap_or(path);
        let known =
            |k: &str| self.module.functions.contains_key(k) || self.module.tasks.contains_key(k);
        if known(path) {
            return Some(path.to_string());
        }
        if let Some(rest) = path
            .strip_prefix(self.module.name.as_str())
            .and_then(|r| r.strip_prefix('.'))
            .filter(|r| known(r))
            // A same-named local/object receiver shadows the implicit top
            // name, even when null. Only explicit $root bypasses that binding.
            .filter(|_| {
                explicit_root
                    || (self.dyn_name_lookup(&self.module.name).is_none()
                        && self.eval_ident_handle(&self.module.name).is_none())
            })
        {
            return Some(rest.to_string());
        }
        // §25.3: the formal interface-port path is an alias, so it need
        // not be a suffix of any physical subroutine key in the table.
        let top_shadowed = !explicit_root
            && path
                .strip_prefix(self.module.name.as_str())
                .is_some_and(|s| s.starts_with('.'))
            && (self.dyn_name_lookup(&self.module.name).is_some()
                || self.eval_ident_handle(&self.module.name).is_some());
        if !self.module.interface_port_aliases.is_empty() && !top_shadowed {
            let lookup = |candidate: &str| {
                let bound = xezim_core::elaborate::interface_ports::resolve(
                    candidate,
                    &self.module.interface_port_aliases,
                    &self.module.name,
                );
                (bound != candidate
                    && (self.module.functions.contains_key(bound.as_ref())
                        || self.module.tasks.contains_key(bound.as_ref())))
                .then(|| bound.into_owned())
            };
            if let Some(bound) = lookup(path) {
                return Some(bound);
            }
            if !explicit_root {
                let hint = self.name_resolve_hint.borrow();
                let mut scope = hint.as_deref();
                while let Some(current) = scope {
                    if let Some(bound) = lookup(&format!("{current}.{path}")) {
                        return Some(bound);
                    }
                    scope = current.rsplit_once('.').map(|(p, _)| p);
                }
            }
        }
        // Only a path that ends some subroutine key can resolve below.
        if !self.class_member_names().subroutine_suffixes.contains(path) {
            return None;
        }
        let hint = self.name_resolve_hint.borrow().clone();
        let mut scope = hint.as_deref()?;
        loop {
            let cand = format!("{}.{}", scope, path);
            if known(&cand) {
                return Some(cand);
            }
            scope = scope.rsplit_once('.')?.0;
        }
    }

    /// Is any class declared inside a module other than the top (computed
    /// once)?
    #[inline]
    pub(super) fn any_modclass(&self) -> bool {
        match self.has_modclass.get() {
            Some(b) => b,
            None => {
                let top = self.module.name.as_str();
                let b = self
                    .module
                    .classes
                    .values()
                    .any(|c| c.declaring_module.as_deref().is_some_and(|m| m != top));
                self.has_modclass.set(Some(b));
                b
            }
        }
    }

    /// Is a method of a class declared inside a non-top module running? Its
    /// bare names resolve per instance (§23.8).
    pub(super) fn module_class_method_active(&self) -> bool {
        if !self.any_modclass() {
            return false;
        }
        let Some(Some(ctx)) = self.class_context_stack.last() else {
            return false;
        };
        self.module
            .classes
            .get(ctx)
            .and_then(|cd| cd.declaring_module.as_deref())
            .is_some_and(|m| m != self.module.name)
    }

    /// The instance subroutine key a BARE `name` called from a method of a
    /// class declared inside a (non-top) module names: `name` under the
    /// object's instance or its nearest
    /// ancestor scope that declares it (§23.8). None outside such a method,
    /// so other bare calls are unchanged.
    pub(super) fn class_module_subroutine_key(&self, name: &str) -> Option<String> {
        if !self.module_class_method_active()
            || !self.class_member_names().subroutine_suffixes.contains(name)
        {
            return None;
        }
        // §8 / §23.8: the method body lives in the instance holding its
        // object, whichever process called it. The resolve hint is the
        // CALLING process's scope once an inlined method body runs (each
        // `run_process_stmts` activation reinstalls it), so a call reached
        // through a handle passed to a sibling instance found no subroutine
        // there and the module task was dropped.
        let hint = self.name_resolve_hint.borrow().clone();
        let inst = self
            .this_stack
            .last()
            .copied()
            .flatten()
            .and_then(|h| self.modclass_instance(h));
        // A descendant caller still belongs to another lexical module scope.
        // Its same-named task cannot override the object's declaring module.
        let start = inst.or(hint)?;
        let mut scope = start.as_str();
        loop {
            let cand = format!("{}.{}", scope, name);
            if self.module.functions.contains_key(&cand) || self.module.tasks.contains_key(&cand) {
                return Some(cand);
            }
            scope = scope.rsplit_once('.')?.0;
        }
    }

    /// Run the instance subroutine registered as `full` (a function's value,
    /// or 0 after a task) with its instance as the resolution scope.
    pub(super) fn call_scoped_subroutine(&mut self, full: &str, args: &[Expression]) -> Value {
        let scope = full.rsplit_once('.').map(|(s, _)| s.to_string());
        let saved = self.name_resolve_hint.borrow().clone();
        *self.name_resolve_hint.borrow_mut() = scope.clone();
        // §20.3: `$time`/`%m` inside the body belong to the defining instance.
        let saved_ts = self.timescale_scope_override.take();
        self.timescale_scope_override = scope;
        let r = if let Some(fd) = self.fn_decl_rc(full) {
            self.exec_function_call(&fd, args)
        } else if let Some(td) = self.task_decl_rc(full) {
            self.exec_task_call(&td, args);
            Value::zero(32)
        } else {
            Value::zero(32)
        };
        self.timescale_scope_override = saved_ts;
        *self.name_resolve_hint.borrow_mut() = saved;
        r
    }

    /// The segments of a hierarchical reference with every select evaluated
    /// (`g[1].a.d` -> `["g[1]", "a", "d"]`); `None` for any other shape.
    /// Selector probing is read-only: `eval_scalar_self` rejects calls and
    /// increments so a class receiver fallback performs their effects once.
    pub(super) fn hier_segments(&self, e: &Expression) -> Option<Vec<String>> {
        fn segs_of(sim: &Simulator, e: &Expression, out: &mut Vec<String>) -> Option<()> {
            match &e.kind {
                ExprKind::Ident(h) => {
                    for seg in &h.path {
                        let mut s = seg.name.name.clone();
                        for sel in &seg.selects {
                            s.push_str(&format!("[{}]", sim.eval_scalar_self(sel)?));
                        }
                        out.push(s);
                    }
                }
                ExprKind::MemberAccess { expr, member } => {
                    segs_of(sim, expr, out)?;
                    out.push(member.name.clone());
                }
                ExprKind::Index { expr, index } => {
                    segs_of(sim, expr, out)?;
                    let i = sim.eval_scalar_self(index)?;
                    out.last_mut()?.push_str(&format!("[{}]", i));
                }
                _ => return None,
            }
            Some(())
        }
        let mut segs = Vec::new();
        segs_of(self, e, &mut segs)?;
        Some(segs)
    }

    /// §13.5.3 / §25.9: materialize a formal's interface identity while
    /// its actual/default expression is still in the correct lexical scope.
    pub(super) fn vif_argument_value(
        &self,
        port: &crate::ast::decl::FunctionPort,
        expr: &Expression,
        value: Value,
    ) -> Value {
        if !Self::formal_may_carry_vif(&port.data_type)
            || !self.is_virtual_iface_type(&port.data_type)
        {
            return value;
        }
        if matches!(port.direction, crate::ast::types::PortDirection::Output) {
            return Value::zero(32);
        }
        if value
            .to_u64()
            .and_then(|token| self.iface_by_token(token))
            .is_some()
        {
            return value;
        }
        self.resolve_vif_rhs_name_strict(expr)
            .map(|bound| Value::from_u64(Self::iface_handle_token(&bound), 32))
            .unwrap_or(value)
    }

    /// §25.9: dispatch through the identity captured while binding the formal.
    /// Only a supplied actual may fall back to its caller-scope name.
    pub(super) fn vif_formal_alias(
        &self,
        port: &crate::ast::decl::FunctionPort,
        arg: Option<&Expression>,
        locals: &HashMap<String, Value>,
    ) -> Option<(String, String)> {
        if !self.is_virtual_iface_type(&port.data_type) {
            return None;
        }
        let formal = port.name.name.as_str();
        // §13.5.2: an output starts at its type's default, with no copy-in.
        if matches!(port.direction, crate::ast::types::PortDirection::Output) {
            return Some((formal.to_string(), String::new()));
        }
        // §13.5.3: omitted defaults were already evaluated in the declaration
        // scope. Reuse their value rather than resolving them in the caller.
        let bound = locals
            .get(formal)
            .and_then(|value| value.to_u64())
            .filter(|&token| token != 0)
            .and_then(|token| self.iface_by_token(token))
            .map(str::to_string)
            .or_else(|| arg.and_then(|arg| self.resolve_vif_rhs_name_strict(arg)));
        Some((formal.to_string(), bound.unwrap_or_default()))
    }

    /// A getter used as an argument has already delivered its token to the
    /// formal. Its return marker must not bind the callee's scalar result.
    pub(super) fn consume_vif_argument_return(&mut self) {
        if self.vif_return_pending {
            self.signals.remove("__vif_return__");
            self.vif_return_pending = false;
        }
        self.last_vif_return = None;
    }

    /// LRM §25.9: resolve a leading identifier through the active
    /// virtual-interface alias scope. The innermost task/function call
    /// frame (vif FORMALS) wins; a plain `virtual <iface>` VARIABLE
    /// alias (`vif = bus;` — see `viface_var_aliases`) is the
    /// fallback. Returns the bound interface instance name.
    /// Resolve a dotted clocking-block reference (`cb`, `iface.cb`, or
    /// `vif.cb` through a virtual-interface alias) to its `clocking_meta`
    /// key. `segs` is the dotted path WITHOUT the trailing signal segment
    /// (for `cb.sig` forms) or the full path (for `@(cb)` forms).
    /// §14.13/§25.9: a plain dotted MemberAccess chain (`vif.cb`,
    /// `vif.cb.data`) as the FLAT Ident the module-scope paths expect. Inside
    /// a subroutine body the parser emits the chain form, and both the
    /// event-sensitivity collector and the clocking-drive arm matched only the
    /// flat spelling — so `@(vif.cb)` armed on nothing (returned immediately)
    /// and `vif.cb.data <= v` missed the clocking check and drove a phantom.
    /// `None` for any shape with indexing or calls.
    pub(super) fn member_chain_as_flat_ident(e: &Expression) -> Option<Expression> {
        Self::member_chain_as_flat_ident_impl(e, false)
    }

    /// Sensitivity-list variant: also flattens chains rooted at a literal
    /// `this` (`@(posedge this.drv.bus.clk_i)`). Such a chain collected
    /// NOTHING, so the wait armed on an empty sensitivity and the event
    /// control became a no-op: a `forever … @(posedge this.drv.bus.clk_i)`
    /// monitor spun at time 0 forever (the snitch driver-class stall). The
    /// literal "this" segment is resolved by the waiter's handle-chain late
    /// binding. Lvalue call sites must NOT take this arm — an assignment
    /// through `this.vif.member` has to keep its MemberAccess shape so the
    /// handle walk applies the §25.8 virtual-interface redirect.
    pub(super) fn member_chain_as_flat_ident_for_sens(e: &Expression) -> Option<Expression> {
        Self::member_chain_as_flat_ident_impl(e, true)
    }

    pub(super) fn member_chain_as_flat_ident_impl(
        e: &Expression,
        allow_this: bool,
    ) -> Option<Expression> {
        fn collect(
            e: &Expression,
            out: &mut Vec<crate::ast::Identifier>,
            allow_this: bool,
        ) -> bool {
            match &e.kind {
                ExprKind::Ident(h) if h.path.iter().all(|s| s.selects.is_empty()) => {
                    out.extend(h.path.iter().map(|s| s.name.clone()));
                    true
                }
                ExprKind::This if allow_this => {
                    out.push(crate::ast::Identifier {
                        name: "this".to_string(),
                        span: e.span,
                    });
                    true
                }
                ExprKind::MemberAccess { expr, member } => {
                    if !collect(expr, out, allow_this) {
                        return false;
                    }
                    out.push(member.clone());
                    true
                }
                _ => false,
            }
        }
        if !matches!(e.kind, ExprKind::MemberAccess { .. }) {
            return None;
        }
        let mut names = Vec::new();
        if !collect(e, &mut names, allow_this) || names.len() < 2 {
            return None;
        }
        let span = e.span;
        Some(Expression::new(
            ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                root: None,
                path: names
                    .into_iter()
                    .map(|name| crate::ast::expr::HierPathSegment {
                        name,
                        selects: Vec::new(),
                    })
                    .collect(),
                span,
                cached_signal_id: std::cell::Cell::new(None),
                cached_resolved_name: std::cell::OnceCell::new(),
            }),
            span,
        ))
    }

    pub(super) fn resolve_clocking_key(&self, segs: &[&str]) -> Option<String> {
        if segs.is_empty() || self.clocking_meta.is_empty() {
            return None;
        }
        let raw = segs.join(".");
        if self.clocking_meta.contains_key(&raw) {
            return Some(raw);
        }
        // §14.3: a clocking block declared in a module or interface INSTANCE
        // is registered under its instance path (`m.cb`). A bare `@(cb)` in
        // that instance's own task or process resolves through the executing
        // scope — unresolved, it waited on a signal literally named `cb`,
        // which in an interface task returned at once and spun the caller's
        // sampling loop. An inlined instance-task body runs under its frame's
        // scope, which process re-entry does not keep in the transient hint.
        {
            let hint = self.name_resolve_hint.borrow();
            let scopes = [
                hint.as_deref(),
                self.task_cleanup
                    .last()
                    .and_then(|c| c.frame_scope_hint.as_deref()),
                self.process_scope_hint
                    .get(&self.current_pid)
                    .map(String::as_str),
            ];
            for sc in scopes.into_iter().flatten().filter(|s| !s.is_empty()) {
                let scoped = format!("{}.{}", sc, raw);
                if self.clocking_meta.contains_key(&scoped) {
                    return Some(scoped);
                }
            }
        }
        // §25.9 virtual interface: rewrite the head through the vif alias
        // (`vif.cb_main` → `master_if.cb_main`). The CENTRAL binding, so a
        // class-property vif (`@(vif.cb)` in a driver's run task) resolves
        // too — `iface_alias_for` alone never saw those.
        if let Some(bound) = self.vif_bound_for_root(segs[0]) {
            let mut aliased = bound;
            for seg in &segs[1..] {
                aliased.push('.');
                aliased.push_str(seg);
            }
            if self.clocking_meta.contains_key(&aliased) {
                return Some(aliased);
            }
        }
        None
    }

    /// §14.3 clocking-block input read for a dotted path whose LAST segment is
    /// the signal: `cb.sig`, `iface.cb.sig`, `vif.cb.sig`. Returns the value
    /// sampled at the block's most recent clock edge (`#1step` input skew), or
    /// the live net before the first edge. `None` when the path doesn't name a
    /// clocking block, so callers fall through to generic resolution.
    ///
    /// Shared by the flat-hier-Ident and the MemberAccess shapes — inside a
    /// task/function body the parser emits `MemberAccess`, so without the
    /// second call site `cb.sig` read 0 in every subroutine (and in `wait()`
    /// conditions), while the same read worked at module scope.
    /// Flatten a plain dotted MemberAccess/Ident chain to its segments.
    /// `None` for any shape with indexing/calls (not a simple hierarchical
    /// name), so callers don't misread `a[i].b` as a clocking path.
    pub(super) fn flatten_member_path(expr: &Expression) -> Option<Vec<String>> {
        match &expr.kind {
            ExprKind::Ident(h) if h.path.iter().all(|s| s.selects.is_empty()) => {
                Some(h.path.iter().map(|s| s.name.name.clone()).collect())
            }
            ExprKind::MemberAccess { expr: base, member } => {
                let mut v = Self::flatten_member_path(base)?;
                v.push(member.name.clone());
                Some(v)
            }
            _ => None,
        }
    }

    /// First segment of a `flatten_member_path` chain, without building it.
    pub(super) fn member_path_head(expr: &Expression) -> Option<&str> {
        match &expr.kind {
            ExprKind::Ident(h) if h.path.iter().all(|s| s.selects.is_empty()) => {
                h.path.first().map(|s| s.name.name.as_str())
            }
            ExprKind::MemberAccess { expr: base, .. } => Self::member_path_head(base),
            _ => None,
        }
    }

    /// `flatten_member_path` joined with `.` into `out`, without the per-
    /// segment copies: the number of segments written, or `None` for a shape
    /// it does not flatten.
    pub(super) fn push_member_path(expr: &Expression, out: &mut String) -> Option<usize> {
        match &expr.kind {
            ExprKind::Ident(h) if h.path.iter().all(|s| s.selects.is_empty()) => {
                for (i, s) in h.path.iter().enumerate() {
                    if i > 0 {
                        out.push('.');
                    }
                    out.push_str(&s.name.name);
                }
                Some(h.path.len())
            }
            ExprKind::MemberAccess { expr: base, member } => {
                let n = Self::push_member_path(base, out)?;
                if n > 0 {
                    out.push('.');
                }
                out.push_str(&member.name);
                Some(n + 1)
            }
            _ => None,
        }
    }

    /// Can `<recv>.<member>` read a clocking-block signal? Every key
    /// `resolve_clocking_key` tries for the receiver ends in the receiver's
    /// last segment, except the vif alias of its first segment, which needs
    /// a vif-capable name.
    pub(super) fn clocking_read_possible(&self, recv: &Expression) -> bool {
        if self.clocking_meta.is_empty() {
            return false;
        }
        let mut cur = recv;
        let mut last: Option<&str> = None;
        let first = loop {
            match &cur.kind {
                ExprKind::MemberAccess { expr: base, member } => {
                    last.get_or_insert(member.name.as_str());
                    cur = base;
                }
                ExprKind::Ident(h) if h.path.iter().all(|s| s.selects.is_empty()) => {
                    let Some(f) = h.path.first() else {
                        return false;
                    };
                    if last.is_none() {
                        last = h.path.last().map(|s| s.name.name.as_str());
                    }
                    break f.name.name.as_str();
                }
                _ => return false,
            }
        };
        let last = last.unwrap_or(first);
        self.vif_name_possible(first)
            || self
                .clocking_meta
                .keys()
                .any(|k| k == last || k.strip_suffix(last).is_some_and(|p| p.ends_with('.')))
    }

    /// §14.3: `cb.sig[i]` where `sig` is an unpacked-array clockvar — the
    /// sample of element `i` taken at the block's last clocking event.
    /// `None` for any other indexed read, before the index is evaluated.
    pub(super) fn clocking_array_elem_read(
        &mut self,
        base: &Expression,
        index: &Expression,
    ) -> Option<Value> {
        if self.clocking_array_leaves.is_empty() {
            return None;
        }
        let leaf = match &base.kind {
            ExprKind::MemberAccess { member, .. } => member.name.as_str(),
            ExprKind::Ident(h) => h.path.last()?.name.name.as_str(),
            _ => return None,
        };
        if !self.clocking_array_leaves.contains(leaf) {
            return None;
        }
        let segs = Self::flatten_member_path(base)?;
        let (leaf, owner) = segs.split_last()?;
        let owner: Vec<&str> = owner.iter().map(String::as_str).collect();
        let cb = self.resolve_clocking_key(&owner)?;
        // Samples are keyed by the clocking-visible name, which for a block
        // of an instance is the net's path (`bus.ready[0]`).
        let prefix = format!("{}[", leaf);
        let is_elem_of = |k: &str| k.rsplit('.').next().is_some_and(|l| l.starts_with(&prefix));
        if !self
            .clocking_snapshots
            .get(&cb)?
            .keys()
            .any(|k| is_elem_of(k))
        {
            return None;
        }
        let iv = self.eval_expr(index);
        let i = iv.to_i64().filter(|_| !iv.has_xz())?;
        let elem = format!("{}{}]", prefix, i);
        self.clocking_snapshots
            .get(&cb)?
            .iter()
            .find(|(k, _)| k.rsplit('.').next() == Some(elem.as_str()))
            .map(|(_, v)| v.clone())
    }

    pub(super) fn try_clocking_signal_read(&self, segs: &[&str]) -> Option<Value> {
        if segs.len() < 2 {
            return None;
        }
        let cb = self.resolve_clocking_key(&segs[..segs.len() - 1])?;
        let raw_sig = segs[segs.len() - 1];
        // §14.3 renaming: `cb.alias` samples the bound net.
        let sig: &str = self
            .clocking_sig_alias
            .get(&cb)
            .and_then(|m| m.get(raw_sig))
            .map(|s| s.as_str())
            .unwrap_or(raw_sig);
        let matches_sig = |k: &str| k == sig || k.rsplit('.').next() == Some(sig);
        let snap = self.clocking_snapshots.get(&cb)?;
        // Samples are keyed by the clocking-visible name (`clocking_vis`).
        if let Some(v) = snap.get(raw_sig) {
            return Some(v.clone());
        }
        if let Some((_, v)) = snap
            .iter()
            .find(|(k, _)| k.rsplit('.').next() == Some(raw_sig))
        {
            return Some(v.clone());
        }
        if let Some(v) = snap.get(sig) {
            return Some(v.clone());
        }
        if let Some((_, v)) = snap.iter().find(|(k, _)| matches_sig(k)) {
            return Some(v.clone());
        }
        // §14.13: a clockvar read before the block's first clocking event
        // has not been sampled yet and holds its type's default (x for a
        // 4-state net), not the live net value.
        let (_, sigs) = self.clocking_meta.get(&cb)?;
        let Some((net, _)) = sigs.iter().find(|(n, is_in)| *is_in && matches_sig(n)) else {
            // §14.3 makes reading an OUTPUT clockvar illegal;
            // Verilator answers with the driven net's current value
            // (`$display(mif.cb.addr)` after `mif.cb.addr <= a`), so do the
            // same instead of reading x.
            let (net, _) = sigs.iter().find(|(n, _)| matches_sig(n))?;
            return self.get_signal_value_by_name(net);
        };
        match self.signal_name_to_id.get(net.as_str()).copied() {
            Some(id) if id < self.signal_widths.len() => {
                let w = self.signal_widths[id];
                if self.signal_two_state.get(id).copied().unwrap_or(false) {
                    Some(Value::zero(w))
                } else {
                    Some(Value::all_x(w))
                }
            }
            _ => self.get_signal_value_by_name(net),
        }
    }

    /// §8.13/§25.9: the virtual-interface declaration of property `prop` as
    /// an object of class `class_name` sees it — the class's own, else the
    /// nearest ancestor's (a derived class inherits its base's vif
    /// properties). Returns the DECLARING class and `(interface type,
    /// modport)`; `None` when the nearest declaration of `prop` on the chain
    /// is not a virtual interface. Every vif consumer asks this one question:
    /// keyed on the object's leaf class alone, a vif declared in a base class
    /// was invisible to binding, reads, writes and event controls alike.
    pub(super) fn class_vif_decl<'a>(
        &'a self,
        class_name: &'a str,
        prop: &str,
    ) -> Option<(&'a str, &'a (String, Option<String>))> {
        let mut cur: Option<&'a str> = Some(class_name);
        while let Some(cn) = cur {
            let cd = self.module.classes.get(cn)?;
            if let Some(d) = cd.virtual_iface_properties.get(prop) {
                return Some((cn, d));
            }
            if cd.properties.contains_key(prop)
                || cd.param_defaults.iter().any(|(name, _)| name == prop)
            {
                return None;
            }
            cur = cd.extends.as_deref();
        }
        None
    }

    /// §6.18: the class a class-scope virtual-interface receiver NAMES, once
    /// typedef aliases are unwound. `typedef box_c box_t;` makes `box_t::svif`
    /// the very same member as `box_c::svif`, because a static property
    /// belongs to the class rather than to the spelling used to reach it.
    /// `None` means the name resolves to no class at all, so callers keep
    /// whatever they were about to do. The common case — a real class name —
    /// costs one hash lookup.
    pub(super) fn vif_class_of_scope_name(&self, name: &str) -> Option<String> {
        if self.module.classes.contains_key(name) {
            return Some(name.to_string());
        }
        self.resolve_class_scope_prefix(name)
            .map(|(base, _)| base)
            .filter(|base| self.module.classes.contains_key(base))
    }

    /// §25.8/§25.10/§8.9: the `virtual_iface_bindings` slot of the vif
    /// property named by `key` (`prop`, or `prop[i]` for an element of a vif
    /// array) on object `handle` of class `class_name`, with the property's
    /// declared modport. A STATIC property has one cell per declaring class,
    /// keyed under the null handle; an instance property needs a live object.
    pub(super) fn vif_slot(
        &self,
        class_name: &str,
        handle: usize,
        key: &str,
    ) -> Option<((usize, String), Option<String>)> {
        let base = key.split('[').next().unwrap_or(key);
        // §6.18: a TYPEDEF ALIAS names the same class, so `box_t::svif` and
        // `box_c::svif` are ONE cell — a static property belongs to the class,
        // not to the spelling used to reach it. Canonicalising here, the single
        // choke point every vif read, write and comparison passes through,
        // keeps the alias from forking a second cell keyed by its own name.
        let canon = self.vif_class_of_scope_name(class_name);
        let class_name = canon.as_deref().unwrap_or(class_name);
        let (decl, (_, modport)) = self.class_vif_decl(class_name, base)?;
        if self
            .module
            .classes
            .get(decl)
            .is_some_and(|cd| cd.static_properties.contains(base))
        {
            return Some(((0, format!("{}::{}", decl, key)), modport.clone()));
        }
        (handle != 0).then(|| ((handle, key.to_string()), modport.clone()))
    }

    /// The interface instance (and declared modport) bound to vif property
    /// `key` of object `handle` / class `class_name` (see [`vif_slot`]).
    pub(super) fn vif_prop_target(
        &self,
        class_name: &str,
        handle: usize,
        key: &str,
    ) -> Option<(String, Option<String>)> {
        let (slot, modport) = self.vif_slot(class_name, handle, key)?;
        if slot.0 == 0 && !key.contains('[') {
            // §8.26: object and typedef spellings retain the specialization
            // of the declaring class, including an inherited static.
            let spec = self
                .heap
                .get(handle)
                .and_then(|o| o.as_ref())
                .and_then(|o| o.spec.as_ref());
            return self
                .static_vif_binding_of_scope(class_name, key, spec)
                .map(|bound| (bound, modport));
        }
        let (bound, _) = self.virtual_iface_bindings.get(&slot)?;
        Some((bound.clone(), modport))
    }

    /// Can `name` be a virtual-interface PROPERTY of some class? Every
    /// binding is keyed by such a name, so a receiver whose trailing name
    /// fails this never resolves through one. A borrowed lookup behind a
    /// first-byte filter: this runs on member chains of every class design.
    #[inline]
    pub(super) fn vif_prop_name_possible(&self, name: &str) -> bool {
        let n = self.class_member_names();
        name.as_bytes()
            .first()
            .is_some_and(|&b| n.vif_first[b as usize])
            && n.vif_props.contains(name)
    }

    /// The object (or, for a class name, the class scope, handle 0) owning a
    /// vif property reached through the select-free path `segs` — `d`,
    /// `d.a.cfg`, `this.cfg`, `Cls` — walked through the heap without side
    /// effects.
    pub(super) fn vif_owner_of_path<'a>(
        &'a self,
        segs: &'a [crate::ast::expr::HierPathSegment],
    ) -> Option<(&'a str, usize)> {
        let (first, rest) = segs.split_first()?;
        let head = first.name.name.as_str();
        let mut h = match first.selects.as_slice() {
            [] if head == "this" => self.this_stack.last().copied().flatten()?,
            [] => match self.eval_ident_handle(head) {
                Some(h) if h != 0 => h,
                // `Cls::svif` — a class scope owns only its statics.
                _ if rest.is_empty() && self.vif_class_of_scope_name(head).is_some() => {
                    return Some((head, 0));
                }
                // §26.3: `pkg::Cls::p` — a PACKAGE-qualified class scope. The
                // class is the LAST segment of the owner path, the same head
                // `extract_call_spec` reads for `pkg::Cls#(args)::p`.
                _ if rest.len() == 1
                    && rest.iter().all(|s| s.selects.is_empty())
                    && self.module.classes.contains_key(rest[0].name.name.as_str()) =>
                {
                    return Some((rest[0].name.name.as_str(), 0));
                }
                _ => return None,
            },
            // An element of a handle collection (`ag[0].cfg`).
            [sel] => {
                let coll = self
                    .instance_assoc_member(head)
                    .unwrap_or_else(|| head.to_string());
                self.coll_elem_handle(&coll, head, sel)?
            }
            _ => return None,
        };
        for s in rest {
            if h == 0 {
                return None;
            }
            h = match s.selects.as_slice() {
                [] => self.member_handle(h, &s.name.name)?,
                [sel] => {
                    let coll = self.handle_collection_name(h, &s.name.name)?;
                    self.coll_elem_handle(&coll, &s.name.name, sel)?
                }
                _ => return None,
            };
        }
        if h == 0 {
            return None;
        }
        Some((self.heap.get(h)?.as_ref()?.class_name.as_str(), h))
    }

    /// The owner of `owner.prop`: `this`, a class scope, or any handle chain
    /// `eval_handle_expr` resolves side-effect-free (a local or formal
    /// handle, `a.cfg`, `arr[i].cfg`).
    pub(super) fn vif_owner_of_expr<'a>(
        &'a self,
        owner: &'a Expression,
    ) -> Option<(&'a str, usize)> {
        let h = match &owner.kind {
            ExprKind::This => self.this_stack.last().copied().flatten()?,
            ExprKind::Ident(h) if h.root.is_none() => return self.vif_owner_of_path(&h.path),
            ExprKind::Paren(inner) => return self.vif_owner_of_expr(inner),
            _ => self.eval_handle_expr(owner)?,
        };
        if h == 0 {
            return None;
        }
        Some((self.heap.get(h)?.as_ref()?.class_name.as_str(), h))
    }

    /// A BARE vif name (`key` is `name`, or `name[i]` for an array element):
    /// a vif formal or plain `virtual` variable (the alias frames), else a vif
    /// property of `this` — own or inherited, instance or static — or, in a
    /// static method, a static one of the lexical class. A frame local of
    /// that name shadows the property.
    pub(super) fn vif_bare_target(
        &self,
        name: &str,
        key: &str,
    ) -> Option<(String, Option<String>)> {
        if let Some(b) = self.iface_alias_for(key) {
            return Some((b, None));
        }
        if self
            .local_stack
            .last()
            .is_some_and(|l| l.contains_key(name))
        {
            return None;
        }
        // §8.26: a static virtual interface of a PARAMETERIZED class has one
        // cell PER SPECIALIZATION, and the write files the binding in
        // `static_vif_bindings` under the canonical spec key `C#(T)::p`. Inside
        // a static method the executing specialization IS the context, so ask
        // with it BEFORE the per-object table below — a design whose only bound
        // virtual interfaces are class statics never fills
        // `virtual_iface_bindings` at all, and the emptiness gate that lookup
        // used to sit behind declined the read outright. `vif_prop_target`
        // canonicalises the class and asks for `C::p`, missing the spec cell:
        // that is why the same static read back fine through the qualified
        // spelling but not bare. A miss is no opinion, and the paths below run
        // exactly as before.
        if key == name
            && self.vif_prop_name_possible(name)
            && let Some(ctx) = self.class_context_stack.last().cloned().flatten()
            && let Some(b) =
                self.static_vif_binding_of_scope(&ctx, name, self.current_spec.as_ref())
        {
            let modport = self.vif_slot(&ctx, 0, key).and_then(|(_, mp)| mp);
            return Some((b, modport));
        }
        if self.virtual_iface_bindings.is_empty() || !self.vif_prop_name_possible(name) {
            return None;
        }
        match self.this_stack.last().copied().flatten() {
            Some(th) if th != 0 => {
                let cn = self.heap.get(th)?.as_ref()?.class_name.as_str();
                self.vif_prop_target(cn, th, key)
            }
            _ => {
                let ctx = self.class_context_stack.last()?.as_deref()?;
                self.vif_prop_target(ctx, 0, key)
            }
        }
    }

    /// [`vif_recv_target`] for a flattened path whose LAST segment names the
    /// vif (optionally with one element select).
    pub(super) fn vif_recv_path(
        &self,
        segs: &[crate::ast::expr::HierPathSegment],
    ) -> Option<(String, Option<String>)> {
        let (last, owner) = segs.split_last()?;
        let elem = match last.selects.as_slice() {
            [] => None,
            [sel] => Some(sel),
            _ => return None,
        };
        let name = last.name.name.as_str();
        if owner.is_empty() {
            return self.vif_named_target(None, name, elem);
        }
        if !self.vif_prop_name_possible(name) {
            return None;
        }
        let owner = self.vif_owner_of_path(owner)?;
        self.vif_named_target(Some(owner), name, elem)
    }

    /// The vif named `name` — element `elem` of it for a §25.10 vif array or a
    /// vif collection — on `owner` (object class and handle; handle 0 for a
    /// class scope), or, with no owner, a BARE name (see [`vif_bare_target`]).
    pub(super) fn vif_named_target(
        &self,
        owner: Option<(&str, usize)>,
        name: &str,
        elem: Option<&Expression>,
    ) -> Option<(String, Option<String>)> {
        if owner.is_some() || !self.iface_alias_possible() {
            // Nothing but a vif property, or (bare) a `virtual` collection
            // variable holding handle tokens, can resolve.
            if !self.vif_prop_name_possible(name)
                && !(owner.is_none() && elem.is_some() && self.vif_tokens_stored)
            {
                return None;
            }
        }
        // §7.9/§7.10/§25.10: the current element store is authoritative
        // after copying, deletion, shifting or reallocation.
        if let Some(index) = elem
            && (owner.is_some()
                || self.vif_collection(name)
                || self
                    .class_context_stack
                    .last()
                    .and_then(|c| c.as_deref())
                    .is_some_and(|c| self.static_fixed_key_in(c, name).is_some()))
        {
            return self.vif_elem_by_value(owner, name, index);
        }
        let key: std::borrow::Cow<str> = match elem {
            None => std::borrow::Cow::Borrowed(name),
            Some(ix) => {
                let storage = match owner {
                    Some((cn, 0)) => self.spec_static_coll_key_in_class(name, cn),
                    Some((_, handle)) => format!("{}#{}", handle, name),
                    None => self
                        .instance_assoc_member(name)
                        .unwrap_or_else(|| name.to_string()),
                };
                let index = if self.is_string_keyed_array(&storage) {
                    self.eval_index_value_self(ix)?.to_sv_string()
                } else {
                    self.eval_scalar_self(ix)?.to_string()
                };
                std::borrow::Cow::Owned(format!("{}[{}]", name, index))
            }
        };
        let hit = match owner {
            None => self.vif_bare_target(name, &key),
            Some((cn, oh)) => self.vif_prop_target(cn, oh, &key),
        };
        if hit.is_some() || !self.vif_tokens_stored {
            return hit;
        }
        self.vif_elem_by_value(owner, name, elem?)
    }

    /// §25.9/§7.10: a vif held as an ELEMENT of a collection — a queue,
    /// dynamic or associative array of virtual interfaces — has no binding
    /// slot (none would survive `push_front`/`insert`/`delete`): the element
    /// holds the handle token itself (see `queue_eval_arg`), which names the
    /// instance. `owner` owns the collection property `name`; with none,
    /// `name` is a property of `this` or a plain `virtual` variable.
    pub(super) fn vif_elem_by_value(
        &self,
        owner: Option<(&str, usize)>,
        name: &str,
        index: &Expression,
    ) -> Option<(String, Option<String>)> {
        let owner = owner.or_else(|| {
            let th = self
                .this_stack
                .last()
                .copied()
                .flatten()
                .filter(|&h| h != 0)?;
            let cn = self.heap.get(th)?.as_ref()?.class_name.as_str();
            self.class_vif_decl(cn, name).map(|_| (cn, th))
        });
        // §8.9: a static member reached through an object still names the
        // declaring class's one store.
        let owner = owner.map(|(cn, oh)| {
            if let Some((decl, _)) = self.class_vif_decl(cn, name)
                && self
                    .module
                    .classes
                    .get(decl)
                    .is_some_and(|cd| cd.static_properties.contains(name))
            {
                (decl, 0)
            } else {
                (cn, oh)
            }
        });
        let (coll, modport) = match owner {
            // §8.9: a static collection of a class scope (`S::pq[0]`).
            Some((cn, 0)) => {
                let (_, (_, mp)) = self.class_vif_decl(cn, name)?;
                (
                    self.static_fixed_key_in(cn, name)
                        .unwrap_or_else(|| self.spec_static_coll_key_in_class(name, cn)),
                    mp.clone(),
                )
            }
            Some((cn, oh)) => {
                let (_, (_, mp)) = self.class_vif_decl(cn, name)?;
                (self.handle_collection_name(oh, name)?, mp.clone())
            }
            None if self.vif_collection(name)
                || self
                    .class_context_stack
                    .last()
                    .and_then(|c| c.as_deref())
                    .is_some_and(|c| self.static_fixed_key_in(c, name).is_some()) =>
            {
                (
                    self.class_context_stack
                        .last()
                        .and_then(|c| c.as_deref())
                        .and_then(|c| self.static_fixed_key_in(c, name))
                        .or_else(|| self.static_method_coll_key(name))
                        .unwrap_or_else(|| name.to_string()),
                    None,
                )
            }
            None => return None,
        };
        if !self.is_associative_array(&coll)
            && !self.vif_index_in_bounds(&coll, self.eval_scalar_self(index)?)
        {
            return None;
        }
        let token = self.coll_elem_handle(&coll, name, index)? as u64;
        self.iface_by_token(token)
            .map(|inst| (inst.to_string(), modport))
    }

    /// §7.4/§7.5/§7.10: spare backing slots never denote live elements.
    fn vif_index_in_bounds(&self, collection: &str, index: i64) -> bool {
        if self.module.dynamic_arrays.contains(collection)
            || self.module.queue_vars.contains(collection)
            || self.signals.contains_key(&format!("{}.size", collection))
        {
            index >= 0 && (index as u64) < self.get_queue_size(collection)
        } else if let Some(&(lo, hi, _)) = self.module.arrays.get(collection) {
            (lo..=hi).contains(&index)
        } else {
            true
        }
    }

    /// §25.8/§25.9/§25.10 — THE receiver resolution step: the interface
    /// instance (with the declared modport) that a receiver expression of
    /// VIRTUAL-INTERFACE type denotes, whatever its spelling:
    /// * a bare name — a vif formal or plain `virtual` variable, or a vif
    ///   property of `this`, own or inherited, instance or static;
    /// * a vif property reached through any class-handle chain — `cfg.vif`,
    ///   `a.cfg.vif`, `this.cfg.vif`, `c.vif` (a local or formal handle),
    ///   `d.cfg.vif` at module scope, `Cls::svif`;
    /// * an element of a vif array or collection — `vifs[i]`, `d.vifs[i]`,
    ///   `cfg.vifs[i]`, `vq[0]`.
    ///
    /// The owner chain resolves side-effect-free (`&self`): a receiver that
    /// needs a CALL to reach its owner never resolves here, so no consumer
    /// evaluates a receiver twice. `None` when `recv` is no bound vif.
    pub(super) fn vif_recv_target(&self, recv: &Expression) -> Option<(String, Option<String>)> {
        if !self.module.packages.is_empty()
            && let Some(stripped) = self.strip_package_lvalue(recv)
        {
            return self.vif_recv_target(&stripped);
        }
        match &recv.kind {
            ExprKind::Paren(inner) => self.vif_recv_target(inner),
            // The receiver-call machinery has already evaluated this call
            // once; its virtual-interface result carries the instance token.
            ExprKind::Call { .. } => {
                let token = self.recv_call_memo_hit(recv)?.to_u64()?;
                self.iface_by_token(token).map(|b| (b.to_string(), None))
            }
            ExprKind::Ident(h) if h.root.is_none() => self.vif_recv_path(&h.path),
            ExprKind::MemberAccess {
                expr: owner,
                member,
            } => {
                // §8.25/§25.9: a static vif of a specialization,
                // `G#(virtual pif)::one`, binds per specialization.
                // §26.3: a static vif of a PACKAGE-qualified class,
                // `pkg::G::one`, reaches the same class-scope cell; neither
                // spelling resolves an owner HANDLE, so consult the static
                // binding directly instead of walking a nonexistent chain.
                let class_scope_head = match &owner.kind {
                    ExprKind::Specialization { .. } => true,
                    ExprKind::MemberAccess { member, .. } => {
                        self.module.classes.contains_key(member.name.as_str())
                    }
                    _ => false,
                };
                if class_scope_head && let Some(b) = self.static_vif_binding(recv) {
                    return Some((b, None));
                }
                if !self.vif_prop_name_possible(&member.name) {
                    return None;
                }
                let owner = self.vif_owner_of_expr(owner)?;
                self.vif_named_target(Some(owner), &member.name, None)
            }
            ExprKind::Index { expr: base, index } => {
                // §8.25 / §25.10: a specialized static collection stores
                // interface tokens under its per-specialization key.
                if self.vif_tokens_stored
                    && let Some(coll) = self.static_vif_key(base)
                    && let Some(token) = self.coll_elem_handle(&coll, "", index)
                    && let Some(bound) = self.iface_by_token(token as u64)
                {
                    return Some((bound.to_string(), None));
                }
                match &base.kind {
                    ExprKind::Ident(h)
                        if h.root.is_none()
                            && h.path.last().is_some_and(|s| s.selects.is_empty()) =>
                    {
                        let (last, owner) = h.path.split_last()?;
                        let name = last.name.name.as_str();
                        if owner.is_empty() {
                            return self.vif_named_target(None, name, Some(index));
                        }
                        if !self.vif_prop_name_possible(name) {
                            return None;
                        }
                        let owner = self.vif_owner_of_path(owner)?;
                        self.vif_named_target(Some(owner), name, Some(index))
                    }
                    ExprKind::MemberAccess {
                        expr: owner,
                        member,
                    } => {
                        if !self.vif_prop_name_possible(&member.name) {
                            return None;
                        }
                        let owner = self.vif_owner_of_expr(owner)?;
                        self.vif_named_target(Some(owner), &member.name, Some(index))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// §25.8/§25.9: the interface INSTANCE a bare name (or `name[i]` element
    /// key) is bound to — [`vif_bare_target`] without the modport.
    pub(super) fn vif_bound_for_root(&self, root: &str) -> Option<String> {
        let base = root.split('[').next().unwrap_or(root);
        self.vif_bare_target(base, root).map(|(b, _)| b)
    }

    /// A flat identifier expression over `path`.
    pub(super) fn hier_ident_expr(
        path: Vec<crate::ast::expr::HierPathSegment>,
        span: crate::ast::Span,
    ) -> Expression {
        Expression::new(
            ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                root: None,
                path,
                span,
                cached_signal_id: std::cell::Cell::new(None),
                cached_resolved_name: std::cell::OnceCell::new(),
            }),
            span,
        )
    }

    /// §25.9: a resolved virtual interface denotes a physical instance, not
    /// another class property. Root the rewritten path so a matching property
    /// name cannot rebind it again (e.g. `vif` bound to `vif.child`).
    fn resolved_iface_expr(
        path: Vec<crate::ast::expr::HierPathSegment>,
        span: crate::ast::Span,
    ) -> Expression {
        let mut expr = Self::hier_ident_expr(path, span);
        if let ExprKind::Ident(h) = &mut expr.kind {
            h.root = Some("$root".to_string());
        }
        expr
    }

    /// The path segments of interface instance `bound` (`bus`, `top.u.bus`,
    /// `INT[1]`).
    pub(super) fn iface_path_segs(
        bound: &str,
        span: crate::ast::Span,
    ) -> Vec<crate::ast::expr::HierPathSegment> {
        bound
            .split('.')
            .map(|part| crate::ast::expr::HierPathSegment {
                name: crate::ast::Identifier {
                    name: part.to_string(),
                    span,
                },
                selects: Vec::new(),
            })
            .collect()
    }

    /// Could any name along the chain `e` be a vif property? The prefilter of
    /// [`vif_rebase_expr`] when no alias exists.
    pub(super) fn chain_may_name_vif(&self, e: &Expression) -> bool {
        // The member a chain SELECTS (`data` in `cfg.vif.data`, the method in
        // `cfg.vif.t()`, the leaf of a flat `d.vif.data`) is never a receiver:
        // start below it.
        let mut cur = e;
        while let ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } = &cur.kind {
            cur = expr;
        }
        match &cur.kind {
            ExprKind::MemberAccess { expr, .. } => cur = expr,
            ExprKind::Call { func, .. } => match &func.kind {
                ExprKind::MemberAccess { expr, .. } => cur = expr,
                ExprKind::Ident(h) if h.path.len() >= 2 => {
                    return h.path[..h.path.len() - 1]
                        .iter()
                        .any(|s| self.vif_prop_name_possible(&s.name.name));
                }
                _ => return false,
            },
            ExprKind::Ident(h) if h.path.len() >= 2 => {
                return h.path[..h.path.len() - 1]
                    .iter()
                    .any(|s| self.vif_prop_name_possible(&s.name.name));
            }
            _ => {}
        }
        loop {
            match &cur.kind {
                ExprKind::MemberAccess { expr, member } => {
                    if self.vif_prop_name_possible(&member.name) {
                        return true;
                    }
                    cur = expr;
                }
                ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => cur = expr,
                ExprKind::Call { func, .. } => cur = func,
                ExprKind::Paren(inner) => cur = inner,
                ExprKind::Ident(h) => {
                    return h
                        .path
                        .iter()
                        .any(|s| self.vif_prop_name_possible(&s.name.name));
                }
                _ => return false,
            }
        }
    }

    /// Rewrite an expression chain whose RECEIVER is a bound virtual
    /// interface — at any depth, spelled any way [`vif_recv_target`]
    /// resolves — into a DIRECT access of the bound instance, so everything
    /// downstream (read, write, NBA, event and wait controls, clocking
    /// blocks, interface subroutine calls) sees the instance path, which
    /// already works in every shape. Inside a subroutine body the parser
    /// emits `MemberAccess`/`Index`/`Call` chains rather than one flat
    /// `Ident`; at module scope the same source is one flat `Ident`. `None`
    /// when no receiver in the chain is a bound vif — the common case, gated
    /// to a few emptiness and name checks.
    pub(super) fn vif_rebase_expr(&self, e: &Expression) -> Option<Expression> {
        // A definition-qualified self reference is an instance access even
        // without a virtual handle. Normalize the whole selected chain so
        // writes, reads and calls use the same instance as vif returns do.
        if let Some(local) = self.strip_own_definition_name(e) {
            return Some(local);
        }
        let alias_possible = self.iface_alias_possible();
        // Returned handles live in the current receiver memo. They must not
        // enable collection-token lookup on every later member expression.
        let returned_possible = !self.recv_call_memo.is_empty();
        if !alias_possible
            && self.virtual_iface_bindings.is_empty()
            && self.static_vif_bindings.is_empty()
            && !self.vif_tokens_stored
            && !returned_possible
        {
            return None;
        }
        // Only CHAINS are rebased: a bare `v` read/write (or `d.v`, `varr[0]`)
        // is a HANDLE operation (`v2 = v;`, `v == null`) and stays untouched.
        match &e.kind {
            ExprKind::Ident(h) if h.path.len() >= 2 => {}
            ExprKind::MemberAccess { .. }
            | ExprKind::Call { .. }
            | ExprKind::Index { .. }
            | ExprKind::RangeSelect { .. } => {}
            _ => return None,
        }
        if !alias_possible
            && !self.vif_tokens_stored
            && !returned_possible
            && !self.chain_may_name_vif(e)
        {
            return None;
        }
        self.vif_rebase_chain(e)
    }

    /// A flattened path: the first prefix `path[..=i]` (with at least one
    /// segment after it) that is a bound vif is replaced by the bound
    /// instance's path.
    pub(super) fn vif_rebase_ident(
        &self,
        h: &crate::ast::expr::HierarchicalIdentifier,
        span: crate::ast::Span,
    ) -> Option<Expression> {
        if h.root.is_some() {
            return None;
        }
        let n = h.path.len();
        let alias_possible = self.iface_alias_possible();
        for i in 0..n.saturating_sub(1) {
            let seg = &h.path[i];
            let name = seg.name.name.as_str();
            if !(self.vif_prop_name_possible(name)
                || (i == 0 && (alias_possible || self.vif_tokens_stored)))
            {
                continue;
            }
            let Some((bound, _)) = self.vif_recv_path(&h.path[..=i]) else {
                continue;
            };
            // IDENTITY REWRITE GUARD. Every consumer RE-ENTERS on the rewrite,
            // so a rewrite that reproduces its input recurses until the stack
            // dies. The universal UVM convention names the class property after
            // the interface instance (`virtual foo_if vif;` bound to
            // `foo_if vif()`), so `vif.x` would rebase to `vif.x`. Declining is
            // exactly equivalent: the path already names the instance.
            if i == 0 && seg.selects.is_empty() && bound == name {
                return None;
            }
            let mut segs = Self::iface_path_segs(&bound, span);
            segs.extend(h.path[i + 1..].iter().cloned());
            return Some(Self::resolved_iface_expr(segs, span));
        }
        None
    }

    /// Recursive half of [`vif_rebase_expr`]: the innermost receiver in the
    /// chain that is a bound vif becomes the instance identifier; the
    /// selects, members and call around it are rebuilt unchanged.
    pub(super) fn vif_rebase_chain(&self, e: &Expression) -> Option<Expression> {
        match &e.kind {
            ExprKind::Ident(h) if h.path.len() >= 2 => self.vif_rebase_ident(h, e.span),
            ExprKind::MemberAccess { expr: recv, member } => {
                if let Some((bound, _)) = self.vif_recv_target(recv) {
                    // Identity guard (see `vif_rebase_ident`): the receiver
                    // already names the bound instance.
                    if matches!(&recv.kind, ExprKind::Ident(rh)
                        if rh.path.len() == 1
                            && rh.path[0].selects.is_empty()
                            && rh.path[0].name.name == bound)
                    {
                        return None;
                    }
                    // The instance's member as ONE flat identifier — exactly
                    // the module-scope spelling `bus.data` of the same access,
                    // which every consumer (width inference and NBA targets
                    // included) resolves.
                    let mut segs = Self::iface_path_segs(&bound, e.span);
                    segs.push(crate::ast::expr::HierPathSegment {
                        name: member.clone(),
                        selects: Vec::new(),
                    });
                    return Some(Self::resolved_iface_expr(segs, e.span));
                }
                let nb = self.vif_rebase_chain(recv)?;
                // A plain member of a rebased instance path stays flat too
                // (`cfg.vif.cb.data` → `bus.cb.data`).
                if let ExprKind::Ident(h) = &nb.kind {
                    let mut path = h.path.clone();
                    path.push(crate::ast::expr::HierPathSegment {
                        name: member.clone(),
                        selects: Vec::new(),
                    });
                    return Some(Self::resolved_iface_expr(path, e.span));
                }
                Some(Expression::new(
                    ExprKind::MemberAccess {
                        expr: Box::new(nb),
                        member: member.clone(),
                    },
                    e.span,
                ))
            }
            ExprKind::Index { expr, index } => {
                let nb = self.vif_rebase_chain(expr)?;
                Some(Expression::new(
                    ExprKind::Index {
                        expr: Box::new(nb),
                        index: index.clone(),
                    },
                    e.span,
                ))
            }
            ExprKind::RangeSelect {
                expr,
                left,
                right,
                kind,
            } => {
                let nb = self.vif_rebase_chain(expr)?;
                Some(Expression::new(
                    ExprKind::RangeSelect {
                        expr: Box::new(nb),
                        left: left.clone(),
                        right: right.clone(),
                        kind: *kind,
                    },
                    e.span,
                ))
            }
            ExprKind::Call { func, args } => {
                let nf = self.vif_rebase_chain(func)?;
                Some(Expression::new(
                    ExprKind::Call {
                        func: Box::new(nf),
                        args: args.clone(),
                    },
                    e.span,
                ))
            }
            _ => None,
        }
    }

    /// May some interface alias exist for any name (an alias frame entry or
    /// a module-level vif variable alias)?
    #[inline]
    pub(super) fn iface_alias_possible(&self) -> bool {
        self.local_iface_aliases
            .last()
            .is_some_and(|m| !m.is_empty())
            || !self.viface_var_aliases.is_empty()
    }

    /// Could the bare name `raw` resolve through any virtual-interface
    /// source: an alias, a vif property (binding keys are vif properties),
    /// a `__vif_local__` key, or an interface instance of that name? When
    /// false every vif lookup on it misses.
    pub(super) fn vif_name_possible(&self, raw: &str) -> bool {
        self.iface_alias_possible()
            || self.class_member_names().vif_props.contains(raw)
            || self.vif_local_names.contains(raw)
            || self.is_interface_instance(raw)
    }

    /// Could `resolve_vif_rhs_name(e)` come back with an interface-instance
    /// name? Its sources: a vif binding (keyed by a vif property name), a
    /// `__vif_local__` key, an alias, the name itself, an interface instance
    /// reached by path (whose last segment is a tail of an instance path),
    /// or an element `a[i]` of an interface-instance array. When false the
    /// strict resolver returns None and callers filtering on
    /// `is_interface_instance` find nothing; the resolver only reads state.
    pub(super) fn vif_rhs_possible(&self, e: &Expression) -> bool {
        if !self.module.packages.is_empty()
            && let Some(stripped) = self.strip_package_lvalue(e)
        {
            return self.vif_rhs_possible(&stripped);
        }
        if let Some(local) = self.strip_own_definition_name(e) {
            return self.vif_rhs_possible(&local);
        }
        if !self.static_vif_names.is_empty() {
            let leaf = match &e.kind {
                ExprKind::Ident(h) => h.path.last().map(|s| s.name.name.as_str()),
                ExprKind::MemberAccess { member, .. } => Some(member.name.as_str()),
                _ => None,
            };
            if leaf.is_some_and(|l| self.static_vif_names.contains(l)) {
                return true;
            }
        }
        // `o.leaf.mp`, `o.arr[1]`: a nested interface instance (#612).
        if !matches!(self.nested_iface_gate(e), (false, false)) {
            return true;
        }
        match &e.kind {
            ExprKind::Ident(h) if h.path.len() == 1 => self.vif_name_possible(&h.path[0].name.name),
            ExprKind::Ident(h) => {
                if h.path.iter().any(|s| !s.selects.is_empty()) {
                    return true;
                }
                let last = h.path[h.path.len() - 1].name.name.as_str();
                (h.path.len() == 2
                    && (self.is_interface_instance(&h.path[0].name.name)
                        || self.class_member_names().vif_props.contains(last)
                        // §25.9: `t.v` where `v` is a module-scope
                        // virtual-interface VARIABLE alias, e.g. an omitted
                        // formal's default.
                        || (h.path[0].name.name == self.module.name
                            && self.viface_var_aliases.contains_key(last))))
                    || self.iface_path_parts().0.contains(last)
            }
            ExprKind::MemberAccess { expr, member } => {
                matches!(&expr.kind, ExprKind::Ident(bh)
                    if bh.path.len() == 1 && self.is_interface_instance(&bh.path[0].name.name))
                    || self
                        .class_member_names()
                        .vif_props
                        .contains(member.name.as_str())
                    || self.iface_path_parts().0.contains(member.name.as_str())
                    // §25.9: `t.v` where `v` is a module-scope virtual-interface
                    // VARIABLE alias, e.g. an omitted formal's default.
                    || (matches!(&expr.kind, ExprKind::Ident(bh)
                            if bh.path.len() == 1
                                && bh.path[0].name.name == self.module.name)
                        && self.viface_var_aliases.contains_key(member.name.as_str()))
            }
            ExprKind::Index { expr: base, .. } => {
                if self.vif_tokens_stored && self.static_vif_key(base).is_some() {
                    return true;
                }
                match &base.kind {
                    ExprKind::Ident(h) if h.path.len() == 1 => {
                        let n = h.path[0].name.name.as_str();
                        self.iface_path_parts().1.contains(n)
                        // §25.10: an element of a vif array / collection
                        // (`vq[i]`, `pa[i]`).
                        || self.vif_prop_name_possible(n)
                        || self.vif_local_names.contains(n)
                        || self.module.var_decl_types.get(n)
                            .is_some_and(|dt| self.is_virtual_iface_type(dt))
                        || (self.vif_tokens_stored && self.vif_collection(n))
                    }
                    ExprKind::Ident(h) => h
                        .path
                        .last()
                        .is_some_and(|s| self.vif_prop_name_possible(&s.name.name)),
                    ExprKind::MemberAccess { member, .. } => {
                        self.vif_prop_name_possible(&member.name)
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// Every `.`-separated tail of an interface-instance path (the path
    /// included), and every prefix of one that ends before a `[` (with its
    /// last segment alone).
    pub(super) fn iface_path_parts(&self) -> &(HashSet<String>, HashSet<String>) {
        self.iface_path_parts.get_or_init(|| {
            let mut tails: HashSet<String> = HashSet::default();
            let mut heads: HashSet<String> = HashSet::default();
            for inst in self
                .module
                .instances
                .iter()
                .filter(|i| self.module.interfaces.contains(&i.def_name))
            {
                let p = inst.path.as_str();
                tails.insert(p.to_string());
                for (k, c) in p.char_indices() {
                    match c {
                        '.' => {
                            tails.insert(p[k + 1..].to_string());
                        }
                        '[' => {
                            heads.insert(p[..k].to_string());
                            // The array's own name, for `o.arr[1]` (#612).
                            if let Some(leaf) = p[..k].rsplit('.').next() {
                                heads.insert(leaf.to_string());
                            }
                        }
                        _ => {}
                    }
                }
            }
            (tails, heads)
        })
    }

    /// §26.3: binding keys follow the same package ownership as signal
    /// storage. A frame local still hides an imported package variable.
    pub(super) fn vif_variable_name<'a>(&self, name: &'a str) -> std::borrow::Cow<'a, str> {
        let (base, suffix) = name
            .split_once('[')
            .map_or((name, ""), |(base, _)| (base, &name[base.len()..]));
        if !self.contested_pkg_vars.contains(base)
            || self
                .local_stack
                .last()
                .is_some_and(|f| f.contains_key(base))
        {
            return std::borrow::Cow::Borrowed(name);
        }
        let qualified = match self.pkg_scope_stack.last().and_then(|p| p.as_deref()) {
            Some(pkg) => self.pkg_var_storage(pkg, base),
            None => self.contested_pkg_var_in_scope(base),
        };
        match qualified {
            Some(mut qualified) => {
                qualified.push_str(suffix);
                std::borrow::Cow::Owned(qualified)
            }
            None => std::borrow::Cow::Borrowed(name),
        }
    }

    /// §23.9/§25.9: task formals use their binding frame. Function formals
    /// retain flat-slot lookup for bound values and an explicit null marker
    /// for unbound ones. Generic class aliases are not vif variables.
    pub(super) fn vif_frame_alias(&self, name: &str) -> Option<Option<String>> {
        let frame = self.local_iface_aliases.last()?;
        if !frame.contains_key("__xezim_task_vif_frame__") {
            // Function formals also carry an explicit null marker. Bound
            // function values keep their existing flat-slot lookup order.
            return (frame.get(name).is_some_and(String::is_empty)
                && self
                    .local_stack
                    .last()
                    .is_some_and(|f| f.contains_key(name)))
            .then_some(None);
        }
        let bound = frame.get(name)?;
        let owned = self
            .local_stack
            .last()
            .is_some_and(|f| f.contains_key(name))
            || self
                .ref_alias_stack
                .last()
                .is_some_and(|f| f.contains_key(name));
        if owned && (bound.is_empty() || self.is_interface_instance(bound)) {
            Some(self.iface_alias_for(name))
        } else {
            None
        }
    }

    pub(super) fn iface_alias_for(&self, name: &str) -> Option<String> {
        if let Some(b) = self
            .local_iface_aliases
            .last()
            .filter(|frame| !frame.is_empty())
            .and_then(|frame| frame.get(name))
        {
            if self
                .local_iface_aliases
                .last()
                .is_some_and(|f| f.contains_key("__xezim_task_vif_frame__"))
                && let Some(storage) = self.ref_alias_stack.last().and_then(|m| m.get(name))
            {
                return self
                    .get_signal_value_by_name(storage)
                    .and_then(|v| v.to_u64())
                    .and_then(|t| self.iface_by_token(t))
                    .map(str::to_string);
            }
            return (!b.is_empty()).then(|| b.clone());
        }
        if self.viface_var_aliases.is_empty() {
            return None; // fast path: no vif variables bound anywhere
        }
        let bound = self
            .viface_var_aliases
            .get(self.vif_variable_name(name).as_ref())?;
        // §23.9 / §8.4: a frame-local or class property hides an enclosing
        // module variable. Formal aliases above still take precedence.
        if self
            .local_stack
            .last()
            .is_some_and(|frame| frame.contains_key(name))
        {
            return None;
        }
        let class = self
            .this_stack
            .last()
            .copied()
            .flatten()
            .and_then(|handle| self.heap.get(handle).and_then(|object| object.as_ref()))
            .map(|object| object.class_name.as_str())
            .or_else(|| {
                self.class_context_stack
                    .last()
                    .and_then(|class| class.as_deref())
            });
        if class.is_some_and(|class| self.class_vif_decl(class, name).is_some()) {
            return None;
        }
        Some(bound.clone())
    }

    /// §26.3: the task a `pkg::t` callee names — the package-qualified key
    /// elaboration gives a colliding declaration, else the hoisted bare one.
    /// None unless the head is a registered package that no signal shadows.
    pub(super) fn package_task_target(&self, func: &Expression) -> Option<TaskDeclaration> {
        let (pkg, name) = match &func.kind {
            ExprKind::Ident(h)
                if h.path.len() == 2 && h.path.iter().all(|s| s.selects.is_empty()) =>
            {
                (&h.path[0].name.name, &h.path[1].name.name)
            }
            ExprKind::MemberAccess { expr: recv, member } => match &recv.kind {
                ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                    (&h.path[0].name.name, &member.name)
                }
                _ => return None,
            },
            _ => return None,
        };
        if !self.module.packages.contains(pkg)
            || self.signal_name_to_id.contains_key(pkg.as_str())
            || self.signals.contains_key(pkg)
        {
            return None;
        }
        self.module
            .tasks
            .get(&format!("{}::{}", pkg, name))
            .or_else(|| self.module.tasks.get(name))
            .cloned()
    }

    /// Resolve a dotted/interface task ENABLE target to its full key in
    /// `module.tasks` ("u_if.sample_one"), or None if the callee is not a
    /// known hierarchical/interface task. Handles both syntactic forms
    /// (flattened multi-segment Ident, and MemberAccess in class bodies)
    /// and both binding shapes: a vif alias/property on the HEAD name
    /// (`vif.t()`), and a vif property reached through a handle chain
    /// (`cfg.vif.t()`, `this.vif.t()` — the owner object is evaluated and
    /// its property binding consulted).
    pub(super) fn resolve_hier_task_target(&self, func: &Expression) -> Option<String> {
        // §25.8/§25.9: a callee reached through a bound virtual-interface
        // receiver (`cfg.vif.t()`, `d.a.cfg.vif.t()`, `vifs[i].t()`) names the
        // bound instance's task.
        if let Some(rebased) = self.vif_rebase_expr(func) {
            return self.resolve_hier_task_target(&rebased);
        }
        // The path itself, else under the executing scope and upward (§23.8).
        let in_tasks = |full: String| -> Option<String> {
            if self.module.tasks.contains_key(&full) {
                Some(full)
            } else {
                self.scoped_subroutine_key(&full)
                    .filter(|p| self.module.tasks.contains_key(p))
            }
        };
        // Form 1: head-name binding — alias/formal, then the §25.8
        // class-property binding on the current `this`.
        let head_parts: Option<(String, Vec<String>)> = match &func.kind {
            ExprKind::Ident(h)
                if h.path.len() >= 2 && h.path.iter().all(|sg| sg.selects.is_empty()) =>
            {
                Some((
                    h.path[0].name.name.clone(),
                    h.path[1..].iter().map(|sg| sg.name.name.clone()).collect(),
                ))
            }
            // Inside a subroutine body a dotted name parses as a MemberAccess
            // chain — `ai.bi.tick` is `(ai.bi).tick` — so flatten the whole
            // chain down to its select-free root, not just one level. With
            // only one level a three-segment enable in a task never inlined:
            // it ran synchronously and its `#delay` loop outlived `$finish`.
            ExprKind::MemberAccess { .. } => {
                let mut members: Vec<String> = Vec::new();
                let mut cur = func;
                loop {
                    match &cur.kind {
                        ExprKind::MemberAccess { expr: recv, member } => {
                            members.push(member.name.clone());
                            cur = recv;
                        }
                        ExprKind::Ident(h1)
                            if !h1.path.is_empty()
                                && h1.path.iter().all(|sg| sg.selects.is_empty()) =>
                        {
                            let mut tail: Vec<String> =
                                h1.path[1..].iter().map(|sg| sg.name.name.clone()).collect();
                            tail.extend(members.into_iter().rev());
                            break Some((h1.path[0].name.name.clone(), tail));
                        }
                        _ => break None,
                    }
                }
            }
            _ => None,
        };
        if let Some((head, tail)) = head_parts {
            let head_owned = self
                .iface_alias_for(&head)
                .or_else(|| {
                    let this_h = self.this_stack.last().copied().flatten()?;
                    self.virtual_iface_bindings
                        .get(&(this_h, head.clone()))
                        .map(|(bound, _)| bound.clone())
                })
                .unwrap_or(head);
            let mut segs: Vec<&str> = vec![head_owned.as_str()];
            segs.extend(tail.iter().map(|t| t.as_str()));
            if let Some(full) = in_tasks(segs.join(".")) {
                return Some(full);
            }
        } else if let Some(segs) = self.hier_segments(func) {
            // A path through generate-block / instance-array selects
            // (`g[0].a.d.wait_rst()`) names the task by its evaluated path.
            if segs.len() >= 2 && segs.iter().any(|sg| sg.contains('[')) {
                if let Some(full) = in_tasks(segs.join(".")) {
                    return Some(full);
                }
            }
        }
        // Form 2: nested owner — split the callee into
        // (owner expr, vif property, method) and consult the owner OBJECT's
        // property binding.
        let (owner, prop, meth): (Expression, String, String) = match &func.kind {
            ExprKind::Ident(h)
                if h.path.len() >= 3 && h.path.iter().all(|sg| sg.selects.is_empty()) =>
            {
                let n = h.path.len();
                let mut owner_h = h.clone();
                owner_h.path.truncate(n - 2);
                (
                    Expression::new(ExprKind::Ident(owner_h), func.span),
                    h.path[n - 2].name.name.clone(),
                    h.path[n - 1].name.name.clone(),
                )
            }
            ExprKind::MemberAccess { expr: recv, member } => match &recv.kind {
                ExprKind::MemberAccess {
                    expr: r2,
                    member: p2,
                } => ((**r2).clone(), p2.name.clone(), member.name.clone()),
                ExprKind::Ident(h1)
                    if h1.path.len() >= 2 && h1.path.iter().all(|sg| sg.selects.is_empty()) =>
                {
                    let n = h1.path.len();
                    let mut owner_h = h1.clone();
                    owner_h.path.truncate(n - 1);
                    (
                        Expression::new(ExprKind::Ident(owner_h), recv.span),
                        h1.path[n - 1].name.name.clone(),
                        member.name.clone(),
                    )
                }
                _ => return None,
            },
            _ => return None,
        };
        let oh = self.eval_handle_expr(&owner)?;
        if oh == 0 {
            return None;
        }
        let bound = self.virtual_iface_bindings.get(&(oh, prop))?.0.clone();
        in_tasks(format!("{}.{}", bound, meth))
    }

    pub(super) fn indexed_iface_alias_for(&self, expr: &Expression) -> Option<String> {
        let ExprKind::Index { expr: base, index } = &expr.kind else {
            return None;
        };
        let ExprKind::Ident(h) = &base.kind else {
            return None;
        };
        if h.path.len() != 1 || !h.path[0].selects.is_empty() {
            return None;
        }
        let idx = self.eval_scalar_self(index)?;
        self.iface_alias_for(&format!("{}[{}]", h.path[0].name.name, idx))
    }

    /// Is `path` an elaborated INTERFACE instance (e.g. `bus` of
    /// definition `bus_if`)? Used to validate the RHS of a
    /// `vif = <instance>` alias assignment (LRM §25.9) so an unrelated
    /// value assignment never records a bogus alias.
    pub(super) fn is_interface_instance(&self, path: &str) -> bool {
        // Built once: the instance list is fixed after elaboration, and this
        // is asked per virtual-interface operand at run time (a linear scan
        // with a string compare per instance was 1.5 % of a c910 run).
        let set = self.iface_instance_paths.get_or_init(|| {
            self.module
                .instances
                .iter()
                .filter(|i| self.module.interfaces.contains(&i.def_name))
                .map(|i| i.path.clone())
                .collect()
        });
        set.contains(path)
    }

    /// §25.9: the VALUE a virtual interface bound to instance `path` holds —
    /// a non-null token derived from the path (FNV-1a folded to 31 bits, low
    /// bit set), the same for every holder, so handles compare equal exactly
    /// when they name the same instance.
    pub(super) fn iface_handle_token(path: &str) -> u64 {
        let mut hsh: u64 = 0xcbf29ce484222325;
        for b in path.bytes() {
            hsh ^= b as u64;
            hsh = hsh.wrapping_mul(0x100000001b3);
        }
        (hsh & 0x7FFF_FFFF) | 1
    }

    /// The interface instance a virtual-interface VALUE names (the inverse
    /// of [`iface_handle_token`]) — how a vif held in storage with no binding
    /// slot (a queue, dynamic or associative array element) resolves.
    pub(super) fn iface_by_token(&self, token: u64) -> Option<&str> {
        self.iface_by_token
            .get_or_init(|| {
                self.module
                    .instances
                    .iter()
                    .filter(|i| self.module.interfaces.contains(&i.def_name))
                    .map(|i| (Self::iface_handle_token(&i.path), i.path.clone()))
                    .collect()
            })
            .get(&token)
            .map(String::as_str)
    }

    /// Is the collection stored as `coll` — a class property's
    /// `<handle>#<prop>` store or a plain variable — declared with
    /// virtual-interface elements (`virtual bus_if vq[$]`)?
    pub(super) fn vif_collection(&self, coll: &str) -> bool {
        if self.module.interfaces.is_empty() {
            return false;
        }
        match coll.split_once('#') {
            Some((h, _)) if h.parse::<usize>().is_err() => self.static_vif_collection(coll),
            Some((h, prop)) => {
                self.vif_prop_name_possible(prop)
                    && h.parse::<usize>()
                        .ok()
                        .and_then(|h| self.heap.get(h)?.as_ref())
                        .is_some_and(|i| self.class_vif_decl(&i.class_name, prop).is_some())
            }
            None => {
                self.module
                    .var_decl_types
                    .get(coll)
                    .is_some_and(|dt| self.is_virtual_iface_type(dt))
                    || self.static_vif_collection(coll)
            }
        }
    }

    /// §8.9/§25.9: is `coll` the store of a STATIC collection some class
    /// declares with virtual-interface elements (`static virtual pif pq[$]`,
    /// stored bare or as `Cls::pq`)?
    pub(super) fn static_vif_collection(&self, coll: &str) -> bool {
        let leaf = coll.rsplit("::").next().unwrap_or(coll);
        if self.vif_prop_name_possible(leaf)
            && self.module.classes.values().any(|cd| {
                cd.virtual_iface_properties.contains_key(leaf)
                    && cd.static_collections.iter().any(|(n, ..)| n == leaf)
            })
        {
            return true;
        }
        let Some((base, _)) = &self.current_spec else {
            return false;
        };
        let Some(cd) = self.module.classes.get(base) else {
            return false;
        };
        cd.static_collections.iter().any(|(n, ..)| n == leaf)
            && cd
                .property_types
                .get(leaf)
                .is_some_and(|dt| self.is_virtual_iface_type(dt))
    }

    /// §25.9: does `dt` denote a virtual-interface handle type — `virtual
    /// <iface>[.modport]` spelled out, a bare interface type name, or a
    /// typedef chain (package-scoped, `$unit` or local) ending at either?
    /// Is `dt` a class handle type (a class name, through typedefs)?
    pub(super) fn dt_is_class_handle(dt: &DataType, module: &ElaboratedModule) -> bool {
        match super::elaborate::resolve_typedef_chain(dt, &module.typedef_types) {
            DataType::TypeReference { name, .. } => {
                module.classes.contains_key(&name.name.name)
                    && !module.interfaces.contains(&name.name.name)
            }
            _ => false,
        }
    }

    pub(super) fn dt_is_virtual_iface(dt: &DataType, module: &ElaboratedModule) -> bool {
        match super::elaborate::resolve_typedef_chain(dt, &module.typedef_types) {
            DataType::Interface { .. } => true,
            DataType::TypeReference { name, .. } => module.interfaces.contains(&name.name.name),
            _ => false,
        }
    }

    /// Is `dt` a virtual-interface variable type — the `virtual <iface>`
    /// form (`DataType::Interface`) or a TypeReference naming a known
    /// interface definition? Mirrors the formal-arg check in
    /// `vif_formal_alias` (LRM §25.9).
    pub(super) fn is_virtual_iface_type(&self, dt: &crate::ast::types::DataType) -> bool {
        use crate::ast::types::DataType;
        // §6.18 / §25.9: package variables retain their declared typedef;
        // its resolved interface type carries the binding semantics too.
        match super::elaborate::resolve_typedef_chain(dt, &self.module.typedef_types) {
            DataType::Interface { .. } => true,
            DataType::TypeReference { name, .. } => {
                let n = &name.name.name;
                if self.module.interfaces.contains(n) {
                    // Name collision resolution (§3.12): a plain (non-`virtual`)
                    // `TypeReference` whose NAME is BOTH an interface and a
                    // class (e.g. `interface shared;` + `class shared;`) is an
                    // ambiguous probe. SV resolves the procedural block-local
                    // declaration `shared h;` to the CLASS handle, NOT a
                    // virtual interface, so prefer the class when one exists.
                    // Only an explicit `virtual <iface>` (`DataType::Interface`
                    // above) unambiguously denotes a virtual interface.
                    if self.module.classes.contains_key(n)
                        || self.module.covergroups.contains_key(n)
                    {
                        return false;
                    }
                    return true;
                }
                // A TYPE-PARAM formal (`function void set(T value)` inside
                // `uvm_config_db#(virtual pipe_if)`) is a virtual-interface
                // type when the ACTIVE SPECIALIZATION binds it to one —
                // without this, the §25.9 formal alias never engaged and a
                // vif passed through config_db arrived as nothing
                // (issue #113: the driver's vif stayed null).
                if let Some(resolved) = self.resolve_type_param_binding(n) {
                    let explicit_virtual = resolved.trim_start().starts_with("virtual");
                    let base = resolved
                        .trim_start_matches("virtual")
                        .trim()
                        .split(['#', '.'])
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if self.module.interfaces.contains(&base) {
                        // §3.13: a bare type argument that also names a class
                        // (`registry#(mon)` beside `interface mon`) is the
                        // class; only `virtual mon` denotes the interface.
                        return explicit_virtual
                            || !(self.module.classes.contains_key(&base)
                                || self.module.covergroups.contains_key(&base));
                    }
                }
                false
            }
            _ => false,
        }
    }

    /// Resolve a `(handle, prop)` virtual-interface binding to its bound
    /// interface instance name, if any.
    #[allow(dead_code)]
    pub(super) fn virtual_iface_bound_name(&self, handle: usize, prop: &str) -> Option<String> {
        self.virtual_iface_bindings
            .get(&(handle, prop.to_string()))
            .map(|(n, _)| n.clone())
    }
}
