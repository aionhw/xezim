//! IEEE 1800-2023 §4.4: a finite phase stays in its original time slot.
//!
//! Sample functional data and suspended control only after a guard window
//! expires. Ordinary signal/property writes pay no progress-counter cost.

use super::*;
use std::hash::{Hash, Hasher};

struct Digest {
    hash: std::collections::hash_map::DefaultHasher,
    valid: bool,
}

impl std::io::Write for Digest {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.hash.write(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Digest {
    fn value<T: serde::Serialize + ?Sized>(&mut self, value: &T) {
        if bincode::serde::encode_into_std_write(
            value,
            &mut *self,
            xezim_core::xez_bincode_config(),
        )
        .is_err()
        {
            // An unsupported state cannot establish recurrence.
            self.valid = false;
        }
    }

    fn cont(&mut self, cont: &ProcCont) {
        let mut frame = Some(cont);
        while let Some(c) = frame {
            self.value(&(c.start, c.loop_tail_at, c.repeat_remaining, c.stmts.len()));
            for st in c.stmts.iter() {
                std::mem::discriminant(&st.kind).hash(&mut self.hash);
                // ForeachTail embeds an allocation address in a synthetic
                // span. Its iterator position is captured in locals/arena.
                if !matches!(st.kind, StatementKind::ForeachTail { .. }) {
                    self.value(&(st.span.start, st.span.end));
                }
            }
            frame = c.next.as_deref();
        }
    }

    fn queued<'a>(&mut self, entries: impl Iterator<Item = &'a (usize, ProcCont)>) {
        for (pid, cont) in entries {
            self.value(pid);
            self.cont(cont);
        }
    }
}

impl Simulator {
    pub(super) fn progress_state_hash(&self, current: Option<&ProcCont>) -> Option<u64> {
        let mut d = Digest {
            hash: std::collections::hash_map::DefaultHasher::new(),
            valid: true,
        };
        // Serializing Value includes every wide value/XZ limb, not just the
        // low 64 bits. Exclude store generations and diagnostic counters.
        d.value(&self.signal_table);
        for (name, value) in self.signals.iter() {
            d.value(&(name, value));
        }
        d.value(&(
            &self.heap,
            &self.packed,
            &self.static_fn_ret,
            &self.mailboxes,
            &self.semaphores,
        ));
        d.value(&(&self.rng, &self.proc_rng, &self.obj_rng));
        d.value(&(
            &self.local_stack,
            &self.this_stack,
            &self.class_context_stack,
            &self.foreach_arena,
        ));
        for (pid, context) in &self.process_contexts {
            d.value(&(
                pid,
                &context.local_stack,
                &context.this_stack,
                &context.class_context_stack,
                &context.local_dyn,
                &context.formal_dyn,
                &context.queue_frame_saves,
            ));
        }
        for (pid, f) in &self.proc_fsm {
            d.value(&(pid, f.pc, &f.regs));
            if let Some(ts) = &f.ts {
                d.value(&(ts.valid, &ts.regs));
            }
            #[cfg(feature = "jit")]
            d.value(&f.native_frame);
        }
        d.value(&(
            &self.nba_fast,
            &self.nba_queue,
            &self.packed_nba,
            &self.pending_nba_triggers,
            &self.pending_nba_instance_triggers,
            &self.delayed_updates,
            &self.delayed_nba_queue,
        ));
        d.value(&(
            &self.clocking_output_pending,
            &self.clocking_cycle_pending,
            &self.clocking_edge_out_pending,
        ));
        if let Some(cont) = current {
            d.cont(cont);
        }
        for (slot, events) in self.event_queue.wheel.iter().enumerate() {
            d.value(&(slot, events.len()));
            d.queued(events.iter());
        }
        for (time, events) in &self.event_queue.overflow {
            d.value(&(time, events.len()));
            d.queued(events.iter());
        }
        d.queued(self.inactive_queue.iter());
        d.queued(self.condition_waiters.items.iter());
        d.queued(self.ready_condition_waiters.items.iter());
        d.queued(self.reactive_conts.iter());
        d.queued(self.deferred_clocking_conts.iter());
        d.queued(self.nba_region_waiters.iter());
        for waiter in &self.event_waiters {
            d.value(&(
                waiter.pid,
                waiter.remaining_events,
                &waiter.captured_prev,
                &waiter.captured_prev_wide,
                &waiter.guard_prev,
            ));
            d.cont(&waiter.continuation);
        }
        for waiter in self.instance_event_waiters.entries.values() {
            d.value(&(waiter.pid, &waiter.key));
            d.cont(&waiter.continuation);
        }
        for waiter in &self.join_waiters {
            d.value(&(
                waiter.parent_pid,
                &waiter.child_pids,
                &waiter.finished_children,
            ));
            d.cont(&waiter.continuation);
        }
        for (mailbox, waiters) in &self.mailbox_get_waiters {
            d.value(mailbox);
            for waiter in waiters {
                d.value(&(waiter.pid, waiter.is_peek, &waiter.lvalue));
                d.cont(&waiter.cont);
            }
        }
        for (mailbox, waiters) in &self.mailbox_put_waiters {
            d.value(mailbox);
            for waiter in waiters {
                d.value(&(waiter.pid, &waiter.value));
                d.cont(&waiter.cont);
            }
        }
        for (semaphore, waiters) in &self.semaphore_get_waiters {
            d.value(semaphore);
            for waiter in waiters {
                d.value(&(waiter.pid, waiter.n));
                d.cont(&waiter.cont);
            }
        }
        d.valid.then(|| d.hash.finish().max(1))
    }
}
