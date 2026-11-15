//! Nonblocking assignment of an assignment pattern to an unpacked aggregate
//! (IEEE 1800-2023 §10.4.2, §4.9.4, §9.4.5).
//!
//! An unpacked array or unpacked struct has no storage of its own: its
//! elements and members are separate signals, so a pattern written to it is
//! spread over them (§10.9). Done straight away inside a nonblocking
//! assignment, the spread was a blocking write: reads later in the same time
//! step already saw the new value, `a <= '{b[0], b[1]}; b <= '{a[0], a[1]};`
//! copied one side onto the other, and `w <= #1 '{..}` ignored its delay.
//! Here the spread runs with its leaf writes captured, and each leaf is then
//! posted as an ordinary nonblocking update of its signal.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    /// Ticks until a nonblocking assignment matures: its intra-assignment
    /// delay when it has one, else the statement delay, else 0.
    pub(super) fn nba_delay_ticks(
        &mut self,
        intra_d: Option<u64>,
        delay: Option<&Expression>,
    ) -> u64 {
        match intra_d {
            Some(d) => d,
            None => delay
                .map(|de| self.eval_expr(de).to_u64().unwrap_or(0))
                .unwrap_or(0),
        }
    }

    /// Post a nonblocking update of signal `id` to `v`, `d` ticks out.
    pub(super) fn post_signal_nba(&mut self, id: usize, mut v: Value, d: u64) {
        if d == 0 {
            if self.nba_behind_queued_partial(id) {
                // §4.6 b): a partial update to this signal is already
                // queued; `nba_fast` would commit this one first.
                self.nba_queue.push(NbaEntry {
                    lhs: None,
                    value: v,
                    resolved_id: Some(id),
                    static_key: None,
                    static_slice: None,
                    this_handle: None,
                    root_frame: None,
                });
            } else {
                // Track via index map so subsequent partial-NBAs
                // (NbaAssignRange / NbaAssignBitDyn from the bytecode VM)
                // targeting the same signal merge into this entry rather than
                // re-seeding from signal_table — same invariant as the
                // bytecode NbaAssign Insn.
                self.nba_fast_index.insert(id, self.nba_fast.len());
                self.nba_fast.push(NbaFast {
                    block_index: 0,
                    signal_id: id,
                    value: v,
                });
            }
            return;
        }
        // Delayed NBA is transport, not inertial: push directly so pending
        // same-signal updates are NOT cancelled. §4.9.4: matures in the
        // target slot's NBA REGION — the separate queue commits inside
        // apply_nba, so Active reads at the same time still see the old value.
        //
        // Reference-parity (#145): a delayed NBA whose captured value equals
        // the CURRENT value is elided at SCHEDULE time (`q = 0; q <= #5 0;
        // #1 q = 1;` leaves q at 1 — the reference never lands the update).
        // Elide only when no delayed NBA is already pending for the id: a
        // pending entry, not the register, is the truth then (the #142 rule).
        //
        // Normalize signedness to the TARGET before comparing — the apply
        // site does the same at maturity, and without it the elision compare
        // saw identical bits as unequal Values and always pushed.
        v.is_signed = self.signal_signed[id];
        let pending_here = self.delayed_nba.iter().any(|(_, sid, _)| *sid == id);
        if pending_here || self.signal_table[id] != v {
            self.delayed_nba.push((self.time + d, id, v));
        }
    }

    /// §10.9.2, §10.10: spread pattern `items` (the items of `rvalue`) over
    /// the leaves of `lvalue` when its leaves are separate signals, as the
    /// nonblocking assignment `lvalue <= rvalue`. Every item is evaluated now;
    /// the leaves are updated `d` ticks out in the NBA region. False when the
    /// target is not such an aggregate, leaving it to the other paths.
    ///
    /// A leaf with no module signal (a frame local, a packed-arena cell) is
    /// still written at once, as before.
    pub(super) fn spread_pattern_nba(
        &mut self,
        lvalue: &Expression,
        rvalue: &Expression,
        items: &[AssignmentPatternItem],
        intra_d: Option<u64>,
        delay: Option<&Expression>,
    ) -> bool {
        let outer = self.pattern_nba_capture.replace(Vec::new());
        let spread = if matches!(&lvalue.kind, ExprKind::Ident(_)) {
            self.pattern_aggregate_names(lvalue)
                .into_iter()
                .any(|n| self.assign_pattern_aggregate(&n, items))
        } else {
            self.assign_pattern_element(lvalue, rvalue)
                || match self.flat_member_name(lvalue) {
                    Some(flat) => self.assign_pattern_flat(&flat, items),
                    None => false,
                }
        };
        let leaves = std::mem::replace(&mut self.pattern_nba_capture, outer).unwrap_or_default();
        if !spread && leaves.is_empty() {
            return false;
        }
        let d = self.nba_delay_ticks(intra_d, delay);
        for (id, v) in leaves {
            self.post_signal_nba(id, v, d);
        }
        true
    }
}
