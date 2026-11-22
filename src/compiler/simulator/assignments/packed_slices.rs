//! Packed struct member slices, packed path slices and packed slot flattening.

use super::*;

impl Simulator {
    /// The registered packed-struct base of a dotted member lvalue, as
    /// `(base_name, bit_offset, width)` of the selected member within it.
    ///
    /// `packed_struct_fields[base]` is FLATTENED — a nested member is a key in
    /// its own right (`"hdr.d.x"`), and no entry exists for an intermediate
    /// member on its own (`packed_struct_fields["f.hdr"]` is None). So the
    /// single split at the last dot only ever resolved a DEPTH-1 member, and
    /// `f.hdr.d = v` inside a subroutine — where the lvalue parses as
    /// MemberAccess rather than a dotted Ident — matched nothing and was
    /// silently dropped. Walk the split points LONGEST BASE FIRST: the base
    /// may itself be instance-scoped (`u.r2`), so the first-dot split is
    /// wrong too.
    pub(in crate::compiler::simulator) fn packed_struct_member_slice(
        &mut self,
        lvalue: &Expression,
    ) -> Option<(String, u32, u32)> {
        let flat = self.flat_member_name(lvalue)?;
        for (i, _) in flat
            .match_indices('.')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            let (base, prefix) = (&flat[..i], &flat[i + 1..]);
            let Some(layout) = self.module.packed_struct_fields.get(base) else {
                continue;
            };
            if let Some((_, o, w)) = layout.iter().find(|(k, _, _)| k == prefix) {
                return Some((base.to_string(), *o, *w));
            }
        }
        None
    }

    /// Resolve a class-property assignment target: `obj.prop` (a
    /// handle-valued name), `this.prop` / `super.prop`, or a bare `prop`
    /// inside a method. Returns the heap handle and the property's storage
    /// key in that object.
    pub(in crate::compiler::simulator) fn class_prop_pattern_target(
        &mut self,
        lvalue: &Expression,
    ) -> Option<(usize, String)> {
        // §8.14/§8.15: `super.p`, and `h.p` / `this.p` naming a property a
        // derived class redeclares, select a base class's cell; take the
        // spelling `assign_value` writes, never the leaf's `p`.
        if let Some(rw) = self
            .super_rewrite(lvalue)
            .or_else(|| self.shadowed_member_rewrite(lvalue))
        {
            return self.class_prop_pattern_target(&rw);
        }
        let cand: Option<(usize, String)> = match &lvalue.kind {
            // `obj.prop` as a dotted identifier.
            ExprKind::Ident(hier)
                if hier.path.len() == 2 && hier.path.iter().all(|s| s.selects.is_empty()) =>
            {
                // `this.prop` / `super.prop` fold into a dotted identifier
                // whose head is the keyword, not a handle variable: the head
                // cannot be evaluated by name, so resolve it through
                // this_stack (the object `super` denotes here as well).
                if hier.path[0].name.name.as_str() == "this"
                    || hier.path[0].name.name.as_str() == "super"
                {
                    self.this_stack
                        .last()
                        .copied()
                        .flatten()
                        .map(|hd| (hd, hier.path[1].name.name.clone()))
                } else {
                    let head = hier_prefix(hier);
                    if !self.head_denotes_class_object(&head) {
                        return None;
                    }
                    let handle = self
                        .eval_expr(&Expression::new(ExprKind::Ident(head), lvalue.span))
                        .to_u64()? as usize;
                    Some((handle, hier.path[1].name.name.clone()))
                }
            }
            // `recv.prop` where `recv` is a bare handle name or `this`/`super`.
            ExprKind::MemberAccess { expr, member } => match &expr.kind {
                // `this.prop` with the keyword as its own expression form.
                ExprKind::This => self
                    .this_stack
                    .last()
                    .copied()
                    .flatten()
                    .map(|hd| (hd, member.name.clone())),
                ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].selects.is_empty() => {
                    let name = h.path[0].name.name.as_str();
                    if name == "this" || name == "super" {
                        self.this_stack
                            .last()
                            .copied()
                            .flatten()
                            .map(|hd| (hd, member.name.clone()))
                    } else {
                        // Only a genuine class channel: a plain integral
                        // value must not be misread as a heap handle.
                        if !self.bare_name_is_class_channel(name) {
                            return None;
                        }
                        self.eval_ident_handle(name).map(|hd| (hd, member.name.clone()))
                    }
                }
                _ => None,
            },
            // Bare `prop` inside a method: the current `this` object. A
            // shadowing local/argument was already served by the
            // registered-signal path above; only a real property remains.
            ExprKind::Ident(hier) if hier.path.len() == 1 && hier.path[0].selects.is_empty() => {
                let handle = self.this_stack.last().copied().flatten()?;
                let name = hier.path[0].name.name.clone();
                let key = self.shadowed_prop_key(&name, false).unwrap_or(name);
                if !self
                    .heap
                    .get(handle)?
                    .as_ref()?
                    .properties
                    .contains_key(&key)
                {
                    return None;
                }
                Some((handle, key))
            }
            _ => None,
        };
        let (handle, prop) = cand?;
        // A live object holding this property.
        if !self
            .heap
            .get(handle)?
            .as_ref()?
            .properties
            .contains_key(&prop)
        {
            return None;
        }
        Some((handle, prop))
    }

    /// Resolve `arr[i]…[j].field…[k]` over a PACKED array of packed structs to
    /// the `(backing_signal, lsb, width)` slice it denotes, where `base` is the
    /// expression left of the trailing `[k]` and `idx` is that index.
    ///
    /// Returns None for anything that is not this shape — an unpacked array of
    /// structs (whose elements ARE separate signals), a plain struct, or a base
    /// with no registered element/field metadata — so callers fall through to
    /// the existing paths unchanged.
    /// Walk a `.field` / `[const]` chain down to its root identifier, returning
    /// the root's flat storage name and the steps below it, outermost first.
    /// A step is `(Some(field), _)` or `(None, index)`.
    pub(in crate::compiler::simulator) fn packed_path_steps<'e>(
        &mut self,
        e: &'e Expression,
    ) -> Option<(String, Vec<(Option<&'e str>, i64)>)> {
        let mut steps: Vec<(Option<&'e str>, i64)> = Vec::new();
        let mut cur: &'e Expression = e;
        loop {
            match &cur.kind {
                ExprKind::MemberAccess { expr, member } => {
                    steps.push((Some(member.name.as_str()), 0));
                    cur = expr.as_ref();
                }
                ExprKind::Index { expr, index } => {
                    steps.push((None, self.eval_expr(index).to_i64()?));
                    cur = expr.as_ref();
                }
                _ => break,
            }
        }
        steps.reverse();
        let ExprKind::Ident(h) = &cur.kind else {
            return None;
        };
        Some((self.resolve_hier_name(h).into_owned(), steps))
    }

    /// Resolve a walked path over a PACKED aggregate to the
    /// `(backing_signal, lsb, width)` slice it denotes.
    ///
    /// The steps alternate in the nested case — `node.words[0].words[0]` is field,
    /// index, field, index — so the path is rendered into the dotted-with-
    /// indices key elaboration already registers (`words[0].words`) rather than
    /// being matched against a fixed "indices then fields" shape. Matching that
    /// fixed shape resolved the first group and then gave up, so every access
    /// below one level of nesting read as 0.
    ///
    /// Returns None for anything not this shape, so callers fall through to the
    /// existing paths unchanged.
    pub(in crate::compiler::simulator) fn packed_path_slice(
        &mut self,
        root: &str,
        steps: &[(Option<&str>, i64)],
    ) -> Option<(String, u32, u32)> {
        // Inside an inlined instance a member access folds into the root
        // (`cw.loc.descriptors`): that name carries an element width but no
        // field layout, so every nested read below it came back 0. Peel
        // trailing segments back into field steps until the root names the
        // struct signal itself.
        if !self.module.packed_struct_fields.contains_key(root)
            && !self.module.arrays.contains_key(root)
        {
            if let Some((head, seg)) = root.rsplit_once('.') {
                let mut widened: Vec<(Option<&str>, i64)> = Vec::with_capacity(steps.len() + 1);
                widened.push((Some(seg), 0));
                widened.extend_from_slice(steps);
                return self.packed_path_slice(head, &widened);
            }
        }
        // Leading indices apply to the root's own dimensions; the rest form the
        // field path, which may carry indices of its own.
        let lead_len = steps
            .iter()
            .take_while(|(field, _)| field.is_none())
            .count();
        let rest = &steps[lead_len..];
        if rest.is_empty() {
            return None; // a pure index chain is not a field path
        }
        // An UNPACKED dimension stores each element as its own signal, so the
        // leading indices covering it belong in the signal NAME; the layout
        // stays keyed by the array's base name.
        let mut storage = root.to_string();
        let mut consumed = 0usize;
        if self.module.arrays.contains_key(root) {
            for &(_, i) in &steps[..lead_len] {
                let cand = format!("{}[{}]", storage, i);
                if self.get_signal_value_by_name(&cand).is_some() {
                    storage = cand;
                    consumed += 1;
                } else {
                    break;
                }
            }
            if consumed == 0 {
                return None;
            }
        } else if let Some(nu) = self
            .module
            .arrays_2d
            .contains_key(root)
            .then_some(2)
            .or_else(|| self.module.arrays_nd.get(root).map(|(s, _)| s.len()))
        {
            // §7.4.1: an element of a multi-dimensional unpacked array
            // (`s3[1][0].f0[1]`) is the signal named by all its unpacked
            // indices; none of its prefixes is one.
            if lead_len < nu {
                return None;
            }
            for &(_, i) in &steps[..nu] {
                storage.push('[');
                Self::push_i64(&mut storage, i);
                storage.push(']');
            }
            self.get_signal_value_by_name(&storage)?;
            consumed = nu;
        }
        let pre = &steps[consumed..lead_len];
        let fields = self.module.packed_struct_fields.get(root)?;
        // Width of ONE struct, taken from the field layout. NOT
        // `packed_signal_elem_widths[root]`: that is the width of an element of
        // the OUTERMOST dimension, which for `t [0:0][1:0]` is the whole [1:0]
        // sub-array (two structs), not one.
        let struct_w = fields.iter().map(|(_, o, w)| o + w).max()?;
        // Flatten the leading chain to a slot. `packed_full_dims` carries the
        // declared ranges so a non-zero-based or ascending dimension maps the
        // way §7.4.1 labels it; without dims fall back to row-major.
        let slot = if pre.is_empty() {
            0
        } else {
            let dims = self.module.packed_full_dims.get(root).map(Vec::as_slice);
            Self::flatten_packed_slot_iter(pre.iter().map(|(_, index)| *index), dims)?
        };
        // Render the field path with indices inline, matching the registered key.
        let render_key = |steps: &[(Option<&str>, i64)]| -> String {
            let mut key = String::new();
            for (f, i) in steps {
                match f {
                    Some(f) => {
                        if !key.is_empty() {
                            key.push('.');
                        }
                        key.push_str(f);
                    }
                    None => {
                        key.push('[');
                        Self::push_i64(&mut key, *i);
                        key.push(']');
                    }
                }
            }
            key
        };
        let key = render_key(rest);
        // Checked: an out-of-range index must miss, not wrap into another bit.
        let base = slot.checked_mul(struct_w)?;
        // Exact hit: the whole path names a registered field.
        if let Some((_, off, w)) = fields.iter().find(|(m, _, _)| m == &key) {
            let (off, w) = (*off, *w);
            let total = self.get_signal_value_by_name(&storage)?.width;
            let lsb = base.checked_add(off)?;
            return (lsb.checked_add(w)? <= total).then_some((storage, lsb, w));
        }
        // Otherwise walk the path: fields and indices interleaved at any
        // depth (`wdata[0]`, `descriptors[0].region`, `m[1].words[2]`). Every
        // registered key is element-0-relative along its whole path, so the
        // deepest field supplies the offset and each index step adds its
        // member's element displacement on top. The trailing-index-only
        // shape used to be the ceiling; a field AFTER an index read 0.
        // Layouts come in two key conventions: element-relative
        // (`descriptors.region`) and expanded per element with the index
        // inline (`wdata[0].wdata`, `wdata[1].wdata`). A field step tries the
        // path with its ACTUAL indices first (absolute offset, so the index
        // displacement collected so far is dropped), then the index-free
        // form, then the element-0 form (both element-0-relative, so the
        // displacement stays).
        let mut norm = String::new();
        let mut actual = String::new();
        let mut zero = String::new();
        let mut off: u32 = 0;
        let mut fw: u32 = struct_w;
        let mut disp: u32 = 0;
        let mut i = 0;
        while i < rest.len() {
            match rest[i] {
                (Some(f), _) => {
                    for k in [&mut norm, &mut actual, &mut zero] {
                        if !k.is_empty() {
                            k.push('.');
                        }
                        k.push_str(f);
                    }
                    if let Some((_, o, w)) = fields.iter().find(|(m, _, _)| m == &actual) {
                        off = *o;
                        fw = *w;
                        disp = 0;
                    } else {
                        let (_, o, w) = fields.iter().find(|(m, _, _)| m == &norm || m == &zero)?;
                        off = *o;
                        fw = *w;
                    }
                    i += 1;
                }
                (None, _) => {
                    if norm.is_empty() {
                        return None;
                    }
                    let mkey = format!("{}.{}", root, norm);
                    let mut idxs: Vec<i64> = Vec::new();
                    while i < rest.len() && rest[i].0.is_none() {
                        idxs.push(rest[i].1);
                        actual.push('[');
                        Self::push_i64(&mut actual, rest[i].1);
                        actual.push(']');
                        zero.push_str("[0]");
                        i += 1;
                    }
                    let ew = self.module.packed_signal_elem_widths.get(&mkey).copied();
                    let (step_w, slot) = if idxs.len() == 1 {
                        // A member with no element width is a plain vector:
                        // the index is a bit select. §7.4.1: an element of a
                        // packed-array member is labelled by its declared
                        // range, so `pair[0]` of `lane_t [0:1] pair` is the
                        // MOST significant lane.
                        let slot = match self.module.packed_full_dims.get(&mkey) {
                            Some(dims) if ew.is_some() => Self::flatten_packed_slot_iter(
                                idxs.iter().copied(),
                                Some(dims.as_slice()),
                            )?,
                            _ => u32::try_from(idxs[0]).ok()?,
                        };
                        (ew.unwrap_or(1), slot)
                    } else {
                        let ew = ew?;
                        let dims = self.module.packed_full_dims.get(&mkey)?;
                        let slot = Self::flatten_packed_slot_iter(
                            idxs.iter().copied(),
                            Some(dims.as_slice()),
                        )?;
                        let inner: u32 = dims
                            .iter()
                            .skip(1)
                            .take(idxs.len() - 1)
                            .map(|(l, r)| ((l - r).unsigned_abs() + 1) as u32)
                            .product();
                        (ew / inner.max(1), slot)
                    };
                    disp = disp.checked_add(slot.checked_mul(step_w)?)?;
                    fw = step_w.min(fw);
                }
            }
        }
        let lsb = base.checked_add(off)?.checked_add(disp)?;
        let total = self.get_signal_value_by_name(&storage)?.width;
        (lsb.checked_add(fw)? <= total).then_some((storage, lsb, fw))
    }

    /// Resolve `arr[i]…[j].field…[k]` over a PACKED array of packed structs to
    /// the `(backing_signal, lsb, width)` slice it denotes, where `base` is the
    /// expression left of the trailing `[k]` and `idx` is that index.
    pub(in crate::compiler::simulator) fn packed_struct_elem_slice(
        &mut self,
        base: &Expression,
        idx: &Expression,
    ) -> Option<(String, u32, u32)> {
        // Left of the trailing index must be `<...>.field`, possibly with
        // more indices after the field: §7.4.1, `sa[1].f0[3][7]` selects an
        // element of a multi-dimensional packed member at every index, and
        // as `<...>.field[3]` this shape was declined, so the store was
        // dropped.
        let mut field = base;
        while let ExprKind::Index { expr, .. } = &field.kind {
            field = expr;
        }
        if !matches!(field.kind, ExprKind::MemberAccess { .. }) {
            return None;
        }
        let (root, mut steps) = self.packed_path_steps(base)?;
        // With no index anywhere but the trailing one this is a plain struct
        // member select, which the existing member path already handles — leave
        // it there rather than re-deriving it here.
        if steps.iter().all(|(f, _)| f.is_some()) {
            return None;
        }
        steps.push((None, self.eval_expr(idx).to_i64()?));
        self.packed_path_slice(&root, &steps)
    }

    /// Resolve `<...>.member` where the receiver is itself an indexed packed
    /// aggregate (`n.wdata[0].amask`). Used as a LAST resort, after every other
    /// member path has declined, so established lookups keep priority.
    pub(in crate::compiler::simulator) fn packed_member_slice<'e>(
        &mut self,
        expr: &'e Expression,
        member: &'e str,
    ) -> Option<(String, u32, u32)> {
        if !matches!(expr.kind, ExprKind::Index { .. }) {
            return None;
        }
        let (root, mut steps) = self.packed_path_steps(expr)?;
        steps.push((Some(member), 0));
        self.packed_path_slice(&root, &steps)
    }

    /// Row-major flatten of an index chain over a packed array's declared
    /// dimensions. `dims` are outermost-first `(left, right)` declared ranges;
    /// the trailing entries that describe the ELEMENT type are ignored.
    pub(in crate::compiler::simulator) fn flatten_packed_slot(
        indices: &[u64],
        dims: Option<&[(i64, i64)]>,
    ) -> Option<u32> {
        Self::flatten_packed_slot_iter(indices.iter().map(|&index| index as i64), dims)
    }

    pub(in crate::compiler::simulator) fn flatten_packed_slot_iter(
        indices: impl IntoIterator<Item = i64>,
        dims: Option<&[(i64, i64)]>,
    ) -> Option<u32> {
        let Some(dims) = dims else {
            // No declared dims on file: treat the chain as row-major over
            // equal-sized dimensions, which is the historical behaviour.
            return indices
                .into_iter()
                .last()
                .and_then(|index| u32::try_from(index).ok());
        };
        let mut slot: u64 = 0;
        let mut any = false;
        for (n, i) in indices.into_iter().enumerate() {
            any = true;
            let (l, r) = *dims.get(n)?;
            let (lo, hi) = (l.min(r), l.max(r));
            let count = (hi - lo + 1) as u64;
            let off = (i - lo) as u64;
            if off >= count {
                return None;
            }
            // §7.4.1: a descending range labels the LEFT bound as the most
            // significant element; an ascending one reverses the slot order.
            let s = if l >= r { off } else { count - 1 - off };
            slot = slot * count + s;
        }
        any.then(|| u32::try_from(slot).ok()).flatten()
    }
}
