//! Time-0 hold of combinational entries with observable effects.
//!
//! IEEE 1800-2023 §9.2.2.2: an `always_comb` procedure is triggered once at
//! time zero, after all `initial` and `always` procedures have been started.
//! §4.9.1 and §10.3 leave the time-0 order of a continuous assignment against
//! the procedures open (§4.7). The pre-process settle evaluates every entry
//! against the declaration-time state so that a process started at time 0
//! reads settled values, which is unobservable for an entry that only
//! computes its outputs. An entry whose evaluation does more — prints, runs
//! an immediate assertion, calls a class method or a DPI import, or calls a
//! function that writes a variable outside its own frame — would show that
//! extra evaluation: the assertion fires on the type defaults, the method runs
//! on a null handle. Such an entry is held out of every settle of the time-0
//! start batch and evaluated once when the last process has started.

use super::*;
use crate::ast::stmt::ForInit;

/// System tasks and functions whose call is observable: output, simulation
/// control, file access, the random generators and the memory loaders.
fn sys_call_has_effect(name: &str) -> bool {
    let n = name.strip_prefix('$').unwrap_or(name);
    const EXACT: &[&str] = &[
        "finish",
        "stop",
        "exit",
        "fatal",
        "error",
        "warning",
        "info",
        "fopen",
        "fclose",
        "fflush",
        "fgets",
        "fgetc",
        "fscanf",
        "fread",
        "ungetc",
        "fseek",
        "rewind",
        "random",
        "urandom",
        "urandom_range",
        "system",
        "srandom",
    ];
    const PREFIX: &[&str] = &[
        "display", "write", "strobe", "monitor", "fdisplay", "fwrite", "fstrobe", "fmonitor",
        "dist_", "readmem", "writemem", "dump",
    ];
    EXACT.contains(&n) || PREFIX.iter().any(|p| n.starts_with(p))
}

/// The leftmost name of an assignment target (`a` of `a.b[i]`), if it is one.
fn lvalue_root(e: &Expression) -> Option<&str> {
    match &e.kind {
        ExprKind::Ident(h) if h.root.is_none() => h.path.first().map(|s| s.name.name.as_str()),
        ExprKind::Index { expr, .. }
        | ExprKind::RangeSelect { expr, .. }
        | ExprKind::MemberAccess { expr, .. } => lvalue_root(expr),
        ExprKind::Paren(e) => lvalue_root(e),
        _ => None,
    }
}

/// Every name a routine body declares: block locals at any depth, `for` and
/// `foreach` loop variables (§12.7, §13.4).
fn body_locals<'a>(st: &'a Statement, out: &mut HashSet<&'a str>) {
    match &st.kind {
        StatementKind::VarDecl { declarators, .. }
        | StatementKind::ParamDecl { declarators, .. } => {
            for d in declarators {
                out.insert(d.name.name.as_str());
            }
        }
        StatementKind::For { init, body, .. } => {
            for i in init {
                if let ForInit::VarDecl { name, .. } = i {
                    out.insert(name.name.as_str());
                }
            }
            body_locals(body, out);
        }
        StatementKind::Foreach { vars, body, .. } => {
            for v in vars.iter().flatten() {
                out.insert(v.name.as_str());
            }
            body_locals(body, out);
        }
        StatementKind::If {
            then_stmt,
            else_stmt,
            ..
        } => {
            body_locals(then_stmt, out);
            if let Some(e) = else_stmt {
                body_locals(e, out);
            }
        }
        StatementKind::Case { items, .. } => {
            for it in items {
                body_locals(&it.stmt, out);
            }
        }
        StatementKind::While { body, .. }
        | StatementKind::DoWhile { body, .. }
        | StatementKind::Repeat { body, .. }
        | StatementKind::Forever { body } => body_locals(body, out),
        StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
            for s in stmts.iter() {
                body_locals(s, out);
            }
        }
        _ => {}
    }
}

/// The names a body owns: a routine's formals, return variable and locals, or
/// an `always_comb` body's locals and the loop variables its `for` loops
/// assign. A routine's write to any other name leaves its frame; the entry's
/// own body writes its outputs.
pub(super) struct T0Frame<'a> {
    names: HashSet<&'a str>,
    routine: bool,
}

impl<'a> T0Frame<'a> {
    /// The frame of a continuous assignment's expressions: no names.
    pub(super) fn expr() -> Self {
        T0Frame {
            names: HashSet::default(),
            routine: false,
        }
    }

    /// The frame of an `always_comb` body.
    pub(super) fn block(body: &'a Statement) -> Self {
        let mut names = HashSet::default();
        body_locals(body, &mut names);
        loop_vars(body, &mut names);
        T0Frame {
            names,
            routine: false,
        }
    }
}

/// A compound assignment (`n += 1`) is parsed as `n = n + 1` with the target
/// cloned, span included, as the left operand.
fn is_compound_assign(lvalue: &Expression, rvalue: &Expression) -> bool {
    matches!(&rvalue.kind, ExprKind::Binary { left, .. }
        if left.span == lvalue.span
            && std::mem::discriminant(&left.kind) == std::mem::discriminant(&lvalue.kind))
}

/// The variables a body gives a value of its own: the targets of its plain
/// assignments (`y = '0;` ahead of `y |= b;`) and the variables its `for`
/// loops initialize (`for (i = 0; ...; i++)` with `i` declared outside). A
/// variable the body only steps (`n++`, `n += 1`) is not one.
fn loop_vars<'a>(st: &'a Statement, out: &mut HashSet<&'a str>) {
    match &st.kind {
        StatementKind::BlockingAssign { lvalue, rvalue }
        | StatementKind::NonblockingAssign { lvalue, rvalue, .. } => {
            if !is_compound_assign(lvalue, rvalue)
                && let Some(r) = lvalue_root(lvalue)
            {
                out.insert(r);
            }
        }
        StatementKind::For { init, body, .. } => {
            for i in init {
                if let ForInit::Assign { lvalue, .. } = i
                    && let Some(r) = lvalue_root(lvalue)
                {
                    out.insert(r);
                }
            }
            loop_vars(body, out);
        }
        StatementKind::If {
            then_stmt,
            else_stmt,
            ..
        } => {
            loop_vars(then_stmt, out);
            if let Some(e) = else_stmt {
                loop_vars(e, out);
            }
        }
        StatementKind::Case { items, .. } => {
            for it in items {
                loop_vars(&it.stmt, out);
            }
        }
        StatementKind::Foreach { body, .. }
        | StatementKind::While { body, .. }
        | StatementKind::DoWhile { body, .. }
        | StatementKind::Repeat { body, .. }
        | StatementKind::Forever { body } => loop_vars(body, out),
        StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
            for s in stmts.iter() {
                loop_vars(s, out);
            }
        }
        _ => {}
    }
}

/// The walk's state: the routines being decided (a recursive call adds
/// nothing its first activation does not already have) and the verdicts.
#[derive(Default)]
pub(super) struct Time0Effects {
    active: HashSet<String>,
    memo: HashMap<String, bool>,
}

impl Simulator {
    /// Whether a call's base names a class handle: `h` of `h.get()`.
    fn t0_names_class_handle(&self, base: &Expression, scope: Option<&str>) -> bool {
        if self.module.classes.is_empty() {
            return false;
        }
        let name = match &base.kind {
            ExprKind::Ident(h) => Self::resolve_hier_name_static(h, &self.module),
            ExprKind::Index { expr, .. } | ExprKind::Paren(expr) => {
                return self.t0_names_class_handle(expr, scope);
            }
            // `a.b.m()`: a property of an object, or a member of a struct.
            ExprKind::MemberAccess { expr, .. } => {
                return self.t0_names_class_handle(expr, scope);
            }
            ExprKind::This => return true,
            _ => return false,
        };
        if self.class_of_var(&name).is_some() {
            return true;
        }
        scope.is_some_and(|s| self.class_of_var(&format!("{s}.{name}")).is_some())
    }

    /// Whether a proper prefix of the path `h` names a class handle: `h` of
    /// `h.v` and of `h.get`.
    fn t0_path_through_handle(&self, h: &HierarchicalIdentifier, scope: Option<&str>) -> bool {
        if self.module.classes.is_empty() || h.root.is_some() {
            return false;
        }
        let mut name = String::new();
        for (i, s) in h.path[..h.path.len() - 1].iter().enumerate() {
            if i > 0 {
                name.push('.');
            }
            name.push_str(&s.name.name);
            if self.class_of_var(&name).is_some()
                || scope.is_some_and(|sc| self.class_of_var(&format!("{sc}.{name}")).is_some())
            {
                return true;
            }
        }
        false
    }

    /// The registration of the routine a call names and whether it is a
    /// function (`module.functions`) or a task (`module.tasks`): the bare or
    /// scoped name, or the `<inst>.<name>` of an inlined instance.
    fn t0_routine_key(&self, func: &Expression, scope: Option<&str>) -> Option<(String, bool)> {
        if let Some(key) = Self::scoped_function_key(func, &self.module) {
            return Some((key, true));
        }
        let ExprKind::Ident(hier) = &func.kind else {
            return None;
        };
        if hier.path.iter().any(|s| !s.selects.is_empty()) {
            return None;
        }
        let full = Self::resolve_hier_name_static(hier, &self.module);
        let mut cands: Vec<String> = Vec::with_capacity(3);
        if let Some(s) = scope {
            cands.push(format!("{s}.{full}"));
        }
        cands.push(full);
        for c in cands {
            if self.module.functions.contains_key(&c) {
                return Some((c, true));
            }
            if self.module.tasks.contains_key(&c) {
                return Some((c, false));
            }
        }
        None
    }

    /// Whether running the routine registered as `key` is observable from
    /// outside its frame.
    fn t0_routine_has_effects(&self, key: &str, is_fn: bool, st: &mut Time0Effects) -> bool {
        if let Some(&v) = st.memo.get(key) {
            return v;
        }
        if !st.active.insert(key.to_string()) {
            return false;
        }
        let scope = key.rsplit_once('.').map(|(s, _)| s.to_string());
        let (ports, items, leaf) = if is_fn {
            let Some(f) = self.module.functions.get(key) else {
                return false;
            };
            (&f.ports, &f.items, f.name.name.name.as_str())
        } else {
            let Some(t) = self.module.tasks.get(key) else {
                return false;
            };
            (&t.ports, &t.items, t.name.name.name.as_str())
        };
        let mut names: HashSet<&str> = ports.iter().map(|p| p.name.name.as_str()).collect();
        names.insert(leaf);
        for s in items.iter() {
            body_locals(s, &mut names);
        }
        let frame = T0Frame {
            names,
            routine: true,
        };
        let v = items
            .iter()
            .any(|s| self.t0_stmt_has_effects(s, scope.as_deref(), &frame, st));
        st.active.remove(key);
        st.memo.insert(key.to_string(), v);
        v
    }

    /// Whether a write to `lv` leaves a routine's frame. The entry's own
    /// body writes its outputs.
    fn t0_write_escapes(lv: &Expression, frame: &T0Frame) -> bool {
        frame.routine && Self::t0_not_owned(lv, frame)
    }

    /// Whether `lv` names a variable the body does not own. Stepping such a
    /// variable from its own value (`n++`, `n += 1`) counts the evaluations
    /// of the entry.
    fn t0_not_owned(lv: &Expression, frame: &T0Frame) -> bool {
        match &lv.kind {
            ExprKind::Concatenation(xs) => xs.iter().any(|x| Self::t0_not_owned(x, frame)),
            _ => lvalue_root(lv).is_none_or(|r| !frame.names.contains(r)),
        }
    }

    /// Whether evaluating `e` is observable beyond the value it yields.
    pub(super) fn t0_expr_has_effects(
        &self,
        e: &Expression,
        scope: Option<&str>,
        frame: &T0Frame,
        st: &mut Time0Effects,
    ) -> bool {
        let mut found = false;
        super::elaborate::for_each_sub_expr(e, &mut |x| {
            if found {
                return;
            }
            found = match &x.kind {
                ExprKind::SystemCall { name, .. } => sys_call_has_effect(name),
                ExprKind::Call { func, .. } => {
                    if let Some((key, is_fn)) = self.t0_routine_key(func, scope) {
                        self.t0_routine_has_effects(&key, is_fn, st)
                    } else {
                        match &func.kind {
                            // A DPI import runs foreign code unless it is
                            // declared `pure` (§35.5.2).
                            ExprKind::Ident(h) if h.path.len() == 1 => self
                                .module
                                .dpi_imports
                                .get(h.path[0].name.name.as_str())
                                .is_some_and(|d| {
                                    !matches!(d.property, Some(crate::ast::decl::DPIProperty::Pure))
                                }),
                            // `C::m()`, a static method (§8.10), and `h.m()`,
                            // a method of the object `h` names (§8.6).
                            ExprKind::Ident(h) => {
                                self.module
                                    .classes
                                    .contains_key(h.path[0].name.name.as_str())
                                    || self.t0_path_through_handle(h, scope)
                            }
                            ExprKind::MemberAccess { expr, .. } => {
                                self.t0_names_class_handle(expr, scope)
                            }
                            _ => false,
                        }
                    }
                }
                // §8.4: a property read through a handle that is still null
                // at time 0 is an error.
                ExprKind::MemberAccess { expr, .. } => self.t0_names_class_handle(expr, scope),
                ExprKind::Ident(h) if h.path.len() > 1 => self.t0_path_through_handle(h, scope),
                ExprKind::AssignExpr { lvalue, .. } => Self::t0_not_owned(lvalue, frame),
                ExprKind::Unary { op, operand }
                    if matches!(
                        op,
                        UnaryOp::PreIncr | UnaryOp::PreDecr | UnaryOp::PostIncr | UnaryOp::PostDecr
                    ) =>
                {
                    Self::t0_not_owned(operand, frame)
                }
                _ => false,
            };
        });
        found
    }

    /// Whether executing `stmt` is observable beyond the variables an
    /// `always_comb` body assigns.
    pub(super) fn t0_stmt_has_effects(
        &self,
        stmt: &Statement,
        scope: Option<&str>,
        frame: &T0Frame,
        st: &mut Time0Effects,
    ) -> bool {
        if self.t0_stmt_kind_has_effects(stmt, frame) {
            return true;
        }
        let mut found = false;
        super::elaborate::for_each_stmt_expr(stmt, &mut |e| {
            if !found {
                found = self.t0_expr_has_effects(e, scope, frame, st);
            }
        });
        found
    }

    /// The statement-kind half of `t0_stmt_has_effects`, over `stmt` and the
    /// statements nested in it: `for_each_stmt_expr` hands over only their
    /// expressions.
    fn t0_stmt_kind_has_effects(&self, stmt: &Statement, frame: &T0Frame) -> bool {
        match &stmt.kind {
            // §16.3, §16.4: an immediate assertion reports or runs its action
            // block when it is executed.
            StatementKind::Assertion(a) if !a.is_property && !a.is_sequence => return true,
            StatementKind::BlockingAssign { lvalue, .. }
            | StatementKind::NonblockingAssign { lvalue, .. }
                if Self::t0_write_escapes(lvalue, frame) =>
            {
                return true;
            }
            StatementKind::BlockingAssign { lvalue, rvalue }
                if Self::t0_not_owned(lvalue, frame) && is_compound_assign(lvalue, rvalue) =>
            {
                return true;
            }
            StatementKind::For { init, .. } => {
                for i in init {
                    if let ForInit::Assign { lvalue, .. } = i
                        && Self::t0_write_escapes(lvalue, frame)
                    {
                        return true;
                    }
                }
            }
            _ => {}
        }
        let nested = |s: &Statement| self.t0_stmt_kind_has_effects(s, frame);
        match &stmt.kind {
            StatementKind::If {
                then_stmt,
                else_stmt,
                ..
            } => nested(then_stmt) || else_stmt.as_ref().is_some_and(|e| nested(e)),
            StatementKind::Case { items, .. } => items.iter().any(|it| nested(&it.stmt)),
            StatementKind::For { body, .. }
            | StatementKind::Foreach { body, .. }
            | StatementKind::While { body, .. }
            | StatementKind::DoWhile { body, .. }
            | StatementKind::Repeat { body, .. }
            | StatementKind::Forever { body }
            | StatementKind::TimingControl { stmt: body, .. }
            | StatementKind::Wait { stmt: body, .. } => nested(body),
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                stmts.iter().any(nested)
            }
            _ => false,
        }
    }

    /// Take the entries with time-0 effects out of the settle until the
    /// start batch is over (`release_time0_held`). The always_comb blocks
    /// without inputs are `comb_time0_deferred`'s.
    pub(super) fn hold_time0_effect_entries(&mut self) {
        debug_assert!(self.comb_time0_held.is_empty());
        for (i, e) in self.comb_entries.iter_mut().enumerate() {
            if !e.cold.time0_effects {
                continue;
            }
            let held = match &e.item {
                CombItem::ContAssign { .. } | CombItem::CompiledContAssign { .. } => true,
                CombItem::AlwaysBlock { is_always_comb, .. }
                | CombItem::CompiledAlwaysBlock { is_always_comb, .. } => {
                    *is_always_comb && !e.cold.read_signal_ids.is_empty()
                }
                _ => false,
            };
            if held {
                let item = std::mem::replace(&mut e.item, CombItem::Noop);
                self.comb_time0_held.push((i, item));
            }
        }
    }

    /// §9.2.2.2: every `initial` and `always` procedure has started; give
    /// the held entries back to the settle and evaluate each once.
    pub(super) fn release_time0_held(&mut self) {
        if self.comb_time0_held.is_empty() || self.settling {
            return;
        }
        // Taken first: the settles below must not come back here.
        let held = std::mem::take(&mut self.comb_time0_held);
        // A neutral scope, as for the deferred continuous-assignment updates
        // (`drain_deferred_proc_settle`).
        let saved_hint = self.name_resolve_hint.borrow_mut().take();
        // The updates the start batch deferred (`deferred_proc_entries`)
        // reach the other entries first, while the held ones still stand
        // aside: each held entry then runs once, on the settled state.
        self.drain_deferred_proc_settle();
        if self.dirty_any && !self.finished {
            self.settle_combinatorial();
        }
        let mut idx: Vec<usize> = Vec::with_capacity(held.len());
        for (i, item) in held {
            if let Some(e) = self.comb_entries.get_mut(i) {
                e.item = item;
                idx.push(i);
            }
        }
        // Evaluated as the settle evaluates them at any later change: in the
        // entry's scope, and with a deferred assertion report owned by the
        // block, not by the process that happens to be current, so that a
        // re-evaluation in the same time step supersedes it (§16.4.2).
        self.comb_direct_eval = true;
        for i in idx {
            if self.finished {
                break;
            }
            let entry = self.comb_entries[i].clone();
            *self.name_resolve_hint.borrow_mut() = entry.cold.scope_hint.clone();
            self.eval_comb_entry_full(&entry);
        }
        self.comb_direct_eval = false;
        *self.name_resolve_hint.borrow_mut() = None;
        if self.dirty_any && !self.finished {
            self.settle_combinatorial();
        }
        *self.name_resolve_hint.borrow_mut() = saved_hint;
    }
}
