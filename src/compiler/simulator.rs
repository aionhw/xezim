//! Event-driven simulator for SystemVerilog combinatorial and sequential logic.
//!
//! Implements a simplified IEEE 1800 scheduling model:
//!   Active region:  blocking assigns, continuous assigns, always_comb
//!   NBA region:     non-blocking assign updates
//!   Reactive:       edge-triggered always_ff/always_latch blocks

use std::collections::{HashMap, HashSet, BTreeMap};
use std::io::Write;
use crate::ast::expr::*;
use crate::ast::stmt::*;
use crate::ast::decl::AlwaysKind;
use super::value::{Value, LogicBit};
use super::elaborate::{ElaboratedModule, AlwaysBlock};

#[derive(Debug, Clone)]
pub struct SimOutput { pub time: u64, pub message: String }

#[derive(Debug, Clone)]
struct NbaEntry { lhs: Expression, value: Value, resolved_target: Option<String> }

#[derive(Debug, Clone)]
struct EdgeSensitiveBlock { sensitivities: Vec<Sensitivity>, stmt: Statement, kind: AlwaysKind }

#[derive(Debug, Clone)]
struct Sensitivity { signal_name: String, edge: EdgeKind }

#[derive(Debug, Clone, Copy, PartialEq)]
enum EdgeKind { Posedge, Negedge, AnyEdge }

/// A process waiting for a signal edge event.
#[derive(Debug, Clone)]
struct EventWaiter {
    pid: usize,
    sensitivities: Vec<Sensitivity>,
    continuation: Vec<Statement>,
    registered_time: u64,
}

/// Pad a string to a given width with spaces (or zeros if zero_pad).
fn pad_string(s: &str, width: usize, zero_pad: bool) -> String {
    if width == 0 || s.len() >= width { return s.to_string(); }
    let pad = width - s.len();
    if zero_pad { format!("{}{}", "0".repeat(pad), s) }
    else { format!("{}{}", " ".repeat(pad), s) }
}

pub struct Simulator {
    pub signals: HashMap<String, Value>,
    pub widths: HashMap<String, u32>,
    pub signed_signals: HashSet<String>,
    prev_signals: HashMap<String, Value>,
    pub time: u64,
    pub output: Vec<SimOutput>,
    pub finished: bool,
    pub monitor: Option<(String, Vec<Expression>)>,
    pub monitor_prev: HashMap<String, Value>,
    pub max_time: u64,
    module: ElaboratedModule,
    settling: bool,
    in_edge_block: bool,
    nba_queue: Vec<NbaEntry>,
    edge_blocks: Vec<EdgeSensitiveBlock>,
    event_queue: BTreeMap<u64, Vec<(usize, Vec<Statement>)>>,
    next_pid: usize,
    break_flag: bool,
    continue_flag: bool,
    /// Processes waiting for signal edge events (@(posedge clk), etc.)
    event_waiters: Vec<EventWaiter>,
    /// VCD dump state
    vcd_file: Option<String>,
    vcd_writer: Option<std::io::BufWriter<std::fs::File>>,
    vcd_id_map: HashMap<String, String>,
    vcd_enabled: bool,
    vcd_last_time: u64,
    vcd_prev_signals: HashMap<String, Value>,
}

impl Simulator {
    pub fn new(module: ElaboratedModule, max_time: u64) -> Self {
        let mut signals = HashMap::new();
        let mut widths = HashMap::new();
        let mut signed_signals = HashSet::new();
        for (name, sig) in &module.signals {
            let mut val = sig.value.clone();
            if sig.is_signed { val.is_signed = true; signed_signals.insert(name.clone()); }
            signals.insert(name.clone(), val);
            widths.insert(name.clone(), sig.width);
        }
        for (name, val) in &module.parameters {
            if val.is_signed { signed_signals.insert(name.clone()); }
            signals.insert(name.clone(), val.clone());
            widths.insert(name.clone(), val.width);
        }
        Self {
            prev_signals: signals.clone(), signals, widths, signed_signals,
            time: 0, output: Vec::new(), finished: false,
            monitor: None, monitor_prev: HashMap::new(),
            max_time, module, settling: false, in_edge_block: false,
            nba_queue: Vec::new(), edge_blocks: Vec::new(),
            event_queue: BTreeMap::new(), next_pid: 0,
            break_flag: false, continue_flag: false,
            event_waiters: Vec::new(),
            vcd_file: None,
            vcd_writer: None,
            vcd_id_map: HashMap::new(),
            vcd_enabled: false,
            vcd_last_time: u64::MAX,
            vcd_prev_signals: HashMap::new(),
        }
    }

    pub fn run(&mut self) {
        self.classify_always_blocks();
        self.settle_combinatorial();
        let initial_blocks = self.module.initial_blocks.clone();
        for ib in &initial_blocks {
            let stmts = match &ib.stmt.kind {
                StatementKind::SeqBlock { stmts, .. } => stmts.clone(),
                _ => vec![ib.stmt.clone()],
            };
            let pid = self.next_pid; self.next_pid += 1;
            self.event_queue.entry(0).or_default().push((pid, stmts));
        }
        self.event_loop();
        self.vcd_finish();
    }

    fn classify_always_blocks(&mut self) {
        let blocks = self.module.always_blocks.clone();
        let mut remaining = Vec::new();
        for (idx, ab) in blocks.iter().enumerate() {
            // Check for edge-sensitive: always_ff @(posedge ...) or always @(posedge ...)
            if let Some((sens, body)) = self.extract_sensitivity(&ab.stmt) {
                if !sens.is_empty() {
                    self.edge_blocks.push(EdgeSensitiveBlock { sensitivities: sens, stmt: body, kind: ab.kind });
                    continue;
                }
                // @(*) or @* — treat as combinatorial (strip the event control)
                remaining.push(AlwaysBlock { kind: AlwaysKind::AlwaysComb, stmt: body });
                continue;
            }
            // Check for always #delay body — schedule as repeating process
            if ab.kind == AlwaysKind::Always {
                if let StatementKind::TimingControl { control: TimingControl::Delay(_), .. } = &ab.stmt.kind {
                    let forever_stmt = Statement::new(
                        StatementKind::Forever { body: Box::new(ab.stmt.clone()) }, ab.stmt.span,
                    );
                    let pid = self.next_pid; self.next_pid += 1;
                    self.event_queue.entry(0).or_default().push((pid, vec![forever_stmt]));
                    continue;
                }
                // Check for always blocks with internal blocking (delays, events, waits)
                // These must be scheduled as processes, not treated as combinatorial
                if self.stmt_is_blocking(&ab.stmt) {
                    let forever_stmt = Statement::new(
                        StatementKind::Forever { body: Box::new(ab.stmt.clone()) }, ab.stmt.span,
                    );
                    let pid = self.next_pid; self.next_pid += 1;
                    self.event_queue.entry(0).or_default().push((pid, vec![forever_stmt]));
                    continue;
                }
            }
            remaining.push(ab.clone());
        }
        self.module.always_blocks = remaining;
    }

    fn extract_sensitivity(&self, stmt: &Statement) -> Option<(Vec<Sensitivity>, Statement)> {
        match &stmt.kind {
            StatementKind::TimingControl { control, stmt: body } => {
                if let TimingControl::Event(event) = control {
                    return Some((self.event_to_sens(event), *body.clone()));
                }
                None
            }
            StatementKind::SeqBlock { stmts, name } => {
                if let Some(first) = stmts.first() {
                    if let StatementKind::TimingControl { control, stmt: body } = &first.kind {
                        if let TimingControl::Event(event) = control {
                            let sens = self.event_to_sens(event);
                            let mut new_stmts = vec![*body.clone()];
                            new_stmts.extend_from_slice(&stmts[1..]);
                            return Some((sens, Statement::new(
                                StatementKind::SeqBlock { name: name.clone(), stmts: new_stmts }, stmt.span)));
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn event_to_sens(&self, event: &EventControl) -> Vec<Sensitivity> {
        match event {
            EventControl::EventExpr(exprs) => exprs.iter().filter_map(|ee| {
                let sig = match &ee.expr.kind { ExprKind::Ident(h) => Some(self.resolve_hier_name(h)), _ => None }?;
                let edge = match ee.edge {
                    Some(Edge::Posedge) => EdgeKind::Posedge,
                    Some(Edge::Negedge) => EdgeKind::Negedge,
                    _ => EdgeKind::AnyEdge,
                };
                Some(Sensitivity { signal_name: sig, edge })
            }).collect(),
            EventControl::Identifier(id) => vec![Sensitivity { signal_name: id.name.clone(), edge: EdgeKind::AnyEdge }],
            _ => Vec::new(),
        }
    }

    fn event_loop(&mut self) {
        let mut iters: u64 = 0;
        let max_iters = self.max_time * 1000;
        while !self.finished && iters < max_iters {
            iters += 1;

            // Check for events to process
            let has_timed = !self.event_queue.is_empty();
            let has_waiters = !self.event_waiters.is_empty();

            if !has_timed && !has_waiters { break; }

            if has_timed {
                let next_time = self.event_queue.keys().next().copied().unwrap();
                if next_time > self.max_time { break; }
                if next_time > self.time { self.time = next_time; }

                // Snapshot prev_signals BEFORE executing processes.
                self.prev_signals = self.signals.clone();

                let processes = self.event_queue.remove(&self.time).unwrap_or_default();
                for (pid, stmts) in processes {
                    if self.finished { break; }
                    self.run_process_stmts(pid, &stmts);
                }

                self.apply_nba();
                self.settle_combinatorial();
                self.check_edges(); self.apply_nba(); self.settle_combinatorial();
                self.prev_signals = self.signals.clone();
                self.check_monitor();
                self.vcd_write_changes();
            } else {
                // Only waiters, no timed events — deadlock (no clock driver)
                break;
            }
        }
    }

    fn run_process_stmts(&mut self, pid: usize, stmts: &[Statement]) {
        let mut i = 0;
        while i < stmts.len() && !self.finished {
            let stmt = &stmts[i];

            // Expand SeqBlocks: flatten begin/end so that timing controls and waits
            // inside them are properly handled with process suspension.
            if let StatementKind::SeqBlock { stmts: inner, .. } = &stmt.kind {
                if self.stmts_have_blocking(inner) {
                    let mut expanded = inner.clone();
                    expanded.extend_from_slice(&stmts[i+1..]);
                    self.run_process_stmts(pid, &expanded);
                    return;
                }
            }

            // Check for timing control — delay or event
            if let StatementKind::TimingControl { control, stmt: body } = &stmt.kind {
                match control {
                    TimingControl::Delay(d) => {
                        let delay = self.eval_expr(d).to_u64().unwrap_or(0);
                        let mut cont = vec![*body.clone()];
                        cont.extend_from_slice(&stmts[i+1..]);
                        self.event_queue.entry(self.time + delay).or_default().push((pid, cont));
                        return;
                    }
                    TimingControl::Event(event) => {
                        // Suspend process until the event fires
                        let sens = self.event_to_sens(event);
                        if !sens.is_empty() {
                            let mut cont = vec![*body.clone()];
                            cont.extend_from_slice(&stmts[i+1..]);
                            self.event_waiters.push(EventWaiter {
                                pid, sensitivities: sens, continuation: cont,
                                registered_time: self.time,
                            });
                            return;
                        }
                        // Star/empty sensitivity — just execute body
                    }
                }
                self.exec_statement(body);
                i += 1;
                continue;
            }

            // Check for wait statement — blocks until condition is true
            if let StatementKind::Wait { condition, stmt: body } = &stmt.kind {
                if self.eval_expr(condition).is_true() {
                    self.exec_statement(body);
                    i += 1;
                    continue;
                } else {
                    let sig_names = self.extract_signal_names(condition);
                    let sens: Vec<Sensitivity> = sig_names.into_iter().map(|name| {
                        Sensitivity { signal_name: name, edge: EdgeKind::AnyEdge }
                    }).collect();
                    if !sens.is_empty() {
                        let mut cont = vec![stmt.clone()];
                        cont.extend_from_slice(&stmts[i+1..]);
                        self.event_waiters.push(EventWaiter {
                            pid, sensitivities: sens, continuation: cont,
                            registered_time: self.time,
                        });
                        return;
                    }
                    i += 1;
                    continue;
                }
            }

            // Check for forever with delays/events
            if let StatementKind::Forever { body } = &stmt.kind {
                self.exec_forever_sched(pid, body, &stmts[i+1..]);
                return;
            }

            // Check for repeat with event waits inside
            if let StatementKind::Repeat { count, body } = &stmt.kind {
                let n = self.eval_expr(count).to_u64().unwrap_or(0);
                if n > 0 && self.stmt_has_event_wait(body) {
                    // Unroll: execute body once, then schedule rest
                    let remaining_n = n - 1;
                    let mut cont = Vec::new();
                    // Expand body (may contain @event)
                    let body_stmts = match &body.kind {
                        StatementKind::SeqBlock { stmts, .. } => stmts.clone(),
                        _ => vec![*body.clone()],
                    };
                    cont.extend(body_stmts);
                    // Re-schedule remaining repeats
                    if remaining_n > 0 {
                        cont.push(Statement::new(
                            StatementKind::Repeat {
                                count: Expression::new(
                                    ExprKind::Number(NumberLiteral::Integer {
                                        size: None, signed: false,
                                        base: NumberBase::Decimal,
                                        value: remaining_n.to_string(),
                                    }),
                                    body.span,
                                ),
                                body: body.clone(),
                            },
                            stmt.span,
                        ));
                    }
                    cont.extend_from_slice(&stmts[i+1..]);
                    self.run_process_stmts(pid, &cont);
                    return;
                }
            }

            self.exec_statement(stmt);
            i += 1;
        }
    }

    /// Check if a statement contains @(event) waits.
    fn stmt_has_event_wait(&self, stmt: &Statement) -> bool {
        match &stmt.kind {
            StatementKind::TimingControl { control: TimingControl::Event(_), .. } => true,
            StatementKind::TimingControl { control: TimingControl::Delay(_), .. } => true,
            StatementKind::SeqBlock { stmts, .. } => stmts.iter().any(|s| self.stmt_has_event_wait(s)),
            _ => false,
        }
    }

    /// Check if any statements contain blocking constructs (timing, events, wait).
    fn stmts_have_blocking(&self, stmts: &[Statement]) -> bool {
        stmts.iter().any(|s| self.stmt_is_blocking(s))
    }
    fn stmt_is_blocking(&self, stmt: &Statement) -> bool {
        match &stmt.kind {
            StatementKind::TimingControl { .. } => true,
            StatementKind::Wait { .. } => true,
            StatementKind::SeqBlock { stmts, .. } => stmts.iter().any(|s| self.stmt_is_blocking(s)),
            StatementKind::If { then_stmt, else_stmt, .. } => {
                self.stmt_is_blocking(then_stmt) || else_stmt.as_ref().map_or(false, |e| self.stmt_is_blocking(e))
            }
            StatementKind::Forever { body } => self.stmt_is_blocking(body),
            StatementKind::For { body, .. } | StatementKind::While { body, .. } => self.stmt_is_blocking(body),
            _ => false,
        }
    }

    fn exec_forever_sched(&mut self, pid: usize, body: &Statement, after: &[Statement]) {
        let body_stmts = match &body.kind {
            StatementKind::SeqBlock { stmts, .. } => stmts.clone(),
            _ => vec![body.clone()],
        };
        for (i, s) in body_stmts.iter().enumerate() {
            if self.finished { return; }
            if let StatementKind::TimingControl { control, stmt: tbody } = &s.kind {
                match control {
                    TimingControl::Delay(d) => {
                        let delay = self.eval_expr(d).to_u64().unwrap_or(0);
                        let mut cont = vec![*tbody.clone()];
                        cont.extend_from_slice(&body_stmts[i+1..]);
                        cont.push(Statement::new(StatementKind::Forever { body: Box::new(body.clone()) }, body.span));
                        cont.extend_from_slice(after);
                        self.event_queue.entry(self.time + delay).or_default().push((pid, cont));
                        return;
                    }
                    TimingControl::Event(event) => {
                        let sens = self.event_to_sens(event);
                        if !sens.is_empty() {
                            let mut cont = vec![*tbody.clone()];
                            cont.extend_from_slice(&body_stmts[i+1..]);
                            cont.push(Statement::new(StatementKind::Forever { body: Box::new(body.clone()) }, body.span));
                            cont.extend_from_slice(after);
                            self.event_waiters.push(EventWaiter {
                                pid, sensitivities: sens, continuation: cont,
                                registered_time: self.time,
                            });
                            return;
                        }
                    }
                }
            }
            self.exec_statement(s);
        }
        // No delay/event in forever body — safety limit
        let mut safety = 0;
        while !self.finished && safety < 10000 { safety += 1; for s in &body_stmts { self.exec_statement(s); } }
    }

    /// Resolve NBA target at schedule time to capture array indices/part-selects
    fn resolve_nba_target(&self, lhs: &Expression) -> Option<String> {
        match &lhs.kind {
            ExprKind::Ident(hier) => {
                Some(self.resolve_hier_name(hier))
            }
            ExprKind::Index { expr, index } => {
                if let ExprKind::Ident(hier) = &expr.kind {
                    let name = self.resolve_hier_name(hier);
                    if self.module.arrays.contains_key(&name) {
                        let idx = self.eval_expr(index).to_u64().unwrap_or(0);
                        return Some(format!("{}[{}]", name, idx));
                    }
                }
                None // bit select - let assign_value handle it
            }
            _ => None, // range select, concatenation - let assign_value handle it
        }
    }

    fn apply_nba(&mut self) {
        let nba = std::mem::take(&mut self.nba_queue);
        for e in nba {
            if let Some(ref target) = e.resolved_target {
                // Pre-resolved target (array element or part-select)
                if let Some(sig) = self.signals.get_mut(target) {
                    let resized = e.value.resize(sig.width);
                    sig.bits = resized.bits;
                } else {
                    // May be a simple ident
                    self.assign_value(&e.lhs, &e.value);
                }
            } else {
                self.assign_value(&e.lhs, &e.value);
            }
        }
    }

    fn check_edges(&mut self) {
        let blocks = self.edge_blocks.clone();
        self.in_edge_block = true;
        for block in &blocks {
            let mut trigger = false;
            for sens in &block.sensitivities {
                if let (Some(cur), Some(prev)) = (self.signals.get(&sens.signal_name), self.prev_signals.get(&sens.signal_name)) {
                    trigger = match sens.edge {
                        EdgeKind::Posedge => {
                            let cb = cur.bits.first().copied().unwrap_or(LogicBit::X);
                            let pb = prev.bits.first().copied().unwrap_or(LogicBit::X);
                            pb != LogicBit::One && cb == LogicBit::One
                        }
                        EdgeKind::Negedge => {
                            let cb = cur.bits.first().copied().unwrap_or(LogicBit::X);
                            let pb = prev.bits.first().copied().unwrap_or(LogicBit::X);
                            pb != LogicBit::Zero && cb == LogicBit::Zero
                        }
                        EdgeKind::AnyEdge => cur.bits != prev.bits,
                    };
                }
                if trigger { break; }
            }
            if trigger { self.exec_statement(&block.stmt); }
        }

        // Wake up event_waiters whose sensitivity conditions are met
        let waiters = std::mem::take(&mut self.event_waiters);
        let mut still_waiting = Vec::new();
        for waiter in waiters {
            // Don't trigger waiters registered during this same time step
            if waiter.registered_time == self.time {
                still_waiting.push(waiter);
                continue;
            }
            let mut triggered = false;
            for sens in &waiter.sensitivities {
                if let (Some(cur), Some(prev)) = (self.signals.get(&sens.signal_name), self.prev_signals.get(&sens.signal_name)) {
                    triggered = match sens.edge {
                        EdgeKind::Posedge => {
                            let cb = cur.bits.first().copied().unwrap_or(LogicBit::X);
                            let pb = prev.bits.first().copied().unwrap_or(LogicBit::X);
                            pb != LogicBit::One && cb == LogicBit::One
                        }
                        EdgeKind::Negedge => {
                            let cb = cur.bits.first().copied().unwrap_or(LogicBit::X);
                            let pb = prev.bits.first().copied().unwrap_or(LogicBit::X);
                            pb != LogicBit::Zero && cb == LogicBit::Zero
                        }
                        EdgeKind::AnyEdge => cur.bits != prev.bits,
                    };
                }
                if triggered { break; }
            }
            if triggered {
                self.event_queue.entry(self.time).or_default().push((waiter.pid, waiter.continuation));
            } else {
                still_waiting.push(waiter);
            }
        }
        self.event_waiters = still_waiting;
    }

    fn settle_combinatorial(&mut self) {
        if self.settling { return; }
        self.settling = true;
        for _ in 0..100 {
            let mut changed = false;
            let assigns: Vec<_> = self.module.continuous_assigns.clone();
            for ca in &assigns {
                let w = self.infer_lhs_width(&ca.lhs);
                let val = self.eval_expr_ctx(&ca.rhs, w).resize(w);
                if self.assign_value(&ca.lhs, &val) { changed = true; }
            }
            let blocks: Vec<_> = self.module.always_blocks.clone();
            for ab in &blocks {
                if matches!(ab.kind, AlwaysKind::AlwaysComb | AlwaysKind::Always) {
                    let prev = self.signals.clone();
                    self.exec_statement(&ab.stmt);
                    if self.signals != prev { changed = true; }
                }
            }
            if !changed { break; }
        }
        self.settling = false;
    }

    fn assign_value(&mut self, lhs: &Expression, val: &Value) -> bool {
        match &lhs.kind {
            ExprKind::Ident(hier) => {
                let name = self.resolve_hier_name(hier);
                let width = self.widths.get(&name).copied().unwrap_or(val.width);
                let mut resized = val.resize(width);
                // Preserve signal's declared signedness
                resized.is_signed = self.signed_signals.contains(&name);
                let changed = self.signals.get(&name).map_or(true, |p| *p != resized);
                self.signals.insert(name, resized); changed
            }
            ExprKind::Index { expr, index } => {
                if let ExprKind::Ident(hier) = &expr.kind {
                    let name = self.resolve_hier_name(hier);
                    let idx = self.eval_expr(index).to_u64().unwrap_or(0);
                    // Check if this is an array element assignment
                    if self.module.arrays.contains_key(&name) {
                        let elem_name = format!("{}[{}]", name, idx);
                        if let Some(sig) = self.signals.get_mut(&elem_name) {
                            let resized = val.resize(sig.width);
                            let changed = sig.bits != resized.bits;
                            sig.bits = resized.bits;
                            return changed;
                        }
                        return false;
                    }
                    // Fall back to bit select assignment
                    let idx = idx as usize;
                    if let Some(sig) = self.signals.get_mut(&name) {
                        if idx < sig.bits.len() {
                            let nb = val.bits.first().copied().unwrap_or(LogicBit::X);
                            let c = sig.bits[idx] != nb; sig.bits[idx] = nb; return c;
                        }
                    }
                }
                false
            }
            ExprKind::RangeSelect { expr, left, right, .. } => {
                let msb = self.eval_expr(left).to_u64().unwrap_or(0) as usize;
                let lsb = self.eval_expr(right).to_u64().unwrap_or(0) as usize;
                // Resolve the target signal name (handles both ident and array index)
                let target_name = match &expr.kind {
                    ExprKind::Ident(hier) => Some(self.resolve_hier_name(hier)),
                    ExprKind::Index { expr: arr_expr, index } => {
                        if let ExprKind::Ident(hier) = &arr_expr.kind {
                            let name = self.resolve_hier_name(hier);
                            if self.module.arrays.contains_key(&name) {
                                let idx = self.eval_expr(index).to_u64().unwrap_or(0);
                                Some(format!("{}[{}]", name, idx))
                            } else { None }
                        } else { None }
                    }
                    _ => None,
                };
                if let Some(name) = target_name {
                    if let Some(sig) = self.signals.get_mut(&name) {
                        let mut changed = false;
                        for i in lsb..=msb.min(sig.bits.len().saturating_sub(1)) {
                            let nb = val.bits.get(i - lsb).copied().unwrap_or(LogicBit::Zero);
                            if sig.bits[i] != nb { sig.bits[i] = nb; changed = true; }
                        }
                        return changed;
                    }
                }
                false
            }
            ExprKind::Concatenation(parts) => {
                let tw: u32 = parts.iter().map(|p| self.infer_width(p)).sum();
                let rv = val.resize(tw);
                let mut off = 0usize; let mut changed = false;
                for part in parts.iter().rev() {
                    let pw = self.infer_width(part);
                    let pv = rv.range_select(off + pw as usize - 1, off);
                    if self.assign_value(part, &pv) { changed = true; }
                    off += pw as usize;
                }
                changed
            }
            _ => false,
        }
    }

    pub fn eval_expr(&self, expr: &Expression) -> Value {
        self.eval_expr_ctx(expr, 0)
    }

    /// Evaluate expression with a context width hint (for proper shift sizing).
    /// When ctx_width > 0, shift operators widen their left operand to ctx_width.
    pub fn eval_expr_ctx(&self, expr: &Expression, ctx_width: u32) -> Value {
        match &expr.kind {
            ExprKind::Number(num) => self.eval_number(num),
            ExprKind::StringLiteral(s) => {
                let w = (s.len() * 8) as u32;
                let mut val = Value::zero(w.max(8));
                for (i, byte) in s.bytes().rev().enumerate() {
                    for bit in 0..8 { if (byte >> bit) & 1 == 1 { if let Some(b) = val.bits.get_mut(i*8+bit) { *b = LogicBit::One; } } }
                }
                val
            }
            ExprKind::Ident(hier) => { let name = self.resolve_hier_name(hier); let mut v = self.signals.get(&name).cloned().unwrap_or_else(|| Value::new(1)); if self.signed_signals.contains(&name) { v.is_signed = true; } v }
            ExprKind::Unary { op, operand } => {
                let v = self.eval_expr(operand);
                match op {
                    UnaryOp::Plus => v, UnaryOp::Minus => { let mut r = Value::zero(v.width).sub(&v).resize(v.width); r.is_signed = true; r },
                    UnaryOp::LogNot => v.logic_not(), UnaryOp::BitNot => v.bitwise_not(),
                    UnaryOp::BitAnd => v.reduce_and(), UnaryOp::BitOr => v.reduce_or(), UnaryOp::BitXor => v.reduce_xor(),
                    UnaryOp::BitNand => v.reduce_and().logic_not(), UnaryOp::BitNor => v.reduce_or().logic_not(), UnaryOp::BitXnor => v.reduce_xor().logic_not(),
                    UnaryOp::PreIncr | UnaryOp::PostIncr => v.add(&Value::from_u64(1, v.width)),
                    UnaryOp::PreDecr | UnaryOp::PostDecr => v.sub(&Value::from_u64(1, v.width)),
                }
            }
            ExprKind::Binary { op, left, right } => {
                // For context-dependent binary ops, propagate context width
                let ctx_w = if ctx_width > 0 { ctx_width } else { 0 };
                let l = self.eval_expr_ctx(left, ctx_w);
                let r = self.eval_expr_ctx(right, ctx_w);
                // For non-shift arithmetic, widen operands to max context width
                let wl = if ctx_w > 0 && matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul
                    | BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor | BinaryOp::BitXnor) {
                    l.resize(l.width.max(r.width).max(ctx_w))
                } else { l };
                let wr = if ctx_w > 0 && matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul
                    | BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor | BinaryOp::BitXnor) {
                    r.resize(wl.width)
                } else { r };
                match op {
                    BinaryOp::Add => wl.add(&wr), BinaryOp::Sub => wl.sub(&wr), BinaryOp::Mul => wl.mul(&wr), BinaryOp::Div => wl.div(&wr),
                    BinaryOp::Mod => wl.modulo(&wr), BinaryOp::Power => wl.power(&wr),
                    BinaryOp::BitAnd => wl.bitwise_and(&wr), BinaryOp::BitOr => wl.bitwise_or(&wr),
                    BinaryOp::BitXor => wl.bitwise_xor(&wr), BinaryOp::BitXnor => wl.bitwise_xor(&wr).bitwise_not(),
                    BinaryOp::LogAnd => wl.logic_and(&wr), BinaryOp::LogOr => wl.logic_or(&wr),
                    BinaryOp::Eq => wl.eq(&wr), BinaryOp::Neq => wl.neq(&wr),
                    BinaryOp::CaseEq => wl.case_eq(&wr), BinaryOp::CaseNeq => wl.case_eq(&wr).logic_not(),
                    BinaryOp::Lt => wl.lt(&wr), BinaryOp::Leq => wl.leq(&wr), BinaryOp::Gt => wl.gt(&wr), BinaryOp::Geq => wl.geq(&wr),
                    BinaryOp::ShiftLeft | BinaryOp::ArithShiftLeft => {
                        // Widen left operand to context width before shifting
                        let wide_l = if ctx_w > wl.width { wl.resize(ctx_w) } else { wl };
                        wide_l.shift_left(&wr)
                    }
                    BinaryOp::ShiftRight => wl.shift_right(&wr), BinaryOp::ArithShiftRight => wl.arith_shift_right(&wr),
                    _ => Value::new(wl.width.max(wr.width)),
                }
            }
            ExprKind::Conditional { condition, then_expr, else_expr } => {
                let c = self.eval_expr(condition);
                if c.has_unknown() { let t = self.eval_expr_ctx(then_expr, ctx_width); let e = self.eval_expr_ctx(else_expr, ctx_width); if t == e { t } else { Value::new(t.width.max(e.width)) } }
                else if c.is_true() { self.eval_expr_ctx(then_expr, ctx_width) } else { self.eval_expr_ctx(else_expr, ctx_width) }
            }
            ExprKind::Concatenation(parts) => { let mut r = Value::zero(0); for p in parts.iter().rev() { r = self.eval_expr(p).concat(&r); } r }
            ExprKind::Replication { count, exprs } => {
                let n = self.eval_expr(count).to_u64().unwrap_or(1);
                let mut inner = Value::zero(0); for e in exprs.iter().rev() { inner = self.eval_expr(e).concat(&inner); }
                let mut r = Value::zero(0); for _ in 0..n { r = inner.concat(&r); } r
            }
            ExprKind::Index { expr, index } => {
                // Check if this is an array element access (memory[idx]) vs bit select
                if let ExprKind::Ident(hier) = &expr.kind {
                    let name = self.resolve_hier_name(hier);
                    if self.module.arrays.contains_key(&name) {
                        // Array element access: look up signal "name[idx]"
                        let idx = self.eval_expr(index).to_u64().unwrap_or(0);
                        let elem_name = format!("{}[{}]", name, idx);
                        let mut v = self.signals.get(&elem_name).cloned().unwrap_or_else(|| Value::new(1));
                        if self.signed_signals.contains(&elem_name) { v.is_signed = true; }
                        return v;
                    }
                }
                // Fall back to bit select
                self.eval_expr(expr).bit_select(self.eval_expr(index).to_u64().unwrap_or(0) as usize)
            }
            ExprKind::RangeSelect { expr, left, right, kind, .. } => {
                let base = self.eval_expr(expr); let l = self.eval_expr(left).to_u64().unwrap_or(0) as usize; let r = self.eval_expr(right).to_u64().unwrap_or(0) as usize;
                let result = match kind { RangeKind::Constant => base.range_select(l, r), RangeKind::IndexedUp => base.range_select(l+r-1, l), RangeKind::IndexedDown => base.range_select(l, l.saturating_sub(r-1)) };
                result
            }
            ExprKind::Paren(inner) => self.eval_expr_ctx(inner, ctx_width),
            ExprKind::SystemCall { name, args } => match name.as_str() {
                "$clog2" => { let v = args.first().map(|a| self.eval_expr(a).to_u64().unwrap_or(0)).unwrap_or(0); Value::from_u64(if v <= 1 { 1 } else { 64 - (v-1).leading_zeros() } as u64, 32) }
                "$bits" => args.first().map(|a| Value::from_u64(self.eval_expr(a).width as u64, 32)).unwrap_or(Value::zero(32)),
                "$signed" => { let mut v = args.first().map(|a| self.eval_expr(a)).unwrap_or(Value::zero(32)); v.is_signed = true; v }
                "$unsigned" => { let mut v = args.first().map(|a| self.eval_expr(a)).unwrap_or(Value::zero(32)); v.is_signed = false; v }
                "$time" => Value::from_u64(self.time, 64),
                "$test$plusargs" => Value::from_u64(0, 1), // no plusargs in simulation
                "$value$plusargs" => Value::from_u64(0, 1),
                "$random" => Value::from_u64(0, 32), // stub
                _ => Value::zero(32),
            },
            ExprKind::Dollar => Value::from_u64(u64::MAX, 32),
            ExprKind::Null | ExprKind::Empty => Value::zero(1),
            ExprKind::AssignmentPattern(parts) => { let mut r = Value::zero(0); for p in parts.iter().rev() { r = self.eval_expr(p).concat(&r); } r }
            _ => Value::zero(32),
        }
    }

    fn eval_number(&self, num: &NumberLiteral) -> Value {
        match num {
            NumberLiteral::Integer { size, signed, base, value } => {
                let w = size.unwrap_or(32);
                let r = match base { NumberBase::Binary => 2, NumberBase::Octal => 8, NumberBase::Hex => 16, NumberBase::Decimal => 10 };
                let mut v = Value::from_str_radix(value, r, w); v.is_signed = *signed; v
            }
            NumberLiteral::Real(f) => Value::from_u64(*f as u64, 64),
            NumberLiteral::UnbasedUnsized(c) => match c {
                '0' => Value::zero(32),
                '1' => Value::ones(32),
                'x' | 'X' => Value::new(32),  // all X
                'z' | 'Z' => Value { bits: vec![LogicBit::Z; 32], width: 32, is_signed: false },
                _ => Value::new(32),
            },
        }
    }

    pub fn exec_statement(&mut self, stmt: &Statement) {
        if self.finished || self.time > self.max_time || self.break_flag || self.continue_flag { return; }
        match &stmt.kind {
            StatementKind::Null => {}
            StatementKind::Expr(expr) => self.exec_expr_stmt(expr),
            StatementKind::BlockingAssign { lvalue, rvalue } => {
                let w = self.infer_lhs_width(lvalue);
                let val = self.eval_expr_ctx(rvalue, w); self.assign_value(lvalue, &val);
                self.settle_combinatorial();
            }
            StatementKind::NonblockingAssign { lvalue, rvalue, .. } => {
                let w = self.infer_lhs_width(lvalue);
                let val = self.eval_expr_ctx(rvalue, w);
                // Resolve the LHS target NOW to capture array indices at schedule time
                let resolved = self.resolve_nba_target(lvalue);
                self.nba_queue.push(NbaEntry { lhs: lvalue.clone(), value: val.resize(w), resolved_target: resolved });
            }
            StatementKind::If { condition, then_stmt, else_stmt, .. } => {
                if self.eval_expr(condition).is_true() { self.exec_statement(then_stmt); }
                else if let Some(el) = else_stmt { self.exec_statement(el); }
            }
            StatementKind::Case { expr, items, .. } => {
                let val = self.eval_expr(expr); let mut matched = false;
                for (iidx, item) in items.iter().enumerate() { if item.is_default { continue; } for pat in &item.patterns { if val.case_eq(&self.eval_expr(pat)).is_true() {
                    self.exec_statement(&item.stmt); matched = true; break; } } if matched { break; } }
                if !matched { for item in items { if item.is_default {
                    self.exec_statement(&item.stmt); break; } } }
            }
            StatementKind::For { init, condition, step, body } => {
                for fi in init { match fi {
                    ForInit::VarDecl { data_type, name, init: e } => { let v = self.eval_expr(e); let w = super::elaborate::resolve_type_width(data_type); self.widths.insert(name.name.clone(), w); self.signals.insert(name.name.clone(), v.resize(w)); }
                    ForInit::Assign { lvalue, rvalue } => { let v = self.eval_expr(rvalue); self.assign_value(lvalue, &v); }
                }}
                let mut iters = 0;
                loop {
                    if iters > 10000 || self.finished { break; } iters += 1;
                    if let Some(c) = condition { if !self.eval_expr(c).is_true() { break; } }
                    self.break_flag = false; self.continue_flag = false; self.exec_statement(body);
                    if self.break_flag { self.break_flag = false; break; } self.continue_flag = false;
                    for s in step { self.exec_expr_stmt(s); }
                }
            }
            StatementKind::Foreach { array, vars, body } => {
                if let ExprKind::Ident(hier) = &array.kind {
                    let name = self.resolve_hier_name(hier);
                    let size = self.widths.get(&name).copied().unwrap_or(1) as u64;
                    if let Some(var) = vars.first().and_then(|v| v.as_ref()) {
                        self.widths.insert(var.name.clone(), 32);
                        for i in 0..size { if self.finished { break; } self.signals.insert(var.name.clone(), Value::from_u64(i, 32)); self.exec_statement(body); }
                    }
                }
            }
            StatementKind::While { condition, body } => { let mut i = 0; loop { if i > 10000 || self.finished { break; } i += 1; if !self.eval_expr(condition).is_true() { break; } self.break_flag = false; self.exec_statement(body); if self.break_flag { self.break_flag = false; break; } } }
            StatementKind::DoWhile { body, condition } => { let mut i = 0; loop { if i > 10000 || self.finished { break; } i += 1; self.break_flag = false; self.exec_statement(body); if self.break_flag { self.break_flag = false; break; } if !self.eval_expr(condition).is_true() { break; } } }
            StatementKind::Repeat { count, body } => { let n = self.eval_expr(count).to_u64().unwrap_or(0); for _ in 0..n.min(10000) { if self.finished { break; } self.exec_statement(body); } }
            StatementKind::Forever { body } => { let mut i = 0; loop { if i > 100000 || self.finished || self.time > self.max_time { break; } i += 1; self.exec_statement(body); } }
            StatementKind::SeqBlock { stmts, .. } => { for s in stmts { if self.finished || self.break_flag || self.continue_flag { break; } self.exec_statement(s); } }
            StatementKind::ParBlock { stmts, .. } => { for s in stmts { if self.finished { break; } self.exec_statement(s); } }
            StatementKind::TimingControl { control, stmt } => {
                match control {
                    TimingControl::Delay(d) => {
                        let delay = self.eval_expr(d).to_u64().unwrap_or(0);
                        self.apply_nba(); self.settle_combinatorial(); self.prev_signals = self.signals.clone();
                        self.time += delay;
                        self.settle_combinatorial(); self.check_monitor();
                    }
                    TimingControl::Event(_) => {}
                }
                self.exec_statement(stmt);
                // After body executes, check for edges (e.g., clk toggled)
                self.settle_combinatorial();
                self.check_edges();
                self.apply_nba();
                self.settle_combinatorial();
                self.prev_signals = self.signals.clone();
            }
            StatementKind::Break => { self.break_flag = true; }
            StatementKind::Continue => { self.continue_flag = true; }
            StatementKind::Return(_) | StatementKind::Disable(_) | StatementKind::WaitFork => {}
            StatementKind::Wait { condition, stmt } => { if self.eval_expr(condition).is_true() { self.exec_statement(stmt); } }
            StatementKind::Assertion(a) => {
                if !self.eval_expr(&a.expr).is_true() { if let Some(ea) = &a.else_action { self.exec_statement(ea); } }
                else if let Some(ac) = &a.action { self.exec_statement(ac); }
            }
            StatementKind::ProceduralContinuous(pc) => {
                match pc {
                    ProceduralContinuous::Assign { lvalue, rvalue } | ProceduralContinuous::Force { lvalue, rvalue } => { let v = self.eval_expr(rvalue); self.assign_value(lvalue, &v); }
                    _ => {}
                }
            }
            StatementKind::VarDecl { data_type, declarators, .. } => {
                let w = super::elaborate::resolve_type_width(data_type);
                for d in declarators { let v = d.init.as_ref().map(|i| self.eval_expr(i).resize(w)).unwrap_or(Value::new(w)); self.widths.insert(d.name.name.clone(), w); self.signals.insert(d.name.name.clone(), v); }
            }
        }
    }

    fn exec_expr_stmt(&mut self, expr: &Expression) {
        match &expr.kind {
            ExprKind::SystemCall { name, args } => self.exec_system_task(name, args),
            ExprKind::Binary { op: BinaryOp::Assign, left, right } => {
                let val = self.eval_expr(right);
                self.assign_value(left, &val);
            }
            ExprKind::Unary { op, operand } => match op {
                UnaryOp::PreIncr | UnaryOp::PostIncr => { let v = self.eval_expr(operand); let nv = v.add(&Value::from_u64(1, v.width)); self.assign_value(operand, &nv); }
                UnaryOp::PreDecr | UnaryOp::PostDecr => { let v = self.eval_expr(operand); let nv = v.sub(&Value::from_u64(1, v.width)); self.assign_value(operand, &nv); }
                _ => { self.eval_expr(expr); }
            },
            _ => { self.eval_expr(expr); }
        }
    }

    fn exec_system_task(&mut self, name: &str, args: &[Expression]) {
        match name {
            "$display" | "$displayb" | "$displayh" | "$displayo" => { let m = self.format_args(args, name); self.output.push(SimOutput { time: self.time, message: m.clone() }); println!("{}", m); }
            "$write" | "$writeb" | "$writeh" | "$writeo" => { let m = self.format_args(args, name); self.output.push(SimOutput { time: self.time, message: m.clone() }); print!("{}", m); }
            "$monitor" | "$monitorb" | "$monitorh" | "$monitoro" => { self.monitor = Some((name.to_string(), args.to_vec())); self.check_monitor(); }
            "$monitoroff" => { self.monitor = None; }
            "$finish" | "$stop" => { self.finished = true; }
            "$dumpfile" => {
                if let Some(arg) = args.first() {
                    if let ExprKind::StringLiteral(s) = &arg.kind {
                        self.vcd_file = Some(s.clone());
                    } else {
                        self.vcd_file = Some("dump.vcd".to_string());
                    }
                }
            }
            "$dumpvars" => {
                self.vcd_start_dump();
            }
            "$dumpoff" => { self.vcd_enabled = false; }
            "$dumpon" => { self.vcd_enabled = true; }
            _ => {}
        }
    }

    fn format_args(&self, args: &[Expression], tn: &str) -> String {
        if args.is_empty() { return String::new(); }
        if let ExprKind::StringLiteral(fmt) = &args[0].kind { return self.format_string(fmt, &args[1..], tn); }
        let r = if tn.ends_with('b') { 'b' } else if tn.ends_with('h') { 'h' } else { 'd' };
        args.iter().map(|a| { let v = self.eval_expr(a); match r { 'b' => v.to_bin_string(), 'h' => v.to_hex_string(), _ => v.to_dec_string() } }).collect::<Vec<_>>().join(" ")
    }

    fn format_string(&self, fmt: &str, args: &[Expression], _tn: &str) -> String {
        let mut result = String::new(); let mut ai = 0; let mut chars = fmt.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '%' {
                let mut width_str = String::new();
                while chars.peek().map_or(false, |c| c.is_ascii_digit()) { width_str.push(chars.next().unwrap()); }
                let pad_width: usize = width_str.parse().unwrap_or(0);
                let zero_pad = width_str.starts_with('0');
                if let Some(&spec) = chars.peek() { chars.next(); match spec {
                    '%' => result.push('%'),
                    't' | 'T' => { if ai < args.len() { if let ExprKind::SystemCall { name, .. } = &args[ai].kind { if name == "$time" { let s = format!("{}", self.time); result.push_str(&pad_string(&s, pad_width, zero_pad)); ai += 1; continue; } } let s = self.eval_expr(&args[ai]).to_dec_string(); result.push_str(&pad_string(&s, pad_width, zero_pad)); ai += 1; } }
                    _ => { if ai < args.len() { let v = self.eval_expr(&args[ai]); ai += 1; match spec {
                        'd' | 'D' => { let s = v.to_dec_string(); result.push_str(&pad_string(&s, pad_width, zero_pad)); }
                        'b' | 'B' => { let s = v.to_bin_string(); result.push_str(&pad_string(&s, pad_width, zero_pad)); }
                        'h' | 'H' | 'x' | 'X' => { let s = v.to_hex_string(); result.push_str(&pad_string(&s, pad_width, zero_pad)); }
                        'o' | 'O' => { let s = if let Some(u) = v.to_u64() { format!("{:o}", u) } else { "x".to_string() }; result.push_str(&pad_string(&s, pad_width, zero_pad)); }
                        's' | 'S' => { if let ExprKind::StringLiteral(s) = &args[ai-1].kind { result.push_str(s); } else { result.push_str(&v.to_dec_string()); } }
                        'm' | 'M' => { result.push_str(&self.module.name); ai -= 1; }
                        _ => { result.push('%'); result.push_str(&width_str); result.push(spec); ai -= 1; }
                    }}}
                }}
            } else if c == '\\' { if let Some(&e) = chars.peek() { chars.next(); match e { 'n' => result.push('\n'), 't' => result.push('\t'), '\\' => result.push('\\'), '"' => result.push('"'), _ => { result.push('\\'); result.push(e); } } } }
            else { result.push(c); }
        }
        result
    }

    fn check_monitor(&mut self) {
        if let Some((tn, args)) = self.monitor.clone() {
            let m = self.format_args(&args, &tn);
            let mut changed = self.monitor_prev.is_empty();
            for (n, v) in &self.signals { if let Some(p) = self.monitor_prev.get(n) { if p != v { changed = true; break; } } }
            if changed { self.output.push(SimOutput { time: self.time, message: m.clone() }); println!("{}", m); self.monitor_prev = self.signals.clone(); }
        }
    }

    fn resolve_hier_name(&self, hier: &HierarchicalIdentifier) -> String { hier.path.last().map(|s| s.name.name.clone()).unwrap_or_default() }

    /// Extract all signal names referenced in an expression (for wait statement).
    fn extract_signal_names(&self, expr: &Expression) -> Vec<String> {
        let mut names = Vec::new();
        self.collect_signal_names(expr, &mut names);
        names.sort(); names.dedup(); names
    }
    fn collect_signal_names(&self, expr: &Expression, names: &mut Vec<String>) {
        match &expr.kind {
            ExprKind::Ident(hier) => { names.push(self.resolve_hier_name(hier)); }
            ExprKind::Unary { operand, .. } => { self.collect_signal_names(operand, names); }
            ExprKind::Binary { left, right, .. } => { self.collect_signal_names(left, names); self.collect_signal_names(right, names); }
            ExprKind::Conditional { condition, then_expr, else_expr } => { self.collect_signal_names(condition, names); self.collect_signal_names(then_expr, names); self.collect_signal_names(else_expr, names); }
            ExprKind::Index { expr, index } => { self.collect_signal_names(expr, names); self.collect_signal_names(index, names); }
            ExprKind::Paren(inner) => { self.collect_signal_names(inner, names); }
            _ => {}
        }
    }
    fn infer_width(&self, expr: &Expression) -> u32 { match &expr.kind { ExprKind::Ident(h) => { let n = self.resolve_hier_name(h); self.widths.get(&n).copied().unwrap_or(1) } ExprKind::Number(NumberLiteral::Integer { size, .. }) => size.unwrap_or(32), ExprKind::Concatenation(p) => p.iter().map(|x| self.infer_width(x)).sum(), _ => self.eval_expr(expr).width } }
    fn infer_lhs_width(&self, expr: &Expression) -> u32 { match &expr.kind { ExprKind::Concatenation(p) => p.iter().map(|x| self.infer_lhs_width(x)).sum(), ExprKind::Ident(h) => { let n = self.resolve_hier_name(h); self.widths.get(&n).copied().unwrap_or(32) } ExprKind::RangeSelect { left, right, .. } => { let l = self.eval_expr(left).to_u64().unwrap_or(0); let r = self.eval_expr(right).to_u64().unwrap_or(0); if l >= r { (l-r+1) as u32 } else { (r-l+1) as u32 } } ExprKind::Index { expr: e, index } => { if let ExprKind::Ident(h) = &e.kind { let n = self.resolve_hier_name(h); if let Some((_, _, w)) = self.module.arrays.get(&n) { return *w; } } 1 } _ => self.infer_width(expr) } }
    pub fn get_signal(&self, name: &str) -> Option<&Value> { self.signals.get(name) }
    pub fn set_signal(&mut self, name: &str, val: Value) { if let Some(w) = self.widths.get(name) { self.signals.insert(name.to_string(), val.resize(*w)); } else { self.widths.insert(name.to_string(), val.width); self.signals.insert(name.to_string(), val); } }

    // ═══════════════════════════════════════════════════════════════
    // VCD dump support ($dumpfile / $dumpvars)
    // ═══════════════════════════════════════════════════════════════

    /// Generate a VCD identifier code from an index (!, ", #, ... multi-char for large designs)
    fn vcd_id_code(mut idx: usize) -> String {
        let mut code = String::new();
        loop {
            code.push((b'!' + (idx % 94) as u8) as char);
            idx /= 94;
            if idx == 0 { break; }
            idx -= 1;
        }
        code
    }

    /// Start VCD dumping: open file, write header, record initial values
    fn vcd_start_dump(&mut self) {
        let filename = self.vcd_file.clone().unwrap_or_else(|| "dump.vcd".to_string());
        let file = match std::fs::File::create(&filename) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("Warning: cannot create VCD file '{}': {}", filename, e);
                return;
            }
        };
        let mut w = std::io::BufWriter::new(file);

        // Collect and sort signal names for deterministic output
        let mut sig_names: Vec<String> = self.signals.keys().cloned().collect();
        sig_names.sort();

        // Assign VCD identifier codes
        let mut id_map = HashMap::new();
        for (idx, name) in sig_names.iter().enumerate() {
            id_map.insert(name.clone(), Self::vcd_id_code(idx));
        }

        // Write VCD header
        let _ = writeln!(w, "$date\n  Simulation generated by sisvsim\n$end");
        let _ = writeln!(w, "$version\n  sisvsim 0.1\n$end");
        let _ = writeln!(w, "$timescale\n  1ns\n$end");

        // Write variable definitions
        // Group signals by module prefix (split on '.')
        let _ = writeln!(w, "$scope module top $end");
        for name in &sig_names {
            let width = self.widths.get(name).copied().unwrap_or(1);
            let id = &id_map[name];
            // Use the signal name directly, replacing '.' with '_' for display
            let display_name = name.replace('.', "_");
            let _ = writeln!(w, "$var wire {} {} {} $end", width, id, display_name);
        }
        let _ = writeln!(w, "$upscope $end");
        let _ = writeln!(w, "$enddefinitions $end");

        // Write initial values
        let _ = writeln!(w, "$dumpvars");
        for name in &sig_names {
            let val = self.signals.get(name).cloned().unwrap_or_else(|| Value::new(1));
            let id = &id_map[name];
            Self::vcd_write_value(&mut w, &val, id);
        }
        let _ = writeln!(w, "$end");

        // Record initial snapshot
        let vcd_prev = self.signals.clone();

        self.vcd_id_map = id_map;
        self.vcd_writer = Some(w);
        self.vcd_enabled = true;
        self.vcd_last_time = self.time;
        self.vcd_prev_signals = vcd_prev;
    }

    /// Write a single value to VCD
    fn vcd_write_value(w: &mut impl Write, val: &Value, id: &str) {
        if val.width == 1 {
            // Scalar: single char + id
            let ch = match val.bits.first().unwrap_or(&LogicBit::X) {
                LogicBit::Zero => '0',
                LogicBit::One => '1',
                LogicBit::X => 'x',
                LogicBit::Z => 'z',
            };
            let _ = writeln!(w, "{}{}", ch, id);
        } else {
            // Vector: b<bits> <id>
            let mut s = String::with_capacity(val.width as usize + 2);
            s.push('b');
            let mut all_zero = true;
            for i in (0..val.width as usize).rev() {
                let ch = match val.bits.get(i).unwrap_or(&LogicBit::Zero) {
                    LogicBit::Zero => { if !all_zero { s.push('0'); } '0' }
                    LogicBit::One => { all_zero = false; s.push('1'); '1' }
                    LogicBit::X => { all_zero = false; s.push('x'); 'x' }
                    LogicBit::Z => { all_zero = false; s.push('z'); 'z' }
                };
                let _ = ch;
            }
            if all_zero { s.push('0'); }
            let _ = writeln!(w, "{} {}", s, id);
        }
    }

    /// Write VCD value changes for the current timestep
    fn vcd_write_changes(&mut self) {
        if !self.vcd_enabled || self.vcd_writer.is_none() { return; }

        // Collect changes
        let mut changes: Vec<(String, Value)> = Vec::new();
        for (name, val) in &self.signals {
            if let Some(id) = self.vcd_id_map.get(name) {
                let changed = match self.vcd_prev_signals.get(name) {
                    Some(prev) => prev.bits != val.bits,
                    None => true,
                };
                if changed {
                    changes.push((id.clone(), val.clone()));
                }
            }
        }

        if changes.is_empty() { return; }

        let w = self.vcd_writer.as_mut().unwrap();

        // Write timestamp if we haven't yet for this time
        if self.time != self.vcd_last_time {
            let _ = writeln!(w, "#{}", self.time);
            self.vcd_last_time = self.time;
        }

        // Write changed values
        for (id, val) in &changes {
            Self::vcd_write_value(w, val, id);
        }

        // Update previous snapshot
        self.vcd_prev_signals = self.signals.clone();
    }

    /// Flush and close VCD file
    fn vcd_finish(&mut self) {
        if let Some(ref mut w) = self.vcd_writer {
            let _ = w.flush();
        }
        self.vcd_writer = None;
    }
}
