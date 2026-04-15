//! Bytecode VM for high-performance simulation execution.
//! Compiles AST expressions and statements into a flat instruction array
//! that can be executed without pointer-chasing through Box<Expression> trees.

use super::value::Value;
use crate::ast::expr::*;
use crate::ast::stmt::*;
use std::sync::Arc;
use ahash::AHashMap as HashMap;

/// A register in the bytecode VM. Registers hold Values.
type RegId = u16;

/// Bytecode instruction set. Stack-free, register-based design.
/// Each instruction specifies source and destination registers explicitly,
/// enabling the VM to iterate a flat Vec<Insn> with predictable memory access.
#[derive(Debug, Clone)]
pub enum Insn {
    /// Load a constant value into a register.
    LoadConst(RegId, Value),
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
    /// Concatenation: dest = {parts...}, part register IDs stored in the Vec.
    Concat(RegId, Vec<RegId>),

    /// Conditional branch: if reg is false, jump to target instruction index.
    BranchIfFalse(RegId, u32),       // (cond_reg, jump_target)
    /// Unconditional jump.
    Jump(u32),

    /// Non-blocking assign: signal_table[id] <= reg (scheduled via NBA queue).
    NbaAssign(usize, RegId, u32),  // (signal_id, value_reg, width)
    /// Non-blocking partial assign: signal_table[id][hi:lo] <= reg.
    /// Read-modify-write at exec time using current signal value as base.
    NbaAssignRange(usize, u32, u32, RegId), // (signal_id, hi, lo, value_reg)
    /// Non-blocking bit assign: signal_table[id][bit_idx_reg] <= reg.
    NbaAssignBitDyn(usize, RegId, RegId), // (signal_id, idx_reg, value_reg)
    /// Blocking assign: signal_table[id] = reg.
    BlockingAssign(usize, RegId, u32), // (signal_id, value_reg, width)

    /// Load array element: dest = signal_table[array_base + eval(index_reg)]
    LoadArrayElem(RegId, String, RegId), // (dest, array_name, index_reg)
    /// NBA assign to array element.
    NbaAssignArray(String, RegId, RegId, u32), // (array_name, index_reg, value_reg, width)

    /// Marks end of a compiled block (no-op, helps debugging).
    /// Copy src register to dest register.
    Move(RegId, RegId),       // (dest, src)

    /// Fallback: invoke the AST interpreter on an untranslated statement.
    /// Used for rare constructs (e.g. $display, complex LHS) so an edge
    /// block containing one unsupported stmt can still run most of its
    /// body as fast bytecode instead of falling back wholesale to AST.
    StmtFallback(Arc<Statement>),

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
    signal_name_to_id: &'a HashMap<String, usize>,
    signal_signed: &'a [bool],
    signal_widths: &'a [u32],
    arrays: &'a HashMap<String, (i64, i64, u32)>,
    widths: &'a HashMap<String, u32>,
    pub bail_reason: Option<&'static str>,
    /// When true, unsupported statements emit `StmtFallback` instead of
    /// failing compilation. Safe for edge blocks where the AST interpreter's
    /// statement path is the same one used by the non-compiled fallback.
    pub allow_ast_fallback: bool,
}

impl<'a> BytecodeCompiler<'a> {
    pub fn new(
        signal_name_to_id: &'a HashMap<String, usize>,
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
        }
    }

    pub fn set_ast_fallback(&mut self, allow: bool) {
        self.allow_ast_fallback = allow;
    }

    fn emit_fallback(&mut self, stmt: &Statement) -> bool {
        if self.allow_ast_fallback {
            self.emit(Insn::StmtFallback(Arc::new(stmt.clone())));
            true
        } else {
            false
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
        if let Some(&id) = self.signal_name_to_id.get(&raw) {
            return Some(id);
        }
        if hier.path.len() == 1 {
            let leaf = &hier.path[0].name.name;
            return self.signal_name_to_id.get(leaf).copied();
        }
        None
    }

    fn lookup_array_name(&self, hier: &HierarchicalIdentifier) -> Option<String> {
        let raw = Self::hier_raw_name(hier);
        if self.arrays.contains_key(&raw) {
            return Some(raw);
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
        if self.compile_stmt_strict(stmt) {
            return true;
        }
        if self.allow_ast_fallback {
            self.insns.truncate(start);
            self.next_reg = start_reg;
            self.emit(Insn::StmtFallback(Arc::new(stmt.clone())));
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
                    if width > 0 {
                        self.emit(Insn::Resize(val_reg, width));
                    }
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
                    ExprKind::Unary { op, .. } => match op {
                        UnaryOp::PreIncr => "Expr_PreIncr",
                        UnaryOp::PreDecr => "Expr_PreDecr",
                        UnaryOp::PostIncr => "Expr_PostIncr",
                        UnaryOp::PostDecr => "Expr_PostDecr",
                        _ => "Expr_UnaryOther",
                    },
                    ExprKind::Binary { .. } => "Expr_Binary",
                    ExprKind::Ident(_) => "Expr_Ident",
                    ExprKind::Number(_) => "Expr_Number",
                    ExprKind::Paren(_) => "Expr_Paren",
                    ExprKind::Index { .. } => "Expr_Index",
                    ExprKind::RangeSelect { .. } => "Expr_RangeSelect",
                    ExprKind::Conditional { .. } => "Expr_Conditional",
                    ExprKind::Concatenation(_) => "Expr_Concat",
                    ExprKind::Replication { .. } => "Expr_Replication",
                    ExprKind::MemberAccess { .. } => "Expr_MemberAccess",
                    ExprKind::AssignmentPattern(_) => "Expr_AsgnPat",
                    _ => "Expr_other",
                };
                self.bail(n);
                self.emit_fallback(stmt)
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
                self.emit(Insn::LoadConst(r, val));
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
                    UnaryOp::LogNot => self.emit(Insn::LogNot(dest, src)),
                    UnaryOp::BitNot => self.emit(Insn::BitNot(dest, src)),
                    UnaryOp::BitAnd => self.emit(Insn::ReduceAnd(dest, src)),
                    UnaryOp::BitOr => self.emit(Insn::ReduceOr(dest, src)),
                    UnaryOp::BitXor => self.emit(Insn::ReduceXor(dest, src)),
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
                let cond = self.compile_expr(condition, 0)?;
                let dest = self.alloc_reg();
                let branch_idx = self.insns.len();
                self.emit(Insn::BranchIfFalse(cond, 0)); // placeholder
                let then_reg = self.compile_expr(then_expr, ctx_width)?;
                self.emit(Insn::Move(dest, then_reg));
                let jump_idx = self.insns.len();
                self.emit(Insn::Jump(0)); // placeholder
                let else_start = self.insns.len() as u32;
                self.insns[branch_idx] = Insn::BranchIfFalse(cond, else_start);
                let else_reg = self.compile_expr(else_expr, ctx_width)?;
                self.emit(Insn::Move(dest, else_reg));
                let end = self.insns.len() as u32;
                self.insns[jump_idx] = Insn::Jump(end);
                Some(dest)
            }
            ExprKind::Paren(inner) => self.compile_expr(inner, ctx_width),
            ExprKind::Index { expr, index } => {
                // Array element access
                if let ExprKind::Ident(hier) = &expr.kind {
                    if let Some(name) = self.lookup_array_name(hier) {
                        let idx_reg = self.compile_expr(index, 0)?;
                        let dest = self.alloc_reg();
                        self.emit(Insn::LoadArrayElem(dest, name, idx_reg));
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
                if *kind != RangeKind::Constant { self.bail("RangeSelect_nonconst"); return None; }
                let base = self.compile_expr(expr, 0)?;
                let l = self.compile_expr(left, 0)?;
                let r = self.compile_expr(right, 0)?;
                let dest = self.alloc_reg();
                self.emit(Insn::RangeSelect(dest, base, l, r));
                Some(dest)
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
                self.emit(Insn::Concat(dest, regs));
                Some(dest)
            }
            ExprKind::Concatenation(parts) => {
                let mut regs = Vec::new();
                for p in parts {
                    let r = self.compile_expr(p, 0)?;
                    regs.push(r);
                }
                let dest = self.alloc_reg();
                self.emit(Insn::Concat(dest, regs));
                Some(dest)
            }
            ExprKind::SystemCall { name, args } => {
                match name.as_str() {
                    "$signed" => {
                        let r = self.compile_expr(args.first()?, 0)?;
                        // Mark as signed - we'll handle this in the VM
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
                            self.emit(Insn::NbaAssignArray(name, idx_reg, val_reg, width));
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
                        if let (Some(hi), Some(lo)) = (self.eval_const_expr(left), self.eval_const_expr(right)) {
                            self.emit(Insn::NbaAssignRange(id, hi, lo, val_reg));
                            let _ = width;
                            return true;
                        }
                    }
                }
                self.bail("nba_range_unresolved");
                false
            }
            ExprKind::Concatenation(_) => { self.bail("nba_concat"); false }
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
                } else { false }
            }
            _ => false,
        }
    }

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
                    self.widths.get(&raw).copied().unwrap_or(32)
                } else { 32 }
            }
            _ => 32,
        }
    }

    fn eval_const_expr(&self, e: &Expression) -> Option<u32> {
        match &e.kind {
            ExprKind::Number(n) => self.eval_number_static(n)?.to_u64().map(|v| v as u32),
            ExprKind::Paren(inner) => self.eval_const_expr(inner),
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

    pub fn finish(self) -> CompiledBlock {
        CompiledBlock {
            num_regs: self.next_reg,
            instructions: self.insns,
        }
    }
}
