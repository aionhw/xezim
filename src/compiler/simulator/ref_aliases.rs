//! §13.5.2 `ref` formals aliased to storage that is not a plain module
//! variable: a class property and a hierarchical or member name.

use super::*;

/// Name prefix of the hidden module-scope handle variable that pins an
/// object while a `ref` formal is aliased to one of its properties. Its
/// value is the handle itself, so `<pin>.<prop>` names the property from any
/// scope through the ordinary variable-dot-property machinery.
const REF_PIN_PREFIX: &str = "__xz_ref_obj";

/// Alias storage prefix of a `static` class property, `<prefix><Class>::<p>`:
/// the class-scope name reaches the one shared cell from any scope (§8.9).
const REF_STATIC_PREFIX: &str = "__xz_ref_static:";

/// Alias-frame key whose value is the generation of the local frame that
/// owns the frame's aliased struct leaves (see `ref_frame_leaves`).
const REF_LEAF_FRAME_KEY: &str = "\u{1}leaf_frame";

impl Simulator {
    /// `(handle, property)` of an alias storage name that pins a property.
    pub(super) fn ref_pin_parts(storage: &str) -> Option<(usize, &str)> {
        let rest = storage.strip_prefix(REF_PIN_PREFIX)?;
        let (h, prop) = rest.split_once('.')?;
        Some((h.parse().ok()?, prop))
    }

    /// `(class, property)` of a static-property alias storage name.
    pub(super) fn ref_static_parts(storage: &str) -> Option<(&str, &str)> {
        storage.strip_prefix(REF_STATIC_PREFIX)?.split_once("::")
    }

    /// The two-segment identifier of a pinned (`<pin>.<prop>`) or static
    /// (`Class::prop`) storage name.
    pub(super) fn ref_pin_hier(
        storage: &str,
        span: crate::ast::Span,
    ) -> Option<crate::ast::expr::HierarchicalIdentifier> {
        let (pin, prop) = match Self::ref_static_parts(storage) {
            Some(parts) => parts,
            None => {
                let (_, prop) = Self::ref_pin_parts(storage)?;
                (&storage[..storage.len() - prop.len() - 1], prop)
            }
        };
        let seg = |name: &str| crate::ast::expr::HierPathSegment {
            name: crate::ast::Identifier {
                name: name.to_string(),
                span,
            },
            selects: Vec::new(),
        };
        Some(crate::ast::expr::HierarchicalIdentifier {
            root: None,
            path: vec![seg(pin), seg(prop)],
            span,
            cached_signal_id: crate::ast::cache::SignalCache::new(None),
            cached_resolved_name: crate::ast::cache::NameCache::new(),
            bound: crate::ast::binding::Bound::UNSET,
        })
    }

    /// Whether `name` is a class-handle formal of the running frame aliased
    /// to one ELEMENT of an array (`st` bound to `s[3]`). The storage
    /// spelling `s[3]` is not a variable's name, so a member path through
    /// the formal cannot be rewritten onto it; the frame slot is refreshed
    /// from the element instead and the member is reached through the
    /// handle it holds.
    pub(super) fn ref_elem_handle_formal(&self, name: &str) -> bool {
        self.ref_alias_stack
            .last()
            .and_then(|m| m.get(name))
            .is_some_and(|storage| storage.ends_with(']'))
            && self.frame_class_handle(name).is_some()
    }

    /// §13.5.2: alias storage of one leaf of a `ref` unpacked struct or
    /// union formal: the member of the actual it copies (`caller_leaf`),
    /// resolved in the CALLER's scope. A member of a property of a live
    /// object is pinned through the object; a member of module-visible
    /// storage is named by its flattened name. None keeps the copy.
    pub(super) fn ref_leaf_alias_target(&mut self, caller_leaf: &Expression) -> Option<String> {
        if let ExprKind::MemberAccess { expr, member } = &caller_leaf.kind
            && let Some(ClassAggRef::Unpacked { handle, key, .. }) =
                self.class_agg_member_parts(expr, &member.name)
        {
            return (handle != 0).then(|| format!("{REF_PIN_PREFIX}{handle}.{key}"));
        }
        let root = Self::expr_root_first_seg(caller_leaf)?;
        // A struct of the caller's own frame (a local, or its own formal).
        if self
            .local_stack
            .last()
            .is_some_and(|f| f.contains_key(root) || f.contains_key(format!("{root}.").as_str()))
            || self
                .ref_binding_stack
                .last()
                .is_some_and(|m| m.contains_key(root))
            || self.this_stack.last().copied().flatten().is_some_and(|h| {
                self.heap.get(h).and_then(|o| o.as_ref()).is_some_and(|i| {
                    i.properties.keys().any(|k| {
                        k.strip_prefix(root)
                            .is_some_and(|r| r.is_empty() || r.starts_with('.'))
                    })
                })
            })
        {
            return None;
        }
        let flat = self.flat_member_name(caller_leaf)?;
        let exists = |sim: &Self, n: &str| {
            sim.signal_name_to_id.contains_key(n) || sim.signals.contains_key(n)
        };
        if exists(self, &flat) {
            return Some(flat);
        }
        self.hint_scoped(&flat).filter(|s| exists(self, s))
    }

    /// The aliased struct leaves of the running frame, `(frame key,
    /// storage)`; empty unless the innermost alias frame belongs to the
    /// innermost local frame.
    fn ref_frame_leaves(&self) -> Vec<(String, String)> {
        let Some(m) = self.ref_alias_stack.last() else {
            return Vec::new();
        };
        let Some(owner) = m.get(REF_LEAF_FRAME_KEY) else {
            return Vec::new();
        };
        if self
            .local_gen_stack
            .last()
            .is_none_or(|g| g.to_string() != *owner)
        {
            return Vec::new();
        }
        m.iter()
            .filter(|(k, _)| k.as_str() != REF_LEAF_FRAME_KEY && k.contains('.'))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// The alias-frame entry that names the local frame owning the aliased
    /// struct leaves beside it.
    pub(super) fn ref_leaf_frame_marker(&self) -> Option<(String, String)> {
        Some((
            REF_LEAF_FRAME_KEY.to_string(),
            self.local_gen_stack.last()?.to_string(),
        ))
    }

    /// The running process is about to leave its frame (it parks, or the
    /// call returns): write every aliased struct leaf whose frame copy
    /// differs to the actual. A leaf write goes through at once; this
    /// covers the writers that fill the frame directly (a whole-struct
    /// assignment to the formal).
    pub(super) fn ref_frame_flush(&mut self) {
        if !self.ref_redirect_possible() {
            return;
        }
        for (key, storage) in self.ref_frame_leaves() {
            let Some(v) = self.local_stack.last().and_then(|f| f.get(&key)).cloned() else {
                continue;
            };
            if self.ref_alias_peek(&storage).as_ref() != Some(&v) {
                self.ref_alias_write(&storage, &v);
            }
        }
    }

    /// The running process is back in its frame (it resumed, or a callee
    /// returned): the frame copies of the aliased struct leaves, and of the
    /// handle formals aliased to an element, take the actual's value, which
    /// another process may have written meanwhile.
    pub(super) fn ref_frame_refresh(&mut self) {
        if !self.ref_redirect_possible() {
            return;
        }
        for (key, storage) in self.ref_frame_leaves() {
            if let Some(v) = self.ref_alias_peek(&storage)
                && let Some(slot) = self.local_stack.last_mut().and_then(|f| f.get_mut(&key))
            {
                *slot = v;
            }
        }
        let handles: Vec<String> = self
            .ref_alias_stack
            .last()
            .map(|m| {
                m.iter()
                    .filter(|(_, s)| s.ends_with(']'))
                    .map(|(k, _)| k.clone())
                    .collect()
            })
            .unwrap_or_default();
        for name in handles {
            if self.frame_class_handle(&name).is_some() {
                self.ref_alias_refresh_slot(&name);
            }
        }
    }

    /// §7.8.7: an associative-array entry that does not exist is allocated
    /// when it is the actual of a `ref` argument, with the array's default
    /// or the element type's. Run in the caller's scope, before the element
    /// is frozen as the formal's storage.
    fn ref_alloc_assoc_actual(&mut self, arg: &Expression) {
        let ExprKind::Index { expr: base, index } = &arg.kind else {
            return;
        };
        let ExprKind::Ident(bh) = &base.kind else {
            return;
        };
        if bh.root.is_some() || bh.path.len() != 1 || !bh.path[0].selects.is_empty() {
            return;
        }
        let bname = bh.path[0].name.name.as_str();
        if !self.module.associative_arrays.contains_key(bname)
            || self
                .local_stack
                .last()
                .is_some_and(|f| f.contains_key(bname))
            || self
                .ref_binding_stack
                .last()
                .is_some_and(|m| m.contains_key(bname))
        {
            return;
        }
        let iv = self.eval_expr(index);
        if iv.has_xz() {
            return;
        }
        let cell = format!("{}[{}]", bname, self.assoc_key_str(bname, &iv));
        if self.signals.contains_key(&cell) || self.signal_name_to_id.contains_key(cell.as_str()) {
            return;
        }
        let v = self.eval_expr(arg);
        self.assign_value(arg, &v);
    }

    /// Current value of an aliased formal's storage, read without running
    /// any code (`&self` paths: a method receiver, a member write's handle).
    pub(super) fn ref_alias_peek(&self, storage: &str) -> Option<Value> {
        if let Some((class, prop)) = Self::ref_static_parts(storage) {
            let key = self.static_prop_key(class, prop)?;
            return self.class_statics.get(&key).cloned();
        }
        match Self::ref_pin_parts(storage) {
            Some((h, prop)) => self
                .heap
                .get(h)
                .and_then(|o| o.as_ref())
                .and_then(|inst| inst.properties.get(prop).cloned()),
            None => self.get_signal_value_by_name(storage),
        }
    }

    /// §13.5.2: read the storage an aliased `ref` formal names.
    pub(super) fn ref_alias_read(&mut self, storage: &str) -> Option<Value> {
        // A pinned instance property is read straight from its object.
        if Self::ref_pin_parts(storage).is_some() {
            return self.ref_alias_peek(storage);
        }
        match Self::ref_pin_hier(storage, crate::ast::Span::dummy()) {
            Some(h) => {
                let e = Expression::new(ExprKind::Ident(h), crate::ast::Span::dummy());
                Some(self.eval_expr(&e))
            }
            None => self.get_signal_value_by_name(storage),
        }
    }

    /// §13.5.2: write the storage an aliased `ref` formal names; true when
    /// the value changed.
    pub(super) fn ref_alias_write(&mut self, storage: &str, val: &Value) -> bool {
        if let Some((h, prop)) = Self::ref_pin_parts(storage) {
            let prop = prop.to_string();
            let changed = self.set_prop_if_changed(h, &prop, val.clone());
            // A process parked on the property (`wait (h.p == v)`) rechecks.
            if changed
                && !self.condition_waiters.is_empty()
                && let Some(hier) = Self::ref_pin_hier(storage, crate::ast::Span::dummy())
            {
                let pin = Expression::new(
                    ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                        path: hier.path[..1].to_vec(),
                        ..hier.clone()
                    }),
                    crate::ast::Span::dummy(),
                );
                let lhs = Expression::new(
                    ExprKind::MemberAccess {
                        expr: Box::new(pin),
                        member: hier.path[1].name.clone(),
                    },
                    crate::ast::Span::dummy(),
                );
                self.check_condition_waiters_for_write(&lhs);
            }
            return changed;
        }
        if let Some(hier) = Self::ref_static_parts(storage)
            .and_then(|_| Self::ref_pin_hier(storage, crate::ast::Span::dummy()))
        {
            let prev = self.ref_alias_peek(storage);
            let lhs = Expression::new(ExprKind::Ident(hier), crate::ast::Span::dummy());
            self.assign_value(&lhs, val);
            return prev != self.ref_alias_peek(storage);
        }
        let prev = self.get_signal_value_by_name(storage);
        self.set_signal_value_by_name(storage, val.clone());
        prev.as_ref() != self.get_signal_value_by_name(storage).as_ref()
    }

    /// `event` rewritten onto the actual when one of its terms is a `ref`
    /// formal aliased to a pinned property (the class-property waits take
    /// it from there; a signal sensitivity has nothing to watch).
    pub(super) fn ref_event_on_pinned_prop(&self, event: &EventControl) -> Option<EventControl> {
        if !self.ref_redirect_possible() {
            return None;
        }
        let ev = self.ref_event_control(event)?;
        let pinned = |e: &Expression| {
            let mut e = e;
            loop {
                match &e.kind {
                    ExprKind::Paren(x)
                    | ExprKind::Index { expr: x, .. }
                    | ExprKind::RangeSelect { expr: x, .. }
                    | ExprKind::MemberAccess { expr: x, .. } => e = x,
                    ExprKind::Ident(h) => {
                        return h
                            .path
                            .first()
                            .is_some_and(|s| s.name.name.starts_with(REF_PIN_PREFIX));
                    }
                    _ => return false,
                }
            }
        };
        let any = match &ev {
            EventControl::HierIdentifier(e) => pinned(e),
            EventControl::EventExpr(terms) => terms.iter().any(|t| pinned(&t.expr)),
            _ => false,
        };
        any.then_some(ev)
    }

    /// The current value of the storage the `ref` formal `base` is aliased
    /// to (a packed struct's member is read and written through it).
    pub(super) fn ref_alias_base_value(&self, base: &str) -> Option<Value> {
        if !self.ref_redirect_possible() {
            return None;
        }
        let storage = self.ref_alias_stack.last()?.get(base)?;
        if storage == base {
            return None;
        }
        self.ref_alias_peek(storage)
    }

    /// Write `val` through the `ref` formal `base` to its actual, refreshing
    /// the frame copy.
    pub(super) fn ref_alias_store_base(&mut self, base: &str, val: &Value) {
        let Some(storage) = self
            .ref_alias_stack
            .last()
            .and_then(|m| m.get(base))
            .cloned()
        else {
            return;
        };
        if let Some(slot) = self.local_stack.last_mut().and_then(|f| f.get_mut(base)) {
            *slot = val.clone();
        }
        self.ref_alias_write(&storage, val);
    }

    /// Refresh the frame copy of the aliased `ref` formal `name` from the
    /// storage it names, for the by-name paths that read the frame.
    pub(super) fn ref_alias_refresh_slot(&mut self, name: &str) {
        let Some(storage) = self
            .ref_alias_stack
            .last()
            .and_then(|m| m.get(name))
            .filter(|s| s.as_str() != name)
            .cloned()
        else {
            return;
        };
        if !self
            .local_stack
            .last()
            .is_some_and(|f| f.contains_key(name))
        {
            return;
        }
        if let Some(v) = self.ref_alias_read(&storage)
            && let Some(slot) = self.local_stack.last_mut().and_then(|f| f.get_mut(name))
        {
            *slot = v;
        }
    }

    /// §13.5.2: alias storage for a `ref` actual that `ref_alias_target`
    /// cannot freeze by itself, resolved in the CALLER's scope (before the
    /// callee's frame exists): a property of a live object (`h.p`,
    /// `o.in.p`, `this.p`, a bare property of `this`), and a hierarchical or
    /// struct-member name of module-visible storage (`u_sub.v`, `s.f`).
    /// Formals with unpacked dimensions or an unpacked aggregate type keep
    /// their copy-in/copy-out machinery.
    pub(super) fn ref_scope_alias_target(
        &mut self,
        port: &crate::ast::decl::FunctionPort,
        arg: &Expression,
    ) -> Option<String> {
        if !port.dimensions.is_empty() || self.ref_formal_is_unpacked_aggregate(&port.data_type) {
            return None;
        }
        let mut arg = arg;
        while let ExprKind::Paren(inner) = &arg.kind {
            arg = inner;
        }
        if !self.module.associative_arrays.is_empty() {
            self.ref_alloc_assoc_actual(arg);
        }
        let (handle, prop): (usize, &str) = match &arg.kind {
            ExprKind::MemberAccess { expr, member } => match self.eval_handle_expr(expr) {
                Some(h) => (h, member.name.as_str()),
                // A member of an element of an instance array
                // (`u_ifa[1].iv`) parses as a member access too.
                None => {
                    return self.ref_leaf_alias_target(arg).filter(|flat| {
                        !(self.module.arrays.contains_key(flat.as_str())
                            || self.module.dynamic_arrays.contains(flat)
                            || self.module.queue_vars.contains(flat)
                            || self.module.associative_arrays.contains_key(flat))
                    });
                }
            },
            ExprKind::Ident(h)
                if h.root.is_none()
                    && h.path.len() >= 2
                    && h.path.last().is_some_and(|s| s.selects.is_empty()) =>
            {
                if h.path.iter().any(|s| !s.selects.is_empty()) {
                    return self.ref_hier_alias_target(h);
                }
                // `C::p`: a static property named through its class.
                if h.path.len() == 2
                    && self.module.classes.contains_key(&h.path[0].name.name)
                    && !self
                        .local_stack
                        .last()
                        .is_some_and(|f| f.contains_key(&h.path[0].name.name))
                {
                    let class = h.path[0].name.name.clone();
                    return self.ref_static_alias_target(&class, &h.path[1].name.name);
                }
                let prefix = Expression::new(ExprKind::Ident(hier_prefix(h)), arg.span);
                match self.eval_handle_expr(&prefix).filter(|&x| x != 0) {
                    Some(x) => (x, h.path.last()?.name.name.as_str()),
                    None => return self.ref_hier_alias_target(h),
                }
            }
            ExprKind::Ident(h)
                if h.root.is_none() && h.path.len() == 1 && h.path[0].selects.is_empty() =>
            {
                let name = h.path[0].name.name.as_str();
                // A local or formal of the caller (chained refs included) is
                // `ref_alias_target`'s to decide.
                if self
                    .local_stack
                    .last()
                    .is_some_and(|f| f.contains_key(name))
                    || self
                        .ref_binding_stack
                        .last()
                        .is_some_and(|m| m.contains_key(name))
                {
                    return None;
                }
                (self.this_stack.last().copied().flatten()?, name)
            }
            _ => return None,
        };
        if handle == 0 {
            return None;
        }
        let inst = self.heap.get(handle).and_then(|o| o.as_ref())?;
        // A name some class of the chain redeclares, or whose nearest
        // declaration is a constant, keeps the copy path (see
        // `ref_name_redeclared`).
        if self.ref_name_redeclared(&inst.class_name, prop)
            || self.nearest_decl_is_constant(&inst.class_name, prop)
        {
            return None;
        }
        if self.member_is_static_prop(&inst.class_name, prop) {
            let class = inst.class_name.clone();
            return self.ref_static_alias_target(&class, prop);
        }
        if !inst.properties.contains_key(prop)
            || (self.shadowed_prop_mask != 0 && self.prop_maybe_shadowed(prop))
        {
            return None;
        }
        Some(format!("{REF_PIN_PREFIX}{handle}.{prop}"))
    }

    /// Whether more than one class of `class`'s chain declares `name` (a
    /// property, or a constant hiding one, §8.14): which one the actual means
    /// depends on the reference's declared type, so it keeps the copy path.
    fn ref_name_redeclared(&self, class: &str, name: &str) -> bool {
        let mut seen = 0;
        let mut cur = Some(class);
        while let Some(c) = cur {
            let Some(cd) = self.module.classes.get(c) else {
                break;
            };
            if cd.properties.contains_key(name)
                || cd.property_types.contains_key(name)
                || cd.param_defaults.iter().any(|(n, _)| n == name)
            {
                seen += 1;
            }
            cur = cd.extends.as_deref();
        }
        seen > 1
    }

    /// The storage name of the static property `prop` of `class` (or of the
    /// ancestor declaring it); None for a parameterized class, whose statics
    /// are per specialization.
    fn ref_static_alias_target(&self, class: &str, prop: &str) -> Option<String> {
        let mut cur = Some(class.to_string());
        while let Some(c) = cur {
            let cd = self.module.classes.get(&c)?;
            if !cd.param_order.is_empty()
                || !cd.type_param_names.is_empty()
                || cd.param_defaults.iter().any(|(n, _)| n == prop)
            {
                return None;
            }
            if cd.static_properties.contains(prop) {
                self.static_prop_key(&c, prop)?;
                return Some(format!("{REF_STATIC_PREFIX}{c}::{prop}"));
            }
            cur = cd.extends.clone();
        }
        None
    }

    /// A hierarchical or dotted actual (`u_sub.v`, `pkg::v`, `s.f`) whose
    /// flattened name is module-visible scalar storage.
    fn ref_hier_alias_target(
        &mut self,
        h: &crate::ast::expr::HierarchicalIdentifier,
    ) -> Option<String> {
        let head = h.path[0].name.name.as_str();
        // A struct local of the caller lives in its frame.
        if self
            .local_stack
            .last()
            .is_some_and(|f| f.contains_key(head) || f.contains_key(format!("{head}.").as_str()))
            || self
                .ref_binding_stack
                .last()
                .is_some_and(|m| m.contains_key(head))
        {
            return None;
        }
        let flat = self.resolve_hier_name(h).into_owned();
        if self.module.arrays.contains_key(flat.as_str())
            || self.module.dynamic_arrays.contains(&flat)
            || self.module.queue_vars.contains(&flat)
            || self.module.associative_arrays.contains_key(&flat)
        {
            return None;
        }
        (self.signal_name_to_id.contains_key(flat.as_str()) || self.signals.contains_key(&flat))
            .then_some(flat)
    }

    /// Whether a formal's declared type is an unpacked struct or union, which
    /// binds member-wise into the callee's frame.
    fn ref_formal_is_unpacked_aggregate(&self, dt: &crate::ast::types::DataType) -> bool {
        use crate::ast::types::DataType;
        let mut cur = dt;
        for _ in 0..8 {
            match cur {
                // An untagged union is one piece of storage (§7.3).
                DataType::Struct(s) => return Self::spreads_member_wise(s),
                DataType::TypeReference {
                    name, dimensions, ..
                } => {
                    if !dimensions.is_empty() {
                        return true;
                    }
                    match self.module.typedef_types.get(&name.name.name) {
                        Some(next) => cur = next,
                        None => return false,
                    }
                }
                _ => return false,
            }
        }
        false
    }

    /// Create (or count one more user of) the pin of every pinned storage in
    /// a callee's alias frame.
    pub(super) fn ref_pins_acquire(&mut self, frame: &HashMap<String, String>) {
        for storage in frame.values() {
            if let Some((h, prop)) = Self::ref_pin_parts(storage) {
                let pin = storage[..storage.len() - prop.len() - 1].to_string();
                let n = self.ref_pin_users.entry(h).or_insert(0);
                *n += 1;
                if *n == 1 {
                    self.signals.insert(pin, Value::from_u64(h as u64, 32));
                }
            }
        }
    }

    /// Drop one user of every pin a popped alias frame named; the last user
    /// removes the pin, so it does not keep its object alive.
    pub(super) fn ref_pins_release(&mut self, frame: &HashMap<String, String>) {
        for storage in frame.values() {
            if let Some((h, prop)) = Self::ref_pin_parts(storage) {
                let pin = storage[..storage.len() - prop.len() - 1].to_string();
                if let Some(n) = self.ref_pin_users.get_mut(&h) {
                    *n -= 1;
                    if *n == 0 {
                        self.ref_pin_users.remove(&h);
                        self.signals.remove(&pin);
                    }
                }
            }
        }
    }
}
