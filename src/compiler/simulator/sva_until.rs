//! Property-valued until operands (IEEE 1800 §16.12.12).

use super::*;

impl Simulator {
    pub(super) fn sva_advance_until(
        &mut self,
        left: &SvaNode,
        right: &SvaNode,
        env: &SvaEnv,
        nonvacuous: &mut bool,
        ticks: &mut VecDeque<SvaUntilTick>,
        closed: &mut bool,
    ) -> Option<SvaOutcome> {
        // §16.12.14: an abort step between ticks starts no evaluation.
        if !*closed && !self.sva_abort_step {
            ticks.push_back(SvaUntilTick {
                left: Self::sva_new_state(left, env),
                right: Self::sva_new_state(right, env),
                left_result: None,
                right_result: None,
            });
        }
        for tick in ticks.iter_mut() {
            if tick.right_result.is_none() {
                tick.right_result = self.sva_advance(right, &mut tick.right);
            }
            if tick.left_result.is_none() {
                tick.left_result = self.sva_advance(left, &mut tick.left);
            }
        }
        // Fold finished front ticks into the attempt's accumulated
        // nonvacuity (issue #510): an earliest tick whose left held and
        // whose right has already failed can no longer affect the outcome.
        // Its only lasting effect is that if its operands were nonvacuous
        // the whole `until` attempt is nonvacuous (§16.14.8); because its
        // left held it also keeps the prefix-holds invariant true, which is
        // the default in `sva_until_fails_at_end`. Dropping it bounds the
        // per-cycle work and the memory by the evaluations still pending.
        while let Some(front) = ticks.front() {
            let (l, r) = (front.left_result, front.right_result);
            // Fold only front ticks whose left HELD (Pass or Vacuous) and
            // whose right has already FAILED: those can no longer affect the
            // outcome. A tick whose left failed settles the attempt and must
            // be retained; one whose right is pending must be retained too.
            if matches!(
                (l, r),
                (
                    Some(SvaOutcome::Pass) | Some(SvaOutcome::Vacuous),
                    Some(SvaOutcome::Fail),
                )
            ) {
                // A failed property can be vacuous (for example `not` of a
                // vacuous implication). Retain its recursive nonvacuity.
                *nonvacuous |= Self::sva_op_nonvacuous(&l, left, &front.left)
                    || Self::sva_op_nonvacuous(&r, right, &front.right);
                ticks.pop_front();
                continue;
            }
            break;
        }
        // A right-hand property beginning at tick k requires every left-hand
        // property beginning BEFORE k to succeed. Completion times may differ:
        // an operand still pending is an obligation, not a successful match.
        let mut prefix_pending = false;
        let mut possible = false;
        let mut acc_nonvacuous = *nonvacuous;
        for tick in ticks.iter() {
            // §16.14.8(v/w): either operand may make an eligible tick
            // nonvacuous; two vacuous implications remain vacuous.
            acc_nonvacuous |= Self::sva_op_nonvacuous(&tick.left_result, left, &tick.left)
                || Self::sva_op_nonvacuous(&tick.right_result, right, &tick.right);
            if tick.right_result != Some(SvaOutcome::Fail) {
                possible = true;
                if tick.right_result.is_some() && !prefix_pending {
                    *nonvacuous = acc_nonvacuous;
                    return Some(if acc_nonvacuous {
                        SvaOutcome::Pass
                    } else {
                        SvaOutcome::Vacuous
                    });
                }
            }
            match tick.left_result {
                Some(SvaOutcome::Fail) => {
                    *closed = true;
                    break;
                }
                None => prefix_pending = true,
                _ => {}
            }
        }
        *nonvacuous = acc_nonvacuous;
        if *closed && !possible {
            Some(SvaOutcome::Fail)
        } else {
            None
        }
    }

    pub(super) fn sva_until_fails_at_end(
        left: &SvaNode,
        right: &SvaNode,
        strong: bool,
        ticks: &VecDeque<SvaUntilTick>,
    ) -> bool {
        let mut prefix_holds = true;
        for tick in ticks {
            let right_holds = match tick.right_result {
                Some(result) => result != SvaOutcome::Fail,
                None => !Self::sva_fails_at_end(right, &tick.right),
            };
            if prefix_holds && right_holds {
                return false;
            }
            prefix_holds &= match tick.left_result {
                Some(result) => result != SvaOutcome::Fail,
                None => !Self::sva_fails_at_end(left, &tick.left),
            };
        }
        strong || !prefix_holds
    }
}
