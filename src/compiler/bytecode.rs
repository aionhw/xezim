//! Bytecode VM for high-performance simulation execution.
//! Compiles AST expressions and statements into a flat instruction array
//! that can be executed without pointer-chasing through Box<Expression> trees.

use super::value::Value;
use crate::ast::expr::*;
use crate::ast::stmt::*;
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
    /// Blocking assign: signal_table[id] = reg.
    BlockingAssign(usize, RegId, u32), // (signal_id, value_reg, width)

    /// Load array element: dest = signal_table[array_base + eval(index_reg)]
    LoadArrayElem(RegId, String, RegId), // (dest, array_name, index_reg)
    /// NBA assign to array element.
    NbaAssignArray(String, RegId, RegId, u32), // (array_name, index_reg, value_reg, width)

    /// Marks end of a compiled block (no-op, helps debugging).
    /// Copy src register to dest register.
    Move(RegId, RegId),       // (dest, src)

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

    /// Compile a statement. Returns true if successfully compiled.
    pub fn compile_stmt(&mut self, stmt: &Statement) -> bool {
        match &stmt.kind {
            StatementKind::Null => true,
            StatementKind::NonblockingAssign { lvalue, rvalue, .. } => {
                let width = self.infer_lhs_width(lvalue);
                if let Some(val_reg) = self.compile_expr(rvalue, width) {
                    if width > 0 {
                        self.emit(Insn::Resize(val_reg, width));
                    }
                    self.compile_nba_target(lvalue, val_reg, width)
                } else {
                    false
                }
            }
            StatementKind::BlockingAssign { lvalue, rvalue } => {
                let width = self.infer_lhs_width(lvalue);
                if let Some(val_reg) = self.compile_expr(rvalue, width) {
                    if width > 0 {
                        self.emit(Insn::Resize(val_reg, width));
                    }
                    self.compile_blocking_target(lvalue, val_reg, width)
                } else {
                    false
                }
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
            _ => false,
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
                let name = hier.path.last().map(|s| s.name.name.as_str())?;
                let &id = self.signal_name_to_id.get(name)?;
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
                    _ => return None, // bail on incr/decr etc.
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
                    _ => return None,
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
                    let name = hier.path.last().map(|s| s.name.name.as_str())?;
                    if self.arrays.contains_key(name) {
                        let idx_reg = self.compile_expr(index, 0)?;
                        let dest = self.alloc_reg();
                        self.emit(Insn::LoadArrayElem(dest, name.to_string(), idx_reg));
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
                if *kind != RangeKind::Constant { return None; }
                let base = self.compile_expr(expr, 0)?;
                let l = self.compile_expr(left, 0)?;
                let r = self.compile_expr(right, 0)?;
                let dest = self.alloc_reg();
                self.emit(Insn::RangeSelect(dest, base, l, r));
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
                    _ => None, // bail on other system calls
                }
            }
            _ => None,
        }
    }

    fn compile_nba_target(&mut self, lhs: &Expression, val_reg: RegId, width: u32) -> bool {
        match &lhs.kind {
            ExprKind::Ident(hier) => {
                let name = hier.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
                if let Some(&id) = self.signal_name_to_id.get(name) {
                    self.emit(Insn::NbaAssign(id, val_reg, width));
                    true
                } else { false }
            }
            ExprKind::Index { expr, index } => {
                if let ExprKind::Ident(hier) = &expr.kind {
                    let name = hier.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
                    if self.arrays.contains_key(name) {
                        if let Some(idx_reg) = self.compile_expr(index, 0) {
                            self.emit(Insn::NbaAssignArray(name.to_string(), idx_reg, val_reg, width));
                            return true;
                        }
                    }
                }
                false
            }
            // TODO: bit-select NBA, range-select NBA
            _ => false,
        }
    }

    fn compile_blocking_target(&mut self, lhs: &Expression, val_reg: RegId, width: u32) -> bool {
        match &lhs.kind {
            ExprKind::Ident(hier) => {
                let name = hier.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
                if let Some(&id) = self.signal_name_to_id.get(name) {
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
                let name = hier.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
                if let Some(&id) = self.signal_name_to_id.get(name) {
                    self.signal_widths[id]
                } else {
                    self.widths.get(name).copied().unwrap_or(32)
                }
            }
            ExprKind::Index { expr, .. } => {
                if let ExprKind::Ident(hier) = &expr.kind {
                    let name = hier.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
                    if let Some((_, _, elem_w)) = self.arrays.get(name) {
                        return *elem_w;
                    }
                    self.widths.get(name).copied().unwrap_or(32)
                } else { 32 }
            }
            _ => 32,
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
