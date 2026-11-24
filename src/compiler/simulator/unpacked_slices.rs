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
use crate::ast::types::SimpleType;

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

pub(super) struct UnpackedLayout<'e> {
    root: &'e Expression,
    root_name: String,
    dims: Vec<(i64, i64)>,
    ew: u32,
    steps: Vec<Step<'e>>,
}

impl UnpackedLayout<'_> {
    pub(super) fn object_member(&self) -> bool {
        self.root_name
            .split_once('#')
            .is_some_and(|(handle, _)| handle.parse::<usize>().is_ok())
    }
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
                Some(d) if d.len() == 2 => (d.to_vec(), ew),
                _ => (vec![r0, r1], ew),
            });
        }
        let (shape, ew) = self.module.arrays_nd.get(name)?;
        Some(match self.module.unpacked_decl_dims.get(name) {
            Some(d) if d.len() == shape.len() => (d.to_vec(), *ew),
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
        let layout = self.unpacked_operand_layout::<false>(e)?;
        self.unpacked_elems_from_layout(layout, false)
    }

    fn unpacked_operand_layout<'e, const FIXED_ROW: bool>(
        &mut self,
        e: &'e Expression,
    ) -> Option<UnpackedLayout<'e>> {
        let mut steps: Vec<Step> = Vec::new();
        let mut cur = e;
        let mut selects = 0;
        loop {
            match &cur.kind {
                ExprKind::Index { expr, index } => {
                    selects += 1;
                    if !FIXED_ROW {
                        steps.push(Step::Index(index));
                    }
                    cur = expr;
                }
                ExprKind::RangeSelect {
                    expr,
                    kind,
                    left,
                    right,
                } if !FIXED_ROW => {
                    steps.push(Step::Range(*kind, left, right));
                    cur = expr;
                }
                _ => break,
            }
        }
        steps.reverse();
        if FIXED_ROW {
            match &cur.kind {
                ExprKind::Ident(h) if h.path.iter().all(|seg| seg.selects.is_empty()) => {}
                ExprKind::MemberAccess { expr, .. }
                    if matches!(expr.kind, ExprKind::This)
                        || Self::cond_expr_is_effect_free(cur, false) => {}
                _ => return None,
            }
        }
        let plain_row_root = FIXED_ROW
            && matches!(&cur.kind, ExprKind::Ident(h) if h.root.is_none() && h.path.len() == 1)
            && !self.this_stack.last().is_some_and(Option::is_some)
            && !self.class_context_stack.last().is_some_and(Option::is_some);
        let scoped = if plain_row_root {
            None
        } else {
            self.expr_assoc_name(cur)
        };
        let root_name = scoped.or_else(|| {
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
            let name = local
                .map(std::borrow::Cow::Borrowed)
                .unwrap_or_else(|| self.resolve_hier_name(h));
            if FIXED_ROW
                && (self.fixed_concat_rank(&name) != Some(selects + 1)
                    || self.ident_shadowed_by_non_collection(h))
            {
                return None;
            }
            Some(name.into_owned())
        })?;
        if FIXED_ROW && !plain_row_root && self.fixed_concat_rank(&root_name) != Some(selects + 1) {
            return None;
        }
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
        if FIXED_ROW
            && (selects + 1 != dims.len()
                || self.module.dynamic_arrays.contains(&root_name)
                || self.module.queue_vars.contains(&root_name)
                || self.coll_shape_at(&root_name).is_some_and(|(shape, depth)| {
                    shape.dims.iter().skip(depth).any(|d| !matches!(d, Dim::Fixed(_)))
                })
                || self.concat_row_elem_type(&root_name).is_some_and(|dt| {
                    matches!(self.resolve_dt_ref(&dt), DataType::Struct(su) if Self::spreads_member_wise(su))
                }))
        {
            return None;
        }
        if FIXED_ROW {
            let mut cur = e;
            steps.reserve(selects);
            while let ExprKind::Index { expr, index } = &cur.kind {
                steps.push(Step::Index(index));
                cur = expr;
            }
            steps.reverse();
        }
        Some(UnpackedLayout {
            root: cur,
            root_name,
            dims,
            ew,
            steps,
        })
    }

    fn unpacked_elems_from_layout(
        &mut self,
        layout: UnpackedLayout<'_>,
        fixed_row: bool,
    ) -> Option<UnpackedElems> {
        let UnpackedLayout {
            root,
            root_name,
            dims,
            ew,
            steps,
        } = layout;
        let sliced = matches!(steps.last(), Some(Step::Range(..)));
        let selected = sliced || (!steps.is_empty() && steps.len() < dims.len());
        let mut names: Vec<Option<String>> = vec![Some(root_name.clone())];
        for (d, step) in steps.iter().enumerate() {
            let (l, r) = dims[d];
            let (lo, hi) = (l.min(r), l.max(r));
            let idx: Vec<i64> = match step {
                Step::Index(ix) => {
                    let v = self.eval_expr(ix);
                    let i = v.to_index();
                    if !fixed_row {
                        vec![i?]
                    } else if let Some(i) = i.filter(|&i| {
                        (v.is_signed || i >= 0)
                            && (v.width <= 64 || v.resize(64).resize(v.width) == v)
                    }) {
                        vec![i]
                    } else {
                        names.fill(None);
                        continue;
                    }
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
            if !fixed_row && names.len().saturating_mul(idx.len()) > 1 << 20 {
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
            root: root.clone(),
            root_name,
            names,
            ew,
            sliced,
            selected,
        })
    }

    /// §10.10: admit an indexed fixed operand with one remaining dimension
    /// before evaluating any selector.
    pub(super) fn fixed_row_concat_shape<'e>(
        &mut self,
        e: &'e Expression,
    ) -> Option<UnpackedLayout<'e>> {
        let e = Self::unparen(e);
        if !matches!(e.kind, ExprKind::Index { .. })
            || (self.module.arrays_2d.is_empty() && self.module.arrays_nd.is_empty())
        {
            return None;
        }
        self.unpacked_operand_layout::<true>(e)
    }

    /// Reject element selects before building a layout or allocating dimensions.
    fn fixed_concat_rank(&self, name: &str) -> Option<usize> {
        if self.module.arrays_2d.contains_key(name) {
            return Some(2);
        }
        if let Some((dims, _)) = self.module.arrays_nd.get(name) {
            return Some(dims.len());
        }
        let (handle, member) = name.split_once('#')?;
        let class = self.class_prop_declarer(handle.parse().ok()?, member)?;
        class
            .array_nd_properties
            .get(member)
            .map(|(dims, _)| dims.len())
    }

    /// §10.10: an array-valued destination element takes a compatible row whole.
    pub(super) fn concat_has_non_array_elements(&self, dst: &str) -> bool {
        self.coll_shape_at(dst)
            .is_none_or(|(shape, depth)| shape.dims.len() <= depth + 1)
    }

    pub(super) fn concat_element_type_is_array(&self, dt: &DataType) -> bool {
        let DataType::TypeReference {
            name, dimensions, ..
        } = dt
        else {
            return false;
        };
        let class = self
            .class_context_stack
            .last()
            .and_then(|c| c.as_deref())
            .unwrap_or("");
        dimensions.is_empty()
            && (Self::typedef_dims_via_tables(&self.module, class, &name.name.name).is_some()
                || (!name.scopes.is_empty()
                    && Self::typedef_dims_via_tables(&self.module, class, &name.qualified())
                        .is_some()))
    }

    fn concat_row_elem_type(&self, root: &str) -> Option<std::borrow::Cow<'_, DataType>> {
        self.class_member_coll_type(root)
            .map(std::borrow::Cow::Borrowed)
            .or_else(|| self.p_elem_type_ref(root))
    }

    pub(super) fn append_fixed_row_concat_values(
        &mut self,
        layout: UnpackedLayout<'_>,
        values: &mut Vec<Value>,
    ) {
        if let Some(row) = self.unpacked_elems_from_layout(layout, true) {
            self.append_unpacked_elem_values(&row, values);
        }
    }

    /// §7.4.5, §10.10: a slice of a fixed unpacked array or of one of its
    /// rows (`a[i +: 2]`, `w[3 -: 2]`, `m[1][0:1]`) contributes its elements
    /// left to right, in the order the range selects them. False when `e` is
    /// no such slice (a queue or dynamic-array slice keeps its own path).
    pub(super) fn append_fixed_slice_concat_values(
        &mut self,
        e: &Expression,
        values: &mut Vec<Value>,
    ) -> bool {
        // Decline before naming any element: a queue slice (`q[1:$]`) is the
        // common operand here, and it keeps its own path.
        let Some(layout) = self
            .unpacked_operand_layout::<false>(e)
            .filter(|l| matches!(l.steps.last(), Some(Step::Range(..))))
            .filter(|l| self.live_coll_size(&l.root_name).is_none())
        else {
            return false;
        };
        let Some(el) = self.unpacked_elems_from_layout(layout, false) else {
            return false;
        };
        self.append_unpacked_elem_values(&el, values);
        true
    }

    /// §10.10: a fixed one-dimensional array member of a structure
    /// (`s.a`, `sa[1].a`) contributes its elements left to right. False when
    /// `e` is no such member; a collection member keeps its own path.
    pub(super) fn append_member_array_concat_values(
        &mut self,
        e: &Expression,
        values: &mut Vec<Value>,
    ) -> bool {
        if !matches!(e.kind, ExprKind::MemberAccess { .. }) {
            return false;
        }
        let Some(layout) = self
            .unpacked_operand_layout::<false>(e)
            .filter(|l| l.steps.is_empty() && l.dims.len() == 1)
            .filter(|l| self.live_coll_size(&l.root_name).is_none())
        else {
            return false;
        };
        let Some(el) = self.unpacked_elems_from_layout(layout, false) else {
            return false;
        };
        let Some(first) = el.names.iter().flatten().next() else {
            return false;
        };
        if self.get_signal_value_by_name(first).is_none() {
            return false;
        }
        self.append_unpacked_elem_values(&el, values);
        true
    }

    /// The values of `el`'s elements; one outside the declared range reads
    /// as the element type's default (§7.4.5 Table 7-1).
    fn append_unpacked_elem_values(&self, el: &UnpackedElems, values: &mut Vec<Value>) {
        let mut missing = None;
        values.extend(el.names.iter().map(|name| {
            self.read_unpacked_elem(name.as_deref()).unwrap_or_else(|| {
                missing
                    .get_or_insert_with(|| self.unpacked_elem_default(&el.root_name, el.ew))
                    .clone()
            })
        }));
    }

    /// §7.4.5 Table 7-1: the value an invalid element of the fixed array
    /// `root` reads, by its element type.
    fn unpacked_elem_default(&self, root: &str, ew: u32) -> Value {
        let dt = self.concat_row_elem_type(root);
        match dt.as_deref().map(|dt| self.resolve_dt_ref(dt)) {
            Some(DataType::Simple {
                kind: SimpleType::String,
                ..
            }) => Value::from_string(""),
            Some(DataType::Simple {
                kind: SimpleType::Chandle | SimpleType::Event,
                ..
            }) => Value::zero(ew),
            Some(dt) if self.decl_is_virtual_iface(dt) => Value::zero(ew),
            Some(
                dt @ (DataType::IntegerAtom { .. }
                | DataType::IntegerVector { .. }
                | DataType::Struct(_)
                | DataType::Enum(_)),
            ) => {
                let types = &self.module.typedef_types;
                let mut value = if super::elaborate::is_type_two_state_resolved(dt, types) {
                    Value::zero(ew)
                } else {
                    Value::new(ew)
                };
                value.is_signed = super::elaborate::is_type_signed_resolved(dt, types);
                value
            }
            _ => self
                .coll_missing_elem_value(root)
                .unwrap_or_else(|| Value::new(ew)),
        }
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
                self.read_unpacked_elem(n.as_deref())
                    .unwrap_or_else(|| Value::new(el.ew))
            })
            .collect()
    }

    fn read_unpacked_elem(&self, name: Option<&str>) -> Option<Value> {
        name.and_then(|n| self.get_signal_value_by_name(n))
    }

    /// A fixed-array function result can target a selected row even when
    /// neither side has an explicit range select (§7.6, §13.4).
    fn selected_array_call(&self, lhs: &Expression, rhs: &Expression) -> bool {
        matches!(lhs.kind, ExprKind::Index { .. } | ExprKind::RangeSelect { .. })
            && matches!(&Self::unparen(rhs).kind, ExprKind::Call { func, .. }
                if self.call_returns_collection(func))
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
        if !Self::is_range_select_chain(lvalue)
            && !Self::is_range_select_chain(rvalue)
            && !self.selected_array_call(lvalue, rvalue)
        {
            return false;
        }
        let Some(dst) = self.unpacked_value_elems(lvalue) else {
            return false;
        };
        // §10.9.1: a slice is an unpacked target, so a pattern supplies one
        // item per selected element in left-to-right order, not packed bits.
        if let Some(values) = self.unpacked_slice_rhs_values(&dst, rvalue) {
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

    /// §10.10, §7.6: `dst = {…}` where `dst` is a one-dimensional fixed
    /// unpacked array that the whole-name store does not reach: a row of a
    /// multidimensional array (`m[1]`) or a structure member (`s.a`,
    /// `sa[1].a`). The items' elements fill the target from its left bound;
    /// every item is read before any element is written. False for any other
    /// target, including a slice, a collection and an array of unpacked
    /// structures (whose elements are member leaves).
    pub(super) fn assign_unpacked_concat_elems(
        &mut self,
        lvalue: &Expression,
        rvalue: &Expression,
    ) -> bool {
        let ExprKind::Concatenation(parts) = &rvalue.kind else {
            return false;
        };
        // A row needs a multidimensional array; a member, a structure or an
        // object.
        match lvalue.kind {
            ExprKind::Index { .. }
                if !(self.module.arrays_2d.is_empty() && self.module.arrays_nd.is_empty()) => {}
            ExprKind::MemberAccess { .. } => {}
            _ => return false,
        }
        let Some(layout) = self
            .unpacked_operand_layout::<false>(lvalue)
            .filter(|l| l.dims.len() == l.steps.len() + 1)
            .filter(|l| !l.steps.iter().any(|s| matches!(s, Step::Range(..))))
            .filter(|l| self.live_coll_size(&l.root_name).is_none())
        else {
            return false;
        };
        // Element selects run once here; the stores below reuse the names.
        let Some(dst) = self.unpacked_elems_from_layout(layout, false) else {
            return false;
        };
        let Some(first) = dst.names.iter().flatten().next() else {
            return false;
        };
        if self.get_signal_value_by_name(first).is_none() {
            return false;
        }
        let values = self.unpacked_concat_operand_values(parts.iter().collect(), true, None);
        for (name, v) in dst.names.iter().zip(values) {
            if let Some(name) = name {
                self.set_signal_value_by_name(name, v);
            }
        }
        true
    }

    /// Snapshot every pattern item before storing a slice: an item can read
    /// another selected element, and the two sides may overlap (§10.9.1).
    fn unpacked_slice_rhs_values(
        &mut self,
        dst: &UnpackedElems,
        rvalue: &Expression,
    ) -> Option<Vec<Value>> {
        // A returned array is already snapshotted in declaration order.
        // Once the call has run, a missing or differently sized result
        // consumes the assignment without evaluating the call again.
        let rhs = Self::unparen(rvalue);
        if dst.selected && matches!(rhs.kind, ExprKind::Call { .. }) {
            let values = self.unpacked_item_values(rhs)?;
            return Some(if values.len() == dst.names.len() { values } else { Vec::new() });
        }
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
        let dt = self
            .class_member_coll_type(&dst.root_name)
            .cloned()
            .or_else(|| self.p_elem_type(&dst.root_name));
        let elems = match dt.as_ref() {
            Some(dt) => self.pattern_elems_for_type(items, &indices, dt),
            None => self.pattern_elems(items, &indices),
        };
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
        if !Self::is_range_select_chain(lvalue)
            && !Self::is_range_select_chain(rvalue)
            && !self.selected_array_call(lvalue, rvalue)
        {
            return None;
        }
        let dst = self.unpacked_value_elems(lvalue)?;
        if let Some(values) = self.unpacked_slice_rhs_values(&dst, rvalue) {
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
