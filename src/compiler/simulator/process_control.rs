//! Simulator process control.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    pub(super) fn is_pid_suspended(&self, pid: usize) -> bool {
        // §9.7: a process explicitly suspended via `process::suspend()` is
        // suspended (and has been removed from all scheduling queues).
        if self.suspended_pids.contains(&pid) {
            return true;
        }
        if self.event_queue.has_pid(pid) {
            return true;
        }
        if self.event_waiters.iter().any(|w| w.pid == pid) {
            return true;
        }
        if self.condition_waiters.iter().any(|(p, _)| *p == pid) {
            return true;
        }
        if self.ready_condition_waiters.iter().any(|(p, _)| *p == pid) {
            return true;
        }
        // A process parked by `#0` in the Inactive-region queue (§4.4.2.3)
        // is suspended, not finished — its continuation resumes after the
        // NBA region of the current tick.
        if self.inactive_queue.iter().any(|(p, _)| *p == pid) {
            return true;
        }
        if self.nba_region_waiters.iter().any(|(p, _)| *p == pid) {
            return true;
        }
        if self.reactive_conts.iter().any(|(p, _)| *p == pid) {
            return true;
        }
        // §9.4.2 / §14.13: @(cb) reached after its raw edge parks until
        // Reactive, including while an iff guard awaits this edge's samples.
        // Keep the task's locals/this alive rather than treating it as done.
        if self.deferred_clocking_conts.iter().any(|(p, _)| *p == pid)
            || self
                .deferred_clocking_guarded
                .iter()
                .any(|(w, _)| w.pid == pid)
        {
            return true;
        }
        // A process blocked on `fork...join`/`join_any` is the PARENT of a
        // join_waiter — also suspended. Without this it was treated as finished,
        // so its process context (this_stack, locals) was dropped and the
        // join-resume restored an empty context (this=None) — forked tasks lost
        // the enclosing instance `this`.
        if self.join_waiters.iter().any(|w| w.parent_pid == pid) {
            return true;
        }
        // A process blocked on a mailbox get/peek (`m_req_fifo.peek` in `get_next_item`)
        // parks ONLY in mailbox_get_waiters. Without this it was treated as finished →
        // its process context (local_stack with the peek's output formal) was dropped,
        // so the put-wake's writeback chain back to the caller's `req` read empty and
        // delivered a null handle (item data=0). Count it as suspended.
        if self
            .mailbox_get_waiters
            .values()
            .any(|q| q.iter().any(|w| w.pid == pid))
        {
            return true;
        }
        // A process parked on a class-field named event (`@m_event` inside a
        // method on `this`) is suspended, not finished — see
        // `InstanceEventWaiter`. Without this a `fork...join_any` whose child
        // is the `@m_event` waiter (e.g. event abort branch) fired
        // the join prematurely, killing the loop branch.
        if self.instance_event_waiters.iter().any(|w| w.pid == pid) {
            return true;
        }
        // §15.4.1: a producer parked on a FULL bounded mailbox, and §15.3.3 a
        // process parked on a semaphore, are equally suspended. Only the GET
        // side was listed, so a blocked producer looked FINISHED: its
        // ProcessContext was discarded (losing its loop state) and an
        // enclosing `fork ... join` completed while it was still mid-loop.
        if self
            .mailbox_put_waiters
            .values()
            .any(|q| q.iter().any(|w| w.pid == pid))
        {
            return true;
        }
        if self
            .semaphore_get_waiters
            .values()
            .any(|q| q.iter().any(|w| w.pid == pid))
        {
            return true;
        }
        // A process parked on a `process::await()` (a forked child waiting on
        // the producer sequence's process, as `m_safe_select_item`'s re-arbitration
        // loop does) is SUSPENDED, not finished. Without this it looked FINISHED,
        // so its ProcessContext (sequencer `this` + the task's `selected_sequence_request`
        // local) was dropped and it resumed with this=None/lstack=0, dereferencing
        // the recalled item handle as null.
        if self.await_waiters.iter().any(|w| w.waiter_pid == pid) {
            return true;
        }
        false
    }

    /// §9.7: check if `expr` is a `handle.await()` call on a process handle.
    /// Handles both parse shapes: `Call{MemberAccess}` and the flattened
    /// `Call{Ident([handle, await])}`. Returns the target pid if so.
    pub(super) fn extract_proc_await_target(&mut self, expr: &Expression) -> Option<usize> {
        if let ExprKind::Call { func, args } = &expr.kind {
            if args.is_empty() {
                // MemberAccess form: `job.await()`
                if let ExprKind::MemberAccess {
                    expr: receiver,
                    member,
                } = &func.kind
                {
                    if member.name.as_str() == "await" {
                        let h = self.eval_expr(receiver).to_u64().unwrap_or(0);
                        return Self::proc_handle_to_pid(h);
                    }
                }
                // Flattened Ident form: `job.await` parses as Ident([job, await])
                if let ExprKind::Ident(hier) = &func.kind {
                    if hier.path.len() == 2 && hier.path[1].name.name == "await" {
                        // Clone and truncate to get just the receiver `job`.
                        let recv_hier = hier_prefix(hier);
                        let receiver = Expression::new(ExprKind::Ident(recv_hier), func.span);
                        let h = self.eval_expr(&receiver).to_u64().unwrap_or(0);
                        return Self::proc_handle_to_pid(h);
                    }
                }
            }
        }
        None
    }

    // ═══════════════════════════════════════════════════════════════
    // §9.7 Fine-grain process control (process class methods)
    // ═══════════════════════════════════════════════════════════════

    /// Extract a pid from a §9.7 process handle. Returns None if the handle
    /// is not a process token (i.e. it's a regular heap object or null).
    pub(super) fn proc_handle_to_pid(handle_val: u64) -> Option<usize> {
        if handle_val >= PROCESS_HANDLE_BASE {
            Some((handle_val - PROCESS_HANDLE_BASE) as usize)
        } else {
            None
        }
    }

    /// §9.7 `status()`: return the process state enum.
    ///   FINISHED=0, RUNNING=1, WAITING=2, SUSPENDED=3, KILLED=4
    pub(super) fn proc_status(&self, pid: usize) -> u32 {
        if self.killed_pids.contains(&pid) {
            return 4; // KILLED
        }
        if self.finished_pids.contains(&pid) {
            return 0; // FINISHED
        }
        if self.suspended_pids.contains(&pid) {
            return 3; // SUSPENDED
        }
        if pid == self.current_pid {
            return 1; // RUNNING
        }
        if self.is_pid_suspended(pid) {
            return 2; // WAITING
        }
        1 // RUNNING
    }

    /// §9.7 `kill()`: forcibly terminate the process and all its descendant
    /// subprocesses. All such processes shall be terminated before kill()
    /// returns.
    pub(super) fn proc_kill(&mut self, pid: usize) {
        // Collect the process + all descendants.
        let mut to_kill: HashSet<usize> = HashSet::default();
        to_kill.insert(pid);
        to_kill.extend(self.collect_fork_descendants(pid));
        for &p in &to_kill {
            self.killed_pids.insert(p);
            self.release_killed_process_state(p);
            self.vc_cancel_process(p);
            self.finished_pids.remove(&p);
            self.suspended_pids.remove(&p);
            self.suspended_proc_info.remove(&p);
            self.process_parents.remove(&p);
            self.process_contexts.remove(&p);
        }
        // Purge from every scheduling structure — every DESCENDANT's
        // future-time #delay too, or the wheel keeps reporting activity
        // and a quiet run never hits the idle break (§9.6.2).
        for &p in &to_kill {
            self.event_queue.remove_pid(p);
        }
        self.event_waiters.retain(|w| !to_kill.contains(&w.pid));
        self.instance_event_waiters
            .retain(|w| !to_kill.contains(&w.pid));
        self.condition_waiters.retain(|(p, _)| !to_kill.contains(p));
        self.ready_condition_waiters
            .retain(|(p, _)| !to_kill.contains(p));
        for p in &to_kill {
            self.cond_waiter_reads.remove(p);
            self.cond_read_union_dirty = true;
            self.cond_waiter_conditions.remove(p);
        }
        self.inactive_queue.retain(|(p, _)| !to_kill.contains(p));
        self.nba_region_waiters
            .retain(|(p, _)| !to_kill.contains(p));
        for q in self.mailbox_get_waiters.values_mut() {
            q.retain(|w| !to_kill.contains(&w.pid));
        }
        for q in self.semaphore_get_waiters.values_mut() {
            q.retain(|w| !to_kill.contains(&w.pid));
        }
        self.await_waiters
            .retain(|w| !to_kill.contains(&w.waiter_pid));
        self.release_killed_from_join_waiters(&to_kill);
        // Wake any process that was awaiting the killed process.
        self.wake_await_waiters(&to_kill);
    }

    /// §9.7 `await()`: block the calling process until the target process
    /// terminates (FINISHED or KILLED). Schedules the caller's continuation
    /// for later wake-up. Returns true if the caller was suspended (the
    /// dispatch should then stop executing the caller's current statement
    /// stream); false if the target is already terminated.
    pub(super) fn proc_await(
        &mut self,
        target_pid: usize,
        caller_pid: usize,
        continuation: ProcCont,
    ) -> bool {
        let terminated =
            self.killed_pids.contains(&target_pid) || self.finished_pids.contains(&target_pid);
        if terminated {
            return false; // already done — caller continues
        }
        self.await_waiters.push(AwaitWaiter {
            target_pid,
            waiter_pid: caller_pid,
            continuation,
        });
        true
    }

    /// §9.7 `suspend()`: suspend the process, desensitizing it from whatever
    /// it is blocked on. The process will not run until `resume()`.
    pub(super) fn proc_suspend(&mut self, pid: usize) {
        if self.suspended_pids.contains(&pid) || self.killed_pids.contains(&pid) {
            return; // already suspended or killed — no effect
        }
        // Try to extract the process from wherever it's parked.
        // 1) Delay (event_queue)
        if let Some((expiry, stmts)) = self.event_queue.remove_pid(pid) {
            self.suspended_pids.insert(pid);
            self.suspended_proc_info.insert(
                pid,
                SuspendedProc {
                    continuation: stmts,
                    original_delay_expiry: Some(expiry),
                },
            );
            return;
        }
        // 2) Event control (event_waiters)
        if let Some(idx) = self.event_waiters.iter().position(|w| w.pid == pid) {
            let waiter = self.event_waiters.remove(idx);
            self.suspended_pids.insert(pid);
            self.suspended_proc_info.insert(
                pid,
                SuspendedProc {
                    continuation: waiter.continuation,
                    original_delay_expiry: None,
                },
            );
            return;
        }
        // 3) Condition wait (wait(expr))
        if let Some(idx) = self.condition_waiters.iter().position(|(p, _)| *p == pid) {
            let (_, stmts) = self.condition_waiters.remove(idx);
            self.cond_waiter_reads.remove(&pid);
            self.cond_read_union_dirty = true;
            self.cond_waiter_conditions.remove(&pid);
            self.suspended_pids.insert(pid);
            self.suspended_proc_info.insert(
                pid,
                SuspendedProc {
                    continuation: stmts,
                    original_delay_expiry: None,
                },
            );
            return;
        }
        if let Some(idx) = self
            .ready_condition_waiters
            .iter()
            .position(|(p, _)| *p == pid)
        {
            let (_, stmts) = self.ready_condition_waiters.remove(idx);
            self.cond_waiter_reads.remove(&pid);
            self.cond_read_union_dirty = true;
            self.cond_waiter_conditions.remove(&pid);
            self.suspended_pids.insert(pid);
            self.suspended_proc_info.insert(
                pid,
                SuspendedProc {
                    continuation: stmts,
                    original_delay_expiry: None,
                },
            );
            return;
        }
        // 4) Inactive queue (#0)
        if let Some(idx) = self.inactive_queue.iter().position(|(p, _)| *p == pid) {
            let (_, stmts) = self.inactive_queue.remove(idx);
            self.suspended_pids.insert(pid);
            self.suspended_proc_info.insert(
                pid,
                SuspendedProc {
                    continuation: stmts,
                    original_delay_expiry: Some(self.time), // #0 — expired immediately
                },
            );
            return;
        }
        // 5) Mailbox get / semaphore get
        for q in self.mailbox_get_waiters.values_mut() {
            if let Some(idx) = q.iter().position(|w| w.pid == pid) {
                if let Some(waiter) = q.remove(idx) {
                    self.suspended_pids.insert(pid);
                    self.suspended_proc_info.insert(
                        pid,
                        SuspendedProc {
                            continuation: waiter.cont,
                            original_delay_expiry: None,
                        },
                    );
                    return;
                }
            }
        }
        for q in self.semaphore_get_waiters.values_mut() {
            if let Some(idx) = q.iter().position(|w| w.pid == pid) {
                if let Some(waiter) = q.remove(idx) {
                    self.suspended_pids.insert(pid);
                    self.suspended_proc_info.insert(
                        pid,
                        SuspendedProc {
                            continuation: waiter.cont,
                            original_delay_expiry: None,
                        },
                    );
                    return;
                }
            }
        }
        // Process is RUNNING (self) or not found — for self-suspend we'd need
        // to capture the continuation at the call site. For now this is a no-op
        // for processes that are actively executing (not blocked).
    }

    /// §9.7 `resume()`: restart a previously suspended process.
    pub(super) fn proc_resume(&mut self, pid: usize) {
        if let Some(info) = self.suspended_proc_info.remove(&pid) {
            self.suspended_pids.remove(&pid);
            // LRM §9.7: if suspended while WAITING on a delay, resensitize.
            // If the original delay has transpired, continue immediately.
            let schedule_time = match info.original_delay_expiry {
                Some(expiry) if expiry <= self.time => self.time,
                Some(expiry) => expiry,
                None => self.time, // event/condition: re-evaluate immediately
            };
            self.event_queue
                .schedule(schedule_time, pid, info.continuation);
        }
    }

    /// Wake all `await()` waiters whose target process has terminated.
    pub(super) fn wake_await_waiters(&mut self, terminated_pids: &HashSet<usize>) {
        let to_wake: Vec<usize> = self
            .await_waiters
            .iter()
            .enumerate()
            .filter(|(_, w)| terminated_pids.contains(&w.target_pid))
            .map(|(i, _)| i)
            .collect();
        // Remove in reverse order to keep indices valid.
        for i in to_wake.into_iter().rev() {
            let waiter = self.await_waiters.remove(i);
            if !self.killed_pids.contains(&waiter.waiter_pid) {
                self.event_queue
                    .schedule(self.time, waiter.waiter_pid, waiter.continuation);
            }
        }
    }

    /// Collect all active fork descendants of `root` (children, grandchildren,
    /// …) via the `process_parents` tree. `root` itself is NOT included.
    /// Used by `disable fork` (§9.6.2) and `wait fork` (§9.6.1), both of which
    /// act on the entire sub-process tree, not just direct children.
    pub(super) fn collect_fork_descendants(&self, root: usize) -> HashSet<usize> {
        let mut out: HashSet<usize> = HashSet::default();
        let mut stack = vec![root];
        while let Some(p) = stack.pop() {
            for (&child, &parent) in self.process_parents.iter() {
                if parent == p && out.insert(child) {
                    stack.push(child);
                }
            }
        }
        out
    }

    /// Remove killed pids from every join waiter's tracked children and wake
    /// any waiter whose join condition is now met (e.g. a `join` whose only
    /// remaining child was just killed by `disable fork`). Without this a
    /// parent blocked in `join` would hang forever on a terminated child.
    pub(super) fn release_killed_from_join_waiters(&mut self, killed: &HashSet<usize>) {
        let mut to_wake: Vec<usize> = Vec::new();
        for (i, w) in self.join_waiters.iter_mut().enumerate() {
            let before = w.child_pids.len();
            w.child_pids.retain(|p| !killed.contains(p));
            w.finished_children.retain(|p| !killed.contains(p));
            if w.child_pids.len() == before {
                continue; // unaffected
            }
            let met = match w.join_type {
                JoinType::Join => w.finished_children.len() == w.child_pids.len(),
                JoinType::JoinAny => !w.finished_children.is_empty() || w.child_pids.is_empty(),
                JoinType::JoinNone => true,
            };
            if met {
                to_wake.push(i);
            }
        }
        to_wake.sort_by(|a, b| b.cmp(a));
        for i in to_wake {
            let waiter = self.join_waiters.remove(i);
            self.event_queue
                .schedule(self.time, waiter.parent_pid, waiter.continuation);
        }
    }

    /// Process ids are never reused. Compiled loops and prepared forever
    /// continuations belonging to a terminated process cannot run again.
    /// Per-process state a killed process (`kill()`, `disable fork`,
    /// `disable <label>`) never releases through `child_finished`.
    pub(super) fn release_killed_process_state(&mut self, pid: usize) {
        self.release_process_execution_caches(pid);
        self.fork_baselines.remove(&pid);
        self.fork_local_scopes.remove(&pid);
        self.release_fork_collections(pid);
        self.fork_signal_captures.remove(&pid);
        self.suspended_loop_frames.remove(&pid);
    }

    fn release_process_execution_caches(&mut self, pid: usize) {
        if !self.exec_cache_pids.remove(&pid) {
            return;
        }
        self.process_loop_bytecode.retain(|&(p, _, _), _| p != pid);
        self.process_cond_bytecode.retain(|&(p, _, _), _| p != pid);
        self.forever_cont_cache.retain(|&(p, _, _, _, _), _| p != pid);
        self.forever_sens_cache.retain(|&(p, _), _| p != pid);
    }

    pub(super) fn child_finished(&mut self, child_pid: usize) {
        self.finished_pids.insert(child_pid);
        self.release_process_execution_caches(child_pid);
        // A fork child's inherited `%m` chain (`inherit_fork_m_scope`) is
        // never unwound by its own statements.
        if !self.process_m_scope.is_empty() {
            self.process_m_scope.remove(&child_pid);
        }
        if !self.program_pids.is_empty() {
            self.program_pids.remove(&child_pid);
        }
        if !self.process_m_label.is_empty() && self.process_parents.contains_key(&child_pid) {
            self.process_m_label.remove(&child_pid);
        }
        self.suspended_loop_frames.remove(&child_pid);
        self.fork_baselines.remove(&child_pid);
        self.fork_local_scopes.remove(&child_pid);
        self.release_fork_collections(child_pid);
        self.fork_signal_captures.remove(&child_pid);
        self.process_contexts.remove(&child_pid);
        // §9.7: wake any process awaiting this one's termination.
        if !self.await_waiters.is_empty() {
            let mut terminated = HashSet::default();
            terminated.insert(child_pid);
            self.wake_await_waiters(&terminated);
        }

        // Reparent any still-running children of the completing process to its
        // own parent, so the fork descendant tree stays connected (a `wait fork`
        // higher up must still see grandchildren whose intermediate parent has
        // finished). Without this, completing `X` would orphan its child `G`.
        if let Some(&grandparent) = self.process_parents.get(&child_pid) {
            let regrandkids: Vec<usize> = self
                .process_parents
                .iter()
                .filter(|&(_, &p)| p == child_pid)
                .map(|(&c, _)| c)
                .collect();
            for c in regrandkids {
                self.process_parents.insert(c, grandparent);
            }
        }
        self.process_parents.remove(&child_pid);

        // Record the completion against every matching plain waiter first
        // (mutable pass), deferring the wake decision so the `wait fork`
        // re-check below can borrow `self` immutably.
        for waiter in self.join_waiters.iter_mut() {
            if waiter.child_pids.contains(&child_pid) {
                waiter.finished_children.insert(child_pid);
            }
        }

        let mut finished_parents = Vec::new();
        for i in 0..self.join_waiters.len() {
            let w = &self.join_waiters[i];
            let should_wake = if w.wait_fork {
                // §9.6.1: `wait fork` waits on the IMMEDIATE children captured
                // when it ran — never on grandchildren. Recomputing the
                // descendant set here was doubly wrong: a `join_none`
                // grandchild extended the wait past every child's completion,
                // and completed children REPARENT their grandkids to this very
                // parent (see above), so the wait could chase the tree forever
                // — a persistent monitor spawned by a child hung it for good.
                w.child_pids.iter().all(|p| w.finished_children.contains(p))
            } else if w.child_pids.contains(&child_pid) {
                match w.join_type {
                    JoinType::Join => w.finished_children.len() == w.child_pids.len(),
                    JoinType::JoinAny => !w.finished_children.is_empty(),
                    JoinType::JoinNone => true,
                }
            } else {
                false
            };
            if should_wake {
                sim_dbg_eprintln!(
                    "[DEBUG] join waiter for parent process {} triggered at time {}",
                    self.join_waiters[i].parent_pid,
                    self.time
                );
                finished_parents.push(i);
            }
        }

        finished_parents.sort_by(|a, b| b.cmp(a));
        for i in finished_parents {
            let waiter = self.join_waiters.remove(i);
            self.event_queue
                .schedule(self.time, waiter.parent_pid, waiter.continuation);
        }
    }
}
