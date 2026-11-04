//! IEEE 1800 §35.5.2, §35.9: imported DPI tasks that consume time.
//!
//! An imported task may call an exported task that waits, so the C frames of
//! the import have to survive while the simulation runs on. Each call of an
//! imported task from a process runs on its own stack (a fiber: a stackful
//! coroutine over an `mmap`ed stack). When SystemVerilog code reached from it waits,
//! the calling process parks like any other process — its continuation
//! starts with a resume marker — and the fiber switches back to the
//! scheduler. When the process wakes, the marker switches into the fiber,
//! which carries on from the wait. One context runs at a time, so the
//! simulator stays single-threaded.
//!
//! Before this, the wait ran the scheduler nested inside the C call, so a
//! second process entering an imported task stacked its C frames above the
//! first one's and the first could not return until the second had: two
//! callers unwound last-in first-out instead of each resuming on its own.
//!
//! State that belongs to whichever context is running (the current process,
//! the scope hints, the unwind flags, the active DPI scope, ...) is swapped
//! at every switch (`DpiAmbient`). The frames the exported task pushes are
//! the calling process's, and travel in its process context like those of
//! any parked process.
//!
//! Disable (§35.9): a process killed, or unwound out of a block by another
//! process's `disable`, while it is parked inside an imported task resumes
//! the task in the disabled state: the waiting exported task returns 1 at
//! once, `svIsDisabledState()` reports it, and the C code must return 1
//! without calling another export (both checked, as the clause requires).
//!
//! Only armed when an imported task is declared and a library is loaded
//! (`dpi_task_names` non-empty); otherwise nothing here runs. Where there
//! are no fibers (an architecture other than x86_64 or aarch64), or with
//! `XEZIM_DPI_FIBERS=0`, a wait inside an imported task runs the scheduler
//! nested in the C call instead (`run_nested_until`).

use super::*;

const RESUME_MARKER: &str = "$__xz_dpi_task_resume";
const UNWIND_MARKER: &str = "$__xz_dpi_task_unwind";

/// One running call of an imported task.
pub(super) struct DpiTask {
    fiber: fiber::Fiber,
    /// The process that made the call.
    owner: usize,
    name: String,
    args: Vec<Expression>,
    via: Option<String>,
    /// What the owner resumes with while the call is in progress: the
    /// resume marker, then the statements after the call.
    after: ProcCont,
    disabled: bool,
    /// The killed owner's process context, for the unwind.
    victim: Option<ProcessContext>,
    /// The exported subroutines running on the fiber, innermost last.
    exports: Vec<String>,
    /// An exported task on the fiber that another process disabled: it
    /// returns (0) to the C code when the fiber resumes.
    export_disable: Option<String>,
}

/// The state that belongs to the running context rather than to a process:
/// saved before a switch and put back after it, by each side for itself.
struct DpiAmbient {
    current_pid: usize,
    hint: Option<String>,
    activation: Option<String>,
    ts_override: Option<String>,
    export_depth: u32,
    in_edge_block: bool,
    break_flag: bool,
    continue_flag: bool,
    return_flag: bool,
    disable_target: Option<String>,
    disable_check_pending: bool,
    exec_park_cont: Option<ProcCont>,
    parked_from_exec: bool,
    m_scope_stack: Vec<String>,
    current_static_task: Option<String>,
    pkg_scope_stack: Vec<Option<String>>,
    decl_shadow_log: Vec<DeclShadowFrame>,
    auto_loop_vars: Vec<String>,
    cur_task: Option<u64>,
    unwinding: bool,
    rps_depth: usize,
    active_sim: *mut Simulator,
    active_scope: *mut libc::c_void,
}

/// Whether imported tasks run on their own stacks: where the platform has
/// fibers, unless `XEZIM_DPI_FIBERS=0` selects the nested-scheduler fallback
/// (the path other platforms take, so it can be tested anywhere).
pub(super) fn fibers_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        fiber::SUPPORTED && std::env::var("XEZIM_DPI_FIBERS").ok().as_deref() != Some("0")
    })
}

fn marker(name: &str, id: u64) -> Statement {
    let span = crate::ast::Span::dummy();
    let arg = Expression::new(
        ExprKind::Number(NumberLiteral::Integer {
            size: Some(64),
            signed: false,
            base: NumberBase::Decimal,
            value: id.to_string(),
            cached_val: Cell::new(Some((id, 0u64, 64u32))),
        }),
        span,
    );
    Statement::new(
        StatementKind::Expr(Expression::new(
            ExprKind::SystemCall {
                name: name.to_string(),
                args: vec![arg],
            },
            span,
        )),
        span,
    )
}

/// `Some((id, unwind))` when `stmt` is a resume or unwind marker.
fn marker_of(stmt: &Statement) -> Option<(u64, bool)> {
    let StatementKind::Expr(e) = &stmt.kind else {
        return None;
    };
    let ExprKind::SystemCall { name, args } = &e.kind else {
        return None;
    };
    let unwind = match name.as_str() {
        RESUME_MARKER => false,
        UNWIND_MARKER => true,
        _ => return None,
    };
    let Some(ExprKind::Number(NumberLiteral::Integer { value, .. })) =
        args.first().map(|a| &a.kind)
    else {
        return None;
    };
    Some((value.parse().ok()?, unwind))
}

impl Simulator {
    /// After the libraries are loaded: arm the mechanism when an imported
    /// task could be called.
    pub(super) fn dpi_tasks_setup(&mut self) {
        if !fibers_enabled() || self.dpi_libraries.is_empty() {
            return;
        }
        for (sv, spec) in &self.module.dpi_imports {
            if matches!(spec.proto, crate::ast::decl::DPIProto::Task(_)) {
                let leaf = sv.rsplit("::").next().unwrap_or(sv);
                self.dpi_task_names.insert(leaf.to_string());
                self.dpi_task_names.insert(sv.clone());
            }
        }
    }

    /// The callee of `e` when it calls an imported task: its name and the
    /// instance path it is called through (`u_a.c_task()`).
    pub(super) fn dpi_task_callee(&self, e: &Expression) -> Option<(String, Option<String>)> {
        // Resolve declarations even on the nested-scheduler path: normal SV
        // task inlining must not capture a package-qualified imported task.
        let imported = |name: &str| self.module.dpi_imports.get(name).is_some_and(|spec|
            matches!(spec.proto, crate::ast::decl::DPIProto::Task(_))
        );
        let func = match &e.kind {
            ExprKind::Call { func, .. } => &**func,
            ExprKind::Ident(_) => e,
            _ => return None,
        };
        if let ExprKind::MemberAccess { expr, member } = &func.kind
            && let ExprKind::Ident(h) = &expr.kind
            && h.path.len() == 1 && h.path[0].selects.is_empty()
            && self.module.packages.contains(&h.path[0].name.name)
            && !self.this_prop_hides(&h.path[0].name.name)
            && !self.local_stack.last().is_some_and(|m| m.contains_key(&h.path[0].name.name))
            && !self.signal_name_to_id.contains_key(&h.path[0].name.name)
        {
            let key = format!("{}::{}", h.path[0].name.name, member.name);
            return imported(&key).then_some((key, None));
        }
        let ExprKind::Ident(h) = &func.kind else {
            return None;
        };
        let last = h.path.last()?;
        if !last.selects.is_empty() {
            return None;
        }
        if h.path.len() == 1 {
            let name = self.resolve_dpi_call_name(&last.name.name, None)?.into_owned();
            return imported(&name).then_some((name, None));
        }
        if h.path.len() == 2 && self.module.packages.contains(&h.path[0].name.name)
            && !self.this_prop_hides(&h.path[0].name.name)
            && !self.local_stack.last().is_some_and(|m| m.contains_key(&h.path[0].name.name))
            && !self.signal_name_to_id.contains_key(&h.path[0].name.name)
        {
            let key = format!("{}::{}", h.path[0].name.name, last.name.name);
            return imported(&key).then_some((key, None));
        }
        // As the call path does (`exec_dpi_import_call_in`): a prefix that
        // names an instance is the scope the call runs in.
        let prefix = h.path[..h.path.len() - 1]
            .iter()
            .map(|s| s.name.name.as_str())
            .collect::<Vec<_>>()
            .join(".");
        if !self.module.instances.iter().any(|i| i.path == prefix) {
            return None;
        }
        let name = self
            .resolve_dpi_call_name(&last.name.name, Some(&prefix))?
            .into_owned();
        imported(&name).then_some((name, Some(prefix)))
    }

    /// `stmt_is_blocking`: an imported task may consume time (§35.5.2), so
    /// a block or subroutine that calls one runs on the suspend-aware path.
    pub(super) fn expr_calls_dpi_task(&self, e: &Expression) -> bool {
        !self.dpi_task_names.is_empty() && self.dpi_task_callee(e).is_some()
    }

    /// `run_process_stmts`: `stmt` (index `i` of `pc`) is a resume or unwind
    /// marker. `Some(true)` when the process carries on with the next
    /// statement, `Some(false)` when it parked again.
    pub(super) fn dpi_task_marker(
        &mut self,
        pc: &ProcCont,
        i: usize,
        stmt: &Statement,
    ) -> Option<bool> {
        let (id, unwind) = marker_of(stmt)?;
        if unwind {
            self.dpi_task_unwind(id);
            return Some(true);
        }
        let Some(task) = self.dpi_tasks.get_mut(&id) else {
            return Some(true);
        };
        // §9.6.2: a `disable` posted by another process (`disable_remote_block`)
        // unwinds this process out of a block around the call: the task is
        // disabled first (§35.9), then the unwind carries on.
        if self.break_flag && self.disable_target.is_some() {
            task.disabled = true;
        }
        task.after = pc.resume_at(pc.start + i);
        let owner = task.owner;
        Some(self.dpi_task_switch(id) && !self.killed_pids.contains(&owner))
    }

    /// `run_process_stmts`: when `stmt` (index `i` of `pc`) calls an imported
    /// task, run the call on its own stack. `Some(true)` when it returned,
    /// `Some(false)` when the process parked inside it.
    pub(super) fn dpi_task_start(
        &mut self,
        pid: usize,
        pc: &ProcCont,
        i: usize,
        stmt: &Statement,
    ) -> Option<bool> {
        let StatementKind::Expr(e) = &stmt.kind else {
            return None;
        };
        let (name, via) = self.dpi_task_callee(e)?;
        let args = match &e.kind {
            ExprKind::Call { args, .. } => args.clone(),
            _ => Vec::new(),
        };
        let id = self.next_dpi_task;
        self.next_dpi_task += 1;
        let resume = marker(RESUME_MARKER, id);
        let mut task = Box::new(DpiTask {
            fiber: fiber::Fiber::new(),
            owner: pid,
            name,
            args,
            via,
            after: pc.pushed(vec![resume], pc.start + i + 1),
            disabled: false,
            victim: None,
            exports: Vec::new(),
            export_disable: None,
        });
        let sim: *mut Simulator = self;
        let tp: *mut DpiTask = &mut *task;
        task.fiber.set_entry(Box::new(move || {
            // SAFETY: the simulator outlives every fiber it runs, and the
            // task is boxed (it does not move) and stays in `dpi_tasks`
            // until its fiber is done.
            unsafe { (*sim).dpi_task_body(id, tp) }
        }));
        self.dpi_tasks.insert(id, task);
        // A process its own task's code killed does not carry on.
        Some(self.dpi_task_switch(id) && !self.killed_pids.contains(&pid))
    }

    /// The fiber's body: the import call itself.
    fn dpi_task_body(&mut self, id: u64, tp: *mut DpiTask) {
        self.dpi_cur_task = Some(id);
        // SAFETY: see `dpi_task_start`; nothing writes these fields while
        // the call runs.
        let (name, args, via) = unsafe { (&(*tp).name, &(*tp).args, (*tp).via.as_deref()) };
        let _ = self.exec_dpi_import_call_in(name, args, via);
    }

    /// Switch into task `id` until it waits or returns. True when it
    /// returned (and is gone).
    fn dpi_task_switch(&mut self, id: u64) -> bool {
        let Some(task) = self.dpi_tasks.get_mut(&id) else {
            return true;
        };
        let fp: *mut fiber::Fiber = &mut task.fiber;
        let mine = self.dpi_ambient_save();
        // SAFETY: the fiber is boxed in `dpi_tasks` and is removed only
        // below, once done.
        unsafe { fiber::Fiber::resume(fp) };
        self.dpi_ambient_restore(mine);
        let (done, panic) = unsafe { ((*fp).done(), (*fp).take_panic()) };
        if done {
            self.dpi_tasks.remove(&id);
        }
        if let Some(p) = panic {
            std::panic::resume_unwind(p);
        }
        done
    }

    /// On the fiber: switch back to the scheduler. The caller has parked the
    /// owner on what it waits for; this returns once the owner is resumed.
    fn dpi_suspend(&mut self) {
        if self.dpi_unwinding {
            self.break_flag = true;
            self.return_flag = true;
            return;
        }
        let Some(task) = self.dpi_cur_task.and_then(|id| self.dpi_tasks.get_mut(&id)) else {
            return;
        };
        let tp: *mut DpiTask = &mut **task;
        let mine = self.dpi_ambient_save();
        // SAFETY: as in `dpi_task_switch`.
        unsafe { fiber::Fiber::suspend(&raw mut (*tp).fiber) };
        self.dpi_ambient_restore(mine);
        // Other processes ran meanwhile (see `close_decl_shadow_frame`).
        self.nested_run_epoch += 1;
        if let Some(name) = unsafe { (*tp).export_disable.take() } {
            // §35.9: the exported task itself was disabled — it ends here and
            // returns 0; its imported caller is not disabled.
            self.disable_target = Some(name);
            self.break_flag = true;
        }
        if unsafe { (*tp).disabled } {
            // §35.9: the rest of the exported task is skipped, and every
            // subroutine between here and the C code returns at once.
            self.dpi_unwinding = true;
            self.break_flag = true;
            self.return_flag = true;
        }
    }

    /// On the fiber: the owner and the continuation it parks with.
    fn dpi_park_target(&self) -> Option<(usize, ProcCont)> {
        if self.dpi_unwinding {
            return None;
        }
        let t = self.dpi_tasks.get(&self.dpi_cur_task?)?;
        Some((t.owner, t.after.clone()))
    }

    /// Where a wait made on the synchronous path parks: on the calling
    /// process inside an imported task on its own stack, else on a fresh
    /// wake marker for the nested scheduler. `(wake id, pid, continuation)`.
    pub(super) fn sync_wait_target(&mut self) -> (u64, usize, ProcCont) {
        if self.dpi_cur_task.is_some() {
            if let Some((owner, cont)) = self.dpi_park_target() {
                return (0, owner, cont);
            }
        }
        let (id, pid, wake) = self.new_sync_wake();
        (id, pid, vec![wake].into())
    }

    /// Wait for the target `sync_wait_target` registered.
    pub(super) fn sync_wait(&mut self, id: u64) {
        if self.dpi_cur_task.is_some() {
            self.dpi_suspend();
        } else {
            self.run_nested_until(|sim| sim.sync_wakes.remove(&id));
        }
    }

    /// `#d` inside an imported task on its own stack.
    pub(super) fn dpi_wait_delay(&mut self, d: &Expression) {
        let ticks = self.eval_delay_ticks(d);
        let Some((owner, cont)) = self.dpi_park_target() else {
            return self.dpi_suspend();
        };
        if ticks == 0 {
            // §4.4.2.3: `#0` resumes in the Inactive region.
            self.inactive_queue.push((owner, cont));
        } else {
            self.event_queue.schedule(self.time + ticks, owner, cont);
        }
        self.dpi_suspend();
    }

    /// `wait (cond)` with `cond` false, inside an imported task on its own
    /// stack: park until it holds.
    pub(super) fn dpi_wait_condition(&mut self, cond: &Expression) {
        loop {
            let Some((owner, cont)) = self.dpi_park_target() else {
                return self.dpi_suspend();
            };
            self.park_condition_waiter(owner, cont, cond);
            self.dpi_suspend();
            if self.finished || self.dpi_unwinding || self.wait_condition_true(cond) {
                return;
            }
        }
    }

    /// §9.6.1 `wait fork` inside an imported task on its own stack: the
    /// calling process waits for its children.
    pub(super) fn dpi_wait_fork(&mut self) {
        let Some((owner, cont)) = self.dpi_park_target() else {
            return self.dpi_suspend();
        };
        let children: HashSet<usize> = self
            .process_parents
            .iter()
            .filter(|&(_, &p)| p == owner)
            .map(|(&c, _)| c)
            .collect();
        if children.is_empty() {
            return;
        }
        self.join_waiters.push(JoinWaiter {
            parent_pid: owner,
            child_pids: children,
            join_type: JoinType::Join,
            continuation: cont,
            finished_children: HashSet::default(),
            wait_fork: true,
        });
        self.dpi_suspend();
    }

    /// A kill site (`vc_cancel_process`): process `pid` is being killed. If it
    /// is parked inside an imported task, unwind the task by the §35.9
    /// disable protocol — from the scheduler, in the process's own context.
    pub(super) fn dpi_task_killed(&mut self, pid: usize) {
        let Some((&id, _)) = self
            .dpi_tasks
            .iter()
            .find(|(_, t)| t.owner == pid && !t.disabled)
        else {
            return;
        };
        if self.dpi_cur_task == Some(id) {
            // The task's own code killed its process: unwind from here.
            if let Some(t) = self.dpi_tasks.get_mut(&id) {
                t.disabled = true;
            }
            self.dpi_unwinding = true;
            self.break_flag = true;
            self.return_flag = true;
            return;
        }
        let ctx = self.process_contexts.remove(&pid).unwrap_or_default();
        if let Some(t) = self.dpi_tasks.get_mut(&id) {
            t.disabled = true;
            t.victim = Some(ctx);
        }
        let upid = self.next_pid;
        self.next_pid += 1;
        self.event_queue
            .schedule(self.time, upid, vec![marker(UNWIND_MARKER, id)].into());
    }

    /// `run_dpi_export`: record an exported subroutine starting (`enter`) or
    /// ending on the running fiber. True when there is a fiber.
    pub(super) fn dpi_note_export(&mut self, name: &str, enter: bool) -> bool {
        let Some(task) = self.dpi_cur_task.and_then(|id| self.dpi_tasks.get_mut(&id)) else {
            return false;
        };
        if enter {
            task.exports.push(name.to_string());
        } else {
            task.exports.pop();
        }
        true
    }

    /// §35.9, §9.6.2 `disable <task>` where some of `pids` are parked inside
    /// an imported task in which `task` is an exported task running: that
    /// export alone ends, returning 0 to the C code, instead of the process
    /// being killed. Those pids leave `pids`; true when there were any.
    pub(super) fn dpi_disable_exported_task(
        &mut self,
        task: &str,
        pids: &mut HashSet<usize>,
    ) -> bool {
        let leaf = |n: &str| n.rsplit('.').next().unwrap_or(n).to_string();
        let hits: Vec<(u64, usize)> = self
            .dpi_tasks
            .iter()
            .filter(|(_, t)| {
                pids.contains(&t.owner)
                    && !t.disabled
                    && t.exports.iter().any(|e| e == task || leaf(e) == task)
            })
            .map(|(&id, t)| (id, t.owner))
            .collect();
        let mut any = false;
        for (id, owner) in hits {
            let Some(cont) = self.take_parked_cont(owner) else {
                continue;
            };
            if let Some(t) = self.dpi_tasks.get_mut(&id) {
                t.export_disable = Some(task.to_string());
            }
            self.event_queue.schedule(self.time, owner, cont);
            pids.remove(&owner);
            any = true;
        }
        any
    }

    /// The unwind marker: run the killed owner's task to its end, in the
    /// owner's context.
    fn dpi_task_unwind(&mut self, id: u64) {
        let Some(task) = self.dpi_tasks.get_mut(&id) else {
            return;
        };
        let victim = task.victim.take().unwrap_or_default();
        let saved = self.take_process_context();
        self.restore_process_context(victim);
        self.dpi_task_switch(id);
        let _ = self.take_process_context();
        self.restore_process_context(saved);
    }

    /// On a fiber whose stack is nearly used up (C -> SV -> C recursion
    /// through exports): stop with a fatal error instead of faulting on the
    /// guard pages.
    pub(super) fn dpi_stack_exhausted(&mut self) -> bool {
        let Some(task) = self.dpi_cur_task.and_then(|id| self.dpi_tasks.get(&id)) else {
            return false;
        };
        let here = 0u8;
        let sp = &raw const here as usize;
        if sp >= task.fiber.stack_floor() + 2 * 1024 * 1024 {
            return false;
        }
        let name = task.name.clone();
        self.emit_severity_text(
            "Fatal",
            &format!(
                "imported task '{}' recursed through exports until its stack ran out \
                 (XEZIM_DPI_STACK_MB sets the size)",
                name
            ),
        );
        self.fatal_finish_number = Some(1);
        self.finished = true;
        true
    }

    /// A violation of the §35.9 disable protocol is fatal.
    pub(super) fn dpi_protocol_fatal(&mut self, what: &str) {
        let msg = format!("DPI disable protocol (IEEE 1800 clause 35.9): {}", what);
        self.emit_severity_text("Fatal", &msg);
        self.fatal_finish_number = Some(1);
        self.finished = true;
    }

    fn dpi_ambient_save(&self) -> DpiAmbient {
        DpiAmbient {
            current_pid: self.current_pid,
            hint: self.name_resolve_hint.borrow().clone(),
            activation: self.activation_scope.borrow().clone(),
            ts_override: self.timescale_scope_override.clone(),
            export_depth: self.dpi_export_depth,
            in_edge_block: self.in_edge_block,
            break_flag: self.break_flag,
            continue_flag: self.continue_flag,
            return_flag: self.return_flag,
            disable_target: self.disable_target.clone(),
            disable_check_pending: self.disable_check_pending,
            exec_park_cont: self.exec_park_cont.clone(),
            parked_from_exec: self.parked_from_exec,
            m_scope_stack: self.m_scope_stack.clone(),
            current_static_task: self.current_static_task.clone(),
            pkg_scope_stack: self.pkg_scope_stack.clone(),
            decl_shadow_log: self.decl_shadow_log.clone(),
            auto_loop_vars: self.auto_loop_vars.clone(),
            cur_task: self.dpi_cur_task,
            unwinding: self.dpi_unwinding,
            rps_depth: RPS_DEPTH.with(|c| c.get()),
            active_sim: ACTIVE_SIMULATOR.with(|c| c.get()),
            active_scope: ACTIVE_SCOPE.with(|c| c.get()),
        }
    }

    fn dpi_ambient_restore(&mut self, a: DpiAmbient) {
        self.current_pid = a.current_pid;
        *self.name_resolve_hint.borrow_mut() = a.hint;
        *self.activation_scope.borrow_mut() = a.activation;
        self.timescale_scope_override = a.ts_override;
        self.dpi_export_depth = a.export_depth;
        self.in_edge_block = a.in_edge_block;
        self.break_flag = a.break_flag;
        self.continue_flag = a.continue_flag;
        self.return_flag = a.return_flag;
        self.disable_target = a.disable_target;
        self.disable_check_pending = a.disable_check_pending;
        self.exec_park_cont = a.exec_park_cont;
        self.parked_from_exec = a.parked_from_exec;
        self.m_scope_stack = a.m_scope_stack;
        self.current_static_task = a.current_static_task;
        self.pkg_scope_stack = a.pkg_scope_stack;
        self.decl_shadow_log = a.decl_shadow_log;
        self.auto_loop_vars = a.auto_loop_vars;
        self.dpi_cur_task = a.cur_task;
        self.dpi_unwinding = a.unwinding;
        RPS_DEPTH.with(|c| c.set(a.rps_depth));
        ACTIVE_SIMULATOR.with(|c| c.set(a.active_sim));
        ACTIVE_SCOPE.with(|c| c.set(a.active_scope));
    }

    /// The export entry `id` names one C symbol; when instances export it,
    /// the call runs the copy found from the current DPI scope (§35.5.3: the
    /// calling context import's, or `svSetScope`'s) — in that scope or the
    /// nearest scope above it, as the reference simulator looks it up. None
    /// when no copy is visible from there.
    pub(super) fn dpi_export_for_scope(&self, id: usize) -> Option<usize> {
        let exports = &self.module.dpi_exports;
        let c_names = &self.module.dpi_export_c_names;
        let c = c_names.get(id)?;
        // The instance an entry belongs to ("" for the top module); None
        // for a package's.
        let instance_of = |n: &str| -> Option<String> {
            if n.contains("::") {
                return None;
            }
            Some(n.rsplit_once('.').map(|(p, _)| p).unwrap_or("").to_string())
        };
        if !exports[id].contains('.') && c_names.iter().filter(|n| *n == c).count() < 2 {
            return Some(id);
        }
        let scope = ACTIVE_SCOPE.with(|cell| cell.get());
        if scope.is_null() {
            return Some(id);
        }
        let name = dpi_scope_name(scope);
        let rel = vpi_strip_top(self, &name);
        let within = |inst: &str| {
            inst.is_empty()
                || rel == inst
                || (rel.len() > inst.len()
                    && rel.starts_with(inst)
                    && rel.as_bytes()[inst.len()] == b'.')
        };
        let mut best: Option<(usize, usize)> = None;
        let mut pkg: Option<usize> = None;
        for i in (0..exports.len()).filter(|&i| c_names.get(i) == Some(c)) {
            match instance_of(&exports[i]) {
                Some(inst) if within(&inst) => {
                    if best.is_none_or(|(_, l)| inst.len() > l) {
                        best = Some((i, inst.len()));
                    }
                }
                Some(_) => {}
                None => {
                    pkg.get_or_insert(i);
                }
            }
        }
        best.map(|(i, _)| i).or(pkg)
    }
}

/// The name a DPI scope handle stands for ("" for none).
pub(super) fn dpi_scope_name(scope: *mut libc::c_void) -> String {
    if scope.is_null() {
        return String::new();
    }
    // SAFETY: scope handles are the `DpiScope`s `svGetScopeFromName` and
    // the import call hand out, never freed while the simulator lives.
    let s = unsafe { &*(scope as *const DpiScope) };
    String::from_utf8_lossy(&s.name[..s.name_len]).into_owned()
}

/// §35.9 / H.9.5: true while the current imported subroutine runs in the
/// disabled state.
#[unsafe(no_mangle)]
pub extern "C" fn svIsDisabledState() -> libc::c_int {
    try_active_sim("svIsDisabledState", |sim| sim.dpi_unwinding as libc::c_int).unwrap_or(0)
}

/// §35.9 / H.9.5: the C code acknowledges the disable. The protocol checks
/// what it returns, so nothing more is recorded.
#[unsafe(no_mangle)]
pub extern "C" fn svAckDisabledState() {}

#[cfg(all(unix, any(target_arch = "x86_64", target_arch = "aarch64")))]
mod fiber {
    //! A stackful coroutine (`corosensei`, which switches stacks with a few
    //! instructions of its own per architecture): `resume` runs it until it
    //! `suspend`s or its entry returns. The same code serves x86_64 and
    //! aarch64 on Linux (glibc or musl) and macOS (#290). A `Fiber` must not
    //! move once resumed (it lives boxed in a `DpiTask`).
    use corosensei::stack::{Stack, StackPointer};
    use corosensei::{Coroutine, CoroutineResult, Yielder};
    use std::any::Any;
    use std::cell::RefCell;

    pub(in super::super) const SUPPORTED: bool = true;

    /// Inaccessible pages below each stack: an overflow faults instead of
    /// writing over whatever is mapped there.
    const GUARD: usize = 64 * 1024;

    fn stack_len() -> usize {
        static LEN: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
        // Reserved, not committed: pages are backed only once touched. The
        // statement interpreter recurses deeply, and an imported task may
        // recurse through exports and imports, so the default is generous.
        *LEN.get_or_init(|| {
            std::env::var("XEZIM_DPI_STACK_MB")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .filter(|&mb| mb > 0)
                .unwrap_or(256)
                * 1024
                * 1024
                + GUARD
        })
    }

    thread_local! {
        /// Stacks of finished fibers, kept for reuse.
        static POOL: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    }
    const POOL_MAX: usize = 16;

    /// One `mmap`ed stack: `stack_len()` bytes from `map`, the lowest
    /// `GUARD` of them inaccessible.
    struct MappedStack {
        map: usize,
    }

    // SAFETY: the mapping is `stack_len()` bytes with a guard region at the
    // bottom; both ends are page-aligned, so 16-byte aligned.
    unsafe impl Stack for MappedStack {
        fn base(&self) -> StackPointer {
            StackPointer::new(self.map + stack_len()).expect("stack mapping")
        }
        fn limit(&self) -> StackPointer {
            StackPointer::new(self.map).expect("stack mapping")
        }
    }

    type Co = Coroutine<(), (), (), MappedStack>;

    pub(in super::super) struct Fiber {
        co: Option<Co>,
        /// Set by the coroutine when it starts; valid while it lives.
        yielder: *const Yielder<(), ()>,
        entry: Option<Box<dyn FnOnce()>>,
        floor: usize,
        done: bool,
        panic: Option<Box<dyn Any + Send>>,
    }

    impl Fiber {
        pub(in super::super) fn new() -> Self {
            Fiber {
                co: None,
                yielder: std::ptr::null(),
                entry: None,
                floor: 0,
                done: false,
                panic: None,
            }
        }

        pub(in super::super) fn set_entry(&mut self, f: Box<dyn FnOnce()>) {
            self.entry = Some(f);
        }

        pub(in super::super) fn done(&self) -> bool {
            self.done
        }

        pub(in super::super) fn take_panic(&mut self) -> Option<Box<dyn Any + Send>> {
            self.panic.take()
        }

        /// The lowest usable address of the stack (it grows down to here).
        pub(in super::super) fn stack_floor(&self) -> usize {
            self.floor
        }

        /// Run the fiber until it suspends or finishes.
        ///
        /// # Safety
        /// `this` is valid, does not move while the fiber lives, and is not
        /// running.
        pub(in super::super) unsafe fn resume(this: *mut Fiber) {
            unsafe {
                if (*this).done {
                    return;
                }
                if (*this).co.is_none() {
                    let stack = alloc_stack();
                    (*this).floor = stack.map + GUARD;
                    let entry = (*this).entry.take();
                    (*this).co = Some(Coroutine::with_stack(
                        stack,
                        move |y: &Yielder<(), ()>, ()| {
                            (*this).yielder = y;
                            // A panic is carried back to the resumer, which
                            // re-raises it on its own stack.
                            let r =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
                                    if let Some(f) = entry {
                                        f();
                                    }
                                }));
                            if let Err(p) = r {
                                (*this).panic = Some(p);
                            }
                        },
                    ));
                }
                let co: *mut Co = (*this).co.as_mut().expect("fiber");
                if let CoroutineResult::Return(()) = (*co).resume(()) {
                    (*this).done = true;
                }
            }
        }

        /// From inside the fiber: switch back to whoever resumed it.
        ///
        /// # Safety
        /// Called on `this` fiber's own stack.
        pub(in super::super) unsafe fn suspend(this: *mut Fiber) {
            unsafe {
                let y = (*this).yielder;
                if !y.is_null() {
                    (*y).suspend(());
                }
            }
        }
    }

    fn alloc_stack() -> MappedStack {
        if let Some(p) = POOL.with(|p| p.borrow_mut().pop()) {
            return MappedStack { map: p };
        }
        let len = stack_len();
        #[cfg(any(target_os = "linux", target_os = "android"))]
        let flags = libc::MAP_PRIVATE | libc::MAP_ANONYMOUS | libc::MAP_NORESERVE | libc::MAP_STACK;
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        let flags = libc::MAP_PRIVATE | libc::MAP_ANON;
        // SAFETY: a fresh anonymous mapping.
        unsafe {
            let p = libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                flags,
                -1,
                0,
            );
            if p == libc::MAP_FAILED {
                panic!(
                    "cannot map a {} MiB stack for an imported DPI task (XEZIM_DPI_STACK_MB)",
                    len >> 20
                );
            }
            libc::mprotect(p, GUARD, libc::PROT_NONE);
            MappedStack { map: p as usize }
        }
    }

    impl Drop for Fiber {
        fn drop(&mut self) {
            let Some(mut co) = self.co.take() else {
                return;
            };
            // A fiber dropped while suspended (the run ended inside an
            // imported task) leaves its frames unrun, C frames among them:
            // nothing on it is unwound, and the stack is only memory.
            let suspended = !self.done;
            if suspended {
                // SAFETY: nothing on the abandoned stack is ever resumed.
                unsafe { co.force_reset() };
            }
            let stack = co.into_stack();
            let len = stack_len();
            let kept = !suspended
                && POOL.with(|p| {
                    let mut p = p.borrow_mut();
                    if p.len() >= POOL_MAX {
                        return false;
                    }
                    // Give back what the call touched, except the hot top.
                    let top = 256 * 1024;
                    unsafe {
                        libc::madvise(
                            (stack.map + GUARD) as *mut libc::c_void,
                            len - GUARD - top,
                            libc::MADV_DONTNEED,
                        );
                    }
                    p.push(stack.map);
                    true
                });
            if !kept {
                unsafe {
                    libc::munmap(stack.map as *mut libc::c_void, len);
                }
            }
        }
    }
}

#[cfg(not(all(unix, any(target_arch = "x86_64", target_arch = "aarch64"))))]
mod fiber {
    //! No fibers here: imported tasks keep the nested-scheduler fallback
    //! (`dpi_tasks_setup` never arms the mechanism).
    use std::any::Any;

    pub(in super::super) const SUPPORTED: bool = false;

    pub(in super::super) struct Fiber;

    impl Fiber {
        pub(in super::super) fn new() -> Self {
            Fiber
        }
        pub(in super::super) fn set_entry(&mut self, _f: Box<dyn FnOnce()>) {}
        pub(in super::super) fn done(&self) -> bool {
            true
        }
        pub(in super::super) fn take_panic(&mut self) -> Option<Box<dyn Any + Send>> {
            None
        }
        pub(in super::super) fn stack_floor(&self) -> usize {
            0
        }
        pub(in super::super) unsafe fn resume(_this: *mut Fiber) {}
        pub(in super::super) unsafe fn suspend(_this: *mut Fiber) {}
    }
}
