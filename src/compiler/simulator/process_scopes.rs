//! Simulator process scopes.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    /// §9.6.2: identity of a statement frame (its address).
    pub(super) fn frame_key(f: &Arc<[Statement]>) -> usize {
        Arc::as_ptr(f) as *const Statement as usize
    }

    /// A `disable` target (possibly hierarchical, `top.blk`) naming block `n`.
    pub(super) fn block_name_matches(target: &str, n: &str) -> bool {
        target == n || target.rsplit('.').next() == Some(n)
    }

    /// §9.6.2: whether continuation `pc` is inside the named block `x`.
    pub(super) fn cont_has_named_block(&self, pc: &ProcCont, x: &str) -> bool {
        if self.named_block_frames.is_empty() {
            return false;
        }
        let mut cur = Some(pc);
        while let Some(f) = cur {
            if let Some(n) = self.named_block_frames.get(&Self::frame_key(&f.stmts)) {
                if Self::block_name_matches(x, n) {
                    return true;
                }
            }
            cur = f.next.as_deref();
        }
        false
    }

    /// §9.6.2: is process `pid` (running continuation `pc`) unwinding a
    /// pending `disable` through its own frames? Resolves a freshly executed
    /// `disable` first: a target that is not this process's own block or
    /// task is handed to `disable_remote_block` and the flags cleared here.
    pub(super) fn disable_unwinding(&mut self, pid: usize, pc: &ProcCont) -> bool {
        let Some(x) = self.disable_target.clone() else {
            return false;
        };
        let own = |slf: &Self| {
            slf.killed_pids.contains(&pid)
                || slf.cont_has_named_block(pc, &x)
                || slf
                    .active_task_pids
                    .get(&x)
                    .is_some_and(|s| s.contains(&pid))
        };
        if self.disable_check_pending {
            self.disable_check_pending = false;
            if own(self) {
                // The block's own fork children die with it.
                if !self.killed_pids.contains(&pid) && self.cont_has_named_block(pc, &x) {
                    let kids = self.children_forked_in(pid, &x);
                    for k in kids {
                        self.proc_kill(k);
                    }
                }
            } else {
                self.disable_target = None;
                self.break_flag = false;
                self.disable_remote_block(&x);
                return self.break_flag
                    && self.disable_target.is_some()
                    && self.killed_pids.contains(&pid);
            }
        }
        own(self)
    }

    /// Run a task-frame cleanup passed while unwinding a `disable`; the
    /// unwind ends there when the task is the target.
    pub(super) fn disable_pop_task_frame(&mut self) {
        let Some(c) = self.task_cleanup.pop() else {
            return;
        };
        let tname = c.task_name.clone();
        let target = self.disable_target.clone();
        self.unwind_task_frame(c);
        let consumed = match (tname.as_deref(), target.as_deref()) {
            (Some(tn), Some(t)) => {
                t == tn || Self::block_name_matches(t, tn.rsplit('.').next().unwrap_or(tn))
            }
            _ => false,
        };
        if consumed && !self.killed_pids.contains(&self.current_pid) {
            self.disable_target = None;
            self.break_flag = false;
            self.continue_flag = false;
        } else {
            self.disable_target = target;
            self.break_flag = true;
        }
    }

    /// Record a run-loop fork child's spawn point (§9.6.2), pruning entries
    /// of children that have ended.
    pub(super) fn note_fork_spawn(&mut self, child: usize, pc: &ProcCont) {
        if self.named_block_frames.is_empty() {
            return;
        }
        self.fork_spawn_cont.insert(child, pc.clone());
        if self.fork_spawn_cont.len() > self.fork_spawn_prune_at {
            let parents = &self.process_parents;
            self.fork_spawn_cont.retain(|k, _| parents.contains_key(k));
            self.fork_spawn_prune_at = (self.fork_spawn_cont.len() * 2).max(1024);
        }
    }

    /// Is `anc` a fork ancestor of `pid`?
    pub(super) fn is_fork_ancestor(&self, anc: usize, pid: usize) -> bool {
        let mut c = pid;
        let mut steps = 0;
        while let Some(&p) = self.process_parents.get(&c) {
            if p == anc {
                return true;
            }
            c = p;
            steps += 1;
            if steps > 100_000 {
                break;
            }
        }
        false
    }

    /// Direct fork children of `pid` that were spawned inside named block `x`.
    pub(super) fn children_forked_in(&self, pid: usize, x: &str) -> Vec<usize> {
        let mut kids: Vec<usize> = self
            .process_parents
            .iter()
            .filter(|&(_, &p)| p == pid)
            .map(|(&c, _)| c)
            .filter(|c| {
                self.fork_spawn_cont
                    .get(c)
                    .is_some_and(|sp| self.cont_has_named_block(sp, x))
            })
            .collect();
        kids.sort_unstable();
        kids
    }

    /// The continuation process `pid` is parked on, if it is parked.
    pub(super) fn parked_cont_of(&self, pid: usize) -> Option<&ProcCont> {
        if let Some(w) = self
            .join_waiters
            .iter()
            .find(|w| w.parent_pid == pid && !w.wait_fork)
        {
            return Some(&w.continuation);
        }
        if let Some(c) = self.event_queue.pending_stmts_for(pid) {
            return Some(c);
        }
        if let Some(w) = self.event_waiters.iter().find(|w| w.pid == pid) {
            return Some(&w.continuation);
        }
        if let Some((_, c)) = self.condition_waiters.iter().find(|(p, _)| *p == pid) {
            return Some(c);
        }
        if let Some((_, c)) = self.inactive_queue.iter().find(|(p, _)| *p == pid) {
            return Some(c);
        }
        if let Some((_, c)) = self.deferred_clocking_conts.iter().find(|(p, _)| *p == pid) {
            return Some(c);
        }
        if let Some((w, _)) = self
            .deferred_clocking_guarded
            .iter()
            .find(|(w, _)| w.pid == pid)
        {
            return Some(&w.continuation);
        }
        None
    }

    /// Remove process `pid` from wherever it is parked and return its
    /// continuation.
    pub(super) fn take_parked_cont(&mut self, pid: usize) -> Option<ProcCont> {
        if let Some(idx) = self
            .join_waiters
            .iter()
            .position(|w| w.parent_pid == pid && !w.wait_fork)
        {
            return Some(self.join_waiters.remove(idx).continuation);
        }
        // Remote disable abandons the rest of the named block, including
        // its clocking guard/count. This is not a process::suspend extraction:
        // resuming an explicit suspension must retain those wait conditions.
        if let Some(idx) = self
            .deferred_clocking_conts
            .iter()
            .position(|(p, _)| *p == pid)
        {
            return Some(self.deferred_clocking_conts.remove(idx).1);
        }
        if let Some(idx) = self
            .deferred_clocking_guarded
            .iter()
            .position(|(w, _)| w.pid == pid)
        {
            return Some(self.deferred_clocking_guarded.remove(idx).0.continuation);
        }
        self.proc_suspend(pid);
        let info = self.suspended_proc_info.remove(&pid)?;
        self.suspended_pids.remove(&pid);
        Some(info.continuation)
    }

    /// IEEE 1800-2023 §9.6.2: `disable x` where `x` is a block (or task)
    /// that ANOTHER process is executing — an ancestor the disabling process
    /// was forked inside, or any process parked inside it. That process
    /// resumes at the end of the block, and every process forked inside the
    /// block, the disabling one included, is terminated.
    pub(super) fn disable_remote_block(&mut self, x: &str) {
        let me = self.current_pid;
        let mut owner: Option<usize> = None;
        let mut task_owner = false;
        let mut c = me;
        let mut steps = 0;
        while let Some(&a) = self.process_parents.get(&c) {
            if self
                .fork_spawn_cont
                .get(&c)
                .is_some_and(|sp| self.cont_has_named_block(sp, x))
            {
                owner = Some(a);
                break;
            }
            if self.active_task_pids.get(x).is_some_and(|s| s.contains(&a)) {
                owner = Some(a);
                task_owner = true;
                break;
            }
            c = a;
            steps += 1;
            if steps > 100_000 {
                break;
            }
        }
        if owner.is_none() && !self.named_block_frames.is_empty() {
            let mut cands: Vec<usize> = self.join_waiters.iter().map(|w| w.parent_pid).collect();
            cands.extend(self.event_waiters.iter().map(|w| w.pid));
            cands.extend(self.condition_waiters.iter().map(|(p, _)| *p));
            cands.extend(self.inactive_queue.iter().map(|(p, _)| *p));
            cands.extend(self.deferred_clocking_conts.iter().map(|(p, _)| *p));
            cands.extend(self.deferred_clocking_guarded.iter().map(|(w, _)| w.pid));
            cands.extend(
                self.event_queue
                    .pending_pid_times()
                    .into_iter()
                    .map(|(p, _)| p),
            );
            cands.sort_unstable();
            cands.dedup();
            owner = cands.into_iter().find(|&p| {
                p != me
                    && !self.killed_pids.contains(&p)
                    && self
                        .parked_cont_of(p)
                        .is_some_and(|pc| self.cont_has_named_block(pc, x))
            });
        }
        let Some(a) = owner else {
            return;
        };
        let parked_inside = self
            .parked_cont_of(a)
            .is_some_and(|pc| task_owner || self.cont_has_named_block(pc, x));
        if !parked_inside {
            return;
        }
        let kids: Vec<usize> = if task_owner {
            let mut v: Vec<usize> = self
                .process_parents
                .iter()
                .filter(|&(_, &p)| p == a)
                .map(|(&c, _)| c)
                .collect();
            v.sort_unstable();
            v
        } else {
            self.children_forked_in(a, x)
        };
        let mut doomed: HashSet<usize> = HashSet::default();
        for &k in &kids {
            doomed.insert(k);
            doomed.extend(self.collect_fork_descendants(k));
        }
        let Some(cont) = self.take_parked_cont(a) else {
            return;
        };
        for k in kids {
            self.proc_kill(k);
        }
        self.pending_block_disable.insert(a, x.to_string());
        self.event_queue.schedule(self.time, a, cont);
        if doomed.contains(&me) {
            self.disable_target = Some(x.to_string());
            self.break_flag = true;
        }
    }

    /// Check if a statement contains @(event) waits.
    pub(super) fn stmt_has_event_wait(&self, stmt: &Statement) -> bool {
        match &stmt.kind {
            StatementKind::TimingControl {
                control: TimingControl::Event(_),
                ..
            } => true,
            StatementKind::TimingControl {
                control: TimingControl::Delay(_),
                ..
            } => true,
            // §9.4.5 intra-assignment delay / event control suspends like a
            // `#d` / `@(e)` statement.
            StatementKind::BlockingAssign { rvalue, .. } => Self::intra_timing_suspends(rvalue),
            StatementKind::SeqBlock { stmts, .. } => {
                stmts.iter().any(|s| self.stmt_has_event_wait(s))
            }
            _ => false,
        }
    }

    /// §9.7: structurally check if `expr` is a `handle.await()` call on a
    /// process handle (no `&mut self` needed — a static shape check). Used by
    /// `stmt_is_blocking` so that a `forever { h.await(); ... }` body is
    /// detected as blocking and routed through `run_process_stmts`, where the
    /// await is intercepted and the process parks in `await_waiters`.
    pub(super) fn expr_is_proc_await(expr: &Expression) -> bool {
        if let ExprKind::Call { func, args } = &expr.kind {
            if args.is_empty() {
                // MemberAccess form: `job.await()`
                if let ExprKind::MemberAccess { member, .. } = &func.kind {
                    return member.name.as_str() == "await";
                }
                // Flattened Ident form: `job.await` parses as Ident([job, await])
                if let ExprKind::Ident(hier) = &func.kind {
                    return hier.path.len() == 2 && hier.path[1].name.name == "await";
                }
            }
        }
        false
    }

    /// Every name a block of `stmt` declares (any depth), once per
    /// declarator, and whether it has unpacked dimensions.
    pub(super) fn for_each_decl_name<'a>(stmt: &'a Statement, f: &mut dyn FnMut(&'a str, bool)) {
        let mut walk = |s: &'a Statement| Self::for_each_decl_name(s, f);
        match &stmt.kind {
            StatementKind::VarDecl { declarators, .. } => {
                for d in declarators {
                    f(&d.name.name, !d.dimensions.is_empty());
                }
            }
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                stmts.iter().for_each(walk)
            }
            StatementKind::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                walk(then_stmt);
                if let Some(e) = else_stmt {
                    walk(e);
                }
            }
            StatementKind::Case { items, .. } => items.iter().for_each(|it| walk(&it.stmt)),
            StatementKind::For { body, .. }
            | StatementKind::While { body, .. }
            | StatementKind::DoWhile { body, .. }
            | StatementKind::Repeat { body, .. }
            | StatementKind::Forever { body }
            | StatementKind::Foreach { body, .. }
            | StatementKind::TimingControl { stmt: body, .. }
            | StatementKind::Wait { stmt: body, .. } => walk(body),
            _ => {}
        }
    }

    /// Whether `rename_nested_shadowed` may rename anything in `stmt`: some
    /// collection local named like a claimed name or like an earlier local
    /// of the body (a read-only scan, so a body without one is not cloned).
    pub(super) fn shadow_rename_possible(stmt: &Statement, claims: &dyn Fn(&str) -> bool) -> bool {
        let mut seen: Vec<&str> = Vec::new();
        let mut hit = false;
        Self::for_each_decl_name(stmt, &mut |n, dims| {
            if dims && !hit && (seen.contains(&n) || claims(n)) {
                hit = true;
            }
            seen.push(n);
        });
        hit
    }

    /// §6.21/§13.3/§13.4: a variable declared in a STATIC task or function
    /// (one neither declared nor defaulted `automatic`) is allocated once and
    /// keeps its value from one call to the next. Mark each such declaration
    /// whose value can carry over (`static_local_carries`) explicitly
    /// `static`, which gives it the subroutine's persistent store; the
    /// others behave the same either way and stay as they are, so their
    /// subroutine still compiles and inlines.
    pub(super) fn collect_nba_roots(stmt: &Statement, names: &mut HashSet<String>) {
        fn root(e: &Expression, names: &mut HashSet<String>) {
            match &e.kind {
                ExprKind::Index { expr, .. }
                | ExprKind::RangeSelect { expr, .. }
                | ExprKind::MemberAccess { expr, .. }
                | ExprKind::Paren(expr) => root(expr, names),
                ExprKind::Ident(h) if h.root.is_none() && !h.path.is_empty() => {
                    names.insert(h.path[0].name.name.clone());
                }
                ExprKind::Concatenation(parts) => {
                    for e in parts {
                        root(e, names);
                    }
                }
                _ => {}
            }
        }
        match &stmt.kind {
            StatementKind::NonblockingAssign { lvalue, .. } => root(lvalue, names),
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                for s in stmts {
                    Self::collect_nba_roots(s, names);
                }
            }
            StatementKind::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                Self::collect_nba_roots(then_stmt, names);
                if let Some(s) = else_stmt {
                    Self::collect_nba_roots(s, names);
                }
            }
            StatementKind::Case { items, .. } => {
                for i in items {
                    Self::collect_nba_roots(&i.stmt, names);
                }
            }
            StatementKind::For { body, .. }
            | StatementKind::While { body, .. }
            | StatementKind::DoWhile { body, .. }
            | StatementKind::Repeat { body, .. }
            | StatementKind::Forever { body }
            | StatementKind::Foreach { body, .. }
            | StatementKind::TimingControl { stmt: body, .. }
            | StatementKind::Wait { stmt: body, .. } => {
                Self::collect_nba_roots(body, names);
            }
            _ => {}
        }
    }

    pub(super) fn mark_static_locals(items: &mut [Statement]) {
        let mut nba_names = HashSet::default();
        for stmt in items.iter() {
            Self::collect_nba_roots(stmt, &mut nba_names);
        }
        for i in 0..items.len() {
            let (head, rest) = items.split_at_mut(i + 1);
            let s = &mut head[i];
            match &mut s.kind {
                StatementKind::VarDecl {
                    lifetime,
                    declarators,
                    data_type,
                } => {
                    if lifetime.is_none()
                        // Keep the chosen policy for initialized declarations
                        // without an explicit lifetime, including mixed lists.
                        && declarators.iter().all(|d| d.init.is_none())
                        && declarators.iter().any(|d| {
                            d.init.is_none()
                                && (!d.dimensions.is_empty()
                                    || nba_names.contains(&d.name.name)
                                    || (matches!(
                                        data_type,
                                        DataType::IntegerAtom { .. }
                                            | DataType::IntegerVector { .. }
                                            | DataType::Real { .. }
                                    )
                                        && super::bytecode::BytecodeCompiler::static_local_carries(
                                            &d.name.name,
                                            rest,
                                        )))
                        })
                    {
                        *lifetime = Some(crate::ast::types::Lifetime::Static);
                    }
                }
                StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                    Self::mark_static_locals(stmts)
                }
                StatementKind::If {
                    then_stmt,
                    else_stmt,
                    ..
                } => {
                    Self::mark_static_locals(std::slice::from_mut(&mut **then_stmt));
                    if let Some(e) = else_stmt {
                        Self::mark_static_locals(std::slice::from_mut(&mut **e));
                    }
                }
                StatementKind::Case { items, .. } => {
                    for it in items.iter_mut() {
                        Self::mark_static_locals(std::slice::from_mut(&mut it.stmt));
                    }
                }
                StatementKind::For { body, .. }
                | StatementKind::While { body, .. }
                | StatementKind::DoWhile { body, .. }
                | StatementKind::Repeat { body, .. }
                | StatementKind::Forever { body }
                | StatementKind::Foreach { body, .. }
                | StatementKind::TimingControl { stmt: body, .. }
                | StatementKind::Wait { stmt: body, .. } => {
                    Self::mark_static_locals(std::slice::from_mut(&mut **body))
                }
                _ => {}
            }
        }
    }

    /// §6.21 / §13.3 / §13.4: module subroutine statics have one
    /// declaration-owned store, shared by recursion and allocated before
    /// time zero. Allocate all cells first, then evaluate their initializers.
    pub(super) fn hoist_subroutine_statics(
        module: &mut ElaboratedModule,
    ) -> (HashSet<String>, HashSet<String>) {
        fn selected(s: &Statement, nba_names: &HashSet<String>) -> bool {
            match &s.kind {
                StatementKind::VarDecl {
                    lifetime: Some(crate::ast::types::Lifetime::Static),
                    declarators,
                    ..
                } => declarators.iter().any(|d| {
                    d.init.is_some() || !d.dimensions.is_empty() || nba_names.contains(&d.name.name)
                }),
                StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                    stmts.iter().any(|s| selected(s, nba_names))
                }
                StatementKind::If {
                    then_stmt,
                    else_stmt,
                    ..
                } => {
                    selected(then_stmt, nba_names)
                        || else_stmt.as_deref().is_some_and(|s| selected(s, nba_names))
                }
                StatementKind::Case { items, .. } => {
                    items.iter().any(|i| selected(&i.stmt, nba_names))
                }
                StatementKind::For { body, .. }
                | StatementKind::While { body, .. }
                | StatementKind::DoWhile { body, .. }
                | StatementKind::Repeat { body, .. }
                | StatementKind::Forever { body }
                | StatementKind::Foreach { body, .. }
                | StatementKind::TimingControl { stmt: body, .. }
                | StatementKind::Wait { stmt: body, .. } => selected(body, nba_names),
                _ => false,
            }
        }
        fn collect(
            s: &Statement,
            module: &ElaboratedModule,
            formals: &HashMap<String, Expression>,
            allocate: &mut Vec<Statement>,
            initialize: &mut Vec<Statement>,
            names: &mut HashSet<String>,
            nba_names: &HashSet<String>,
        ) {
            match &s.kind {
                StatementKind::VarDecl {
                    data_type,
                    lifetime: Some(crate::ast::types::Lifetime::Static),
                    declarators,
                } => {
                    for d in declarators {
                        if d.init.is_none()
                            && d.dimensions.is_empty()
                            && !nba_names.contains(&d.name.name)
                        {
                            continue;
                        }
                        names.insert(d.name.name.clone());
                        let mut storage = d.clone();
                        storage.init = None;
                        allocate.push(Statement::new(
                            StatementKind::VarDecl {
                                data_type: data_type.clone(),
                                lifetime: Some(crate::ast::types::Lifetime::Static),
                                declarators: vec![storage],
                            },
                            s.span,
                        ));
                        if let Some(init) = &d.init {
                            let empty = std::collections::HashSet::new();
                            let rhs = super::elaborate::rewrite_expr(
                                init,
                                "",
                                formals,
                                &empty,
                                &HashMap::default(),
                            );
                            let lhs = Expression::new(
                                ExprKind::Ident(HierarchicalIdentifier {
                                    root: None,
                                    path: vec![crate::ast::expr::HierPathSegment {
                                        name: d.name.clone(),
                                        selects: vec![],
                                    }],
                                    span: d.span,
                                    cached_signal_id: std::cell::Cell::new(None),
                                    cached_resolved_name: std::cell::OnceCell::new(),
                                }),
                                d.span,
                            );
                            initialize.push(Statement::new(
                                StatementKind::BlockingAssign {
                                    lvalue: lhs,
                                    rvalue: rhs,
                                },
                                s.span,
                            ));
                        }
                    }
                }
                StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                    for st in stmts {
                        collect(st, module, formals, allocate, initialize, names, nba_names);
                    }
                }
                StatementKind::If {
                    then_stmt,
                    else_stmt,
                    ..
                } => {
                    collect(
                        then_stmt, module, formals, allocate, initialize, names, nba_names,
                    );
                    if let Some(st) = else_stmt {
                        collect(st, module, formals, allocate, initialize, names, nba_names);
                    }
                }
                StatementKind::Case { items, .. } => {
                    for i in items {
                        collect(
                            &i.stmt, module, formals, allocate, initialize, names, nba_names,
                        );
                    }
                }
                StatementKind::For { body, .. }
                | StatementKind::While { body, .. }
                | StatementKind::DoWhile { body, .. }
                | StatementKind::Repeat { body, .. }
                | StatementKind::Forever { body }
                | StatementKind::Foreach { body, .. }
                | StatementKind::TimingControl { stmt: body, .. }
                | StatementKind::Wait { stmt: body, .. } => collect(
                    body, module, formals, allocate, initialize, names, nba_names,
                ),
                _ => {}
            }
        }
        fn prepare(
            module: &ElaboratedModule,
            items: &mut Vec<Statement>,
            ports: &[crate::ast::decl::FunctionPort],
            routine: &str,
            ordinal: usize,
            allocate: &mut Vec<Statement>,
            initialize: &mut Vec<Statement>,
            names: &mut HashSet<String>,
        ) -> bool {
            let mut nba_names = HashSet::default();
            for stmt in items.iter() {
                Simulator::collect_nba_roots(stmt, &mut nba_names);
            }
            if !items.iter().any(|s| selected(s, &nba_names)) {
                return false;
            }
            let span = items
                .first()
                .map(|s| s.span)
                .unwrap_or_else(crate::ast::Span::dummy);
            let mut body = Statement::new(
                StatementKind::SeqBlock {
                    name: Some(crate::ast::Identifier {
                        name: routine.to_string(),
                        span,
                    }),
                    stmts: std::mem::take(items),
                },
                span,
            );
            // Rename inner declarations before their enclosing ones so every
            // static owns a unique cell even when sibling blocks reuse a name.
            Simulator::rename_nested_shadowed(
                &mut body,
                &|_| true,
                &mut Vec::new(),
                &mut 0,
                &format!("psub{ordinal}_"),
                1,
                "",
                &mut Vec::new(),
            );
            let mut formals = HashMap::default();
            for port in ports {
                let w = super::elaborate::resolve_type_width(
                    &port.data_type,
                    Some(&module.parameters),
                    Some(&module.typedefs),
                )
                .max(1);
                let two = super::elaborate::is_type_two_state_resolved(
                    &port.data_type,
                    &module.typedef_types,
                );
                let expression = if matches!(
                    &port.data_type,
                    DataType::Simple {
                        kind: crate::ast::types::SimpleType::String,
                        ..
                    }
                ) {
                    Expression::new(ExprKind::StringLiteral(String::new()), port.span)
                } else if super::elaborate::is_type_real(&port.data_type) {
                    Expression::new(
                        ExprKind::Number(crate::ast::expr::NumberLiteral::Real(0.0)),
                        port.span,
                    )
                } else {
                    Expression::new(
                        ExprKind::Number(crate::ast::expr::NumberLiteral::Integer {
                            size: Some(w),
                            signed: super::elaborate::is_type_signed_resolved(
                                &port.data_type,
                                &module.typedef_types,
                            ),
                            base: crate::ast::expr::NumberBase::Binary,
                            value: if two {
                                "0".repeat(w as usize)
                            } else {
                                "x".repeat(w as usize)
                            },
                            cached_val: std::cell::Cell::new(None),
                        }),
                        port.span,
                    )
                };
                formals.insert(port.name.name.clone(), expression);
            }
            nba_names.clear();
            Simulator::collect_nba_roots(&body, &mut nba_names);
            collect(
                &body, module, &formals, allocate, initialize, names, &nba_names,
            );
            if let StatementKind::SeqBlock { stmts, .. } = body.kind {
                *items = stmts;
            }
            true
        }
        let mut names = HashSet::default();
        let mut routines = HashSet::default();
        let mut allocate = Vec::new();
        let mut initialize = Vec::new();
        let mut ordinal = 0usize;
        let mut functions: Vec<_> = module.functions.keys().cloned().collect();
        functions.sort();
        let mut rewritten: HashMap<String, Vec<Statement>> = HashMap::default();
        for key in functions {
            let Some(original) = module.functions.get(&key) else {
                continue;
            };
            if !original.name.scopes.is_empty() {
                continue;
            }
            let mut fd = original.clone();
            if let Some(items) = rewritten.get(&fd.name.name.name) {
                fd.items = items.clone();
            } else {
                ordinal += 1;
                if !prepare(
                    module,
                    &mut fd.items,
                    &fd.ports,
                    &fd.name.name.name,
                    ordinal,
                    &mut allocate,
                    &mut initialize,
                    &mut names,
                ) {
                    continue;
                }
                routines.insert(fd.name.name.name.clone());
                rewritten.insert(fd.name.name.name.clone(), fd.items.clone());
            }
            module.functions.insert(key, fd);
        }
        let mut tasks: Vec<_> = module.tasks.keys().cloned().collect();
        tasks.sort();
        for key in tasks {
            let Some(mut td) = module.tasks.remove(&key) else {
                continue;
            };
            if td.name.scopes.is_empty() {
                ordinal += 1;
                if prepare(
                    module,
                    &mut td.items,
                    &td.ports,
                    &td.name.name.name,
                    ordinal,
                    &mut allocate,
                    &mut initialize,
                    &mut names,
                ) {
                    routines.insert(td.name.name.name.clone());
                }
            }
            module.tasks.insert(key, td);
        }
        if !allocate.is_empty() {
            let block = |stmts: Vec<Statement>| super::elaborate::InitialBlock {
                stmt: Statement::new(
                    StatementKind::SeqBlock { name: None, stmts },
                    crate::ast::Span::dummy(),
                ),
                scope: String::new(),
            };
            module.static_init_blocks.insert(0, block(allocate));
            if !initialize.is_empty() {
                module.static_init_blocks.push(block(initialize));
            }
        }
        (names, routines)
    }

    /// Would `mark_static_locals` mark anything in `items` (read-only, so a
    /// subroutine with nothing to mark is not cloned)?
    pub(super) fn static_locals_to_mark(items: &[Statement]) -> bool {
        let mut probe = items.to_vec();
        Self::mark_static_locals(&mut probe);
        probe.iter().zip(items).any(|(a, b)| {
            let mut na = 0usize;
            let mut nb = 0usize;
            Self::count_static_decls(a, &mut na);
            Self::count_static_decls(b, &mut nb);
            na != nb
        })
    }

    pub(super) fn count_static_decls(s: &Statement, n: &mut usize) {
        if let StatementKind::VarDecl {
            lifetime: Some(crate::ast::types::Lifetime::Static),
            ..
        } = &s.kind
        {
            *n += 1;
        }
        let mut walk = |c: &Statement| Self::count_static_decls(c, n);
        match &s.kind {
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                stmts.iter().for_each(&mut walk)
            }
            StatementKind::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                walk(then_stmt);
                if let Some(e) = else_stmt {
                    walk(e);
                }
            }
            StatementKind::Case { items, .. } => items.iter().for_each(|it| walk(&it.stmt)),
            StatementKind::For { body, .. }
            | StatementKind::While { body, .. }
            | StatementKind::DoWhile { body, .. }
            | StatementKind::Repeat { body, .. }
            | StatementKind::Forever { body }
            | StatementKind::Foreach { body, .. }
            | StatementKind::TimingControl { stmt: body, .. }
            | StatementKind::Wait { stmt: body, .. } => walk(body),
            _ => {}
        }
    }

    /// Does `stmt` declare a variable that is not `automatic`?
    pub(super) fn stmt_has_static_decl(stmt: &Statement) -> bool {
        let mut decls = Vec::new();
        Self::collect_static_decls(stmt, &mut decls);
        !decls.is_empty()
    }

    /// Is `e` built from literals and variable reads alone (no call)?
    pub(super) fn init_is_plain(e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Number(_) | ExprKind::StringLiteral(_) => true,
            ExprKind::Ident(h) => h
                .path
                .iter()
                .all(|seg| seg.selects.iter().all(Self::init_is_plain)),
            ExprKind::Paren(x) | ExprKind::Unary { operand: x, .. } => Self::init_is_plain(x),
            ExprKind::Binary { left, right, .. } => {
                Self::init_is_plain(left) && Self::init_is_plain(right)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::init_is_plain(condition)
                    && Self::init_is_plain(then_expr)
                    && Self::init_is_plain(else_expr)
            }
            ExprKind::Concatenation(xs) => xs.iter().all(Self::init_is_plain),
            _ => false,
        }
    }

    /// Does `stmt` declare a typedef in one of its blocks?
    pub(super) fn stmt_has_local_typedef(stmt: &Statement) -> bool {
        match &stmt.kind {
            StatementKind::Typedef(_) => true,
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                stmts.iter().any(Self::stmt_has_local_typedef)
            }
            StatementKind::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                Self::stmt_has_local_typedef(then_stmt)
                    || else_stmt
                        .as_deref()
                        .is_some_and(Self::stmt_has_local_typedef)
            }
            StatementKind::Case { items, .. } => items
                .iter()
                .any(|it| Self::stmt_has_local_typedef(&it.stmt)),
            StatementKind::For { body, .. }
            | StatementKind::While { body, .. }
            | StatementKind::DoWhile { body, .. }
            | StatementKind::Repeat { body, .. }
            | StatementKind::Forever { body }
            | StatementKind::Foreach { body, .. }
            | StatementKind::TimingControl { stmt: body, .. }
            | StatementKind::Wait { stmt: body, .. } => Self::stmt_has_local_typedef(body),
            _ => false,
        }
    }

    /// The declarations of `stmt`'s static block variables (§6.21: any
    /// block variable of a process not declared `automatic`), in order.
    pub(super) fn collect_static_decls(stmt: &Statement, out: &mut Vec<Statement>) {
        let mut walk = |c: &Statement| Self::collect_static_decls(c, out);
        match &stmt.kind {
            StatementKind::VarDecl {
                lifetime,
                declarators,
                data_type,
            } => {
                // See `exec_stmt_var_decl`: an initializer needs an explicit
                // `static`.
                let explicit = matches!(lifetime, Some(crate::ast::types::Lifetime::Static));
                if !matches!(lifetime, Some(crate::ast::types::Lifetime::Automatic)) {
                    // An initializer that calls something (`process::self()`,
                    // `new`) runs where the block first reaches it.
                    let ds: Vec<VarDeclarator> = declarators
                        .iter()
                        .filter(|d| match &d.init {
                            None => true,
                            Some(e) => explicit && Self::init_is_plain(e),
                        })
                        .cloned()
                        .collect();
                    if !ds.is_empty() {
                        out.push(Statement::new(
                            StatementKind::VarDecl {
                                lifetime: lifetime.clone(),
                                declarators: ds,
                                data_type: data_type.clone(),
                            },
                            stmt.span,
                        ));
                    }
                }
            }
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                stmts.iter().for_each(walk)
            }
            StatementKind::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                walk(then_stmt);
                if let Some(e) = else_stmt {
                    walk(e);
                }
            }
            StatementKind::Case { items, .. } => items.iter().for_each(|it| walk(&it.stmt)),
            StatementKind::For { body, .. }
            | StatementKind::While { body, .. }
            | StatementKind::DoWhile { body, .. }
            | StatementKind::Repeat { body, .. }
            | StatementKind::Forever { body }
            | StatementKind::Foreach { body, .. }
            | StatementKind::TimingControl { stmt: body, .. }
            | StatementKind::Wait { stmt: body, .. } => walk(body),
            _ => {}
        }
    }

    /// §6.21: a static block variable is created, and its initializer run,
    /// once before any process starts — not when its block is first entered:
    /// `blk.v` reads its initial value at time 0 even while the block waits
    /// on its first event. Run a process body's static declarations in a
    /// static-initializer process of its scope; the block's own declaration
    /// then finds them done (`static_block_decls`).
    pub(super) fn hoist_static_decls(&mut self, stmt: &Statement, scope: &str) {
        let mut decls = Vec::new();
        Self::collect_static_decls(stmt, &mut decls);
        // A declaration typed by a typedef local to the process exists only
        // once the block has run that typedef: leave the process alone.
        if decls.is_empty() || Self::stmt_has_local_typedef(stmt) {
            return;
        }
        let span = stmt.span;
        self.module
            .static_init_blocks
            .push(super::elaborate::InitialBlock {
                stmt: Statement::new(
                    StatementKind::SeqBlock {
                        name: None,
                        stmts: decls,
                    },
                    span,
                ),
                scope: scope.to_string(),
            });
    }

    /// An instance's always block, rewritten for its instance, with its
    /// block locals given storage of their own (`rename_instance_locals`).
    pub(super) fn materialize_pending_always(p: super::elaborate::PendingAlways) -> AlwaysBlock {
        let mut ab = p.materialize();
        Self::rename_instance_locals(&mut ab.stmt, &ab.scope);
        ab
    }

    /// §6.21/§23.6: a process of a module INSTANCE (`scope` its path) keeps
    /// its block locals' bare names while the module's items carry the
    /// instance path, so every instance of the module — and every
    /// generate-loop copy — shared one variable per local. A static local
    /// keeps its value between activations, so the instances then saw each
    /// other's values. Store each local of the block, at any depth, under a
    /// name of its own instance: a named block's under its hierarchical name
    /// (`scope.blk.v`), an unnamed block's salted with the scope. The names
    /// depend only on the block and its scope, so materializing the same
    /// block again gives the same storage.
    pub(super) fn rename_instance_locals(stmt: &mut Statement, scope: &str) {
        if scope.is_empty() {
            return;
        }
        let mut declares = false;
        Self::for_each_decl_name(stmt, &mut |_, _| declares = true);
        if !declares {
            return;
        }
        let mut tag = String::with_capacity(scope.len() + 2);
        tag.push('p');
        tag.extend(
            scope
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }),
        );
        tag.push('_');
        Self::rename_nested_shadowed(
            stmt,
            &|_| true,
            &mut Vec::new(),
            &mut 0,
            &tag,
            1,
            scope,
            &mut Vec::new(),
        );
    }

    /// §6.21/§9.3.4: alpha-rename, innermost first, the collection locals
    /// (declarators with unpacked dimensions) of every block below `depth`
    /// 0 of `stmt` that shadow a name `claims` (a module item) or a local of
    /// an enclosing block (`enclosing`). `n` numbers the renamed blocks,
    /// `tag` prefixes that number in the fresh names. In an instance's
    /// process (tag `p`) locals of every kind are renamed. A named block's
    /// locals are stored under their hierarchical name — `scope` (the
    /// instance path, or the subroutine) and the enclosing block labels
    /// (`labels`) — so `inst.blk.var` still reaches them (§23.6).
    /// True when something was renamed.
    pub(super) fn rename_nested_shadowed(
        stmt: &mut Statement,
        claims: &dyn Fn(&str) -> bool,
        enclosing: &mut Vec<String>,
        n: &mut usize,
        tag: &str,
        depth: usize,
        scope: &str,
        labels: &mut Vec<String>,
    ) -> bool {
        let unlabeled = tag.starts_with('p');
        let mut any = false;
        let mut walk =
            |s: &mut Statement, enc: &mut Vec<String>, labels: &mut Vec<String>, any: &mut bool| {
                *any |=
                    Self::rename_nested_shadowed(s, claims, enc, n, tag, depth + 1, scope, labels);
            };
        match &mut stmt.kind {
            StatementKind::SeqBlock { stmts, name } => {
                let label_mark = labels.len();
                if let Some(l) = name {
                    labels.push(l.name.clone());
                }
                let mark = enclosing.len();
                let mut mine: Vec<String> = Vec::new();
                for c in stmts.iter() {
                    if let StatementKind::VarDecl { declarators, .. } = &c.kind {
                        for d in declarators {
                            if (unlabeled || !d.dimensions.is_empty())
                                && (claims(&d.name.name)
                                    || enclosing[..mark].contains(&d.name.name))
                            {
                                mine.push(d.name.name.clone());
                            }
                            enclosing.push(d.name.name.clone());
                        }
                    }
                }
                for c in stmts.iter_mut() {
                    walk(c, enclosing, labels, &mut any);
                }
                enclosing.truncate(mark);
                let labeled = labels.len() > label_mark;
                if depth > 0 && !mine.is_empty() {
                    *n += 1;
                    let taken: std::collections::HashSet<String> = mine.into_iter().collect();
                    // §23.6: a named block's locals stay reachable by their
                    // hierarchical name: they are stored under it — the
                    // named-block path, after the instance path in an
                    // instance's process (unique per instance) — and an
                    // unnamed block's are salted.
                    let label = match &mut stmt.kind {
                        StatementKind::SeqBlock { name: Some(l), .. } if labeled => {
                            let mut path = labels.join(".");
                            if !scope.is_empty() {
                                path = format!("{}.{}", scope, path);
                            }
                            Some(std::mem::replace(&mut l.name, path))
                        }
                        _ => None,
                    };
                    if let Some(new) = super::elaborate::rename_process_shadowed_locals(
                        stmt,
                        &taken,
                        &format!("{}{}", tag, n),
                    ) {
                        *stmt = new;
                        any = true;
                    }
                    if let (Some(l), StatementKind::SeqBlock { name: Some(n), .. }) =
                        (label, &mut stmt.kind)
                    {
                        n.name = l;
                    }
                }
                labels.truncate(label_mark);
            }
            StatementKind::ParBlock { stmts, name, .. } => {
                let label_mark = labels.len();
                if let Some(l) = name {
                    labels.push(l.name.clone());
                }
                for c in stmts.iter_mut() {
                    walk(c, enclosing, labels, &mut any);
                }
                labels.truncate(label_mark);
            }
            StatementKind::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                walk(then_stmt, enclosing, labels, &mut any);
                if let Some(e) = else_stmt {
                    walk(e, enclosing, labels, &mut any);
                }
            }
            StatementKind::Case { items, .. } => {
                for it in items.iter_mut() {
                    walk(&mut it.stmt, enclosing, labels, &mut any);
                }
            }
            StatementKind::For { body, .. }
            | StatementKind::While { body, .. }
            | StatementKind::DoWhile { body, .. }
            | StatementKind::Repeat { body, .. }
            | StatementKind::Forever { body }
            | StatementKind::Foreach { body, .. } => walk(body, enclosing, labels, &mut any),
            StatementKind::TimingControl { stmt: s, .. } | StatementKind::Wait { stmt: s, .. } => {
                walk(s, enclosing, labels, &mut any)
            }
            _ => {}
        }
        any
    }

    /// Does a block declare variables directly (§6.21: it then opens a scope)?
    pub(super) fn stmts_declare_vars(stmts: &[Statement]) -> bool {
        stmts
            .iter()
            .any(|s| matches!(s.kind, StatementKind::VarDecl { .. }))
    }

    /// The statements a blocking loop or branch body splices into the process
    /// stream: an unnamed `begin…end` body's own statements, unless it
    /// declares variables. Named bodies retain their scope and disable
    /// target (§9.3, §9.6.2); declarations retain their lifetime (§6.21).
    pub(super) fn spliced_body(body: &Statement) -> Vec<Statement> {
        match &body.kind {
            StatementKind::SeqBlock { stmts, name: None } if !Self::stmts_declare_vars(stmts) => {
                stmts.clone()
            }
            _ => vec![body.clone()],
        }
    }

    /// Cached `Arc<[Statement]>` form of `spliced_body(body)`, keyed by the
    /// static AST node address of `body` so repeated loop iterations and
    /// case/foreach entries share one allocation.
    pub(super) fn spliced_body_frame(
        &mut self,
        owner: &Arc<[Statement]>,
        body: &Statement,
    ) -> Arc<[Statement]> {
        match &body.kind {
            StatementKind::SeqBlock { stmts, name: None } if !Self::stmts_declare_vars(stmts) => {
                self.blocking_cont_frame(owner, body, 8, stmts)
            }
            _ => self.blocking_cont_frame(owner, body, 8, std::slice::from_ref(body)),
        }
    }

    /// §6.21: open the scope of a block whose statements are `stmts`. Its
    /// declarations land in the enclosing call frame under their bare names
    /// (and collections in `local_dyn`), so record what they will shadow.
    /// `None` without a call frame or without declarations: nothing to undo.
    /// Allocation-free unless a declaration shadows something.
    pub(super) fn block_scope_enter(&mut self, stmts: &[Statement]) -> Option<BlockScopeSave> {
        if self.local_stack.is_empty() || !Self::stmts_declare_vars(stmts) {
            return None;
        }
        let mut sv = BlockScopeSave {
            frame_gen: self.local_gen_stack.last().copied().unwrap_or(0),
            pid_mark: self.next_pid,
            ..Default::default()
        };
        // §6.21: a block variable that is not `static` hides the
        // subroutine's same-named static until the block ends, so its reads
        // and writes must not reach the static's shared cell.
        let depth = self.static_local_syncs.len();
        if !self.static_local_names.is_empty()
            && let Some((_, syncs)) = self.static_local_syncs.last_mut()
        {
            for s in stmts {
                if let StatementKind::VarDecl {
                    lifetime,
                    declarators,
                    ..
                } = &s.kind
                    && !matches!(lifetime, Some(crate::ast::types::Lifetime::Static))
                {
                    for d in declarators {
                        let nm = &d.name.name;
                        if !self.static_local_names.contains(nm) {
                            continue;
                        }
                        if let Some(i) = syncs.iter().position(|(n, _)| n == nm) {
                            let hidden = syncs.remove(i);
                            sv.static_hidden
                                .get_or_insert_with(|| (depth, Vec::new()))
                                .1
                                .push(hidden);
                        }
                    }
                }
            }
        }
        for s in stmts {
            let StatementKind::VarDecl {
                data_type,
                lifetime,
                declarators,
            } = &s.kind
            else {
                continue;
            };
            if sv.static_mark.is_none()
                && matches!(lifetime, Some(crate::ast::types::Lifetime::Static))
            {
                sv.static_mark = self
                    .static_local_syncs
                    .last()
                    .map(|(_, v)| (self.static_local_syncs.len(), v.len()));
            }
            let plain = Self::type_is_plain_scalar(data_type);
            for d in declarators {
                let nm = d.name.name.as_str();
                if d.dimensions.is_empty() && self.is_virtual_iface_type(data_type) {
                    // §6.21 / §25.9: hiding a variable also hides its binding.
                    // Values alone cannot restore the enclosing vif after a
                    // nested block has rebound or cleared the same bare name.
                    if let Some(aliases) = self.local_iface_aliases.last_mut() {
                        sv.vif_aliases.push((nm.to_string(), aliases.remove(nm)));
                    }
                    let mut keys = vec![format!("__vif_local__{nm}")];
                    if let Some(th) = self
                        .this_stack
                        .last()
                        .copied()
                        .flatten()
                        .filter(|&h| h != 0)
                    {
                        keys.push(format!("__vif_local__{th}#{nm}"));
                    }
                    for key in keys {
                        let saved = self.signals.remove(&key);
                        sv.vif_slots.push((key, saved));
                    }
                }
                let type_changes = match self.module.var_decl_types.get(nm) {
                    Some(old) => !Self::dt_trivially_equal(old, data_type),
                    None => self.widths.contains_key(nm),
                };
                if type_changes {
                    sv.declarations.push(DeclShadow {
                        width: self.widths.get(nm).copied(),
                        signed: self.signed_signals.contains(nm),
                        real: self.real_signals.contains(nm),
                        string: self.string_signals.contains(nm),
                        meta: self.snapshot_formal_metadata(nm),
                    });
                }
                let frame = self.local_stack.last()?;
                if let Some(v) = frame.get(nm) {
                    if !sv.shadowed.iter().any(|(n, _)| n == nm) {
                        sv.shadowed.push((nm.to_string(), v.clone()));
                        sv.string_kinds
                            .push((nm.to_string(), self.task_frame_string_flag(nm)));
                    }
                }
                if plain && d.dimensions.is_empty() {
                    continue;
                }
                // An unpacked-struct local: its `<name>.` marker and leaves.
                let marker = format!("{}.", nm);
                if frame.contains_key(&marker) {
                    for (k, v) in frame.iter() {
                        if k.starts_with(&marker) && !sv.shadowed.iter().any(|(n, _)| n == k) {
                            sv.shadowed.push((k.clone(), v.clone()));
                        }
                    }
                }
                // A collection local: move the enclosing rename aside.
                if let Some(ld) = self.local_dyn.last_mut() {
                    if let Some(pos) = ld.iter().rposition(|(k, _)| k == nm) {
                        sv.dyn_shadowed.push(ld.remove(pos));
                    }
                }
            }
        }
        Some(sv)
    }

    /// §6.21: close a block scope opened by `block_scope_enter`: the block's
    /// variables go out of scope and the enclosing ones they shadowed are
    /// visible again. A `static` block variable keeps its persistent value;
    /// only the frame's view of it ends here.
    pub(super) fn block_scope_exit(&mut self, stmts: &[Statement], mut sv: BlockScopeSave) {
        if self.local_gen_stack.last().copied().unwrap_or(0) != sv.frame_gen
            || self.local_stack.is_empty()
        {
            return;
        }
        if let Some((depth, len)) = sv.static_mark {
            if self.static_local_syncs.len() == depth {
                let added = self
                    .static_local_syncs
                    .last_mut()
                    .map(|(_, v)| v.split_off(len.min(v.len())))
                    .unwrap_or_default();
                // What `sync_static_locals` would write back on return, for
                // the statics that are not live-sourced.
                for (nm, key) in added {
                    if !self.static_local_names.contains(&nm) {
                        if let Some(v) = self.local_stack.last().and_then(|f| f.get(&nm)) {
                            let v = v.clone();
                            self.static_local_vars.insert(key, v);
                        }
                    }
                }
            }
        }
        // The statics the block's variables hid are visible again.
        if let Some((depth, hidden)) = sv.static_hidden.take()
            && self.static_local_syncs.len() == depth
            && let Some((_, syncs)) = self.static_local_syncs.last_mut()
        {
            syncs.extend(hidden);
        }
        let forked = self.next_pid != sv.pid_mark;
        for s in stmts {
            let StatementKind::VarDecl {
                data_type,
                declarators,
                ..
            } = &s.kind
            else {
                continue;
            };
            let plain = Self::type_is_plain_scalar(data_type);
            for d in declarators {
                let nm = d.name.name.as_str();
                if let Some(frame) = self.local_stack.last_mut() {
                    frame.remove(nm);
                    if !(plain && d.dimensions.is_empty()) {
                        let marker = format!("{}.", nm);
                        if frame.contains_key(&marker) {
                            frame.retain(|k, _| !k.starts_with(&marker));
                        }
                    }
                }
                if plain && d.dimensions.is_empty() {
                    continue;
                }
                let gone = self.local_dyn.last_mut().and_then(|ld| {
                    ld.iter()
                        .rposition(|(k, _)| k == nm)
                        .map(|pos| ld.remove(pos))
                });
                if let Some((_, key)) = gone {
                    if !forked {
                        self.cleanup_dyn_storage(&key);
                    }
                }
            }
        }
        if let Some(frame) = self.local_stack.last_mut() {
            for (k, v) in sv.shadowed.drain(..) {
                frame.insert(k, v);
            }
        }
        if !sv.dyn_shadowed.is_empty() {
            if let Some(ld) = self.local_dyn.last_mut() {
                ld.extend(sv.dyn_shadowed);
            }
        }
        if let Some(aliases) = self.local_iface_aliases.last_mut() {
            for (name, previous) in sv.vif_aliases {
                match previous {
                    Some(bound) => {
                        aliases.insert(name, bound);
                    }
                    None => {
                        aliases.remove(&name);
                    }
                }
            }
        }
        for (key, previous) in sv.vif_slots {
            match previous {
                Some(value) => {
                    self.signals.insert(key, value);
                }
                None => {
                    self.signals.remove(&key);
                }
            }
        }
        self.restore_decl_shadows(sv.declarations);
        for (name, kind) in sv.string_kinds {
            if let Some(kind) = kind {
                self.record_frame_string_kind(&name, kind);
            } else if let Some(&g) = self.local_gen_stack.last() {
                if let Some(e) = self.task_string_formals.iter_mut().find(|e| e.fgen == g) {
                    e.strings.retain(|n| n != &name);
                    e.others.retain(|n| n != &name);
                }
            }
        }
        if sv.owns_loop_frame {
            self.pop_loop_frame();
        }
        self.restore_loop_vars(&sv.loop_vars);
    }

    /// §6.21 for a synchronous loop: open the body block's scope once for
    /// the whole loop rather than once per iteration (each iteration's
    /// declarations replace the previous one's), and close it after the
    /// loop. Only when nothing the loop evaluates between iterations
    /// (`between`, the loop variables `names`) can name a declared variable.
    pub(super) fn loop_scope_open(
        &mut self,
        body: &Statement,
        between: &[&Expression],
        names: &[String],
    ) -> Option<(BlockScopeSave, (usize, u64))> {
        let StatementKind::SeqBlock { stmts, .. } = &body.kind else {
            return None;
        };
        if self.local_stack.is_empty() || !Self::stmts_declare_vars(stmts) {
            return None;
        }
        for s in stmts {
            if let StatementKind::VarDecl { declarators, .. } = &s.kind {
                for d in declarators {
                    let nm = d.name.name.as_str();
                    if names.iter().any(|n| n == nm)
                        || between.iter().any(|e| Self::expr_may_name(e, nm))
                    {
                        return None;
                    }
                }
            }
        }
        let sv = self.block_scope_enter(stmts)?;
        let prev = std::mem::replace(
            &mut self.hoisted_scope,
            (body as *const Statement as usize, sv.frame_gen),
        );
        Some((sv, prev))
    }

    /// Close a scope `loop_scope_open` opened.
    pub(super) fn loop_scope_close(
        &mut self,
        body: &Statement,
        open: Option<(BlockScopeSave, (usize, u64))>,
    ) {
        let Some((sv, prev)) = open else {
            return;
        };
        self.hoisted_scope = prev;
        if let StatementKind::SeqBlock { stmts, .. } = &body.kind {
            self.block_scope_exit(stmts, sv);
        }
    }

    /// Could evaluating `e` read the bare name `nm`? Conservative: any
    /// expression form not listed here answers yes.
    pub(super) fn expr_may_name(e: &Expression, nm: &str) -> bool {
        match &e.kind {
            ExprKind::Number(_) | ExprKind::StringLiteral(_) => false,
            ExprKind::Ident(h) => {
                h.path.first().is_some_and(|s| {
                    s.name.name == nm || s.selects.iter().any(|x| Self::expr_may_name(x, nm))
                }) || h
                    .path
                    .iter()
                    .skip(1)
                    .any(|s| s.selects.iter().any(|x| Self::expr_may_name(x, nm)))
            }
            ExprKind::Paren(i) | ExprKind::Unary { operand: i, .. } => Self::expr_may_name(i, nm),
            ExprKind::Binary { left, right, .. } => {
                Self::expr_may_name(left, nm) || Self::expr_may_name(right, nm)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::expr_may_name(condition, nm)
                    || Self::expr_may_name(then_expr, nm)
                    || Self::expr_may_name(else_expr, nm)
            }
            ExprKind::Index { expr, index } => {
                Self::expr_may_name(expr, nm) || Self::expr_may_name(index, nm)
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                Self::expr_may_name(expr, nm)
                    || Self::expr_may_name(left, nm)
                    || Self::expr_may_name(right, nm)
            }
            ExprKind::MemberAccess { expr, .. } => Self::expr_may_name(expr, nm),
            ExprKind::Concatenation(parts) => parts.iter().any(|p| Self::expr_may_name(p, nm)),
            ExprKind::Call { func, args } => {
                Self::expr_may_name(func, nm) || args.iter().any(|a| Self::expr_may_name(a, nm))
            }
            ExprKind::SystemCall { args, .. } => args.iter().any(|a| Self::expr_may_name(a, nm)),
            _ => true,
        }
    }

    /// The trailing `Null` of a flattened scoped block's frame: close the
    /// innermost open block scope when it belongs to this frame.
    #[inline(never)]
    pub(super) fn block_scope_sentinel(&mut self, pc: &ProcCont) {
        let key = Self::frame_key(&pc.stmts);
        if self.block_scope_saves.last().map(|sv| sv.frame_key) != Some(key) {
            return;
        }
        let sv = self.block_scope_saves.pop().expect("checked above");
        let n = pc.stmts.len().saturating_sub(1);
        self.block_scope_exit(&pc.stmts[..n], sv);
    }

    /// The frame of a flattened unnamed block that opened a scope: its
    /// statements and a trailing `Null` that closes the scope (see
    /// `block_scope_sentinel`).
    pub(super) fn scoped_block_frame(
        &mut self,
        owner: &Arc<[Statement]>,
        stmt: &Statement,
        inner: &[Statement],
    ) -> Arc<[Statement]> {
        let key = (stmt as *const Statement as usize, 3u8);
        if let Some((_, f)) = self.blocking_cont_cache.get(&key) {
            return f.clone();
        }
        let mut v: Vec<Statement> = Vec::with_capacity(inner.len() + 1);
        v.extend_from_slice(inner);
        v.push(Statement::new(StatementKind::Null, stmt.span));
        let f: Arc<[Statement]> = Arc::from(v);
        self.blocking_cont_cache
            .insert(key, (Arc::clone(owner), f.clone()));
        f
    }

    /// class-perf loop-heat probe: does this statement tree contain a looping
    /// statement (`for`/`while`/`do`/`foreach`/`repeat`/`forever`)? The
    /// adaptive tier counter measures heat by *number of calls*; this is the
    /// *iterations-of-one-call* heat signal that once-called loop-heavy bodies
    /// (a UVM `run_phase` holding a 5M loop) expose. Structural only — cheap,
    /// pure traversal, no per-iteration cost, generality independent of any
    /// library.
    pub(super) fn body_has_loop(&self, stmts: &[Statement]) -> bool {
        stmts.iter().any(|s| self.stmt_has_loop(s))
    }
    /// class-perf loop-heat boost (MEMOIZATION / pass B): whether a method is
    /// a wait-free loop candidate = its body structurally contains a loop and
    /// does not block. This is the probe the maintainer flagged as a per-call
    /// cost; it is a pure function of the declaration (instance-independent),
    /// so it is memoized once per method id and never re-walked.
    pub(super) fn is_loop_candidate(
        &mut self,
        key: (u32, u32, u32),
        cname: &str,
        kind: &CompiledBodyKind,
        body: &[Statement],
    ) -> bool {
        if let Some(v) = self.compiled_method_boost_qual.get(&key) {
            return *v;
        }
        let has_loop = self.body_has_loop(body);
        let wait_free = match kind {
            CompiledBodyKind::Task(t) => !self.method_body_can_suspend(cname, &t.ports, &t.items),
            _ => !self.stmts_have_blocking(body),
        };
        let v = has_loop && wait_free;
        self.compiled_method_boost_qual.insert(key, v);
        v
    }
    /// class-perf loop-heat boost (MEASURED HEAT / pass C): record `iters`
    /// loop back-edges (iterations) executed by the innermost not-yet-compiled
    /// loop-candidate method currently on the AST interpreter. Called once per
    /// loop EXIT (never per iteration), and a no-op when the heat stack is
    /// empty (boost off), so the default path pays only the `last()` branch.
    pub(super) fn record_ast_loop_heat(&mut self, iters: u64) {
        if let Some(&key) = self.compiled_method_heat_stack.last() {
            let e = self.compiled_method_loop_heat.entry(key).or_insert(0);
            *e = e.saturating_add(iters);
        }
    }

    pub(super) fn stmt_has_loop(&self, stmt: &Statement) -> bool {
        match &stmt.kind {
            StatementKind::For { body: _, .. }
            | StatementKind::While { body: _, .. }
            | StatementKind::DoWhile { body: _, .. }
            | StatementKind::Foreach { body: _, .. }
            | StatementKind::Repeat { body: _, .. }
            | StatementKind::Forever { body: _ } => true,
            StatementKind::SeqBlock { stmts, .. } => stmts.iter().any(|s| self.stmt_has_loop(s)),
            StatementKind::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                self.stmt_has_loop(then_stmt)
                    || else_stmt.as_ref().is_some_and(|e| self.stmt_has_loop(e))
            }
            StatementKind::Case { items, .. } => {
                items.iter().any(|it| self.stmt_has_loop(&it.stmt))
            }
            StatementKind::RandCase { items } => items.iter().any(|(_, s)| self.stmt_has_loop(s)),
            StatementKind::ParBlock { stmts, .. } => stmts.iter().any(|s| self.stmt_has_loop(s)),
            _ => false,
        }
    }

    /// Check if any statements contain blocking constructs (timing, events, wait).
    pub(super) fn stmts_have_blocking(&self, stmts: &[Statement]) -> bool {
        stmts.iter().any(|s| self.stmt_is_blocking(s))
    }
    pub(super) fn stmt_is_blocking(&self, stmt: &Statement) -> bool {
        match &stmt.kind {
            StatementKind::TimingControl { .. } => true,
            StatementKind::Wait { .. } => true,
            StatementKind::WaitOrder { .. } => true,
            // §9.6.1: `wait fork` suspends until all descendants finish —
            // inside a begin-block it must route through the suspend-aware
            // runner (the plain exec arm is a no-op).
            StatementKind::WaitFork => true,
            StatementKind::RandCase { items } => {
                items.iter().any(|(_, s)| self.stmt_is_blocking(s))
            }
            // §9.4.5 intra-assignment delay / event control suspends the
            // process. Missing the event form ran `always v = @(e) x;` as a
            // plain statement: no wait, and the marker call evaluated to 0.
            StatementKind::BlockingAssign { rvalue, .. } => Self::intra_timing_suspends(rvalue),
            StatementKind::SeqBlock { stmts, .. } => stmts.iter().any(|s| self.stmt_is_blocking(s)),
            StatementKind::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                self.stmt_is_blocking(then_stmt)
                    || else_stmt.as_ref().is_some_and(|e| self.stmt_is_blocking(e))
            }
            StatementKind::Forever { body } => self.stmt_is_blocking(body),
            StatementKind::For { body, .. }
            | StatementKind::While { body, .. }
            | StatementKind::DoWhile { body, .. }
            | StatementKind::Foreach { body, .. } => self.stmt_is_blocking(body),
            StatementKind::Case { items, .. } => {
                items.iter().any(|it| self.stmt_is_blocking(&it.stmt))
            }
            // `fork...join` / `join_any` suspends the calling process until its
            // children finish, so a task containing one must be inlined (run via
            // the suspend-aware run_process_stmts) — otherwise the synchronous
            // path mishandles the join and drops the post-join continuation
            // (this is what stalls tasks that fork sequence bodies).
            // `join_none` returns immediately, so it doesn't make the enclosing
            // task blocking on its own.
            StatementKind::ParBlock { join_type, .. } => !matches!(join_type, JoinType::JoinNone),
            // A call to a blocking task. Forwarding task chains forward through
            // thin wrappers whose body is a single call; such a wrapper is
            // blocking-via-call even though it has no structural wait, so it
            // must inline for the eventual suspend point to reach the
            // suspend-aware path. Recognise the canonical blocking task names.
            StatementKind::Expr(e) => {
                // §9.7: `process_handle.await()` blocks the calling process
                // until the target terminates. A `forever { h.await(); ... }`
                // body (the sequencer's re-arbitration fork) must be detected
                // as blocking so exec_forever_sched routes it through
                // run_process_stmts, where the await is intercepted and the
                // process parks in await_waiters instead of busy-spinning.
                if Self::expr_is_proc_await(e) {
                    return true;
                }
                if self.expr_calls_dpi_task(e) {
                    return true;
                }
                // Follow user task calls into their bodies so blocking tasks
                // reached through loop/conditional bodies still use the
                // suspend-aware runner. The name whitelist remains useful for
                // built-in or canonical blocking task calls whose source
                // declaration is unavailable.
                Self::call_is_blocking_task(e) || self.callee_transitively_blocks(e)
            }
            StatementKind::Repeat { body, .. } => self.stmt_is_blocking(body),
            _ => false,
        }
    }

    /// Does this statement need a real *process* context (the ability to
    /// suspend and resume) — as opposed to merely containing a `#delay`?
    ///
    /// `stmt_is_blocking` answers the broader question "must this run through
    /// the suspend-aware runner", and says YES for a plain `#delay` too. But
    /// the synchronous `exec_statement` path DOES implement `TimingControl::
    /// Delay`: it flushes NBA, settles, advances time and runs the intervening
    /// event regions. Only genuinely suspending constructs — event controls,
    /// `wait`, `fork…join`, `forever` — cannot run there.
    ///
    /// `exec_expr_stmt` uses this instead of `stmt_is_blocking` so that a
    /// parenless enable of a delay-only task still RUNS. It previously used the
    /// broad predicate and silently DROPPED the whole call: a vendor
    /// register-file cell enabled its write task from an `always @(wclk)`
    /// block, that task reached a `#0` two levels down, and so every write was
    /// discarded — the memory read back all-X. Tasks that truly need
    /// suspension keep the old (skip) behaviour, which this does not change.
    pub(super) fn stmt_needs_process_context(&self, stmt: &Statement, depth: u32) -> bool {
        // Guard against deep/cyclic call chains; treat an over-deep chain as
        // needing a process (conservative = preserves the old skip).
        if depth > 8 {
            return true;
        }
        match &stmt.kind {
            StatementKind::TimingControl {
                control,
                stmt: inner,
            } => match control {
                // A pure delay is executable synchronously.
                TimingControl::Delay(_) => self.stmt_needs_process_context(inner, depth),
                // Event control / repeat-event genuinely suspend.
                _ => true,
            },
            StatementKind::Wait { .. }
            | StatementKind::WaitOrder { .. }
            | StatementKind::WaitFork => true,
            // `forever` in a synchronous context would spin without ever
            // yielding to the scheduler.
            StatementKind::Forever { .. } => true,
            StatementKind::ParBlock { join_type, .. } => !matches!(join_type, JoinType::JoinNone),
            // Intra-assignment delay: keep the conservative old behaviour.
            StatementKind::BlockingAssign { rvalue, .. } => {
                Self::intra_delay_marker(rvalue).is_some()
            }
            StatementKind::RandCase { items } => items
                .iter()
                .any(|(_, s)| self.stmt_needs_process_context(s, depth)),
            StatementKind::SeqBlock { stmts, .. } => stmts
                .iter()
                .any(|s| self.stmt_needs_process_context(s, depth)),
            StatementKind::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                self.stmt_needs_process_context(then_stmt, depth)
                    || else_stmt
                        .as_ref()
                        .is_some_and(|e| self.stmt_needs_process_context(e, depth))
            }
            StatementKind::For { body, .. }
            | StatementKind::While { body, .. }
            | StatementKind::DoWhile { body, .. }
            | StatementKind::Foreach { body, .. }
            | StatementKind::Repeat { body, .. } => self.stmt_needs_process_context(body, depth),
            StatementKind::Case { items, .. } => items
                .iter()
                .any(|it| self.stmt_needs_process_context(&it.stmt, depth)),
            StatementKind::Expr(e) => {
                if Self::expr_is_proc_await(e) {
                    return true;
                }
                // A call whose name matches the built-in blocking task
                // whitelist has no visible body — assume it suspends.
                if Self::call_is_blocking_task(e) {
                    return true;
                }
                // Follow user task/function calls into their bodies.
                let callee = match &e.kind {
                    ExprKind::Call { func, .. } => &**func,
                    ExprKind::Ident(_) | ExprKind::MemberAccess { .. } => e,
                    _ => return false,
                };
                let name = match &callee.kind {
                    ExprKind::Ident(h) => h.path.last().map(|s| s.name.name.as_str()),
                    ExprKind::MemberAccess { member, .. } => Some(member.name.as_str()),
                    _ => None,
                };
                let Some(n) = name else { return false };
                // Resolve against both the leaf name and the instance-scoped
                // key the elaborator bakes in (`u_inst.the_task`).
                let items = self.module.tasks.get(n).map(|t| &t.items).or_else(|| {
                    self.module
                        .tasks
                        .iter()
                        .find(|(k, _)| k.rsplit('.').next() == Some(n))
                        .map(|(_, t)| &t.items)
                });
                match items {
                    Some(its) => its
                        .iter()
                        .any(|s| self.stmt_needs_process_context(s, depth + 1)),
                    None => false,
                }
            }
            _ => false,
        }
    }

    /// Transitive blocking detection: true if the call `expr` targets a user
    /// subroutine whose body eventually reaches a blocking construct.
    pub(super) fn callee_transitively_blocks(&self, expr: &Expression) -> bool {
        // A subroutine call in statement position can parse as Call{func},
        // a bare Ident (`task_name;` with no args), or a MemberAccess
        // (`obj.method;`). Extract the leaf callee name from any of these.
        let callee = match &expr.kind {
            ExprKind::Call { func, .. } => &**func,
            other_kind @ (ExprKind::Ident(_) | ExprKind::MemberAccess { .. }) => {
                let _ = other_kind;
                expr
            }
            _ => return false,
        };
        let name = match &callee.kind {
            ExprKind::Ident(h) => h.path.last().map(|s| s.name.name.as_str()),
            ExprKind::MemberAccess { member, .. } => Some(member.name.as_str()),
            _ => None,
        };
        match name {
            Some(n) => self.subroutine_name_blocks(n),
            None => false,
        }
    }

    /// Memoized: does the subroutine named `name` (free task/function, or any
    /// same-named class method — over-approximated by name) transitively reach a
    /// blocking construct? A tentative `false` is cached before recursing so
    /// cyclic/recursive calls terminate (a pure cycle with no real blocker is
    /// correctly non-blocking).
    pub(super) fn subroutine_name_blocks(&self, name: &str) -> bool {
        // A call cycle (`a -> b -> a`) must not leave a member cached as
        // non-blocking on the strength of the cycle's own assumption (see
        // `CycleMemo`).
        self.tb_cache
            .get(name, || self.subroutine_name_blocks_uncached(name))
    }

    pub(super) fn subroutine_name_blocks_uncached(&self, name: &str) -> bool {
        let mut blocks = false;
        if let Some(t) = self.module.tasks.get(name) {
            blocks = self.stmts_have_blocking(&t.items);
        }
        if !blocks {
            if let Some(f) = self.module.functions.get(name) {
                blocks = self.stmts_have_blocking(&f.items);
            }
        }
        if !blocks {
            // Hierarchical / interface tasks are keyed by their FULL inlined
            // path ("u_if.sample_one"), so the leaf-name lookup above misses
            // them — a monitor BFM task with `@(negedge clk)` was classified
            // non-blocking, executed synchronously, and its event control
            // aborted the body instead of suspending (the caller's forever
            // loop then spun at t=0). Over-approximate by leaf suffix: if ANY
            // task keyed `*.name` blocks, treat the call as blocking (the
            // suspend-aware path is correct for non-blocking bodies too,
            // merely slower).
            let dotted = format!(".{}", name);
            for (key, t) in self.module.tasks.iter() {
                if key.ends_with(dotted.as_str()) && self.stmts_have_blocking(&t.items) {
                    blocks = true;
                    break;
                }
            }
        }
        if !blocks {
            // Method-by-name across all classes (the receiver's dynamic type
            // isn't known here, so over-approximate: blocking if ANY class's
            // same-named method body blocks).
            'outer: for cls in self.module.classes.values() {
                if let Some(m) = cls.methods.get(name) {
                    let items = match &m.kind {
                        crate::ast::decl::ClassMethodKind::Function(f)
                        | crate::ast::decl::ClassMethodKind::Extern(f)
                        | crate::ast::decl::ClassMethodKind::PureVirtual(f) => &f.items,
                        crate::ast::decl::ClassMethodKind::Task(t) => &t.items,
                    };
                    if self.stmts_have_blocking(items) {
                        blocks = true;
                        break 'outer;
                    }
                }
            }
        }
        blocks
    }

    /// True if `expr` is a call to a built-in or canonical blocking task/method
    /// (e.g. mailbox `put`/`get`/`peek`, TLM pull, sequencer arbitration, or
    /// sequence control). Used to inline thin forwarding wrappers so their
    /// nested waits suspend.
    pub(super) fn call_is_blocking_task(expr: &Expression) -> bool {
        let func = match &expr.kind {
            ExprKind::Call { func, .. } => func,
            _ => return false,
        };
        let name = match &func.kind {
            ExprKind::Ident(h) => h.path.last().map(|s| s.name.name.as_str()),
            ExprKind::MemberAccess { member, .. } => Some(member.name.as_str()),
            _ => None,
        };
        matches!(
            name,
            Some(
                "get_next_item"
                    | "try_next_item"
                    | "peek"
                    | "get"
                    // §15.4.1: `put` blocks on a FULL bounded mailbox, so a
                    // block containing one must flatten into the suspend-aware
                    // runner. A non-mailbox / unbounded `put` simply falls
                    // through the park handler (its handle has no bound).
                    | "put"
                    | "wait_for_sequences"
                    | "get_response"
                    | "finish_item"
                    | "start_item"
                    | "wait_for_grant"
                    | "wait_for_item_done"
                    | "body"
                    | "start"
                    | "m_select_sequence"
                    | "m_wait_for_available_sequence"
                    | "m_wait_for_arbitration_completed"
                    | "m_wait_arb_not_equal"
                    | "m_safe_select_item"
                    | "uvm_wait_for_nba_region"
                    | "execute_item"
                    | "send_request"
                    | "do_get"
                    | "do_peek"
                    | "transport"
                    | "wait_for_state"
            )
        )
    }

    /// For a statement-position method call (`obj.task(...)`, or `obj.a.task(...)`)
    /// whose receiver handle resolves to null (0), the call cannot suspend: a
    /// null handler runs no body, so weaving the loop through the suspend-aware
    /// runner imprints one recursion per iteration and livelocks (`forever {
    /// null_handle.wait_trigger(); }` spins at 100% CPU instead of being bounded).
    /// The reference simulator treats dereferencing `this` of a null handle as a
    /// runtime fatal and aborts; we mirror that with an explicit error plus
    /// terminate (not a stall), so the run stops instead of burning the CPU. A
    /// null receiver on a mailbox `get`/`put`/`peek` is NOT handled here —
    /// that has its own dedicated null-mailbox diagnostic.
    pub(super) fn blocking_call_on_null_receiver(&mut self, s: &Statement) -> bool {
        let StatementKind::Expr(expr) = &s.kind else {
            return false;
        };
        let ExprKind::Call { func, .. } = &expr.kind else {
            return false;
        };
        // §25.5.4/§25.9: a task of an interface or module INSTANCE, reached by
        // hierarchical path or through a virtual interface, has no class
        // receiver. The receiver name holds no object handle (a vif property
        // reads 0), so the check below took `vif.t()` in a `forever` for a
        // null dereference and silently retired the loop.
        if self.resolve_hier_task_target(func).is_some() {
            return false;
        }
        let rh = match &func.kind {
            // `w.m(...)` / `w.a.m(...)` — flattened multi-segment Ident.
            ExprKind::Ident(h) if h.path.len() >= 2 => {
                let head = hier_prefix(h);
                let recv = Expression::new(ExprKind::Ident(head), expr.span);
                self.eval_handle_expr(&recv)
            }
            // `w.m(...)` — MemberAccess form.
            ExprKind::MemberAccess { expr: recv, .. } => self.eval_handle_expr(recv),
            _ => None,
        };
        if rh != Some(0) {
            return false;
        }
        let what = match &func.kind {
            ExprKind::Ident(h) => h
                .path
                .iter()
                .map(|s| s.name.name.as_str())
                .collect::<Vec<_>>()
                .join("."),
            ExprKind::MemberAccess { expr, member, .. } => format!(
                "{}.{}",
                self.span_source_snippet_in(expr.span, None)
                    .unwrap_or_else(|| "<null>".to_string()),
                member.name
            ),
            _ => "<call>".to_string(),
        };
        // Mirror the property-deref path (§8.4): a null `this` deref at the TOP
        // level (module/TB process body) is the user's bug — fatal, like the
        // reference simulator aborting. But inside a class method (deep UVM
        // library code, an emulation gap can leave a handle null) report a
        // corrected diagnostic and stop just this forever process instead of
        // killing the whole UVM run.
        let in_method = self.this_stack.last().copied().flatten().is_some()
            || self.class_context_stack.last().cloned().flatten().is_some()
            || !self.local_stack.is_empty();
        if !in_method {
            let m = format!(
                "[xezim][error] null receiver at time {} — task/function call \
                 `{}` on a null class handle. The reference simulator dereferences \
                 'this' of null and aborts; a blocking call here cannot suspend, so a \
                 `forever` around it would spin forever.",
                self.time, what
            );
            self.record_output(m.clone());
            self.stdout_writeln(&m);
            self.abort_run();
            return true;
        }
        true
    }

    /// Is `s` a (potentially blocking) mailbox `get`/`peek` call — `mb.get(v)`
    /// or `mb.peek(v)` (either MemberAccess or flattened hier-Ident form)? Such
    /// a call blocks when the mailbox is empty; inside a `forever` it must route
    /// through the suspend-aware runner (which parks on an empty box) instead of
    /// running synchronously and delta-spinning the loop.
    pub(super) fn stmt_is_mailbox_get(s: &Statement) -> bool {
        if let StatementKind::Expr(e) = &s.kind {
            if let ExprKind::Call { func, args } = &e.kind {
                if args.is_empty() {
                    return false;
                }
                let m = match &func.kind {
                    ExprKind::MemberAccess { member, .. } => member.name.as_str(),
                    ExprKind::Ident(h) if h.path.len() >= 2 => {
                        h.path.last().unwrap().name.name.as_str()
                    }
                    _ => "",
                };
                return m == "get" || m == "peek";
            }
        }
        false
    }

    /// `after` used to be a flat slice of the CURRENT frame's remainder, so
    /// everything behind that frame in the continuation chain was dropped:
    /// a `forever` nested inside a `begin...end` lost every statement after
    /// the enclosing block (its `$finish` included) the moment it broke out,
    /// which presented as a hang. Take the chain and splice with
    /// `ProcCont::pushed`, exactly like the other suspend-aware arms.
    /// The continuation a forever loop parks with at statement `i` of its
    /// body: `[first, rest of the body, ForeverTail { body }]`, built once
    /// per (body, i, site) and shared afterwards. `pc.stmts` — the frame
    /// that owns `body` — is kept alive by the entry so the address key
    /// cannot be recycled by another allocation. The ForeverTail inside the
    /// shared frame is what the next iteration keys on, so from the second
    /// wake-up on every iteration of every loop is a lookup.
    /// Shared continuation frame for an immutable statement list entered
    /// from the process path (a blocking `begin/end` or a blocking chosen
    /// `if` branch). Keyed per process like `forever_cont_cache`: sibling
    /// instances share spans but never a pid.
    /// §9.6.2: the frame of a flattened NAMED block — its statements and a
    /// trailing `Null`, so the frame is never dropped from a continuation
    /// chain as exhausted while the block is still running (`ProcCont::pushed`
    /// elides spent frames) — registered so a `disable` can find where the
    /// block ends.
    pub(super) fn named_block_frame(
        &mut self,
        owner: &Arc<[Statement]>,
        stmt: &Statement,
        inner: &[Statement],
        name: &str,
    ) -> Arc<[Statement]> {
        let key = (stmt as *const Statement as usize, 2u8);
        if let Some((_, f)) = self.blocking_cont_cache.get(&key) {
            return f.clone();
        }
        let mut v: Vec<Statement> = Vec::with_capacity(inner.len() + 1);
        v.extend_from_slice(inner);
        v.push(Statement::new(StatementKind::Null, stmt.span));
        let f: Arc<[Statement]> = Arc::from(v);
        self.named_block_frames
            .insert(Self::frame_key(&f), name.to_string());
        self.blocking_cont_cache
            .insert(key, (Arc::clone(owner), f.clone()));
        f
    }

    pub(super) fn blocking_cont_frame(
        &mut self,
        owner: &Arc<[Statement]>,
        node: &Statement,
        kind: u8,
        stmts: &[Statement],
    ) -> Arc<[Statement]> {
        let key = (node as *const Statement as usize, kind);
        if let Some((_, f)) = self.blocking_cont_cache.get(&key) {
            return f.clone();
        }
        let f: Arc<[Statement]> = Arc::from(stmts.to_vec());
        self.blocking_cont_cache
            .insert(key, (Arc::clone(owner), f.clone()));
        f
    }

    pub(super) fn fork_child_frame(&mut self, s: &Statement) -> Arc<[Statement]> {
        let key = (s as *const Statement as usize, 5u8);
        if let Some((_, f)) = self.blocking_cont_cache.get(&key)
            && f.first().is_some_and(|st| st.span == s.span)
        {
            return f.clone();
        }
        let f: Arc<[Statement]> = Arc::from(std::slice::from_ref(s));
        self.blocking_cont_cache
            .insert(key, (Arc::clone(&f), f.clone()));
        f
    }

    /// A class task's body followed by its `ScopePop` sentinel (spanned at
    /// the call), shared per (task, call site) instead of cloned per call.
    /// The entry keeps the method alive, so its address cannot be recycled.
    pub(super) fn class_task_body_frame(
        &mut self,
        tm: &Arc<crate::ast::decl::ClassMethod>,
        call_span: crate::ast::Span,
    ) -> Arc<[Statement]> {
        let key = (Arc::as_ptr(tm) as usize, call_span.start, call_span.end);
        if let Some((_, f)) = self.class_task_frame_cache.get(&key) {
            return f.clone();
        }
        let crate::ast::decl::ClassMethodKind::Task(td) = &tm.kind else {
            unreachable!("class_task_body_frame takes tasks only")
        };
        let mut cont: Vec<Statement> = Vec::with_capacity(td.items.len() + 1);
        cont.extend_from_slice(&td.items);
        cont.push(Statement::new(StatementKind::ScopePop, call_span));
        let f: Arc<[Statement]> = Arc::from(cont);
        self.class_task_frame_cache
            .insert(key, (tm.clone(), f.clone()));
        f
    }

    pub(super) fn forever_cont(
        &mut self,
        pid: usize,
        body: &Statement,
        body_stmts: &[Statement],
        i: usize,
        kind: u8,
        first: &Statement,
    ) -> Arc<[Statement]> {
        let key = (pid, body.span.start, body.span.end, i, kind);
        if let Some(cont) = self.forever_cont_cache.get(&key) {
            return cont.clone();
        }
        let mut cont: Vec<Statement> = Vec::with_capacity(body_stmts.len() - i + 1);
        cont.push(first.clone());
        cont.extend_from_slice(&body_stmts[i + 1..]);
        cont.push(Statement::new(
            StatementKind::ForeverTail {
                body: Box::new(body.clone()),
            },
            body.span,
        ));
        let cont: Arc<[Statement]> = Arc::from(cont);
        self.forever_cont_cache.insert(key, cont.clone());
        cont
    }

    pub(super) fn exec_forever_sched(
        &mut self,
        pid: usize,
        body: &Statement,
        pc: &ProcCont,
        idx: usize,
    ) {
        let resume_at = pc.start + idx + 1;
        let body_stmts: &[Statement] = match &body.kind {
            StatementKind::SeqBlock { stmts, .. } => stmts.as_slice(),
            _ => std::slice::from_ref(body),
        };
        for (i, s) in body_stmts.iter().enumerate() {
            if self.finished {
                return;
            }
            if let StatementKind::TimingControl {
                control,
                stmt: tbody,
            } = &s.kind
            {
                match control {
                    TimingControl::Delay(d) => {
                        let delay = self.eval_delay_ticks(d);
                        // `forever #(expr) sig=~sig;` variable-period clock:
                        // when `expr` evaluates to 0 at the very start of time 0
                        // its period signal (a real cont-assign like `assign
                        // xbOH = dTx*d0j;`) has not settled from the initial
                        // blocks yet — this clock process (a low pid) ran first.
                        // Toggling now drops a spurious t=0 edge and INVERTS the
                        // clock phase for the whole run. Restart the loop so
                        // `#(expr)` RE-EVALUATES after the time-0 active+NBA+
                        // settle; one-shot per pid so a genuine zero-period loop
                        // still spins. Only for the pure `#(expr) …` head (i==0)
                        // and a non-literal delay (literal `#0` untouched).
                        if i == 0
                            && delay == 0
                            && self.time == 0
                            && !matches!(d.kind, ExprKind::Number(_))
                            && self.t0_delay_deferred.insert(pid)
                        {
                            let restart = vec![Statement::new(
                                StatementKind::Forever {
                                    body: Box::new(body.clone()),
                                },
                                body.span,
                            )];
                            self.inactive_queue
                                .push((pid, pc.pushed(restart, resume_at)));
                            return;
                        }
                        let cont = self.forever_cont(pid, body, body_stmts, i, 0, tbody);
                        if delay == 0 {
                            // `forever begin ... #0 ... end` — same §4.4.2.3
                            // Inactive-region parking as the run_process_stmts
                            // Delay handler above: resume only after this
                            // tick's NBA region (commercial consensus).
                            self.inactive_queue
                                .push((pid, pc.pushed_frame(cont, resume_at)));
                        } else {
                            self.event_queue.schedule(
                                self.time + delay,
                                pid,
                                pc.pushed_frame(cont, resume_at),
                            );
                        }
                        return;
                    }
                    TimingControl::Event(event) => {
                        let key = (pid, s.span.start as u32);
                        let is_star = matches!(event, EventControl::Star | EventControl::ParenStar);
                        let cached = self.forever_sens_cache.get(&key).cloned();
                        let (resolved, is_clk_ev) = match cached {
                            Some(c) => c,
                            None => {
                                let sens = if is_star {
                                    self.star_sens_from_body(tbody)
                                } else {
                                    self.event_to_sens(event)
                                };
                                let is_clk_ev = self.is_clocking_event(event);
                                let resolved = self.resolve_sens_ids(&sens);
                                if !sens.is_empty() && resolved.len() == sens.len() {
                                    self.forever_sens_cache
                                        .insert(key, (resolved.clone(), is_clk_ev));
                                }
                                if sens.is_empty() {
                                    (Vec::new(), is_clk_ev)
                                } else {
                                    (resolved, is_clk_ev)
                                }
                            }
                        };
                        // §9.4.2.2: an inferred control with no inputs can
                        // never wake. Do not fall through to synchronous exec.
                        if is_star && resolved.is_empty() {
                            return;
                        }
                        if !resolved.is_empty() {
                            let cont = self.forever_cont(pid, body, body_stmts, i, 1, tbody);
                            {
                                let w = self.make_event_waiter_resolved(
                                    pid,
                                    resolved,
                                    pc.pushed_frame(cont, resume_at),
                                    is_clk_ev,
                                );
                                self.event_waiters.push(w);
                            }
                            return;
                        }
                    }
                }
            }
            // A non-TimingControl body statement that blocks (a `wait(...)`,
            // `repeat(N) @event`, `if/while` with an inner wait, or a blocking
            // method call) the top-level scan above can't suspend on. Route the
            // remainder of this iteration + a Forever continuation through
            // run_process_stmts so its waits suspend instead of spinning
            // synchronously. (Top-level `#delay`/`@event` — e.g. `always #5 clk`
            // clock generators — are handled above and never reach here.)
            if !matches!(&s.kind, StatementKind::TimingControl { .. })
                && (self.stmt_is_blocking(s) || Self::stmt_is_mailbox_get(s))
            {
                // A blocking task/function call on a NULL class-handle receiver
                // cannot suspend and is a runtime error in the reference
                // simulator (it dereferences `this` of null and aborts). The
                // helper has already emitted its diagnostic and terminated (the
                // whole run at top level, or just this forever process inside
                // deep library method code). Don't weave the loop through
                // recursion (which spins on each iteration) or let it fall to
                // the plain stall report.
                if !Self::stmt_is_mailbox_get(s) && self.blocking_call_on_null_receiver(s) {
                    return;
                }
                let cont = self.forever_cont(pid, body, body_stmts, i, 2, s);

                // The statement is only POTENTIALLY blocking: `wait (cond)` with
                // cond already true runs straight through, reaches the Forever
                // continuation, and lands back here — one recursion per loop
                // iteration. Unbounded, that overflows the Rust stack (an abort,
                // with no hint of which loop did it) instead of presenting as
                // what it is: a zero-delay livelock. Past a small depth, hand the
                // continuation to the scheduler as a delta at the current time.
                // The loop then spins through the event loop, where it is bounded,
                // and the stall detector can see it and name it.
                let contp = pc.pushed_frame(cont, resume_at);
                if self.forever_depth >= Self::FOREVER_RECURSION_LIMIT {
                    self.event_queue.schedule(self.time, pid, contp);
                    return;
                }
                self.forever_depth += 1;
                self.run_process_stmts(pid, &contp);
                self.forever_depth -= 1;
                return;
            }
            self.exec_statement(s);
        }
        // No delay/event in the forever body: the loop can never yield, so
        // running it is a zero-delay livelock (`always fb = ~fb;`). The old
        // code silently STOPPED after 10k iterations and let the sim continue
        // with whatever state the truncated loop left — silent wrongness.
        // Run up to the stall limit, then report with full attribution.
        let cap = if self.stall_limit > 0 {
            self.stall_limit
        } else {
            10_000
        };
        let mut safety: u64 = 0;
        let mut broke = false;
        while !self.finished && safety < cap {
            safety += 1;
            if safety & LOOP_INTERRUPT_POLL_MASK == 0 && loop_stop_requested() {
                self.take_interrupt();
                break;
            }
            for s in body_stmts {
                self.exec_statement(s);
                // §12.7.2: `break` (and a `return` out of the enclosing
                // task/function, which sets the same flag) LEAVES the loop —
                // a timing-free `forever` with a break in it terminates and is
                // not a livelock. Ignoring the flag here ran the body to the
                // stall cap and then reported a false stall, killing the run
                // before the process could reach its own `$finish`.
                if self.break_flag {
                    broke = true;
                    break;
                }
                // §12.7.2 `continue` ends only THIS iteration.
                if self.continue_flag {
                    self.continue_flag = false;
                    break;
                }
            }
            if broke {
                break;
            }
        }
        if broke {
            // The loop is done; the statements AFTER it still have to run.
            self.break_flag = false;
            let rest = pc.resume_at(resume_at);
            if !rest.is_exhausted() {
                self.run_process_stmts(pid, &rest);
            }
            return;
        }
        if !self.finished && safety >= cap {
            eprintln!(
                "[xezim][error] simulation STALLED at time {} — a `forever`/`always` body with no timing control ran {} times.",
                self.time, safety
            );
            let id = self.stall_pid_identity(pid);
            eprintln!("{} — no `#delay`/`@event`/`wait` anywhere in its body", id);
            if let Some(src) = self.stall_pid_source(pid) {
                eprintln!("                 its source: {}", src);
            }
            eprintln!(
                "               Simulated time can never advance past this loop. Add a timing control,"
            );
            eprintln!("               or raise/disable the check with XEZIM_STALL_LIMIT=<n>.");
            self.abort_run();
        }
    }

    /// Resolve NBA target at schedule time to capture array indices/part-selects
    /// §10.4.2: the index/range expressions of a non-blocking assignment's
    /// TARGET are evaluated when the assignment is SCHEDULED, not when it is
    /// applied. An lvalue that `resolve_nba_target` cannot reduce to a signal id
    /// — any bit- or part-select — is stored as an expression and re-evaluated
    /// in the NBA region, by which time a `for` loop's variable is long gone:
    /// `for (int j…) q[j] <= …;` re-read `j` as 0/x and the write landed on the
    /// wrong bit or nowhere at all. Substituting the evaluated indices as
    /// literals freezes them at the correct moment, and also fixes the same
    /// staleness for a signal-valued index (`mem[addr] <= d;` where `addr`
    /// changes before the region runs).
    /// `$`'s value for an index whose base is `base`: the last valid index of
    /// that collection (§11.4.12). None when the base is not an indexable
    /// collection, in which case there is no bound to install.
    pub(super) fn dollar_bound_for_base(&mut self, base: &Expression) -> Option<i64> {
        let n = match &base.kind {
            ExprKind::Ident(h) => Some(self.resolve_hier_name(h)),
            _ => self.flat_member_name(base).map(std::borrow::Cow::Owned),
        }?;
        if self.module.dynamic_arrays.contains(&*n) {
            Some(self.get_queue_size(&n) as i64 - 1)
        } else {
            self.module.arrays.get(&*n).map(|&(_, hi, _)| hi)
        }
    }

    /// Does any INDEX or RANGE bound inside this lvalue mention `$`? Scans the
    /// whole lvalue tree, unlike `expr_contains_dollar`, which only looks at
    /// one index expression.
    pub(super) fn lvalue_mentions_dollar(e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Index { expr, index } => {
                Self::expr_contains_dollar(index) || Self::lvalue_mentions_dollar(expr)
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                Self::expr_contains_dollar(left)
                    || Self::expr_contains_dollar(right)
                    || Self::lvalue_mentions_dollar(expr)
            }
            ExprKind::Paren(i) => Self::lvalue_mentions_dollar(i),
            ExprKind::MemberAccess { expr, .. } => Self::lvalue_mentions_dollar(expr),
            _ => false,
        }
    }

    /// Rewrite every `$`-bearing index/range bound in an lvalue into a literal,
    /// each evaluated against ITS OWN base's bound. Returns a clone; only
    /// called when `lvalue_mentions_dollar` says there is work to do.
    pub(super) fn resolve_dollar_in_lvalue(&mut self, e: &Expression) -> Expression {
        let lit = |v: i64, span| {
            Expression::new(
                ExprKind::Number(NumberLiteral::Integer {
                    size: Some(64),
                    signed: true,
                    base: NumberBase::Decimal,
                    value: v.to_string(),
                    cached_val: Cell::new(Some((v as u64, 0u64, 64u32))),
                }),
                span,
            )
        };
        match &e.kind {
            ExprKind::Index { expr, index } => {
                let base = self.resolve_dollar_in_lvalue(expr);
                if !Self::expr_contains_dollar(index) {
                    return Expression::new(
                        ExprKind::Index {
                            expr: Box::new(base),
                            index: index.clone(),
                        },
                        e.span,
                    );
                }
                // The bound belongs to the base being indexed here, which is
                // why this recurses rather than pushing one bound for the
                // whole tree: `mem[q[$]]` takes q's bound, not mem's.
                let idx = match self.dollar_bound_for_base(&base) {
                    Some(b) => {
                        self.dollar_bound.push(b);
                        let v = self.eval_expr(index).to_i64().unwrap_or(0);
                        self.dollar_bound.pop();
                        lit(v, index.span)
                    }
                    None => (**index).clone(),
                };
                Expression::new(
                    ExprKind::Index {
                        expr: Box::new(base),
                        index: Box::new(idx),
                    },
                    e.span,
                )
            }
            ExprKind::RangeSelect {
                expr,
                kind,
                left,
                right,
            } => {
                let base = self.resolve_dollar_in_lvalue(expr);
                // A range's `$` refers to the base being sliced.
                let b = self.dollar_bound_for_base(&base);
                let mut ev = |x: &Expression| -> Box<Expression> {
                    if !Self::expr_contains_dollar(x) {
                        return Box::new(x.clone());
                    }
                    match b {
                        Some(bb) => {
                            self.dollar_bound.push(bb);
                            let v = self.eval_expr(x).to_i64().unwrap_or(0);
                            self.dollar_bound.pop();
                            Box::new(lit(v, x.span))
                        }
                        None => Box::new(x.clone()),
                    }
                };
                let l = ev(left);
                let r = ev(right);
                Expression::new(
                    ExprKind::RangeSelect {
                        expr: Box::new(base),
                        kind: *kind,
                        left: l,
                        right: r,
                    },
                    e.span,
                )
            }
            _ => e.clone(),
        }
    }

    /// Does this expression mention `$` (§11.4.12 — the last index of the
    /// collection being indexed)? Used to decide whether an lvalue index has to
    /// be evaluated with a `dollar_bound` installed.
    pub(super) fn expr_contains_dollar(e: &Expression) -> bool {
        match &e.kind {
            ExprKind::Dollar => true,
            ExprKind::Paren(i) => Self::expr_contains_dollar(i),
            ExprKind::Unary { operand, .. } => Self::expr_contains_dollar(operand),
            ExprKind::Binary { left, right, .. } => {
                Self::expr_contains_dollar(left) || Self::expr_contains_dollar(right)
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::expr_contains_dollar(condition)
                    || Self::expr_contains_dollar(then_expr)
                    || Self::expr_contains_dollar(else_expr)
            }
            _ => false,
        }
    }

    pub(super) fn freeze_lvalue_indices(&mut self, e: &Expression) -> Expression {
        let lit = |v: i64, span| {
            Expression::new(
                ExprKind::Number(NumberLiteral::Integer {
                    size: Some(64),
                    signed: true,
                    base: NumberBase::Decimal,
                    value: v.to_string(),
                    cached_val: Cell::new(Some((v as u64, 0u64, 64u32))),
                }),
                span,
            )
        };
        // A STRING key (`aa["name"] <= v`, or a string variable) must not be
        // folded into a decimal literal — the associative key would change.
        // Same for wide (>64-bit) keys, which are string-keyed in disguise
        // (`pool#(string,…)`). Those indices keep their expression; §10.4.2
        // staleness only bites loop/temp variables, which are integral.
        let freezable = |sim: &mut Self, idx: &Expression| -> Option<i64> {
            match &idx.kind {
                ExprKind::StringLiteral(_) => return None,
                ExprKind::Ident(h) => {
                    let n = sim.resolve_hier_name(h);
                    if sim.string_signals.contains(&*n) {
                        return None;
                    }
                }
                _ => {}
            }
            let v = sim.eval_expr(idx);
            if v.is_real || v.width > 64 {
                return None;
            }
            // An x/z bound must NOT freeze into a literal: `to_i64` masks the
            // unknown bits away, so `i = 'hx; x[i +: 2] <= 2'b11` froze to
            // `x[0 +: 2]` and wrote the low bits. Leaving the expression alone
            // lets the apply-time path see the x and discard the write (§11.5.1).
            if v.has_xz() {
                return None;
            }
            v.to_i64()
        };
        match &e.kind {
            ExprKind::Index { expr, index } => {
                let base = self.freeze_lvalue_indices(expr);
                // Same `$` bound the apply-time path installs, so a frozen
                // `q[$]` literal is the queue's last index and not u64::MAX.
                let qbound = if Self::expr_contains_dollar(index) {
                    self.dollar_bound_for_base(expr)
                } else {
                    None
                };
                if let Some(b) = qbound {
                    self.dollar_bound.push(b);
                }
                let frozen_idx = freezable(self, index);
                if qbound.is_some() {
                    self.dollar_bound.pop();
                }
                let Some(iv) = frozen_idx else {
                    return Expression::new(
                        ExprKind::Index {
                            expr: Box::new(base),
                            index: index.clone(),
                        },
                        e.span,
                    );
                };
                Expression::new(
                    ExprKind::Index {
                        expr: Box::new(base),
                        index: Box::new(lit(iv, index.span)),
                    },
                    e.span,
                )
            }
            ExprKind::RangeSelect {
                expr,
                kind,
                left,
                right,
            } => {
                let base = self.freeze_lvalue_indices(expr);
                let (Some(lv), Some(rv)) = (freezable(self, left), freezable(self, right)) else {
                    return Expression::new(
                        ExprKind::RangeSelect {
                            expr: Box::new(base),
                            kind: *kind,
                            left: left.clone(),
                            right: right.clone(),
                        },
                        e.span,
                    );
                };
                Expression::new(
                    ExprKind::RangeSelect {
                        expr: Box::new(base),
                        kind: *kind,
                        left: Box::new(lit(lv, left.span)),
                        right: Box::new(lit(rv, right.span)),
                    },
                    e.span,
                )
            }
            // §10.4.2: a MEMBER of an indexed element (`arr[t].hi <= v`) needs
            // its element index captured NOW too — the catch-all cloned the
            // lvalue unfrozen, so a loop-variable index was stale (or gone) by
            // the NBA region and every such write was silently dropped.
            ExprKind::MemberAccess { expr, member } => Expression::new(
                ExprKind::MemberAccess {
                    expr: Box::new(self.freeze_lvalue_indices(expr)),
                    member: member.clone(),
                },
                e.span,
            ),
            _ => e.clone(),
        }
    }

    /// True when an NBA's target is an ASSOCIATIVE-array element (module-level
    /// `aa[k]`, a class member `m[k]` / `obj.m[k]`, or a static collection).
    ///
    /// Those lvalues have no signal id, so the NBA rode the expression queue —
    /// where the value was first cut to `infer_lhs_width`, which knows nothing
    /// about an assoc element and answered 1. Every `aa[key] <= v` therefore
    /// committed the LOW BIT of v (sign-extended: 11 arrived as -1), while the
    /// blocking form stored v at its own width. The queue now skips the resize
    /// for these targets, restoring blocking/NBA parity.
    pub(super) fn nba_target_is_assoc_elem(&mut self, lvalue: &Expression) -> bool {
        let ExprKind::Index { expr: base, .. } = &lvalue.kind else {
            return false;
        };
        if self.expr_assoc_name(base).is_some() {
            return true;
        }
        if let ExprKind::Ident(h) = &base.kind {
            let n = self.resolve_hier_name(h);
            if self.is_associative_array(&n) {
                return true;
            }
        }
        false
    }

    /// LRM §25.8 — resolve `vif.member` (or `this.vif.member`) to the FLAT
    /// name of the bound interface signal, `"<iface_instance>.<member>"`.
    ///
    /// `resolve_nba_target` asks the same question but only ever returns a
    /// registered signal id, which is no use to a caller that needs the NAME
    /// — to look up the member's packed dimensions, say. The binding lives
    /// either in `local_iface_aliases` (a vif passed as a task formal) or in
    /// `virtual_iface_bindings` keyed by the current `this`; both lookups are
    /// the ones that arm already does.
    pub(super) fn vif_member_flat_name(&self, expr: &Expression) -> Option<String> {
        let ExprKind::MemberAccess { expr: base, member } = &expr.kind else {
            return None;
        };
        let prop = match &base.kind {
            ExprKind::Ident(hier) if hier.path.len() == 1 => hier.path[0].name.name.as_str(),
            ExprKind::MemberAccess {
                expr: inner,
                member: vprop,
            } if matches!(&inner.kind, ExprKind::This) => vprop.name.as_str(),
            _ => return None,
        };
        if let Some(bound) = self
            .local_iface_aliases
            .last()
            .and_then(|frame| frame.get(prop))
        {
            return Some(format!("{}.{}", bound, member.name));
        }
        let this_h = self.this_stack.last().copied().flatten()?;
        let cls_name = self
            .heap
            .get(this_h)
            .and_then(|o| o.as_ref())
            .map(|i| i.class_name.as_str())?;
        if self.class_vif_decl(cls_name, prop).is_none() {
            return None;
        }
        let (bound_name, _modport) = self
            .virtual_iface_bindings
            .get(&(this_h, prop.to_string()))?;
        Some(format!("{}.{}", bound_name, member.name))
    }

    /// §23.9: a local or instance property wins over a same-named module
    /// signal for selected writes, just as it does for whole-variable writes.
    pub(super) fn bare_selected_storage_shadows_signal(&self, h: &HierarchicalIdentifier) -> bool {
        if h.root.is_some() || h.path.len() != 1 || !h.path[0].selects.is_empty() {
            return false;
        }
        let name = &h.path[0].name.name;
        self.local_stack
            .last()
            .is_some_and(|f| f.contains_key(name))
            || self
                .this_stack
                .last()
                .copied()
                .flatten()
                .and_then(|h| self.heap.get(h))
                .and_then(|o| o.as_ref())
                .is_some_and(|o| o.properties.contains_key(name))
    }

    pub(super) fn resolve_nba_target(&mut self, lhs: &Expression) -> Option<usize> {
        match &lhs.kind {
            ExprKind::Ident(hier) => {
                if self.bare_selected_storage_shadows_signal(hier) {
                    return None;
                }
                let is_ambiguous_leaf =
                    hier.path.len() == 1 && !hier.path[0].name.name.contains('.');
                // Use cached signal ID if available
                if let Some(id) = hier.cached_signal_id.get() {
                    if !is_ambiguous_leaf {
                        return Some(id);
                    }
                }
                let name = self.resolve_hier_name(hier);
                if let Some(&id) = self.signal_name_to_id.get(name.as_ref()) {
                    hier.cached_signal_id.set(Some(id));
                    return Some(id);
                }
                None
            }
            ExprKind::Index { expr, index } => {
                // §7.6 / §10.4.2: a fixed multidimensional leaf is a
                // stable signal too, including for intra-assignment delays.
                if matches!(expr.kind, ExprKind::Index { .. })
                    && Self::cond_expr_is_effect_free(lhs, false)
                {
                    let mut root = expr.as_ref();
                    while let ExprKind::Index { expr, .. } = &root.kind {
                        root = expr;
                    }
                    if let ExprKind::Ident(h) = &root.kind {
                        let name = self.resolve_hier_name(h);
                        if self.module.arrays_2d.contains_key(name.as_ref())
                            || self.module.arrays_nd.contains_key(name.as_ref())
                        {
                            let leaf = self.flat_member_name(lhs)?;
                            return self.signal_name_to_id.get(leaf.as_str()).copied();
                        }
                    }
                }
                if let ExprKind::Ident(hier) = &expr.kind {
                    let name = self.resolve_hier_name(hier);
                    // A DYNAMIC array / QUEUE element carries a signal TWIN in
                    // `signal_name_to_id`, so the lookup below used to resolve
                    // it and the NBA took the fast path: `apply_nba_entry`
                    // wrote that id and marked IT dirty, but a comb reader of
                    // the COLLECTION does not depend on that id, so it was
                    // never re-evaluated. The value landed (a direct read
                    // returned it) while every reader silently served stale
                    // data --- and a later BLOCKING write did notify, which is
                    // what made it look intermittent.
                    //
                    // Returning None routes them through the queued-lvalue
                    // path, which commits via `assign_value` --- the same
                    // function blocking assignment uses, and the one
                    // ASSOCIATIVE arrays already take (they are not in
                    // `module.arrays`, which is why assoc was always correct).
                    // Static arrays keep the fast path: their element ids ARE
                    // what comb readers depend on.
                    if self.module.dynamic_arrays.contains(&*name)
                        || self.module.queue_vars.contains(&*name)
                    {
                        return None;
                    }
                    if self.module.arrays.contains_key(&*name) {
                        // `$` here is the collection's last index (§11.4.12);
                        // without the bound it evaluates to u64::MAX and the
                        // element lookup misses, silently dropping the write.
                        let db = if Self::expr_contains_dollar(index) {
                            self.dollar_bound_for_base(expr)
                        } else {
                            None
                        };
                        if let Some(b) = db {
                            self.dollar_bound.push(b);
                        }
                        let idx = self.eval_expr(index).to_i64();
                        if db.is_some() {
                            self.dollar_bound.pop();
                        }
                        let idx = idx?;
                        // Use a small buffer to avoid allocation for common array names
                        let elem = format!("{}[{}]", name, idx);
                        return self.signal_name_to_id.get(elem.as_str()).copied();
                    }
                }
                None
            }
            // LRM §25.8 — NBA write through a virtual-interface property:
            // `vif.member <= val` where `vif` is a class property declared
            // `virtual <iface_t>`. The NBA slow path (`nba_queue`) commits
            // via `assign_value` AFTER the process context is restored to the
            // event-loop caller, so `this_stack` is empty at apply time and
            // the binding lookup in `assign_value` always misses.
            //
            // Fix: resolve the binding NOW, while `this_stack` is still valid
            // (we are inside the process that owns the vif property). Convert
            // to a fast-path signal ID so the NBA commits to the right signal
            // without needing `this_stack` at apply time.
            ExprKind::MemberAccess { expr, member } => {
                // `vif.member` and `this.vif.member` name the same property;
                // the latter parses as MemberAccess(This, vif) and must resolve
                // identically. Without the `this.` form a NONBLOCKING write
                // (`this.vif.addr <= …`, the usual shape in a constructor)
                // missed the redirect and landed on a phantom signal, so the
                // real interface member stayed x.
                let vif_prop_name: Option<&str> = match &expr.kind {
                    ExprKind::Ident(hier) if hier.path.len() == 1 => {
                        Some(hier.path[0].name.name.as_str())
                    }
                    ExprKind::MemberAccess {
                        expr: inner,
                        member: vprop,
                    } if matches!(&inner.kind, ExprKind::This) => Some(vprop.name.as_str()),
                    _ => None,
                };
                if let Some(prop) = vif_prop_name {
                    {
                        // Check `local_iface_aliases` first (task formal arg vif).
                        if let Some(frame) = self.local_iface_aliases.last() {
                            if let Some(bound) = frame.get(prop).cloned() {
                                let target = format!("{}.{}", bound, member.name);
                                if let Some(&id) = self.signal_name_to_id.get(target.as_str()) {
                                    return Some(id);
                                }
                            }
                        }
                        // Check `virtual_iface_bindings` via `this_stack`.
                        if let Some(Some(this_h)) = self.this_stack.last().copied() {
                            let cls_name = self
                                .heap
                                .get(this_h)
                                .and_then(|o| o.as_ref())
                                .map(|i| i.class_name.clone());
                            let is_vif = cls_name
                                .as_ref()
                                .is_some_and(|cn| self.class_vif_decl(cn, prop).is_some());
                            if is_vif {
                                let bound = self
                                    .virtual_iface_bindings
                                    .get(&(this_h, prop.to_string()))
                                    .map(|(n, _)| n.clone());
                                if let Some(bound_name) = bound {
                                    let target = format!("{}.{}", bound_name, member.name);
                                    if let Some(&id) = self.signal_name_to_id.get(target.as_str()) {
                                        return Some(id);
                                    }
                                }
                            }
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Queue an indexed write into a packed signal without retaining an AST
    /// lvalue for the commit phase. The selected index is sampled now, while
    /// the process locals are live, and subsequent writes to the same signal
    /// merge into the existing pending value in source order.
    pub(super) fn enqueue_context_nba(&mut self, entry: NbaEntry, delay: u64) {
        if delay == 0 {
            self.nba_queue.push(entry);
        } else {
            self.delayed_nba_queue
                .push((self.time.saturating_add(delay), entry));
        }
    }

    pub(super) fn try_schedule_static_packed_nba(
        &mut self,
        lhs: &Expression,
        val: &Value,
        delay: u64,
    ) -> bool {
        let Some(name) = Self::lvalue_bare_root(lhs) else {
            return false;
        };
        if self
            .ref_alias_stack
            .last()
            .is_some_and(|m| m.contains_key(name))
            || self
                .ref_binding_stack
                .last()
                .is_some_and(|m| m.contains_key(name))
        {
            return false;
        }
        let key = self.packed_local_static_key(name);
        if key.is_none() && !self.hoisted_sub_static_names.contains(name) {
            return false;
        }
        let Some((root, _, steps, hops, two)) = self.frame_packed_target(lhs) else {
            return false;
        };
        let (width, slice) = self.packed_hops_eval(&hops, &steps);
        let Some((dst, src, len)) = slice else {
            return true;
        };
        let mut value = if val.is_real {
            Self::real_to_int(val.to_f64(), width)
        } else {
            val.resize_for_assign(width)
        };
        if two {
            value = value.to_two_state();
        }
        self.enqueue_context_nba(
            NbaEntry {
                lhs: key.is_none().then(|| root.into_owned()),
                value,
                resolved_id: None,
                static_key: key,
                static_slice: Some((dst as u32, src as u32, len as u32)),
                this_handle: None,
            },
            delay,
        );
        true
    }

    pub(super) fn try_schedule_packed_index_nba(&mut self, lhs: &Expression, val: &Value) -> bool {
        let ExprKind::Index { expr: base, index } = &lhs.kind else {
            return false;
        };
        let ExprKind::Ident(hier) = &base.kind else {
            return false;
        };
        if hier.path.len() != 1 || !hier.path[0].selects.is_empty() {
            return false;
        }
        let leaf = hier.path[0].name.name.as_str();
        if self
            .local_stack
            .last()
            .is_some_and(|frame| frame.contains_key(leaf))
        {
            return false;
        }
        let name = self.resolve_hier_name(hier);
        if self.module.arrays.contains_key(&*name)
            || self.module.queue_vars.contains(&*name)
            || self.module.dynamic_arrays.contains(&*name)
            || self.module.associative_arrays.contains_key(&*name)
        {
            return false;
        }
        let Some(&id) = self.signal_name_to_id.get(name.as_ref()) else {
            return false;
        };
        let selected = self.eval_expr(index);
        if selected.has_xz() && !selected.is_real {
            return true;
        }
        let Some(label) = selected.to_i64() else {
            return true;
        };
        let elem_width = self
            .module
            .packed_signal_elem_widths
            .get(&*name)
            .copied()
            .unwrap_or(1);
        let Some(low) = self.packed_elem_lsb(&name, label, elem_width) else {
            return true;
        };
        if low.saturating_add(elem_width as usize) > self.signal_widths[id] as usize {
            return true;
        }
        let piece = val.resize_for_assign(elem_width);
        if let Some(entry_index) = self.nba_fast_index.get(id) {
            self.nba_fast[entry_index]
                .value
                .copy_bits_from(low, &piece, 0, elem_width as usize);
            return true;
        }
        let mut pending = self.signal_table[id].clone();
        if pending.copy_bits_from(low, &piece, 0, elem_width as usize) {
            self.nba_fast_index.insert(id, self.nba_fast.len());
            self.nba_fast.push(NbaFast {
                block_index: 0,
                signal_id: id,
                value: pending,
            });
        } else {
            self.prof_nba_elided += 1;
        }
        true
    }

    /// JIT Stage 4 Tier C Path C1: drain the side queue populated by
    /// JIT-generated NBA writes directly into `nba_fast`.  Called at
    /// the start of `apply_nba` BEFORE the per-window index clear —
    /// so we skip the `nba_fast_index.insert` work the regular bridge
    /// does (the index is about to be wiped anyway).
    ///
    /// No-op when `jit_nba_side_len == 0` (default path / no JIT).
    pub(super) fn drain_jit_nba_side_queue(&mut self) {
        let n = self.jit_nba_side_len as usize;
        if n == 0 {
            return;
        }
        for i in 0..n {
            let entry = self.jit_nba_side_queue[i];
            let id = entry.signal_id as usize;
            let sig_w = self.signal_widths[id];
            let mut val = Value::from_u64(entry.val_bits, sig_w);
            val.is_signed = self.signal_signed[id];
            self.nba_fast.push(NbaFast {
                block_index: 0,
                signal_id: id,
                value: val,
            });
        }
        self.jit_nba_side_len = 0;
    }

    pub(super) fn apply_nba(&mut self) {
        if self.finished {
            return;
        }
        // Value-trace attribution: NBA commits run outside the scheduling
        // process, so current_pid is stale here -- stamp the phase so the
        // drain prints "nba" instead of a misleading process origin.
        let __vt_prev_phase = self.edge_dispatch_phase;
        self.edge_dispatch_phase = "nba";
        self.apply_nba_inner();
        self.edge_dispatch_phase = __vt_prev_phase;
    }

    pub(super) fn apply_nba_inner(&mut self) {
        // Packed-arena NBAs (issue #147): commit queued cell writes in the
        // NBA region. Mirrors the old immediate write's observable effects
        // (set_raw + table_modified) — packed cells have no signal slot, so
        // no dirty/edge bookkeeping applies.
        if !self.packed_nba.is_empty() {
            let entries = std::mem::take(&mut self.packed_nba);
            for (eid, v, x) in &entries {
                self.packed_store(*eid, *v, *x);
            }
            let mut reuse = entries;
            reuse.clear();
            self.packed_nba = reuse;
        }

        // §4.9.4: commit matured future-time NBAs first — they were scheduled
        // in an earlier slot, so they precede this slot's freshly queued NBAs.
        if !self.delayed_nba.is_empty() {
            let mut i = 0;
            while i < self.delayed_nba.len() {
                if self.delayed_nba[i].0 <= self.time {
                    let (_, id, mut val) = self.delayed_nba.swap_remove(i);
                    val.is_signed = self.signal_signed[id];
                    if self.signal_table[id] != val {
                        write_sig!(self, id, val);
                        self.mark_dirty_id(id);
                        self.table_modified = true;
                    }
                } else {
                    i += 1;
                }
            }
        }
        // JIT Stage 4 Path C1: drain any JIT-emitted inline NBA writes
        // into nba_fast BEFORE clearing the per-window index, so
        // partial-bit follow-ups (NbaAssignRange) see them.
        self.drain_jit_nba_side_queue();
        // Reset the per-window partial-NBA index.
        self.nba_fast_index.clear();
        let mut nba = std::mem::take(&mut self.nba_fast);

        for entry in nba.drain(..) {
            self.apply_nba_entry(entry);
        }
        self.nba_fast = nba;
        // Drain nba_queue by-value so the per-entry lhs Expression and value
        // Value can be moved into the call site instead of cloned. The
        // previous `for i in 0..self.nba_queue.len() { … self.nba_queue[i].lhs.clone() … }`
        // pattern existed because we needed to release the borrow on
        // `self.nba_queue` before calling `self.assign_value(&self, &mut self)`;
        // mem::take achieves the same release without the clones.
        if !self.delayed_nba_queue.is_empty() {
            let immediate = std::mem::take(&mut self.nba_queue);
            let pending = std::mem::take(&mut self.delayed_nba_queue);
            for (time, entry) in pending {
                if time <= self.time {
                    self.nba_queue.push(entry);
                } else {
                    self.delayed_nba_queue.push((time, entry));
                }
            }
            self.nba_queue.extend(immediate);
        }
        let queue = std::mem::take(&mut self.nba_queue);
        for entry in queue {
            // A `static` subroutine local commits to the key captured when the
            // NBA was SCHEDULED; re-resolving here would consult whichever call
            // frame is current in the NBA region (see NbaEntry::static_key).
            if let Some(key) = entry.static_key {
                if let Some((dst, src, len)) = entry.static_slice {
                    if let Some(stored) = self.static_local_vars.get_mut(&key) {
                        stored.copy_bits_from(
                            dst as usize,
                            &entry.value,
                            src as usize,
                            len as usize,
                        );
                    }
                } else {
                    self.static_local_vars.insert(key, entry.value);
                }
                continue;
            }
            if let Some(lhs) = entry.lhs {
                // Re-establish the scheduling context so a class-property
                // target resolves (see NbaEntry::this_handle).
                if let Some(h) = entry.this_handle {
                    self.this_stack.push(Some(h));
                }
                let value = if let Some((dst, src, len)) = entry.static_slice {
                    let mut stored = self.eval_expr(&lhs);
                    stored.copy_bits_from(dst as usize, &entry.value, src as usize, len as usize);
                    stored
                } else {
                    entry.value
                };
                self.assign_value(&lhs, &value);
                if entry.this_handle.is_some() {
                    self.this_stack.pop();
                }
            }
        }
        // §15.5.2 deferred `->>` triggers fire with the NBA flush.
        if !self.pending_nba_triggers.is_empty() {
            let evs = std::mem::take(&mut self.pending_nba_triggers);
            for ev in evs {
                self.fire_named_event(&ev);
            }
        }
        if !self.pending_nba_instance_triggers.is_empty() {
            let ievs = std::mem::take(&mut self.pending_nba_instance_triggers);
            for key in ievs {
                self.fire_instance_event(key);
            }
        }
    }

    /// §10.7 implicit-conversion fit: coerce `val` to the type of signal `id`
    /// (real target, 2-state X/Z drop, sign/X-Z-aware width padding, signedness)
    /// exactly as an assignment / continuous-assign RHS is converted to its LHS.
    /// The single chokepoint used by every commit path so blocking, nonblocking
    /// and continuous assignments agree byte-for-byte.
    #[inline]
    pub(super) fn fit_value_to_signal(&self, id: usize, val: &Value) -> Value {
        let width = self.signal_widths[id];
        if self.signal_real[id] {
            let mut r = if val.is_real {
                val.clone()
            } else {
                Value::from_f64(val.to_f64())
            };
            // Match the destination's stored signedness so an unchanged real
            // write elides (a mismatched is_signed made the value compare
            // unequal and emitted a spurious duplicate VCD record).
            r.is_signed = self.signal_signed[id];
            return r;
        }
        if width == 0 {
            // Untracked class-handle var — never truncate to nothing.
            return val.clone();
        }
        // §6.16: a string has no declared length; the table width is only a
        // placeholder. Resizing to it truncates from the TOP, which for a
        // packed string means losing the FRONT of the text.
        if self.signal_is_string.get(id).copied().unwrap_or(false) && !val.is_real {
            return val.clone();
        }
        let mut v = if val.is_real {
            Self::real_to_int(val.to_f64(), width)
        } else {
            val.resize_for_assign(width)
        };
        if self.signal_two_state.get(id).copied().unwrap_or(false) {
            v = v.to_two_state();
        }
        v.is_signed = self.signal_signed[id];
        v
    }

    #[inline]
    /// Queue an NBA into a packed-arena cell: §10.4.2 last-write-wins against
    /// the pending entry first, then eval-time elision against the cell.
    pub(super) fn queue_packed_nba(&mut self, eid: usize, v: u64, x: u64) {
        if let Some(e) = self.packed_nba.iter_mut().find(|(i, _, _)| *i == eid) {
            e.1 = v;
            e.2 = x;
        } else {
            let (cv, cx, _) = self.packed.raw(eid);
            if cv != v || cx != x {
                self.packed_nba.push((eid, v, x));
            } else {
                self.prof_nba_elided += 1;
            }
        }
    }

    pub(super) fn apply_nba_entry(&mut self, entry: NbaFast) {
        // Defensive landing for a packed-arena id: the per-signal table
        // indexing below cannot take one (it would index far out of bounds).
        if is_packed_id(entry.signal_id) {
            let (v, x) = entry.value.raw_bits();
            self.packed_store(entry.signal_id, v, x);
            return;
        }
        let id = entry.signal_id;
        let signed = self.signal_signed[id];
        // §10.7: fit the queued value to the destination type (real / 2-state /
        // width / signedness). A bare `resize` here dropped real conversions
        // (integer bits reinterpreted as a double) and left X/Z in 2-state nets.
        // A value already at the signal's width (the usual `q <= d`) is
        // MOVED into place instead of cloned through the fitting path: for
        // a wide value that clone was a fresh allocation per non-blocking
        // assignment per edge. The move is exact — a resize to the same
        // width copies the bits unchanged — and the shapes the fitting
        // path converts (fills, reals, strings, two-state targets with x/z)
        // are excluded.
        let width = self.signal_widths[id];
        let v = &entry.value;
        let mut val = if width != 0
            && v.width == width
            && !v.is_fill
            && !v.is_real
            && !self.signal_real[id]
            && !self.signal_is_string.get(id).copied().unwrap_or(false)
            && !(self.signal_two_state.get(id).copied().unwrap_or(false) && v.has_xz())
        {
            entry.value
        } else {
            self.fit_value_to_signal(id, v)
        };
        if val.is_signed != signed {
            val.is_signed = signed;
        }
        if self.signal_table[id] != val {
            if !self.dirty_signals[id] {
                self.dirty_signals[id] = true;
                self.dirty_list.push(id);
            }
            self.dirty_any = true;
            write_sig!(self, id, val);
            self.table_modified = true;
            // Clocks-only fast-path bookkeeping: a real (non-no-op) NBA
            // write to a non-clock edge signal is the case we must catch —
            // missing it would let check_edges_clocks_only skip a posedge
            // that should have fired.  The lookup is a single bool load.
            if self
                .is_edge_signal_non_clock
                .get(id)
                .copied()
                .unwrap_or(false)
            {
                self.nba_touched_edge_non_clock = true;
            }
        }
    }

    /// Apply delayed signal updates that are due at or before the current time.
    /// Returns true if any updates were applied.
    pub(super) fn apply_delayed_updates(&mut self) -> bool {
        if self.delayed_updates.is_empty() {
            return false;
        }
        let mut applied = false;
        let mut i = 0;
        while i < self.delayed_updates.len() {
            if self.delayed_updates[i].0 <= self.time {
                let (_, id, mut val) = self.delayed_updates.swap_remove(i);
                // The committed value carries the TARGET's signedness — a
                // delayed NBA of a signed 32-bit literal into a plain
                // `reg [3:0]` must not read back signed (-7 for 4'b1001).
                val.is_signed = self.signal_signed[id];
                if self.signal_table[id] != val {
                    write_sig!(self, id, val);
                    self.mark_dirty_id(id);
                    self.table_modified = true;
                    applied = true;
                }
            } else {
                i += 1;
            }
        }
        applied
    }

    /// §4.4, §30.4: a delayed update already due now (a module path or SDF
    /// delay that rounds to zero ticks, or a path-delayed net with no enabled
    /// path) is a change of this slot's Active region. Apply it, and any
    /// zero-delay hop it causes in turn, then settle, so the caller's next
    /// edge check sees it: an edge it raises (a clock through a zero-delay
    /// library cell) must sample pre-NBA values, like the undelayed edge
    /// beside it. It used to wait for the next pass of the time step, after
    /// this slot's NBAs had committed. Out of line: callers test
    /// `delayed_updates.is_empty()` first, so the common case costs one load.
    #[inline(never)]
    pub(super) fn apply_due_delayed_now(&mut self) -> bool {
        let mut applied = false;
        let mut hops = 0;
        while hops < 1024 && self.apply_delayed_updates() {
            applied = true;
            self.dirty_any = true;
            self.settle_combinatorial();
            hops += 1;
        }
        applied
    }

    /// Get the next time a delayed update is due (for time advancement).
    pub(super) fn next_delayed_time(&self) -> Option<u64> {
        let a = self.delayed_updates.iter().map(|(t, _, _)| *t).min();
        let b = self
            .delayed_nba
            .iter()
            .map(|(t, _, _)| *t)
            .chain(self.delayed_nba_queue.iter().map(|(t, _)| *t))
            .min();
        match (a, b) {
            (Some(x), Some(y)) => Some(x.min(y)),
            (x, None) => x,
            (None, y) => y,
        }
    }

    /// Next scheduler-visible timer: a VPI simulation-time callback (see
    /// `next_vpi_cb_time`) or a time-based timing-check deadline.
    pub(super) fn next_timer_time(&self) -> Option<u64> {
        match (self.next_vpi_cb_time(), self.next_timing_timer()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// End of a delta: the VPI work that belongs there (`vpi_tick_end`) —
    /// cbReadWriteSynch, which is where a VPI client applies its QUEUED
    /// signal writes (cocotb drives clocks and resets from there, not from
    /// the `Timer` callback itself), plus value changes made by paths that
    /// bypass `write_sig!` and deferred release/disable notifications.
    /// cbReadOnlySynch belongs to the end of the whole slot (`vpi_slot_end`).
    pub(super) fn fire_vpi_synch_cbs(&mut self) {
        if self.vpi_cb_mask != 0 {
            self.vpi_tick_end();
        }
    }

    /// Any future-time NBA matured at (or before) the current time?
    pub(super) fn delayed_nba_due(&self) -> bool {
        self.delayed_nba.iter().any(|(t, _, _)| *t <= self.time)
            || self.delayed_nba_queue.iter().any(|(t, _)| *t <= self.time)
    }

    /// Schedule a delayed signal update (inertial delay model).
    pub(super) fn schedule_delayed(&mut self, id: usize, val: Value) {
        let delay = self.sdf_delays.get(id).copied().unwrap_or(0);
        self.schedule_delayed_with_delay(id, val, delay);
    }

    /// §10.3.3 inertial cancellation: a driver reverting to the signal's
    /// CURRENT value while an opposite-value update is still pending means
    /// the pulse was narrower than the delay — the pending update must be
    /// dropped (no visible glitch), not left to fire.
    pub(super) fn cancel_delayed(&mut self, id: usize) {
        self.delayed_updates.retain(|(_, sid, _)| *sid != id);
    }

    /// §10.3.3 inertial delay: is an update of `id` to exactly `val` already
    /// in flight? A delayed driver whose operands moved but whose result did
    /// not (`c ^ d` through `10 -> 00 -> 11`) is not a new transition: the
    /// update keeps its time. Rescheduling it pushed the change a whole delay
    /// later for every such re-evaluation.
    pub(super) fn delayed_update_pending_is(&self, id: usize, val: &Value) -> bool {
        self.delayed_updates
            .iter()
            .any(|(_, sid, v)| *sid == id && v == val)
    }

    /// Schedule a delayed signal update with an explicit delay (inertial delay model).
    pub(super) fn schedule_delayed_with_delay(&mut self, id: usize, val: Value, delay: u64) {
        // §30.4 path-delayed net: the module paths pick the delay.
        if let Some(k) = self.module_paths.as_ref().and_then(|mp| mp.table_of(id)) {
            self.schedule_module_path(id, k, val);
            return;
        }
        // §28.11: a gate declared `#(rise, fall)` uses the FALL value for a
        // transition to 0. The lowered cont-assign only carries the rise
        // delay, so consult the per-net fall map here — this is the single
        // choke point where both the target and the new value are known.
        // Scalar gate outputs are the case that matters; "to 0" is judged on
        // the whole value so a vector net keeps using the rise delay unless it
        // goes fully zero.
        let delay = if self.gate_fall_delay_by_id.is_empty() {
            delay
        } else {
            // NB: `to_u64()` MASKS x/z to 0 rather than returning None, so it
            // cannot distinguish a genuine 0 from z — use an explicit bit test.
            // The initial x/z -> 0 settle counts as a fall as well, so the
            // test is "the new value is a real 0 and the old one was not".
            let all_zero = |v: &Value| -> bool {
                !v.has_xz() && (0..v.width as usize).all(|i| v.get_bit(i) == LogicBit::Zero)
            };
            let fall = self.gate_fall_delay_by_id.get(&id).copied();
            let off = self.gate_off_delay_by_id.get(&id).copied();
            let all_z = |v: &Value| -> bool {
                v.width > 0 && (0..v.width as usize).all(|i| v.get_bit(i) == LogicBit::Z)
            };
            if fall.is_none() && off.is_none() {
                delay
            } else if all_zero(&val) && !all_zero(&self.signal_table[id]) {
                fall.unwrap_or(delay)
            } else if all_z(&val) {
                // §10.3.3 / §28.11: to z uses the turn-off delay; with only
                // two delays given, the smaller of rise and fall.
                off.unwrap_or_else(|| fall.map_or(delay, |f| f.min(delay)))
            } else if val.has_xz() {
                // To x: the smallest of the delays given.
                [Some(delay), fall, off]
                    .into_iter()
                    .flatten()
                    .min()
                    .unwrap_or(delay)
            } else {
                delay
            }
        };
        let target_time = self.time + delay;
        // Inertial delay: remove any pending update for this signal
        self.delayed_updates.retain(|(_, sid, _)| *sid != id);
        self.delayed_updates.push((target_time, id, val));
    }

    /// Schedule a delayed update to a SLICE of `id`, composing against the
    /// PENDING delayed value when one exists (else the current value).
    ///
    /// `schedule_delayed_with_delay` is whole-signal inertial — it replaces
    /// any pending entry for the id — so two slice drivers on one base
    /// clobbered each other's bits, and the callers' "did anything change"
    /// compares ran against `signal_table` while a pending entry held the
    /// truth: the #142 elision class, in the delayed dimension. Composing
    /// into pending-or-current keeps every slice's contribution, and a slice
    /// that reverts to the CURRENT value reduces the pending entry (cancelling
    /// it outright when nothing is left to change — the inertial glitch
    /// swallow, issues #143/#144).
    pub(super) fn schedule_delayed_slice(
        &mut self,
        id: usize,
        lo: u32,
        sel_w: u32,
        sel_val: &Value,
        delay: u64,
    ) {
        let pending = self
            .delayed_updates
            .iter()
            .find(|(_, sid, _)| *sid == id)
            .map(|(_, _, v)| v.clone());
        let in_flight = pending.is_some();
        let mut merged = pending.unwrap_or_else(|| self.signal_table[id].clone());
        // The slice already holds these bits in the pending update: nothing
        // new to schedule, and the update keeps its time (see
        // `delayed_update_pending_is`).
        if in_flight
            && (0..sel_w as usize)
                .all(|i| merged.get_bit_code(lo as usize + i) == sel_val.get_bit_code(i))
        {
            return;
        }
        for i in 0..sel_w as usize {
            merged.set_bit_code(lo as usize + i, sel_val.get_bit_code(i));
        }
        if merged == self.signal_table[id] {
            self.cancel_delayed(id);
        } else {
            self.schedule_delayed_with_delay(id, merged, delay);
        }
    }

    /// Base signal id + (physical lo bit, width) for a CONSTANT select LHS —
    /// `sig[hi:lo]` or `sig[bit]` on a plain vector. Zero-based declared
    /// ranges only (a non-zero-based base would need rebasing; returning None
    /// keeps the caller's fallback behavior). Used by the delayed
    /// cont-assign path, which historically handled bare idents only.
    pub(super) fn delayed_lhs_select_range(
        &mut self,
        lhs: &Expression,
    ) -> Option<(usize, u32, u32)> {
        let (base, hi, lo) = match &lhs.kind {
            ExprKind::RangeSelect {
                expr,
                left,
                right,
                kind,
            } => {
                // §11.5.1: for the indexed forms the RIGHT operand is a
                // WIDTH, not a second index. Only `Constant` was accepted
                // here, so `assign #1 d[64 +: 32] = ...` fell through to the
                // caller's silent-drop path and the slice stayed z forever —
                // the same drop this helper was created to close for
                // constant ranges (issue #143).
                let l = self.eval_expr(left).to_u64()?;
                let r = self.eval_expr(right).to_u64()?;
                match kind {
                    RangeKind::Constant => (expr, l.max(r) as u32, l.min(r) as u32),
                    RangeKind::IndexedUp => {
                        if r == 0 {
                            return None;
                        }
                        (expr, (l + r - 1) as u32, l as u32)
                    }
                    RangeKind::IndexedDown => {
                        if r == 0 || l + 1 < r {
                            return None;
                        }
                        (expr, l as u32, (l + 1 - r) as u32)
                    }
                }
            }
            ExprKind::Index { expr, index } => {
                let i = self.eval_expr(index).to_u64()? as u32;
                (expr, i, i)
            }
            _ => return None,
        };
        let ExprKind::Ident(_) = &base.kind else {
            return None;
        };
        let id = self.get_lhs_signal_id(base)?;
        let w = self.signal_widths[id] as u32;
        if hi >= w {
            return None;
        }
        Some((id, lo, hi - lo + 1))
    }

    /// Get the signal ID for a simple LHS identifier expression.
    pub(super) fn get_lhs_signal_id(&self, lhs: &Expression) -> Option<usize> {
        if let ExprKind::Ident(hier) = &lhs.kind {
            let is_ambiguous_leaf = hier.path.len() == 1 && !hier.path[0].name.name.contains('.');
            if let Some(id) = hier.cached_signal_id.get() {
                if !is_ambiguous_leaf {
                    return Some(id);
                }
            }
            let resolved = self.resolve_hier_name(hier);
            if let Some(&id) = self.signal_name_to_id.get(resolved.as_ref()) {
                return Some(id);
            }
            // Fallback for legacy single-segment names.
            let leaf = hier.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
            self.signal_name_to_id.get(leaf).copied()
        } else {
            None
        }
    }

    /// Snapshot only edge-sensitive signals + event_waiter signals into
    /// the prev_val/prev_xz parallel arrays (A3 from the compression
    /// analysis). Wide signals (> 64 bits) also update `prev_wide`.
    /// Under the dirty-edge scan the edge-signal walk below is redundant:
    /// every scanned signal that changed was re-baselined by
    /// `check_edges_inner` (`fired_snap`), and a signal no write hook touched
    /// already equals its baseline — so only the initial baseline, the shadow
    /// mode and dirty-edge-off take the full walk. The event-waiter
    /// sensitivities are always refreshed.
    pub(super) fn snapshot_edge_signals(&mut self) {
        self.snapshot_edge_signals_inner(false);
    }

    pub(super) fn snapshot_edge_signals_full(&mut self) {
        self.snapshot_edge_signals_inner(true);
    }

    pub(super) fn snapshot_edge_signals_inner(&mut self, full: bool) {
        // New snapshot generation: waiters registered before this point
        // become eligible for edges detected against it (see EventWaiter).
        self.snap_gen = self.snap_gen.wrapping_add(1);
        let walk_edges = full || !self.dirty_edge || self.dirty_edge_shadow;
        #[inline]
        fn snap_one(
            id: usize,
            signal_table: &[Value],
            signal_widths: &[u32],
            prev_val: &mut [u64],
            prev_xz: &mut [u64],
            prev_wide: &mut HashMap<usize, Value>,
        ) {
            let (v, x) = signal_table[id].raw_bits();
            prev_val[id] = v;
            prev_xz[id] = x;
            if signal_widths[id] > 64 {
                if let Some(p) = prev_wide.get_mut(&id) {
                    p.copy_from(&signal_table[id]);
                }
            }
        }
        // Hot loop: this runs over the WHOLE edge-signal set on every snapshot
        // (once per tick, plus once per edge/NBA cascade round), so anything
        // constant per id is multiplied by ~|edge_signal_ids| × passes.
        //
        // Two costs are removed versus calling `snap_one` per id:
        //  * `snap_one` did not actually inline (six parameters, one a
        //    `&mut HashMap`), so every id paid a call plus a seven-register
        //    prologue/epilogue — visible as the `push`/`pop` block at the head
        //    of its disassembly.
        //  * each of the four table indexings re-derived its (ptr, len) from
        //    `self` and bounds-checked separately. Borrowing the tables once
        //    as local slices keeps the bases in registers, and a single
        //    `id < lim` test (lim = the shortest of the four) proves all four
        //    accesses in range.
        // `edge_signal_ids` is NOT guaranteed sorted — `sampled_watch_for`
        // appends to it at run time — so `lim` is compared per id rather than
        // once against the last element. Any id at or past `lim` falls back to
        // the checked path, preserving the original panic-on-out-of-range.
        if walk_edges {
            let st: &[Value] = &self.signal_table;
            let widths: &[u32] = &self.signal_widths;
            let pv: &mut [u64] = &mut self.prev_val;
            let px: &mut [u64] = &mut self.prev_xz;
            let pw: &mut HashMap<usize, Value> = &mut self.prev_wide;
            let lim = st.len().min(widths.len()).min(pv.len()).min(px.len());
            for &id in self.edge_signal_ids.iter() {
                if id >= lim {
                    // Out of range for at least one table: keep the original
                    // (indexing, therefore panicking) behavior.
                    let (v, x) = st[id].raw_bits();
                    pv[id] = v;
                    px[id] = x;
                    if widths[id] > 64 {
                        if let Some(p) = pw.get_mut(&id) {
                            p.copy_from(&st[id]);
                        }
                    }
                    continue;
                }
                // SAFETY: `id < lim <= len` of each of the four slices.
                let sv = unsafe { st.get_unchecked(id) };
                let (v, x) = sv.raw_bits();
                unsafe {
                    *pv.get_unchecked_mut(id) = v;
                    *px.get_unchecked_mut(id) = x;
                }
                if unsafe { *widths.get_unchecked(id) } > 64 {
                    if let Some(p) = pw.get_mut(&id) {
                        p.copy_from(sv);
                    }
                }
            }
        }
        if !walk_edges {
            for k in 0..self.preponed_ids.len() {
                let sid = self.preponed_ids[k];
                snap_one(
                    sid,
                    &self.signal_table,
                    &self.signal_widths,
                    &mut self.prev_val,
                    &mut self.prev_xz,
                    &mut self.prev_wide,
                );
            }
        }
        for i in 0..self.event_waiters.len() {
            for j in 0..self.event_waiters[i].resolved_sensitivities.len() {
                let sid = self.event_waiters[i].resolved_sensitivities[j].signal_id;
                // A cell of a memory stored without element names (arena or
                // bulk) has no edge snapshot; its waiter compares against
                // its own `captured_prev`.
                if sid >= self.prev_val.len() {
                    continue;
                }
                snap_one(
                    sid,
                    &self.signal_table,
                    &self.signal_widths,
                    &mut self.prev_val,
                    &mut self.prev_xz,
                    &mut self.prev_wide,
                );
            }
        }
        for i in 0..self.cg_event_waiters.len() {
            for j in 0..self.cg_event_waiters[i].1.len() {
                let sid = self.cg_event_waiters[i].1[j].signal_id;
                snap_one(
                    sid,
                    &self.signal_table,
                    &self.signal_widths,
                    &mut self.prev_val,
                    &mut self.prev_xz,
                    &mut self.prev_wide,
                );
            }
        }
    }

    /// Check edge: compare signal_table[id] vs (prev_val[id], prev_xz[id]).
    #[inline]
    pub(super) fn check_edge_id(&self, id: usize, edge: EdgeKind) -> bool {
        self.edge_fires_prev(
            id,
            edge,
            self.prev_val[id],
            self.prev_xz[id],
            self.prev_wide.get(&id),
        )
    }

    /// Edge comparison against an EXPLICIT prev baseline (rather than the
    /// global `prev_val`/`prev_xz` snapshot). Used by event_waiters, which
    /// capture each sensitivity's value at registration so they fire on a
    /// change relative to that point — critical for `nba <= v; @(nba)`
    /// patterns, where the NBA commits to the new value AFTER registration in the
    /// same tick and the waiter must wake (IEEE 1800-2023 §4.10, §9.4.2.3).
    pub(super) fn edge_fires_prev(
        &self,
        id: usize,
        edge: EdgeKind,
        prev_v: u64,
        prev_x: u64,
        prev_wide: Option<&Value>,
    ) -> bool {
        let (cur_v, cur_x) = self.signal_table[id].raw_bits();
        // LogicBit at bit 0: One iff v&1==1 && x&1==0; Zero iff v&1==0 && x&1==0.
        let cb_one = (cur_v & 1) == 1 && (cur_x & 1) == 0;
        let cb_zero = (cur_v & 1) == 0 && (cur_x & 1) == 0;
        let pb_one = (prev_v & 1) == 1 && (prev_x & 1) == 0;
        let pb_zero = (prev_v & 1) == 0 && (prev_x & 1) == 0;
        match edge {
            EdgeKind::Posedge => (pb_zero && !cb_zero) || (!pb_one && cb_one),
            EdgeKind::Negedge => (pb_one && !cb_one) || (!pb_zero && cb_zero),
            EdgeKind::AnyEdge => {
                if self.signal_widths[id] > 64 {
                    if let Some(p) = prev_wide {
                        return self.signal_table[id] != *p;
                    }
                }
                cur_v != prev_v || cur_x != prev_x
            }
            // §9.4.2 `@(edge x)`: posedge OR negedge of the LSB.
            EdgeKind::LsbEdge => {
                (pb_zero && !cb_zero)
                    || (!pb_one && cb_one)
                    || (pb_one && !cb_one)
                    || (!pb_zero && cb_zero)
            }
        }
    }

    pub(super) fn check_edges(&mut self) {
        self.check_edges_pass();
        self.drain_edge_exec_rescan();
    }

    /// §9.2 re-trigger drain. A blocking write to an edge-sensitive signal
    /// made WHILE an edge block executed (recorded into `edge_exec_wrote`
    /// via write_sig!/note_edge_write) is a change that must re-trigger
    /// blocks sensitive to it — check_edges used to coalesce it away
    /// (prev was only refreshed by the NEXT snapshot, which erased the
    /// pending edge before any detect saw it: silent event loss). Re-detect
    /// exactly the written positions, repeating while the re-triggered
    /// blocks keep producing new writes. A mutual ping-pong
    /// (`always @(a) b=~b;` / `always @(b) a=~a;`) is then a genuine
    /// zero-delay livelock: bounded by the stall detector
    /// (XEZIM_STALL_LIMIT), which terminates with an attributed report
    /// naming the blocks instead of hanging.
    pub(super) fn drain_edge_exec_rescan(&mut self) {
        if self.edge_exec_wrote.is_empty() {
            return;
        }
        let mut passes: u64 = 0;
        self.edge_rescan_block_hits.clear();
        self.in_edge_rescan = true;
        let prev_phase = self.edge_dispatch_phase;
        self.edge_dispatch_phase = "rescan";
        let mut subset: Vec<usize> = Vec::new();
        while !self.edge_exec_wrote.is_empty() && !self.finished {
            subset.clear();
            std::mem::swap(&mut subset, &mut self.edge_exec_wrote);
            for &p in &subset {
                if p < self.edge_exec_seen.len() {
                    self.edge_exec_seen[p] = false;
                }
            }
            passes += 1;
            if self.stall_limit > 0 && passes > self.stall_limit {
                self.report_edge_rescan_stall(passes);
                self.abort_run();
                break;
            }
            self.check_edges_inner(Some(&subset), false);
        }
        self.in_edge_rescan = false;
        self.edge_dispatch_phase = prev_phase;
    }

    /// A zero-delay livelock sustained entirely by edge-triggered always
    /// blocks re-triggering each other through blocking writes (no process
    /// ever re-enters the scheduler, so the pid-based stall report cannot
    /// see it). Names the most frequently re-triggered blocks — kind,
    /// file:line, instance scope, sensitivity list — mirroring
    /// `report_zero_delay_stall`'s shape.
    pub(super) fn report_edge_rescan_stall(&mut self, passes: u64) {
        eprintln!(
            "[xezim][error] simulation STALLED at time {} — no progress after {} edge-cascade delta cycles at this timestamp.",
            self.time, passes
        );
        eprintln!(
            "               This is a zero-delay (delta) livelock: edge-sensitive always blocks keep"
        );
        eprintln!(
            "               re-triggering each other at the same timestamp, so simulated time can never move."
        );
        let mut top: Vec<(usize, u64)> = self
            .edge_rescan_block_hits
            .iter()
            .map(|(b, c)| (*b, *c))
            .collect();
        top.sort_by_key(|&(bi, c)| (std::cmp::Reverse(c), bi));
        top.truncate(5);
        if !top.is_empty() {
            eprintln!("               Most frequently re-triggered blocks:");
            for (bi, count) in top {
                let Some(block) = self.edge_blocks.get(bi) else {
                    continue;
                };
                let kind = match block.kind {
                    AlwaysKind::AlwaysFf => "always_ff block",
                    AlwaysKind::AlwaysComb => "always_comb block",
                    AlwaysKind::AlwaysLatch => "always_latch block",
                    AlwaysKind::Always => "always block",
                };
                let mut line = format!("                 {}", kind);
                let scope = if block.scope.is_empty() {
                    None
                } else {
                    Some(block.scope.as_str())
                };
                let src_file = self.scope_src_file(scope);
                if let Some(loc) = self.span_file_line_in(block.stmt.span, src_file) {
                    line.push_str(" at ");
                    line.push_str(&loc);
                }
                if let Some(s) = scope {
                    match self.scope_def_module(Some(s)) {
                        Some(m) if m != self.module.name => {
                            line.push_str(&format!(" ({}, module {})", s, m));
                        }
                        _ => line.push_str(&format!(" ({})", s)),
                    }
                }
                let sens = block
                    .resolved_sensitivities
                    .iter()
                    .map(|si| {
                        let edge = si.edge.print_str();
                        format!("{}{}", edge, self.name_for_id(si.signal_id))
                    })
                    .collect::<Vec<_>>()
                    .join(" or ");
                if !sens.is_empty() {
                    line.push_str(&format!(" — sensitive to @({})", sens));
                }
                line.push_str(&format!(" — ran {} times at this timestamp", count));
                eprintln!("{}", line);
            }
        }
        eprintln!("               Common causes:");
        eprintln!(
            "                 * two always blocks each writing a signal in the other's sensitivity list with no delay"
        );
        eprintln!(
            "                 * a combinational feedback loop expressed through @(...) blocks"
        );
        eprintln!(
            "               Raise or disable the check with XEZIM_STALL_LIMIT=<n> (0 = never stall-check)."
        );
    }

    /// One edge detect+dispatch pass (mode dispatcher). Callers should use
    /// `check_edges`, which additionally drains the §9.2 re-triggers
    /// produced by blocking writes during edge-block execution.
    pub(super) fn check_edges_pass(&mut self) {
        // Time-0 init detect is special (initial values set via non-hot paths
        // vs uninitialized prev → everything "fires"). Run it as a full scan
        // and reset the dirty accumulator so steady-state (time>0) starts clean.
        if (self.dirty_edge || self.dirty_edge_shadow) && self.time == 0 {
            self.check_edges_inner(None, false);
            let positions = std::mem::take(&mut self.changed_edge_pos);
            for &p in &positions {
                if p < self.edge_pos_seen.len() {
                    self.edge_pos_seen[p] = false;
                }
            }
            let mut s = positions;
            s.clear();
            self.changed_edge_pos = s;
            return;
        }
        if self.dirty_edge && !self.dirty_edge_shadow {
            // Dirty-driven detect: scan only edge positions that changed since
            // the last detect (written via write_sig!/after_signal_write hooks)
            // plus the clocks that toggled this iter. Merge the toggled clocks
            // into the changed set (deduped), scan that subset, then clear.
            for &cp in &self.toggled_clock_positions {
                if cp < self.edge_pos_seen.len() && !self.edge_pos_seen[cp] {
                    self.edge_pos_seen[cp] = true;
                    self.changed_edge_pos.push(cp);
                }
            }
            let subset = std::mem::take(&mut self.changed_edge_pos);
            self.changed_edge_pos = std::mem::take(&mut self.edge_pos_scratch);
            self.check_edges_inner(Some(&subset), false);
            for &p in &subset {
                if p < self.edge_pos_seen.len() {
                    self.edge_pos_seen[p] = false;
                }
            }
            // A process resumed INSIDE the scan (an edge continuation) may
            // write an edge-sensitive signal; that position is pushed onto
            // the list during the scan. Overwriting the list with the
            // emptied subset dropped it while its `edge_pos_seen` stayed
            // set, so no later write to that signal could ever queue it
            // again: an `always @(sig)` fired once and was dead for the
            // rest of the run. Carry those positions into the next pass.
            let mut pushed_during_scan = std::mem::take(&mut self.changed_edge_pos);
            let mut s = subset;
            s.clear();
            s.extend_from_slice(&pushed_during_scan);
            for &p in &pushed_during_scan {
                if p < self.edge_pos_seen.len() {
                    self.edge_pos_seen[p] = true;
                }
            }
            self.changed_edge_pos = s;
            pushed_during_scan.clear();
            self.edge_pos_scratch = pushed_during_scan;
            return;
        }
        if self.dirty_edge_shadow {
            // Shadow: full scan drives the sim (truth); mark the dirty set
            // (clocks + accumulated writes) so check_edges_inner can assert
            // every FIRING position was captured — validating hook coverage.
            for &cp in &self.toggled_clock_positions {
                if cp < self.edge_pos_seen.len() && !self.edge_pos_seen[cp] {
                    self.edge_pos_seen[cp] = true;
                    self.changed_edge_pos.push(cp);
                }
            }
            self.check_edges_inner(None, true);
            let positions = std::mem::take(&mut self.changed_edge_pos);
            for &p in &positions {
                if p < self.edge_pos_seen.len() {
                    self.edge_pos_seen[p] = false;
                }
            }
            let mut s = positions;
            s.clear();
            self.changed_edge_pos = s;
            return;
        }
        self.check_edges_inner(None, false);
    }
}
