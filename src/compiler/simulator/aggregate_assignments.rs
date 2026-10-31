//! Simulator aggregate assignments.

use super::*;

impl Simulator {
    /// Spread a whole-value pattern over a fixed unpacked array property.
    /// Per-instance array elements live under an instance-scoped storage name,
    /// while the bare property used inside a method has no scalar container.
    pub(super) fn assign_class_fixed_array_pattern(
        &mut self,
        lvalue: &Expression,
        items: &[AssignmentPatternItem],
    ) -> bool {
        if items.is_empty() {
            return false;
        }
        // §7.4.2: a selected ROW of a multi-dimensional member
        // (`c.m[1] = '{..}`) takes the pattern over the remaining dimensions.
        let (base_expr, row_selects) = Self::split_row_selects(lvalue);
        let Some(scoped) = self.expr_assoc_name(&base_expr) else {
            return false;
        };
        let row: Vec<i64> = row_selects
            .iter()
            .map(|e| self.eval_expr(e).to_i64().unwrap_or(0))
            .collect();
        let Some((handle_text, member)) = scoped.split_once('#') else {
            return false;
        };
        let Ok(handle) = handle_text.parse::<usize>() else {
            return false;
        };
        let Some(class_name) = self
            .heap
            .get(handle)
            .and_then(|entry| entry.as_ref())
            .map(|instance| instance.class_name.clone())
        else {
            return false;
        };

        let mut current = Some(class_name);
        let mut layout: Option<(Vec<(i64, i64)>, DataType)> = None;
        // §7.4.2 / §10.9: each dimension's declared direction — the first
        // item of a pattern goes to the LEFT bound (`int ds[3:1]`).
        let mut descending: Vec<bool> = Vec::new();
        while let Some(name) = current {
            let Some(class_def) = self.module.classes.get(&name) else {
                break;
            };
            if let Some(dd) = class_def.array_prop_decl_dims.get(member) {
                descending = dd.iter().map(|&(l, r)| l > r).collect();
            }
            if let Some((shape, _)) = class_def.array_nd_properties.get(member) {
                if let Some(data_type) = class_def.property_types.get(member) {
                    let shape = self
                        .inst_nd_shape(class_def, handle, member)
                        .unwrap_or_else(|| shape.clone());
                    layout = Some((shape, data_type.clone()));
                    break;
                }
            }
            if let Some(&(low, high, _)) = class_def.array_properties.get(member) {
                if let Some(data_type) = class_def.property_types.get(member) {
                    let (low, high) =
                        self.inst_fixed_bounds(class_def, handle, member, (low, high));
                    layout = Some((vec![(low, high)], data_type.clone()));
                    break;
                }
            }
            current = class_def.extends.clone();
        }
        let Some((dimensions, data_type)) = layout else {
            return false;
        };
        if dimensions.is_empty() {
            return false;
        }
        if descending.len() != dimensions.len() {
            descending.clear();
        }
        if !row.is_empty() {
            if row.len() >= dimensions.len() {
                return false;
            }
            let mut target = scoped.clone();
            for i in &row {
                target.push_str(&format!("[{i}]"));
            }
            let rest = dimensions[row.len()..].to_vec();
            let rest_desc = if descending.is_empty() {
                Vec::new()
            } else {
                descending[row.len()..].to_vec()
            };
            self.assign_pattern_array(&target, &rest, &data_type, items, &rest_desc);
            return true;
        }
        self.assign_pattern_array(&scoped, &dimensions, &data_type, items, &descending);
        true
    }

    /// `c.m[1]` / `c.m[1][2]` (either parse shape) as the unselected member
    /// expression and its index selects, outermost first. Range selects and
    /// anything else come back whole, with no selects.
    fn split_row_selects(e: &Expression) -> (Expression, Vec<Expression>) {
        let is_range =
            |x: &Expression| matches!(x.kind, ExprKind::Range(..) | ExprKind::RangeSelect { .. });
        let mut sels: Vec<Expression> = Vec::new();
        let mut cur = e;
        while let ExprKind::Index { expr, index } = &cur.kind {
            if is_range(index) {
                return (e.clone(), Vec::new());
            }
            sels.push((**index).clone());
            cur = expr;
        }
        sels.reverse();
        if let ExprKind::Ident(h) = &cur.kind
            && let Some(last) = h.path.last()
            && !last.selects.is_empty()
            && h.path[..h.path.len() - 1]
                .iter()
                .all(|s| s.selects.is_empty())
        {
            if last.selects.iter().any(is_range) {
                return (e.clone(), Vec::new());
            }
            let mut bare = h.clone();
            let lsel = bare
                .path
                .last_mut()
                .map(|s| std::mem::take(&mut s.selects))
                .unwrap_or_default();
            bare.cached_signal_id = std::cell::Cell::new(None);
            bare.cached_resolved_name = std::cell::OnceCell::new();
            let mut all = lsel;
            all.extend(sels);
            return (Expression::new(ExprKind::Ident(bare), cur.span), all);
        }
        (cur.clone(), sels)
    }

    /// Whole-assignment pattern to a CLASS COLLECTION property accessed by
    /// handle — `obj.q = '{1,2,3}` (`queue`/dynamic array) or
    /// `c.aa = '{k:v,...}` (associative array). This lvalue is a
    /// `MemberAccess` (`obj.q`/`this.q`), so the bare-Ident aggregate path
    /// misses it and it falls to a packed concat whose width is never
    /// recorded — leaving `.size`/`.num` 0 and the elements absent (the
    /// failure behind UVM's `uvm_pack_queue` writing size 0). Store the
    /// elements under the instance-scoped `<h>#<name>[i]`/`[key]` names and
    /// set the collection size, matching the in-method write.
    pub(super) fn assign_class_collection_pattern(
        &mut self,
        lvalue: &Expression,
        items: &[AssignmentPatternItem],
    ) -> bool {
        // The lvalue is either `obj.q`/`this.q` (MemberAccess) or a two-
        // segment `c.q` (flattened Ident). Resolve the object handle + member
        // from both forms.
        let (handle, member) = match &lvalue.kind {
            ExprKind::MemberAccess { expr: base, member } => {
                let h = match &base.kind {
                    ExprKind::This => self.this_stack.last().copied().flatten().unwrap_or(0),
                    ExprKind::Ident(b) if b.path.len() == 1 => {
                        let nm = b.path[0].name.name.clone();
                        self.eval_ident_handle(&nm).unwrap_or(0)
                    }
                    _ => 0,
                };
                (h, member.name.clone())
            }
            ExprKind::Ident(hi) if hi.path.len() == 2 => {
                let base_nm = hi.path[0].name.name.clone();
                let h = self.eval_ident_handle(&base_nm).unwrap_or(0);
                (h, hi.path[1].name.name.clone())
            }
            // A bare member inside a method (`m = '{...}`): `this`'s store.
            ExprKind::Ident(hi) if hi.path.len() == 1 && hi.path[0].selects.is_empty() => {
                let nm = &hi.path[0].name.name;
                match self.instance_assoc_member(nm) {
                    Some(scoped) => match scoped.split_once('#') {
                        Some((h, m)) if m == nm => (h.parse().unwrap_or(0), m.to_string()),
                        _ => (0, String::new()),
                    },
                    None => (0, String::new()),
                }
            }
            _ => (0, String::new()),
        };
        if handle == 0 {
            return false;
        }
        let Some(cn) = self
            .heap
            .get(handle)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.clone())
        else {
            return false;
        };
        let member = &member;
        let scoped = format!("{}#{}", handle, member);
        if items.is_empty() {
            // `'{}` leaves the queue or dynamic array with no elements.
            // `class_queue_prop` also finds one the class inherits.
            if self.class_queue_prop(&scoped) {
                self.set_queue_size(&scoped, 0);
                return true;
            }
            return false;
        }
        // §8.3: inherited properties use their declaring class's collection
        // metadata. Stop at the first declaration so a shadow remains local.
        let owners = self.prop_owners(&cn);
        let Some((_, cd)) = owners.get(member) else {
            return false;
        };
        // Associative array: `'{k:v,...}` — keyed items.
        if cd.assoc_properties.contains_key(member) {
            let val_w = cd.properties.get(member).map(|s| s.width).unwrap_or(32);
            self.assign_class_assoc_pattern(&scoped, val_w, items);
            return true;
        }
        // Queue / dynamic array: positional items are the elements; they set
        // the size (so `.size`/`uvm_pack_queue` see the live count).
        if let Some(&(ew, _)) = cd.queue_properties.get(member) {
            let exprs = self.pattern_ordered(items);
            if exprs.is_empty() {
                return false;
            }
            for (i, e) in exprs.iter().enumerate() {
                self.pattern_leaf_at_elem_width(&format!("{}[{}]", scoped, i), e, ew);
            }
            self.set_queue_size(&scoped, exprs.len() as u64);
            return true;
        }
        false
    }

    /// §7.9.11 / §10.9.2: assign the pattern `'{k: v, ..., default: d}` to
    /// the class associative-array member stored at `scoped`
    /// (`<handle>#<member>`): it replaces the array's contents with the keyed
    /// items, and `d` becomes the value read for every other key.
    pub(super) fn assign_class_assoc_pattern(
        &mut self,
        scoped: &str,
        val_w: u32,
        items: &[AssignmentPatternItem],
    ) {
        for k in self
            .signals
            .keys_with_elem_prefix(&Self::name_with_suffix(scoped, "["))
        {
            self.signals.remove(&k);
        }
        self.class_assoc_defaults.remove(scoped);
        let elem_dt = self
            .class_member_coll_type(scoped)
            .cloned()
            .filter(|dt| matches!(self.resolve_dt_ref(dt), DataType::Struct(_)));
        for item in items {
            match item {
                AssignmentPatternItem::Keyed(k, v) => {
                    let kv = self.eval_expr(k);
                    let key = self.assoc_key_str(scoped, &kv);
                    let target = Self::name_with_key(scoped, &key);
                    match &elem_dt {
                        Some(dt) => self.assign_pattern_or_leaf(&target, dt, v),
                        None => self.pattern_leaf_at_elem_width(&target, v, val_w),
                    }
                }
                AssignmentPatternItem::Default(e) if elem_dt.is_none() => {
                    let mut v = if val_w > 0 {
                        self.eval_expr_ctx(e, val_w)
                    } else {
                        self.eval_expr(e)
                    };
                    if val_w > 0 && !v.is_real && v.width != val_w {
                        v = v.resize_for_assign(val_w);
                    }
                    if let Some(dt) = self.class_member_coll_type(scoped) {
                        if super::elaborate::is_type_two_state_resolved(
                            dt,
                            &self.module.typedef_types,
                        ) {
                            v = v.to_two_state();
                        }
                        v.is_signed = super::elaborate::is_type_signed_resolved(
                            dt,
                            &self.module.typedef_types,
                        );
                    }
                    self.class_assoc_defaults.insert(scoped.to_string(), v);
                }
                _ => {}
            }
        }
    }

    /// Evaluate one collection-element pattern item at its ELEMENT width and
    /// write it to the flat `target` name — the scalar analogue of
    /// `assign_pattern_or_leaf` (whose width comes from a typed declaration,
    /// which a by-handle class collection lacks).
    pub(super) fn pattern_leaf_at_elem_width(&mut self, target: &str, e: &Expression, ew: u32) {
        let mut v = if ew > 0 {
            self.eval_expr_ctx(e, ew)
        } else {
            self.eval_expr(e)
        };
        if ew > 0 && !v.is_real && v.width != ew {
            v = v.resize_for_assign(ew);
        }
        self.set_signal_value_by_name(target, v);
    }

    /// Whether an aggregate keeps each member in its OWN signal, so an
    /// assignment pattern must be spread across them. An unpacked struct is one
    /// signal; so is an untagged union — one storage shared by every member
    /// (§7.3) — and both take the packed-value path instead.
    pub(super) fn spreads_member_wise(su: &crate::ast::types::StructUnionType) -> bool {
        let untagged_union =
            matches!(su.kind, crate::ast::types::StructUnionKind::Union) && !su.tagged;
        !su.packed && !untagged_union
    }

    /// Assign `e` to the flattened target `target` of declared type `dt`. When
    /// `e` is itself an assignment pattern and `dt` an unpacked struct, recurse
    /// member-wise; otherwise evaluate and write the leaf.
    pub(super) fn assign_pattern_or_leaf(&mut self, target: &str, dt: &DataType, e: &Expression) {
        if let ExprKind::AssignmentPattern(sub) = &e.kind {
            if !sub.is_empty() {
                if let DataType::Struct(su) = self.resolve_dt(dt) {
                    if Self::spreads_member_wise(&su) {
                        self.assign_pattern_into_struct(target, &su, sub);
                        return;
                    }
                }
            }
        } else if let DataType::Struct(su) = self.resolve_dt(dt) {
            // §10.9.2: `default:` applies RECURSIVELY to members of aggregate
            // members. A struct-typed member reached with a plain default
            // expression previously took the leaf path, writing one packed
            // container value whose per-member leaves nobody reads — the old
            // member values survived an assignment that should overwrite
            // everything. Re-enter the spread with a synthetic default item.
            if Self::spreads_member_wise(&su) {
                let synthetic = [AssignmentPatternItem::Default(e.clone())];
                self.assign_pattern_into_struct(target, &su, &synthetic);
                return;
            }
        }
        // A `string` leaf is stored as-is: the width resize below padded the
        // text with NULs to the element's nominal width, so a 2-D string
        // local's `'{'{"a","bb"},…}` read back as a run of blanks before
        // each value.
        if matches!(
            self.resolve_dt(dt),
            DataType::Simple {
                kind: crate::ast::types::SimpleType::String,
                ..
            }
        ) {
            let v = self.eval_expr(e);
            self.set_signal_value_by_name(target, v);
            return;
        }
        // §10.9.2: each item is evaluated in the context of the ELEMENT type —
        // `int d[]; d = '{1'b1 + 1'b1}` is 2, not a 1-bit wrap to 0, and a
        // signed narrower item sign-extends. Self-determined evaluation gave
        // the wrapped/zero-extended value.
        let elem_w = super::elaborate::resolve_type_width(
            dt,
            Some(&self.module.parameters),
            Some(&self.module.typedefs),
        );
        let mut v = if elem_w > 0 {
            self.eval_expr_ctx(e, elem_w)
        } else {
            self.eval_expr(e)
        };
        if elem_w > 0 && !v.is_real && v.width != elem_w {
            v = v.resize_for_assign(elem_w);
        }
        // §10.9.2/§10.7: the item is assigned to an element of the declared
        // type, so it takes that type's signedness and (2-state) drops x/z.
        // A block-local `logic [1:0] s[4] = '{0,1,2,3}` kept the items' `int`
        // signedness, so `s[2]` read -2.
        if !v.is_real
            && matches!(
                super::elaborate::resolve_typedef_chain(dt, &self.module.typedef_types),
                DataType::IntegerVector { .. } | DataType::IntegerAtom { .. }
            )
        {
            if super::elaborate::is_type_two_state_resolved(dt, &self.module.typedef_types) {
                v = v.to_two_state();
            }
            v.is_signed = self.type_is_signed_concrete(dt);
        }
        // A `real` leaf keeps its real value even when the element signal was
        // never flagged real (array elements are synthesized without the flag);
        // otherwise `real m[3] = '{1.5, ...}` truncates to integers.
        if matches!(self.resolve_dt(dt), DataType::Real { .. }) {
            let rv = if v.is_real {
                v
            } else {
                Value::from_f64(v.to_f64())
            };
            self.set_signal_value_by_name(target, rv);
            return;
        }
        self.write_leaf_by_name(target, v);
    }

    /// §7.5/§7.8/§7.10: assign the collection-valued expression `e` to the
    /// whole collection `name`, whose elements have type `elem_dt`: an
    /// assignment pattern (positional elements, or `key: value` pairs and a
    /// `default:` for an associative array), an unpacked-array concatenation,
    /// another collection, or a call returning one. The old contents are
    /// replaced, not merged. Returns false (nothing written) for any other
    /// expression.
    #[inline(never)]
    pub(super) fn assign_whole_coll(
        &mut self,
        name: &str,
        elem_dt: Option<&DataType>,
        e: &Expression,
    ) -> bool {
        let is_assoc = self.is_associative_array(name);
        // Without a declared element type (a class struct property's member),
        // items take the registered element width.
        let elem_w = self
            .module
            .arrays
            .get(name)
            .map(|t| t.2)
            .or_else(|| self.module.assoc_elem_widths.get(name).copied())
            .unwrap_or(32);
        match &e.kind {
            ExprKind::AssignmentPattern(items) => {
                if is_assoc {
                    self.eval_builtin_method(name, "delete", &[]);
                    self.module.assoc_defaults.remove(name);
                    for item in items {
                        match item {
                            AssignmentPatternItem::Keyed(k, v) => {
                                let kv = self.eval_expr(k);
                                let key = self.assoc_key_str(name, &kv);
                                let cell = format!("{}[{}]", name, key);
                                match elem_dt {
                                    Some(dt) => self.assign_pattern_or_leaf(&cell, dt, v),
                                    None => self.pattern_leaf_at_elem_width(&cell, v, elem_w),
                                }
                            }
                            AssignmentPatternItem::Default(d) => {
                                self.module
                                    .assoc_defaults
                                    .insert(name.to_string(), d.clone());
                            }
                            _ => {}
                        }
                    }
                    if !items.is_empty() {
                        self.mark_struct_coll_owner_present(name);
                    }
                } else {
                    let exprs: Vec<Expression> =
                        self.pattern_ordered(items).into_iter().cloned().collect();
                    for (i, x) in exprs.iter().enumerate() {
                        let cell = format!("{}[{}]", name, i);
                        match elem_dt {
                            Some(dt) => self.assign_pattern_or_leaf(&cell, dt, x),
                            None => self.pattern_leaf_at_elem_width(&cell, x, elem_w),
                        }
                    }
                    self.set_queue_size(name, exprs.len() as u64);
                }
                true
            }
            ExprKind::Concatenation(exprs) if !is_assoc => {
                let vals = self.unpacked_concat_operand_values(exprs.iter().collect());
                let n = vals.len() as u64;
                for (i, v) in vals.into_iter().enumerate() {
                    self.set_signal_value_by_name(&format!("{}[{}]", name, i), v);
                }
                self.set_queue_size(name, n);
                true
            }
            ExprKind::Call { .. } => {
                self.pending_ret_collection = None;
                let _ = self.eval_expr(e);
                match self.pending_ret_collection.take() {
                    Some(src) if !is_assoc => {
                        if src != name {
                            self.copy_whole_queue(name, &src);
                        }
                        true
                    }
                    // §13.4, §7.9.9: an associative array returned by a
                    // function (see `snapshot_ret_assoc`).
                    Some(src) if src == Self::RET_ASSOC_SNAP => {
                        self.copy_assoc_entries(name, &src);
                        true
                    }
                    _ => false,
                }
            }
            _ => {
                let src = match &e.kind {
                    ExprKind::Ident(h) => {
                        let mut nm = self.resolve_hier_name(h).into_owned();
                        if let Some(sc) = self.instance_assoc_member(&nm) {
                            nm = sc;
                        }
                        Some(nm)
                    }
                    _ => self.expr_assoc_name(e),
                };
                let Some(src) = src else {
                    return false;
                };
                if src == name {
                    return true;
                }
                if is_assoc && self.is_associative_array(&src) {
                    self.copy_assoc_entries(name, &src);
                    true
                } else if !is_assoc && self.module.dynamic_arrays.contains(&src) {
                    self.copy_whole_queue(name, &src);
                    true
                } else {
                    false
                }
            }
        }
    }

    /// §7.9.9: `dst = src` where one side is a class property (`l_assoc` /
    /// `r_assoc`, from `expr_assoc_name`) and the other a plain module, local
    /// or formal associative variable (`lf`/`rf`: a bare current-frame
    /// formal or local, resolved through the frame only), and §7.10.2.5 /
    /// §7.10.2.6 `got = q.pop_back()` on a queue of associative arrays.
    /// Returns whether it assigned.
    #[inline(never)]
    pub(super) fn assoc_copy_with_plain_side(
        &mut self,
        lvalue: &Expression,
        rvalue: &Expression,
        lf: bool,
        rf: bool,
        l_assoc: &Option<String>,
        r_assoc: &Option<String>,
    ) -> bool {
        // §7.10.2.5 / §7.10.2.6: `got = q.pop_back()` on a queue of
        // associative arrays hands the popped element's entries to an
        // associative target (the element has no single value to return).
        if let ExprKind::Call { func, args } = &rvalue.kind
            && args.is_empty()
            && self.callee_method_name_is(func, |m| matches!(m, "pop_front" | "pop_back"))
            && let Some(dst) = l_assoc
                .clone()
                .or_else(|| self.plain_assoc_operand(lvalue))
                .filter(|n| self.is_associative_array(n))
            && let Some((qn, m)) = self.collection_method_of(func)
        {
            let sz = self.get_queue_size(&qn);
            let elem = format!(
                "{}[{}]",
                qn,
                if m == "pop_front" { 0 } else { sz.max(1) - 1 }
            );
            if sz > 0 && self.module.associative_arrays.contains_key(&elem) {
                self.copy_assoc_entries(&dst, &elem);
                self.eval_expr(rvalue);
                self.settle_after_proc_write();
                return true;
            }
        }
        let plain_pair = if lf || rf {
            // A formal side must itself be associative before the property
            // side is resolved: most formals are not.
            match (lf, rf) {
                (true, false) => self
                    .plain_assoc_operand(lvalue)
                    .and_then(|d| Some((d, self.expr_assoc_name(rvalue)?))),
                (false, true) => self
                    .plain_assoc_operand(rvalue)
                    .and_then(|s| Some((self.expr_assoc_name(lvalue)?, s))),
                _ => None,
            }
        } else {
            match (&l_assoc, &r_assoc) {
                (Some(d), None) => self.plain_assoc_operand(rvalue).map(|s| (d.clone(), s)),
                (None, Some(s)) => self.plain_assoc_operand(lvalue).map(|d| (d, s.clone())),
                (None, None) => self
                    .plain_assoc_operand(lvalue)
                    .and_then(|d| Some((d, self.plain_assoc_operand(rvalue)?))),
                _ => None,
            }
        };
        if let Some((dst, src)) = plain_pair
            && self.is_associative_array(&dst)
            && self.is_associative_array(&src)
        {
            if dst != src {
                self.copy_assoc_entries(&dst, &src);
            }
            return true;
        }
        false
    }

    /// A plain (non-member) associative-array operand — a module variable, or
    /// a local's or formal's per-call storage — as its storage name.
    pub(super) fn plain_assoc_operand(&self, e: &Expression) -> Option<String> {
        let ExprKind::Ident(h) = &e.kind else {
            return None;
        };
        if h.path.len() != 1
            || !h.path[0].selects.is_empty()
            || self.ident_shadowed_by_non_collection(h)
        {
            return None;
        }
        let n = self.resolve_hier_name(h);
        self.is_associative_array(&n).then(|| n.into_owned())
    }

    /// §7.9.9: `dst = src` between associative arrays — `dst` ends up with
    /// exactly `src`'s entries.
    /// Reserved storage for an associative array a function returns.
    pub(super) const RET_ASSOC_SNAP: &'static str = "__xz_ret_aa__";

    /// Copy the returned associative array `src` to the reserved snapshot,
    /// register it and hand it to the caller via `pending_ret_collection`.
    pub(super) fn snapshot_ret_assoc(&mut self, src: &str) {
        let snap = Self::RET_ASSOC_SNAP;
        if src != snap {
            let is_str = self.is_string_keyed_array(src);
            self.module
                .associative_arrays
                .insert(snap.to_string(), is_str);
            self.copy_assoc_entries(snap, src);
        }
        self.pending_ret_collection = Some(snap.to_string());
    }

    /// The storage of an associative-array operand (`aa`, a renamed local,
    /// an instance member), `None` when `e` is not one.
    pub(super) fn ret_assoc_operand_name(&mut self, e: &Expression) -> Option<String> {
        let name = match &e.kind {
            ExprKind::Ident(h) => {
                if self.ident_shadowed_by_non_collection(h) {
                    return None;
                }
                let bare = h.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
                let mut nm = match self.dyn_name_lookup(bare) {
                    Some(r) if h.path.len() == 1 => r.to_string(),
                    _ => self.resolve_hier_name(h).into_owned(),
                };
                if let Some(sc) = self.instance_assoc_member(&nm) {
                    nm = sc;
                }
                nm
            }
            _ => self.expr_assoc_name(e)?,
        };
        self.is_associative_array(&name).then_some(name)
    }

    pub(super) fn copy_assoc_entries(&mut self, dst: &str, src: &str) {
        self.eval_builtin_method(dst, "delete", &[]);
        // §7.9.9: every element, including the member leaves of an
        // unpacked-struct element (`src[k].x`), which do not end in `]`.
        let prefix = format!("{}[", src);
        let entries: Vec<(String, Value)> = self
            .signals
            .keys_with_elem_prefix(&prefix)
            .into_iter()
            .filter_map(|k| {
                let v = self.signals.get(&k)?.clone();
                Some((format!("{}{}", dst, &k[src.len()..]), v))
            })
            .collect();
        let any = !entries.is_empty();
        for (k, v) in entries {
            self.signals.insert(k, v);
        }
        if let Some(d) = self.module.assoc_defaults.get(src).cloned() {
            self.module.assoc_defaults.insert(dst.to_string(), d);
        }
        if any {
            self.mark_struct_coll_owner_present(dst);
        }
    }

    /// Could `lvalue` name a dynamic member of a struct (`x.q`, `a[k].q`)?
    /// Only its trailing member name is consulted.
    #[inline]
    pub(super) fn lvalue_may_name_struct_coll(&self, lvalue: &Expression) -> bool {
        let m = match &lvalue.kind {
            ExprKind::MemberAccess { member, .. } => member.name.as_str(),
            ExprKind::Ident(h) if h.path.len() >= 2 => match h.path.last() {
                Some(seg) => seg.name.name.as_str(),
                None => return false,
            },
            _ => return false,
        };
        self.struct_coll_member_possible(m)
    }

    /// The struct-aggregate assignments the flat-name paths below cannot see:
    /// a whole dynamic member of a struct reached through a select
    /// (`aa[k].q = {1, 2}`, §7.2.1), and a pattern written to an
    /// unpacked-struct class property (§10.9.2). Returns whether it assigned.
    #[cold]
    #[inline(never)]
    pub(super) fn try_struct_aggregate_assign(
        &mut self,
        stmt: &Statement,
        lvalue: &Expression,
        rvalue: &Expression,
    ) -> bool {
        let trailing_member = match &lvalue.kind {
            ExprKind::MemberAccess { member, .. } => Some(member.name.as_str()),
            ExprKind::Ident(h) if h.path.len() >= 2 => h.path.last().map(|s| s.name.name.as_str()),
            _ => None,
        };
        if trailing_member.is_some_and(|m| self.struct_coll_member_possible(m))
            && self.assign_struct_member_coll(lvalue, rvalue)
        {
            return true;
        }
        if let ExprKind::AssignmentPattern(items) = &rvalue.kind
            && !items.is_empty()
            && !self.no_class_objects()
            && Self::receiver_prop_name(lvalue).is_some_and(|p| self.struct_prop_name_possible(p))
            && self.assign_class_struct_prop_pattern(stmt, lvalue, items)
        {
            return true;
        }
        false
    }

    /// `aa[k].q = rhs`, `q[i].d = rhs`: a whole assignment to a dynamic member
    /// of a struct reached through selects (§7.2.1). The flattened-name
    /// collection paths only see a plain identifier target.
    #[cold]
    #[inline(never)]
    pub(super) fn assign_struct_member_coll(
        &mut self,
        lvalue: &Expression,
        rvalue: &Expression,
    ) -> bool {
        // A member of a struct CLASS PROPERTY (`c.s.q`, `s.q` in a method)
        // lives under the instance and has no declared element type there.
        let Some(name) = self.class_struct_coll_name(lvalue) else {
            return false;
        };
        let dt = self.struct_member_coll_of(&name).map(|(_, dt, _)| dt);
        self.assign_whole_coll(&name, dt.as_ref(), rvalue)
    }

    /// §10.9.2/§7.2: an assignment pattern written to an unpacked-struct
    /// CLASS PROPERTY (`s = '{x: 1, q: '{2, 3}}` in a method, `c.s = ...`):
    /// each member is assigned its item through the property's member path,
    /// whose storage is the instance's. Spread by the flat-name machinery,
    /// the pattern landed in module-scope cells nothing reads and every
    /// member kept its old value. A `default:` item fills a nested struct
    /// member member-wise and leaves a dynamic member alone.
    #[cold]
    #[inline(never)]
    pub(super) fn assign_class_struct_prop_pattern(
        &mut self,
        stmt: &Statement,
        lvalue: &Expression,
        items: &[AssignmentPatternItem],
    ) -> bool {
        let Some((handle, prop)) = self.class_prop_receiver(lvalue) else {
            return false;
        };
        // Only a struct-typed property itself — an ARRAY or collection of
        // structs has its own element-wise paths.
        let mut cur = self
            .heap
            .get(handle)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.clone());
        while let Some(cn) = cur {
            let Some(cd) = self.module.classes.get(&cn) else {
                break;
            };
            if cd.array_properties.contains_key(&prop)
                || cd.array_nd_properties.contains_key(&prop)
                || cd.queue_properties.contains_key(&prop)
                || cd.assoc_properties.contains_key(&prop)
            {
                return false;
            }
            if cd.property_types.contains_key(&prop) {
                break;
            }
            cur = cd.extends.clone();
        }
        let Some(su) = self.class_prop_struct(handle, &prop) else {
            return false;
        };
        if !Self::spreads_member_wise(&su) {
            return false;
        }
        self.assign_class_struct_members(stmt, lvalue, &su, items, 0);
        true
    }

    /// The member loop of [`assign_class_struct_prop_pattern`]: each member
    /// of `su` below `lvalue` takes its item; a nested struct member given a
    /// pattern (or a `default:` value) recurses.
    pub(super) fn assign_class_struct_members(
        &mut self,
        stmt: &Statement,
        lvalue: &Expression,
        su: &crate::ast::types::StructUnionType,
        items: &[AssignmentPatternItem],
        depth: usize,
    ) {
        for (mname, mdt, dims, e, explicit) in self.bind_struct_pattern_items(su, items) {
            let Some(e) = e else {
                continue;
            };
            if !explicit && dims.len() == 1 && Self::coll_kind_of(&dims).is_some() {
                continue;
            }
            let lhs = Self::append_member_expr(lvalue, &mname);
            if dims.is_empty()
                && depth < 8
                && let Some(inner) = self.unpacked_struct_of(&mdt)
            {
                match &e.kind {
                    ExprKind::AssignmentPattern(sub) if !sub.is_empty() => {
                        self.assign_class_struct_members(stmt, &lhs, &inner, sub, depth + 1);
                        continue;
                    }
                    _ if !explicit => {
                        let dflt = [AssignmentPatternItem::Default(e.clone())];
                        self.assign_class_struct_members(stmt, &lhs, &inner, &dflt, depth + 1);
                        continue;
                    }
                    _ => {}
                }
            }
            self.exec_stmt_blocking_assign_body(stmt, &lhs, e);
        }
    }

    /// §10.9.2: bind the items of an assignment pattern to the members of
    /// the unpacked struct `su` — `name:` items by member name, bare items
    /// by position, `type:` items by member type, `default:` for the rest.
    /// One entry per member: `(name, type, unpacked dimensions, item,
    /// item-is-explicit)`; `item` is `None` for a member the pattern leaves
    /// alone.
    #[allow(clippy::type_complexity)]
    pub(super) fn bind_struct_pattern_items<'a>(
        &self,
        su: &crate::ast::types::StructUnionType,
        items: &'a [AssignmentPatternItem],
    ) -> Vec<(
        String,
        DataType,
        Vec<crate::ast::types::UnpackedDimension>,
        Option<&'a Expression>,
        bool,
    )> {
        let members: Vec<(String, DataType, Vec<crate::ast::types::UnpackedDimension>)> = su
            .members
            .iter()
            .flat_map(|m| {
                m.declarators.iter().map(move |md| {
                    (
                        md.name.name.clone(),
                        m.data_type.clone(),
                        md.dimensions.clone(),
                    )
                })
            })
            .collect();

        let mut by_name: Vec<Option<&'a Expression>> = vec![None; members.len()];
        let mut ordered: Vec<&'a Expression> = Vec::new();
        let mut default: Option<&'a Expression> = None;
        for item in items {
            match item {
                AssignmentPatternItem::Named(id, e) => {
                    if let Some(i) = members.iter().position(|(n, _, _)| *n == id.name) {
                        by_name[i] = Some(e);
                    }
                }
                AssignmentPatternItem::Default(e) => default = Some(e),
                AssignmentPatternItem::Ordered(e) => ordered.push(e),
                // §10.9.2 `type:value` — binds every member of that type. A
                // `name:` key still wins over it, and `default:` fills the rest.
                AssignmentPatternItem::Typed(key_dt, e) => {
                    for (i, (_, mdt, _)) in members.iter().enumerate() {
                        if by_name[i].is_none() && self.pattern_type_matches(mdt, key_dt) {
                            by_name[i] = Some(e);
                        }
                    }
                }
                _ => {}
            }
        }

        members
            .into_iter()
            .enumerate()
            .map(|(i, (mname, mdt, dims))| {
                let explicit = by_name[i].or_else(|| ordered.get(i).copied());
                let e = explicit.or(default);
                (mname, mdt, dims, e, explicit.is_some())
            })
            .collect()
    }

    /// Spread an assignment pattern across the members of the UNPACKED struct
    /// `su` rooted at `base` (IEEE 1800-2017 §10.9.1): `name:` items bind by
    /// member name, bare items bind positionally, `default:` fills the rest.
    pub(super) fn assign_pattern_into_struct(
        &mut self,
        base: &str,
        su: &crate::ast::types::StructUnionType,
        items: &[AssignmentPatternItem],
    ) {
        for (mname, mdt, dims, e, explicit) in self.bind_struct_pattern_items(su, items) {
            let Some(e) = e else {
                continue;
            };
            let target = format!("{}.{}", base, mname);
            if dims.is_empty() {
                self.assign_pattern_or_leaf(&target, &mdt, e);
                continue;
            }
            // §10.9.2/§7.2.1: a DYNAMIC member (queue / dynamic / associative)
            // is a whole collection, assigned like any other — its item is a
            // collection-valued expression (`'{7, 8}`, `{q, 4}`, `other.q`)
            // that REPLACES the old contents. A `default:` value names an
            // element, never a whole collection, so it leaves one alone.
            if dims.len() == 1 && Self::coll_kind_of(&dims).is_some() {
                if explicit && self.ensure_struct_member_coll(&target) {
                    self.assign_whole_coll(&target, Some(&mdt), e);
                }
                continue;
            }
            // Array member: spread over every dimension.
            let Some(lists) = self.member_dim_indices(&dims) else {
                continue;
            };
            self.assign_pattern_over_dims(&target, &mdt, &lists, e);
        }
    }

    /// Spread one pattern item over the unpacked array rooted at `target`,
    /// whose remaining dimensions are `lists` (declared order, outermost
    /// first). IEEE 1800-2017 §10.10: a nested pattern supplies the elements
    /// of the outermost dimension left bound first — positionally, by
    /// `index:` key, or through `default:` for any element left unset — and
    /// each item recurses into the next dimension. Anything that is not a
    /// pattern (a `default:` value of the enclosing struct pattern, say)
    /// fills every element below.
    pub(super) fn assign_pattern_over_dims(
        &mut self,
        target: &str,
        dt: &DataType,
        lists: &[Vec<i64>],
        e: &Expression,
    ) {
        let Some((first, rest)) = lists.split_first() else {
            self.assign_pattern_or_leaf(target, dt, e);
            return;
        };
        match &e.kind {
            ExprKind::AssignmentPattern(sub) => {
                self.assign_pattern_items_over_dims(target, dt, lists, sub);
            }
            _ => {
                for idx in first {
                    self.assign_pattern_over_dims(&format!("{}[{}]", target, idx), dt, rest, e);
                }
            }
        }
    }

    /// The pattern-items half of [`assign_pattern_over_dims`]: `sub` are the
    /// items of a pattern over the outermost of `lists`.
    pub(super) fn assign_pattern_items_over_dims(
        &mut self,
        target: &str,
        dt: &DataType,
        lists: &[Vec<i64>],
        sub: &[AssignmentPatternItem],
    ) {
        let Some((first, rest)) = lists.split_first() else {
            return;
        };
        let mut per_elem: Vec<Option<&Expression>> = vec![None; first.len()];
        let mut dflt: Option<&Expression> = None;
        let mut pos = 0usize;
        for item in sub {
            match item {
                AssignmentPatternItem::Ordered(v) => {
                    if let Some(slot) = per_elem.get_mut(pos) {
                        *slot = Some(v);
                    }
                    pos += 1;
                }
                AssignmentPatternItem::Keyed(k, v) => {
                    if let Some(key) = self.eval_expr(k).to_i64() {
                        if let Some(p) = first.iter().position(|&i| i == key) {
                            per_elem[p] = Some(v);
                        }
                    }
                }
                AssignmentPatternItem::Default(v) => dflt = Some(v),
                _ => {}
            }
        }
        for (p, idx) in first.iter().enumerate() {
            if let Some(v) = per_elem[p].or(dflt) {
                self.assign_pattern_over_dims(&format!("{}[{}]", target, idx), dt, rest, v);
            }
        }
    }

    /// Whether `src` actually has per-member storage for `su` — guards against
    /// treating an unrelated same-named scalar as a struct source.
    /// §6.8/§7.2.1: a member of an UNPACKED struct with a 2-STATE type
    /// (bit/byte/int/...) defaults to 0. Member-wise struct storage is lazy
    /// (absent leaf reads x), so never-written 2-state members read x —
    /// seed a zero for each such leaf at startup. Declared initializers run
    /// later and simply overwrite the seed.
    pub(super) fn init_two_state_struct_members(&mut self) {
        let decls: Vec<(String, crate::ast::types::DataType)> = self
            .module
            .var_decl_types
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (vname, dt) in decls {
            // var_decl_types drops unpacked dims — a queue/array/assoc OF
            // structs resolves to the element struct here. Seeding member
            // leaves on the CONTAINER name would corrupt its storage.
            if self.module.dynamic_arrays.contains(&vname) || self.is_associative_array(&vname) {
                continue;
            }
            // A FIXED array of unpacked structs: every element's 2-state
            // members default to 0 as well (§7.2.1, §7.4) — seeded per
            // element leaf (`arr[1].a`), never on the container name.
            if self.packed_type_is_mixed(&dt) {
                self.mixed_packed_vars.insert(vname.clone());
            }
            let fixed_1d = self.module.arrays.get(&vname).copied();
            let fixed_2d = self.module.arrays_2d.get(&vname).copied();
            if fixed_1d.is_some() || fixed_2d.is_some() {
                let su = match self.resolve_dt(&dt) {
                    DataType::Struct(su) if Self::spreads_member_wise(&su) => su,
                    _ => continue,
                };
                let elems: Vec<String> = match (fixed_1d, fixed_2d) {
                    (Some((lo, hi, _)), _) if hi >= lo && hi - lo < 1 << 16 => {
                        (lo..=hi).map(|i| format!("{}[{}]", vname, i)).collect()
                    }
                    (None, Some(((l0, h0), (l1, h1), _)))
                        if h0 >= l0 && h1 >= l1 && (h0 - l0 + 1) * (h1 - l1 + 1) <= 1 << 16 =>
                    {
                        (l0..=h0)
                            .flat_map(|i| (l1..=h1).map(move |j| (i, j)))
                            .map(|(i, j)| format!("{}[{}][{}]", vname, i, j))
                            .collect()
                    }
                    _ => Vec::new(),
                };
                for e in elems {
                    self.zero_two_state_members(&e, &su);
                }
                continue;
            }
            if let DataType::Struct(su) = self.resolve_dt(&dt) {
                // §7.2.1: a packed struct with any 4-state member is 4-state
                // as a whole and starts all x; its 2-state members convert
                // on read.
                if su.packed
                    && !super::elaborate::is_type_two_state_resolved(
                        &dt,
                        &self.module.typedef_types,
                    )
                {
                    continue;
                }
                // Packed-style storage: the var is ONE signal and members
                // are field slices — zero the 2-state members' X bits in
                // place (4-state members stay x).
                let fields = self.module.packed_struct_fields.get(&vname).cloned();
                let cur = self.get_signal_value_by_name(&vname);
                if let (Some(fields), Some(mut cur)) = (fields, cur) {
                    let mut ts: HashSet<String> = HashSet::default();
                    Self::collect_two_state_leaves(self, &su, "", &mut ts);
                    let mut changed = false;
                    for (fname, off, w) in fields {
                        if !ts.contains(&fname) {
                            continue;
                        }
                        for i in 0..w {
                            let idx = (off + i) as usize;
                            if idx < cur.width as usize
                                && !matches!(cur.get_bit(idx), LogicBit::Zero | LogicBit::One)
                            {
                                cur.set_bit(idx, LogicBit::Zero);
                                changed = true;
                            }
                        }
                    }
                    if changed {
                        self.set_signal_value_by_name(&vname, cur);
                    }
                    continue;
                }
                if Self::spreads_member_wise(&su) {
                    self.zero_two_state_members(&vname, &su.clone());
                }
            }
        }
        if !self.mixed_packed_vars.is_empty() {
            let mixed = &self.mixed_packed_vars;
            self.bc_packed_fields = Some(
                self.module
                    .packed_struct_fields
                    .iter()
                    .filter(|(k, _)| !mixed.contains(k.as_str()))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            );
        }
    }

    /// §7.2.1: a packed struct type that is 4-state (some member is) and
    /// also has a 2-state member, whose reads and writes convert.
    pub(super) fn packed_type_is_mixed(&self, dt: &DataType) -> bool {
        let DataType::Struct(su) = self.resolve_dt_ref(dt) else {
            return false;
        };
        if !su.packed
            || super::elaborate::is_type_two_state_resolved(dt, &self.module.typedef_types)
        {
            return false;
        }
        let mut ts: HashSet<String> = HashSet::default();
        self.collect_two_state_leaves(su, "", &mut ts);
        !ts.is_empty()
    }

    /// §7.2.1: whether `field` (a dotted member path) of the packed-struct
    /// variable `root` is a 2-state member of a 4-state struct, so a value
    /// read from or written to it converts x/z to 0.
    #[inline(never)]
    pub(super) fn packed_member_two_state(&self, root: &str, field: &str) -> bool {
        // An element of an array of such structs (`arr[1]`).
        let root = root.split('[').next().unwrap_or(root);
        if !self.mixed_packed_vars.contains(root) {
            return false;
        }
        let Some(dt) = self.module.var_decl_types.get(root) else {
            return false;
        };
        let DataType::Struct(su) = self.resolve_dt_ref(dt) else {
            return false;
        };
        let mut ts: HashSet<String> = HashSet::default();
        self.collect_two_state_leaves(su, "", &mut ts);
        ts.contains(field)
    }

    /// §7.2.1: the value written to member `name` (`<base>.<field>`) of a
    /// packed struct — x/z bits become 0 for a 2-state member of a 4-state
    /// struct.
    #[inline(never)]
    pub(super) fn packed_member_piece(&self, base: &str, name: &str, piece: Value) -> Value {
        if !piece.has_xz() || self.mixed_packed_vars.is_empty() {
            return piece;
        }
        match name.strip_prefix(base).and_then(|f| f.strip_prefix('.')) {
            Some(field) if self.packed_member_two_state(base, field) => {
                let signed = piece.is_signed;
                let mut v = piece.to_two_state();
                v.is_signed = signed;
                v
            }
            _ => piece,
        }
    }

    /// §7.2.1: whether member type `dt` of packed struct `su` is 2-state
    /// while `su` as a whole is 4-state.
    pub(super) fn mixed_member_two_state(
        &self,
        su: &crate::ast::types::StructUnionType,
        dt: &DataType,
    ) -> bool {
        let tt = &self.module.typedef_types;
        su.packed
            && super::elaborate::is_type_two_state_resolved(dt, tt)
            && !su
                .members
                .iter()
                .all(|m| super::elaborate::is_type_two_state_resolved(&m.data_type, tt))
    }

    /// §7.2.1: the 2-state form of `val` when `lhs` names a 2-state member
    /// of a 4-state packed-struct variable (`s.b`, `s.inner.b`, `s.b[1]`);
    /// `None` otherwise.
    #[inline(never)]
    pub(super) fn mixed_packed_member_write(
        &mut self,
        lhs: &Expression,
        val: &Value,
    ) -> Option<Value> {
        let mut cur = lhs;
        while let ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } = &cur.kind {
            cur = expr;
        }
        let (root, field) = match &cur.kind {
            ExprKind::Ident(h) if h.path.len() >= 2 => {
                let names: Vec<&str> = h.path.iter().map(|s| s.name.name.as_str()).collect();
                (1..names.len())
                    .rev()
                    .map(|k| (names[..k].join("."), names[k..].join(".")))
                    .find(|(r, _)| self.resolve_mixed_root(r).is_some())?
            }
            ExprKind::MemberAccess { .. } => {
                let flat = self.flat_member_name(cur)?;
                let (r, f) = flat.split_once('.')?;
                (r.to_string(), f.to_string())
            }
            _ => return None,
        };
        let root = self.resolve_mixed_root(&root)?;
        if !self.packed_member_two_state(&root, &field) {
            return None;
        }
        let mut v = val.to_two_state();
        v.is_signed = val.is_signed;
        Some(v)
    }

    /// The `mixed_packed_vars` entry `root` names, directly or under the
    /// current scope hint.
    pub(super) fn resolve_mixed_root(&self, root: &str) -> Option<String> {
        if self.mixed_packed_vars.contains(root) {
            return Some(root.to_string());
        }
        let hint = self.name_resolve_hint.borrow();
        let q = format!("{}.{}", hint.as_deref()?, root);
        self.mixed_packed_vars.contains(q.as_str()).then_some(q)
    }

    /// Relative paths of 2-STATE leaf members of `su` (recursing into
    /// nested structs), matching packed_struct_fields' key format.
    pub(super) fn collect_two_state_leaves(
        &self,
        su: &crate::ast::types::StructUnionType,
        prefix: &str,
        out: &mut HashSet<String>,
    ) {
        use crate::ast::types::{DataType as DT, IntegerAtomType as IAT, IntegerVectorType as IVT};
        for m in &su.members {
            let resolved = self.resolve_dt(&m.data_type);
            for md in &m.declarators {
                let leaf = if prefix.is_empty() {
                    md.name.name.clone()
                } else {
                    format!("{}.{}", prefix, md.name.name)
                };
                match &resolved {
                    DT::Struct(inner) => {
                        self.collect_two_state_leaves(inner, &leaf, out);
                    }
                    DT::IntegerVector { kind: IVT::Bit, .. }
                    | DT::IntegerAtom {
                        kind: IAT::Byte | IAT::ShortInt | IAT::Int | IAT::LongInt,
                        ..
                    } => {
                        out.insert(leaf);
                    }
                    _ => {}
                }
            }
        }
    }

    pub(super) fn zero_two_state_members(
        &mut self,
        base: &str,
        su: &crate::ast::types::StructUnionType,
    ) {
        use crate::ast::types::{DataType as DT, IntegerAtomType as IAT, IntegerVectorType as IVT};
        for m in &su.members {
            let resolved = self.resolve_dt(&m.data_type);
            for md in &m.declarators {
                let mbase = format!("{}.{}", base, md.name.name);
                // §7.4.2: a member with fixed unpacked dimensions is one leaf
                // per element, each defaulting like a scalar member; a
                // dynamic member has no leaves to seed.
                let leaves: Vec<String> = if md.dimensions.is_empty() {
                    vec![mbase]
                } else {
                    match self.member_elem_indices(&md.dimensions) {
                        Some(tups) => tups
                            .iter()
                            .map(|t| format!("{}{}", mbase, Self::index_suffix(t)))
                            .collect(),
                        None => continue,
                    }
                };
                for leaf in leaves {
                    match &resolved {
                        DT::Struct(inner) if Self::spreads_member_wise(inner) => {
                            let inner = inner.clone();
                            self.zero_two_state_members(&leaf, &inner);
                        }
                        // §8.4: a class-handle member starts null.
                        DT::TypeReference { .. }
                            if Self::dt_is_class_handle(&resolved, &self.module) =>
                        {
                            let cur = self.get_signal_value_by_name(&leaf);
                            if let Some(v) = cur.filter(|v| v.has_xz()) {
                                self.set_signal_value_by_name(&leaf, Value::zero(v.width.max(1)));
                            }
                        }
                        DT::IntegerVector { kind: IVT::Bit, .. }
                        | DT::IntegerAtom {
                            kind: IAT::Byte | IAT::ShortInt | IAT::Int | IAT::LongInt,
                            ..
                        } => {
                            // The leaf is 2-state: later writes drop X/Z too.
                            if let Some(&id) = self.signal_name_to_id.get(leaf.as_str()) {
                                if let Some(t) = self.signal_two_state.get_mut(id) {
                                    *t = true;
                                }
                            }
                            // Overwrite an ALL-X current value too: the member
                            // signal may be pre-registered x-filled. A restored
                            // declared initializer is never fully x, so it wins.
                            let cur = self.get_signal_value_by_name(&leaf);
                            // Compared with the flag cleared: a signed
                            // member (`int`, `byte`) is pre-registered as a
                            // SIGNED all-x value, which never equalled the
                            // unsigned `Value::new`, so those members kept x.
                            let untouched = cur.as_ref().is_none_or(|v| {
                                let mut u = v.clone();
                                u.is_signed = false;
                                u == Value::new(v.width)
                            });
                            if untouched {
                                let w = cur.map(|v| v.width).unwrap_or_else(|| {
                                    crate::elaborate::resolve_type_width(
                                        &m.data_type,
                                        Some(&self.module.parameters),
                                        Some(&self.module.typedefs),
                                    )
                                    .max(1)
                                });
                                self.set_signal_value_by_name(&leaf, Value::zero(w));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    pub(super) fn struct_storage_exists(
        &self,
        src: &str,
        su: &crate::ast::types::StructUnionType,
    ) -> bool {
        su.members.iter().any(|m| {
            m.declarators.iter().any(|md| {
                let mut leaf = format!("{}.{}", src, md.name.name);
                // A member with fixed unpacked dimensions is stored per
                // element, and a nested unpacked struct per leaf: probe the
                // first of them (a struct whose members are all arrays, like
                // `struct { bit [7:0] m [2][2]; }`, has no `src.m` at all).
                if let Some(lists) = self.member_dim_indices(&md.dimensions) {
                    let first: Vec<i64> = lists.iter().map(|l| l[0]).collect();
                    leaf.push_str(&Self::index_suffix(&first));
                }
                if let Some(inner) = self.unpacked_struct_of(&m.data_type) {
                    if self.struct_storage_exists(&leaf, &inner) {
                        return true;
                    }
                }
                self.signal_name_to_id.contains_key(leaf.as_str())
                    || self.signals.contains_key(&leaf)
                    // a subroutine-local struct / member-wise-bound FORMAL
                    // keeps its leaves in the call frame (§13.4.2) — without
                    // this check a whole-struct copy from a formal declined
                    // and fell to a packed spread that read x.
                    || self
                        .local_stack
                        .last()
                        .is_some_and(|f| f.contains_key(&leaf))
                    // a member that is itself a dynamic collection has no
                    // scalar leaf — its registration is the storage
                    || self.module.dynamic_arrays.contains(&leaf)
                    || self.signals.contains_key(&format!("{}.size", leaf))
            })
        })
    }

    /// IEEE 1800-2017 §10.4.2: the per-leaf updates of a nonblocking
    /// assignment whose target is a whole UNPACKED struct (a variable, an
    /// array element or a struct-typed member), as `(leaf lvalue, value)`
    /// pairs with every source value read now. The struct has no storage of
    /// its own, so the single update the generic path queues against the
    /// struct's name landed nowhere and every member — scalar ones included —
    /// read back garbage. `None` when the target is anything else.
    /// §7.6 / §10.4.2: whole fixed arrays assign by declared position.
    /// Snapshot every source before posting any leaf update, including aliases.
    pub(super) fn unpacked_array_nba_leaves(
        &mut self,
        lvalue: &Expression,
        rvalue: &Expression,
    ) -> Option<Vec<(Expression, Value)>> {
        let dst = self.foreach_check_operand_name(lvalue)?;
        let src = self.foreach_check_operand_name(rvalue)?;
        if self.module.dynamic_arrays.contains(&dst)
            || self.module.dynamic_arrays.contains(&src)
            || self.is_associative_array(&dst)
            || self.is_associative_array(&src)
        {
            return None;
        }
        let (targets, _) = self.stream_elem_order(&dst)?;
        let (sources, _) = self.stream_elem_order(&src)?;
        if targets.len() != sources.len() {
            return None;
        }
        let mut leaves = Vec::with_capacity(targets.len());
        for (target, source) in targets.into_iter().zip(sources) {
            let lhs = Self::append_leaf_path(lvalue, &target[dst.len()..]);
            let rhs = Self::append_leaf_path(rvalue, &source[src.len()..]);
            if self.queue_elem_struct(&src).is_some()
                && let Some(member_leaves) = self.unpacked_struct_nba_leaves(&lhs, &rhs)
            {
                leaves.extend(member_leaves);
                continue;
            }
            let value = self.eval_expr(&rhs);
            leaves.push((lhs, value));
        }
        Some(leaves)
    }

    pub(super) fn unpacked_struct_nba_leaves(
        &mut self,
        lvalue: &Expression,
        rvalue: &Expression,
    ) -> Option<Vec<(Expression, Value)>> {
        match &lvalue.kind {
            ExprKind::Ident(h) if h.path.iter().all(|s| s.selects.is_empty()) => {}
            ExprKind::MemberAccess { .. } | ExprKind::Index { .. } => {}
            _ => return None,
        }
        if matches!(rvalue.kind, ExprKind::AssignmentPattern(_))
            || !Self::expr_root_first_seg(lvalue).is_some_and(|r| self.struct_root_possible(r))
        {
            return None;
        }
        let dst = self.flat_member_name(lvalue)?;
        let (dst, su) = self.struct_copy_target(dst);
        let su = su?;
        if self.module.dynamic_arrays.contains(&dst)
            || self.module.arrays.contains_key(&dst)
            || self.module.associative_arrays.contains_key(&dst)
        {
            return None;
        }
        let mut leaves = Vec::new();
        self.unpacked_struct_leaves(&dst, lvalue, &su, 0, &mut leaves);
        if leaves.is_empty() {
            return None;
        }
        // A source with storage of its own is read leaf by leaf, like the
        // blocking copy; anything else (a call, a class property) is one
        // packed value laid out like `struct_leaf_layout` — reverse leaf
        // order, ascending offsets.
        let src = self
            .flat_member_name(rvalue)
            .filter(|s| self.struct_storage_exists(s, &su));
        // Each leaf target in the parse shape of its source text
        // (`b.mm[1][0]`), which every NBA target resolver understands.
        let leaves: Vec<(String, Expression, u32, bool)> = leaves
            .into_iter()
            .map(|(key, _, w, is_real)| {
                let lexpr = Self::append_leaf_path(lvalue, &key[dst.len()..]);
                (key, lexpr, w, is_real)
            })
            .collect();
        let mut out = Vec::with_capacity(leaves.len());
        match src {
            Some(src) => {
                for (key, lexpr, _, _) in leaves {
                    let skey = Self::name_with_suffix(&src, &key[dst.len()..]);
                    if let Some(v) = self.get_signal_value_by_name(&skey) {
                        out.push((lexpr, v));
                    }
                }
            }
            None => {
                let v = self.eval_expr(rvalue);
                let mut off = 0u32;
                let mut rev = Vec::with_capacity(leaves.len());
                for (_, lexpr, w, is_real) in leaves.into_iter().rev() {
                    let mut mv = Value::new(w);
                    for i in 0..w {
                        mv.set_bit(i as usize, v.get_bit((off + i) as usize));
                    }
                    off += w;
                    if is_real {
                        mv = Value::from_f64(f64::from_bits(mv.to_u64().unwrap_or(0)));
                    }
                    rev.push((lexpr, mv));
                }
                out.extend(rev.into_iter().rev());
            }
        }
        Some(out)
    }

    /// `base` followed by the member / index path `suffix` of a leaf key
    /// (`.mm[1][0]`, `.body.sd[1]`), built in the parse shape of the same
    /// source text: a member step extends an identifier path, an index step
    /// wraps in `Index`.
    pub(super) fn append_leaf_path(base: &Expression, suffix: &str) -> Expression {
        let mut out = base.clone();
        let mut rest = suffix;
        loop {
            if let Some(r) = rest.strip_prefix('.') {
                let end = r.find(['.', '[']).unwrap_or(r.len());
                out = Self::append_member_expr(&out, &r[..end]);
                rest = &r[end..];
            } else if let Some(r) = rest.strip_prefix('[') {
                let Some(end) = r.find(']') else { break };
                let Ok(i) = r[..end].parse::<i64>() else {
                    break;
                };
                out = Self::index_expr(&out, i);
                rest = &r[end + 1..];
            } else {
                break;
            }
        }
        out
    }

    /// Copy every leaf of an unpacked struct from `src` to `dst`.
    pub(super) fn copy_unpacked_struct(
        &mut self,
        dst: &str,
        src: &str,
        su: &crate::ast::types::StructUnionType,
    ) {
        for m in &su.members {
            for md in &m.declarators {
                let d = format!("{}.{}", dst, md.name.name);
                let sname = format!("{}.{}", src, md.name.name);
                if md.dimensions.is_empty() {
                    if let DataType::Struct(inner) = self.resolve_dt(&m.data_type) {
                        if Self::spreads_member_wise(&inner) {
                            self.copy_unpacked_struct(&d, &sname, &inner);
                            continue;
                        }
                    }
                    if let Some(v) = self.get_signal_value_by_name(&sname) {
                        self.write_leaf_by_name(&d, v);
                    }
                } else if md.dimensions.iter().any(|dim| {
                    matches!(
                        dim,
                        crate::ast::types::UnpackedDimension::Unsized(_)
                            | crate::ast::types::UnpackedDimension::Queue { .. }
                            | crate::ast::types::UnpackedDimension::Associative { .. }
                    )
                }) {
                    // §7.2.1: a DYNAMIC member (queue/dynamic array) copies by
                    // VALUE — size + elements. member_dim_indices returns
                    // None for these, so the member was skipped entirely and
                    // `b = a` lost `a.d[]`'s contents.
                    // The destination may be a composed name elaboration never
                    // saw (`pq[0].data`) — register it so `.size()`/push_back
                    // on it hit the collection paths instead of reading 0.
                    // §7.9.9: an ASSOCIATIVE member copies its entries by key.
                    if md.dimensions.len() == 1
                        && matches!(
                            md.dimensions[0],
                            crate::ast::types::UnpackedDimension::Associative { .. }
                        )
                    {
                        self.ensure_struct_member_coll(&sname);
                        if !self.ensure_struct_member_coll(&d) {
                            let sk = self.string_keyed_dim(&md.dimensions[0]);
                            self.register_struct_member_coll_as(
                                &d,
                                StructCollKind::Assoc,
                                &m.data_type,
                                sk,
                            );
                        }
                        self.copy_assoc_entries(&d, &sname);
                        continue;
                    }
                    if !self.ensure_struct_member_coll(&d) {
                        self.module.dynamic_arrays.insert(d.clone());
                    }
                    let n = self.get_queue_size(&sname);
                    for j in 0..n {
                        if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", sname, j))
                        {
                            self.set_signal_value_by_name(&format!("{}[{}]", d, j), v);
                        }
                    }
                    self.set_queue_size(&d, n);
                } else if let Some(tups) = self.member_elem_indices(&md.dimensions) {
                    // An array of unpacked STRUCTS has no element leaf of its
                    // own — each element copies member by member.
                    let elem_su = self.unpacked_struct_of(&m.data_type);
                    for t in tups {
                        let sfx = Self::index_suffix(&t);
                        let (di, si) = (format!("{}{}", d, sfx), format!("{}{}", sname, sfx));
                        if let Some(inner) = &elem_su {
                            self.copy_unpacked_struct(&di, &si, inner);
                        } else if let Some(v) = self.get_signal_value_by_name(&si) {
                            self.write_leaf_by_name(&di, v);
                        }
                    }
                }
            }
        }
    }

    /// The unpacked-struct element type of queue `obj_name`, if it has one.
    pub(super) fn queue_elem_struct(
        &self,
        obj_name: &str,
    ) -> Option<crate::ast::types::StructUnionType> {
        if let Some(dt) = self.class_member_coll_type(obj_name) {
            return match self.resolve_dt_ref(dt) {
                DataType::Struct(su) if Self::spreads_member_wise(su) => Some(su.clone()),
                _ => None,
            };
        }
        if let Some(dt) = self.p_elem_type_ref(obj_name) {
            if let DataType::Struct(su) = self.resolve_dt_ref(&dt) {
                if Self::spreads_member_wise(su) {
                    return Some(su.clone());
                }
            }
            return None;
        }
        // STATIC class collections store globally under the bare name but
        // record their element type in the declaring CLASS (`static
        // rsrc_info_t ri_tab [uvm_resource_base]` — UVM's resource pool).
        // Without this the struct-element write gate never matched and
        // `ri_tab[key].scope = v` was dropped (issue #113).
        if !obj_name.contains('#') && !obj_name.contains('.') {
            for cd in self.module.classes.values() {
                if cd.static_collections.iter().any(|(n, _, _)| n == obj_name) {
                    if let Some(dt) = cd.property_types.get(obj_name) {
                        // The element type may be a CLASS-LOCAL typedef
                        // (`typedef struct {...} rsrc_info_t;`) invisible to
                        // the module-level resolve — chase it through the
                        // class's own typedef table first.
                        let resolved = match dt {
                            DataType::TypeReference { name: tn, .. } => cd
                                .typedef_targets
                                .get(&tn.name.name)
                                .cloned()
                                .unwrap_or_else(|| dt.clone()),
                            other => other.clone(),
                        };
                        if let DataType::Struct(su) = self.resolve_dt(&resolved) {
                            if Self::spreads_member_wise(&su) {
                                return Some(su);
                            }
                        }
                    }
                }
            }
        }
        // INSTANCE member `<handle>#<member>`: the element type lives in the
        // declaring class's `property_types` (same typedef-chase as the
        // static case). Without this a class-member dyn array OF STRUCTS
        // never matched the struct-element write gate and `slices[t].path =
        // v` was dropped (UVM's uvm_hdl_path_concat, so every backdoor
        // access saw empty hdl-path slices).
        if let Some(pos) = obj_name.find('#') {
            if let Ok(handle) = obj_name[..pos].parse::<usize>() {
                let member = &obj_name[pos + 1..];
                if !member.contains('[') {
                    let mut cur = self
                        .heap
                        .get(handle)
                        .and_then(|o| o.as_ref())
                        .map(|i| i.class_name.clone());
                    while let Some(cn) = cur {
                        let Some(cd) = self.module.classes.get(&cn) else {
                            break;
                        };
                        if let Some(dt) = cd.property_types.get(member) {
                            let resolved = match dt {
                                DataType::TypeReference { name: tn, .. } => cd
                                    .typedef_targets
                                    .get(&tn.name.name)
                                    .cloned()
                                    .unwrap_or_else(|| dt.clone()),
                                other => other.clone(),
                            };
                            if let DataType::Struct(su) = self.resolve_dt(&resolved) {
                                if Self::spreads_member_wise(&su) {
                                    return Some(su);
                                }
                            }
                            return None;
                        }
                        cur = cd.extends.clone();
                    }
                }
            }
        }
        None
    }

    /// Copy element `si` of queue `src_obj` onto element `di` of `dst_obj`,
    /// member-wise when the element is an unpacked struct (a single packed copy
    /// would lose every member — the struct has no container signal).
    pub(super) fn queue_copy_elem(&mut self, src_obj: &str, si: u64, dst_obj: &str, di: u64) {
        let (src, dst) = (
            format!("{}[{}]", src_obj, si),
            format!("{}[{}]", dst_obj, di),
        );
        if let Some(su) = self.queue_elem_struct(src_obj) {
            self.copy_unpacked_struct(&dst, &src, &su);
            return;
        }
        // Queue of FIXED arrays: an element is a whole ROW of `[j]` slots.
        if let Some(&(_, (jlo, jhi), _)) = self.module.arrays_2d.get(src_obj) {
            for j in jlo..=jhi {
                let v = self
                    .get_signal_value_by_name(&format!("{}[{}]", src, j))
                    .unwrap_or_else(|| Value::zero(32));
                self.set_signal_value_by_name(&format!("{}[{}]", dst, j), v);
            }
            return;
        }
        // A queue of ASSOCIATIVE arrays: an element is a set of keyed entries.
        if self.module.associative_arrays.contains_key(&src) {
            self.store_assoc_elem(&dst, &src);
            return;
        }
        if let Some(v) = self.get_signal_value_by_name(&src) {
            self.set_signal_value_by_name(&dst, v);
        }
    }

    /// An associative-array VALUE operand (`src`, `h.m`, a formal or local):
    /// its storage name.
    pub(super) fn assoc_value_operand(&mut self, e: &Expression) -> Option<String> {
        if !matches!(e.kind, ExprKind::Ident(_) | ExprKind::MemberAccess { .. }) {
            return None;
        }
        self.plain_assoc_operand(e)
            .or_else(|| self.expr_assoc_name(e))
            .filter(|n| self.is_associative_array(n))
    }

    /// §7.8: make collection element `elem` an associative array holding
    /// exactly the entries of associative array `src`.
    pub(super) fn store_assoc_elem(&mut self, elem: &str, src: &str) {
        let is_str = self.is_string_keyed_array(src);
        self.module
            .associative_arrays
            .insert(elem.to_string(), is_str);
        if let Some(w) = self.module.assoc_elem_widths.get(src).copied() {
            self.module.assoc_elem_widths.insert(elem.to_string(), w);
        }
        self.copy_assoc_entries(elem, src);
    }

    /// §7.2.1: copy the unpacked struct class property `key` of object `h`
    /// (`prop`, or a nested `prop.inner`) onto the flat struct storage `dst`
    /// member by member: a leaf from the object's cell, each element of a
    /// fixed member array, and a dynamic member (declared so or through a
    /// collection typedef) as a whole collection from `<h>#<key>.<m>`.
    pub(super) fn copy_class_struct_prop_to(
        &mut self,
        dst: &str,
        h: usize,
        key: &str,
        su: &crate::ast::types::StructUnionType,
    ) {
        for m in &su.members {
            for md in &m.declarators {
                let d = format!("{}.{}", dst, md.name.name);
                let k = format!("{}.{}", key, md.name.name);
                let coll_dims = if !md.dimensions.is_empty() {
                    Some(md.dimensions.clone())
                } else if let DataType::TypeReference {
                    name, dimensions, ..
                } = &m.data_type
                    && dimensions.is_empty()
                    && !name.has_scope()
                {
                    self.class_chain_typedef_dims(h, &name.name.name)
                } else {
                    None
                };
                if let Some(cd) = coll_dims.as_deref()
                    && let Some(kind) = Self::coll_kind_of(cd)
                {
                    let string_key = self.string_keyed_dim(&cd[0]);
                    let ew = resolve_type_width(
                        &m.data_type,
                        Some(&self.module.parameters),
                        Some(&self.module.typedefs),
                    )
                    .max(1);
                    self.register_struct_coll(h, &k, kind, ew, string_key);
                    if !self.ensure_struct_member_coll(&d) {
                        self.register_struct_member_coll_as(&d, kind, &m.data_type, string_key);
                    }
                    self.clear_coll(&d, kind);
                    self.copy_coll(&d, &format!("{}#{}", h, k), kind);
                    continue;
                }
                if md.dimensions.is_empty() {
                    if let DataType::Struct(inner) = self.resolve_dt(&m.data_type)
                        && Self::spreads_member_wise(&inner)
                    {
                        self.copy_class_struct_prop_to(&d, h, &k, &inner);
                        continue;
                    }
                    let v = self
                        .heap
                        .get(h)
                        .and_then(|o| o.as_ref())
                        .and_then(|i| i.properties.get(&k))
                        .cloned();
                    if let Some(v) = v {
                        self.write_leaf_by_name(&d, v);
                    }
                } else if let Some(tups) = self.member_elem_indices(&md.dimensions) {
                    let inner = self.unpacked_struct_of(&m.data_type);
                    for t in tups {
                        let sfx = Self::index_suffix(&t);
                        let (de, ke) = (format!("{}{}", d, sfx), format!("{}{}", k, sfx));
                        match &inner {
                            Some(inner) if Self::spreads_member_wise(inner) => {
                                self.copy_class_struct_prop_to(&de, h, &ke, inner)
                            }
                            _ => {
                                let v = self
                                    .heap
                                    .get(h)
                                    .and_then(|o| o.as_ref())
                                    .and_then(|i| i.properties.get(&ke))
                                    .cloned();
                                if let Some(v) = v {
                                    self.write_leaf_by_name(&de, v);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// The unpacked dimensions of typedef `name` as object `h`'s class chain
    /// sees it: a class-local typedef first, then the design's.
    pub(super) fn class_chain_typedef_dims(
        &self,
        h: usize,
        name: &str,
    ) -> Option<Vec<crate::ast::types::UnpackedDimension>> {
        let mut cur = self
            .heap
            .get(h)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.as_str());
        let mut hops = 0;
        while let Some(cn) = cur {
            hops += 1;
            let cd = self.module.classes.get(cn).filter(|_| hops < 64)?;
            if let Some(d) = cd.typedef_unpacked_dims.get(name) {
                return Some(d.clone());
            }
            cur = cd.extends.as_deref();
        }
        self.typedef_unpacked_dims_of(name).cloned()
    }

    /// Move queue element `from` to `to` within one queue.
    pub(super) fn queue_move_elem(&mut self, obj_name: &str, from: u64, to: u64) {
        self.queue_copy_elem(obj_name, from, obj_name, to);
    }

    /// Copy every element of `src` onto `dst` and set its size (`r = q`).
    /// §10.7/§11.8.2: the context width an assignment gives its right-hand
    /// side. A REAL target has no bit width to lend: an integral RHS is
    /// evaluated self-determined and then converted (`r = u - 2` with
    /// `int unsigned u = 1` is 4294967295.0).
    /// The right-hand side of an assignment to `lvalue` (width `w`); see
    /// `assign_rhs_ctx`.
    #[inline]
    pub(super) fn eval_assign_rhs(
        &mut self,
        lvalue: &Expression,
        rvalue: &Expression,
        w: u32,
    ) -> Value {
        let w = if Self::rhs_takes_context(rvalue) {
            self.assign_rhs_ctx(lvalue, w)
        } else {
            w
        };
        self.eval_expr_ctx(rvalue, w)
    }

    /// The right-hand side of an assignment the interpreter evaluates apart
    /// from the blocking path: a nonblocking one (§10.4.2) and an
    /// intra-assignment event control (§9.4.5). Its operators still take the
    /// target's width as context (§11.6.1), so `s <= a16 + b16` into an
    /// 18-bit `s` keeps the carry. Evaluated self-determined, the sum lost
    /// it whenever the statement ran here instead of as bytecode, for
    /// example in a `for` loop bounded by `$clog2(...)`.
    ///
    /// §11.4.1 has a side-effecting lvalue index (`v[i++ +: 18] = @(e) ...`)
    /// evaluated once, when the assignment captures its target, so the
    /// width lookup must not evaluate it (see `effect_free_target_width`).
    pub(super) fn eval_target_rhs(&mut self, lvalue: &Expression, rvalue: &Expression) -> Value {
        if !Self::rhs_takes_context(rvalue) {
            return self.eval_expr(rvalue);
        }
        match self.effect_free_target_width(lvalue) {
            0 => self.eval_expr(rvalue),
            w => self.eval_assign_rhs(lvalue, rvalue, w),
        }
    }

    /// The width of an assignment target, or 0 when finding it would
    /// evaluate a side-effecting index. Only an indexed part-select of a
    /// plain base (`v[i++ +: 18]`) has a width that doesn't depend on such an
    /// index: `infer_lhs_width` reads only its constant width. An element or
    /// member target (`arr[i++].f`) is sized by reading it, and the
    /// class-aggregate lookup may evaluate indices, so those give 0 and keep
    /// the old self-determined evaluation.
    pub(super) fn effect_free_target_width(&mut self, lvalue: &Expression) -> u32 {
        if !Self::lvalue_has_incdec_index(lvalue) {
            return self.infer_lhs_width(lvalue);
        }
        match &lvalue.kind {
            ExprKind::RangeSelect {
                expr,
                right,
                kind: RangeKind::IndexedUp | RangeKind::IndexedDown,
                ..
            } if self.no_class_objects()
                && !Self::lvalue_has_incdec_index(expr)
                && Self::cond_expr_is_effect_free(right, false) =>
            {
                self.infer_lhs_width(lvalue)
            }
            _ => 0,
        }
    }

    /// Only an operator takes a width from its context (§11.6.1: unary `+`,
    /// `-` and `~` too); a plain operand converts to the same real either
    /// way, so only then does an assignment need `assign_rhs_ctx`.
    #[inline]
    pub(super) fn rhs_takes_context(e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Paren(x) => Self::rhs_takes_context(x),
            ExprKind::Binary { .. } | ExprKind::Conditional { .. } => true,
            ExprKind::Unary { op, .. } => {
                matches!(op, UnaryOp::Plus | UnaryOp::Minus | UnaryOp::BitNot)
            }
            _ => false,
        }
    }

    #[inline(never)]
    pub(super) fn assign_rhs_ctx(&mut self, lvalue: &Expression, w: u32) -> u32 {
        // A real variable is 64 bits wide; any other width is integral.
        if w == 64 && matches!(lvalue.kind, ExprKind::Ident(_)) && self.expr_is_real_static(lvalue)
        {
            0
        } else {
            w
        }
    }

    /// §11.8.2: true when `e` is REAL-typed by declaration — a real literal,
    /// a real variable, a real-valued system function, a cast to a real type,
    /// or an arithmetic/conditional operator over such an operand. Mirrors
    /// the bytecode compiler's `expr_is_real_static`; a shape it cannot type
    /// answers false and keeps the integral rules.
    pub(super) fn expr_is_real_static(&mut self, e: &Expression) -> bool {
        // Every real-typed expression is 64 bits wide, so a node whose STATIC
        // width is already known to be anything else is integral — the
        // common case, answered without a walk or a name lookup.
        if e.cached_width.get().is_some_and(|w| w != 64) {
            return false;
        }
        match &e.kind {
            ExprKind::Number(n) => matches!(
                n,
                crate::ast::expr::NumberLiteral::Real(_) | crate::ast::expr::NumberLiteral::Time(_)
            ),
            ExprKind::Ident(h) => {
                if h.path.len() == 1
                    && h.root.is_none()
                    && let Some(v) = self
                        .local_stack
                        .last()
                        .and_then(|m| m.get(&h.path[0].name.name))
                {
                    return v.is_real;
                }
                let n = self.resolve_hier_name(h);
                self.lookup_signal_real(&n)
            }
            ExprKind::Paren(x) => self.expr_is_real_static(x),
            ExprKind::Unary {
                op: UnaryOp::Plus | UnaryOp::Minus,
                operand,
            } => self.expr_is_real_static(operand),
            ExprKind::Binary {
                op: BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Power,
                left,
                right,
            } => self.expr_is_real_static(left) || self.expr_is_real_static(right),
            ExprKind::Conditional {
                then_expr,
                else_expr,
                ..
            } => self.expr_is_real_static(then_expr) || self.expr_is_real_static(else_expr),
            ExprKind::SystemCall { name, args } => match name.as_str() {
                "$__xz_type_cast" => matches!(
                    args.first().map(|a| &a.kind),
                    Some(ExprKind::TypeLiteral(dt))
                        if crate::compiler::elaborate::is_type_real(dt)
                ),
                n => super::bytecode::system_function_is_real(n),
            },
            _ => false,
        }
    }

    /// §13.4.1: `e` calls a free function declared to return a real type.
    pub(super) fn call_returns_real(&self, e: &Expression) -> bool {
        let ExprKind::Call { func, .. } = &e.kind else {
            return false;
        };
        let ExprKind::Ident(h) = &func.kind else {
            return false;
        };
        h.path.len() == 1
            && self
                .module
                .functions
                .get(&h.path[0].name.name)
                .is_some_and(|fd| {
                    super::elaborate::is_type_real_resolved(
                        &fd.return_type,
                        &self.module.typedef_types,
                    )
                })
    }

    /// A constant expression by its shape — literals, parameters, and
    /// operators, casts and real conversions over them.
    pub(super) fn expr_is_constant(&mut self, e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Number(_) => true,
            ExprKind::Ident(h) => {
                if h.path.len() == 1
                    && h.root.is_none()
                    && self
                        .local_stack
                        .last()
                        .is_some_and(|m| m.contains_key(&h.path[0].name.name))
                {
                    return false;
                }
                let n = self.resolve_hier_name(h);
                self.module.parameters.contains_key(&*n)
            }
            ExprKind::Paren(x) | ExprKind::Unary { operand: x, .. } => self.expr_is_constant(x),
            ExprKind::Binary { left, right, .. } => {
                self.expr_is_constant(left) && self.expr_is_constant(right)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.expr_is_constant(condition)
                    && self.expr_is_constant(then_expr)
                    && self.expr_is_constant(else_expr)
            }
            ExprKind::SystemCall { name, args } => match name.as_str() {
                "$__xz_type_cast" => args.get(1).is_some_and(|a| self.expr_is_constant(a)),
                n if n != "$realtime" && super::bytecode::system_function_is_real(n) => {
                    args.iter().all(|a| self.expr_is_constant(a))
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// §11.8.2 operators with a REAL operand (see the bytecode compiler's
    /// `compile_real_operator`): an arithmetic operator converts each integral
    /// operand from its SELF-determined value; a relational or equality
    /// operator propagates the real type one operator into a CONSTANT integral
    /// side. None when neither operand is real.
    #[inline(never)]
    pub(super) fn eval_real_operator(
        &mut self,
        op: BinaryOp,
        left: &Expression,
        right: &Expression,
    ) -> Option<Value> {
        if !matches!(
            op,
            BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::Power
                | BinaryOp::Eq
                | BinaryOp::Neq
                | BinaryOp::Lt
                | BinaryOp::Leq
                | BinaryOp::Gt
                | BinaryOp::Geq
        ) {
            return None;
        }
        let cmp = matches!(
            op,
            BinaryOp::Eq
                | BinaryOp::Neq
                | BinaryOp::Lt
                | BinaryOp::Leq
                | BinaryOp::Gt
                | BinaryOp::Geq
        );
        let lr = self.expr_is_real_static(left);
        let rr = self.expr_is_real_static(right);
        if !lr && !rr {
            return None;
        }
        // Only a CONSTANT integral side takes the real type in a comparison,
        // as in the reference simulator (see the bytecode compiler).
        let l = if cmp && !lr && self.expr_is_constant(left) {
            self.eval_in_real_context(left)
        } else {
            self.real_operand(left)
        };
        let r = if cmp && !rr && self.expr_is_constant(right) {
            self.eval_in_real_context(right)
        } else {
            self.real_operand(right)
        };
        Some(Self::real_binop(op, &l, &r))
    }

    /// `+ - *` with a REAL operand (§11.8.2), once the width probes have
    /// sized — and for some shapes evaluated — the operands: each integral
    /// operand is converted from its self-determined value. None (and the
    /// probed values left in place) when neither operand is real.
    #[inline(never)]
    pub(super) fn real_arith_probed(
        &mut self,
        op: BinaryOp,
        left: &Expression,
        right: &Expression,
        lv: &mut Option<Value>,
        rv: &mut Option<Value>,
    ) -> Option<Value> {
        let lr = match lv {
            Some(v) => v.is_real,
            None => self.expr_is_real_static(left),
        };
        let rr = match rv {
            Some(v) => v.is_real,
            None => self.expr_is_real_static(right),
        };
        if !lr && !rr {
            return None;
        }
        let l = match lv.take() {
            Some(v) => v,
            None => self.eval_expr_ctx(left, 0),
        };
        let r = match rv.take() {
            Some(v) => v,
            None => self.eval_expr_ctx(right, 0),
        };
        let real = |v: Value| {
            if v.is_real {
                v
            } else {
                Value::from_f64(v.to_f64())
            }
        };
        Some(Self::real_binop(op, &real(l), &real(r)))
    }

    /// `?:` with a REAL arm (§11.4.11/§11.8.2): the taken arm as a real — a
    /// constant integral arm in real context, a run-time one converted from
    /// its self-determined value. None when neither arm is real.
    #[inline(never)]
    pub(super) fn eval_real_conditional(
        &mut self,
        condition: &Expression,
        then_expr: &Expression,
        else_expr: &Expression,
    ) -> Option<Value> {
        if !self.expr_is_real_static(then_expr) && !self.expr_is_real_static(else_expr) {
            return None;
        }
        let c = self.eval_expr(condition);
        let taken = if !c.has_unknown() && c.is_true() {
            then_expr
        } else {
            else_expr
        };
        Some(if self.expr_is_constant(taken) {
            self.eval_in_real_context(taken)
        } else {
            self.real_operand(taken)
        })
    }

    pub(super) fn real_binop(op: BinaryOp, l: &Value, r: &Value) -> Value {
        match op {
            BinaryOp::Add => l.add(r),
            BinaryOp::Sub => l.sub(r),
            BinaryOp::Mul => l.mul(r),
            BinaryOp::Div => l.div(r),
            BinaryOp::Power => l.power(r),
            BinaryOp::Eq => l.is_equal(r),
            BinaryOp::Neq => l.is_not_equal(r),
            BinaryOp::Lt => l.less_than(r),
            BinaryOp::Leq => l.less_equal(r),
            BinaryOp::Gt => l.greater_than(r),
            _ => l.greater_equal(r),
        }
    }

    /// An operand of a real operator, SELF-determined and converted to real
    /// (§6.12.2: x and z bits read as 0).
    pub(super) fn real_operand(&mut self, e: &Expression) -> Value {
        let v = self.eval_expr_ctx(e, 0);
        if v.is_real {
            v
        } else {
            Value::from_f64(v.to_f64())
        }
    }

    /// §11.8.2: an integral operand that takes REAL type from its context (the
    /// other side of a relational operator, the other arm of `?:`): the real
    /// type reaches its top operator, whose own operands are self-determined
    /// and converted — `(u - 2) > 1.0` compares -1.0, `a / b < 3.2` divides
    /// as reals. Anything else is self-determined and converted whole.
    pub(super) fn eval_in_real_context(&mut self, e: &Expression) -> Value {
        match &e.kind {
            ExprKind::Paren(x) => self.eval_in_real_context(x),
            ExprKind::Binary {
                op:
                    op @ (BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Power),
                left,
                right,
            } => {
                let l = self.real_operand(left);
                let r = self.real_operand(right);
                Self::real_binop(*op, &l, &r)
            }
            ExprKind::Unary {
                op: UnaryOp::Minus,
                operand,
            } => self.real_operand(operand).negate(),
            ExprKind::Unary {
                op: UnaryOp::Plus,
                operand,
            } => self.real_operand(operand),
            _ => self.real_operand(e),
        }
    }

    /// §6.16.6 compare/icompare, matching the reference simulator: the
    /// byte difference at the first mismatch within the common length,
    /// or ±1 by length when one string is a prefix of the other.
    /// `compare` takes the difference of the characters as SIGNED bytes
    /// (`"\x80".compare("\x7f")` is -255), `icompare` as unsigned bytes
    /// after ASCII case folding. Both operands are string text forms (one
    /// Latin-1 char per byte), so they are walked by char, never by their
    /// UTF-8 encoding.
    pub(super) fn sv_strcmp(a: &str, b: &str, icase: bool) -> i64 {
        Self::sv_strcmp_by(a, b, |b| {
            if icase {
                b.to_ascii_lowercase() as i64
            } else {
                b as i8 as i64
            }
        })
    }

    /// The lexicographic ORDER of two strings for the `==`/`<` operator
    /// family (§6.16 Table 6-9): unsigned byte differences.
    pub(super) fn sv_str_order(a: &str, b: &str) -> i64 {
        Self::sv_strcmp_by(a, b, |b| b as i64)
    }

    pub(super) fn sv_strcmp_by(a: &str, b: &str, byte: impl Fn(u8) -> i64) -> i64 {
        let byte = |c: char| byte(c as u32 as u8);
        for (ca, cb) in a.chars().zip(b.chars()) {
            let (ca, cb) = (byte(ca), byte(cb));
            if ca != cb {
                return ca - cb;
            }
        }
        match a.chars().count().cmp(&b.chars().count()) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    }

    /// §6.16: a string character is a `byte` — SIGNED 8-bit. `s[i]` and
    /// `s.getc(i)` of `"\xff"` read -1, and `s[0] + s[1]` is an 8-bit signed
    /// sum.
    #[inline]
    pub(super) fn sv_byte(b: u8) -> Value {
        let mut v = Value::from_u64(b as u64, 8);
        v.is_signed = true;
        v
    }

    /// §6.16: a string `Value` built from raw BYTES. The text form of a
    /// string (`to_sv_string`) holds one Latin-1 char per byte, so bytes
    /// round-trip through chars, never through UTF-8: `from_utf8_lossy` made
    /// every byte >= 0x80 a U+FFFD, and `String::into_bytes` of a text form
    /// yields two bytes for each of them.
    pub(super) fn sv_string_from_bytes(b: &[u8]) -> Value {
        let s: String = b.iter().map(|&c| char::from(c)).collect();
        Value::from_string(&s)
    }

    /// §6.16: integral-to-string conversion removes all NUL characters.
    pub(super) fn sv_string_cast(value: &Value) -> Value {
        let text: String = value
            .sv_string_bytes()
            .into_iter()
            .filter(|&byte| byte != 0)
            .map(char::from)
            .collect();
        Value::from_string(&text)
    }

    /// §6.16 `toupper`/`tolower`: ASCII letters only; every other byte,
    /// including Latin-1 letters, is unchanged (and keeps its one-byte width).
    pub(super) fn sv_ascii_case(s: &str, upper: bool) -> String {
        if upper {
            s.to_ascii_uppercase()
        } else {
            s.to_ascii_lowercase()
        }
    }

    /// §6.24.1: pack a call-returned dynamic array/queue (recorded in
    /// `pending_ret_collection`) into a `target_w`-bit vector — element 0
    /// most significant, each element sliced to target_w / n bits (the cast
    /// is only legal when total bits match, so the division is exact).
    pub(super) fn pack_ret_collection(&mut self, src: &str, target_w: u32) -> Option<Value> {
        let n = self.get_queue_size(src);
        if n == 0 || target_w == 0 || (target_w as u64) % n != 0 {
            return None;
        }
        let ew = (target_w as u64 / n) as u32;
        let elems: Vec<Value> = (0..n)
            .map(|i| {
                self.get_signal_value_by_name(&format!("{}[{}]", src, i))
                    .map(|v| v.resize(ew))
                    .unwrap_or_else(|| Value::zero(ew))
            })
            .collect();
        Some(Value::concat(&elems))
    }

    /// §7.5.1/§7.6: copy the dynamic array or queue at `src` into `dst`,
    /// a row that is itself a dynamic array or queue (`int d[][]`,
    /// `int q[][$]`, deeper too) with its own size and elements. `dst`'s
    /// rows are registered like the source's and start empty.
    pub(super) fn copy_dyn_tree(&mut self, dst: &str, src: &str) {
        let n = self.get_queue_size(src);
        for i in 0..n {
            let (srow, drow) = (format!("{src}[{i}]"), format!("{dst}[{i}]"));
            if self.module.dynamic_arrays.contains(&srow) {
                self.register_dyn_row_like(&drow, &srow);
                self.copy_dyn_tree(&drow, &srow);
            } else {
                self.queue_copy_elem(src, i, dst, i);
            }
        }
        self.set_queue_size(dst, n);
    }

    /// Register `row` as an empty dynamic row shaped like `like`.
    pub(super) fn register_dyn_row_like(&mut self, row: &str, like: &str) {
        self.clear_dyn_row(row);
        self.module.dynamic_arrays.insert(row.to_string());
        let shape = self.module.arrays.get(like).copied().unwrap_or((0, 63, 32));
        self.module.arrays.insert(row.to_string(), shape);
        if self.module.queue_vars.contains(like) {
            self.module.queue_vars.insert(row.to_string());
        }
        self.set_queue_size(row, 0);
    }

    pub(super) fn copy_whole_queue(&mut self, dst: &str, src: &str) {
        let n = self.get_queue_size(src);
        for i in 0..n {
            self.queue_copy_elem(src, i, dst, i);
        }
        self.set_queue_size(dst, n);
    }

    /// Populate a per-instance queue / dynamic-array member from an inline
    /// class-property initializer (`int q[$] = '{1,2,3}`). The initializer is
    /// an ordered `AssignmentPattern`; each item is evaluated in the
    /// instance context and written to `<queue>[i]`, then the queue size is
    /// set to the number of items. Mirrors the AssignmentPattern assignment
    /// path in `assign_value`.
    pub(super) fn populate_queue_from_init(&mut self, scoped_q: &str, init: &Expression) {
        // §7.10.1: both the pattern form `'{1,2,3}` and the queue
        // concatenation form `{1,2,3}` initialize element-wise.
        let elems: Vec<&Expression> = match &init.kind {
            ExprKind::AssignmentPattern(items) => items.iter().map(|it| it.expr()).collect(),
            ExprKind::Concatenation(parts) => parts.iter().collect(),
            _ => return,
        };
        let (_lo, _hi, w) = self
            .module
            .arrays
            .get(scoped_q)
            .copied()
            .unwrap_or((0, 63, 32));
        for (i, item) in elems.iter().enumerate() {
            let mut v = self.eval_expr(item);
            // §5.7.1: an unsized fill element takes the element width.
            if v.is_fill && !v.is_real {
                v = v.resize(w.max(1));
            }
            self.set_signal_value_by_name(&format!("{}[{}]", scoped_q, i), v);
            self.widths.insert(format!("{}[{}]", scoped_q, i), w);
        }
        self.set_queue_size(scoped_q, elems.len() as u64);
    }

    /// Relative leaf paths of an unpacked struct (`a`, `inner.b`, `arr[0]`).
    pub(super) fn struct_leaf_suffixes(
        &self,
        su: &crate::ast::types::StructUnionType,
    ) -> Vec<String> {
        let mut out = Vec::new();
        for m in &su.members {
            for md in &m.declarators {
                let base = md.name.name.clone();
                if md.dimensions.is_empty() {
                    if let DataType::Struct(inner) = self.resolve_dt(&m.data_type) {
                        if Self::spreads_member_wise(&inner) {
                            for sfx in self.struct_leaf_suffixes(&inner) {
                                out.push(format!("{}.{}", base, sfx));
                            }
                            continue;
                        }
                    }
                    out.push(base);
                } else if let Some(tups) = self.member_elem_indices(&md.dimensions) {
                    let inner = self.unpacked_struct_of(&m.data_type);
                    for t in tups {
                        let elem = format!("{}{}", base, Self::index_suffix(&t));
                        match &inner {
                            Some(inner) => {
                                for sfx in self.struct_leaf_suffixes(inner) {
                                    out.push(format!("{}.{}", elem, sfx));
                                }
                            }
                            None => out.push(elem),
                        }
                    }
                }
            }
        }
        out
    }

    /// Every DYNAMIC member below the unpacked struct `su` — its own queue /
    /// dynamic / associative members, and those of nested structs and of
    /// fixed member arrays of structs — as `(path relative to the struct,
    /// kind, element type, string-keyed)`. The counterpart of
    /// `struct_leaf_suffixes`, which skips them.
    pub(super) fn struct_coll_members(
        &self,
        su: &crate::ast::types::StructUnionType,
    ) -> Vec<(String, StructCollKind, DataType, bool)> {
        let mut out = Vec::new();
        for m in &su.members {
            for md in &m.declarators {
                let base = md.name.name.as_str();
                let inner = self.unpacked_struct_of(&m.data_type);
                if md.dimensions.is_empty() {
                    if let Some(inner) = inner {
                        for (sfx, k, dt, sk) in self.struct_coll_members(&inner) {
                            out.push((format!("{}.{}", base, sfx), k, dt, sk));
                        }
                    }
                } else if md.dimensions.len() == 1
                    && let Some(kind) = Self::coll_kind_of(&md.dimensions)
                {
                    let string_key = self.string_keyed_dim(&md.dimensions[0]);
                    out.push((base.to_string(), kind, m.data_type.clone(), string_key));
                } else if let (Some(inner), Some(tups)) =
                    (inner, self.member_elem_indices(&md.dimensions))
                {
                    let subs = self.struct_coll_members(&inner);
                    for t in tups {
                        let elem = format!("{}{}", base, Self::index_suffix(&t));
                        for (sfx, k, dt, sk) in &subs {
                            out.push((format!("{}.{}", elem, sfx), *k, dt.clone(), *sk));
                        }
                    }
                }
            }
        }
        out
    }

    /// Whether `dim` is an associative dimension keyed by `string` (§7.8.1).
    pub(super) fn string_keyed_dim(&self, dim: &crate::ast::types::UnpackedDimension) -> bool {
        matches!(
            dim,
            crate::ast::types::UnpackedDimension::Associative { data_type: Some(k), .. }
                if matches!(
                    self.resolve_dt_ref(k),
                    DataType::Simple {
                        kind: crate::ast::types::SimpleType::String,
                        ..
                    }
                )
        )
    }

    /// The paths of [`struct_coll_members`].
    pub(super) fn struct_coll_suffixes(
        &self,
        su: &crate::ast::types::StructUnionType,
    ) -> Vec<String> {
        self.struct_coll_members(su)
            .into_iter()
            .map(|(sfx, ..)| sfx)
            .collect()
    }

    /// Register the dynamic member `<base>.<sfx>` of a struct whose storage
    /// root has no declared type of its own (a formal, a return snapshot).
    pub(super) fn register_struct_member_coll_as(
        &mut self,
        name: &str,
        kind: StructCollKind,
        dt: &DataType,
        string_key: bool,
    ) {
        let w = resolve_type_width(
            dt,
            Some(&self.module.parameters),
            Some(&self.module.typedefs),
        )
        .max(1);
        self.register_coll_name(name, kind, w, string_key);
    }

    /// Empty the collection `name` (§7.5.3/§7.9.3/§7.10.2.7 `delete()`).
    pub(super) fn clear_coll(&mut self, name: &str, kind: StructCollKind) {
        match kind {
            StructCollKind::Assoc => {
                self.eval_builtin_method(name, "delete", &[]);
            }
            _ => self.set_queue_size(name, 0),
        }
    }

    /// Copy the whole collection `src` onto `dst` (same kind).
    pub(super) fn copy_coll(&mut self, dst: &str, src: &str, kind: StructCollKind) {
        if dst == src {
            return;
        }
        match kind {
            StructCollKind::Assoc => self.copy_assoc_entries(dst, src),
            _ => self.copy_whole_queue(dst, src),
        }
    }

    /// §6.21/§7.2.1: a fresh automatic struct variable `base` of type `su`
    /// starts with EMPTY dynamic members of its own. They are named by the
    /// variable's path (`v.q`), so the enclosing frame's same-named storage
    /// is snapshotted for restore when this frame pops, and the members are
    /// registered and emptied.
    pub(super) fn fresh_struct_colls(
        &mut self,
        base: &str,
        su: &crate::ast::types::StructUnionType,
    ) {
        for (sfx, kind, dt, sk) in self.struct_coll_members(su) {
            let n = format!("{}.{}", base, sfx);
            self.snapshot_queue_local(&n);
            self.register_struct_member_coll_as(&n, kind, &dt, sk);
            self.clear_coll(&n, kind);
        }
    }

    /// The storage root and type of the unpacked struct `e` names: a
    /// subroutine local, a formal, a variable, or an element / member of one.
    pub(super) fn struct_storage_of(
        &mut self,
        e: &Expression,
    ) -> Option<(String, crate::ast::types::StructUnionType)> {
        if let Some(r) = self.frame_struct_local(e) {
            return Some(r);
        }
        let n = self.flat_member_name(e)?;
        let dt = self.p_elem_type(&n)?;
        let su = self.unpacked_struct_of(&dt)?;
        Some((n, su))
    }

    /// §13.4.1: `return s;` of a struct with dynamic members — the packed
    /// return value cannot carry them, and a subroutine local's storage is
    /// torn down with its frame, so stage each member under a reserved name
    /// for the caller's assignment (`copy_ret_struct_colls`).
    #[cold]
    #[inline(never)]
    pub(super) fn stage_ret_struct_colls(&mut self, e: &Expression) {
        self.pending_ret_struct = None;
        let Some((name, su)) = self.struct_storage_of(e) else {
            return;
        };
        let colls = self.struct_coll_members(&su);
        if colls.is_empty() {
            return;
        }
        const RET_STRUCT: &str = "__xz_ret_struct__";
        for (sfx, kind, dt, sk) in colls {
            let src = format!("{}.{}", name, sfx);
            let dst = format!("{}.{}", RET_STRUCT, sfx);
            self.register_struct_member_coll_as(&dst, kind, &dt, sk);
            self.clear_coll(&dst, kind);
            if self.module.dynamic_arrays.contains(&src)
                || self.module.associative_arrays.contains_key(&src)
                || self.ensure_struct_member_coll(&src)
            {
                self.copy_coll(&dst, &src, kind);
            }
        }
        self.pending_ret_struct = Some(RET_STRUCT.to_string());
    }

    /// The caller half of `stage_ret_struct_colls`: `dst = f(...)` gets the
    /// returned struct's dynamic members.
    pub(super) fn copy_ret_struct_colls(
        &mut self,
        dst: &str,
        su: &crate::ast::types::StructUnionType,
    ) {
        let Some(src) = self.pending_ret_struct.take() else {
            return;
        };
        for (sfx, kind, dt, sk) in self.struct_coll_members(su) {
            let d = format!("{}.{}", dst, sfx);
            if !self.ensure_struct_member_coll(&d) {
                self.register_struct_member_coll_as(&d, kind, &dt, sk);
            }
            self.clear_coll(&d, kind);
            self.copy_coll(&d, &format!("{}.{}", src, sfx), kind);
        }
    }

    /// Whether the collections `a` and `b` hold the same elements (§7.2.1):
    /// the same size and equal elements for a queue / dynamic array, the
    /// same keys with equal values for an associative array. `None` when an
    /// element carries x/z (the comparison is then unknown).
    pub(super) fn coll_contents_eq(&mut self, a: &str, b: &str) -> Option<bool> {
        let a_ok = self.ensure_struct_member_coll(a);
        let b_ok = self.ensure_struct_member_coll(b);
        if !(a_ok && b_ok) {
            return Some(true);
        }
        let pairs: Vec<(Option<Value>, Option<Value>)> = if self.is_associative_array(a) {
            let mut ka = self.assoc_key_strs(a);
            let mut kb = self.assoc_key_strs(b);
            ka.sort();
            kb.sort();
            if ka != kb {
                return Some(false);
            }
            ka.iter()
                .map(|k| {
                    (
                        self.get_signal_value_by_name(&format!("{}[{}]", a, k)),
                        self.get_signal_value_by_name(&format!("{}[{}]", b, k)),
                    )
                })
                .collect()
        } else {
            let (na, nb) = (self.get_queue_size(a), self.get_queue_size(b));
            if na != nb {
                return Some(false);
            }
            (0..na)
                .map(|i| {
                    (
                        self.get_signal_value_by_name(&format!("{}[{}]", a, i)),
                        self.get_signal_value_by_name(&format!("{}[{}]", b, i)),
                    )
                })
                .collect()
        };
        let mut equal = true;
        for (x, y) in pairs {
            match (x, y) {
                (Some(x), Some(y)) => {
                    if x.has_unknown() || y.has_unknown() {
                        return None;
                    }
                    if x != y {
                        equal = false;
                    }
                }
                (None, None) => {}
                _ => equal = false,
            }
        }
        Some(equal)
    }

    /// Rearrange a queue so that position `i` holds what used to be at
    /// `order[i]`. Snapshots first: an in-place shuffle would overwrite an
    /// element it still has to read. Struct elements move leaf by leaf.
    pub(super) fn queue_permute(&mut self, obj_name: &str, order: &[usize]) {
        if let Some(su) = self.queue_elem_struct(obj_name) {
            let sfxs = self.struct_leaf_suffixes(&su);
            let mut snap: Vec<Vec<Option<Value>>> = Vec::with_capacity(order.len());
            for &old in order {
                let mut row = Vec::with_capacity(sfxs.len());
                for sfx in &sfxs {
                    row.push(
                        self.get_signal_value_by_name(&format!("{}[{}].{}", obj_name, old, sfx)),
                    );
                }
                snap.push(row);
            }
            for (newi, row) in snap.into_iter().enumerate() {
                for (sfx, v) in sfxs.iter().zip(row) {
                    if let Some(v) = v {
                        self.write_leaf_by_name(&format!("{}[{}].{}", obj_name, newi, sfx), v);
                    }
                }
            }
            return;
        }
        let mut snap: Vec<Option<Value>> = Vec::with_capacity(order.len());
        for &old in order {
            snap.push(self.get_signal_value_by_name(&format!("{}[{}]", obj_name, old)));
        }
        for (newi, v) in snap.into_iter().enumerate() {
            if let Some(v) = v {
                self.set_signal_value_by_name(&format!("{}[{}]", obj_name, newi), v);
            }
        }
    }

    /// A method call on a queue / dynamic array / fixed collection: the
    /// receiver's flat name and the method name. Purely syntactic + table
    /// lookup — never evaluates the receiver.
    pub(super) fn collection_method_of(&mut self, func: &Expression) -> Option<(String, String)> {
        let (obj, mname) = match &func.kind {
            ExprKind::MemberAccess { expr, member } => {
                (self.flat_member_name(expr)?, member.name.clone())
            }
            ExprKind::Ident(h) if h.path.len() >= 2 => {
                let mname = h.path.last()?.name.name.clone();
                let obj = h.path[..h.path.len() - 1]
                    .iter()
                    .map(|s| s.name.name.as_str())
                    .collect::<Vec<_>>()
                    .join(".");
                (obj, mname)
            }
            _ => return None,
        };
        if self.module.dynamic_arrays.contains(&obj) || self.module.arrays.contains_key(&obj) {
            Some((obj, mname))
        } else {
            None
        }
    }

    /// §7.10.2.5/§7.10.2.6: the value `pop_front`/`pop_back` returns, element
    /// `idx` of queue `obj`. An UNPACKED-STRUCT element has no packed cell of
    /// its own, only its members' leaves, so it travels packed like a struct
    /// function result (`pack_unpacked_struct`): `r = q.pop_front()` on a
    /// class-property queue, `return q.pop_front()` or a declaration
    /// initializer then spread it back over the target's members.
    pub(super) fn queue_elem_value(&mut self, obj: &str, idx: i64) -> Value {
        let elem = Self::name_with_index(obj, idx);
        if let Some(su) = self.queue_elem_struct(obj) {
            if let Some(v) = self.pack_unpacked_struct(&elem, &su) {
                return v;
            }
        }
        self.get_signal_value_by_name(&elem)
            .unwrap_or_else(|| Value::zero(32))
    }

    /// `q.pop_front()` / `q.pop_back()` — the queue name and which end.
    pub(super) fn queue_pop_call(&mut self, e: &Expression) -> Option<(String, String)> {
        let ExprKind::Call { func, .. } = &e.kind else {
            return None;
        };
        // `q.pop_front()` parses either as MemberAccess or — for a plain
        // variable receiver — as a 2-segment hierarchical identifier.
        let (obj, mname) = match &func.kind {
            ExprKind::MemberAccess { expr, member } => {
                (self.flat_member_name(expr)?, member.name.clone())
            }
            ExprKind::Ident(h) if h.path.len() >= 2 => {
                let mname = h.path.last()?.name.name.clone();
                let obj = h.path[..h.path.len() - 1]
                    .iter()
                    .map(|s| s.name.name.as_str())
                    .collect::<Vec<_>>()
                    .join(".");
                (obj, mname)
            }
            _ => return None,
        };
        if mname != "pop_front" && mname != "pop_back" {
            return None;
        }
        if self.module.dynamic_arrays.contains(&obj) {
            Some((obj, mname))
        } else {
            None
        }
    }

    /// Store `arg` into queue element `elem`. An UNPACKED-STRUCT element keeps
    /// each member in its own signal, so writing one packed value (what
    /// `push_back` used to do) loses every member.
    pub(super) fn queue_store_elem(&mut self, obj_name: &str, elem: &str, arg: &Expression) {
        // §15.5.5: pushing an event variable stores its synchronization-object
        // KEY (a string), so a handle later pulled out of the container
        // (`e = q[0]`, `q[0].triggered`) still names the same object.
        if let Some(k) = self.event_ref_key(arg) {
            let canon = self.resolve_event_key(&k);
            self.set_signal_value_by_name(elem, Value::from_string(&canon));
            return;
        }
        // §7.4.5: a queue of FIXED arrays (`int q[$][4]`) stores each row's
        // elements at `q[i][j]` — evaluating the row to one packed value
        // would land in a phantom scalar `q[i]` that no read ever visits.
        if let Some(&(_, (jlo, jhi), _)) = self.module.arrays_2d.get(obj_name) {
            // §10.9.1 / §7.6: the row pairs by position from its declared
            // left bound (`int q[$][3:1]` takes `'{1,2,3}` as q[i][3] = 1).
            let order = |me: &Self, name: &str, lo: i64, hi: i64, nth: usize| -> Vec<i64> {
                match me
                    .unpacked_decl_dims_of(name)
                    .and_then(|d| d.get(nth).copied())
                {
                    Some((l, r)) if l > r => (r..=l).rev().collect(),
                    _ => (lo..=hi).collect(),
                }
            };
            let dst = order(self, obj_name, jlo, jhi, 1);
            if let Some(src) = self.flat_member_name(arg) {
                if let Some(&(slo, shi, _)) = self.module.arrays.get(&src) {
                    let from = order(self, &src, slo, shi, 0);
                    for (j, si) in dst.iter().zip(from.iter()) {
                        let v = self
                            .get_signal_value_by_name(&format!("{}[{}]", src, si))
                            .unwrap_or_else(|| Value::zero(32));
                        self.set_signal_value_by_name(&format!("{}[{}]", elem, j), v);
                    }
                    return;
                }
            }
            if let ExprKind::AssignmentPattern(items) = &arg.kind {
                let vals: Vec<Value> = items.iter().map(|it| self.eval_expr(it.expr())).collect();
                for (k, j) in dst.iter().enumerate() {
                    let v = vals.get(k).cloned().unwrap_or_else(|| Value::zero(32));
                    self.set_signal_value_by_name(&format!("{}[{}]", elem, j), v);
                }
                return;
            }
        }
        // §7.10: pushing a QUEUE/dynamic-array VALUE (queue-of-queue) copies
        // the source's elements into the new row's compound storage —
        // evaluating the row to one scalar dropped the contents entirely
        // (`qq.push_back(tmp)` left qq[0] empty).
        if let Some(src) = self.flat_member_name(arg) {
            if self.module.dynamic_arrays.contains(&src)
                || self.signals.contains_key(&format!("{}.size", src))
            {
                let n = self.get_queue_size(&src);
                for j in 0..n {
                    let v = self
                        .get_signal_value_by_name(&format!("{}[{}]", src, j))
                        .unwrap_or_else(|| Value::zero(32));
                    self.set_signal_value_by_name(&format!("{}[{}]", elem, j), v);
                }
                self.set_queue_size(elem, n);
                return;
            }
        }
        // §7.10 / §7.8: pushing an ASSOCIATIVE-array value (a queue of
        // associative arrays, `int q[$][string]`) copies the source's entries
        // into the new element, which becomes an associative array of the
        // source's key kind. Evaluated as one value it kept no entries.
        if let Some(src) = self.assoc_value_operand(arg) {
            self.store_assoc_elem(elem, &src);
            return;
        }
        if let Some(su) = self.queue_elem_struct(obj_name) {
            if Self::spreads_member_wise(&su) {
                if let ExprKind::AssignmentPattern(items) = &arg.kind {
                    if !items.is_empty() {
                        self.assign_pattern_into_struct(elem, &su, items);
                        return;
                    }
                }
                // §7.10.2/§7.2.1: a struct CLASS PROPERTY (`sq.push_back(s)`
                // in a method, `q.push_back(o.s)`) keeps its members in the
                // object: its cells, and `<handle>#s.<m>` for a dynamic one.
                if !self.no_class_objects()
                    && Self::receiver_prop_name(arg)
                        .is_some_and(|p| self.struct_prop_name_possible(p))
                    && let Some((h, prop)) = self.class_prop_receiver(arg)
                    && self
                        .class_prop_struct(h, &prop)
                        .is_some_and(|p| Self::spreads_member_wise(&p))
                {
                    self.copy_class_struct_prop_to(elem, h, &prop, &su);
                    return;
                }
                if let Some(src) = self.flat_member_name(arg) {
                    self.copy_unpacked_struct(elem, &src, &su.clone());
                    return;
                }
                // §7.10.2.2/§13.4.1: a VALUE actual — a function's struct
                // return (`q.push_back(make(...))`), or any other
                // value-shaped source (ternary, method call) — has no leaf
                // source above. Evaluate it ONCE and scatter it across the
                // element's member leaves, as `r = make(...)` does, so the
                // element travels by value into callees (`first_c(q)`),
                // queue copies (`q2 = q`) and foreach loops with its string
                // members intact. Storing only the packed scalar left every
                // leaf-based consumer reading "".
                let v = self.queue_eval_arg(obj_name, arg);
                self.spread_into_unpacked_struct(elem, &su, &v);
                // Keep the packed view even when no leaves can be laid out.
                // The argument has already run; falling through would repeat
                // its side effects (§13.5.1).
                self.set_signal_value_by_name(elem, v);
                return;
            }
        }
        let mut val = self.queue_eval_arg(obj_name, arg);
        // §5.7.1: an unsized fill (`q.push_back('1)`) takes the element's
        // width, not one bit.
        if val.is_fill && !val.is_real {
            if let Some(&(_, _, w)) = self.module.arrays.get(obj_name) {
                if w > 0 {
                    val = val.resize(w);
                }
            }
        }
        let val = self.fit_queue_push_elem(obj_name, val);
        self.set_signal_value_by_name(elem, val);
    }

    /// §7.10.2.2/§10.7: `push_back`, `push_front` and `insert` store their
    /// argument converted to the queue's element type, as an indexed store
    /// does: an INTEGRAL element (an integer vector or atom, an enum, a
    /// packed struct or union, or a type parameter bound to one) takes the
    /// element's width, signedness and 2-state value set. Any other element
    /// (string, real, class handle, unpacked struct, chandle, virtual
    /// interface) keeps the value as evaluated.
    #[inline(never)]
    pub(super) fn fit_queue_push_elem(&self, coll: &str, val: Value) -> Value {
        let Some((w, signed, two_state)) = self.integral_queue_elem(coll) else {
            return val;
        };
        // §6.12.2: a real argument rounds to the integral element type.
        let val = if val.is_real {
            Self::real_to_int(val.to_f64(), 64)
        } else {
            val
        };
        let mut v = if w != val.width || val.is_fill {
            val.resize_for_assign(w)
        } else {
            val
        };
        if two_state && v.has_xz() {
            v = v.to_two_state();
        }
        v.is_signed = signed;
        v
    }

    /// Whether `name` is the instance store (`<handle>#m`) of a queue or
    /// dynamic-array property `m` of that object's class.
    pub(super) fn class_queue_prop(&self, name: &str) -> bool {
        let Some((h, m)) = name.split_once('#') else {
            return false;
        };
        let Some(inst) = h
            .parse::<usize>()
            .ok()
            .and_then(|h| self.heap.get(h)?.as_ref())
        else {
            return false;
        };
        let mut cur = Some(inst.class_name.as_str());
        for _ in 0..64 {
            let Some(cd) = cur.and_then(|c| self.module.classes.get(c)) else {
                return false;
            };
            if cd.properties.contains_key(m) {
                return cd.queue_properties.contains_key(m);
            }
            cur = cd.extends.as_deref();
        }
        false
    }

    /// `integral_queue_elem` for a STATIC queue property stored as `coll`:
    /// the bare member name, `Class::member` when two classes declare that
    /// name (§8.9), or `Class#spec::member` for a parameterized class, whose
    /// type-parameter element follows that specialization (§8.25).
    pub(super) fn static_integral_queue_elem(&self, coll: &str) -> Option<(u32, bool, bool)> {
        // A specialization's store (`Class#spec::member`) is created on
        // first use; any other static store is registered up front.
        let w = match self.module.arrays.get(coll) {
            Some(&(_, _, w)) => w,
            None if coll.contains('#') => 0,
            None => return None,
        };
        let (owner, member) = match coll.rsplit_once("::") {
            Some((o, m)) => (Some(o), m),
            None => (None, coll),
        };
        let (owner, sig) = match owner.and_then(|o| o.split_once('#')) {
            Some((b, s)) => (Some(b), Some(s)),
            None => (owner, None),
        };
        let declares = |cd: &super::elaborate::ElaboratedClass| {
            cd.static_collections.iter().any(|(n, ..)| n == member)
        };
        let mut decl = None;
        let mut cur = owner.or_else(|| self.class_context_stack.last()?.as_deref());
        for _ in 0..64 {
            let Some(cn) = cur else { break };
            let cd = self.module.classes.get(cn)?;
            if declares(cd) {
                decl = Some(cd.as_ref());
                break;
            }
            cur = cd.extends.as_deref();
        }
        // Reached from outside any class (`A::sq.push_back(v)`): an
        // unshadowed static collection name has one declaring class.
        let cd = match decl {
            Some(cd) => cd,
            None if owner.is_none() => self.module.classes.values().find(|cd| declares(cd))?,
            None => return None,
        };
        let dt = self.class_prop_decl_type(cd, member)?;
        if let DataType::TypeReference {
            name, dimensions, ..
        } = dt
            && !name.has_scope()
            && cd.type_param_names.contains(&name.name.name)
        {
            let i = cd.param_order.iter().position(|p| *p == name.name.name)?;
            let b = Self::split_spec_args(sig?).get(i)?.trim().to_string();
            if !dimensions.is_empty() || self.module.classes.contains_key(&b) {
                return None;
            }
            if let Some(t) = self.module.typedef_types.get(b.as_str())
                && !Self::integral_dt(self.resolve_dt_ref(t))
            {
                return None;
            }
            let w = self.concrete_type_width(&b)?;
            return Some((
                w,
                self.concrete_type_signed(&b).unwrap_or(false),
                self.concrete_type_two_state(&b),
            ));
        }
        if !Self::integral_dt(self.resolve_dt_ref(dt)) || w == 0 {
            return None;
        }
        Some((
            w,
            self.type_is_signed_concrete(dt),
            super::elaborate::is_type_two_state_resolved(dt, &self.module.typedef_types),
        ))
    }

    /// An integral type (§6.11.1): an integer vector or atom, an enum, or a
    /// packed struct or union.
    pub(super) fn integral_dt(dt: &DataType) -> bool {
        match dt {
            DataType::IntegerVector { .. } | DataType::IntegerAtom { .. } | DataType::Enum(_) => {
                true
            }
            DataType::Struct(su) => su.packed,
            _ => false,
        }
    }

    /// `(width, signed, two_state)` of the element type of the queue stored
    /// as `coll` (a `<handle>#member` class property, or a module, static or
    /// subroutine-local name) when that type is integral; `None` otherwise.
    pub(super) fn integral_queue_elem(&self, coll: &str) -> Option<(u32, bool, bool)> {
        let integral = Self::integral_dt;
        if let Some((h, member)) = coll.split_once('#')
            && let Ok(h) = h.parse::<usize>()
        {
            let member = member.split('[').next().unwrap_or(member);
            let inst = self.heap.get(h)?.as_ref()?;
            let mut cd = self.module.classes.get(inst.class_name.as_str())?.as_ref();
            for _ in 0..64 {
                if cd.properties.contains_key(member) {
                    break;
                }
                cd = self.module.classes.get(cd.extends.as_deref()?)?.as_ref();
            }
            let dt = self.class_prop_decl_type(cd, member)?;
            if let DataType::TypeReference { name, .. } = dt
                && !name.has_scope()
                && cd.type_param_names.contains(&name.name.name)
            {
                // A type parameter: integral when its binding on this
                // object is (`concrete_type_width` refuses strings, reals,
                // class handles and unsized `bit`/`logic`).
                let b = self.class_prop_type_name(h, member)?;
                let bt = self.module.typedef_types.get(b.as_str());
                if bt.is_some_and(|t| !integral(self.resolve_dt_ref(t))) {
                    return None;
                }
                self.concrete_type_width(&b)?;
                let w = self.class_prop_width_of(h, member)?;
                return Some((
                    w,
                    self.class_prop_signed_of(h, member),
                    self.concrete_type_two_state(&b),
                ));
            }
            if !integral(self.resolve_dt_ref(dt)) {
                return None;
            }
            let w = self.class_prop_width_of(h, member).filter(|w| *w > 0)?;
            return Some((
                w,
                self.class_prop_signed_of(h, member),
                self.class_prop_two_state(h, member),
            ));
        }
        let leaf = coll.rsplit('.').next().unwrap_or(coll);
        let Some(dt) = self
            .module
            .var_decl_types
            .get(coll)
            .or_else(|| self.module.var_decl_types.get(leaf))
            .or_else(|| {
                // A collection formal's or local's per-call key `@name#N`.
                let bare = coll.strip_prefix('@')?.rsplit_once('#')?.0;
                self.module.var_decl_types.get(bare)
            })
        else {
            return self.static_integral_queue_elem(coll);
        };
        if !integral(self.resolve_dt_ref(dt)) {
            return None;
        }
        let w = self
            .module
            .arrays
            .get(coll)
            .map(|t| t.2)
            .filter(|w| *w > 0)?;
        Some((
            w,
            self.type_is_signed_concrete(dt),
            super::elaborate::is_type_two_state_resolved(dt, &self.module.typedef_types),
        ))
    }

    /// Whether a struct member's declared type matches a `type:value` key of an
    /// assignment pattern (§10.9.2), after resolving typedefs on both sides.
    pub(super) fn pattern_type_matches(&self, member_dt: &DataType, key_dt: &DataType) -> bool {
        use crate::ast::types::DataType as D;
        match (self.resolve_dt(member_dt), self.resolve_dt(key_dt)) {
            (D::IntegerAtom { kind: a, .. }, D::IntegerAtom { kind: b, .. }) => a == b,
            (D::IntegerVector { kind: a, .. }, D::IntegerVector { kind: b, .. }) => a == b,
            (D::Real { kind: a, .. }, D::Real { kind: b, .. }) => a == b,
            (D::Simple { kind: a, .. }, D::Simple { kind: b, .. }) => a == b,
            (D::TypeReference { name: a, .. }, D::TypeReference { name: b, .. }) => {
                a.name.name == b.name.name
            }
            _ => false,
        }
    }

    /// Element expressions of ONE array dimension, in the order of `indices`.
    /// Resolves the §10.9.1 forms: positional items, `N{expr}` replication,
    /// `index:expr` keys, and `default:expr` for whatever is left over.
    pub(super) fn pattern_elems<'a>(
        &mut self,
        items: &'a [AssignmentPatternItem],
        indices: &[i64],
    ) -> Vec<Option<&'a Expression>> {
        let n = indices.len();
        let mut out: Vec<Option<&Expression>> = vec![None; n];
        let mut default: Option<&Expression> = None;
        let mut pos = 0usize;
        for item in items {
            match item {
                AssignmentPatternItem::Default(e) => default = Some(e),
                AssignmentPatternItem::Keyed(k, v) => {
                    let ki = self.eval_expr(k).to_i64().unwrap_or(0);
                    if let Some(p) = indices.iter().position(|&i| i == ki) {
                        out[p] = Some(v);
                    }
                }
                AssignmentPatternItem::Ordered(e) => {
                    if let ExprKind::Replication { count, exprs } = &e.kind {
                        let c = self.eval_expr(count).to_u64().unwrap_or(0) as usize;
                        for _ in 0..c {
                            for sub in exprs {
                                if pos < n {
                                    out[pos] = Some(sub);
                                    pos += 1;
                                }
                            }
                        }
                    } else if pos < n {
                        out[pos] = Some(e);
                        pos += 1;
                    }
                }
                _ => {}
            }
        }
        for slot in out.iter_mut() {
            if slot.is_none() {
                *slot = default;
            }
        }
        out
    }

    /// Assign `e` to every leaf of the sub-array `base` spanning `dims`
    /// (`m = '{default:0}` on a multi-dimensional array).
    pub(super) fn fill_dims(
        &mut self,
        base: &str,
        dims: &[(i64, i64)],
        dt: &DataType,
        e: &Expression,
    ) {
        if dims.is_empty() {
            self.assign_pattern_or_leaf(base, dt, e);
            return;
        }
        let (lo, hi) = dims[0];
        for i in lo..=hi {
            let target = format!("{}[{}]", base, i);
            self.fill_dims(&target, &dims[1..], dt, e);
        }
    }

    /// Spread an assignment pattern across a FIXED unpacked array of any rank
    /// (§10.9.1). A nested pattern descends one dimension; anything else fills
    /// the whole sub-array.
    pub(super) fn assign_pattern_array(
        &mut self,
        base: &str,
        dims: &[(i64, i64)],
        dt: &DataType,
        items: &[AssignmentPatternItem],
        descending: &[bool],
    ) {
        let (lo, hi) = dims[0];
        // §10.10: positional items fill the LEFT bound first, so a
        // descending dimension (`[1:0]`) takes its first item at index 1.
        let indices: Vec<i64> = if descending.first().copied().unwrap_or(false) {
            (lo..=hi).rev().collect()
        } else {
            (lo..=hi).collect()
        };
        let inner_desc = descending.get(1..).unwrap_or(&[]);
        let elems = self.pattern_elems(items, &indices);
        for (k, e) in elems.into_iter().enumerate() {
            let Some(e) = e else { continue };
            let target = format!("{}[{}]", base, indices[k]);
            if dims.len() > 1 {
                match &e.kind {
                    ExprKind::AssignmentPattern(sub) if !sub.is_empty() => {
                        self.assign_pattern_array(&target, &dims[1..], dt, sub, inner_desc);
                    }
                    _ => self.fill_dims(&target, &dims[1..], dt, e),
                }
            } else {
                self.assign_pattern_or_leaf(&target, dt, e);
            }
        }
    }

    /// Positional element expressions after expanding `N{expr}` replication —
    /// what a queue / dynamic array pattern (`q = '{3{5}}`) yields.
    pub(super) fn pattern_ordered<'a>(
        &mut self,
        items: &'a [AssignmentPatternItem],
    ) -> Vec<&'a Expression> {
        let mut out: Vec<&Expression> = Vec::new();
        for item in items {
            if let AssignmentPatternItem::Ordered(e) = item {
                if let ExprKind::Replication { count, exprs } = &e.kind {
                    let c = self.eval_expr(count).to_u64().unwrap_or(0) as usize;
                    for _ in 0..c {
                        out.extend(exprs.iter());
                    }
                } else {
                    out.push(e);
                }
            }
        }
        out
    }

    /// IEEE 1800-2017 §10.9.2 / §10.10: an assignment pattern assigned to an
    /// aggregate whose leaves each live in their OWN signal — an unpacked
    /// struct, an unpacked array of unpacked structs, or an associative array —
    /// must be spread member-/element-/key-wise. Collapsing it into one packed
    /// value (the scalar path) writes a container signal nobody ever reads, so
    /// every member stays X and an associative array stays empty.
    ///
    /// Returns false when `base` really IS a single signal (a scalar, a packed
    /// struct, an array of packed elements) — those keep the packed path.
    /// Build the packed value for a positional pattern applied to a packed
    /// multi-D target. `dims` are the DECLARED packed dims outermost-first;
    /// each nesting level consumes one. Returns None when the shape cannot be
    /// honored (item count mismatch and no default) so the caller can fall
    /// back rather than store garbage.
    pub(super) fn packed_pattern_value(
        &mut self,
        items: &[AssignmentPatternItem],
        dims: &[(i64, i64)],
        total_w: u32,
    ) -> Option<Value> {
        let count: u64 = match dims.first() {
            Some(&(l, r)) => (l - r).unsigned_abs() + 1,
            None => items.len() as u64,
        };
        if count == 0 || total_w == 0 || total_w as u64 % count != 0 {
            return None;
        }
        let elem_w = total_w / count as u32;
        // Positional items, with `default:` filling any positions not given.
        let mut default_v: Option<Value> = None;
        let mut ordered: Vec<&Expression> = Vec::new();
        for item in items {
            match item {
                AssignmentPatternItem::Ordered(e) => ordered.push(e),
                AssignmentPatternItem::Default(e) => {
                    let dv = self.eval_expr(e);
                    default_v = Some(self.fit_pattern_item_value(dv, elem_w));
                }
                _ => return None,
            }
        }
        if default_v.is_none() && ordered.len() as u64 != count {
            return None;
        }
        let mut acc = Value::zero(0);
        for k in (0..count as usize).rev() {
            let ev = match ordered.get(k) {
                Some(e) => {
                    if let ExprKind::AssignmentPattern(sub) = &e.kind {
                        self.packed_pattern_value(sub, dims.get(1..).unwrap_or(&[]), elem_w)?
                    } else {
                        let v = self.eval_expr(e);
                        self.fit_pattern_item_value(v, elem_w)
                    }
                }
                None => default_v.clone()?,
            };
            acc = ev.concat_with(&acc);
        }
        Some(acc)
    }

    /// Convert one pattern item to the element type: reals round (§6.12.2),
    /// everything else fits by ordinary assignment resize.
    pub(super) fn fit_pattern_item_value(&self, v: Value, elem_w: u32) -> Value {
        if v.is_real {
            Self::real_to_int(v.to_f64(), elem_w)
        } else {
            v.resize_for_assign(elem_w)
        }
    }

    /// Packed-pattern value for an assignment TARGET (bare packed vector, or
    /// an element of an unpacked array of packed vectors). None when the
    /// target has no registered packed element shape.
    pub(super) fn packed_pattern_for_lhs(
        &mut self,
        lhs: &Expression,
        items: &[AssignmentPatternItem],
    ) -> Option<Value> {
        let (name, is_elem) = match &lhs.kind {
            ExprKind::Ident(h) => (self.resolve_hier_name(h), false),
            ExprKind::Index { expr, .. } => {
                if let ExprKind::Ident(h) = &expr.kind {
                    (self.resolve_hier_name(h), true)
                } else {
                    return None;
                }
            }
            _ => return None,
        };
        let packed_elem_w = self.module.packed_signal_elem_widths.get(&*name).copied();
        let total_w = if is_elem {
            self.module.arrays.get(&*name).map(|t| t.2)?
        } else {
            if self.module.arrays.contains_key(&*name) {
                return None; // whole-array target: the unpacked path owns it
            }
            self.lookup_signal_width(&name)?
        };
        let dims: Vec<(i64, i64)> = match packed_elem_w {
            Some(ew) => self
                .module
                .packed_full_dims
                .get(&*name)
                .cloned()
                .unwrap_or_else(|| vec![((total_w as i64 / ew as i64) - 1, 0)]),
            None => {
                // §10.9.2: a plain packed VECTOR is a packed array of 1-bit
                // elements, so `'{default:1'b1}` sets EVERY bit — `logic [15:0]
                // v = '{default:1'b1};` is 16'hffff. With no entry in
                // `packed_signal_elem_widths` this used to bail to the generic
                // concat-the-items fallback, which produced a ONE-BIT 1 and
                // zero-extended it to 16'h0001 (a reset that armed only bank 0).
                // Aggregates with their own pattern path must not be diverted
                // here.
                if self.module.packed_struct_fields.contains_key(&*name)
                    || self.module.associative_arrays.contains_key(&*name)
                    || self.module.dynamic_arrays.contains(&*name)
                    || self.module.arrays_2d.contains_key(&*name)
                    || self.module.arrays_nd.contains_key(&*name)
                    || self.module.string_signals.contains(&*name)
                    || self.real_signals.contains(&*name)
                    || total_w <= 1
                {
                    return None;
                }
                vec![(total_w as i64 - 1, 0)]
            }
        };
        self.packed_pattern_value(items, &dims, total_w)
    }

    /// The names an assignment-pattern lvalue may be registered under. An
    /// unpacked aggregate has no signal of its own — only member leaves — so
    /// `resolve_hier_name` cannot find `bi.us` and collapses it to the leaf
    /// `us`, which owns no type. Try the fully-joined path too.
    pub(super) fn pattern_aggregate_names(&mut self, lvalue: &Expression) -> Vec<String> {
        let mut out = Vec::new();
        if let ExprKind::Ident(h) = &lvalue.kind {
            out.push(self.resolve_hier_name(h).into_owned());
            if h.path.len() > 1 {
                let joined = h
                    .path
                    .iter()
                    .map(|s| s.name.name.as_str())
                    .collect::<Vec<_>>()
                    .join(".");
                if !out.contains(&joined) {
                    out.push(joined);
                }
            }
        }
        if let Some(flat) = self.flat_member_name(lvalue) {
            if !out.contains(&flat) {
                out.push(flat);
            }
        }
        out
    }

    pub(super) fn assign_pattern_aggregate(
        &mut self,
        base: &str,
        items: &[AssignmentPatternItem],
    ) -> bool {
        if items.is_empty() {
            return false;
        }
        // For an array this is the ELEMENT type; for a scalar, its own type.
        let Some(dt) = self.module.var_decl_types.get(base).cloned().or_else(|| {
            // Struct-field sub-path (e.g. `info.addr`): resolve through the
            // type system via `flat_path_type`.
            self.flat_path_type(base).map(|(d, _)| d)
        }) else {
            return false;
        };

        // Associative array: `'{key:value, ...}`. A `default:` value is the
        // fallback for any key not explicitly listed (§7.8), so record it
        // for absent-key reads (elaboration records declaration defaults via
        // `assoc_defaults`; a RUN-TIME `m = '{...}` must record it here).
        if self.module.associative_arrays.contains_key(base) {
            let mut wrote = false;
            for item in items {
                if let AssignmentPatternItem::Keyed(k, v) = item {
                    let kv = self.eval_expr(k);
                    let key = self.assoc_key_str(base, &kv);
                    self.assign_pattern_or_leaf(&format!("{}[{}]", base, key), &dt, v);
                    wrote = true;
                } else if let AssignmentPatternItem::Default(e) = item {
                    self.module
                        .assoc_defaults
                        .insert(base.to_string(), e.clone());
                    wrote = true;
                }
            }
            return wrote;
        }

        // Queue / dynamic array: the pattern's positional items ARE the
        // elements, and they set its size. `'{3{5}}` replication expands here.
        if self.module.dynamic_arrays.contains(base) {
            let exprs = self.pattern_ordered(items);
            if exprs.is_empty() {
                return false;
            }
            for (i, e) in exprs.iter().enumerate() {
                self.assign_pattern_or_leaf(&format!("{}[{}]", base, i), &dt, e);
            }
            self.set_queue_size(base, exprs.len() as u64);
            return true;
        }

        // Struct-field sub-path queue that is NOT yet registered in
        // `dynamic_arrays` (first write): detect via the type system.
        // An UNPACKED STRUCT is never a queue — the member-wise branch at the
        // bottom owns it. Without this the dotted-name heuristic below fired on
        // an INSTANCE-qualified struct (`u.inl` contains a dot just like a
        // struct sub-path does), registered it in `dynamic_arrays` and wrote
        // `u.inl[0]`/`u.inl[1]`, so every member stayed x.
        let dt_is_unpacked_struct = match self.resolve_dt(&dt) {
            DataType::Struct(su) => Self::spreads_member_wise(&su),
            _ => false,
        };
        // A dotted name that is a REGISTERED fixed array (an inlined
        // instance's `u.v[2][2]`, a generate block's `g.w`) keeps its shape:
        // the dynamic-array fallback below reclassified it and handed each
        // row a bare pattern.
        let registered_fixed = self.module.arrays.contains_key(base)
            || self.module.arrays_2d.contains_key(base)
            || self.module.arrays_nd.contains_key(base);
        if !dt_is_unpacked_struct && !self.module.dynamic_arrays.contains(base) && !registered_fixed
        {
            // Only register SUB-PATHS (dotted, like `info.addr`) in
            // `dynamic_arrays` — a bare variable like `src` is NOT a queue
            // even if `p_elem_type` finds an element type for it.
            if base.contains('.') && {
                let pet = self.p_elem_type(base);
                pet.is_some()
            } {
                self.module.dynamic_arrays.insert(base.to_string());
                let exprs = self.pattern_ordered(items);
                if exprs.is_empty() {
                    return false;
                }
                for (i, e) in exprs.iter().enumerate() {
                    self.assign_pattern_or_leaf(&format!("{}[{}]", base, i), &dt, e);
                }
                self.set_queue_size(base, exprs.len() as u64);
                return true;
            }
        }

        // §7.4.2/§10.9: a positional pattern onto a PACKED multi-D vector —
        // `bit [3:0][3:0] x; x = '{1'b1, 2, 3.0, "TEST"};` — converts each
        // item to the ELEMENT type and packs MSB-element-first, recursing one
        // packed dimension per nesting level (a nested `'{...}` inside a 4-bit
        // element splits it into four 1-bit sub-elements). Falling through to
        // the generic eval concatenated the items at their OWN widths, so a
        // 1-bit item shifted every later element. Real items convert (§6.12.2
        // rounds), strings keep their low bits.
        if !self.module.arrays.contains_key(base)
            && self.module.packed_signal_elem_widths.contains_key(base)
        {
            if let Some(total_w) = self.lookup_signal_width(base) {
                let dims: Vec<(i64, i64)> = self
                    .module
                    .packed_full_dims
                    .get(base)
                    .cloned()
                    .unwrap_or_else(|| {
                        let ew = self.module.packed_signal_elem_widths[base] as i64;
                        vec![((total_w as i64 / ew) - 1, 0)]
                    });
                if let Some(v) = self.packed_pattern_value(items, &dims, total_w) {
                    self.set_signal_value_by_name(base, v);
                    self.settle_after_proc_write();
                    return true;
                }
            }
        }

        // Fixed unpacked array of any rank (§10.9.1): positional items,
        // `N{expr}` replication, `index:expr` keys and `default:expr`, with
        // nested patterns descending one dimension at a time.
        if let Some(dims) = self.foreach_dims(base) {
            let cells: i64 = dims.iter().map(|&(lo, hi)| (hi - lo + 1).max(0)).product();
            if cells <= 0 || cells > (1 << 20) {
                return false;
            }
            // Each dimension's direction from its declared bounds; without
            // them only the outermost one is known.
            let descending: Vec<bool> = match self
                .module
                .unpacked_decl_dims
                .get(base)
                .filter(|d| d.len() == dims.len())
            {
                Some(decl) => decl.iter().map(|&(l, r)| l > r).collect(),
                None => vec![dims.len() == 1 && self.module.descending_arrays.contains(base)],
            };
            self.assign_pattern_array(base, &dims, &dt, items, &descending);
            return true;
        }

        if let DataType::Struct(su) = self.resolve_dt(&dt) {
            if Self::spreads_member_wise(&su) {
                self.assign_pattern_into_struct(base, &su, items);
                return true;
            }
        }
        false
    }

    /// `arr[i] = '{...}` / `assoc[key] = '{...}` where the element is an
    /// unpacked struct: spread the pattern into that element's members.
    pub(super) fn assign_pattern_element(
        &mut self,
        lvalue: &Expression,
        rvalue: &Expression,
    ) -> bool {
        let (ExprKind::Index { expr: b, index }, ExprKind::AssignmentPattern(items)) =
            (&lvalue.kind, &rvalue.kind)
        else {
            return false;
        };
        if items.is_empty() {
            return false;
        }
        let ExprKind::Ident(bh) = &b.kind else {
            return false;
        };
        let bname = self.resolve_hier_name(bh);
        let Some(dt) = self.module.var_decl_types.get(&*bname).cloned() else {
            return false;
        };
        let DataType::Struct(su) = self.resolve_dt(&dt) else {
            return false;
        };
        if !Self::spreads_member_wise(&su) {
            return false;
        }
        let iv = self.eval_expr(index);
        let key = if self.module.associative_arrays.contains_key(&*bname) {
            self.assoc_key_str(&bname, &iv)
        } else {
            iv.to_i64().unwrap_or(0).to_string()
        };
        self.assign_pattern_into_struct(&format!("{}[{}]", bname, key), &su, items);
        true
    }

    /// `c.nodes = '{...}` / `c.nodes[0] = '{...}`: an assignment pattern to a
    /// flattened sub-path — an unpacked-struct member, or an unpacked array
    /// member (whole or one element). The type comes from the base variable's
    /// declaration, since a sub-path has none of its own.
    pub(super) fn assign_pattern_flat(
        &mut self,
        flat: &str,
        items: &[AssignmentPatternItem],
    ) -> bool {
        if items.is_empty() {
            return false;
        }
        let Some((dt, arr)) = self.flat_path_type(flat) else {
            return false;
        };
        // Unindexed unpacked array member: one pattern item per element. The
        // element type need not be a struct — `real m[3] = '{1.1, 2.2, 3.3}`
        // spreads exactly the same way.
        // A member with several dimensions takes one nested pattern level
        // per dimension (§10.10).
        if let Some(lists) = arr {
            if !items
                .iter()
                .all(|i| matches!(i, AssignmentPatternItem::Ordered(_)))
            {
                return false;
            }
            self.assign_pattern_items_over_dims(flat, &dt, &lists, items);
            return true;
        }
        let DataType::Struct(su) = self.resolve_dt(&dt) else {
            return false;
        };
        if !Self::spreads_member_wise(&su) {
            return false;
        }
        self.assign_pattern_into_struct(flat, &su, items);
        true
    }

    /// Split a flattened path (`c.nodes[0].str`) into `(segment, index_count)`
    /// pairs. Dots inside brackets belong to an associative key, not the path.
    pub(super) fn split_flat_path(flat: &str) -> Vec<(String, usize)> {
        let mut out = Vec::new();
        let mut name = String::new();
        let mut idx = 0usize;
        let mut depth = 0usize;
        for ch in flat.chars() {
            match ch {
                '[' => {
                    depth += 1;
                    if depth == 1 {
                        idx += 1;
                    }
                }
                ']' => depth = depth.saturating_sub(1),
                '.' if depth == 0 => {
                    out.push((std::mem::take(&mut name), idx));
                    idx = 0;
                }
                _ if depth == 0 => name.push(ch),
                _ => {}
            }
        }
        out.push((name, idx));
        out
    }

    /// The base of a flat path `base[..][..]` that `split_flat_path` reads as
    /// ONE segment named by the text before the first `[`; None for any
    /// other shape (a `.` or other text outside the brackets).
    pub(super) fn flat_single_base(flat: &str) -> Option<&str> {
        let mut depth = 0usize;
        let mut first_open: Option<usize> = None;
        for (i, ch) in flat.char_indices() {
            match ch {
                '[' => {
                    depth += 1;
                    first_open.get_or_insert(i);
                }
                ']' if depth > 0 => depth -= 1,
                _ if depth > 0 => {}
                '.' | ']' => return None,
                _ if first_open.is_some() => return None,
                _ => {}
            }
        }
        Some(&flat[..first_open.unwrap_or(flat.len())])
    }

    /// Declared type of a flattened SUB-PATH (`c.nodes[0].str`, `cmb[20]`),
    /// which has no declaration of its own — walk the base variable's declared
    /// type. The second element holds the index list of every dimension the
    /// path leaves unselected when it still names an unpacked array — all of
    /// a member's dimensions for `s.mm`, the inner ones for `s.mm[1]` — so
    /// `%p`, `$bits` and pattern assignment see the whole sub-array.
    pub(super) fn flat_path_type(&self, flat: &str) -> Option<(DataType, Option<Vec<Vec<i64>>>)> {
        // Without a select the first segment is the text before the first
        // `.`; a base with no declared type answers None before any split.
        if !flat.as_bytes().contains(&b'[') {
            let base = flat.split('.').next().unwrap_or(flat);
            if !self.module.var_decl_types.contains_key(base) {
                return None;
            }
        }
        let segs = Self::split_flat_path(flat);
        let (base, base_idx) = segs.first()?.clone();
        // For an array (or associative array) this is already the ELEMENT type.
        let mut dt = self.module.var_decl_types.get(&base)?.clone();
        let mut arr: Option<Vec<Vec<i64>>> =
            if base_idx > 0 || self.module.associative_arrays.contains_key(&base) {
                None
            } else {
                self.module.arrays.get(&base).and_then(|&(lo, hi, _)| {
                    if hi >= lo && hi - lo < 4096 {
                        Some(vec![(lo..=hi).collect()])
                    } else {
                        None
                    }
                })
            };

        for (seg, nidx) in segs.iter().skip(1) {
            let DataType::Struct(su) = self.resolve_dt(&dt) else {
                return None;
            };
            let (mdt, dims) = su.members.iter().find_map(|m| {
                m.declarators
                    .iter()
                    .find(|md| md.name.name == *seg)
                    .map(|md| (m.data_type.clone(), md.dimensions.clone()))
            })?;
            dt = mdt;
            arr = self
                .member_dim_indices(&dims)
                .filter(|lists| *nidx < lists.len())
                .map(|lists| lists[*nidx..].to_vec());
        }
        Some((dt, arr))
    }

    /// Remaining dimensions of a partially indexed multi-dimensional array
    /// (`m[1]` of `int m[2][3]` still has one dimension left).
    pub(super) fn partial_index_dims(&self, name: &str) -> Option<Vec<(i64, i64)>> {
        if !name.ends_with(']') {
            return None;
        }
        let segs = Self::split_flat_path(name);
        if segs.len() != 1 {
            return None;
        }
        let (base, nidx) = &segs[0];
        let dims = self.foreach_dims(base)?;
        if *nidx == 0 || *nidx >= dims.len() {
            return None;
        }
        Some(dims[*nidx..].to_vec())
    }

    /// Element type recorded for `name`, or derived from its base when `name`
    /// is a flattened sub-path (`q[i]`, which has no declaration of its own).
    pub(super) fn p_elem_type(&self, name: &str) -> Option<DataType> {
        self.p_elem_type_ref(name).map(std::borrow::Cow::into_owned)
    }

    /// `p_elem_type` without the clone: the declared type is borrowed from the
    /// design's type tables whenever it lives there (the common case), and
    /// only the flat-path fallback materialises an owned value. The blocking
    /// assignment path consults this on every write purely as a predicate,
    /// which used to clone a `DataType` (and drop it) per assignment.
    pub(super) fn p_elem_type_ref(&self, name: &str) -> Option<std::borrow::Cow<'_, DataType>> {
        use std::borrow::Cow;
        if let Some(dt) = self.module.var_decl_types.get(name) {
            return Some(Cow::Borrowed(dt));
        }
        // A collection formal's per-call key `@name#N` (see
        // `formal_dyn_key`): the formal's own declared type.
        if let Some(bare) = name
            .strip_prefix('@')
            .and_then(|s| s.rsplit_once('#'))
            .map(|(b, _)| b)
            && let Some(dt) = self.module.var_decl_types.get(bare)
        {
            return Some(Cow::Borrowed(dt));
        }
        // A plain identifier without `[` or `.` already missed `var_decl_types`
        // above; skip `flat_single_base` (which would return `Some(name)` and
        // probe the same key a second time).
        if !name.as_bytes().contains(&b'[') && !name.as_bytes().contains(&b'.') {
            return None;
        }
        match Self::flat_single_base(name) {
            // `flat_path_type` of a one-segment path is its base's
            // declared type: borrow it.
            Some(base) => self.module.var_decl_types.get(base).map(Cow::Borrowed),
            None => self.flat_path_type(name).map(|(d, _)| Cow::Owned(d)),
        }
        .or_else(|| {
            // An ELEMENT (`arr[2]`) carries no type of its own, but
            // `var_decl_types` holds the container's ELEMENT type — so the
            // element's type is the container's entry. Without this an
            // instance-scoped `arr[2] = arr[0]` found no struct type and
            // copied a packed container nobody reads, leaving the members
            // x, while the same statement at top level resolved through
            // `flat_path_type`.
            let base = name.rfind('[').map(|i| &name[..i])?;
            if base.is_empty() {
                return None;
            }
            self.module.var_decl_types.get(base).map(Cow::Borrowed)
        })
    }

    /// Nested element list for a multi-dimensional unpacked array. The innermost
    /// element may itself be a collection (an array of queues), so it goes back
    /// through `render_p_var` first.
    pub(super) fn render_p_dims(
        &mut self,
        base: &str,
        dims: &[(i64, i64)],
        dt: &DataType,
    ) -> String {
        let lists: Vec<Vec<i64>> = dims.iter().map(|&(lo, hi)| (lo..=hi).collect()).collect();
        self.render_p_dim_lists(base, &lists, dt)
    }

    /// `render_p_dims` over explicit per-dimension index lists, so a
    /// descending dimension prints left bound first (§21.2.1.7).
    pub(super) fn render_p_dim_lists(
        &mut self,
        base: &str,
        lists: &[Vec<i64>],
        dt: &DataType,
    ) -> String {
        let mut parts: Vec<String> = Vec::new();
        for &i in &lists[0] {
            let elem = format!("{}[{}]", base, i);
            if lists.len() == 1 {
                let rendered = self
                    .render_p_var(&elem)
                    .unwrap_or_else(|| self.render_p_typed(&elem, dt));
                parts.push(rendered);
            } else {
                parts.push(self.render_p_dim_lists(&elem, &lists[1..], dt));
            }
        }
        format!("'{{{}}}", parts.join(", "))
    }

    /// Is a SUBROUTINE-LOCAL variable `name` a QUEUE/assoc/dynamic/fixed
    /// array (as opposed to a plain scalar)? Local collections are handled by
    /// `render_p_var`'s name-based element path; a scalar local is rendered by
    /// evaluating it directly (its value is not a module signal).
    pub(super) fn local_ident_is_collection(&self, name: &str) -> bool {
        self.module.dynamic_arrays.contains(name)
            || self.module.associative_arrays.contains_key(name)
            || self.module.arrays.contains_key(name)
            || self.module.arrays_2d.contains_key(name)
            || self.module.arrays_nd.contains_key(name)
    }

    /// Does a SUBROUTINE-LOCAL variable `name` of declared type
    /// `var_decl_types[name]` hold a `string`? Resolves a TYPE PARAMETER (e.g.
    /// `T _t;` where `T`→`string`) through the active specialization so the
    /// `%p` argument quoting matches the concrete bound type.
    pub(super) fn p_local_is_string(&self, name: &str) -> bool {
        self.task_frame_string_flag(name)
            .unwrap_or_else(|| self.declared_name_is_string(name))
    }

    pub(super) fn declared_name_is_string(&self, name: &str) -> bool {
        use crate::ast::types::{DataType, SimpleType};
        let Some(mut dt) = self.module.var_decl_types.get(name) else {
            return self.string_signals.contains(name);
        };
        // A typedef may alias a specialization of a class of the SAME name
        // (`typedef req_t #(.DW(DW)) req_t;`, snitch mem_test) — following
        // it lands on itself. Stop on any name seen before, or after a few
        // hops, instead of spinning at time 0 forever.
        let mut seen: Vec<String> = Vec::new();
        'walk: loop {
            if matches!(
                dt,
                DataType::Simple {
                    kind: SimpleType::String,
                    ..
                }
            ) {
                return true;
            }
            match dt {
                DataType::TypeReference { name: tn, .. } => {
                    let n = tn.name.name.as_str();
                    if seen.iter().any(|sn| sn == n) || seen.len() >= 16 {
                        return false;
                    }
                    seen.push(n.to_string());
                    // Type parameter -> concrete bound type name.
                    if let Some(bound) = self.resolve_type_param_binding(n) {
                        if let Some(bdt) = self.module.typedef_types.get(&bound) {
                            dt = bdt;
                            continue 'walk;
                        }
                        // `T` bound to a BUILT-IN (e.g. `T`→`int`/`string`):
                        // only `string` is a quoted `%p` scalar.
                        return bound == "string";
                    }
                    // Named typedef -> its base.
                    if let Some(bdt) = self.module.typedef_types.get(n) {
                        dt = bdt;
                        continue 'walk;
                    }
                    // A class handle is not a string scalar.
                    return false;
                }
                _ => return false,
            }
        }
    }

    /// Is a declared type SIGNED after resolving a TYPE PARAMETER to its
    /// concrete bound (e.g. `T`→`int`) and following typedef aliases?
    /// `is_type_signed(_resolved)` alone cannot bind `TypeReference("T")`, so a
    /// scalar local `T _t` (in a `res#(int)` method) was never stamped signed
    /// and `%0d`/`%p` rendered it unsigned (`3735928559`) where the reference
    /// prints `-559038737`. Resolves the type param via the active
    /// specialization (with the receiver-instance fallback a `super` call
    /// needs) and judges the base kind like `is_type_signed`.
    pub(super) fn type_is_signed_concrete(&self, dt: &crate::ast::types::DataType) -> bool {
        use crate::ast::types::DataType;
        // §8.25: the receiver's type argument shadows a same-named global
        // typedef, including an explicitly unsigned argument.
        if let DataType::TypeReference { name, .. } = dt
            && !name.has_scope()
            && let Some(bound) = self.resolve_type_param_binding(&name.name.name)
            && let Some(signed) = self.concrete_type_signed(&bound)
        {
            return signed;
        }
        // Fast path: resolvable without a type parameter (typedefs only).
        if super::elaborate::is_type_signed_resolved(dt, &self.module.typedef_types) {
            return true;
        }
        let DataType::TypeReference { name: tn, .. } = dt else {
            return super::elaborate::is_type_signed(dt);
        };
        let mut n = tn.name.name.to_string();
        for _ in 0..64 {
            // Is `n` a type parameter bound by the active specialization?
            if let Some(bound) = self.resolve_type_param_binding(&n) {
                // Binding to a concrete atom: judge like `is_type_signed`.
                match bound.as_str() {
                    "int" | "integer" | "longint" | "shortint" | "byte" => return true,
                    _ => {
                        let bdt = self.module.typedef_types.get(&bound).cloned();
                        match bdt {
                            Some(bt) if super::elaborate::is_type_signed(&bt) => return true,
                            Some(DataType::TypeReference { name: bn, .. }) => {
                                n = bn.name.name.to_string();
                                continue;
                            }
                            _ => return false,
                        }
                    }
                }
            }
            // Named typedef -> its base.
            let bdt = self.module.typedef_types.get(&n).cloned();
            match bdt {
                Some(bt) if super::elaborate::is_type_signed(&bt) => return true,
                Some(DataType::TypeReference { name: bn, .. }) => {
                    n = bn.name.name.to_string();
                    continue;
                }
                _ => return false,
            }
        }
        false
    }

    /// LRM §21.2.1.7 `%p` on the variable `name`: recursively render nested
    /// structs, unpacked arrays, enums, strings and reals in assignment-pattern
    /// form. `None` if neither the variable nor its base has a declared type.
    /// The rows of the dynamic array `name` (`sz` of them) when each row is
    /// itself an unpacked array: a fixed tail (`T a[][N]`, registered with a
    /// 2-D or N-D shape) or dynamic rows (`T a[][]`, each row with its own
    /// size). `None` for a dynamic array of scalars.
    /// Declared (left, right) bounds of each unpacked dimension of the array
    /// stored at `name`: a module or local array's, or a class member's
    /// (`<handle>#prop`). A dynamic dimension may be listed or left out.
    fn unpacked_decl_dims_of(&self, name: &str) -> Option<Vec<(i64, i64)>> {
        if let Some(d) = self.module.unpacked_decl_dims.get(name) {
            return Some(d.clone());
        }
        let (h, prop) = name.split_once('#')?;
        let handle = h.parse::<usize>().ok()?;
        let cd = self.class_prop_declarer(handle, prop)?;
        if let Some(d) = cd.array_prop_decl_dims.get(prop) {
            return Some(d.clone());
        }
        // A dynamic outer dimension over fixed rows (`int a[][1:0]`): the
        // rows' declared bounds, evaluated here.
        let tail = cd.dynamic_tail_properties.get(prop)?;
        let p = Some(&self.module.parameters);
        tail.iter()
            .map(|d| match d {
                crate::ast::types::UnpackedDimension::Range { left, right, .. } => Some((
                    super::elaborate::const_eval_i64_with_params(left, p)?,
                    super::elaborate::const_eval_i64_with_params(right, p)?,
                )),
                crate::ast::types::UnpackedDimension::Expression { expr, .. } => {
                    let n = super::elaborate::const_eval_i64_with_params(expr, p)?;
                    (n > 0).then_some((0, n - 1))
                }
                _ => None,
            })
            .collect()
    }

    pub(super) fn render_p_dyn_rows(&mut self, name: &str, sz: u64) -> Option<Vec<String>> {
        let inner: Option<Vec<(i64, i64)>> = self
            .module
            .arrays_2d
            .get(name)
            .map(|&(_, d2, _)| vec![d2])
            .or_else(|| {
                self.module
                    .arrays_nd
                    .get(name)
                    .map(|(sh, _)| sh[1..].to_vec())
            });
        let dyn_rows = inner.is_none()
            && (0..sz).any(|i| {
                let row = format!("{}[{}]", name, i);
                self.module.dynamic_arrays.contains(&row)
                    || self
                        .signals
                        .contains_key(&Self::name_with_suffix(&row, ".size"))
            });
        if inner.is_none() && !dyn_rows {
            return None;
        }
        let elem_dt = match self.class_member_coll_type(name) {
            Some(dt) if !matches!(self.resolve_dt_ref(dt), DataType::TypeReference { .. }) => {
                Some(dt.clone())
            }
            Some(_) => None,
            None => self.p_elem_type(name),
        };
        let mut leaf = |me: &mut Self, key: &str| -> String {
            match &elem_dt {
                Some(dt) => me.render_p_typed(key, dt),
                None => {
                    let v = me
                        .get_signal_value_by_name(key)
                        .unwrap_or_else(|| Value::zero(32));
                    Self::render_p_value(&v, false)
                }
            }
        };
        // §21.2.1.7 / §10.9.1: each fixed dimension prints from its declared
        // left bound to its right one (`[1:0]` gives element 1 first), as a
        // fixed array does; the normalized (lo, hi) printed it upward (#597).
        let inner = inner.map(|dims| {
            let decl = self.unpacked_decl_dims_of(name);
            match decl {
                Some(d) if d.len() == dims.len() + 1 => d[1..].to_vec(),
                Some(d) if d.len() == dims.len() => d,
                _ => dims,
            }
        });
        fn fixed(
            me: &mut Simulator,
            key: &str,
            dims: &[(i64, i64)],
            leaf: &mut dyn FnMut(&mut Simulator, &str) -> String,
        ) -> String {
            let Some((&(l, r), rest)) = dims.split_first() else {
                return leaf(me, key);
            };
            let order: Vec<i64> = if l <= r {
                (l..=r).collect()
            } else {
                (r..=l).rev().collect()
            };
            let parts: Vec<String> = order
                .into_iter()
                .map(|i| fixed(me, &format!("{}[{}]", key, i), rest, leaf))
                .collect();
            format!("'{{{}}}", parts.join(", "))
        }
        let mut out = Vec::with_capacity(sz as usize);
        for i in 0..sz {
            let row = format!("{}[{}]", name, i);
            out.push(match &inner {
                Some(dims) => fixed(self, &row, dims, &mut leaf),
                None => {
                    let m = self.get_queue_size(&row);
                    let parts: Vec<String> = (0..m)
                        .map(|j| leaf(self, &format!("{}[{}]", row, j)))
                        .collect();
                    format!("'{{{}}}", parts.join(", "))
                }
            });
        }
        Some(out)
    }

    /// Whether the element `elem` of an associative array is itself a
    /// collection with its own store (a queue, dynamic or associative row).
    fn p_elem_is_collection(&self, elem: &str) -> bool {
        self.module.dynamic_arrays.contains(elem)
            || self.is_associative_array(elem)
            || self
                .get_signal_value_by_name(&Self::name_with_suffix(elem, ".size"))
                .is_some()
    }

    pub(super) fn render_p_var(&mut self, name: &str) -> Option<String> {
        // Elements of a collection are nested leaves (`p_top_level`).
        if self.module.dynamic_arrays.contains(name)
            || self.module.associative_arrays.contains_key(name)
            || self.module.arrays.contains_key(name)
            || self.module.arrays_2d.contains_key(name)
            || self.module.arrays_nd.contains_key(name)
        {
            self.p_top_level = false;
        }
        // A queue / dynamic array prints exactly its `size()` elements — not the
        // fixed 64-slot buffer it is backed by. Its ELEMENTS (`q[i]` of an
        // array of queues) have no declared type of their own. A static
        // queue keyed per specialization (`C#int::q`, §8.25) is known by
        // its size shadow.
        if self.module.dynamic_arrays.contains(name)
            || (name.contains("::")
                && self
                    .signals
                    .contains_key(&Self::name_with_suffix(name, ".size")))
        {
            let sz = self.get_queue_size(name);
            // §7.4.5/§21.2.1.7: a dynamic outer dimension over unpacked rows
            // (`T a[][N]`, `T a[][]`) lists each row as a nested list.
            if let Some(rows) = self.render_p_dyn_rows(name, sz) {
                return Some(format!("'{{{}}}", rows.join(", ")));
            }
            // A LOCAL queue has no entry in `var_decl_types`, so `p_elem_type`
            // returns None; fall back to rendering each element from its stored
            // value (string elements quoted) instead of bailing to `x`.
            // A class member's element type is on its class; one that does
            // not resolve here (a type parameter) keeps the value path.
            let elem_dt = match self.class_member_coll_type(name) {
                Some(dt) if !matches!(self.resolve_dt_ref(dt), DataType::TypeReference { .. }) => {
                    Some(dt.clone())
                }
                Some(_) => None,
                None => self.p_elem_type(name),
            };
            let parts: Vec<String> = match elem_dt {
                Some(dt) => (0..sz)
                    .map(|i| self.render_p_typed(&format!("{}[{}]", name, i), &dt))
                    .collect(),
                None => {
                    let is_str = self.string_signals.contains(name)
                        || self.string_signals.contains(&format!("{}[0]", name));
                    (0..sz)
                        .map(|i| {
                            let elem = format!("{}[{}]", name, i);
                            // An element may itself be a collection (`int
                            // q[$][$]`): recurse so it renders as a nested list
                            // rather than a packed value.
                            if let Some(nested) = self.render_p_var(&elem) {
                                nested
                            } else {
                                let ev = self
                                    .get_signal_value_by_name(&elem)
                                    .unwrap_or_else(|| Value::zero(32));
                                Self::render_p_value(&ev, is_str)
                            }
                        })
                        .collect()
                }
            };
            return Some(format!("'{{{}}}", parts.join(", ")));
        }
        // A MULTI-dimensional unpacked array is a nested element list; only the
        // 1-D case was handled, so `int m[2][3]` printed `'{x, x}`.
        if self.module.arrays_2d.contains_key(name) || self.module.arrays_nd.contains_key(name) {
            let dims = self.foreach_dims(name)?;
            let dt = self.p_elem_type(name)?;
            // Declared bounds give each dimension's direction: `[1:0]`
            // prints element 1 first.
            if let Some(decl) = self
                .module
                .unpacked_decl_dims
                .get(name)
                .filter(|d| d.len() == dims.len())
            {
                let lists: Vec<Vec<i64>> = decl
                    .iter()
                    .map(|&(l, r)| {
                        if l <= r {
                            (l..=r).collect()
                        } else {
                            (r..=l).rev().collect()
                        }
                    })
                    .collect();
                return Some(self.render_p_dim_lists(name, &lists, &dt));
            }
            return Some(self.render_p_dims(name, &dims, &dt));
        }
        // A PARTIAL index into one (`q3[1]` of `int q3[2][2][$]`) still names a
        // sub-array: render the dimensions it has left.
        if let Some(dims) = self.partial_index_dims(name) {
            let dt = self.p_elem_type(name)?;
            return Some(self.render_p_dims(name, &dims, &dt));
        }
        // Associative array — handled before the type gate so a LOCAL assoc
        // (no `var_decl_types` entry) still renders instead of bailing to `x`.
        // Element type comes from the declared type when known, else each
        // element renders from its raw value.
        if self.module.associative_arrays.contains_key(name) {
            // A class member (`<h>#mem`) has its element type on its class;
            // without it the elements printed untyped, in decimal (§21.2.1.7).
            let dt = self
                .module
                .var_decl_types
                .get(name)
                .cloned()
                .or_else(|| self.flat_path_type(name).map(|(d, _)| d))
                .or_else(|| {
                    self.class_member_coll_type(name)
                        .filter(|d| {
                            !matches!(self.resolve_dt_ref(d), DataType::TypeReference { .. })
                        })
                        .cloned()
                });
            let prefix = format!("{}[", name);
            let registered = self.signal_name_to_id.names_with_prefix(&prefix);
            let mut keys: Vec<String> = self
                .signals
                .keys()
                .map(|k| k.as_str())
                .chain(registered.iter().map(|k| &**k))
                .filter_map(|k| k.strip_prefix(prefix.as_str()))
                .filter_map(|rest| rest.split(']').next())
                .map(|s| s.to_string())
                .collect();
            keys.sort();
            keys.dedup();
            if keys.iter().all(|k| k.parse::<i64>().is_ok()) {
                keys.sort_by_key(|k| k.parse::<i64>().unwrap_or(0));
            }
            let string_keyed = self.is_string_keyed_array(name);
            let val_is_str = self.string_signals.contains(name);
            let parts: Vec<String> = keys
                .iter()
                .map(|k| {
                    let elem = format!("{}[{}]", name, k);
                    let v = match &dt {
                        // §21.2.1.7: an element that is itself a queue,
                        // dynamic or associative array (`int aq[int][$]`,
                        // `int aa[int][int]`) prints as a nested list.
                        _ if self.p_elem_is_collection(&elem) => self
                            .render_p_var(&elem)
                            .unwrap_or_else(|| "'{}".to_string()),
                        Some(d) => self.render_p_typed(&elem, d),
                        None => {
                            let ev = self
                                .get_signal_value_by_name(&elem)
                                .unwrap_or_else(|| Value::zero(32));
                            Self::render_p_value(&ev, val_is_str)
                        }
                    };
                    if string_keyed {
                        format!("\"{}\":{}", k, v)
                    } else {
                        format!("{}:{}", k, v)
                    }
                })
                .collect();
            return Some(format!("'{{{}}}", parts.join(", ")));
        }
        let decl = self.module.var_decl_types.get(name).cloned().or_else(|| {
            // §8.9: a static fixed-array member's store (`Cls::S`).
            (name.contains("::") && self.module.arrays.contains_key(name))
                .then(|| self.class_member_coll_type(name).cloned())
                .flatten()
        });
        let Some(dt) = decl else {
            let (dt, arr) = self.flat_path_type(name)?;
            if let Some(lists) = arr {
                return Some(self.render_p_elem_lists(name, &dt, &lists));
            }
            return Some(self.render_p_typed(name, &dt));
        };
        // Associative array -> `'{key:value, ...}` over its populated keys
        // (LRM §21.2.1.7). Elements live in `signals` as `name[key]…`.
        if self.module.associative_arrays.contains_key(name) {
            let prefix = format!("{}[", name);
            // Elements written at run time live in `signals`; elements created
            // by a declaration initializer live in the compact signal table.
            let registered = self.signal_name_to_id.names_with_prefix(&prefix);
            let mut keys: Vec<String> = self
                .signals
                .keys()
                .map(|k| k.as_str())
                .chain(registered.iter().map(|k| &**k))
                .filter_map(|k| k.strip_prefix(prefix.as_str()))
                .filter_map(|rest| rest.split(']').next())
                .map(|s| s.to_string())
                .collect();
            keys.sort();
            keys.dedup();
            // Numeric keys sort numerically, not lexically.
            if keys.iter().all(|k| k.parse::<i64>().is_ok()) {
                keys.sort_by_key(|k| k.parse::<i64>().unwrap_or(0));
            }
            let string_keyed = self.is_string_keyed_array(name);
            let parts: Vec<String> = keys
                .iter()
                .map(|k| {
                    let elem = format!("{}[{}]", name, k);
                    let v = if self.p_elem_is_collection(&elem) {
                        self.render_p_var(&elem)
                            .unwrap_or_else(|| "'{}".to_string())
                    } else {
                        self.render_p_typed(&elem, &dt)
                    };
                    if string_keyed {
                        format!("\"{}\":{}", k, v)
                    } else {
                        format!("{}:{}", k, v)
                    }
                })
                .collect();
            return Some(format!("'{{{}}}", parts.join(", ")));
        }
        // Fixed unpacked array -> element list. An element may itself be a
        // collection (`int q[3][$]`), so it goes back through `render_p_var`.
        if let Some(&(lo, hi, _)) = self.module.arrays.get(name) {
            if hi >= lo && (hi - lo) < 4096 {
                // §21.2.1.7: elements print in declared (left to right)
                // order — a descending `[2:0]` array starts at index 2.
                let descending = match self.module.unpacked_decl_dims.get(name) {
                    Some(d) if d.len() == 1 => d[0].0 > d[0].1,
                    _ => self.module.descending_arrays.contains(name),
                };
                if descending {
                    let list: Vec<i64> = (lo..=hi).rev().collect();
                    return Some(self.render_p_dim_lists(name, &[list], &dt));
                }
                return Some(self.render_p_dims(name, &[(lo, hi)], &dt));
            }
            return None;
        }
        Some(self.render_p_typed(name, &dt))
    }
}
