//! IEEE 1800-2017 §7.4.6 (§7.4.5 in 1800-2023), §7.6: a slice of an
//! unpacked array, `a[l:r]`, `a[b +: w]` or `a[b -: w]`, is itself an
//! unpacked array, of constant size and with a base that may vary at run
//! time. An assignment, a nonblocking assignment or an argument pairs its
//! elements with the other side's left to right, whatever the direction of
//! each range (§7.6, §10.4.2, §13.5). The paths that copy arrays knew whole
//! arrays by name only, so a slice on either side read or wrote the wrong
//! elements, or none (#867). These helpers name a slice's elements in
//! left-to-right order for all of them.

use super::*;

/// The elements of an unpacked-array operand, left to right.
pub(super) struct UnpackedElems {
    /// The identifier the operand selects from, and its storage name.
    pub root: Expression,
    pub root_name: String,
    /// The storage name of each element, `None` for one outside the
    /// declared range (read as the default, never written).
    pub names: Vec<Option<String>>,
    /// The width of one element.
    pub ew: u32,
    /// Whether the operand is a slice (its last select is a range).
    pub sliced: bool,
    /// Whether it is a selected part of an array, a slice or a sub-array
    /// (`m[1]`), not the whole array or one element.
    pub selected: bool,
}

/// One select applied to an unpacked array, outermost dimension first.
enum Step<'e> {
    Index(&'e Expression),
    Range(RangeKind, &'e Expression, &'e Expression),
}

impl Simulator {
    /// Whether `e` is written as a range select of something, possibly
    /// under element selects (`a[0+:4]`, `m[1][2:0]`): the only shapes the
    /// slice paths below take, checked before any name is resolved.
    pub(super) fn is_range_select_chain(e: &Expression) -> bool {
        let mut cur = e;
        loop {
            match &cur.kind {
                ExprKind::RangeSelect { .. } => return true,
                ExprKind::Index { expr, .. } => cur = expr,
                _ => return false,
            }
        }
    }

    /// The declared `(left, right)` of every unpacked dimension of the fixed
    /// array `name`, outermost first, and its element width. Dynamic arrays
    /// and queues use their live extent; class properties use their receiver's
    /// specialization. Associative arrays have no slice range.
    pub(super) fn unpacked_decl_shape(&self, name: &str) -> Option<(Vec<(i64, i64)>, u32)> {
        if self.is_associative_array(name) {
            return None;
        }
        if let Some(size) = self.live_coll_size(name) {
            if size == 0
                || size > 1 << 20
                || self
                    .coll_shape_at(name)
                    .is_some_and(|(shape, depth)| shape.dims.len() - depth != 1)
            {
                return None;
            }
            let ew = self
                .lookup_signal_width(&format!("{name}[0]"))
                .or_else(|| {
                    self.get_signal_value_by_name(&format!("{name}[0]"))
                        .map(|v| v.width)
                })
                .or_else(|| self.module.arrays.get(name).map(|a| a.2))
                .unwrap_or(32);
            return Some((vec![(0, size as i64 - 1)], ew));
        }
        if let Some((h, member)) = name.split_once('#')
            && let Ok(handle) = h.parse::<usize>()
        {
            let mut class = self.heap.get(handle)?.as_ref()?.class_name.as_str();
            for _ in 0..64 {
                let cd = self.module.classes.get(class)?;
                let layout = if let Some((dims, ew)) = cd.array_nd_properties.get(member) {
                    Some((
                        self.inst_nd_shape(cd, handle, member)
                            .unwrap_or_else(|| dims.clone()),
                        *ew,
                    ))
                } else if let Some(&(lo, hi, ew)) = cd.array_properties.get(member) {
                    let bounds = self.inst_fixed_bounds(cd, handle, member, (lo, hi));
                    Some((vec![bounds], ew))
                } else {
                    None
                };
                if let Some((mut dims, ew)) = layout {
                    if let Some(declared) = cd.array_prop_decl_dims.get(member) {
                        for (dim, &(l, r)) in dims.iter_mut().zip(declared) {
                            if l > r {
                                *dim = (dim.1, dim.0);
                            }
                        }
                    }
                    let first = dims
                        .iter()
                        .map(|d| format!("[{}]", d.0))
                        .collect::<String>();
                    let ew = self
                        .get_signal_value_by_name(&format!("{name}{first}"))
                        .map_or(ew, |v| v.width);
                    return Some((dims, ew));
                }
                class = cd.extends.as_deref()?;
            }
            return None;
        }
        if let Some(&(lo, hi, ew)) = self.module.arrays.get(name) {
            let d = match self.module.unpacked_decl_dims.get(name) {
                Some(d) if d.len() == 1 => d[0],
                _ if self.module.descending_arrays.contains(name) => (hi, lo),
                _ => (lo, hi),
            };
            return Some((vec![d], ew));
        }
        if let Some(&(r0, r1, ew)) = self.module.arrays_2d.get(name) {
            return Some(match self.module.unpacked_decl_dims.get(name) {
                Some(d) if d.len() == 2 => (d.clone(), ew),
                _ => (vec![r0, r1], ew),
            });
        }
        let (shape, ew) = self.module.arrays_nd.get(name)?;
        Some(match self.module.unpacked_decl_dims.get(name) {
            Some(d) if d.len() == shape.len() => (d.clone(), *ew),
            _ => (shape.clone(), *ew),
        })
    }

    /// The indices a range select picks from a dimension, left to right:
    /// `[l:r]` from `l` to `r`; `[b +: w]` covers `b .. b+w-1` and
    /// `[b -: w]` covers `b-w+1 .. b`, read in the dimension's direction
    /// (`desc`: declared `[hi:lo]`). `None` for a bound with x or z bits.
    fn slice_indices(
        &mut self,
        kind: RangeKind,
        left: &Expression,
        right: &Expression,
        desc: bool,
    ) -> Option<Vec<i64>> {
        let konst = |sim: &mut Self, e: &Expression| -> Option<i64> {
            let v = sim.eval_expr(e);
            if v.has_xz() { None } else { v.to_i64() }
        };
        let (lo, hi, rev) = match kind {
            RangeKind::Constant => {
                let (l, r) = (konst(self, left)?, konst(self, right)?);
                (l.min(r), l.max(r), l > r)
            }
            RangeKind::IndexedUp | RangeKind::IndexedDown => {
                let b = konst(self, left)?;
                let w = konst(self, right)?;
                if w <= 0 || w > 1 << 20 {
                    return None;
                }
                if kind == RangeKind::IndexedUp {
                    (b, b + w - 1, desc)
                } else {
                    (b - w + 1, b, desc)
                }
            }
        };
        if hi - lo >= 1 << 20 {
            return None;
        }
        Some(if rev {
            (lo..=hi).rev().collect()
        } else {
            (lo..=hi).collect()
        })
    }

    /// The elements of `e` when it names a fixed unpacked array, one of its
    /// sub-arrays (`m[1]`), or a slice of either (`a[0+:6]`, `m[1][2:0]`,
    /// `c[0+:2]`), left to right (§7.4.6, §7.6). Element and range selects
    /// are evaluated now. `None` for anything else.
    pub(super) fn unpacked_operand_elems(&mut self, e: &Expression) -> Option<UnpackedElems> {
        let mut steps: Vec<Step> = Vec::new();
        let mut cur = e;
        loop {
            match &cur.kind {
                ExprKind::Index { expr, index } => {
                    steps.push(Step::Index(index));
                    cur = expr;
                }
                ExprKind::RangeSelect {
                    expr,
                    kind,
                    left,
                    right,
                } => {
                    steps.push(Step::Range(*kind, left, right));
                    cur = expr;
                }
                _ => break,
            }
        }
        steps.reverse();
        let root_name = self.expr_assoc_name(cur).or_else(|| {
            // A hierarchical aggregate has element storage but no scalar
            // signal of its own. Preserve its registered full path before
            // resolving a same-named local in the active subroutine.
            if (matches!(cur.kind, ExprKind::MemberAccess { .. })
                || matches!(&cur.kind, ExprKind::Ident(h) if h.path.len() > 1))
                && let Some(name) = self
                    .flat_member_name(cur)
                    .filter(|name| self.unpacked_decl_shape(name).is_some())
            {
                return Some(name);
            }
            let ExprKind::Ident(h) = &cur.kind else {
                return None;
            };
            if h.path.iter().any(|s| !s.selects.is_empty()) {
                return None;
            }
            let bare = h.path.last()?.name.name.as_str();
            let local = (h.root.is_none() && h.path.len() == 1)
                .then(|| self.dyn_name_lookup(bare))
                .flatten();
            Some(
                local
                    .map(str::to_string)
                    .unwrap_or_else(|| self.resolve_hier_name(h).into_owned()),
            )
        })?;
        let (dims, ew) = self.unpacked_decl_shape(&root_name)?;
        // Every select must address an unpacked dimension, and only the last
        // one may be a range: a slice has no element select of its own.
        if steps.len() > dims.len()
            || steps
                .iter()
                .rev()
                .skip(1)
                .any(|s| matches!(s, Step::Range(..)))
        {
            return None;
        }
        let sliced = matches!(steps.last(), Some(Step::Range(..)));
        let selected = sliced || (!steps.is_empty() && steps.len() < dims.len());
        let mut names: Vec<Option<String>> = vec![Some(root_name.clone())];
        for (d, step) in steps.iter().enumerate() {
            let (l, r) = dims[d];
            let (lo, hi) = (l.min(r), l.max(r));
            let idx: Vec<i64> = match step {
                Step::Index(ix) => {
                    let v = self.eval_expr(ix);
                    if v.has_xz() {
                        return None;
                    }
                    vec![v.to_i64()?]
                }
                Step::Range(kind, sl, sr) => self.slice_indices(*kind, sl, sr, l > r)?,
            };
            names = names
                .iter()
                .flat_map(|p| {
                    idx.iter().map(move |&i| {
                        p.as_ref()
                            .filter(|_| (lo..=hi).contains(&i))
                            .map(|p| format!("{p}[{i}]"))
                    })
                })
                .collect();
        }
        for &(l, r) in &dims[steps.len()..] {
            let idx: Vec<i64> = if l <= r {
                (l..=r).collect()
            } else {
                (r..=l).rev().collect()
            };
            if names.len().saturating_mul(idx.len()) > 1 << 20 {
                return None;
            }
            names = names
                .iter()
                .flat_map(|p| {
                    idx.iter()
                        .map(move |i| p.as_ref().map(|p| format!("{p}[{i}]")))
                })
                .collect();
        }
        Some(UnpackedElems {
            root: cur.clone(),
            root_name,
            names,
            ew,
            sliced,
            selected,
        })
    }

    /// The elements of a selected part of an unpacked array, a slice or a
    /// sub-array (`m[1]`), whose elements are plain values (not unpacked
    /// structs, which are member leaves), or `None`.
    pub(super) fn unpacked_selected_operand(&mut self, e: &Expression) -> Option<UnpackedElems> {
        if !matches!(
            e.kind,
            ExprKind::Index { .. } | ExprKind::RangeSelect { .. }
        ) {
            return None;
        }
        self.unpacked_value_elems(e).filter(|el| el.selected)
    }

    /// `unpacked_operand_elems` when every in-range element is a value
    /// signal.
    fn unpacked_value_elems(&mut self, e: &Expression) -> Option<UnpackedElems> {
        let el = self.unpacked_operand_elems(e)?;
        let first = el.names.iter().flatten().next()?;
        self.get_signal_value_by_name(first)?;
        Some(el)
    }

    /// The values of `el`'s elements; one outside the declared range reads
    /// as x (§7.4.6).
    pub(super) fn read_unpacked_elems(&mut self, el: &UnpackedElems) -> Vec<Value> {
        el.names
            .iter()
            .map(|n| {
                n.as_ref()
                    .and_then(|n| self.get_signal_value_by_name(n))
                    .unwrap_or_else(|| Value::new(el.ew))
            })
            .collect()
    }

    /// §7.6: `lhs = rhs` when either side is a slice of an unpacked array:
    /// the elements pair left to right, the source read in full before any
    /// element is written (the two sides may overlap). False when it is not
    /// that shape, or the two sides differ in size.
    pub(super) fn assign_unpacked_slice(
        &mut self,
        lvalue: &Expression,
        rvalue: &Expression,
    ) -> bool {
        if !Self::is_range_select_chain(lvalue) && !Self::is_range_select_chain(rvalue) {
            return false;
        }
        let Some(dst) = self.unpacked_value_elems(lvalue) else {
            return false;
        };
        // §10.9.1: a slice is an unpacked target, so a pattern supplies one
        // item per selected element in left-to-right order, not packed bits.
        if let Some(values) = self.unpacked_slice_pattern_values(&dst, rvalue) {
            for (name, value) in dst.names.iter().zip(values) {
                if let Some(name) = name {
                    self.set_signal_value_by_name(name, value);
                }
            }
            return true;
        }
        let Some(src) = self.unpacked_value_elems(rvalue) else {
            return false;
        };
        if !(dst.sliced || src.sliced) || dst.names.len() != src.names.len() {
            return false;
        }
        let values = self.read_unpacked_elems(&src);
        for (name, v) in dst.names.iter().zip(values) {
            let Some(name) = name else {
                continue;
            };
            let w = self.lookup_signal_width(name).unwrap_or(dst.ew);
            self.set_signal_value_by_name(name, v.resize(w));
        }
        true
    }

    /// Snapshot every pattern item before storing a slice: an item can read
    /// another selected element, and the two sides may overlap (§10.9.1).
    fn unpacked_slice_pattern_values(
        &mut self,
        dst: &UnpackedElems,
        rvalue: &Expression,
    ) -> Option<Vec<Value>> {
        let ExprKind::AssignmentPattern(items) = &self.unpacked_pattern(rvalue).kind else {
            return None;
        };
        if !dst.sliced {
            return None;
        }
        let indices: Vec<i64> = dst
            .names
            .iter()
            .map(|n| {
                n.as_ref()
                    .and_then(|n| n.rsplit_once('['))
                    .and_then(|(_, n)| n.strip_suffix(']'))
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(i64::MIN)
            })
            .collect();
        let elems = self.pattern_elems(items, &indices);
        let dt = self
            .class_member_coll_type(&dst.root_name)
            .cloned()
            .or_else(|| self.p_elem_type(&dst.root_name));
        Some(
            elems
                .into_iter()
                .map(|e| match (e, dt.as_ref()) {
                    (Some(e), Some(dt)) => self.eval_pattern_leaf_value(dt, e, Some(dst.ew)),
                    (Some(e), None) => {
                        let v = self.eval_expr_ctx(e, dst.ew);
                        self.fit_pattern_item_value(v, dst.ew)
                    }
                    _ => Value::new(dst.ew),
                })
                .collect(),
        )
    }

    /// §10.4.2: the per-element updates of `lhs <= rhs` when either side is
    /// a slice of an unpacked array, every source element read now.
    pub(super) fn unpacked_slice_nba_leaves(
        &mut self,
        lvalue: &Expression,
        rvalue: &Expression,
    ) -> Option<Vec<(Expression, Value)>> {
        if !Self::is_range_select_chain(lvalue) && !Self::is_range_select_chain(rvalue) {
            return None;
        }
        let dst = self.unpacked_value_elems(lvalue)?;
        if let Some(values) = self.unpacked_slice_pattern_values(&dst, rvalue) {
            return Some(
                dst.names
                    .iter()
                    .zip(values)
                    .filter_map(|(name, v)| {
                        let suffix = name.as_ref()?.strip_prefix(&dst.root_name)?;
                        Some((Self::append_leaf_path(&dst.root, suffix), v))
                    })
                    .collect(),
            );
        }
        let src = self.unpacked_value_elems(rvalue)?;
        if !(dst.sliced || src.sliced) || dst.names.len() != src.names.len() {
            return None;
        }
        let values = self.read_unpacked_elems(&src);
        Some(
            dst.names
                .iter()
                .zip(values)
                .filter_map(|(name, v)| {
                    let name = name.as_ref()?;
                    let suffix = name.strip_prefix(&dst.root_name)?;
                    Some((Self::append_leaf_path(&dst.root, suffix), v))
                })
                .collect(),
        )
    }

    /// §13.5: bind the one-dimensional array formal `param` (`lo..=hi`,
    /// declared `[hi:lo]` when `desc`) to a slice or sub-array actual
    /// (`a[0 +: 6]`, `m[1]`): its elements are
    /// copied in left to right, and for an `output`, `inout` or `ref` formal
    /// the returned caller key names the slice's elements for
    /// `writeback_array_args`. `None` when `arg` is no slice or the sizes
    /// differ.
    pub(super) fn bind_slice_array_arg(
        &mut self,
        port: &crate::ast::decl::FunctionPort,
        arg: &Expression,
        lo: i64,
        hi: i64,
        desc: bool,
    ) -> Option<(String, String, i64, i64)> {
        let src = self.unpacked_selected_operand(arg)?;
        let order: Vec<i64> = if desc {
            (lo..=hi).rev().collect()
        } else {
            (lo..=hi).collect()
        };
        if order.len() != src.names.len() {
            return None;
        }
        let param = self.array_formal_key(port);
        let out_default = self.output_formal_elem_default(port, src.ew);
        let values = match &out_default {
            Some(d) => vec![d.clone(); order.len()],
            None => self.read_unpacked_elems(&src),
        };
        for (&i, v) in order.iter().zip(values) {
            self.signals.insert(format!("{param}[{i}]"), v);
        }
        self.module.arrays.insert(param.clone(), (lo, hi, src.ew));
        let copies_back = !matches!(port.direction, crate::ast::types::PortDirection::Input);
        let caller = if copies_back {
            self.slice_arg_seq += 1;
            let key = format!("{SLICE_ARG_MARKER}{}", self.slice_arg_seq);
            let pairs = order.iter().copied().zip(src.names).collect();
            self.slice_arg_targets.insert(key.clone(), pairs);
            key
        } else {
            String::new()
        };
        Some((param, caller, lo, hi))
    }

    /// `writeback_array_args` for a slice actual: each formal element goes
    /// back to the slice element it was paired with. False when `caller` is
    /// not such a key.
    pub(super) fn writeback_slice_arg(&mut self, param: &str, caller: &str) -> bool {
        if !caller.starts_with(SLICE_ARG_MARKER) {
            return false;
        }
        let Some(pairs) = self.slice_arg_targets.remove(caller) else {
            return true;
        };
        for (i, name) in pairs {
            let Some(name) = name else {
                continue;
            };
            if let Some(v) = self.get_signal_value_by_name(&format!("{param}[{i}]")) {
                self.set_signal_value_by_name(&name, v);
            }
        }
        true
    }

    /// §20.7: `$size` of a slice of an unpacked array is its element count,
    /// the width of `[b +: w]` or `|l - r| + 1`; it read the slice's width in
    /// bits. `None` when `e` is no such slice.
    pub(super) fn unpacked_slice_size(&mut self, e: &Expression) -> Option<i64> {
        let ExprKind::RangeSelect {
            kind, left, right, ..
        } = &e.kind
        else {
            return None;
        };
        self.unpacked_operand_elems(e).filter(|el| el.sliced)?;
        Some(match kind {
            RangeKind::Constant => {
                let (l, r) = (
                    self.eval_expr(left).to_i64()?,
                    self.eval_expr(right).to_i64()?,
                );
                (l - r).abs() + 1
            }
            RangeKind::IndexedUp | RangeKind::IndexedDown => self.eval_expr(right).to_i64()?,
        })
    }
}

/// The prefix of the caller key of a slice actual (see
/// `bind_slice_array_arg`); no storage name starts with it.
pub(super) const SLICE_ARG_MARKER: &str = "\u{1}slice-arg:";
