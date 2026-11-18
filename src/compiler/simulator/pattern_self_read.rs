//! A blocking assignment of a pattern whose items read the target itself
//! (IEEE 1800-2023 §10.4.1, §10.9.1, §10.9.2).
//!
//! The right-hand side is evaluated before the target is written, so
//! `a = '{a[1], a[0]}` swaps. The pattern store evaluated each item just
//! before writing its element, so a later item read an element already
//! overwritten and `a` became `'{2, 2}`.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

/// How a self-reading pattern is stored.
pub(super) enum SelfRead {
    /// A queue, dynamic array or one-dimensional fixed array: the items are
    /// read in element order into a pattern of saved values.
    Elements,
    /// Other fixed storage: the store runs with its leaf writes collected,
    /// then the leaves are written.
    Leaves,
}

impl Simulator {
    /// Whether the blocking store of `rvalue` to `lvalue` must read every
    /// item before writing: `rvalue` is a pattern and some item names the
    /// target variable.
    pub(super) fn self_reading_pattern(
        &mut self,
        lvalue: &Expression,
        rvalue: &Expression,
    ) -> Option<SelfRead> {
        if !matches!(rvalue.kind, ExprKind::AssignmentPattern(_)) {
            return None;
        }
        let mut root = lvalue;
        while let ExprKind::Index { expr, .. } | ExprKind::MemberAccess { expr, .. } = &root.kind {
            root = expr;
        }
        let ExprKind::Ident(h) = &root.kind else {
            return None;
        };
        let var = h.path.first()?.name.name.clone();
        if !Self::pattern_reads_var(rvalue, &var) {
            return None;
        }
        let name = self.resolve_hier_name(h).into_owned();
        if self.is_associative_array(&name) {
            return None;
        }
        if self.module.dynamic_arrays.contains(&name) || self.module.queue_vars.contains(&name) {
            return Some(SelfRead::Elements);
        }
        // A string leaf is stored without the width resize of a leaf write.
        if let Some(dt) = self.module.var_decl_types.get(&name)
            && Self::is_string_data_type(self.resolve_dt_ref(dt))
        {
            return None;
        }
        let one_dim = matches!(&lvalue.kind, ExprKind::Ident(_))
            && !self.module.arrays_2d.contains_key(&name)
            && !self.module.arrays_nd.contains_key(&name);
        Some(if one_dim {
            SelfRead::Elements
        } else {
            SelfRead::Leaves
        })
    }

    /// `e`, a pattern, reads `var` in one of its items or keys.
    fn pattern_reads_var(e: &Expression, var: &str) -> bool {
        use crate::ast::expr::AssignmentPatternItem as Item;
        match &e.kind {
            ExprKind::Paren(inner) => Self::pattern_reads_var(inner, var),
            ExprKind::AssignmentPattern(items) => items.iter().any(|item| {
                Self::pattern_reads_var(item.expr(), var)
                    || matches!(item, Item::Keyed(k, _) if Self::expr_reads_var(k, var))
            }),
            ExprKind::Replication { count, exprs } => {
                Self::expr_reads_var(count, var)
                    || exprs.iter().any(|x| Self::pattern_reads_var(x, var))
            }
            _ => Self::expr_reads_var(e, var),
        }
    }

    /// Run the pattern store `store` with its leaf writes collected, then
    /// write them in order: every item is read before any element changes.
    /// An enclosing collection (a nonblocking pattern) receives the leaves.
    pub(super) fn store_pattern_after_reads(
        &mut self,
        store: impl FnOnce(&mut Self) -> bool,
    ) -> bool {
        let outer = self.pattern_nba_leaves.replace(Vec::new());
        let stored = store(self);
        let leaves = self.pattern_nba_leaves.take().unwrap_or_default();
        self.pattern_nba_leaves = outer;
        if stored {
            for (name, value) in leaves {
                match &mut self.pattern_nba_leaves {
                    Some(outer) => outer.push((name, value)),
                    None => self.write_leaf_by_name(&name, value),
                }
            }
        }
        stored
    }
}
