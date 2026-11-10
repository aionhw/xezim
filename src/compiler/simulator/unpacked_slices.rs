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
    /// array `name`, outermost first, and its element width. `None` for a
    /// dynamic, queue or associative array, or anything else.
    pub(super) fn unpacked_decl_shape(&self, name: &str) -> Option<(Vec<(i64, i64)>, u32)> {
        if self.module.dynamic_arrays.contains(name) || self.is_associative_array(name) {
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
        let ExprKind::Ident(h) = &cur.kind else {
            return None;
        };
        if h.root.is_some() || h.path.iter().any(|s| !s.selects.is_empty()) {
            return None;
        }
        let root_name = self.resolve_hier_name(h).into_owned();
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
