use super::*;

#[derive(Debug, Clone)]
pub(crate) struct JoinWaiter {
    pub(crate) parent_pid: usize,
    pub(crate) child_pids: HashSet<usize>,
    pub(crate) join_type: JoinType,
    pub(crate) continuation: ProcCont,
    pub(crate) finished_children: HashSet<usize>,
    /// `true` for a `wait fork` waiter. Unlike a plain `join`, its wake
    /// condition is re-evaluated against the *live* descendant tree on every
    /// child completion (LRM §9.6.1): it wakes only when the parent has zero
    /// remaining descendants, so it also waits for grandchildren spawned
    /// *after* the `wait fork` executed.
    pub(crate) wait_fork: bool,
}
/// IEEE 1800-2023 §9.7: info stored when a process is explicitly suspended
/// via `process::suspend()`. The process is removed from whatever queue it
/// was parked in; `resume()` uses this to re-schedule it.
#[derive(Clone)]
pub(crate) struct SuspendedProc {
    /// Continuation statements to run when the process resumes.
    pub(crate) continuation: ProcCont,
    /// Original scheduled expiry time if suspended while blocked on a `#delay`.
    /// `None` if blocked on an event/condition/join/mailbox. On resume, if the
    /// original delay has transpired (original_time <= self.time), the process
    /// continues immediately (LRM §9.7).
    pub(crate) original_delay_expiry: Option<u64>,
}
/// §9.7 `process::await()`: a process blocked waiting for another's termination.
pub(crate) struct AwaitWaiter {
    pub(crate) target_pid: usize,
    pub(crate) waiter_pid: usize,
    pub(crate) continuation: ProcCont,
}
/// A suspended process's remaining work: a shared statement list plus a cursor,
/// chained to whatever runs after that list is exhausted.
///
/// It is NOT an index into the AST, and it cannot be. `run_process_stmts`
/// SYNTHESIZES the list it executes — a blocking `begin/end` is flattened into
/// the caller's stream, a blocking task call is spliced in front of the caller's
/// tail — so the statements a parked process still owes are routinely a
/// concatenation that exists nowhere in the source. What CAN be named is a
/// shared handle to that synthesized list plus an offset, and that is what
/// removes the copying:
///
/// * parking used to deep-clone the tail of the statement list — everything the
///   process had left — on every `#delay` and every `@(posedge)`;
/// * splicing used to clone the body and then copy the caller's whole tail onto
///   the end of it, every time a blocking block or task was entered.
///
/// `next` is the frame chain: a splice pushes `body` with `next` = the caller's
/// tail, and the executor follows the chain when a frame runs out. Frames are
/// `Arc`, so capturing a continuation costs O(depth) pointer bumps instead of
/// O(work remaining) statement clones.
///
/// Measured on `bench/run_uvm_bench.sh`: -3.6% median across the UVM examples,
/// which spend ~98% of the loop on this path. It is a TRADE, not a free win —
/// `Arc::from` per splice costs a tight `forever` re-splicing a large block
/// (+9% on the `cont_post_100` synthetic). See
/// docs/perf_dump_offload_2026-07-28.md §6b.
#[derive(Clone, Debug)]
pub(crate) struct ProcCont {
    pub(crate) stmts: Arc<[Statement]>,
    pub(crate) start: usize,
    pub(crate) next: Option<Arc<ProcCont>>,
}

/// Defense in depth for the chain length: the derived drop for a linked list
/// recurses once per link, so a long chain aborts the process instead of
/// returning memory. Unlink iteratively, stopping as soon as a node is still
/// shared (someone else owns the rest).
impl Drop for ProcCont {
    fn drop(&mut self) {
        let mut cur = self.next.take();
        while let Some(arc) = cur {
            match Arc::try_unwrap(arc) {
                Ok(mut node) => cur = node.next.take(),
                Err(_) => break,
            }
        }
    }
}

impl ProcCont {
    /// A continuation over a freshly synthesized statement list.
    pub(crate) fn from_vec(stmts: Vec<Statement>) -> Self {
        ProcCont { stmts: Arc::from(stmts), start: 0, next: None }
    }

    /// Nothing to run.
    pub(crate) fn empty() -> Self {
        ProcCont { stmts: Arc::from(Vec::new()), start: 0, next: None }
    }

    /// Resume at `idx` of THIS frame, keeping the rest of the chain. This is the
    /// operation that used to be a deep clone.
    pub(crate) fn resume_at(&self, idx: usize) -> Self {
        ProcCont {
            stmts: Arc::clone(&self.stmts),
            start: idx.min(self.stmts.len()),
            next: self.next.clone(),
        }
    }

    /// Run `stmts` first, then this continuation from `resume_at`. Replaces
    /// "copy the caller's tail onto the end of the spliced body".
    pub(crate) fn pushed(&self, stmts: Vec<Statement>, resume_at: usize) -> Self {
        // A frame with nothing left contributes NOTHING but a link. Splice the
        // rest of the chain in directly instead of wrapping it.
        //
        // This is what made suspend-aware `while`/`for` unbounded: the loop
        // re-pushes its continuation from the LAST statement of its own frame,
        // so every iteration wrapped an already-exhausted frame and the chain
        // grew by one link per iteration — O(N) memory, and a recursive `Drop`
        // of that list overflowed the stack at ~2000 iterations
        // (`for (int i=0;i<2000;i++) @(posedge clk);`). `repeat`/`forever` were
        // unaffected only because they have their own counted-waiter path.
        let next = if resume_at >= self.stmts.len() {
            self.next.clone()
        } else {
            Some(Arc::new(self.resume_at(resume_at)))
        };
        ProcCont { stmts: Arc::from(stmts), start: 0, next }
    }

    /// This frame only, from the cursor.
    pub(crate) fn frame(&self) -> &[Statement] {
        &self.stmts[self.start.min(self.stmts.len())..]
    }

    /// Nothing left here or behind.
    pub(crate) fn is_exhausted(&self) -> bool {
        self.start >= self.stmts.len() && self.next.is_none()
    }

    /// Statements still owed across the whole chain. Walks the chain — keep it
    /// off hot paths; it exists for diagnostics and for the few callers that
    /// must hand a flat list to code outside the process executor.
    pub(crate) fn len(&self) -> usize {
        let mut n = self.frame().len();
        let mut cur = self.next.as_ref();
        while let Some(f) = cur {
            n += f.frame().len();
            cur = f.next.as_ref();
        }
        n
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.is_exhausted()
    }

    /// The next statement this continuation would execute.
    pub(crate) fn first(&self) -> Option<&Statement> {
        if let Some(s) = self.frame().first() {
            return Some(s);
        }
        let mut cur = self.next.as_ref();
        while let Some(f) = cur {
            if let Some(s) = f.frame().first() {
                return Some(s);
            }
            cur = f.next.as_ref();
        }
        None
    }

    /// Flatten the chain. Diagnostics / snapshot paths only.
    pub(crate) fn to_vec(&self) -> Vec<Statement> {
        let mut out: Vec<Statement> = self.frame().to_vec();
        let mut cur = self.next.clone();
        while let Some(f) = cur {
            out.extend_from_slice(f.frame());
            cur = f.next.clone();
        }
        out
    }
}
#[derive(Debug, Clone, Default)]
pub(crate) struct ProcessContext {
    pub(crate) this_stack: Vec<Option<usize>>,
    pub(crate) local_stack: Vec<HashMap<String, Value>>,
    pub(crate) class_context_stack: Vec<Option<String>>,
    pub(crate) cg_this: Option<usize>,
    pub(crate) return_value: Option<Value>,
    pub(crate) break_flag: bool,
    pub(crate) continue_flag: bool,
    pub(crate) return_flag: bool,
    // Carried so a task that suspends mid-body keeps its full call context
    // (needed once blocking task/method calls are inlined into the process
    // statement stream — see `task_cleanup` and `StatementKind::ScopePop`).
    pub(crate) local_iface_aliases: Vec<HashMap<String, String>>,
    pub(crate) ref_binding_stack: Vec<HashMap<String, Expression>>,
    pub(crate) ref_alias_stack: Vec<HashMap<String, Expression>>,
    pub(crate) queue_frame_saves: Vec<HashMap<String, QueueLocalSave>>,
    pub(crate) task_cleanup: Vec<TaskCleanup>,
    // Per-call-frame map of a local dynamic-array/queue/assoc LOCAL's bare
    // name to a process-unique storage key (e.g. `edges` -> `@edges#7`).
    // SystemVerilog automatic locals are per-invocation: two concurrent
    // task calls each declaring `int edges[$]` must NOT share the global
    // `signals` keys `edges.size` / `edges[i]`. By renaming at the VarDecl
    // and resolving the bare name through this map (see `dyn_name_lookup`
    // + the `resolve_hier_name` early return), each invocation's data lives
    // under a distinct key. Stack of frames pushed/popped in sync with
    // `push_queue_frame`/`pop_and_restore_queue_frame`.
    pub(crate) local_dyn: Vec<HashMap<String, String>>,
}

impl super::Simulator {

    pub(crate) fn snapshot_process_context(&self) -> ProcessContext {
        ProcessContext {
            this_stack: self.oop.this_stack.clone(),
            local_stack: self.local_stack.clone(),
            class_context_stack: self.oop.class_context_stack.clone(),
            cg_this: self.cg_this,
            return_value: self.return_value.clone(),
            break_flag: self.break_flag,
            continue_flag: self.continue_flag,
            return_flag: self.return_flag,
            local_iface_aliases: self.local_iface_aliases.clone(),
            ref_binding_stack: self.ref_binding_stack.clone(),
            ref_alias_stack: self.ref_alias_stack.clone(),
            queue_frame_saves: self.queue_frame_saves.clone(),
            task_cleanup: self.task_cleanup.clone(),
            local_dyn: self.local_dyn.clone(),
        }
    }
    pub(crate) fn take_process_context(&mut self) -> ProcessContext {
        ProcessContext {
            this_stack: std::mem::take(&mut self.oop.this_stack),
            local_stack: std::mem::take(&mut self.local_stack),
            class_context_stack: std::mem::take(&mut self.oop.class_context_stack),
            cg_this: self.cg_this.take(),
            return_value: self.return_value.take(),
            break_flag: std::mem::replace(&mut self.break_flag, false),
            continue_flag: std::mem::replace(&mut self.continue_flag, false),
            return_flag: std::mem::replace(&mut self.return_flag, false),
            local_iface_aliases: std::mem::take(&mut self.local_iface_aliases),
            ref_binding_stack: std::mem::take(&mut self.ref_binding_stack),
            ref_alias_stack: std::mem::take(&mut self.ref_alias_stack),
            queue_frame_saves: std::mem::take(&mut self.queue_frame_saves),
            task_cleanup: std::mem::take(&mut self.task_cleanup),
            local_dyn: std::mem::take(&mut self.local_dyn),
        }
    }
    pub(crate) fn restore_process_context(&mut self, ctx: ProcessContext) {
        self.oop.this_stack = ctx.this_stack;
        self.local_stack = ctx.local_stack;
        self.oop.class_context_stack = ctx.class_context_stack;
        self.cg_this = ctx.cg_this;
        self.return_value = ctx.return_value;
        self.break_flag = ctx.break_flag;
        self.continue_flag = ctx.continue_flag;
        self.return_flag = ctx.return_flag;
        self.local_iface_aliases = ctx.local_iface_aliases;
        self.ref_binding_stack = ctx.ref_binding_stack;
        self.ref_alias_stack = ctx.ref_alias_stack;
        self.queue_frame_saves = ctx.queue_frame_saves;
        self.task_cleanup = ctx.task_cleanup;
        self.local_dyn = ctx.local_dyn;
    }
    pub(crate) fn run_scheduled_process(&mut self, pid: usize, stmts: &ProcCont) {

        self.proc_depth += 1;
        self.run_scheduled_process_inner(pid, stmts);
        self.proc_depth -= 1;
        if self.proc_depth == 0 {
            self.drain_deferred_comb();
        }
    }
    pub(crate) fn run_scheduled_process_inner(&mut self, pid: usize, stmts: &ProcCont) {
        // Flags from a prior process must not leak into this one.
        self.break_flag = false;
        self.return_flag = false;
        self.continue_flag = false;
        self.proc.parked_from_exec = false;
        // A process terminated by `disable fork` (LRM §9.6.2) must not run any
        // remaining queued continuation (e.g. the resume of a `#delay` it was
        // parked on when killed). Pids are monotonic and never reused, so it is
        // safe to leave the entry in `killed_pids` permanently. This is the one
        // chokepoint every dispatch site funnels through.
        if self.proc.killed_pids.contains(&pid) {
            return;
        }
        // A freshly-activated scheduled process must resolve unqualified names
        // from a clean slate. `name_resolve_hint` is a transient sibling-scope
        // hint set while resolving DOTTED names (resolve_hier_name records the
        // parent scope); it is NOT part of the per-process ProcessContext. If a
        // prior process (e.g. a monitor touching `ivif.clk`) left it set to
        // `ivif`, this process's bare top-level names (e.g. the clock gen's
        // `clk`) would mis-resolve to `ivif.clk` — freezing the real `clk` net.
        // Save + clear it for the run, restore on EVERY exit path so the hint
        // never leaks across scheduled processes (or back to the caller).
        let saved_hint = self.name_resolve_hint.borrow_mut().take();
        // Fork-capture names (`auto_loop_vars`) are scoped to one process
        // activation. Structured statements pop their own pushes, but a
        // blocking `begin/end` gets FLATTENED into the process's statement
        // stream (see run_process_stmts), so an `automatic` local declared
        // there is pushed with no enclosing SeqBlock arm to pop it. Truncate
        // on both exits so nothing leaks into other processes — such a var
        // stays capturable for the rest of its (flattened) block, which is
        // exactly its scope.
        let saved_auto_len = self.auto_loop_vars.len();
        // Install the process's instance scope (from its initial block) as the
        // resolution hint so AST-evaluated bare names — e.g. a
        // `std::randomize(sig)` target — resolve to THIS instance's signals in
        // a multiply-instantiated module, not the first instance's.
        if let Some(scope) = self.proc.process_scope_hint.get(&pid).cloned() {
            *self.name_resolve_hint.borrow_mut() = Some(scope.clone());
            self.current_scope = scope;
        } else {
            self.current_scope.clear();
        }
        // A process is not a comb/edge block, so the scope the settle loop
        // last recorded must not leak into this process's `%m`.
        self.m_block_scope.clear();
        self.m_block_scope_id = u32::MAX;
        // This retained payload is one non-suspending blocking assignment.
        // Isolate it from a caller task's locals by moving that context aside;
        // the generic path clones the entire context because arbitrary
        // continuations may suspend, which is unnecessary for this shape.
        if stmts.is_empty() && self.fast_delay_always.contains_key(&pid) {
            let saved = self.take_process_context();
            self.run_fast_delay_always(pid);
            self.restore_process_context(saved);
            self.auto_loop_vars.truncate(saved_auto_len);
            *self.name_resolve_hint.borrow_mut() = saved_hint;
            return;
        }
        // Fast path: if we have no saved process context for this pid AND
        // the caller's execution context is empty, skip the full snapshot /
        // restore dance. Forever-loop bodies like `jclk = ~jclk` that run
        // with no locals don't need context bookkeeping; each call paid
        // several `Vec<HashMap<String, Value>>`-level clones for nothing.
        let saved_ctx_needed = !self.oop.this_stack.is_empty()
            || !self.local_stack.is_empty()
            || !self.oop.class_context_stack.is_empty();
        let has_pid_ctx = self.proc.process_contexts.contains_key(&pid);
        if !saved_ctx_needed && !has_pid_ctx {
            self.run_process_payload(pid, stmts);
            let susp = self.is_pid_suspended(pid);
            if susp {
                // Only snapshot if actually suspended and has state worth saving.
                if !self.oop.this_stack.is_empty()
                    || !self.local_stack.is_empty()
                    || !self.oop.class_context_stack.is_empty()
                {
                    self.proc.process_contexts
                        .insert(pid, self.snapshot_process_context());
                }
            }
            self.auto_loop_vars.truncate(saved_auto_len);
            *self.name_resolve_hint.borrow_mut() = saved_hint;
            return;
        }
        let mut saved = self.snapshot_process_context();
        let ctx = self.proc.process_contexts.remove(&pid).unwrap_or_default();
        self.restore_process_context(ctx);
        self.run_process_payload(pid, stmts);
        if self.is_pid_suspended(pid) {
            self.proc.process_contexts
                .insert(pid, self.snapshot_process_context());
        } else {
            self.proc.process_contexts.remove(&pid);
        }
        // IEEE 1800-2023 §6.21/§9.3.2: automatic variables declared in the
        // parent scope are SHARED with fork children — a child's write must be
        // visible to the parent. xezim gives each fork child a COPY of the
        // parent's locals (inherit_fork_child_context), so the child's writes
        // live in its private copy and must be propagated back. Two storage
        // models must be bridged:
        //   (a) SUBROUTINE locals — live in a `local_stack` frame; merged into
        //       the parent's frame (in `process_contexts` if suspended, or in
        //       `saved` if the parent is the currently-active process whose
        //       context is on THIS Rust stack — e.g. the child ran inside the
        //       parent's own `#delay` via `run_events_until`).
        //   (b) PROCEDURAL block-locals (`initial`/`always`, no call frame) —
        //       live in the global `self.signals`; `inherit_fork_child_context`
        //       copied them into the child's frame and recorded the names in
        //       `fork_signal_captures`. Write the child's changed values back
        //       into `self.signals`, which is what the parent reads from.
        let child_frames: Vec<HashMap<String, Value>> = self.local_stack.clone();
        let baseline = self.fork_baselines.get(&pid).cloned();
        let signal_caps = self.fork_signal_captures.get(&pid).cloned();
        if !child_frames.is_empty() {
            if let Some(parent_pid) = self.proc.process_parents.get(&pid).copied() {
                // (a) subroutine-frame merge
                if let Some(parent_ctx) = self.proc.process_contexts.get_mut(&parent_pid) {
                    Self::merge_fork_writes(
                        &mut parent_ctx.local_stack,
                        &child_frames,
                        baseline.as_ref(),
                    );
                } else {
                    // Parent is the active process — its context is `saved`.
                    Self::merge_fork_writes(
                        &mut saved.local_stack,
                        &child_frames,
                        baseline.as_ref(),
                    );
                }
            }
        }
        // (b) signal-capture write-back: procedural block-locals that live in
        // `self.signals`. Write only keys the child CHANGED (relative to the
        // fork-time baseline) so an inherited-unchanged value doesn't
        // clobber a sibling's or the parent's concurrent write.
        if let Some(caps) = &signal_caps {
            let top = child_frames.last();
            for nm in caps {
                let Some(v) = top.and_then(|f| f.get(nm)) else {
                    continue;
                };
                let inherited_unchanged = baseline
                    .as_ref()
                    .and_then(|b| b.last())
                    .and_then(|f| f.get(nm))
                    .is_some_and(|old| old == v);
                if !inherited_unchanged {
                    self.signals.insert(nm.clone(), v.clone());
                }
            }
        }
        if !self.is_pid_suspended(pid) {
            self.fork_baselines.remove(&pid);
            self.fork_signal_captures.remove(&pid);
        }
        self.restore_process_context(saved);
        self.auto_loop_vars.truncate(saved_auto_len);
        *self.name_resolve_hint.borrow_mut() = saved_hint;
    }
    pub(crate) fn run_process_payload(&mut self, pid: usize, stmts: &ProcCont) {
        if stmts.is_empty() && self.fast_delay_always.contains_key(&pid) {
            self.run_fast_delay_always(pid);
        } else {
            self.run_process_stmts(pid, stmts);
        }
    }
    pub(crate) fn run_process_stmts(&mut self, pid: usize, pc: &ProcCont) {
        let stmts: &[Statement] = pc.frame();
        self.proc.current_pid = pid;
        // Install THIS process's own instance scope as the resolution hint.
        // The hint is transient sibling-scope state: the previous process (or
        // a $display argument it evaluated) may have left its scope behind,
        // and a bare name in this process would then resolve into THAT
        // instance — `user`'s wildcard-imported enum member `C` read the
        // sibling `shadower`'s local `int C` (truncated to the enum's width)
        // purely because shadower's process happened to run first. The reset
        // helper already existed for loop re-entry; a fresh activation needs
        // it just as much.
        self.reset_hint_to_process_scope();
        // Track run_process_stmts recursion depth so the suspend-aware loop
        // handlers (While/For/Repeat below) can trampoline through the event
        // queue instead of recursing when a synchronous loop body never
        // suspends — otherwise a multi-iteration loop of non-blocking task calls
        // recurses deeply and overflows the stack cloning the continuation.
        // The Cell/guard is always on (a few ns per call); overflow-prevention
        // is not debug-only.
        let depth = RPS_DEPTH.with(|c| {
            let d = c.get();
            c.set(d + 1);
            d
        });
        struct DepthGuard;
        impl Drop for DepthGuard {
            fn drop(&mut self) {
                RPS_DEPTH.with(|c| c.set(c.get().saturating_sub(1)));
            }
        }
        let _dg = DepthGuard;
        // (a process that keeps re-scheduling at one time and never lets the
        // clock advance). The livelock pattern only manifests at the outermost
        // dispatch, so the guard stays accurate while re-entrant work is free
        // to recurse up to the trampoline/stack limits above.
        if self.stall_limit > 0 && depth == 0 {
            let hits = self.stall_pid_hits.entry(pid).or_insert(0);
            // One process re-activated this many times at a SINGLE timestamp is
            // not a busy design, it is a livelock: it keeps re-arming itself and
            // time can never advance. (Counting per-process, rather than
            // counting activations at the timestamp, is what makes this safe for
            // a wide design that legitimately wakes thousands of DISTINCT
            // processes at time 0.) run_one_tick's inner drain loop re-reads the
            // queue at the current time, so a self-rescheduling process never
            // reaches the outer event loop — the check has to live here.
            // Check BEFORE counting: the activation that trips the guard is
            // re-parked UNEXECUTED and must not inflate "ran N times".
            if *hits >= self.stall_limit {
                // A single process re-arming this many times at one timestamp is
                // a zero-delay livelock (e.g. `always #(period) clk=~clk` whose
                // real `period` momentarily glitched to 0). Rather than abort the
                // whole run here, RE-PARK this un-executed activation at the
                // current time and ask the outer event loop to defer-and-advance
                // to the next scheduled event — a commercial simulator lets time
                // move past a #0 spinner once its driving value recovers (here,
                // once a reference-clock edge updates the measured period). The outer
                // loop declares it fatal only if nothing is scheduled ahead.
                self.event_queue.schedule(self.time, pid, pc.clone());
                self.zero_delay_defer_pending = true;
                return;
            }
            *hits += 1;
        }
        if sim_debug_enabled() {
            eprintln!(
                "[DEBUG] running process {} ({} stmts) at time {}",
                pid,
                stmts.len(),
                self.time
            );
        }
        let mut i = 0;
        while i < stmts.len() && !self.finished {
            // `resolve_hier_name` installs the parent scope as the resolution
            // hint whenever it resolves a DOTTED name, and nothing restored it.
            // So after a testbench statement read `u_dut.sig`, the hint stayed
            // "u_dut" and the NEXT statement's unqualified names resolved into
            // the DUT: `sel = 3'd4;` wrote `u_dut.sel` (promptly overwritten by
            // the port's continuous assign), so the stimulus silently stopped
            // reaching the design. Re-anchor to this process's own scope at
            // every statement boundary; a hint deliberately installed for a
            // statement is set inside that statement's own handler.
            self.reset_hint_to_process_scope();
            let stmt = &stmts[i];

            // An inlined task/method `return` (return_flag set) must unwind to
            // the enclosing task's `ScopePop` sentinel, skipping the rest of the
            // task body in between (including blocking statements after the `return`).
            // The ScopePop runs `unwind_task_frame`, which restores the caller's
            // saved break/continue/return flags — consuming the return. Without
            // this, return_flag/break_flag leaked past the ScopePop and skipped
            // the caller's next real statement.
            if self.return_flag {
                if let StatementKind::ScopePop = &stmt.kind {
                    if let Some(c) = self.task_cleanup.pop() {
                        self.unwind_task_frame(c);
                    } else {
                        self.return_flag = false;
                    }
                }
                i += 1;
                continue;
            }

            // §12.7.2 for-step barrier (see `StatementKind::LoopStep`). Checked
            // HERE, ahead of the generic dispatch, because `exec_statement`
            // skips every statement while `continue_flag` is set — which is
            // exactly the flag this sentinel exists to consume.
            if let StatementKind::LoopStep = &stmt.kind {
                if !self.break_flag && !self.return_flag {
                    self.continue_flag = false;
                }
                i += 1;
                continue;
            }

            // Expand SeqBlocks: flatten begin/end so that timing controls and waits
            // inside them are properly handled with process suspension.
            if let StatementKind::SeqBlock { stmts: inner, .. } = &stmt.kind {
                if self.stmts_have_blocking(inner) {
                    let mut expanded = inner.clone();
                    // Chain the caller's tail instead of copying it onto the end of
                    // the spliced body (ProcCont::pushed).
                    self.run_process_stmts(pid, &pc.pushed(expanded, pc.start + i + 1));
                    return;
                }
            }

            // Stage 0: normalize a parenless task/method call (LRM §13.5
            // footnote 42 + §13.5.5: parentheses may be omitted for tasks, void
            // functions, and class methods). A call written without
            // parentheses — `t;`, `c.m;`, `m_run_phases;`, `run_test;` — parses
            // as a bare `Expr(Ident([name]))` or `Expr(MemberAccess{...})`, NOT
            // as `Expr(Call { func, args: [] })`. Every Stage 1/1b/1c inlining
            // guard below matches on `Call`, so without this rewrite a blocking
            // task called without parentheses bypasses inlining and falls
            // through to the synchronous `exec_statement`, which cannot honour
            // the body's `#delay`/`wait`/`fork` and silently drops it — the
            // caller never blocks (e.g. `task t; #10; endtask; ... t;` returns
            // at t=0 instead of t=10). Rewrite the parenless subroutine
            // reference into the equivalent zero-argument `Call` and
            // re-dispatch, so the existing guards fire uniformly. Only rewrite
            // when the callee resolves to a free task (`module.tasks`) or a
            // method (on the current `this` or on an object handle); a plain
            // variable reference `x;` is left to the normal expression path.
            // Non-blocking tasks: the rewritten Call still won't satisfy
            // `stmts_have_blocking`, so it falls through to `exec_statement`
            // unchanged — `t()` / `c.m()` with explicit parens already worked.
            //
            // Recognized parenless forms (LRM §13.5 `tf_call`/`method_call`):
            //   `t;`                    -> Ident([t])
            //   `m;`  (this-method)     -> Ident([m])
            //   `c.m;`                  -> MemberAccess{expr:Ident([c]), member:m}
            //   `c.m;` (flattened)      -> Ident([c, m])
            if let StatementKind::Expr(e) = &stmt.kind {
                let rewritten: Option<Expression> = match &e.kind {
                    // Bare name: free task or method on current `this`.
                    ExprKind::Ident(h) if h.path.len() == 1 => {
                        let nm = h.path[0].name.name.clone();
                        let is_free_task = self.module.tasks.contains_key(&nm);
                        let is_this_method = self
                            .oop.this_stack
                            .last()
                            .copied()
                            .flatten()
                            .and_then(|hh| {
                                self.oop.heap
                                    .get(hh)
                                    .and_then(|o| o.as_ref())
                                    .map(|inst| inst.class_name.clone())
                            })
                            .is_some_and(|cls| self.class_has_method(&cls, &nm));
                        if is_free_task || is_this_method {
                            Some(Expression::new(
                                ExprKind::Call {
                                    func: Box::new(e.clone()),
                                    args: vec![],
                                },
                                e.span,
                            ))
                        } else {
                            None
                        }
                    }
                    // `obj.method;` via member-access dot syntax.
                    ExprKind::MemberAccess { expr: recv, member } => {
                        let recv_ok = self
                            .eval_handle_expr(recv)
                            .and_then(|hh| {
                                self.oop.heap
                                    .get(hh)
                                    .and_then(|o| o.as_ref())
                                    .map(|inst| inst.class_name.clone())
                            })
                            .is_some_and(|cls| self.class_has_method(&cls, &member.name));
                        if recv_ok {
                            Some(Expression::new(
                                ExprKind::Call {
                                    func: Box::new(e.clone()),
                                    args: vec![],
                                },
                                e.span,
                            ))
                        } else {
                            None
                        }
                    }
                    // `obj.method;` via flattened 2-segment ident.
                    ExprKind::Ident(h) if h.path.len() == 2 => {
                        let vn = h.path[0].name.name.clone();
                        let mn = h.path[1].name.name.clone();
                        let recv_ok = self
                            .eval_ident_handle(&vn)
                            .and_then(|hh| {
                                self.oop.heap
                                    .get(hh)
                                    .and_then(|o| o.as_ref())
                                    .map(|inst| inst.class_name.clone())
                            })
                            .is_some_and(|cls| self.class_has_method(&cls, &mn));
                        if recv_ok {
                            Some(Expression::new(
                                ExprKind::Call {
                                    func: Box::new(e.clone()),
                                    args: vec![],
                                },
                                e.span,
                            ))
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(call) = rewritten {
                    let call_stmt = Statement::new(StatementKind::Expr(call), stmt.span);
                    let mut cont = vec![call_stmt];
                    // Chain the caller's tail instead of copying it onto the end of
                    // the spliced body (ProcCont::pushed).
                    self.run_process_stmts(pid, &pc.pushed(cont, pc.start + i + 1));
                    return;
                }
            }

            // Stage 1: a call to a free task whose body blocks (top-level
            // `#delay`/`@event`/`wait`) is INLINED — bind the call frame, splice
            // the body + a ScopePop sentinel + the rest into this process's
            // stream, and recurse. The body's waits then suspend the process via
            // the machinery below instead of running synchronously (which spins
            // to a loop cap). Non-blocking task calls keep the synchronous path.
            if let StatementKind::Expr(expr) = &stmt.kind {
                if let ExprKind::Call { func, args } = &expr.kind {
                    if let ExprKind::Ident(h) = &func.kind {
                        if h.path.len() == 1 {
                            // A bare name that is ALSO a method on the current
                            // `this`'s class must inline via the method path
                            // (Stage 1b, with this-context), not as a free task —
                            // otherwise free task resolution without `this` causes
                            // member accesses to fail to resolve.
                            let is_this_method = self
                                .oop.this_stack
                                .last()
                                .copied()
                                .flatten()
                                .and_then(|hh| {
                                    self.oop.heap
                                        .get(hh)
                                        .and_then(|o| o.as_ref())
                                        .map(|inst| inst.class_name.clone())
                                })
                                .is_some_and(|cls| {
                                    self.class_has_method(&cls, &h.path[0].name.name)
                                });
                            if let Some(td) = (!is_this_method)
                                .then(|| self.module.tasks.get(&h.path[0].name.name).cloned())
                                .flatten()
                            {

                                if self.stmts_have_blocking(&td.items) {
                                    let cleanup = self.bind_task_frame(&td, args);
                                    self.task_cleanup.push(cleanup);
                                    let mut cont: Vec<Statement> = td.items.clone();
                                    cont.push(Statement::new(StatementKind::ScopePop, stmt.span));
                                    // Chain the caller's tail instead of copying it onto the end of
                                    // the spliced body (ProcCont::pushed).
                                    self.run_process_stmts(pid, &pc.pushed(cont, pc.start + i + 1));
                                    return;
                                }
                            }
                        }
                    }
                }
            }

            // Stage 1b: a call to a blocking CLASS METHOD (a `task` with a
            // top-level wait/#/@/forever in its body) — bare `m(args)` on the
            // current `this`, or `obj.m(args)` — is INLINED like the free-task
            // case above: bind the frame, splice body + ScopePop + the rest,
            // recurse. The method's waits then suspend the process instead of
            // spinning to the loop cap. Without this, run_phase bodies that call
            // e.g. `collect_data()` (`forever @clk`) or `seq.start()` run
            // synchronously and hang at t=0. Non-blocking methods (functions, or
            // tasks with no top-level wait) keep the synchronous path.
            if let StatementKind::Expr(expr) = &stmt.kind {
                if let ExprKind::Call { func, args } = &expr.kind {
                    let resolved: Option<(usize, String, bool)> = match &func.kind {
                        // (receiver_handle, method_name, this_changes)
                        ExprKind::Ident(h) if h.path.len() == 1 => self
                            .oop.this_stack
                            .last()
                            .copied()
                            .flatten()
                            .map(|hh| (hh, h.path[0].name.name.clone(), false)),
                        ExprKind::Ident(h) if h.path.len() == 2 => self
                            .eval_ident_handle(&h.path[0].name.name)
                            .map(|hh| (hh, h.path[1].name.name.clone(), true)),
                        ExprKind::MemberAccess { expr: recv, member } => self
                            .eval_handle_expr(recv)
                            .map(|hh| (hh, member.name.clone(), true)),
                        _ => None,
                    };
                    if let Some((rh, mn, mut this_changes)) = resolved {
                        if rh != 0 {
                            let is_super = matches!(&func.kind, ExprKind::MemberAccess { expr: recv, .. }
                                if matches!(&recv.kind, ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].name.name == "super"));
                            if is_super {
                                this_changes = false;
                            }
                            let cls = if is_super {
                                self.oop.class_context_stack
                                    .last()
                                    .cloned()
                                    .flatten()
                                    .and_then(|c| self.module.classes.get(&c))
                                    .and_then(|cd| cd.extends.clone())
                            } else {
                                self.oop.heap
                                    .get(rh)
                                    .and_then(|o| o.as_ref())
                                    .map(|inst| inst.class_name.clone())
                            };
                            if let Some(cls) = cls {
                                if let Some((td, mclass)) = self.resolve_class_task(&cls, &mn) {
                                    if self.stmts_have_blocking(&td.items) {
                                        let mut cleanup = self.bind_task_frame(&td, args);
                                        if this_changes {
                                            self.oop.this_stack.push(Some(rh));
                                            self.oop.class_context_stack.push(Some(mclass));
                                            cleanup.pushed_method_this = true;
                                        }
                                        self.task_cleanup.push(cleanup);
                                        let mut cont: Vec<Statement> = td.items.clone();
                                        cont.push(Statement::new(
                                            StatementKind::ScopePop,
                                            stmt.span,
                                        ));
                                        // Chain the caller's tail instead of copying it onto the end of
                                        // the spliced body (ProcCont::pushed).
                                        self.run_process_stmts(pid, &pc.pushed(cont, pc.start + i + 1));
                                        return;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Stage 1c: a blocking STATIC method call
            // `Class::method()` with no receiver handle — Stage 1b skipped it
            // because `path[0]` is a class name, not a handle var. For a static
            // method containing blocking calls (e.g. `forever begin hopper.get(...); fork ... join_none; end`),
            // running synchronously prevents its blocking `get` from suspending.
            // Inline it (static context = the class, null `this`) so those waits
            // suspend the process.
            if let StatementKind::Expr(expr) = &stmt.kind {
                if let ExprKind::Call { func, args } = &expr.kind {
                    // `Class::method()` reaches here in two parse shapes:
                    // a flattened 2-segment Ident `[Class, method]`, OR a
                    // MemberAccess `{ expr: Ident(Class), member: member }`
                    // (the `::` static form). Stage 1b already tried — and
                    // failed — to resolve the receiver as a handle, so a
                    // bare class name reaching here is a static call.
                    let scoped: Option<(String, String)> = match &func.kind {
                        ExprKind::Ident(h)
                            if h.path.len() == 2
                                && self.module.classes.contains_key(&h.path[0].name.name) =>
                        {
                            Some((h.path[0].name.name.clone(), h.path[1].name.name.clone()))
                        }
                        ExprKind::MemberAccess { expr: recv, member } => match &recv.kind {
                            ExprKind::Ident(h)
                                if h.path.len() == 1
                                    && self
                                        .module
                                        .classes
                                        .contains_key(&h.path[0].name.name) =>
                            {
                                Some((h.path[0].name.name.clone(), member.name.clone()))
                            }
                            _ => None,
                        },
                        _ => None,
                    };
                    if let Some((cls, mn)) = scoped {
                        if let Some((td, mclass)) = self.resolve_class_task(&cls, &mn) {
                            if self.stmts_have_blocking(&td.items) {
                                let mut cleanup = self.bind_task_frame(&td, args);
                                // Static context: push the declaring class for
                                // member/static resolution, with a null `this`.
                                self.oop.this_stack.push(None);
                                self.oop.class_context_stack.push(Some(mclass));
                                cleanup.pushed_method_this = true;
                                self.task_cleanup.push(cleanup);
                                let mut cont: Vec<Statement> = td.items.clone();
                                cont.push(Statement::new(StatementKind::ScopePop, stmt.span));
                                // Chain the caller's tail instead of copying it onto the end of
                                // the spliced body (ProcCont::pushed).
                                self.run_process_stmts(pid, &pc.pushed(cont, pc.start + i + 1));
                                return;
                            }
                        }
                    }
                }
            }

            // LRM §15.4.2: blocking mailbox.get(var) on an empty mailbox.
            // When the next statement is `<mb>.get(<lvalue>);` and the
            // resolved mailbox queue is empty, suspend this process: park
            // the destination expression + post-get continuation in
            // mailbox_get_waiters and return. A subsequent `put` will
            // assign + reschedule the continuation.
            if let StatementKind::Expr(expr) = &stmt.kind {
                if let ExprKind::Call { func, args } = &expr.kind {
                    // `mb.get(v)` parses as either
                    //   Call{ func: Ident(hier=[mb,get]), args=[v] }   (common)
                    //   Call{ func: MemberAccess{expr=mb, member=get} }
                    // Recognise both forms; the receiver in the hier-Ident
                    // case is the path with the last segment stripped.
                    let (recv_expr_opt, method): (Option<Expression>, String) = match &func.kind {
                        ExprKind::MemberAccess { expr: recv, member } => {
                            (Some((**recv).clone()), member.name.clone())
                        }
                        ExprKind::Ident(hier) if hier.path.len() >= 2 => {
                            let mut head = hier.clone();
                            let last = head.path.pop().unwrap();
                            let recv = Expression::new(ExprKind::Ident(head), expr.span);
                            (Some(recv), last.name.name)
                        }
                        _ => (None, String::new()),
                    };
                    if (method == "get" || method == "peek") && !args.is_empty() {
                        if let Some(recv) = recv_expr_opt.clone() {
                            let recv_val = self.eval_expr(&recv);
                            let handle = recv_val.to_u64().unwrap_or(0) as usize;
                            // §15.4.2: `get`/`peek` on a mailbox handle that was
                            // never `new`ed. The parking path below is keyed on a
                            // LIVE handle, so a null one fell through to generic
                            // dispatch and returned immediately without blocking.
                            // Wrapped in the usual `forever` that reads a mailbox,
                            // that spins until the stall detector fires — which
                            // then blames a missing timing control, sending the
                            // user looking for a `#delay` in a loop whose real
                            // fault is an unconstructed mailbox. Fail where the
                            // fault actually is.
                            if handle == 0 && self.is_declared_mailbox(&recv) {
                                // Named from the receiver EXPRESSION, not from
                                // its span: the `Ident([mb, get])` parse shape
                                // is rebuilt with the whole call's span, so a
                                // source snippet would quote `mb.get(v)` back
                                // as if that were the handle's name.
                                let what = match &recv.kind {
                                    ExprKind::Ident(h) => h
                                        .path
                                        .iter()
                                        .map(|s| s.name.name.as_str())
                                        .collect::<Vec<_>>()
                                        .join("."),
                                    _ => self
                                        .span_source_snippet_in(recv.span, None)
                                        .unwrap_or_else(|| "<mailbox>".to_string()),
                                };
                                eprintln!(
                                    "[xezim][error] null mailbox handle at time {} — `{}.{}(...)` \
                                     on a mailbox that was never constructed.",
                                    self.time, what, method
                                );
                                eprintln!(
                                    "               A blocking `{}` cannot suspend on a null \
                                     handle, so a `forever` loop around it would spin forever.",
                                    method
                                );
                                eprintln!(
                                    "               Construct it (`{} = new();`) before the \
                                     first `{}`.",
                                    what, method
                                );
                                self.finished = true;
                                return;
                            }
                            let mbx_empty = self
                                .ipc.mailboxes
                                .get(&handle)
                                .map(|q| q.is_empty())
                                .unwrap_or(false);
                            if mbx_empty {
                                // §15.4.1/§15.4.2: a producer parked on a FULL
                                // bounded mailbox can proceed now that the box is
                                // empty. Only a consuming `get` used to admit it,
                                // so a get that PARKED on the empty box left both
                                // sides asleep — a zero-delay producer/consumer
                                // pair deadlocked after filling the bound once.
                                self.admit_mailbox_put_waiter(handle);
                            }
                            if let Some(q) = self.ipc.mailboxes.get(&handle) {
                                if q.is_empty() {
                                    // Blocking get/peek on an empty mailbox: park
                                    // until a put hands over a value. peek leaves
                                    // the item in the box (`m_req_fifo.peek` in
                                    // `get_next_item`, then `item_done`
                                    // try_gets it).
                                    let lvalue = args[0].clone();
                                    let cont = pc.resume_at(pc.start + i + 1);
                                    self.ipc.mailbox_get_waiters
                                        .entry(handle)
                                        .or_default()
                                        .push_back(MailboxGetWaiter {
                                            pid,
                                            lvalue,
                                            cont,
                                            is_peek: method == "peek",
                                        });
                                    return;
                                }
                            }
                        }
                    }
                    // §15.4.1: blocking `mailbox.put(v)` on a FULL bounded
                    // mailbox. Capture the value now, park the producer, and
                    // return; a later get/try_get admits it and resumes here.
                    if method == "put" && !args.is_empty() {
                        if let Some(recv) = recv_expr_opt.clone() {
                            let recv_val = self.eval_expr(&recv);
                            let handle = recv_val.to_u64().unwrap_or(0) as usize;
                            let bound = self.ipc.mailbox_bound.get(&handle).copied().unwrap_or(0);
                            if bound > 0 {
                                let len = self.ipc.mailboxes.get(&handle).map(|q| q.len()).unwrap_or(0);
                                if len >= bound {
                                    let value = self.eval_expr(&args[0]);
                                    let cont = pc.resume_at(pc.start + i + 1);
                                    self.ipc.mailbox_put_waiters
                                        .entry(handle)
                                        .or_default()
                                        .push_back(MailboxPutWaiter { pid, value, cont });
                                    return;
                                }
                            }
                        }
                    }
                    // §15.3.3: blocking `semaphore.get(n)` on an under-full
                    // semaphore. Park until a `put` raises the count enough; the
                    // decrement happens at wake. A get that CAN proceed falls
                    // through to the method handler, which decrements there.
                    if method == "get" {
                        if let Some(recv) = recv_expr_opt {
                            let recv_val = self.eval_expr(&recv);
                            let handle = recv_val.to_u64().unwrap_or(0) as usize;
                            if let Some(&count) = self.ipc.semaphores.get(&handle) {
                                let n = args
                                    .first()
                                    .map(|a| self.eval_expr(a).to_u64().unwrap_or(1))
                                    .unwrap_or(1) as i64;
                                if count < n {
                                    let cont = pc.resume_at(pc.start + i + 1);
                                    self.ipc.semaphore_get_waiters
                                        .entry(handle)
                                        .or_default()
                                        .push_back(SemGetWaiter { pid, n, cont });
                                    return;
                                }
                            }
                        }
                    }
                }
            }

            // IEEE 1800-2017 §9.4.5 intra-assignment EVENT control
            // `lhs = [repeat(n)] @(edge sig) rhs` (canonicalized to
            // `$__xz_intra_ev(n, edge, sig, rhs)`): evaluate the RHS NOW,
            // then wait the event n times, then assign the saved value.
            // Expand into n chained event controls + the saved assign so the
            // existing top-level TimingControl::Event park machinery does
            // the waiting.
            if let StatementKind::BlockingAssign { lvalue, rvalue } = &stmt.kind {
                if let ExprKind::SystemCall { name, args } = &rvalue.kind {
                    if name == crate::intra_delay::INTRA_EVENT_MARKER && args.len() == 4 {
                        let n_val = self.eval_expr(&args[0]).to_u64().unwrap_or(0) as i64;
                        let edge_code = self.eval_expr(&args[1]).to_u64().unwrap_or(0);
                        let val = self.eval_expr(&args[3]);
                        let saved = self.make_intra_saved_expr(val, rvalue.span);
                        let assign = Statement::new(
                            StatementKind::BlockingAssign {
                                lvalue: lvalue.clone(),
                                rvalue: saved,
                            },
                            stmt.span,
                        );
                        let mut cont: Vec<Statement> = Vec::new();
                        // §9.4.5: a zero/negative repeat count degenerates to
                        // an immediate assignment (no event wait).
                        let edge = match edge_code {
                            1 => Some(Edge::Posedge),
                            2 => Some(Edge::Negedge),
                            _ => None,
                        };
                        for _ in 0..n_val.max(0) {
                            cont.push(Statement::new(
                                StatementKind::TimingControl {
                                    control: TimingControl::Event(EventControl::EventExpr(vec![
                                        EventExpr {
                                            edge,
                                            expr: args[2].clone(),
                                            iff: None,
                                            span: stmt.span,
                                        },
                                    ])),
                                    stmt: Box::new(Statement::new(
                                        StatementKind::Null,
                                        stmt.span,
                                    )),
                                },
                                stmt.span,
                            ));
                        }
                        cont.push(assign);
                        // Chain the caller's tail instead of copying it onto the end of
                        // the spliced body (ProcCont::pushed).
                        self.run_process_stmts(pid, &pc.pushed(cont, pc.start + i + 1));
                        return;
                    }
                }
            }

            // IEEE 1800-2017 §9.4.5 intra-assignment delay `lhs = #d rhs`:
            // the RHS is evaluated NOW, the process suspends d time units,
            // then the pre-computed value is assigned — i.e. behave like
            // `#d;` followed by `lhs = <saved value>;`.
            if let StatementKind::BlockingAssign { lvalue, rvalue } = &stmt.kind {
                if let Some((d_expr, rhs)) = Self::intra_delay_marker(rvalue) {
                    let val = self.eval_expr(rhs);
                    let delay = self.eval_delay_ticks(d_expr);
                    let saved = self.make_intra_saved_expr(val, rvalue.span);
                    let mut cont = vec![Statement::new(
                        StatementKind::BlockingAssign {
                            lvalue: lvalue.clone(),
                            rvalue: saved,
                        },
                        stmt.span,
                    )];
                    // Chain the caller's tail rather than copying it (ProcCont::pushed).
                    let cont = pc.pushed(cont, pc.start + i + 1);
                    self.event_queue.schedule(self.time + delay, pid, cont);
                    return;
                }
            }

            // Check for timing control — delay or event
            // §15.5.3 wait_order (a, b, c) pass else fail — a state machine
            // over the event list. Each step parks the process on ALL not-yet-
            // expected events (so an out-of-order fire wakes us too), with the
            // continuation re-entering this statement `armed`. On wake the
            // `event_triggered_time` stamps identify which event fired: the
            // expected one advances the sequence (completing it runs `pass`),
            // any later one runs `fail` (or a loud warning without an else).
            if let StatementKind::WaitOrder {
                events,
                pass,
                fail,
                armed,
                idx,
                span,
            } = &stmt.kind
            {
                let k = *idx as usize;
                let n = events.len();
                let stamp_now = |sim: &Self, nm: &str| -> bool {
                    let now = sim.time;
                    let canon = sim.resolve_event_key(nm);
                    if sim.ipc.event_triggered_time.get(canon.as_str()) == Some(&now) {
                        return true;
                    }
                    let pref = format!("{}.{}", sim.module.name, canon);
                    sim.ipc.event_triggered_time.get(pref.as_str()) == Some(&now)
                };
                let mut next_k = k;
                let mut outcome: Option<bool> = None; // Some(true)=pass, Some(false)=fail
                if *armed {
                    let expected_fired = k < n && stamp_now(self, &events[k].name);
                    let later_fired = events[k.saturating_add(1)..]
                        .iter()
                        .any(|e| stamp_now(self, &e.name));
                    if expected_fired {
                        if k + 1 == n {
                            outcome = Some(true);
                        } else {
                            next_k = k + 1;
                        }
                    } else if later_fired {
                        outcome = Some(false);
                    }
                    // Neither fired this slot (spurious wake): re-park at k.
                }
                match outcome {
                    Some(ok) => {
                        let action = if ok { pass } else { fail };
                        if !ok && fail.is_none() {
                            eprintln!(
                                "[xezim][warning] wait_order sequence violation at time {}: an event fired out of order and the construct has no else clause (IEEE 1800-2017 §15.5.3)",
                                self.time
                            );
                        }
                        let mut cont: Vec<Statement> = Vec::new();
                        if let Some(a) = action {
                            match &a.kind {
                                StatementKind::SeqBlock { stmts: b, .. } => {
                                    cont.extend_from_slice(b)
                                }
                                _ => cont.push((**a).clone()),
                            }
                        }
                        // Chain the caller's tail instead of copying it onto the end of
                        // the spliced body (ProcCont::pushed).
                        self.run_process_stmts(pid, &pc.pushed(cont, pc.start + i + 1));
                        return;
                    }
                    None => {
                        // Park on events[next_k..]; continuation re-enters armed.
                        let evs: Vec<crate::ast::stmt::EventExpr> = events[next_k..]
                            .iter()
                            .map(|id| crate::ast::stmt::EventExpr {
                                edge: None,
                                expr: Expression::new(
                                    ExprKind::Ident(crate::ast::expr::HierarchicalIdentifier {
                                        root: None,
                                        path: vec![crate::ast::expr::HierPathSegment {
                                            name: id.clone(),
                                            selects: Vec::new(),
                                        }],
                                        span: *span,
                                        cached_signal_id: std::cell::Cell::new(None),
                                        cached_resolved_name: std::cell::OnceCell::new(),
                                    }),
                                    *span,
                                ),
                                iff: None,
                                span: *span,
                            })
                            .collect();
                        let sens =
                            self.event_to_sens(&crate::ast::stmt::EventControl::EventExpr(evs));
                        let mut cont = vec![Statement::new(
                            StatementKind::WaitOrder {
                                events: events.clone(),
                                pass: pass.clone(),
                                fail: fail.clone(),
                                armed: true,
                                idx: next_k as u32,
                                span: *span,
                            },
                            stmt.span,
                        )];
                        // Chain the caller's tail rather than copying it (ProcCont::pushed).
                        let cont = pc.pushed(cont, pc.start + i + 1);
                        if !sens.is_empty()
                            && sens.iter().any(|s| {
                                self.signal_name_to_id.contains_key(s.signal_name.as_str())
                            })
                        {
                            { let w = self.make_event_waiter(pid, sens, cont); self.ipc.event_waiters.push(w); }
                        } else {
                            eprintln!(
                                "[xezim][warning] wait_order at time {}: none of the named events resolve to declared events; process will not resume (IEEE 1800-2017 §15.5.3)",
                                self.time
                            );
                        }
                        return;
                    }
                }
            }

            if let StatementKind::TimingControl {
                control,
                stmt: body,
            } = &stmt.kind
            {
                match control {
                    TimingControl::Delay(d) => {
                        let delay = self.eval_delay_ticks(d);
                        // A NON-LITERAL `#(expr)` that evaluates to 0 at the
                        // very start of time 0 almost always means its period
                        // signal has not settled yet: e.g. `assign xbOH =
                        // dTx*d0j;` feeding `always #(xbOH/2) clk=~clk;`, where
                        // the clock process (a lower pid, scheduled during
                        // classify_always_blocks) runs before the `initial`
                        // block that seeds `dTx`. Toggling now would drop a
                        // spurious edge at t=0 and INVERT the clock's phase for
                        // the whole run (so a later strobe samples the wrong
                        // half-cycle). Re-park the WHOLE timing control — not
                        // the post-`body` continuation — so `#(expr)` RE-
                        // EVALUATES after the time-0 active+NBA+settle has run
                        // the initial blocks and propagated the cont-assign.
                        // One-shot per pid (`t0_delay_deferred`): a genuine
                        // zero-period loop re-parks the continuation as usual on
                        // its second pass. Literal `#0` is untouched.
                        if delay == 0
                            && self.time == 0
                            && !matches!(d.kind, ExprKind::Number(_))
                            && self.t0_delay_deferred.insert(pid)
                        {
                            let mut whole = vec![Statement::new(
                                StatementKind::TimingControl {
                                    control: TimingControl::Delay(d.clone()),
                                    stmt: body.clone(),
                                },
                                stmt.span,
                            )];
                            // Chain the caller's tail rather than copying it (ProcCont::pushed).
                            let whole = pc.pushed(whole, pc.start + i + 1);
                            self.proc.inactive_queue.push((pid, whole));
                            return;
                        }
                        let mut cont = vec![*body.clone()];
                        // Chain the caller's tail rather than copying it (ProcCont::pushed).
                        let cont = pc.pushed(cont, pc.start + i + 1);
                        if delay == 0 {
                            // `#0` — IEEE 1800-2017 §4.4.2.3: suspend into the
                            // Inactive region of the SAME time slot, not the
                            // Active region. Scheduling into event_queue at
                            // self.time would resume it in the same batch
                            // drain, BEFORE apply_nba — so an NBA posted
                            // before the `#0` would not be visible after it.
                            // Commercial consensus (VCS/Riviera): it IS
                            // visible. Park here; run_one_tick promotes after
                            // the NBA region of this tick has been applied.
                            self.proc.inactive_queue.push((pid, cont));
                        } else {
                            self.event_queue.schedule(self.time + delay, pid, cont);
                        }
                        return;
                    }
                    TimingControl::Event(event) => {
                        // §14.11 `##0`: SYNCHRONIZE to the default clocking
                        // event — a no-op when the process is already
                        // executing in the time slot of that event (its edge
                        // fired at the current time); otherwise park exactly
                        // like `@(__xz_default_clocking)`.
                        if let EventControl::Identifier(id) = event {
                            if id.name == "__xz_default_clocking0"
                                && self
                                    .default_clocking_cb
                                    .as_ref()
                                    .and_then(|cb| self.clocking_last_edge.get(cb))
                                    == Some(&self.time)
                            {
                                // The loop increments `i` at its bottom; a bare
                                // `continue` would re-execute this statement
                                // forever.
                                self.exec_statement(body);
                                i += 1;
                                continue;
                            }
                        }
                        // Suspend process until the event fires
                        // Class-field named event (`@m_event` inside a method
                        // on `this`): park on the per-instance event identity.
                        // A class `event` field has no backing signal, so
                        // without this it fell through to the delta-yield
                        // (NBA) branch below and `@m_event` returned
                        // immediately — breaking class event / heartbeat
                        // synchronization (a `start()`ed heartbeat died at t=0
                        // after one spurious check). Module-scope named events
                        // are backed by real signals and take the event_waiters
                        // path further below. Inside a class method the
                        // elaborator rewrites `@field` to a `HierIdentifier`,
                        // so accept both shapes.
                        if let Some(fname) = self.event_control_field_name(event) {
                            if let Some(key) = self.resolve_this_event_field(&fname) {
                                let mut cont = vec![*body.clone()];
                                // Chain the caller's tail rather than copying it (ProcCont::pushed).
                                let cont = pc.pushed(cont, pc.start + i + 1);
                                self.ipc.instance_event_waiters.push(InstanceEventWaiter {
                                    key,
                                    pid,
                                    continuation: cont,
                                });
                                return;
                            }
                        }
                        // `@(h.ce)`: a class-property event reached through a
                        // handle from OUTSIDE the class. `resolve_this_event_field`
                        // above only covers a bare field on `this`; the general
                        // resolver adds receivers with runtime selects
                        // (`@(m_events[obj].all_dropped)` — the UVM objection
                        // wait, #109) and chained handles.
                        if let Some(expr) = Self::event_control_single_expr(event) {
                            if let Some(key) = self
                                .expr_handle_event_field(&expr)
                                .or_else(|| self.expr_instance_event_field_general(&expr))
                            {
                                let cont = vec![*body.clone()];
                                // Chain the caller's tail rather than copying it.
                                let cont = pc.pushed(cont, pc.start + i + 1);
                                self.ipc.instance_event_waiters.push(InstanceEventWaiter {
                                    key,
                                    pid,
                                    continuation: cont,
                                });
                                return;
                            }
                        }
                        // §15.5 a DECLARED named event — including an array
                        // element (`@ev[1]`) and one reached through an alias
                        // (`e1 = e2; @e1`). Both need the resolved key rather
                        // than the raw text: `event_to_sens` walks PAST an
                        // Index to the base ident, so `@ev[1]` armed on the
                        // array name `ev` (not a 1-bit signal) and fell into
                        // the delta-yield below, waking instantly at t=0. And
                        // an aliased waiter armed on its own name, so it never
                        // saw `-> e2` — only the TRIGGER side canonicalized
                        // (§15.5.4). Resolving here fixes both, and keeps the
                        // delta-yield fallback for genuine non-event locals.
                        if let Some(key) = self.event_control_event_key(event) {
                            let canon = self.resolve_event_key(&key);
                            if self.signal_name_to_id.contains_key(canon.as_str()) {
                                let cont = vec![*body.clone()];
                                // Chain the caller's tail rather than copying it.
                                let cont = pc.pushed(cont, pc.start + i + 1);
                                let sens = vec![Sensitivity {
                                    signal_name: canon,
                                    edge: EdgeKind::AnyEdge,
                                    iff: None,
                                    value_of: None,
                                }];
                                { let w = self.make_event_waiter_kind(pid, sens, cont, false); self.ipc.event_waiters.push(w); }
                                return;
                            }
                        }
                        let is_star = matches!(
                            event,
                            EventControl::Star | EventControl::ParenStar
                        );
                        let sens = if is_star {
                            self.star_sens_from_body(body)
                        } else {
                            self.event_to_sens(event)
                        };
                        let is_clk_ev = self.is_clocking_event(event);
                        // `@(*)` over a body that reads NOTHING can never
                        // trigger again — park the process instead of looping.
                        if is_star && sens.is_empty() {
                            return;
                        }
                        if !sens.is_empty() {
                            let mut cont = vec![*body.clone()];
                            // Chain the caller's tail rather than copying it (ProcCont::pushed).
                            let cont = pc.pushed(cont, pc.start + i + 1);
                            let has_real = sens.iter().any(|s| {
                                self.signal_name_to_id.contains_key(s.signal_name.as_str())
                            });
                            if has_real {
                                { let w = self.make_event_waiter_kind(pid, sens, cont, is_clk_ev); self.ipc.event_waiters.push(w); }
                            } else {
                                // `@(x)` where x is not a real signal — a
                                // procedural local that was NBA-assigned then
                                // waited on (uvm_wait_for_nba_region:
                                // `nba <= next_nba; @(nba)`). Its event_waiter
                                // would resolve to no signal_id and park forever;
                                // treat it as a one-delta yield (its purpose is to
                                // yield across the NBA region).
                                self.event_queue.schedule(self.time, pid, cont);
                            }
                            return;
                        }
                        // Star/empty sensitivity — just execute body
                    }
                }
                self.exec_statement(body);
                i += 1;
                continue;
            }

            if let StatementKind::Wait {
                condition,
                stmt: body,
            } = &stmt.kind
            {
                let cond_val = self.eval_expr(condition);
                if cond_val.is_true() {
                    self.cond_progress = self.cond_progress.wrapping_add(1);
                    self.exec_statement(body);
                    i += 1;
                    continue;
                } else {
                    // IEEE 1800-2023 §9.7.4: `wait(cond)` is LEVEL-
                    // sensitive — the process resumes the moment the
                    // condition is true, including in the SAME timestep via a
                    // delta-cycle (#0) write from another process. The
                    // condition-waiter fixpoint in run_one_tick re-evaluates
                    // the actual expression every tick, so it handles same-
                    // timestep (delta), cross-timestep (#delay), and time-0
                    // init uniformly. Routing a signal-naming wait through the
                    // edge-triggered event_waiter path instead BREAKS delta-
                    // cycle wakeup: its registration-generation guard
                    // (correct for edge-sensitive `@()`) skips edges from the
                    // current snapshot window, so `wait(sig==v)` may miss a
                    // peer process writing `sig` at the same simtime. Level-sensitive
                    // condition handoffs depend on exactly this delta-cycle
                    // handoff. Always park in condition_waiters.
                    let mut cont = vec![stmt.clone()];
                    // Chain the caller's tail rather than copying it (ProcCont::pushed).
                    let cont = pc.pushed(cont, pc.start + i + 1);
                    self.proc.condition_waiters.push((pid, cont));
                    return;
                }
            }

            // Check for forever with delays/events. FIRST entry only — the
            // body's first iteration runs inside `exec_forever_sched`, which
            // re-appends a `ForeverTail` sentinel (not `Forever`) so that on
            // RESUME after a suspension the `ForeverTail` arm below runs the
            // break/continue/return gate. Gating here on first entry was the
            // old approach: it deadlocked a `forever` whose body only raises
            // a control flag AFTER its first iteration blocks (a stale or
            // transient flag fired before the
            // body ran even once. The sentinel split (first entry vs re-entry)
            // is what makes `break` safe now. (§9.3.3)
            if let StatementKind::Forever { body } = &stmt.kind {
                self.exec_forever_sched(pid, body, pc, i);
                return;
            }

            // INTERNAL: `ForeverTail` continuation sentinel — re-entry point
            // for a blocking-body `forever` after its body suspended (via
            // `exec_forever_sched`'s event/delay/blocking continuation). The
            // body statements up to the suspension point have just run; a
            // `break`/`continue`/`return` raised anywhere in that body slice
            // is honoured HERE (§9.3.3), which the old `Forever` re-append
            // silently dropped.
            if let StatementKind::ForeverTail { body } = &stmt.kind {
                // `return` from an inlined task body: do NOT consume — unwind
                // to the task's ScopePop (the top-of-loop return_flag skip
                // drives it). Exit this forever.
                if self.return_flag {
                    i += 1;
                    continue;
                }
                // `break` exits the forever; consume break+continue.
                if self.break_flag {
                    self.break_flag = false;
                    self.continue_flag = false;
                    i += 1;
                    continue;
                }
                // `continue` (or no flag): consume it and run the next
                // iteration. exec_forever_sched re-appends another
                // ForeverTail, so this gate runs again on the next resume.
                self.continue_flag = false;
                self.exec_forever_sched(pid, body, pc, i);
                return;
            }

            // Check for repeat with event waits inside
            if let StatementKind::Repeat { count, body } = &stmt.kind {
                let n = self.repeat_count(count);
                // Mirror the While/For arms: unroll not only when the body has
                // a direct `@event`/`#delay`, but also when it blocks via a
                // CALL to a task whose own body contains a `wait`/`#delay`/
                // `@event` (a loop like `repeat(N) obj.do_step();` where
                // do_step() blocks). Otherwise the repeat falls through to the
                // SYNCHRONOUS exec_statement, the body's calls are never
                // inlined (Stage 1b), and their nested `wait`s fall through
                // (IEEE 1800-2023 §9.7.4: a false `wait(cond)` must suspend)
                // instead of parking the process, busy-spinning the loop.
                if self.stmt_has_event_wait(body) || self.stmt_is_blocking(body) {
                    // n == 0 (initial count zero, or the natural-exhaustion
                    // sentinel re-entering after the final iteration's body):
                    // clear any loop-control flag left by that body — a
                    // trailing `continue` must not leak past the loop and
                    // suppress the statements after it. (§9.3.3)
                    if n == 0 {
                        self.break_flag = false;
                        self.continue_flag = false;
                        i += 1;
                        continue;
                    }
                    // A `break`/`continue` set during a previous iteration's
                    // body is consumed here: break exits the repeat, continue
                    // proceeds to the next iteration. (§9.3.3)
                    if self.blocking_loop_flag_gate() {
                        i += 1;
                        continue;
                    }
                    // `repeat (N) @(event);` has no per-iteration action. Keep
                    // one counted waiter parked across the N events rather
                    // than cloning/evaluating a Repeat tail and resolving the
                    // same sensitivity after every edge. Clocking-block events
                    // retain the general path because their intermediate
                    // Reactive-region scheduling is observable.
                    if let StatementKind::TimingControl {
                        control: TimingControl::Event(event),
                        stmt: event_body,
                    } = &body.kind
                    {
                        if matches!(event_body.kind, StatementKind::Null)
                            && !self.is_clocking_event(event)
                        {
                            let sens = self.event_to_sens(event);
                            let has_real = sens.iter().any(|s| {
                                self.signal_name_to_id.contains_key(s.signal_name.as_str())
                            });
                            if !sens.is_empty() && has_real {
                                let mut waiter = self.make_event_waiter(
                                    pid,
                                    sens,
                                    pc.resume_at(pc.start + i + 1),
                                );
                                waiter.remaining_events = n;
                                self.ipc.event_waiters.push(waiter);
                                return;
                            }
                        }
                    }
                    // Unroll: execute body once, then schedule rest
                    let remaining_n = n - 1;
                    let mut cont = Vec::new();
                    // Expand body (may contain @event)
                    let body_stmts = match &body.kind {
                        StatementKind::SeqBlock { stmts, .. } => stmts.clone(),
                        _ => vec![*body.clone()],
                    };
                    cont.extend(body_stmts);
                    // Always re-schedule a Repeat tail — at remaining 0 it
                    // becomes the exhaustion sentinel whose n==0 arm above
                    // clears a trailing continue/break before `after` runs.
                    cont.push(Statement::new(
                        StatementKind::Repeat {
                            count: Expression::new(
                                ExprKind::Number(NumberLiteral::Integer {
                                    size: None,
                                    signed: false,
                                    base: NumberBase::Decimal,
                                    value: remaining_n.to_string(),
                                    cached_val: Cell::new(None),
                                }),
                                body.span,
                            ),
                            body: body.clone(),
                        },
                        stmt.span,
                    ));
                    // Chain the caller's tail rather than copying it (ProcCont::pushed).
                    let cont = pc.pushed(cont, pc.start + i + 1);
                    self.continue_stmts_or_trampoline(pid, cont);
                    return;
                }
            }

            // While loop with event/timing waits inside: unroll one iteration,
            // re-append the while statement so the condition is re-checked
            // after suspension.
            // If-statement whose chosen branch contains blocking stmts: descend
            // into the branch via run_process_stmts so repeat/while/@event
            // inside the branch can properly suspend the process.
            if let StatementKind::If {
                condition,
                then_stmt,
                else_stmt,
                ..
            } = &stmt.kind
            {
                // `if (e matches p)` binds pattern variables, which only
                // `exec_statement` does; evaluating it here would produce the
                // match result without the bindings. Leave it alone.
                if !matches!(condition.kind, ExprKind::Matches { .. }) {
                    // The condition is evaluated EXACTLY ONCE. Deciding whether
                    // the chosen branch blocks used to evaluate it here and then
                    // fall through to `exec_statement`, which evaluated it again
                    // — so any side effect in an `if` condition (`$random()`,
                    // `$urandom()`, a VPI `$systf`, `i++`) happened twice.
                    let chosen: Option<&Statement> = if self.eval_expr(condition).is_true() {
                        Some(then_stmt.as_ref())
                    } else {
                        else_stmt.as_ref().map(|b| b.as_ref())
                    };
                    match chosen {
                        Some(branch) if self.stmt_is_blocking(branch) => {
                            let branch_stmts: Vec<Statement> = match &branch.kind {
                                StatementKind::SeqBlock { stmts, .. } => stmts.clone(),
                                _ => vec![branch.clone()],
                            };
                            let mut cont = branch_stmts;
                            // Chain the caller's tail instead of copying it onto the end of
                            // the spliced body (ProcCont::pushed).
                            self.run_process_stmts(pid, &pc.pushed(cont, pc.start + i + 1));
                            return;
                        }
                        // Run the branch we already selected, rather than
                        // re-entering the whole `if`.
                        Some(branch) => self.exec_statement(branch),
                        None => {}
                    }
                    i += 1;
                    continue;
                }
            }

            // Suspend-aware Case (mirror of the If handler above). An inlined
            // task body that is a `case` containing blocking statements
            // (`case(op) wait(...); endcase`) must select its arm HERE — falling
            // through to `exec_statement(Case)` would run the matched arm's
            // `wait` on the SYNCHRONOUS path, where a false condition with no
            // parking continuation silently falls through instead of
            // blocking (IEEE 1800-2023 §9.7.4). Evaluate the selector ONCE
            // (side-effect conditions must not double-fire), pick the first
            // matching item or the default, and if the chosen arm blocks,
            // flatten its body and recurse so its waits reach the suspend-aware
            // Wait arm.
            if let StatementKind::Case {
                kind,
                expr,
                items,
                unique_priority: _,
            } = &stmt.kind
            {
                // Pattern case (`case(x) matches p: ...`) binds variables;
                // delegate to the synchronous path which performs the bindings.
                // Likewise a case with NO blocking arm: `exec_statement(Case)`
                // owns the §12.5.3 unique/unique0/priority violation checks,
                // so only intercept when some arm can actually suspend.
                if !items.iter().any(|i| i.pattern.is_some()) && self.stmt_is_blocking(stmt) {
                    let val = self.eval_expr(expr);
                    let chosen: Option<&Statement> = items.iter().find_map(|item| {
                        if item.is_default {
                            return None;
                        }
                        for pat in &item.patterns {
                            let hit = match kind {
                                CaseKind::CaseInside => self.case_inside_match(&val, pat),
                                CaseKind::Casez => {
                                    let pv = self.eval_expr(pat);
                                    val.casez_eq(&pv).is_true()
                                }
                                CaseKind::Casex => {
                                    let pv = self.eval_expr(pat);
                                    val.casex_eq(&pv).is_true()
                                }
                                _ => {
                                    let pv = self.eval_expr(pat);
                                    val.case_eq(&pv).is_true()
                                }
                            };
                            if hit {
                                return Some(&item.stmt);
                            }
                        }
                        None
                    }).or_else(|| {
                        items
                            .iter()
                            .find(|item| item.is_default)
                            .map(|item| &item.stmt)
                    });
                    match chosen {
                        Some(arm) if self.stmt_is_blocking(arm) => {
                            let arm_stmts: Vec<Statement> = match &arm.kind {
                                StatementKind::SeqBlock { stmts, .. } => stmts.clone(),
                                _ => vec![arm.clone()],
                            };
                            let mut cont = arm_stmts;
                            // Chain the caller's tail instead of copying it onto the end of
                            // the spliced body (ProcCont::pushed).
                            self.run_process_stmts(pid, &pc.pushed(cont, pc.start + i + 1));
                            return;
                        }
                        Some(arm) => self.exec_statement(arm),
                        None => {}
                    }
                    i += 1;
                    continue;
                }
            }

            // A `for` loop with a blocking body (e.g. `for (i=0; i<n; i++) task_call()`)
            // must iterate via the suspend-aware path — otherwise the synchronous
            // exec runs the body once and the loop never advances. Run the init now,
            // then lower to `while (cond) { body; step; }` and recurse.
            if let StatementKind::For {
                init,
                condition,
                step,
                body,
            } = &stmt.kind
            {
                if condition.is_some()
                    && (self.stmt_has_event_wait(body) || self.stmt_is_blocking(body))
                {
                    for fi in init {
                        match fi {
                            ForInit::VarDecl {
                                data_type,
                                name,
                                init: e,
                            } => {
                                let v = self.eval_expr(e);
                                let w = crate::compiler::elaborate::resolve_type_width(
                                    data_type,
                                    Some(&self.module.parameters),
                                    Some(&self.module.typedefs),
                                );
                                self.widths.insert(name.name.clone(), w);
                                let mut rv = v.resize(w);
                                if crate::compiler::elaborate::is_type_signed(data_type) {
                                    rv.is_signed = true;
                                }
                                if let Some(frame) = self.local_stack.last_mut() {
                                    frame.insert(name.name.clone(), rv);
                                } else {
                                    // §12.7.1: a variable declared in the for-init
                                    // has AUTOMATIC lifetime — it is local to the
                                    // loop. With no call frame (an initial block or
                                    // a fork child) it used to land in the GLOBAL
                                    // signal map, so two concurrent processes each
                                    // running `for (int i ...)` shared one counter
                                    // and clobbered each other's index. Only the
                                    // suspend-aware path can interleave, so give
                                    // the process its own frame here.
                                    let mut f: HashMap<String, Value> = HashMap::default();
                                    f.insert(name.name.clone(), rv);
                                    self.local_stack.push(f);
                                }
                            }
                            ForInit::Assign { lvalue, rvalue } => {
                                let v = self.eval_expr(rvalue);
                                self.assign_value(lvalue, &v);
                            }
                        }
                    }
                    let mut body_stmts = match &body.kind {
                        StatementKind::SeqBlock { stmts, .. } => stmts.clone(),
                        _ => vec![(**body).clone()],
                    };
                    // §12.7.2: a `continue` skips the rest of the body but the
                    // step STILL runs. Without this barrier the step — appended
                    // to the body by this very lowering — was skipped along
                    // with it, so the index never advanced and the loop hung.
                    if !step.is_empty() {
                        body_stmts
                            .push(Statement::new(StatementKind::LoopStep, stmt.span));
                    }
                    for s in step {
                        body_stmts.push(Statement::new(StatementKind::Expr(s.clone()), stmt.span));
                    }
                    let while_body = Statement::new(
                        StatementKind::SeqBlock {
                            name: None,
                            stmts: body_stmts,
                        },
                        stmt.span,
                    );
                    let while_stmt = Statement::new(
                        StatementKind::While {
                            condition: condition.clone().unwrap(),
                            body: Box::new(while_body),
                        },
                        stmt.span,
                    );
                    let mut cont = vec![while_stmt];
                    // Chain the caller's tail rather than copying it (ProcCont::pushed).
                    let cont = pc.pushed(cont, pc.start + i + 1);
                    self.continue_stmts_or_trampoline(pid, cont);
                    return;
                }
            }

            if let StatementKind::While { condition, body } = &stmt.kind {
                // Descend into a blocking while-body (event waits, #delays, or
                // blocking method calls) so each iteration suspends via the
                // suspend-aware path instead of spinning synchronously.
                if self.stmt_has_event_wait(body) || self.stmt_is_blocking(body) {
                    if self.blocking_loop_flag_gate() {
                        // `break`/`return` exits the while loop.
                        i += 1;
                        continue;
                    }
                    self.reset_hint_to_process_scope();
                    let cond_val = self.eval_expr(condition).is_true();
                    if cond_val {
                        let body_stmts = match &body.kind {
                            StatementKind::SeqBlock { stmts, .. } => stmts.clone(),
                            _ => vec![*body.clone()],
                        };
                        let mut cont: Vec<Statement> = body_stmts;
                        cont.push(stmt.clone());
                        // Chain the caller's tail rather than copying it (ProcCont::pushed).
                        let cont = pc.pushed(cont, pc.start + i + 1);
                        self.continue_stmts_or_trampoline(pid, cont);
                        return;
                    } else {
                        i += 1;
                        continue;
                    }
                }
            }

            // do...while with a blocking body: run the body once, then continue
            // as a regular while(cond) body (above). Without this, a blocking
            // do...while body spins synchronously.
            if let StatementKind::DoWhile { condition, body } = &stmt.kind {
                if self.stmt_is_blocking(body) {
                    let body_stmts = match &body.kind {
                        StatementKind::SeqBlock { stmts, .. } => stmts.clone(),
                        _ => vec![*body.clone()],
                    };
                    let mut cont: Vec<Statement> = body_stmts;
                    cont.push(Statement::new(
                        StatementKind::While {
                            condition: condition.clone(),
                            body: body.clone(),
                        },
                        stmt.span,
                    ));
                    // Chain the caller's tail instead of copying it onto the end of
                    // the spliced body (ProcCont::pushed).
                    self.run_process_stmts(pid, &pc.pushed(cont, pc.start + i + 1));
                    return;
                }
            }

            // Check for ParBlock (fork...join)
            if let StatementKind::ParBlock {
                stmts: sub_stmts,
                join_type,
                name: block_name,
                ..
            } = &stmt.kind
            {
                // Fork-block declarations run in THIS process first (§9.3.2), so
                // the children snapshot them instead of racing over them.
                let (spawnable, saved_auto_len) = self.exec_fork_block_decls(sub_stmts);
                let mut child_pids = HashSet::default();
                for s in spawnable {
                    let pid_child = self.proc.next_pid;
                    self.proc.next_pid += 1;
                    self.proc.process_parents.insert(pid_child, pid);
                    // §9.3.2: a fork child executes in the forking process's
                    // scope, so it inherits the parent's instance-scope hint
                    // (additive — a child previously had none at all).
                    if let Some(h) = self.proc.process_scope_hint.get(&pid).cloned() {
                        self.proc.process_scope_hint.insert(pid_child, h);
                    }
                    self.process_origin
                        .insert(pid_child, (s.span, "fork child"));
                    // §9.6.2: `disable <label>` where the label names this
                    // child's own top-level `begin : name` block terminates
                    // the child. `disable_labels` was populated ONLY for
                    // initial blocks at start-up, so a fork child's label was
                    // unknown: the disable found no target, fell through to
                    // the self-unwind path, and the child kept running — a
                    // later `wait fork` then blocked on it forever.
                    if let StatementKind::SeqBlock { name: Some(n), .. } = &s.kind {
                        self.proc.disable_labels.insert(n.name.clone(), pid_child);
                    }
                    self.inherit_fork_child_context(pid_child);
                    // §9.4.5: a child that IS an intra-assignment delay
                    // (`fork lhs = #d rhs; join_none`) captures its RHS at the
                    // fork point — a join_none parent keeps running and may
                    // overwrite RHS operands before the child would start, so
                    // deferring the capture to child start-up reads the
                    // post-fork values. Evaluate now and schedule the
                    // pre-computed assignment directly at t+d.
                    if let StatementKind::BlockingAssign { lvalue, rvalue } = &s.kind {
                        if let Some((d_expr, rhs)) = Self::intra_delay_marker(rvalue) {
                            let val = self.eval_expr(rhs);
                            let delay = self.eval_delay_ticks(d_expr);
                            let saved = self.make_intra_saved_expr(val, rvalue.span);
                            let assign = Statement::new(
                                StatementKind::BlockingAssign {
                                    lvalue: lvalue.clone(),
                                    rvalue: saved,
                                },
                                s.span,
                            );
                            self.event_queue
                                .schedule(self.time + delay, pid_child, vec![assign].into());
                            child_pids.insert(pid_child);
                            continue;
                        }
                    }
                    // Schedule children to run at current time
                    self.event_queue
                        .schedule(self.time, pid_child, vec![s.clone()].into());
                    child_pids.insert(pid_child);
                }
                self.auto_loop_vars.truncate(saved_auto_len);
                if let Some(nm) = block_name {
                    self.proc.fork_block_children
                        .insert(nm.name.clone(), child_pids.clone());
                }

                // An empty fork (or one whose only items were declarations)
                // has nothing to wait for and completes at once, whatever the
                // join type. A JoinWaiter is re-checked only when a child
                // finishes, so a waiter with no children would never fire —
                // `fork join` with an empty body used to drop the entire
                // continuation.
                if *join_type == JoinType::JoinNone || child_pids.is_empty() {
                    // Continue immediately
                    i += 1;
                    continue;
                } else {
                    // Suspend current process and wait for children
                    let cont = pc.resume_at(pc.start + i + 1);
                    self.proc.join_waiters.push(JoinWaiter {
                        parent_pid: pid,
                        child_pids,
                        join_type: *join_type,
                        continuation: cont,
                        finished_children: HashSet::default(),
                        wait_fork: false,
                    });
                    return;
                }
            } else {
                // §9.7: `process_handle.await()` blocks the calling process
                // until the target terminates. Must be intercepted here (not
                // in exec_method_call) because the continuation is needed.
                if let StatementKind::Expr(expr) = &stmt.kind {
                    if let Some(target_pid) = self.extract_proc_await_target(expr) {
                        let cont = pc.resume_at(pc.start + i + 1);
                        if self.proc_await(target_pid, pid, cont) {
                            return; // caller suspended — don't execute await
                        }
                        // target already terminated — fall through
                    }
                }
                // INTERNAL: `ForeachTail` continuation sentinel — run the
                // next iteration (or exit) of a blocking-body `foreach`.
                // §9.4.3: a process blocked inside the loop body resumes HERE,
                // at the next iteration, not by restarting the whole foreach.
                if let StatementKind::ForeachTail {
                    loop_var,
                    var_scope,
                    body,
                    keys,
                    is_str,
                    key_type,
                    idx,
                    fe_auto_len,
                    live_size_name,
                } = &stmt.kind
                {
                    // A `return` from an inlined task body propagates to the
                    // task's ScopePop (handled by the top-of-loop return_flag
                    // skip). Just exit this foreach without consuming flags.
                    if self.return_flag {
                        self.auto_loop_vars.truncate(*fe_auto_len);
                        i += 1;
                        continue;
                    }
                    // A `break` (set WITHOUT return_flag) exits this loop —
                    // consume it, mirroring the synchronous `while` at L30181.
                    if self.break_flag {
                        self.break_flag = false;
                        self.continue_flag = false;
                        self.auto_loop_vars.truncate(*fe_auto_len);
                        i += 1;
                        continue;
                    }
                    self.continue_flag = false;
                    // Bounds check. For dynamic arrays/queues whose size can
                    // change between iterations (because the body suspended
                    // and another process shrunk the collection), re-check the
                    // LIVE size instead of the frozen key count.
                    let exhausted = if let Some(ln) = live_size_name {
                        *idx as u64 >= self.get_queue_size(ln)
                    } else {
                        *idx >= keys.len()
                    };
                    if exhausted {
                        // loop exhausted — restore automatic-loop-var scope
                        self.auto_loop_vars.truncate(*fe_auto_len);
                        i += 1;
                        continue;
                    }
                    // advance the loop variable to keys[idx]
                    if let Some(vn) = loop_var {
                        // For a live-size collection, synthesize the index key
                        // from `idx` (the frozen keys may be stale).
                        let key_str = if live_size_name.is_some() {
                            idx.to_string()
                        } else {
                            keys[*idx].clone()
                        };
                        let kv = if *is_str {
                            Value::from_string(&key_str)
                        } else if key_type.1 {
                            let mut v = Value::from_u64(
                                key_str.parse::<i64>().unwrap_or(0) as u64,
                                key_type.0,
                            );
                            v.is_signed = true;
                            v
                        } else {
                            Value::from_u64(key_str.parse::<u64>().unwrap_or(0), key_type.0)
                        };
                        self.set_loop_var_aliased(var_scope.as_deref(), vn, kv);
                    }
                    let body_stmts = match &body.kind {
                        StatementKind::SeqBlock { stmts, .. } => stmts.clone(),
                        _ => vec![(**body).clone()],
                    };
                    let mut cont = body_stmts;
                    cont.push(Statement::new(
                        StatementKind::ForeachTail {
                            loop_var: loop_var.clone(),
                            var_scope: var_scope.clone(),
                            body: body.clone(),
                            keys: keys.clone(),
                            is_str: *is_str,
                            key_type: *key_type,
                            idx: idx + 1,
                            fe_auto_len: *fe_auto_len,
                            live_size_name: live_size_name.clone(),
                        },
                        stmt.span,
                    ));
                    // Chain the caller's tail rather than copying it (ProcCont::pushed).
                    let cont = pc.pushed(cont, pc.start + i + 1);
                    self.continue_stmts_or_trampoline(pid, cont);
                    return;
                }
                // Blocking-body `foreach` (single loop variable): unroll one
                // iteration and re-append a `ForeachTail` sentinel, exactly
                // like the `while`/`for` unroll above, so a `wait`/`#delay`
                // inside the body (often nested several inlined-task frames
                // deep) parks and resumes at the NEXT iteration instead of
                // restarting from index 0. The previous `exec_park_cont` replay
                // re-ran the whole loop (and its bodies' pre-wait side effects),
                // corrupting consuming handshakes and never actually blocking.
                if let StatementKind::Foreach { array, vars, body } = &stmt.kind {
                    if self.stmt_is_blocking(body) {
                        if let Some((keys, is_str, var_scope, live_size_name, key_type)) =
                            self.foreach_materialize_keys_1d(array, vars)
                        {
                            let fe_names: Vec<String> = vars
                                .iter()
                                .filter_map(|v| v.as_ref().map(|id| id.name.clone()))
                                .collect();
                            let fe_auto_len = self.auto_loop_vars.len();
                            // foreach index vars are automatic (§9.7.3); like
                            // for-init vars, record for fork capture.
                            if self.local_stack.last().is_none() {
                                for nm in &fe_names {
                                    self.auto_loop_vars.push(nm.clone());
                                }
                            }
                            let loop_var =
                                vars.first().and_then(|v| v.as_ref().map(|id| id.name.clone()));
                            if let Some(vn) = &loop_var {
                                self.widths.insert(vn.clone(), key_type.0);
                            }
                            if keys.is_empty() {
                                self.auto_loop_vars.truncate(fe_auto_len);
                                i += 1;
                                continue;
                            }
                            // run the FIRST iteration now
                            if let Some(vn) = &loop_var {
                                let kv = if is_str {
                                    Value::from_string(&keys[0])
                                } else if key_type.1 {
                                    let mut v = Value::from_u64(
                                        keys[0].parse::<i64>().unwrap_or(0) as u64,
                                        key_type.0,
                                    );
                                    v.is_signed = true;
                                    v
                                } else {
                                    Value::from_u64(keys[0].parse::<u64>().unwrap_or(0), key_type.0)
                                };
                                self.set_loop_var_aliased(var_scope.as_deref(), vn, kv);
                            }
                            self.continue_flag = false;
                            let body_stmts = match &body.kind {
                                StatementKind::SeqBlock { stmts, .. } => stmts.clone(),
                                _ => vec![(**body).clone()],
                            };
                            let mut cont = body_stmts;
                            cont.push(Statement::new(
                                StatementKind::ForeachTail {
                                    loop_var,
                                    var_scope,
                                    body: body.clone(),
                                    keys,
                                    is_str,
                                    key_type,
                                    idx: 1,
                                    fe_auto_len,
                                    live_size_name,
                                },
                                stmt.span,
                            ));
                            // Chain the caller's tail rather than copying it (ProcCont::pushed).
                            let cont = pc.pushed(cont, pc.start + i + 1);
                            self.continue_stmts_or_trampoline(pid, cont);
                            return;
                        }
                        // else: multi-var / unhandled shape → fall through to
                        // the synchronous exec_statement (exec_park_cont) below.
                    }
                }
                // Set up a parking continuation for blocking waits inside
                // loop bodies (foreach) processed by exec_statement's
                // synchronous path. exec_statement's Wait handler reads this
                // to park the process with `[stmt, rest]` when a wait
                // condition is false, so the process re-runs this statement
                // when resumed.
                //
                // FALLBACK only: the common single-loop-variable case is
                // handled by the suspend-aware unroll above (which resumes at
                // the NEXT iteration per §9.4.3). This replay-from-zero path
                // remains for multi-variable / unhandled shapes that
                // `foreach_materialize_keys_1d` declined.
                if matches!(&stmt.kind, StatementKind::Foreach { .. }) {
                    self.proc.exec_park_cont = Some({
                        let mut c = vec![stmt.clone()];
                        // Chain the caller's tail rather than copying it (ProcCont::pushed).
                        let c = pc.pushed(c, pc.start + i + 1);
                        c
                    });
                }
                self.exec_statement(stmt);
                self.proc.exec_park_cont = None;
                self.proc.parked_from_exec = false;
            }

            // Check for WaitFork
            if let StatementKind::WaitFork = &stmt.kind {
                // §9.6.1: `wait fork` blocks until the IMMEDIATE child
                // subprocesses of the calling process complete — and no
                // further. The asymmetry with `disable fork` (which kills all
                // DESCENDANTS) is the LRM's, not an accident. Waiting on the
                // transitive closure made a `join_none` grandchild extend the
                // wait arbitrarily — or forever, for a persistent monitor
                // spawned by a child, which is exactly the shape UVM drivers
                // use.
                let children: HashSet<usize> = self
                    .proc.process_parents
                    .iter()
                    .filter(|&(_, &parent)| parent == pid)
                    .map(|(&child, _)| child)
                    .collect();

                if children.is_empty() {
                    i += 1;
                    continue;
                } else {
                    let cont = pc.resume_at(pc.start + i + 1);
                    self.proc.join_waiters.push(JoinWaiter {
                        parent_pid: pid,
                        child_pids: children,
                        join_type: JoinType::Join,
                        continuation: cont,
                        finished_children: HashSet::default(),
                        wait_fork: true,
                    });
                    return;
                }
            }

            i += 1;
        }
        // This frame is exhausted. Follow the chain: a spliced task body or a
        // flattened block runs with the caller's tail linked behind it
        // (ProcCont::pushed), so finishing the body means continuing into the
        // caller rather than returning.
        //
        // Recursive, and deliberately so: the previous code recursed once per
        // splice too (`self.run_process_stmts(pid, &expanded); return;`), so the
        // depth is the same splice nesting it always was and RPS_DEPTH still
        // bounds it. What changed is that a level costs a pointer instead of a
        // copy of everything the caller had left.
        if !self.finished {
            if let Some(frame) = pc.next.clone() {
                self.run_process_stmts(pid, &frame);
            }
        }
    }
    pub(crate) fn proc_await(
        &mut self,
        target_pid: usize,
        caller_pid: usize,
        continuation: ProcCont,
    ) -> bool {
        let terminated = self.proc.killed_pids.contains(&target_pid)
            || !self.is_pid_suspended(target_pid) && target_pid != self.proc.current_pid;
        if terminated {
            return false; // already done — caller continues
        }
        self.proc.await_waiters.push(AwaitWaiter {
            target_pid,
            waiter_pid: caller_pid,
            continuation,
        });
        true
    }
    pub(crate) fn proc_suspend(&mut self, pid: usize) {
        if self.proc.suspended_pids.contains(&pid) || self.proc.killed_pids.contains(&pid) {
            return; // already suspended or killed — no effect
        }
        // Try to extract the process from wherever it's parked.
        // 1) Delay (event_queue)
        if let Some((expiry, stmts)) = self.event_queue.remove_pid(pid) {
            self.proc.suspended_pids.insert(pid);
            self.proc.suspended_proc_info.insert(
                pid,
                SuspendedProc {
                continuation: stmts,
                original_delay_expiry: Some(expiry),
                },
            );
            return;
        }
        // 2) Event control (event_waiters)
        if let Some(idx) = self.ipc.event_waiters.iter().position(|w| w.pid == pid) {
            let waiter = self.ipc.event_waiters.remove(idx);
            self.proc.suspended_pids.insert(pid);
            self.proc.suspended_proc_info.insert(
                pid,
                SuspendedProc {
                continuation: waiter.continuation,
                original_delay_expiry: None,
                },
            );
            return;
        }
        // 3) Condition wait (wait(expr))
        if let Some(idx) = self.proc.condition_waiters.iter().position(|(p, _)| *p == pid) {
            let (_, stmts) = self.proc.condition_waiters.remove(idx);
            self.proc.suspended_pids.insert(pid);
            self.proc.suspended_proc_info.insert(
                pid,
                SuspendedProc {
                continuation: stmts,
                original_delay_expiry: None,
                },
            );
            return;
        }
        // 4) Inactive queue (#0)
        if let Some(idx) = self.proc.inactive_queue.iter().position(|(p, _)| *p == pid) {
            let (_, stmts) = self.proc.inactive_queue.remove(idx);
            self.proc.suspended_pids.insert(pid);
            self.proc.suspended_proc_info.insert(
                pid,
                SuspendedProc {
                continuation: stmts,
                original_delay_expiry: Some(self.time), // #0 — expired immediately
                },
            );
            return;
        }
        // 5) Mailbox get / semaphore get
        for q in self.ipc.mailbox_get_waiters.values_mut() {
            if let Some(idx) = q.iter().position(|w| w.pid == pid) {
                if let Some(waiter) = q.remove(idx) {
                    self.proc.suspended_pids.insert(pid);
                    self.proc.suspended_proc_info.insert(
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
        for q in self.ipc.semaphore_get_waiters.values_mut() {
            if let Some(idx) = q.iter().position(|w| w.pid == pid) {
                if let Some(waiter) = q.remove(idx) {
                    self.proc.suspended_pids.insert(pid);
                    self.proc.suspended_proc_info.insert(
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
    pub(crate) fn proc_resume(&mut self, pid: usize) {
        if let Some(info) = self.proc.suspended_proc_info.remove(&pid) {
            self.proc.suspended_pids.remove(&pid);
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

}
