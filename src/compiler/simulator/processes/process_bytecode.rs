//! Loop trampolines and the bytecode of process loops and conditions.

use super::*;

impl Simulator {
    /// Continue executing `cont` for `pid`. Normally recurses (fast path), but
    /// when the `run_process_stmts` recursion is deep — a long synchronous loop
    /// whose body never actually suspends — TRAMPOLINE through the event queue
    /// at the current simulation time to bound the stack. Same time + same
    /// continuation ⇒ identical semantics: `is_pid_suspended` sees the scheduled
    /// entry (via `event_queue.has_pid`) and `run_scheduled_process` snapshots /
    /// restores the process context, exactly as an immediate `@event` re-arm.
    pub(in crate::compiler::simulator) fn continue_stmts_or_trampoline(
        &mut self,
        pid: usize,
        cont: ProcCont,
    ) {
        if RPS_DEPTH.with(|c| c.get()) > RPS_TRAMPOLINE_DEPTH {
            self.cont_pids.insert(pid);
            // The context stays live; mark the entry as this pid's own
            // continuation (see `run_scheduled_process_inner`).
            if !self.local_stack.is_empty() || !self.this_stack.is_empty() {
                self.trampolined_pids.insert(pid);
            }
            self.event_queue.schedule(self.time, pid, cont);
        } else {
            self.run_process_stmts(pid, &cont);
        }
    }

    /// Phase 2 (§9.3.3 `break`/`continue` in suspend-aware loops): when a
    /// blocking loop is RE-ENTERED in the flattened process stream after its
    /// body ran, decide what to do with the loop-control flags the body set.
    ///
    /// Returns `true` if the caller should EXIT this loop now (`break`/
    /// `return` — skip to the statement after the loop); `false` if the loop
    /// should PROCEED (re-enter for the next iteration, or first entry).
    ///
    /// - `return_flag` (set with `break_flag` by a `return` in an inlined
    ///   task body): do NOT consume — unwind to the task's ScopePop is driven
    ///   by the top-of-loop `return_flag` skip. Exit this loop.
    /// - `break_flag` alone: the body's `break` exits this loop — consume it
    ///   (clear break+continue) and exit.
    /// - `continue_flag` alone: consume it and proceed (re-enter the next
    ///   iteration); the body statements after the `continue` were already
    ///   skipped by `exec_statement`'s flag-check at entry.
    ///
    /// First entry (all flags clear) is a no-op → proceed normally.
    pub(in crate::compiler::simulator) fn blocking_loop_flag_gate(&mut self) -> bool {
        if self.return_flag {
            return true;
        }
        if self.break_flag {
            self.break_flag = false;
            self.continue_flag = false;
            return true;
        }
        self.continue_flag = false;
        false
    }

    /// §23.6 / §7.4: resolve literal instance selectors and fixed memory
    /// banks once, before checking a process loop for combinational feedback.
    /// A packed word selector remains an ordinary select on its stored signal.
    fn loop_memory_path(
        &self,
        e: &Expression,
        locals: &[&str],
        scope: Option<&str>,
    ) -> Option<Expression> {
        let ExprKind::Index { expr: base, .. } = &e.kind else {
            return None;
        };
        let literal_index = |e: &Expression| {
            let ExprKind::Number(n) = &e.kind else {
                return None;
            };
            let v = self.eval_number(n);
            if v.is_real {
                return None;
            }
            v.to_index()
        };
        let (name, root) = match &base.kind {
            ExprKind::MemberAccess { .. } => {
                let mut path = Vec::new();
                if !Self::fold_hier_path_segments(base, &mut path)
                    || !path.iter().any(|seg| !seg.selects.is_empty())
                    || path.iter().flat_map(|seg| seg.selects.iter()).any(|i| {
                        literal_index(i)
                            .is_none_or(|v| Some(v) != Self::constant_array_index(i, &self.module))
                    })
                {
                    return None;
                }
                let name = self.constant_selected_storage(base)?;
                let (scope, _) = name.rsplit_once('.')?;
                if self.instance_by_path(&scope).is_none() {
                    return None;
                }
                (name, path.first()?.name.name.clone())
            }
            ExprKind::Index { expr, index } if matches!(index.kind, ExprKind::Number(_)) => {
                let ExprKind::Ident(h) = &expr.kind else {
                    return None;
                };
                if h.root.is_some() || h.path.len() != 1 || !h.path[0].selects.is_empty() {
                    return None;
                }
                let root = &h.path[0].name.name;
                let &(lo, hi, width) = self.module.arrays.get(root)?;
                let idx = literal_index(index)?;
                if !(lo.min(hi)..=lo.max(hi)).contains(&idx)
                    || !self.module.packed_signal_elem_widths.contains_key(root)
                {
                    return None;
                }
                let name = format!("{root}[{idx}]");
                let id = *self.signal_name_to_id.get(name.as_str())?;
                if self.signal_widths.get(id) != Some(&width) {
                    return None;
                }
                (name, root.clone())
            }
            _ => return None,
        };
        if locals.contains(&root.as_str())
            || self.frame_get(&root).is_some()
            || scope.is_some_and(|s| {
                self.signal_name_to_id
                    .contains_key(format!("{s}.{name}").as_str())
            })
        {
            return None;
        }
        Some(Self::flat_name_ident(name, base.span))
    }

    fn loop_memory_stmt<'a>(
        &self,
        stmt: &'a Statement,
        locals: &mut Vec<&'a str>,
        scope: Option<&str>,
    ) -> Option<Statement> {
        use crate::ast::stmt::ForInit;
        use crate::compiler::elaborate::{for_each_sub_expr, for_each_sub_expr_mut};
        if !self.this_stack.is_empty() {
            return None;
        }
        let mut out;
        match &stmt.kind {
            StatementKind::For { init, body, .. } => {
                let len = locals.len();
                for i in init {
                    if let ForInit::VarDecl { name, .. } = i {
                        locals.push(&name.name);
                    }
                }
                let changed = self.loop_memory_stmt(body, locals, scope);
                locals.truncate(len);
                let child = changed?;
                out = stmt.clone();
                if let StatementKind::For { body, .. } = &mut out.kind {
                    **body = child;
                }
            }
            StatementKind::SeqBlock { name, stmts } => {
                // A block declaration may shadow an instance or memory bank.
                if stmts.iter().any(|s| {
                    matches!(
                        s.kind,
                        StatementKind::VarDecl { .. } | StatementKind::ParamDecl { .. }
                    )
                }) {
                    return None;
                }
                let mut changed = None;
                for (i, s) in stmts.iter().enumerate() {
                    if let Some(child) = self.loop_memory_stmt(s, locals, scope) {
                        changed.get_or_insert_with(|| stmts.clone())[i] = child;
                    }
                }
                let stmts = changed?;
                out = Statement::new(
                    StatementKind::SeqBlock {
                        name: name.clone(),
                        stmts,
                    },
                    stmt.span,
                );
            }
            StatementKind::BlockingAssign { lvalue, rvalue } => {
                let rewrite = |e: &Expression| {
                    let mut hit = false;
                    for_each_sub_expr(e, &mut |sub| {
                        if !hit && self.loop_memory_path(sub, locals, scope).is_some() {
                            hit = true;
                        }
                    });
                    if !hit {
                        return None;
                    }
                    let mut result = e.clone();
                    for_each_sub_expr_mut(&mut result, &mut |sub| {
                        if let Some(folded) = self.loop_memory_path(sub, locals, scope)
                            && let ExprKind::Index { expr, .. } = &mut sub.kind
                        {
                            **expr = folded;
                        }
                    });
                    Some(result)
                };
                let l = rewrite(lvalue);
                let r = rewrite(rvalue);
                if l.is_none() && r.is_none() {
                    return None;
                }
                out = Statement::new(
                    StatementKind::BlockingAssign {
                        lvalue: l.unwrap_or_else(|| lvalue.clone()),
                        rvalue: r.unwrap_or_else(|| rvalue.clone()),
                    },
                    stmt.span,
                );
            }
            _ => return None,
        }
        Some(out)
    }

    /// Execute a process's continuation: the current frame from its cursor, then
    /// each chained frame behind it (see `ProcCont`).
    ///
    /// `stmts`/`i` below are the CURRENT frame and an index within it, exactly as
    /// before. What changed is how a continuation is CAPTURED:
    /// `pc.resume_at(pc.start + i + 1)` names "the rest of my work" in O(1) where
    /// `pc.resume_at(pc.start + i + 1)` deep-cloned it, and
    /// `pc.pushed(body, pc.start + i + 1)` splices a task body or flattened block
    /// in front of the caller's tail without copying that tail.
    pub(in crate::compiler::simulator) fn try_exec_process_loop_bytecode(
        &mut self,
        pid: usize,
        stmt: &Statement,
    ) -> bool {
        // No call-frame gate: a process whose own variables were promoted to
        // the signal table has an EMPTY local_stack, and that is fine — there
        // are simply no frame locals to bind. Requiring a frame sent every
        // top-level stimulus loop (`repeat (N) begin @(posedge clk); for …`)
        // through the AST interpreter.
        //
        // `If` joins `For` because a per-cycle self-check
        // (`if (out !== expected(...)) fail++;`) re-executes as many times as
        // any loop — the (pid, span) cache amortizes its compile the same
        // way. The deep, call-following suspend check replaces the shallow
        // event-wait scan for it; anything the compiler cannot honestly
        // express (waits, loop control with no enclosing compiled loop,
        // returns) still bails to the AST path at compile time.
        // Compiled writes land directly in the signal table and would punch
        // through an active `force` (the AST path routes every assign through
        // the force filter). Forces are rare and usually short-lived: fall
        // back to the AST while ANY is active, per execution, keeping the
        // cache intact for when they lift.
        if !self.forced_signals.is_empty()
            || !self.forced_names.is_empty()
            || !self.active_force_exprs.is_empty()
        {
            return false;
        }
        // The vectorized packed-fill fast path beats an N-iteration compiled
        // loop and `exec_statement` would have tried it first — keep that
        // priority now that this runs ahead of `exec_statement`.
        if let StatementKind::For {
            init,
            condition,
            step,
            body,
        } = &stmt.kind
        {
            if self.try_exec_full_packed_fill(init, condition.as_ref(), step, body) {
                return true;
            }
        }
        let eligible = match &stmt.kind {
            StatementKind::For { .. }
            | StatementKind::While { .. }
            | StatementKind::DoWhile { .. }
            | StatementKind::Repeat { .. } => !self.stmt_has_event_wait(stmt),
            _ => false,
        };
        if !eligible || stmt.span.start == 0 && stmt.span.end == 0 {
            static LOOP_STATS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
            if *LOOP_STATS.get_or_init(|| std::env::var_os("XEZIM_PROC_LOOP_STATS").is_some())
                && matches!(
                    stmt.kind,
                    StatementKind::For { .. } | StatementKind::If { .. }
                )
            {
                eprintln!(
                    "[PROC-LOOP] pid={} span={}..{} gate: eligible={} span0={}",
                    pid,
                    stmt.span.start,
                    stmt.span.end,
                    eligible,
                    stmt.span.start == 0 && stmt.span.end == 0
                );
            }
            return false;
        }
        let key = (pid, stmt.span.start, stmt.span.end);
        if !self.process_loop_bytecode.contains_key(&key) {
            let scope_hint = self.process_scope_hint.get(&pid).cloned();
            let mut compiler = super::bytecode::BytecodeCompiler::new(
                &self.signal_name_to_id,
                &self.signal_signed,
                &self.signal_widths,
                &self.module.arrays,
                &self.widths,
            );
            compiler.set_unpacked_structs(&self.unpacked_struct_leaves);
            compiler.set_ast_fallback(false);
            compiler.set_pkg_enum_consts(&self.pkg_enum_consts);
            compiler.set_scope_hint(scope_hint);
            compiler.set_tasks(&self.module.tasks);
            compiler.set_functions(&self.module.functions);
            compiler.set_deferred_callables(
                &self.module.deferred_function_bodies,
                &self.module.deferred_task_bodies,
                &self.module.packages,
            );
            compiler.set_params(&self.module.parameters);
            compiler.set_param_leaf_idx(self.param_leaf_index());
            compiler.set_cast_widths(&self.cast_widths);
            compiler.set_packed_struct_fields(
                self.bc_packed_fields
                    .as_ref()
                    .unwrap_or(&self.module.packed_struct_fields),
            );
            compiler.set_packed_elem_widths(&self.module.packed_signal_elem_widths);
            compiler.set_typedefs(&self.module.typedefs, &self.module.typedef_elem_widths);
            compiler.set_typedef_types(&self.module.typedef_types);
            compiler.set_assoc_elem_widths(&self.module.assoc_elem_widths);
            compiler.set_assoc_arrays(&self.module.associative_arrays);
            compiler.set_packed_full_dims(&self.module.packed_full_dims);
            Self::set_pattern_type_tables(&self.module, &mut compiler);
            compiler.set_packed_struct_fields(
                self.bc_packed_fields
                    .as_ref()
                    .unwrap_or(&self.module.packed_struct_fields),
            );
            compiler.set_multi_dim_arrays(&self.multi_dim_array_names);
            compiler.set_arrays_2d(&self.module.arrays_2d);
            compiler.set_collection_denies(&self.module.dynamic_arrays, &self.module.queue_vars);
            compiler.set_array_first_id(&self.array_first_id);
            compiler.set_string_signals(&self.module.string_signals);
            compiler.set_signal_real(&self.signal_real);
            compiler.set_signal_two_state(&self.signal_two_state);
            compiler.top_module_name = Some(self.module.name.clone());
            if let Some(frame) = self.local_stack.last() {
                compiler.set_process_locals(frame);
            }
            // The feedback walk is per-SPAN (cached below), not per
            // activation — collect_stmt_reads is too heavy to re-run on a
            // 10k-cycle stimulus loop.
            let folded = self.loop_memory_stmt(
                stmt,
                &mut Vec::new(),
                self.process_scope_hint.get(&pid).map(String::as_str),
            );
            let stmt = folded.as_ref().unwrap_or(stmt);
            let feedback = self.stmt_has_comb_feedback(stmt);
            let ok = !feedback && compiler.compile_stmt(stmt);
            if std::env::var_os("XEZIM_PROC_LOOP_STATS").is_some() {
                eprintln!(
                    "[PROC-LOOP] pid={} span={}..{} compiled={} reason={}",
                    pid,
                    stmt.span.start,
                    stmt.span.end,
                    ok,
                    if ok {
                        "-"
                    } else if feedback {
                        "comb_feedback"
                    } else {
                        compiler.bail_reason.unwrap_or("unknown")
                    }
                );
            }
            let compiled = ok.then(|| compiler.finish());
            self.exec_cache_pids.insert(key.0);
            self.process_loop_bytecode.insert(key, compiled);
        }
        let Some(Some(compiled)) = self.process_loop_bytecode.remove(&key) else {
            self.process_loop_bytecode.entry(key).or_insert(None);
            return false;
        };
        if self.vm_regs.len() < compiled.num_regs as usize {
            self.vm_regs
                .resize(compiled.num_regs as usize, Value::zero(1));
        }
        self.exec_insns(&compiled.instructions);
        self.process_loop_bytecode.insert(key, Some(compiled));
        true
    }

    /// Write→comb→read feedback check for the process statement caches: does
    /// this statement WRITE a signal whose (transitive) combinational fan-out
    /// produces a signal the statement also READS? The AST path settles
    /// between statements and loop iterations, so such feedback re-reads a
    /// FRESH value mid-statement (a mutable loop bound driven by always_comb
    /// from the loop body's own writes); a straight-line compiled block only
    /// settles at its boundary and would read stale state. Conservative:
    /// unresolvable names are ignored (they compile-bail elsewhere).
    pub(in crate::compiler::simulator) fn stmt_has_comb_feedback(&self, stmt: &Statement) -> bool {
        // Writes first: the read set names every element of a dynamically
        // indexed array (65,536 per testbench memory-load loop on c906), and
        // it only matters once some comb entry is downstream of a write.
        let mut writes: HashSet<String> = HashSet::default();
        let mut arrays: HashSet<String> = HashSet::default();
        Self::collect_stmt_rw(stmt, &self.module, None, &mut writes, Some(&mut arrays));
        if writes.is_empty() && arrays.is_empty() {
            return false;
        }
        // A DYNAMIC array is a collection, not a signal, so its name maps to
        // no id below: the dependency walk found nothing, the loop compiled,
        // and the compiled element store skipped the dirty-marking the AST
        // interpreter store does. `always_comb` readers were then never
        // re-evaluated and silently kept stale (or x) data --- `for (k) a[k] =
        // v;` on `logic [7:0] a [];` left a reader at x while `a[0]` really
        // held the written value. Treat any write to one as feedback so the
        // loop stays interpreted, which notifies correctly. Static arrays and
        // queues resolve through the walk normally and keep compiling.
        if arrays.iter().any(|name| self.module.dynamic_arrays.contains(name)) || writes.iter().any(|w| {
            let base = w.split('[').next().unwrap_or(w);
            self.module.dynamic_arrays.contains(base)
        }) {
            return true;
        }
        let to_id = |n: &String| self.signal_name_to_id.get(n.as_str()).copied();
        // Every signal the writes reach through comb entries.
        let mut reached: HashSet<usize> = HashSet::default();
        let mut frontier: Vec<usize> = writes.iter().filter_map(to_id).collect();
        // An indexed write may cover a large contiguous fixed memory. The
        // same per-element dependency ids are already registered together.
        // Packed arena cells have no entries in the comb dependency table.
        let deps_end = self.comb_dep_offsets.len().saturating_sub(1);
        for name in &arrays {
            if let Some(&(first, lo, hi)) = self.array_first_id.get(name.as_str()) {
                let count = (hi - lo + 1).max(0) as usize;
                let end = first.saturating_add(count).min(deps_end);
                frontier.extend((first..end).filter(|&id| {
                    self.comb_dep_offsets[id] != self.comb_dep_offsets[id + 1]
                }));
            } else if let Some(&(lo, hi, _)) = self.module.arrays.get(name) {
                frontier.extend((lo..=hi).filter_map(|idx| {
                    self.signal_name_to_id.get(&format!("{name}[{idx}]")).copied()
                }));
            }
        }
        let mut seen_entries: HashSet<u32> = HashSet::default();
        while let Some(id) = frontier.pop() {
            if id + 1 >= self.comb_dep_offsets.len() {
                continue;
            }
            let lo = self.comb_dep_offsets[id] as usize;
            let hi = self.comb_dep_offsets[id + 1] as usize;
            for &e in &self.comb_dep_entries[lo..hi] {
                if !seen_entries.insert(e) {
                    continue;
                }
                for &out in &self.comb_entries[e as usize].cold.write_signal_ids {
                    if reached.insert(out) {
                        frontier.push(out);
                    }
                }
            }
        }
        if reached.is_empty() {
            return false;
        }
        let mut reads: HashSet<String> = HashSet::default();
        writes.clear();
        arrays.clear();
        Self::collect_stmt_rw(stmt, &self.module, Some(&mut reads), &mut writes, Some(&mut arrays));
        reads
            .iter()
            .filter_map(to_id)
            .any(|id| reached.contains(&id))
    }

    /// Is this expression free of side effects, structurally? Conservative:
    /// unknown shapes are NOT effect-free. Calls may be allowed only when
    /// the bytecode compiler subsequently verifies their purity. Runtime
    /// shape probes must reject them before evaluating an index speculatively.
    /// System calls are limited to pure value readers.
    pub(in crate::compiler::simulator) fn cond_expr_is_effect_free(
        e: &Expression,
        compiler_checks_calls: bool,
    ) -> bool {
        let pure = |expr: &Expression| Self::cond_expr_is_effect_free(expr, compiler_checks_calls);
        match &e.kind {
            ExprKind::Number(_) | ExprKind::StringLiteral(_) => true,
            ExprKind::Ident(h) => h.path.iter().all(|seg| seg.selects.iter().all(pure)),
            ExprKind::Paren(i) => pure(i),
            ExprKind::Unary { op, operand } => {
                !matches!(
                    op,
                    UnaryOp::PreIncr | UnaryOp::PostIncr | UnaryOp::PreDecr | UnaryOp::PostDecr
                ) && pure(operand)
            }
            ExprKind::Binary { left, right, .. } => pure(left) && pure(right),
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => pure(condition) && pure(then_expr) && pure(else_expr),
            ExprKind::Concatenation(xs) => xs.iter().all(pure),
            ExprKind::Replication { count, exprs } => pure(count) && exprs.iter().all(pure),
            ExprKind::Index { expr, index } => pure(expr) && pure(index),
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => pure(expr) && pure(left) && pure(right),
            ExprKind::MemberAccess { expr, .. } => pure(expr),
            ExprKind::SystemCall { name, args } => {
                matches!(
                    name.as_str(),
                    "$signed"
                        | "$unsigned"
                        | "$bits"
                        | "$clog2"
                        | "$__xz_named_cast"
                        | "$__xz_size_cast"
                        | SVA_SIG_MARKER
                ) && args.iter().all(pure)
            }
            ExprKind::Call { func, args } => {
                // §7.9/§7.10: a built-in collection QUERY (`q.size()`,
                // `aa.num()`, `aa.exists(k)`, `s.len()`) only reads, so it
                // stays probe-safe. `slices[slices.size()-1] = s` must keep
                // its aggregate-shape probes or the struct copy is lost.
                if let ExprKind::MemberAccess { expr, member } = &func.kind {
                    if matches!(member.name.as_str(), "size" | "num" | "len" | "exists")
                        && pure(expr)
                        && args.iter().all(pure)
                    {
                        return true;
                    }
                }
                compiler_checks_calls
                    && matches!(func.kind, ExprKind::Ident(_))
                    && args.iter().all(pure)
            }
            _ => false,
        }
    }

    /// Evaluate a per-cycle `if` CONDITION through cached bytecode, for the
    /// self-check shape whose statement as a whole cannot compile (its fail
    /// branch may touch a name-keyed initial-scope variable the VM cannot
    /// address). The condition — a wide compare against an inlinable helper
    /// call — is where the time goes; the branch still runs via the AST.
    /// Returns the condition's truth, or None to fall back to `eval_expr`.
    /// Same honesty contract as the statement cache: if the expression does
    /// not fully compile (unknown name, un-inlinable call), the cache pins
    /// None and the AST path stays authoritative.
    pub(in crate::compiler::simulator) fn try_eval_process_cond_bytecode(
        &mut self,
        pid: usize,
        cond: &Expression,
    ) -> Option<bool> {
        if cond.span.start == 0 && cond.span.end == 0 {
            return None;
        }
        // FRAMELESS only: inside a subroutine frame a `static` local needs
        // the AST's dynamic scope (per-instance storage) that the compiled
        // lookup cannot reproduce.
        if !self.local_stack.is_empty() {
            return None;
        }
        let key = (pid, cond.span.start, cond.span.end);
        if !self.process_cond_bytecode.contains_key(&key) {
            self.exec_cache_pids.insert(pid);
            // Side effects would DOUBLE-FIRE during the self-audit below —
            // only cache conditions that provably have none.
            if !Self::cond_expr_is_effect_free(cond, true) {
                self.process_cond_bytecode.insert(key, None);
                return None;
            }
            let scope_hint = self.process_scope_hint.get(&pid).cloned();
            let mut compiler = super::bytecode::BytecodeCompiler::new(
                &self.signal_name_to_id,
                &self.signal_signed,
                &self.signal_widths,
                &self.module.arrays,
                &self.widths,
            );
            compiler.set_unpacked_structs(&self.unpacked_struct_leaves);
            compiler.set_ast_fallback(false);
            compiler.set_pkg_enum_consts(&self.pkg_enum_consts);
            compiler.set_scope_hint(scope_hint);
            compiler.set_tasks(&self.module.tasks);
            compiler.set_functions(&self.module.functions);
            compiler.set_deferred_callables(
                &self.module.deferred_function_bodies,
                &self.module.deferred_task_bodies,
                &self.module.packages,
            );
            compiler.set_params(&self.module.parameters);
            compiler.set_param_leaf_idx(self.param_leaf_index());
            compiler.set_cast_widths(&self.cast_widths);
            compiler.set_packed_struct_fields(
                self.bc_packed_fields
                    .as_ref()
                    .unwrap_or(&self.module.packed_struct_fields),
            );
            compiler.set_packed_elem_widths(&self.module.packed_signal_elem_widths);
            compiler.set_typedefs(&self.module.typedefs, &self.module.typedef_elem_widths);
            compiler.set_typedef_types(&self.module.typedef_types);
            compiler.set_assoc_elem_widths(&self.module.assoc_elem_widths);
            compiler.set_assoc_arrays(&self.module.associative_arrays);
            compiler.set_packed_full_dims(&self.module.packed_full_dims);
            Self::set_pattern_type_tables(&self.module, &mut compiler);
            compiler.set_multi_dim_arrays(&self.multi_dim_array_names);
            compiler.set_arrays_2d(&self.module.arrays_2d);
            compiler.set_collection_denies(&self.module.dynamic_arrays, &self.module.queue_vars);
            compiler.set_array_first_id(&self.array_first_id);
            compiler.set_string_signals(&self.module.string_signals);
            compiler.set_signal_real(&self.signal_real);
            compiler.set_signal_two_state(&self.signal_two_state);
            compiler.top_module_name = Some(self.module.name.clone());
            if let Some(frame) = self.local_stack.last() {
                compiler.set_process_locals(frame);
            }
            let result = compiler.compile_root_expr(cond);
            if std::env::var_os("XEZIM_PROC_LOOP_STATS").is_some() {
                eprintln!(
                    "[PROC-COND] pid={} span={}..{} compiled={} reason={}",
                    pid,
                    cond.span.start,
                    cond.span.end,
                    result.is_some(),
                    if result.is_some() {
                        "-"
                    } else {
                        compiler.bail_reason.unwrap_or("unknown")
                    }
                );
            }
            let compiled = result.map(|reg| (compiler.finish(), reg, COND_AUDIT_EVALS));
            if std::env::var_os("XEZIM_PROC_LOOP_STATS").is_some() {
                if let Some((cb, _, _)) = &compiled {
                    eprintln!(
                        "[PROC-COND-INSNS] {:?}",
                        cb.instructions
                            .iter()
                            .map(|i| format!("{:?}", i))
                            .collect::<Vec<_>>()
                    );
                }
            }
            self.process_cond_bytecode.insert(key, compiled);
        }
        let Some(Some((compiled, reg, audits))) = self.process_cond_bytecode.remove(&key) else {
            self.process_cond_bytecode.entry(key).or_insert(None);
            return None;
        };
        if self.vm_regs.len() < compiled.num_regs as usize {
            self.vm_regs
                .resize(compiled.num_regs as usize, Value::zero(1));
        }
        self.exec_insns(&compiled.instructions);
        let truth = self.vm_regs[reg as usize].is_true();
        // Self-audit: the first activations (and every one under the audit
        // env) also evaluate through the AST. A disagreement means the
        // bytecode compiled this expression with different semantics — pin
        // the span to the AST permanently and use the AST's answer.
        static COND_VERIFY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if audits > 0
            || *COND_VERIFY.get_or_init(|| std::env::var_os("XEZIM_PROC_COND_VERIFY").is_some())
        {
            let ast = self.eval_expr(cond).is_true();
            if ast != truth {
                if std::env::var_os("XEZIM_PROC_LOOP_STATS").is_some()
                    || std::env::var_os("XEZIM_PROC_COND_VERIFY").is_some()
                {
                    eprintln!(
                        "[PROC-COND-DIVERGE] pid={} span={}..{} compiled={} ast={} — pinned to AST",
                        pid, cond.span.start, cond.span.end, truth, ast
                    );
                }
                self.process_cond_bytecode.insert(key, None);
                return Some(ast);
            }
            self.process_cond_bytecode
                .insert(key, Some((compiled, reg, audits.saturating_sub(1))));
            return Some(truth);
        }
        self.process_cond_bytecode
            .insert(key, Some((compiled, reg, 0)));
        Some(truth)
    }
}
