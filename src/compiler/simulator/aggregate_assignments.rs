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

    /// §7.6, §10.9.1, §10.10: a class member's fixed-size row lives in
    /// the object's indexed leaf cells, including under a queue or dynamic
    /// outer dimension. Snapshot the RHS before changing any destination.
    pub(super) fn assign_class_scalar_row(&mut self, lhs: &Expression, rhs: &Expression) -> bool {
        if self.module.arrays_2d.is_empty() || self.no_class_objects() {
            return false;
        }
        if !matches!(&lhs.kind, ExprKind::Index { .. })
            && !matches!(&lhs.kind, ExprKind::Ident(h) if h.path.last().is_some_and(|p| !p.selects.is_empty()))
        {
            return false;
        }
        let (base, selects) = Self::split_row_selects(lhs);
        let [index] = selects.as_slice() else {
            return false;
        };
        let Some(name) = self.expr_assoc_name(&base) else {
            return false;
        };
        if Self::class_storage_owner(&name).is_none() {
            return false;
        }
        let Some(&(outer, inner, width)) = self.module.arrays_2d.get(&name) else {
            return false;
        };
        let Some(dt) = self.class_member_coll_type(&name).cloned() else {
            return false;
        };
        if self.unpacked_struct_of(&dt).is_some() {
            return false;
        }
        // A dynamic dimension may be omitted from the declared bounds.
        let (left, right) = self
            .unpacked_decl_dims_of(&name)
            .and_then(|d| d.last().copied())
            .unwrap_or(inner);
        let mut positions: Vec<_> = (inner.0..=inner.1).collect();
        if left > right {
            positions.reverse();
        }
        let values: Vec<(i64, Value)> = match &self.unpacked_pattern(rhs).kind {
            ExprKind::AssignmentPattern(items) => {
                // §11.4.12.1: the operands of a replication are evaluated
                // once, however many elements the copies fill, so
                // `k.a[0] = '{2{f()}}` calls `f` once. The copies are the
                // operand itself, so each operand is evaluated at its first
                // element, in the row's element type, and that value is
                // stored in the others.
                let replicated: Vec<*const Expression> = items
                    .iter()
                    .filter_map(|item| match item {
                        AssignmentPatternItem::Ordered(e) => match &e.kind {
                            ExprKind::Replication { exprs, .. } => Some(exprs),
                            _ => None,
                        },
                        _ => None,
                    })
                    .flatten()
                    .map(|e| e as *const Expression)
                    .collect();
                let mut once: Vec<(*const Expression, Value)> = Vec::new();
                let items = self.pattern_elems_for_type(items, &positions, &dt);
                positions
                    .into_iter()
                    .zip(items)
                    .filter_map(|(j, item)| {
                        let item = item?;
                        let key = item as *const Expression;
                        if !replicated.contains(&key) {
                            return Some((j, self.eval_pattern_leaf_value(&dt, item, Some(width))));
                        }
                        if let Some((_, v)) = once.iter().find(|(k, _)| *k == key) {
                            return Some((j, v.clone()));
                        }
                        let v = self.eval_pattern_leaf_value(&dt, item, Some(width));
                        once.push((key, v.clone()));
                        Some((j, v))
                    })
                    .collect()
            }
            kind => {
                let vals = if let ExprKind::Concatenation(items) = kind {
                    self.unpacked_concat_operand_values(items.iter().collect(), true, None)
                } else if let Some((src, suffixes, _)) = self.nd_array_operand(rhs) {
                    suffixes
                        .iter()
                        .map(|s| {
                            self.get_signal_value_by_name(&format!("{src}{s}"))
                                .unwrap_or_else(|| Value::new(width))
                        })
                        .collect()
                } else {
                    return false;
                };
                let ty = self.resolve_dt_ref(&dt);
                let real = self.dt_is_real(&dt);
                let signed = self.type_is_signed_concrete(&dt);
                let two_state =
                    super::elaborate::is_type_two_state_resolved(&dt, &self.module.typedef_types);
                positions
                    .into_iter()
                    .zip(vals)
                    .map(|(j, v)| {
                        (
                            j,
                            Self::fit_local_decl_init(v, width, ty, two_state, real, signed),
                        )
                    })
                    .collect()
            }
        };
        let dynamic = self.module.dynamic_arrays.contains(&name);
        let hi = if dynamic {
            self.get_queue_size(&name) as i64 - 1
        } else {
            outer.1
        };
        self.dollar_bound.push(hi);
        let selected = self.eval_expr(index);
        self.dollar_bound.pop();
        let i = if dynamic && self.is_queue_store(&name) {
            self.prepare_sequence_write(&name, &selected)
        } else {
            let low = if dynamic { 0 } else { outer.0 };
            selected.to_index().filter(|i| (low..=hi).contains(i))
        };
        if let Some(i) = i {
            for (j, value) in values {
                self.set_signal_value_by_name(&format!("{name}[{i}][{j}]"), value);
            }
        }
        true
    }

    /// `c.m[1]` / `c.m[1][2]` (either parse shape) as the unselected member
    /// expression and its index selects, outermost first. Range selects and
    /// anything else come back whole, with no selects.
    pub(super) fn split_row_selects(e: &Expression) -> (Expression, Vec<Expression>) {
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
            bare.cached_signal_id = crate::ast::cache::SignalCache::new(None);
            bare.cached_resolved_name = crate::ast::cache::NameCache::new();
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
        // §8.9, §8.23: `Cls::q = '{...}` names the class's static
        // collection without an object.
        if let ExprKind::Ident(hi) = &lvalue.kind
            && hi.path.len() == 2
            && hi.path.iter().all(|s| s.selects.is_empty())
            && self.module.classes.contains_key(&hi.path[0].name.name)
        {
            let (cls, member) = (&hi.path[0].name.name, &hi.path[1].name.name);
            return match self.class_scoped_static_coll(cls, member) {
                Some(key) => self.assign_static_collection_pattern(cls, member, &key, items),
                None => false,
            };
        }
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
        // §8.9: a STATIC collection reached through the object is the
        // class's one store, the same one `obj.q[i]` and `Cls::q` use.
        if self.collection_is_static_in(&cn, member) {
            return match self.handle_collection_name(handle, member) {
                Some(key) => self.assign_static_collection_pattern(&cn, member, &key, items),
                None => false,
            };
        }
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

    /// §8.9, §10.9: assign the pattern to the STATIC associative array,
    /// queue or dynamic array `member` of `class`'s chain, stored at `key`
    /// (see `handle_collection_name`). Keyed items replace an associative
    /// array's contents; positional items are a queue's or dynamic array's
    /// elements and set its size.
    fn assign_static_collection_pattern(
        &mut self,
        class: &str,
        member: &str,
        key: &str,
        items: &[AssignmentPatternItem],
    ) -> bool {
        let mut cur = Some(class);
        let mut decl = None;
        while let Some(cn) = cur {
            let Some(cd) = self.module.classes.get(cn) else {
                break;
            };
            if let Some(&(_, is_assoc, w)) =
                cd.static_collections.iter().find(|(n, ..)| n == member)
            {
                decl = Some((is_assoc, w));
                break;
            }
            cur = cd.extends.as_deref();
        }
        let Some((is_assoc, w)) = decl else {
            return false;
        };
        if is_assoc {
            self.assign_class_assoc_pattern(key, w, items);
            return true;
        }
        let exprs = self.pattern_ordered(items);
        for (i, e) in exprs.iter().enumerate() {
            self.pattern_leaf_at_elem_width(&format!("{}[{}]", key, i), e, w);
        }
        self.set_queue_size(key, exprs.len() as u64);
        true
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
            if let AssignmentPatternItem::Default(e) = item {
                if elem_dt.is_none() {
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
                continue;
            }
            let Some(k) = self.array_pattern_key(item) else {
                continue;
            };
            let kv = self.eval_expr(&k);
            let key = self.assoc_key_str(scoped, &kv);
            let target = Self::name_with_key(scoped, &key);
            match &elem_dt {
                Some(dt) => self.assign_pattern_or_leaf(&target, dt, item.expr()),
                None => self.pattern_leaf_at_elem_width(&target, item.expr(), val_w),
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
        let capture = self.pattern_nba_leaves.take();
        if let ExprKind::AssignmentPattern(sub) = &self.unpacked_pattern(e).kind {
            if !sub.is_empty() {
                if let DataType::Struct(su) = self.resolve_dt(dt) {
                    if Self::spreads_member_wise(&su) {
                        self.pattern_nba_leaves = capture;
                        self.assign_pattern_into_struct(target, &su, sub);
                        return;
                    }
                }
                // §10.9.2: a pattern for a packed element takes the
                // element's type.
                if let Some(v) = self.eval_in_packed_type(e, dt) {
                    self.pattern_nba_leaves = capture;
                    self.write_pattern_leaf(target, v);
                    return;
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
                self.pattern_nba_leaves = capture;
                // §7.2, §10.9.2: a whole struct of this type (a variable, an
                // element of an array of them) is the element's value: its
                // members are copied one by one, read now. Spread as a
                // default, each member received the whole struct.
                if self.assign_whole_struct_item(target, &su, e) {
                    return;
                }
                let synthetic = [AssignmentPatternItem::Default(e.clone())];
                self.assign_pattern_into_struct(target, &su, &synthetic);
                return;
            }
        }
        let value = self.eval_pattern_leaf_value(dt, e, None);
        self.pattern_nba_leaves = capture;
        if self.pattern_nba_leaves.is_none()
            && (value.is_real || Self::is_string_data_type(self.resolve_dt_ref(dt)))
        {
            self.set_signal_value_by_name(target, value);
        } else {
            self.write_pattern_leaf(target, value);
        }
    }

    /// Copy the whole unpacked struct `e` (of type `su`, with leaves of its
    /// own: a design or frame variable, an element of an array of them) to
    /// the struct element `target`, every member read before any is
    /// written. False, with nothing read, when `e` is no such struct.
    fn assign_whole_struct_item(
        &mut self,
        target: &str,
        su: &crate::ast::types::StructUnionType,
        e: &Expression,
    ) -> bool {
        let e = Self::unparen(e);
        if !matches!(
            e.kind,
            ExprKind::Ident(_) | ExprKind::Index { .. } | ExprKind::MemberAccess { .. }
        ) || !Self::cond_expr_is_effect_free(e, false)
        {
            return false;
        }
        let capture = self.pattern_nba_leaves.take();
        let src = self
            .flat_member_name(e)
            .filter(|flat| self.struct_storage_exists(flat, su));
        let Some(src) = src else {
            self.pattern_nba_leaves = capture;
            return false;
        };
        let mut leaves = Vec::new();
        self.unpacked_struct_leaves(target, e, su, 0, &mut leaves);
        let values: Vec<(String, Option<Value>)> = leaves
            .into_iter()
            .map(|(key, _, _, _)| {
                let v = self.get_signal_value_by_name(&format!("{src}{}", &key[target.len()..]));
                (key, v)
            })
            .collect();
        self.pattern_nba_leaves = capture;
        for (key, v) in values {
            if let Some(v) = v {
                self.write_pattern_leaf(&key, v);
            }
        }
        true
    }

    /// §10.4.2: collect target writes only; RHS function side effects remain
    /// immediate because item evaluation runs outside the capture context.
    pub(super) fn write_pattern_leaf(&mut self, target: &str, value: Value) {
        if let Some(leaves) = &mut self.pattern_nba_leaves {
            leaves.push((target.to_string(), value));
        } else {
            self.write_leaf_by_name(target, value);
        }
    }

    /// Evaluate one scalar pattern item in its declared element type.
    /// Shared by whole-array writes and slices, which snapshot the items.
    pub(super) fn eval_pattern_leaf_value(
        &mut self,
        dt: &DataType,
        e: &Expression,
        width: Option<u32>,
    ) -> Value {
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
            return self.eval_expr(e);
        }
        // §10.9.2: each item is evaluated in the context of the ELEMENT type —
        // `int d[]; d = '{1'b1 + 1'b1}` is 2, not a 1-bit wrap to 0, and a
        // signed narrower item sign-extends. Self-determined evaluation gave
        // the wrapped/zero-extended value.
        let elem_w = width.unwrap_or_else(|| {
            super::elaborate::resolve_type_width(
                dt,
                Some(&self.module.parameters),
                Some(&self.module.typedefs),
            )
        });
        // §10.8: a pattern item takes the packed element type.
        let mut v = if let Some(v) = self.eval_in_packed_type(e, dt) {
            v
        } else if elem_w > 0 {
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
        // §6.12/§20.5: a SHORTREAL member holds a binary32 value — writing
        // 16777216.25 rounds to IEEE float 16777216.0, not the f64 double.
        if matches!(self.resolve_dt(dt), DataType::Real { .. }) {
            let f = v.to_f64();
            let rlim = if matches!(
                self.resolve_dt(dt),
                DataType::Real {
                    kind: crate::ast::types::RealType::ShortReal,
                    ..
                }
            ) {
                f as f32 as f64
            } else {
                f
            };
            return Value::from_f64(rlim);
        }
        v
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
        match &self.unpacked_pattern(e).kind {
            ExprKind::AssignmentPattern(items) => {
                if is_assoc {
                    self.eval_builtin_method(name, "delete", &[]);
                    self.module.assoc_defaults.remove(name);
                    for item in items {
                        if let AssignmentPatternItem::Default(d) = item {
                            self.module
                                .assoc_defaults
                                .insert(name.to_string(), d.clone());
                            continue;
                        }
                        let Some(k) = self.array_pattern_key(item) else {
                            continue;
                        };
                        let kv = self.eval_expr(&k);
                        let key = self.assoc_key_str(name, &kv);
                        let cell = format!("{}[{}]", name, key);
                        let v = item.expr();
                        match elem_dt {
                            Some(dt) => self.assign_pattern_or_leaf(&cell, dt, v),
                            None => self.pattern_leaf_at_elem_width(&cell, v, elem_w),
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
                if self.assign_struct_concat(name, exprs.iter().collect()) {
                    return true;
                }
                let mut flat: Vec<&Expression> = exprs.iter().collect();
                while let Some(i) = flat
                    .iter()
                    .position(|x| matches!(x.kind, ExprKind::Concatenation(_)))
                {
                    if let ExprKind::Concatenation(c) = &flat.remove(i).kind {
                        flat.splice(i..i, c);
                    }
                }
                let expand_rows = self.concat_has_non_array_elements(name);
                let vals = self.unpacked_concat_operand_values(flat, expand_rows, None);
                self.set_queue_size(name, vals.len() as u64);
                for (i, v) in vals.into_iter().enumerate() {
                    let v = self.fit_queue_push_elem(name, v);
                    self.set_signal_value_by_name(&format!("{}[{}]", name, i), v);
                }
                true
            }
            // §7.5.1, §8.7: `new[n]` / `new[n](src)` parse as calls of
            // `new` (see `sized_new_parts`). A collection target makes them
            // the dynamic-array constructor, which the caller's sized-new
            // store runs; evaluating them here would call the enclosing
            // class's constructor on `this` (and the size expression twice).
            ExprKind::Call { .. } if Self::sized_new_parts(e).is_some() => false,
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

    /// Reserved storage for a fixed array of unpacked structs a function
    /// returns (see `snapshot_ret_fixed`).
    pub(super) const RET_STRUCT_SNAP: &'static str = "__xz_ret_sa__";

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
        // §7.4.5: each copied row is a collection of its own; create it as
        // declared (creating empties it, so before its cells land).
        for (k, _) in &entries {
            if let Some(row) = k.strip_suffix(".size")
                && self.is_declared_row(row)
            {
                self.ensure_row(row);
            }
        }
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
        if let ExprKind::AssignmentPattern(items) = &self.unpacked_pattern(rvalue).kind
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
        let is_member = |n: &str| {
            su.members
                .iter()
                .any(|m| m.declarators.iter().any(|md| md.name.name == n))
        };
        for (mname, mdt, dims, e, explicit) in self.bind_struct_pattern_items(su, items) {
            // §10.9.1, §10.9.2: a fixed unpacked-array member takes a pattern
            // element by element, and a value reached through a type key of
            // its element type (the last one written, before `default:`) or
            // through `default:` is a value of each element. Assigned whole,
            // neither wrote any element.
            let fixed_lists = (!dims.is_empty() && Self::coll_kind_of(&dims).is_none())
                .then(|| self.member_dim_indices(&dims))
                .flatten();
            let whole_item = items.iter().any(|it| match it {
                AssignmentPatternItem::Named(id, _) => id.name == mname,
                AssignmentPatternItem::Ordered(_) => true,
                _ => false,
            });
            let mut elem_key = None;
            if fixed_lists.is_some() && !whole_item {
                for it in items.iter().rev() {
                    let member_key =
                        matches!(it, AssignmentPatternItem::Named(id, _) if is_member(&id.name));
                    if !member_key
                        && let Some((key_dt, v)) = self.array_pattern_type_key(it)
                        && self.pattern_type_matches(&mdt, &key_dt)
                    {
                        elem_key = Some(v);
                        break;
                    }
                }
            }
            let Some(e) = elem_key.or(e) else {
                continue;
            };
            if !explicit && dims.len() == 1 && Self::coll_kind_of(&dims).is_some() {
                continue;
            }
            let lhs = Self::append_member_expr(lvalue, &mname);
            if let Some(lists) = &fixed_lists
                && (!whole_item
                    || matches!(
                        self.unpacked_pattern(e).kind,
                        ExprKind::AssignmentPattern(_)
                    ))
            {
                self.assign_class_member_elems(stmt, &lhs, &mdt, lists, e);
                continue;
            }
            if dims.is_empty()
                && depth < 8
                && let Some(inner) = self.unpacked_struct_of(&mdt)
            {
                match &self.unpacked_pattern(e).kind {
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

    /// The elements of the fixed unpacked-array member `lhs` of a class
    /// struct property, indexed by `lists` (one index list per dimension),
    /// take `e`: a pattern spreads over the outermost dimension (its type
    /// keys matching `elem_dt` at the last one) and anything else is the
    /// value of every element.
    fn assign_class_member_elems(
        &mut self,
        stmt: &Statement,
        lhs: &Expression,
        elem_dt: &DataType,
        lists: &[Vec<i64>],
        e: &Expression,
    ) {
        let Some((indices, rest)) = lists.split_first() else {
            self.exec_stmt_blocking_assign_body(stmt, lhs, e);
            return;
        };
        let items: Vec<Option<Expression>> = match &self.unpacked_pattern(e).kind {
            ExprKind::AssignmentPattern(items) => {
                let elems = if rest.is_empty() {
                    self.pattern_elems_for_type(items, indices, elem_dt)
                } else {
                    self.pattern_elems(items, indices)
                };
                elems.into_iter().map(|x| x.cloned()).collect()
            }
            _ => vec![Some(e.clone()); indices.len()],
        };
        for (&i, item) in indices.iter().zip(items) {
            if let Some(item) = item {
                let elem = Self::index_expr(lhs, i);
                self.assign_class_member_elems(stmt, &elem, elem_dt, rest, &item);
            }
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
        &mut self,
        su: &crate::ast::types::StructUnionType,
        items: &'a [AssignmentPatternItem],
    ) -> Vec<(
        String,
        DataType,
        Vec<crate::ast::types::UnpackedDimension>,
        Option<&'a Expression>,
        bool,
    )> {
        let mut members = Vec::new();
        for m in &su.members {
            for md in &m.declarators {
                members.push((
                    md.name.name.clone(),
                    m.data_type.clone(),
                    self.member_dims(&m.data_type, &md.dimensions).into_owned(),
                ));
            }
        }

        let mut by_member: Vec<Option<&'a Expression>> = vec![None; members.len()];
        let mut by_type: Vec<Option<&'a Expression>> = vec![None; members.len()];
        let mut ordered: Vec<&'a Expression> = Vec::new();
        let mut default: Option<&'a Expression> = None;
        for item in items {
            match item {
                AssignmentPatternItem::Named(id, e)
                    if members.iter().any(|(n, _, _)| *n == id.name) =>
                {
                    if let Some(i) = members.iter().position(|(n, _, _)| *n == id.name) {
                        by_member[i] = Some(e);
                    }
                }
                AssignmentPatternItem::Default(e) => default = Some(e),
                AssignmentPatternItem::Ordered(e) => ordered.push(e),
                _ => {
                    // §10.9.2 `type:value` — binds every member of that type
                    // (last matching type key wins); a member-name key still
                    // wins over it, and `default:` fills the rest.
                    if let Some((key_dt, e)) = self.array_pattern_type_key(item) {
                        for (i, (_, mdt, dims)) in members.iter().enumerate() {
                            if dims.is_empty() && self.pattern_type_matches(mdt, &key_dt) {
                                by_type[i] = Some(e);
                            } else if !dims.is_empty()
                                && self.member_dim_indices(dims).is_some_and(|lists| {
                                    self.pattern_subarray_matches(mdt, &lists, &key_dt)
                                })
                            {
                                by_type[i] = Some(e);
                            }
                        }
                    }
                }
            }
        }

        if let Some(expanded) = self.struct_replication_items(su, items, members.len()) {
            ordered = expanded;
        }

        members
            .into_iter()
            .enumerate()
            .map(|(i, (mname, mdt, dims))| {
                let explicit = by_member[i]
                    .or(by_type[i])
                    .or_else(|| ordered.get(i).copied());
                let e = explicit.or(default);
                (mname, mdt, dims, e, explicit.is_some())
            })
            .collect()
    }

    /// IEEE 1800-2023 §10.9.2: the member expressions of a replicated
    /// structure pattern `'{n{a, b}}` that sets exactly the `members` members
    /// of `su`, in declaration order. The count is a constant expression
    /// (A.6.7.1), so a parameter or localparam count is evaluated here; an
    /// integer literal needs no evaluation. None for any other pattern, a
    /// count other than the member count. Nested member patterns retain
    /// their own array dimensions during binding.
    pub(super) fn struct_replication_items<'a>(
        &mut self,
        _su: &crate::ast::types::StructUnionType,
        items: &'a [AssignmentPatternItem],
        members: usize,
    ) -> Option<Vec<&'a Expression>> {
        // Decide "not a replication" first: every structure pattern
        // assignment comes through here, and the member scan below allocates.
        let [AssignmentPatternItem::Ordered(e)] = items else {
            return None;
        };
        if !matches!(e.kind, ExprKind::Replication { .. }) {
            return None;
        }
        let expanded =
            super::elaborate::struct_pattern_replication_with(items, members, &mut |count| {
                super::elaborate::struct_pattern_count_value(&self.eval_expr(count))
            })?;
        Some(expanded)
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
        self.assign_pattern_into_struct_with_types(base, su, items, &[]);
    }

    fn assign_pattern_into_struct_with_types<'a>(
        &mut self,
        base: &str,
        su: &crate::ast::types::StructUnionType,
        items: &'a [AssignmentPatternItem],
        outer_types: &[(DataType, &'a Expression)],
    ) {
        let is_struct_member = |name: &str| {
            su.members
                .iter()
                .any(|m| m.declarators.iter().any(|md| md.name.name == name))
        };
        let mut local_types: Vec<(DataType, &'a Expression)> = Vec::new();
        for item in items {
            match item {
                AssignmentPatternItem::Ordered(_) | AssignmentPatternItem::Default(_) => {}
                AssignmentPatternItem::Named(id, _) if is_struct_member(&id.name) => {}
                _ => {
                    if let Some((dt, e)) = self.array_pattern_type_key(item) {
                        local_types.push((dt, e));
                    }
                }
            }
        }
        let combined_owned: Vec<(DataType, &'a Expression)>;
        let combined_types: &[(DataType, &'a Expression)] = if local_types.is_empty() {
            outer_types
        } else if outer_types.is_empty() {
            local_types.as_slice()
        } else {
            local_types.extend_from_slice(outer_types);
            combined_owned = local_types;
            combined_owned.as_slice()
        };

        let mut target = String::with_capacity(base.len() + 16);
        target.push_str(base);
        target.push('.');
        let prefix_len = target.len();

        for (mname, mdt, dims, e, explicit) in self.bind_struct_pattern_items(su, items) {
            target.truncate(prefix_len);
            target.push_str(&mname);
            let matched_outer = if !outer_types.is_empty() && dims.is_empty() {
                outer_types
                    .iter()
                    .rev()
                    .find(|(kt, _)| self.pattern_type_matches(&mdt, kt))
                    .map(|(_, v)| *v)
            } else {
                None
            };
            if let Some(ov) = matched_outer {
                self.assign_pattern_or_leaf(&target, &mdt, ov);
                continue;
            }
            if dims.is_empty() {
                if !explicit && !combined_types.is_empty() {
                    if let DataType::Struct(inner) = self.resolve_dt(&mdt)
                        && Self::spreads_member_wise(&inner)
                    {
                        let synth_single;
                        let sub: &[AssignmentPatternItem] = match e.map(|x| &self.unpacked_pattern(x).kind) {
                            Some(ExprKind::AssignmentPattern(sub)) => sub.as_slice(),
                            _ => match e {
                                Some(x) => {
                                    synth_single = [AssignmentPatternItem::Default(x.clone())];
                                    &synth_single
                                }
                                None => &[],
                            },
                        };
                        self.assign_pattern_into_struct_with_types(
                            &target,
                            &inner,
                            sub,
                            combined_types,
                        );
                        continue;
                    }
                    if let Some(e) = e
                        && let Some(v) = self.eval_packed_with_outer_types(
                            e,
                            &mdt,
                            combined_types.iter().map(|(kt, v)| (kt, *v)),
                        )
                    {
                        self.write_pattern_leaf(&target, v);
                        continue;
                    }
                }
                let Some(e) = e else {
                    continue;
                };
                // §10.9.2: the type and default keys reach a packed member
                // no other key sets.
                if !explicit {
                    let capture = self.pattern_nba_leaves.take();
                    let value =
                        self.eval_default_in_packed_type(items, &mdt, &is_struct_member);
                    self.pattern_nba_leaves = capture;
                    if let Some(v) = value {
                        self.write_pattern_leaf(&target, v);
                        continue;
                    }
                }
                // §6.6.7, §7.2.2, §10.9.2: an explicit struct-valued item
                // (including a resolver driver's nested member) supplies its
                // own leaves. Only a scalar default broadcasts to every leaf.
                if explicit
                    && let ExprKind::Ident(h) = &e.kind
                    && h.path.iter().all(|seg| seg.selects.is_empty())
                    && let Some(inner) = self.unpacked_struct_of(&mdt)
                    && let Some(src) = self.flat_member_name(e)
                    && self.struct_storage_exists(&src, &inner)
                {
                    let leaves: Vec<_> = self.struct_leaf_suffixes(&inner)
                        .into_iter()
                        .map(|suffix| {
                            let path = format!(".{suffix}");
                            let source = Self::append_leaf_path(e, &path);
                            (format!("{target}{path}"), self.eval_expr(&source))
                        })
                        .collect();
                    for (key, value) in leaves {
                        self.write_pattern_leaf(&key, value);
                    }
                    continue;
                }
                self.assign_pattern_or_leaf(&target, &mdt, e);
                continue;
            }
            // §10.9.2/§7.2.1: a DYNAMIC member (queue / dynamic / associative)
            // is a whole collection, assigned like any other — its item is a
            // collection-valued expression (`'{7, 8}`, `{q, 4}`, `other.q`)
            // that REPLACES the old contents. A `default:` value names an
            // element, never a whole collection, so it leaves one alone.
            if dims.len() == 1 && Self::coll_kind_of(&dims).is_some() {
                if let Some(e) = e
                    && explicit
                    && self.ensure_struct_member_coll(&target)
                {
                    self.assign_whole_coll(&target, Some(&mdt), e);
                }
                continue;
            }
            // Array member: spread over every dimension.
            let Some(lists) = self.member_dim_indices(&dims) else {
                continue;
            };
            let member_outer = if explicit {
                outer_types
            } else {
                combined_types
            };
            if e.is_some() || !member_outer.is_empty() {
                self.assign_pattern_over_dims_with_types(&target, &mdt, &lists, e, !explicit, member_outer);
            }
        }
    }

    /// Evaluate `e` (a pattern or scalar default) for a packed aggregate `dt`
    /// with `outer_types` applied above `e` (so outer type keys win over items
    /// inside a `default:'{...}`). Never wraps a scalar for a simple bit vector
    /// (which would spread the scalar across individual bits).
    pub(super) fn eval_packed_with_outer_types<'a>(
        &mut self,
        e: &Expression,
        dt: &DataType,
        outer_types: impl IntoIterator<Item = (&'a DataType, &'a Expression)>,
    ) -> Option<Value> {
        let is_pat = matches!(
            self.unpacked_pattern(e).kind,
            ExprKind::AssignmentPattern(_)
        );
        if !is_pat && !crate::compiler::packed_assignment_pattern_plan::is_packed_aggregate(dt, self)
        {
            return None;
        }
        let mut synth: Vec<AssignmentPatternItem> = match &self.unpacked_pattern(e).kind {
            ExprKind::AssignmentPattern(sub) => sub.clone(),
            _ => vec![AssignmentPatternItem::Default(e.clone())],
        };
        for (kt, v) in outer_types {
            synth.push(AssignmentPatternItem::Typed(kt.clone(), v.clone()));
        }
        let expr = Expression::new(ExprKind::AssignmentPattern(synth), e.span);
        let capture = self.pattern_nba_leaves.take();
        let value = self.eval_in_packed_type(&expr, dt);
        self.pattern_nba_leaves = capture;
        value
    }

    fn is_struct_elem_pattern(&mut self, e: &Expression, dt: &DataType) -> bool {
        let DataType::Struct(su) = self.resolve_dt(dt) else {
            return false;
        };
        if !Self::spreads_member_wise(&su) {
            return false;
        }
        if let ExprKind::TypedPattern { ty, .. } = &e.kind
            && self.pattern_type_matches(dt, ty)
        {
            return true;
        }
        let ExprKind::AssignmentPattern(sub) = &self.unpacked_pattern(e).kind else {
            return false;
        };
        sub.iter().any(|it| {
            matches!(
                it,
                AssignmentPatternItem::Named(id, _)
                    if su.members.iter().any(|m| m.declarators.iter().any(|d| d.name.name == id.name))
            )
        })
    }

    fn leaf_elem_type<'a>(&self, dt: &'a DataType) -> std::borrow::Cow<'a, DataType> {
        let mut cur: std::borrow::Cow<'a, DataType> = std::borrow::Cow::Borrowed(dt);
        for _ in 0..16 {
            let DataType::TypeReference {
                name, dimensions, ..
            } = cur.as_ref()
            else {
                break;
            };
            if !dimensions.is_empty() {
                break;
            }
            let key_cow: std::borrow::Cow<'_, str> = if name.has_scope() {
                std::borrow::Cow::Owned(name.qualified())
            } else {
                std::borrow::Cow::Borrowed(name.name.name.as_str())
            };
            if self
                .typedef_unpacked_dims_of(key_cow.as_ref())
                .is_some_and(|d| !d.is_empty())
                && let Some(target) = self.lookup_typedef_target(key_cow.as_ref())
            {
                cur = std::borrow::Cow::Owned(target);
                continue;
            }
            break;
        }
        cur
    }

    fn assign_pattern_over_dims_with_types<'a>(
        &mut self,
        target: &str,
        dt: &DataType,
        lists: &[Vec<i64>],
        e: Option<&'a Expression>,
        from_default: bool,
        outer_types: &[(DataType, &'a Expression)],
    ) {
        let Some((first, rest)) = lists.split_first() else {
            let leaf_dt = self.leaf_elem_type(dt);
            for (key_dt, val) in outer_types.iter().rev() {
                if self.pattern_type_matches(leaf_dt.as_ref(), key_dt) {
                    self.assign_pattern_or_leaf(target, leaf_dt.as_ref(), val);
                    return;
                }
            }
            if !outer_types.is_empty()
                && let DataType::Struct(su) = self.resolve_dt(leaf_dt.as_ref())
                && Self::spreads_member_wise(&su)
            {
                let synth_single;
                let sub: &[AssignmentPatternItem] = match e.map(|x| &self.unpacked_pattern(x).kind) {
                    Some(ExprKind::AssignmentPattern(items)) => items.as_slice(),
                    _ => match e {
                        Some(x) => {
                            synth_single = [AssignmentPatternItem::Default(x.clone())];
                            &synth_single
                        }
                        None => &[],
                    },
                };
                self.assign_pattern_into_struct_with_types(target, &su, sub, outer_types);
                return;
            }
            if let Some(e) = e {
                if !outer_types.is_empty()
                    && let Some(v) = self.eval_packed_with_outer_types(
                        e,
                        leaf_dt.as_ref(),
                        outer_types.iter().map(|(kt, v)| (kt, *v)),
                    )
                {
                    self.write_pattern_leaf(target, v);
                    return;
                }
                // §10.9.1: an inherited default can descend further into the
                // packed element; a value of matching type is stored whole.
                if from_default && !matches!(e.kind, ExprKind::AssignmentPattern(_))
                    && crate::compiler::packed_assignment_pattern_plan::is_packed_aggregate(leaf_dt.as_ref(), self)
                {
                    let items = [AssignmentPatternItem::Default(e.clone())];
                    let capture = self.pattern_nba_leaves.take();
                    let v = self.eval_default_in_packed_type(&items, leaf_dt.as_ref(), &|_| false);
                    self.pattern_nba_leaves = capture;
                    if let Some(v) = v {
                        self.write_pattern_leaf(target, v);
                        return;
                    }
                }
                self.assign_pattern_or_leaf(target, leaf_dt.as_ref(), e);
            }
            return;
        };
        if let Some(e) = e {
            match &self.unpacked_pattern(e).kind {
                ExprKind::AssignmentPattern(sub) if !self.is_struct_elem_pattern(e, dt) => {
                    self.assign_pattern_items_over_dims_with_types(
                        target,
                        dt,
                        lists,
                        sub,
                        outer_types,
                    );
                    return;
                }
                // §10.9.1, §10.10: an unpacked array value supplies the elements.
                _ if outer_types.is_empty() && self.assign_array_item(target, lists, dt, e) => {
                    return;
                }
                _ => {}
            }
        }
        use std::fmt::Write;
        let mut elem_target = String::with_capacity(target.len() + 8);
        elem_target.push_str(target);
        let base_len = elem_target.len();
        for idx in first {
            elem_target.truncate(base_len);
            let _ = write!(&mut elem_target, "[{}]", idx);
            self.assign_pattern_over_dims_with_types(&elem_target, dt, rest, e, from_default, outer_types);
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
        self.assign_pattern_items_over_dims_with_types(target, dt, lists, sub, &[]);
    }

    fn assign_pattern_items_over_dims_with_types<'a>(
        &mut self,
        target: &str,
        dt: &DataType,
        lists: &[Vec<i64>],
        sub: &'a [AssignmentPatternItem],
        outer_types: &[(DataType, &'a Expression)],
    ) {
        let Some((first, rest)) = lists.split_first() else {
            return;
        };
        // IEEE 1800-2023 §10.9.1/§10.10.1: replication supplies a whole
        // dimension, with each expression converted to its element's type.
        // This helper checks the replication shape before evaluating counts
        // or allocating, and leaves mismatched counts on the existing path.
        if let Some(expanded) =
            super::elaborate::struct_pattern_replication_with(sub, first.len(), &mut |count| {
                super::elaborate::struct_pattern_count_value(&self.eval_expr(count))
            })
        {
            use std::fmt::Write;
            let mut elem_target = String::with_capacity(target.len() + 8);
            elem_target.push_str(target);
            let base_len = elem_target.len();
            for (idx, v) in first.iter().zip(expanded) {
                elem_target.truncate(base_len);
                let _ = write!(&mut elem_target, "[{}]", idx);
                self.assign_pattern_over_dims_with_types(
                    &elem_target,
                    dt,
                    rest,
                    Some(v),
                    false,
                    outer_types,
                );
            }
            return;
        }
        let mut per_elem: Vec<Option<&'a Expression>> = vec![None; first.len()];
        let mut local_types: Vec<(DataType, &'a Expression)> = Vec::new();
        let mut dflt: Option<&'a Expression> = None;
        let mut pos = 0usize;
        for item in sub {
            match item {
                AssignmentPatternItem::Ordered(v) => {
                    if let Some(slot) = per_elem.get_mut(pos) {
                        *slot = Some(v);
                    }
                    pos += 1;
                }
                AssignmentPatternItem::Default(v) => dflt = Some(v),
                AssignmentPatternItem::Typed(key_dt, v) => {
                    local_types.push((key_dt.clone(), v));
                }
                AssignmentPatternItem::Keyed(k, v) => {
                    if matches!(k.kind, ExprKind::Ident(_) | ExprKind::MemberAccess { .. })
                        && let Some((key_dt, _)) = self.array_pattern_type_key(item)
                    {
                        local_types.push((key_dt, v));
                        continue;
                    }
                    if let Some(key) = self.eval_expr(k).to_i64()
                        && let Some(p) = first.iter().position(|&i| i == key)
                    {
                        per_elem[p] = Some(v);
                    }
                }
                AssignmentPatternItem::Named(id, v) => {
                    if let Some((key_dt, _)) = self.array_pattern_type_key(item) {
                        local_types.push((key_dt, v));
                        continue;
                    }
                    let key_expr = crate::compiler::packed_assignment_pattern_plan::ident_expr(id);
                    if let Some(key) = self.eval_expr(&key_expr).to_i64()
                        && let Some(p) = first.iter().position(|&i| i == key)
                    {
                        per_elem[p] = Some(v);
                    }
                }
            }
        }
        let combined_owned: Vec<(DataType, &'a Expression)>;
        let combined_types: &[(DataType, &'a Expression)] = if local_types.is_empty() {
            outer_types
        } else if outer_types.is_empty() {
            local_types.as_slice()
        } else {
            local_types.extend_from_slice(outer_types);
            combined_owned = local_types;
            combined_owned.as_slice()
        };
        use std::fmt::Write;
        let mut elem_target = String::with_capacity(target.len() + 8);
        elem_target.push_str(target);
        let base_len = elem_target.len();
        for (p, idx) in first.iter().enumerate() {
            elem_target.truncate(base_len);
            let _ = write!(&mut elem_target, "[{}]", idx);
            if let Some(v) = per_elem[p] {
                self.assign_pattern_over_dims_with_types(
                    &elem_target,
                    dt,
                    rest,
                    Some(v),
                    false,
                    outer_types,
                );
                continue;
            }
            if !rest.is_empty()
                && let Some(matched) = combined_types
                    .iter()
                    .rev()
                    .find(|(kt, _)| self.pattern_subarray_matches(dt, rest, kt))
                    .map(|(_, v)| *v)
            {
                self.assign_pattern_over_dims_with_types(&elem_target, dt, rest, Some(matched), false, &[]);
                continue;
            }
            if dflt.is_some() || !combined_types.is_empty() {
                self.assign_pattern_over_dims_with_types(
                    &elem_target,
                    dt,
                    rest,
                    dflt,
                    true,
                    combined_types,
                );
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
                    // §7.2.2: each element starts with its members' declared
                    // defaults, as a scalar variable of the type does (the
                    // elaborator writes those as initial assignments).
                    self.apply_struct_member_defaults(&e, &su, 0);
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
                    for (fname, off, w) in &fields {
                        if !ts.contains(fname) {
                            continue;
                        }
                        for i in 0..*w {
                            let idx = (*off + i) as usize;
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
                    // §8.4: a class handle starts null; it has no x state.
                    DT::TypeReference { .. }
                        if Self::dt_is_class_handle(&resolved, &self.module) =>
                    {
                        out.insert(leaf);
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

    /// §7.2.2: write each declared member default of the unpacked struct
    /// `su` (`real a = 1.5;`) into the leaves under `base`, converted to the
    /// member's type; nested unpacked struct members recurse. Members with
    /// unpacked dimensions keep their defaults.
    pub(super) fn apply_struct_member_defaults(
        &mut self,
        base: &str,
        su: &crate::ast::types::StructUnionType,
        depth: u32,
    ) {
        if depth > 8 {
            return;
        }
        for m in &su.members {
            let resolved = self.resolve_dt(&m.data_type);
            for md in &m.declarators {
                if !md.dimensions.is_empty() {
                    continue;
                }
                let leaf = format!("{}.{}", base, md.name.name);
                match (&resolved, &md.init) {
                    (DataType::Struct(inner), None) if Self::spreads_member_wise(inner) => {
                        let inner = inner.clone();
                        self.apply_struct_member_defaults(&leaf, &inner, depth + 1);
                    }
                    (_, Some(init)) if self.signal_name_to_id.contains_key(leaf.as_str()) => {
                        let w = super::elaborate::resolve_type_width(
                            &m.data_type,
                            Some(&self.module.parameters),
                            Some(&self.module.typedefs),
                        )
                        .max(1);
                        let raw = self.eval_expr_ctx(init, w);
                        let two_state = super::elaborate::is_type_two_state_resolved(
                            &m.data_type,
                            &self.module.typedef_types,
                        );
                        let is_real = self.dt_is_real(&m.data_type);
                        let signed = self.type_is_signed_concrete(&m.data_type);
                        let v = Self::fit_local_decl_init(
                            raw,
                            w,
                            &m.data_type,
                            two_state,
                            is_real,
                            signed,
                        );
                        self.set_signal_value_by_name(&leaf, v);
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
                let dims = self.member_dims(&m.data_type, &md.dimensions);
                let mbase = format!("{}.{}", base, md.name.name);
                // §7.4.2: a member with fixed unpacked dimensions is one leaf
                // per element, each defaulting like a scalar member; a
                // dynamic member has no leaves to seed.
                let leaves: Vec<String> = if dims.is_empty() {
                    vec![mbase]
                } else {
                    match self.member_elem_indices(&dims) {
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
                let dims = self.member_dims(&m.data_type, &md.dimensions);
                if let Some(lists) = self.member_dim_indices(&dims) {
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
        // §7.4.6, §10.4.2: a slice of an unpacked array on either side (#867).
        if let Some(leaves) = self.unpacked_slice_nba_leaves(lvalue, rvalue) {
            return Some(leaves);
        }
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

    /// Declared bounds of a fixed multi-dimensional unpacked array, outermost
    /// first, or None for anything else (queues, associative arrays, 1-D
    /// arrays, arrays of unpacked structs).
    fn fixed_unpacked_dims(&self, name: &str) -> Option<Vec<(i64, i64)>> {
        if self.module.dynamic_arrays.contains(name) || self.is_associative_array(name) {
            return None;
        }
        if let Some(dt) = self.module.var_decl_types.get(name)
            && matches!(self.resolve_dt(dt), DataType::Struct(_))
        {
            return None;
        }
        if let Some(&(r0, r1, _)) = self.module.arrays_2d.get(name) {
            return Some(match self.module.unpacked_decl_dims.get(name) {
                Some(d) if d.len() == 2 => d.clone(),
                _ => vec![r0, r1],
            });
        }
        let (shape, _) = self.module.arrays_nd.get(name)?;
        Some(match self.module.unpacked_decl_dims.get(name) {
            Some(d) if d.len() == shape.len() => d.clone(),
            _ => shape.clone(),
        })
    }

    /// For an array operand that is a whole array or a row of a multi-
    /// dimensional unpacked array (`q`, `q[1]`), the dimensions left below the
    /// selected indices and the leaf-path suffixes in declared order.
    pub(super) fn unpacked_row_suffixes(&mut self, expr: &Expression) -> Option<Vec<String>> {
        let mut idx_exprs: Vec<&Expression> = Vec::new();
        let mut cur = expr;
        while let ExprKind::Index { expr: inner, index } = &cur.kind {
            idx_exprs.push(index);
            cur = inner;
        }
        let ExprKind::Ident(h) = &cur.kind else {
            return None;
        };
        let nm = self.resolve_hier_name(h).into_owned();
        let dims = self.fixed_unpacked_dims(&nm)?;
        let k = idx_exprs.len();
        if k >= dims.len() {
            return None;
        }
        for ie in &idx_exprs {
            if !Self::cond_expr_is_effect_free(ie, false) || self.eval_expr(ie).has_xz() {
                return None;
            }
        }
        let mut total: u64 = 1;
        for &(l, r) in &dims[k..] {
            total = total
                .checked_mul(l.abs_diff(r) + 1)
                .filter(|&t| t <= 1 << 16)?;
        }
        let mut suffixes = vec![String::new()];
        for &(l, r) in &dims[k..] {
            let idx: Vec<i64> = if l <= r {
                (l..=r).collect()
            } else {
                (r..=l).rev().collect()
            };
            suffixes = suffixes
                .iter()
                .flat_map(|p| idx.iter().map(move |i| format!("{}[{}]", p, i)))
                .collect();
        }
        Some(suffixes)
    }

    /// §10.4.2 / §7.6: a nonblocking assignment whose target is a ROW of a
    /// multi-dimensional unpacked array (`q[1] <= in_i`, `q[i] <= q[i+1]`,
    /// `q[0] <= '{default: 0}`). Each element is its own signal, so the row is
    /// updated as one nonblocking assignment per element, every source read
    /// now. Without this the single update queued against `q[1]` landed on no
    /// storage and the row read back x.
    pub(super) fn unpacked_row_nba_leaves(
        &mut self,
        lvalue: &Expression,
        rvalue: &Expression,
    ) -> Option<Vec<(Expression, Value)>> {
        // Only a 2-D or N-D unpacked array has rows (`fixed_unpacked_dims`);
        // most designs have none, and every indexed NBA comes through here.
        if !matches!(lvalue.kind, ExprKind::Index { .. })
            || (self.module.arrays_2d.is_empty() && self.module.arrays_nd.is_empty())
        {
            return None;
        }
        let targets = self.unpacked_row_suffixes(lvalue)?;
        let mut values: Vec<Value> = Vec::with_capacity(targets.len());
        if let ExprKind::AssignmentPattern(items) = &rvalue.kind {
            // A flat pattern over a single remaining dimension: element i is
            // the i-th positional item, the item keyed by its declared index,
            // or the default.
            let first = targets.first()?;
            if first.matches('[').count() != 1 {
                return None;
            }
            let indices: Vec<i64> = targets
                .iter()
                .map(|s| s.trim_matches(['[', ']']).parse().ok())
                .collect::<Option<_>>()?;
            // §11.8.2: a real element lends no integral width. Lending the
            // 64-bit element width wraps `u - 2` at 64 bits.
            let mut cur = lvalue;
            let mut idx = vec![indices[0]];
            while let ExprKind::Index { expr, index } = &cur.kind {
                idx.insert(0, self.eval_expr(index).to_i64().unwrap_or(0));
                cur = expr;
            }
            let base_name = match &cur.kind {
                ExprKind::Ident(h) => Some(self.resolve_hier_name(h).into_owned()),
                _ => None,
            };
            let elem_dt = base_name
                .as_deref()
                .and_then(|n| self.module.var_decl_types.get(n).cloned());
            let elems = match elem_dt.as_ref() {
                Some(dt) => self.pattern_elems_for_type(items, &indices, dt),
                None => self.pattern_elems(items, &indices),
            };
            let real_row = base_name
                .as_deref()
                .is_some_and(|n| self.lookup_signal_real(&Self::elem_name(n, &idx)));
            for (sfx, e) in targets.iter().zip(elems) {
                let e = e?;
                values.push(if real_row {
                    self.eval_expr(e)
                } else {
                    self.eval_target_rhs(&Self::append_leaf_path(lvalue, sfx), e)
                });
            }
        } else if let ExprKind::Concatenation(parts) = &rvalue.kind {
            // §10.10: an unpacked array concatenation over a row of one
            // dimension (`q[1] <= {a, b}`): every operand is read now, and
            // one that is an unpacked array contributes each of its elements.
            if parts.is_empty() || targets.first()?.matches('[').count() != 1 {
                return None;
            }
            values = self.unpacked_concat_operand_values(parts.iter().collect(), true, None);
            if values.len() != targets.len() {
                return None;
            }
        } else {
            // A row or whole multi-dimensional array, or a plain 1-D unpacked
            // array of the same element count (`q[1] <= in_i`).
            let sources = match self.unpacked_row_suffixes(rvalue) {
                Some(s) => s,
                None => {
                    let src = self.foreach_check_operand_name(rvalue)?;
                    if self.module.dynamic_arrays.contains(&src) || self.is_associative_array(&src)
                    {
                        return None;
                    }
                    let (elems, _) = self.stream_elem_order(&src)?;
                    elems.iter().map(|e| e[src.len()..].to_string()).collect()
                }
            };
            if sources.len() != targets.len() {
                return None;
            }
            for sfx in &sources {
                let leaf = Self::append_leaf_path(rvalue, sfx);
                values.push(self.eval_expr(&leaf));
            }
        }
        Some(
            targets
                .iter()
                .zip(values)
                .map(|(sfx, v)| (Self::append_leaf_path(lvalue, sfx), v))
                .collect(),
        )
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
        if matches!(
            self.unpacked_pattern(rvalue).kind,
            ExprKind::AssignmentPattern(_)
        ) || !Self::expr_root_first_seg(lvalue).is_some_and(|r| self.struct_root_possible(r))
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
                let dims = self.member_dims(&m.data_type, &md.dimensions);
                let d = format!("{}.{}", dst, md.name.name);
                let sname = format!("{}.{}", src, md.name.name);
                if dims.is_empty() {
                    if let DataType::Struct(inner) = self.resolve_dt(&m.data_type) {
                        if Self::spreads_member_wise(&inner) {
                            self.copy_unpacked_struct(&d, &sname, &inner);
                            continue;
                        }
                    }
                    if let Some(v) = self.struct_leaf_value(&sname) {
                        let is_str = matches!(
                            self.resolve_dt_ref(&m.data_type),
                            DataType::Simple {
                                kind: crate::ast::types::SimpleType::String,
                                ..
                            }
                        );
                        self.write_struct_leaf(&d, v, is_str);
                    }
                } else if dims.iter().any(|dim| {
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
                    if dims.len() == 1
                        && matches!(
                            dims[0],
                            crate::ast::types::UnpackedDimension::Associative { .. }
                        )
                    {
                        self.ensure_struct_member_coll(&sname);
                        if !self.ensure_struct_member_coll(&d) {
                            let sk = self.string_keyed_dim(&dims[0]);
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
                } else if let Some(tups) = self.member_elem_indices(&dims) {
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

    /// The leaf `flat` of an unpacked struct, wherever it is stored: a signal,
    /// or, for an element of a class property's fixed array of unpacked
    /// structs (`<h>#fx[1].a`), the instance's own cell `fx[1].a`.
    pub(super) fn struct_leaf_value(&self, flat: &str) -> Option<Value> {
        if let Some(v) = self.get_signal_value_by_name(flat) {
            return Some(v);
        }
        let (h, key) = flat.split_once('#')?;
        let inst = self.heap.get(h.parse::<usize>().ok()?)?.as_ref()?;
        inst.properties.get(key).cloned()
    }

    /// Store `v` into the struct leaf `flat` ([`Self::struct_leaf_value`]):
    /// an instance cell that exists keeps its width (a string its length).
    pub(super) fn write_struct_leaf(&mut self, flat: &str, v: Value, is_str: bool) {
        if let Some((h, key)) = flat.split_once('#')
            && let Ok(h) = h.parse::<usize>()
            && let Some(Some(inst)) = self.heap.get_mut(h)
            && let Some(old) = inst.properties.get_mut(key)
        {
            *old = if is_str || v.is_real || old.is_real {
                v
            } else {
                let mut nv = v.resize(old.width);
                nv.is_signed = old.is_signed;
                nv
            };
            return;
        }
        self.write_leaf_by_name(flat, v);
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
    pub(super) fn queue_copy_elem(
        &mut self,
        src_obj: &str,
        si: impl std::fmt::Display,
        dst_obj: &str,
        di: u64,
    ) {
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
        self.queue_elem_moves += 1;
        self.queue_copy_elem(obj_name, from, obj_name, to);
    }

    /// The placeholder element cells of queue `obj` — `Some((first id,
    /// count))`, or `None` when it has none — if every element of `obj` is
    /// one plain value that a by-name read or write of `obj[i]` finds in
    /// such a cell (below `count`) or in the name-keyed store (from `count`
    /// on). `Err` for any other queue: an element spread over several
    /// entries (an unpacked struct, a fixed array, a collection), or a name
    /// some other storage could answer for (a call frame, a forced name, a
    /// packed container, a scope-qualified twin).
    fn plain_queue_cells(&self, obj: &str) -> Result<Option<(usize, u64)>, ()> {
        if !self.forced_names.is_empty() || self.module.arrays_2d.contains_key(obj) {
            return Err(());
        }
        if self.has_coll_rows
            && self
                .coll_shape_at(obj)
                .is_some_and(|(shape, depth)| shape.dims.len() > depth + 1)
        {
            return Err(());
        }
        if self.queue_elem_struct(obj).is_some() {
            return Err(());
        }
        let first_elem = Self::name_with_index(obj, 0);
        if self.module.associative_arrays.contains_key(&first_elem)
            || self.module.dynamic_arrays.contains(&first_elem)
        {
            return Err(());
        }
        if obj.contains('.') {
            // A name a call frame holds, `<top>.<name>` and a member of a
            // packed container are each resolved before the element stores.
            let below = |k: &str| {
                k.len() > obj.len() && k.as_bytes()[obj.len()] == b'[' && k.starts_with(obj)
            };
            if self
                .local_stack
                .last()
                .is_some_and(|l| l.keys().any(|k| below(k)))
            {
                return Err(());
            }
            let top = self.module.name.as_str();
            if obj.len() > top.len() && obj.as_bytes()[top.len()] == b'.' && obj.starts_with(top) {
                return Err(());
            }
            if obj
                .match_indices('.')
                .any(|(p, _)| self.module.packed_struct_fields.contains_key(&obj[..p]))
            {
                return Err(());
            }
        } else if let Some(hint) = self.name_resolve_hint.borrow().as_deref() {
            // An undotted read tries the hinted scope's `<hint>.<name>` first.
            if self
                .signal_name_to_id
                .contains_key(&format!("{}.{}", hint, first_elem))
            {
                return Err(());
            }
        }
        let cells = match self.array_first_id.get(obj) {
            Some(&(first, 0, hi)) if hi >= 0 && !is_packed_id(first) => {
                Some((first, hi as u64 + 1))
            }
            Some(_) => return Err(()),
            None => None,
        };
        // Past the cells no element has a table id of its own.
        let past = Self::name_with_index(obj, cells.map_or(0, |(_, n)| n as i64));
        if self.signal_name_to_id.contains_key(past.as_str())
            || resolve_array_elem_id(&past, &self.array_first_id).is_some()
        {
            return Err(());
        }
        Ok(cells)
    }

    /// §7.10.2: shift the elements of queue `obj` (`size` of them) for the
    /// removal of element `idx` (`open == false`: the elements above it move
    /// down one index) or for an insertion at `idx` (`open == true`: the
    /// elements from `idx` on move up one, and `obj[idx]` keeps its value
    /// until the caller stores the new element). The caller sets the new
    /// size.
    ///
    /// The elements held in the name-keyed store move as one run
    /// (`SignalMap::run_remove_at` / `run_open_at`), not one by one by name,
    /// so the cost does not grow with the queue; the placeholder cells of a
    /// design-scope queue (at most 64) are copied cell to cell. `false`,
    /// with nothing changed, for a queue `plain_queue_cells` does not vouch
    /// for or whose stored elements are not exactly one run: the caller then
    /// moves the elements one by one.
    pub(super) fn queue_shift_fast(&mut self, obj: &str, idx: u64, size: u64, open: bool) -> bool {
        if idx >= size || (!open && idx + 1 == size) {
            // Nothing above `idx` to move: the caller's new size is the
            // whole change.
            return true;
        }
        let Ok(cells) = self.plain_queue_cells(obj) else {
            return false;
        };
        let n_cells = cells.map_or(0, |(_, n)| n);
        if size > n_cells && !self.signals.run_covers(obj, n_cells, size) {
            return false;
        }
        // What a by-name element write marks besides the element: the size
        // proxy of the name before the first `[` (§9.2.2.2).
        let base = &obj[..obj.find('[').unwrap_or(obj.len())];
        let touch_id = (!base.is_empty()
            && (self.module.dynamic_arrays.contains(base)
                || self.module.associative_arrays.contains_key(base)))
        .then(|| {
            self.signal_name_to_id
                .get(Self::name_with_suffix(base, ".size").as_str())
                .copied()
        })
        .flatten();
        let touch = |me: &mut Self| {
            if let Some(id) = touch_id {
                me.mark_dirty_id(id);
                me.dirty_any = true;
            }
        };
        let copy_cell = |me: &mut Self, first: usize, from: u64, to: u64| {
            let (src, dst) = (first + from as usize, first + to as usize);
            let mut v = me.signal_table[src].clone();
            if me.signal_signed[src] {
                v.is_signed = true;
            }
            if me.store_named_cell(dst, v) {
                touch(me);
            }
        };
        // The elements from `n_cells` on, as the run holds them.
        let tail = size.saturating_sub(n_cells);
        if open {
            // Highest element first, as the one-by-one move goes.
            if tail > 0 {
                self.signals
                    .run_open_at(obj, n_cells, idx.max(n_cells), size);
                touch(self);
            }
            if let Some((first, n)) = cells
                && idx < n
            {
                if size >= n {
                    // The last cell's element moves into the name-keyed store.
                    let last = Self::name_with_index(obj, n as i64 - 1);
                    if let Some(v) = self.get_signal_value_by_name(&last) {
                        self.set_signal_value_by_name(&Self::name_with_index(obj, n as i64), v);
                    }
                }
                for i in (idx..size.min(n - 1)).rev() {
                    copy_cell(self, first, i, i + 1);
                }
            }
        } else {
            if let Some((first, n)) = cells
                && idx < n
            {
                for i in idx..(size - 1).min(n - 1) {
                    copy_cell(self, first, i + 1, i);
                }
                if tail > 0 {
                    // The first name-keyed element moves into the last cell.
                    let next = Self::name_with_index(obj, n as i64);
                    if let Some(v) = self.get_signal_value_by_name(&next) {
                        self.set_signal_value_by_name(&Self::name_with_index(obj, n as i64 - 1), v);
                    }
                }
            }
            if tail > 0 {
                let at = idx.max(n_cells);
                self.signals.run_remove_at(obj, n_cells, at, size);
                if at + 1 < size {
                    touch(self);
                }
            }
        }
        true
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
        if let Some(v) = self.eval_pattern_for_lvalue(lvalue, rvalue) {
            return v;
        }
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

    /// §5.7.1, §6.12.1: an unbased unsized literal stored into a real target
    /// converts at its self-determined one bit (`r <= '1` stores 1.0); the
    /// target's 64-bit storage width would otherwise widen it to 2**64-1
    /// first. Any other value is returned unchanged.
    pub(super) fn fill_into_real_target(&mut self, lvalue: &Expression, val: Value) -> Value {
        if !val.is_fill {
            return val;
        }
        let real = self.assign_rhs_ctx(lvalue, 64) == 0
            || self
                .resolve_nba_target(lvalue)
                .is_some_and(|id| self.signal_real.get(id).copied().unwrap_or(false))
            || match &lvalue.kind {
                // An element of a real array, queue or associative array.
                ExprKind::Index { expr: base, .. } => match &base.kind {
                    ExprKind::Ident(h) => {
                        let n = self.resolve_hier_name(h).into_owned();
                        self.coll_elem_is_real(&n)
                    }
                    _ => false,
                },
                _ => false,
            };
        if real {
            Value::from_f64(val.to_f64())
        } else {
            val
        }
    }

    #[inline(never)]
    pub(super) fn assign_rhs_ctx(&mut self, lvalue: &Expression, w: u32) -> u32 {
        // A real variable is 64 bits wide; any other width is integral.
        if w == 64 && matches!(lvalue.kind, ExprKind::Ident(_)) && self.expr_is_real_static(lvalue)
        {
            return 0;
        }
        // §11.8.2: a REAL class property also lends only its storage width
        // (64) — `infer_lhs_width` sizes it so a real NBA RHS Value survives
        // the schedule-time resize — but a real target has no bit width to
        // lend an integral RHS either: `r = u - 2` with `int unsigned u = 1`
        // evaluates `u - 2` self-determined (32-bit wrap → 4294967295.0),
        // not at 64, where the read of `u` widens and 1-2 wraps to 2^64-1.
        // The flat tables above cannot see properties, so ask the declared
        // types; a 64-bit INTEGRAL property (`logic [63:0]`) keeps the 64
        // context, and only an Ident-shaped lvalue is consulted.
        if w == 64
            && let ExprKind::Ident(h) = &lvalue.kind
            && h.root.is_none()
            && h.path
                .last()
                .is_some_and(|s| self.may_name_real_class_prop(&s.name.name))
        {
            // Bare leaf in a method: this object's declared property.
            if h.path.len() == 1 && h.path[0].selects.is_empty() {
                if let Some(&Some(handle)) = self.this_stack.last()
                    && self.class_prop_is_real(handle, &h.path[0].name.name)
                {
                    return 0;
                }
            } else if h.path.len() > 1 && self.head_denotes_class_object(h) {
                let prop = &h.path.last().unwrap().name.name;
                let head = &h.path[0].name.name;
                // Handle-qualified (`h.prop`): the head's live object.
                if let Some(handle) = self.eval_ident_handle(head) {
                    if self.class_prop_is_real(handle, prop) {
                        return 0;
                    }
                } else if let Some(cn) = self.class_of_var(head) {
                    // Owner-qualified static (`C::prop`): the class itself.
                    if self.class_prop_is_real_in(&cn, prop, 0) {
                        return 0;
                    }
                }
            }
        }
        w
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
                        if self.dt_is_real(dt)
                ),
                // §6.24.1: `R'(i)` with `typedef real R` is a real operand.
                "$__xz_named_cast" => args
                    .first()
                    .and_then(|a| self.named_cast_key(a))
                    .and_then(|k| self.module.typedef_types.get(&k))
                    .is_some_and(|dt| self.dt_is_real(dt)),
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
                .is_some_and(|fd| self.dt_is_real(&fd.return_type))
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
                n if n != "$realtime"
                    && n != "$__xz_realtime_at"
                    && super::bytecode::system_function_is_real(n) =>
                {
                    args.iter().all(|a| self.expr_is_constant(a))
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// §11.8.2: convert each integral operand from its self-determined value.
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
        let lr = self.expr_is_real_static(left);
        let rr = self.expr_is_real_static(right);
        if !lr && !rr {
            return None;
        }
        let l = self.real_operand(left);
        let r = self.real_operand(right);
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
        let mut arm = |e: &Expression| {
            if self.expr_is_constant(e) {
                self.eval_in_real_context(e)
            } else {
                self.real_operand(e)
            }
        };
        Some(match c.is_nonzero() {
            Some(true) => arm(then_expr),
            Some(false) => arm(else_expr),
            None => arm(then_expr).merge_unknown(&arm(else_expr)),
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

    /// IEEE 1800-2023 §6.16.6 and Table 6-9 use unsigned byte ordering.
    /// The text representation stores one Latin-1 character per SV byte.
    pub(super) fn sv_strcmp(a: &str, b: &str, icase: bool) -> i64 {
        let byte = |c: char| {
            let b = c as u8;
            i64::from(if icase { b.to_ascii_lowercase() } else { b })
        };
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
        let vals: Vec<Value> = match &self.unpacked_pattern(init).kind {
            ExprKind::AssignmentPattern(items) => {
                items.iter().map(|it| self.eval_expr(it.expr())).collect()
            }
            // §10.10: an array operand of the concatenation (`{w, 5}`, a
            // row `m[1]`, a slice `a[0 +: 2]`) contributes all its elements,
            // and a nested concatenation splices its own.
            ExprKind::Concatenation(parts) => {
                let mut flat: Vec<&Expression> = parts.iter().collect();
                while let Some(i) = flat
                    .iter()
                    .position(|x| matches!(x.kind, ExprKind::Concatenation(_)))
                {
                    if let ExprKind::Concatenation(c) = &flat.remove(i).kind {
                        flat.splice(i..i, c);
                    }
                }
                let expand_rows = self.concat_has_non_array_elements(scoped_q);
                self.unpacked_concat_operand_values(flat, expand_rows, None)
            }
            _ => return,
        };
        let (_lo, _hi, w) = self
            .module
            .arrays
            .get(scoped_q)
            .copied()
            .unwrap_or((0, 63, 32));
        let n = vals.len();
        for (i, mut v) in vals.into_iter().enumerate() {
            // §5.7.1: an unsized fill element takes the element width.
            if v.is_fill && !v.is_real {
                v = v.resize(w.max(1));
            }
            self.set_signal_value_by_name(&format!("{}[{}]", scoped_q, i), v);
            self.widths.insert(format!("{}[{}]", scoped_q, i), w);
        }
        self.set_queue_size(scoped_q, n as u64);
    }

    /// Relative leaf paths of an unpacked struct (`a`, `inner.b`, `arr[0]`).
    pub(super) fn struct_leaf_suffixes(
        &self,
        su: &crate::ast::types::StructUnionType,
    ) -> Vec<String> {
        let mut out = Vec::new();
        for m in &su.members {
            for md in &m.declarators {
                let dims = self.member_dims(&m.data_type, &md.dimensions);
                let base = md.name.name.clone();
                if dims.is_empty() {
                    if let DataType::Struct(inner) = self.resolve_dt(&m.data_type) {
                        if Self::spreads_member_wise(&inner) {
                            for sfx in self.struct_leaf_suffixes(&inner) {
                                out.push(format!("{}.{}", base, sfx));
                            }
                            continue;
                        }
                    }
                    out.push(base);
                } else if let Some(tups) = self.member_elem_indices(&dims) {
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
                let dims = self.member_dims(&m.data_type, &md.dimensions);
                let base = md.name.name.as_str();
                let inner = self.unpacked_struct_of(&m.data_type);
                if dims.is_empty() {
                    if let Some(inner) = inner {
                        for (sfx, k, dt, sk) in self.struct_coll_members(&inner) {
                            out.push((format!("{}.{}", base, sfx), k, dt, sk));
                        }
                    }
                } else if dims.len() == 1
                    && let Some(kind) = Self::coll_kind_of(&dims)
                {
                    let string_key = self.string_keyed_dim(&dims[0]);
                    out.push((base.to_string(), kind, m.data_type.clone(), string_key));
                } else if let (Some(inner), Some(tups)) =
                    (inner, self.member_elem_indices(&dims))
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

    /// Whether the collections `a` and `b` hold the same elements under
    /// `mode` (§7.2.1, §11.4.5): the same size and equal elements for a
    /// queue / dynamic array, the same keys with equal values for an
    /// associative array.
    pub(super) fn coll_contents_cmp(
        &mut self,
        a: &str,
        b: &str,
        mode: super::collection_methods::PatternCmpMode,
    ) -> PatCmp {
        let a_ok = self.ensure_struct_member_coll(a);
        let b_ok = self.ensure_struct_member_coll(b);
        if !(a_ok && b_ok) {
            return PatCmp::Equal;
        }
        let pairs: Vec<(Option<Value>, Option<Value>)> = if self.is_associative_array(a) {
            let mut ka = self.assoc_key_strs(a);
            let mut kb = self.assoc_key_strs(b);
            ka.sort();
            kb.sort();
            if ka != kb {
                return PatCmp::Unequal;
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
                return PatCmp::Unequal;
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
        let mut acc = PatCmp::Equal;
        for (x, y) in pairs {
            acc = acc.and(match (x, y) {
                (Some(x), Some(y)) => Self::coll_elem_cmp(&x, &y, None, mode),
                (None, None) => PatCmp::Equal,
                _ => PatCmp::Unequal,
            });
        }
        acc
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

    /// §7.10.2.4, §7.10.2.5: `q.pop_front()` / `q.pop_back()` on a queue whose
    /// elements are fixed-size arrays (`int q[$][2]`, `pair_t q[$]`) returns
    /// the removed row. A row has no packed cell of its own, only its
    /// elements, so it travels as a call returning an unpacked array does:
    /// copied in declared order to the return store, which the caller reads
    /// by name (`%p`, a whole-array assignment, an unpacked concatenation).
    /// An empty queue returns a row of the element type's default (§7.10.2).
    /// The call is evaluated once. `None`, with nothing evaluated, when `e`
    /// is no such pop.
    pub(super) fn pop_row_operand(&mut self, e: &Expression) -> Option<String> {
        const RET_SNAP: &str = "__xz_ret_coll__";
        let (obj, pop) = self.row_queue_pop_call(e)?;
        let &(_, (jlo, jhi), w) = self.module.arrays_2d.get(&obj)?;
        let dt = self.row_queue_elem_type(&obj)?;
        let order: Vec<i64> = match self
            .unpacked_decl_dims_of(&obj)
            .and_then(|d| d.get(1).copied())
        {
            Some((l, r)) if l > r => (r..=l).rev().collect(),
            _ => (jlo..=jhi).collect(),
        };
        let size = self.get_queue_size(&obj);
        let idx = if pop == "pop_front" {
            0
        } else {
            size.saturating_sub(1)
        };
        let default =
            if super::elaborate::is_type_two_state_resolved(&dt, &self.module.typedef_types) {
                Value::zero(w)
            } else {
                Value::new(w)
            };
        let row: Vec<Value> = order
            .iter()
            .map(|j| {
                (size > 0)
                    .then(|| self.get_signal_value_by_name(&format!("{obj}[{idx}][{j}]")))
                    .flatten()
                    .unwrap_or_else(|| default.clone())
            })
            .collect();
        self.eval_builtin_method(&obj, &pop, &[]);
        for (pos, v) in row.iter().enumerate() {
            self.set_signal_value_by_name(&format!("{RET_SNAP}[{pos}]"), v.clone());
        }
        self.set_queue_size(RET_SNAP, row.len() as u64);
        self.module.arrays.insert(RET_SNAP.to_string(), (0, -1, w));
        self.module.dynamic_arrays.insert(RET_SNAP.to_string());
        self.module.var_decl_types.insert(RET_SNAP.to_string(), dt);
        Some(RET_SNAP.to_string())
    }

    /// §7.6, §7.4.6: copy the row in the return store `src` (zero-based) to a
    /// part of an unpacked array that `assign_pending_ret_collection` does not
    /// take: a slice (`d[1:2]`) or a row of a fixed multi-dimensional array
    /// (`m[1]`), pairing elements left to right. False, with nothing
    /// written, when `lvalue` is neither or the element counts differ.
    pub(super) fn copy_ret_row_to_part(&mut self, lvalue: &Expression, src: &str) -> bool {
        // The selects are read once, before any element is written: a write
        // may change what an index reads (`m[m[0][0]] = q.pop_front()`).
        let lvalue = &self.freeze_index_chain(lvalue);
        let targets: Vec<Expression> = match &lvalue.kind {
            ExprKind::RangeSelect {
                expr,
                kind: crate::ast::expr::RangeKind::Constant,
                left,
                right,
            } if self.fixed_array_operand(expr).is_some() => {
                let (l, r) = (self.eval_expr(left), self.eval_expr(right));
                let (Some(l), Some(r)) = (
                    l.to_i64().filter(|_| !l.has_xz()),
                    r.to_i64().filter(|_| !r.has_xz()),
                ) else {
                    return false;
                };
                let idx: Vec<i64> = if l <= r {
                    (l..=r).collect()
                } else {
                    (r..=l).rev().collect()
                };
                idx.into_iter()
                    .map(|i| {
                        Expression::new(
                            ExprKind::Index {
                                expr: expr.clone(),
                                index: Box::new(
                                    self.value_method_arg_expr(&Self::signed_loop_val(i)),
                                ),
                            },
                            lvalue.span,
                        )
                    })
                    .collect()
            }
            ExprKind::Index { .. } => match self.unpacked_row_suffixes(lvalue) {
                Some(sfx) => sfx
                    .iter()
                    .map(|s| Self::append_leaf_path(lvalue, s))
                    .collect(),
                None => return false,
            },
            _ => return false,
        };
        if targets.len() as u64 != self.get_queue_size(src) {
            return false;
        }
        for (k, t) in targets.iter().enumerate() {
            let v = self
                .get_signal_value_by_name(&format!("{src}[{k}]"))
                .unwrap_or_else(|| Value::zero(32));
            self.assign_value(t, &v);
        }
        true
    }

    /// `e` with the index of each element select along its chain, and the
    /// base of a slice, replaced by a literal of its value now, so writes
    /// through it select the same elements however they change what an
    /// index reads.
    fn freeze_index_chain(&mut self, e: &Expression) -> Expression {
        let kind = match &e.kind {
            ExprKind::Index { expr, index } => ExprKind::Index {
                expr: Box::new(self.freeze_index_chain(expr)),
                index: Box::new(self.capture_index_literal(index, expr)),
            },
            ExprKind::RangeSelect {
                expr,
                kind,
                left,
                right,
            } => ExprKind::RangeSelect {
                expr: Box::new(self.freeze_index_chain(expr)),
                kind: *kind,
                left: left.clone(),
                right: right.clone(),
            },
            _ => return e.clone(),
        };
        Expression::new(kind, e.span)
    }

    /// Whether `e` is a pop that [`Self::pop_row_operand`] returns a row for.
    /// Nothing is evaluated.
    pub(super) fn is_pop_row_call(&mut self, e: &Expression) -> bool {
        self.row_queue_pop_call(e)
            .is_some_and(|(obj, _)| self.module.arrays_2d.contains_key(&obj))
    }

    /// `queue_pop_call`, or a pop of an object's queue property
    /// (`k.q.pop_front()`, `q.pop_back()` in a method of the class), whose
    /// store is `<handle>#q`. Nothing is evaluated but the receiver's handle.
    fn row_queue_pop_call(&mut self, e: &Expression) -> Option<(String, String)> {
        if let Some(found) = self.queue_pop_call(e) {
            return Some(found);
        }
        let ExprKind::Call { func, args } = &e.kind else {
            return None;
        };
        if !args.is_empty() {
            return None;
        }
        let mname = match &func.kind {
            ExprKind::MemberAccess { member, .. } => member.name.as_str(),
            ExprKind::Ident(h) if h.path.len() >= 2 => h.path.last()?.name.name.as_str(),
            _ => return None,
        };
        if mname != "pop_front" && mname != "pop_back" {
            return None;
        }
        let mname = mname.to_string();
        let recv = match &func.kind {
            ExprKind::MemberAccess { expr, .. } => (**expr).clone(),
            ExprKind::Ident(h) => {
                let mut recv = h.clone();
                recv.path.pop();
                recv.cached_signal_id = crate::ast::cache::SignalCache::new(None);
                recv.cached_resolved_name = crate::ast::cache::NameCache::new();
                recv.bound = crate::ast::binding::Bound::UNSET;
                Expression::new(ExprKind::Ident(recv), func.span)
            }
            _ => return None,
        };
        let store = self.expr_assoc_name(&recv)?;
        self.module
            .dynamic_arrays
            .contains(&store)
            .then_some((store, mname))
    }

    /// The declared element type of the queue of rows stored as `obj`: a
    /// design variable's, or an object's property's (`<handle>#q`).
    fn row_queue_elem_type(&self, obj: &str) -> Option<DataType> {
        if let Some(dt) = self.module.var_decl_types.get(obj) {
            return Some(dt.clone());
        }
        let (h, prop) = obj.split_once('#')?;
        let cd = self.class_prop_declarer(h.parse().ok()?, prop)?;
        cd.property_types.get(prop).cloned()
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
            return Some((obj, mname));
        }
        // A subroutine-local queue named in the two-segment form is stored
        // under its per-call key (`@q#3`), which the pop itself resolves to.
        let local = self.dyn_name_lookup(&obj)?;
        self.module
            .dynamic_arrays
            .contains(local)
            .then(|| (local.to_string(), mname))
    }

    /// Whether `name` is a row of a declared collection whose row kind is a
    /// queue or dynamic array (`qq[k]` of `int qq[$][$]`), not a fixed or
    /// associative row.
    fn is_queue_or_dyn_row(&self, name: &str) -> bool {
        self.coll_shape_at(name).is_some_and(|(shape, depth)| {
            depth >= 1 && matches!(shape.dims.get(depth), Some(Dim::Queue | Dim::Dyn))
        })
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
            let values = match &self.unpacked_pattern(arg).kind {
                ExprKind::AssignmentPattern(items) => {
                    Some(items.iter().map(|it| self.eval_expr(it.expr())).collect())
                }
                // §10.10: an array operand contributes its elements in order.
                ExprKind::Concatenation(parts) => {
                    let expand_rows = self.concat_has_non_array_elements(elem);
                    Some(self.unpacked_concat_operand_values(
                        parts.iter().collect(),
                        expand_rows,
                        None,
                    ))
                }
                _ => None,
            };
            if let Some(values) = values {
                let mut values = values.into_iter();
                for j in dst {
                    let v = values.next().unwrap_or_else(|| Value::zero(32));
                    self.set_signal_value_by_name(&format!("{elem}[{j}]"), v);
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
        // §7.10.2.2, §10.10: a concatenation (`{1, 2}`) or pattern
        // (`'{3, 4, 5}`) pushed into a queue whose rows are queues or
        // dynamic arrays is an unpacked-array value of the row type: each
        // item is one element, and a collection operand contributes all of
        // its elements. Evaluated as one packed value it left the row empty.
        let pattern = self.unpacked_pattern(arg);
        if matches!(
            pattern.kind,
            ExprKind::AssignmentPattern(_) | ExprKind::Concatenation(_)
        ) && self.is_queue_or_dyn_row(elem)
        {
            let vals = match &pattern.kind {
                // §10.9.1: a replicated item (`'{3{7}}`) is that many items.
                ExprKind::AssignmentPattern(items) => {
                    let items = self.pattern_ordered(items);
                    Some(items.iter().map(|e| self.eval_expr(e)).collect::<Vec<_>>())
                }
                ExprKind::Concatenation(parts) => {
                    let expand_rows = self.concat_has_non_array_elements(elem);
                    Some(self.unpacked_concat_operand_values(
                        parts.iter().collect(),
                        expand_rows,
                        None,
                    ))
                }
                _ => None,
            };
            if let Some(vals) = vals {
                let n = vals.len();
                for (j, v) in vals.into_iter().enumerate() {
                    let v = self.fit_queue_push_elem(elem, v);
                    self.set_signal_value_by_name(&format!("{}[{}]", elem, j), v);
                }
                self.set_queue_size(elem, n as u64);
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
                if let ExprKind::AssignmentPattern(items) = &self.unpacked_pattern(arg).kind {
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
        // §10.8, §10.9, §7.10.2: an assignment pattern pushed into a queue of
        // packed elements takes the element type, so `q.push_back('{k, v})`
        // of a packed struct sizes each item to its member; concatenated at
        // 32 bits and cut to the element, every member but the last read 0.
        if crate::compiler::packed_assignment_pattern_plan::has_pattern(arg)
            && let Some(dt) = self.coll_elem_decl_type(obj_name)
            && matches!(self.resolve_dt_ref(&dt), DataType::Struct(su) if su.packed)
            && let Some(v) = self.eval_in_packed_type(arg, &dt)
        {
            let v = self.fit_queue_push_elem(obj_name, v);
            self.set_signal_value_by_name(elem, v);
            return;
        }
        let mut val = self.queue_eval_arg(obj_name, arg);
        // §5.7.1: an unsized fill (`q.push_back('1)`) takes the element's
        // width, not one bit.
        // A real element converts it at that one bit (§6.12.1), not at the
        // element's 64-bit storage width.
        if val.is_fill && !val.is_real {
            if self.coll_elem_is_real(obj_name) {
                val = Value::from_f64(val.to_f64());
            } else if let Some(&(_, _, w)) = self.module.arrays.get(obj_name) {
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
        Self::fit_queue_elem_to(self.integral_queue_elem(coll), val)
    }

    /// `fit_queue_push_elem` for a compiled element store (`StoreCollElem`).
    /// The element type comes from a small cache of the last collections
    /// stored to, so a store loop resolves it once per collection instead of
    /// on every store (the lookup walks the object's class chain several
    /// times). A class member's entry also keys on the object's class, and an
    /// object with type-parameter bindings or a specialization is never
    /// cached, so a reused handle cannot see another object's element type.
    pub(super) fn fit_coll_store_elem(&mut self, coll: &str, val: Value) -> Value {
        let inst = coll
            .split_once('#')
            .and_then(|(h, _)| h.parse::<usize>().ok())
            .and_then(|h| self.heap.get(h)?.as_deref());
        let cacheable = inst.is_none_or(|i| i.type_bindings.is_empty() && i.spec.is_none());
        let class = inst.map(|i| i.class_name.as_str());
        let hit = cacheable
            .then(|| {
                self.coll_store_fit_cache
                    .iter()
                    .flatten()
                    .find(|(name, cls, _)| name == coll && cls.as_deref() == class)
                    .map(|&(_, _, info)| info)
            })
            .flatten();
        let info = match hit {
            Some(info) => info,
            None => {
                let info = self.integral_queue_elem(coll);
                if cacheable {
                    let slot = self.coll_store_fit_next % self.coll_store_fit_cache.len();
                    self.coll_store_fit_next = slot + 1;
                    self.coll_store_fit_cache[slot] =
                        Some((coll.to_string(), class.map(str::to_string), info));
                }
                info
            }
        };
        Self::fit_queue_elem_to(info, val)
    }

    /// Fit `val` to the integral element type `info` (`integral_queue_elem`).
    fn fit_queue_elem_to(info: Option<(u32, bool, bool)>, val: Value) -> Value {
        let Some((w, signed, two_state)) = info else {
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

    /// Whether a member's declared type matches a `type:value` key (§10.9.2,
    /// §6.22.1).
    pub(super) fn pattern_type_matches(&mut self, member_dt: &DataType, key_dt: &DataType) -> bool {
        crate::compiler::packed_assignment_pattern_plan::types_match(member_dt, key_dt, self)
    }

    fn pattern_subarray_matches(
        &mut self,
        elem_dt: &DataType,
        rest: &[Vec<i64>],
        key_dt: &DataType,
    ) -> bool {
        let DataType::TypeReference {
            name, dimensions, ..
        } = key_dt
        else {
            return false;
        };
        if !dimensions.is_empty() {
            return false;
        }
        let key_str = if name.has_scope() {
            name.qualified()
        } else if let Some((scoped, _)) = self.lookup_hint_scoped_typedef(&name.name.name) {
            scoped
        } else {
            name.name.name.clone()
        };
        let Some(unpacked) = self.typedef_unpacked_dims_of(&key_str) else {
            return false;
        };
        if unpacked.len() != rest.len() {
            return false;
        }
        let Some(key_lists) = self.member_dim_indices(&unpacked) else {
            return false;
        };
        if key_lists != rest {
            return false;
        }
        let Some(target) = self.lookup_typedef_target(&key_str) else {
            return false;
        };
        let leaf_elem = self.leaf_elem_type(elem_dt);
        self.pattern_type_matches(&leaf_elem, &target)
    }

    fn is_value_name_in_scope(&self, name: &str) -> bool {
        if self.frame_scalar_shadows(name)
            || self.module.parameters.contains_key(name)
            || self.signals.contains_key(name)
            || self.signal_name_to_id.contains_key(name)
        {
            return true;
        }
        let hint = self.name_resolve_hint.borrow();
        let Some(mut scope) = hint.as_deref() else {
            return false;
        };
        loop {
            let key = format!("{scope}.{name}");
            if self.module.parameters.contains_key(&key)
                || self.signals.contains_key(&key)
                || self.signal_name_to_id.contains_key(&key)
            {
                return true;
            }
            let Some((outer, _)) = scope.rsplit_once('.') else {
                return false;
            };
            scope = outer;
        }
    }

    pub(super) fn lookup_hint_scoped_typedef(&self, name: &str) -> Option<(String, DataType)> {
        let hint = self.name_resolve_hint.borrow();
        let mut scope = hint.as_deref()?;
        loop {
            let key = format!("{scope}.{name}");
            if let Some(dt) = self.module.typedef_types.get(&key) {
                return Some((key, dt.clone()));
            }
            let Some((outer, _)) = scope.rsplit_once('.') else {
                return None;
            };
            scope = outer;
        }
    }

    pub(super) fn resolve_type_binding_dt(&self, bound: &str) -> Option<DataType> {
        let b = bound.trim();
        if b.is_empty() {
            return None;
        }
        let span = crate::ast::Span::dummy();
        let builtin = match b {
            "bit" => Some(DataType::IntegerVector {
                kind: crate::ast::types::IntegerVectorType::Bit,
                signing: None,
                dimensions: Vec::new(),
                span,
            }),
            "logic" => Some(DataType::IntegerVector {
                kind: crate::ast::types::IntegerVectorType::Logic,
                signing: None,
                dimensions: Vec::new(),
                span,
            }),
            "reg" => Some(DataType::IntegerVector {
                kind: crate::ast::types::IntegerVectorType::Reg,
                signing: None,
                dimensions: Vec::new(),
                span,
            }),
            "byte" => Some(DataType::IntegerAtom {
                kind: crate::ast::types::IntegerAtomType::Byte,
                signing: None,
                span,
            }),
            "shortint" => Some(DataType::IntegerAtom {
                kind: crate::ast::types::IntegerAtomType::ShortInt,
                signing: None,
                span,
            }),
            "int" => Some(DataType::IntegerAtom {
                kind: crate::ast::types::IntegerAtomType::Int,
                signing: None,
                span,
            }),
            "longint" => Some(DataType::IntegerAtom {
                kind: crate::ast::types::IntegerAtomType::LongInt,
                signing: None,
                span,
            }),
            "integer" => Some(DataType::IntegerAtom {
                kind: crate::ast::types::IntegerAtomType::Integer,
                signing: None,
                span,
            }),
            "time" => Some(DataType::IntegerAtom {
                kind: crate::ast::types::IntegerAtomType::Time,
                signing: None,
                span,
            }),
            "real" => Some(DataType::Real {
                kind: crate::ast::types::RealType::Real,
                span,
            }),
            "shortreal" => Some(DataType::Real {
                kind: crate::ast::types::RealType::ShortReal,
                span,
            }),
            "realtime" => Some(DataType::Real {
                kind: crate::ast::types::RealType::RealTime,
                span,
            }),
            "string" => Some(DataType::Simple {
                kind: crate::ast::types::SimpleType::String,
                span,
            }),
            _ => None,
        };
        if builtin.is_some() {
            return builtin;
        }
        if let Some((_, dt)) = self.lookup_hint_scoped_typedef(b) {
            return Some(dt);
        }
        if let Some(dt) = self.lookup_typedef_target(b) {
            return Some(dt);
        }
        let src = format!("typedef {b} __xezim_tp_t;");
        let tokens = crate::lexer::Lexer::new(&src).tokenize();
        let mut parser = sv_parser::parse::Parser::new(tokens);
        let cu = parser.parse_source_text();
        if parser
            .diagnostics()
            .iter()
            .any(|d| d.severity == sv_parser::diagnostics::Severity::Error)
        {
            return None;
        }
        for desc in cu.descriptions {
            if let crate::ast::Description::TypedefDecl(td) = desc {
                return Some(td.data_type);
            }
        }
        None
    }

    pub(super) fn resolve_type_key_name(&self, name: &str) -> Option<DataType> {
        if self.is_value_name_in_scope(name) {
            return None;
        }
        if name.contains("::") {
            if self.module.parameters.contains_key(name) {
                return None;
            }
            if let Some(bare) = crate::sv_parser::strip_unit_scope_name(name) {
                if self.module.typedef_types.contains_key(name) {
                    return Some(crate::compiler::packed_assignment_pattern_plan::type_ref(
                        name,
                    ));
                }
                if !self.is_value_name_in_scope(bare)
                    && self.module.typedef_types.contains_key(bare)
                {
                    return Some(crate::compiler::packed_assignment_pattern_plan::type_ref(
                        bare,
                    ));
                }
                return None;
            }
            return self.module.typedef_types.contains_key(name).then(|| {
                crate::compiler::packed_assignment_pattern_plan::scoped_type_ref(name)
            });
        }
        if let Some((scoped_key, _)) = self.lookup_hint_scoped_typedef(name) {
            return Some(crate::compiler::packed_assignment_pattern_plan::type_ref(
                &scoped_key,
            ));
        }
        if let Some(bound) = self.resolve_type_param_binding(name) {
            return self.resolve_type_binding_dt(&bound);
        }
        self.lookup_typedef_target(name)
            .map(|_| crate::compiler::packed_assignment_pattern_plan::type_ref(name))
    }

    /// Resolve a `Class#(args)::td`, `pkg::Class#(args)::td`,
    /// `Class#(args)::Inner::td`, or `alias::td` (where `alias` is a typedef of
    /// a class specialization) expression to its concrete `DataType` (§8.25.1).
    pub(super) fn resolve_specialized_class_typedef_expr(
        &self,
        expr: &Expression,
    ) -> Option<DataType> {
        let resolve_typedef_alias_spec = |head_name: &str| -> Option<(String, String)> {
            let mut alias_name = head_name.to_string();
            for _ in 0..16 {
                let DataType::TypeReference {
                    name: tn,
                    type_args,
                    dimensions,
                    ..
                } = self.lookup_typedef_target(&alias_name)?
                else {
                    return None;
                };
                if !dimensions.is_empty() {
                    return None;
                }
                let target_cls = if tn.has_scope() {
                    tn.qualified()
                } else {
                    tn.name.name.clone()
                };
                if !type_args.is_empty() && self.module.classes.contains_key(&target_cls) {
                    let frags: Vec<String> = type_args
                        .iter()
                        .filter_map(|a| self.expr_to_spec_fragment(a))
                        .collect();
                    return Some((target_cls, frags.join(",")));
                }
                if tn.scopes.is_empty() && type_args.is_empty() {
                    alias_name = tn.name.name;
                    continue;
                }
                return None;
            }
            None
        };

        let (leaf, mut tail, spec_hit): (&str, Vec<&str>, Option<(String, String)>) =
            match &expr.kind {
                ExprKind::MemberAccess {
                    expr: parent,
                    member: leaf_id,
                } => {
                    let mut tail: Vec<&str> = Vec::new();
                    let mut cur = parent.as_ref();
                    let mut spec_hit: Option<(String, String)> = None;
                    for _ in 0..16 {
                        match &cur.kind {
                            ExprKind::Specialization {
                                base,
                                type_args_text,
                            } => {
                                let base_name = match &base.kind {
                                    ExprKind::Ident(h)
                                        if h.root.is_none()
                                            && h.path.len() == 1
                                            && h.path[0].selects.is_empty() =>
                                    {
                                        h.path[0].name.name.clone()
                                    }
                                    _ => crate::compiler::packed_assignment_pattern_plan::scoped_name_key(
                                        base,
                                    )?,
                                };
                                spec_hit = Some((base_name, type_args_text.clone()));
                                break;
                            }
                            ExprKind::MemberAccess { expr: inner, member } => {
                                tail.push(member.name.as_str());
                                cur = inner.as_ref();
                            }
                            ExprKind::Ident(h)
                                if h.root.is_none()
                                    && h.path.len() == 1
                                    && h.path[0].selects.is_empty() =>
                            {
                                spec_hit = resolve_typedef_alias_spec(&h.path[0].name.name);
                                break;
                            }
                            _ => break,
                        }
                    }
                    (leaf_id.name.as_str(), tail, spec_hit)
                }
                ExprKind::Ident(h)
                    if h.root.is_none()
                        && h.path.len() >= 2
                        && h.path.iter().all(|s| s.selects.is_empty()) =>
                {
                    let leaf = h.path.last()?.name.name.as_str();
                    let head = h.path[0].name.name.as_str();
                    let tail: Vec<&str> = h.path[1..h.path.len() - 1]
                        .iter()
                        .rev()
                        .map(|s| s.name.name.as_str())
                        .collect();
                    (leaf, tail, resolve_typedef_alias_spec(head))
                }
                _ => return None,
            };
        let (mut cls_name, raw_sig) = spec_hit?;
        if !tail.is_empty() {
            tail.reverse();
            for seg in tail {
                cls_name.push_str("::");
                cls_name.push_str(seg);
            }
        }
        let orig_cls_name = cls_name.clone();
        if !self.module.classes.contains_key(&cls_name)
            && let Some((_, rest)) = cls_name.split_once("::")
            && self.module.classes.contains_key(rest)
        {
            cls_name = rest.to_string();
        }
        if !self.module.classes.contains_key(&cls_name) {
            return None;
        }
        let canon_sig = self.canonicalize_spec_sig(&cls_name, &raw_sig);

        // Walk `cls_name` and its `extends` chain to find `leaf` in `typedef_targets`.
        let mut cur_cls = cls_name.clone();
        for _ in 0..32 {
            let cd = self.module.classes.get(&cur_cls)?;
            let bare_cls = cur_cls.rsplit_once("::").map_or(cur_cls.as_str(), |(_, r)| r);
            let target = cd
                .typedef_targets
                .get(leaf)
                .or_else(|| cd.typedef_targets.get(&format!("{cur_cls}::{leaf}")))
                .or_else(|| {
                    (bare_cls != cur_cls)
                        .then(|| cd.typedef_targets.get(&format!("{bare_cls}::{leaf}")))
                        .flatten()
                })
                .or_else(|| {
                    (cur_cls == cls_name && orig_cls_name != cls_name)
                        .then(|| cd.typedef_targets.get(&format!("{orig_cls_name}::{leaf}")))
                        .flatten()
                });
            if let Some(target) = target {
                let cur_sig = self
                    .ancestor_spec(&cls_name, &canon_sig, &cur_cls)
                    .unwrap_or_else(|| canon_sig.clone());
                let spec_args = Self::split_spec_args(&cur_sig);

                // §8.25: `typedef T cw_t;` where `T` is a class type parameter.
                if let DataType::TypeReference {
                    name: tn,
                    dimensions,
                    type_args,
                    ..
                } = target
                    && tn.scopes.is_empty()
                    && type_args.is_empty()
                    && cd.type_param_names.iter().any(|p| p == &tn.name.name)
                    && let Some(pos) = cd.param_order.iter().position(|p| p == &tn.name.name)
                    && let Some(arg) = spec_args.get(pos)
                    && let Some(mut bound_dt) = self.resolve_type_binding_dt(arg)
                {
                    if !dimensions.is_empty() {
                        match &mut bound_dt {
                            DataType::IntegerVector {
                                dimensions: bd, ..
                            }
                            | DataType::Implicit { dimensions: bd, .. }
                            | DataType::TypeReference { dimensions: bd, .. } => {
                                let mut merged = dimensions.clone();
                                merged.extend(bd.drain(..));
                                *bd = merged;
                            }
                            _ => {}
                        }
                    }
                    return Some(bound_dt);
                }

                // §8.25: `typedef logic [W-1:0] vec_t;` where packed dims
                // reference a value parameter of the specialized class.
                let has_value_params = cd
                    .param_order
                    .iter()
                    .any(|p| !cd.type_param_names.iter().any(|tp| tp == p));
                if has_value_params && Self::data_type_has_packed_dims(target) {
                    let mut val_params: HashMap<String, Value> = HashMap::default();
                    for (pn, init) in &cd.param_defaults {
                        if let Some(init) = init
                            && let Some(v) = super::elaborate::const_eval_i64_with_params(
                                init,
                                Some(&self.module.parameters),
                            )
                        {
                            let mut val = Value::from_u64(v as u64, 32);
                            val.is_signed = true;
                            val_params.insert(pn.clone(), val);
                        }
                    }
                    for (i, arg) in spec_args.iter().enumerate() {
                        if let Some(pn) = cd.param_order.get(i)
                            && !cd.type_param_names.iter().any(|tp| tp == pn)
                            && let Ok(v) = arg.trim().parse::<i64>()
                        {
                            let mut val = Value::from_u64(v as u64, 32);
                            val.is_signed = true;
                            val_params.insert(pn.clone(), val);
                        }
                    }
                    if !val_params.is_empty() {
                        return Some(Self::fold_packed_dims_with_params(target, &val_params));
                    }
                }
                return Some(target.clone());
            }
            let Some(next) = cd.extends.as_ref() else {
                break;
            };
            cur_cls = next.clone();
        }
        None
    }

    fn data_type_has_packed_dims(dt: &DataType) -> bool {
        match dt {
            DataType::IntegerVector { dimensions, .. }
            | DataType::Implicit { dimensions, .. }
            | DataType::TypeReference { dimensions, .. } => !dimensions.is_empty(),
            _ => false,
        }
    }

    fn fold_packed_dims_with_params(
        dt: &DataType,
        params: &HashMap<String, Value>,
    ) -> DataType {
        let fold_dims = |dims: &[crate::ast::types::PackedDimension]| {
            dims.iter()
                .map(|d| match d {
                    crate::ast::types::PackedDimension::Range { left, right, span } => {
                        let l = super::elaborate::const_eval_i64_with_params(left, Some(params))
                            .map(|v| {
                                Box::new(Expression::new(
                                    ExprKind::Number(crate::ast::expr::NumberLiteral::Integer {
                                        size: None,
                                        signed: true,
                                        base: crate::ast::expr::NumberBase::Decimal,
                                        value: v.to_string(),
                                        cached_val: Default::default(),
                                    }),
                                    left.span,
                                ))
                            })
                            .unwrap_or_else(|| left.clone());
                        let r = super::elaborate::const_eval_i64_with_params(right, Some(params))
                            .map(|v| {
                                Box::new(Expression::new(
                                    ExprKind::Number(crate::ast::expr::NumberLiteral::Integer {
                                        size: None,
                                        signed: true,
                                        base: crate::ast::expr::NumberBase::Decimal,
                                        value: v.to_string(),
                                        cached_val: Default::default(),
                                    }),
                                    right.span,
                                ))
                            })
                            .unwrap_or_else(|| right.clone());
                        crate::ast::types::PackedDimension::Range {
                            left: l,
                            right: r,
                            span: *span,
                        }
                    }
                    other => other.clone(),
                })
                .collect()
        };
        let mut out = dt.clone();
        match &mut out {
            DataType::IntegerVector { dimensions, .. }
            | DataType::Implicit { dimensions, .. }
            | DataType::TypeReference { dimensions, .. } => {
                *dimensions = fold_dims(dimensions);
            }
            _ => {}
        }
        out
    }

    pub(super) fn array_pattern_type_key<'a>(
        &self,
        item: &'a AssignmentPatternItem,
    ) -> Option<(DataType, &'a Expression)> {
        match item {
            AssignmentPatternItem::Typed(dt, e) => Some((dt.clone(), e)),
            AssignmentPatternItem::Named(id, e) => {
                if self.is_value_name_in_scope(&id.name) {
                    return None;
                }
                if let Some(tk) = id.type_key.as_deref() {
                    return self.resolve_type_key_name(tk).map(|dt| (dt, e));
                }
                self.resolve_type_key_name(&id.name).map(|dt| (dt, e))
            }
            AssignmentPatternItem::Keyed(k, e) => {
                if !matches!(k.kind, ExprKind::Ident(_) | ExprKind::MemberAccess { .. }) {
                    return None;
                }
                if let ExprKind::Ident(h) = &k.kind
                    && h.path.len() == 1
                    && h.root.is_none()
                {
                    let n = h.path[0].name.name.as_str();
                    return self.resolve_type_key_name(n).map(|dt| (dt, e));
                }
                let root = Self::expr_root_first_seg(k);
                if root.is_some_and(|r| {
                    self.is_value_name_in_scope(r) || self.module.var_decl_types.contains_key(r)
                }) {
                    return None;
                }
                if let Some(key) = crate::compiler::packed_assignment_pattern_plan::scoped_name_key(k)
                    && let Some(dt) = self.resolve_type_key_name(&key)
                {
                    return Some((dt, e));
                }
                self.resolve_specialized_class_typedef_expr(k)
                    .map(|dt| (dt, e))
            }
            _ => None,
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
        self.pattern_elems_inner(items, indices, None)
    }

    /// Like [`Self::pattern_elems`], also matching `type:expr` keys against
    /// `elem_dt` before falling back to `default:expr` (§10.9.1).
    pub(super) fn pattern_elems_for_type<'a>(
        &mut self,
        items: &'a [AssignmentPatternItem],
        indices: &[i64],
        elem_dt: &DataType,
    ) -> Vec<Option<&'a Expression>> {
        self.pattern_elems_inner(items, indices, Some(elem_dt))
    }

    fn pattern_elems_inner<'a>(
        &mut self,
        items: &'a [AssignmentPatternItem],
        indices: &[i64],
        elem_dt: Option<&DataType>,
    ) -> Vec<Option<&'a Expression>> {
        let n = indices.len();
        let mut out: Vec<Option<&Expression>> = vec![None; n];
        let mut typed: Option<&Expression> = None;
        let mut default: Option<&Expression> = None;
        let mut pos = 0usize;
        for item in items {
            match item {
                AssignmentPatternItem::Default(e) => default = Some(e),
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
                AssignmentPatternItem::Typed(key_dt, e) => {
                    if let Some(edt) = elem_dt
                        && self.pattern_type_matches(edt, key_dt)
                    {
                        typed = Some(e);
                    }
                }
                AssignmentPatternItem::Keyed(k, e) => {
                    if matches!(k.kind, ExprKind::Ident(_) | ExprKind::MemberAccess { .. })
                        && let Some((key_dt, _)) = self.array_pattern_type_key(item)
                    {
                        if let Some(edt) = elem_dt
                            && self.pattern_type_matches(edt, &key_dt)
                        {
                            typed = Some(e);
                        }
                        continue;
                    }
                    let ki = self.eval_expr(k).to_i64().unwrap_or(0);
                    if let Some(p) = indices.iter().position(|&i| i == ki) {
                        out[p] = Some(e);
                    }
                }
                AssignmentPatternItem::Named(id, e) => {
                    if let Some((key_dt, _)) = self.array_pattern_type_key(item) {
                        if let Some(edt) = elem_dt
                            && self.pattern_type_matches(edt, &key_dt)
                        {
                            typed = Some(e);
                        }
                        continue;
                    }
                    let key_expr = crate::compiler::packed_assignment_pattern_plan::ident_expr(id);
                    let ki = self.eval_expr(&key_expr).to_i64().unwrap_or(0);
                    if let Some(p) = indices.iter().position(|&i| i == ki) {
                        out[p] = Some(e);
                    }
                }
            }
        }
        let fallback = typed.or(default);
        for slot in out.iter_mut() {
            if slot.is_none() {
                *slot = fallback;
            }
        }
        out
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
        let lists: Vec<Vec<i64>> = dims
            .iter()
            .enumerate()
            .map(|(k, &(lo, hi))| {
                if descending.get(k).copied().unwrap_or(false) {
                    (lo..=hi).rev().collect()
                } else {
                    (lo..=hi).collect()
                }
            })
            .collect();
        self.assign_pattern_items_over_dims(base, dt, &lists, items);
    }

    /// §10.9.1 — the index expression of a keyed array-pattern item.
    ///
    /// `'{ K : v }` parses as a struct member designator (`Named`) whenever
    /// the key is a bare identifier, because a member name and an index are
    /// spelled the same (§10.9.1, §10.9.2). On an ARRAY target the
    /// identifier is a constant or variable used as an index, which is how
    /// `elaborate::array_params` already reads it for parameters. Reading it
    /// as a member name here dropped the item instead, so
    /// `'{ PM_PAGE : '{1,1} }` left an associative array empty. A name that
    /// denotes a type is a type key (`'{ byte_t : 3 }`), not an index, and is
    /// not read here.
    fn array_pattern_key(&self, item: &AssignmentPatternItem) -> Option<Expression> {
        if self.array_pattern_type_key(item).is_some() {
            return None;
        }
        match item {
            AssignmentPatternItem::Keyed(k, _) => Some(k.clone()),
            AssignmentPatternItem::Named(id, _) => {
                Some(crate::compiler::packed_assignment_pattern_plan::ident_expr(id))
            }
            _ => None,
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

    /// Convert one pattern item to the element type: reals round (§6.12.2),
    /// everything else fits by ordinary assignment resize.
    pub(super) fn fit_pattern_item_value(&self, v: Value, elem_w: u32) -> Value {
        if v.is_real {
            Self::real_to_int(v.to_f64(), elem_w)
        } else {
            v.resize_for_assign(elem_w)
        }
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

    /// §10.9.2, §10.10: spread a pattern over an aggregate whose leaves are
    /// separate signals (an unpacked struct, an unpacked array of them, an
    /// associative array). False when `base` is one signal.
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
                if let AssignmentPatternItem::Default(e) = item {
                    self.module
                        .assoc_defaults
                        .insert(base.to_string(), e.clone());
                    wrote = true;
                } else if let Some(k) = self.array_pattern_key(item) {
                    let kv = self.eval_expr(&k);
                    let key = self.assoc_key_str(base, &kv);
                    let elem = format!("{}[{}]", base, key);
                    if !self.assign_coll_row_item(base, &elem, &dt, item.expr()) {
                        self.assign_pattern_or_leaf(&elem, &dt, item.expr());
                    }
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
                let elem = format!("{}[{}]", base, i);
                if !self.assign_coll_row_item(base, &elem, &dt, e) {
                    self.assign_pattern_or_leaf(&elem, &dt, e);
                }
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
        // §10.9: a declared packed variable takes a pattern as one value.
        if !registered_fixed
            && self.module.var_decl_types.contains_key(base)
            && is_packed(&dt, self)
        {
            return false;
        }
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

        // §10.9.1: a packed multi-D vector takes a pattern as one packed
        // value, evaluated against its declared type by the caller. An
        // unpacked array of any rank whose element is packed multi-D also
        // has an inner element width recorded, so test the fixed-array
        // tables, not only the 1-D one.
        if !registered_fixed && self.module.packed_signal_elem_widths.contains_key(base) {
            return false;
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

    /// §10.9.1, §10.10, §7.6: a row value written to a ROW of an unpacked
    /// array: a row of a fixed multi-dimensional array (`c[0] = '{V, V}` on
    /// `logic [39:0] c [2][2]`, `d[1] = '{'{..}, '{..}}` on `d [2][2][2]`,
    /// `m[1] = {3, 4}` on `int m[3][2]`), or an element of a queue, dynamic
    /// array or associative array whose elements are fixed-size arrays
    /// (`q[0] = '{1, 2}` on `int q[$][2]`, `aa["k"] = p` on `pair_t
    /// aa[string]`). Each element is its own signal, so the value spreads
    /// over the dimensions left below the selected indices, as a whole-array
    /// pattern does. Without this the row was stored as one packed value under
    /// a name with no storage, and every element read x or 0. The value is an
    /// assignment pattern or an unpacked array concatenation and, for an
    /// element of a collection, also an array of the row's shape (see
    /// `assign_fixed_row`). False when the target is not such a row or an
    /// index is not a known in-range value, leaving the other paths to it.
    pub(super) fn assign_unpacked_row(&mut self, lvalue: &Expression, rvalue: &Expression) -> bool {
        let rvalue = self.unpacked_pattern(rvalue);
        let row_value = match &rvalue.kind {
            ExprKind::AssignmentPattern(items) => !items.is_empty(),
            ExprKind::Concatenation(parts) => !parts.is_empty(),
            _ => false,
        };
        let mut idx_exprs: Vec<&Expression> = Vec::new();
        let mut cur = lvalue;
        while let ExprKind::Index { expr, index } = &cur.kind {
            idx_exprs.push(index);
            cur = expr;
        }
        let ExprKind::Ident(h) = &cur.kind else {
            return false;
        };
        // §8.4, §23.9: in a method a bare name is the property of `this` (or
        // a local), not a same-named module array; the class row spread
        // owns it.
        if idx_exprs.is_empty() || self.bare_selected_storage_shadows_signal(h) {
            return false;
        }
        idx_exprs.reverse();
        let name = self.resolve_hier_name(h).into_owned();
        if self.module.dynamic_arrays.contains(&name) || self.is_associative_array(&name) {
            return self.assign_coll_fixed_row(&name, &idx_exprs, rvalue, row_value);
        }
        if !row_value {
            return false;
        }
        // Any fixed element type, unpacked structs included: the spread
        // below takes a struct element's pattern member-wise, as the
        // whole-array spread does.
        let Some(dims) = self.foreach_dims(&name) else {
            return false;
        };
        let k = idx_exprs.len();
        if k >= dims.len() {
            return false;
        }
        let mut idx: Vec<i64> = Vec::with_capacity(k);
        for (ie, &(lo, hi)) in idx_exprs.iter().zip(&dims) {
            if !Self::cond_expr_is_effect_free(ie, false) {
                return false;
            }
            let v = self.eval_expr(ie);
            match v.to_i64() {
                Some(i) if !v.has_xz() && (lo..=hi).contains(&i) => idx.push(i),
                _ => return false,
            }
        }
        let cells: i64 = dims[k..]
            .iter()
            .map(|&(lo, hi)| (hi - lo + 1).max(0))
            .product();
        if cells <= 0 || cells > (1 << 20) {
            return false;
        }
        let Some(dt) = self.module.var_decl_types.get(&name).cloned() else {
            return false;
        };
        // Each remaining dimension's direction from its declared bounds.
        let descending: Vec<bool> = match self
            .module
            .unpacked_decl_dims
            .get(&name)
            .filter(|d| d.len() == dims.len())
        {
            Some(decl) => decl[k..].iter().map(|&(l, r)| l > r).collect(),
            None => vec![false; dims.len() - k],
        };
        let base = Self::elem_name(&name, &idx);
        self.assign_fixed_row(&base, &dims[k..], &descending, &dt, rvalue)
    }

    /// §7.4.5: the dimensions of one element of the collection `name` (a
    /// queue, dynamic array or associative array) when that element is a
    /// fixed-size unpacked array (`int q[$][2]`, or `pair_t aa[string]` with
    /// `typedef int pair_t[2]`): each dimension's `(lo, hi)` bounds, outermost
    /// first, and whether it is declared descending (`[2:1]`). `None` for any
    /// other collection.
    pub(super) fn coll_fixed_row_dims(&self, name: &str) -> Option<(Vec<(i64, i64)>, Vec<bool>)> {
        let Some((shape, 0)) = self.coll_shape_at(name) else {
            return None;
        };
        let (outer, row) = shape.dims.split_first()?;
        if matches!(outer, Dim::Fixed(_)) || row.is_empty() {
            return None;
        }
        let dims: Vec<(i64, i64)> = row
            .iter()
            .map(|d| match d {
                Dim::Fixed(Some(b)) => Some(*b),
                _ => None,
            })
            .collect::<Option<_>>()?;
        let descending = match self.unpacked_decl_dims_of(name) {
            Some(d) if d.len() == shape.dims.len() => d[1..].iter().map(|&(l, r)| l > r).collect(),
            _ => vec![false; dims.len()],
        };
        Some((dims, descending))
    }

    /// §7.4.5, §10.9.1: one item of a pattern written to a whole queue,
    /// dynamic array or associative array `coll` whose elements are
    /// fixed-size arrays (`q = '{'{1, 2}, p}`) is a whole row: spread it over
    /// the row `elem` (`assign_fixed_row`). False when `coll` has no such
    /// rows or the item is no row value, for the element-wise path.
    fn assign_coll_row_item(
        &mut self,
        coll: &str,
        elem: &str,
        dt: &DataType,
        item: &Expression,
    ) -> bool {
        let Some((dims, descending)) = self.coll_fixed_row_dims(coll) else {
            return false;
        };
        let dt = self.leaf_elem_type(dt);
        self.assign_fixed_row(elem, &dims, &descending, &dt, item)
    }

    /// The index lists of each dimension of a fixed-size row, from its
    /// declared left bound to its right one (`descending` per dimension).
    fn fixed_row_lists(dims: &[(i64, i64)], descending: &[bool]) -> Vec<Vec<i64>> {
        dims.iter()
            .enumerate()
            .map(|(k, &(lo, hi))| {
                if descending.get(k).copied().unwrap_or(false) {
                    (lo..=hi).rev().collect()
                } else {
                    (lo..=hi).collect()
                }
            })
            .collect()
    }

    /// The element suffixes (`[i][j]`) of a fixed-size row in declared
    /// order, the order in which §7.6 pairs it with another array.
    pub(super) fn fixed_row_suffixes(dims: &[(i64, i64)], descending: &[bool]) -> Vec<String> {
        let mut out = vec![String::new()];
        for list in Self::fixed_row_lists(dims, descending) {
            out = out
                .iter()
                .flat_map(|p| list.iter().map(move |i| format!("{p}[{i}]")))
                .collect();
        }
        out
    }

    /// The collection half of [`assign_unpacked_row`]: `idx` selects one
    /// element of the queue, dynamic array or associative array `name`
    /// whose elements are fixed-size arrays, and possibly a row inside it.
    /// §7.10.1, §7.8.6, §7.4.6: a write with an unknown index or one past the
    /// end of a queue or dynamic array writes nothing, except `q[$+1]`,
    /// which appends to a queue.
    fn assign_coll_fixed_row(
        &mut self,
        name: &str,
        idx: &[&Expression],
        rvalue: &Expression,
        row_value: bool,
    ) -> bool {
        let Some((dims, descending)) = self.coll_fixed_row_dims(name) else {
            return false;
        };
        let k = idx.len() - 1;
        if k >= dims.len()
            || idx
                .iter()
                .any(|ie| !Self::cond_expr_is_effect_free(ie, false))
        {
            return false;
        }
        let Some(dt) = self.module.var_decl_types.get(name).cloned() else {
            return false;
        };
        let assoc = self.is_associative_array(name);
        let size = if assoc { 0 } else { self.get_queue_size(name) };
        let first = if assoc {
            self.eval_expr(idx[0])
        } else {
            self.dollar_bound.push(size as i64 - 1);
            let v = self.eval_expr(idx[0]);
            self.dollar_bound.pop();
            v
        };
        // §7.10.5: a bounded queue (`[$:N]`) that is full takes no append.
        let append = !assoc
            && matches!(self.declared_dim(name), Some(Dim::Queue))
            && self
                .module
                .queue_max_sizes
                .get(name)
                .is_none_or(|&m| size < m as u64);
        let mut base = match first.to_i64() {
            _ if first.has_xz() => None,
            _ if assoc => Some(format!("{}[{}]", name, self.assoc_key_str(name, &first))),
            Some(i) if i >= 0 && ((i as u64) < size || (append && i as u64 == size)) => {
                Some(format!("{}[{}]", name, i))
            }
            _ => None,
        };
        for (ie, &(lo, hi)) in idx[1..].iter().zip(&dims) {
            let v = self.eval_expr(ie);
            base = match (base, v.to_i64()) {
                (Some(b), Some(i)) if !v.has_xz() && (lo..=hi).contains(&i) => {
                    Some(format!("{}[{}]", b, i))
                }
                _ => None,
            };
        }
        let Some(base) = base else {
            // The write is ignored, but the right-hand side is still
            // evaluated, so its calls run (§10.4.1). Only a row value is known
            // to be one; any other right-hand side is left to the other paths.
            if row_value {
                self.eval_row_value_for_effects(rvalue);
            }
            return row_value;
        };
        let dt = self.leaf_elem_type(&dt);
        if !self.assign_fixed_row(&base, &dims[k..], &descending[k..], &dt, rvalue) {
            return false;
        }
        if assoc {
            return true;
        }
        if first.to_i64() == Some(size as i64) {
            self.set_queue_size(name, size + 1);
        }
        // Comb readers of `q[i]` depend on the collection's `.size` proxy.
        self.touch_queue(name);
        true
    }

    /// Evaluate the items of the row value `e` (a pattern or a concatenation)
    /// whose store is discarded, for their side effects only.
    fn eval_row_value_for_effects(&mut self, e: &Expression) {
        match &self.unpacked_pattern(e).kind {
            ExprKind::AssignmentPattern(items) => {
                for item in items {
                    let _ = self.eval_expr(item.expr());
                }
            }
            ExprKind::Concatenation(parts) => {
                let _ = self.unpacked_concat_operand_values(parts.iter().collect(), true, None);
            }
            _ => {}
        }
    }

    /// §7.8.6: the value an element of a nonexistent row of an associative
    /// array of fixed-size arrays reads (`row` is `aa[k]`): the element
    /// type's default, x for a 4-state integral type, 0 for a 2-state one and
    /// 0.0 for a real one. `None` when `row` is no such row.
    pub(super) fn assoc_row_missing_value(&self, row: &str) -> Option<Value> {
        let (coll, _) = row.strip_suffix(']')?.rsplit_once('[')?;
        if !self.is_associative_array(coll) {
            return None;
        }
        self.coll_fixed_row_dims(coll)?;
        let leaf = self.leaf_elem_type(self.module.var_decl_types.get(coll)?);
        if matches!(self.resolve_dt(&leaf), DataType::Real { .. }) {
            return Some(Value::from_f64(0.0));
        }
        let w = super::elaborate::resolve_type_width(
            &leaf,
            Some(&self.module.parameters),
            Some(&self.module.typedefs),
        );
        if w == 0 {
            return None;
        }
        Some(
            if super::elaborate::is_type_two_state_resolved(&leaf, &self.module.typedef_types) {
                Value::zero(w)
            } else {
                Value::new(w)
            },
        )
    }

    /// §7.6, §10.9.1, §10.10: assign the row value `e` to the fixed-size
    /// unpacked array stored under `base` (`m[1]`, `q[0]`, `aa[k]`), with
    /// dimensions `dims` (`descending` gives each one's direction) and
    /// elements of type `dt`. `e` is an assignment pattern, an unpacked
    /// array concatenation (`concat_row_items`) or an array of the row's
    /// shape, copied element by element in declared order. False, with
    /// nothing written, for any other expression.
    fn assign_fixed_row(
        &mut self,
        base: &str,
        dims: &[(i64, i64)],
        descending: &[bool],
        dt: &DataType,
        e: &Expression,
    ) -> bool {
        let e = self.unpacked_pattern(e);
        match &e.kind {
            ExprKind::AssignmentPattern(items) if !items.is_empty() => {
                self.assign_pattern_array(base, dims, dt, items, descending);
                true
            }
            ExprKind::Concatenation(parts) => match self.concat_row_items(parts, dims.len()) {
                Some(items) => {
                    self.assign_pattern_array(base, dims, dt, &items, descending);
                    true
                }
                None => false,
            },
            ExprKind::Ident(_) | ExprKind::Index { .. } | ExprKind::MemberAccess { .. } => {
                // §7.2: an unpacked-struct element is its member leaves; the
                // struct copy paths own it.
                if self.unpacked_struct_of(dt).is_some() {
                    return false;
                }
                let Some((src, sfx, _)) = self.nd_array_operand(e) else {
                    return false;
                };
                let dst = Self::fixed_row_suffixes(dims, descending);
                if dst.len() != sfx.len()
                    || format!("{src}{}", sfx[0]) == format!("{base}{}", dst[0])
                {
                    return false;
                }
                let vals: Vec<Value> = sfx
                    .iter()
                    .map(|s| {
                        self.get_signal_value_by_name(&format!("{src}{s}"))
                            .unwrap_or_else(|| Value::zero(32))
                    })
                    .collect();
                for (d, v) in dst.iter().zip(vals) {
                    self.write_pattern_leaf(&format!("{base}{d}"), v);
                }
                true
            }
            _ => false,
        }
    }

    /// §10.10: an unpacked array concatenation written to a fixed-size row,
    /// as the items of the equivalent assignment pattern. Over a row of one
    /// dimension each operand is evaluated now, once and left to right, and
    /// one that is an unpacked array contributes each of its elements
    /// (`unpacked_concat_operand_values`); each value is then assigned to its
    /// element as a pattern item is. Over a row of more dimensions each
    /// operand is one sub-row, taken only when every operand is a pattern.
    fn concat_row_items(
        &mut self,
        parts: &[Expression],
        rank: usize,
    ) -> Option<Vec<AssignmentPatternItem>> {
        if parts.is_empty() {
            return None;
        }
        if rank > 1 {
            let all_patterns = parts.iter().all(|p| {
                matches!(
                    self.unpacked_pattern(p).kind,
                    ExprKind::AssignmentPattern(_)
                )
            });
            if !all_patterns {
                return None;
            }
            // §10.10, §11.4.2: every operand is read before any element of
            // the row is written, so a row that reads the array it is
            // written to (`m[0] = {'{m[0][1][0], m[0][1][1]}, '{m[0][0][0],
            // m[0][0][1]}}`, a swap) takes the values from before the store.
            let mut items = Vec::with_capacity(parts.len());
            for p in parts {
                let frozen = self.frozen_row_pattern(p, rank - 1);
                items.push(AssignmentPatternItem::Ordered(frozen));
            }
            return Some(items);
        }
        let vals = self.unpacked_concat_operand_values(parts.iter().collect(), true, None);
        Some(
            vals.iter()
                .map(|v| AssignmentPatternItem::Ordered(self.value_method_arg_expr(v)))
                .collect(),
        )
    }

    /// The sub-row pattern `e`, `depth` dimensions above its elements, with
    /// each element item evaluated now into a literal (see
    /// `concat_row_items`). An item that is not a pattern above the element
    /// level (a row variable) is kept as written.
    fn frozen_row_pattern(&mut self, e: &Expression, depth: usize) -> Expression {
        let p = self.unpacked_pattern(e).clone();
        if depth == 0 {
            if matches!(p.kind, ExprKind::AssignmentPattern(_)) {
                return p;
            }
            let v = self.eval_expr(&p);
            return self.value_method_arg_expr(&v);
        }
        let ExprKind::AssignmentPattern(items) = &p.kind else {
            return p;
        };
        let mut out = Vec::with_capacity(items.len());
        for it in items {
            out.push(match it {
                AssignmentPatternItem::Ordered(x) => {
                    AssignmentPatternItem::Ordered(self.frozen_row_pattern(x, depth - 1))
                }
                other => other.clone(),
            });
        }
        Expression::new(ExprKind::AssignmentPattern(out), p.span)
    }

    /// `arr[i] = '{...}` / `assoc[key] = '{...}` where the element is an
    /// unpacked struct: spread the pattern into that element's members.
    pub(super) fn assign_pattern_element(
        &mut self,
        lvalue: &Expression,
        rvalue: &Expression,
    ) -> bool {
        let rvalue = self.unpacked_pattern(rvalue);
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
        let capture = self.pattern_nba_leaves.take();
        let iv = self.eval_expr(index);
        self.pattern_nba_leaves = capture;
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
    /// §27.6: a generate block's variable is keyed by its scope (`g.q`,
    /// `l[0].q`), so the declared base of a flat path may span several
    /// segments: the shortest dotted prefix, its trailing selects counted,
    /// that names a declared variable, then the rest as `split_flat_path`
    /// gives it.
    fn flat_path_scoped_segs(&self, flat: &str) -> Option<Vec<(String, usize)>> {
        let mut depth = 0usize;
        let ends = flat
            .char_indices()
            .filter(|&(_, c)| {
                match c {
                    '[' => depth += 1,
                    ']' => depth = depth.saturating_sub(1),
                    _ => {}
                }
                c == '.' && depth == 0
            })
            .map(|(i, _)| i)
            .skip(1)
            .chain(std::iter::once(flat.len()));
        for end in ends {
            let mut key = &flat[..end];
            let mut selects = 0usize;
            while key.ends_with(']') {
                let mut d = 0usize;
                let open = key.char_indices().rev().find_map(|(i, c)| {
                    match c {
                        ']' => d += 1,
                        '[' => d -= 1,
                        _ => {}
                    }
                    (d == 0).then_some(i)
                })?;
                key = &key[..open];
                selects += 1;
            }
            if self.module.var_decl_types.contains_key(key) {
                let mut segs = vec![(key.to_string(), selects)];
                if end < flat.len() {
                    segs.extend(Self::split_flat_path(&flat[end + 1..]));
                }
                return Some(segs);
            }
        }
        None
    }

    pub(super) fn flat_path_type(&self, flat: &str) -> Option<(DataType, Option<Vec<Vec<i64>>>)> {
        // Without a select the first segment is the text before the first
        // `.`; a base with no declared type answers None before any split.
        if !flat.as_bytes().contains(&b'[') {
            let base = flat.split('.').next().unwrap_or(flat);
            if !self.module.var_decl_types.contains_key(base) && base.len() == flat.len() {
                return None;
            }
        }
        let mut segs = Self::split_flat_path(flat);
        if !self.module.var_decl_types.contains_key(&segs.first()?.0) {
            segs = self.flat_path_scoped_segs(flat)?;
        }
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
                    .map(|md| (m.data_type.clone(), self.member_dims(&m.data_type, &md.dimensions)))
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
        // Its elements (`@name#N[i]`) take the same entry, as below; a
        // member of an element (`@d#9[1].d`) is not the element: it resolves
        // through the declared member types, or a struct element's leaf
        // `d` was taken for the struct and the write spread over it.
        if let Some((bare, rest)) = name.strip_prefix('@').and_then(|s| s.rsplit_once('#')) {
            let sel = rest.trim_start_matches(|c: char| c.is_ascii_digit());
            if sel.len() < rest.len()
                && sel.starts_with('[')
                && sel
                    .rfind(']')
                    .is_some_and(|p| sel[p + 1..].starts_with('.'))
            {
                return self
                    .flat_path_type(&format!("{bare}{sel}"))
                    .map(|(d, _)| Cow::Owned(d));
            }
            if let Some(dt) = self.module.var_decl_types.get(bare) {
                return Some(Cow::Borrowed(dt));
            }
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
            // A MEMBER of that element (`u.arr[2].f`) is not the element.
            // `flat_path_type` walks from the first segment, so an instance
            // prefix (`u.arr[2].f`) misses and used to fall through to this
            // element lookup. The member path then answered the struct type,
            // a nonblocking `arr[i].f <= v` expanded as a whole-struct copy,
            // and each leaf was `arr[i].f.f` — the same shape, so the
            // expansion recursed until the stack died. Only a name that
            // ends on the element select (`u.arr[2]`) is the element.
            let end = name.rfind(']')?;
            if end + 1 != name.len() {
                return None;
            }
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

    /// §21.2.1.7: the fixed-size array stored under `base` (an element of a
    /// collection), as a nested list over the index lists `lists`; each
    /// element prints by its type `dt`, or from its value when there is none.
    fn render_p_fixed_row(
        &mut self,
        base: &str,
        lists: &[Vec<i64>],
        dt: Option<&DataType>,
        is_str: bool,
    ) -> String {
        let Some((list, rest)) = lists.split_first() else {
            return match dt {
                Some(d) => self.render_p_typed(base, d),
                None => {
                    let v = self
                        .get_signal_value_by_name(base)
                        .unwrap_or_else(|| Value::zero(32));
                    Self::render_p_value(&v, is_str)
                }
            };
        };
        let parts: Vec<String> = list
            .iter()
            .map(|i| self.render_p_fixed_row(&format!("{base}[{i}]"), rest, dt, is_str))
            .collect();
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
    pub(super) fn unpacked_decl_dims_of(&self, name: &str) -> Option<Vec<(i64, i64)>> {
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
        // §7.4.5: the rows are queues or dynamic arrays as declared.
        let dyn_rows = inner.is_none()
            && match self.coll_shape_at(name) {
                Some((shape, depth)) => {
                    matches!(shape.dims.get(depth + 1), Some(Dim::Queue | Dim::Dyn))
                }
                // No declared shape (a returned or copied queue of queues):
                // its stored rows tell.
                None => (0..sz).any(|i| {
                    let row = format!("{}[{}]", name, i);
                    self.module.dynamic_arrays.contains(&row)
                        || self
                            .signals
                            .contains_key(&Self::name_with_suffix(&row, ".size"))
                }),
            };
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

    /// §21.2.1.7: a level of a declared collection of collections whose rows
    /// are collections themselves (`d` and `d[i][k]` of `int d[][string][$]`,
    /// not `d[i]`) prints each row as the collection its declaration names at
    /// that depth; a declared associative row with no entries prints empty.
    /// `None` leaves the level to the other renderers (an associative level,
    /// a leaf row, or a shape with fixed levels, printed by their bounds).
    fn render_p_shaped(&mut self, name: &str) -> Option<String> {
        let (shape, depth) = self.coll_shape_at(name)?;
        if shape.dims.iter().any(|d| matches!(d, Dim::Fixed(_))) {
            return None;
        }
        let rows_are_colls = depth + 1 < shape.dims.len();
        match shape.dims.get(depth)? {
            Dim::Queue | Dim::Dyn if rows_are_colls => {
                self.p_top_level = false;
                let n = match self.declared_dim(name) {
                    Some(_) => self.get_queue_size(name),
                    None => 0,
                };
                let parts: Vec<String> = (0..n)
                    .map(|i| {
                        let row = format!("{name}[{i}]");
                        self.render_p_var(&row).unwrap_or_else(|| "'{}".to_string())
                    })
                    .collect();
                Some(format!("'{{{}}}", parts.join(", ")))
            }
            Dim::Assoc(_)
                if depth > 0
                    && !self.module.associative_arrays.contains_key(name)
                    && !self.signal_name_prefix_present(&format!("{name}[")) =>
            {
                Some("'{}".to_string())
            }
            _ => None,
        }
    }

    /// §8.9: the element type of a static queue or dynamic array stored
    /// under its bare name, as a statically unique static collection is (see
    /// `spec_static_coll_key_in_class_with`). No variable declares it, so its
    /// string elements printed as numbers.
    fn bare_static_coll_elem_type(&self, name: &str) -> Option<DataType> {
        if name.contains(['[', '.', ':', '#']) {
            return None;
        }
        self.module.classes.values().find_map(|cd| {
            (cd.static_properties.contains(name)
                && cd
                    .static_collections
                    .iter()
                    .any(|(n, is_assoc, _)| n == name && !*is_assoc))
            .then(|| self.class_prop_elem_type(cd, name).cloned())
            .flatten()
        })
    }

    pub(super) fn render_p_var(&mut self, name: &str) -> Option<String> {
        if let Some(s) = self.render_p_shaped(name) {
            return Some(s);
        }
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
                None => self
                    .p_elem_type(name)
                    .or_else(|| self.bare_static_coll_elem_type(name)),
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
        // element renders from its raw value. A declared associative row
        // (`d[0]` of `int d[][string][$]`) is one whether or not registered.
        if self.module.associative_arrays.contains_key(name)
            || matches!(self.declared_dim(name), Some(Dim::Assoc(_)))
        {
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
            let keys = self.assoc_top_level_keys_shared(name);
            let string_keyed = self.is_string_keyed_array(name);
            let val_is_str = self.string_signals.contains(name);
            // §21.2.1.7: an element that is a fixed-size array
            // (`int aa[string][2]`) prints as a nested list in declared order.
            let rows = self.coll_fixed_row_dims(name).map(|(dims, desc)| {
                (
                    Self::fixed_row_lists(&dims, &desc),
                    dt.as_ref().map(|d| self.leaf_elem_type(d)),
                )
            });
            let parts: Vec<String> = keys
                .iter()
                .map(|k| {
                    let elem = format!("{}[{}]", name, k);
                    let v = match (&dt, &rows) {
                        // §21.2.1.7: an element that is itself a queue,
                        // dynamic or associative array (`int aq[int][$]`,
                        // `int aa[int][int]`) prints as a nested list.
                        _ if self.p_elem_is_collection(&elem) => self
                            .render_p_var(&elem)
                            .unwrap_or_else(|| "'{}".to_string()),
                        (_, Some((lists, leaf))) => {
                            self.render_p_fixed_row(&elem, lists, leaf.as_deref(), val_is_str)
                        }
                        (Some(d), None) => self.render_p_typed(&elem, d),
                        (None, None) => {
                            let ev = self
                                .get_signal_value_by_name(&elem)
                                .unwrap_or_else(|| Value::zero(32));
                            Self::render_p_value(&ev, val_is_str)
                        }
                    };
                    if string_keyed {
                        format!("\"{}\":{}", decode_name_key(k), v)
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
            let keys = self.assoc_top_level_keys_shared(name);
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
                        format!("\"{}\":{}", decode_name_key(k), v)
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
