//! Code coverage: statement, branch and toggle counts, collected when the run
//! asks for them (`--code-coverage`, `+cover`) and written to the results
//! file beside the functional coverage.
//!
//! Statements and branches are instrumented at compile time, on the
//! elaborated design, before any block is lowered: a counted statement gets a
//! `$__xz_cov(<n>)` call in front of it, a branch arm one at its start, and a
//! `?:` condition is wrapped in `$__xz_covc(<n>, cond)`. The bytecode
//! compiler turns those into `Insn::CovHit`; the interpreter counts them
//! itself. Every execution path therefore counts the same way, and a run
//! without code coverage compiles exactly the design it did before.
//! Continuous assignments count in their comb entry (`ca_counter`). Toggles
//! compare each tracked signal with its value at the end of the previous
//! time slot, off the dirty set the waveform writers share; a slot sampled
//! more than once (once per tick and at its end) counts only its net change.
use super::*;
use crate::ast::Span;
use crate::compiler::elaborate::InitialBlock;

pub const KIND_STATEMENT: u8 = 1;
pub const KIND_BRANCH: u8 = 2;
pub const KIND_TOGGLE: u8 = 4;
/// Result names of the kinds, bit `i` of a kind mask naming entry `i`.
const KIND_NAMES: [&str; 3] = ["statement", "branch", "toggle"];
type Tallies = [Tally; KIND_NAMES.len()];

/// `$__xz_cov(<n>)`: counts counter `n` (a statement or a branch arm).
pub(crate) const HIT_TASK: &str = "$__xz_cov";
/// `$__xz_covc(<n>, cond)`: yields `cond`, counting counter `n` when it is
/// true and `n + 1` when it is false (a `?:` condition).
pub(crate) const COND_FN: &str = "$__xz_covc";

/// Library packages left out unless `--code-coverage-scope` names them.
const LIBRARY_PACKAGES: [&str; 2] = ["std", "uvm_pkg"];

/// What the command line asked for. `scopes` limits every kind to the
/// instances at or below the listed paths and to the listed packages (empty:
/// the whole design).
#[derive(Clone, Debug, Default)]
pub struct CodeCoverage {
    pub kinds: u8,
    pub scopes: Vec<String>,
}

static CONFIG: Mutex<Option<CodeCoverage>> = Mutex::new(None);

/// Select code coverage for the simulators built after this call; `None` or
/// no kinds turns it off.
pub fn set_code_coverage(cfg: Option<CodeCoverage>) {
    if let Ok(mut c) = CONFIG.lock() {
        *c = cfg.filter(|c| c.kinds != 0);
    }
}

pub(crate) fn code_coverage() -> Option<CodeCoverage> {
    CONFIG.lock().ok().and_then(|c| c.clone())
}

/// The kinds a `--code-coverage` list names: `stmt` (or `statement`, `s`),
/// `branch` (`b`), `toggle` (`t`) or `all`, separated by commas.
pub fn parse_kinds(spec: &str) -> Result<u8, String> {
    let mut kinds = 0u8;
    for k in spec.split(',').map(str::trim).filter(|k| !k.is_empty()) {
        kinds |= match k {
            "stmt" | "statement" | "s" => KIND_STATEMENT,
            "branch" | "b" => KIND_BRANCH,
            "toggle" | "t" => KIND_TOGGLE,
            "all" => KIND_STATEMENT | KIND_BRANCH | KIND_TOGGLE,
            _ => {
                return Err(format!(
                    "unknown code coverage kind '{}' (want stmt, branch, toggle or all)",
                    k
                ));
            }
        };
    }
    if kinds == 0 {
        return Err("--code-coverage needs at least one kind".to_string());
    }
    Ok(kinds)
}

/// The counter a coverage marker's first argument carries.
pub(crate) fn marker_id(e: &Expression) -> Option<u32> {
    match &e.kind {
        ExprKind::Number(NumberLiteral::Integer { value, .. }) => value.parse().ok(),
        _ => None,
    }
}

fn number(n: u32, span: Span) -> Expression {
    Expression::new(
        ExprKind::Number(NumberLiteral::Integer {
            size: None,
            signed: false,
            base: NumberBase::Decimal,
            value: n.to_string(),
            cached_val: Cell::new(None),
        }),
        span,
    )
}

fn hit(counter: u32, span: Span) -> Statement {
    Statement::new(
        StatementKind::Expr(Expression::new(
            ExprKind::SystemCall {
                name: HIT_TASK.to_string(),
                args: vec![number(counter, span)],
            },
            span,
        )),
        span,
    )
}

/// One statement list as a single statement.
fn block(mut v: Vec<Statement>, span: Span) -> Statement {
    if v.len() == 1 {
        v.pop().unwrap()
    } else {
        Statement::new(
            StatementKind::SeqBlock {
                name: None,
                stmts: v,
            },
            span,
        )
    }
}

/// A reported scope: a module instance, or the package (or `$unit`, or the
/// declaring module) that holds classes and subroutines outside instances.
#[derive(serde::Serialize)]
struct Scope {
    /// Printed name: the hierarchical instance path, or the package name.
    name: String,
    unit: String,
    kind: &'static str,
    src: Option<u32>,
}

#[derive(serde::Serialize)]
struct StmtSite {
    scope: u32,
    span: Span,
    kind: &'static str,
    counter: u32,
}

#[derive(serde::Serialize)]
struct Arm {
    span: Span,
    label: &'static str,
    /// `None` for the implicit `else` / `default`: the branch's entry count
    /// minus the other arms.
    counter: Option<u32>,
}

#[derive(serde::Serialize)]
struct BranchSite {
    scope: u32,
    span: Span,
    kind: &'static str,
    entry: Option<u32>,
    arms: Vec<Arm>,
}

/// One 64-bit word of a tracked signal, as toggle coverage last saw it.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
struct Word {
    /// Value at the last sample.
    v: u64,
    x: u64,
    /// Time of the last change, `u64::MAX` before the first.
    at: u64,
    /// Value at the end of the slot before that change: the slot's toggles
    /// count from it.
    bv: u64,
    bx: u64,
}

/// Per-bit rise/fall counts of the tracked signals.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Toggle {
    /// Signal id -> tracked index, `u32::MAX` when not tracked.
    slot: Vec<u32>,
    ids: Vec<u32>,
    width: Vec<u32>,
    /// First word of each tracked signal in `words`.
    word0: Vec<u32>,
    /// First bit of each tracked signal in `rise` / `fall`.
    bit0: Vec<u32>,
    words: Vec<Word>,
    rise: Vec<u32>,
    fall: Vec<u32>,
    /// (scope, name within the scope, tracked index); aliases of one signal
    /// share the tracked index.
    names: Vec<(u32, String, u32)>,
}

impl Toggle {
    /// Sample word `word` of tracked signal `k` at time `now`. A slot can be
    /// sampled more than once (every tick flushes, and so does the end of
    /// the slot); the counts hold the change from the end of the previous
    /// slot to the latest sample, so a value changed and changed back within
    /// one slot does not toggle.
    fn sample(&mut self, k: usize, v: u64, x: u64, word: usize, now: u64) {
        let w = self.word0[k] as usize + word;
        let s = &mut self.words[w];
        let (pv, px) = (s.v, s.x);
        if v == pv && x == px {
            return;
        }
        s.v = v;
        s.x = x;
        let base = self.bit0[k] as usize + word * 64;
        if s.at == now {
            self.resample(w, base, pv, px);
            return;
        }
        // The slot's first change: the previous sample is the value at the
        // end of the previous slot.
        s.at = now;
        s.bv = pv;
        s.bx = px;
        let known = !(x | px);
        let mut rise = v & !pv & known;
        let mut fall = pv & !v & known;
        while rise != 0 {
            let b = rise.trailing_zeros() as usize;
            self.rise[base + b] = self.rise[base + b].saturating_add(1);
            rise &= rise - 1;
        }
        while fall != 0 {
            let b = fall.trailing_zeros() as usize;
            self.fall[base + b] = self.fall[base + b].saturating_add(1);
            fall &= fall - 1;
        }
    }

    /// Word `w` changed again in the slot it last changed in: move its
    /// counts from the transition counted so far (baseline to `pv`/`px`)
    /// to the one from the baseline to its new value.
    #[cold]
    fn resample(&mut self, w: usize, base: usize, pv: u64, px: u64) {
        let s = self.words[w];
        let (v, x, bv, bx) = (s.v, s.x, s.bv, s.bx);
        let was = !(bx | px);
        let known = !(bx | x);
        Self::recount(&mut self.rise, base, pv & !bv & was, v & !bv & known);
        Self::recount(&mut self.fall, base, bv & !pv & was, bv & !v & known);
    }

    /// Move the counts of bits `bit..bit + 64` from the transitions in `old`
    /// to those in `new`.
    fn recount(n: &mut [u32], bit: usize, old: u64, new: u64) {
        let mut add = new & !old;
        while add != 0 {
            let b = bit + add.trailing_zeros() as usize;
            n[b] = n[b].saturating_add(1);
            add &= add - 1;
        }
        let mut sub = old & !new;
        while sub != 0 {
            let b = bit + sub.trailing_zeros() as usize;
            n[b] = n[b].saturating_sub(1);
            sub &= sub - 1;
        }
    }
}

#[derive(serde::Serialize)]
pub(crate) struct CodeCov {
    kinds: u8,
    limit: Vec<String>,
    scopes: Vec<Scope>,
    scope_index: HashMap<String, u32>,
    stmts: Vec<StmtSite>,
    branches: Vec<BranchSite>,
    counters: u32,
    /// Continuous assignment (origin span start, end, scope) -> its counter.
    ca_counter: HashMap<(usize, usize, String), u32>,
    /// Instance paths below the top.
    inst_paths: HashSet<String>,
    pub(crate) toggle: Option<Toggle>,
}

/// Rewrites one scope's statements; see the module docs.
struct Instr<'a> {
    cov: &'a mut CodeCov,
    kinds: u8,
    scope: u32,
}

impl Instr<'_> {
    fn counter(&mut self) -> u32 {
        let c = self.cov.counters;
        self.cov.counters += 1;
        c
    }

    fn count(&mut self, span: Span, kind: &'static str, out: &mut Vec<Statement>) {
        if self.kinds & KIND_STATEMENT == 0 {
            return;
        }
        let counter = self.counter();
        self.cov.stmts.push(StmtSite {
            scope: self.scope,
            span,
            kind,
            counter,
        });
        out.push(hit(counter, span));
    }

    /// A statement in a single-statement position.
    fn wrap(&mut self, s: Statement) -> Statement {
        let span = s.span;
        let mut v = Vec::new();
        self.stmt(s, &mut v);
        block(v, span)
    }

    /// A branch arm: its counter, then the arm's own statements.
    fn arm(&mut self, s: Statement, counter: u32) -> Statement {
        let span = s.span;
        let mut v = vec![hit(counter, span)];
        self.stmt(s, &mut v);
        block(v, span)
    }

    fn stmts(&mut self, items: Vec<Statement>) -> Vec<Statement> {
        let mut v = Vec::with_capacity(items.len() * 2);
        for s in items {
            self.stmt(s, &mut v);
        }
        v
    }

    /// An `always` body: the construct itself counts each time the body
    /// starts (after its leading event or delay control, if any), and the
    /// body's statements count as usual.
    fn always(&mut self, s: Statement, kind: &'static str) -> Statement {
        let span = s.span;
        let mut v = Vec::new();
        match s.kind {
            StatementKind::TimingControl {
                control,
                stmt: inner,
            } => {
                self.count(span, kind, &mut v);
                self.stmt(*inner, &mut v);
                Statement::new(
                    StatementKind::TimingControl {
                        control,
                        stmt: Box::new(block(v, span)),
                    },
                    span,
                )
            }
            other => {
                self.count(span, kind, &mut v);
                self.stmt(Statement::new(other, span), &mut v);
                block(v, span)
            }
        }
    }

    fn stmt(&mut self, s: Statement, out: &mut Vec<Statement>) {
        let span = s.span;
        let rebuilt = |kind| Statement::new(kind, span);
        match s.kind {
            StatementKind::SeqBlock { name, stmts } => {
                let stmts = self.stmts(stmts);
                out.push(rebuilt(StatementKind::SeqBlock { name, stmts }));
            }
            // Each fork branch stays one process.
            StatementKind::ParBlock {
                name,
                join_type,
                stmts,
            } => {
                let stmts = stmts.into_iter().map(|c| self.wrap(c)).collect();
                out.push(rebuilt(StatementKind::ParBlock {
                    name,
                    join_type,
                    stmts,
                }));
            }
            StatementKind::If {
                unique_priority,
                condition,
                then_stmt,
                else_stmt,
            } => self.if_stmt(unique_priority, condition, then_stmt, else_stmt, span, out),
            StatementKind::Case {
                unique_priority,
                kind,
                expr,
                items,
            } => self.case_stmt(unique_priority, kind, expr, items, span, out),
            StatementKind::RandCase { items } => {
                let items = if self.kinds & KIND_BRANCH != 0 {
                    let mut arms = Vec::new();
                    let items = items
                        .into_iter()
                        .map(|(w, st)| {
                            let c = self.counter();
                            arms.push(Arm {
                                span: st.span,
                                label: "item",
                                counter: Some(c),
                            });
                            (w, self.arm(st, c))
                        })
                        .collect();
                    self.cov.branches.push(BranchSite {
                        scope: self.scope,
                        span,
                        kind: "randcase",
                        entry: None,
                        arms,
                    });
                    items
                } else {
                    items
                        .into_iter()
                        .map(|(w, st)| (w, self.wrap(st)))
                        .collect()
                };
                out.push(rebuilt(StatementKind::RandCase { items }));
            }
            StatementKind::For {
                init,
                condition,
                step,
                body,
            } => {
                self.count(span, "for", out);
                let body = Box::new(self.wrap(*body));
                out.push(rebuilt(StatementKind::For {
                    init,
                    condition,
                    step,
                    body,
                }));
            }
            StatementKind::Foreach { array, vars, body } => {
                self.count(span, "foreach", out);
                let body = Box::new(self.wrap(*body));
                out.push(rebuilt(StatementKind::Foreach { array, vars, body }));
            }
            StatementKind::While { condition, body } => {
                self.count(span, "while", out);
                let body = Box::new(self.wrap(*body));
                out.push(rebuilt(StatementKind::While { condition, body }));
            }
            StatementKind::DoWhile { body, condition } => {
                self.count(span, "do", out);
                let body = Box::new(self.wrap(*body));
                out.push(rebuilt(StatementKind::DoWhile { body, condition }));
            }
            StatementKind::Repeat { count, body } => {
                self.count(span, "repeat", out);
                let body = Box::new(self.wrap(*body));
                out.push(rebuilt(StatementKind::Repeat { count, body }));
            }
            StatementKind::Forever { body } => {
                self.count(span, "forever", out);
                let body = Box::new(self.wrap(*body));
                out.push(rebuilt(StatementKind::Forever { body }));
            }
            StatementKind::TimingControl {
                control,
                stmt: inner,
            } => {
                let kind = match &control {
                    TimingControl::Delay(_) => "delay",
                    TimingControl::Event(_) => "event",
                };
                self.count(span, kind, out);
                let inner = Box::new(self.wrap(*inner));
                out.push(rebuilt(StatementKind::TimingControl {
                    control,
                    stmt: inner,
                }));
            }
            StatementKind::Wait {
                condition,
                stmt: inner,
            } => {
                self.count(span, "wait", out);
                let inner = Box::new(self.wrap(*inner));
                out.push(rebuilt(StatementKind::Wait {
                    condition,
                    stmt: inner,
                }));
            }
            // Not statements of their own: declarations, and the markers and
            // bodies the executor and other constructs own.
            kind @ (StatementKind::Null
            | StatementKind::VarDecl { .. }
            | StatementKind::Typedef(_)
            | StatementKind::Coverpoint { .. }
            | StatementKind::Cross { .. }
            | StatementKind::ScopePop
            | StatementKind::LoopStep
            | StatementKind::ForeachTail { .. }
            | StatementKind::ForeverTail { .. }
            | StatementKind::RsAction { .. }
            | StatementKind::RsReturn) => out.push(rebuilt(kind)),
            StatementKind::BlockingAssign { lvalue, mut rvalue } => {
                self.count(span, "statement", out);
                self.expr(&mut rvalue);
                out.push(rebuilt(StatementKind::BlockingAssign { lvalue, rvalue }));
            }
            StatementKind::NonblockingAssign {
                lvalue,
                delay,
                mut rvalue,
            } => {
                self.count(span, "statement", out);
                self.expr(&mut rvalue);
                out.push(rebuilt(StatementKind::NonblockingAssign {
                    lvalue,
                    delay,
                    rvalue,
                }));
            }
            StatementKind::Expr(mut e) => {
                self.count(span, "statement", out);
                match &mut e.kind {
                    ExprKind::Call { args, .. } => {
                        for a in args.iter_mut() {
                            self.expr(a);
                        }
                    }
                    ExprKind::AssignExpr { rvalue, .. } => self.expr(rvalue),
                    ExprKind::Binary {
                        op: BinaryOp::Assign,
                        right,
                        ..
                    } => self.expr(right),
                    _ => {}
                }
                out.push(rebuilt(StatementKind::Expr(e)));
            }
            StatementKind::Return(Some(mut e)) => {
                self.count(span, "statement", out);
                self.expr(&mut e);
                out.push(rebuilt(StatementKind::Return(Some(e))));
            }
            kind => {
                self.count(span, "statement", out);
                out.push(rebuilt(kind));
            }
        }
    }

    /// An `if`, with its `else if` chain as one branch: one arm per
    /// condition plus the final `else`, written or implicit. A `unique` or
    /// `priority` chain has no implicit arm (no match is a violation).
    fn if_stmt(
        &mut self,
        unique_priority: Option<UniquePriority>,
        mut condition: Expression,
        then_stmt: Box<Statement>,
        else_stmt: Option<Box<Statement>>,
        span: Span,
        out: &mut Vec<Statement>,
    ) {
        if self.kinds & KIND_BRANCH == 0 {
            let then_stmt = Box::new(self.wrap(*then_stmt));
            let else_stmt = else_stmt.map(|e| Box::new(self.wrap(*e)));
            out.push(Statement::new(
                StatementKind::If {
                    unique_priority,
                    condition,
                    then_stmt,
                    else_stmt,
                },
                span,
            ));
            return;
        }
        let mut has_else = false;
        let mut tail = else_stmt.as_deref();
        while let Some(e) = tail {
            match &e.kind {
                StatementKind::If { else_stmt, .. } => tail = else_stmt.as_deref(),
                _ => {
                    has_else = true;
                    break;
                }
            }
        }
        let implicit = !has_else && unique_priority.is_none();
        let entry = implicit.then(|| self.counter());
        let site = self.cov.branches.len();
        self.cov.branches.push(BranchSite {
            scope: self.scope,
            span,
            kind: "if",
            entry,
            arms: Vec::new(),
        });
        let mut arms = Vec::new();
        self.expr(&mut condition);
        let rewritten = self.if_arms(
            unique_priority,
            condition,
            then_stmt,
            else_stmt,
            span,
            true,
            &mut arms,
        );
        if implicit {
            arms.push(Arm {
                span,
                label: "else (implicit)",
                counter: None,
            });
        }
        self.cov.branches[site].arms = arms;
        if let Some(e) = entry {
            out.push(hit(e, span));
        }
        out.push(rewritten);
    }

    #[allow(clippy::too_many_arguments)]
    fn if_arms(
        &mut self,
        unique_priority: Option<UniquePriority>,
        condition: Expression,
        then_stmt: Box<Statement>,
        else_stmt: Option<Box<Statement>>,
        span: Span,
        first: bool,
        arms: &mut Vec<Arm>,
    ) -> Statement {
        let c = self.counter();
        arms.push(Arm {
            span: then_stmt.span,
            label: if first { "if" } else { "else if" },
            counter: Some(c),
        });
        let then_stmt = Box::new(self.arm(*then_stmt, c));
        let else_stmt = else_stmt.map(|e| {
            let e = *e;
            let espan = e.span;
            Box::new(match e.kind {
                StatementKind::If {
                    unique_priority: up,
                    condition: mut cond,
                    then_stmt: t,
                    else_stmt: el,
                } => {
                    self.expr(&mut cond);
                    self.if_arms(up, cond, t, el, espan, false, arms)
                }
                kind => {
                    let c = self.counter();
                    arms.push(Arm {
                        span: espan,
                        label: "else",
                        counter: Some(c),
                    });
                    self.arm(Statement::new(kind, espan), c)
                }
            })
        });
        Statement::new(
            StatementKind::If {
                unique_priority,
                condition,
                then_stmt,
                else_stmt,
            },
            span,
        )
    }

    /// A `case`: one arm per item, plus an implicit `default` when there is
    /// none (except for `unique` / `priority` cases).
    fn case_stmt(
        &mut self,
        unique_priority: Option<UniquePriority>,
        kind: CaseKind,
        expr: Expression,
        mut items: Vec<CaseItem>,
        span: Span,
        out: &mut Vec<Statement>,
    ) {
        if self.kinds & KIND_BRANCH == 0 {
            for it in items.iter_mut() {
                let st = std::mem::replace(&mut it.stmt, Statement::new(StatementKind::Null, span));
                it.stmt = self.wrap(st);
            }
        } else {
            let implicit = !items.iter().any(|it| it.is_default) && unique_priority.is_none();
            let entry = implicit.then(|| self.counter());
            let site = self.cov.branches.len();
            self.cov.branches.push(BranchSite {
                scope: self.scope,
                span,
                kind: match kind {
                    CaseKind::Case => "case",
                    CaseKind::Casez => "casez",
                    CaseKind::Casex => "casex",
                    CaseKind::CaseInside => "case inside",
                },
                entry,
                arms: Vec::new(),
            });
            let mut arms = Vec::new();
            for it in items.iter_mut() {
                let c = self.counter();
                arms.push(Arm {
                    span: it.span,
                    label: if it.is_default { "default" } else { "item" },
                    counter: Some(c),
                });
                let st = std::mem::replace(&mut it.stmt, Statement::new(StatementKind::Null, span));
                it.stmt = self.arm(st, c);
            }
            if implicit {
                arms.push(Arm {
                    span,
                    label: "default (implicit)",
                    counter: None,
                });
            }
            self.cov.branches[site].arms = arms;
            if let Some(e) = entry {
                out.push(hit(e, span));
            }
        }
        out.push(Statement::new(
            StatementKind::Case {
                unique_priority,
                kind,
                expr,
                items,
            },
            span,
        ));
    }

    /// Wrap the condition of every `?:` in `e` (see `COND_FN`). Stops at
    /// system calls, whose arguments some tasks read by shape.
    fn expr(&mut self, e: &mut Expression) {
        if self.kinds & KIND_BRANCH == 0 {
            return;
        }
        let espan = e.span;
        match &mut e.kind {
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.expr(condition);
                self.expr(then_expr);
                self.expr(else_expr);
                let t = self.counter();
                let f = self.counter();
                self.cov.branches.push(BranchSite {
                    scope: self.scope,
                    span: espan,
                    kind: "ternary",
                    entry: None,
                    arms: vec![
                        Arm {
                            span: then_expr.span,
                            label: "true",
                            counter: Some(t),
                        },
                        Arm {
                            span: else_expr.span,
                            label: "false",
                            counter: Some(f),
                        },
                    ],
                });
                let cspan = condition.span;
                let cond =
                    std::mem::replace(&mut **condition, Expression::new(ExprKind::Empty, cspan));
                **condition = Expression::new(
                    ExprKind::SystemCall {
                        name: COND_FN.to_string(),
                        args: vec![number(t, cspan), cond],
                    },
                    cspan,
                );
            }
            ExprKind::Unary { operand, .. } => self.expr(operand),
            ExprKind::Binary { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            ExprKind::Paren(inner) => self.expr(inner),
            ExprKind::Concatenation(parts) => {
                for p in parts.iter_mut() {
                    self.expr(p);
                }
            }
            ExprKind::Replication { exprs, .. } => {
                for p in exprs.iter_mut() {
                    self.expr(p);
                }
            }
            ExprKind::Index { expr, index } => {
                self.expr(expr);
                self.expr(index);
            }
            ExprKind::Call { args, .. } => {
                for a in args.iter_mut() {
                    self.expr(a);
                }
            }
            _ => {}
        }
    }
}

/// Newline offsets of each retained source text, built on first use.
struct Lines<'a> {
    module: &'a ElaboratedModule,
    nl: HashMap<usize, Vec<usize>>,
}

impl Lines<'_> {
    /// File and line of `span`, in the text `src` names or else the only
    /// text long enough to contain it.
    fn locate(&mut self, span: Span, src: Option<u32>) -> Option<(String, u32)> {
        let texts = &self.module.source_texts;
        let i = match src
            .map(|s| s as usize)
            .filter(|&s| texts.get(s).is_some_and(|t| span.start < t.len()))
        {
            Some(i) => i,
            None => {
                let mut fits = texts
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| span.start < t.len());
                let (i, _) = fits.next()?;
                if fits.next().is_some() {
                    return None;
                }
                i
            }
        };
        let nl = self.nl.entry(i).or_insert_with(|| {
            texts[i]
                .bytes()
                .enumerate()
                .filter(|&(_, b)| b == b'\n')
                .map(|(p, _)| p)
                .collect()
        });
        let out_line = nl.partition_point(|&p| p < span.start);
        if let Some(m) = self.module.source_line_maps.get(i).and_then(|m| m.as_ref()) {
            if let Some((f, l)) = m.file_line(out_line) {
                return Some((f.to_string(), l));
            }
        }
        let file = self.module.source_files.get(i).filter(|f| !f.is_empty())?;
        Some((file.clone(), out_line as u32 + 1))
    }
}

/// One reported statement, arm or toggle row after locating and merging.
#[derive(Clone)]
struct StmtRow {
    file: String,
    line: u32,
    start: usize,
    kind: &'static str,
    count: u64,
}

#[derive(Clone)]
struct ArmRow {
    line: u32,
    label: &'static str,
    count: u64,
}

#[derive(Clone)]
struct BranchRow {
    file: String,
    line: u32,
    start: usize,
    kind: &'static str,
    arms: Vec<ArmRow>,
}

#[derive(Clone)]
struct ToggleRow {
    name: String,
    rise: Vec<u32>,
    fall: Vec<u32>,
}

#[derive(Default)]
struct Report {
    stmts: Vec<StmtRow>,
    branches: Vec<BranchRow>,
    toggles: Vec<ToggleRow>,
}

#[derive(Clone, Copy, Default)]
struct Tally {
    covered: u64,
    total: u64,
}

impl Tally {
    fn add(&mut self, o: Tally) {
        self.covered += o.covered;
        self.total += o.total;
    }

    fn json(&self) -> String {
        if self.total == 0 {
            "{\"covered\": 0, \"total\": 0, \"percent\": null}".to_string()
        } else {
            format!(
                "{{\"covered\": {}, \"total\": {}, \"percent\": {:.2}}}",
                self.covered,
                self.total,
                self.percent()
            )
        }
    }

    fn percent(&self) -> f64 {
        100.0 * self.covered as f64 / self.total.max(1) as f64
    }

    fn text(&self) -> String {
        if self.total == 0 {
            "0/0".to_string()
        } else {
            format!("{}/{} ({:.2}%)", self.covered, self.total, self.percent())
        }
    }
}

impl Report {
    fn tallies(&self) -> Tallies {
        let mut t = Tallies::default();
        for s in &self.stmts {
            t[0].total += 1;
            t[0].covered += (s.count > 0) as u64;
        }
        for b in &self.branches {
            for a in &b.arms {
                t[1].total += 1;
                t[1].covered += (a.count > 0) as u64;
            }
        }
        for r in &self.toggles {
            t[2].total += 2 * r.rise.len() as u64;
            t[2].covered += r.rise.iter().filter(|&&c| c > 0).count() as u64
                + r.fall.iter().filter(|&&c| c > 0).count() as u64;
        }
        t
    }

    /// Sort, and add up rows that name the same source construct (the
    /// copies a parameterized class or an expanded assignment makes).
    fn normalize(&mut self) {
        self.stmts.sort_by(|a, b| {
            (&a.file, a.line, a.start, a.kind).cmp(&(&b.file, b.line, b.start, b.kind))
        });
        let mut merged: Vec<StmtRow> = Vec::with_capacity(self.stmts.len());
        for s in self.stmts.drain(..) {
            match merged.last_mut() {
                Some(m) if m.file == s.file && m.start == s.start && m.kind == s.kind => {
                    m.count += s.count
                }
                _ => merged.push(s),
            }
        }
        self.stmts = merged;
        self.branches.sort_by(|a, b| {
            (&a.file, a.line, a.start, a.kind).cmp(&(&b.file, b.line, b.start, b.kind))
        });
        let mut merged: Vec<BranchRow> = Vec::with_capacity(self.branches.len());
        for b in self.branches.drain(..) {
            match merged.last_mut() {
                Some(m)
                    if m.file == b.file
                        && m.start == b.start
                        && m.kind == b.kind
                        && m.arms.len() == b.arms.len() =>
                {
                    for (x, y) in m.arms.iter_mut().zip(&b.arms) {
                        x.count += y.count;
                    }
                }
                _ => merged.push(b),
            }
        }
        self.branches = merged;
        self.toggles.sort_by(|a, b| a.name.cmp(&b.name));
    }

    /// Add another instance of the same design unit (rows keyed by source
    /// position and signal name).
    fn merge(&mut self, o: &Report) {
        self.stmts.extend(o.stmts.iter().cloned());
        self.branches.extend(o.branches.iter().cloned());
        let mut by_name: HashMap<String, usize> = self
            .toggles
            .iter()
            .enumerate()
            .map(|(i, r)| (r.name.clone(), i))
            .collect();
        for r in &o.toggles {
            match by_name.get(&r.name) {
                Some(&i) if self.toggles[i].rise.len() == r.rise.len() => {
                    let t = &mut self.toggles[i];
                    for (a, b) in t.rise.iter_mut().zip(&r.rise) {
                        *a = a.saturating_add(*b);
                    }
                    for (a, b) in t.fall.iter_mut().zip(&r.fall) {
                        *a = a.saturating_add(*b);
                    }
                }
                _ => {
                    by_name.insert(r.name.clone(), self.toggles.len());
                    self.toggles.push(r.clone());
                }
            }
        }
        self.normalize();
    }

    fn json(&self, kinds: u8, indent: &str, out: &mut String) {
        let t = self.tallies();
        for (i, name) in KIND_NAMES.iter().enumerate() {
            if kinds & (1 << i) != 0 {
                out.push_str(&format!("{}\"{}\": {},\n", indent, name, t[i].json()));
            }
        }
        if kinds & KIND_STATEMENT != 0 {
            out.push_str(&format!("{}\"statements\": [", indent));
            for (i, s) in self.stmts.iter().enumerate() {
                out.push_str(&format!(
                    "{}{{\"file\": \"{}\", \"line\": {}, \"kind\": \"{}\", \"count\": {}}}",
                    if i == 0 { "" } else { ", " },
                    json_escape(&s.file),
                    s.line,
                    s.kind,
                    s.count
                ));
            }
            out.push_str("],\n");
        }
        if kinds & KIND_BRANCH != 0 {
            out.push_str(&format!("{}\"branches\": [", indent));
            for (i, b) in self.branches.iter().enumerate() {
                out.push_str(&format!(
                    "{}{{\"file\": \"{}\", \"line\": {}, \"kind\": \"{}\", \"arms\": [",
                    if i == 0 { "" } else { ", " },
                    json_escape(&b.file),
                    b.line,
                    b.kind
                ));
                for (j, a) in b.arms.iter().enumerate() {
                    out.push_str(&format!(
                        "{}{{\"line\": {}, \"arm\": \"{}\", \"count\": {}}}",
                        if j == 0 { "" } else { ", " },
                        a.line,
                        a.label,
                        a.count
                    ));
                }
                out.push_str("]}");
            }
            out.push_str("],\n");
        }
        if kinds & KIND_TOGGLE != 0 {
            out.push_str(&format!("{}\"toggles\": [", indent));
            let list = |v: &[u32]| {
                v.iter()
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            for (i, r) in self.toggles.iter().enumerate() {
                out.push_str(&format!(
                    "{}{{\"signal\": \"{}\", \"width\": {}, \"rise\": [{}], \"fall\": [{}]}}",
                    if i == 0 { "" } else { ", " },
                    json_escape(&r.name),
                    r.rise.len(),
                    list(&r.rise),
                    list(&r.fall)
                ));
            }
            out.push_str("],\n");
        }
        // Drop the trailing comma of the last member.
        if out.ends_with(",\n") {
            out.truncate(out.len() - 2);
            out.push('\n');
        }
    }
}

impl Simulator {
    /// Instrument the design for the code coverage the command line asked
    /// for. Runs first in `compile`, before any block is lowered.
    pub(super) fn instrument_code_coverage(&mut self) {
        let Some(cfg) = code_coverage() else {
            return;
        };
        // Every always body is instrumented as a tree (normally none is
        // lazy under coverage; this covers a configuration set late).
        self.flush_deferred_always();
        let kinds = cfg.kinds;
        let inst_paths: HashSet<String> = self
            .module
            .instances
            .iter()
            .map(|i| i.path.clone())
            .collect();
        let mut cov = CodeCov {
            kinds,
            limit: cfg.scopes,
            scopes: Vec::new(),
            scope_index: HashMap::default(),
            stmts: Vec::new(),
            branches: Vec::new(),
            counters: 0,
            ca_counter: HashMap::default(),
            inst_paths: inst_paths.clone(),
            toggle: None,
        };
        // Initial blocks and continuous assignments are instrumented where
        // `compile` and `build_comb_entries` take them (`cov_initial`,
        // `cov_contassign`): most of them are still lazy here, and building
        // them early would change the order and the passes they go through.
        if kinds & (KIND_STATEMENT | KIND_BRANCH) != 0 {
            let mut program = std::mem::take(&mut self.module.program_initial_blocks);
            for ib in program.iter_mut() {
                if let Some(sc) =
                    Self::cov_instance_scope(&self.module, &mut cov, &inst_paths, &ib.scope)
                {
                    let span = ib.stmt.span;
                    let s =
                        std::mem::replace(&mut ib.stmt, Statement::new(StatementKind::Null, span));
                    ib.stmt = Instr {
                        cov: &mut cov,
                        kinds,
                        scope: sc,
                    }
                    .wrap(s);
                }
            }
            self.module.program_initial_blocks = program;
            let mut fin = std::mem::take(&mut self.module.final_blocks);
            for fb in fin.iter_mut() {
                if let Some(sc) =
                    Self::cov_instance_scope(&self.module, &mut cov, &inst_paths, &fb.scope)
                {
                    let span = fb.stmt.span;
                    let s =
                        std::mem::replace(&mut fb.stmt, Statement::new(StatementKind::Null, span));
                    fb.stmt = Instr {
                        cov: &mut cov,
                        kinds,
                        scope: sc,
                    }
                    .wrap(s);
                }
            }
            self.module.final_blocks = fin;
            let mut always = std::mem::take(&mut self.module.always_blocks);
            for ab in always.iter_mut() {
                if let Some(sc) =
                    Self::cov_instance_scope(&self.module, &mut cov, &inst_paths, &ab.scope)
                {
                    let kind = match ab.kind {
                        crate::ast::decl::AlwaysKind::Always => "always",
                        crate::ast::decl::AlwaysKind::AlwaysComb => "always_comb",
                        crate::ast::decl::AlwaysKind::AlwaysFf => "always_ff",
                        crate::ast::decl::AlwaysKind::AlwaysLatch => "always_latch",
                    };
                    let span = ab.stmt.span;
                    let s =
                        std::mem::replace(&mut ab.stmt, Statement::new(StatementKind::Null, span));
                    ab.stmt = Instr {
                        cov: &mut cov,
                        kinds,
                        scope: sc,
                    }
                    .always(s, kind);
                }
            }
            self.module.always_blocks = always;
            // Subroutines: a package's belong to the package, the others to
            // the instance their qualified name starts with.
            // A bare-named copy of an instance's subroutine (same source
            // span) is not the top module's own.
            let dotted: HashSet<(String, usize)> = self
                .module
                .functions
                .iter()
                .map(|(k, f)| (k, f.span))
                .chain(self.module.tasks.iter().map(|(k, t)| (k, t.span)))
                .filter_map(|(k, sp)| {
                    let (_, leaf) = k.rsplit_once('.')?;
                    Some((leaf.to_string(), sp.start))
                })
                .collect();
            let copy = |k: &str, sp: Span| {
                !k.contains('.') && !k.contains("::") && dotted.contains(&(k.to_string(), sp.start))
            };
            let mut functions = std::mem::take(&mut self.module.functions).into_map();
            for (key, fd) in functions.iter_mut() {
                if copy(key, fd.span) && !self.module.pkg_subr_owner.contains_key(key.as_str()) {
                    continue;
                }
                if let Some(sc) = self.cov_subroutine_scope(&mut cov, &inst_paths, key) {
                    let items = std::mem::take(&mut fd.items);
                    fd.items = Instr {
                        cov: &mut cov,
                        kinds,
                        scope: sc,
                    }
                    .stmts(items);
                }
            }
            self.module.functions = xezim_core::elaborate::FuncTable::from_map(functions);
            let mut tasks = std::mem::take(&mut self.module.tasks);
            for (key, td) in tasks.iter_mut() {
                if copy(key, td.span) && !self.module.pkg_subr_owner.contains_key(key.as_str()) {
                    continue;
                }
                if let Some(sc) = self.cov_subroutine_scope(&mut cov, &inst_paths, key) {
                    let items = std::mem::take(&mut td.items);
                    td.items = Instr {
                        cov: &mut cov,
                        kinds,
                        scope: sc,
                    }
                    .stmts(items);
                }
            }
            self.module.tasks = tasks;
            self.cov_instrument_classes(&mut cov);
        }
        if kinds & KIND_TOGGLE != 0 {
            let names = std::mem::take(&mut self.code_cov_toggle_names);
            cov.toggle = Some(self.cov_build_toggle(&mut cov, &inst_paths, names));
            self.enable_dump_dirty_tracking();
        }
        self.code_cov_hits = vec![0; cov.counters as usize];
        self.code_cov = Some(Box::new(cov));
    }

    /// An initial block elaboration made rather than the source: a
    /// variable declaration's initializer, or a module-level assertion.
    fn cov_synthesized_initial(s: &Statement) -> bool {
        let dummy = |sp: Span| sp.start == 0 && sp.end == 0;
        match &s.kind {
            _ if dummy(s.span) => true,
            StatementKind::BlockingAssign { lvalue, .. } => dummy(lvalue.span),
            StatementKind::Assertion(a) => a.is_property || a.deferred.is_some(),
            _ => false,
        }
    }

    /// Instrument an initial block as `compile` schedules it.
    pub(super) fn cov_initial(&mut self, mut ib: InitialBlock) -> InitialBlock {
        let Some(mut cov) = self.code_cov.take() else {
            return ib;
        };
        let kinds = cov.kinds;
        if kinds & (KIND_STATEMENT | KIND_BRANCH) != 0 && !Self::cov_synthesized_initial(&ib.stmt) {
            let paths = std::mem::take(&mut cov.inst_paths);
            if let Some(sc) = Self::cov_instance_scope(&self.module, &mut cov, &paths, &ib.scope) {
                let span = ib.stmt.span;
                let s = std::mem::replace(&mut ib.stmt, Statement::new(StatementKind::Null, span));
                ib.stmt = Instr {
                    cov: &mut cov,
                    kinds,
                    scope: sc,
                }
                .wrap(s);
            }
            cov.inst_paths = paths;
            self.code_cov_hits.resize(cov.counters as usize, 0);
        }
        self.code_cov = Some(cov);
        ib
    }

    /// Instrument a continuous assignment as `build_comb_entries` takes it:
    /// its `?:`s become branches, and it gets its statement counter (shared
    /// by the pieces one assignment expands to), which is returned. Takes
    /// the fields it needs: the caller's iterator holds `module`.
    pub(super) fn cov_contassign(
        module: &ElaboratedModule,
        code_cov: &mut Option<Box<CodeCov>>,
        hits: &mut Vec<u64>,
        ca: &mut crate::compiler::elaborate::ContinuousAssignment,
    ) -> Option<u32> {
        let (span, scope) = ca.origin.clone()?;
        let cov = code_cov.as_mut()?;
        let kinds = cov.kinds;
        let scope = if scope.is_empty() {
            Self::cov_lhs_base(&ca.lhs)
                .and_then(|n| n.rsplit_once('.').map(|(p, _)| p.to_string()))
                .unwrap_or_default()
        } else {
            scope
        };
        let paths = std::mem::take(&mut cov.inst_paths);
        let sc = Self::cov_instance_scope(module, cov, &paths, &scope);
        cov.inst_paths = paths;
        let sc = sc?;
        let mut ins = Instr {
            cov,
            kinds,
            scope: sc,
        };
        ins.expr(&mut ca.rhs);
        let mut counter = None;
        if kinds & KIND_STATEMENT != 0 {
            let key = (span.start, span.end, scope);
            counter = Some(match ins.cov.ca_counter.get(&key) {
                Some(&c) => c,
                None => {
                    let c = ins.counter();
                    ins.cov.stmts.push(StmtSite {
                        scope: sc,
                        span,
                        kind: "assign",
                        counter: c,
                    });
                    ins.cov.ca_counter.insert(key, c);
                    c
                }
            });
        }
        hits.resize(cov.counters as usize, 0);
        counter
    }

    /// The signal-name base of a continuous assignment's target.
    fn cov_lhs_base(e: &Expression) -> Option<String> {
        match &e.kind {
            ExprKind::Ident(h) => Some(
                h.path
                    .iter()
                    .map(|s| s.name.name.as_str())
                    .collect::<Vec<_>>()
                    .join("."),
            ),
            ExprKind::Index { expr, .. }
            | ExprKind::RangeSelect { expr, .. }
            | ExprKind::MemberAccess { expr, .. }
            | ExprKind::Paren(expr) => Self::cov_lhs_base(expr),
            _ => None,
        }
    }

    /// The instance a block scope (`u0.u1.genblk2`) belongs to: its longest
    /// prefix that names an instance, else the top. `None` when the command
    /// line's scope limit leaves it out.
    fn cov_instance_scope(
        module: &ElaboratedModule,
        cov: &mut CodeCov,
        inst_paths: &HashSet<String>,
        scope: &str,
    ) -> Option<u32> {
        let mut path = scope;
        let inst = loop {
            if path.is_empty() || inst_paths.contains(path) {
                break path;
            }
            match path.rsplit_once('.') {
                Some((p, _)) => path = p,
                None => path = "",
            }
        };
        let wrapper = module.name == xezim_core::MULTI_TOP_WRAPPER;
        let name = if wrapper {
            inst.to_string()
        } else if inst.is_empty() {
            module.name.clone()
        } else {
            format!("{}.{}", module.name, inst)
        };
        if name.is_empty() {
            return None;
        }
        if !cov.limit.is_empty()
            && !cov.limit.iter().any(|l| {
                name == *l
                    || name.starts_with(&format!("{}.", l))
                    || (!wrapper && inst == l.as_str())
                    || (!wrapper && inst.starts_with(&format!("{}.", l)))
            })
        {
            return None;
        }
        let key = format!("i:{}", inst);
        if let Some(&i) = cov.scope_index.get(&key) {
            return Some(i);
        }
        let unit = if inst.is_empty() {
            module.name.clone()
        } else {
            module
                .instances
                .iter()
                .find(|i| i.path == inst)
                .map(|i| i.def_name.clone())
                .unwrap_or_default()
        };
        let src = module.src_file_of_module.get(&unit).copied();
        let i = cov.scopes.len() as u32;
        cov.scopes.push(Scope {
            name,
            unit,
            kind: "instance",
            src,
        });
        cov.scope_index.insert(key, i);
        Some(i)
    }

    /// A scope outside the instance tree: a package, `$unit`, or the module
    /// that declares a class. Library packages count only when named.
    fn cov_package_scope(
        &self,
        cov: &mut CodeCov,
        name: &str,
        src_key: &str,
        kind: &'static str,
    ) -> Option<u32> {
        let listed = cov.limit.iter().any(|l| l == name);
        if !cov.limit.is_empty() && !listed {
            return None;
        }
        if !listed && LIBRARY_PACKAGES.contains(&name) {
            return None;
        }
        let key = format!("p:{}", name);
        if let Some(&i) = cov.scope_index.get(&key) {
            return Some(i);
        }
        let src = self
            .module
            .src_file_of_module
            .get(src_key)
            .or_else(|| self.module.src_file_of_module.get(name))
            .copied();
        let i = cov.scopes.len() as u32;
        cov.scopes.push(Scope {
            name: name.to_string(),
            unit: name.to_string(),
            kind,
            src,
        });
        cov.scope_index.insert(key, i);
        Some(i)
    }

    fn cov_subroutine_scope(
        &self,
        cov: &mut CodeCov,
        inst_paths: &HashSet<String>,
        key: &str,
    ) -> Option<u32> {
        if let Some((pkg, _)) = key.split_once("::") {
            return self.cov_package_scope(cov, pkg, pkg, "package");
        }
        if let Some(pkg) = self.module.pkg_subr_owner.get(key) {
            let pkg = pkg.clone();
            return self.cov_package_scope(cov, &pkg, &pkg, "package");
        }
        let scope = key.rsplit_once('.').map(|(p, _)| p).unwrap_or("");
        Self::cov_instance_scope(&self.module, cov, inst_paths, scope)
    }

    fn cov_instrument_classes(&mut self, cov: &mut CodeCov) {
        let kinds = cov.kinds;
        let mut classes = std::mem::take(&mut self.module.classes);
        let mut done: HashMap<usize, Arc<crate::compiler::elaborate::ElaboratedClass>> =
            HashMap::default();
        let mut keys: Vec<String> = classes.keys().cloned().collect();
        keys.sort();
        for key in keys {
            let Some(arc) = classes.get(&key) else {
                continue;
            };
            let ptr = Arc::as_ptr(arc) as usize;
            if let Some(same) = done.get(&ptr) {
                classes.insert(key, same.clone());
                continue;
            }
            let (scope_name, src_key, kind) = match self.module.class_decl_pkg.get(&arc.name) {
                Some(p) => (p.clone(), p.clone(), "package"),
                None => match &arc.declaring_module {
                    Some(m) => (m.clone(), m.clone(), "module"),
                    None => ("$unit".to_string(), arc.name.clone(), "unit"),
                },
            };
            let Some(sc) = self.cov_package_scope(cov, &scope_name, &src_key, kind) else {
                continue;
            };
            if scope_name == "$unit" || cov.scopes[sc as usize].src.is_none() {
                // A file-scope class: its own entry locates its methods.
                if let Some(&f) = self.module.src_file_of_module.get(&arc.name) {
                    if cov.scopes[sc as usize].src.is_none() {
                        cov.scopes[sc as usize].src = Some(f);
                    }
                }
            }
            let mut arc = classes.remove(&key).unwrap();
            let cls = Arc::make_mut(&mut arc);
            let mut names: Vec<String> = cls.methods.keys().cloned().collect();
            names.sort();
            for m in names {
                let Some(method) = Arc::make_mut(&mut cls.methods).get_mut(&m) else {
                    continue;
                };
                let items = match &mut method.kind {
                    crate::ast::decl::ClassMethodKind::Function(fd)
                    | crate::ast::decl::ClassMethodKind::Extern(fd) => &mut fd.items,
                    crate::ast::decl::ClassMethodKind::Task(td) => &mut td.items,
                    crate::ast::decl::ClassMethodKind::PureVirtual(_) => continue,
                };
                let body = std::mem::take(items);
                *items = Instr {
                    cov: &mut *cov,
                    kinds,
                    scope: sc,
                }
                .stmts(body);
            }
            done.insert(ptr, arc.clone());
            classes.insert(key, arc);
        }
        self.module.classes = classes;
    }

    /// Count counter `c` (`Insn::CovHit`). Out of line, so the executors'
    /// hot loops compile as they do without code coverage.
    #[inline(never)]
    pub(super) fn cov_count(&mut self, c: u32) {
        if let Some(h) = self.code_cov_hits.get_mut(c as usize) {
            *h += 1;
        }
    }

    /// `TsInsn::CovHit`: count, and note the count on the save list so a
    /// bail takes it back (`ts_restore_saved`) before the block runs again.
    #[inline(never)]
    pub(super) fn ts_cov_hit(&mut self, c: u32) {
        self.cov_count(c);
        self.ts_save_list.push((c | TS_SAVE_COV, 0, 0));
    }

    /// Count a coverage marker's counter (interpreter side).
    pub(super) fn cov_hit(&mut self, arg: Option<&Expression>) {
        if let Some(c) = arg
            .and_then(marker_id)
            .and_then(|id| self.code_cov_hits.get_mut(id as usize))
        {
            *c += 1;
        }
    }

    /// `$__xz_covc(<n>, cond)`: count the condition's outcome, yield it.
    pub(super) fn cov_cond(&mut self, args: &[Expression]) -> Value {
        let Some(cond) = args.get(1) else {
            return Value::new(1);
        };
        let v = self.eval_expr(cond);
        if let Some(id) = args.first().and_then(marker_id) {
            let slot = match v.is_nonzero() {
                Some(true) => Some(id),
                Some(false) => Some(id + 1),
                None => None,
            };
            if let Some(c) = slot.and_then(|s| self.code_cov_hits.get_mut(s as usize)) {
                *c += 1;
            }
        }
        v
    }

    /// Nets and variables toggle coverage tracks, by name, taken while the
    /// elaboration signal table still holds their declarations: the integral
    /// ones of instances, without input and inout ports (the parent's signal
    /// carries those toggles), unpacked arrays, `time` variables, subroutine
    /// variables and class properties.
    pub(super) fn cov_toggle_candidates(module: &ElaboratedModule) -> Vec<String> {
        let wants = code_coverage().is_some_and(|c| c.kinds & KIND_TOGGLE != 0);
        if !wants {
            return Vec::new();
        }
        let inst_paths: HashSet<&str> = module.instances.iter().map(|i| i.path.as_str()).collect();
        // Each instance's own name, without its array index (`u` for `u[0]`
        // and `g[0].u`), and its module.
        let inst_names: HashSet<(&str, &str)> = module
            .instances
            .iter()
            .map(|i| {
                let leaf = i.path.rsplit('.').next().unwrap_or_default();
                let name = leaf.split('[').next().unwrap_or_default();
                (name, i.def_name.as_str())
            })
            .collect();
        let subr_prefixes: Vec<String> = module
            .functions
            .keys()
            .chain(module.tasks.keys())
            .map(|k| format!("{}.", k))
            .collect();
        let mut out: Vec<String> = module
            .signals
            .iter()
            .filter(|(name, sig)| {
                if sig.is_const
                    || sig.is_real
                    || sig.width == 0
                    || matches!(
                        sig.direction,
                        Some(PortDirection::Input) | Some(PortDirection::Inout)
                    )
                {
                    return false;
                }
                if name.contains("__xz") || name.starts_with("__") || name.contains("::") {
                    return false;
                }
                if let Some((base, _)) = name.split_once('[') {
                    if module.arrays.contains_key(base)
                        || module.arrays_nd.contains_key(base)
                        || module.dynamic_arrays.contains(base)
                        || module.associative_arrays.contains_key(base)
                    {
                        return false;
                    }
                }
                let n = name.as_str();
                // Enum constants and parameters, and the placeholders named
                // after instances.
                if module.parameters.contains_key(n) || inst_paths.contains(n) {
                    return false;
                }
                if module.arrays.contains_key(n)
                    || module.arrays_nd.contains_key(n)
                    || module.dynamic_arrays.contains(n)
                    || module.associative_arrays.contains_key(n)
                    || module.queue_vars.contains(n)
                    || module.string_signals.contains(n)
                    || module.events.contains(n)
                {
                    return false;
                }
                if let Some(t) = &sig.type_name {
                    if module.classes.contains_key(t) || module.interfaces.contains(t) {
                        return false;
                    }
                    // Instances and generate blocks are scopes, not signals
                    // (§23.6). Elaboration gives the top module a 1-bit
                    // placeholder signal under each one's name, so that
                    // hierarchical names through it validate: typed
                    // `$genblock` for a generate block and by its module for
                    // an instance, including an instance array and one in a
                    // generate block, which have no instance path of that
                    // name.
                    if t == "$genblock" || inst_names.contains(&(n, t.as_str())) {
                        return false;
                    }
                }
                match module.var_decl_types.get(n) {
                    Some(DataType::IntegerAtom {
                        kind: IntegerAtomType::Time,
                        ..
                    })
                    | Some(DataType::Real { .. })
                    | Some(DataType::Simple { .. })
                    | Some(DataType::Interface { .. }) => return false,
                    _ => {}
                }
                !subr_prefixes.iter().any(|p| n.starts_with(p.as_str()))
            })
            .map(|(name, _)| name.clone())
            .collect();
        out.sort();
        out
    }

    fn cov_build_toggle(
        &mut self,
        cov: &mut CodeCov,
        inst_paths: &HashSet<String>,
        names: Vec<String>,
    ) -> Toggle {
        let mut t = Toggle {
            slot: vec![u32::MAX; self.signal_table.len()],
            ids: Vec::new(),
            width: Vec::new(),
            word0: Vec::new(),
            bit0: Vec::new(),
            words: Vec::new(),
            rise: Vec::new(),
            fall: Vec::new(),
            names: Vec::new(),
        };
        let top_prefix = format!("{}.", self.module.name);
        for name in names {
            let Some(&id) = self.signal_name_to_id.get(name.as_str()).or_else(|| {
                self.signal_name_to_id
                    .get(format!("{}{}", top_prefix, name).as_str())
            }) else {
                continue;
            };
            let width = self.signal_widths.get(id).copied().unwrap_or(0);
            if width == 0 || self.signal_real.get(id).copied().unwrap_or(false) {
                continue;
            }
            // The owning instance: the longest instance path the name
            // starts with; the rest is the signal's name within it.
            let mut inst = "";
            let mut rest = name.as_str();
            let mut cut = name.len();
            while let Some(p) = name[..cut].rfind('.') {
                if inst_paths.contains(&name[..p]) {
                    inst = &name[..p];
                    rest = &name[p + 1..];
                    break;
                }
                cut = p;
            }
            let Some(sc) = Self::cov_instance_scope(&self.module, cov, inst_paths, inst) else {
                continue;
            };
            let k = if t.slot[id] != u32::MAX {
                t.slot[id]
            } else {
                let k = t.ids.len() as u32;
                t.slot[id] = k;
                t.ids.push(id as u32);
                t.width.push(width);
                t.word0.push(t.words.len() as u32);
                t.bit0.push(t.rise.len() as u32);
                let words = width.div_ceil(64) as usize;
                let v = &self.signal_table[id];
                for w in 0..words {
                    let n = (width as usize - w * 64).min(64);
                    let (pv, px) = v.slice_bits_swar(w * 64, n);
                    t.words.push(Word {
                        v: pv,
                        x: px,
                        at: u64::MAX,
                        bv: pv,
                        bx: px,
                    });
                }
                t.rise.resize(t.rise.len() + width as usize, 0);
                t.fall.resize(t.fall.len() + width as usize, 0);
                k
            };
            t.names.push((sc, rest.to_string(), k));
        }
        t
    }

    /// End of a tick or of a time slot: compare every tracked signal written
    /// since the last sample with its value at the end of the previous slot.
    pub(super) fn cov_toggle_sample(&mut self) {
        let Some(t) = self.code_cov.as_mut().and_then(|c| c.toggle.as_mut()) else {
            return;
        };
        let now = self.time;
        let full = !self.dump_dirty_active;
        let n = if full {
            t.ids.len()
        } else {
            self.dump_dirty.len()
        };
        for i in 0..n {
            let k = if full {
                i
            } else {
                match t.slot.get(self.dump_dirty[i] as usize) {
                    Some(&k) if k != u32::MAX => k as usize,
                    _ => continue,
                }
            };
            let id = t.ids[k] as usize;
            let width = t.width[k] as usize;
            if width <= 64 {
                let (v, x) = match self.signal_inline_bits.get(id) {
                    Some(sl) => (sl[0], sl[1]),
                    None => self.signal_table[id].raw_bits(),
                };
                let mask = if width == 64 {
                    u64::MAX
                } else {
                    (1u64 << width) - 1
                };
                t.sample(k, v & mask, x & mask, 0, now);
            } else {
                for w in 0..width.div_ceil(64) {
                    let n = (width - w * 64).min(64);
                    let (v, x) = self.signal_table[id].slice_bits_swar(w * 64, n);
                    t.sample(k, v, x, w, now);
                }
            }
        }
    }

    fn cov_report(&self) -> Vec<Report> {
        let Some(cov) = self.code_cov.as_ref() else {
            return Vec::new();
        };
        let mut lines = Lines {
            module: &self.module,
            nl: HashMap::default(),
        };
        let mut reports: Vec<Report> = (0..cov.scopes.len()).map(|_| Report::default()).collect();
        let count = |c: u32| self.code_cov_hits.get(c as usize).copied().unwrap_or(0);
        for s in &cov.stmts {
            let src = cov.scopes[s.scope as usize].src;
            let (file, line) = lines.locate(s.span, src).unwrap_or_default();
            reports[s.scope as usize].stmts.push(StmtRow {
                file,
                line,
                start: s.span.start,
                kind: s.kind,
                count: count(s.counter),
            });
        }
        for b in &cov.branches {
            let src = cov.scopes[b.scope as usize].src;
            let (file, line) = lines.locate(b.span, src).unwrap_or_default();
            let taken: u64 = b.arms.iter().filter_map(|a| a.counter).map(count).sum();
            let arms = b
                .arms
                .iter()
                .map(|a| ArmRow {
                    line: lines.locate(a.span, src).map(|(_, l)| l).unwrap_or(line),
                    label: a.label,
                    count: match a.counter {
                        Some(c) => count(c),
                        None => b.entry.map(count).unwrap_or(0).saturating_sub(taken),
                    },
                })
                .collect();
            reports[b.scope as usize].branches.push(BranchRow {
                file,
                line,
                start: b.span.start,
                kind: b.kind,
                arms,
            });
        }
        if let Some(t) = &cov.toggle {
            for (sc, name, k) in &t.names {
                let k = *k as usize;
                let (b, w) = (t.bit0[k] as usize, t.width[k] as usize);
                reports[*sc as usize].toggles.push(ToggleRow {
                    name: name.clone(),
                    rise: t.rise[b..b + w].to_vec(),
                    fall: t.fall[b..b + w].to_vec(),
                });
            }
        }
        for r in reports.iter_mut() {
            r.normalize();
        }
        reports
    }

    /// The `code_coverage` member of the results file, and the `[COV]`
    /// summary lines.
    pub(super) fn code_coverage_json(&self) -> Option<String> {
        let cov = self.code_cov.as_ref()?;
        let kinds = cov.kinds;
        let reports = self.cov_report();
        let mut order: Vec<usize> = (0..cov.scopes.len()).collect();
        order.sort_by(|&a, &b| {
            (cov.scopes[a].kind, &cov.scopes[a].name)
                .cmp(&(cov.scopes[b].kind, &cov.scopes[b].name))
        });
        let mut total = Tallies::default();
        for r in &reports {
            for (t, x) in total.iter_mut().zip(r.tallies()) {
                t.add(x);
            }
        }
        let kind_names: Vec<&str> = KIND_NAMES
            .iter()
            .enumerate()
            .filter(|(i, _)| kinds & (1 << i) != 0)
            .map(|(_, n)| *n)
            .collect();
        let summary = |t: &Tallies| -> String {
            KIND_NAMES
                .iter()
                .enumerate()
                .filter(|(i, _)| kinds & (1 << i) != 0)
                .map(|(i, n)| format!("{} {}", n, t[i].text()))
                .collect::<Vec<_>>()
                .join(", ")
        };
        chatter!("[COV] code coverage: {}", summary(&total));
        const SHOWN: usize = 50;
        for &i in order.iter().take(SHOWN) {
            let s = &cov.scopes[i];
            chatter!(
                "[COV]   {} ({}): {}",
                s.name,
                s.unit,
                summary(&reports[i].tallies())
            );
        }
        if order.len() > SHOWN {
            chatter!(
                "[COV]   ... {} more scopes in the results file",
                order.len() - SHOWN
            );
        }
        let mut json = String::new();
        json.push_str("  \"code_coverage\": {\n");
        json.push_str(&format!(
            "    \"kinds\": [{}],\n",
            kind_names
                .iter()
                .map(|k| format!("\"{}\"", k))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        for (i, n) in KIND_NAMES.iter().enumerate() {
            if kinds & (1 << i) != 0 {
                json.push_str(&format!("    \"{}\": {},\n", n, total[i].json()));
            }
        }
        json.push_str("    \"scopes\": [\n");
        for (j, &i) in order.iter().enumerate() {
            let s = &cov.scopes[i];
            json.push_str(&format!(
                "      {{\n        \"scope\": \"{}\",\n        \"design_unit\": \"{}\",\n        \"kind\": \"{}\",\n",
                json_escape(&s.name),
                json_escape(&s.unit),
                s.kind
            ));
            reports[i].json(kinds, "        ", &mut json);
            json.push_str(if j + 1 < order.len() {
                "      },\n"
            } else {
                "      }\n"
            });
        }
        json.push_str("    ],\n");
        // Design units: every instance of a module added together.
        let mut units: BTreeMap<String, (usize, Report)> = BTreeMap::new();
        for &i in &order {
            let s = &cov.scopes[i];
            let e = units
                .entry(s.unit.clone())
                .or_insert((0, Report::default()));
            e.0 += 1;
            e.1.merge(&reports[i]);
        }
        json.push_str("    \"design_units\": [\n");
        let n_units = units.len();
        for (j, (unit, (n, r))) in units.iter().enumerate() {
            json.push_str(&format!(
                "      {{\n        \"design_unit\": \"{}\",\n        \"instances\": {},\n",
                json_escape(unit),
                n
            ));
            r.json(kinds, "        ", &mut json);
            json.push_str(if j + 1 < n_units {
                "      },\n"
            } else {
                "      }\n"
            });
        }
        json.push_str("    ]\n  }");
        Some(json)
    }
}
