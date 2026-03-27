//! Event-driven simulator for SystemVerilog combinatorial and sequential logic.
//!
//! Implements a simplified IEEE 1800 scheduling model:
//!   Active region:  blocking assigns, continuous assigns, always_comb
//!   NBA region:     non-blocking assign updates
//!   Reactive:       edge-triggered always_ff/always_latch blocks

use std::collections::BTreeMap;
use std::cell::Cell;
use ahash::{AHashMap as HashMap, AHashSet as HashSet};
use std::io::Write;
use crate::ast::expr::*;
use crate::ast::stmt::*;
use crate::ast::decl::AlwaysKind;
use super::value::{Value, LogicBit};
use super::elaborate::{ElaboratedModule, AlwaysBlock};

/// A combinatorial item (continuous assign or always @*/always_comb block)
/// with pre-computed sensitivity set for efficient evaluation.
#[derive(Clone)]
enum CombItem {
    ContAssign { lhs: Expression, rhs: Expression },
    /// Fast path: direct signal-to-signal copy (assign b = a) with pre-resolved IDs.
    DirectCopy { dst_id: usize, src_id: usize, width: u32 },
    AlwaysBlock { stmt: Statement, is_always_comb: bool },
}

#[derive(Clone)]
struct CombEntry {
    item: CombItem,
    /// Set of signal names this block reads. If ANY of these changed,
    /// the block needs re-evaluation.
    read_signals: HashSet<String>,
    /// Set of signal names this block writes (for change tracking).
    write_signals: HashSet<String>,
    /// Pre-resolved signal IDs for read_signals (for fast dependency lookup).
    read_signal_ids: Vec<usize>,
    /// Pre-resolved (signal_id, signal_name) pairs for write_signals.
    write_signal_ids: Vec<(usize, String)>,
}

#[derive(Debug, Clone)]
pub struct SimOutput { pub time: u64, pub message: String }

#[derive(Debug, Clone)]
/// Slow-path NBA entry: carries full AST LHS for unresolved targets.
struct NbaEntry { lhs: Option<Expression>, value: Value, resolved_id: Option<usize> }

/// Fast-path NBA entry: compact (signal_id, value) pair for pre-resolved targets.
/// 99%+ of NBA entries use this path. Smaller struct = better cache utilization.
struct NbaFast { signal_id: usize, value: Value }

#[derive(Debug, Clone)]
struct EdgeSensitiveBlock {
    sensitivities: Vec<Sensitivity>,
    /// Pre-resolved signal IDs for O(1) edge checking (populated during classify)
    resolved_sensitivities: Vec<SensitivityId>,
    stmt: Statement,
    kind: AlwaysKind,
}

#[derive(Debug, Clone)]
struct SensitivityId { signal_id: usize, edge: EdgeKind }

#[derive(Debug, Clone)]
struct Sensitivity { signal_name: String, edge: EdgeKind }

#[derive(Debug, Clone, Copy, PartialEq)]
enum EdgeKind { Posedge, Negedge, AnyEdge }

/// A process waiting for a signal edge event.
#[derive(Debug, Clone)]
struct EventWaiter {
    pid: usize,
    sensitivities: Vec<Sensitivity>,
    /// Pre-resolved signal IDs for O(1) edge checking
    resolved_sensitivities: Vec<SensitivityId>,
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

/// Timing wheel for O(1) near-future event scheduling.
/// Events within WHEEL_SIZE ticks of current time use a circular array.
/// Events further out fall back to a BTreeMap.
const WHEEL_SIZE: usize = 256;
/// Number of u64 words needed for the occupancy bitmap (256 / 64 = 4).
const BITMAP_WORDS: usize = WHEEL_SIZE / 64;

type EventList = Vec<(usize, Vec<Statement>)>;

/// Built-in clock generator: replaces `always #N clk = ~clk` with O(1) toggle.
/// Eliminates AST cloning and traversal for the most common simulation pattern.
struct ClockGen {
    signal_id: usize,
    half_period: u64,
    next_toggle_time: u64,
}

struct TimingWheel {
    wheel: Vec<EventList>,       // circular array of WHEEL_SIZE slots
    bitmap: [u64; BITMAP_WORDS], // occupancy bitmap: bit set = slot non-empty
    overflow: BTreeMap<u64, EventList>, // far-future events
    current_time: u64,           // last known simulation time
}

impl TimingWheel {
    fn new() -> Self {
        let mut wheel = Vec::with_capacity(WHEEL_SIZE);
        for _ in 0..WHEEL_SIZE { wheel.push(Vec::new()); }
        TimingWheel { wheel, bitmap: [0u64; BITMAP_WORDS], overflow: BTreeMap::new(), current_time: 0 }
    }

    #[inline(always)]
    fn slot(time: u64) -> usize { (time as usize) & (WHEEL_SIZE - 1) }

    /// Set bitmap bit for a slot.
    #[inline(always)]
    fn bitmap_set(&mut self, slot: usize) {
        self.bitmap[slot >> 6] |= 1u64 << (slot & 63);
    }

    /// Clear bitmap bit for a slot.
    #[inline(always)]
    fn bitmap_clear(&mut self, slot: usize) {
        self.bitmap[slot >> 6] &= !(1u64 << (slot & 63));
    }

    /// Schedule an event at the given time.
    fn schedule(&mut self, time: u64, pid: usize, stmts: Vec<Statement>) {
        if time < self.current_time + WHEEL_SIZE as u64 {
            let s = Self::slot(time);
            self.wheel[s].push((pid, stmts));
            self.bitmap_set(s);
        } else {
            self.overflow.entry(time).or_default().push((pid, stmts));
        }
    }

    /// Schedule multiple events at the given time.
    fn schedule_push(&mut self, time: u64, entry: (usize, Vec<Statement>)) {
        self.schedule(time, entry.0, entry.1);
    }

    fn is_empty(&self) -> bool {
        self.bitmap == [0u64; BITMAP_WORDS] && self.overflow.is_empty()
    }

    /// Get the next scheduled time (minimum) using bitmap scan.
    /// Uses trailing_zeros to find the next occupied slot in O(1) per word.
    fn next_time(&self) -> Option<u64> {
        let start_slot = Self::slot(self.current_time);
        // Scan bitmap from current_time's slot position.
        // We need to handle wrap-around: scan from start_slot to 255, then 0 to start_slot-1.
        // But with bitmap, we can do this efficiently per-word.

        // Phase 1: scan from start_slot to end of bitmap
        let start_word = start_slot >> 6;
        let start_bit = start_slot & 63;

        // Mask off bits below start_bit in the first word
        let first_masked = self.bitmap[start_word] & (!0u64 << start_bit);
        if first_masked != 0 {
            let bit = first_masked.trailing_zeros() as usize;
            let slot = (start_word << 6) | bit;
            let delta = if slot >= start_slot { slot - start_slot } else { slot + WHEEL_SIZE - start_slot };
            return Some(self.current_time + delta as u64);
        }
        // Scan remaining words after start_word
        for w in 1..BITMAP_WORDS {
            let word_idx = (start_word + w) & (BITMAP_WORDS - 1);
            if self.bitmap[word_idx] != 0 {
                let bit = self.bitmap[word_idx].trailing_zeros() as usize;
                let slot = (word_idx << 6) | bit;
                let delta = if slot >= start_slot { slot - start_slot } else { slot + WHEEL_SIZE - start_slot };
                return Some(self.current_time + delta as u64);
            }
        }
        // Check overflow
        self.overflow.keys().next().copied()
    }

    /// Remove and return all events at the given time.
    fn remove(&mut self, time: u64) -> EventList {
        self.current_time = time;
        // Drain overflow events that now fit in the wheel (rare)
        if !self.overflow.is_empty() {
            let cutoff = time + WHEEL_SIZE as u64;
            let mut to_move = Vec::new();
            for (&t, _) in self.overflow.range(..cutoff) {
                to_move.push(t);
            }
            for t in to_move {
                if let Some(events) = self.overflow.remove(&t) {
                    let s = Self::slot(t);
                    self.wheel[s].extend(events);
                    self.bitmap_set(s);
                }
            }
        }

        let s = Self::slot(time);
        let events = std::mem::take(&mut self.wheel[s]);
        if !events.is_empty() {
            // Only clear bitmap if slot is truly empty
            self.bitmap_clear(s);
        }
        events
    }
}

pub struct Simulator {
    pub signals: HashMap<String, Value>,
    /// Fast signal table: indexed by signal_id for O(1) access.
    signal_table: Vec<Value>,
    /// Map signal name → signal_id for fast lookup.
    signal_name_to_id: HashMap<String, usize>,
    id_to_name: Vec<String>,
    /// Map signal_id → width (for fast width lookup).
    signal_widths: Vec<u32>,
    /// Set of signal IDs that are signed.
    signal_signed: Vec<bool>,
    pub widths: HashMap<String, u32>,
    pub signed_signals: HashSet<String>,
    prev_signals: HashMap<String, Value>,
    /// Fast prev signal table for edge detection (indexed by signal_id).
    prev_table: Vec<Value>,
    edge_signal_names: HashSet<String>,
    /// Edge sensitivity resolved to signal IDs.
    edge_signal_ids: Vec<usize>,
    pub time: u64,
    pub output: Vec<SimOutput>,
    pub finished: bool,
    pub monitor: Option<(String, Vec<Expression>)>,
    pub monitor_prev: HashMap<String, Value>,
    pub max_time: u64,
    /// Maximum iterations for combinatorial settling per cycle.
    pub settle_limit: u32,
    module: ElaboratedModule,
    settling: bool,
    in_edge_block: bool,
    nba_queue: Vec<NbaEntry>,
    /// Fast-path NBA buffer: pre-resolved (signal_id, value) pairs.
    nba_fast: Vec<NbaFast>,
    edge_blocks: Vec<EdgeSensitiveBlock>,
    /// Bytecode-compiled edge blocks (for blocks that compiled successfully).
    /// Index matches edge_blocks. None = fallback to AST interpreter.
    compiled_edge_blocks: Vec<Option<super::bytecode::CompiledBlock>>,
    /// VM register file (reusable across executions to avoid allocation).
    vm_regs: Vec<Value>,
    /// Built-in clock generators (optimized always #N clk = ~clk)
    clock_generators: Vec<ClockGen>,
    event_queue: TimingWheel,
    next_pid: usize,
    break_flag: bool,
    continue_flag: bool,
    /// Processes waiting for signal edge events (@(posedge clk), etc.)
    event_waiters: Vec<EventWaiter>,
    /// Swap buffer for event_waiters filtering (avoids allocation per cycle)
    event_waiters_swap: Vec<EventWaiter>,
    /// VCD dump state
    vcd_file: Option<String>,
    vcd_writer: Option<std::io::BufWriter<std::fs::File>>,
    vcd_id_map: HashMap<String, String>,
    vcd_enabled: bool,
    vcd_last_time: u64,
    vcd_prev_signals: HashMap<String, Value>,
    /// Pre-computed combinatorial entries with sensitivity sets.
    comb_entries: Vec<CombEntry>,
    /// Reverse index: signal_id → list of comb_entry indices that read this signal.
    comb_dep_by_id: Vec<Vec<usize>>,
    /// Bitvec: dirty_signals[signal_id] = true if signal changed since last settle.
    dirty_signals: Vec<bool>,
    /// Explicit list of dirty signal IDs (maintained alongside dirty_signals bitvec)
    /// This avoids O(num_signals) scan in settle_combinatorial.
    dirty_list: Vec<usize>,
    dirty_any: bool,
    /// When true, signal_table has been modified and signals HashMap is stale.
    table_modified: bool,
    settle_calls: u64,
    // Profiling accumulators (nanoseconds)
    prof_settle: u64,
    prof_edges: u64,
    prof_nba: u64,
    prof_process: u64,
    prof_snapshot: u64,
    prof_vcd: u64,
    /// Persistent buffers for settle_combinatorial (avoid repeated allocation)
    settle_triggered: Vec<bool>,
    settle_dirty_ids: Vec<usize>,
    /// Pre-allocated buffer for always block write_signal snapshots during settle
    settle_prev_values: Vec<(usize, Value)>,
    /// Track which entries were triggered (for selective clearing)
    settle_triggered_list: Vec<usize>,
    loop_iters: u64,
    t_prevclone: std::time::Duration,
    t_process: std::time::Duration,
    t_settle_total: std::time::Duration,
    t_edges: std::time::Duration,
    entry_evals: u64,
    settle_iters: u64,
    max_settle_iters: u64,
    /// Per-comb-entry trigger count (for --activity-mon).
    activity_counts: Vec<u64>,
    /// Per-signal change count (for --activity-mon).
    signal_toggle_counts: Vec<u64>,
    /// Whether activity monitoring is enabled.
    pub activity_mon: bool,
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
        // Build fast signal table (Vec-based, indexed by signal_id)
        let mut sig_names_sorted: Vec<String> = signals.keys().cloned().collect();
        sig_names_sorted.sort();
        let mut signal_name_to_id = HashMap::new();
        let mut signal_table = Vec::with_capacity(sig_names_sorted.len());
        let mut signal_widths_vec = Vec::with_capacity(sig_names_sorted.len());
        let mut signal_signed_vec = Vec::with_capacity(sig_names_sorted.len());
        for (id, name) in sig_names_sorted.iter().enumerate() {
            signal_name_to_id.insert(name.clone(), id);
            signal_table.push(signals[name].clone());
            signal_widths_vec.push(widths.get(name).copied().unwrap_or(1));
            signal_signed_vec.push(signed_signals.contains(name));
        }
        let num_signals = sig_names_sorted.len();
        let prev_table = signal_table.clone();

        Self {
            prev_signals: HashMap::new(),
            prev_table,
            edge_signal_names: HashSet::new(),
            edge_signal_ids: Vec::new(),
            signals, widths, signed_signals,
            signal_table, signal_name_to_id, id_to_name: sig_names_sorted, signal_widths: signal_widths_vec, signal_signed: signal_signed_vec,
            time: 0, output: Vec::new(), finished: false,
            monitor: None, monitor_prev: HashMap::new(),
            max_time, settle_limit: 100, module, settling: false, in_edge_block: false,
            nba_queue: Vec::new(), nba_fast: Vec::new(), edge_blocks: Vec::new(), compiled_edge_blocks: Vec::new(), vm_regs: Vec::new(), clock_generators: Vec::new(),
            event_queue: TimingWheel::new(), next_pid: 0,
            break_flag: false, continue_flag: false,
            event_waiters: Vec::new(),
            event_waiters_swap: Vec::new(),
            vcd_file: None,
            vcd_writer: None,
            vcd_id_map: HashMap::new(),
            vcd_enabled: false,
            vcd_last_time: u64::MAX,
            vcd_prev_signals: HashMap::new(),
            comb_entries: Vec::new(),
            comb_dep_by_id: Vec::new(),
            dirty_signals: vec![false; num_signals],
            dirty_list: Vec::with_capacity(num_signals),
            dirty_any: false,
            table_modified: false,
            settle_calls: 0, settle_triggered: Vec::new(), settle_dirty_ids: Vec::new(),
            settle_prev_values: Vec::new(), settle_triggered_list: Vec::new(), loop_iters: 0,
            prof_settle: 0, prof_edges: 0, prof_nba: 0, prof_process: 0, prof_snapshot: 0, prof_vcd: 0,
            t_prevclone: std::time::Duration::ZERO,
            t_process: std::time::Duration::ZERO,
            t_settle_total: std::time::Duration::ZERO,
            t_edges: std::time::Duration::ZERO,
            entry_evals: 0,
            settle_iters: 0,
            max_settle_iters: 0,
            activity_counts: Vec::new(),
            signal_toggle_counts: vec![0u64; num_signals],
            activity_mon: false,
        }
    }

    pub fn run(&mut self) {
        self.classify_always_blocks();
        self.compile_edge_blocks();
        self.build_comb_entries();
        if self.activity_mon {
            self.activity_counts = vec![0u64; self.comb_entries.len()];
        }
        // Collect all edge-sensitive signal names for targeted prev snapshots
        for block in &self.edge_blocks {
            for sens in &block.sensitivities {
                self.edge_signal_names.insert(sens.signal_name.clone());
                if let Some(&id) = self.signal_name_to_id.get(&sens.signal_name) {
                    self.edge_signal_ids.push(id);
                }
            }
        }
        // Also collect from event waiters that are registered at time 0
        self.edge_signal_ids.sort_unstable();
        self.edge_signal_ids.dedup();
        // IEEE 1800: at time 0, always_comb blocks execute unconditionally.
        // always @* blocks do NOT execute at time 0 unless inputs change.
        // Mark all signals dirty so continuous assigns and always_comb run.
        self.dirty_list.clear();
        for i in 0..self.dirty_signals.len() { self.dirty_signals[i] = true; self.dirty_list.push(i); }
        self.dirty_any = true;
        self.settle_combinatorial();
        let initial_blocks = self.module.initial_blocks.clone();
        for ib in &initial_blocks {
            let stmts = match &ib.stmt.kind {
                StatementKind::SeqBlock { stmts, .. } => stmts.clone(),
                _ => vec![ib.stmt.clone()],
            };
            let pid = self.next_pid; self.next_pid += 1;
            self.event_queue.schedule(0, pid, stmts);
        }
        self.event_loop();
        self.vcd_finish();
    }

    /// Try to detect `always #N var = ~var` pattern and extract as a ClockGen.
    fn try_extract_clock_gen(&self, body: &Statement, half_period: u64) -> Option<ClockGen> {
        // Body should be: var = ~var (blocking assign)
        let assign = match &body.kind {
            StatementKind::BlockingAssign { lvalue, rvalue, .. } => Some((lvalue, rvalue)),
            StatementKind::SeqBlock { stmts, .. } if stmts.len() == 1 => {
                if let StatementKind::BlockingAssign { lvalue, rvalue, .. } = &stmts[0].kind {
                    Some((lvalue, rvalue))
                } else { None }
            }
            _ => None,
        }?;
        let (lhs, rhs) = assign;

        // LHS must be a simple identifier
        let lhs_name = match &lhs.kind {
            ExprKind::Ident(hier) => hier.path.last().map(|s| s.name.name.as_str()),
            _ => None,
        }?;
        let &signal_id = self.signal_name_to_id.get(lhs_name)?;

        // RHS must be ~LHS or !LHS
        match &rhs.kind {
            ExprKind::Unary { op: UnaryOp::BitNot, operand } |
            ExprKind::Unary { op: UnaryOp::LogNot, operand } => {
                if let ExprKind::Ident(hier) = &operand.kind {
                    let rhs_name = hier.path.last().map(|s| s.name.name.as_str())?;
                    if rhs_name == lhs_name {
                        return Some(ClockGen {
                            signal_id,
                            half_period,
                            next_toggle_time: half_period, // first toggle at t=half_period
                        });
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Advance all clock generators to the given time, toggling signals as needed.
    /// Returns the next toggle time across all clock generators (for time advancement).
    #[inline]
    fn advance_clock_generators(&mut self) -> Option<u64> {
        let mut next_time = u64::MAX;
        for cg in &mut self.clock_generators {
            while cg.next_toggle_time <= self.time {
                // Should not happen in normal operation, but guard against it
                cg.next_toggle_time += cg.half_period;
            }
            if cg.next_toggle_time == self.time + 0 {
                // Toggle at current time (handled below)
            }
            next_time = next_time.min(cg.next_toggle_time);
        }
        if next_time == u64::MAX { None } else { Some(next_time) }
    }

    /// Toggle clock generators that fire at the current time.
    fn fire_clock_generators(&mut self) {
        for cg in &mut self.clock_generators {
            if cg.next_toggle_time == self.time {
                let cur = self.signal_table[cg.signal_id].bits_first();
                let new_val = if cur == LogicBit::One {
                    Value::from_u64(0, self.signal_widths[cg.signal_id])
                } else {
                    Value::from_u64(1, self.signal_widths[cg.signal_id])
                };
                self.signal_table[cg.signal_id] = new_val;
                if !self.dirty_signals[cg.signal_id] {
                    self.dirty_signals[cg.signal_id] = true;
                    self.dirty_list.push(cg.signal_id);
                }
                self.dirty_any = true;
                self.table_modified = true;
                cg.next_toggle_time += cg.half_period;
            }
        }
    }

    fn classify_always_blocks(&mut self) {
        let blocks = self.module.always_blocks.clone();
        let mut remaining = Vec::new();
        for (_idx, ab) in blocks.iter().enumerate() {
            // Check for edge-sensitive: always_ff @(posedge ...) or always @(posedge ...)
            if let Some((sens, body)) = self.extract_sensitivity(&ab.stmt) {
                if !sens.is_empty() {
                    let resolved: Vec<SensitivityId> = sens.iter().filter_map(|s| {
                        self.signal_name_to_id.get(&s.signal_name).map(|&id| SensitivityId { signal_id: id, edge: s.edge })
                    }).collect();
                    self.edge_blocks.push(EdgeSensitiveBlock { sensitivities: sens, resolved_sensitivities: resolved, stmt: body, kind: ab.kind });
                    continue;
                }
                // @(*) or @* — combinatorial but NOT always_comb.
                // IEEE 1800: always @* does NOT execute at time 0 unless inputs change.
                // Keep original kind (Always) to distinguish from always_comb.
                remaining.push(AlwaysBlock { kind: ab.kind, stmt: body });
                continue;
            }
            // Check for always #delay body — schedule as repeating process
            if ab.kind == AlwaysKind::Always {
                if let StatementKind::TimingControl { control: TimingControl::Delay(d), stmt: body } = &ab.stmt.kind {
                    // Try to detect clock generator: always #N var = ~var
                    let delay_val = self.eval_expr(d).to_u64().unwrap_or(0);
                    if delay_val > 0 {
                        if let Some(clock_gen) = self.try_extract_clock_gen(body, delay_val) {
                            eprintln!("[OPT] clock generator: signal {} period {} (always #{} pattern)",
                                self.id_to_name[clock_gen.signal_id], delay_val * 2, delay_val);
                            self.clock_generators.push(clock_gen);
                            continue;
                        }
                    }
                    let forever_stmt = Statement::new(
                        StatementKind::Forever { body: Box::new(ab.stmt.clone()) }, ab.stmt.span,
                    );
                    let pid = self.next_pid; self.next_pid += 1;
                    self.event_queue.schedule(0, pid, vec![forever_stmt]);
                    continue;
                }
                // Check for always blocks with internal blocking (delays, events, waits)
                // These must be scheduled as processes, not treated as combinatorial
                if self.stmt_is_blocking(&ab.stmt) {
                    let forever_stmt = Statement::new(
                        StatementKind::Forever { body: Box::new(ab.stmt.clone()) }, ab.stmt.span,
                    );
                    let pid = self.next_pid; self.next_pid += 1;
                    self.event_queue.schedule(0, pid, vec![forever_stmt]);
                    continue;
                }
            }
            remaining.push(ab.clone());
        }
        self.module.always_blocks = remaining;
    }

    /// Build pre-computed combinatorial entries with sensitivity sets.
    /// Called once after classify_always_blocks.
    /// Compile edge blocks to bytecode where possible.
    fn compile_edge_blocks(&mut self) {
        use super::bytecode::BytecodeCompiler;
        let mut compiled = Vec::with_capacity(self.edge_blocks.len());
        let mut bc_count = 0;
        let mut max_regs: u16 = 0;
        for block in &self.edge_blocks {
            let mut compiler = BytecodeCompiler::new(
                &self.signal_name_to_id,
                &self.signal_signed,
                &self.signal_widths,
                &self.module.arrays,
                &self.widths,
            );
            if compiler.compile_stmt(&block.stmt) {
                let cb = compiler.finish();
                if cb.num_regs > max_regs { max_regs = cb.num_regs; }
                bc_count += 1;
                compiled.push(Some(cb));
            } else {
                compiled.push(None);
            }
        }
        self.compiled_edge_blocks = compiled;
        // Pre-allocate register file for the largest compiled block
        self.vm_regs = vec![Value::zero(1); max_regs as usize];
        eprintln!("[OPT] bytecode compiled: {}/{} edge blocks", bc_count, self.edge_blocks.len());
    }

    /// Execute a compiled bytecode block. Returns true if executed successfully.
    #[inline]
    fn exec_bytecode(&mut self, block_idx: usize) -> bool {
        // Get a raw pointer to the instructions to avoid borrow conflict.
        // Safety: exec_insns does not modify compiled_edge_blocks.
        let (insns_ptr, insns_len, num_regs) = match &self.compiled_edge_blocks[block_idx] {
            Some(cb) => (cb.instructions.as_ptr(), cb.instructions.len(), cb.num_regs as usize),
            None => return false,
        };
        if self.vm_regs.len() < num_regs {
            self.vm_regs.resize(num_regs, Value::zero(1));
        }
        let insns = unsafe { std::slice::from_raw_parts(insns_ptr, insns_len) };
        self.exec_insns(insns);
        true
    }

    /// Core bytecode VM loop.
    #[inline]
    fn exec_insns(&mut self, insns: &[super::bytecode::Insn]) {
        use super::bytecode::Insn;
        let mut pc: usize = 0;
        let len = insns.len();
        while pc < len {
            match &insns[pc] {
                Insn::LoadConst(dest, val) => {
                    self.vm_regs[*dest as usize] = val.clone();
                }
                Insn::LoadSignal(dest, sig_id) => {
                    self.vm_regs[*dest as usize] = self.signal_table[*sig_id].clone();
                }
                Insn::LoadSignalSigned(dest, sig_id) => {
                    let mut v = self.signal_table[*sig_id].clone();
                    v.is_signed = true;
                    self.vm_regs[*dest as usize] = v;
                }
                Insn::Resize(reg, width) => {
                    let r = *reg as usize;
                    if self.vm_regs[r].width != *width {
                        let resized = self.vm_regs[r].resize(*width);
                        self.vm_regs[r] = resized;
                    }
                }
                Insn::Add(d, l, r) => { self.vm_regs[*d as usize] = self.vm_regs[*l as usize].add(&self.vm_regs[*r as usize]); }
                Insn::Sub(d, l, r) => { self.vm_regs[*d as usize] = self.vm_regs[*l as usize].sub(&self.vm_regs[*r as usize]); }
                Insn::Mul(d, l, r) => { self.vm_regs[*d as usize] = self.vm_regs[*l as usize].mul(&self.vm_regs[*r as usize]); }
                Insn::Div(d, l, r) => { self.vm_regs[*d as usize] = self.vm_regs[*l as usize].div(&self.vm_regs[*r as usize]); }
                Insn::Mod(d, l, r) => { self.vm_regs[*d as usize] = self.vm_regs[*l as usize].modulo(&self.vm_regs[*r as usize]); }
                Insn::BitAnd(d, l, r) => { self.vm_regs[*d as usize] = self.vm_regs[*l as usize].bitwise_and(&self.vm_regs[*r as usize]); }
                Insn::BitOr(d, l, r) => { self.vm_regs[*d as usize] = self.vm_regs[*l as usize].bitwise_or(&self.vm_regs[*r as usize]); }
                Insn::BitXor(d, l, r) => { self.vm_regs[*d as usize] = self.vm_regs[*l as usize].bitwise_xor(&self.vm_regs[*r as usize]); }
                Insn::BitXnor(d, l, r) => { self.vm_regs[*d as usize] = self.vm_regs[*l as usize].bitwise_xor(&self.vm_regs[*r as usize]).bitwise_not(); }
                Insn::LogAnd(d, l, r) => { self.vm_regs[*d as usize] = self.vm_regs[*l as usize].logic_and(&self.vm_regs[*r as usize]); }
                Insn::LogOr(d, l, r) => { self.vm_regs[*d as usize] = self.vm_regs[*l as usize].logic_or(&self.vm_regs[*r as usize]); }
                Insn::Eq(d, l, r) => { self.vm_regs[*d as usize] = self.vm_regs[*l as usize].is_equal(&self.vm_regs[*r as usize]); }
                Insn::Neq(d, l, r) => { self.vm_regs[*d as usize] = self.vm_regs[*l as usize].is_not_equal(&self.vm_regs[*r as usize]); }
                Insn::CaseEq(d, l, r) => { self.vm_regs[*d as usize] = self.vm_regs[*l as usize].case_eq(&self.vm_regs[*r as usize]); }
                Insn::Lt(d, l, r) => { self.vm_regs[*d as usize] = self.vm_regs[*l as usize].less_than(&self.vm_regs[*r as usize]); }
                Insn::Leq(d, l, r) => { self.vm_regs[*d as usize] = self.vm_regs[*l as usize].less_equal(&self.vm_regs[*r as usize]); }
                Insn::Gt(d, l, r) => { self.vm_regs[*d as usize] = self.vm_regs[*l as usize].greater_than(&self.vm_regs[*r as usize]); }
                Insn::Geq(d, l, r) => { self.vm_regs[*d as usize] = self.vm_regs[*l as usize].greater_equal(&self.vm_regs[*r as usize]); }
                Insn::Shl(d, l, r) => { self.vm_regs[*d as usize] = self.vm_regs[*l as usize].shift_left(&self.vm_regs[*r as usize]); }
                Insn::Shr(d, l, r) => { self.vm_regs[*d as usize] = self.vm_regs[*l as usize].shift_right(&self.vm_regs[*r as usize]); }
                Insn::AShr(d, l, r) => { self.vm_regs[*d as usize] = self.vm_regs[*l as usize].arith_shift_right(&self.vm_regs[*r as usize]); }
                Insn::BitNot(d, s) => { self.vm_regs[*d as usize] = self.vm_regs[*s as usize].bitwise_not(); }
                Insn::LogNot(d, s) => { self.vm_regs[*d as usize] = self.vm_regs[*s as usize].logic_not(); }
                Insn::Negate(d, s) => {
                    let w = self.vm_regs[*s as usize].width;
                    let mut r = Value::zero(w).sub(&self.vm_regs[*s as usize]).resize(w);
                    r.is_signed = true;
                    self.vm_regs[*d as usize] = r;
                }
                Insn::ReduceAnd(d, s) => { self.vm_regs[*d as usize] = self.vm_regs[*s as usize].reduce_and(); }
                Insn::ReduceOr(d, s) => { self.vm_regs[*d as usize] = self.vm_regs[*s as usize].reduce_or(); }
                Insn::ReduceXor(d, s) => { self.vm_regs[*d as usize] = self.vm_regs[*s as usize].reduce_xor(); }
                Insn::BitSelect(d, base, idx) => {
                    let i = self.vm_regs[*idx as usize].to_u64().unwrap_or(0) as usize;
                    self.vm_regs[*d as usize] = self.vm_regs[*base as usize].bit_select(i);
                }
                Insn::RangeSelect(d, base, l, r) => {
                    let li = self.vm_regs[*l as usize].to_u64().unwrap_or(0) as usize;
                    let ri = self.vm_regs[*r as usize].to_u64().unwrap_or(0) as usize;
                    self.vm_regs[*d as usize] = self.vm_regs[*base as usize].range_select(li, ri);
                }
                Insn::Concat(d, part_regs) => {
                    let parts: Vec<Value> = part_regs.iter()
                        .map(|r| self.vm_regs[*r as usize].clone())
                        .collect();
                    self.vm_regs[*d as usize] = Value::concat(&parts);
                }
                Insn::BranchIfFalse(reg, target) => {
                    if !self.vm_regs[*reg as usize].is_true() {
                        pc = *target as usize;
                        continue;
                    }
                }
                Insn::Jump(target) => {
                    pc = *target as usize;
                    continue;
                }
                Insn::NbaAssign(sig_id, val_reg, width) => {
                    let val = self.vm_regs[*val_reg as usize].resize(*width);
                    self.nba_fast.push(NbaFast { signal_id: *sig_id, value: val });
                }
                Insn::BlockingAssign(sig_id, val_reg, width) => {
                    let val = self.vm_regs[*val_reg as usize].resize(*width);
                    if self.signal_table[*sig_id] != val {
                        if !self.dirty_signals[*sig_id] {
                            self.dirty_signals[*sig_id] = true;
                            self.dirty_list.push(*sig_id);
                        }
                        self.dirty_any = true;
                        self.signal_table[*sig_id] = val;
                        self.table_modified = true;
                    }
                }
                Insn::LoadArrayElem(dest, array_name, idx_reg) => {
                    let idx = self.vm_regs[*idx_reg as usize].to_u64().unwrap_or(0);
                    let elem_name = format!("{}[{}]", array_name, idx);
                    if let Some(&eid) = self.signal_name_to_id.get(&elem_name) {
                        self.vm_regs[*dest as usize] = self.signal_table[eid].clone();
                    } else {
                        self.vm_regs[*dest as usize] = Value::new(1);
                    }
                }
                Insn::NbaAssignArray(array_name, idx_reg, val_reg, width) => {
                    let idx = self.vm_regs[*idx_reg as usize].to_u64().unwrap_or(0);
                    let elem_name = format!("{}[{}]", array_name, idx);
                    if let Some(&eid) = self.signal_name_to_id.get(&elem_name) {
                        let val = self.vm_regs[*val_reg as usize].resize(*width);
                        self.nba_fast.push(NbaFast { signal_id: eid, value: val });
                    }
                }
                Insn::Move(d, s) => {
                    self.vm_regs[*d as usize] = self.vm_regs[*s as usize].clone();
                }
                Insn::Nop => {}
            }
            pc += 1;
        }
    }

    fn build_comb_entries(&mut self) {
        let mut entries = Vec::new();

        // Continuous assigns
        for ca in &self.module.continuous_assigns {
            let mut reads = HashSet::new();
            let mut writes = HashSet::new();
            Self::collect_expr_reads(&ca.rhs, &self.module, &mut reads);
            Self::collect_lhs_writes(&ca.lhs, &self.module, &mut writes);
            let wids: Vec<(usize, String)> = writes.iter()
                .filter_map(|w| self.signal_name_to_id.get(w).map(|&id| (id, w.clone())))
                .collect();
            let rids: Vec<usize> = reads.iter()
                .filter_map(|r| self.signal_name_to_id.get(r).copied())
                .collect();

            // Detect identity assigns: assign dst = src (simple signal-to-signal copy)
            let direct_copy = if let (ExprKind::Ident(lhs_hier), ExprKind::Ident(rhs_hier)) = (&ca.lhs.kind, &ca.rhs.kind) {
                let dst_name = Self::resolve_hier_name_static(lhs_hier, &self.module);
                let src_name = Self::resolve_hier_name_static(rhs_hier, &self.module);
                if let (Some(&dst_id), Some(&src_id)) = (self.signal_name_to_id.get(&dst_name), self.signal_name_to_id.get(&src_name)) {
                    let width = self.signal_widths[dst_id];
                    if width == self.signal_widths[src_id] {
                        Some(CombItem::DirectCopy { dst_id, src_id, width })
                    } else { None }
                } else { None }
            } else { None };

            let item = direct_copy.unwrap_or_else(|| CombItem::ContAssign { lhs: ca.lhs.clone(), rhs: ca.rhs.clone() });
            entries.push(CombEntry {
                item,
                read_signals: reads,
                write_signals: writes,
                read_signal_ids: rids,
                write_signal_ids: wids,
            });
        }

        // Always @* and always_comb blocks
        for ab in &self.module.always_blocks {
            if matches!(ab.kind, AlwaysKind::AlwaysComb | AlwaysKind::Always) {
                let is_always_comb = ab.kind == AlwaysKind::AlwaysComb;
                let mut reads = HashSet::new();
                let mut writes = HashSet::new();
                Self::collect_stmt_reads(&ab.stmt, &self.module, &mut reads, &mut writes);
                let wids: Vec<(usize, String)> = writes.iter()
                    .filter_map(|w| self.signal_name_to_id.get(w).map(|&id| (id, w.clone())))
                    .collect();
                let rids: Vec<usize> = reads.iter()
                    .filter_map(|r| self.signal_name_to_id.get(r).copied())
                    .collect();
                entries.push(CombEntry {
                    item: CombItem::AlwaysBlock { stmt: ab.stmt.clone(), is_always_comb },
                    read_signals: reads,
                    write_signals: writes,
                    read_signal_ids: rids,
                    write_signal_ids: wids,
                });
            }
        }


        // Build reverse dependency index by signal ID
        let num_signals = self.signal_table.len();
        let mut dep_by_id: Vec<Vec<usize>> = vec![Vec::new(); num_signals];
        for (idx, entry) in entries.iter().enumerate() {
            for &sig_id in &entry.read_signal_ids {
                if sig_id < num_signals {
                    dep_by_id[sig_id].push(idx);
                }
            }
        }
        self.comb_dep_by_id = dep_by_id;
        let dc_count = entries.iter().filter(|e| matches!(&e.item, CombItem::DirectCopy { .. })).count();
        let ca_count = entries.iter().filter(|e| matches!(&e.item, CombItem::ContAssign { .. })).count();
        let ab_count = entries.iter().filter(|e| matches!(&e.item, CombItem::AlwaysBlock { .. })).count();
        if dc_count > 0 {
            eprintln!("[OPT] comb entries: {} direct-copy, {} cont-assign, {} always-block", dc_count, ca_count, ab_count);
            eprintln!("[OPT] edge blocks: {}, event_waiters: {}", self.edge_blocks.len(), self.event_waiters.len());
        }
        self.comb_entries = entries;
    }

    /// Collect all signal names read by an expression.
    fn collect_expr_reads(expr: &Expression, module: &ElaboratedModule, reads: &mut HashSet<String>) {
        match &expr.kind {
            ExprKind::Ident(hier) => {
                let name = Self::resolve_hier_name_static(hier, module);
                reads.insert(name);
            }
            ExprKind::Index { expr: base, index } => {
                // For array[idx]: conservatively add all array elements
                if let ExprKind::Ident(hier) = &base.kind {
                    let name = Self::resolve_hier_name_static(hier, module);
                    if let Some((lo, hi, _)) = module.arrays.get(&name) {
                        for i in *lo..=*hi { reads.insert(format!("{}[{}]", name, i)); }
                    } else {
                        reads.insert(name);
                    }
                }
                Self::collect_expr_reads(index, module, reads);
            }
            ExprKind::RangeSelect { expr: base, left, right, .. } => {
                Self::collect_expr_reads(base, module, reads);
                Self::collect_expr_reads(left, module, reads);
                Self::collect_expr_reads(right, module, reads);
            }
            ExprKind::Unary { operand, .. } => Self::collect_expr_reads(operand, module, reads),
            ExprKind::Binary { left, right, .. } => {
                Self::collect_expr_reads(left, module, reads);
                Self::collect_expr_reads(right, module, reads);
            }
            ExprKind::Conditional { condition, then_expr, else_expr } => {
                Self::collect_expr_reads(condition, module, reads);
                Self::collect_expr_reads(then_expr, module, reads);
                Self::collect_expr_reads(else_expr, module, reads);
            }
            ExprKind::Concatenation(exprs) | ExprKind::AssignmentPattern(exprs) => {
                for e in exprs { Self::collect_expr_reads(e, module, reads); }
            }
            ExprKind::Replication { count, exprs } => {
                Self::collect_expr_reads(count, module, reads);
                for e in exprs { Self::collect_expr_reads(e, module, reads); }
            }
            ExprKind::Call { func, args } => {
                Self::collect_expr_reads(func, module, reads);
                for a in args { Self::collect_expr_reads(a, module, reads); }
            }
            ExprKind::SystemCall { args, .. } => {
                for a in args { Self::collect_expr_reads(a, module, reads); }
            }
            ExprKind::Paren(e) => Self::collect_expr_reads(e, module, reads),
            ExprKind::MemberAccess { expr: e, .. } => Self::collect_expr_reads(e, module, reads),
            _ => {} // Number, StringLiteral, Dollar, Null, etc.
        }
    }

    /// Collect signal names written by an LHS expression.
    fn collect_lhs_writes(lhs: &Expression, module: &ElaboratedModule, writes: &mut HashSet<String>) {
        match &lhs.kind {
            ExprKind::Ident(hier) => { writes.insert(Self::resolve_hier_name_static(hier, module)); }
            ExprKind::Index { expr: base, .. } => {
                if let ExprKind::Ident(hier) = &base.kind {
                    let name = Self::resolve_hier_name_static(hier, module);
                    if let Some((lo, hi, _)) = module.arrays.get(&name) {
                        for i in *lo..=*hi { writes.insert(format!("{}[{}]", name, i)); }
                    } else { writes.insert(name); }
                }
            }
            ExprKind::RangeSelect { expr: base, .. } => Self::collect_lhs_writes(base, module, writes),
            ExprKind::Concatenation(exprs) => { for e in exprs { Self::collect_lhs_writes(e, module, writes); } }
            _ => {}
        }
    }

    /// Collect reads/writes from a statement (for always @* / always_comb blocks).
    fn collect_stmt_reads(stmt: &Statement, module: &ElaboratedModule, reads: &mut HashSet<String>, writes: &mut HashSet<String>) {
        match &stmt.kind {
            StatementKind::BlockingAssign { lvalue, rvalue } | StatementKind::NonblockingAssign { lvalue, rvalue, .. } => {
                Self::collect_expr_reads(rvalue, module, reads);
                Self::collect_lhs_writes(lvalue, module, writes);
                // Also read the index expression of the LHS if it's an array/range select
                Self::collect_lhs_index_reads(lvalue, module, reads);
            }
            StatementKind::If { condition, then_stmt, else_stmt, .. } => {
                Self::collect_expr_reads(condition, module, reads);
                Self::collect_stmt_reads(then_stmt, module, reads, writes);
                if let Some(el) = else_stmt { Self::collect_stmt_reads(el, module, reads, writes); }
            }
            StatementKind::Case { expr, items, .. } => {
                Self::collect_expr_reads(expr, module, reads);
                for item in items {
                    for pat in &item.patterns { Self::collect_expr_reads(pat, module, reads); }
                    Self::collect_stmt_reads(&item.stmt, module, reads, writes);
                }
            }
            StatementKind::For { init, condition, step, body } => {
                for fi in init {
                    match fi {
                        ForInit::Assign { rvalue, .. } | ForInit::VarDecl { init: rvalue, .. } => {
                            Self::collect_expr_reads(rvalue, module, reads);
                        }
                    }
                }
                if let Some(c) = condition { Self::collect_expr_reads(c, module, reads); }
                for s in step { Self::collect_expr_reads(s, module, reads); }
                Self::collect_stmt_reads(body, module, reads, writes);
            }
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                for s in stmts { Self::collect_stmt_reads(s, module, reads, writes); }
            }
            StatementKind::Expr(e) => { Self::collect_expr_reads(e, module, reads); }
            StatementKind::While { condition, body } | StatementKind::DoWhile { body, condition } => {
                Self::collect_expr_reads(condition, module, reads);
                Self::collect_stmt_reads(body, module, reads, writes);
            }
            StatementKind::Forever { body } | StatementKind::Repeat { body, .. } | StatementKind::Foreach { body, .. } => {
                Self::collect_stmt_reads(body, module, reads, writes);
            }
            _ => {}
        }
    }

    /// Collect reads from index expressions on the LHS (e.g., array[idx] — idx is read).
    fn collect_lhs_index_reads(lhs: &Expression, module: &ElaboratedModule, reads: &mut HashSet<String>) {
        match &lhs.kind {
            ExprKind::Index { index, .. } => { Self::collect_expr_reads(index, module, reads); }
            ExprKind::RangeSelect { left, right, .. } => {
                Self::collect_expr_reads(left, module, reads);
                Self::collect_expr_reads(right, module, reads);
            }
            _ => {}
        }
    }

    /// Static version of resolve_hier_name (doesn't need &self).
    fn resolve_hier_name_static(hier: &HierarchicalIdentifier, module: &ElaboratedModule) -> String {
        let raw = hier.path.iter().map(|s| s.name.name.as_str()).collect::<Vec<_>>().join(".");
        // Check if signal exists; if not and has a prefix, try alternatives
        if module.signals.contains_key(&raw) { return raw.to_string(); }
        raw.to_string()
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

    /// Create an EventWaiter with pre-resolved sensitivity IDs for O(1) edge checking.
    fn make_event_waiter(&self, pid: usize, sens: Vec<Sensitivity>, continuation: Vec<Statement>) -> EventWaiter {
        let resolved = sens.iter().filter_map(|s| {
            self.signal_name_to_id.get(&s.signal_name).map(|&id| SensitivityId { signal_id: id, edge: s.edge })
        }).collect();
        EventWaiter { pid, sensitivities: sens, resolved_sensitivities: resolved, continuation, registered_time: self.time }
    }

    fn event_loop(&mut self) {
        let sim_start = std::time::Instant::now();
        let mut iters: u64 = 0;
        let max_iters = self.max_time * 1000;
        let mut t_settle: u64 = 0;
        let mut t_edges: u64 = 0;
        let mut t_nba: u64 = 0;
        let mut t_process: u64 = 0;
        let mut t_snap: u64 = 0;
        let mut t_sched: u64 = 0;
        while !self.finished && iters < max_iters {
            iters += 1;

            let has_timed = !self.event_queue.is_empty();
            let has_waiters = !self.event_waiters.is_empty();
            let has_clocks = !self.clock_generators.is_empty();

            if !has_timed && !has_waiters && !has_clocks { break; }

            // Determine next time: minimum of event queue and clock generators
            let next_eq_time = self.event_queue.next_time();
            let next_clk_time = if has_clocks {
                self.clock_generators.iter().map(|c| c.next_toggle_time).min()
            } else { None };
            let next_time = match (next_eq_time, next_clk_time) {
                (Some(a), Some(b)) => a.min(b),
                (Some(a), None) => a,
                (None, Some(b)) => b,
                (None, None) => {
                    if has_waiters { self.time } else { break; }
                }
            };

            if next_time > self.max_time { break; }
            if next_time > self.time { self.time = next_time; }

            {
                let _t = std::time::Instant::now();
                self.snapshot_edge_signals();
                t_snap += _t.elapsed().as_nanos() as u64;

                // Fire clock generators at current time (O(1) toggle, no AST)
                self.fire_clock_generators();

                // Process timed events from queue
                let _t = std::time::Instant::now();
                let processes = self.event_queue.remove(self.time);
                t_sched += _t.elapsed().as_nanos() as u64;
                let _t = std::time::Instant::now();
                for (pid, stmts) in processes {
                    if self.finished { break; }
                    self.run_process_stmts(pid, &stmts);
                }
                t_process += _t.elapsed().as_nanos() as u64;

                // First NBA + settle pass
                let _t = std::time::Instant::now();
                if !self.nba_fast.is_empty() || !self.nba_queue.is_empty() {
                    self.apply_nba();
                }
                t_nba += _t.elapsed().as_nanos() as u64;
                let _t = std::time::Instant::now();
                if self.dirty_any {
                    self.settle_combinatorial();
                }
                t_settle += _t.elapsed().as_nanos() as u64;
                let _t = std::time::Instant::now();
                self.check_edges();
                t_edges += _t.elapsed().as_nanos() as u64;
                // Second NBA + settle pass (from edge-triggered blocks)
                if !self.nba_fast.is_empty() || !self.nba_queue.is_empty() {
                    let _t2 = std::time::Instant::now();
                    self.apply_nba();
                    t_nba += _t2.elapsed().as_nanos() as u64;
                    if self.dirty_any {
                        let _t2 = std::time::Instant::now();
                        self.settle_combinatorial();
                        t_settle += _t2.elapsed().as_nanos() as u64;
                    }
                }
                let _t = std::time::Instant::now();
                self.snapshot_edge_signals();
                t_snap += _t.elapsed().as_nanos() as u64;

                self.check_monitor();
                self.vcd_write_changes();
                self.loop_iters += 1;
            }
        }
        let sim_elapsed = sim_start.elapsed();
        eprintln!("[PROF] settle={:.1}ms edges={:.1}ms nba={:.1}ms process={:.1}ms snap={:.1}ms sched={:.1}ms",
            t_settle as f64/1e6, t_edges as f64/1e6, t_nba as f64/1e6,
            t_process as f64/1e6, t_snap as f64/1e6, t_sched as f64/1e6);
        eprintln!("[PHASE] simulate: {:.1}ms ({} iters, {:.2}µs/iter)",
            sim_elapsed.as_secs_f64() * 1000.0, self.loop_iters,
            sim_elapsed.as_secs_f64() * 1e6 / self.loop_iters.max(1) as f64);

        // Activity monitor report
        if self.activity_mon {
            self.print_activity_report();
        }
    }

    fn print_activity_report(&self) {
        eprintln!();
        eprintln!("╔══════════════════════════════════════════════════════════════╗");
        eprintln!("║                    ACTIVITY MONITOR                         ║");
        eprintln!("╚══════════════════════════════════════════════════════════════╝");

        // Helper: check if a signal name is a clock signal
        let is_clock = |name: &str| -> bool {
            let lower = name.to_ascii_lowercase();
            let leaf = lower.rsplit('.').next().unwrap_or(&lower);
            leaf == "clk" || leaf == "clock" || leaf.ends_with(".clk") || leaf.ends_with(".clock")
        };

        // Helper: extract block prefix from a signal name.
        // "uut._dff_0_.CLK" → "uut"  (strip gate-level instance)
        // "uut._n879_" → "uut"
        // "uut.sub.sig" → "uut.sub"
        // "clk" → "(top)"
        // Gate-level instances typically have names like _dff_N_, _mux2_N_, _inv_N_, etc.
        let is_gate_instance = |seg: &str| -> bool {
            (seg.starts_with('_') && seg.ends_with('_') && seg.len() > 2)
                || seg.starts_with("sky130_")
        };
        let block_prefix = |name: &str| -> String {
            let parts: Vec<&str> = name.split('.').collect();
            if parts.len() <= 1 { return "(top)".to_string(); }
            // parts: ["uut", "_dff_0_", "CLK"] → block = "uut"
            // parts: ["uut", "_n879_"] → block = "uut"
            // parts: ["uut", "sub", "_mux2_1_", "X"] → block = "uut.sub"
            let mut end = parts.len() - 1; // skip leaf (signal name / port)
            // If the second-to-last segment looks like a gate instance, skip it too
            if end >= 1 && is_gate_instance(parts[end - 1]) {
                end -= 1;
            }
            if end == 0 { return parts[0].to_string(); }
            parts[..end].join(".")
        };

        // Aggregate comb entry triggers by block
        if !self.activity_counts.is_empty() {
            let mut block_triggers: HashMap<String, u64> = HashMap::new();
            let mut block_entry_count: HashMap<String, usize> = HashMap::new();
            for (eidx, &count) in self.activity_counts.iter().enumerate() {
                if count == 0 { continue; }
                let entry = &self.comb_entries[eidx];
                // Get destination signal name to determine block
                let dst_name = match &entry.item {
                    CombItem::DirectCopy { dst_id, .. } => &self.id_to_name[*dst_id],
                    CombItem::ContAssign { lhs, .. } => {
                        // Use first write signal
                        if let Some((id, _)) = entry.write_signal_ids.first() {
                            &self.id_to_name[*id]
                        } else { continue; }
                    }
                    CombItem::AlwaysBlock { .. } => {
                        if let Some((id, _)) = entry.write_signal_ids.first() {
                            &self.id_to_name[*id]
                        } else { continue; }
                    }
                };
                // Skip clock signals
                if is_clock(dst_name) { continue; }
                let block = block_prefix(dst_name);
                *block_triggers.entry(block.clone()).or_insert(0) += count;
                *block_entry_count.entry(block).or_insert(0) += 1;
            }

            let mut sorted: Vec<_> = block_triggers.into_iter().collect();
            sorted.sort_by(|a, b| b.1.cmp(&a.1));
            sorted.truncate(10);

            eprintln!();
            eprintln!("  Top 10 most active blocks (comb triggers, excl. clocks):");
            eprintln!("  {:>10}  {:>6}  {}", "triggers", "entries", "block");
            eprintln!("  {:>10}  {:>6}  {}", "----------", "------", "-----");
            for (block, count) in &sorted {
                let entries = block_entry_count.get(block.as_str()).copied().unwrap_or(0);
                eprintln!("  {:>10}  {:>6}  {}", count, entries, block);
            }
        }

        // Aggregate signal toggles by block, exclude clocks
        {
            let mut block_toggles: HashMap<String, u64> = HashMap::new();
            let mut block_sig_count: HashMap<String, usize> = HashMap::new();
            let mut block_top_signal: HashMap<String, (String, u64)> = HashMap::new();
            for (id, &count) in self.signal_toggle_counts.iter().enumerate() {
                if count == 0 { continue; }
                let name = &self.id_to_name[id];
                if is_clock(name) { continue; }
                let block = block_prefix(name);
                *block_toggles.entry(block.clone()).or_insert(0) += count;
                *block_sig_count.entry(block.clone()).or_insert(0) += 1;
                let top = block_top_signal.entry(block).or_insert((name.clone(), 0));
                if count > top.1 { *top = (name.clone(), count); }
            }

            let mut sorted: Vec<_> = block_toggles.into_iter().collect();
            sorted.sort_by(|a, b| b.1.cmp(&a.1));
            sorted.truncate(10);

            eprintln!();
            eprintln!("  Top 10 most toggling blocks (signal changes, excl. clocks):");
            eprintln!("  {:>10}  {:>5}  {:40}  {}", "toggles", "sigs", "block", "hottest signal");
            eprintln!("  {:>10}  {:>5}  {:40}  {}", "----------", "-----", "-----", "--------------");
            for (block, count) in &sorted {
                let sigs = block_sig_count.get(block.as_str()).copied().unwrap_or(0);
                let (hot_name, hot_count) = block_top_signal.get(block.as_str())
                    .map(|(n, c)| (n.as_str(), *c)).unwrap_or(("?", 0));
                let hot_short = hot_name.strip_prefix(block.as_str())
                    .and_then(|s| s.strip_prefix('.'))
                    .unwrap_or(hot_name);
                eprintln!("  {:>10}  {:>5}  {:40}  {} ({})",
                    count, sigs, block, hot_short, hot_count);
            }
        }
        eprintln!();
    }

    /// Extract signal name from an expression (for display).
    fn expr_signal_name(&self, expr: &Expression) -> String {
        match &expr.kind {
            ExprKind::Ident(hier) => {
                hier.path.iter().map(|s| s.name.name.as_str()).collect::<Vec<_>>().join(".")
            }
            ExprKind::Index { expr, index } => {
                format!("{}[{}]", self.expr_signal_name(expr), self.expr_signal_name(index))
            }
            _ => "?".to_string(),
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
                        self.event_queue.schedule(self.time + delay, pid, cont);
                        return;
                    }
                    TimingControl::Event(event) => {
                        // Suspend process until the event fires
                        let sens = self.event_to_sens(event);
                        if !sens.is_empty() {
                            let mut cont = vec![*body.clone()];
                            cont.extend_from_slice(&stmts[i+1..]);
                            self.event_waiters.push(self.make_event_waiter(pid, sens, cont));
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
                        self.event_waiters.push(self.make_event_waiter(pid, sens, cont));
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
                                        cached_val: Cell::new(None),
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
                        self.event_queue.schedule(self.time + delay, pid, cont);
                        return;
                    }
                    TimingControl::Event(event) => {
                        let sens = self.event_to_sens(event);
                        if !sens.is_empty() {
                            let mut cont = vec![*tbody.clone()];
                            cont.extend_from_slice(&body_stmts[i+1..]);
                            cont.push(Statement::new(StatementKind::Forever { body: Box::new(body.clone()) }, body.span));
                            cont.extend_from_slice(after);
                            self.event_waiters.push(self.make_event_waiter(pid, sens, cont));
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
    fn resolve_nba_target(&self, lhs: &Expression) -> Option<usize> {
        match &lhs.kind {
            ExprKind::Ident(hier) => {
                // Use cached signal ID if available
                if let Some(id) = hier.cached_signal_id.get() {
                    return Some(id);
                }
                let name = self.resolve_hier_name(hier);
                self.signal_name_to_id.get(&name).copied()
            }
            ExprKind::Index { expr, index } => {
                if let ExprKind::Ident(hier) = &expr.kind {
                    let name = self.resolve_hier_name(hier);
                    if self.module.arrays.contains_key(&name) {
                        let idx = self.eval_expr(index).to_u64().unwrap_or(0);
                        // Use a small buffer to avoid allocation for common array names
                        let elem = format!("{}[{}]", name, idx);
                        return self.signal_name_to_id.get(&elem).copied();
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn apply_nba(&mut self) {
        for i in 0..self.nba_fast.len() {
            let id = self.nba_fast[i].signal_id;
            let width = self.signal_widths[id];
            if self.nba_fast[i].value.width != width {
                self.nba_fast[i].value = self.nba_fast[i].value.resize(width);
            }
            if self.signal_table[id] != self.nba_fast[i].value {
                if !self.dirty_signals[id] { self.dirty_signals[id] = true; self.dirty_list.push(id); }
                self.dirty_any = true;
                let mut val = Value::zero(1);
                std::mem::swap(&mut val, &mut self.nba_fast[i].value);
                self.signal_table[id] = val;
                self.table_modified = true;
            }
        }
        self.nba_fast.clear();
        for i in 0..self.nba_queue.len() {
            if let Some(ref lhs) = self.nba_queue[i].lhs {
                let lhs = lhs.clone();
                let val = self.nba_queue[i].value.clone();
                self.assign_value(&lhs, &val);
            }
        }
        self.nba_queue.clear();
    }

    /// Snapshot only edge-sensitive signals + event_waiter signals into prev_signals.
    fn snapshot_edge_signals(&mut self) {
        for &id in &self.edge_signal_ids {
            self.prev_table[id].copy_from(&self.signal_table[id]);
        }
        for i in 0..self.event_waiters.len() {
            for j in 0..self.event_waiters[i].resolved_sensitivities.len() {
                let sid = self.event_waiters[i].resolved_sensitivities[j].signal_id;
                self.prev_table[sid].copy_from(&self.signal_table[sid]);
            }
        }
    }

    /// Check edge: compare signal_table[id] vs prev_table[id]
    #[inline]
    fn check_edge_id(&self, id: usize, edge: EdgeKind) -> bool {
        let cur = &self.signal_table[id];
        let prev = &self.prev_table[id];
        match edge {
            EdgeKind::Posedge => {
                let cb = cur.bits_first();
                let pb = prev.bits_first();
                pb != LogicBit::One && cb == LogicBit::One
            }
            EdgeKind::Negedge => {
                let cb = cur.bits_first();
                let pb = prev.bits_first();
                pb != LogicBit::Zero && cb == LogicBit::Zero
            }
            EdgeKind::AnyEdge => *cur != *prev,
        }
    }

    fn check_edges(&mut self) {
        let blocks = std::mem::take(&mut self.edge_blocks);
        self.in_edge_block = true;
        for (block_idx, block) in blocks.iter().enumerate() {
            let mut trigger = false;
            for sid in &block.resolved_sensitivities {
                trigger = self.check_edge_id(sid.signal_id, sid.edge);
                if trigger { break; }
            }
            if trigger {
                // Try bytecode VM first (flat instruction array, cache-friendly)
                if !self.exec_bytecode(block_idx) {
                    // Fallback: AST interpreter
                    self.exec_statement(&block.stmt);
                }
            }
        }

        // Wake up event_waiters whose sensitivity conditions are met
        let waiters = std::mem::take(&mut self.event_waiters);
        self.event_waiters_swap.clear();
        for waiter in waiters {
            if waiter.registered_time == self.time {
                self.event_waiters_swap.push(waiter);
                continue;
            }
            let mut triggered = false;
            for sid in &waiter.resolved_sensitivities {
                triggered = self.check_edge_id(sid.signal_id, sid.edge);
                if triggered { break; }
            }
            if triggered {
                self.event_queue.schedule(self.time, waiter.pid, waiter.continuation);
            } else {
                self.event_waiters_swap.push(waiter);
            }
        }
        std::mem::swap(&mut self.event_waiters, &mut self.event_waiters_swap);
        self.edge_blocks = blocks;
        self.in_edge_block = false;
    }

    fn settle_combinatorial(&mut self) {
        if self.settling { return; }
        if !self.dirty_any { return; }
        self.settling = true;
        self.settle_calls += 1;

        let entries = std::mem::take(&mut self.comb_entries);
        let dep_by_id = std::mem::take(&mut self.comb_dep_by_id);
        let num_entries = entries.len();

        // Resize persistent buffers if needed (only happens once)
        if self.settle_triggered.len() < num_entries {
            self.settle_triggered.resize(num_entries, false);
        }

        let mut total_iters = 0u64;
        let limit = self.settle_limit as u64;
        for iteration in 0..limit {
            if !self.dirty_any && iteration > 0 { break; }
            total_iters += 1;

            // Collect dirty signal IDs from dirty_list (O(num_dirty) instead of O(num_signals))
            self.settle_dirty_ids.clear();
            for &id in &self.dirty_list {
                if self.dirty_signals[id] {
                    self.settle_dirty_ids.push(id);
                    self.dirty_signals[id] = false;
                }
            }
            self.dirty_list.clear();
            self.dirty_any = false;

            // Build triggered set using reverse dependency index
            // Clear only entries that were triggered last iteration
            for &eidx in &self.settle_triggered_list {
                self.settle_triggered[eidx] = false;
            }
            self.settle_triggered_list.clear();
            for &sig_id in &self.settle_dirty_ids {
                if sig_id < dep_by_id.len() {
                    for &eidx in &dep_by_id[sig_id] {
                        if !self.settle_triggered[eidx] {
                            self.settle_triggered[eidx] = true;
                            self.settle_triggered_list.push(eidx);
                        }
                    }
                }
            }

            // Evaluate triggered entries directly from the triggered list.
            // On iteration 0: also include entries with empty read sets and time-0 always_comb.
            if iteration == 0 {
                // First iteration: add special-case entries that fire unconditionally
                for eidx in 0..num_entries {
                    if !self.settle_triggered[eidx] {
                        if entries[eidx].read_signal_ids.is_empty()
                            || (self.time == 0 && matches!(&entries[eidx].item, CombItem::AlwaysBlock { is_always_comb: true, .. }))
                        {
                            self.settle_triggered[eidx] = true;
                            self.settle_triggered_list.push(eidx);
                        }
                    }
                }
            }

            for tidx in 0..self.settle_triggered_list.len() {
                let eidx = self.settle_triggered_list[tidx];

                self.entry_evals += 1;
                if self.activity_mon && !self.activity_counts.is_empty() {
                    self.activity_counts[eidx] += 1;
                }
                match &entries[eidx].item {
                    CombItem::DirectCopy { dst_id, src_id, width } => {
                        let src_val = self.signal_table[*src_id].clone();
                        let resized = if src_val.width != *width { src_val.resize(*width) } else { src_val };
                        if self.signal_table[*dst_id] != resized {
                            self.mark_dirty_id(*dst_id);
                            self.signal_table[*dst_id] = resized;
                            self.table_modified = true;
                        }
                    }
                    CombItem::ContAssign { lhs, rhs } => {
                        let w = self.infer_lhs_width(lhs);
                        let val = self.eval_expr_ctx(rhs, w).resize(w);
                        self.assign_value(lhs, &val);
                    }
                    CombItem::AlwaysBlock { stmt, .. } => {
                        let write_ids = &entries[eidx].write_signal_ids;
                        self.settle_prev_values.clear();
                        for (id, _name) in write_ids {
                            self.settle_prev_values.push((*id, self.signal_table[*id].clone()));
                        }
                        self.exec_statement(stmt);
                        for i in 0..self.settle_prev_values.len() {
                            let (id, ref old_val) = self.settle_prev_values[i];
                            if self.signal_table[id] != *old_val {
                                if !self.dirty_signals[id] { self.dirty_signals[id] = true; self.dirty_list.push(id); }
                                self.dirty_any = true;
                            }
                        }
                    }
                }
            }

            if !self.dirty_any { break; }
        }

        self.comb_entries = entries;
        self.comb_dep_by_id = dep_by_id;
        self.settle_iters += total_iters;
        if total_iters > self.max_settle_iters {
            if total_iters >= limit && self.dirty_any {
                eprintln!("[WARN] settle limit hit ({} iters) at time {} — signals may not have converged. Use --settle-limit to increase.",
                    limit, self.time);
            }
            self.max_settle_iters = total_iters;
        }
        self.settling = false;
    }


    fn assign_value(&mut self, lhs: &Expression, val: &Value) -> bool {
        match &lhs.kind {
            ExprKind::Ident(hier) => {
                let name_ref = hier.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
                if let Some(id) = hier.cached_signal_id.get() {
                    let width = self.signal_widths[id];
                    let mut resized = val.resize(width);
                    resized.is_signed = self.signal_signed[id];
                    let changed = self.signal_table[id] != resized;
                    if changed {
                        self.mark_dirty_id(id);
                        self.signal_table[id] = resized;
                        self.table_modified = true;
                    }
                    return changed;
                }
                if let Some(&id) = self.signal_name_to_id.get(name_ref) {
                    hier.cached_signal_id.set(Some(id));
                    let width = self.signal_widths[id];
                    let mut resized = val.resize(width);
                    resized.is_signed = self.signal_signed[id];
                    let changed = self.signal_table[id] != resized;
                    if changed {
                        self.mark_dirty_id(id);
                        self.signal_table[id] = resized;
                        self.table_modified = true;
                    }
                    return changed;
                }
                // Fallback (slow path): allocate name
                // Fallback (slow path): allocate name, sync HashMap
                self.sync_table_to_hashmap();
                let name = name_ref.to_string();
                let width = self.widths.get(&name).copied().unwrap_or(val.width);
                let mut resized = val.resize(width);
                resized.is_signed = self.signed_signals.contains(&name);
                let changed = self.signals.get(&name).map_or(true, |p| *p != resized);
                if changed { self.mark_dirty(&name); }
                self.signals.insert(name, resized); changed
            }
            ExprKind::Index { expr, index } => {
                if let ExprKind::Ident(hier) = &expr.kind {
                    let name = self.resolve_hier_name(hier);
                    let idx = self.eval_expr(index).to_u64().unwrap_or(0);
                    // Check if this is an array element assignment
                    if self.module.arrays.contains_key(&name) {
                        let elem_name = format!("{}[{}]", name, idx);
                        if let Some(&id) = self.signal_name_to_id.get(&elem_name) {
                            let width = self.signal_widths[id];
                            let resized = val.resize(width);
                            let changed = self.signal_table[id] != resized;
                            if changed {
                                if !self.dirty_signals[id] { self.dirty_signals[id] = true; self.dirty_list.push(id); }
                                self.dirty_any = true;
                                self.signal_table[id] = resized;
                                self.table_modified = true;
                            }
                            return changed;
                        }
                        return false;
                    }
                    // Fall back to bit select assignment
                    let idx = idx as usize;
                    // Bit select needs signal_table
                    if let Some(&id) = self.signal_name_to_id.get(&name) {
                        if idx < self.signal_widths[id] as usize {
                            let nb = val.bits_first();
                            let old = self.signal_table[id].get_bit(idx);
                            let c = old != nb;
                            if c {
                                self.signal_table[id].set_bit(idx, nb);
                                self.table_modified = true;
                                self.mark_dirty(&name);
                            }
                            return c;
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
                    if let Some(&id) = self.signal_name_to_id.get(&name) {
                        let width = self.signal_widths[id] as usize;
                        let mut changed = false;
                        for i in lsb..=msb.min(width.saturating_sub(1)) {
                            let nb = val.get_bit(i - lsb);
                            if self.signal_table[id].get_bit(i) != nb {
                                self.signal_table[id].set_bit(i, nb);
                                changed = true;
                            }
                        }
                        if changed {
                            self.table_modified = true;
                            self.mark_dirty(&name);
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
                    for bit in 0..8 { if (byte >> bit) & 1 == 1 { if i*8+bit < val.width as usize { val.set_bit(i*8+bit, LogicBit::One); } } }
                }
                val
            }
            ExprKind::Ident(hier) => self.fast_signal_read(hier),
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
                let is_arith_or_bitwise = matches!(op, BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul
                    | BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor | BinaryOp::BitXnor);
                // IEEE §11.6.1: For context-determined operations, the width is
                // max(lhs_width, rhs_width, context_width), computed BEFORE evaluation
                // so that sub-expressions are widened to the full expression width.
                let self_det_w = if is_arith_or_bitwise {
                    let lw = self.infer_width(left);
                    let rw = self.infer_width(right);
                    lw.max(rw).max(ctx_width)
                } else { ctx_width };
                let l = self.eval_expr_ctx(left, self_det_w);
                let r = self.eval_expr_ctx(right, self_det_w);
                let max_w = l.width.max(r.width).max(self_det_w);
                let wl = if is_arith_or_bitwise && max_w > l.width { l.resize(max_w) } else { l };
                let wr = if is_arith_or_bitwise && max_w > r.width { r.resize(max_w) } else { r };
                match op {
                    BinaryOp::Add => wl.add(&wr), BinaryOp::Sub => wl.sub(&wr), BinaryOp::Mul => wl.mul(&wr), BinaryOp::Div => wl.div(&wr),
                    BinaryOp::Mod => wl.modulo(&wr), BinaryOp::Power => wl.power(&wr),
                    BinaryOp::BitAnd => wl.bitwise_and(&wr), BinaryOp::BitOr => wl.bitwise_or(&wr),
                    BinaryOp::BitXor => wl.bitwise_xor(&wr), BinaryOp::BitXnor => wl.bitwise_xor(&wr).bitwise_not(),
                    BinaryOp::LogAnd => wl.logic_and(&wr), BinaryOp::LogOr => wl.logic_or(&wr),
                    BinaryOp::Eq => wl.is_equal(&wr), BinaryOp::Neq => wl.is_not_equal(&wr),
                    BinaryOp::CaseEq => wl.case_eq(&wr), BinaryOp::CaseNeq => wl.case_eq(&wr).logic_not(),
                    BinaryOp::Lt => wl.less_than(&wr), BinaryOp::Leq => wl.leq(&wr), BinaryOp::Gt => wl.greater_than(&wr), BinaryOp::Geq => wl.geq(&wr),
                    BinaryOp::ShiftLeft | BinaryOp::ArithShiftLeft => {
                        // Widen left operand to context width before shifting
                        let wide_l = if self_det_w > wl.width { wl.resize(self_det_w) } else { wl };
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
            ExprKind::Concatenation(parts) => { let mut r = Value::zero(0); for p in parts.iter().rev() { r = self.eval_expr(p).concat_with(&r); } r }
            ExprKind::Replication { count, exprs } => {
                let n = self.eval_expr(count).to_u64().unwrap_or(1);
                let mut inner = Value::zero(0); for e in exprs.iter().rev() { inner = self.eval_expr(e).concat_with(&inner); }
                let mut r = Value::zero(0); for _ in 0..n { r = inner.concat_with(&r); } r
            }
            ExprKind::Index { expr, index } => {
                // Check if this is an array element access (memory[idx]) vs bit select
                if let ExprKind::Ident(hier) = &expr.kind {
                    let name = self.resolve_hier_name(hier);
                    if self.module.arrays.contains_key(&name) {
                        // Array element access: look up signal "name[idx]"
                        let idx = self.eval_expr(index).to_u64().unwrap_or(0);
                        let elem_name = format!("{}[{}]", name, idx);
                        if let Some(&eid) = self.signal_name_to_id.get(&elem_name) {
                            let mut v = self.signal_table[eid].clone();
                            if self.signal_signed[eid] { v.is_signed = true; }
                            return v;
                        }
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
            ExprKind::AssignmentPattern(parts) => { let mut r = Value::zero(0); for p in parts.iter().rev() { r = self.eval_expr(p).concat_with(&r); } r }
            _ => Value::zero(32),
        }
    }

    fn eval_number(&self, num: &NumberLiteral) -> Value {
        match num {
            NumberLiteral::Integer { size, signed, base, value, cached_val } => {
                let w = size.unwrap_or(32);
                // Fast path: return cached value (avoids re-parsing string)
                if let Some((vb, xz, cw)) = cached_val.get() {
                    if cw == w {
                        let mut v = Value::from_inline(vb, xz, w);
                        v.is_signed = *signed;
                        return v;
                    }
                }
                let r = match base { NumberBase::Binary => 2, NumberBase::Octal => 8, NumberBase::Hex => 16, NumberBase::Decimal => 10 };
                let mut v = Value::from_str_radix(value, r, w);
                // Cache inline values (width <= 64)
                if w <= 64 {
                    if let Some((vb, xz)) = v.inline_bits() {
                        cached_val.set(Some((vb, xz, w)));
                    }
                }
                v.is_signed = *signed; v
            }
            NumberLiteral::Real(f) => Value::from_u64(*f as u64, 64),
            NumberLiteral::UnbasedUnsized(c) => match c {
                '0' => Value::zero(32),
                '1' => Value::ones(32),
                'x' | 'X' => Value::new(32),  // all X
                'z' | 'Z' => Value::all_z(32),
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
                // Only settle for blocking assigns in combinational blocks
                // Edge-triggered blocks (always @posedge) don't need immediate settle
                if !self.in_edge_block {
                    self.settle_combinatorial();
                }
            }
            StatementKind::NonblockingAssign { lvalue, rvalue, .. } => {
                let w = self.infer_lhs_width(lvalue);
                let val = self.eval_expr_ctx(rvalue, w);
                // Resolve the LHS target NOW to capture array indices at schedule time
                let resolved_id = self.resolve_nba_target(lvalue);
                if let Some(id) = resolved_id {
                    // Fast path: push to compact nba_fast buffer
                    self.nba_fast.push(NbaFast { signal_id: id, value: val.resize(w) });
                } else {
                    // Slow path: unresolved target needs full Expression
                    self.nba_queue.push(NbaEntry { lhs: Some(lvalue.clone()), value: val.resize(w), resolved_id: None });
                }
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
                        self.apply_nba(); self.settle_combinatorial(); self.snapshot_edge_signals();
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
                self.prev_signals = self.signals.clone();  // rare path - full clone OK
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
            self.sync_table_to_hashmap();
            let m = self.format_args(&args, &tn);
            let mut changed = self.monitor_prev.is_empty();
            for (n, v) in &self.signals { if let Some(p) = self.monitor_prev.get(n) { if p != v { changed = true; break; } } }
            if changed { self.output.push(SimOutput { time: self.time, message: m.clone() }); println!("{}", m); self.monitor_prev = self.signals.clone(); }
        }
    }

    fn resolve_hier_name(&self, hier: &HierarchicalIdentifier) -> String {
        if hier.path.len() == 1 {
            // Fast path: single-segment name (the common case after inlining)
            return hier.path[0].name.name.clone();
        }
        // Multi-segment: join with dots (e.g., uut.cpu_state → "uut.cpu_state")
        let raw = hier.path.iter().map(|s| s.name.name.as_str()).collect::<Vec<_>>().join(".");
        // Check if the dotted name exists in the signal table
        if self.signal_name_to_id.contains_key(&raw) {
            return raw;
        }
        // Fallback: try just the last segment (for backwards compatibility)
        hier.path.last().map(|s| s.name.name.clone()).unwrap_or_default()
    }

    /// Fast signal read avoiding String allocation.
    /// Uses cached_signal_id to remember the signal name as &str key for HashMap lookup.
    #[inline]
    #[inline]
    fn fast_signal_read(&self, hier: &HierarchicalIdentifier) -> Value {
        // Try cached signal ID first (O(1) Vec access)
        if let Some(id) = hier.cached_signal_id.get() {
            let mut v = self.signal_table[id].clone();
            if self.signal_signed[id] { v.is_signed = true; }
            return v;
        }
        // First access: resolve name and cache ID
        let name = hier.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
        if let Some(&id) = self.signal_name_to_id.get(name) {
            hier.cached_signal_id.set(Some(id));
            let mut v = self.signal_table[id].clone();
            if self.signal_signed[id] { v.is_signed = true; }
            return v;
        }
        // Fallback
        let mut v = self.signals.get(name).cloned().unwrap_or_else(|| Value::new(1));
        if self.signed_signals.contains(name) { v.is_signed = true; }
        v
    }

    /// Sync a signal from the HashMap to the signal_table (after in-place mutation).
    #[inline]
    fn sync_signal_to_table(&mut self, name: &str) {
        if let Some(&id) = self.signal_name_to_id.get(name) {
            if let Some(val) = self.signals.get(name) {
                self.signal_table[id] = val.clone();
            }
        }
    }

    /// Batch-sync signal_table → signals HashMap.
    /// Called lazily before any code that reads from the HashMap.
    fn sync_table_to_hashmap(&mut self) {
        if !self.table_modified { return; }
        for (id, name) in self.id_to_name.iter().enumerate() {
            self.signals.insert(name.clone(), self.signal_table[id].clone());
        }
        self.table_modified = false;
    }

    /// Mark a signal as dirty by name (for settle_combinatorial)
    #[inline]
    fn mark_dirty(&mut self, name: &str) {
        if let Some(&id) = self.signal_name_to_id.get(name) {
            if !self.dirty_signals[id] {
                self.dirty_signals[id] = true;
                self.dirty_list.push(id);
            }
            self.dirty_any = true;
        }
    }

    /// Mark a signal as dirty by ID
    #[inline]
    fn mark_dirty_id(&mut self, id: usize) {
        if !self.dirty_signals[id] {
            self.dirty_signals[id] = true;
            self.dirty_list.push(id);
        }
        self.dirty_any = true;
        if self.activity_mon {
            self.signal_toggle_counts[id] += 1;
        }
    }

    /// Fast signal write: update both signal_table and signals HashMap.
    #[inline]
    fn fast_signal_write(&mut self, name: &str, val: Value) -> bool {
        if let Some(&id) = self.signal_name_to_id.get(name) {
            let width = self.signal_widths[id];
            let mut resized = val.resize(width);
            resized.is_signed = self.signal_signed[id];
            let changed = self.signal_table[id] != resized;
            if changed {
                self.signal_table[id] = resized;
                self.table_modified = true;
                self.mark_dirty(name);
            }
            changed
        } else {
            // Fallback
            self.sync_table_to_hashmap();
            let width = self.widths.get(name).copied().unwrap_or(val.width);
            let mut resized = val.resize(width);
            resized.is_signed = self.signed_signals.contains(name);
            let changed = self.signals.get(name).map_or(true, |p| *p != resized);
            if changed { self.mark_dirty(name); }
            self.signals.insert(name.to_string(), resized);
            changed
        }
    }

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
    fn infer_lhs_width(&self, expr: &Expression) -> u32 {
        match &expr.kind {
            ExprKind::Concatenation(p) => p.iter().map(|x| self.infer_lhs_width(x)).sum(),
            ExprKind::Ident(h) => {
                // Fast path: use cached signal ID
                if let Some(id) = h.cached_signal_id.get() {
                    return self.signal_widths[id];
                }
                let name_ref = h.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
                if let Some(&id) = self.signal_name_to_id.get(name_ref) {
                    h.cached_signal_id.set(Some(id));
                    return self.signal_widths[id];
                }
                self.widths.get(name_ref).copied().unwrap_or(32)
            }
            ExprKind::RangeSelect { left, right, .. } => {
                let l = self.eval_expr(left).to_u64().unwrap_or(0);
                let r = self.eval_expr(right).to_u64().unwrap_or(0);
                if l >= r { (l-r+1) as u32 } else { (r-l+1) as u32 }
            }
            ExprKind::Index { expr: e, index: _ } => {
                if let ExprKind::Ident(h) = &e.kind {
                    let n = self.resolve_hier_name(h);
                    if let Some((_, _, w)) = self.module.arrays.get(&n) { return *w; }
                }
                1
            }
            _ => self.infer_width(expr)
        }
    }
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
        self.sync_table_to_hashmap();
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

        // Build hierarchical signal tree from dotted names.
        // Signal "uut.cpu.reg_op1" → hierarchy ["uut", "cpu"], leaf "reg_op1"
        // Signal "clk" → hierarchy [], leaf "clk"
        // Signal "uut.cpuregs[5]" → hierarchy ["uut"], leaf "cpuregs[5]"
        use std::collections::BTreeMap;
        struct ScopeNode {
            children: BTreeMap<String, ScopeNode>,
            signals: Vec<(String, u32, String)>, // (leaf_name, width, vcd_id)
        }
        impl ScopeNode {
            fn new() -> Self { ScopeNode { children: BTreeMap::new(), signals: Vec::new() } }
        }

        let mut root = ScopeNode::new();
        for name in &sig_names {
            let width = self.widths.get(name).copied().unwrap_or(1);
            let id = id_map[name].clone();
            // Split into hierarchy parts
            let parts: Vec<&str> = name.split('.').collect();
            let (scope_parts, leaf) = if parts.len() > 1 {
                (&parts[..parts.len()-1], parts[parts.len()-1])
            } else {
                (&[][..], parts[0].as_ref())
            };
            // Navigate/create scope tree
            let mut node = &mut root;
            for part in scope_parts {
                node = node.children.entry(part.to_string()).or_insert_with(ScopeNode::new);
            }
            node.signals.push((leaf.to_string(), width, id));
        }

        // Emit VCD scopes recursively
        fn emit_scope(w: &mut impl Write, name: &str, node: &ScopeNode) {
            let _ = writeln!(w, "$scope module {} $end", name);
            // Emit signals at this level
            for (leaf, width, id) in &node.signals {
                let _ = writeln!(w, "$var wire {} {} {} $end", width, id, leaf);
            }
            // Emit child scopes
            for (child_name, child_node) in &node.children {
                emit_scope(w, child_name, child_node);
            }
            let _ = writeln!(w, "$upscope $end");
        }

        // Use actual top module name
        let top_name = &self.module.name;
        let _ = writeln!(w, "$scope module {} $end", top_name);
        // Emit top-level signals (no dot prefix)
        for (leaf, width, id) in &root.signals {
            let _ = writeln!(w, "$var wire {} {} {} $end", width, id, leaf);
        }
        // Emit sub-module scopes
        for (child_name, child_node) in &root.children {
            emit_scope(&mut w, child_name, child_node);
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
            let ch = match (val.bits_first()) {
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
                let ch = match val.get_bit(i) {
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

        // Collect changes using signal_table (no HashMap sync needed)
        let mut changes: Vec<(String, Value)> = Vec::new();
        for (id, name) in self.id_to_name.iter().enumerate() {
            if let Some(vcd_id) = self.vcd_id_map.get(name) {
                let val = &self.signal_table[id];
                let changed = match self.vcd_prev_signals.get(name) {
                    Some(prev) => prev != val,
                    None => true,
                };
                if changed {
                    changes.push((vcd_id.clone(), val.clone()));
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

        // Update previous snapshot from signal_table
        for (id, name) in self.id_to_name.iter().enumerate() {
            self.vcd_prev_signals.insert(name.clone(), self.signal_table[id].clone());
        }
    }

    /// Flush and close VCD file
    fn vcd_finish(&mut self) {
        if let Some(ref mut w) = self.vcd_writer {
            let _ = w.flush();
        }
        self.vcd_writer = None;
    }
}

// This will be injected at the right place
