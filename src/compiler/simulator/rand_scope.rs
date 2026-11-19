//! §18.7 — name binding inside `obj.randomize() with { … }`.
//!
//! A name in the inline block binds first in the randomized OBJECT's class
//! scope and only then in the scope of the randomize() call; `local::name`
//! skips the object and binds in the call's scope directly (§18.7.1). The
//! solve itself runs with `this` switched to the object, where the caller's
//! locals and its class members no longer resolve. Every operand that binds
//! in the caller's scope is therefore a state variable of the solve: it is
//! evaluated once, before `this` switches, and replaced by a literal of its
//! value. Operands that bind in the object (its members, the receiver handle,
//! `this`, foreach iterators) are left for the solver.
use super::*;
use crate::ast::decl::DistWeight;

/// §18.6: only the active rand state of reachable child objects belongs to
/// a solve transaction. Unrelated objects and non-rand callback state do not.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct RandChildSnapshot {
    handle: usize,
    properties: Vec<(String, Option<Value>)>,
    collections: Vec<(String, Vec<(String, Value)>)>,
}

#[derive(serde::Serialize, serde::Deserialize)]

pub(super) struct RandChildTransaction {
    pub(super) owner: usize,
    pub(super) children: Vec<RandChildSnapshot>,
}

impl Simulator {
    /// §18.8/§18.9: whole-object mode changes apply to each declared member.
    /// Store names, so a later per-member enable can override a bulk disable.
    pub(super) fn set_object_random_mode(
        &mut self,
        handle: usize,
        rand_mode: bool,
        target: Option<&str>,
        enable: bool,
    ) {
        let mut names = HashSet::default();
        // §18.5.11: a static block's mode is one class-wide flag, keyed by
        // its declaring class, so a bulk change reaches it there.
        let mut static_blocks = Vec::new();
        if target.is_none() {
            let mut cls = self
                .heap
                .get(handle)
                .and_then(|o| o.as_ref())
                .map(|o| o.class_name.as_str());
            while let Some(name) = cls {
                let Some(def) = self.module.classes.get(name) else {
                    break;
                };
                if rand_mode {
                    names.extend(def.random_properties.iter().cloned());
                } else {
                    for (con_name, con) in &def.constraints {
                        if con.is_static {
                            static_blocks.push((name.to_string(), con_name.clone()));
                        } else {
                            names.insert(con_name.clone());
                        }
                    }
                }
                cls = def.extends.as_deref();
            }
        }
        for key in static_blocks {
            if enable {
                self.static_constraint_disabled.remove(&key);
            } else {
                self.static_constraint_disabled.insert(key);
            }
        }
        let table = if rand_mode {
            &mut self.rand_mode_disabled
        } else {
            &mut self.constraint_mode_disabled
        };
        let disabled = table.entry(handle).or_default();
        match (target, enable) {
            (Some(name), true) => {
                disabled.remove(name);
                // Enabling a whole array enables each of its elements too.
                if !name.contains('[') {
                    let elem = format!("{name}[");
                    disabled.retain(|k| !k.starts_with(&elem));
                }
            }
            (Some(name), false) => {
                disabled.insert(name.to_string());
            }
            (None, true) => disabled.clear(),
            (None, false) => {
                disabled.extend(names);
            }
        }
    }
    /// §18.8, §18.9: `obj.sub.x.rand_mode(0)` and
    /// `obj.sub.c.constraint_mode(0)` name the random variable or constraint
    /// `x` / `c` of the object `obj.sub`, not a member `sub.x` of `obj`, so
    /// the randomize() of `obj.sub` (on its own or as a sub-object) sees the
    /// mode. Descend from `h` through the leading parts of the dotted
    /// `target` that are class-handle properties holding an object.
    pub(super) fn rand_mode_owner(&self, mut h: usize, target: &str) -> (usize, String) {
        let mut parts: Vec<&str> = target.split('.').collect();
        while parts.len() > 1 {
            let p = parts[0];
            if p.contains('[') || self.prop_class_type(h, p).is_none() {
                break;
            }
            let Some(next) = self
                .heap
                .get(h)
                .and_then(|o| o.as_ref())
                .and_then(|i| i.properties.get(p))
                .and_then(|v| v.to_u64())
                .map(|v| v as usize)
                .filter(|&v| v != 0 && self.heap.get(v).is_some_and(|o| o.is_some()))
            else {
                break;
            };
            h = next;
            parts.remove(0);
        }
        (h, parts.join("."))
    }

    /// §18.8: the object and element key (`arr[2]`, `m[1][0]`) a
    /// `rand_mode` receiver names when it selects an element of an unpacked
    /// rand array: `obj.arr[i]`, or `arr[i]` inside the class.
    pub(super) fn rand_mode_element_target(
        &mut self,
        recv: &Expression,
    ) -> Option<(usize, String)> {
        let mut idx = Vec::new();
        let mut base = recv;
        while let ExprKind::Index { expr, index } = &base.kind {
            idx.push(self.eval_expr(index).to_i64()?);
            base = expr;
        }
        if idx.is_empty() {
            return None;
        }
        let (handle, name) = match &base.kind {
            ExprKind::MemberAccess { expr, member } => {
                (self.eval_expr(expr).to_u64()? as usize, member.name.clone())
            }
            ExprKind::Ident(h) if h.path.len() == 1 => (
                self.this_stack.last().copied().flatten()?,
                h.path[0].name.name.clone(),
            ),
            // `obj.arr` flattened into one hierarchical name.
            ExprKind::Ident(h) if h.path.last().is_some_and(|s| s.selects.is_empty()) => {
                let mut owner = h.clone();
                let prop = owner.path.pop()?.name.name;
                owner.cached_signal_id = crate::ast::cache::SignalCache::new(None);
                owner.cached_resolved_name = crate::ast::cache::NameCache::new();
                let owner = Expression::new(ExprKind::Ident(owner), base.span);
                (self.eval_expr(&owner).to_u64()? as usize, prop)
            }
            _ => return None,
        };
        if handle == 0 || !self.object_declares_property(handle, &name) {
            return None;
        }
        let mut key = name;
        for i in idx.iter().rev() {
            key.push_str(&format!("[{i}]"));
        }
        Some((handle, key))
    }

    /// §18.8: an element whose rand_mode is off is a state variable for this
    /// call, so it keeps its value and the other constraints see that value:
    /// one `elem == value` constraint per disabled element that exists now.
    pub(super) fn rand_mode_element_pins(&mut self, handle: usize) -> Vec<ConstraintItem> {
        let keys: Vec<String> = match self.rand_mode_disabled.get(&handle) {
            Some(d) => d.iter().filter(|k| k.contains('[')).cloned().collect(),
            None => return Vec::new(),
        };
        let mut pins = Vec::new();
        for key in keys {
            if !self.signals.contains_key(&format!("{handle}#{key}")) {
                continue;
            }
            let (name, rest) = key.split_at(key.find('[').unwrap());
            let mut elem = super::vpi_model::ident_expr(name);
            for i in rest.trim_matches(|c| c == '[' || c == ']').split("][") {
                let index = Expression::new(
                    ExprKind::Number(crate::ast::expr::NumberLiteral::Integer {
                        size: None,
                        signed: true,
                        base: crate::ast::expr::NumberBase::Decimal,
                        value: i.to_string(),
                        cached_val: crate::ast::cache::LiteralCache::new(None),
                    }),
                    crate::ast::Span::dummy(),
                );
                elem = Expression::new(
                    ExprKind::Index {
                        expr: Box::new(elem),
                        index: Box::new(index),
                    },
                    crate::ast::Span::dummy(),
                );
            }
            self.this_stack.push(Some(handle));
            let v = self.eval_expr(&elem);
            self.this_stack.pop();
            if v.has_xz() {
                continue;
            }
            let value = Expression::new(
                ExprKind::Number(crate::ast::expr::NumberLiteral::Integer {
                    size: Some(v.width),
                    signed: false,
                    base: crate::ast::expr::NumberBase::Binary,
                    value: v.to_bin_string(),
                    cached_val: crate::ast::cache::LiteralCache::new(None),
                }),
                crate::ast::Span::dummy(),
            );
            pins.push(ConstraintItem::Expr(Expression::new(
                ExprKind::Binary {
                    op: BinaryOp::Eq,
                    left: Box::new(elem),
                    right: Box::new(value),
                },
                crate::ast::Span::dummy(),
            )));
        }
        pins
    }

    /// §18.5.9: whether constraint item `item` of object `obj` reads, through
    /// handle members, a member of one of the first `upto` objects of the
    /// running call's chain (`rand_solve_chain`): the enclosing objects a
    /// handle of the sub-object points back at (`item.addr`).
    pub(super) fn rand_item_reads_enclosing(
        &self,
        obj: usize,
        item: &ConstraintItem,
        upto: usize,
    ) -> bool {
        let upto = upto.min(self.rand_solve_chain.len());
        let targets: Vec<usize> = self.rand_solve_chain[..upto]
            .iter()
            .copied()
            .filter(|&h| h != obj)
            .collect();
        if targets.is_empty() {
            return false;
        }
        let leaf = |e: &Expression| self.csp_path_reaches_any(obj, e, &targets);
        Self::csp_item_match(item, &leaf, &|_| false)
    }

    /// §18.5.9: judge a sub-object's back-reference item (see
    /// `rand_item_reads_enclosing`) on the current values. Unlike the
    /// general checker, which leaves a `foreach` and the collection queries
    /// to the solver arms that enforce them, nothing else enforces such an
    /// item, so every form is evaluated: a `foreach` over a state collection
    /// of the sub-object element by element. A loop over a shape that
    /// cannot be enumerated passes, as in `check_member_foreach`.
    pub(super) fn check_back_reference_item(&mut self, sub: usize, item: &ConstraintItem) -> bool {
        self.this_stack.push(Some(sub));
        let ok = self.check_back_reference_item_impl(sub, item);
        self.this_stack.pop();
        ok
    }

    fn advance_indices(idx: &mut [i64], dims: &[(i64, i64)]) -> bool {
        for k in (0..dims.len()).rev() {
            idx[k] += 1;
            if idx[k] <= dims[k].1 {
                return true;
            }
            idx[k] = dims[k].0;
        }
        false
    }

    fn unpacked_array_dims(&self, h: usize, prop: &str) -> Option<Vec<(i64, i64)>> {
        if let Some((dims, _)) = self.fixed_foreach_dims(h, prop) {
            return Some(dims);
        }
        if let Some((dims, _)) = self.dynamic_prop_foreach_dims(h, prop) {
            return Some(dims);
        }
        let mut cn = self
            .heap
            .get(h)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.clone());
        while let Some(name) = cn {
            let cd = self.module.classes.get(&name)?;
            if cd.queue_properties.contains_key(prop) {
                let n = self.get_queue_size(&format!("{}#{}", h, prop)) as i64;
                return Some(vec![(0, n - 1)]);
            }
            if cd.properties.contains_key(prop) {
                return None;
            }
            cn = cd.extends.clone();
        }
        None
    }

    /// §18.5.8.1: associative state arrays iterate their existing keys,
    /// including string keys, rather than a numeric range of positions.
    pub(super) fn unroll_assoc_constraint_foreach(
        &self,
        handle: usize,
        array: &Expression,
        vars: &[Option<crate::ast::Identifier>],
        body: &ConstraintItem,
    ) -> Option<Vec<ConstraintItem>> {
        let [Some(var)] = vars else { return None; };
        let (owner, name) = if Self::foreach_plain_member(array) {
            (handle, Self::foreach_base_name(array)?)
        } else {
            self.csp_resolve_member_owner(handle, array)?
        };
        let mut class = self.heap.get(owner)?.as_ref()?.class_name.as_str();
        loop {
            let cd = self.module.classes.get(class)?;
            if cd.assoc_properties.contains_key(&name) {
                break;
            }
            if cd.properties.contains_key(&name) {
                return None;
            }
            class = cd.extends.as_deref()?;
        }
        let scoped = format!("{owner}#{name}");
        let string_keys = self.is_string_keyed_array(&scoped);
        let mut items = Vec::new();
        for key in self.assoc_key_strs(&scoped) {
            let literal = if string_keys {
                Expression::new(ExprKind::StringLiteral(decode_name_key(&key).to_string()), var.span)
            } else if let Ok(value) = key.parse::<i64>() {
                Self::literal_of(&Self::signed_loop_val(value), var.span)
            } else {
                Self::literal_of(&Value::from_u64(key.parse::<u64>().ok()?, 64), var.span)
            };
            let mut item = body.clone();
            Self::subst_ident_item(&mut item, &var.name, &literal);
            items.push(item);
        }
        Some(items)
    }

    fn check_back_reference_item_impl(&mut self, sub: usize, item: &ConstraintItem) -> bool {
        match item {
            ConstraintItem::Block(items) => items
                .iter()
                .all(|it| self.check_back_reference_item_impl(sub, it)),
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => {
                !self.eval_expr(condition).is_true()
                    || self.check_back_reference_item_impl(sub, constraint)
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                if self.eval_expr(condition).is_true() {
                    self.check_back_reference_item_impl(sub, then_item)
                } else {
                    else_item
                        .as_ref()
                        .is_none_or(|e| self.check_back_reference_item_impl(sub, e))
                }
            }
            ConstraintItem::Foreach {
                array,
                vars,
                item: body,
                ..
            } => {
                if let Some(items) = self.unroll_assoc_constraint_foreach(sub, array, vars, body) {
                    return items.iter().all(|it| self.check_back_reference_item_impl(sub, it));
                }
                let resolve_dims = |target_h: usize, arr: &Expression, name: &str| {
                    self.state_foreach_dims(target_h, arr, name, vars.len())
                        .or_else(|| self.fixed_foreach_dims(target_h, name).map(|d| d.0))
                        .or_else(|| {
                            (vars.len() == 1)
                                .then(|| {
                                    self.dynamic_prop_foreach_dims(target_h, name)
                                        .map(|d| d.0)
                                })
                                .flatten()
                        })
                };
                let dims = if Self::foreach_plain_member(array) {
                    Self::foreach_base_name(array)
                        .and_then(|name| resolve_dims(sub, array, &name))
                } else {
                    self.csp_resolve_member_owner(sub, array)
                        .and_then(|(target_h, leaf)| {
                            let plain = super::vpi_model::ident_expr(&leaf);
                            resolve_dims(target_h, &plain, &leaf)
                        })
                };
                let Some(dims) = dims else {
                    return true;
                };
                if dims.len() != vars.len() || dims.iter().any(|&(lo, hi)| hi < lo) {
                    return true;
                }
                let mut idx: Vec<i64> = dims.iter().map(|d| d.0).collect();
                let outer = self.local_stack.last().cloned().unwrap_or_default();
                loop {
                    let mut frame = outer.clone();
                    for (k, v) in vars.iter().enumerate() {
                        if let Some(v) = v {
                            frame.insert(v.name.clone(), Self::signed_loop_val(idx[k]));
                        }
                    }
                    self.push_local_frame(frame);
                    let ok = self.check_back_reference_item_impl(sub, body);
                    self.pop_local_frame();
                    if !ok {
                        return false;
                    }
                    if !Self::advance_indices(&mut idx, &dims) {
                        return true;
                    }
                }
            }
            ConstraintItem::Unique { exprs, span } => {
                let mut elems: Vec<Expression> = Vec::new();
                for e in exprs {
                    let dims = if let Some(name) = Self::plain_ident_name(e) {
                        self.unpacked_array_dims(sub, &name)
                    } else if !Self::foreach_plain_member(e) {
                        self.csp_resolve_member_owner(sub, e)
                            .and_then(|(th, leaf)| self.unpacked_array_dims(th, &leaf))
                    } else {
                        None
                    };
                    if let Some(dims) = dims {
                        if dims.iter().any(|&(lo, hi)| hi < lo) {
                            continue;
                        }
                        let mut idx: Vec<i64> = dims.iter().map(|d| d.0).collect();
                        loop {
                            let mut ex = e.clone();
                            for &i in &idx {
                                ex = Expression::new(
                                    ExprKind::Index {
                                        expr: Box::new(ex),
                                        index: Box::new(Self::literal_of(
                                            &Self::signed_loop_val(i),
                                            e.span,
                                        )),
                                    },
                                    e.span,
                                );
                            }
                            elems.push(ex);
                            if !Self::advance_indices(&mut idx, &dims) {
                                break;
                            }
                        }
                    } else {
                        elems.push(e.clone());
                    }
                }
                if elems.len() <= 1 {
                    return true;
                }
                let vals: Vec<Value> = elems.iter().map(|e| self.eval_expr(e)).collect();
                if vals.iter().any(|v| v.has_xz()) {
                    return false;
                }
                if let Some(first) = vals.first()
                    && !first.is_real
                    && first.width <= 64
                    && vals.iter().all(|v| {
                        !v.is_real && v.width == first.width && v.is_signed == first.is_signed
                    })
                {
                    let mut seen = HashSet::default();
                    return vals
                        .iter()
                        .all(|v| v.to_u64().is_some_and(|u| seen.insert(u)));
                }
                for i in 0..vals.len() {
                    for j in (i + 1)..vals.len() {
                        let eq = Expression::new(
                            ExprKind::Binary {
                                op: BinaryOp::Eq,
                                left: Box::new(Self::literal_of(&vals[i], *span)),
                                right: Box::new(Self::literal_of(&vals[j], *span)),
                            },
                            *span,
                        );
                        if self.eval_expr(&eq).is_true() {
                            return false;
                        }
                    }
                }
                true
            }
            other => self.check_constraint_item_impl(other),
        }
    }

    /// Budget exhaustion can occur in the parent after a child rolled back.
    /// Include active child constraints when identifying the failed solve.
    pub(super) fn failed_child_constraint_names(&mut self, handle: usize) -> Vec<String> {
        let members = self.active_child_rand_members(handle);
        let children = self.rand_child_handles(handle, &members);
        let mut names = Vec::new();
        for child in children {
            let class = self
                .heap
                .get(child)
                .and_then(|o| o.as_ref())
                .map(|o| o.class_name.clone());
            let constraints = self.active_constraints(child);
            self.class_context_stack.push(class);
            let chain = self.rand_solve_chain.len();
            let above = self
                .rand_solve_chain
                .iter()
                .position(|&h| h == handle)
                .unwrap_or(0);
            for constraint in constraints {
                if constraint.items.iter().any(|item| {
                    // An item that reads an enclosing object through a
                    // handle back to it is judged in every form.
                    if self.rand_item_reads_enclosing(child, item, chain) {
                        if above > 0 && self.rand_item_reads_enclosing(child, item, above) {
                            return false;
                        }
                        return !self.check_back_reference_item(child, item);
                    }
                    !Self::constraint_unmodeled(item) && !self.check_constraint_item(child, item)
                }) {
                    names.push(constraint.name.name.clone());
                }
            }
            self.class_context_stack.pop();
        }
        names.sort();
        names.dedup();
        names
    }

    /// Active rand members of a child object. A caller's member-subset list
    /// applies only to the parent, not to the recursively randomized child.
    fn active_child_rand_members(&self, handle: usize) -> Vec<String> {
        let mut out = Vec::new();
        let disabled = self.rand_mode_disabled.get(&handle);
        if disabled.is_some_and(|d| d.contains("*")) {
            return out;
        }
        let mut seen = HashSet::default();
        let mut cls = self
            .heap
            .get(handle)
            .and_then(|o| o.as_ref())
            .map(|o| o.class_name.as_str());
        while let Some(name) = cls {
            let Some(cd) = self.module.classes.get(name) else {
                break;
            };
            for prop in &cd.property_order {
                if seen.insert(prop.clone())
                    && cd.random_properties.contains(prop)
                    && !disabled.is_some_and(|d| d.contains(prop))
                {
                    out.push(prop.clone());
                }
            }
            cls = cd.extends.as_deref();
        }
        out
    }

    /// Storage keys of a class collection, or None for a scalar property.
    /// Fixed arrays use their declared labels, including inherited ranges.
    pub(super) fn rand_child_collection_keys(
        &self,
        handle: usize,
        prop: &str,
    ) -> Option<Vec<String>> {
        let scoped = format!("{}#{}", handle, prop);
        let mut cls = self
            .heap
            .get(handle)?
            .as_ref()
            .map(|o| o.class_name.as_str());
        while let Some(name) = cls {
            let cd = self.module.classes.get(name)?;
            if let Some(&(lo, hi, _)) = cd.array_properties.get(prop) {
                return Some((lo..=hi).map(|i| format!("{}[{}]", scoped, i)).collect());
            }
            if cd.array_nd_properties.contains_key(prop) {
                let mut keys = self.signals.keys_with_elem_prefix(&format!("{}[", scoped));
                keys.retain(|k| k.ends_with(']'));
                keys.sort();
                return Some(keys);
            }
            if cd.assoc_properties.contains_key(prop) {
                return Some(
                    self.assoc_key_strs(&scoped)
                        .into_iter()
                        .map(|k| format!("{}[{}]", scoped, k))
                        .collect(),
                );
            }
            if cd.queue_properties.contains_key(prop) {
                return Some(
                    (0..self.get_queue_size(&scoped))
                        .map(|i| format!("{}[{}]", scoped, i))
                        .collect(),
                );
            }
            if cd.properties.contains_key(prop) {
                return None;
            }
            cls = cd.extends.as_deref();
        }
        None
    }

    /// §18.5.9: enumerate reachable rand objects once, including elements of
    /// handle collections. Shared handles and cycles must not duplicate work.
    pub(super) fn rand_child_handles(&self, handle: usize, props: &[String]) -> Vec<usize> {
        if props.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut seen = HashSet::default();
        seen.insert(handle);
        let mut pending = vec![(handle, props.to_vec())];
        while let Some((owner, members)) = pending.pop() {
            for prop in members {
                if self.prop_class_type(owner, &prop).is_none() {
                    continue;
                }
                let values: Vec<Value> = match self.rand_child_collection_keys(owner, &prop) {
                    Some(keys) => keys.iter().filter_map(|k| self.read_coll_elem(k)).collect(),
                    None => self
                        .heap
                        .get(owner)
                        .and_then(|o| o.as_ref())
                        .and_then(|o| o.properties.get(&prop))
                        .cloned()
                        .into_iter()
                        .collect(),
                };
                for v in values {
                    let sub = v.to_u64().unwrap_or(0) as usize;
                    if sub != 0
                        && self.heap.get(sub).and_then(|o| o.as_ref()).is_some()
                        && seen.insert(sub)
                    {
                        out.push(sub);
                        pending.push((sub, self.active_child_rand_members(sub)));
                    }
                }
            }
        }
        out
    }

    pub(super) fn snapshot_rand_children(&self, handles: &[usize]) -> Vec<RandChildSnapshot> {
        handles
            .iter()
            .map(|&handle| {
                let mut properties = Vec::new();
                let mut collections = Vec::new();
                for prop in self.active_child_rand_members(handle) {
                    if self.rand_child_collection_keys(handle, &prop).is_some() {
                        let scoped = format!("{}#{}", handle, prop);
                        let mut keys = self.signals.keys_with_elem_prefix(&format!("{}[", scoped));
                        keys.push(format!("{}.size", scoped));
                        let saved = keys
                            .into_iter()
                            .filter_map(|key| self.signals.get(&key).cloned().map(|v| (key, v)))
                            .collect();
                        collections.push((scoped, saved));
                    } else {
                        let mut keys = vec![prop.clone()];
                        // Unpacked rand aggregates use separate property
                        // slots for their declared rand/randc members.
                        if let Some(tn) = self.class_prop_type_name(handle, &prop)
                            && let Some(dt) = self.module.typedef_types.get(&tn)
                            && let DataType::Struct(su) =
                                Self::resolve_type_ref(dt, &self.module.typedef_types)
                            && Self::spreads_member_wise(&su)
                        {
                            for member in &su.members {
                                if member.rand_qualifier.is_some() {
                                    keys.extend(
                                        member
                                            .declarators
                                            .iter()
                                            .map(|d| format!("{}.{}", prop, d.name.name)),
                                    );
                                }
                            }
                        }
                        for key in keys {
                            let value = self
                                .heap
                                .get(handle)
                                .and_then(|o| o.as_ref())
                                .and_then(|o| o.properties.get(&key))
                                .cloned();
                            properties.push((key, value));
                        }
                    }
                }
                RandChildSnapshot {
                    handle,
                    properties,
                    collections,
                }
            })
            .collect()
    }

    pub(super) fn restore_rand_children(&mut self, saved: Vec<RandChildSnapshot>) {
        for snap in saved {
            if let Some(Some(obj)) = self.heap.get_mut(snap.handle) {
                for (prop, value) in snap.properties {
                    match value {
                        Some(v) => {
                            obj.properties.insert(prop, v);
                        }
                        None => {
                            obj.properties.remove(&prop);
                        }
                    }
                }
            }
            for (scoped, values) in snap.collections {
                for key in self.signals.keys_with_elem_prefix(&format!("{}[", scoped)) {
                    self.signals.remove(&key);
                }
                self.signals.remove(&format!("{}.size", scoped));
                for (key, value) in values {
                    self.write_coll_elem(&key, value);
                }
            }
        }
    }

    /// A callback's explicit writes are not solver draws. Preserve just its
    /// changed rand slots in enclosing transactions, even if a solve fails.
    pub(super) fn exec_pre_randomize_preserving_children(&mut self, handle: usize) {
        let tracked = self.rand_child_snapshots.iter().any(|transaction| {
            transaction
                .children
                .iter()
                .any(|snapshot| snapshot.handle == handle)
        });
        let before = if tracked {
            let members = self.active_child_rand_members(handle);
            let mut handles = self.rand_child_handles(handle, &members);
            handles.push(handle);
            self.snapshot_rand_children(&handles)
        } else {
            Vec::new()
        };
        self.exec_method_call(handle, "pre_randomize", &[]);
        // A callback may replace a rand handle. Its new graph joins the
        // transaction at the post-callback state, before any solver draws.
        if tracked {
            let members = self.active_child_rand_members(handle);
            let reachable = self.rand_child_handles(handle, &members);
            let added = self.snapshot_rand_children(&reachable);
            for transaction in &mut self.rand_child_snapshots {
                if !transaction
                    .children
                    .iter()
                    .any(|snapshot| snapshot.handle == handle)
                {
                    continue;
                }
                for snapshot in &added {
                    if snapshot.handle != transaction.owner
                        && !transaction
                            .children
                            .iter()
                            .any(|saved| saved.handle == snapshot.handle)
                    {
                        transaction.children.push(RandChildSnapshot {
                            handle: snapshot.handle,
                            properties: snapshot.properties.clone(),
                            collections: snapshot.collections.clone(),
                        });
                    }
                }
            }
        }
        for prior in before {
            let owner = prior.handle;
            let mut properties = Vec::new();
            for (key, old) in prior.properties {
                let new = self
                    .heap
                    .get(owner)
                    .and_then(|o| o.as_ref())
                    .and_then(|o| o.properties.get(&key))
                    .cloned();
                if old != new {
                    properties.push((key, new));
                }
            }
            let mut collections = Vec::new();
            for (scoped, old) in prior.collections {
                let mut keys = self.signals.keys_with_elem_prefix(&format!("{}[", scoped));
                keys.push(format!("{}.size", scoped));
                keys.extend(old.iter().map(|(key, _)| key.clone()));
                keys.sort();
                keys.dedup();
                let mut changed = Vec::new();
                for key in keys {
                    let before = old.iter().find(|(k, _)| k == &key).map(|(_, v)| v);
                    let after = self.signals.get(&key);
                    if before != after {
                        changed.push((key, after.cloned()));
                    }
                }
                if !changed.is_empty() {
                    collections.push((scoped, changed));
                }
            }
            for transaction in &mut self.rand_child_snapshots {
                for snapshot in transaction
                    .children
                    .iter_mut()
                    .filter(|s| s.handle == owner)
                {
                    for (key, value) in &properties {
                        if let Some((_, saved)) =
                            snapshot.properties.iter_mut().find(|(k, _)| k == key)
                        {
                            *saved = value.clone();
                        }
                    }
                    for (scoped, changed) in &collections {
                        if let Some((_, values)) =
                            snapshot.collections.iter_mut().find(|(s, _)| s == scoped)
                        {
                            for (key, value) in changed {
                                values.retain(|(k, _)| k != key);
                                if let Some(value) = value {
                                    values.push((key.clone(), value.clone()));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Where the root name of an operand binds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Bind {
    /// The randomized object (or a foreach iterator): left to the solver.
    Object,
    /// The scope of the randomize() call.
    Caller,
    /// Neither: parameters, enum constants, package and module names —
    /// they resolve the same way in either scope.
    Neutral,
}

pub(super) struct CallerScope {
    /// Class of the randomized object.
    obj_class: String,
    /// Plain name of the receiver (`req` in `req.randomize()`), which names
    /// the object itself.
    receiver: Option<String>,
    /// Class of the caller (`this`'s class, or the class of a static method).
    caller_class: Option<String>,
}

impl Simulator {
    /// Rewrite the inline constraints of `obj.randomize() with {…}` so every
    /// caller-scope operand is a literal (see the module docs). Called with
    /// the CALLER's `this` and local frame still active. Returns None when
    /// nothing binds in the caller's scope, so the original items are used.
    pub(super) fn freeze_caller_scope_refs(
        &mut self,
        handle: usize,
        receiver: Option<String>,
        items: &[ConstraintItem],
    ) -> Option<Vec<ConstraintItem>> {
        let obj_class = self.heap.get(handle)?.as_ref()?.class_name.clone();
        let caller_class = match self.this_stack.last().copied().flatten() {
            Some(h) => self
                .heap
                .get(h)
                .and_then(|o| o.as_ref())
                .map(|i| i.class_name.clone()),
            None => self.class_context_stack.last().cloned().flatten(),
        };
        let cx = CallerScope {
            obj_class,
            receiver,
            caller_class,
        };
        let mut bound: Vec<String> = Vec::new();
        if !items
            .iter()
            .any(|it| self.item_has_caller_ref(it, &cx, &mut bound))
        {
            return None;
        }
        let mut out = items.to_vec();
        for it in &mut out {
            self.freeze_item(it, &cx, &mut bound);
        }
        Some(out)
    }

    /// Does the class chain of `class` declare `name` (a property or a
    /// value parameter)?
    fn class_chain_declares(&self, class: Option<&str>, name: &str) -> bool {
        let mut cur = class;
        while let Some(cn) = cur {
            let Some(cd) = self.module.classes.get(cn) else {
                return false;
            };
            if cd.properties.contains_key(name)
                || cd.static_properties.contains(name)
                || cd.queue_properties.contains_key(name)
                || cd.assoc_properties.contains_key(name)
                || cd.param_defaults.iter().any(|(n, _)| n == name)
            {
                return true;
            }
            cur = cd.extends.as_deref();
        }
        false
    }

    fn bind_of(&self, h: &HierarchicalIdentifier, cx: &CallerScope, bound: &[String]) -> Bind {
        let Some(first) = h.path.first() else {
            return Bind::Neutral;
        };
        if h.root.as_deref() == Some(LOCAL_SCOPE_ROOT) {
            return Bind::Caller;
        }
        if h.root.is_some() {
            return Bind::Neutral;
        }
        let n = first.name.name.as_str();
        if n == "this"
            || n == "super"
            || bound.iter().any(|b| b == n)
            || cx.receiver.as_deref() == Some(n)
            || self.class_chain_declares(Some(&cx.obj_class), n)
        {
            return Bind::Object;
        }
        if self.local_stack.last().is_some_and(|f| f.contains_key(n))
            || self.dyn_name_lookup_innermost(n).is_some()
            || self.class_chain_declares(cx.caller_class.as_deref(), n)
        {
            return Bind::Caller;
        }
        Bind::Neutral
    }

    /// Does `e` read any caller-scope name (read-only pre-scan)?
    fn expr_has_caller_ref(&self, e: &Expression, cx: &CallerScope, bound: &[String]) -> bool {
        let mut hit = false;
        Self::walk_operands(e, &mut |x| {
            if let ExprKind::Ident(h) = &x.kind {
                if self.bind_of(h, cx, bound) == Bind::Caller {
                    hit = true;
                }
            }
        });
        hit
    }

    /// Visit `e` and every sub-expression, including select expressions held
    /// on identifier path segments.
    pub(super) fn walk_operands(e: &Expression, f: &mut dyn FnMut(&Expression)) {
        f(e);
        let mut sub = |x: &Expression| Self::walk_operands(x, f);
        match &e.kind {
            ExprKind::Ident(h) => {
                for seg in &h.path {
                    seg.selects.iter().for_each(&mut sub);
                }
            }
            ExprKind::Unary { operand, .. } => sub(operand),
            ExprKind::Binary { left, right, .. } => {
                sub(left);
                sub(right);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                sub(condition);
                sub(then_expr);
                sub(else_expr);
            }
            ExprKind::Concatenation(parts) => parts.iter().for_each(&mut sub),
            ExprKind::Replication { count, exprs } => {
                sub(count);
                exprs.iter().for_each(&mut sub);
            }
            ExprKind::Call { func, args } => {
                sub(func);
                args.iter().for_each(&mut sub);
            }
            ExprKind::SystemCall { args, .. } => args.iter().for_each(&mut sub),
            ExprKind::Inside { expr, ranges } => {
                sub(expr);
                ranges.iter().for_each(&mut sub);
            }
            ExprKind::MemberAccess { expr, .. } | ExprKind::Paren(expr) => sub(expr),
            ExprKind::Index { expr, index } => {
                sub(expr);
                sub(index);
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                sub(expr);
                sub(left);
                sub(right);
            }
            ExprKind::Range(a, b) => {
                sub(a);
                sub(b);
            }
            _ => {}
        }
    }

    fn item_has_caller_ref(
        &self,
        it: &ConstraintItem,
        cx: &CallerScope,
        bound: &mut Vec<String>,
    ) -> bool {
        match it {
            ConstraintItem::Expr(e) => self.expr_has_caller_ref(e, cx, bound),
            ConstraintItem::Inside {
                expr,
                range,
                dist_weights,
                ..
            } => {
                self.expr_has_caller_ref(expr, cx, bound)
                    || range.iter().any(|r| match r {
                        ConstraintRange::Value(v) => self.expr_has_caller_ref(v, cx, bound),
                        ConstraintRange::Range { lo, hi } => {
                            self.expr_has_caller_ref(lo, cx, bound)
                                || self.expr_has_caller_ref(hi, cx, bound)
                        }
                    })
                    || dist_weights.iter().flatten().any(|w| match w {
                        DistWeight::Each(e) | DistWeight::Total(e) => {
                            self.expr_has_caller_ref(e, cx, bound)
                        }
                    })
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => {
                self.expr_has_caller_ref(condition, cx, bound)
                    || self.item_has_caller_ref(constraint, cx, bound)
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                self.expr_has_caller_ref(condition, cx, bound)
                    || self.item_has_caller_ref(then_item, cx, bound)
                    || else_item
                        .as_ref()
                        .is_some_and(|e| self.item_has_caller_ref(e, cx, bound))
            }
            ConstraintItem::Foreach {
                array, vars, item, ..
            } => {
                if self.expr_has_caller_ref(array, cx, bound) {
                    return true;
                }
                let depth = bound.len();
                bound.extend(vars.iter().flatten().map(|v| v.name.clone()));
                let r = self.item_has_caller_ref(item, cx, bound);
                bound.truncate(depth);
                r
            }
            ConstraintItem::Soft(inner) => self.item_has_caller_ref(inner, cx, bound),
            ConstraintItem::Block(items) => {
                items.iter().any(|i| self.item_has_caller_ref(i, cx, bound))
            }
            ConstraintItem::Unique { exprs, .. } => {
                exprs.iter().any(|e| self.expr_has_caller_ref(e, cx, bound))
            }
            ConstraintItem::Solve { .. } => false,
        }
    }

    fn freeze_item(&mut self, it: &mut ConstraintItem, cx: &CallerScope, bound: &mut Vec<String>) {
        match it {
            ConstraintItem::Expr(e) => self.freeze_expr(e, cx, bound),
            ConstraintItem::Inside {
                expr,
                range,
                is_dist,
                dist_weights,
                ..
            } => {
                self.freeze_expr(expr, cx, bound);
                let mut out: Vec<ConstraintRange> = Vec::with_capacity(range.len());
                for mut r in std::mem::take(range) {
                    match &mut r {
                        ConstraintRange::Value(v) => {
                            if !*is_dist {
                                if let Some(elems) = self.caller_set_elems(v, cx, bound) {
                                    out.extend(elems.into_iter().map(ConstraintRange::Value));
                                    continue;
                                }
                            }
                            self.freeze_expr(v, cx, bound);
                        }
                        ConstraintRange::Range { lo, hi } => {
                            self.freeze_expr(lo, cx, bound);
                            self.freeze_expr(hi, cx, bound);
                        }
                    }
                    out.push(r);
                }
                *range = out;
                for w in dist_weights.iter_mut().flatten() {
                    match w {
                        DistWeight::Each(e) | DistWeight::Total(e) => {
                            self.freeze_expr(e, cx, bound)
                        }
                    }
                }
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => {
                self.freeze_expr(condition, cx, bound);
                self.freeze_item(constraint, cx, bound);
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                self.freeze_expr(condition, cx, bound);
                self.freeze_item(then_item, cx, bound);
                if let Some(e) = else_item {
                    self.freeze_item(e, cx, bound);
                }
            }
            ConstraintItem::Foreach {
                array, vars, item, ..
            } => {
                if let Some(iters) = self.unroll_caller_foreach(array, vars, item, cx, bound) {
                    let mut block = ConstraintItem::Block(iters);
                    self.freeze_item(&mut block, cx, bound);
                    *it = block;
                    return;
                }
                self.freeze_expr(array, cx, bound);
                let depth = bound.len();
                bound.extend(vars.iter().flatten().map(|v| v.name.clone()));
                self.freeze_item(item, cx, bound);
                bound.truncate(depth);
            }
            ConstraintItem::Soft(inner) => self.freeze_item(inner, cx, bound),
            ConstraintItem::Block(items) => {
                for i in items {
                    self.freeze_item(i, cx, bound);
                }
            }
            ConstraintItem::Unique { exprs, .. } => {
                for e in exprs {
                    self.freeze_expr(e, cx, bound);
                }
            }
            ConstraintItem::Solve { .. } => {}
        }
    }

    /// Where an OPERAND binds as a whole: `Some(true)` when it reads a
    /// caller-scope name and nothing of the object, `Some(false)` when it
    /// reads neither, None when it reads the object (or has a shape that
    /// cannot be evaluated ahead of the solve).
    fn operand_bind(&self, e: &Expression, cx: &CallerScope, bound: &[String]) -> Option<bool> {
        let all = |xs: &[&Expression]| -> Option<bool> {
            let mut any = false;
            for x in xs {
                any |= self.operand_bind(x, cx, bound)?;
            }
            Some(any)
        };
        match &e.kind {
            ExprKind::Number(_) | ExprKind::StringLiteral(_) | ExprKind::TypeLiteral(_) => {
                Some(false)
            }
            ExprKind::Ident(h) => {
                let own = match self.bind_of(h, cx, bound) {
                    Bind::Object => return None,
                    Bind::Caller => true,
                    Bind::Neutral => false,
                };
                let sels: Vec<&Expression> = h.path.iter().flat_map(|s| s.selects.iter()).collect();
                let sel = all(&sels)?;
                Some(own || sel)
            }
            ExprKind::Index { expr, index } => all(&[expr.as_ref(), index.as_ref()]),
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => all(&[expr.as_ref(), left.as_ref(), right.as_ref()]),
            ExprKind::MemberAccess { expr, .. } | ExprKind::Paren(expr) => {
                self.operand_bind(expr, cx, bound)
            }
            ExprKind::Unary { operand, .. } => self.operand_bind(operand, cx, bound),
            ExprKind::Binary { left, right, .. } => all(&[left.as_ref(), right.as_ref()]),
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => all(&[condition.as_ref(), then_expr.as_ref(), else_expr.as_ref()]),
            ExprKind::Concatenation(parts) => all(&parts.iter().collect::<Vec<_>>()),
            ExprKind::SystemCall { args, .. } => all(&args.iter().collect::<Vec<_>>()),
            // A method of a caller-scope object (`q.size()`, `cfg.get()`);
            // a bare call binds in the object's class first, so it stays.
            ExprKind::Call { func, args } => {
                let recv = match &func.kind {
                    ExprKind::MemberAccess { expr, .. } => self.operand_bind(expr, cx, bound)?,
                    ExprKind::Ident(h) if h.path.len() > 1 => match self.bind_of(h, cx, bound) {
                        Bind::Object => return None,
                        Bind::Caller => true,
                        Bind::Neutral => false,
                    },
                    _ => return None,
                };
                Some(recv || all(&args.iter().collect::<Vec<_>>())?)
            }
            _ => None,
        }
    }

    /// Replace each maximal caller-scope operand of `e` with a literal.
    /// Operators are kept (not folded), so the context-determined width
    /// rules of §11.6 still apply to the frozen operands.
    fn freeze_expr(&mut self, e: &mut Expression, cx: &CallerScope, bound: &[String]) {
        let operand = matches!(
            e.kind,
            ExprKind::Ident(_)
                | ExprKind::Index { .. }
                | ExprKind::RangeSelect { .. }
                | ExprKind::MemberAccess { .. }
                | ExprKind::Call { .. }
                | ExprKind::SystemCall { .. }
        );
        if operand && self.operand_bind(e, cx, bound) == Some(true) {
            // A collection stays live while its foreach index changes. Bind
            // its storage in caller scope rather than leaving a caller handle
            // to be looked up after the solver switches to object scope.
            if matches!(e.kind, ExprKind::Ident(_) | ExprKind::MemberAccess { .. }) {
                if let Some(name) = self.array_operand_name(e) {
                    *e = Self::chain_expr(&[name], e.span);
                    return;
                }
            }
            let v = self.eval_expr(e);
            *e = Self::literal_of(&v, e.span);
            return;
        }
        match &mut e.kind {
            ExprKind::Ident(h) => {
                // `local::name` whose value has no literal form: bind the
                // plain name (the object never shadows it at this point).
                if h.root.as_deref() == Some(LOCAL_SCOPE_ROOT) {
                    h.root = None;
                }
                for seg in &mut h.path {
                    for s in &mut seg.selects {
                        self.freeze_expr(s, cx, bound);
                    }
                }
            }
            ExprKind::Unary { operand, .. } => self.freeze_expr(operand, cx, bound),
            ExprKind::Binary { left, right, .. } => {
                self.freeze_expr(left, cx, bound);
                self.freeze_expr(right, cx, bound);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.freeze_expr(condition, cx, bound);
                self.freeze_expr(then_expr, cx, bound);
                self.freeze_expr(else_expr, cx, bound);
            }
            ExprKind::Concatenation(parts) => {
                for p in parts {
                    self.freeze_expr(p, cx, bound);
                }
            }
            ExprKind::Replication { count, exprs } => {
                self.freeze_expr(count, cx, bound);
                for x in exprs {
                    self.freeze_expr(x, cx, bound);
                }
            }
            ExprKind::Call { func, args } => {
                if let ExprKind::MemberAccess { expr, .. } = &mut func.kind {
                    self.freeze_expr(expr, cx, bound);
                }
                for a in args {
                    self.freeze_expr(a, cx, bound);
                }
            }
            ExprKind::SystemCall { args, .. } => {
                for a in args {
                    self.freeze_expr(a, cx, bound);
                }
            }
            ExprKind::Inside { expr, ranges } => {
                self.freeze_expr(expr, cx, bound);
                let mut out: Vec<Expression> = Vec::with_capacity(ranges.len());
                for mut r in std::mem::take(ranges) {
                    if let Some(elems) = self.caller_set_elems(&r, cx, bound) {
                        out.extend(elems);
                        continue;
                    }
                    self.freeze_expr(&mut r, cx, bound);
                    out.push(r);
                }
                *ranges = out;
            }
            ExprKind::MemberAccess { expr, .. } | ExprKind::Paren(expr) => {
                self.freeze_expr(expr, cx, bound)
            }
            ExprKind::Index { expr, index } => {
                self.freeze_expr(expr, cx, bound);
                self.freeze_expr(index, cx, bound);
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                self.freeze_expr(expr, cx, bound);
                self.freeze_expr(left, cx, bound);
                self.freeze_expr(right, cx, bound);
            }
            ExprKind::Range(a, b) => {
                self.freeze_expr(a, cx, bound);
                self.freeze_expr(b, cx, bound);
            }
            _ => {}
        }
    }

    /// §11.4.13: a caller-scope array/queue named in an `inside` set stands
    /// for its elements; return them as literals (none for an empty one).
    /// None when `r` is not such a collection.
    fn caller_set_elems(
        &mut self,
        r: &Expression,
        cx: &CallerScope,
        bound: &[String],
    ) -> Option<Vec<Expression>> {
        if !matches!(r.kind, ExprKind::Ident(_) | ExprKind::MemberAccess { .. })
            || self.operand_bind(r, cx, bound) != Some(true)
        {
            return None;
        }
        let nm = self.array_operand_name(r)?;
        let n = self.get_queue_size(&nm);
        let mut out = Vec::with_capacity(n as usize);
        for i in 0..n {
            let v = self.get_signal_value_by_name(&format!("{}[{}]", nm, i))?;
            out.push(Self::literal_of(&v, r.span));
        }
        Some(out)
    }

    /// A literal expression that evaluates to exactly `v` (width, sign and
    /// every x/z bit included).
    pub(super) fn literal_of(v: &Value, span: crate::ast::Span) -> Expression {
        if v.is_real {
            return Expression::new(ExprKind::Number(NumberLiteral::Real(v.to_f64())), span);
        }
        let w = v.width.max(1);
        let mut digits = String::with_capacity(w as usize);
        for i in (0..w as usize).rev() {
            digits.push(match v.get_bit(i) {
                LogicBit::Zero => '0',
                LogicBit::One => '1',
                LogicBit::X => 'x',
                LogicBit::Z => 'z',
            });
        }
        let cached = if w <= 64 {
            v.resize(w).inline_bits().map(|(b, x)| (b, x, w))
        } else {
            None
        };
        Expression::new(
            ExprKind::Number(NumberLiteral::Integer {
                size: Some(w),
                signed: v.is_signed,
                base: NumberBase::Binary,
                value: digits,
                cached_val: crate::ast::cache::LiteralCache::new(cached),
            }),
            span,
        )
    }
}

/// What `rebase_expr` strips from an operand path: `prop.` for a rand handle
/// member, or `prop[elem].` for one element of a rand handle collection.
struct Rebase<'a> {
    prop: &'a str,
    /// Plain name of the randomize() receiver (`<receiver>.prop.…`).
    recv: Option<&'a str>,
    /// `Some(k)`: rebase onto element `k` of the collection `prop`.
    elem: Option<u64>,
    /// The `foreach (prop[i])` iterator, bound to `elem` (§18.5.8.1).
    iter: Option<&'a str>,
}

impl Simulator {
    /// §18.5.9 — the constraint items of an enclosing solve that constrain
    /// ONLY the rand sub-object behind member `prop`, rewritten relative to
    /// that object (`ctrl.fld.value inside {…}` becomes `fld.value inside
    /// {…}` for `ctrl`) so the sub-object's own solve honours them. The
    /// enclosing solve can only re-pin a nested member by `==`; a relational,
    /// `inside` or deeper-nested item was left to chance and the acceptance
    /// check, so it failed every trial. An item reading anything but `prop.…`
    /// paths and literals stays with the enclosing solve only.
    pub(super) fn pushdown_items(
        &self,
        prop: &str,
        constraints: &[ClassConstraint],
    ) -> Vec<ConstraintItem> {
        self.pushdown_into(prop, None, constraints)
    }

    /// §18.4/§18.5.9 — the same for element `elem` of a rand collection of
    /// object handles (`rand obj arr[N]`): `arr[elem].fld …` items, and the
    /// body of `foreach (arr[i])` with `i` bound to `elem`, constrain only
    /// that element's object.
    pub(super) fn pushdown_elem_items(
        &self,
        prop: &str,
        elem: u64,
        constraints: &[ClassConstraint],
    ) -> Vec<ConstraintItem> {
        self.pushdown_into(prop, Some(elem), constraints)
    }

    fn pushdown_into(
        &self,
        prop: &str,
        elem: Option<u64>,
        constraints: &[ClassConstraint],
    ) -> Vec<ConstraintItem> {
        let t = Rebase {
            prop,
            recv: self.rand_receiver.as_deref(),
            elem,
            iter: None,
        };
        let mut out = Vec::new();
        for con in constraints {
            for it in &con.items {
                Self::pushdown_item(it, &t, &mut out);
            }
        }
        out
    }

    fn pushdown_item(it: &ConstraintItem, t: &Rebase<'_>, out: &mut Vec<ConstraintItem>) {
        if let ConstraintItem::Block(items) = it {
            for i in items {
                Self::pushdown_item(i, t, out);
            }
            return;
        }
        // `foreach (prop[i]) body` over the collection itself: the body, with
        // `i` bound to this element, is a candidate item for the element.
        if let (
            Some(_),
            ConstraintItem::Foreach {
                array, vars, item, ..
            },
        ) = (t.elem, it)
        {
            let over_prop = matches!(&array.kind, ExprKind::Ident(h)
                if h.root.is_none()
                    && h.path.len() == 1
                    && h.path[0].selects.is_empty()
                    && h.path[0].name.name == t.prop);
            if let ([Some(iter)], true) = (vars.as_slice(), over_prop) {
                let ti = Rebase {
                    iter: Some(&iter.name),
                    ..*t
                };
                Self::pushdown_item(item, &ti, out);
            }
            return;
        }
        let mut c = it.clone();
        let mut hit = false;
        if Self::rebase_item(&mut c, t, &mut hit) && hit {
            out.push(c);
        }
    }

    fn rebase_item(it: &mut ConstraintItem, t: &Rebase<'_>, hit: &mut bool) -> bool {
        match it {
            ConstraintItem::Expr(e) => Self::rebase_expr(e, t, hit),
            ConstraintItem::Inside { expr, range, .. } => {
                Self::rebase_expr(expr, t, hit)
                    && range.iter_mut().all(|r| match r {
                        ConstraintRange::Value(v) => Self::rebase_expr(v, t, hit),
                        ConstraintRange::Range { lo, hi } => {
                            Self::rebase_expr(lo, t, hit) && Self::rebase_expr(hi, t, hit)
                        }
                    })
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => Self::rebase_expr(condition, t, hit) && Self::rebase_item(constraint, t, hit),
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                Self::rebase_expr(condition, t, hit)
                    && Self::rebase_item(then_item, t, hit)
                    && else_item
                        .as_mut()
                        .is_none_or(|e| Self::rebase_item(e, t, hit))
            }
            ConstraintItem::Soft(inner) => Self::rebase_item(inner, t, hit),
            ConstraintItem::Block(items) => items.iter_mut().all(|i| Self::rebase_item(i, t, hit)),
            _ => false,
        }
    }

    /// Strip `prop.` (or `<receiver>.prop.`) from every operand path of `e`
    /// (`prop.x.y` parses as a member-access chain over `prop`); false when
    /// an operand is anything else a sub-object solve could not read the
    /// same way (an enclosing member, a call, `this`). For an element
    /// rebase the prefix is `prop[elem].`, and a bound foreach iterator
    /// becomes the element's index.
    fn rebase_expr(e: &mut Expression, t: &Rebase<'_>, hit: &mut bool) -> bool {
        if let Some(k) = t.elem {
            if let ExprKind::Ident(h) = &e.kind
                && t.iter.is_some_and(|i| {
                    h.root.is_none()
                        && h.path.len() == 1
                        && h.path[0].selects.is_empty()
                        && h.path[0].name.name == i
                })
            {
                *e = Self::literal_of(&Value::from_u64(k, 32), e.span);
                return true;
            }
            if let Some(rest) = Self::elem_path_rest(e, t) {
                return match rest {
                    Some(names) => {
                        *e = Self::chain_expr(&names, e.span);
                        *hit = true;
                        true
                    }
                    None => false,
                };
            }
        }
        if matches!(e.kind, ExprKind::Ident(_) | ExprKind::MemberAccess { .. }) {
            // A plain `prop.x` path names no single element of a collection.
            if t.elem.is_some() {
                return false;
            }
            let Some(names) = Self::member_chain(e) else {
                return Self::rebase_selected_chain(e, t, hit);
            };
            // `prop` is one name, or `struct_prop.member` for a handle
            // member of an unpacked-struct property (§18.4).
            let n = t.prop.split('.').count();
            let at = |from: usize| {
                names.len() > from + n
                    && t.prop
                        .split('.')
                        .zip(&names[from..])
                        .all(|(p, name)| p == name.as_str())
            };
            let skip = usize::from(t.recv.is_some_and(|r| names[0] == r && at(1)));
            if !at(skip) {
                return false;
            }
            *e = Self::chain_expr(&names[skip + n..], e.span);
            *hit = true;
            return true;
        }
        match &mut e.kind {
            ExprKind::Number(_) | ExprKind::StringLiteral(_) => true,
            ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                Self::rebase_expr(operand, t, hit)
            }
            ExprKind::Binary { left, right, .. }
            | ExprKind::Range(left, right)
            | ExprKind::Index {
                expr: left,
                index: right,
            } => Self::rebase_expr(left, t, hit) && Self::rebase_expr(right, t, hit),
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::rebase_expr(condition, t, hit)
                    && Self::rebase_expr(then_expr, t, hit)
                    && Self::rebase_expr(else_expr, t, hit)
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                Self::rebase_expr(expr, t, hit)
                    && Self::rebase_expr(left, t, hit)
                    && Self::rebase_expr(right, t, hit)
            }
            ExprKind::Concatenation(parts) => {
                parts.iter_mut().all(|p| Self::rebase_expr(p, t, hit))
            }
            ExprKind::Inside { expr, ranges } => {
                Self::rebase_expr(expr, t, hit)
                    && ranges.iter_mut().all(|r| Self::rebase_expr(r, t, hit))
            }
            // A method of a member below `prop` (`prop.r.sum()`): the receiver
            // is rebased and the call kept, so the sub-object's own solve
            // reads `r.sum()` of its array.
            ExprKind::Call { func, args } => {
                let ExprKind::MemberAccess { expr, .. } = &mut func.kind else {
                    return false;
                };
                Self::rebase_expr(expr, t, hit)
                    && args.iter_mut().all(|a| Self::rebase_expr(a, t, hit))
            }
            _ => false,
        }
    }

    /// §18.5.9: `prop.g.item[1].key` — a path below `prop` that selects an
    /// element on the way (an unrolled `foreach` over a member array of the
    /// sub-object) — rebased like a plain chain, keeping its selects. Only
    /// constant selects qualify: an index naming a variable of this object
    /// could not be read in the sub-object's solve.
    fn rebase_selected_chain(e: &mut Expression, t: &Rebase<'_>, hit: &mut bool) -> bool {
        fn constant(x: &Expression) -> bool {
            match &x.kind {
                ExprKind::Number(_) => true,
                ExprKind::Paren(i) | ExprKind::Unary { operand: i, .. } => constant(i),
                ExprKind::Binary { left, right, .. } => constant(left) && constant(right),
                _ => false,
            }
        }
        match &mut e.kind {
            ExprKind::Ident(h) => {
                let skip = usize::from(t.recv.is_some_and(|r| {
                    h.path.len() >= 3 && h.path[0].name.name == r && h.path[1].name.name == t.prop
                }));
                if h.root.is_some()
                    || h.path.len() < skip + 2
                    || h.path[skip].name.name != t.prop
                    || h.path[..=skip].iter().any(|s| !s.selects.is_empty())
                    || !h.path[skip + 1..]
                        .iter()
                        .all(|s| s.selects.iter().all(constant))
                {
                    return false;
                }
                h.path.drain(..=skip);
                h.cached_signal_id = crate::ast::cache::SignalCache::new(None);
                h.cached_resolved_name = crate::ast::cache::NameCache::new();
                *hit = true;
                true
            }
            // `prop.g.item[1].key` parsed as a member access over an index:
            // the index operand is rebased (a constant stays as it is).
            ExprKind::MemberAccess { expr, .. } => Self::rebase_expr(expr, t, hit),
            _ => false,
        }
    }

    /// An operand path through an element of the collection `t.prop`
    /// (`prop[idx].a.b`, or `<receiver>.prop[idx].a.b`). `None`: not such a
    /// path. `Some(Some(rest))`: it names element `t.elem` — `rest` is the
    /// member chain below it (`[a, b]`). `Some(None)`: it names another
    /// element, an index unknown before the solve, or the element handle
    /// itself, so it cannot be read inside that element's solve.
    fn elem_path_rest(e: &Expression, t: &Rebase<'_>) -> Option<Option<Vec<String>>> {
        let k = t.elem?;
        let idx_is_elem = |idx: &Expression| {
            let iter = matches!(&idx.kind, ExprKind::Ident(h)
                if t.iter.is_some_and(|i| h.root.is_none()
                    && h.path.len() == 1
                    && h.path[0].selects.is_empty()
                    && h.path[0].name.name == i));
            iter || Self::try_const_u64(idx) == Some(k)
        };
        // `prop[idx].a.b` as one identifier path whose first segment carries
        // the select.
        if let ExprKind::Ident(h) = &e.kind {
            let first = h.path.first()?;
            if h.root.is_some()
                || first.name.name != t.prop
                || first.selects.len() != 1
                || h.path[1..].iter().any(|s| !s.selects.is_empty())
            {
                return None;
            }
            if h.path.len() < 2 || !idx_is_elem(&first.selects[0]) {
                return Some(None);
            }
            return Some(Some(
                h.path[1..].iter().map(|s| s.name.name.clone()).collect(),
            ));
        }
        // `prop[idx].a.b` as a member-access chain over an index expression.
        let mut names = Vec::new();
        let mut cur = e;
        while let ExprKind::MemberAccess { expr, member } = &cur.kind {
            names.push(member.name.clone());
            cur = expr;
        }
        let ExprKind::Index { expr: arr, index } = &cur.kind else {
            return None;
        };
        let names_arr = Self::member_chain(arr)?;
        let on_prop = names_arr == [t.prop]
            || t.recv.is_some_and(|r| {
                names_arr.len() == 2 && names_arr[0] == r && names_arr[1] == t.prop
            });
        if !on_prop {
            return None;
        }
        if names.is_empty() || !idx_is_elem(index) {
            return Some(None);
        }
        names.reverse();
        Some(Some(names))
    }
}

impl Simulator {
    /// The names of a plain operand path: an identifier (no selects, no
    /// root) or a member-access chain over one. None for anything else.
    pub(super) fn member_chain(e: &Expression) -> Option<Vec<String>> {
        match &e.kind {
            ExprKind::Ident(h) => {
                if h.root.is_some() || h.path.iter().any(|s| !s.selects.is_empty()) {
                    return None;
                }
                Some(h.path.iter().map(|s| s.name.name.clone()).collect())
            }
            ExprKind::MemberAccess { expr, member } => {
                let mut v = Self::member_chain(expr)?;
                v.push(member.name.clone());
                Some(v)
            }
            _ => None,
        }
    }

    /// Rebuild `a.b.c` as the parser shapes it: an identifier `a` under a
    /// member-access chain.
    fn chain_expr(names: &[String], span: crate::ast::Span) -> Expression {
        let ident = |n: &str| crate::ast::Identifier {
            name: n.to_string(),
            span,
        };
        let mut e = Expression::new(
            ExprKind::Ident(HierarchicalIdentifier {
                root: None,
                path: vec![crate::ast::expr::HierPathSegment {
                    name: ident(&names[0]),
                    selects: Vec::new(),
                }],
                span,
                cached_signal_id: crate::ast::cache::SignalCache::new(None),
                cached_resolved_name: crate::ast::cache::NameCache::new(),
                bound: crate::ast::binding::Bound::UNSET,
            }),
            span,
        );
        for n in &names[1..] {
            e = Expression::new(
                ExprKind::MemberAccess {
                    expr: Box::new(e),
                    member: ident(n),
                },
                span,
            );
        }
        e
    }
}

impl Simulator {
    /// §18.5.6/§18.5.7 for `std::randomize(…) with {…}`: a `->` or `if`
    /// whose condition reads no random variable is decided by the state
    /// alone, so it is resolved before solving — to its consequent (or
    /// branch) when the condition holds and to nothing otherwise. The
    /// per-variable solver below never reasons about a guard, so a guard on
    /// a state variable (`dbg -> (x == 0)`, `if (mode) {…} else {…}`) left
    /// every guarded constraint unenforced and the call failed its check.
    /// `rand_roots` names the random variables; `foreach` indices and a
    /// `with` iterator are treated as random (not yet known) too.
    pub(super) fn fold_state_guards(
        &mut self,
        items: &[ConstraintItem],
        rand_roots: &HashSet<String>,
    ) -> Vec<ConstraintItem> {
        let mut bound: Vec<String> = Vec::new();
        items
            .iter()
            .filter_map(|it| self.fold_state_guard_item(it, rand_roots, &mut bound))
            .collect()
    }

    fn reads_unknown(e: &Expression, rand_roots: &HashSet<String>, bound: &[String]) -> bool {
        let mut hit = false;
        Self::walk_operands(e, &mut |x| {
            if let ExprKind::Ident(h) = &x.kind {
                if let Some(first) = h.path.first() {
                    let n = first.name.name.as_str();
                    if rand_roots.contains(n)
                        || bound.iter().any(|b| b == n)
                        || n == "item"
                        || n == "index"
                    {
                        hit = true;
                    }
                }
            }
        });
        hit
    }

    /// Is `e` a state condition that holds now? None when it reads a random
    /// variable (or cannot be judged), so the guard stays.
    fn state_condition(
        &mut self,
        e: &Expression,
        rand_roots: &HashSet<String>,
        bound: &[String],
    ) -> Option<bool> {
        if Self::reads_unknown(e, rand_roots, bound) {
            return None;
        }
        // §11.4.7: an x/z condition is not true.
        Some(self.eval_expr(e).is_true())
    }

    /// §7.4.1/§18.5.8.1: the declared dimensions (unpacked, then packed) of
    /// a `foreach` array operand, known before the solve even when its
    /// values are random — including a row selected by an enclosing loop
    /// (`grid[0]`) and a member reached through a struct or a sub-object
    /// (`g.item`, `a.g.item`). None for a queue, dynamic or associative array.
    fn fixed_constraint_foreach_dims(&self, array: &Expression) -> Option<Vec<(i64, i64)>> {
        if let ExprKind::Ident(h) = &array.kind
            && let Some(last) = h.path.last()
            && !last.selects.is_empty()
        {
            let count = last.selects.len();
            let mut h = h.clone();
            h.path.last_mut()?.selects.clear();
            let base = Expression::new(ExprKind::Ident(h), array.span);
            let dims = self.fixed_constraint_foreach_dims(&base)?;
            return Some(dims.get(count..)?.to_vec());
        }
        if let ExprKind::Index { expr, .. } = &array.kind {
            let mut dims = self.fixed_constraint_foreach_dims(expr)?;
            if dims.is_empty() {
                return None;
            }
            dims.remove(0);
            return Some(dims);
        }
        let shape = self.query_operand_decl_dims(array)?;
        Some(shape.dims)
    }

    /// The iterations of `foreach (array[vars]) body` over the fixed shape of
    /// `array` (see `fixed_constraint_foreach_dims`), each loop variable
    /// replaced by its index; an omitted variable (`grid[, j]`) iterates
    /// nothing of its own (§12.7.3).
    fn unroll_fixed_foreach(
        &self,
        array: &Expression,
        vars: &[Option<crate::ast::Identifier>],
        body: &ConstraintItem,
    ) -> Option<Vec<ConstraintItem>> {
        let dims = self.fixed_constraint_foreach_dims(array)?;
        let dims: Vec<(i64, i64)> = dims
            .get(..vars.len())?
            .iter()
            .zip(vars)
            .map(|(&(l, r), v)| {
                if v.is_some() {
                    (l.min(r), l.max(r))
                } else {
                    (0, 0)
                }
            })
            .collect();
        Self::unroll_foreach_dims(&dims, vars, body)
    }

    /// Whether the solvers already model `foreach (array[vars])` as written:
    /// a plain class property (`a`, `this.a`) with one loop variable per
    /// unpacked dimension, or a packed property iterated by its packed
    /// dimensions.
    fn foreach_shape_modeled(
        &self,
        array: &Expression,
        vars: &[Option<crate::ast::Identifier>],
    ) -> bool {
        let plain = match &array.kind {
            ExprKind::Ident(h) => {
                h.root.is_none()
                    && h.path.iter().all(|s| s.selects.is_empty())
                    && (h.path.len() == 1 || (h.path.len() == 2 && h.path[0].name.name == "this"))
            }
            ExprKind::MemberAccess { expr, .. } => matches!(expr.kind, ExprKind::This),
            _ => false,
        };
        plain
            && self.query_operand_decl_dims(array).is_some_and(|shape| {
                shape.unpacked == 0
                    || (shape.unpacked == vars.len() && vars.iter().all(Option::is_some))
            })
    }

    /// §18.5.8.1: unroll every `foreach` of `item` whose array has a fixed
    /// shape the solvers do not model as written — an array reached through
    /// a struct member or a sub-object (`g.item`, `a.g.item`), a row of a
    /// multi-dimensional array, fewer loop variables than unpacked
    /// dimensions, an omitted variable — so its body is solved like any
    /// other constraint instead of being dropped. `all` unrolls every
    /// fixed-shape loop (for the `randomize(null)` check, which models no
    /// `foreach`). None when nothing changed.
    pub(super) fn expand_fixed_foreach(
        &self,
        item: &ConstraintItem,
        all: bool,
    ) -> Option<ConstraintItem> {
        let recurse = |x: &ConstraintItem| self.expand_fixed_foreach(x, all);
        match item {
            ConstraintItem::Foreach {
                array,
                vars,
                item: body,
                span,
            } => {
                if all || !self.foreach_shape_modeled(array, vars) {
                    if let Some(items) = self.unroll_fixed_foreach(array, vars, body) {
                        return Some(ConstraintItem::Block(
                            items
                                .into_iter()
                                .map(|x| recurse(&x).unwrap_or(x))
                                .collect(),
                        ));
                    }
                }
                Some(ConstraintItem::Foreach {
                    array: array.clone(),
                    vars: vars.clone(),
                    item: Box::new(recurse(body)?),
                    span: *span,
                })
            }
            ConstraintItem::Block(items) => {
                let new: Vec<Option<ConstraintItem>> = items.iter().map(recurse).collect();
                if new.iter().all(Option::is_none) {
                    return None;
                }
                Some(ConstraintItem::Block(
                    new.into_iter()
                        .zip(items)
                        .map(|(n, o)| n.unwrap_or_else(|| o.clone()))
                        .collect(),
                ))
            }
            ConstraintItem::Soft(body) => Some(ConstraintItem::Soft(Box::new(recurse(body)?))),
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                span,
            } => {
                let then_new = recurse(then_item);
                let else_new = else_item.as_ref().map(|e| recurse(e));
                if then_new.is_none() && else_new.as_ref().is_none_or(Option::is_none) {
                    return None;
                }
                Some(ConstraintItem::IfElse {
                    condition: condition.clone(),
                    then_item: Box::new(then_new.unwrap_or_else(|| (**then_item).clone())),
                    else_item: else_item
                        .as_ref()
                        .map(|e| Box::new(else_new.flatten().unwrap_or_else(|| (**e).clone()))),
                    span: *span,
                })
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                span,
            } => Some(ConstraintItem::Implication {
                condition: condition.clone(),
                constraint: Box::new(recurse(constraint)?),
                span: *span,
            }),
            _ => None,
        }
    }

    /// `expand_fixed_foreach` over a list of items, rewriting in place (the
    /// list's buffer is kept, so a scope keyed on it stays valid). Cheap when
    /// no item holds a `foreach`.
    pub(super) fn expand_fixed_foreach_items(&self, items: &mut [ConstraintItem], all: bool) {
        for it in items {
            if let Some(n) = self.expand_fixed_foreach(it, all) {
                *it = n;
            }
        }
    }

    /// The iterations of `foreach (array[vars]) item` with every loop
    /// variable replaced by its index literal, when `array` is a state
    /// container (a 1-D/N-D unpacked array, queue or dynamic array, or a
    /// packed vector iterated by bit) of at most 4096 iterations. None
    /// otherwise.
    fn unroll_state_foreach(
        &mut self,
        array: &Expression,
        vars: &[Option<crate::ast::Identifier>],
        item: &ConstraintItem,
        rand_roots: &HashSet<String>,
        bound: &[String],
    ) -> Option<Vec<ConstraintItem>> {
        if Self::reads_unknown(array, rand_roots, bound) || vars.is_empty() {
            return None;
        }
        let dims: Vec<(i64, i64)> = if let Some(name) = self.foreach_check_operand_name(array) {
            self.foreach_check_dims(&name, vars.len())
        } else if vars.len() == 1 && matches!(array.kind, ExprKind::Ident(_)) {
            // A packed vector: its bits, LSB = 0.
            let v = self.eval_expr(array);
            if v.is_real || v.width == 0 {
                return None;
            }
            vec![(0, v.width as i64 - 1)]
        } else {
            return None;
        };
        Self::unroll_foreach_dims(&dims, vars, item)
    }

    /// §18.5.8.1/§18.7: `foreach (arr[vars]) item` whose array binds in the
    /// scope of the randomize() call (`cfg.held` read from a method
    /// of the calling class). The solve runs with `this` switched to the
    /// object, where the caller's array no longer resolves, so the loop is
    /// unrolled here over the array's current shape. None when the array is
    /// not a caller-scope collection.
    fn unroll_caller_foreach(
        &mut self,
        array: &Expression,
        vars: &[Option<crate::ast::Identifier>],
        item: &ConstraintItem,
        cx: &CallerScope,
        bound: &[String],
    ) -> Option<Vec<ConstraintItem>> {
        if vars.is_empty() || self.operand_bind(array, cx, bound) != Some(true) {
            return None;
        }
        let dims = match self.foreach_check_operand_name(array) {
            Some(name) if self.is_associative_array(&name) => return None,
            Some(name) => self.foreach_check_dims(&name, vars.len()),
            None => self.query_foreach_dims(array, vars.len())?,
        };
        Self::unroll_foreach_dims(&dims, vars, item)
    }

    /// §20.7: (low, high) of the first `nvars` unpacked dimensions of
    /// `array`, read with the array query functions. Covers operands the
    /// name-keyed stores do not (`cfg.grid` for a 2-D member of a handle).
    fn query_foreach_dims(&mut self, array: &Expression, nvars: usize) -> Option<Vec<(i64, i64)>> {
        let call = |name: &str, d: Option<usize>| {
            let mut args = vec![array.clone()];
            if let Some(d) = d {
                args.push(Self::literal_of(&Value::from_u64(d as u64, 32), array.span));
            }
            Expression::new(
                ExprKind::SystemCall {
                    name: name.to_string(),
                    args,
                },
                array.span,
            )
        };
        let nd = self.eval_expr(&call("$unpacked_dimensions", None)).to_u64()? as usize;
        if nd < nvars {
            return None;
        }
        (1..=nvars)
            .map(|d| {
                let l = self.eval_expr(&call("$left", Some(d))).to_i64()?;
                let r = self.eval_expr(&call("$right", Some(d))).to_i64()?;
                Some((l.min(r), l.max(r)))
            })
            .collect()
    }

    /// The iterations of `foreach (...[vars]) item` over `dims`, each loop
    /// variable replaced by its index literal. None above 4096 iterations or
    /// when the dimensions do not match the loop variables.
    fn unroll_foreach_dims(
        dims: &[(i64, i64)],
        vars: &[Option<crate::ast::Identifier>],
        item: &ConstraintItem,
    ) -> Option<Vec<ConstraintItem>> {
        if dims.len() != vars.iter().filter(|v| v.is_some()).count() && dims.len() != vars.len() {
            return None;
        }
        let mut total: u64 = 1;
        for &(lo, hi) in dims {
            if hi < lo {
                return Some(Vec::new());
            }
            total = total.saturating_mul((hi - lo + 1) as u64);
        }
        if total > 4096 {
            return None;
        }
        let mut out = Vec::with_capacity(total as usize);
        let mut idx: Vec<i64> = dims.iter().map(|d| d.0).collect();
        loop {
            let mut it = item.clone();
            for (k, v) in vars.iter().enumerate() {
                let (Some(v), Some(&i)) = (v, idx.get(k)) else {
                    continue;
                };
                let lit = Self::literal_of(&Self::signed_loop_val(i), v.span);
                Self::subst_ident_item(&mut it, &v.name, &lit);
            }
            out.push(it);
            let mut k = idx.len();
            loop {
                if k == 0 {
                    return Some(out);
                }
                k -= 1;
                if idx[k] < dims[k].1 {
                    idx[k] += 1;
                    break;
                }
                idx[k] = dims[k].0;
            }
        }
    }

    /// Replace the plain identifier `name` with `lit` throughout `it`
    /// (stopping at a nested `foreach` that rebinds it).
    pub(super) fn subst_ident_item(it: &mut ConstraintItem, name: &str, lit: &Expression) {
        let se = |e: &mut Expression| Self::subst_ident_expr(e, name, lit);
        match it {
            ConstraintItem::Expr(e) => Self::subst_ident_expr(e, name, lit),
            ConstraintItem::Inside {
                expr,
                range,
                dist_weights,
                ..
            } => {
                Self::subst_ident_expr(expr, name, lit);
                for r in range.iter_mut() {
                    r.for_each_expr_mut(&mut |e| Self::subst_ident_expr(e, name, lit));
                }
                for w in dist_weights.iter_mut().flatten() {
                    match w {
                        DistWeight::Each(e) | DistWeight::Total(e) => {
                            Self::subst_ident_expr(e, name, lit)
                        }
                    }
                }
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => {
                Self::subst_ident_expr(condition, name, lit);
                Self::subst_ident_item(constraint, name, lit);
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                Self::subst_ident_expr(condition, name, lit);
                Self::subst_ident_item(then_item, name, lit);
                if let Some(e) = else_item {
                    Self::subst_ident_item(e, name, lit);
                }
            }
            ConstraintItem::Foreach {
                array, vars, item, ..
            } => {
                Self::subst_ident_expr(array, name, lit);
                if !vars.iter().flatten().any(|v| v.name == name) {
                    Self::subst_ident_item(item, name, lit);
                }
            }
            ConstraintItem::Soft(x) => Self::subst_ident_item(x, name, lit),
            ConstraintItem::Block(xs) => {
                for x in xs {
                    Self::subst_ident_item(x, name, lit);
                }
            }
            ConstraintItem::Unique { exprs, .. } => exprs.iter_mut().for_each(se),
            // §18.5.10: an operand path may pick an element by the index.
            ConstraintItem::Solve {
                before_paths,
                after_paths,
                ..
            } => before_paths
                .iter_mut()
                .chain(after_paths.iter_mut())
                .for_each(se),
        }
    }

    pub(super) fn subst_ident_expr(e: &mut Expression, name: &str, lit: &Expression) {
        if let ExprKind::Ident(h) = &e.kind {
            if h.root.is_none()
                && h.path.len() == 1
                && h.path[0].selects.is_empty()
                && h.path[0].name.name == name
            {
                *e = lit.clone();
                return;
            }
        }
        let mut sub = |x: &mut Expression| Self::subst_ident_expr(x, name, lit);
        match &mut e.kind {
            ExprKind::Ident(h) => {
                for seg in &mut h.path {
                    seg.selects.iter_mut().for_each(&mut sub);
                }
                // The resolution caches describe the old expression.
                h.cached_signal_id.set(None);
            }
            ExprKind::Unary { operand, .. } => sub(operand),
            ExprKind::Binary { left, right, .. } => {
                sub(left);
                sub(right);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                sub(condition);
                sub(then_expr);
                sub(else_expr);
            }
            ExprKind::Concatenation(parts) => parts.iter_mut().for_each(&mut sub),
            ExprKind::Replication { count, exprs } => {
                sub(count);
                exprs.iter_mut().for_each(&mut sub);
            }
            ExprKind::Call { func, args } => {
                sub(func);
                args.iter_mut().for_each(&mut sub);
            }
            ExprKind::SystemCall { args, .. } => args.iter_mut().for_each(&mut sub),
            ExprKind::Inside { expr, ranges } => {
                sub(expr);
                ranges.iter_mut().for_each(&mut sub);
            }
            ExprKind::MemberAccess { expr, .. } | ExprKind::Paren(expr) => sub(expr),
            ExprKind::Index { expr, index } => {
                sub(expr);
                sub(index);
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                sub(expr);
                sub(left);
                sub(right);
            }
            ExprKind::Range(a, b) => {
                sub(a);
                sub(b);
            }
            ExprKind::WithClause { expr, filter } => {
                sub(expr);
                sub(filter);
            }
            _ => {}
        }
    }

    /// §11.3.1/§11.8.2: `t REL r` with `t` integral and `r` a real value of
    /// the state compares `t` converted to real. For an integer `t` that is
    /// the integer bound `t >= ceil(r)`, `t > floor(r)`, `t <= floor(r)`,
    /// `t < ceil(r)` — which the integral solver can use. None unless one
    /// side is random and the other a non-negative real state value.
    fn integral_real_bound(
        &mut self,
        e: &Expression,
        rand_roots: &HashSet<String>,
        bound: &[String],
    ) -> Option<Expression> {
        let ExprKind::Binary { op, left, right } = &e.kind else {
            return None;
        };
        if !matches!(
            op,
            BinaryOp::Lt | BinaryOp::Leq | BinaryOp::Gt | BinaryOp::Geq
        ) {
            return None;
        }
        let l_rand = Self::reads_unknown(left, rand_roots, bound);
        let r_rand = Self::reads_unknown(right, rand_roots, bound);
        if l_rand == r_rand {
            return None;
        }
        let (state, rand_on_left) = if l_rand { (right, true) } else { (left, false) };
        let v = self.eval_expr(state);
        if !v.is_real {
            return None;
        }
        let r = v.to_f64();
        if !r.is_finite() || r < 0.0 || r > i64::MAX as f64 {
            return None;
        }
        // Read every comparison as `t OP r`.
        let op_t = if rand_on_left {
            *op
        } else {
            match op {
                BinaryOp::Lt => BinaryOp::Gt,
                BinaryOp::Leq => BinaryOp::Geq,
                BinaryOp::Gt => BinaryOp::Lt,
                _ => BinaryOp::Leq,
            }
        };
        let k = match op_t {
            BinaryOp::Geq | BinaryOp::Lt => r.ceil(),
            _ => r.floor(),
        } as i64;
        // Signed, so a signed target keeps a signed comparison; `k >= 0`, so
        // an unsigned target reads the same value.
        let mut kv = Value::from_u64(k as u64, 64);
        kv.is_signed = true;
        let lit = Self::literal_of(&kv, state.span);
        let (l, r) = if rand_on_left {
            ((**left).clone(), lit)
        } else {
            (lit, (**right).clone())
        };
        Some(Expression::new(
            ExprKind::Binary {
                op: *op,
                left: Box::new(l),
                right: Box::new(r),
            },
            e.span,
        ))
    }

    fn fold_state_guard_item(
        &mut self,
        it: &ConstraintItem,
        rand_roots: &HashSet<String>,
        bound: &mut Vec<String>,
    ) -> Option<ConstraintItem> {
        let fold = |me: &mut Self, x: &ConstraintItem, bound: &mut Vec<String>| {
            me.fold_state_guard_item(x, rand_roots, bound)
        };
        match it {
            ConstraintItem::Implication {
                condition,
                constraint,
                span,
            } => match self.state_condition(condition, rand_roots, bound) {
                Some(true) => fold(self, constraint, bound),
                Some(false) => None,
                None => Some(ConstraintItem::Implication {
                    condition: condition.clone(),
                    constraint: Box::new(
                        fold(self, constraint, bound).unwrap_or(ConstraintItem::Block(Vec::new())),
                    ),
                    span: *span,
                }),
            },
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                span,
            } => match self.state_condition(condition, rand_roots, bound) {
                Some(true) => fold(self, then_item, bound),
                Some(false) => else_item.as_ref().and_then(|e| fold(self, e, bound)),
                None => Some(ConstraintItem::IfElse {
                    condition: condition.clone(),
                    then_item: Box::new(
                        fold(self, then_item, bound).unwrap_or(ConstraintItem::Block(Vec::new())),
                    ),
                    else_item: else_item.as_ref().map(|e| {
                        Box::new(fold(self, e, bound).unwrap_or(ConstraintItem::Block(Vec::new())))
                    }),
                    span: *span,
                }),
            },
            // `c -> x` written as an expression constraint.
            ConstraintItem::Expr(e) => {
                // `(a >= 3);` is the same constraint as `a >= 3;` — the
                // per-variable solver only recognizes the bare form.
                let mut inner = e;
                while let ExprKind::Paren(x) = &inner.kind {
                    inner = x;
                }
                if let ExprKind::Binary {
                    op: BinaryOp::LogImplies,
                    left,
                    right,
                } = &inner.kind
                {
                    match self.state_condition(left, rand_roots, bound) {
                        Some(true) => {
                            return fold(self, &ConstraintItem::Expr((**right).clone()), bound);
                        }
                        Some(false) => return None,
                        None => {}
                    }
                }
                if let Some(e) = self.integral_real_bound(inner, rand_roots, bound) {
                    return Some(ConstraintItem::Expr(e));
                }
                Some(ConstraintItem::Expr(inner.clone()))
            }
            ConstraintItem::Block(xs) => Some(ConstraintItem::Block(
                xs.iter().filter_map(|x| fold(self, x, bound)).collect(),
            )),
            ConstraintItem::Soft(x) => {
                fold(self, x, bound).map(|x| ConstraintItem::Soft(Box::new(x)))
            }
            ConstraintItem::Foreach {
                array,
                vars,
                item,
                span,
            } => {
                // §18.5.8.1: a `foreach` over a STATE container iterates a
                // known index set — unroll it, so each iteration's guards
                // (`if (mask[i])`) are decided like any other state guard.
                // So does one over a random array of fixed declared shape
                // (`s.item` of a struct variable).
                if let Some(unrolled) = self
                    .unroll_state_foreach(array, vars, item, rand_roots, bound)
                    .or_else(|| self.unroll_fixed_foreach(array, vars, item))
                {
                    let body: Vec<ConstraintItem> = unrolled
                        .iter()
                        .filter_map(|x| fold(self, x, bound))
                        .collect();
                    return Some(ConstraintItem::Block(body));
                }
                let mark = bound.len();
                bound.extend(vars.iter().flatten().map(|v| v.name.clone()));
                let body = fold(self, item, bound);
                bound.truncate(mark);
                body.map(|b| ConstraintItem::Foreach {
                    array: array.clone(),
                    vars: vars.clone(),
                    item: Box::new(b),
                    span: *span,
                })
            }
            _ => Some(it.clone()),
        }
    }
}

impl Simulator {
    /// §18.7: an unqualified name in `obj.randomize() with { … }` binds in
    /// the object's class scope first. Inside a module instance the
    /// elaborator has already rewritten the block's names that the INSTANCE
    /// declares to their instance-scoped storage names (`u1.x`, a single
    /// dotted segment), as for every other operand of the instance. Where the
    /// object's class also declares the leaf, the object hides the instance's
    /// object: restore the plain leaf so it binds to the member. Returns None
    /// when no name needs it (the original items are used).
    pub(super) fn unscope_object_member_names(
        &self,
        handle: usize,
        items: &[ConstraintItem],
    ) -> Option<Vec<ConstraintItem>> {
        let class = self.heap.get(handle)?.as_ref()?.class_name.clone();
        let hides = |name: &str| -> Option<String> {
            let (_, leaf) = name.rsplit_once('.')?;
            self.class_chain_declares(Some(&class), leaf)
                .then(|| leaf.to_string())
        };
        let mut any = false;
        let mut probe = |n: &str| any |= n.contains('.') && hides(n).is_some();
        for it in items {
            Self::visit_constraint_root_names(it, &mut probe);
        }
        if !any {
            return None;
        }
        let mut scan = items.to_vec();
        let mut fix = |id: &mut crate::ast::Identifier| {
            if let Some(leaf) = hides(&id.name) {
                id.name = leaf;
            }
        };
        for it in &mut scan {
            Self::for_each_constraint_root_name(it, &mut fix);
        }
        Some(scan)
    }

    /// Read-only twin of [`Self::for_each_constraint_root_name`].
    fn visit_constraint_root_names(it: &ConstraintItem, f: &mut dyn FnMut(&str)) {
        let mut fx = |e: &Expression| Self::visit_expr_root_names(e, f);
        match it {
            ConstraintItem::Expr(e) => fx(e),
            ConstraintItem::Inside {
                expr,
                range,
                dist_weights,
                ..
            } => {
                fx(expr);
                for r in range {
                    match r {
                        ConstraintRange::Value(v) => fx(v),
                        ConstraintRange::Range { lo, hi } => {
                            fx(lo);
                            fx(hi);
                        }
                    }
                }
                for w in dist_weights.iter().flatten() {
                    match w {
                        DistWeight::Each(e) | DistWeight::Total(e) => fx(e),
                    }
                }
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => {
                fx(condition);
                Self::visit_constraint_root_names(constraint, f);
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                fx(condition);
                Self::visit_constraint_root_names(then_item, f);
                if let Some(e) = else_item {
                    Self::visit_constraint_root_names(e, f);
                }
            }
            ConstraintItem::Foreach { array, item, .. } => {
                fx(array);
                Self::visit_constraint_root_names(item, f);
            }
            ConstraintItem::Soft(inner) => Self::visit_constraint_root_names(inner, f),
            ConstraintItem::Block(items) => {
                for i in items {
                    Self::visit_constraint_root_names(i, f);
                }
            }
            ConstraintItem::Unique { exprs, .. } => exprs.iter().for_each(fx),
            ConstraintItem::Solve {
                before,
                after,
                before_paths,
                after_paths,
                ..
            } => {
                for id in before.iter().chain(after.iter()) {
                    f(&id.name);
                }
                for e in before_paths.iter().chain(after_paths.iter()) {
                    Self::visit_expr_root_names(e, f);
                }
            }
        }
    }

    fn visit_expr_root_names(e: &Expression, f: &mut dyn FnMut(&str)) {
        Self::walk_operands(e, &mut |x| match &x.kind {
            ExprKind::Ident(h) if h.root.is_none() => {
                if let Some(seg) = h.path.first() {
                    f(&seg.name.name);
                }
            }
            ExprKind::WithClause { expr, filter } => {
                Self::visit_expr_root_names(expr, f);
                Self::visit_expr_root_names(filter, f);
            }
            _ => {}
        });
    }

    /// Visit the root identifier of every unrooted name in `it` (the first
    /// path segment, and each `solve ... before` operand).
    fn for_each_constraint_root_name(
        it: &mut ConstraintItem,
        f: &mut dyn FnMut(&mut crate::ast::Identifier),
    ) {
        let mut fx = |e: &mut Expression| Self::for_each_expr_root_name(e, f);
        match it {
            ConstraintItem::Expr(e) => fx(e),
            ConstraintItem::Inside {
                expr,
                range,
                dist_weights,
                ..
            } => {
                fx(expr);
                for r in range.iter_mut() {
                    r.for_each_expr_mut(&mut fx);
                }
                for w in dist_weights.iter_mut().flatten() {
                    match w {
                        DistWeight::Each(e) | DistWeight::Total(e) => fx(e),
                    }
                }
            }
            ConstraintItem::Implication {
                condition,
                constraint,
                ..
            } => {
                fx(condition);
                Self::for_each_constraint_root_name(constraint, f);
            }
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                ..
            } => {
                fx(condition);
                Self::for_each_constraint_root_name(then_item, f);
                if let Some(e) = else_item {
                    Self::for_each_constraint_root_name(e, f);
                }
            }
            ConstraintItem::Foreach { array, item, .. } => {
                fx(array);
                Self::for_each_constraint_root_name(item, f);
            }
            ConstraintItem::Soft(inner) => Self::for_each_constraint_root_name(inner, f),
            ConstraintItem::Block(items) => {
                for i in items {
                    Self::for_each_constraint_root_name(i, f);
                }
            }
            ConstraintItem::Unique { exprs, .. } => exprs.iter_mut().for_each(fx),
            ConstraintItem::Solve {
                before,
                after,
                before_paths,
                after_paths,
                ..
            } => {
                before.iter_mut().chain(after.iter_mut()).for_each(&mut *f);
                for e in before_paths.iter_mut().chain(after_paths.iter_mut()) {
                    Self::for_each_expr_root_name(e, f);
                }
            }
        }
    }

    fn for_each_expr_root_name(e: &mut Expression, f: &mut dyn FnMut(&mut crate::ast::Identifier)) {
        if let ExprKind::Ident(h) = &mut e.kind {
            if h.root.is_none() {
                if let Some(seg) = h.path.first_mut() {
                    f(&mut seg.name);
                }
            }
        }
        let mut sub = |x: &mut Expression| Self::for_each_expr_root_name(x, f);
        match &mut e.kind {
            ExprKind::Ident(h) => {
                for seg in &mut h.path {
                    seg.selects.iter_mut().for_each(&mut sub);
                }
            }
            ExprKind::Unary { operand, .. } => sub(operand),
            ExprKind::Binary { left, right, .. } => {
                sub(left);
                sub(right);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                sub(condition);
                sub(then_expr);
                sub(else_expr);
            }
            ExprKind::Concatenation(parts) => parts.iter_mut().for_each(sub),
            ExprKind::Replication { count, exprs } => {
                sub(count);
                exprs.iter_mut().for_each(sub);
            }
            ExprKind::Call { func, args } => {
                sub(func);
                args.iter_mut().for_each(sub);
            }
            ExprKind::SystemCall { args, .. } => args.iter_mut().for_each(sub),
            ExprKind::Inside { expr, ranges } => {
                sub(expr);
                ranges.iter_mut().for_each(sub);
            }
            ExprKind::MemberAccess { expr, .. } | ExprKind::Paren(expr) => sub(expr),
            ExprKind::Index { expr, index } => {
                sub(expr);
                sub(index);
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                sub(expr);
                sub(left);
                sub(right);
            }
            ExprKind::Range(a, b) => {
                sub(a);
                sub(b);
            }
            ExprKind::WithClause { expr, filter } => {
                sub(expr);
                sub(filter);
            }
            _ => {}
        }
    }

    fn lookup_obj_handle(&self, owner: usize, prop: &str) -> Option<usize> {
        let h = self
            .heap
            .get(owner)
            .and_then(|o| o.as_ref())
            .and_then(|i| i.properties.get(prop))
            .and_then(|v| v.to_u64())
            .unwrap_or(0) as usize;
        (h != 0 && self.heap.get(h).and_then(|o| o.as_ref()).is_some()).then_some(h)
    }

    fn class_has_property(&self, handle: usize, prop: &str) -> bool {
        let mut cur = self
            .heap
            .get(handle)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.clone());
        while let Some(cn) = cur {
            let Some(cd) = self.module.classes.get(&cn) else {
                break;
            };
            if cd.properties.contains_key(prop)
                || cd.array_properties.contains_key(prop)
                || cd.array_nd_properties.contains_key(prop)
                || cd.queue_properties.contains_key(prop)
                || cd.assoc_properties.contains_key(prop)
            {
                return true;
            }
            cur = cd.extends.clone();
        }
        false
    }

    fn member_is_coll(&self, owner: usize, prop: &str) -> bool {
        let coll_name = format!("{}#{}", owner, prop);
        if self.is_associative_array(&coll_name) {
            return true;
        }
        let mut cn = self
            .heap
            .get(owner)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.clone());
        while let Some(c) = cn {
            let Some(cd) = self.module.classes.get(&c) else {
                break;
            };
            if cd.queue_properties.contains_key(prop) || cd.array_properties.contains_key(prop) {
                return true;
            }
            cn = cd.extends.clone();
        }
        false
    }

    /// Walk active rand sub-objects reachable from `handle` via `rand_obj_props`,
    /// recording `(sub_handle, access_expr)` in DFS/BFS order for non-CSP solving.
    fn collect_reachable_child_handles(
        &self,
        handle: usize,
        rand_obj_props: &[String],
    ) -> Vec<(usize, Expression)> {
        let span = crate::ast::Span::new(0, 0);
        let mut out: Vec<(usize, Expression)> = Vec::new();
        let mut seen: HashSet<usize> = HashSet::default();
        seen.insert(handle);
        let mut queue: Vec<(usize, Option<Expression>, Vec<String>)> = vec![(
            handle,
            None,
            rand_obj_props.to_vec(),
        )];
        while let Some((owner, owner_access, props)) = queue.pop() {
            let owner_disabled = self
                .rand_mode_disabled
                .get(&owner)
                .cloned()
                .unwrap_or_default();
            for prop in props {
                let base = match &owner_access {
                    None => Self::chain_expr(std::slice::from_ref(&prop), span),
                    Some(oa) => Expression::new(
                        ExprKind::MemberAccess {
                            expr: Box::new(oa.clone()),
                            member: crate::ast::Identifier { name: prop.clone(), span },
                        },
                        span,
                    ),
                };
                if self.member_is_coll(owner, &prop) {
                    let coll_name = format!("{}#{}", owner, prop);
                    if self.is_associative_array(&coll_name) {
                        for key in self.assoc_key_strs(&coll_name) {
                            if owner_disabled.contains(&format!("{}[{}]", prop, key)) {
                                continue;
                            }
                            let key_expr = if self.is_string_keyed_array(&coll_name) {
                                Expression::new(
                                    ExprKind::StringLiteral(decode_name_key(&key).to_string()),
                                    span,
                                )
                            } else if let Ok(n) = key.parse::<i64>() {
                                Self::literal_of(&Self::signed_loop_val(n), span)
                            } else if let Ok(u) = key.parse::<u64>() {
                                Self::literal_of(&Value::from_u64(u, 64), span)
                            } else {
                                continue;
                            };
                            let Some(v) =
                                self.get_signal_value_by_name(&format!("{}[{}]", coll_name, key))
                            else {
                                continue;
                            };
                            let sub = v.to_u64().unwrap_or(0) as usize;
                            if sub != 0
                                && self.heap.get(sub).and_then(|o| o.as_ref()).is_some()
                                && seen.insert(sub)
                            {
                                let access = Expression::new(
                                    ExprKind::Index {
                                        expr: Box::new(base.clone()),
                                        index: Box::new(key_expr),
                                    },
                                    span,
                                );
                                let next_props = self.active_child_rand_members(sub);
                                out.push((sub, access.clone()));
                                queue.push((sub, Some(access), next_props));
                            }
                        }
                    } else {
                        let n = self.get_queue_size(&coll_name);
                        for i in 0..n {
                            if owner_disabled.contains(&format!("{}[{}]", prop, i)) {
                                continue;
                            }
                            let Some(v) =
                                self.get_signal_value_by_name(&format!("{}[{}]", coll_name, i))
                            else {
                                continue;
                            };
                            let sub = v.to_u64().unwrap_or(0) as usize;
                            if sub != 0
                                && self.heap.get(sub).and_then(|o| o.as_ref()).is_some()
                                && seen.insert(sub)
                            {
                                let access = Expression::new(
                                    ExprKind::Index {
                                        expr: Box::new(base.clone()),
                                        index: Box::new(Self::literal_of(
                                            &Value::from_u64(i, 32),
                                            span,
                                        )),
                                    },
                                    span,
                                );
                                let next_props = self.active_child_rand_members(sub);
                                out.push((sub, access.clone()));
                                queue.push((sub, Some(access), next_props));
                            }
                        }
                    }
                } else if let Some(sub) = self.lookup_obj_handle(owner, &prop)
                    && seen.insert(sub)
                {
                    let next_props = self.active_child_rand_members(sub);
                    out.push((sub, base.clone()));
                    queue.push((sub, Some(base), next_props));
                }
            }
        }
        out
    }

    /// Collect and rebase all active constraint items on reachable sub-objects
    /// that read `handle` through a back-reference handle, so the non-CSP
    /// solver (`!csp_ok`) solves `handle` against them directly.
    pub(super) fn collect_enclosing_back_ref_items(
        &mut self,
        handle: usize,
        rand_obj_props: &[String],
    ) -> Vec<ConstraintItem> {
        let children = self.collect_reachable_child_handles(handle, rand_obj_props);
        let chain = self.rand_solve_chain.len();
        let above = self
            .rand_solve_chain
            .iter()
            .position(|&h| h == handle)
            .unwrap_or(0);
        let mut out = Vec::new();
        for (sub, sub_access) in children {
            let class = self
                .heap
                .get(sub)
                .and_then(|o| o.as_ref())
                .map(|i| i.class_name.clone());
            let constraints = self.active_constraints(sub);
            self.this_stack.push(Some(sub));
            self.class_context_stack.push(class);
            let mut bound = Vec::new();
            for con in constraints {
                for item in &con.items {
                    if !self.rand_item_reads_enclosing(sub, item, chain) {
                        continue;
                    }
                    if above > 0 && self.rand_item_reads_enclosing(sub, item, above) {
                        continue;
                    }
                    let mut unrolled = Vec::new();
                    if self.csp_unroll_item(sub, None, item, &mut unrolled).is_none() {
                        unrolled.push(item.clone());
                    }
                    for ui in &unrolled {
                        out.push(self.rewrite_back_ref_item(
                            sub,
                            handle,
                            &sub_access,
                            ui,
                            &mut bound,
                        ));
                    }
                }
            }
            self.class_context_stack.pop();
            self.this_stack.pop();
        }
        out
    }

    fn rewrite_back_ref_item(
        &self,
        sub: usize,
        target: usize,
        sub_access: &Expression,
        item: &ConstraintItem,
        bound: &mut Vec<String>,
    ) -> ConstraintItem {
        let rw_e = |e: &Expression, b: &[String]| {
            self.rewrite_back_ref_expr(sub, target, sub_access, e, b)
        };
        match item {
            ConstraintItem::Expr(e) => ConstraintItem::Expr(rw_e(e, bound)),
            ConstraintItem::Inside {
                expr,
                range,
                is_dist,
                dist_weights,
                span,
            } => ConstraintItem::Inside {
                expr: rw_e(expr, bound),
                range: range
                    .iter()
                    .map(|r| match r {
                        ConstraintRange::Value(v) => ConstraintRange::Value(rw_e(v, bound)),
                        ConstraintRange::Range { lo, hi } => ConstraintRange::Range {
                            lo: rw_e(lo, bound),
                            hi: rw_e(hi, bound),
                        },
                    })
                    .collect(),
                is_dist: *is_dist,
                dist_weights: dist_weights
                    .iter()
                    .map(|ow| {
                        ow.as_ref().map(|w| match w {
                            DistWeight::Each(e) => DistWeight::Each(rw_e(e, bound)),
                            DistWeight::Total(e) => DistWeight::Total(rw_e(e, bound)),
                        })
                    })
                    .collect(),
                span: *span,
            },
            ConstraintItem::Implication {
                condition,
                constraint,
                span,
            } => ConstraintItem::Implication {
                condition: rw_e(condition, bound),
                constraint: Box::new(self.rewrite_back_ref_item(
                    sub,
                    target,
                    sub_access,
                    constraint,
                    bound,
                )),
                span: *span,
            },
            ConstraintItem::IfElse {
                condition,
                then_item,
                else_item,
                span,
            } => ConstraintItem::IfElse {
                condition: rw_e(condition, bound),
                then_item: Box::new(self.rewrite_back_ref_item(
                    sub,
                    target,
                    sub_access,
                    then_item,
                    bound,
                )),
                else_item: else_item.as_ref().map(|e| {
                    Box::new(self.rewrite_back_ref_item(sub, target, sub_access, e, bound))
                }),
                span: *span,
            },
            ConstraintItem::Foreach {
                array,
                vars,
                item: body,
                span,
            } => {
                let new_arr = rw_e(array, bound);
                let depth = bound.len();
                bound.extend(vars.iter().flatten().map(|v| v.name.clone()));
                let new_body = self.rewrite_back_ref_item(sub, target, sub_access, body, bound);
                bound.truncate(depth);
                ConstraintItem::Foreach {
                    array: new_arr,
                    vars: vars.clone(),
                    item: Box::new(new_body),
                    span: *span,
                }
            }
            ConstraintItem::Soft(inner) => ConstraintItem::Soft(Box::new(
                self.rewrite_back_ref_item(sub, target, sub_access, inner, bound),
            )),
            ConstraintItem::Block(items) => ConstraintItem::Block(
                items
                    .iter()
                    .map(|i| self.rewrite_back_ref_item(sub, target, sub_access, i, bound))
                    .collect(),
            ),
            ConstraintItem::Unique { exprs, span } => ConstraintItem::Unique {
                exprs: exprs.iter().map(|e| rw_e(e, bound)).collect(),
                span: *span,
            },
            ConstraintItem::Solve { .. } => item.clone(),
        }
    }

    fn rewrite_back_ref_expr(
        &self,
        sub: usize,
        target: usize,
        sub_access: &Expression,
        e: &Expression,
        bound: &[String],
    ) -> Expression {
        match self.rewrite_back_ref_expr_inner(sub, target, sub_access, e, bound) {
            Some(BackRefTarget::Expr(ex)) => ex,
            Some(BackRefTarget::TargetThis) => {
                Self::chain_expr(&["this".to_string()], e.span)
            }
            Some(BackRefTarget::SubThis) => sub_access.clone(),
            None => e.clone(),
        }
    }

    fn rewrite_hier_selects(
        &self,
        sub: usize,
        target: usize,
        sub_access: &Expression,
        segs: &[crate::ast::expr::HierPathSegment],
        bound: &[String],
    ) -> Vec<crate::ast::expr::HierPathSegment> {
        segs.iter()
            .map(|seg| crate::ast::expr::HierPathSegment {
                name: seg.name.clone(),
                selects: seg
                    .selects
                    .iter()
                    .map(|s| self.rewrite_back_ref_expr(sub, target, sub_access, s, bound))
                    .collect(),
            })
            .collect()
    }

    fn wrap_selects(
        &self,
        mut base: Expression,
        selects: Vec<Expression>,
        span: crate::ast::Span,
    ) -> Expression {
        for sel in selects {
            base = Expression::new(
                ExprKind::Index {
                    expr: Box::new(base),
                    index: Box::new(sel),
                },
                span,
            );
        }
        base
    }

    fn rewrite_back_ref_expr_inner(
        &self,
        sub: usize,
        target: usize,
        sub_access: &Expression,
        e: &Expression,
        bound: &[String],
    ) -> Option<BackRefTarget> {
        let rw = |x: &Expression| self.rewrite_back_ref_expr(sub, target, sub_access, x, bound);
        let out = match &e.kind {
            ExprKind::Ident(hier) => {
                if hier.root.is_none() && !hier.path.is_empty() {
                    if hier.path.len() == 1
                        && hier.path[0].selects.is_empty()
                        && bound.contains(&hier.path[0].name.name)
                    {
                        return Some(BackRefTarget::Expr(e.clone()));
                    }
                    let has_this = hier.path[0].name.name == "this" && hier.path[0].selects.is_empty();
                    if has_this && hier.path.len() == 1 {
                        return Some(BackRefTarget::SubThis);
                    }
                    let start = usize::from(has_this);
                    let mut cur_h = sub;
                    for i in start..hier.path.len() {
                        let seg = &hier.path[i];
                        if !seg.selects.is_empty() {
                            break;
                        }
                        let Some(next_h) = self.lookup_obj_handle(cur_h, &seg.name.name) else {
                            break;
                        };
                        if next_h == target {
                            let rest = self.rewrite_hier_selects(
                                sub,
                                target,
                                sub_access,
                                &hier.path[i + 1..],
                                bound,
                            );
                            if rest.is_empty() {
                                return Some(BackRefTarget::TargetThis);
                            }
                            if rest.iter().all(|s| s.selects.is_empty()) {
                                let names: Vec<String> =
                                    rest.iter().map(|s| s.name.name.clone()).collect();
                                return Some(BackRefTarget::Expr(Self::chain_expr(&names, e.span)));
                            }
                            let mut cur = Self::chain_expr(
                                std::slice::from_ref(&rest[0].name.name),
                                e.span,
                            );
                            cur = self.wrap_selects(cur, rest[0].selects.clone(), e.span);
                            for seg in &rest[1..] {
                                cur = Expression::new(
                                    ExprKind::MemberAccess {
                                        expr: Box::new(cur),
                                        member: seg.name.clone(),
                                    },
                                    e.span,
                                );
                                cur = self.wrap_selects(cur, seg.selects.clone(), e.span);
                            }
                            return Some(BackRefTarget::Expr(cur));
                        }
                        cur_h = next_h;
                    }
                    let root_name = &hier.path[start].name.name;
                    if !bound.contains(root_name) && self.class_has_property(sub, root_name) {
                        let segs = self.rewrite_hier_selects(
                            sub,
                            target,
                            sub_access,
                            &hier.path[start..],
                            bound,
                        );
                        let mut cur = Expression::new(
                            ExprKind::MemberAccess {
                                expr: Box::new(sub_access.clone()),
                                member: segs[0].name.clone(),
                            },
                            e.span,
                        );
                        cur = self.wrap_selects(cur, segs[0].selects.clone(), e.span);
                        for seg in &segs[1..] {
                            cur = Expression::new(
                                ExprKind::MemberAccess {
                                    expr: Box::new(cur),
                                    member: seg.name.clone(),
                                },
                                e.span,
                            );
                            cur = self.wrap_selects(cur, seg.selects.clone(), e.span);
                        }
                        return Some(BackRefTarget::Expr(cur));
                    }
                }
                let mut h = hier.clone();
                h.path = self.rewrite_hier_selects(sub, target, sub_access, &hier.path, bound);
                h.cached_signal_id = crate::ast::cache::SignalCache::new(None);
                h.cached_resolved_name = crate::ast::cache::NameCache::new();
                Expression::new(ExprKind::Ident(h), e.span)
            }
            ExprKind::MemberAccess { expr, member } => {
                match self.rewrite_back_ref_expr_inner(sub, target, sub_access, expr, bound) {
                    Some(BackRefTarget::TargetThis) => {
                        Self::chain_expr(std::slice::from_ref(&member.name), e.span)
                    }
                    Some(BackRefTarget::SubThis) => {
                        if self.lookup_obj_handle(sub, &member.name) == Some(target) {
                            return Some(BackRefTarget::TargetThis);
                        }
                        if self.class_has_property(sub, &member.name) {
                            Expression::new(
                                ExprKind::MemberAccess {
                                    expr: Box::new(sub_access.clone()),
                                    member: member.clone(),
                                },
                                e.span,
                            )
                        } else {
                            Expression::new(
                                ExprKind::MemberAccess {
                                    expr: Box::new(expr.as_ref().clone()),
                                    member: member.clone(),
                                },
                                e.span,
                            )
                        }
                    }
                    Some(BackRefTarget::Expr(inner)) => Expression::new(
                        ExprKind::MemberAccess {
                            expr: Box::new(inner),
                            member: member.clone(),
                        },
                        e.span,
                    ),
                    None => e.clone(),
                }
            }
            ExprKind::Paren(inner) => match self
                .rewrite_back_ref_expr_inner(sub, target, sub_access, inner, bound)
            {
                Some(BackRefTarget::TargetThis) => return Some(BackRefTarget::TargetThis),
                Some(BackRefTarget::SubThis) => return Some(BackRefTarget::SubThis),
                Some(BackRefTarget::Expr(ex)) => Expression::new(ExprKind::Paren(Box::new(ex)), e.span),
                None => e.clone(),
            },
            ExprKind::Unary { op, operand } => Expression::new(
                ExprKind::Unary {
                    op: *op,
                    operand: Box::new(rw(operand)),
                },
                e.span,
            ),
            ExprKind::Binary { op, left, right } => Expression::new(
                ExprKind::Binary {
                    op: *op,
                    left: Box::new(rw(left)),
                    right: Box::new(rw(right)),
                },
                e.span,
            ),
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => Expression::new(
                ExprKind::Conditional {
                    condition: Box::new(rw(condition)),
                    then_expr: Box::new(rw(then_expr)),
                    else_expr: Box::new(rw(else_expr)),
                },
                e.span,
            ),
            ExprKind::Concatenation(parts) => {
                Expression::new(ExprKind::Concatenation(parts.iter().map(rw).collect()), e.span)
            }
            ExprKind::Replication { count, exprs } => Expression::new(
                ExprKind::Replication {
                    count: Box::new(rw(count)),
                    exprs: exprs.iter().map(rw).collect(),
                },
                e.span,
            ),
            ExprKind::Call { func, args } => Expression::new(
                ExprKind::Call {
                    func: Box::new(rw(func)),
                    args: args.iter().map(rw).collect(),
                },
                e.span,
            ),
            ExprKind::SystemCall { name, args } => Expression::new(
                ExprKind::SystemCall {
                    name: name.clone(),
                    args: args.iter().map(rw).collect(),
                },
                e.span,
            ),
            ExprKind::Inside { expr, ranges } => Expression::new(
                ExprKind::Inside {
                    expr: Box::new(rw(expr)),
                    ranges: ranges.iter().map(rw).collect(),
                },
                e.span,
            ),
            ExprKind::Index { expr, index } => Expression::new(
                ExprKind::Index {
                    expr: Box::new(rw(expr)),
                    index: Box::new(rw(index)),
                },
                e.span,
            ),
            ExprKind::RangeSelect {
                expr,
                kind,
                left,
                right,
            } => Expression::new(
                ExprKind::RangeSelect {
                    expr: Box::new(rw(expr)),
                    kind: *kind,
                    left: Box::new(rw(left)),
                    right: Box::new(rw(right)),
                },
                e.span,
            ),
            ExprKind::Range(a, b) => {
                Expression::new(ExprKind::Range(Box::new(rw(a)), Box::new(rw(b))), e.span)
            }
            ExprKind::WithClause { expr, filter } => Expression::new(
                ExprKind::WithClause {
                    expr: Box::new(rw(expr)),
                    filter: Box::new(rw(filter)),
                },
                e.span,
            ),
            _ => return None,
        };
        Some(BackRefTarget::Expr(out))
    }
}

enum BackRefTarget {
    TargetThis,
    SubThis,
    Expr(Expression),
}
