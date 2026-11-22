//! An assignment pattern item whose value is an unpacked array (IEEE
//! 1800-2023 §10.9.1, §10.10).
//!
//! An item matched to a sub-array (`'{wc, r}` on `int w [2][2]`, a `default:`
//! value, a function returning an array, a row `m[1]`) supplies that
//! sub-array's elements. The pattern store evaluated such an item as one
//! scalar and wrote it to every element below, so the rows read 0.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    /// Whether `e` can name or return an unpacked array; a cheap filter run
    /// before any lookup, since every scalar fill comes through here.
    fn may_be_array_item(e: &Expression) -> bool {
        matches!(
            e.kind,
            ExprKind::Ident(_)
                | ExprKind::Index { .. }
                | ExprKind::RangeSelect { .. }
                | ExprKind::MemberAccess { .. }
                | ExprKind::Call { .. }
        )
    }

    /// §10.9.1, §10.10: store the unpacked array value `e` to the sub-array
    /// `target` whose dimensions list their indices in declared order, left
    /// bound first. Every source element is read before any is written, and
    /// each write goes through the pattern leaf store, so a nonblocking
    /// pattern still collects it; an element that is an unpacked struct is
    /// written member by member. False, with nothing written, when `e` is
    /// not an unpacked array or its element count differs; the caller then
    /// fills the elements with `e`.
    pub(super) fn assign_array_item(
        &mut self,
        target: &str,
        lists: &[Vec<i64>],
        dt: &DataType,
        e: &Expression,
    ) -> bool {
        let e = Self::unparen(e);
        if lists.is_empty() || !Self::may_be_array_item(e) {
            return false;
        }
        let cells: usize = lists.iter().map(Vec::len).product();
        // §10.9.1, §7.6: a slice (`big[1:2]`), or an array whose elements
        // are unpacked structs (`'{sa, sa}`), supplies its elements too,
        // each struct member by member. A slice repeated its first element
        // and a struct array stored zeros.
        if !matches!(e.kind, ExprKind::Call { .. }) {
            let capture = self.pattern_nba_leaves.take();
            let leaves = self.unpacked_array_leaves(e);
            self.pattern_nba_leaves = capture;
            if let Some(leaves) = leaves {
                let names = Self::cell_names(target, lists);
                let mut writes = Vec::with_capacity(leaves.len());
                let mut elem = None::<(String, usize)>;
                for (path, v) in leaves {
                    let split = path.find('.').unwrap_or(path.len());
                    let ord = match &elem {
                        Some((p, k)) if *p == path[..split] => *k,
                        Some((_, k)) => k + 1,
                        None => 0,
                    };
                    let Some(dst) = names.get(ord) else {
                        return false;
                    };
                    writes.push((format!("{dst}{}", &path[split..]), v));
                    elem = Some((path[..split].to_string(), ord));
                }
                if elem.is_none_or(|(_, k)| k + 1 != cells) {
                    return false;
                }
                for (name, v) in writes {
                    self.write_pattern_leaf(&name, v);
                }
                return true;
            }
        }
        if let DataType::Struct(su) = self.resolve_dt(dt)
            && Self::spreads_member_wise(&su)
        {
            return false;
        }
        let capture = self.pattern_nba_leaves.take();
        let vals = self.unpacked_item_values(e);
        self.pattern_nba_leaves = capture;
        let Some(vals) = vals else {
            return false;
        };
        if vals.len() != cells {
            // A call has already run; filling with it would run it again.
            return matches!(e.kind, ExprKind::Call { .. });
        }
        for (name, v) in Self::cell_names(target, lists).iter().zip(vals) {
            self.write_pattern_leaf(name, v);
        }
        true
    }

    /// The element names of the sub-array `target` whose dimensions list
    /// their indices in declared order, left bound first.
    fn cell_names(target: &str, lists: &[Vec<i64>]) -> Vec<String> {
        let mut names = vec![target.to_string()];
        for list in lists {
            names = names
                .iter()
                .flat_map(|p| list.iter().map(move |i| format!("{p}[{i}]")))
                .collect();
        }
        names
    }

    /// §9.4.5: the elements of the unpacked array named by `e` (a whole
    /// array or a row of one, not a call) read now, each with its leaf path
    /// below `e` (`[1][0]`, or `[1].a` under an element that is an unpacked
    /// struct), in declared order. None when `e` names no unpacked array.
    pub(super) fn array_item_elements(&mut self, e: &Expression) -> Option<Vec<(String, Value)>> {
        let e = Self::unparen(e);
        if !Self::may_be_array_item(e) || matches!(e.kind, ExprKind::Call { .. }) {
            return None;
        }
        let capture = self.pattern_nba_leaves.take();
        let out = self.array_item_elements_now(e);
        self.pattern_nba_leaves = capture;
        out.filter(|v| !v.is_empty() && v.iter().all(|(s, _)| s.starts_with('[')))
    }

    /// §7.6, §7.4.6: the leaves of the unpacked array `e`, a whole fixed
    /// array, a subarray (`m[1]`, `m3[1][0]`) or a slice (`a[1:2]`,
    /// `m[1][0+:2]`) of a design variable, a local, a formal or a class
    /// property, read now in declared order, left bound first. Each leaf
    /// carries its position below `e`, one `[k]` per dimension counted from
    /// the left bound, then `.member` steps under an element that is an
    /// unpacked struct (whose members are separate leaves). An element
    /// outside the declared range reads as x. None when `e` is anything else
    /// (a call, a single element, a dynamic or associative array).
    pub(super) fn unpacked_array_leaves(&mut self, e: &Expression) -> Option<Vec<(String, Value)>> {
        let e = Self::unparen(e);
        // A select with a side effect anywhere in the chain (`m[i++]`, or
        // `objs[i++].v` below a member, §11.4.2) is left to the caller's
        // single evaluation: resolving the operand here would run it first.
        if !Self::cond_expr_is_effect_free(e, false) {
            return None;
        }
        let mut cur = e;
        let mut sliced = false;
        if let ExprKind::RangeSelect { expr, .. } = &cur.kind {
            sliced = true;
            cur = expr;
        }
        let mut fixed = 0usize;
        while let ExprKind::Index { expr, .. } = &cur.kind {
            fixed += 1;
            cur = expr;
        }
        if !matches!(cur.kind, ExprKind::Ident(_) | ExprKind::MemberAccess { .. }) {
            return None;
        }
        // A plain design name, with no class object or subroutine local to
        // resolve it otherwise, is answered from the declared ranks: every
        // scalar fill and element read of a pattern store comes through here.
        if let ExprKind::Ident(h) = &cur.kind
            && h.root.is_none()
            && h.path.len() == 1
            && h.path[0].selects.is_empty()
            && self.no_class_objects()
            && self.dyn_name_lookup(&h.path[0].name.name).is_none()
        {
            let name = self.resolve_hier_name(h);
            let rank = if self.module.arrays.contains_key(&*name) {
                1
            } else if self.module.arrays_2d.contains_key(&*name) {
                2
            } else {
                self.module.arrays_nd.get(&*name).map_or(0, |(s, _)| s.len())
            };
            if rank == 0 || fixed == rank || fixed + usize::from(sliced) > rank {
                return None;
            }
        }
        let el = self.unpacked_operand_elems(e)?;
        if !el.selected && (fixed > 0 || sliced) {
            return None;
        }
        let (dims, _) = self.unpacked_decl_shape(&el.root_name)?;
        if self.live_coll_size(&el.root_name).is_some() {
            return None;
        }
        // The extent of each dimension below `e`: a slice's own length for
        // the sliced one.
        let mut extents: Vec<usize> = dims
            .iter()
            .skip(fixed)
            .map(|&(l, r)| l.abs_diff(r) as usize + 1)
            .collect();
        if sliced {
            let below: usize = extents.iter().skip(1).product();
            *extents.first_mut()? = el.names.len() / below.max(1);
        }
        if extents.iter().product::<usize>() != el.names.len() {
            return None;
        }
        let elem_su = self
            .class_member_coll_type(&el.root_name)
            .cloned()
            .or_else(|| self.p_elem_type(&el.root_name))
            .and_then(|dt| self.unpacked_struct_of(&dt));
        let mut out = Vec::new();
        for (pos, name) in el.names.iter().enumerate() {
            let mut path = String::new();
            let mut rest = pos;
            let mut steps = Vec::with_capacity(extents.len());
            for &n in extents.iter().rev() {
                steps.push(rest % n);
                rest /= n;
            }
            for k in steps.iter().rev() {
                path.push_str(&format!("[{k}]"));
            }
            match &elem_su {
                Some(su) => {
                    let Some(name) = name else {
                        return None;
                    };
                    let mut leaves = Vec::new();
                    self.unpacked_struct_leaves(name, e, su, 0, &mut leaves);
                    for (key, _, w, _) in leaves {
                        let v = self
                            .get_signal_value_by_name(&key)
                            .unwrap_or_else(|| Value::new(w.max(1)));
                        out.push((format!("{path}{}", &key[name.len()..]), v));
                    }
                }
                None => {
                    let v = name
                        .as_ref()
                        .and_then(|n| self.get_signal_value_by_name(n))
                        .unwrap_or_else(|| Value::new(el.ew));
                    out.push((path, v));
                }
            }
        }
        (!out.is_empty()).then_some(out)
    }

    fn array_item_elements_now(&mut self, e: &Expression) -> Option<Vec<(String, Value)>> {
        if let Some(leaves) = self.unpacked_array_leaves(e) {
            return Some(leaves);
        }
        if let Some(suffixes) = self.unpacked_row_suffixes(e) {
            return Some(
                suffixes
                    .into_iter()
                    .map(|s| {
                        let v = self.eval_expr(&Self::append_leaf_path(e, &s));
                        (s, v)
                    })
                    .collect(),
            );
        }
        let src = self.foreach_check_operand_name(e)?;
        if self.is_associative_array(&src) {
            return None;
        }
        let (names, _) = self.stream_elem_order(&src)?;
        names
            .iter()
            .map(|n| {
                let v = self.get_signal_value_by_name(n)?;
                Some((n.get(src.len()..)?.to_string(), v))
            })
            .collect()
    }

    /// The elements of the unpacked array `e` in declared order, left bound
    /// first and rows before columns: a whole array or a row of one, a queue
    /// or dynamic array, or a call returning an unpacked array (which runs
    /// once). None for anything else.
    pub(super) fn unpacked_item_values(&mut self, e: &Expression) -> Option<Vec<Value>> {
        if matches!(e.kind, ExprKind::Call { .. }) {
            // `Some(None)`: the call ran and returned no collection.
            let Some(src) = self.ret_collection_operand(e)? else {
                return Some(Vec::new());
            };
            if self.is_associative_array(&src) {
                return Some(Vec::new());
            }
            let values: Option<Vec<_>> = (0..self.get_queue_size(&src))
                .map(|j| self.get_signal_value_by_name(&format!("{src}[{j}]")))
                .collect();
            return Some(values.unwrap_or_default());
        }
        if let Some(suffixes) = self.unpacked_row_suffixes(e) {
            return Some(
                suffixes
                    .iter()
                    .map(|s| {
                        let leaf = Self::append_leaf_path(e, s);
                        self.eval_expr(&leaf)
                    })
                    .collect(),
            );
        }
        let src = self.foreach_check_operand_name(e)?;
        if self.is_associative_array(&src) {
            return None;
        }
        let (names, _) = self.stream_elem_order(&src)?;
        names
            .iter()
            .map(|n| self.get_signal_value_by_name(n))
            .collect()
    }
}
