//! An unpacked array concatenation, or a whole queue, stored to a collection
//! by a nonblocking assignment or after an intra-assignment timing control
//! (IEEE 1800-2023 §10.10, §10.4.2, §9.4.5).
//!
//! The blocking store splices such a right-hand side element by element. A
//! nonblocking or delayed store evaluated it as one packed value instead, so
//! `q <= {q, 1}` and `q = #1 {q, 1}` kept one element. Here the elements are
//! read when the statement runs and saved; the store is the blocking one,
//! run in the NBA region or after the timing control, over a concatenation
//! of the saved values.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

/// A collection right-hand side read when its statement ran: a
/// concatenation of saved values, and the saved-value slots it holds.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct CapturedCollection {
    pub(super) rhs: Expression,
    pub(super) slots: Vec<u64>,
}

impl Simulator {
    /// The collection the blocking store writes for `lvalue` when the
    /// right-hand side is an unpacked array concatenation, and whether it is
    /// a queue or dynamic array. `None` for any other target, for an
    /// associative array and for a collection whose elements are arrays.
    fn concat_store_target(&mut self, lvalue: &Expression) -> Option<(String, bool)> {
        let ExprKind::Ident(h) = &lvalue.kind else {
            return None;
        };
        if self.ident_shadowed_by_non_collection(h) {
            return None;
        }
        let mut name = self.resolve_hier_name(h).into_owned();
        if let Some(s) = self.instance_assoc_member(&name) {
            name = s;
        }
        let dynamic = if self.module.arrays.contains_key(name.as_str()) {
            self.module.dynamic_arrays.contains(name.as_str())
        } else {
            name = self
                .expr_assoc_name(lvalue)
                .filter(|n| self.class_queue_prop(n))?;
            true
        };
        if self.is_associative_array(&name) || self.foreach_dims(&name).is_some_and(|d| d.len() > 1)
        {
            return None;
        }
        Some((name, dynamic))
    }

    /// §10.10, §9.4.5: read `rhs` now for a store to `lvalue` that runs
    /// later, in the NBA region or after a timing control, when the
    /// blocking store would splice it: an unpacked array concatenation into
    /// a collection, or a whole queue or array into a queue or dynamic
    /// array. Every element is read in the blocking store's order.
    pub(super) fn capture_collection_rhs(
        &mut self,
        lvalue: &Expression,
        rhs: &Expression,
    ) -> Option<CapturedCollection> {
        fn flatten<'a>(exprs: &'a [Expression], out: &mut Vec<&'a Expression>) {
            for e in exprs {
                match &e.kind {
                    ExprKind::Concatenation(inner) => flatten(inner, out),
                    _ => out.push(e),
                }
            }
        }
        let mut inner = rhs;
        while let ExprKind::Paren(e) = &inner.kind {
            inner = e;
        }
        let (_, dynamic) = self.concat_store_target(lvalue)?;
        let items: Vec<&Expression> = match &inner.kind {
            ExprKind::Concatenation(exprs) => {
                let mut out = Vec::new();
                flatten(exprs, &mut out);
                out
            }
            _ if dynamic && self.array_operand_name(inner).is_some() => vec![inner],
            _ => return None,
        };
        // §7.8: an associative operand is one whole element; the blocking
        // store copies its entries, which a saved value cannot carry.
        if !self.module.associative_arrays.is_empty()
            && items.iter().any(|e| self.assoc_value_operand(e).is_some())
        {
            return None;
        }
        let span = inner.span;
        let vals = self.unpacked_concat_operand_values(items);
        let mut slots = Vec::with_capacity(vals.len());
        let mut exprs = Vec::with_capacity(vals.len());
        for v in vals {
            let idx = self.intra_saved_next;
            self.intra_saved_next += 1;
            self.intra_saved.insert(idx, v);
            slots.push(idx);
            exprs.push(Expression::new(
                ExprKind::SystemCall {
                    name: INTRA_KEPT_MARKER.to_string(),
                    args: vec![Self::intra_slot_literal(idx, span)],
                },
                span,
            ));
        }
        Some(CapturedCollection {
            rhs: Expression::new(ExprKind::Concatenation(exprs), span),
            slots,
        })
    }

    /// Store a collection captured by `capture_collection_rhs` to `lhs`
    /// with the blocking store, then free its saved values.
    pub(super) fn store_captured_collection(&mut self, lhs: Expression, cap: CapturedCollection) {
        let span = lhs.span;
        self.exec_statement(&Statement::new(
            StatementKind::BlockingAssign {
                lvalue: lhs,
                rvalue: cap.rhs,
            },
            span,
        ));
        for idx in &cap.slots {
            self.intra_saved.remove(idx);
        }
    }
}
