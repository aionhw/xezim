//! Running the statements of a process.

use super::*;

impl Simulator {
    pub(in crate::compiler::simulator) fn run_process_stmts(&mut self, pid: usize, pc: &ProcCont) {
        // A call chain that never waits nests one activation per call; stop
        // it with a diagnostic before it exhausts the native stack.
        let at_limit = self.stall_limit != 0 && u64::from(self.stmts_depth) >= self.stall_limit;
        if (at_limit || self.stmts_depth >= 64) && self.process_nesting_exceeded(pid) {
            return;
        }
        self.stmts_depth += 1;
        self.run_process_stmts_scoped(pid, pc);
        self.stmts_depth -= 1;
    }

    /// Whether the activation about to nest at depth `stmts_depth` must stop.
    /// A task that can wait (`@`, `#`, `wait`) runs inside its caller's
    /// activation and resumes the caller's remaining statements from inside
    /// its own, so a process whose calls never actually wait — a task that
    /// calls itself, or two that call each other, at one time step — nests
    /// one native frame per call. That is a zero-delay loop: simulated time
    /// can never advance. It is reported, naming the process, when the depth
    /// reaches `XEZIM_STALL_LIMIT` or when the native stack is nearly
    /// exhausted, whichever comes first; overflowing the stack aborted the
    /// whole run with no message about the design.
    #[cold]
    #[inline(never)]
    fn process_nesting_exceeded(&mut self, pid: usize) -> bool {
        if self.finished {
            return true;
        }
        let depth = u64::from(self.stmts_depth);
        let over_limit = self.stall_limit != 0 && depth >= self.stall_limit;
        let low_stack = depth % 16 == 0 && stack_nearly_exhausted();
        if !over_limit && !low_stack {
            return false;
        }
        eprintln!(
            "[xezim][error] simulation STALLED at time {} — a process nested {} task calls and blocks without waiting.",
            self.time, depth
        );
        let id = self.stall_pid_identity(pid);
        eprintln!(
            "{} — calls a task that can wait but does not, again and again",
            id
        );
        if let Some(src) = self.stall_pid_source(pid) {
            eprintln!("                 its source: {}", src);
        }
        eprintln!(
            "               This is a zero-delay loop (a task that calls itself, or tasks that call each"
        );
        eprintln!(
            "               other, with no wait taken): simulated time can never advance past it."
        );
        if low_stack {
            eprintln!(
                "               It was stopped before the native stack ran out. If the recursion is finite,"
            );
            eprintln!("               raise the stack with XEZIM_STACK_MB=<MiB>.");
        } else {
            eprintln!(
                "               Raise or disable the check with XEZIM_STALL_LIMIT=<n> (0 = never stall-check)."
            );
        }
        self.abort_run();
        true
    }

    fn run_process_stmts_scoped(&mut self, pid: usize, pc: &ProcCont) {
        // Install this activation's own scope (see `activation_scope`) —
        // the ratchet-proof twin of the hint reset performed inside. A
        // mid-block hierarchical reference (`u_child.x = 1` right before
        // a loop) ratchets the hint away, but a bare name this process's
        // own scope declares must still resolve there, never downward
        // into the instance. Saved and restored around the activation so
        // a re-entrant call (synchronous task re-entry) restores its
        // caller's scope, and compile-time evals after this activation
        // see None, not a stale scope. The body lives in
        // run_process_stmts_inner because a Drop guard holding
        // &self.activation_scope would block every &mut self call in it.
        // An active free task (including a root task whose scope is the
        // empty string) takes precedence over the process's declaration scope.
        let task_scope = self.task_cleanup.iter().rev().find_map(|t| {
            t.frame_scope_hint
                .as_ref()
                .map(|scope| (scope.clone(), t.caller_body_scope.is_some()))
        });
        // The scheduled process's context has been installed by this point.
        // Looking at task_cleanup before that swap reads another process's
        // frame and can bind both sibling instances to the first one's scope.
        if let Some((scope, free_task)) = &task_scope {
            *self.name_resolve_hint.borrow_mut() = (!scope.is_empty()).then(|| scope.clone());
            if *free_task {
                self.current_scope.clone_from(scope);
            }
        }
        let scope = task_scope
            .as_ref()
            .map(|(scope, _)| scope.as_str())
            .or_else(|| self.process_scope_hint.get(&pid).map(String::as_str))
            .unwrap_or("");
        let same_scope = self.activation_scope.borrow().as_deref() == Some(scope);
        if same_scope {
            return self.run_process_stmts_inner(pid, pc);
        }
        let scope = scope.to_string();
        let saved_activation_scope = self.activation_scope.replace(Some(scope));
        self.run_process_stmts_inner(pid, pc);
        *self.activation_scope.borrow_mut() = saved_activation_scope;
    }

    pub(in crate::compiler::simulator) fn run_process_stmts_inner(
        &mut self,
        pid: usize,
        pc: &ProcCont,
    ) {
        let stmts: &[Statement] = pc.frame();
        self.current_pid = pid;
        // §16.4.2: the flush happens where the process resumes
        // (`run_scheduled_process_inner`); every continuation of the running
        // activation enters here too (a chained frame, an `always` body
        // looping back to its event control), and is none.
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
        // Only count TOP-LEVEL (depth == 0) activations toward the per-pid
        // zero-delay livelock limit. A re-entrant call (depth > 0) is a
        // synchronous sub-invocation fired from within this same activation —
        // e.g. a function/task the process called, or a trampolined
        // while/for/repeat body that re-entered run_process_stmts. Such
        // recursion is bounded (it returns) and is NOT the process re-arming
        // itself via a #0 / event wait, so inflating `stall_pid_hits` with it
        // would conflate ordinary recursion with genuine zero-delay livelock
        // (a process that keeps re-scheduling at one time and never lets the
        // clock advance). The livelock pattern only manifests at the outermost
        // dispatch, so the guard stays accurate while re-entrant work is free
        // to recurse up to the trampoline/stack limits above.
        if self.stall_limit > 0 && depth == 0 {
            // One process re-activated `stall_limit` times at a SINGLE
            // timestamp with NO observable state change in between is not a
            // busy design, it is a livelock: it keeps re-arming itself and
            // time can never advance (`stall_pid_tripped` is progress-aware
            // unless XEZIM_STALL_LIMIT was set explicitly). Counting
            // per-process, rather than counting activations at the timestamp,
            // keeps a wide design that legitimately wakes thousands of
            // DISTINCT processes at time 0 safe. run_one_tick's inner drain
            // loop re-reads the queue at the current time, so a
            // self-rescheduling process never reaches the outer event loop —
            // the check has to live here. The activation that trips the guard
            // is re-parked UNEXECUTED and must not inflate "ran N times".
            if self.stall_pid_tripped(pid, Some(pc)) {
                // A single process re-arming like this is a zero-delay
                // livelock (e.g. `always #(period) clk=~clk` whose real
                // `period` momentarily glitched to 0). Rather than abort the
                // whole run here, RE-PARK this un-executed activation at the
                // current time and ask the outer event loop to
                // defer-and-advance to the next scheduled event — a
                // commercial simulator lets time move past a #0 spinner once
                // its driving value recovers. The outer loop declares it
                // fatal only if nothing is scheduled ahead.
                self.event_queue.schedule(self.time, pid, pc.clone());
                self.zero_delay_defer_pending = true;
                return;
            }
        }
        sim_dbg_eprintln!(
            "[DEBUG] running process {} ({} stmts) at time {}",
            pid,
            stmts.len(),
            self.time
        );
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

            // §6.21: the trailing `Null` of a scoped block's frame closes the
            // block's scope, whatever flag is unwinding through it.
            if matches!(stmt.kind, StatementKind::Null)
                && i + 1 == stmts.len()
                && !self.block_scope_saves.is_empty()
            {
                self.block_scope_sentinel(pc);
            }

            // §35.5.2: the resume marker of a process parked inside an
            // imported task (dpi_task.rs). Ahead of the flag skips below: a
            // disable posted meanwhile unwinds the task before the block.
            if !self.dpi_task_names.is_empty() {
                if let Some(carry_on) = self.dpi_task_marker(pc, i, stmt) {
                    if !carry_on {
                        return;
                    }
                    i += 1;
                    continue;
                }
            }

            // An inlined task/method `return` (return_flag set) must unwind to
            // the enclosing task's `ScopePop` sentinel, skipping the rest of the
            // task body in between (including blocking statements after the `return`).
            // The ScopePop runs `unwind_task_frame`, which restores the caller's
            // saved break/continue/return flags — consuming the return. Without
            // this, return_flag/break_flag leaked past the ScopePop and skipped
            // the caller's next real statement.
            if self.return_flag {
                match &stmt.kind {
                    StatementKind::ScopePop => {
                        if let Some(c) = self.task_cleanup.pop() {
                            self.unwind_task_frame(c);
                        } else {
                            self.return_flag = false;
                        }
                    }
                    StatementKind::ForeachTail { fe_auto_len, .. } => {
                        self.auto_loop_vars.truncate(*fe_auto_len);
                    }
                    _ => {}
                }
                i += 1;
                continue;
            }

            // §9.6.2 `disable <block|task>`: skip to the end of the named
            // block's frame (or the task's `ScopePop`), running only the task
            // cleanups passed on the way.
            if self.break_flag && self.disable_target.is_some() && self.disable_unwinding(pid, pc) {
                if let StatementKind::ScopePop = &stmt.kind {
                    self.disable_pop_task_frame();
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

            // §12.7 / §9.3.3: while a `break`/`continue` is pending, the REST of
            // the loop body must be skipped — INCLUDING blocking statements. The
            // synchronous `exec_statement` already no-ops every statement while
            // these flags are set (its top guard), and `blocking_loop_flag_gate`
            // relies on that: it assumes "the body statements after the `continue`
            // were already skipped". But a BLOCKING statement — a call to a
            // time-consuming task/method, a `#delay`, an `@event`, a `fork`, or a
            // blocking begin/end — is intercepted in THIS function AHEAD of
            // `exec_statement`, and inlining a blocking task even SAVES AND CLEARS
            // these flags for the callee's body (`bind_task_frame`). So without
            // this guard a `continue`/`break` guarding a call to a blocking
            // subroutine was ignored and the call ran for the skipped iterations
            // — bug 28: the mesh smoke seq's
            // `if (!has_neighbor(r,dir)) continue; send_directed(...);` leaked
            // off-mesh flits because `send_directed` blocks in
            // `start_item`/`finish_item`. Inline `#delay`s escaped the bug only
            // because the flag PERSISTS across suspend/resume, so the post-delay
            // assignment was still skipped; a called task clears it.
            //
            // Do NOT skip the statements that CONSUME the flag: `LoopStep`
            // (continue's barrier, handled just above), `ScopePop`
            // (task-frame cleanup), the `ForeachTail`/`ForeverTail` re-entry
            // sentinels (unambiguous — `Foreach`/`Forever` always lower to
            // one of these for re-entry, so the BASE kind is never itself a
            // tail and is never exempted here), and — ONLY at the exact
            // position `pc.loop_tail_at` names — a `While`/`Repeat` re-entry.
            //
            // That position check matters because those two kinds are
            // reused for BOTH the genuine re-entry AND an ordinary fresh
            // occurrence of the same kind reached for the first time right
            // after the `break`/`continue` (a nested loop in source, sitting
            // in the body BEFORE the re-appended tail). The two are the same
            // `StatementKind` variant, so matching on kind alone treats the
            // fresh one as if it were consuming ITS OWN flag: a `continue`
            // followed by a nested `for`/`while`/`repeat` ran anyway, and a
            // `break` followed by a nested `foreach`/loop got swallowed by
            // it instead of stopping the outer loop (PR #174 review, on top
            // of the original bug 28 fix — e.g. the mesh seq's
            // `if (!has_neighbor(...)) continue; for (dir2 ...) ...`).
            // `pc.loop_tail_at`, set only at the three sites that actually
            // re-append a `While`/`Repeat` as a tail (see its doc comment on
            // `ProcCont`; `For`/`DoWhile` lower to a `While` tail and are
            // themselves only ever reached fresh), is what tells the two
            // apart. A `disable` (disable_target set) drives its own unwind
            // through `break_flag` — leave that path untouched.
            if (self.break_flag || self.continue_flag) && self.disable_target.is_none() {
                let is_flag_consumer = matches!(
                    &stmt.kind,
                    StatementKind::ScopePop
                        | StatementKind::ForeachTail { .. }
                        | StatementKind::ForeverTail { .. }
                ) || (pc.loop_tail_at == Some(pc.start + i)
                    && matches!(
                        &stmt.kind,
                        StatementKind::While { .. } | StatementKind::Repeat { .. }
                    ));
                if !is_flag_consumer {
                    i += 1;
                    continue;
                }
            }
            // §38.36.1 cbStmt: statements this loop runs itself are reported
            // here (see `vpi_stmt_hook`).
            if self.vpi_cb_mask & vpi_cb::m(vpi_cb::CB_STMT) != 0 {
                self.vpi_stmt_hook(stmt, true);
            }

            // §35.5.2: a call of an imported task runs on its own stack, so
            // the process can park while the task waits (dpi_task.rs).
            if !self.dpi_task_names.is_empty() && self.dpi_cur_task.is_none() {
                if let Some(returned) = self.dpi_task_start(pid, pc, i, stmt) {
                    if !returned {
                        return;
                    }
                    i += 1;
                    continue;
                }
            }

            // Expand SeqBlocks: flatten begin/end so that timing controls and waits
            // inside them are properly handled with process suspension.
            if let StatementKind::SeqBlock { stmts: inner, name } = &stmt.kind {
                if self.stmts_have_blocking(inner) {
                    // §6.21: a block that declares variables opens a scope,
                    // closed by its frame's trailing `Null`.
                    let scope = self.block_scope_enter(inner);
                    // Immutable AST: share one Arc frame per AST node
                    // instead of cloning the whole list per process entry.
                    let frame = match name {
                        None if scope.is_some() => self.scoped_block_frame(&pc.stmts, stmt, inner),
                        None => self.blocking_cont_frame(&pc.stmts, stmt, 0, inner),
                        Some(n) => self.named_block_frame(&pc.stmts, stmt, inner, &n.name),
                    };
                    // §21.2.1.7: a NAMED block adds its name to the `%m`
                    // hierarchy while the process runs (and parks) in it;
                    // the frame's end pops it (see the frame-exhausted arm).
                    if let Some(n) = name {
                        self.m_scope_stack.push(n.name.clone());
                    }
                    if let Some(mut sv) = scope {
                        sv.frame_key = Self::frame_key(&frame);
                        self.block_scope_saves.push(sv);
                    }
                    self.run_process_stmts(pid, &pc.pushed_frame(frame, pc.start + i + 1));
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
                            .this_stack
                            .last()
                            .copied()
                            .flatten()
                            .and_then(|hh| {
                                self.heap
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
                                self.heap
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
                                self.heap
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
                    let cont = vec![call_stmt];
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
            if let StatementKind::Expr(expr) = &stmt.kind
                && (self.module.dpi_imports.is_empty() || self.dpi_task_callee(expr).is_none())
            {
                if let ExprKind::Call { func, args } = &expr.kind {
                    if let ExprKind::Ident(h) = &func.kind {
                        if h.path.len() == 1 {
                            // A bare name that is ALSO a method on the current
                            // `this`'s class must inline via the method path
                            // (Stage 1b, with this-context), not as a free task —
                            // otherwise free task resolution without `this` causes
                            // member accesses to fail to resolve. Probe
                            // `task_decl_rc` first so the 99.9% of bare calls
                            // that are not module/package tasks skip the
                            // `this_stack` + `class_has_method` check entirely.
                            let tname = h.path[0].name.name.as_str();
                            // §23.8: in a method of a class declared inside a
                            // module the bare name is that instance's task
                            // (see `class_module_subroutine_key`); None
                            // elsewhere, leaving the free-task lookup as is.
                            let modclass_key = self.class_module_subroutine_key(tname);
                            let td = match &modclass_key {
                                Some(full) => self.task_decl_rc(full),
                                None => self.task_decl_rc(tname),
                            };
                            if let Some(td) = td {
                                let is_this_method = self
                                    .this_stack
                                    .last()
                                    .copied()
                                    .flatten()
                                    .and_then(|hh| self.heap.get(hh).and_then(|o| o.as_ref()))
                                    .map(|inst| inst.class_name.clone())
                                    .is_some_and(|cls| self.class_has_method(&cls, tname))
                                    // §8.10: in a static method a bare name binds
                                    // to the class's methods before any
                                    // compilation-unit task of the same name.
                                    || self
                                        .static_class_context()
                                        .is_some_and(|cls| self.class_has_method(&cls, tname));
                                if !is_this_method && self.stmts_have_blocking(&td.items) {
                                    let mut cleanup = self.bind_task_frame(&td, args, None);
                                    // §13.4/§13.3.2: a free task body must NOT
                                    // see the caller's class context — task-scope
                                    // `static` locals key off
                                    // `class_context_stack` at their declaration,
                                    // so an unshielded body silently gave every
                                    // CALLING CLASS its own copy of the static
                                    // cell (uvm_wait_for_nba_region's shared
                                    // nba/next_nba barrier split per caller
                                    // class). Self-guarded: only shields when
                                    // the caller actually has a `this`.
                                    self.push_instance_task_context(&mut cleanup);
                                    self.enter_free_task_scope(&td, &mut cleanup);
                                    self.task_cleanup.push(cleanup);
                                    let frame = if self.share_task_frames {
                                        self.task_body_frame(&td, stmt.span)
                                    } else {
                                        None
                                    };
                                    if let Some(frame) = frame {
                                        self.run_process_stmts(
                                            pid,
                                            &pc.pushed_frame(frame, pc.start + i + 1),
                                        );
                                    } else {
                                        // Preserve first-call lifetimes and checkpoint numbering.
                                        let mut cont: Vec<Statement> =
                                            crate::ast::cache::with_fresh_scope_caches(|| {
                                                td.items.to_vec()
                                            });
                                        cont.push(Statement::new(
                                            StatementKind::ScopePop,
                                            stmt.span,
                                        ));
                                        self.run_process_stmts(
                                            pid,
                                            &pc.pushed(cont, pc.start + i + 1),
                                        );
                                    }
                                    return;
                                }
                            }
                        }
                    }
                }
            }

            // Stage 1-pkg: a blocking PACKAGE task, `pkg::t(args)` — flattened
            // to Ident([pkg, t]) at module scope, a MemberAccess inside a
            // subroutine body. Stage 1 matches single-segment names only, so
            // these ran on the synchronous path, whose nested delay loop
            // never hands control back: a forked `pkg::t()` with a `forever`
            // body kept its parent's `#25 $finish` from ever running.
            if let StatementKind::Expr(expr) = &stmt.kind
                && (self.module.dpi_imports.is_empty() || self.dpi_task_callee(expr).is_none())
            {
                if let ExprKind::Call { func, args } = &expr.kind {
                    if let Some(td) = self.package_task_target(func) {
                        if self.stmts_have_blocking(&td.items) {
                            let mut cleanup = self.bind_task_frame(&td, args, None);
                            // §13.4/§13.3.2 class-context shield — see the
                            // bare free-task arm above (Stage 1).
                            self.push_instance_task_context(&mut cleanup);
                            self.task_cleanup.push(cleanup);
                            let mut cont: Vec<Statement> =
                                crate::ast::cache::with_fresh_scope_caches(|| td.items.to_vec());
                            cont.push(Statement::new(StatementKind::ScopePop, stmt.span));
                            self.run_process_stmts(pid, &pc.pushed(cont, pc.start + i + 1));
                            return;
                        }
                    }
                }
            }

            // Stage 1-hier: a call to a blocking HIERARCHICAL or interface
            // task — `u_m.sample(args)` / `vif.sample(args)` (LRM §25.5.4,
            // §25.9) — flattens to Call{func: Ident([...])} with 2+ segments.
            // Stage 1 above only matches single-segment names, so these fell
            // through to the synchronous path, whose event controls ABORT the
            // body instead of suspending: a monitor BFM's `@(negedge pclk)`
            // returned immediately and the proxy's forever loop spun at t=0
            // (found running a public UVM AVIP). Resolve exactly like the
            // synchronous Call arm (vif alias on the head, then the joined
            // path, then hint-prefixed) and inline when the body blocks; the
            // frame carries the instance scope so the body's bare interface
            // member names resolve across suspends.
            if let StatementKind::Expr(expr) = &stmt.kind
                && (self.module.dpi_imports.is_empty() || self.dpi_task_callee(expr).is_none())
            {
                if let ExprKind::Call { func, args } = &expr.kind {
                    {
                        {
                            let full = self.resolve_hier_task_target(func);
                            if let Some(full) = full {
                                let td = self.task_decl_rc(&full).unwrap();
                                if self.stmts_have_blocking(&td.items) {
                                    let mut cleanup = self.bind_task_frame(&td, args, None);
                                    self.push_instance_task_context(&mut cleanup);
                                    self.enter_free_task_scope(&td, &mut cleanup);
                                    self.task_cleanup.push(cleanup);
                                    let frame = if self.share_task_frames {
                                        self.task_body_frame(&td, stmt.span)
                                    } else {
                                        None
                                    };
                                    if let Some(frame) = frame {
                                        self.run_process_stmts(
                                            pid,
                                            &pc.pushed_frame(frame, pc.start + i + 1),
                                        );
                                    } else {
                                        // Preserve first-call lifetimes and checkpoint numbering.
                                        let mut cont: Vec<Statement> =
                                            crate::ast::cache::with_fresh_scope_caches(|| {
                                                td.items.to_vec()
                                            });
                                        cont.push(Statement::new(
                                            StatementKind::ScopePop,
                                            stmt.span,
                                        ));
                                        self.run_process_stmts(
                                            pid,
                                            &pc.pushed(cont, pc.start + i + 1),
                                        );
                                    }
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
                    // Preserve receivers evaluated by this probe, including
                    // selected elements, for the nonblocking fallback.
                    let mut call_recv: Option<(usize, String)> = None;
                    let resolved: Option<(usize, String)> = match &func.kind {
                        // (receiver_handle, method_name)
                        ExprKind::Ident(h) if h.path.len() == 1 => self
                            .this_stack
                            .last()
                            .copied()
                            .flatten()
                            .map(|hh| (hh, h.path[0].name.name.clone())),
                        // `a.m(args)` and deeper chains `a.b.m(args)` both
                        // parse as a flattened multi-segment Ident; the
                        // receiver is the path minus the trailing method
                        // segment. Only the 2-segment form was recognised, so
                        // a blocking method reached through a nested handle
                        // (`w.fifo.peek_it(x)` — the uvm_tlm_fifo shape) fell
                        // through to the SYNCHRONOUS executor, where a
                        // blocking mailbox peek cannot suspend and returned
                        // garbage immediately.
                        ExprKind::Ident(h) if h.path.len() >= 2 => {
                            let mut head = h.clone();
                            let last = head.path.pop().unwrap();
                            let recv = Expression::new(ExprKind::Ident(head), expr.span);
                            let selected = matches!(&recv.kind, ExprKind::Ident(hp)
                                if hp.path.iter().any(|s| !s.selects.is_empty()));
                            let handle = if selected {
                                if self
                                    .class_member_names()
                                    .tasks
                                    .contains(last.name.name.as_str())
                                {
                                    let handle = self.eval_live_object_receiver(&recv);
                                    call_recv = handle.map(|hh| (hh, last.name.name.clone()));
                                    handle
                                } else {
                                    None
                                }
                            } else {
                                match self.eval_handle_expr(&recv) {
                                    // §23.6: a hierarchical receiver
                                    // (`u_mem.obj.t()`) names another
                                    // instance's handle, which the
                                    // frame-local walk above cannot read;
                                    // the call ran synchronously and a
                                    // blocking body's event control was
                                    // dropped.
                                    None if self
                                        .class_member_names()
                                        .tasks
                                        .contains(last.name.name.as_str()) =>
                                    {
                                        let handle = self.eval_live_object_receiver(&recv);
                                        call_recv = handle.map(|hh| (hh, last.name.name.clone()));
                                        handle
                                    }
                                    hh => hh,
                                }
                            };
                            handle.map(|hh| (hh, last.name.name))
                        }
                        ExprKind::MemberAccess { expr: recv, member } => {
                            match self.eval_handle_expr(recv) {
                                None if !matches!(&recv.kind, ExprKind::Call { .. })
                                    && self
                                        .class_member_names()
                                        .tasks
                                        .contains(member.name.as_str()) =>
                                {
                                    let handle = self.eval_live_object_receiver(recv);
                                    call_recv = handle.map(|hh| (hh, member.name.clone()));
                                    call_recv.clone()
                                }
                                Some(hh) => Some((hh, member.name.clone())),
                                // A receiver reached through a call result
                                // (`h.pool.get("done").wait_trigger()`) has no
                                // side-effect-free handle form, so the call
                                // ran synchronously and a blocking task
                                // returned at once. For a name some class
                                // declares as a task, run the receiver call
                                // here and keep its result.
                                None if self
                                    .class_member_names()
                                    .tasks
                                    .contains(member.name.as_str()) =>
                                {
                                    match Self::recv_call_node(recv) {
                                        Some(call) => {
                                            let v = self.eval_expr(call);
                                            let hh = self.recv_chain_handle(
                                                v.to_u64().unwrap_or(0) as usize,
                                                recv,
                                            );
                                            call_recv = Some((hh, member.name.clone()));
                                            Some((hh, member.name.clone()))
                                        }
                                        None => None,
                                    }
                                }
                                None => None,
                            }
                        }
                        _ => None,
                    };
                    if let Some((rh, mn)) = resolved {
                        if rh != 0 {
                            let is_super = matches!(&func.kind, ExprKind::MemberAccess { expr: recv, .. }
                                if matches!(&recv.kind, ExprKind::Ident(h) if h.path.len() == 1 && h.path[0].name.name == "super"));
                            let cls = if is_super {
                                self.class_context_stack
                                    .last()
                                    .cloned()
                                    .flatten()
                                    .and_then(|c| self.module.classes.get(&c))
                                    .and_then(|cd| cd.extends.clone())
                            } else {
                                self.heap
                                    .get(rh)
                                    .and_then(|o| o.as_ref())
                                    .map(|inst| inst.class_name.clone())
                            };
                            if let Some(cls) = cls {
                                if let Some((tm, mclass)) = self.resolve_class_task(&cls, &mn) {
                                    let crate::ast::decl::ClassMethodKind::Task(td) = &tm.kind
                                    else {
                                        unreachable!("resolve_class_task yields tasks only")
                                    };
                                    if self.stmts_have_blocking(&td.items) {
                                        let spec = self
                                            .task_method_spec(Some(rh), &mclass)
                                            .or_else(|| self.current_spec.clone());
                                        let mut cleanup = self.bind_task_frame(
                                            td,
                                            args,
                                            Some((spec, Some(rh), mclass.clone())),
                                        );
                                        // Even a same-object bare call changes
                                        // the defining-class context used by
                                        // `super`; push both stacks uniformly.
                                        self.push_task_method_this(
                                            Some(rh),
                                            mclass.clone(),
                                            &mut cleanup,
                                        );
                                        self.task_cleanup.push(cleanup);
                                        // Wait-free compiled fast path (see the
                                        // twin site below): a compiling task
                                        // body runs synchronously; skip the
                                        // process-frame machinery.
                                        if let Some(_v) =
                                            self.try_compiled_task_body(rh, &mclass, &mn, td)
                                        {
                                            if let Some(c) = self.task_cleanup.pop() {
                                                self.unwind_task_frame(c);
                                            }
                                            // The body ran synchronously; fall
                                            // through to the caller's next
                                            // statement (`i += 1`) instead of
                                            // returning — `return` would drop
                                            // the rest of this process's slice
                                            // (the `#5`/item_done tail of a
                                            // tx_driver loop), killing the
                                            // handshake pipeline.
                                            i += 1;
                                            continue;
                                        }
                                        self.refresh_bound_task_string_kinds(&td.ports);
                                        let frame = self.class_task_body_frame(&tm, stmt.span);
                                        // Chain the caller's tail instead of copying it onto the end of
                                        // the spliced body (ProcCont::pushed_frame).
                                        self.run_process_stmts(
                                            pid,
                                            &pc.pushed_frame(frame, pc.start + i + 1),
                                        );
                                        return;
                                    }
                                }
                            }
                        }
                    }
                    // Not inlined: run the call with the receiver value
                    // already computed above rather than evaluating it again.
                    if let Some((rh, member)) = call_recv {
                        let is_obj = rh != 0
                            && self.heap.get(rh).is_some_and(|o| o.is_some())
                            && !self.mailboxes.contains_key(&rh)
                            && !self.semaphores.contains_key(&rh);
                        if is_obj && self.cg_index(rh).is_none() {
                            self.exec_method_call(rh, &member, args);
                            i += 1;
                            continue;
                        }
                        // Not a class object (a mailbox, semaphore, process
                        // or null handle, or a collection property): leave
                        // the call to the later stages and the synchronous
                        // path, which evaluate the receiver themselves.
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
                    let scoped: Option<(String, String, Option<(String, String)>)> =
                        match &func.kind {
                            // §8.10, §13.3: a bare `m(args)` inside a static
                            // method (no `this`) names a method of the
                            // enclosing class. Stage 1b needs a live `this`,
                            // so without this arm a blocking static task
                            // called by bare name ran synchronously and its
                            // waits (a mailbox `get`, `@`, `wait`) returned
                            // at once.
                            ExprKind::Ident(h) if h.path.len() == 1 => self
                                .static_class_context()
                                .map(|c| (c, h.path[0].name.name.clone(), None)),
                            ExprKind::Ident(h)
                                if h.path.len() == 2
                                    && self.module.classes.contains_key(&h.path[0].name.name) =>
                            {
                                Some((
                                    h.path[0].name.name.clone(),
                                    h.path[1].name.name.clone(),
                                    None,
                                ))
                            }
                            ExprKind::Ident(h) if h.path.len() == 2 => {
                                // A module/package-scoped TYPEDEF receiver
                                // (`T::method` where `T` aliases a class or a
                                // specialization, e.g. `typedef
                                // uvm_config_db#(uvm_bitstream_t)
                                // uvm_config_int;`). The typedef alias is not
                                // itself a class, so follow it to the real class
                                // (and its specialization) so a blocking static
                                // task call is recognized and inlined like the
                                // `Class::` / `Class#(params)::` spellings.
                                let tname = &h.path[0].name.name;
                                let spec = self.resolve_typedef_spec(tname);
                                spec.clone()
                                    .map(|(b, _s)| (b, h.path[1].name.name.clone(), spec))
                                    .or_else(|| {
                                        self.resolve_simple_typedef_class(tname)
                                            .map(|c| (c, h.path[1].name.name.clone(), None))
                                    })
                            }
                            ExprKind::MemberAccess { expr: recv, member } => match &recv.kind {
                                // `C#(T)::method(...)` — the receiver is a
                                // parameterized static class. Stage 1b's receiver
                                // is a HANDLE; a `Specialization` receiver is a
                                // TYPE, so it reaches here (static form). Build
                                // the concrete specialized class name and classify
                                // its blocking method like the plain static form.
                                ExprKind::Specialization {
                                    base,
                                    type_args_text,
                                } => {
                                    let bn = Self::leaf_ident_name(base).unwrap_or_default();
                                    if bn.is_empty() {
                                        None
                                    } else {
                                        let spec = format!(
                                            "{}#({})",
                                            bn,
                                            Self::normalize_spec_ws(type_args_text)
                                        );
                                        let mut cand = vec![spec.clone(), bn];
                                        cand.retain(|c| self.module.classes.contains_key(c));
                                        cand.first().map(|c| (c.clone(), member.name.clone(), None))
                                    }
                                }
                                ExprKind::Ident(h) if h.path.len() == 1 => {
                                    let bn = &h.path[0].name.name;
                                    if self.module.classes.contains_key(bn) {
                                        Some((bn.clone(), member.name.clone(), None))
                                    } else {
                                        // A module/package TYPEDEF receiver
                                        // (`uvm_config_int::wait_modified` where
                                        // `typedef uvm_config_db#(uvm_bitstream_t)
                                        // uvm_config_int;`): the typedef alias is
                                        // not itself a class, so the static-call
                                        // dispatcher must follow it to the real
                                        // class (and its specialization, so the
                                        // static-collection store keys match the
                                        // `Class#(params)::` spelling) before
                                        // inlining a blocking task. Otherwise the
                                        // call is treated as synchronous and its
                                        // event wait spins forever.
                                        let spec = self.resolve_typedef_spec(bn);
                                        let simple = if spec.is_none() {
                                            self.resolve_simple_typedef_class(bn)
                                                .map(|c| (c.clone(), member.name.clone(), None))
                                        } else {
                                            None
                                        };
                                        spec.clone()
                                            .map(|(b, _s)| (b, member.name.clone(), spec))
                                            .or(simple)
                                    }
                                }
                                _ => None,
                            },
                            _ => None,
                        };
                    if let Some((cls, mn, tf_spec)) = scoped {
                        // A `C#(params)::task` receiver dispatches on the
                        // bare class but must ALSO establish the receiver
                        // specialization as `current_spec`, or a bare type
                        // argument `T` in the task body (e.g.
                        // `tester#(uvm_object)::do_it()` constructing
                        // `my_cb#(T)` / `uvm_event#(T)`) resolves to the
                        // literal `T` instead of `uvm_object`. The static
                        // METHOD path seeds `current_spec` from the
                        // specialization (§8.25); this static-task path was
                        // missing it, so the body bound its parameterized
                        // locals with `type_bindings={T:"T"}`, casting to
                        // `uvm_callback` failed, and every `$cast` inside the
                        // callback `get_all` saw a plain `uvm_callback` with
                        // no callbacks. A TYPEDEF receiver (`T::task`, T a
                        // typedef alias of a parameterized class) has no
                        // `Specialization` receiver to seed `current_spec` from,
                        // so `tf_spec` (resolved when the typedef was followed to
                        // its class above) carries the same specialization here.
                        let mut recv_spec: Option<(String, String)> = tf_spec;
                        if recv_spec.is_none() {
                            if let ExprKind::MemberAccess { expr: mrecv, .. } = &func.kind {
                                if let ExprKind::Specialization { .. } = &mrecv.kind {
                                    recv_spec = self.resolve_call_spec_params(
                                        Self::extract_call_spec(&func.clone()),
                                        &self.current_spec,
                                    );
                                }
                            }
                        }
                        if let Some((tm, mclass)) = self.resolve_class_task(&cls, &mn) {
                            let crate::ast::decl::ClassMethodKind::Task(td) = &tm.kind else {
                                unreachable!("resolve_class_task yields tasks only")
                            };
                            if self.stmts_have_blocking(&td.items) {
                                let spec = recv_spec.clone().or_else(|| self.current_spec.clone());
                                let mut cleanup = self.bind_task_frame(
                                    td,
                                    args,
                                    Some((spec, None, mclass.clone())),
                                );
                                // Static context: push the declaring class for
                                // member/static resolution, with a null `this`.
                                self.m_scope_stack.clear();
                                self.this_stack.push(None);
                                self.class_context_stack.push(Some(mclass));
                                self.method_local_base
                                    .push(self.local_stack.len().saturating_sub(1));
                                cleanup.pushed_method_this = true;
                                let saved_spec = self.current_spec.clone();
                                // The frame's unwind restores the caller's
                                // specialization (§8.25).
                                cleanup.saved_spec = saved_spec.clone();
                                if let Some((sb, ss)) = recv_spec {
                                    self.ensure_spec_statics(&sb, &ss);
                                    self.current_spec = Some((sb, ss));
                                }
                                self.refresh_bound_task_string_kinds(&td.ports);
                                self.task_cleanup.push(cleanup);
                                let frame = self.class_task_body_frame(&tm, stmt.span);
                                // Chain the caller's tail instead of copying it onto the end of
                                // the spliced body (ProcCont::pushed_frame).
                                self.run_process_stmts(
                                    pid,
                                    &pc.pushed_frame(frame, pc.start + i + 1),
                                );
                                // A parked continuation is still executing
                                // inside its innermost task frame. Snapshot
                                // that binding; ScopePop restores the caller.
                                if !self.is_pid_suspended(pid) {
                                    self.current_spec = saved_spec;
                                }
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
                    // §25.9: an interface task may also be called `get` or
                    // `put`. Probing its getter as a mailbox receiver would
                    // run that function before the actual task dispatch.
                    let vif_receiver = match recv_expr_opt.as_ref().map(|e| &e.kind) {
                        Some(ExprKind::Call { func, .. }) => self
                            .call_return_type(func)
                            .is_some_and(|dt| self.is_virtual_iface_type(&dt)),
                        _ => false,
                    };
                    if !vif_receiver && (method == "get" || method == "peek") && !args.is_empty() {
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
                                self.abort_run();
                                return;
                            }
                            let mbx_empty = self
                                .mailboxes
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
                            if let Some(q) = self.mailboxes.get(&handle) {
                                if q.is_empty() {
                                    // Blocking get/peek on an empty mailbox: park
                                    // until a put hands over a value. peek leaves
                                    // the item in the box (`m_req_fifo.peek` in
                                    // `get_next_item`, then `item_done`
                                    // try_gets it).
                                    let lvalue = args[0].clone();
                                    let cont = pc.resume_at(pc.start + i + 1);
                                    self.mailbox_get_waiters
                                        .entry(handle)
                                        .or_default()
                                        .push_back(MailboxGetWaiter {
                                            pid,
                                            lvalue,
                                            cont,
                                            is_peek: method == "peek",
                                        });
                                    if env_set_cached!("XEZIM_MBX_DBG") {
                                        eprintln!(
                                            "[MBX] t={} pid={} PARKED {} handle={}",
                                            self.time, pid, method, handle
                                        );
                                    }
                                    return;
                                }
                            }
                        }
                    }
                    // §15.4.1: blocking `mailbox.put(v)` on a FULL bounded
                    // mailbox. Capture the value now, park the producer, and
                    // return; a later get/try_get admits it and resumes here.
                    if !vif_receiver && method == "put" && !args.is_empty() {
                        if let Some(recv) = recv_expr_opt.clone() {
                            let recv_val = self.eval_expr(&recv);
                            let handle = recv_val.to_u64().unwrap_or(0) as usize;
                            let bound = self.mailbox_bound.get(&handle).copied().unwrap_or(0);
                            if bound > 0 {
                                let len = self.mailboxes.get(&handle).map(|q| q.len()).unwrap_or(0);
                                if len >= bound {
                                    let value = self.mbx_message(&args[0]);
                                    let cont = pc.resume_at(pc.start + i + 1);
                                    self.mailbox_put_waiters
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
                    if !vif_receiver && method == "get" {
                        if let Some(recv) = recv_expr_opt {
                            let recv_val = self.eval_expr(&recv);
                            let handle = recv_val.to_u64().unwrap_or(0) as usize;
                            if let Some(&count) = self.semaphores.get(&handle) {
                                let n = args
                                    .first()
                                    .map(|a| self.eval_expr(a).to_u64().unwrap_or(1))
                                    .unwrap_or(1) as i64;
                                if count < n {
                                    let cont = pc.resume_at(pc.start + i + 1);
                                    self.semaphore_get_waiters
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
                        let (saved, release) =
                            self.capture_intra_rhs(lvalue, &args[3], rvalue.span);
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
                                    stmt: Box::new(Statement::new(StatementKind::Null, stmt.span)),
                                },
                                stmt.span,
                            ));
                        }
                        cont.push(assign);
                        cont.extend(release);
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
                    let (saved, release) = self.capture_intra_rhs(lvalue, rhs, rvalue.span);
                    let delay = self.eval_delay_ticks(d_expr);
                    let mut cont = vec![Statement::new(
                        StatementKind::BlockingAssign {
                            lvalue: lvalue.clone(),
                            rvalue: saved,
                        },
                        stmt.span,
                    )];
                    cont.extend(release);
                    // Chain the caller's tail rather than copying it (ProcCont::pushed).
                    let cont = pc.pushed(cont, pc.start + i + 1);
                    self.delay_pids.insert(pid, delay > 0);
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
                    if sim.event_triggered_time.get(canon.as_str()) == Some(&now) {
                        return true;
                    }
                    let pref = format!("{}.{}", sim.module.name, canon);
                    sim.event_triggered_time.get(pref.as_str()) == Some(&now)
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
                                        cached_signal_id: crate::ast::cache::SignalCache::new(None),
                                        cached_resolved_name: crate::ast::cache::NameCache::new(),
                                        bound: crate::ast::binding::Bound::UNSET,
                                    }),
                                    *span,
                                ),
                                iff: None,
                                span: *span,
                            })
                            .collect();
                        let sens =
                            self.event_to_sens(&crate::ast::stmt::EventControl::EventExpr(evs));
                        let cont = vec![Statement::new(
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
                            {
                                let w = self.make_event_waiter(pid, sens, cont);
                                self.park_event_waiter(w);
                            }
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
                            let whole = vec![Statement::new(
                                StatementKind::TimingControl {
                                    control: TimingControl::Delay(d.clone()),
                                    stmt: body.clone(),
                                },
                                stmt.span,
                            )];
                            // Chain the caller's tail rather than copying it (ProcCont::pushed).
                            let whole = pc.pushed(whole, pc.start + i + 1);
                            self.inactive_queue.push((pid, whole));
                            return;
                        }
                        // Chain the caller's tail rather than copying it (ProcCont::pushed_body).
                        let cont = pc.pushed_body(body, pc.start + i + 1);
                        self.delay_pids.insert(pid, delay > 0);
                        if delay == 0 {
                            // `#0` — IEEE 1800-2017 §4.4.2.3: suspend into the
                            // Inactive region of the SAME time slot, not the
                            // Active region. Scheduling into event_queue at
                            // self.time would resume it in the same batch
                            // drain, BEFORE apply_nba — so an NBA posted
                            // before the `#0` would not be visible after it.
                            // Commercial consensus: it IS
                            // visible. Park here; run_one_tick promotes after
                            // the NBA region of this tick has been applied.
                            self.inactive_queue.push((pid, cont));
                        } else {
                            self.event_queue.schedule(self.time + delay, pid, cont);
                        }
                        return;
                    }
                    TimingControl::Event(event) => {
                        // §9.4.2 / §15.5: an OR of class events parks on
                        // their identities, not signal sensitivities (class
                        // events have no signal-table slot). One waiter owns
                        // the whole group and is removed after its first wake.
                        if let EventControl::EventExpr(terms) = event
                            && terms.len() > 1
                            && terms.iter().all(|t| {
                                t.edge.is_none()
                                    && t.iff.is_none()
                                    && Self::class_scope_chain(&t.expr).is_some()
                            })
                        {
                            let keys: Option<Vec<_>> = terms
                                .iter()
                                .map(|term| {
                                    self.expr_instance_event_field_general(&term.expr)
                                        .or_else(|| {
                                            Self::plain_ident_name(&term.expr)
                                                .and_then(|n| self.resolve_this_event_field(&n))
                                        })
                                        .or_else(|| self.static_class_event_key(&term.expr))
                                })
                                .collect();
                            if let Some(keys) = keys {
                                self.instance_event_waiters.push(InstanceEventWaiter {
                                    key: InstanceEventKeys::Any(keys),
                                    pid,
                                    continuation: pc.pushed_body(body, pc.start + i + 1),
                                });
                                return;
                            }
                        }
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
                        // §15.5: `@(h.evs[i])` — an element of a class event
                        // array, keyed per element.
                        let elem_expr = match event {
                            EventControl::HierIdentifier(e) => Some(e),
                            EventControl::EventExpr(t) if t.len() == 1 && t[0].edge.is_none() => {
                                Some(&t[0].expr)
                            }
                            _ => None,
                        };
                        if let Some(e) = elem_expr
                            && let Some(key) = self.expr_instance_event_elem(e)
                        {
                            let cont = pc.pushed_body(body, pc.start + i + 1);
                            self.instance_event_waiters.push(InstanceEventWaiter {
                                key: InstanceEventKeys::One(key),
                                pid,
                                continuation: cont,
                            });
                            return;
                        }
                        if let Some(fname) = self.event_control_field_name(event) {
                            if let Some(key) = self.resolve_this_event_field(&fname) {
                                // Chain the caller's tail rather than copying it (ProcCont::pushed_body).
                                let cont = pc.pushed_body(body, pc.start + i + 1);
                                self.instance_event_waiters.push(InstanceEventWaiter {
                                    key: InstanceEventKeys::One(key),
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
                            if let Some(key) = self.expr_instance_prop_field_general(&expr) {
                                // §9.4.2: a selected or nested ordinary
                                // property waits for a value change, too.
                                if !self.class_prop_is_event(key.0, &key.1) {
                                    let cond = self.value_change_cond(expr);
                                    let wait = Self::vc_wait_stmt(cond.clone(), body, stmt.span);
                                    let cont = pc.pushed(vec![wait], pc.start + i + 1);
                                    self.park_condition_waiter(pid, cont, &cond);
                                    return;
                                }
                                // Chain the caller's tail rather than copying it.
                                let cont = pc.pushed_body(body, pc.start + i + 1);
                                self.instance_event_waiters.push(InstanceEventWaiter {
                                    key: InstanceEventKeys::One(key),
                                    pid,
                                    continuation: cont,
                                });
                                return;
                            }
                        }
                        // §15.5: `@ev` / `@(C::ev)` on a STATIC class event.
                        let static_ev = if self.class_member_names().static_or_param.is_empty() {
                            None
                        } else {
                            match event {
                                EventControl::Identifier(id) => self
                                    .class_context_stack
                                    .last()
                                    .cloned()
                                    .flatten()
                                    .filter(|_| self.resolve_this_event_field(&id.name).is_none())
                                    .and_then(|c| self.static_class_event_key_named(&c, &id.name)),
                                _ => {
                                    // Only a name some class declares static
                                    // can be one; look before copying the term.
                                    let leaf = match event {
                                        EventControl::HierIdentifier(e) => Some(e),
                                        EventControl::EventExpr(t)
                                            if t.len() == 1 && t[0].edge.is_none() =>
                                        {
                                            Some(&t[0].expr)
                                        }
                                        _ => None,
                                    }
                                    .and_then(|e| {
                                        match &e.kind {
                                            ExprKind::Ident(h) => {
                                                h.path.last().map(|s| s.name.name.as_str())
                                            }
                                            ExprKind::MemberAccess { member, .. } => {
                                                Some(member.name.as_str())
                                            }
                                            _ => None,
                                        }
                                    });
                                    if leaf.is_some_and(|n| {
                                        self.class_member_names().static_or_param.contains(n)
                                    }) {
                                        Self::event_control_single_expr(event)
                                            .and_then(|e| self.static_class_event_key(&e))
                                    } else {
                                        None
                                    }
                                }
                            }
                        };
                        if let Some(key) = static_ev {
                            let cont = pc.pushed_body(body, pc.start + i + 1);
                            self.instance_event_waiters.push(InstanceEventWaiter {
                                key: InstanceEventKeys::One(key),
                                pid,
                                continuation: cont,
                            });
                            return;
                        }
                        // `@prop` / `@(h.prop)` on a NON-event class property:
                        // a value-change wait, resumed by an ordinary write
                        // of the property (never by `->`).
                        // `@(vif)` / `@(vifs[i])`: a change of the instance a
                        // virtual-interface variable refers to.
                        if let Some(cond) = self.vif_value_change_cond(event) {
                            let wait = Self::vc_wait_stmt(cond.clone(), body, stmt.span);
                            let cont = pc.pushed(vec![wait], pc.start + i + 1);
                            self.park_condition_waiter(pid, cont, &cond);
                            return;
                        }
                        if let Some(cond) = self.class_prop_value_change_cond(event) {
                            let wait = Self::vc_wait_stmt(cond.clone(), body, stmt.span);
                            let cont = pc.pushed(vec![wait], pc.start + i + 1);
                            self.park_condition_waiter(pid, cont, &cond);
                            return;
                        }
                        // `@(posedge prop)` / `@(negedge prop)` on a non-signal
                        // operand (class property or procedural local): an
                        // edge-sensitive value change, not a signal edge.
                        if let Some(cond) = self.class_prop_edge_cond(event) {
                            let wait = Self::vc_wait_stmt(cond.clone(), body, stmt.span);
                            let cont = pc.pushed(vec![wait], pc.start + i + 1);
                            self.park_condition_waiter(pid, cont, &cond);
                            return;
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
                                // Chain the caller's tail rather than copying it.
                                let cont = pc.pushed_body(body, pc.start + i + 1);
                                let sens = vec![Sensitivity {
                                    signal_name: canon,
                                    edge: EdgeKind::AnyEdge,
                                    iff: None,
                                    value_of: None,
                                }];
                                {
                                    let w = self.make_event_waiter_kind(pid, sens, cont, false);
                                    self.park_event_waiter(w);
                                }
                                return;
                            }
                            // §15.5: an event with no signal — a subroutine's
                            // local `event e;` — waits for `fire_named_event`
                            // by name. The signal fallback below resumed it
                            // after the slot's NBA region, i.e. at once. So
                            // does a labeled block's local stored under its
                            // hierarchical name (`a.e`, §6.21).
                            if (!canon.contains('.') || self.module.events.contains(canon.as_str()))
                                && canon != Self::EVENT_NULL_KEY
                            {
                                let cont = pc.pushed_body(body, pc.start + i + 1);
                                self.instance_event_waiters.push(InstanceEventWaiter {
                                    key: InstanceEventKeys::One(self.unbacked_event_key(&canon)),
                                    pid,
                                    continuation: cont,
                                });
                                return;
                            }
                        }
                        let is_star = matches!(event, EventControl::Star | EventControl::ParenStar);
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
                            // Chain the caller's tail rather than copying it (ProcCont::pushed_body).
                            let cont = pc.pushed_body(body, pc.start + i + 1);
                            // Late-bind virtual-interface / handle-chained
                            // names to their concrete instance signals before
                            // deciding whether anything real is waited on.
                            let mut sens = sens;
                            for se in sens.iter_mut() {
                                if !self.signal_name_to_id.contains_key(se.signal_name.as_str()) {
                                    if let Some(resolved) =
                                        self.resolve_sens_name_via_handles(&se.signal_name)
                                    {
                                        se.signal_name = resolved;
                                    }
                                }
                            }
                            let has_real = sens.iter().any(|s| self.sens_term_resolves(s));
                            if has_real && is_clk_ev && self.clocking_event_due_now(event) {
                                if sens.iter().any(|s| s.iff.is_some()) {
                                    // §14.13: judge the `iff` after sampling.
                                    let w = self.make_event_waiter_kind(pid, sens, cont, true);
                                    self.park_clocking_guard(w);
                                } else {
                                    self.deferred_clocking_conts.push((pid, cont));
                                }
                            } else if has_real {
                                let w = self.make_event_waiter_kind(pid, sens, cont, is_clk_ev);
                                self.park_event_waiter(w);
                            } else {
                                // `@(x)` where x is not a real signal — a
                                // procedural local that was NBA-assigned then
                                // waited on (uvm_wait_for_nba_region:
                                // `nba <= next_nba; @(nba)`). Its event_waiter
                                // would resolve to no signal_id and park forever;
                                // park in nba_region_waiters so it resumes only
                                // after the active + inactive (#0) deltas have
                                // settled and the NBA region has committed.
                                self.nba_region_waiters.push((pid, cont));
                            }
                            return;
                        }
                        // Star/empty sensitivity — just execute body
                        if !is_star && env_set_cached!("XEZIM_TRACE_SPIN") {
                            eprintln!(
                                "[EMPTYSENS] pid={} t={} event={:?} span={:?}",
                                pid, self.time, event, stmt.span
                            );
                        }
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
                if self.wait_condition_true(condition) {
                    self.vc_release(condition);
                    self.cond_progress = self.cond_progress.wrapping_add(1);
                    // §9.7.4: in `wait (c) @(posedge clk);` the statement
                    // the condition guards is an event control. Run through
                    // the synchronous `exec_statement` it could not park the
                    // process, which was never resumed. It runs here as the
                    // next statement, ahead of the rest.
                    if self.stmt_is_blocking(body) {
                        let cont = pc.pushed_body(body, pc.start + i + 1);
                        self.continue_stmts_or_trampoline(pid, cont);
                        return;
                    }
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
                    // `stmt` is already `pc.stmts[pc.start + i]`, so resuming
                    // `pc` at `pc.start + i` re-runs `stmt` followed by the
                    // tail without cloning `stmt` or allocating a wrapper frame.
                    let cont = pc.resume_at(pc.start + i);
                    self.park_condition_waiter(pid, cont, condition);
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
                let reentry = pc.loop_tail_at == Some(pc.start + i)
                    && pc.repeat_remaining.is_some();
                let n = if reentry {
                    pc.repeat_remaining.unwrap()
                } else {
                    self.repeat_count(count)
                };
                // Mirror the While/For arms: unroll not only when the body has
                // a direct `@event`/`#delay`, but also when it blocks via a
                // CALL to a task whose own body contains a `wait`/`#delay`/
                // `@event` (a loop like `repeat(N) obj.do_step();` where
                // do_step() blocks). Otherwise the repeat falls through to the
                // SYNCHRONOUS exec_statement, the body's calls are never
                // inlined (Stage 1b), and their nested `wait`s fall through
                // (IEEE 1800-2023 §9.7.4: a false `wait(cond)` must suspend)
                // instead of parking the process, busy-spinning the loop.
                if reentry || self.stmt_has_event_wait(body) || self.stmt_is_blocking(body) {
                    // n == 0 (initial count zero, or the natural-exhaustion
                    // sentinel re-entering after the final iteration's body):
                    // clear any loop-control flag left by that body — a
                    // trailing `continue` must not leak past the loop and
                    // suppress the statements after it. (§9.3.3)
                    if n == 0 {
                        self.break_flag = false;
                        self.continue_flag = false;
                        // §14.11: a RUNTIME `##(expr)` that evaluates to 0 —
                        // the desugared `repeat (0) @(__xz_default_clocking)`
                        // — SYNCHRONIZES to the clocking event like the
                        // literal ##0 (no-op when already at the edge, park
                        // otherwise). A plain zero repeat of any other body
                        // keeps falling straight through.
                        if let StatementKind::TimingControl {
                            control: TimingControl::Event(EventControl::Identifier(id)),
                            ..
                        } = &body.kind
                        {
                            if id.name == "__xz_default_clocking" {
                                let at_edge = self
                                    .default_clocking_cb
                                    .as_ref()
                                    .and_then(|cb| self.clocking_last_edge.get(cb))
                                    == Some(&self.time);
                                if !at_edge {
                                    let sync = Statement::new(
                                        StatementKind::TimingControl {
                                            control: TimingControl::Event(
                                                EventControl::Identifier(crate::ast::Identifier {
                                                    name: "__xz_default_clocking0".to_string(),
                                                    span: stmt.span,
                                                }),
                                            ),
                                            stmt: Box::new(Statement::new(
                                                StatementKind::Null,
                                                stmt.span,
                                            )),
                                        },
                                        stmt.span,
                                    );
                                    let cont = pc.pushed(vec![sync], pc.start + i + 1);
                                    self.event_queue.schedule(self.time, pid, cont);
                                    return;
                                }
                            }
                        }
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
                            let has_real = sens.iter().any(|s| self.sens_term_resolves(s));
                            if !sens.is_empty() && has_real {
                                let mut waiter = self.make_event_waiter(
                                    pid,
                                    sens,
                                    pc.resume_at(pc.start + i + 1),
                                );
                                waiter.remaining_events = n;
                                self.park_event_waiter(waiter);
                                return;
                            }
                        }
                    }
                    // Keep one immutable body/tail frame per loop activation.
                    // Re-cloning the body on each resume gives its statements
                    // fresh addresses, so address-keyed execution caches retain
                    // every iteration's AST. Only the remaining count changes.
                    let mut cont = if reentry {
                        pc.resume_at(0)
                    } else {
                        let mut frame = Self::spliced_body(body);
                        frame.push(stmt.clone());
                        let tail_idx = frame.len() - 1;
                        pc.pushed_with_tail(frame, pc.start + i + 1, tail_idx)
                    };
                    // Zero is an exhaustion sentinel: it still clears a final
                    // continue/break before the caller's continuation executes.
                    cont.repeat_remaining = Some(n - 1);
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
                    // The compiled-condition cache preserves the once-only
                    // contract: the bytecode runs once per activation.
                    let cond_true = self
                        .try_eval_process_cond_bytecode(pid, condition)
                        .unwrap_or_else(|| self.eval_expr(condition).is_true());
                    let chosen: Option<&Statement> = if cond_true {
                        Some(then_stmt.as_ref())
                    } else {
                        else_stmt.as_ref().map(|b| b.as_ref())
                    };
                    match chosen {
                        Some(branch) if self.stmt_is_blocking(branch) => {
                            // The chosen branch is immutable AST: share one
                            // Arc frame per (process, span) instead of cloning
                            // its statements on every activation (a clocked
                            // monitor whose else-chain ends in a rare `#delay`
                            // did this every cycle — 3% of c906).
                            let frame = match &branch.kind {
                                StatementKind::SeqBlock { stmts, .. }
                                    if !Self::stmts_declare_vars(stmts) =>
                                {
                                    self.blocking_cont_frame(&pc.stmts, branch, 1, stmts)
                                }
                                _ => self.blocking_cont_frame(
                                    &pc.stmts,
                                    branch,
                                    4,
                                    std::slice::from_ref(branch),
                                ),
                            };
                            self.run_process_stmts(pid, &pc.pushed_frame(frame, pc.start + i + 1));
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
                    let comparison = self.case_comparison(kind, expr, items);
                    let chosen: Option<&Statement> = items
                        .iter()
                        .find_map(|item| {
                            if item.is_default {
                                return None;
                            }
                            for pat in &item.patterns {
                                if self.case_item_matches(kind, &comparison, pat) {
                                    return Some(&item.stmt);
                                }
                            }
                            None
                        })
                        .or_else(|| {
                            items
                                .iter()
                                .find(|item| item.is_default)
                                .map(|item| &item.stmt)
                        });
                    match chosen {
                        Some(arm) if self.stmt_is_blocking(arm) => {
                            let frame = self.spliced_body_frame(&pc.stmts, arm);
                            // Chain the caller's tail instead of copying it onto the end of
                            // the spliced body (ProcCont::pushed_frame).
                            self.run_process_stmts(pid, &pc.pushed_frame(frame, pc.start + i + 1));
                            return;
                        }
                        Some(arm) => self.exec_statement(arm),
                        None => {}
                    }
                    i += 1;
                    continue;
                }
            }

            // Suspend-aware randcase (§18.16), as for `case` above: a chosen
            // branch that blocks (`1: seq.start(sqr);`) must run on this path,
            // or its first wait returned at once and the loop around it
            // started the same sequence again while it was still running.
            if let StatementKind::RandCase { items } = &stmt.kind {
                if self.stmt_is_blocking(stmt) {
                    if let Some(k) = self.randcase_pick(items) {
                        let arm = &items[k].1;
                        if self.stmt_is_blocking(arm) {
                            let frame = self.spliced_body_frame(&pc.stmts, arm);
                            self.run_process_stmts(pid, &pc.pushed_frame(frame, pc.start + i + 1));
                            return;
                        }
                        self.exec_statement(arm);
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
                    // §12.7.1: retain the enclosing declaration, and close the
                    // automatic loop scope after the final resumed iteration.
                    let names: Vec<String> = init
                        .iter()
                        .filter_map(|fi| match fi {
                            ForInit::VarDecl { name, .. } => Some(name.name.clone()),
                            _ => None,
                        })
                        .collect();
                    let saved = self.snapshot_loop_vars(&names);
                    let owns_loop_frame = self.local_stack.is_empty() && !names.is_empty();
                    if owns_loop_frame {
                        self.push_loop_frame();
                    }
                    // Shown again when the loop's scope closes (`block_scope_exit`).
                    let mut static_hidden = None;
                    for fi in init {
                        match fi {
                            ForInit::VarDecl {
                                data_type,
                                name,
                                init: e,
                            } => {
                                let rv = self.declare_for_init_var(data_type, &name.name, e);
                                if let Some(frame) = self.local_stack.last_mut() {
                                    frame.insert(name.name.clone(), rv);
                                    self.hide_loop_static(&name.name, &mut static_hidden);
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
                                    self.push_local_frame(f);
                                    // Not a call frame: the process's block
                                    // locals stay static (§6.21).
                                    if let Some(&g) = self.local_gen_stack.last() {
                                        self.loop_frame_gens.insert(g);
                                    }
                                }
                            }
                            ForInit::Assign { lvalue, rvalue } => {
                                let v = self.eval_expr(rvalue);
                                self.assign_value(lvalue, &v);
                            }
                        }
                    }
                    let for_key = (stmt as *const Statement as usize, 6u8);
                    let frame = if let Some((_, f)) = self.blocking_cont_cache.get(&for_key) {
                        f.clone()
                    } else {
                        let mut body_stmts = Self::spliced_body(body);
                        // §12.7.2: a `continue` skips the rest of the body but the
                        // step STILL runs. Without this barrier the step — appended
                        // to the body by this very lowering — was skipped along
                        // with it, so the index never advanced and the loop hung.
                        if !step.is_empty() {
                            body_stmts.push(Statement::new(StatementKind::LoopStep, stmt.span));
                        }
                        for s in step {
                            body_stmts
                                .push(Statement::new(StatementKind::Expr(s.clone()), stmt.span));
                        }
                        let while_body = Statement::new(
                            StatementKind::SeqBlock {
                                name: None,
                                stmts: body_stmts.into(),
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
                        let f: Arc<[Statement]> = Arc::from(vec![
                            while_stmt,
                            Statement::new(StatementKind::Null, stmt.span),
                        ]);
                        self.blocking_cont_cache
                            .insert(for_key, (Arc::clone(&pc.stmts), f.clone()));
                        f
                    };
                    if !saved.is_empty() || static_hidden.is_some() {
                        self.block_scope_saves.push(BlockScopeSave {
                            frame_key: Self::frame_key(&frame),
                            frame_gen: self.local_gen_stack.last().copied().unwrap_or(0),
                            loop_vars: saved,
                            owns_loop_frame,
                            static_hidden,
                            ..Default::default()
                        });
                    }
                    // Chain the caller's tail rather than copying it (ProcCont::pushed_frame).
                    let cont = pc.pushed_frame(frame, pc.start + i + 1);
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
                        let while_key = (stmt as *const Statement as usize, 7u8);
                        let frame = if let Some((_, f)) = self.blocking_cont_cache.get(&while_key) {
                            f.clone()
                        } else {
                            let mut prepared = Self::spliced_body(body);
                            prepared.push(stmt.clone());
                            let frame: Arc<[Statement]> = Arc::from(prepared);
                            let tail_ptr = &frame[frame.len() - 1] as *const Statement as usize;
                            self.blocking_cont_cache
                                .insert(while_key, (Arc::clone(&pc.stmts), Arc::clone(&frame)));
                            self.blocking_cont_cache
                                .insert((tail_ptr, 7u8), (Arc::clone(&frame), Arc::clone(&frame)));
                            frame
                        };
                        // `frame` always ends with the re-appended `stmt.clone()`
                        // (freshly built above, or the identical shape pulled
                        // from `blocking_cont_cache` — same span, same tail
                        // position either way): that last slot is this frame's
                        // genuine re-entry (see `loop_tail_at`'s doc comment).
                        let tail_idx = frame.len() - 1;
                        let cont = pc.pushed_frame_with_tail(frame, pc.start + i + 1, tail_idx);
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
                    let body_stmts = Self::spliced_body(body);
                    let mut cont: Vec<Statement> = body_stmts;
                    cont.push(Statement::new(
                        StatementKind::While {
                            condition: condition.clone(),
                            body: body.clone(),
                        },
                        stmt.span,
                    ));
                    // Chain the caller's tail instead of copying it onto the end of
                    // the spliced body (ProcCont::pushed). The appended `While`
                    // is this frame's genuine re-entry tail.
                    let tail_idx = cont.len() - 1;
                    self.run_process_stmts(
                        pid,
                        &pc.pushed_with_tail(cont, pc.start + i + 1, tail_idx),
                    );
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
                // §9.3.2 children start in THIS time slot, ahead of whatever
                // is already queued for it; collected here and pushed to the
                // slot front in reverse below so siblings keep source order.
                let mut start_now: Vec<(usize, Arc<[Statement]>)> = Vec::new();
                for s in spawnable {
                    let pid_child = self.next_pid;
                    self.next_pid += 1;
                    self.process_parents.insert(pid_child, pid);
                    self.note_fork_spawn(pid_child, pc);
                    // §9.3.2: a fork child executes in the forking process's
                    // scope, so it inherits the parent's instance-scope hint
                    // (additive — a child previously had none at all).
                    if let Some(h) = self.process_scope_hint.get(&pid).cloned() {
                        self.process_scope_hint.insert(pid_child, h);
                    }
                    self.process_origin
                        .insert(pid_child, (s.span, FORK_CHILD_ORIGIN));
                    // §9.6.2: `disable <label>` where the label names this
                    // child's own top-level `begin : name` block terminates
                    // the child. `disable_labels` was populated ONLY for
                    // initial blocks at start-up, so a fork child's label was
                    // unknown: the disable found no target, fell through to
                    // the self-unwind path, and the child kept running — a
                    // later `wait fork` then blocked on it forever.
                    if let StatementKind::SeqBlock { name: Some(n), .. } = &s.kind {
                        self.register_disable_label(&n.name, pid_child);
                    }
                    self.inherit_fork_child_context(pid_child);
                    self.inherit_fork_m_scope(pid_child, pid, block_name.as_ref());
                    // §9.3.2, §9.4.5: a child that is an intra-assignment
                    // delay (`fork lhs = #d rhs; join_none`) evaluates its
                    // right-hand side when it starts, like any other child:
                    // after the parent blocks or finishes, so it reads what
                    // the parent wrote after the fork.
                    // Schedule children to run at current time, sharing one
                    // cached Arc<[Statement]> per static fork branch node.
                    let frame = self.blocking_cont_frame(&pc.stmts, s, 5, std::slice::from_ref(s));
                    start_now.push((pid_child, frame));
                    child_pids.insert(pid_child);
                }
                for (pid_child, frame) in start_now.into_iter().rev() {
                    self.event_queue.schedule_front(
                        self.time,
                        pid_child,
                        ProcCont::from_frame(frame),
                    );
                }
                self.auto_loop_vars.truncate(saved_auto_len);
                if let Some(nm) = block_name {
                    self.register_named_fork(&nm.name, &child_pids);
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
                    self.join_waiters.push(JoinWaiter {
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
                        // §12.7.3: the iteration set is fixed when the loop
                        // starts. The live size is re-read ONLY so a collection
                        // shrunk by another process (while the body was
                        // suspended) exits early instead of indexing past the
                        // end — growth must not extend the loop. A `push_back`
                        // in the body grew the bound exactly as fast as `idx`
                        // advanced, so the loop never exhausted: it ran to max
                        // simulation time with no diagnostic at all. Clamping to
                        // the entry count makes ForeachTail provably terminate.
                        *idx as u64 >= self.get_queue_size(ln).min(keys.len() as u64)
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
                            Self::name_key_value(&key_str)
                        } else if key_type.1 {
                            let mut v = Value::from_u64(
                                key_str.parse::<i64>().unwrap_or(0) as u64,
                                key_type.0,
                            );
                            v.is_signed = true;
                            v
                        } else {
                            // §7.9 + §7.8.2: parse SIGNED (a negative key
                            // must not collapse to 0), then land at the
                            // DECLARED index width/signedness when recorded.
                            let kw = if key_type.0 > 0 { key_type.0 } else { 32 };
                            let mut v =
                                Value::from_u64(key_str.parse::<i64>().unwrap_or(0) as u64, kw);
                            v.is_signed = if key_type.0 > 0 { key_type.1 } else { true };
                            v
                        };
                        self.set_loop_var_aliased(var_scope.as_deref(), vn, kv);
                    }
                    let body_id = body.span.start;
                    let body_frame = self
                        .blocking_cont_cache
                        .get(&(body_id, 8u8))
                        .map(|(_, f)| f.clone())
                        .unwrap_or_else(|| self.spliced_body_frame(&pc.stmts, body));
                    let tail = pc.pushed(
                        vec![Statement::new(
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
                        )],
                        pc.start + i + 1,
                    );
                    let cont = tail.pushed_frame(body_frame, 0);
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
                            // §12.7.3 (blocking-body trampoline): this path
                            // bypasses `exec_stmt_foreach`, so the loop
                            // variable's class binding (assoc KEY class, then
                            // element class) must be recorded HERE — a
                            // `foreach (m_successors[succ])` body writing
                            // `succ.m_state = …` dropped every write without
                            // it, because the id-collision receiver gate
                            // could not classify the bare name. Recorded into
                            // the task's live frame, the binding rides the
                            // process-context snapshot through every
                            // park/restore cycle of the ForeachTail unrolling.
                            if let Some(arr_name) = Self::foreach_array_root_name(array) {
                                if let Some(cls) = self
                                    .assoc_key_class_of_foreach(&arr_name, array)
                                    .or_else(|| self.elem_class_of_arr(&arr_name))
                                {
                                    if self.module.classes.contains_key(&cls)
                                        && let Some(v) = vars.first().and_then(|v| v.as_ref())
                                    {
                                        self.record_local_class_type(&v.name, &cls);
                                    }
                                }
                            }
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
                            let loop_var = vars
                                .first()
                                .and_then(|v| v.as_ref().map(|id| id.name.clone()));
                            if let Some(vn) = &loop_var {
                                self.widths.insert(vn.clone(), key_type.0);
                            }
                            if keys.is_empty() {
                                self.auto_loop_vars.truncate(fe_auto_len);
                                i += 1;
                                continue;
                            }
                            // §6.21, §12.7.3: the loop variable hides the
                            // subroutine's same-named `static` local until the
                            // loop's scope closes (`block_scope_exit`).
                            let mut static_hidden = None;
                            if !self.static_local_names.is_empty() {
                                for nm in &fe_names {
                                    self.hide_loop_static(nm, &mut static_hidden);
                                }
                            }
                            // run the FIRST iteration now
                            if let Some(vn) = &loop_var {
                                let kv = if is_str {
                                    Self::name_key_value(&keys[0])
                                } else if key_type.1 {
                                    let mut v = Value::from_u64(
                                        keys[0].parse::<i64>().unwrap_or(0) as u64,
                                        key_type.0,
                                    );
                                    v.is_signed = true;
                                    v
                                } else {
                                    // §7.9 + §7.8.2: signed parse, declared width.
                                    let kw = if key_type.0 > 0 { key_type.0 } else { 32 };
                                    let mut v = Value::from_u64(
                                        keys[0].parse::<i64>().unwrap_or(0) as u64,
                                        kw,
                                    );
                                    v.is_signed = if key_type.0 > 0 { key_type.1 } else { true };
                                    v
                                };
                                self.set_loop_var_aliased(var_scope.as_deref(), vn, kv);
                            }
                            self.continue_flag = false;
                            let body_frame = self.spliced_body_frame(&pc.stmts, body);
                            let body_id = (&**body) as *const Statement as usize;
                            let marker = Box::new(Statement::new(
                                StatementKind::Null,
                                crate::ast::Span {
                                    start: body_id,
                                    end: 0,
                                },
                            ));
                            let mut tail_stmts = vec![Statement::new(
                                StatementKind::ForeachTail {
                                    loop_var,
                                    var_scope,
                                    body: marker,
                                    keys,
                                    is_str,
                                    key_type,
                                    idx: 1,
                                    fe_auto_len,
                                    live_size_name,
                                },
                                stmt.span,
                            )];
                            // Each later iteration's tail resumes this frame
                            // after its first statement, so its trailing
                            // `Null` runs once, when the loop ends, and shows
                            // the hidden statics again (`block_scope_sentinel`).
                            if static_hidden.is_some() {
                                tail_stmts.push(Statement::new(StatementKind::Null, stmt.span));
                            }
                            let tail = pc.pushed(tail_stmts, pc.start + i + 1);
                            if static_hidden.is_some() {
                                self.block_scope_saves.push(BlockScopeSave {
                                    frame_key: Self::frame_key(&tail.stmts),
                                    frame_gen: self.local_gen_stack.last().copied().unwrap_or(0),
                                    static_hidden,
                                    ..Default::default()
                                });
                            }
                            let cont = tail.pushed_frame(body_frame, 0);
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
                if self.try_exec_process_loop_bytecode(pid, stmt) {
                    i += 1;
                    continue;
                }
                let is_foreach_fallback = matches!(&stmt.kind, StatementKind::Foreach { .. });
                if is_foreach_fallback {
                    self.exec_park_cont = Some({
                        let c = vec![stmt.clone()];
                        // Chain the caller's tail rather than copying it (ProcCont::pushed).
                        let c = pc.pushed(c, pc.start + i + 1);
                        c
                    });
                }
                self.exec_statement(stmt);
                // NO-PROGRESS GUARD. This fallback replays the foreach from
                // index 0 on every resume, so a body that parks here makes no
                // forward progress: it re-runs iteration 0, parks again, and
                // repeats — historically forever, with no diagnostic. One or
                // two parks may still complete legitimately (the wait is
                // satisfied on resume and the loop finishes synchronously), so
                // only a run of them is treated as stuck. Naming the array and
                // its source offset turns an unbounded hang into one line.
                if is_foreach_fallback {
                    if self.parked_from_exec {
                        let key = (pid, stmt.span.start);
                        let n = self.foreach_replay_parks.entry(key).or_insert(0);
                        *n += 1;
                        if self.foreach_replay_limit > 0 && *n > self.foreach_replay_limit {
                            let what = match &stmt.kind {
                                StatementKind::Foreach { array, .. } => match &array.kind {
                                    ExprKind::Ident(h) => h
                                        .path
                                        .iter()
                                        .map(|seg| seg.name.name.as_str())
                                        .collect::<Vec<_>>()
                                        .join("."),
                                    _ => "<expr>".to_string(),
                                },
                                _ => "<foreach>".to_string(),
                            };
                            eprintln!(
                                "Error: `foreach ({what}[...])` at offset {} made no progress \
                                 across {} suspensions.",
                                stmt.span.start, self.foreach_replay_limit
                            );
                            eprintln!(
                                "               Its index shape could not be materialized, so \
                                 each resume restarts the loop at index 0."
                            );
                            eprintln!(
                                "               Multi-variable `foreach` over a blocking body is \
                                 the usual cause. Raise or disable with \
                                 XEZIM_FOREACH_REPLAY_LIMIT=<n> (0 = never check)."
                            );
                            self.abort_run();
                        }
                    } else {
                        // Completed without parking — clear any earlier streak
                        // so a foreach inside an outer loop cannot accumulate.
                        self.foreach_replay_parks.remove(&(pid, stmt.span.start));
                    }
                }
                self.exec_park_cont = None;
                self.parked_from_exec = false;
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
                // use. A finished child's children, handed up to this process
                // for `disable fork`, are not its own (`wait_fork_children`).
                let children: HashSet<usize> = self.wait_fork_children(pid);

                if children.is_empty() {
                    i += 1;
                    continue;
                } else {
                    let cont = pc.resume_at(pc.start + i + 1);
                    self.join_waiters.push(JoinWaiter {
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
            // §21.2.1.7: leaving a flattened named block drops its `%m` name.
            if !self.named_block_frames.is_empty() {
                if let Some(n) = self.named_block_frames.get(&Self::frame_key(&pc.stmts)) {
                    if self.m_scope_stack.last() == Some(n) {
                        self.m_scope_stack.pop();
                    }
                }
            }
            // §9.6.2: a `disable` of THIS named block ends here.
            if self.break_flag
                && self.disable_target.is_some()
                && (self.disable_check_pending || !self.named_block_frames.is_empty())
                && self.disable_unwinding(pid, pc)
            {
                if let Some(n) = self.named_block_frames.get(&Self::frame_key(&pc.stmts)) {
                    if self
                        .disable_target
                        .as_deref()
                        .is_some_and(|t| Self::block_name_matches(t, n))
                        && !self.killed_pids.contains(&pid)
                    {
                        self.disable_target = None;
                        self.break_flag = false;
                        self.continue_flag = false;
                    }
                }
            }
            // §9.6.2: a `disable` of the process's own top-level block ends
            // with the process; leave no flag for the next activation.
            if pc.next.is_none()
                && self.break_flag
                && !self.killed_pids.contains(&pid)
                && self
                    .disable_target
                    .as_deref()
                    .is_some_and(|t| self.is_process_top_label(pid, t))
            {
                self.disable_target = None;
                self.break_flag = false;
                self.continue_flag = false;
            }
            if let Some(frame) = pc.next.clone() {
                self.run_process_stmts(pid, &frame);
            }
        }
    }
}
