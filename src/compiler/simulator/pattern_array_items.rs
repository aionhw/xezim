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
                | ExprKind::MemberAccess { .. }
                | ExprKind::Call { .. }
        )
    }

    /// `assign_array_item` over the sub-array spanning `dims` (normalized
    /// bounds); `descending[k]` puts dimension `k`'s left bound at `hi`.
    pub(super) fn assign_array_item_dims(
        &mut self,
        target: &str,
        dims: &[(i64, i64)],
        descending: &[bool],
        dt: &DataType,
        e: &Expression,
    ) -> bool {
        let e = Self::unparen(e);
        if dims.is_empty() || !Self::may_be_array_item(e) {
            return false;
        }
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
        self.assign_array_item(target, &lists, dt, e)
    }

    /// §10.9.1, §10.10: store the unpacked array value `e` to the sub-array
    /// `target` whose dimensions list their indices in declared order, left
    /// bound first. Every source element is read before any is written, and
    /// each write goes through the pattern leaf store, so a nonblocking
    /// pattern still collects it. False, with nothing written, when `e` is
    /// not an unpacked array, its element count differs or the elements are
    /// unpacked structs; the caller then fills the elements with `e`.
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
        let cells: usize = lists.iter().map(Vec::len).product();
        if vals.len() != cells {
            // A call has already run; filling with it would run it again.
            return matches!(e.kind, ExprKind::Call { .. });
        }
        let mut names = vec![target.to_string()];
        for list in lists {
            names = names
                .iter()
                .flat_map(|p| list.iter().map(move |i| format!("{p}[{i}]")))
                .collect();
        }
        for (name, v) in names.iter().zip(vals) {
            self.write_pattern_leaf(name, v);
        }
        true
    }

    /// §9.4.5: the elements of the unpacked array named by `e` (a whole
    /// array or a row of one, not a call) read now, each with its leaf path
    /// below `e` (`[1][0]`), in declared order. None when `e` names no
    /// unpacked array or its elements are unpacked structs.
    pub(super) fn array_item_elements(&mut self, e: &Expression) -> Option<Vec<(String, Value)>> {
        let e = Self::unparen(e);
        if !Self::may_be_array_item(e) || matches!(e.kind, ExprKind::Call { .. }) {
            return None;
        }
        let capture = self.pattern_nba_leaves.take();
        let out = self.array_item_elements_now(e);
        self.pattern_nba_leaves = capture;
        out.filter(|v| !v.is_empty() && v.iter().all(|(s, _)| !s.contains('.')))
    }

    fn array_item_elements_now(&mut self, e: &Expression) -> Option<Vec<(String, Value)>> {
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
    fn unpacked_item_values(&mut self, e: &Expression) -> Option<Vec<Value>> {
        if matches!(e.kind, ExprKind::Call { .. }) {
            // `Some(None)`: the call ran and returned no collection.
            let Some(src) = self.ret_collection_operand(e)? else {
                return Some(Vec::new());
            };
            if self.is_associative_array(&src) {
                return Some(Vec::new());
            }
            return (0..self.get_queue_size(&src))
                .map(|j| self.get_signal_value_by_name(&format!("{src}[{j}]")))
                .collect();
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
