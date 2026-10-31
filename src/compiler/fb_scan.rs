//! What an AST fallback can touch.
//!
//! A compiled block keeps some variables in VM registers only: block-local
//! declarations, `for (int i ...)` counters, unrolled loop constants and the
//! formals and locals of an inlined task. The AST interpreter cannot see a
//! register, so a statement or expression handed to it (`StmtFallback`,
//! `EvalExprFallback`) must carry every such variable it names — the executor
//! binds them in a frame around the fallback and copies them back after it.
//!
//! This walker finds those names from the syntax, and at the same time every
//! construct that makes carrying unsound: control transfers that would leave
//! the fallback (`return`, `disable` of an outer scope, a `break` with no
//! loop around it), anything that can suspend, and constructs that bind
//! names implicitly. The caller refuses the fallback when `bad` is set, which
//! leaves the enclosing unit on the interpreter exactly as before.

use crate::ast::expr::*;
use crate::ast::stmt::*;

/// Name use flags (`FbScan::names`).
pub(crate) const READ: u8 = 0;
/// Used with a bit/part/element select.
pub(crate) const SEL: u8 = 1;
/// Possibly written: an assignment target, an increment operand, or an
/// argument that may bind to an `output`/`ref` formal.
pub(crate) const WRITE: u8 = 2;
/// A segment of a hierarchical name, or the base of a member access — the
/// interpreter resolves those by name through tables a register never enters.
pub(crate) const HIER: u8 = 4;
/// The target of a nonblocking assignment.
pub(crate) const NBA: u8 = 8;
/// An argument of `$strobe`/`$monitor`: evaluated after the statement
/// (end of time step, or on every later change), when no carried frame
/// exists any more.
pub(crate) const DEFER: u8 = 16;

/// Per-argument "is an input formal" for a user subroutine named by a call,
/// or None when the callee is unknown (every argument then counts as
/// possibly written).
pub(crate) type CalleeInputs<'a> = &'a dyn Fn(&str) -> Option<Vec<bool>>;

#[derive(Default)]
pub(crate) struct FbScan<'a> {
    /// Every simple name the fragment uses, with its flags OR-ed together.
    pub names: Vec<(String, u8)>,
    /// Names the fragment itself declares (`VarDecl`, `for (int i ...)`,
    /// `foreach` variables).
    pub decls: Vec<String>,
    /// Why carrying locals across this fragment is unsound, if it is.
    pub bad: Option<&'static str>,
    /// Formal directions of the subroutines the fragment calls.
    pub callee_inputs: Option<CalleeInputs<'a>>,
    loop_depth: u32,
    /// Inside the arguments of a deferred system task (see `DEFER`).
    defer: u32,
    blocks: Vec<String>,
}

/// Built-in methods whose arguments are all inputs (§7.9, §7.10, §7.12,
/// §6.16): any other method may write one (`first(k)`, `next(k)`, ...).
fn method_reads_args(name: &str) -> bool {
    matches!(
        name,
        "exists"
            | "delete"
            | "num"
            | "size"
            | "push_back"
            | "push_front"
            | "insert"
            | "len"
            | "getc"
            | "substr"
            | "toupper"
            | "tolower"
            | "compare"
            | "icompare"
            | "atoi"
            | "atohex"
            | "atooct"
            | "atobin"
            | "atoreal"
    )
}

/// System calls that never write through an argument. Any other system call
/// is assumed to be able to (`$sscanf`, `$cast`, `$value$plusargs`, a
/// `$random` seed, ...).
fn syscall_reads_only(name: &str) -> bool {
    if name.starts_with("$__xz") {
        return true;
    }
    let base = name.trim_end_matches(['b', 'h', 'o']);
    matches!(
        base,
        "$display"
            | "$write"
            | "$strobe"
            | "$monitor"
            | "$fdisplay"
            | "$fwrite"
            | "$fstrobe"
            | "$fmonitor"
    ) || matches!(
        name,
        "$time"
            | "$stime"
            | "$realtime"
            | "$bits"
            | "$signed"
            | "$unsigned"
            | "$clog2"
            | "$size"
            | "$left"
            | "$right"
            | "$low"
            | "$high"
            | "$increment"
            | "$dimensions"
            | "$unpacked_dimensions"
            | "$countones"
            | "$countbits"
            | "$onehot"
            | "$onehot0"
            | "$isunknown"
            | "$itor"
            | "$rtoi"
            | "$bitstoreal"
            | "$realtobits"
            | "$bitstoshortreal"
            | "$shortrealtobits"
            | "$sformatf"
            | "$psprintf"
            | "$info"
            | "$warning"
            | "$error"
            | "$fatal"
            | "$finish"
            | "$stop"
            | "$exit"
            | "$test$plusargs"
            | "$urandom_range"
            | "$typename"
            | "$ln"
            | "$log10"
            | "$exp"
            | "$sqrt"
            | "$pow"
            | "$floor"
            | "$ceil"
            | "$sin"
            | "$cos"
            | "$tan"
            | "$asin"
            | "$acos"
            | "$atan"
            | "$atan2"
            | "$hypot"
            | "$sinh"
            | "$cosh"
            | "$tanh"
            | "$fflush"
            | "$fclose"
    )
}

impl<'a> FbScan<'a> {
    pub(crate) fn new(callee_inputs: Option<CalleeInputs<'a>>) -> Self {
        FbScan {
            callee_inputs,
            ..Default::default()
        }
    }

    /// The result of a finished scan, without the callee lookup it borrowed.
    pub(crate) fn detach(self) -> FbScan<'static> {
        FbScan {
            names: self.names,
            decls: self.decls,
            bad: self.bad,
            ..Default::default()
        }
    }
}

impl FbScan<'_> {
    fn note(&mut self, name: &str, flags: u8) {
        let flags = if self.defer > 0 { flags | DEFER } else { flags };
        if let Some(e) = self.names.iter_mut().find(|(n, _)| n == name) {
            e.1 |= flags;
        } else {
            self.names.push((name.to_string(), flags));
        }
    }

    fn refuse(&mut self, why: &'static str) {
        if self.bad.is_none() {
            self.bad = Some(why);
        }
    }

    fn hier(&mut self, h: &HierarchicalIdentifier, flags: u8) {
        if h.path.len() == 1 && h.root.is_none() {
            let seg = &h.path[0];
            let sel = if seg.selects.is_empty() { 0 } else { SEL };
            self.note(&seg.name.name, flags | sel);
        } else if let Some(head) = h.path.first() {
            // Only the head of a dotted name can be a local: the later
            // segments are members of what it names (`this.name`, `h.f`),
            // so a register local that shares a member's name is not used.
            // A select applied to the name (`foreach (p.q[i])`) selects
            // the last segment, not the head.
            let sel = if head.selects.is_empty() { 0 } else { SEL };
            self.note(&head.name.name, (flags & !SEL) | sel | HIER);
        }
        for seg in &h.path {
            for s in &seg.selects {
                self.expr(s);
            }
        }
    }

    /// An expression in an assignment-target position.
    pub(crate) fn lvalue(&mut self, e: &Expression, flags: u8) {
        match &e.kind {
            ExprKind::Ident(h) => self.hier(h, WRITE | flags),
            ExprKind::Index { expr, index } => {
                self.lvalue(expr, SEL | flags);
                self.expr(index);
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                self.lvalue(expr, SEL | flags);
                self.expr(left);
                self.expr(right);
            }
            ExprKind::MemberAccess { expr, .. } => {
                self.lvalue(expr, HIER | flags);
            }
            ExprKind::Paren(inner) => self.lvalue(inner, flags),
            ExprKind::Concatenation(parts) => {
                for p in parts {
                    self.lvalue(p, flags);
                }
            }
            ExprKind::StreamOp {
                slice_size, exprs, ..
            } => {
                if let Some(s) = slice_size {
                    self.expr(s);
                }
                for p in exprs {
                    self.lvalue(p, flags);
                }
            }
            ExprKind::AssignmentPattern(items) => {
                for it in items {
                    self.lvalue(it.expr(), flags);
                }
            }
            _ => self.expr(e),
        }
    }

    /// An expression whose value is used; `flags` qualifies the use of a
    /// bare name at the top (`SEL` for a select's base, ...).
    fn expr_with(&mut self, e: &Expression, flags: u8) {
        match &e.kind {
            ExprKind::Number(_)
            | ExprKind::StringLiteral(_)
            | ExprKind::TypeLiteral(_)
            | ExprKind::Dollar
            | ExprKind::Null
            | ExprKind::This
            | ExprKind::Empty => {}
            ExprKind::Ident(h) => self.hier(h, flags),
            ExprKind::Unary { op, operand } => match op {
                UnaryOp::PreIncr | UnaryOp::PreDecr | UnaryOp::PostIncr | UnaryOp::PostDecr => {
                    self.lvalue(operand, 0)
                }
                UnaryOp::HashHash | UnaryOp::SEventually | UnaryOp::SAlways => {
                    self.refuse("fb_sva")
                }
                _ => self.expr(operand),
            },
            ExprKind::Binary { op, left, right } => match op {
                BinaryOp::Assign => {
                    self.lvalue(left, 0);
                    self.expr(right);
                }
                BinaryOp::OrMinusArrow
                | BinaryOp::OrFatArrow
                | BinaryOp::HashHash
                | BinaryOp::Iff
                | BinaryOp::Throughout
                | BinaryOp::Within
                | BinaryOp::Intersect
                | BinaryOp::SeqAnd
                | BinaryOp::SeqOr
                | BinaryOp::Until
                | BinaryOp::SUntil
                | BinaryOp::SvaAnd
                | BinaryOp::SvaDisableIff => self.refuse("fb_sva"),
                _ => {
                    self.expr(left);
                    self.expr(right);
                }
            },
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.expr(condition);
                self.expr(then_expr);
                self.expr(else_expr);
            }
            ExprKind::Concatenation(parts) => {
                for p in parts {
                    self.expr(p);
                }
            }
            ExprKind::Replication { count, exprs } => {
                self.expr(count);
                for p in exprs {
                    self.expr(p);
                }
            }
            ExprKind::AssignmentPattern(items) => {
                for it in items {
                    match it {
                        AssignmentPatternItem::Keyed(k, v) => {
                            self.expr(k);
                            self.expr(v);
                        }
                        other => self.expr(other.expr()),
                    }
                }
            }
            ExprKind::Call { func, args } => {
                // The callee is a subroutine name, not a variable — except a
                // method call through a variable (`s.len()`), whose receiver
                // the interpreter resolves by name.
                let mut inputs: Option<Vec<bool>> = None;
                match &func.kind {
                    ExprKind::Ident(h) => {
                        if h.path.len() > 1 {
                            self.note(&h.path[0].name.name, HIER);
                            let m = &h.path[h.path.len() - 1].name.name;
                            if method_reads_args(m) {
                                inputs = Some(vec![true; args.len()]);
                            }
                        } else if h.root.is_none() {
                            inputs = self.callee_inputs.and_then(|f| f(&h.path[0].name.name));
                        }
                        for seg in &h.path {
                            for s in &seg.selects {
                                self.expr(s);
                            }
                        }
                    }
                    ExprKind::MemberAccess { member, .. } => {
                        if method_reads_args(&member.name) {
                            inputs = Some(vec![true; args.len()]);
                        }
                        self.expr_with(func, HIER);
                    }
                    _ => self.expr_with(func, HIER),
                }
                // An argument may bind to an `output`/`ref` formal.
                for (i, a) in args.iter().enumerate() {
                    let input = matches!(&a.kind, ExprKind::NamedArg { .. })
                        .then_some(false)
                        .or_else(|| inputs.as_ref().and_then(|v| v.get(i).copied()))
                        .unwrap_or(false);
                    if input {
                        self.expr(a);
                    } else {
                        self.call_arg(a);
                    }
                }
            }
            ExprKind::SystemCall { name, args } => {
                if matches!(
                    name.as_str(),
                    "$past" | "$rose" | "$fell" | "$stable" | "$changed" | "$sampled"
                ) {
                    self.refuse("fb_sampled");
                }
                let deferred = {
                    let base = name.trim_end_matches(['b', 'h', 'o']);
                    matches!(base, "$strobe" | "$monitor" | "$fstrobe" | "$fmonitor")
                };
                if deferred {
                    self.defer += 1;
                    for a in args {
                        self.expr(a);
                    }
                    self.defer -= 1;
                } else if syscall_reads_only(name) {
                    for a in args {
                        self.expr(a);
                    }
                } else {
                    for a in args {
                        self.call_arg(a);
                    }
                }
            }
            ExprKind::NamedArg { expr, .. } => {
                if let Some(x) = expr {
                    self.call_arg(x);
                }
            }
            ExprKind::Inside { expr, ranges } => {
                self.expr(expr);
                for r in ranges {
                    self.expr(r);
                }
            }
            ExprKind::MemberAccess { expr, .. } => {
                self.expr_with(expr, HIER);
            }
            ExprKind::Specialization { base, .. } => self.expr_with(base, HIER),
            ExprKind::Index { expr, index } => {
                self.expr_with(expr, SEL | flags);
                self.expr(index);
            }
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                self.expr_with(expr, SEL | flags);
                self.expr(left);
                self.expr(right);
            }
            ExprKind::Range(a, b) => {
                self.expr(a);
                self.expr(b);
            }
            ExprKind::Paren(inner) => self.expr_with(inner, flags),
            ExprKind::AssignExpr { lvalue, rvalue } => {
                self.lvalue(lvalue, 0);
                self.expr(rvalue);
            }
            ExprKind::StreamOp {
                slice_size, exprs, ..
            } => {
                if let Some(s) = slice_size {
                    self.expr(s);
                }
                for p in exprs {
                    self.expr(p);
                }
            }
            ExprKind::Tagged { inner, .. } => {
                if let Some(i) = inner {
                    self.expr(i);
                }
            }
            ExprKind::ShallowCopy { source } => self.expr(source),
            // Implicit iterator / pattern / constraint names.
            ExprKind::WithClause { .. } => self.refuse("fb_with_clause"),
            ExprKind::RandomizeWith { .. } => self.refuse("fb_randomize_with"),
            ExprKind::Matches { .. } => self.refuse("fb_matches"),
            ExprKind::SvaClocked { .. } => self.refuse("fb_sva"),
        }
    }

    pub(crate) fn expr(&mut self, e: &Expression) {
        self.expr_with(e, READ);
    }

    fn call_arg(&mut self, a: &Expression) {
        match &a.kind {
            ExprKind::Ident(_)
            | ExprKind::Index { .. }
            | ExprKind::RangeSelect { .. }
            | ExprKind::MemberAccess { .. }
            | ExprKind::Concatenation(_) => self.lvalue(a, 0),
            _ => self.expr(a),
        }
    }

    fn body(&mut self, s: &Statement, is_loop: bool) {
        if is_loop {
            self.loop_depth += 1;
        }
        self.stmt(s);
        if is_loop {
            self.loop_depth -= 1;
        }
    }

    pub(crate) fn stmt(&mut self, s: &Statement) {
        if self.bad.is_some() {
            return;
        }
        match &s.kind {
            StatementKind::Null => {}
            StatementKind::Expr(e) => self.expr(e),
            StatementKind::BlockingAssign { lvalue, rvalue } => {
                if let ExprKind::SystemCall { name, .. } = &rvalue.kind {
                    if name == crate::intra_delay::INTRA_DELAY_MARKER
                        || name == crate::intra_delay::INTRA_EVENT_MARKER
                        || name == crate::intra_delay::INTRA_CYCLE_MARKER
                    {
                        // `x = #d y` suspends the process.
                        self.refuse("fb_blocking_intra_timing");
                    }
                }
                self.lvalue(lvalue, 0);
                self.expr(rvalue);
            }
            StatementKind::NonblockingAssign {
                lvalue,
                delay,
                rvalue,
            } => {
                self.lvalue(lvalue, NBA);
                if let Some(d) = delay {
                    self.expr(d);
                }
                self.expr(rvalue);
            }
            StatementKind::If {
                condition,
                then_stmt,
                else_stmt,
                ..
            } => {
                self.expr(condition);
                self.stmt(then_stmt);
                if let Some(e) = else_stmt {
                    self.stmt(e);
                }
            }
            StatementKind::Case { expr, items, .. } => {
                self.expr(expr);
                for it in items {
                    if it.pattern.is_some() {
                        self.refuse("fb_matches");
                    }
                    for p in &it.patterns {
                        self.expr(p);
                    }
                    self.stmt(&it.stmt);
                }
            }
            StatementKind::For {
                init,
                condition,
                step,
                body,
            } => {
                for i in init {
                    match i {
                        ForInit::VarDecl { name, init, .. } => {
                            self.decls.push(name.name.clone());
                            self.expr(init);
                        }
                        ForInit::Assign { lvalue, rvalue } => {
                            self.lvalue(lvalue, 0);
                            self.expr(rvalue);
                        }
                    }
                }
                if let Some(c) = condition {
                    self.expr(c);
                }
                for st in step {
                    self.expr(st);
                }
                self.body(body, true);
            }
            StatementKind::Foreach { array, vars, body } => {
                self.expr_with(array, SEL);
                for v in vars.iter().flatten() {
                    self.decls.push(v.name.clone());
                }
                self.body(body, true);
            }
            StatementKind::While { condition, body } => {
                self.expr(condition);
                self.body(body, true);
            }
            StatementKind::DoWhile { body, condition } => {
                self.body(body, true);
                self.expr(condition);
            }
            StatementKind::Repeat { count, body } => {
                self.expr(count);
                self.body(body, true);
            }
            StatementKind::SeqBlock { name, stmts } => {
                let depth = self.blocks.len();
                if let Some(n) = name {
                    self.blocks.push(n.name.clone());
                }
                for st in stmts {
                    self.stmt(st);
                }
                self.blocks.truncate(depth);
            }
            StatementKind::VarDecl { declarators, .. } => {
                for d in declarators {
                    self.decls.push(d.name.name.clone());
                    if let Some(i) = &d.init {
                        self.expr(i);
                    }
                }
            }
            StatementKind::RandCase { items } => {
                for (w, st) in items {
                    self.expr(w);
                    self.stmt(st);
                }
            }
            StatementKind::EventTrigger { name, target, .. } => {
                self.note(&name.name, READ);
                if let Some(t) = target {
                    self.expr(t);
                }
            }
            // A `disable` of a block declared inside the fragment stays
            // inside it; anything else (an enclosing block, the task being
            // inlined, another task) is a transfer the compiled code around
            // the fallback would never see.
            StatementKind::Disable(id) => {
                if !self.blocks.iter().any(|b| *b == id.name) {
                    self.refuse("fb_disable_outer");
                }
            }
            StatementKind::Break | StatementKind::Continue => {
                if self.loop_depth == 0 {
                    self.refuse("fb_break_outer");
                }
            }
            StatementKind::Return(_) => self.refuse("fb_return"),
            StatementKind::TimingControl { .. }
            | StatementKind::Wait { .. }
            | StatementKind::WaitFork
            | StatementKind::WaitOrder { .. }
            | StatementKind::ParBlock { .. }
            | StatementKind::Forever { .. }
            | StatementKind::ForeverTail { .. }
            | StatementKind::DisableFork => self.refuse("fb_suspends"),
            StatementKind::Assertion(_)
            | StatementKind::ProceduralContinuous(_)
            | StatementKind::Typedef(_)
            | StatementKind::Coverpoint { .. }
            | StatementKind::Cross { .. }
            | StatementKind::RsAction { .. }
            | StatementKind::RsReturn
            | StatementKind::ScopePop
            | StatementKind::LoopStep
            | StatementKind::ForeachTail { .. } => self.refuse("fb_unsupported_stmt"),
        }
    }
}
