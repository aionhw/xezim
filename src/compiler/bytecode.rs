//! Bytecode VM for high-performance simulation execution.
//! Compiles AST expressions and statements into a flat instruction array
//! that can be executed without pointer-chasing through Box<Expression> trees.

use super::value::Value;
use crate::ast::decl::TaskDeclaration;
use crate::ast::expr::*;
use crate::ast::stmt::*;
use std::sync::Arc;
use ahash::AHashMap as HashMap;

const MAX_INLINE_DEPTH: usize = 8;

/// A register in the bytecode VM. Registers hold Values.
type RegId = u16;

/// Bytecode instruction set. Stack-free, register-based design.
/// Each instruction specifies source and destination registers explicitly,
/// enabling the VM to iterate a flat Vec<Insn> with predictable memory access.
#[derive(Debug, Clone)]
pub enum Insn {
    /// Load a constant value into a register. `Box<Value>` keeps the
    /// variant small (8 B instead of 32 B for the inline Value) — LoadConst
    /// isn't on the hot dispatch path so the extra indirection is cheap
    /// and the 24 B saving compounds with the u32 signal_id fields below
    /// to shrink `Insn` from 40 B to 32 B.
    LoadConst(RegId, Box<Value>),
    /// Load a signal from signal_table[signal_id] into a register.
    LoadSignal(RegId, usize),      // (dest_reg, signal_id)
    /// Load a signal and mark it as signed.
    LoadSignalSigned(RegId, usize),
    /// Resize register to given width.
    Resize(RegId, u32),

    // Binary arithmetic/logic: dest = left op right
    Add(RegId, RegId, RegId),
    Sub(RegId, RegId, RegId),
    Mul(RegId, RegId, RegId),
    Div(RegId, RegId, RegId),
    Mod(RegId, RegId, RegId),
    BitAnd(RegId, RegId, RegId),
    BitOr(RegId, RegId, RegId),
    BitXor(RegId, RegId, RegId),
    BitXnor(RegId, RegId, RegId),
    LogAnd(RegId, RegId, RegId),
    LogOr(RegId, RegId, RegId),
    Eq(RegId, RegId, RegId),
    Neq(RegId, RegId, RegId),
    CaseEq(RegId, RegId, RegId),
    Lt(RegId, RegId, RegId),
    Leq(RegId, RegId, RegId),
    Gt(RegId, RegId, RegId),
    Geq(RegId, RegId, RegId),
    Shl(RegId, RegId, RegId),
    Shr(RegId, RegId, RegId),
    AShr(RegId, RegId, RegId),

    // Unary: dest = op src
    BitNot(RegId, RegId),
    LogNot(RegId, RegId),
    Negate(RegId, RegId),
    ReduceAnd(RegId, RegId),
    ReduceOr(RegId, RegId),
    ReduceXor(RegId, RegId),

    /// Bit select: dest = src[index]
    BitSelect(RegId, RegId, RegId), // (dest, base, index)
    /// Range select: dest = src[left:right]
    RangeSelect(RegId, RegId, RegId, RegId), // (dest, base, left, right)
    /// Concatenation: dest = {parts...}, part register IDs stored in
    /// the boxed Vec. The `Box` keeps the variant at 16 B (Box ptr only)
    /// instead of inlining a 24 B Vec header — Concat is rare on the
    /// hot path so the extra indirection is cheap, and shrinking this
    /// variant lets the whole `Insn` enum drop from 32 B to 24 B.
    Concat(RegId, Box<Vec<RegId>>),

    /// Conditional branch: if reg is false, jump to target instruction index.
    BranchIfFalse(RegId, u32),       // (cond_reg, jump_target)
    /// 4-state select: dest = cond ? then_reg : else_reg, with per-bit X merge
    /// (IEEE 1800 §11.4.11 Table 11-21) when cond has unknown bits. Both
    /// branches are always evaluated (no short-circuit) — used for `?:` so
    /// X conditions don't silently fall through to the false branch.
    Select(RegId, RegId, RegId, RegId), // (dest, cond, then, else)
    /// Unconditional jump.
    Jump(u32),

    /// Non-blocking assign: signal_table[id] <= reg (scheduled via NBA queue).
    NbaAssign(usize, RegId, u32),  // (signal_id, value_reg, width)
    /// Non-blocking partial assign: signal_table[id][hi:lo] <= reg.
    /// Read-modify-write at exec time using current signal value as base.
    NbaAssignRange(usize, u32, u32, RegId), // (signal_id, hi, lo, value_reg)
    /// NBA partial assign with dynamic hi/lo (mirrors `BlockingAssignRangeDyn`):
    /// signal_table[id][hi_reg:lo_reg] <= reg. Lets us compile NBAs with
    /// run-time bit ranges (e.g. `q[idx +: W]`, `q[j:j-W+1]`) instead of
    /// falling back to the AST interpreter — critical on CPUs like c910
    /// where these patterns fire millions of times per simulation.
    NbaAssignRangeDyn(usize, RegId, RegId, RegId), // (signal_id, hi_reg, lo_reg, value_reg)
    /// Non-blocking bit assign: signal_table[id][bit_idx_reg] <= reg.
    NbaAssignBitDyn(usize, RegId, RegId), // (signal_id, idx_reg, value_reg)
    /// Blocking assign: signal_table[id] = reg.
    BlockingAssign(usize, RegId, u32), // (signal_id, value_reg, width)
    /// Blocking range assign: signal_table[id][hi:lo] = reg (read-modify-write).
    BlockingAssignRange(usize, u32, u32, RegId), // (signal_id, hi, lo, value_reg)
    /// Blocking range assign with dynamic hi/lo (for `[idx +: W]` / `[idx -: W]`).
    BlockingAssignRangeDyn(usize, RegId, RegId, RegId), // (signal_id, hi_reg, lo_reg, value_reg)
    /// Blocking bit assign: signal_table[id][idx_reg] = reg[0] (read-modify-write).
    BlockingAssignBitDyn(usize, RegId, RegId), // (signal_id, idx_reg, value_reg)

    /// Load array element: dest = signal_table[array_base + eval(index_reg)]
    /// `Box<String>` keeps the variant at 16 B instead of 32 B.
    LoadArrayElem(RegId, Box<String>, RegId), // (dest, array_name, index_reg)
    /// NBA assign to array element.
    NbaAssignArray(Box<String>, RegId, RegId, u32), // (array_name, index_reg, value_reg, width)

    /// Marks end of a compiled block (no-op, helps debugging).
    /// Copy src register to dest register.
    Move(RegId, RegId),       // (dest, src)

    /// Fallback: invoke the AST interpreter on an untranslated statement.
    /// Used for rare constructs (e.g. $display, complex LHS) so an edge
    /// block containing one unsupported stmt can still run most of its
    /// body as fast bytecode instead of falling back wholesale to AST.
    /// Boxed payload keeps the variant at 8 B (Box ptr) instead of
    /// 24 B (Arc + fat-ptr str). StmtFallback is the AST-interpreter
    /// escape hatch — its dispatch cost dwarfs an extra deref.
    StmtFallback(Box<(Arc<Statement>, &'static str)>),

    SetSigned(RegId),
    Nop,
}

/// A compiled bytecode program for one always block or continuous assign.
#[derive(Debug, Clone)]
pub struct CompiledBlock {
    pub instructions: Vec<Insn>,
    pub num_regs: u16,
}

/// Compiler state for converting AST → bytecode.
pub struct BytecodeCompiler<'a> {
    insns: Vec<Insn>,
    next_reg: RegId,
    signal_name_to_id: &'a HashMap<Arc<str>, usize>,
    signal_signed: &'a [bool],
    signal_widths: &'a [u32],
    arrays: &'a HashMap<String, (i64, i64, u32)>,
    widths: &'a HashMap<String, u32>,
    pub bail_reason: Option<&'static str>,
    /// When true, unsupported statements emit `StmtFallback` instead of
    /// failing compilation. Safe for edge blocks where the AST interpreter's
    /// statement path is the same one used by the non-compiled fallback.
    pub allow_ast_fallback: bool,
    /// Hierarchical scope for resolving unqualified identifiers. An Ident
    /// with a bare local name (`mem_valid`) is first tried verbatim, then
    /// with this prefix applied (`testbench.mem_valid`).
    pub scope_hint: Option<String>,
    /// Per-for-loop leaf-name → signal_id override. Set by `compile_stmt`'s
    /// For arm before compiling condition/step expressions, cleared after.
    /// Re-routes bare-ident lookups for the loop variable so that the step
    /// `i = i+1` writes to the same signal as the init `i = 0`, even when
    /// the elaborator only scope-qualified init's lvalue (see compile_for
    /// for the full c910 hang context).
    pub for_loop_var_ids: std::collections::HashMap<String, usize>,
    /// User-task table for inlining zero-arg, non-blocking task bodies.
    /// Task-enable (`task_name;`) statements that resolve here get their
    /// bodies compiled in place instead of emitting a single StmtFallback
    /// for the whole call — lets the inner simple assigns compile cleanly
    /// and narrows the fallback to just the inner $write/$display.
    tasks: Option<&'a HashMap<String, TaskDeclaration>>,
    inlining_stack: Vec<String>,
    pub tasks_inlined: u32,
    /// Elaborated module parameters — used by `eval_const_expr` so that
    /// bytecode compilation can fold module params (e.g. `CARRY_CHAIN`) into
    /// the compile-time widths of `+:` / `-:` range selects.
    params: Option<&'a HashMap<String, Value>>,
}

impl<'a> BytecodeCompiler<'a> {
    pub fn new(
        signal_name_to_id: &'a HashMap<Arc<str>, usize>,
        signal_signed: &'a [bool],
        signal_widths: &'a [u32],
        arrays: &'a HashMap<String, (i64, i64, u32)>,
        widths: &'a HashMap<String, u32>,
    ) -> Self {
        Self {
            insns: Vec::with_capacity(64),
            next_reg: 0,
            signal_name_to_id,
            signal_signed,
            signal_widths,
            arrays,
            widths,
            bail_reason: None,
            allow_ast_fallback: false,
            scope_hint: None,
            for_loop_var_ids: std::collections::HashMap::new(),
            tasks: None,
            inlining_stack: Vec::new(),
            tasks_inlined: 0,
            params: None,
        }
    }

    pub fn set_params(&mut self, params: &'a HashMap<String, Value>) {
        self.params = Some(params);
    }

    pub fn set_ast_fallback(&mut self, allow: bool) {
        self.allow_ast_fallback = allow;
    }

    pub fn set_scope_hint(&mut self, scope: Option<String>) {
        self.scope_hint = scope;
    }

    pub fn set_tasks(&mut self, tasks: &'a HashMap<String, TaskDeclaration>) {
        self.tasks = Some(tasks);
    }

    fn stmt_has_break_or_continue(stmt: &Statement) -> bool {
        match &stmt.kind {
            StatementKind::Break | StatementKind::Continue => true,
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                stmts.iter().any(Self::stmt_has_break_or_continue)
            }
            StatementKind::If { then_stmt, else_stmt, .. } => {
                Self::stmt_has_break_or_continue(then_stmt)
                    || else_stmt.as_ref().map_or(false, |e| Self::stmt_has_break_or_continue(e))
            }
            StatementKind::Case { items, .. } => {
                items.iter().any(|it| Self::stmt_has_break_or_continue(&it.stmt))
            }
            // Don't descend into nested loops — break/continue there target the
            // inner loop, not the enclosing one.
            _ => false,
        }
    }

    fn stmt_is_blocking(stmt: &Statement) -> bool {
        match &stmt.kind {
            StatementKind::TimingControl { .. } => true,
            StatementKind::Wait { .. } => true,
            StatementKind::Forever { .. } => true,
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                stmts.iter().any(Self::stmt_is_blocking)
            }
            StatementKind::If { then_stmt, else_stmt, .. } => {
                Self::stmt_is_blocking(then_stmt)
                    || else_stmt.as_ref().map_or(false, |e| Self::stmt_is_blocking(e))
            }
            StatementKind::For { body, .. } | StatementKind::While { body, .. } => {
                Self::stmt_is_blocking(body)
            }
            _ => false,
        }
    }

    /// Try to inline a zero-arg, non-blocking user task's body at this
    /// call site. Returns true if successfully inlined.
    fn try_inline_task(&mut self, task_name: &str) -> bool {
        if self.inlining_stack.len() >= MAX_INLINE_DEPTH { return false; }
        if self.inlining_stack.iter().any(|n| n == task_name) { return false; }
        let tasks = match self.tasks { Some(t) => t, None => return false };
        let td = match tasks.get(task_name) { Some(t) => t, None => return false };
        if !td.ports.is_empty() { return false; }
        if td.items.iter().any(Self::stmt_is_blocking) { return false; }
        let body: Vec<Statement> = td.items.clone();
        self.inlining_stack.push(task_name.to_string());
        let mut ok = true;
        for s in &body {
            if !self.compile_stmt(s) { ok = false; break; }
        }
        self.inlining_stack.pop();
        if ok { self.tasks_inlined += 1; }
        ok
    }

    fn emit_fallback(&mut self, stmt: &Statement) -> bool {
        if self.allow_ast_fallback {
            let reason = self.bail_reason.unwrap_or_else(|| Self::stmt_kind_label(stmt));
            self.emit(Insn::StmtFallback(Box::new((Arc::new(stmt.clone()), reason))));
            true
        } else {
            false
        }
    }

    fn stmt_kind_label(stmt: &Statement) -> &'static str {
        match &stmt.kind {
            StatementKind::Null => "Stmt_Null",
            StatementKind::NonblockingAssign { .. } => "Stmt_Nba",
            StatementKind::BlockingAssign { .. } => "Stmt_Blk",
            StatementKind::If { .. } => "Stmt_If",
            StatementKind::Case { .. } => "Stmt_Case",
            StatementKind::SeqBlock { .. } => "Stmt_SeqBlock",
            StatementKind::ParBlock { .. } => "Stmt_ParBlock",
            StatementKind::Expr(_) => "Stmt_Expr",
            StatementKind::For { .. } => "Stmt_For",
            StatementKind::Foreach { .. } => "Stmt_Foreach",
            StatementKind::While { .. } => "Stmt_While",
            StatementKind::DoWhile { .. } => "Stmt_DoWhile",
            StatementKind::Repeat { .. } => "Stmt_Repeat",
            StatementKind::Forever { .. } => "Stmt_Forever",
            StatementKind::TimingControl { .. } => "Stmt_Timing",
            StatementKind::Wait { .. } => "Stmt_Wait",
            StatementKind::Assertion(_) => "Stmt_Assertion",
            StatementKind::VarDecl { .. } => "Stmt_VarDecl",
            _ => "Stmt_other",
        }
    }

    fn bail(&mut self, reason: &'static str) {
        if self.bail_reason.is_none() {
            self.bail_reason = Some(reason);
        }
    }

    fn alloc_reg(&mut self) -> RegId {
        let r = self.next_reg;
        self.next_reg += 1;
        r
    }

    fn emit(&mut self, insn: Insn) {
        self.insns.push(insn);
    }

    fn hier_raw_name(hier: &HierarchicalIdentifier) -> String {
        hier.path
            .iter()
            .map(|s| s.name.name.as_str())
            .collect::<Vec<_>>()
            .join(".")
    }

    fn lookup_signal_id(&self, hier: &HierarchicalIdentifier) -> Option<usize> {
        let raw = Self::hier_raw_name(hier);
        // Targeted override for for-loop variables — see for_loop_var_ids
        // doc + compile_for's comment for the c910 motivation.
        if !self.for_loop_var_ids.is_empty() && hier.path.len() == 1 && !raw.contains('.') {
            if let Some(&id) = self.for_loop_var_ids.get(&raw) {
                return Some(id);
            }
        }
        if let Some(&id) = self.signal_name_to_id.get(raw.as_str()) {
            return Some(id);
        }
        if let Some(scope) = &self.scope_hint {
            let qualified = format!("{}.{}", scope, raw);
            if let Some(&id) = self.signal_name_to_id.get(qualified.as_str()) {
                return Some(id);
            }
        }
        if hier.path.len() == 1 {
            let leaf = &hier.path[0].name.name;
            if let Some(&id) = self.signal_name_to_id.get(leaf.as_str()) {
                return Some(id);
            }
        }
        None
    }

    fn expr_to_signal_id(&self, expr: &Expression) -> Option<usize> {
        match &expr.kind {
            ExprKind::Ident(hier) => self.lookup_signal_id(hier),
            ExprKind::Paren(inner) => self.expr_to_signal_id(inner),
            _ => None,
        }
    }

    fn lookup_array_name(&self, hier: &HierarchicalIdentifier) -> Option<String> {
        let raw = Self::hier_raw_name(hier);
        if self.arrays.contains_key(&raw) {
            return Some(raw);
        }
        if let Some(scope) = &self.scope_hint {
            let qualified = format!("{}.{}", scope, raw);
            if self.arrays.contains_key(&qualified) {
                return Some(qualified);
            }
        }
        if hier.path.len() == 1 {
            let leaf = &hier.path[0].name.name;
            if self.arrays.contains_key(leaf) {
                return Some(leaf.clone());
            }
        }
        None
    }

    /// Compile a statement. Returns true on success.
    /// When `allow_ast_fallback` is set, any nested failure rolls back and
    /// emits a single `StmtFallback` for the whole statement.
    pub fn compile_stmt(&mut self, stmt: &Statement) -> bool {
        let start = self.insns.len();
        let start_reg = self.next_reg;
        let saved_reason = self.bail_reason;
        self.bail_reason = None;
        if self.compile_stmt_strict(stmt) {
            self.bail_reason = saved_reason;
            return true;
        }
        if self.allow_ast_fallback {
            let reason = self.bail_reason.unwrap_or_else(|| Self::stmt_kind_label(stmt));
            self.insns.truncate(start);
            self.next_reg = start_reg;
            self.emit(Insn::StmtFallback(Box::new((Arc::new(stmt.clone()), reason))));
            self.bail_reason = saved_reason;
            return true;
        }
        false
    }

    fn compile_stmt_strict(&mut self, stmt: &Statement) -> bool {
        match &stmt.kind {
            StatementKind::Null => true,
            StatementKind::NonblockingAssign { lvalue, rvalue, .. } => {
                let width = self.infer_lhs_width(lvalue);
                let start = self.insns.len();
                let start_reg = self.next_reg;
                if let Some(val_reg) = self.compile_expr(rvalue, width) {
                    // Note: NbaAssign itself performs §10.7 assignment-padding resize,
                    // so we don't emit a generic (zero-extending) Resize here — that
                    // would strip X/Z from the MSB before the assignment could X/Z-extend.
                    if self.compile_nba_target(lvalue, val_reg, width) {
                        return true;
                    }
                    self.bail("nba_target");
                } else {
                    self.bail("nba_rvalue");
                }
                // Roll back partial work and emit fallback if allowed.
                self.insns.truncate(start);
                self.next_reg = start_reg;
                self.emit_fallback(stmt)
            }
            StatementKind::BlockingAssign { lvalue, rvalue } => {
                let width = self.infer_lhs_width(lvalue);
                let start = self.insns.len();
                let start_reg = self.next_reg;
                if let Some(val_reg) = self.compile_expr(rvalue, width) {
                    if width > 0 {
                        self.emit(Insn::Resize(val_reg, width));
                    }
                    if self.compile_blocking_target(lvalue, val_reg, width) {
                        return true;
                    }
                    self.bail("blocking_target");
                } else {
                    self.bail("blocking_rvalue");
                }
                self.insns.truncate(start);
                self.next_reg = start_reg;
                self.emit_fallback(stmt)
            }
            StatementKind::If { condition, then_stmt, else_stmt, .. } => {
                if let Some(cond_reg) = self.compile_expr(condition, 0) {
                    let branch_idx = self.insns.len();
                    self.emit(Insn::BranchIfFalse(cond_reg, 0)); // placeholder target
                    if !self.compile_stmt(then_stmt) { return false; }
                    if let Some(el) = else_stmt {
                        let jump_idx = self.insns.len();
                        self.emit(Insn::Jump(0)); // placeholder
                        let else_start = self.insns.len() as u32;
                        self.insns[branch_idx] = Insn::BranchIfFalse(cond_reg, else_start);
                        if !self.compile_stmt(el) { return false; }
                        let end = self.insns.len() as u32;
                        self.insns[jump_idx] = Insn::Jump(end);
                    } else {
                        let end = self.insns.len() as u32;
                        self.insns[branch_idx] = Insn::BranchIfFalse(cond_reg, end);
                    }
                    true
                } else {
                    false
                }
            }
            StatementKind::Case { expr, items, .. } => {
                if let Some(val_reg) = self.compile_expr(expr, 0) {
                    let mut end_jumps: Vec<usize> = Vec::new();
                    let mut default_item: Option<&Statement> = None;
                    for item in items {
                        if item.is_default {
                            default_item = Some(&item.stmt);
                            continue;
                        }
                        // Compile pattern match: val === pattern
                        for pat in &item.patterns {
                            if let Some(pat_reg) = self.compile_expr(pat, 0) {
                                let cmp_reg = self.alloc_reg();
                                self.emit(Insn::CaseEq(cmp_reg, val_reg, pat_reg));
                                let branch_idx = self.insns.len();
                                self.emit(Insn::BranchIfFalse(cmp_reg, 0));
                                if !self.compile_stmt(&item.stmt) { return false; }
                                end_jumps.push(self.insns.len());
                                self.emit(Insn::Jump(0));
                                let next = self.insns.len() as u32;
                                self.insns[branch_idx] = Insn::BranchIfFalse(cmp_reg, next);
                            } else {
                                return false;
                            }
                        }
                    }
                    // Default case
                    if let Some(def_stmt) = default_item {
                        if !self.compile_stmt(def_stmt) { return false; }
                    }
                    let end = self.insns.len() as u32;
                    for idx in end_jumps {
                        self.insns[idx] = Insn::Jump(end);
                    }
                    true
                } else {
                    false
                }
            }
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                for s in stmts {
                    if !self.compile_stmt(s) { return false; }
                }
                true
            }
            // Bail out on anything else (timing controls, loops, system tasks, etc.)
            StatementKind::Expr(e) => {
                match &e.kind {
                    // Bare identifier as statement: side-effect-free read, compile as no-op
                    // — BUT only if it actually resolves to a signal. A bare ident that
                    // doesn't resolve is typically a task-enable (`task_name;`) whose
                    // dispatch must happen in the AST interpreter's `exec_expr_stmt`.
                    ExprKind::Ident(hier) if hier.path.len() == 1 => {
                        if self.lookup_signal_id(hier).is_some() { return true; }
                        let name = hier.path[0].name.name.clone();
                        if self.try_inline_task(&name) { return true; }
                        self.bail("Expr_TaskEnable");
                        return self.emit_fallback(stmt);
                    }
                    ExprKind::Ident(hier) if hier.path.len() > 1 => {
                        let mname = hier.path.last().unwrap().name.name.as_str();
                        if matches!(mname, "delete" | "sort" | "rsort" | "reverse" | "unique" | "unique_index" | "pop_front" | "pop_back") {
                            return self.emit_fallback(&Statement::new(stmt.kind.clone(), stmt.span));
                        }
                        if self.lookup_signal_id(hier).is_some() { return true; }
                        let leaf = hier.path.last().unwrap().name.name.clone();
                        if self.try_inline_task(&leaf) { return true; }
                        self.bail("Expr_TaskEnable");
                        return self.emit_fallback(stmt);
                    }
                    ExprKind::Number(_) | ExprKind::Paren(_) => {
                        return true;
                    }
                    // Pre/post increment/decrement have side effects — compile them
                    ExprKind::Unary { op: UnaryOp::PreIncr, operand } |
                    ExprKind::Unary { op: UnaryOp::PostIncr, operand } => {
                        if let Some(sig_id) = self.expr_to_signal_id(operand) {
                            let r = self.alloc_reg();
                            self.emit(Insn::LoadSignal(r, sig_id));
                            let one = self.alloc_reg();
                            let w = self.signal_widths[sig_id];
                            self.emit(Insn::LoadConst(one, Box::new(Value::from_u64(1, w))));
                            let result = self.alloc_reg();
                            self.emit(Insn::Add(result, r, one));
                            self.emit(Insn::Resize(result, w));
                            self.emit(Insn::BlockingAssign(sig_id, result, w));
                            return true;
                        }
                        self.bail("Expr_PreIncr");
                        return self.emit_fallback(stmt);
                    }
                    ExprKind::Unary { op: UnaryOp::PreDecr, operand } |
                    ExprKind::Unary { op: UnaryOp::PostDecr, operand } => {
                        if let Some(sig_id) = self.expr_to_signal_id(operand) {
                            let r = self.alloc_reg();
                            self.emit(Insn::LoadSignal(r, sig_id));
                            let one = self.alloc_reg();
                            let w = self.signal_widths[sig_id];
                            self.emit(Insn::LoadConst(one, Box::new(Value::from_u64(1, w))));
                            let result = self.alloc_reg();
                            self.emit(Insn::Sub(result, r, one));
                            self.emit(Insn::Resize(result, w));
                            self.emit(Insn::BlockingAssign(sig_id, result, w));
                            return true;
                        }
                        self.bail("Expr_PreDecr");
                        return self.emit_fallback(stmt);
                    }
                    _ => {}
                }
                let n: &'static str = match &e.kind {
                    ExprKind::SystemCall { name, .. } => match name.as_str() {
                        "$display" => "Expr_display",
                        "$write" => "Expr_write",
                        "$strobe" => "Expr_strobe",
                        "$monitor" => "Expr_monitor",
                        "$finish" => "Expr_finish",
                        "$stop" => "Expr_stop",
                        _ => "Expr_syscall_other",
                    },
                    ExprKind::Call { .. } => "Expr_Call",
                    ExprKind::Binary { .. } => "Expr_Binary",
                    ExprKind::Concatenation(_) => "Expr_Concat",
                    ExprKind::Replication { .. } => "Expr_Replication",
                    ExprKind::MemberAccess { .. } => "Expr_MemberAccess",
                    ExprKind::AssignmentPattern(_) => "Expr_AsgnPat",
                    ExprKind::Index { .. } => "Expr_Index",
                    ExprKind::RangeSelect { .. } => "Expr_RangeSelect",
                    ExprKind::Conditional { .. } => "Expr_Conditional",
                    _ => "Expr_other",
                };
                self.bail(n);
                self.emit_fallback(stmt)
            }
            StatementKind::For { init, condition, step, body } => {
                // break/continue inside a compiled loop body would set the AST
                // interpreter's break_flag which the bytecode VM never reads —
                // bail so the whole For falls back to the AST path.
                if Self::stmt_has_break_or_continue(body) {
                    self.bail("For_break_continue");
                    return false;
                }
                // Save outer for-loop overrides so nested loops don't leak.
                let saved_for_vars = std::mem::take(&mut self.for_loop_var_ids);
                // Inherit the outer overrides too — a nested loop's body
                // can still reference the outer counter.
                self.for_loop_var_ids = saved_for_vars.clone();
                for fi in init {
                    match fi {
                        ForInit::Assign { lvalue, rvalue } => {
                            let width = self.infer_lhs_width(lvalue);
                            let val_reg = match self.compile_expr(rvalue, width) {
                                Some(r) => r,
                                None => { self.bail("For_init_rvalue"); return false; }
                            };
                            if width > 0 { self.emit(Insn::Resize(val_reg, width)); }
                            if !self.compile_blocking_target(lvalue, val_reg, width) {
                                self.bail("For_init_target"); return false;
                            }
                            // Capture init's lvalue signal_id keyed by leaf
                            // name. The for-loop's step / body expressions
                            // often re-parse bare-ident references that the
                            // elaborator did not scope-qualify (only init's
                            // lvalue gets qualified through an elaboration
                            // path). Without this, a bare `i` in step
                            // `i = i+1` collides with an unrelated top-level
                            // signal of the same name and resolves to the
                            // wrong signal_id. On c910 the always-block
                            // counter was clobbering the testbench's
                            // top-level `integer i` (signal_id 9), and the
                            // actual counter never advanced — the loop ran
                            // forever (10M+ insns per call, hung the sim
                            // in iter 1 of the event loop).
                            // Capture init's resolved signal_id keyed by the
                            // *leaf* of the lvalue's hier path. The
                            // elaborator may have rewritten init's lvalue
                            // from bare `i` to a multi-segment `module.i`
                            // form (which is why init resolves correctly
                            // to the module-local id), while leaving the
                            // for-step's bare `i` untouched. Capturing by
                            // leaf bridges that asymmetry: bare `i` in step
                            // gets re-routed to init's resolved id.
                            if let ExprKind::Ident(hier) = &lvalue.kind {
                                let leaf = if hier.path.len() == 1 && hier.path[0].name.name.contains('.') {
                                    // Parser flattened a hier path into one segment with dots.
                                    hier.path[0].name.name.rsplit('.').next().unwrap_or("").to_string()
                                } else {
                                    hier.path.last().map(|s| s.name.name.clone()).unwrap_or_default()
                                };
                                if !leaf.is_empty() && !leaf.contains('.') {
                                    if let Some(id) = self.lookup_signal_id(hier) {
                                        self.for_loop_var_ids.insert(leaf, id);
                                    }
                                }
                            }
                        }
                        ForInit::VarDecl { .. } => {
                            self.for_loop_var_ids = saved_for_vars;
                            self.bail("For_init_vardecl"); return false;
                        }
                    }
                }
                let loop_start = self.insns.len() as u32;
                let cond_branch_idx = if let Some(c) = condition {
                    let cond_reg = match self.compile_expr(c, 0) {
                        Some(r) => r,
                        None => { self.bail("For_condition"); return false; }
                    };
                    let idx = self.insns.len();
                    self.emit(Insn::BranchIfFalse(cond_reg, 0));
                    Some(idx)
                } else { None };
                if !self.compile_stmt(body) { return false; }
                for s in step {
                    // For-loop step can be either the legacy `Binary{Assign,…}`
                    // shape or the newer `AssignExpr { lvalue, rvalue }` emitted
                    // by the parser for `i = i+1` / `i += 2` / etc. after
                    // xezim-core 8b9c88c (ibex parsing). Both collapse to a
                    // blocking assign.
                    let (lhs, rhs) = match &s.kind {
                        ExprKind::Binary { op: BinaryOp::Assign, left, right } => (&**left, &**right),
                        ExprKind::AssignExpr { lvalue, rvalue } => (&**lvalue, &**rvalue),
                        _ => { self.bail("For_step_other"); return false; }
                    };
                    let width = self.infer_lhs_width(lhs);
                    let val_reg = match self.compile_expr(rhs, width) {
                        Some(r) => r,
                        None => { self.bail("For_step_rvalue"); return false; }
                    };
                    if width > 0 { self.emit(Insn::Resize(val_reg, width)); }
                    if !self.compile_blocking_target(lhs, val_reg, width) {
                        self.bail("For_step_target"); return false;
                    }
                }
                self.emit(Insn::Jump(loop_start));
                let end = self.insns.len() as u32;
                if let Some(idx) = cond_branch_idx {
                    if let Insn::BranchIfFalse(reg, _) = self.insns[idx] {
                        self.insns[idx] = Insn::BranchIfFalse(reg, end);
                    }
                }
                // Restore outer for-loop's override map.
                self.for_loop_var_ids = saved_for_vars;
                true
            }
            other => {
                let name: &'static str = match other {
                    StatementKind::Expr(_) => "Expr",
                    StatementKind::For { .. } => "For",
                    StatementKind::Foreach { .. } => "Foreach",
                    StatementKind::While { .. } => "While",
                    StatementKind::DoWhile { .. } => "DoWhile",
                    StatementKind::Repeat { .. } => "Repeat",
                    StatementKind::Forever { .. } => "Forever",
                    StatementKind::TimingControl { .. } => "TimingControl",
                    StatementKind::EventTrigger { .. } => "EventTrigger",
                    StatementKind::Wait { .. } => "Wait",
                    StatementKind::WaitFork => "WaitFork",
                    StatementKind::Disable(_) => "Disable",
                    StatementKind::Return(_) => "Return",
                    StatementKind::Break => "Break",
                    StatementKind::Continue => "Continue",
                    StatementKind::Assertion(_) => "Assertion",
                    StatementKind::ProceduralContinuous(_) => "ProceduralContinuous",
                    StatementKind::VarDecl { .. } => "VarDecl",
                    StatementKind::Coverpoint { .. } => "Coverpoint",
                    StatementKind::Cross { .. } => "Cross",
                    _ => "Other",
                };
                self.bail_reason = Some(name);
                self.emit_fallback(stmt)
            }
        }
    }

    /// Compile an expression, returning the register holding the result.
    /// Returns None if the expression can't be compiled to bytecode.
    fn compile_expr(&mut self, expr: &Expression, ctx_width: u32) -> Option<RegId> {
        match &expr.kind {
            ExprKind::Number(num) => {
                let val = self.eval_number_static(num)?;
                let r = self.alloc_reg();
                self.emit(Insn::LoadConst(r, Box::new(val)));
                Some(r)
            }
            ExprKind::Ident(hier) => {
                let id = self.lookup_signal_id(hier)?;
                let r = self.alloc_reg();
                if self.signal_signed[id] {
                    self.emit(Insn::LoadSignalSigned(r, id));
                } else {
                    self.emit(Insn::LoadSignal(r, id));
                }
                Some(r)
            }
            ExprKind::Unary { op, operand } => {
                let src = self.compile_expr(operand, ctx_width)?;
                let dest = self.alloc_reg();
                match op {
                    UnaryOp::Plus => return Some(src),
                    UnaryOp::Minus => self.emit(Insn::Negate(dest, src)),
                    UnaryOp::Plus => self.emit(Insn::Move(dest, src)),
                    UnaryOp::LogNot => self.emit(Insn::LogNot(dest, src)),
                    UnaryOp::BitNot => self.emit(Insn::BitNot(dest, src)),
                    UnaryOp::BitAnd => self.emit(Insn::ReduceAnd(dest, src)),
                    UnaryOp::BitNand => {
                        self.emit(Insn::ReduceAnd(dest, src));
                        self.emit(Insn::LogNot(dest, dest));
                    }
                    UnaryOp::BitOr => self.emit(Insn::ReduceOr(dest, src)),
                    UnaryOp::BitNor => {
                        self.emit(Insn::ReduceOr(dest, src));
                        self.emit(Insn::LogNot(dest, dest));
                    }
                    UnaryOp::BitXor => self.emit(Insn::ReduceXor(dest, src)),
                    UnaryOp::BitXnor => {
                        self.emit(Insn::ReduceXor(dest, src));
                        self.emit(Insn::LogNot(dest, dest));
                    }
                    _ => { self.bail("UnaryOp_other"); return None; }
                }
                Some(dest)
            }
            ExprKind::Binary { op, left, right } => {
                let l = self.compile_expr(left, ctx_width)?;
                let r = self.compile_expr(right, ctx_width)?;
                // Context width resizing for arithmetic
                if ctx_width > 0 && matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul
                    | BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor | BinaryOp::BitXnor) {
                    self.emit(Insn::Resize(l, ctx_width));
                    self.emit(Insn::Resize(r, ctx_width));
                }
                let dest = self.alloc_reg();
                match op {
                    BinaryOp::Add => self.emit(Insn::Add(dest, l, r)),
                    BinaryOp::Sub => self.emit(Insn::Sub(dest, l, r)),
                    BinaryOp::Mul => self.emit(Insn::Mul(dest, l, r)),
                    BinaryOp::Div => self.emit(Insn::Div(dest, l, r)),
                    BinaryOp::Mod => self.emit(Insn::Mod(dest, l, r)),
                    BinaryOp::BitAnd => self.emit(Insn::BitAnd(dest, l, r)),
                    BinaryOp::BitOr => self.emit(Insn::BitOr(dest, l, r)),
                    BinaryOp::BitXor => self.emit(Insn::BitXor(dest, l, r)),
                    BinaryOp::BitXnor => self.emit(Insn::BitXnor(dest, l, r)),
                    BinaryOp::LogAnd => self.emit(Insn::LogAnd(dest, l, r)),
                    BinaryOp::LogOr => self.emit(Insn::LogOr(dest, l, r)),
                    BinaryOp::Eq => self.emit(Insn::Eq(dest, l, r)),
                    BinaryOp::Neq => self.emit(Insn::Neq(dest, l, r)),
                    BinaryOp::CaseEq => self.emit(Insn::CaseEq(dest, l, r)),
                    BinaryOp::Lt => self.emit(Insn::Lt(dest, l, r)),
                    BinaryOp::Leq => self.emit(Insn::Leq(dest, l, r)),
                    BinaryOp::Gt => self.emit(Insn::Gt(dest, l, r)),
                    BinaryOp::Geq => self.emit(Insn::Geq(dest, l, r)),
                    BinaryOp::ShiftLeft | BinaryOp::ArithShiftLeft => {
                        if ctx_width > 0 { self.emit(Insn::Resize(l, ctx_width)); }
                        self.emit(Insn::Shl(dest, l, r));
                    }
                    BinaryOp::ShiftRight => self.emit(Insn::Shr(dest, l, r)),
                    BinaryOp::ArithShiftRight => self.emit(Insn::AShr(dest, l, r)),
                    _ => { self.bail("BinaryOp_other"); return None; }
                }
                Some(dest)
            }
            ExprKind::Conditional { condition, then_expr, else_expr } => {
                // Evaluate both branches unconditionally so Select can do a
                // per-bit merge when the condition has X/Z (IEEE 1800 §11.4.11).
                let cond = self.compile_expr(condition, 0)?;
                let then_reg = self.compile_expr(then_expr, ctx_width)?;
                let else_reg = self.compile_expr(else_expr, ctx_width)?;
                let dest = self.alloc_reg();
                self.emit(Insn::Select(dest, cond, then_reg, else_reg));
                Some(dest)
            }
            ExprKind::Paren(inner) => self.compile_expr(inner, ctx_width),
            ExprKind::Index { expr, index } => {
                // Array element access
                if let ExprKind::Ident(hier) = &expr.kind {
                    if let Some(name) = self.lookup_array_name(hier) {
                        let idx_reg = self.compile_expr(index, 0)?;
                        let dest = self.alloc_reg();
                        self.emit(Insn::LoadArrayElem(dest, Box::new(name), idx_reg));
                        return Some(dest);
                    }
                }
                // Bit select
                let base = self.compile_expr(expr, 0)?;
                let idx = self.compile_expr(index, 0)?;
                let dest = self.alloc_reg();
                self.emit(Insn::BitSelect(dest, base, idx));
                Some(dest)
            }
            ExprKind::RangeSelect { expr, left, right, kind, .. } => {
                match kind {
                    RangeKind::Constant => {
                        let base = self.compile_expr(expr, 0)?;
                        let l = self.compile_expr(left, 0)?;
                        let r = self.compile_expr(right, 0)?;
                        let dest = self.alloc_reg();
                        self.emit(Insn::RangeSelect(dest, base, l, r));
                        Some(dest)
                    }
                    RangeKind::IndexedUp | RangeKind::IndexedDown => {
                        // `sig[idx +: W]` / `sig[idx -: W]` — W must be constant.
                        // Emit idx register, then compute hi/lo via Add/Sub with a
                        // const (W-1), and reuse existing RangeSelect insn.
                        let width = match self.eval_const_expr(right) {
                            Some(w) if w > 0 => w,
                            _ => { self.bail("RangeSelect_width_nonconst"); return None; }
                        };
                        let base = self.compile_expr(expr, 0)?;
                        let idx = self.compile_expr(left, 0)?;
                        let dest = self.alloc_reg();
                        if width == 1 {
                            self.emit(Insn::RangeSelect(dest, base, idx, idx));
                        } else {
                            let delta = self.alloc_reg();
                            self.emit(Insn::LoadConst(delta, Box::new(Value::from_u64((width - 1) as u64, 32))));
                            let other = self.alloc_reg();
                            if *kind == RangeKind::IndexedUp {
                                self.emit(Insn::Add(other, idx, delta));
                                self.emit(Insn::RangeSelect(dest, base, other, idx));
                            } else {
                                self.emit(Insn::Sub(other, idx, delta));
                                self.emit(Insn::RangeSelect(dest, base, idx, other));
                            }
                        }
                        Some(dest)
                    }
                }
            }
            ExprKind::Replication { count, exprs } => {
                let n = self.eval_const_expr(count)?;
                if n == 0 || n > 1024 { self.bail("Replication_bad_count"); return None; }
                let mut regs = Vec::with_capacity((exprs.len() * n as usize).max(1));
                for _ in 0..n {
                    for e in exprs {
                        let r = self.compile_expr(e, 0)?;
                        regs.push(r);
                    }
                }
                let dest = self.alloc_reg();
                self.emit(Insn::Concat(dest, Box::new(regs)));
                Some(dest)
            }
            ExprKind::Concatenation(parts) => {
                let mut regs = Vec::new();
                for p in parts {
                    let r = self.compile_expr(p, 0)?;
                    regs.push(r);
                }
                let dest = self.alloc_reg();
                self.emit(Insn::Concat(dest, Box::new(regs)));
                Some(dest)
            }
            ExprKind::SystemCall { name, args } => {
                match name.as_str() {
                    "$signed" => {
                        let r = self.compile_expr(args.first()?, 0)?;
                        self.emit(Insn::SetSigned(r));
                        Some(r)
                    }
                    "$unsigned" => {
                        let r = self.compile_expr(args.first()?, 0)?;
                        Some(r)
                    }
                    other => { self.bail("SystemCall_other"); let _ = other; None }
                }
            }
            other => {
                let n: &'static str = match other {
                    ExprKind::StringLiteral(_) => "Expr_StringLiteral",
                    ExprKind::Replication { .. } => "Expr_Replication",
                    ExprKind::AssignmentPattern(_) => "Expr_AssignmentPattern",
                    ExprKind::Call { .. } => "Expr_Call",
                    ExprKind::Inside { .. } => "Expr_Inside",
                    ExprKind::MemberAccess { .. } => "Expr_MemberAccess",
                    ExprKind::Range(..) => "Expr_Range",
                    ExprKind::NamedArg { .. } => "Expr_NamedArg",
                    _ => "Expr_other",
                };
                self.bail(n);
                None
            }
        }
    }

    fn compile_nba_target(&mut self, lhs: &Expression, val_reg: RegId, width: u32) -> bool {
        match &lhs.kind {
            ExprKind::Ident(hier) => {
                if let Some(id) = self.lookup_signal_id(hier) {
                    self.emit(Insn::NbaAssign(id, val_reg, width));
                    true
                } else {
                    self.bail("nba_ident_unresolved");
                    false
                }
            }
            ExprKind::Index { expr, index } => {
                if let ExprKind::Ident(hier) = &expr.kind {
                    if let Some(name) = self.lookup_array_name(hier) {
                        if let Some(idx_reg) = self.compile_expr(index, 0) {
                            self.emit(Insn::NbaAssignArray(Box::new(name), idx_reg, val_reg, width));
                            return true;
                        }
                    }
                    if let Some(id) = self.lookup_signal_id(hier) {
                        if let Some(idx_reg) = self.compile_expr(index, 0) {
                            self.emit(Insn::NbaAssignBitDyn(id, idx_reg, val_reg));
                            return true;
                        }
                    }
                }
                self.bail("nba_index_other");
                false
            }
            ExprKind::RangeSelect { expr, left, right, kind } => {
                if *kind != RangeKind::Constant { self.bail("nba_range_nonconst"); return false; }
                if let ExprKind::Ident(hier) = &expr.kind {
                    if let Some(id) = self.lookup_signal_id(hier) {
                        // Fast path: compile-time constant range.
                        if let (Some(hi), Some(lo)) = (self.eval_const_expr(left), self.eval_const_expr(right)) {
                            self.emit(Insn::NbaAssignRange(id, hi, lo, val_reg));
                            let _ = width;
                            return true;
                        }
                        // Dynamic range: compile left/right as expressions.
                        // Avoids the AST-interpreter fallback for patterns
                        // like `q[idx+:W]` or `q[j:j-W+1]` that fire
                        // millions of times per c910 run.
                        if let (Some(hi_reg), Some(lo_reg)) =
                            (self.compile_expr(left, 0), self.compile_expr(right, 0))
                        {
                            self.emit(Insn::NbaAssignRangeDyn(id, hi_reg, lo_reg, val_reg));
                            let _ = width;
                            return true;
                        }
                    }
                }
                self.bail("nba_range_unresolved");
                false
            }
            ExprKind::Concatenation(parts) => {
                // {a, b, c} <= value: split value into per-part bit ranges and NBA each part.
                // Concatenation is MSB-first: parts[0] is the highest bits.
                // The RHS may be narrower than the concat width (e.g. $signed of a
                // 12-bit expression assigned to a 32-bit concat LHS). Widen first
                // so the per-part RangeSelects see properly sign/zero-extended bits.
                if width > 0 {
                    self.emit(Insn::Resize(val_reg, width));
                }
                let mut part_widths = Vec::with_capacity(parts.len());
                for p in parts {
                    let w = self.infer_lhs_width(p);
                    part_widths.push(w);
                }
                let mut bit_offset: u32 = 0;
                for (i, p) in parts.iter().enumerate().rev() {
                    let pw = part_widths[i];
                    let lo_reg = self.alloc_reg();
                    self.emit(Insn::LoadConst(lo_reg, Box::new(Value::from_u64(bit_offset as u64, 32))));
                    let hi_val = bit_offset + pw - 1;
                    let hi_reg = self.alloc_reg();
                    self.emit(Insn::LoadConst(hi_reg, Box::new(Value::from_u64(hi_val as u64, 32))));
                    let part_reg = self.alloc_reg();
                    self.emit(Insn::RangeSelect(part_reg, val_reg, hi_reg, lo_reg));
                    self.emit(Insn::Resize(part_reg, pw));
                    if !self.compile_nba_target(p, part_reg, pw) {
                        return false;
                    }
                    bit_offset += pw;
                }
                true
            }
            ExprKind::MemberAccess { .. } => { self.bail("nba_member_access"); false }
            _ => { self.bail("nba_other"); false }
        }
    }

    fn compile_blocking_target(&mut self, lhs: &Expression, val_reg: RegId, width: u32) -> bool {
        match &lhs.kind {
            ExprKind::Ident(hier) => {
                if let Some(id) = self.lookup_signal_id(hier) {
                    self.emit(Insn::BlockingAssign(id, val_reg, width));
                    true
                } else {
                    self.bail("blocking_target");
                    false
                }
            }
            ExprKind::Index { expr, index } => {
                if let ExprKind::Ident(hier) = &expr.kind {
                    if let Some(name) = self.lookup_array_name(hier) {
                        if let Some(idx_reg) = self.compile_expr(index, 0) {
                            let elem_name_reg = self.alloc_reg();
                            let _ = elem_name_reg;
                            // For blocking array assign, we need to compute the element name
                            // and do a blocking assign. Use the same pattern as NbaAssignArray
                            // but with a blocking write.
                            // BlockingAssignArray doesn't exist yet — fall back.
                            let _ = idx_reg;
                            self.bail("blocking_array");
                            return false;
                        }
                    }
                    if let Some(id) = self.lookup_signal_id(hier) {
                        if let Some(idx_reg) = self.compile_expr(index, 0) {
                            self.emit(Insn::BlockingAssignBitDyn(id, idx_reg, val_reg));
                            return true;
                        }
                    }
                }
                self.bail("blocking_target");
                false
            }
            ExprKind::RangeSelect { expr, left, right, kind } => {
                if let ExprKind::Ident(hier) = &expr.kind {
                    if let Some(id) = self.lookup_signal_id(hier) {
                        match kind {
                            RangeKind::Constant => {
                                if let (Some(hi), Some(lo)) = (self.eval_const_expr(left), self.eval_const_expr(right)) {
                                    let (low, high) = if hi >= lo { (lo, hi) } else { (hi, lo) };
                                    let range_w = high - low + 1;
                                    let resized = self.alloc_reg();
                                    self.emit(Insn::Move(resized, val_reg));
                                    self.emit(Insn::Resize(resized, range_w));
                                    self.emit(Insn::BlockingAssignRange(id, hi, lo, resized));
                                    return true;
                                }
                            }
                            RangeKind::IndexedUp | RangeKind::IndexedDown => {
                                let width = match self.eval_const_expr(right) {
                                    Some(w) if w > 0 => w,
                                    _ => { self.bail("blocking_range_width_nonconst"); return false; }
                                };
                                let resized = self.alloc_reg();
                                self.emit(Insn::Move(resized, val_reg));
                                self.emit(Insn::Resize(resized, width));
                                let Some(idx) = self.compile_expr(left, 0) else {
                                    self.bail("blocking_range_base"); return false;
                                };
                                let (hi_reg, lo_reg) = if width == 1 {
                                    (idx, idx)
                                } else {
                                    let delta = self.alloc_reg();
                                    self.emit(Insn::LoadConst(delta, Box::new(Value::from_u64((width - 1) as u64, 32))));
                                    let other = self.alloc_reg();
                                    if *kind == RangeKind::IndexedUp {
                                        self.emit(Insn::Add(other, idx, delta));
                                        (other, idx)
                                    } else {
                                        self.emit(Insn::Sub(other, idx, delta));
                                        (idx, other)
                                    }
                                };
                                self.emit(Insn::BlockingAssignRangeDyn(id, hi_reg, lo_reg, resized));
                                return true;
                            }
                        }
                    }
                }
                self.bail("blocking_target");
                false
            }
            ExprKind::Concatenation(parts) => {
                let mut part_widths = Vec::with_capacity(parts.len());
                for p in parts {
                    let w = self.infer_lhs_width(p);
                    part_widths.push(w);
                }
                let mut bit_offset: u32 = 0;
                for (i, p) in parts.iter().enumerate().rev() {
                    let pw = part_widths[i];
                    let lo_reg = self.alloc_reg();
                    self.emit(Insn::LoadConst(lo_reg, Box::new(Value::from_u64(bit_offset as u64, 32))));
                    let hi_val = bit_offset + pw - 1;
                    let hi_reg = self.alloc_reg();
                    self.emit(Insn::LoadConst(hi_reg, Box::new(Value::from_u64(hi_val as u64, 32))));
                    let part_reg = self.alloc_reg();
                    self.emit(Insn::RangeSelect(part_reg, val_reg, hi_reg, lo_reg));
                    self.emit(Insn::Resize(part_reg, pw));
                    if !self.compile_blocking_target(p, part_reg, pw) {
                        return false;
                    }
                    bit_offset += pw;
                }
                true
            }
            _ => {
                self.bail("blocking_target");
                false
            }
        }
    }

    pub fn infer_lhs_width_pub(&self, lhs: &Expression) -> u32 { self.infer_lhs_width(lhs) }

    fn infer_lhs_width(&self, lhs: &Expression) -> u32 {
        match &lhs.kind {
            ExprKind::Ident(hier) => {
                if let Some(id) = self.lookup_signal_id(hier) {
                    self.signal_widths[id]
                } else {
                    let raw = Self::hier_raw_name(hier);
                    self.widths.get(&raw).copied().unwrap_or(32)
                }
            }
            ExprKind::Index { expr, .. } => {
                if let ExprKind::Ident(hier) = &expr.kind {
                    if let Some(name) = self.lookup_array_name(hier) {
                        if let Some((_, _, elem_w)) = self.arrays.get(&name) {
                            return *elem_w;
                        }
                    }
                    let raw = Self::hier_raw_name(hier);
                    if let Some((_, _, elem_w)) = self.arrays.get(&raw) {
                        return *elem_w;
                    }
                    // Not an array — it's a bit-select on a packed signal; width = 1.
                    1
                } else { 32 }
            }
            ExprKind::RangeSelect { left, right, kind, .. } => {
                match kind {
                    RangeKind::IndexedUp | RangeKind::IndexedDown => {
                        self.eval_const_expr(right).unwrap_or(32)
                    }
                    RangeKind::Constant => {
                        if let (Some(l), Some(r)) = (self.eval_const_expr(left), self.eval_const_expr(right)) {
                            let (hi, lo) = if l >= r { (l, r) } else { (r, l) };
                            hi - lo + 1
                        } else { 32 }
                    }
                }
            }
            ExprKind::Concatenation(parts) => parts.iter().map(|p| self.infer_lhs_width(p)).sum(),
            _ => 32,
        }
    }

    fn eval_const_expr(&self, e: &Expression) -> Option<u32> {
        match &e.kind {
            ExprKind::Number(n) => self.eval_number_static(n)?.to_u64().map(|v| v as u32),
            ExprKind::Paren(inner) => self.eval_const_expr(inner),
            ExprKind::Ident(hier) => {
                let params = self.params?;
                let raw = Self::hier_raw_name(hier);
                if let Some(v) = params.get(&raw) { return v.to_u64().map(|u| u as u32); }
                if let Some(scope) = &self.scope_hint {
                    let q = format!("{}.{}", scope, raw);
                    if let Some(v) = params.get(&q) { return v.to_u64().map(|u| u as u32); }
                }
                if hier.path.len() == 1 {
                    if let Some(v) = params.get(&hier.path[0].name.name) {
                        return v.to_u64().map(|u| u as u32);
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn eval_number_static(&self, num: &NumberLiteral) -> Option<Value> {
        match num {
            NumberLiteral::Integer { size, signed, base, value, cached_val } => {
                let w = size.unwrap_or(32);
                if let Some((vb, xz, cw)) = cached_val.get() {
                    if cw == w {
                        let mut v = Value::from_inline(vb, xz, w);
                        v.is_signed = *signed;
                        return Some(v);
                    }
                }
                let r = match base { NumberBase::Binary => 2, NumberBase::Octal => 8, NumberBase::Hex => 16, NumberBase::Decimal => 10 };
                let mut v = Value::from_str_radix(value, r, w);
                v.is_signed = *signed;
                Some(v)
            }
            NumberLiteral::Real(f) => Some(Value::from_u64(*f as u64, 64)),
            NumberLiteral::UnbasedUnsized(c) => Some(match c {
                '0' => Value::zero(32),
                '1' => Value::ones(32),
                'x' | 'X' => Value::new(32),
                'z' | 'Z' => Value::all_z(32),
                _ => Value::new(32),
            }),
        }
    }

    /// Compile a continuous assign: evaluate RHS, write to pre-resolved LHS.
    /// Returns true if compiled successfully.
    pub fn compile_cont_assign(&mut self, rhs: &Expression, dst_id: usize, width: u32) -> bool {
        // Verilog context width = max(LHS width, max operand width in RHS).
        // Using just the LHS width truncates intermediates when operands
        // (e.g. 32-bit parameters) are wider than the target wire.
        let ctx = width.max(self.expr_max_width(rhs));
        if let Some(val_reg) = self.compile_expr(rhs, ctx) {
            self.emit(Insn::Resize(val_reg, width));
            self.emit(Insn::BlockingAssign(dst_id, val_reg, width));
            true
        } else {
            false
        }
    }

    /// Compile a continuous assign with bit-select, part-select, or concat LHS:
    /// `assign d[i] = rhs`, `assign d[hi:lo] = rhs`, `assign {a,b} = rhs`.
    /// Reuses compile_blocking_target which emits BlockingAssignBitDyn /
    /// BlockingAssignRange / concat-split insns — same sub-range semantics
    /// as the interpreted assign_value path, but at bytecode speed.
    /// Yosys gate-level netlists emit hundreds of per-bit assigns that used
    /// to fall through to the interpreter on every settle iteration.
    pub fn compile_cont_assign_lhs(&mut self, lhs: &Expression, rhs: &Expression, lhs_width: u32) -> bool {
        let ctx = lhs_width.max(self.expr_max_width(rhs));
        if let Some(val_reg) = self.compile_expr(rhs, ctx) {
            self.emit(Insn::Resize(val_reg, lhs_width));
            self.compile_blocking_target(lhs, val_reg, lhs_width)
        } else {
            false
        }
    }

    fn expr_max_width(&self, expr: &Expression) -> u32 {
        match &expr.kind {
            ExprKind::Ident(hier) => {
                self.lookup_signal_id(hier)
                    .map(|id| self.signal_widths[id])
                    .unwrap_or(0)
            }
            ExprKind::Number(n) => {
                self.eval_number_static(n).map(|v| v.width).unwrap_or(32)
            }
            ExprKind::Binary { left, right, .. } => {
                self.expr_max_width(left).max(self.expr_max_width(right))
            }
            ExprKind::Unary { operand, .. } => self.expr_max_width(operand),
            ExprKind::Paren(inner) => self.expr_max_width(inner),
            ExprKind::Call { args, .. } => {
                args.iter().map(|a| self.expr_max_width(a)).max().unwrap_or(0)
            }
            ExprKind::Conditional { then_expr, else_expr, condition, .. } => {
                self.expr_max_width(condition)
                    .max(self.expr_max_width(then_expr))
                    .max(self.expr_max_width(else_expr))
            }
            ExprKind::Concatenation(parts) => {
                parts.iter().map(|p| self.expr_max_width(p)).sum()
            }
            _ => 0,
        }
    }

    pub fn finish(mut self) -> CompiledBlock {
        // Trim unused capacity. `Vec::push` doubles the backing buffer
        // when it overflows, so a freshly compiled block can sit on
        // up to ~50% slack capacity. With ~100K CompiledBlocks on
        // c910, that slack stacks into double-digit MB; one
        // `shrink_to_fit` per finish reclaims it.
        self.insns.shrink_to_fit();
        CompiledBlock {
            num_regs: self.next_reg,
            instructions: self.insns,
        }
    }
}
