//! Simulator assignments.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

mod element_stores;
mod forces;
mod indexed;
mod lvalue_dispatch;
mod packed_slices;
mod ref_formals;
mod target_names;

impl Simulator {
    pub(super) fn assign_value(&mut self, lhs: &Expression, val: &Value) -> bool {
        // §20.2: a function can terminate simulation while its caller is
        // evaluating an assignment's RHS. Do not commit that caller's write.
        // Final procedures run with `finished` cleared (§9.2.3).
        if self.finished {
            return false;
        }
        if matches!(&lhs.kind, ExprKind::MemberAccess { expr, .. }
                if !matches!(&expr.kind, ExprKind::Ident(h) if h.path.len() == 1))
            || matches!(&lhs.kind, ExprKind::Ident(h) if h.path.len() >= 3)
        {
            // §8.14/§8.23: a shadowed member names what the reference's
            // declared class sees (`this.x` in a base method is the base's
            // `x`), so resolve that before a static of the OBJECT's class can
            // claim the write (#944).
            if !self.shadowed_prop_names.is_empty()
                && let Some(rewritten) = self.super_rewrite(lhs)
            {
                return self.assign_value(&rewritten, val);
            }
            if let Some(ok) = self.assign_static_through_chain(lhs, val) {
                return ok;
            }
        }
        // §8.14: the same for a select of a shadowed member (`this.v[3:0]`).
        if matches!(
            lhs.kind,
            ExprKind::Index { .. } | ExprKind::RangeSelect { .. }
        ) && !self.shadowed_prop_names.is_empty()
            && let Some(rewritten) = self.shadowed_select_rewrite(lhs)
        {
            return self.assign_value(&rewritten, val);
        }
        if let ExprKind::MemberAccess { expr: b, .. }
        | ExprKind::Index { expr: b, .. }
        | ExprKind::RangeSelect { expr: b, .. } = &lhs.kind
            && matches!(
                b.kind,
                ExprKind::Call { .. } | ExprKind::MemberAccess { .. }
            )
            && let Some(changed) = self.recv_call_once_assign(lhs, val)
        {
            return changed;
        }
        if !self.static_local_names.is_empty()
            && !self.in_const_param_eval
            && let Some((n, key)) = self.static_select_target(lhs)
        {
            return self.assign_static_select(n, &key, lhs, val);
        }
        // §6.8/§6.11.1: a write to a 2-state procedural local drops X/Z
        // (whole, bit- or part-select). Probed only for an X/Z value.
        let changed = if val.has_xz() {
            self.assign_value_xz(lhs, val)
        } else {
            self.assign_value_inner(lhs, val)
        };
        if changed && !self.condition_waiters.is_empty() {
            self.check_condition_waiters_for_write(lhs);
        }
        changed
    }

    /// §6.21: `assign_value` for a select or member write to the `static`
    /// subroutine local `name`, stored under `key` (`a[7:0] = v`; see
    /// `static_select_target`).
    /// The write updates the frame copy in place, while reads come from the
    /// live shared cell, so bring the copy up to date first and store it
    /// back after, or the write is lost.
    #[inline(never)]
    pub(super) fn assign_static_select(
        &mut self,
        name: &str,
        key: &str,
        lhs: &Expression,
        val: &Value,
    ) -> bool {
        // An index that calls a function can write the same static, so
        // evaluate the indices (once, §11.4.1) before taking the copy. An
        // x/z index selects nothing, so the write is discarded (§11.5.1).
        let frozen;
        let lhs = if Self::select_indices_plain(lhs) {
            lhs
        } else {
            let mut bad = false;
            frozen = self.freeze_select_indices(lhs, &mut bad);
            if bad {
                return false;
            }
            &frozen
        };
        self.static_cell_sync(name, key, false);
        let changed = if val.has_xz() {
            self.assign_value_xz(lhs, val)
        } else {
            self.assign_value_inner(lhs, val)
        };
        self.static_cell_sync(name, key, true);
        if changed && !self.condition_waiters.is_empty() {
            self.check_condition_waiters_for_write(lhs);
        }
        changed
    }

    /// `assign_value_inner` for a value carrying X/Z: a 2-state procedural
    /// local target (`lvalue_is_two_state_local`) receives it with X/Z
    /// mapped to 0.
    #[cold]
    #[inline(never)]
    pub(super) fn assign_value_xz(&mut self, lhs: &Expression, val: &Value) -> bool {
        // §7.2.1: a 2-state member of a 4-state packed struct.
        if !self.mixed_packed_vars.is_empty()
            && let Some(v) = self.mixed_packed_member_write(lhs, val)
        {
            return self.assign_value_inner(lhs, &v);
        }
        if self.lvalue_is_two_state_local(lhs) {
            let v = val.to_two_state();
            self.assign_value_inner(lhs, &v)
        } else {
            self.assign_value_inner(lhs, val)
        }
    }

    /// Does `lhs` (a bare local, or a bit/part/element select of one) write
    /// a procedural local declared with a 2-state type? Its declared type is
    /// the latest `var_decl_types` entry for the name (every declaration
    /// replaces it; a subroutine's are restored on return), so nothing is
    /// paid per declaration — only a write carrying X/Z asks. Table signals
    /// carry their own `signal_two_state` mark, a class property or module
    /// variable seen from a subroutine is no local, and a fallback's carried
    /// locals are fitted on copy-back.
    pub(super) fn lvalue_is_two_state_local(&self, lhs: &Expression) -> bool {
        let mut root = lhs;
        loop {
            match &root.kind {
                ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => root = expr,
                ExprKind::Paren(inner) => root = inner,
                _ => break,
            }
        }
        let ExprKind::Ident(h) = &root.kind else {
            return false;
        };
        if h.root.is_some() || h.path.len() != 1 {
            return false;
        }
        let name = h.path[0].name.name.as_str();
        let in_frame = self
            .local_stack
            .last()
            .is_some_and(|f| f.contains_key(name));
        if in_frame {
            if self.fb_frame_depths.last().copied() == Some(self.local_stack.len() - 1) {
                // A fallback's carried local: its declared kind travelled
                // with it (`fb_frame_push`).
                let start = self.fb_two_state_marks.last().copied().unwrap_or(0);
                return self.fb_two_state_names[start..]
                    .iter()
                    .any(|n| n.as_ref() == name);
            }
        } else if !self.in_process_block_scope()
            || self.signal_name_to_id.contains_key(name)
            || !(self.signals.contains_key(name) || self.module.arrays.contains_key(name))
        {
            // Inside a subroutine every local is in its frame: a name
            // outside it is a class property or a module variable. (A
            // process block's static locals stay in the signal table under
            // a loop's variable frame, §6.21.)
            return false;
        }
        self.module.var_decl_types.get(name).is_some_and(|dt| {
            !self.dt_is_real(dt)
                && super::elaborate::is_type_two_state_resolved(dt, &self.module.typedef_types)
        })
    }

    #[inline]
    pub(super) fn coerce_bit_two_state(b: LogicBit, two_state: bool) -> LogicBit {
        if two_state && matches!(b, LogicBit::X | LogicBit::Z) {
            LogicBit::Zero
        } else {
            b
        }
    }

    pub(super) fn name_is_two_state(&self, name: &str) -> bool {
        if let Some(&id) = self.signal_name_to_id.get(name) {
            return self.signal_two_state.get(id).copied().unwrap_or(false);
        }
        let lookup = |key: &str| {
            self.module.var_decl_types.get(key).is_some_and(|dt| {
                !self.dt_is_real(dt)
                    && super::elaborate::is_type_two_state_resolved(
                        dt,
                        &self.module.typedef_types,
                    )
            })
        };
        if lookup(name) {
            return true;
        }
        if let Some(dt) = self.struct_leaf_data_type(name) {
            return !self.dt_is_real(&dt)
                && super::elaborate::is_type_two_state_resolved(
                    &dt,
                    &self.module.typedef_types,
                );
        }
        false
    }

    pub(super) fn lvalue_class_prop_two_state(&mut self, lhs: &Expression) -> bool {
        let mut cur = match &lhs.kind {
            ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => expr.as_ref(),
            _ => return false,
        };
        while let ExprKind::Index { expr: b, .. } = &cur.kind {
            cur = b.as_ref();
        }
        let Some((h, prop)) = self.prop_chain_root(cur) else {
            return false;
        };
        self.class_prop_two_state(h, &prop)
    }

    pub(super) fn struct_leaf_data_type(&self, name: &str) -> Option<DataType> {
        let dot_pos = name.find('.')?;
        fn strip_idx(s: &str) -> &str {
            s.split('[').next().unwrap_or(s)
        }
        let mut root_end = None;
        let first = strip_idx(&name[..dot_pos]);
        if self.module.var_decl_types.contains_key(first) {
            root_end = Some(dot_pos);
        } else {
            for (idx, _) in name.match_indices('.') {
                let candidate = strip_idx(&name[..idx]);
                if self.module.var_decl_types.contains_key(candidate) {
                    root_end = Some(idx);
                    break;
                }
            }
        }
        let end = root_end?;
        let mut cur = self.module.var_decl_types.get(strip_idx(&name[..end]))?.clone();
        for seg in name[end + 1..].split('.') {
            let member = strip_idx(seg);
            let next = match self.resolve_dt_ref(&cur) {
                DataType::Struct(su) => Self::struct_member_decl_ref(su, member).map(|(mdt, _)| mdt.clone()),
                _ => None,
            }?;
            cur = next;
        }
        Some(cur)
    }

    pub(super) fn name_is_shortreal(&self, name: &str) -> bool {
        if let Some(dt) = self.module.var_decl_types.get(name) {
            return super::elaborate::is_type_shortreal_resolved(dt, &self.module.typedef_types);
        }
        if let Some(base) = name.split('[').next()
            && base != name
            && !base.contains('.')
            && let Some(dt) = self.module.var_decl_types.get(base)
        {
            return super::elaborate::is_type_shortreal_resolved(dt, &self.module.typedef_types);
        }
        if let Some(dt) = self.struct_leaf_data_type(name) {
            return super::elaborate::is_type_shortreal_resolved(
                &dt,
                &self.module.typedef_types,
            );
        }
        false
    }
}
