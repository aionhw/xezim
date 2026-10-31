//! Simulator named events.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    /// §15.5.5 null event handle sentinel (`ev = null`).
    pub(super) const EVENT_NULL_KEY: &'static str = "\x01evnull";

    /// Chase the §15.5.5 alias map to the canonical synchronization-object
    /// key for an event name (bounded, alias chains are user-built).
    pub(super) fn resolve_event_key(&self, name: &str) -> String {
        Self::resolve_event_key_ref(&self.event_aliases, name).to_string()
    }

    pub(super) fn resolve_event_key_ref<'a>(
        aliases: &'a HashMap<String, String>,
        name: &'a str,
    ) -> &'a str {
        let mut cur = name;
        for _ in 0..16 {
            match aliases.get(cur) {
                Some(next) => cur = next,
                None => break,
            }
        }
        cur
    }

    /// A bare `-> e` / `->> e` inside a subroutine dispatched from ANOTHER
    /// scope (§23.4: `mm.fire_m()`, `pif.fire_i()`) must fire the event of
    /// the subroutine's LEXICAL home, not the caller's same-named event.
    /// The hierarchical-call dispatcher parks the callee's home scope in
    /// `name_resolve_hint`; return the hint-qualified event name when that
    /// event exists, else the raw name (same-module calls and already-
    /// hierarchical names keep their existing resolution).
    pub(super) fn lexical_event_name(&self, raw: &str) -> String {
        if raw.contains('.') {
            return raw.to_string();
        }
        if let Some(q) = self.modclass_scoped(raw)
            && self.signal_name_to_id.contains_key(q.as_str())
        {
            return q;
        }
        if self.contested_pkg_vars.contains(raw) {
            let qualified = match self.pkg_scope_stack.last().and_then(|p| p.as_deref()) {
                Some(pkg) => self.pkg_var_storage(pkg, raw),
                None if !self.module.events.contains(raw) => self.contested_pkg_var_in_scope(raw),
                None => None,
            };
            if let Some(key) = qualified.filter(|key| self.module.events.contains(key)) {
                return key;
            }
        }
        let hint = self.name_resolve_hint.borrow().clone();
        if let Some(hint) = hint.filter(|h| !h.is_empty()) {
            for q in [
                format!("{}.{}", hint, raw),
                format!("{}.{}.{}", self.module.name, hint, raw),
            ] {
                if self.signal_name_to_id.contains_key(&q) {
                    return q;
                }
            }
        }
        raw.to_string()
    }

    /// Evaluate a hierarchical expression or member chain into a dotted/indexed
    /// signal name string (e.g. `u.ev`, `ua[0].ev`, `m.l.ev`, `vif.ev`).
    pub(super) fn hier_expr_to_string<'a>(
        &'a self,
        e: &'a Expression,
    ) -> Option<std::borrow::Cow<'a, str>> {
        use std::borrow::Cow;
        match &e.kind {
            ExprKind::Ident(h) => {
                if h.path.is_empty() {
                    return None;
                }
                let first = h.path[0].name.name.as_str();
                let head = if let Some(bound) = self.iface_alias_for(first) {
                    Cow::Owned(bound)
                } else if first == "this" {
                    if let Some(Some(this_h)) = self.this_stack.last() {
                        if h.path.len() >= 2 {
                            let member = &h.path[1].name.name;
                            if let Some((bound, _)) =
                                self.virtual_iface_bindings.get(&(*this_h, member.clone()))
                            {
                                let mut out = bound.clone();
                                for seg in &h.path[2..] {
                                    out.push('.');
                                    out.push_str(&seg.name.name);
                                    for sel in &seg.selects {
                                        let i = self.eval_scalar_self(sel)?;
                                        use std::fmt::Write as _;
                                        let _ = write!(out, "[{}]", i);
                                    }
                                }
                                return Some(Cow::Owned(out));
                            }
                        }
                    }
                    Cow::Borrowed(first)
                } else if let Some(Some(this_h)) = self.this_stack.last() {
                    if let Some((bound, _)) = self
                        .virtual_iface_bindings
                        .get(&(*this_h, first.to_string()))
                    {
                        Cow::Borrowed(bound.as_str())
                    } else {
                        Cow::Borrowed(first)
                    }
                } else {
                    Cow::Borrowed(first)
                };
                // Most probes are ordinary scalar names that are not events.
                // Borrow them; an actual event result becomes owned below.
                if h.path.len() == 1 && h.path[0].selects.is_empty() {
                    return Some(head);
                }
                let mut out = head.into_owned();
                for sel in &h.path[0].selects {
                    let i = self.eval_scalar_self(sel)?;
                    use std::fmt::Write as _;
                    let _ = write!(out, "[{}]", i);
                }
                for seg in &h.path[1..] {
                    out.push('.');
                    out.push_str(&seg.name.name);
                    for sel in &seg.selects {
                        let i = self.eval_scalar_self(sel)?;
                        use std::fmt::Write as _;
                        let _ = write!(out, "[{}]", i);
                    }
                }
                Some(Cow::Owned(out))
            }
            ExprKind::MemberAccess { expr, member } => {
                if let ExprKind::Ident(h) = &expr.kind {
                    if h.path.len() == 1 && h.path[0].name.name == "this" {
                        if let Some(Some(this_h)) = self.this_stack.last() {
                            if let Some((bound, _)) = self
                                .virtual_iface_bindings
                                .get(&(*this_h, member.name.clone()))
                            {
                                return Some(Cow::Borrowed(bound.as_str()));
                            }
                        }
                    }
                }
                let mut base = self.hier_expr_to_string(expr)?;
                base.to_mut().push('.');
                base.to_mut().push_str(&member.name);
                Some(base)
            }
            ExprKind::Index { expr, index } => {
                let mut base = self.hier_expr_to_string(expr)?;
                let i = self.eval_scalar_self(index)?;
                use std::fmt::Write as _;
                let _ = write!(base.to_mut(), "[{}]", i);
                Some(base)
            }
            ExprKind::Paren(inner) => self.hier_expr_to_string(inner),
            _ => None,
        }
    }

    /// If `e` is a plain reference to an event variable (optionally with a
    /// constant/variable element select: `ev`, `ev_arr[i]`, `u.ev`, `ua[0].ev`),
    /// return its storage key (`ev` / `ev_arr[3]` / `u.ev`). Detection is by the
    /// declared-events set, so ordinary signals never take this path.
    pub(super) fn event_ref_key(&mut self, e: &Expression) -> Option<String> {
        // §15.5: a selected event is captured at arm time. Only evaluate
        // runtime selectors after the base is known to be an event, so
        // probing an ordinary expression cannot duplicate its side effects.
        if let ExprKind::Index { expr, index } = &e.kind {
            if let Some(base) = self.event_ref_key(expr) {
                let index = self.eval_expr(index).to_i64()?;
                return Some(format!("{base}[{index}]"));
            }
        }
        if let ExprKind::Ident(h) = &e.kind {
            if h.path.last().is_some_and(|p| !p.selects.is_empty()) {
                let mut base = h.clone();
                let selects = std::mem::take(&mut base.path.last_mut()?.selects);
                let base_expr = Expression::new(ExprKind::Ident(base), e.span);
                if let Some(mut key) = self.event_ref_key(&base_expr) {
                    for select in selects {
                        let index = self.eval_expr(&select).to_i64()?;
                        use std::fmt::Write as _;
                        let _ = write!(key, "[{index}]");
                    }
                    return Some(key);
                }
            }
        }
        self.event_ref_key_static(e)
    }

    /// Read-only classification for sensitivity construction. Runtime
    /// selectors are evaluated only by the process arming the wait.
    pub(super) fn event_ref_key_static(&self, e: &Expression) -> Option<String> {
        let name = self.hier_expr_to_string(e)?;
        if let Some(key) = self.package_event_key(&name) {
            return Some(key);
        }
        let declared = |key: &str| {
            self.module.events.contains(key)
                || (key.ends_with(']')
                    && key
                        .rfind('[')
                        .is_some_and(|i| self.module.events.contains(&key[..i])))
        };
        // §8/§23.9: in a method of a class declared inside a module, the
        // object's instance declares the event.
        if let Some(scoped) = self.modclass_scoped(&name)
            && declared(&scoped)
        {
            return Some(scoped);
        }
        // §26.3: package subroutines retain their declaration scope even
        // when the calling module declares a same-named event.
        if !self.contested_pkg_vars.is_empty() && self.contested_pkg_vars.contains(name.as_ref()) {
            let lexical = self.lexical_event_name(&name);
            if lexical != name && declared(&lexical) {
                return Some(lexical);
            }
        }
        if declared(&name) {
            return Some(name.into_owned());
        }
        if let Some(rest) = name
            .strip_prefix(self.module.name.as_str())
            .and_then(|rest| rest.strip_prefix('.'))
        {
            if declared(rest) {
                return Some(rest.to_string());
            }
        }
        if let Some(hint) = self.name_resolve_hint.borrow().as_ref() {
            let scoped = format!("{}.{}", hint, name);
            if declared(&scoped) {
                return Some(scoped);
            }
        }
        if self.contested_pkg_vars.contains(name.as_ref())
            && let ExprKind::Ident(h) = &e.kind
        {
            let resolved = self.resolve_hier_name(h);
            if declared(&resolved) {
                return Some(resolved.into_owned());
            }
        }
        let canon = Self::resolve_event_key_ref(&self.event_aliases, name.as_ref());
        if declared(canon) {
            return Some(canon.to_string());
        }
        None
    }

    /// §26.3: package spelling to the package event's actual storage cell.
    pub(super) fn package_event_key(&self, name: &str) -> Option<String> {
        let (pkg, leaf) = name.split_once("::").or_else(|| name.split_once('.'))?;
        if leaf.contains('.') || leaf.contains("::") || !self.module.packages.contains(pkg) {
            return None;
        }
        let qualified = format!("{pkg}::{leaf}");
        if !self.module.events.contains(&qualified)
            || !self.module.pkg_var_decls.contains_key(&qualified)
        {
            return None;
        }
        let key = self
            .pkg_var_storage(pkg, leaf)
            .unwrap_or_else(|| leaf.to_string());
        self.signal_name_to_id
            .contains_key(key.as_str())
            .then_some(key)
    }

    /// §15.5.5 handle-assignment interception for `lhs = rhs` where `lhs`
    /// is an event variable. Returns true when the statement was consumed
    /// as an alias/null re-binding (the caller must then skip the normal
    /// value assignment).
    pub(super) fn try_event_handle_assign(
        &mut self,
        lvalue: &Expression,
        rvalue: &Expression,
    ) -> bool {
        let Some(lkey) = self.event_ref_key(lvalue) else {
            return false;
        };
        if matches!(rvalue.kind, ExprKind::Null) {
            self.event_aliases
                .insert(lkey, Self::EVENT_NULL_KEY.to_string());
            return true;
        }
        if let Some(rkey) = self.event_ref_key(rvalue) {
            let target = self.resolve_event_key(&rkey);
            if target == lkey {
                self.event_aliases.remove(&lkey);
            } else {
                self.event_aliases.insert(lkey, target);
            }
            return true;
        }
        // A handle pulled out of a container (`e = q[0]`): the container
        // stores the sync-object key as a string (see queue_store_elem).
        let v = self.eval_expr(rvalue);
        let sv = v.to_sv_string();
        if !sv.is_empty()
            && (self.module.events.contains(sv.as_str())
                || self.event_aliases.contains_key(sv.as_str())
                || self.event_triggered_time.contains_key(sv.as_str()))
        {
            let target = self.resolve_event_key(&sv);
            self.event_aliases.insert(lkey, target);
            return true;
        }
        false
    }

    /// Truth of a `wait` condition. A bare named-event operand (`wait(ev)`) is
    /// true only in the time slot the event was triggered, like
    /// `ev.triggered` (§15.5.3) — the reference simulator waits for each new
    /// trigger. The event's stored value stays set after its first trigger, so
    /// every later `wait(ev)` used to fall straight through.
    pub(super) fn wait_condition_true(&mut self, cond: &Expression) -> bool {
        if matches!(cond.kind, ExprKind::Ident(_))
            && (!self.module.events.is_empty() || !self.event_aliases.is_empty())
        {
            if let Some(t) = self.event_triggered_now(cond) {
                return t;
            }
        }
        self.eval_expr(cond).is_true()
    }

    /// §15.5.3 `.triggered` truth for an event reference (aliases chased,
    /// element keys and container-stored string handles included). None if
    /// `e` is not recognizably an event reference.
    pub(super) fn event_triggered_now(&mut self, e: &Expression) -> Option<bool> {
        let now = self.time;
        // `e.triggered` on a subroutine's local `event e;`: its own
        // activation's stamp (see `fire_named_event`), not the name's.
        if let ExprKind::Ident(h) = &e.kind
            && h.path.len() == 1
            && h.path[0].selects.is_empty()
        {
            let n = h.path[0].name.name.as_str();
            if self.module.events.contains(n)
                && self.local_stack.iter().any(|f| f.contains_key(n))
                && !self
                    .signal_name_to_id
                    .contains_key(self.resolve_event_key(n).as_str())
            {
                let k = self.unbacked_event_key(n);
                return Some(self.event_triggered_time.get(k.1.as_str()) == Some(&now));
            }
        }
        // `h.evs[i].triggered` — an element of a class event array.
        if let Some(k) = self.expr_instance_event_elem(e) {
            let stamp = Self::instance_event_stamp_key(&k);
            return Some(self.event_triggered_time.get(stamp.as_str()) == Some(&now));
        }
        // `h.ce.triggered` / `this.ce.triggered` — a per-instance class event.
        if let Some(k) = self.expr_handle_event_field(e) {
            let stamp = Self::instance_event_stamp_key(&k);
            return Some(self.event_triggered_time.get(stamp.as_str()) == Some(&now));
        }
        // A bare `ce.triggered` in a method of its class is `this.ce`, the
        // field `-> ce` stamps; looked up as a module event it never fired.
        if let ExprKind::Ident(h) = &e.kind
            && h.path.len() == 1
            && h.path[0].selects.is_empty()
            && !self
                .local_stack
                .last()
                .is_some_and(|l| l.contains_key(h.path[0].name.name.as_str()))
            && let Some(k) = self.resolve_this_event_field(&h.path[0].name.name)
        {
            let stamp = Self::instance_event_stamp_key(&k);
            return Some(self.event_triggered_time.get(stamp.as_str()) == Some(&now));
        }
        // `ev.triggered` / `C::ev.triggered` on a static class event.
        if let Some(k) = self.static_class_event_key(e) {
            let stamp = Self::instance_event_stamp_key(&k);
            return Some(self.event_triggered_time.get(stamp.as_str()) == Some(&now));
        }
        let mut keys: Vec<String> = Vec::new();
        if let Some(k) = self.event_ref_key(e) {
            keys.push(k);
        } else if let ExprKind::Ident(h) = &e.kind {
            if let Some(seg) = h.path.last() {
                if seg.selects.is_empty() {
                    keys.push(seg.name.name.clone());
                }
            }
            keys.push(self.resolve_hier_name(h).into_owned());
        }
        // Container element (`q[0].triggered`): the element VALUE is the key.
        if let ExprKind::Ident(h) = &e.kind {
            if h.path.len() == 1 && !h.path[0].selects.is_empty() {
                let v = self.eval_expr(e);
                let sv = v.to_sv_string();
                if !sv.is_empty()
                    && (self.module.events.contains(sv.as_str())
                        || self.event_aliases.contains_key(sv.as_str())
                        || self.event_triggered_time.contains_key(sv.as_str()))
                {
                    keys.push(sv);
                }
            }
        }
        if keys.is_empty() {
            return None;
        }
        let mut known_event = false;
        for k in &keys {
            let canon = self.resolve_event_key(k);
            if canon == Self::EVENT_NULL_KEY {
                known_event = true;
                continue;
            }
            if self.event_triggered_time.get(canon.as_str()) == Some(&now) {
                return Some(true);
            }
            if self.module.events.contains(canon.as_str())
                || self.event_triggered_time.contains_key(canon.as_str())
            {
                known_event = true;
            }
        }
        if known_event { Some(false) } else { None }
    }

    /// If `name` (used in `->name` / `@name`) denotes a CLASS-FIELD event
    /// of the current `this`, return the per-instance synchronization key
    /// `(this_handle, field_name)`. Returns `None` when execution is not
    /// inside a class method (`this` unset) or when `name` is not a property
    /// of the current instance — in which case the caller falls back to the
    /// module-scope named-event path (`signal_table`-backed). Per SV §8.16 a
    /// class member name shadows a same-named module signal inside a method,
    /// so the this-relative field wins when present.
    /// §6.21: does a local of the running method (`event e;` in a task)
    /// shadow the class member `name`? Only the method's own frames count,
    /// not its callers'.
    pub(super) fn method_local_shadows(&self, name: &str) -> bool {
        let base = self.method_local_base.last().copied().unwrap_or(0);
        self.local_stack
            .get(base..)
            .is_some_and(|frames| frames.iter().any(|f| f.contains_key(name)))
    }

    pub(super) fn resolve_this_event_field(&self, name: &str) -> Option<(usize, String)> {
        let handle = self.this_stack.last().copied().flatten()?;
        if self.method_local_shadows(name) {
            return None;
        }
        let inst = self.heap.get(handle).and_then(|o| o.as_ref())?;
        if inst.properties.contains_key(name) && self.class_prop_is_event(handle, name) {
            Some((handle, name.to_string()))
        } else {
            None
        }
    }

    /// §8.9, §15.5: a STATIC class property declared `event` — `ev` inside a
    /// method of its class, or `C::ev` anywhere. It is one object shared by
    /// every instance, so it is keyed like an instance event with handle 0
    /// and the property's static cell name. It has no backing signal: a wait
    /// on it fell through to the NBA-region yield and woke at once.
    pub(super) fn static_class_event_key(&self, e: &Expression) -> Option<(usize, String)> {
        // §8.25 / §15.5: specialized triggers and waits use the same cell.
        if let Some(spec) =
            self.resolve_call_spec_params(Self::extract_call_spec(e), &self.current_spec)
            && let ExprKind::MemberAccess { member, .. } = &e.kind
        {
            return self.static_class_event_key_named_spec(&spec.0, &member.name, Some(&spec));
        }
        let (cls, field): (String, &str) = match &e.kind {
            ExprKind::Ident(h) if h.path.iter().all(|s| s.selects.is_empty()) => {
                match h.path.as_slice() {
                    [one] => (
                        self.class_context_stack.last().cloned().flatten()?,
                        one.name.name.as_str(),
                    ),
                    [c, f] => (c.name.name.clone(), f.name.name.as_str()),
                    _ => return None,
                }
            }
            ExprKind::MemberAccess { expr, member } => match &expr.kind {
                ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                    (h.path[0].name.name.clone(), member.name.as_str())
                }
                _ => return None,
            },
            _ => return None,
        };
        if self.module.classes.contains_key(&cls) {
            self.static_class_event_key_named(&cls, field)
        } else {
            let (base, sig) = self.resolve_class_scope_prefix(&cls)?;
            let spec = sig.map(|sig| (base.clone(), sig));
            self.static_class_event_key_named_spec(&base, field, spec.as_ref())
        }
    }

    pub(super) fn static_class_event_key_named(
        &self,
        cls: &str,
        field: &str,
    ) -> Option<(usize, String)> {
        self.static_class_event_key_named_spec(cls, field, None)
    }

    pub(super) fn static_class_event_key_named_spec(
        &self,
        cls: &str,
        field: &str,
        spec: Option<&(String, String)>,
    ) -> Option<(usize, String)> {
        if !self.class_member_names().static_or_param.contains(field) {
            return None;
        }
        let mut cur = Some(cls.to_string());
        let mut seen: HashSet<String> = HashSet::default();
        while let Some(cn) = cur {
            if !seen.insert(cn.clone()) {
                break;
            }
            let cd = self.module.classes.get(&cn)?;
            if let Some(dt) = cd.property_types.get(field) {
                if !cd.static_properties.contains(field)
                    || !matches!(
                        dt,
                        DataType::Simple {
                            kind: crate::ast::types::SimpleType::Event,
                            ..
                        }
                    )
                {
                    return None;
                }
                return self.static_prop_key_spec(cls, field, spec).map(|k| (0, k));
            }
            cur = cd.extends.clone();
        }
        None
    }

    /// The static class event a `->` names, from its flattened name
    /// (`ev`, `C::ev`) or its target expression.
    pub(super) fn static_class_event_trigger_key(
        &self,
        name: &str,
        target: Option<&Expression>,
    ) -> Option<(usize, String)> {
        if let Some(k) = target.and_then(|t| self.static_class_event_key(t)) {
            return Some(k);
        }
        match name.rsplit_once("::") {
            Some((c, f)) => self.static_class_event_key_named(c, f).or_else(|| {
                let leaf = c.rsplit("::").next()?;
                self.static_class_event_key_named(leaf, f)
            }),
            None if !name.contains('.') => {
                let cls = self.class_context_stack.last().cloned().flatten()?;
                self.static_class_event_key_named(&cls, name)
            }
            None => None,
        }
    }

    /// Is `field` of the class object `handle` declared `event` (searching
    /// the `extends` chain)? A property with no recorded type is assumed to
    /// be an event, as the callers always have been. A plain variable
    /// (`bit on;`) is NOT: `@on` on it is a value-change wait, never woken
    /// by `->` (see `class_prop_value_change_cond`).
    pub(super) fn class_prop_is_event(&self, handle: usize, field: &str) -> bool {
        let Some(inst) = self.heap.get(handle).and_then(|o| o.as_ref()) else {
            return false;
        };
        let mut cur = Some(inst.class_name.clone());
        let mut seen: HashSet<String> = HashSet::default();
        while let Some(cn) = cur {
            if !seen.insert(cn.clone()) {
                break;
            }
            let Some(cd) = self.module.classes.get(&cn) else {
                break;
            };
            if let Some(dt) = cd.property_types.get(field) {
                return matches!(
                    dt,
                    DataType::Simple {
                        kind: crate::ast::types::SimpleType::Event,
                        ..
                    }
                );
            }
            cur = cd.extends.clone();
        }
        true
    }

    /// `@prop` / `@(h.prop)` on a NON-event class property (§9.4.2: any
    /// change of the expression value triggers). Returns a condition that
    /// becomes true when the property differs from its value right now, to
    /// be parked as a `wait` — the same wake path `wait(prop)` uses, driven
    /// by ordinary property writes. `None` when the control is anything
    /// else (an event property, a signal, an edge-qualified term, ...).
    pub(super) fn class_prop_value_change_cond(
        &mut self,
        event: &EventControl,
    ) -> Option<Expression> {
        let exprs = if let EventControl::EventExpr(terms) = event
            && terms.len() > 1
        {
            if terms.iter().any(|t| t.edge.is_some() || t.iff.is_some()) {
                return None;
            }
            // Validate all terms before allocating any saved values. Each
            // field needs its own snapshot: a change to a can leave (a | b)
            // unchanged, but must still trigger @(a or b).
            terms
                .iter()
                .map(|term| {
                    self.class_prop_value_change_expr(&EventControl::EventExpr(vec![term.clone()]))
                })
                .collect::<Option<Vec<_>>>()?
        } else {
            vec![self.class_prop_value_change_expr(event)?]
        };
        let mut conds = exprs.into_iter().map(|expr| self.value_change_cond(expr));
        let first = conds.next()?;
        Some(conds.fold(first, |left, right| {
            let span = left.span;
            Expression::new(
                ExprKind::Binary {
                    op: BinaryOp::LogOr,
                    left: Box::new(left),
                    right: Box::new(right),
                },
                span,
            )
        }))
    }

    fn class_prop_value_change_expr(&mut self, event: &EventControl) -> Option<Expression> {
        let expr = Self::event_control_single_expr(event)?;
        let on_class_prop = if let Some(fname) = self.event_control_field_name(event)
            && !self.method_local_shadows(&fname)
        {
            self.this_stack
                .last()
                .copied()
                .flatten()
                .filter(|&h| !self.class_prop_is_event(h, &fname))
                .and_then(|h| self.heap.get(h).and_then(|o| o.as_ref()))
                .is_some_and(|inst| inst.properties.contains_key(fname.as_str()))
        } else {
            false
        } || self
            .expr_handle_prop_any(&expr)
            .is_some_and(|(h, f)| !self.class_prop_is_event(h, &f));
        if !on_class_prop {
            return None;
        }
        Some(expr)
    }

    pub(super) fn value_change_cond(&mut self, expr: Expression) -> Expression {
        let val = self.eval_expr(&expr);
        let idx = self.intra_saved_next;
        self.intra_saved_next += 1;
        self.vc_saved.insert(idx, (self.current_pid, val));
        let span = expr.span;
        Expression::new(
            ExprKind::SystemCall {
                name: VC_CHANGED_MARKER.to_string(),
                args: vec![
                    Expression::new(
                        ExprKind::Number(NumberLiteral::Integer {
                            size: None,
                            signed: false,
                            base: NumberBase::Decimal,
                            value: idx.to_string(),
                            cached_val: Cell::new(None),
                        }),
                        span,
                    ),
                    expr,
                ],
            },
            span,
        )
    }

    /// Drop the armed value of a resumed value-change wait.
    pub(super) fn vc_release(&mut self, cond: &Expression) {
        if let ExprKind::Binary {
            op: BinaryOp::LogOr,
            left,
            right,
        } = &cond.kind
        {
            // Release even snapshots skipped by short-circuit evaluation.
            self.vc_release(left);
            self.vc_release(right);
        }
        if let ExprKind::SystemCall { name, args } = &cond.kind {
            if name == VC_CHANGED_MARKER {
                if let Some(ExprKind::Number(NumberLiteral::Integer { value, .. })) =
                    args.first().map(|a| &a.kind)
                {
                    if let Ok(idx) = value.parse::<u64>() {
                        self.vc_saved.remove(&idx);
                    }
                }
            }
        }
    }

    /// §9.6.2/§9.7: canceled processes cannot resume their captured waits.
    pub(super) fn vc_cancel_process(&mut self, pid: usize) {
        self.vc_saved.retain(|_, (owner, _)| *owner != pid);
        // Same-slot clocking waits are parked outside event_waiters until
        // Reactive. Every kill/disable path must purge these waits too.
        self.deferred_clocking_conts.retain(|(p, _)| *p != pid);
        self.deferred_clocking_guarded.retain(|(w, _)| w.pid != pid);
        // Every kill site passes here: a process killed while suspended in
        // an imported task unwinds it by the §35.9 disable protocol.
        if !self.dpi_tasks.is_empty() {
            self.dpi_task_killed(pid);
        }
    }

    /// The Wait statement a value-change waiter resumes into.
    pub(super) fn vc_wait_stmt(
        cond: Expression,
        body: &Statement,
        span: crate::ast::Span,
    ) -> Statement {
        Statement::new(
            StatementKind::Wait {
                condition: cond,
                stmt: Box::new(body.clone()),
            },
            span,
        )
    }

    /// Extract the field name from an `@name` event control, whether it
    /// parsed as a bare `Identifier` (`@ev`) or a `HierIdentifier`
    /// (`@this.ev`, which the elaborator rewrites a class-method `@ev` into).
    /// Returns the leaf name when the control targets a single unqualified
    /// name, else `None` (multi-segment paths, event expressions, `*`).
    pub(super) fn event_control_field_name(&self, event: &EventControl) -> Option<String> {
        if let EventControl::Identifier(id) = event {
            return Some(id.name.clone());
        }
        let expr = Self::event_control_single_expr(event)?;
        if let ExprKind::Ident(h) = &expr.kind {
            if h.path.len() == 1 && h.path[0].selects.is_empty() {
                return Some(h.path[0].name.name.clone());
            }
        }
        None
    }

    /// The identity key of an event-valued operand, for `==`/`!=` (§15.5.4):
    /// the alias-resolved storage key of a declared event, or the null key for
    /// a `null` literal. `None` when the operand is not an event, so ordinary
    /// comparisons are untouched.
    pub(super) fn event_operand_key(&mut self, e: &Expression) -> Option<String> {
        if matches!(e.kind, ExprKind::Null) {
            return Some(Self::EVENT_NULL_KEY.to_string());
        }
        let k = self.event_ref_key(e)?;
        Some(self.resolve_event_key(&k))
    }

    /// `.triggered` stamp key for a per-instance class event. Instance events
    /// have no backing signal (the handle is dynamic), so they cannot use the
    /// name-keyed signal path; this gives them a distinct key in the same
    /// `event_triggered_time` table. The `\x02` prefix cannot collide with a
    /// user identifier.
    pub(super) fn instance_event_stamp_key(key: &(usize, String)) -> String {
        format!("\u{2}inst{}#{}", key.0, key.1)
    }

    /// §15.5 an event that is a CLASS PROPERTY reached through a handle:
    /// `h.ce`, `this.ce`, or a dotted name the `->` parser flattened. Returns
    /// the per-instance `(handle, field)` identity. `resolve_this_event_field`
    /// only covers a BARE field name on `this`, so `@(h.ce)` and `-> h.ce`
    /// from outside the class had no identity at all — the waiter fell into
    /// the delta-yield and woke at t=0, and the trigger fired a name nothing
    /// was listening to.
    /// Resolve a dotted sensitivity NAME through class-handle and
    /// virtual-interface hops to a concrete signal name, at WAIT time.
    /// `this.drv.bus.clk_i`: `this` -> live handle; `.drv` -> property
    /// handle; `.bus` -> `virtual_iface_bindings` -> bound instance name;
    /// the remaining tail joins that instance path. Snitch-style class
    /// drivers block on exactly this shape (`@(posedge this.drv.bus.clk_i)`),
    /// and without runtime resolution the waiter fell into the
    /// "not a real signal" same-time yield — a time-0 livelock.
    pub(super) fn resolve_sens_name_via_handles(&self, dotted: &str) -> Option<String> {
        if !dotted.contains('.') {
            return None;
        }
        let segs: Vec<&str> = dotted.split('.').collect();
        let mut i;
        let mut handle: usize = if segs[0] == "this" || segs[0] == "super" {
            i = 1;
            self.this_stack.last().copied().flatten()?
        } else if let Some(h) = self.eval_ident_handle(segs[0]) {
            i = 1;
            h
        } else if self.this_stack.last().copied().flatten().is_some() {
            // Class-scope shorthand: `bus.clk_i` inside a method is
            // `this.bus.clk_i` (§8.11) — the driver classes' own
            // `@(posedge bus.clk_i)` waits are exactly this shape.
            i = 0;
            self.this_stack.last().copied().flatten()?
        } else {
            return None;
        };
        while i < segs.len() {
            let cn = self
                .heap
                .get(handle)
                .and_then(|o| o.as_ref())
                .map(|o| o.class_name.clone())?;
            // Own or inherited (§8.13), instance or static.
            if self.class_vif_decl(&cn, segs[i]).is_some() {
                let (bound, _) = self.vif_prop_target(&cn, handle, segs[i])?;
                let rest = segs[i + 1..].join(".");
                if rest.is_empty() {
                    return None;
                }
                let target = format!("{bound}.{rest}");
                return self
                    .signal_name_to_id
                    .contains_key(target.as_str())
                    .then_some(target);
            }
            let v = self
                .heap
                .get(handle)
                .and_then(|o| o.as_ref())?
                .properties
                .get(segs[i])?
                .clone();
            let h2 = v.to_u64()? as usize;
            if h2 == 0 || self.heap.get(h2).and_then(|o| o.as_ref()).is_none() {
                return None;
            }
            handle = h2;
            i += 1;
        }
        None
    }

    pub(super) fn resolve_handle_event_field(&mut self, dotted: &str) -> Option<(usize, String)> {
        let (h, f) = self.resolve_handle_prop_any(dotted)?;
        if self.class_prop_is_event(h, &f) {
            Some((h, f))
        } else {
            None
        }
    }

    /// `recv.field` -> `(handle, field)` for ANY property (event or not).
    pub(super) fn resolve_handle_prop_any(&mut self, dotted: &str) -> Option<(usize, String)> {
        let (recv, field) = dotted.rsplit_once('.')?;
        if recv.contains('.') {
            return None; // deeper chains are not modelled
        }
        let handle = if recv == "this" || recv == "super" {
            self.this_stack.last().copied().flatten()?
        } else {
            self.eval_ident_handle(recv)?
        };
        let inst = self.heap.get(handle).and_then(|o| o.as_ref())?;
        if inst.properties.contains_key(field) {
            Some((handle, field.to_string()))
        } else {
            None
        }
    }

    /// The same, from an EXPRESSION (`@(h.ce)`, `h.ce.triggered`).
    pub(super) fn expr_handle_event_field(&mut self, e: &Expression) -> Option<(usize, String)> {
        let (h, f) = self.expr_handle_prop_any(e)?;
        if self.class_prop_is_event(h, &f) {
            Some((h, f))
        } else {
            None
        }
    }

    /// As `expr_handle_event_field`, for a property of any type.
    pub(super) fn expr_handle_prop_any(&mut self, e: &Expression) -> Option<(usize, String)> {
        let dotted = match &e.kind {
            ExprKind::MemberAccess { expr, member } => {
                let recv = match &expr.kind {
                    ExprKind::This => "this".to_string(),
                    ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                        h.path[0].name.name.clone()
                    }
                    _ => return None,
                };
                format!("{}.{}", recv, member.name)
            }
            // `h.ce` also reaches here as a two-segment HIERARCHICAL name,
            // which is how `@(h.ce)` parses — the receiver is a class handle,
            // not an instance path, so the hierarchical resolver finds nothing.
            ExprKind::Ident(h)
                if h.path.len() == 2 && h.path.iter().all(|s| s.selects.is_empty()) =>
            {
                format!("{}.{}", h.path[0].name.name, h.path[1].name.name)
            }
            _ => return None,
        };
        self.resolve_handle_prop_any(&dotted)
    }

    /// §15.5: an event CLASS PROPERTY reached through an ARBITRARY receiver
    /// expression — `direct.ev` (property-of-`this` handle), `m_events[key].ev`
    /// (assoc element, runtime key), `q[0].ev` (queue element), `h.grp.ev`
    /// (chained handles). `resolve_handle_event_field` only accepts a
    /// select-free single-segment receiver, so all of these fell into the
    /// delta-yield and woke at t=0 — which is exactly the UVM objection spin:
    /// `uvm_objection::wait_for`'s `@(m_events[obj].all_dropped)` returned
    /// immediately, so `wait_for_self_and_siblings_to_drop` re-entered it
    /// forever and simulation never left the time step (GitHub #109, UVM
    /// 1800.2-2017). The receiver is EVALUATED here (per §9.4.2 the event
    /// expression is sampled when the wait executes), and the field must be a
    /// declared `event` property of the object's class — a non-event member
    /// keeps its existing (value-change) path.
    pub(super) fn expr_instance_event_field_general(
        &mut self,
        e: &Expression,
    ) -> Option<(usize, String)> {
        let key = self.expr_instance_prop_field_general(e)?;
        self.class_prop_is_event(key.0, &key.1).then_some(key)
    }

    /// §15.5, §7.4: an element of a class event-ARRAY property (`evs[i]` in a
    /// method of its class, `this.evs[i]`, `h.evs[i]`), keyed `(handle,
    /// "evs[i]")` with the index evaluated now. Waits, triggers and
    /// `.triggered` all use this key. `None` for anything else, decided by
    /// the expression's shape before anything is evaluated.
    pub(super) fn expr_instance_event_elem(&mut self, e: &Expression) -> Option<(usize, String)> {
        let mut e = e;
        while let ExprKind::Paren(inner) = &e.kind {
            e = inner;
        }
        let (base, index): (std::borrow::Cow<'_, Expression>, &Expression) = match &e.kind {
            ExprKind::Index { expr, index } => (std::borrow::Cow::Borrowed(&**expr), &**index),
            ExprKind::Ident(h)
                if h.root.is_none()
                    && h.path.last().is_some_and(|s| s.selects.len() == 1)
                    && h.path[..h.path.len() - 1]
                        .iter()
                        .all(|s| s.selects.is_empty()) =>
            {
                let mut b = h.clone();
                b.path.last_mut()?.selects.clear();
                b.cached_signal_id = std::cell::Cell::new(None);
                b.cached_resolved_name = std::cell::OnceCell::new();
                let base = Expression::new(ExprKind::Ident(b), e.span);
                (std::borrow::Cow::Owned(base), &h.path.last()?.selects[0])
            }
            _ => return None,
        };
        let (handle, field) = match &base.kind {
            ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                let n = h.path[0].name.name.as_str();
                if self.local_stack.last().is_some_and(|l| l.contains_key(n)) {
                    return None;
                }
                (self.this_stack.last().copied().flatten()?, n.to_string())
            }
            _ => self.expr_instance_prop_field_general(&base)?,
        };
        if !self.class_event_array_prop(handle, &field) {
            return None;
        }
        let iv = self.eval_expr(index);
        let i = iv.to_i64().filter(|_| !iv.has_xz())?;
        Some((handle, format!("{}[{}]", field, i)))
    }

    /// Is `field` a fixed-size array of `event` declared by the class of
    /// object `handle` or an ancestor?
    pub(super) fn class_event_array_prop(&self, handle: usize, field: &str) -> bool {
        let Some(inst) = self.heap.get(handle).and_then(|o| o.as_ref()) else {
            return false;
        };
        let mut cur = Some(inst.class_name.as_str());
        let mut hops = 0;
        while let Some(cn) = cur {
            let Some(cd) = self.module.classes.get(cn) else {
                return false;
            };
            if let Some(dt) = cd.property_types.get(field) {
                return matches!(
                    dt,
                    DataType::Simple {
                        kind: crate::ast::types::SimpleType::Event,
                        ..
                    }
                ) && (cd.array_properties.contains_key(field)
                    || cd.array_nd_properties.contains_key(field));
            }
            hops += 1;
            if hops > 64 {
                return false;
            }
            cur = cd.extends.as_deref();
        }
        false
    }

    /// Resolve a property through a nested or selected object receiver.
    /// Classification is separate so ordinary properties cannot accidentally
    /// fall through to the unsuspendable event fallback.
    pub(super) fn expr_instance_prop_field_general(
        &mut self,
        e: &Expression,
    ) -> Option<(usize, String)> {
        // Split <receiver-expr>.<field>: a MemberAccess head, or a
        // multi-segment hierarchical ident whose LAST segment is select-free.
        let (recv_expr, field): (Expression, String) = match &e.kind {
            ExprKind::MemberAccess { expr, member } => ((**expr).clone(), member.name.clone()),
            ExprKind::Ident(h)
                if h.path.len() >= 2 && h.path.last().is_some_and(|s| s.selects.is_empty()) =>
            {
                let mut recv = h.clone();
                let field = recv.path.pop()?.name.name;
                (Expression::new(ExprKind::Ident(recv), e.span), field)
            }
            _ => return None,
        };
        let handle = match &recv_expr.kind {
            ExprKind::This => self.this_stack.last().copied().flatten()?,
            ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                let n = h.path[0].name.name.as_str();
                if n == "this" || n == "super" {
                    self.this_stack.last().copied().flatten()?
                } else {
                    self.eval_ident_handle(n)?
                }
            }
            // Selects / chains: evaluate the receiver expression to a handle.
            _ => {
                let h = self.eval_expr(&recv_expr).to_u64()? as usize;
                if h == 0 {
                    return None; // null handle
                }
                h
            }
        };
        let inst = self.heap.get(handle).and_then(|o| o.as_ref())?;
        if !inst.properties.contains_key(&field) {
            return None;
        }
        Some((handle, field))
    }

    /// §13.5.2: decide whether a `ref` actual can be TRULY ALIASED — reads of
    /// the formal re-read it, writes write through, no return copy-out. Only
    /// module-visible plain variables qualify (their resolution is caller-
    /// frame-independent, so the frozen expression evaluates identically from
    /// inside the callee). A chained ref (the actual is itself a ref formal of
    /// the caller) resolves through the CALLER's alias frame to the original
    /// storage. Caller-frame locals, aggregate elements, class properties and
    /// events return None and keep the legacy copy-in/copy-out path.
    ///
    /// NOTE: called AFTER the callee's local frame is pushed, so the caller's
    /// frames are second-from-top.
    /// §5 diagnosability: (settle-cap hit count, last hit time). Nonzero
    /// hits mean some time steps may not have fully converged.
    pub fn settle_limit_report(&self) -> (u64, u64) {
        (self.settle_limit_hits, self.settle_limit_last_time)
    }

    pub(super) fn ref_alias_target(&mut self, arg: &Expression) -> Option<String> {
        // §13.5.2: `ref arr[i]` — the ELEMENT identity is frozen at CALL
        // time; a later change to `i` must not retarget the alias (the old
        // copy-out re-evaluated the index at return and wrote a different
        // element).
        if let ExprKind::Index { expr: base, index } = &arg.kind {
            if let ExprKind::Ident(bh) = &base.kind {
                if bh.path.len() == 1 && bh.path[0].selects.is_empty() {
                    let bname = bh.path[0].name.name.clone();
                    let caller_local = self
                        .local_stack
                        .iter()
                        .rev()
                        .nth(1)
                        .is_some_and(|f| f.contains_key(&bname));
                    if !caller_local
                        && self.module.arrays.contains_key(bname.as_str())
                        && !self.module.dynamic_arrays.contains(&bname)
                        && !self.module.queue_vars.contains(&bname)
                        && !self.module.associative_arrays.contains_key(&bname)
                    {
                        let idx = self.eval_expr(index).to_i64()?;
                        if self.get_array_elem_id(&bname, idx).is_some() {
                            return Some(format!("{}[{}]", bname, idx));
                        }
                    }
                }
            }
            return None;
        }
        let h = match &arg.kind {
            ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => h,
            _ => return None,
        };
        let name = h.path[0].name.name.as_str();
        // Chained ref: the caller's alias frame is still top-of-stack here
        // (the callee's is pushed just after resolution).
        if let Some(frozen) = self.ref_alias_stack.last().and_then(|m| m.get(name)) {
            return Some(frozen.clone());
        }
        // A caller-frame local (including a non-aliased ref formal) is not
        // module-visible — the callee cannot reach its storage by name.
        let caller_frame = self.local_stack.iter().rev().nth(1);
        if caller_frame.is_some_and(|f| f.contains_key(name)) {
            return None;
        }
        if self
            .ref_binding_stack
            .last()
            .is_some_and(|m| m.contains_key(name))
        {
            return None; // ref-bound in the caller but not aliasable
        }
        // Aggregates and strings keep their dedicated formal machinery.
        if self.module.arrays.contains_key(name)
            || self.module.dynamic_arrays.contains(name)
            || self.module.queue_vars.contains(name)
            || self.module.associative_arrays.contains_key(name)
            || self.string_signals.contains(name)
        {
            return None;
        }
        // A class property reached via `this` resolves differently inside the
        // callee — reject when the surrounding context could capture it.
        if let Some(Some(handle)) = self.this_stack.last() {
            if self
                .heap
                .get(*handle)
                .and_then(|o| o.as_ref())
                .is_some_and(|i| i.properties.contains_key(name))
            {
                return None;
            }
        }
        // Plain module-visible variable storage only.
        if self.signal_name_to_id.contains_key(name) || self.signals.contains_key(name) {
            Some(name.to_string())
        } else {
            None
        }
    }

    /// §15.5 / §13.5.2: an event passed as a `ref` formal. `-> x` and `@x`
    /// inside the subroutine must act on the CALLER's event, but the formal
    /// name is not itself a declared event, so the trigger fired a name with
    /// no sync object and the caller's waiter hung forever. `ref_binding_stack`
    /// already maps the formal to the caller's actual expression; resolve it
    /// to that event's key. Innermost frame first, so nested calls resolve to
    /// the nearest binding.
    pub(super) fn ref_bound_event_key(&mut self, name: &str) -> Option<String> {
        let bound = self
            .ref_binding_stack
            .iter()
            .rev()
            .find_map(|m| m.get(name).cloned())?;
        let key = self.event_ref_key(&bound)?;
        Some(self.resolve_event_key(&key))
    }

    /// The single un-edged expression of an event control (`@e`, `@(e)`,
    /// `@ev[1]`, `@(h.ce)`), or None for edge terms / multi-term lists / `*`.
    pub(super) fn event_control_single_expr(event: &EventControl) -> Option<Expression> {
        let mut expr = match event {
            EventControl::HierIdentifier(e) => e,
            EventControl::EventExpr(terms) if terms.len() == 1 && terms[0].edge.is_none() => {
                &terms[0].expr
            }
            _ => return None,
        };
        // §9.4.2: redundant parentheses cannot change the kind of wait.
        while let ExprKind::Paren(inner) = &expr.kind {
            expr = inner;
        }
        Some(expr.clone())
    }

    /// If this event control names a single DECLARED event variable — `@ev`,
    /// `@(ev)`, `@ev_arr[i]` — return its storage key (`ev`, `ev_arr[3]`).
    /// `None` for edge-qualified terms, multi-term lists, `*`, and anything
    /// that is not a declared event, all of which keep their existing paths.
    pub(super) fn event_control_event_key(&mut self, event: &EventControl) -> Option<String> {
        let expr = Self::event_control_single_expr(event)?;
        if let Some(k) = self.event_ref_key(&expr) {
            return Some(k);
        }
        // `@x` on a `ref event` formal waits on the caller's event.
        if let ExprKind::Ident(h) = &expr.kind {
            if h.path.len() == 1 && h.path[0].selects.is_empty() {
                let n = h.path[0].name.name.clone();
                return self.ref_bound_event_key(&n);
            }
        }
        None
    }

    /// Wake every process parked on `@<field>` for instance `key` (an edge
    /// trigger: only processes already waiting when `-><field>` fires resume).
    /// Each woken continuation is scheduled for the current time so it runs in
    /// the same slot's remaining active region — mirroring how a blocking
    /// `->e` resumes an `@e` waiter for a module-scope named event.
    pub(super) fn fire_instance_event(&mut self, key: (usize, String)) {
        let now = self.time;
        // §15.5.3: `<h>.<ev>.triggered` must read 1 for the rest of this slot.
        // Instance events have no backing signal, so stamp the same table the
        // name-keyed path uses, under a synthetic per-instance key.
        let stamp = Self::instance_event_stamp_key(&key);
        self.event_triggered_time.insert(stamp, now);
        if !self.cg_instance_event_waiters.is_empty() {
            if let Some(w) = self.cg_instance_event_waiters.get(&key).cloned() {
                self.sample_event_covergroups(w);
            }
        }
        let mut woken = Vec::new();
        self.instance_event_waiters.retain(|w| {
            if w.key.contains(&key) {
                woken.push((w.pid, w.continuation.clone()));
                false
            } else {
                true
            }
        });
        for (pid, cont) in woken {
            self.event_queue.schedule(now, pid, cont);
        }
    }

    /// Resolve signal-backed event spellings once. Event signal IDs are stable
    /// for the compiled design; aliases and covergroup listeners remain live.
    pub(super) fn named_event_plan(&mut self, event: EventId) -> std::rc::Rc<NamedEventPlan> {
        if let Some(plan) = self.event_triggered_time.plan(event) {
            return plan;
        }
        let raw = self.event_triggered_time.name(event).as_ref();
        let trimmed = raw.trim_start_matches('.');
        let mut candidates = vec![raw.to_string(), trimmed.to_string()];
        candidates.push(format!("{}.{}", self.module.name, trimmed));
        candidates.sort();
        candidates.dedup();
        let mut targets = Vec::new();
        for candidate in candidates {
            if let Some(&signal_id) = self.signal_name_to_id.get(candidate.as_str()) {
                let stamp = self.event_triggered_time.intern(&candidate);
                let name = self.event_triggered_time.name(stamp).clone();
                targets.push(NamedEventTarget {
                    name,
                    signal_id,
                    stamp,
                });
            }
        }
        // Preserve distinct spellings even when they map to the same signal:
        // writes, trigger stamps and covergroup sampling occur in name order.
        self.event_triggered_time.set_plan(
            event,
            NamedEventPlan {
                targets: targets.into_boxed_slice(),
            },
        )
    }

    /// Fire a named event NOW: toggle its 1-bit signal (the edge `@(e)`
    /// waiters arm on) and stamp `event_triggered_time` for `.triggered`.
    /// Shared by the blocking `-> e` statement and the NBA-region flush of
    /// deferred `->> e` triggers.
    /// The `instance_event_waiters` key of a declared event that has no
    /// signal (a subroutine's local `event e;`): handle 0 and the event's
    /// canonical name, which no class property spells, qualified by the
    /// generation of the frame declaring it so that two activations' locals
    /// stay apart. A forked child shares its parent's frame generations.
    pub(super) fn unbacked_event_key(&self, canon: &str) -> (usize, String) {
        let fgen = self
            .local_stack
            .iter()
            .rposition(|f| f.contains_key(canon))
            .and_then(|i| self.local_gen_stack.get(i));
        match fgen {
            Some(g) => (0, format!("\u{3}ev#{}@{}", canon, g)),
            None => (0, format!("\u{3}ev#{}", canon)),
        }
    }

    pub(super) fn fire_named_event(&mut self, raw_name: &str) {
        // §15.5.5: resolve the live handle at delivery, before consulting the
        // cache. Rebinding an alias must never reuse its previous object's plan.
        let canon = Self::resolve_event_key_ref(&self.event_aliases, raw_name);
        if canon == Self::EVENT_NULL_KEY {
            return;
        }
        let unbacked = (!self.instance_event_waiters.is_empty()
            || self.local_stack.iter().any(|f| f.contains_key(canon)))
        .then(|| self.unbacked_event_key(canon));
        if self.in_pre_process_settle {
            // §9.2.2.2: waiters register during the time-0 active region.
            // Keep the name, so deferred delivery follows the existing rules.
            self.pending_t0_triggers.push(canon.to_string());
            return;
        }
        let event = self.event_triggered_time.intern(canon);
        let plan = self.named_event_plan(event);
        for target in &plan.targets {
            // Retain scope-aware reads: subroutine frames and scope hints can
            // affect the old name lookup even though its write ID is fixed.
            let cur = self
                .get_signal_value_by_name(&target.name)
                .unwrap_or(Value::zero(1));
            let new_val = if cur.bits_first() == LogicBit::One {
                Value::zero(1)
            } else {
                Value::ones(1)
            };
            sim_dbg_eprintln!(
                "[DEBUG] firing event {} (new_val={:?}) at time {}",
                target.name,
                new_val,
                self.time
            );
            self.fast_signal_write_id(target.signal_id, &new_val);
            self.event_triggered_time.mark(target.stamp, self.time);
            if !self.cg_named_event_waiters.is_empty() {
                let waiters = self.cg_named_event_waiters.get(&target.signal_id).cloned();
                if let Some(waiters) = waiters {
                    self.sample_event_covergroups(waiters);
                }
            }
        }
        // Unregistered and dynamic names still have a .triggered stamp.
        self.event_triggered_time.mark(event, self.time);
        // §15.5: wake `@e` on an event with no signal (a subroutine's local
        // `event e;`), parked by name in `run_process_stmts`.
        if plan.targets.is_empty()
            && let Some(key) = unbacked
        {
            let now = self.time;
            // Its `.triggered` stamp, kept apart per activation like the wait.
            self.event_triggered_time.insert(key.1.clone(), now);
            let mut woken = Vec::new();
            self.instance_event_waiters.retain(|w| {
                if w.key.contains(&key) {
                    woken.push((w.pid, w.continuation.clone()));
                    false
                } else {
                    true
                }
            });
            for (pid, cont) in woken {
                self.event_queue.schedule(now, pid, cont);
            }
        }
        // Preserve scheduler, force, VPI and combinational checkpoints. A
        // listener-only guard would skip work unrelated to this event's fanout.
        self.settle_combinatorial();
    }

    /// IEEE 1800-2017 §6.16 string methods that MUTATE the receiver
    /// (`putc`, `itoa`, `hextoa`, `octtoa`, `bintoa`, `realtoa`) plus `atoreal`.
    /// Every one of them was a silent no-op / returned 0.
    ///
    /// `recv` is the receiver expression; it is assigned back in place. Called
    /// from both the `MemberAccess` dispatch and the flattened `Ident([s, m])`
    /// one, which is how a statement like `s.putc(0, "J");` actually parses.
    pub(super) fn string_method(
        &mut self,
        recv: &Expression,
        mname: &str,
        args: &[Expression],
    ) -> Option<Value> {
        // §6.16.4 `s.putc(i, c)` and §6.16.10 `s.itoa/hextoa/octtoa/bintoa/
        // realtoa(v)` MODIFY the receiver in place. Every one of them was a
        // silent no-op.
        if mname == "putc" && args.len() == 2 {
            let mut b = self.eval_expr(recv).sv_string_bytes();
            let idx = self.eval_expr(&args[0]).to_i64().unwrap_or(-1);
            let ch = (self.eval_expr(&args[1]).to_u64().unwrap_or(0) & 0xFF) as u8;
            // §6.16.4: an out-of-range index or a null character leaves the
            // string unchanged.
            if idx >= 0 && (idx as usize) < b.len() && ch != 0 {
                b[idx as usize] = ch;
                let v = Self::sv_string_from_bytes(&b);
                self.assign_value(recv, &v);
            }
            return Some(Value::zero(1));
        }
        // §6.16.6 compare/icompare — strcmp-style lexicographic difference:
        // 0 when equal, negative/positive otherwise. Neither was implemented,
        // so both returned 0 and `compare()` looked case-insensitive.
        if matches!(mname, "compare" | "icompare") && args.len() == 1 {
            let a = self.eval_expr(recv).to_sv_string();
            let b = self.eval_expr(&args[0]).to_sv_string();
            let d: i64 = Self::sv_strcmp(&a, &b, mname == "icompare");
            let mut v = Value::from_u64(d as u64, 32);
            v.is_signed = true;
            return Some(v);
        }
        if matches!(mname, "itoa" | "hextoa" | "octtoa" | "bintoa" | "realtoa") && args.len() == 1 {
            let v = self.eval_expr(&args[0]);
            let text = match mname {
                // `itoa` renders a SIGNED decimal; the others are unsigned
                // radix conversions of the bit pattern.
                "itoa" => {
                    if v.width > 32 {
                        format!("{}", v.to_i64().unwrap_or(0))
                    } else {
                        format!("{}", v.to_u64().unwrap_or(0) as u32 as i32)
                    }
                }
                "hextoa" => format!("{:x}", v.to_u64().unwrap_or(0)),
                "octtoa" => format!("{:o}", v.to_u64().unwrap_or(0)),
                "bintoa" => format!("{:b}", v.to_u64().unwrap_or(0)),
                // §6.16.10 `realtoa`: the ASCII DECIMAL representation, which
                // is `%g` (6 significant digits, trailing zeros stripped,
                // exponent form when needed) — not `%f`. It printed
                // "11.100000" for 11.1, and "100000000000000000000.000000"
                // where 1e+20 was wanted (ivtest sv_string6).
                _ => Self::format_g(v.to_f64(), 6, false),
            };
            let sv = Value::from_string(&text);
            self.assign_value(recv, &sv);
            return Some(Value::zero(1));
        }
        // §6.16.9 `atoreal` — the longest valid real prefix, 0.0 on no match.
        if mname == "atoreal" && args.is_empty() {
            let raw = self.eval_expr(recv).to_sv_string();
            let t = raw.trim();
            let b = t.as_bytes();
            let mut i = 0usize;
            let mut end = 0usize;
            if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
                i += 1;
            }
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
                end = i;
            }
            if i < b.len() && b[i] == b'.' {
                i += 1;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                    end = i;
                }
            }
            if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
                let save = i;
                i += 1;
                if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
                    i += 1;
                }
                let ds = i;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
                if i > ds {
                    end = i;
                } else {
                    i = save;
                    let _ = i;
                }
            }
            let val: f64 = t.get(..end).and_then(|p| p.parse().ok()).unwrap_or(0.0);
            return Some(Value::from_f64(val));
        }
        None
    }
}
