//! §19.7 `<covergroup>.option.<name>` on a covergroup held by a class.
//!
//! Inside the class, `cc.option.goal` and `this.cc.option.goal` parse as
//! member accesses, not as one hierarchical name; through a handle,
//! `o.cc.option.goal` is a name of four segments. Both reach the same
//! covergroup instance as `cg.option.goal` does at module level.

use super::*;

impl Simulator {
    /// The `option` field a hierarchical name reads or writes:
    /// `<path>.option.<name>` with at least one segment before `option`.
    pub(super) fn cg_option_ident_field(hier: &HierarchicalIdentifier) -> Option<&str> {
        let n = hier.path.len();
        (n >= 3 && hier.path[n - 2].name.name == "option")
            .then(|| hier.path[n - 1].name.name.as_str())
    }

    /// The covergroup instance and field of `base.option.<name>` written as
    /// member accesses. `None` unless the receiver is a plain path (a name,
    /// `this`, or members of those), so a receiver with a call or a select
    /// is never evaluated twice.
    pub(super) fn cg_option_member_target(
        &mut self,
        base: &Expression,
        member: &crate::ast::Identifier,
    ) -> Option<(usize, String)> {
        let ExprKind::MemberAccess {
            expr: recv,
            member: opt,
        } = &base.kind
        else {
            return None;
        };
        if opt.name != "option" || !Self::cg_plain_path(recv) {
            return None;
        }
        let h = self.eval_expr(recv).to_u64()? as usize;
        Some((self.cg_index(h)?, member.name.clone()))
    }

    fn cg_plain_path(e: &Expression) -> bool {
        match &e.kind {
            ExprKind::This => true,
            ExprKind::Ident(h) => h.path.iter().all(|s| s.selects.is_empty()),
            ExprKind::MemberAccess { expr, .. } => Self::cg_plain_path(expr),
            _ => false,
        }
    }
}
