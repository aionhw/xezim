//! Event-driven simulator for SystemVerilog combinatorial and sequential logic.
//!
//! Implements a simplified IEEE 1800 scheduling model:
//!   Active region:  blocking assigns, continuous assigns, always_comb
//!   NBA region:     non-blocking assign updates
//!   Reactive:       edge-triggered always_ff/always_latch blocks

use std::collections::BTreeMap;
use std::cell::{Cell, RefCell};
use ahash::{AHashMap as HashMap, AHashSet as HashSet};
use libffi::middle::{Arg, Cif, CodePtr, Type};
use libloading::Library;
use rand::SeedableRng;
use std::fs::OpenOptions;
use std::ffi::{CStr, CString};
use std::io::Write;
use std::os::raw::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use crate::ast::expr::*;
use crate::ast::stmt::*;
use crate::ast::decl::{AlwaysKind, ConstraintItem, ConstraintRange, CovergroupDeclaration, CovergroupItem, FunctionDeclaration, LetDeclaration, TaskDeclaration};
use crate::ast::types::{DataType, IntegerAtomType, PortDirection};
use super::value::{Value, LogicBit};
use super::elaborate::{DpiImportSpec, ElaboratedModule, AlwaysBlock};
#[allow(unused_imports)]
use crate::{log_println as println, log_eprintln as eprintln};

static SIM_DEBUG_ENABLED: AtomicBool = AtomicBool::new(false);
static DPI_LIB_PATHS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

pub fn set_sim_debug(enabled: bool) {
    SIM_DEBUG_ENABLED.store(enabled, Ordering::Relaxed);
}

#[inline]
fn sim_debug_enabled() -> bool {
    SIM_DEBUG_ENABLED.load(Ordering::Relaxed)
}

fn dpi_lib_paths() -> &'static Mutex<Vec<String>> {
    DPI_LIB_PATHS.get_or_init(|| Mutex::new(Vec::new()))
}

pub fn set_dpi_libs(paths: &[String]) {
    if let Ok(mut guard) = dpi_lib_paths().lock() {
        *guard = paths.to_vec();
    }
}

fn configured_dpi_libs() -> Vec<String> {
    dpi_lib_paths().lock().map(|g| g.clone()).unwrap_or_default()
}

macro_rules! sim_dbg_eprintln {
    ($($arg:tt)*) => {
        if sim_debug_enabled() {
            eprintln!($($arg)*);
        }
    };
}

/// A combinatorial item (continuous assign or always @*/always_comb block)
/// with pre-computed sensitivity set for efficient evaluation.
#[derive(Clone)]
enum CombItem {
    ContAssign { lhs: Expression, rhs: Expression },
    /// Fast path: direct signal-to-signal copy (assign b = a) with pre-resolved IDs.
    DirectCopy { dst_id: usize, src_id: usize, width: u32 },
    /// Bytecode-compiled cont_assign: RHS compiled to VM instructions,
    /// result written to pre-resolved dst_id via BlockingAssign insn.
    CompiledContAssign { compiled: super::bytecode::CompiledBlock },
    AlwaysBlock { stmt: Statement, is_always_comb: bool },
    /// Bytecode-compiled comb always block. Skips the AST interpreter per
    /// settle iteration — dominant hot path in RTL-heavy designs.
    CompiledAlwaysBlock { compiled: super::bytecode::CompiledBlock, is_always_comb: bool },
    /// Fused 1-bit gate: recognizes yosys-generated patterns
    /// `assign d[i] = a op b`, `assign d[i] = ~a`, `assign d[i] = s ? t : e`.
    /// Skips bytecode VM entirely — reads operand bits from signal_table,
    /// computes 4-state logic, writes single bit back.
    FusedGate { op: FusedGate },
}

#[derive(Clone, Copy, Debug)]
struct BitRef { sig_id: u32, bit: u32 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GateBin { And, Or, Xor }

#[derive(Clone, Copy, Debug)]
enum FusedGate {
    /// dst = src, or dst = ~src when invert
    Buf1 { dst: BitRef, src: BitRef, invert: bool },
    /// dst = a op b, or dst = ~(a op b) when invert
    Bin2 { dst: BitRef, a: BitRef, b: BitRef, op: GateBin, invert: bool },
    /// dst = s ? t : e
    Mux2 { dst: BitRef, s: BitRef, t: BitRef, e: BitRef },
}

#[derive(Clone)]
struct CombEntry {
    item: CombItem,
    /// Preferred hierarchical scope for resolving unqualified identifiers.
    scope_hint: Option<String>,
    /// Pre-resolved signal IDs for reads (for fast dependency lookup).
    read_signal_ids: Vec<usize>,
    /// Pre-resolved signal IDs for writes. The original layout
    /// stored a `(usize, String)` tuple but every read site
    /// destructures the String with `_` — name was dead weight.
    /// At c910 scale (~467K entries × ~5 writes each) the dropped
    /// String per write reclaims tens of MB.
    write_signal_ids: Vec<usize>,
    /// True when dependency extraction could not resolve all read signals.
    /// Such entries are conservatively re-evaluated each settle pass.
    has_unresolved_reads: bool,
}

#[derive(Debug, Clone)]
pub struct SimOutput { pub time: u64, pub message: String }

#[derive(Debug, Clone)]
struct NbaEntry { lhs: Option<Expression>, value: Value, resolved_id: Option<usize> }

#[derive(Debug, Clone)]
struct JoinWaiter {
    parent_pid: usize,
    child_pids: HashSet<usize>,
    join_type: JoinType,
    continuation: Vec<Statement>,
    finished_children: HashSet<usize>,
}

/// Fast-path NBA entry: compact (signal_id, value) pair for pre-resolved targets.
/// 99%+ of NBA entries use this path. Smaller struct = better cache utilization.
struct NbaFast { signal_id: usize, value: Value }

#[derive(Debug, Clone)]
struct EdgeSensitiveBlock {
    /// Pre-resolved signal IDs for O(1) edge checking. The unresolved
    /// `Vec<Sensitivity>` (with String name) used to live here too —
    /// dropped, since the only consumer (the init pass collecting
    /// edge_signal_ids/names) was rewritten to use this resolved
    /// list + id_to_name.
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
    /// Pre-resolved signal IDs for O(1) edge checking. The unresolved
    /// `Vec<Sensitivity>` mirror was set at construction but never
    /// consulted afterwards — dropped.
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
    /// Multiset of pids currently scheduled somewhere in the wheel or
    /// overflow. Lets `has_pid` return O(1) instead of scanning all 256
    /// slots (and all overflow) on every `is_pid_suspended` check —
    /// which fires on every run_scheduled_process call (~115K times on c910).
    /// Count-based rather than set-based because the same pid can have
    /// multiple scheduled events at different times.
    pid_counts: std::collections::HashMap<usize, u32>,
}

impl TimingWheel {
    fn new() -> Self {
        let mut wheel = Vec::with_capacity(WHEEL_SIZE);
        for _ in 0..WHEEL_SIZE { wheel.push(Vec::new()); }
        TimingWheel {
            wheel, bitmap: [0u64; BITMAP_WORDS],
            overflow: BTreeMap::new(), current_time: 0,
            pid_counts: std::collections::HashMap::new(),
        }
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
        sim_dbg_eprintln!("[DEBUG] scheduling process {} at time {}", pid, time);
        *self.pid_counts.entry(pid).or_insert(0) += 1;
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
            // Decrement pid_counts for each removed event.
            for (p, _) in &events {
                if let Some(c) = self.pid_counts.get_mut(p) {
                    *c = c.saturating_sub(1);
                    if *c == 0 { self.pid_counts.remove(p); }
                }
            }
        }
        events
    }

    /// O(1) check: is any event scheduled for this pid anywhere in the queue?
    fn has_pid(&self, pid: usize) -> bool {
        self.pid_counts.contains_key(&pid)
    }
}

#[derive(Debug, Clone)]
struct ClassInstance {
    class_name: String,
    properties: HashMap<String, Value>,
}

#[derive(Debug, Clone)]
struct CovergroupInstance {
    cg_name: String,
    /// Hits: coverpoint_name -> Set of observed values
    point_hits: HashMap<String, HashSet<Value>>,
    /// Cross hits: cross_name -> Set of observed tuples
    cross_hits: HashMap<String, HashSet<Vec<Value>>>,
}

#[derive(Debug, Clone, Default)]
struct ProcessContext {
    this_stack: Vec<Option<usize>>,
    local_stack: Vec<HashMap<String, Value>>,
    class_context_stack: Vec<Option<String>>,
    cg_this: Option<usize>,
    return_value: Option<Value>,
    break_flag: bool,
    continue_flag: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DpiRetKind {
    Void,
    Int32,
    Int64,
    Real32,
    Real64,
    Chandle,
    String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DpiArgKind {
    Int32In,
    Int32Out,
    Int64In,
    Int64Out,
    Real32In,
    Real32Out,
    Real64In,
    Real64Out,
    ChandleIn,
    ChandleOut,
    StringIn,
    StringOut,
    OpenArrayI32In,
    OpenArrayI32Out,
    VecLogicIn(u32),
    VecLogicOut(u32),
}

struct DpiBinding {
    ret: DpiRetKind,
    arg_kinds: Vec<DpiArgKind>,
    cif: Cif,
    fn_ptr: CodePtr,
}

#[repr(C)]
struct DpiLogicVecVal {
    aval: *mut u32,
    bval: *mut u32,
}

pub struct Simulator {
    pub signals: HashMap<String, Value>,
    /// Fast signal table: indexed by signal_id for O(1) access.
    signal_table: Vec<Value>,
    /// Map signal name → signal_id for fast lookup. `Arc<str>` keys are
    /// shared with `id_to_name` so each signal name lives on the heap
    /// once instead of twice (saves ~25 MB on c910-scale). `Arc<str>:
    /// Borrow<str>` lets `.get(&str)` lookups work zero-alloc; call sites
    /// that previously had a `&String` use `.as_str()` to convert.
    signal_name_to_id: HashMap<Arc<str>, usize>,
    /// Lazy per-array element-ID cache: array_name → Vec where index is the
    /// element index and the value is the flat signal_id (None if missing).
    /// Populated on first miss in the hot write/read path; avoids per-access
    /// `format!("{}[{}]", name, i)` + HashMap lookup on tight loops like
    /// memory wipe/load in testbenches.
    array_elem_ids: HashMap<String, Vec<i64>>,
    /// Reverse index: leaf-name (last `.`-segment of a signal's full path) →
    /// signal_ids that share that leaf. Built once for the "real" signal set
    /// (excludes per-element array IDs which all end with `]`). Keeps the
    /// `path.len()==1 && bare leaf` resolution path O(1) instead of an
    /// O(N) scan over signal_name_to_id, which on c910 scales to 35M entries
    /// and was the dominant cost (575s) of time-0 settle for testbench-probe
    /// continuous assigns like `assign x = bare_leaf;`.
    leaf_name_to_ids: HashMap<Arc<str>, Vec<usize>>,
    id_to_name: Vec<Arc<str>>,
    /// Map signal_id → width (for fast width lookup).
    signal_widths: Vec<u32>,
    /// Set of signal IDs that are signed.
    signal_signed: Vec<bool>,
    signal_real: Vec<bool>,
    /// Sparse: signal_id → declared user type name (e.g. class/struct
    /// type for `MyClass h;`). Only populated for signals where the
    /// elaborator recorded a non-None `type_name` on the source
    /// `Signal` struct — most regular bit/logic signals have None
    /// here. Populated once at construction so `module.signals` can
    /// be freed afterwards.
    signal_type_names: HashMap<usize, String>,
    pub widths: HashMap<String, u32>,
    pub signed_signals: HashSet<String>,
    pub real_signals: HashSet<String>,
    /// Fast prev signal table for edge detection (indexed by signal_id).
    /// SoA storage for "previous-iter" edge-detection state (A3 from the
    /// compression analysis). Replaces `prev_table: Vec<Value>` for signals
    /// with width ≤ 64 — which is ~95% of typical designs — at half the
    /// per-signal footprint (16 B vs 32 B) and with direct u64 compare
    /// in the check_edges hot path. Wide signals (> 64 bits) fall back to
    /// `prev_wide` below.
    prev_val: Vec<u64>,
    prev_xz: Vec<u64>,
    /// Fallback for signals wider than 64 bits where the inline u64 pair
    /// above can't represent the full state.
    prev_wide: HashMap<usize, Value>,
    edge_signal_names: HashSet<String>,
    /// Edge sensitivity resolved to signal IDs.
    edge_signal_ids: Vec<usize>,
    /// Reverse index signal_id → [(edge_block_idx, edge_kind)] built once after
    /// edge_blocks are classified. Lets `check_edges` iterate edge-sensitive
    /// signals (typically ~100s on c910: clk, rst_b, a few enables) instead
    /// of all edge_blocks (10K+), yielding 50-100× faster edge detection.
    edge_blocks_by_sig: Vec<Vec<(usize, EdgeKind)>>,
    pub time: u64,
    pub output: Vec<SimOutput>,
    capture_output: bool,
    pub finished: bool,
    pub monitor: Option<(String, Vec<Expression>)>,
    pub monitor_prev: HashMap<String, Value>,
    /// Active tag for a tagged union variable: signal name → tag name.
    pub active_union_tag: HashMap<String, String>,
    pub max_time: u64,
    /// Maximum iterations for combinatorial settling per cycle.
    pub settle_limit: u32,
    /// Maximum snapshot→apply_nba→settle→check_edges rounds per event-loop
    /// iter (see drain_edge_cascade). Cached at sim init; override via
    /// XEZIM_CASCADE_LIMIT env var.
    cascade_limit: u32,
    /// SDF delay annotation (None if no SDF loaded).
    pub sdf_annotation: Option<super::sdf::SdfAnnotation>,
    /// Per-signal delay in sim ticks (0 = no delay). Indexed by signal_id.
    sdf_delays: Vec<u64>,
    /// Pending delayed signal updates: (time, signal_id, value)
    delayed_updates: Vec<(u64, usize, Value)>,
    module: ElaboratedModule,
    dpi_libraries: Vec<Library>,
    dpi_bindings: HashMap<String, DpiBinding>,
    dpi_unsupported: HashSet<String>,
    dpi_unresolved: HashSet<String>,
    /// Class instance heap (index 0 is null).
    heap: Vec<Option<ClassInstance>>,
    /// Built-in mailboxes (handle -> queue of values)
    mailboxes: HashMap<usize, std::collections::VecDeque<Value>>,
    /// Built-in semaphores (handle -> current count)
    semaphores: HashMap<usize, i64>,
    /// Covergroup instance heap (index 0 is null).
    cg_heap: Vec<Option<CovergroupInstance>>,
    /// Call stack for tracking 'this' and local variables.
    this_stack: Vec<Option<usize>>,
    local_stack: Vec<HashMap<String, Value>>,
    /// Context for 'super' resolution: stack of (current_class_name).
    class_context_stack: Vec<Option<String>>,
    /// Current covergroup instance if in sampling context.
    cg_this: Option<usize>,
    /// Processes waiting for join
    join_waiters: Vec<JoinWaiter>,
    /// Map from child PID -> parent PID
    process_parents: HashMap<usize, usize>,
    /// Per-process execution context for scheduled class/task processes.
    process_contexts: HashMap<usize, ProcessContext>,
    /// Return value from last function call.
    return_value: Option<Value>,
    /// Random number generator for randomization.
    rng: rand::rngs::StdRng,
    settling: bool,
    in_edge_block: bool,
    nba_queue: Vec<NbaEntry>,
    /// Fast-path NBA buffer: pre-resolved (signal_id, value) pairs.
    nba_fast: Vec<NbaFast>,
    /// Reverse index: signal_id → most recent index in `nba_fast` for that
    /// signal during the *current* NBA accumulation window. Used by the
    /// partial-range/bit NBA Insns (NbaAssignRange, NbaAssignRangeDyn,
    /// NbaAssignBitDyn) to merge into the existing pending entry instead
    /// of re-scanning `nba_fast` linearly. The previous `iter().rposition`
    /// scan was O(N) per call → O(N²) per cycle when an always block had
    /// many partial-range NBAs (c910 testbench wrappers — thousands per
    /// posedge clk). Cleared by `apply_nba` once the entries are drained.
    nba_fast_index: HashMap<usize, usize>,
    edge_blocks: Vec<EdgeSensitiveBlock>,
    /// Bytecode-compiled edge blocks (for blocks that compiled successfully).
    /// Index matches edge_blocks. None = fallback to AST interpreter.
    compiled_edge_blocks: Vec<Option<super::bytecode::CompiledBlock>>,
    /// Parallel array to `compiled_edge_blocks`: when feature=jit is on
    /// and `JitModule::try_compile` succeeded at elaboration time, this
    /// holds the native function pointer for the block. `exec_bytecode`
    /// calls it in place of the interpreter loop. None = use interpreter.
    jit_fns: Vec<Option<super::jit::JitFn>>,
    /// The `JitModule` owns the JIT'd code's memory — it must outlive
    /// all compiled function pointers. Kept on Simulator so the mmapped
    /// code pages stay mapped for the life of the simulation.
    jit_module: Option<super::jit::JitModule>,
    /// True for blocks eligible for parallel execution (no StmtFallback).
    edge_block_parallel: Vec<bool>,
    /// Per-edge-block: cached scope hint (parent module name from first
    /// sensitivity signal) so sequential-exec path can skip per-call rsplit.
    edge_block_scope: Vec<Option<String>>,
    /// Per-edge-block: true if block contains StmtFallback insns (and thus
    /// needs `name_resolve_hint` to be set for AST-exec). When false, the
    /// save/restore clone of the RefCell string can be skipped.
    edge_block_needs_hint: Vec<bool>,
    /// VM register file (reusable across executions to avoid allocation).
    vm_regs: Vec<Value>,
    /// Built-in clock generators (optimized always #N clk = ~clk)
    clock_generators: Vec<ClockGen>,
    event_queue: TimingWheel,
    next_pid: usize,
    current_pid: usize,
    /// Value that `$` resolves to in the current evaluation scope
    /// (e.g. queue upper bound during `q[a:$]`). Stack of overrides.
    dollar_bound: Vec<i64>,
    break_flag: bool,
    continue_flag: bool,
    rs_return_flag: bool,
    /// Processes waiting for signal edge events (@(posedge clk), etc.)
    event_waiters: Vec<EventWaiter>,
    /// Covergroups waiting for sampling events
    cg_event_waiters: Vec<(usize, Vec<SensitivityId>)>,
    /// Swap buffer for event_waiters filtering (avoids allocation per cycle)
    event_waiters_swap: Vec<EventWaiter>,
    /// VCD dump state
    vcd_file: Option<String>,
    vcd_writer: Option<super::vcd_sink::VcdSink>,
    vcd_id_map: HashMap<String, String>,
    vcd_enabled: bool,
    vcd_last_time: u64,
    vcd_prev_signals: HashMap<String, Value>,
    /// Worker-thread count. >=2 routes VCD/AITRACE dumps through a
    /// background writer thread (see vcd_sink::VcdSink).
    threads: usize,
    /// Buffered stdout sink for $display/$write. Lazily initialized on first
    /// write so threaded mode can be enabled by `set_threads` beforehand.
    stdout_sink: Option<super::stdout_sink::StdoutSink>,
    /// AITRACE mode: when true, $dumpfile/$dumpvars emit AITRACE-T instead of VCD
    pub aitrace_mode: bool,
    /// Pre-computed combinatorial entries with sensitivity sets.
    comb_entries: Vec<CombEntry>,
    /// Precomputed indices of comb_entries with has_unresolved_reads=true.
    /// Built once alongside comb_entries; scanning this short list every
    /// settle call is vastly cheaper than iterating all num_entries bits
    /// (can be ~500K on designs like c910).
    comb_unresolved_idx: Vec<usize>,
    /// Precomputed indices of comb_entries that should fire unconditionally
    /// at time=0: entries with empty read_signal_ids, or always_comb blocks.
    /// Avoids an O(num_entries) scan on every settle call while time=0.
    comb_time0_idx: Vec<usize>,
    /// Latches once `comb_time0_idx` has been seeded into settle_triggered.
    /// Every subsequent settle call at time=0 can skip the re-seed; the
    /// worklist and dirty propagation already keep things moving.
    comb_time0_fired: bool,
    /// Reverse index: signal_id → list of comb_entry indices that read
    /// this signal. CSR layout — `comb_dep_entries[comb_dep_offsets[id]
    /// .. comb_dep_offsets[id+1]]` gives the dependents of signal `id`.
    /// Compared to the prior `Vec<Vec<usize>>` this drops 24 B / signal
    /// of empty-Vec-header overhead (≈14 MB on c910's 585K signals)
    /// and packs the entry indices contiguously for better cache use
    /// in `settle_combinatorial`'s hot loop. u32 is sufficient — both
    /// signal_id and comb_entry index fit comfortably in 32 bits at
    /// any practical design size.
    comb_dep_offsets: Vec<u32>,
    comb_dep_entries: Vec<u32>,
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
    prof_edge_detect: u64,
    prof_edge_exec: u64,
    prof_edge_waiters: u64,
    prof_edge_cg: u64,
    prof_waiter_iters: u64,
    prof_edges_fired: u64,
    prof_insns_executed: u64,
    prof_fallback_insns: u64,
    prof_fallback_by_reason: HashMap<&'static str, (u64, u64)>,
    prof_settle_dc_ns: u64,
    prof_settle_ca_ns: u64,
    prof_settle_ab_ns: u64,
    prof_settle_dc_count: u64,
    prof_settle_ca_count: u64,
    prof_settle_ab_count: u64,
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
    /// Runtime plusargs passed from CLI/filelists (e.g. +FOO, +BAR=1).
    plusargs: Vec<String>,
    /// Open file handles for $fopen/$fwrite/$fclose.
    file_handles: HashMap<i32, std::fs::File>,
    /// Per-fd ungetc pushback buffer (LIFO).
    ungetc_buf: HashMap<i32, Vec<u8>>,
    static_task_init: HashSet<String>,
    current_static_task: Option<String>,
    next_file_handle: i32,
    /// Best-effort hierarchical context for resolving ambiguous leaf identifiers.
    name_resolve_hint: RefCell<Option<String>>,
}

impl Simulator {
    pub fn new(module: ElaboratedModule, max_time: u64) -> Self {
        // Static signals + parameters live exclusively in the indexed
        // signal_table / signal_name_to_id / parallel Vecs below. The
        // legacy `signals`/`widths`/`signed_signals`/`real_signals`
        // HashMaps stay empty after construction and are used only as an
        // overflow store for *dynamically* created entries (queue
        // elements, foreach loop vars, in-process var decls, $cast
        // targets). Eliminating the bulk-populate saves ~150–250 MB on
        // c910-scale designs (585K signals × 2 stores × name + Value
        // duplication).
        let signals: HashMap<String, Value> = HashMap::new();
        let widths: HashMap<String, u32> = HashMap::new();
        let signed_signals: HashSet<String> = HashSet::new();
        let real_signals: HashSet<String> = HashSet::new();

        // Collect just *names* from the two source maps and sort them
        // for deterministic id assignment. We avoid an intermediate
        // `Vec<(name, Value, …)>` because cloning each Value into that
        // Vec would peak-spike RSS by exactly the amount we just saved
        // by skipping the legacy bulk-populate.
        let mut names: Vec<String> =
            Vec::with_capacity(module.signals.len() + module.parameters.len());
        for name in module.signals.keys() {
            names.push(name.clone());
        }
        for name in module.parameters.keys() {
            // Parameter and signal can share a name — dedup after sort.
            names.push(name.clone());
        }
        names.sort();
        names.dedup();

        let n = names.len();
        let mut signal_name_to_id: HashMap<Arc<str>, usize> = HashMap::with_capacity(n);
        let mut leaf_name_to_ids: HashMap<Arc<str>, Vec<usize>> = HashMap::new();
        let mut id_to_name: Vec<Arc<str>> = Vec::with_capacity(n);
        let mut signal_table: Vec<Value> = Vec::with_capacity(n);
        let mut signal_widths_vec: Vec<u32> = Vec::with_capacity(n);
        let mut signal_signed_vec: Vec<bool> = Vec::with_capacity(n);
        let mut signal_real_vec: Vec<bool> = Vec::with_capacity(n);
        let mut signal_type_names: HashMap<usize, String> = HashMap::new();
        // Drain `names` so each String moves into the Arc allocation
        // and is freed promptly, instead of being kept alongside the
        // Arc<str> copies until the end of construction.
        for (id, name) in names.drain(..).enumerate() {
            let arc_name: Arc<str> = Arc::from(name.as_str());
            signal_name_to_id.insert(arc_name.clone(), id);
            // Build leaf-name reverse index (skip array-element style names
            // ending with `]`, which are added below and don't participate
            // in bare-leaf lookups).
            let leaf = name.rsplit_once('.').map(|(_, l)| l).unwrap_or(name.as_str());
            if !leaf.is_empty() && !leaf.ends_with(']') {
                let arc_leaf: Arc<str> = Arc::from(leaf);
                leaf_name_to_ids.entry(arc_leaf).or_default().push(id);
            }
            id_to_name.push(arc_name);
            // Fetch value/metadata directly from the source map. The
            // signal entry takes precedence over a same-named parameter.
            if let Some(sig) = module.signals.get(&name) {
                let mut val = sig.value.clone();
                if sig.is_signed { val.is_signed = true; }
                if sig.is_real { val.is_real = true; }
                sim_dbg_eprintln!("[DEBUG] Simulator::new signal {} = {} (signed={})", name, val.to_dec_string(), sig.is_signed);
                signal_table.push(val);
                signal_widths_vec.push(sig.width);
                signal_signed_vec.push(sig.is_signed);
                signal_real_vec.push(sig.is_real);
                if let Some(ref tn) = sig.type_name {
                    signal_type_names.insert(id, tn.clone());
                }
            } else if let Some(val) = module.parameters.get(&name) {
                sim_dbg_eprintln!("[DEBUG] Simulator::new parameter {} = {} (signed={})", name, val.to_dec_string(), val.is_signed);
                signal_table.push(val.clone());
                signal_widths_vec.push(val.width);
                signal_signed_vec.push(val.is_signed);
                signal_real_vec.push(false);
            } else {
                // Should not happen — name came from one of the two maps.
                signal_table.push(Value::new(1));
                signal_widths_vec.push(1);
                signal_signed_vec.push(false);
                signal_real_vec.push(false);
            }
        }
        // Phase 2: synthesize per-element entries for unpacked arrays.
        // Elaborate skips the per-element Signal inserts (memory-as-array
        // fix) because every element shares the same width/signed/real
        // attributes — we rebuild them here from the compact arrays
        // metadata, writing directly into signal_table + signal_name_to_id
        // + the parallel Vecs. The legacy `signals` / `widths` /
        // `signed_signals` / `real_signals` HashMaps are left empty for
        // array elements; all hot-path and fallback accesses go through
        // signal_name_to_id first, so the HashMap miss on fallback paths
        // is harmless. Saves hundreds of MB of HashMap entries on designs
        // with large testbench memories.
        let mut push_elem = |name: String, w: u32,
                             sig_table: &mut Vec<Value>,
                             widths_vec: &mut Vec<u32>,
                             signed_vec: &mut Vec<bool>,
                             real_vec: &mut Vec<bool>,
                             name_to_id: &mut HashMap<Arc<str>, usize>,
                             names: &mut Vec<Arc<str>>| {
            let id = sig_table.len();
            let arc: Arc<str> = Arc::from(name.as_str());
            name_to_id.insert(arc.clone(), id);
            names.push(arc);
            sig_table.push(Value::new(w));
            widths_vec.push(w);
            signed_vec.push(false);
            real_vec.push(false);
        };
        for (base, &(lo, hi, w)) in &module.arrays {
            for idx in lo..=hi {
                push_elem(format!("{}[{}]", base, idx), w,
                    &mut signal_table, &mut signal_widths_vec,
                    &mut signal_signed_vec, &mut signal_real_vec,
                    &mut signal_name_to_id, &mut id_to_name);
            }
        }
        for (base, &((lo1, hi1), (lo2, hi2), w)) in &module.arrays_2d {
            for i in lo1..=hi1 {
                for j in lo2..=hi2 {
                    push_elem(format!("{}[{}][{}]", base, i, j), w,
                        &mut signal_table, &mut signal_widths_vec,
                        &mut signal_signed_vec, &mut signal_real_vec,
                        &mut signal_name_to_id, &mut id_to_name);
                }
            }
        }
        for (base, (shape, w)) in &module.arrays_nd {
            fn enumerate(dims: &[(i64, i64)], prefix: String, out: &mut Vec<String>) {
                if dims.is_empty() { out.push(prefix); return; }
                let (lo, hi) = dims[0];
                for i in lo..=hi {
                    enumerate(&dims[1..], format!("{}[{}]", prefix, i), out);
                }
            }
            let mut names = Vec::new();
            enumerate(shape, base.clone(), &mut names);
            for name in names {
                push_elem(name, *w,
                    &mut signal_table, &mut signal_widths_vec,
                    &mut signal_signed_vec, &mut signal_real_vec,
                    &mut signal_name_to_id, &mut id_to_name);
            }
        }
        let num_signals = id_to_name.len();
        // prev_{val,xz} represent "before time 0" state. Per IEEE 1800,
        // variable initializers `reg x = <v>;` are equivalent to
        // initial-block assignments, so X→<v> at t=0 must generate an edge
        // event for @(posedge x) etc. Initialize to all-X (val=0,
        // xz=mask(width)) so the first check_edges at t=0 detects these
        // initializer-driven transitions.
        let mut prev_val: Vec<u64> = vec![0u64; num_signals];
        let mut prev_xz: Vec<u64> = vec![0u64; num_signals];
        let mut prev_wide: HashMap<usize, Value> = HashMap::new();
        for id in 0..num_signals {
            let w = signal_widths_vec[id];
            let mask = if w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
            prev_xz[id] = mask;
            if w > 64 {
                prev_wide.insert(id, Value::new(w));
            }
        }
        // Free `module.signals` now that the indexed signal_table /
        // signal_widths / signal_signed / signal_real / signal_type_names
        // have absorbed everything we need. On c910 this releases the
        // ~110 MB of `Signal` structs (name + Value + bools + type_name)
        // plus their HashMap overhead. `module.parameters` is kept
        // because `resolve_type_width` still resolves dimension
        // expressions against it at runtime (per step-2's caller fix).
        let mut module = module;
        module.signals = HashMap::new();

        let mut sim = Self {
            prev_val, prev_xz, prev_wide,
            edge_signal_names: HashSet::new(),
            edge_signal_ids: Vec::new(),
            edge_blocks_by_sig: Vec::new(),
            signals, widths, signed_signals, real_signals,
            signal_table, signal_name_to_id, array_elem_ids: HashMap::new(), leaf_name_to_ids, id_to_name, signal_widths: signal_widths_vec, signal_signed: signal_signed_vec, signal_real: signal_real_vec, signal_type_names,
            time: 0, output: Vec::new(), capture_output: true, finished: false,
            monitor: None, monitor_prev: HashMap::new(), active_union_tag: HashMap::new(),
            max_time, settle_limit: 100,
            cascade_limit: std::env::var("XEZIM_CASCADE_LIMIT")
                .ok().and_then(|s| s.parse().ok()).unwrap_or(8),
            sdf_annotation: None, sdf_delays: vec![0u64; num_signals], delayed_updates: Vec::new(), module,
            dpi_libraries: Vec::new(),
            dpi_bindings: HashMap::new(),
            dpi_unsupported: HashSet::new(),
            dpi_unresolved: HashSet::new(),
            heap: vec![None], // index 0 is null
            mailboxes: HashMap::new(),
            semaphores: HashMap::new(),
            cg_heap: vec![None],
            this_stack: vec![],
            local_stack: vec![],
            class_context_stack: vec![],
            cg_this: None,
            join_waiters: Vec::new(),
            process_parents: HashMap::new(),
            process_contexts: HashMap::new(),
            return_value: None,
            rng: rand::rngs::StdRng::from_entropy(),
            settling: false, in_edge_block: false,
            nba_queue: Vec::new(), nba_fast: Vec::new(), nba_fast_index: HashMap::new(), edge_blocks: Vec::new(), compiled_edge_blocks: Vec::new(),
            jit_fns: Vec::new(), jit_module: None,
            edge_block_parallel: Vec::new(), edge_block_scope: Vec::new(), edge_block_needs_hint: Vec::new(), vm_regs: Vec::new(), clock_generators: Vec::new(),
            event_queue: TimingWheel::new(), next_pid: 0, current_pid: 0,
            dollar_bound: Vec::new(),
            break_flag: false, continue_flag: false, rs_return_flag: false,
            event_waiters: Vec::new(),
            cg_event_waiters: Vec::new(),
            event_waiters_swap: Vec::new(),
            vcd_file: None,
            vcd_writer: None,
            vcd_id_map: HashMap::new(),
            vcd_enabled: false,
            vcd_last_time: u64::MAX,
            vcd_prev_signals: HashMap::new(),
            threads: 1,
            stdout_sink: None,
            aitrace_mode: false,
            comb_entries: Vec::new(),
            comb_unresolved_idx: Vec::new(),
            comb_time0_idx: Vec::new(),
            comb_time0_fired: false,
            comb_dep_offsets: Vec::new(),
            comb_dep_entries: Vec::new(),
            dirty_signals: vec![false; num_signals],
            dirty_list: Vec::with_capacity(num_signals),
            dirty_any: false,
            table_modified: false,
            settle_calls: 0, settle_triggered: Vec::new(), settle_dirty_ids: Vec::new(),
            settle_prev_values: Vec::new(), settle_triggered_list: Vec::new(), loop_iters: 0,
            prof_settle: 0, prof_edges: 0, prof_nba: 0, prof_process: 0, prof_snapshot: 0, prof_vcd: 0,
            prof_edge_detect: 0, prof_edge_exec: 0, prof_edge_waiters: 0, prof_edge_cg: 0, prof_waiter_iters: 0, prof_edges_fired: 0, prof_insns_executed: 0, prof_fallback_insns: 0,
            prof_fallback_by_reason: HashMap::new(),
            prof_settle_dc_ns: 0, prof_settle_ca_ns: 0, prof_settle_ab_ns: 0,
            prof_settle_dc_count: 0, prof_settle_ca_count: 0, prof_settle_ab_count: 0,
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
            plusargs: Vec::new(),
            file_handles: HashMap::new(),
            ungetc_buf: HashMap::new(),
            static_task_init: HashSet::new(),
            current_static_task: None,
            next_file_handle: 3,
            name_resolve_hint: RefCell::new(None),
        };
        sim.load_dpi_libraries();
        sim.bind_all_dpi_imports();
        sim
    }

    pub fn set_plusargs(&mut self, plusargs: &[String]) {
        self.plusargs = plusargs.to_vec();
        sim_dbg_eprintln!("[DEBUG] plusargs set: {:?}", self.plusargs);
    }

    /// Configure the worker-thread count. `n >= 2` enables the background
    /// VCD/AITRACE writer thread; `n == 1` keeps the dump path inline.
    pub fn set_threads(&mut self, n: usize) {
        self.threads = n.max(1);
    }

    #[inline]
    fn stdout_write(&mut self, s: &str) {
        let sink = self.stdout_sink.get_or_insert_with(|| {
            if self.threads >= 2 {
                super::stdout_sink::StdoutSink::threaded()
            } else {
                super::stdout_sink::StdoutSink::inline()
            }
        });
        sink.write_str(s);
    }

    #[inline]
    fn stdout_writeln(&mut self, s: &str) {
        let sink = self.stdout_sink.get_or_insert_with(|| {
            if self.threads >= 2 {
                super::stdout_sink::StdoutSink::threaded()
            } else {
                super::stdout_sink::StdoutSink::inline()
            }
        });
        sink.writeln_str(s);
    }

    pub fn flush_stdout(&mut self) {
        if let Some(s) = self.stdout_sink.as_mut() { s.flush(); }
    }

    #[inline]
    fn record_output(&mut self, message: String) {
        if self.capture_output {
            self.output.push(SimOutput { time: self.time, message });
        }
    }

    #[inline]
    fn plusarg_payload<'a>(arg: &'a str) -> &'a str {
        arg.strip_prefix('+').unwrap_or(arg)
    }

    fn test_plusarg(&self, pattern: &str) -> bool {
        if pattern.is_empty() {
            return false;
        }
        let hit = self.plusargs.iter().any(|a| Self::plusarg_payload(a).starts_with(pattern));
        sim_dbg_eprintln!("[DEBUG] $test$plusargs('{}') -> {}", pattern, hit);
        hit
    }

    fn parse_plusarg_format(fmt: &str) -> Option<(&str, char)> {
        let pct = fmt.find('%')?;
        let prefix = &fmt[..pct];
        let mut chars = fmt[pct + 1..].chars().peekable();
        while let Some(c) = chars.peek() {
            if c.is_ascii_digit() || *c == '-' || *c == '+' || *c == '0' || *c == '.' {
                chars.next();
            } else {
                break;
            }
        }
        let spec = chars.next()?;
        Some((prefix, spec.to_ascii_lowercase()))
    }

    fn parse_plusarg_value(raw: &str, spec: char) -> Option<Value> {
        let cleaned: String = raw.chars().filter(|c| *c != '_').collect();
        match spec {
            'd' => {
                if let Ok(v) = cleaned.parse::<i64>() {
                    let mut out = Value::from_u64(v as u64, 64);
                    out.is_signed = true;
                    Some(out)
                } else {
                    None
                }
            }
            'h' | 'x' => {
                let s = cleaned.strip_prefix("0x").or_else(|| cleaned.strip_prefix("0X")).unwrap_or(&cleaned);
                Some(Value::from_str_radix(s, 16, 64))
            }
            'o' => Some(Value::from_str_radix(&cleaned, 8, 64)),
            'b' => Some(Value::from_str_radix(&cleaned, 2, 64)),
            's' => Some(Value::from_string(raw)),
            'f' | 'e' | 'g' => cleaned.parse::<f64>().ok().map(Value::from_f64),
            _ => None,
        }
    }

    fn eval_value_plusargs(&mut self, args: &[Expression]) -> Value {
        if args.len() < 2 {
            return Value::zero(1);
        }
        let fmt = match &args[0].kind {
            ExprKind::StringLiteral(s) => s.clone(),
            _ => self.eval_expr(&args[0]).to_sv_string(),
        };
        let Some((prefix, spec)) = Self::parse_plusarg_format(&fmt) else {
            return Value::zero(1);
        };

        for arg in &self.plusargs {
            let payload = Self::plusarg_payload(arg);
            if !payload.starts_with(prefix) {
                continue;
            }
            let suffix = &payload[prefix.len()..];
            if let Some(v) = Self::parse_plusarg_value(suffix, spec) {
                self.assign_value(&args[1], &v);
                return Value::from_u64(1, 1);
            }
        }
        Value::zero(1)
    }

    fn system_string_arg(&mut self, expr: &Expression) -> String {
        match &expr.kind {
            ExprKind::StringLiteral(s) => s.clone(),
            _ => self.eval_expr(expr).to_sv_string(),
        }
    }

    fn eval_file_handle_arg(&mut self, expr: &Expression) -> i32 {
        self.eval_expr(expr).to_i64().unwrap_or(0) as i32
    }

    fn open_file_handle(&mut self, args: &[Expression]) -> Value {
        if args.is_empty() {
            return Value::zero(32);
        }
        let path = self.system_string_arg(&args[0]);
        if path.is_empty() {
            return Value::zero(32);
        }
        let mode = if args.len() >= 2 {
            self.system_string_arg(&args[1])
        } else {
            "r".to_string()
        };
        let mut opts = OpenOptions::new();
        let has_plus = mode.contains('+');
        if mode.contains('r') {
            opts.read(true);
        }
        if mode.contains('w') {
            opts.write(true).create(true).truncate(true);
        }
        if mode.contains('a') {
            opts.append(true).create(true);
        }
        if has_plus {
            opts.read(true).write(true);
        }
        if !mode.contains('r') && !mode.contains('w') && !mode.contains('a') {
            opts.read(true);
        }
        match opts.open(&path) {
            Ok(file) => {
                let fd = self.next_file_handle;
                self.next_file_handle += 1;
                self.file_handles.insert(fd, file);
                Value::from_u64(fd as u64, 32)
            }
            Err(_) => Value::zero(32),
        }
    }

    fn close_file_handle(&mut self, args: &[Expression]) -> Value {
        if args.is_empty() {
            return Value::zero(32);
        }
        let fd = self.eval_file_handle_arg(&args[0]);
        if let Some(mut f) = self.file_handles.remove(&fd) {
            let _ = f.flush();
        }
        Value::zero(32)
    }

    fn write_file_handle(&mut self, args: &[Expression], newline: bool) -> Value {
        if args.is_empty() {
            return Value::zero(32);
        }
        let fd = self.eval_file_handle_arg(&args[0]);
        let mut payload = if args.len() > 1 {
            self.format_args(&args[1..], "$write")
        } else {
            String::new()
        };
        if newline {
            payload.push('\n');
        }
        let nbytes = payload.len() as u64;
        if fd <= 0 {
            if newline {
                print!("{}", payload);
            } else {
                print!("{}", payload);
            }
            return Value::from_u64(nbytes, 32);
        }
        if let Some(f) = self.file_handles.get_mut(&fd) {
            let _ = f.write_all(payload.as_bytes());
            let _ = f.flush();
        }
        Value::from_u64(nbytes, 32)
    }

    fn resolve_array_name_from_expr(&self, expr: &Expression) -> Option<String> {
        let (resolved, raw) = match &expr.kind {
            ExprKind::Ident(hier) => {
                let resolved = self.resolve_hier_name(hier);
                let raw = hier.path.iter().map(|s| s.name.name.as_str()).collect::<Vec<_>>().join(".");
                (resolved, raw)
            }
            _ => return None,
        };
        if self.module.arrays.contains_key(&resolved) {
            return Some(resolved);
        }
        if self.module.arrays.contains_key(&raw) {
            return Some(raw);
        }
        if let Some(found) = self.module.arrays.keys().find(|k| {
            *k == &resolved || *k == &raw || k.ends_with(&format!(".{}", resolved)) || k.ends_with(&format!(".{}", raw))
        }) {
            return Some(found.clone());
        }
        None
    }

    fn parse_mem_token(token: &str, default_radix: u32) -> Option<(u32, String)> {
        let trimmed = token.trim().trim_end_matches(',').trim_end_matches(';');
        if trimmed.is_empty() {
            return None;
        }
        if let Some(pos) = trimmed.find('\'') {
            let rem = &trimmed[pos + 1..];
            let mut chars = rem.chars();
            let base_ch = chars.next()?.to_ascii_lowercase();
            let digits = chars.as_str();
            if digits.is_empty() {
                return None;
            }
            let radix = match base_ch {
                'b' => 2,
                'o' => 8,
                'd' => 10,
                'h' | 'x' => 16,
                _ => default_radix,
            };
            return Some((radix, digits.to_string()));
        }
        Some((default_radix, trimmed.to_string()))
    }

    fn read_memory_file(&mut self, args: &[Expression], default_radix: u32) -> Value {
        if args.len() < 2 {
            sim_dbg_eprintln!("[DEBUG] $readmem*: missing arguments");
            return Value::zero(32);
        }
        let path = self.system_string_arg(&args[0]);
        if path.is_empty() {
            sim_dbg_eprintln!("[DEBUG] $readmem*: empty path");
            return Value::zero(32);
        }
        let Some(mem_name) = self.resolve_array_name_from_expr(&args[1]) else {
            sim_dbg_eprintln!("[DEBUG] $readmem*: array resolution failed for arg {:?}", args[1]);
            return Value::zero(32);
        };
        let Some((lo, hi, width)) = self.module.arrays.get(&mem_name).copied() else {
            sim_dbg_eprintln!("[DEBUG] $readmem*: array '{}' not found", mem_name);
            return Value::zero(32);
        };
        let mut addr = if args.len() >= 3 {
            self.eval_expr(&args[2]).to_i64().unwrap_or(lo)
        } else {
            lo
        };
        let end_addr = if args.len() >= 4 {
            self.eval_expr(&args[3]).to_i64().unwrap_or(hi)
        } else {
            hi
        };
        let step: i64 = if addr <= end_addr { 1 } else { -1 };
        let min_idx = lo.min(hi);
        let max_idx = lo.max(hi);
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                sim_dbg_eprintln!("[DEBUG] $readmem*: failed to read '{}': {}", path, e);
                return Value::zero(32);
            }
        };
        let mut loaded = 0usize;
        'lines: for raw_line in content.lines() {
            let line = raw_line.split("//").next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            for raw_tok in line.split_whitespace() {
                if step > 0 {
                    if addr > end_addr {
                        break 'lines;
                    }
                } else if addr < end_addr {
                    break 'lines;
                }
                if let Some(rest) = raw_tok.strip_prefix('@') {
                    let addr_hex = rest.trim_start_matches("0x").trim_start_matches("0X");
                    if let Ok(a) = i64::from_str_radix(addr_hex, 16) {
                        addr = a;
                    }
                    continue;
                }
                let Some((radix, digits)) = Self::parse_mem_token(raw_tok, default_radix) else {
                    continue;
                };
                if addr >= min_idx && addr <= max_idx {
                    let val = Value::from_str_radix(&digits, radix, width);
                    let elem = format!("{}[{}]", mem_name, addr);
                    self.fast_signal_write(&elem, val);
                    loaded += 1;
                }
                addr += step;
            }
        }
        sim_dbg_eprintln!("[DEBUG] $readmem*: loaded {} words into '{}' from '{}'", loaded, mem_name, path);
        Value::zero(32)
    }

    fn load_dpi_libraries(&mut self) {
        for path in configured_dpi_libs() {
            // SAFETY: Loading a dynamic library is inherently unsafe; we keep
            // each handle alive for the simulator lifetime.
            match unsafe { Library::new(&path) } {
                Ok(lib) => self.dpi_libraries.push(lib),
                Err(e) => eprintln!("[DPI] failed to load '{}': {}", path, e),
            }
        }
    }

    fn bind_all_dpi_imports(&mut self) {
        for (sv_name, spec) in self.module.dpi_imports.clone() {
            self.try_bind_dpi(&sv_name, &spec);
        }
    }

    fn dpi_atom_kind(&self, dt: &DataType, dims: &[crate::ast::types::UnpackedDimension], dir: PortDirection) -> Option<DpiArgKind> {
        let out_dir = matches!(dir, PortDirection::Output | PortDirection::Ref | PortDirection::Inout);
        if !dims.is_empty() {
            let w = super::elaborate::resolve_type_width(
                dt,
                Some(&self.module.parameters),
                Some(&self.module.typedefs),
            );
            let is_i32 = matches!(dt, DataType::IntegerAtom { kind: IntegerAtomType::Int | IntegerAtomType::Integer | IntegerAtomType::Byte | IntegerAtomType::ShortInt, .. })
                || (matches!(dt, DataType::Implicit { .. }) && w <= 32);
            if is_i32 {
                return Some(if out_dir { DpiArgKind::OpenArrayI32Out } else { DpiArgKind::OpenArrayI32In });
            }
            return None;
        }
        match dt {
            DataType::IntegerAtom { kind, .. } => match kind {
                IntegerAtomType::Int |
                IntegerAtomType::Integer |
                IntegerAtomType::Byte |
                IntegerAtomType::ShortInt => Some(if out_dir { DpiArgKind::Int32Out } else { DpiArgKind::Int32In }),
                IntegerAtomType::LongInt |
                IntegerAtomType::Time => Some(if out_dir { DpiArgKind::Int64Out } else { DpiArgKind::Int64In }),
            },
            DataType::IntegerVector { dimensions, .. } => {
                if dimensions.is_empty() {
                    Some(if out_dir { DpiArgKind::Int32Out } else { DpiArgKind::Int32In })
                } else {
                    let w = super::elaborate::resolve_type_width(
                        dt,
                        Some(&self.module.parameters),
                        Some(&self.module.typedefs),
                    );
                    if w <= 64 {
                        Some(if out_dir { DpiArgKind::Int64Out } else { DpiArgKind::Int64In })
                    } else {
                        Some(if out_dir { DpiArgKind::VecLogicOut(w) } else { DpiArgKind::VecLogicIn(w) })
                    }
                }
            }
            DataType::Implicit { dimensions, .. } if dimensions.is_empty() => {
                Some(if out_dir { DpiArgKind::Int32Out } else { DpiArgKind::Int32In })
            }
            DataType::Implicit { dimensions, .. } => {
                let w = if dimensions.is_empty() {
                    32
                } else {
                    super::elaborate::resolve_type_width(
                        dt,
                        Some(&self.module.parameters),
                        Some(&self.module.typedefs),
                    )
                };
                if w <= 64 {
                    Some(if out_dir { DpiArgKind::Int64Out } else { DpiArgKind::Int64In })
                } else {
                    Some(if out_dir { DpiArgKind::VecLogicOut(w) } else { DpiArgKind::VecLogicIn(w) })
                }
            }
            DataType::Real { kind, .. } => match kind {
                crate::ast::types::RealType::ShortReal => Some(if out_dir { DpiArgKind::Real32Out } else { DpiArgKind::Real32In }),
                _ => Some(if out_dir { DpiArgKind::Real64Out } else { DpiArgKind::Real64In }),
            },
            DataType::Simple { kind, .. } => match kind {
                crate::ast::types::SimpleType::Chandle => Some(if out_dir { DpiArgKind::ChandleOut } else { DpiArgKind::ChandleIn }),
                crate::ast::types::SimpleType::String => Some(if out_dir { DpiArgKind::StringOut } else { DpiArgKind::StringIn }),
                _ => None,
            },
            _ => None,
        }
    }

    fn dpi_return_kind(dt: &DataType) -> Option<DpiRetKind> {
        match dt {
            DataType::Void(_) => Some(DpiRetKind::Void),
            DataType::IntegerAtom { kind, .. } => match kind {
                IntegerAtomType::Int |
                IntegerAtomType::Integer |
                IntegerAtomType::Byte |
                IntegerAtomType::ShortInt => Some(DpiRetKind::Int32),
                IntegerAtomType::LongInt |
                IntegerAtomType::Time => Some(DpiRetKind::Int64),
            },
            DataType::IntegerVector { dimensions, .. } => {
                if dimensions.is_empty() { Some(DpiRetKind::Int32) } else { Some(DpiRetKind::Int64) }
            }
            DataType::Implicit { dimensions, .. } if dimensions.is_empty() => Some(DpiRetKind::Int32),
            DataType::Real { kind, .. } => match kind {
                crate::ast::types::RealType::ShortReal => Some(DpiRetKind::Real32),
                _ => Some(DpiRetKind::Real64),
            },
            DataType::Simple { kind, .. } => match kind {
                crate::ast::types::SimpleType::Chandle => Some(DpiRetKind::Chandle),
                crate::ast::types::SimpleType::String => Some(DpiRetKind::String),
                _ => None,
            },
            _ => None,
        }
    }

    fn dpi_signature(&self, spec: &DpiImportSpec) -> Option<(DpiRetKind, Vec<DpiArgKind>)> {
        match &spec.proto {
            crate::ast::decl::DPIProto::Function(fd) => {
                let ret = Self::dpi_return_kind(&fd.return_type)?;
                let mut args = Vec::with_capacity(fd.ports.len());
                for p in &fd.ports {
                    args.push(self.dpi_atom_kind(&p.data_type, &p.dimensions, p.direction)?);
                }
                Some((ret, args))
            }
            crate::ast::decl::DPIProto::Task(td) => {
                let mut args = Vec::with_capacity(td.ports.len());
                for p in &td.ports {
                    args.push(self.dpi_atom_kind(&p.data_type, &p.dimensions, p.direction)?);
                }
                Some((DpiRetKind::Void, args))
            }
        }
    }

    fn try_bind_dpi(&mut self, sv_name: &str, spec: &DpiImportSpec) {
        if self.dpi_bindings.contains_key(sv_name) || self.dpi_unsupported.contains(sv_name) {
            return;
        }
        let Some((ret, arg_kinds)) = self.dpi_signature(spec) else {
            self.dpi_unsupported.insert(sv_name.to_string());
            eprintln!("[DPI] unsupported prototype for '{}'", sv_name);
            return;
        };
        let mut cname = spec.c_name.clone().into_bytes();
        cname.push(0);
        let arg_types: Vec<Type> = arg_kinds.iter().map(|k| match k {
            DpiArgKind::Int32In => Type::i32(),
            DpiArgKind::Int32Out => Type::pointer(),
            DpiArgKind::Int64In => Type::i64(),
            DpiArgKind::Int64Out => Type::pointer(),
            DpiArgKind::Real32In => Type::f32(),
            DpiArgKind::Real32Out => Type::pointer(),
            DpiArgKind::Real64In => Type::f64(),
            DpiArgKind::Real64Out => Type::pointer(),
            DpiArgKind::ChandleIn => Type::pointer(),
            DpiArgKind::ChandleOut => Type::pointer(),
            DpiArgKind::StringIn => Type::pointer(),
            DpiArgKind::StringOut => Type::pointer(),
            DpiArgKind::OpenArrayI32In => Type::pointer(),
            DpiArgKind::OpenArrayI32Out => Type::pointer(),
            DpiArgKind::VecLogicIn(_) => Type::pointer(),
            DpiArgKind::VecLogicOut(_) => Type::pointer(),
        }).collect();
        let ret_type = match ret {
            DpiRetKind::Void => Type::void(),
            DpiRetKind::Int32 => Type::i32(),
            DpiRetKind::Int64 => Type::i64(),
            DpiRetKind::Real32 => Type::f32(),
            DpiRetKind::Real64 => Type::f64(),
            DpiRetKind::Chandle => Type::pointer(),
            DpiRetKind::String => Type::pointer(),
        };
        let cif = Cif::new(arg_types, ret_type);
        for lib in &self.dpi_libraries {
            let sym = unsafe { lib.get::<*mut c_void>(&cname) };
            if let Ok(s) = sym {
                self.dpi_bindings.insert(sv_name.to_string(), DpiBinding {
                    ret,
                    arg_kinds: arg_kinds.clone(),
                    cif: cif.clone(),
                    fn_ptr: CodePtr::from_ptr(*s),
                });
                return;
            }
        }
    }

    fn dpi_value_to_logic_words(v: &Value, width: u32) -> (Vec<u32>, Vec<u32>) {
        let nwords = ((width + 31) / 32).max(1) as usize;
        let mut aval = vec![0u32; nwords];
        let mut bval = vec![0u32; nwords];
        for bit in 0..(width as usize) {
            let w = bit / 32;
            let m = 1u32 << (bit % 32);
            match v.get_bit(bit) {
                LogicBit::Zero => {}
                LogicBit::One => aval[w] |= m,
                LogicBit::X => bval[w] |= m,
                LogicBit::Z => {
                    aval[w] |= m;
                    bval[w] |= m;
                }
            }
        }
        (aval, bval)
    }

    fn dpi_logic_words_to_value(aval: &[u32], bval: &[u32], width: u32) -> Value {
        let mut out = Value::zero(width.max(1));
        for bit in 0..(width as usize) {
            let w = bit / 32;
            let m = 1u32 << (bit % 32);
            let a = (aval.get(w).copied().unwrap_or(0) & m) != 0;
            let b = (bval.get(w).copied().unwrap_or(0) & m) != 0;
            let lb = match (a, b) {
                (false, false) => LogicBit::Zero,
                (true, false) => LogicBit::One,
                (false, true) => LogicBit::X,
                (true, true) => LogicBit::Z,
            };
            out.set_bit(bit, lb);
        }
        out.resize(width)
    }

    fn dpi_collect_i32_array_arg(&mut self, expr: Option<&Expression>) -> (Option<String>, Vec<i32>) {
        let Some(e) = expr else { return (None, Vec::new()); };
        if let ExprKind::Ident(hier) = &e.kind {
            let name = self.resolve_hier_name(hier);
            if let Some((lo, hi, _elem_w)) = self.module.arrays.get(&name).copied() {
                let mut out = Vec::new();
                for idx in lo..=hi {
                    let elem_name = format!("{}[{}]", name, idx);
                    let vv = if let Some(&id) = self.signal_name_to_id.get(elem_name.as_str()) {
                        self.signal_table[id].clone()
                    } else {
                        self.signals.get(&elem_name).cloned().unwrap_or_else(|| Value::zero(32))
                    };
                    out.push(vv.to_i64().unwrap_or(0) as i32);
                }
                return (Some(name), out);
            }
        }
        (None, vec![self.eval_expr(e).to_i64().unwrap_or(0) as i32])
    }

    fn dpi_writeback_i32_array_arg(&mut self, expr: &Expression, data: &[i32]) {
        if let ExprKind::Ident(hier) = &expr.kind {
            let name = self.resolve_hier_name(hier);
            if let Some((lo, hi, elem_w)) = self.module.arrays.get(&name).copied() {
                let mut k = 0usize;
                for idx in lo..=hi {
                    if k >= data.len() { break; }
                    let elem_name = format!("{}[{}]", name, idx);
                    let mut val = Value::from_u64(data[k] as u32 as u64, elem_w);
                    if let Some(&id) = self.signal_name_to_id.get(elem_name.as_str()) {
                        val.is_signed = self.signal_signed[id];
                    } else if self.signed_signals.contains(&elem_name) {
                        val.is_signed = true;
                    }
                    if let Some(&id) = self.signal_name_to_id.get(elem_name.as_str()) {
                        let changed = self.signal_table[id] != val;
                        if changed {
                            self.mark_dirty_id(id);
                            self.signal_table[id] = val;
                            self.table_modified = true;
                        }
                    } else {
                        let changed = self.signals.get(&elem_name).map_or(true, |p| *p != val);
                        if changed {
                            self.mark_dirty(&elem_name);
                        }
                        self.signals.insert(elem_name, val);
                    }
                    k += 1;
                }
                return;
            }
        }
        if let Some(v0) = data.first() {
            let w = self.infer_lhs_width(expr);
            self.assign_value(expr, &Value::from_u64(*v0 as u32 as u64, w));
        }
    }

    fn exec_dpi_import_call(&mut self, sv_name: &str, args: &[Expression]) -> Option<Value> {
        let spec = self.module.dpi_imports.get(sv_name)?.clone();
        if !self.dpi_bindings.contains_key(sv_name) && !self.dpi_unsupported.contains(sv_name) {
            self.try_bind_dpi(sv_name, &spec);
        }
        if self.dpi_unsupported.contains(sv_name) {
            return Some(Value::zero(32));
        }
        let Some(binding) = self.dpi_bindings.get(sv_name) else {
            if self.dpi_unresolved.insert(sv_name.to_string()) {
                eprintln!("[DPI] unresolved symbol '{}' (C name '{}')", sv_name, spec.c_name);
            }
            return Some(Value::zero(32));
        };
        let ret_kind = binding.ret;
        let arg_kinds = binding.arg_kinds.clone();
        let cif = binding.cif.clone();
        let fn_ptr = binding.fn_ptr;

        let mut arg_refs = Vec::with_capacity(arg_kinds.len());
        let mut i32_vals: Vec<Box<i32>> = Vec::new();
        let mut i64_vals: Vec<Box<i64>> = Vec::new();
        let mut f32_vals: Vec<Box<f32>> = Vec::new();
        let mut f64_vals: Vec<Box<f64>> = Vec::new();
        let mut ptr_vals: Vec<Box<*mut c_void>> = Vec::new();
        let mut string_ptr_cells: Vec<Box<*const i8>> = Vec::new();
        let mut open_i32_vals: Vec<Vec<i32>> = Vec::new();
        let mut cstrings: Vec<CString> = Vec::new();
        let mut logic_aval: Vec<Vec<u32>> = Vec::new();
        let mut logic_bval: Vec<Vec<u32>> = Vec::new();
        let mut logic_hdrs: Vec<Box<DpiLogicVecVal>> = Vec::new();
        let mut writebacks: Vec<(usize, DpiArgKind, Expression)> = Vec::new();

        for (i, kind) in arg_kinds.iter().enumerate() {
            match kind {
                DpiArgKind::Int32In => {
                    let v = Box::new(args.get(i).map(|e| self.eval_expr(e).to_i64().unwrap_or(0) as i32).unwrap_or(0));
                    i32_vals.push(v);
                    arg_refs.push(Arg::new(i32_vals.last().unwrap().as_ref()));
                }
                DpiArgKind::Int64In => {
                    let v = Box::new(args.get(i).map(|e| self.eval_expr(e).to_i64().unwrap_or(0)).unwrap_or(0));
                    i64_vals.push(v);
                    arg_refs.push(Arg::new(i64_vals.last().unwrap().as_ref()));
                }
                DpiArgKind::Real32In => {
                    let v = Box::new(args.get(i).map(|e| self.eval_expr(e).to_f64() as f32).unwrap_or(0.0));
                    f32_vals.push(v);
                    arg_refs.push(Arg::new(f32_vals.last().unwrap().as_ref()));
                }
                DpiArgKind::Real64In => {
                    let v = Box::new(args.get(i).map(|e| self.eval_expr(e).to_f64()).unwrap_or(0.0));
                    f64_vals.push(v);
                    arg_refs.push(Arg::new(f64_vals.last().unwrap().as_ref()));
                }
                DpiArgKind::ChandleIn => {
                    let p = Box::new(args.get(i).map(|e| self.eval_expr(e).to_u64().unwrap_or(0) as usize as *mut c_void).unwrap_or(std::ptr::null_mut()));
                    ptr_vals.push(p);
                    arg_refs.push(Arg::new(ptr_vals.last().unwrap().as_ref()));
                }
                DpiArgKind::StringIn => {
                    let s = args.get(i).map(|e| {
                        if let ExprKind::StringLiteral(t) = &e.kind { t.clone() } else { self.eval_expr(e).to_sv_string() }
                    }).unwrap_or_default();
                    let c = CString::new(s).unwrap_or_else(|_| CString::new("").unwrap());
                    let p = Box::new(c.as_ptr() as *mut c_void);
                    cstrings.push(c);
                    ptr_vals.push(p);
                    arg_refs.push(Arg::new(ptr_vals.last().unwrap().as_ref()));
                }
                DpiArgKind::StringOut => {
                    let init_s = args.get(i).map(|e| {
                        if let ExprKind::StringLiteral(t) = &e.kind { t.clone() } else { self.eval_expr(e).to_sv_string() }
                    }).unwrap_or_default();
                    let init_ptr: *const i8 = if init_s.is_empty() {
                        std::ptr::null()
                    } else {
                        let c = CString::new(init_s).unwrap_or_else(|_| CString::new("").unwrap());
                        let p = c.as_ptr();
                        cstrings.push(c);
                        p
                    };
                    let cell = Box::new(init_ptr);
                    let p = Box::new((&*cell as *const *const i8 as *mut *const i8).cast::<c_void>());
                    string_ptr_cells.push(cell);
                    ptr_vals.push(p);
                    arg_refs.push(Arg::new(ptr_vals.last().unwrap().as_ref()));
                    if let Some(expr) = args.get(i) { writebacks.push((string_ptr_cells.len() - 1, *kind, expr.clone())); }
                }
                DpiArgKind::OpenArrayI32In => {
                    let (_aname, mut arr) = self.dpi_collect_i32_array_arg(args.get(i));
                    let p = Box::new(arr.as_mut_ptr().cast::<c_void>());
                    open_i32_vals.push(arr);
                    ptr_vals.push(p);
                    arg_refs.push(Arg::new(ptr_vals.last().unwrap().as_ref()));
                }
                DpiArgKind::OpenArrayI32Out => {
                    let (_aname, mut arr) = self.dpi_collect_i32_array_arg(args.get(i));
                    let p = Box::new(arr.as_mut_ptr().cast::<c_void>());
                    open_i32_vals.push(arr);
                    ptr_vals.push(p);
                    arg_refs.push(Arg::new(ptr_vals.last().unwrap().as_ref()));
                    if let Some(expr) = args.get(i) { writebacks.push((open_i32_vals.len() - 1, *kind, expr.clone())); }
                }
                DpiArgKind::VecLogicIn(width) => {
                    let vv = args.get(i).map(|e| self.eval_expr(e)).unwrap_or_else(|| Value::zero(*width));
                    let (mut aval, mut bval) = Self::dpi_value_to_logic_words(&vv, *width);
                    let hdr = Box::new(DpiLogicVecVal {
                        aval: aval.as_mut_ptr(),
                        bval: bval.as_mut_ptr(),
                    });
                    let p = Box::new((&*hdr as *const DpiLogicVecVal as *mut DpiLogicVecVal).cast::<c_void>());
                    logic_aval.push(aval);
                    logic_bval.push(bval);
                    logic_hdrs.push(hdr);
                    ptr_vals.push(p);
                    arg_refs.push(Arg::new(ptr_vals.last().unwrap().as_ref()));
                }
                DpiArgKind::VecLogicOut(width) => {
                    let init = args.get(i).map(|e| self.eval_expr(e)).unwrap_or_else(|| Value::zero(*width));
                    let (mut aval, mut bval) = Self::dpi_value_to_logic_words(&init, *width);
                    let hdr = Box::new(DpiLogicVecVal {
                        aval: aval.as_mut_ptr(),
                        bval: bval.as_mut_ptr(),
                    });
                    let p = Box::new((&*hdr as *const DpiLogicVecVal as *mut DpiLogicVecVal).cast::<c_void>());
                    logic_aval.push(aval);
                    logic_bval.push(bval);
                    logic_hdrs.push(hdr);
                    ptr_vals.push(p);
                    arg_refs.push(Arg::new(ptr_vals.last().unwrap().as_ref()));
                    if let Some(expr) = args.get(i) { writebacks.push((logic_hdrs.len() - 1, *kind, expr.clone())); }
                }
                DpiArgKind::Int32Out => {
                    let init = args.get(i).map(|e| self.eval_expr(e).to_i64().unwrap_or(0) as i32).unwrap_or(0);
                    let b = Box::new(init);
                    let p = Box::new((&*b as *const i32 as *mut i32).cast::<c_void>());
                    i32_vals.push(b);
                    ptr_vals.push(p);
                    arg_refs.push(Arg::new(ptr_vals.last().unwrap().as_ref()));
                    if let Some(expr) = args.get(i) { writebacks.push((i32_vals.len() - 1, *kind, expr.clone())); }
                }
                DpiArgKind::Int64Out => {
                    let init = args.get(i).map(|e| self.eval_expr(e).to_i64().unwrap_or(0)).unwrap_or(0);
                    let b = Box::new(init);
                    let p = Box::new((&*b as *const i64 as *mut i64).cast::<c_void>());
                    i64_vals.push(b);
                    ptr_vals.push(p);
                    arg_refs.push(Arg::new(ptr_vals.last().unwrap().as_ref()));
                    if let Some(expr) = args.get(i) { writebacks.push((i64_vals.len() - 1, *kind, expr.clone())); }
                }
                DpiArgKind::Real32Out => {
                    let init = args.get(i).map(|e| self.eval_expr(e).to_f64() as f32).unwrap_or(0.0);
                    let b = Box::new(init);
                    let p = Box::new((&*b as *const f32 as *mut f32).cast::<c_void>());
                    f32_vals.push(b);
                    ptr_vals.push(p);
                    arg_refs.push(Arg::new(ptr_vals.last().unwrap().as_ref()));
                    if let Some(expr) = args.get(i) { writebacks.push((f32_vals.len() - 1, *kind, expr.clone())); }
                }
                DpiArgKind::Real64Out => {
                    let init = args.get(i).map(|e| self.eval_expr(e).to_f64()).unwrap_or(0.0);
                    let b = Box::new(init);
                    let p = Box::new((&*b as *const f64 as *mut f64).cast::<c_void>());
                    f64_vals.push(b);
                    ptr_vals.push(p);
                    arg_refs.push(Arg::new(ptr_vals.last().unwrap().as_ref()));
                    if let Some(expr) = args.get(i) { writebacks.push((f64_vals.len() - 1, *kind, expr.clone())); }
                }
                DpiArgKind::ChandleOut => {
                    let init = args.get(i).map(|e| self.eval_expr(e).to_u64().unwrap_or(0) as usize as *mut c_void).unwrap_or(std::ptr::null_mut());
                    let b = Box::new(init);
                    let p = Box::new((&*b as *const *mut c_void as *mut *mut c_void).cast::<c_void>());
                    ptr_vals.push(b);
                    ptr_vals.push(p);
                    arg_refs.push(Arg::new(ptr_vals.last().unwrap().as_ref()));
                    if let Some(expr) = args.get(i) { writebacks.push((ptr_vals.len() - 2, *kind, expr.clone())); }
                }
            }
        }

        let mut result = match ret_kind {
            DpiRetKind::Void => {
                unsafe { cif.call::<()>(fn_ptr, &arg_refs); }
                Value::zero(32)
            }
            DpiRetKind::Int32 => {
                let rv: i32 = unsafe { cif.call(fn_ptr, &arg_refs) };
                let mut v = Value::from_u64(rv as u32 as u64, 32);
                v.is_signed = true;
                v
            }
            DpiRetKind::Int64 => {
                let rv: i64 = unsafe { cif.call(fn_ptr, &arg_refs) };
                let mut v = Value::from_u64(rv as u64, 64);
                v.is_signed = true;
                v
            }
            DpiRetKind::Real32 => {
                let rv: f32 = unsafe { cif.call(fn_ptr, &arg_refs) };
                Value::from_f64(rv as f64)
            }
            DpiRetKind::Real64 => {
                let rv: f64 = unsafe { cif.call(fn_ptr, &arg_refs) };
                Value::from_f64(rv)
            }
            DpiRetKind::Chandle => {
                let rv: *mut c_void = unsafe { cif.call(fn_ptr, &arg_refs) };
                Value::from_u64(rv as usize as u64, 64)
            }
            DpiRetKind::String => {
                let rv: *const i8 = unsafe { cif.call(fn_ptr, &arg_refs) };
                if rv.is_null() {
                    Value::from_string("")
                } else {
                    let s = unsafe { CStr::from_ptr(rv) }.to_string_lossy();
                    Value::from_string(&s)
                }
            }
        };

        // Write back output/ref/inout values.
        for (idx, kind, expr) in writebacks {
            match kind {
                DpiArgKind::Int32Out => {
                    if let Some(v) = i32_vals.get(idx) {
                        let w = self.infer_lhs_width(&expr);
                        self.assign_value(&expr, &Value::from_u64(**v as u32 as u64, w));
                    }
                }
                DpiArgKind::Int64Out => {
                    if let Some(v) = i64_vals.get(idx) {
                        let w = self.infer_lhs_width(&expr);
                        let mut out = Value::from_u64(**v as u64, w.max(64));
                        out.is_signed = true;
                        self.assign_value(&expr, &out.resize(w));
                    }
                }
                DpiArgKind::Real64Out => {
                    if let Some(v) = f64_vals.get(idx) {
                        self.assign_value(&expr, &Value::from_f64(**v));
                    }
                }
                DpiArgKind::Real32Out => {
                    if let Some(v) = f32_vals.get(idx) {
                        self.assign_value(&expr, &Value::from_f64(**v as f64));
                    }
                }
                DpiArgKind::ChandleOut => {
                    if let Some(v) = ptr_vals.get(idx) {
                        self.assign_value(&expr, &Value::from_u64((**v) as usize as u64, 64));
                    }
                }
                DpiArgKind::StringOut => {
                    if let Some(v) = string_ptr_cells.get(idx) {
                        let s = if (**v).is_null() {
                            String::new()
                        } else {
                            unsafe { CStr::from_ptr(**v) }.to_string_lossy().to_string()
                        };
                        self.assign_value(&expr, &Value::from_string(&s));
                    }
                }
                DpiArgKind::OpenArrayI32Out => {
                    if let Some(arr) = open_i32_vals.get(idx) {
                        self.dpi_writeback_i32_array_arg(&expr, arr);
                    }
                }
                DpiArgKind::VecLogicOut(width) => {
                    if idx < logic_aval.len() && idx < logic_bval.len() {
                        let out = Self::dpi_logic_words_to_value(&logic_aval[idx], &logic_bval[idx], width);
                        let w = self.infer_lhs_width(&expr);
                        self.assign_value(&expr, &out.resize(w));
                    }
                }
                _ => {}
            }
        }

        if spec.property == Some(crate::ast::decl::DPIProperty::Pure) {
            // No side effects expected; kept for future optimization hooks.
            let _ = &mut result;
        }
        Some(result)
    }

    pub fn run(&mut self) {
        // Evaluate parameter expressions whose initializers contained function
        // calls and were deferred by the elaborator.
        let deferred: Vec<(String, Expression)> = self.module.deferred_param_exprs.clone();
        for (pname, expr) in deferred {
            let v = self.eval_expr(&expr);
            let w = self.lookup_signal_width(&pname).unwrap_or(v.width);
            let rv = v.resize(w);
            self.set_signal_value_by_name(&pname, rv);
        }
        self.classify_always_blocks();
        self.compile_edge_blocks();
        // Apply SDF / specify delays BEFORE building comb entries — the fused-gate
        // fast path bails out on signals with nonzero delay, so the delay must be
        // visible at build time or cont_assigns to delayed signals will be fused
        // and bypass `schedule_delayed`.
        if let Some(ref ann) = self.sdf_annotation {
            let mut count = 0;
            for (sig_name, &delay) in &ann.signal_delays {
                if let Some(&id) = self.signal_name_to_id.get(sig_name.as_str()) {
                    self.sdf_delays[id] = delay;
                    count += 1;
                }
            }
            eprintln!("[SDF] annotated {} signals with delays", count);
        }
        for (sig_name, &delay) in &self.module.specify_delays {
            if let Some(&id) = self.signal_name_to_id.get(sig_name.as_str()) {
                self.sdf_delays[id] = self.sdf_delays[id].max(delay);
            }
        }
        self.build_comb_entries();
        if self.activity_mon {
            self.activity_counts = vec![0u64; self.comb_entries.len()];
        }
        // Collect all edge-sensitive signal names for targeted prev
        // snapshots. We work off `resolved_sensitivities` (populated
        // during classify_always_blocks) and recover names from
        // `id_to_name` — the unresolved `Sensitivity` mirror is gone.
        for block in &self.edge_blocks {
            for sens in &block.resolved_sensitivities {
                if sens.signal_id < self.id_to_name.len() {
                    let name: &str = &self.id_to_name[sens.signal_id];
                    self.edge_signal_names.insert(name.to_string());
                    self.edge_signal_ids.push(sens.signal_id);
                }
            }
        }
        // Also collect from event waiters that are registered at time 0
        self.edge_signal_ids.sort_unstable();
        self.edge_signal_ids.dedup();
        // Build reverse index parallel to `edge_signal_ids` (position-indexed,
        // not signal_id-indexed). Dense signal_id indexing would allocate one
        // empty Vec per signal — on c910-scale designs with ~585K signals and
        // only a few thousand edge-sensitive ones, the dense layout wasted
        // ~14 MB on empty Vec stubs. Walk blocks once into a temp HashMap,
        // then materialize in edge_signal_ids order.
        let mut by_sid: HashMap<usize, Vec<(usize, EdgeKind)>> = HashMap::new();
        for (block_idx, block) in self.edge_blocks.iter().enumerate() {
            for sid in &block.resolved_sensitivities {
                by_sid.entry(sid.signal_id).or_default().push((block_idx, sid.edge));
            }
        }
        self.edge_blocks_by_sig = self.edge_signal_ids.iter()
            .map(|sid| by_sid.remove(sid).unwrap_or_default())
            .collect();
        // IEEE 1800: at time 0, always_comb blocks execute unconditionally.
        // always @* blocks do NOT execute at time 0 unless inputs change.
        // Mark all signals dirty so continuous assigns and always_comb run.
        self.dirty_list.clear();
        for i in 0..self.dirty_signals.len() { self.dirty_signals[i] = true; self.dirty_list.push(i); }
        self.dirty_any = true;
        let t_settle0 = std::time::Instant::now();
        let entries_before = self.entry_evals;
        let iters_before = self.settle_iters;
        let dc_ns_before = self.prof_settle_dc_ns;
        let dc_count_before = self.prof_settle_dc_count;
        let ca_ns_before = self.prof_settle_ca_ns;
        let ca_count_before = self.prof_settle_ca_count;
        let ab_ns_before = self.prof_settle_ab_ns;
        let ab_count_before = self.prof_settle_ab_count;
        self.settle_combinatorial();
        let dt = t_settle0.elapsed();
        if dt.as_millis() > 100 {
            eprintln!("[PHASE] time-0 settle: {:.1}ms ({} entry_evals, {} settle_iters, {} comb_entries, {} signals)",
                dt.as_secs_f64() * 1000.0,
                self.entry_evals - entries_before,
                self.settle_iters - iters_before,
                self.comb_entries.len(),
                self.signal_table.len());
            eprintln!("[PHASE] time-0 settle breakdown: DC {:.1}ms/{} ({:.1}µs), CA {:.1}ms/{} ({:.1}µs), AB {:.1}ms/{} ({:.1}µs)",
                (self.prof_settle_dc_ns - dc_ns_before) as f64 / 1e6,
                self.prof_settle_dc_count - dc_count_before,
                if self.prof_settle_dc_count > dc_count_before {
                    (self.prof_settle_dc_ns - dc_ns_before) as f64 / (self.prof_settle_dc_count - dc_count_before) as f64 / 1e3
                } else { 0.0 },
                (self.prof_settle_ca_ns - ca_ns_before) as f64 / 1e6,
                self.prof_settle_ca_count - ca_count_before,
                if self.prof_settle_ca_count > ca_count_before {
                    (self.prof_settle_ca_ns - ca_ns_before) as f64 / (self.prof_settle_ca_count - ca_count_before) as f64 / 1e3
                } else { 0.0 },
                (self.prof_settle_ab_ns - ab_ns_before) as f64 / 1e6,
                self.prof_settle_ab_count - ab_count_before,
                if self.prof_settle_ab_count > ab_count_before {
                    (self.prof_settle_ab_ns - ab_ns_before) as f64 / (self.prof_settle_ab_count - ab_count_before) as f64 / 1e3
                } else { 0.0 });
        }
        // Consume initial_blocks: after scheduling they're no longer needed,
        // so take ownership (drops the Vec when this scope ends) instead of
        // cloning — saves significant memory on large testbenches with
        // memory-init initial blocks holding tens of thousands of statements.
        let initial_blocks = std::mem::take(&mut self.module.initial_blocks);
        for ib in initial_blocks {
            let stmts = match ib.stmt.kind {
                StatementKind::SeqBlock { stmts, .. } => stmts,
                other => vec![Statement::new(other, ib.stmt.span)],
            };
            // Clock-generator fast path:
            //     initial begin VAR = CONST; forever #d VAR = ~VAR; end
            // Otherwise each half-period would schedule a new process
            // (~2.5ms per toggle on c910). Detect this pattern, apply
            // the seed assignment immediately, and register a ClockGen
            // so `fire_clock_generators` can toggle the signal O(1).
            if let Some(cg) = self.try_extract_initial_clock_gen(&stmts) {
                self.clock_generators.push(cg);
                continue;
            }
            let pid = self.next_pid; self.next_pid += 1;
            self.event_queue.schedule(0, pid, stmts);
        }
        self.event_loop();
        if self.aitrace_mode { self.aitrace_finish(); } else { self.vcd_finish(); }
    }

    /// Detect `initial begin VAR = CONST; forever #d VAR = ~VAR; end`
    /// and turn it into a ClockGen. Applies the seed assignment in-place
    /// so subsequent edge detection sees the right starting value, then
    /// returns a ClockGen to push onto `clock_generators`. Returns None
    /// if the initial block's shape doesn't match.
    fn try_extract_initial_clock_gen(&mut self, stmts: &[Statement]) -> Option<ClockGen> {
        if stmts.len() != 2 { return None; }
        // Stmt 0: BlockingAssign(VAR, CONST)
        let (seed_lhs_name, seed_val) = match &stmts[0].kind {
            StatementKind::BlockingAssign { lvalue, rvalue } => {
                let n = match &lvalue.kind {
                    ExprKind::Ident(h) => h.path.last().map(|s| s.name.name.as_str())?,
                    _ => return None,
                };
                // RHS must be a compile-time constant (plain integer,
                // Paren-wrapped, or parameter reference).
                let v = self.eval_expr(rvalue);
                // Reject X/Z seeds; we need a determinate start value.
                if v.has_xz() { return None; }
                (n, v)
            }
            _ => return None,
        };
        // Stmt 1: Forever containing #d VAR = ~VAR
        let (half_period, tog_name) = match &stmts[1].kind {
            StatementKind::Forever { body } => {
                let inner = match &body.kind {
                    StatementKind::SeqBlock { stmts: s, .. } if s.len() == 1 => &s[0],
                    StatementKind::SeqBlock { .. } => return None,
                    _ => &**body,
                };
                // inner must be TimingControl::Delay(d) with BlockingAssign body
                let (d_expr, assign_body) = match &inner.kind {
                    StatementKind::TimingControl { control: TimingControl::Delay(d), stmt } => (d, stmt.as_ref()),
                    _ => return None,
                };
                // Evaluate the delay expression at extraction time. Handles
                // Paren-wrapped integers, parameter references, and
                // `CLK_PERIOD/2`-style constant folding without having to
                // teach the matcher every AST shape.
                let delay = self.eval_expr(d_expr).to_u64()?;
                // assign_body (or inner SeqBlock): BA VAR = ~VAR
                let ba_target = match &assign_body.kind {
                    StatementKind::BlockingAssign { lvalue, rvalue } => (lvalue, rvalue),
                    StatementKind::SeqBlock { stmts: s, .. } if s.len() == 1 => {
                        match &s[0].kind {
                            StatementKind::BlockingAssign { lvalue, rvalue } => (lvalue, rvalue),
                            _ => return None,
                        }
                    }
                    _ => return None,
                };
                let (lhs, rhs) = ba_target;
                let ln = match &lhs.kind {
                    ExprKind::Ident(h) => h.path.last().map(|s| s.name.name.as_str())?,
                    _ => return None,
                };
                // rhs must be ~LHS or !LHS
                let rn = match &rhs.kind {
                    ExprKind::Unary { op: UnaryOp::BitNot, operand } |
                    ExprKind::Unary { op: UnaryOp::LogNot, operand } => {
                        match &operand.kind {
                            ExprKind::Ident(h) => h.path.last().map(|s| s.name.name.as_str())?,
                            _ => return None,
                        }
                    }
                    _ => return None,
                };
                if ln != rn { return None; }
                (delay, ln)
            }
            _ => return None,
        };
        if seed_lhs_name != tog_name { return None; }
        // Resolve signal_id. Use the (hopefully cached) resolve_hier_name
        // via signal_name_to_id directly on the leaf name — initial
        // clock blocks almost always use unqualified names in the tb.
        let sid = if let Some(&id) = self.signal_name_to_id.get(tog_name) {
            id
        } else {
            // Suffix match
            let suffix = format!(".{}", tog_name);
            let found = self.signal_name_to_id.iter()
                .find(|(k, _)| k.ends_with(&suffix))
                .map(|(_, &v)| v)?;
            found
        };
        // Apply seed: sets signal_table[sid] + marks dirty so settle sees it.
        let width = self.signal_widths[sid];
        let seed = seed_val.resize(width);
        if self.signal_table[sid] != seed {
            self.signal_table[sid] = seed;
            self.mark_dirty_id(sid);
            self.table_modified = true;
        }
        Some(ClockGen { signal_id: sid, half_period, next_toggle_time: half_period })
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
        // Take ownership — the remaining-only subset is written back at the
        // end. Avoids a full clone of every always-block AST (significant on
        // c910-scale designs with 20K+ blocks).
        let blocks = std::mem::take(&mut self.module.always_blocks);
        let mut remaining = Vec::new();
        for ab in blocks.into_iter() {
            // Check for edge-sensitive: always_ff @(posedge ...) or always @(posedge ...)
            if let Some((sens, body)) = self.extract_sensitivity(&ab.stmt) {
                if !sens.is_empty() {
                    let resolved: Vec<SensitivityId> = sens.iter().filter_map(|s| {
                        self.signal_name_to_id.get(s.signal_name.as_str()).map(|&id| SensitivityId { signal_id: id, edge: s.edge })
                    }).collect();
                    self.edge_blocks.push(EdgeSensitiveBlock {
                        resolved_sensitivities: resolved,
                        stmt: body,
                        kind: ab.kind,
                    });
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
                            sim_dbg_eprintln!("[OPT] clock generator: signal {} period {} (always #{} pattern)",
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
            remaining.push(ab);
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
            // Derive a scope hint from the block's first sensitivity signal —
            // unqualified idents inside the block are resolved under this
            // parent module scope.
            let scope_hint = block.resolved_sensitivities.first()
                .and_then(|sid| self.id_to_name.get(sid.signal_id))
                .and_then(|full| full.rsplit_once('.').map(|(p, _)| p.to_string()));
            let mut compiler = BytecodeCompiler::new(
                &self.signal_name_to_id,
                &self.signal_signed,
                &self.signal_widths,
                &self.module.arrays,
                &self.widths,
            );
            compiler.set_ast_fallback(true);
            compiler.set_scope_hint(scope_hint);
            compiler.set_tasks(&self.module.tasks);
            compiler.set_params(&self.module.parameters);
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
        sim_dbg_eprintln!("[OPT] bytecode compiled: {}/{} edge blocks", bc_count, self.edge_blocks.len());

        // JIT pass: attempt native codegen for each compiled block.
        // Unsupported Insns inside a block → None (interpreter runs it).
        // When feature=jit is off, `JitModule::new` returns None and
        // every `try_compile` also returns None — zero-cost fallback.
        let enable_jit = std::env::var("XEZIM_JIT").map(|v| v != "0" && v != "").unwrap_or(false);
        self.jit_fns = vec![None; self.compiled_edge_blocks.len()];
        if enable_jit {
            if self.jit_module.is_none() {
                self.jit_module = super::jit::JitModule::new();
            }
            if let Some(jm) = self.jit_module.as_mut() {
                let mut jit_count = 0usize;
                for (idx, cb_opt) in self.compiled_edge_blocks.iter().enumerate() {
                    if let Some(cb) = cb_opt {
                        if let Some(f) = jm.try_compile(&cb.instructions, cb.num_regs as u32) {
                            self.jit_fns[idx] = Some(f);
                            jit_count += 1;
                        }
                    }
                }
                eprintln!("[JIT] compiled {}/{} edge blocks", jit_count, self.compiled_edge_blocks.len());
            } else {
                eprintln!("[JIT] cranelift init failed; interpreter only");
            }
        }
        // Classify blocks for parallel execution: blocks with StmtFallback or
        // BlockingAssign/BlockingAssignRange/BlockingAssignBitDyn must run
        // sequentially — fallbacks need &mut self, blocking assigns mutate
        // signal_table which would race with parallel reads.
        let mut pure_count = 0;
        self.edge_block_parallel.clear();
        self.edge_block_scope.clear();
        self.edge_block_needs_hint.clear();
        for (idx, cb) in self.compiled_edge_blocks.iter().enumerate() {
            let has_fallback = cb.as_ref().map_or(false, |cb|
                cb.instructions.iter().any(|insn| matches!(insn, super::bytecode::Insn::StmtFallback(..)))
            );
            // NbaAssignBitDyn and NbaAssignRange read signal_table.clone() in
            // the parallel path and produce a full-register NbaFast entry with
            // only the addressed sub-range modified. When multiple parallel
            // blocks each write different sub-ranges of the same register
            // (e.g. yosys-synthesized per-bit FFs: `cpu_state[0] <= _00014_;`
            // in one always block, `cpu_state[1] <= _00015_;` in another),
            // merging their entries preserves only the last one — bits
            // written by earlier blocks revert to the snapshot value.
            // The sequential path merges correctly via nba_fast.rposition,
            // so keep these on the sequential path.
            let is_pure = cb.as_ref().map_or(false, |cb|
                !cb.instructions.iter().any(|insn| matches!(insn,
                    super::bytecode::Insn::StmtFallback(..) |
                    super::bytecode::Insn::BlockingAssign(..) |
                    super::bytecode::Insn::BlockingAssignRange(..) |
                    super::bytecode::Insn::BlockingAssignRangeDyn(..) |
                    super::bytecode::Insn::BlockingAssignBitDyn(..) |
                    super::bytecode::Insn::NbaAssignBitDyn(..) |
                    super::bytecode::Insn::NbaAssignRange(..) |
                    super::bytecode::Insn::NbaAssignRangeDyn(..)
                ))
            );
            if is_pure { pure_count += 1; }
            self.edge_block_parallel.push(is_pure);
            // Non-compiled blocks (AST-only) always need hint.
            self.edge_block_needs_hint.push(has_fallback || cb.is_none());
            let scope = self.edge_blocks.get(idx)
                .and_then(|b| b.resolved_sensitivities.first())
                .and_then(|sid| self.id_to_name.get(sid.signal_id))
                .and_then(|full| full.rsplit_once('.').map(|(p, _)| p.to_string()));
            self.edge_block_scope.push(scope);
        }
        sim_dbg_eprintln!("[OPT] parallel-eligible edge blocks: {}/{}", pure_count, self.compiled_edge_blocks.len());
    }

    /// Execute bytecode instructions in isolation (no &mut self).
    /// Returns NBA entries produced. Used for parallel edge block execution.
    fn exec_insns_isolated(
        insns: &[super::bytecode::Insn],
        signal_table: &[Value],
        signal_signed: &[bool],
        signal_name_to_id: &HashMap<Arc<str>, usize>,
        vm_regs: &mut Vec<Value>,
    ) -> Vec<NbaFast> {
        use super::bytecode::Insn;
        let mut nba_out: Vec<NbaFast> = Vec::new();
        let mut pc: usize = 0;
        let len = insns.len();
        while pc < len {
            match &insns[pc] {
                Insn::LoadConst(dest, val) => { vm_regs[*dest as usize] = (**val).clone(); }
                Insn::LoadSignal(dest, sig_id) => { vm_regs[*dest as usize] = signal_table[*sig_id].clone(); }
                Insn::LoadSignalSigned(dest, sig_id) => {
                    let mut v = signal_table[*sig_id].clone();
                    v.is_signed = true;
                    vm_regs[*dest as usize] = v;
                }
                Insn::Resize(reg, width) => {
                    let r = *reg as usize;
                    if vm_regs[r].width != *width {
                        let resized = vm_regs[r].resize(*width);
                        vm_regs[r] = resized;
                    }
                }
                Insn::Add(d, l, r) => { vm_regs[*d as usize] = vm_regs[*l as usize].add(&vm_regs[*r as usize]); }
                Insn::Sub(d, l, r) => { vm_regs[*d as usize] = vm_regs[*l as usize].sub(&vm_regs[*r as usize]); }
                Insn::Mul(d, l, r) => { vm_regs[*d as usize] = vm_regs[*l as usize].mul(&vm_regs[*r as usize]); }
                Insn::Div(d, l, r) => { vm_regs[*d as usize] = vm_regs[*l as usize].div(&vm_regs[*r as usize]); }
                Insn::Mod(d, l, r) => { vm_regs[*d as usize] = vm_regs[*l as usize].modulo(&vm_regs[*r as usize]); }
                Insn::BitAnd(d, l, r) => { vm_regs[*d as usize] = vm_regs[*l as usize].bitwise_and(&vm_regs[*r as usize]); }
                Insn::BitOr(d, l, r) => { vm_regs[*d as usize] = vm_regs[*l as usize].bitwise_or(&vm_regs[*r as usize]); }
                Insn::BitXor(d, l, r) => { vm_regs[*d as usize] = vm_regs[*l as usize].bitwise_xor(&vm_regs[*r as usize]); }
                Insn::BitXnor(d, l, r) => { vm_regs[*d as usize] = vm_regs[*l as usize].bitwise_xor(&vm_regs[*r as usize]).bitwise_not(); }
                Insn::LogAnd(d, l, r) => { vm_regs[*d as usize] = vm_regs[*l as usize].logic_and(&vm_regs[*r as usize]); }
                Insn::LogOr(d, l, r) => { vm_regs[*d as usize] = vm_regs[*l as usize].logic_or(&vm_regs[*r as usize]); }
                Insn::Eq(d, l, r) => { vm_regs[*d as usize] = vm_regs[*l as usize].is_equal(&vm_regs[*r as usize]); }
                Insn::Neq(d, l, r) => { vm_regs[*d as usize] = vm_regs[*l as usize].is_not_equal(&vm_regs[*r as usize]); }
                Insn::CaseEq(d, l, r) => { vm_regs[*d as usize] = vm_regs[*l as usize].case_eq(&vm_regs[*r as usize]); }
                Insn::Lt(d, l, r) => { vm_regs[*d as usize] = vm_regs[*l as usize].less_than(&vm_regs[*r as usize]); }
                Insn::Leq(d, l, r) => { vm_regs[*d as usize] = vm_regs[*l as usize].less_equal(&vm_regs[*r as usize]); }
                Insn::Gt(d, l, r) => { vm_regs[*d as usize] = vm_regs[*l as usize].greater_than(&vm_regs[*r as usize]); }
                Insn::Geq(d, l, r) => { vm_regs[*d as usize] = vm_regs[*l as usize].greater_equal(&vm_regs[*r as usize]); }
                Insn::Shl(d, l, r) => { vm_regs[*d as usize] = vm_regs[*l as usize].shift_left(&vm_regs[*r as usize]); }
                Insn::Shr(d, l, r) => { vm_regs[*d as usize] = vm_regs[*l as usize].shift_right(&vm_regs[*r as usize]); }
                Insn::AShr(d, l, r) => { vm_regs[*d as usize] = vm_regs[*l as usize].arith_shift_right(&vm_regs[*r as usize]); }
                Insn::BitNot(d, s) => { vm_regs[*d as usize] = vm_regs[*s as usize].bitwise_not(); }
                Insn::LogNot(d, s) => { vm_regs[*d as usize] = vm_regs[*s as usize].logic_not(); }
                Insn::Negate(d, s) => {
                    let w = vm_regs[*s as usize].width;
                    let mut r = Value::zero(w).sub(&vm_regs[*s as usize]).resize(w);
                    r.is_signed = true;
                    vm_regs[*d as usize] = r;
                }
                Insn::ReduceAnd(d, s) => { vm_regs[*d as usize] = vm_regs[*s as usize].reduce_and(); }
                Insn::ReduceOr(d, s) => { vm_regs[*d as usize] = vm_regs[*s as usize].reduce_or(); }
                Insn::ReduceXor(d, s) => { vm_regs[*d as usize] = vm_regs[*s as usize].reduce_xor(); }
                Insn::BitSelect(d, base, idx) => {
                    let i = vm_regs[*idx as usize].to_u64().unwrap_or(0) as usize;
                    vm_regs[*d as usize] = vm_regs[*base as usize].bit_select(i);
                }
                Insn::RangeSelect(d, base, l, r) => {
                    let li = vm_regs[*l as usize].to_u64().unwrap_or(0) as usize;
                    let ri = vm_regs[*r as usize].to_u64().unwrap_or(0) as usize;
                    vm_regs[*d as usize] = vm_regs[*base as usize].range_select(li, ri);
                }
                Insn::Concat(d, part_regs) => {
                    let parts: Vec<Value> = part_regs.iter()
                        .map(|r| vm_regs[*r as usize].clone())
                        .collect();
                    vm_regs[*d as usize] = Value::concat(&parts);
                }
                Insn::BranchIfFalse(reg, target) => {
                    if !vm_regs[*reg as usize].is_true() { pc = *target as usize; continue; }
                }
                Insn::Select(dest, cond, then_r, else_r) => {
                    let v = if vm_regs[*cond as usize].has_unknown() {
                        vm_regs[*then_r as usize].merge_unknown(&vm_regs[*else_r as usize])
                    } else if vm_regs[*cond as usize].is_true() {
                        vm_regs[*then_r as usize].clone()
                    } else {
                        vm_regs[*else_r as usize].clone()
                    };
                    vm_regs[*dest as usize] = v;
                }
                Insn::Jump(target) => { pc = *target as usize; continue; }
                Insn::NbaAssign(sig_id, val_reg, width) => {
                    let val = vm_regs[*val_reg as usize].resize_for_assign(*width);
                    nba_out.push(NbaFast { signal_id: *sig_id, value: val });
                }
                Insn::NbaAssignRange(sig_id, hi, lo, val_reg) => {
                    let (low, high) = if hi >= lo { (*lo, *hi) } else { (*hi, *lo) };
                    let w = high - low + 1;
                    let val = vm_regs[*val_reg as usize].resize(w);
                    let existing = nba_out.iter().rposition(|n| n.signal_id == *sig_id);
                    let mut new_val = if let Some(i) = existing { nba_out[i].value.clone() } else { signal_table[*sig_id].clone() };
                    for bit_pos in low..=high {
                        new_val.set_bit(bit_pos as usize, val.get_bit((bit_pos - low) as usize));
                    }
                    if let Some(i) = existing { nba_out[i].value = new_val; }
                    else { nba_out.push(NbaFast { signal_id: *sig_id, value: new_val }); }
                }
                Insn::NbaAssignBitDyn(sig_id, idx_reg, val_reg) => {
                    let idx = vm_regs[*idx_reg as usize].to_u64().unwrap_or(0) as usize;
                    let bit = vm_regs[*val_reg as usize].get_bit(0);
                    let existing = nba_out.iter().rposition(|n| n.signal_id == *sig_id);
                    let mut new_val = if let Some(i) = existing { nba_out[i].value.clone() } else { signal_table[*sig_id].clone() };
                    new_val.set_bit(idx, bit);
                    if let Some(i) = existing { nba_out[i].value = new_val; }
                    else { nba_out.push(NbaFast { signal_id: *sig_id, value: new_val }); }
                }
                Insn::LoadArrayElem(dest, array_name, idx_reg) => {
                    // Isolated/parallel path has no mutable access to the
                    // array_elem_ids cache, so keep the format-based lookup
                    // here. Sequential path (exec_insns) uses the cache.
                    let idx = vm_regs[*idx_reg as usize].to_u64().unwrap_or(0);
                    let elem_name = format!("{}[{}]", array_name, idx);
                    if let Some(&eid) = signal_name_to_id.get(elem_name.as_str()) {
                        vm_regs[*dest as usize] = signal_table[eid].clone();
                    } else {
                        vm_regs[*dest as usize] = Value::new(1);
                    }
                }
                Insn::NbaAssignArray(array_name, idx_reg, val_reg, width) => {
                    let idx = vm_regs[*idx_reg as usize].to_u64().unwrap_or(0);
                    let elem_name = format!("{}[{}]", array_name, idx);
                    if let Some(&eid) = signal_name_to_id.get(elem_name.as_str()) {
                        let val = vm_regs[*val_reg as usize].resize(*width);
                        nba_out.push(NbaFast { signal_id: eid, value: val });
                    }
                }
                Insn::Move(d, s) => { vm_regs[*d as usize] = vm_regs[*s as usize].clone(); }
                Insn::SetSigned(reg) => { vm_regs[*reg as usize].is_signed = true; }
                // These should never appear in parallel-eligible blocks
                Insn::StmtFallback(..) | Insn::BlockingAssign(..) |
                Insn::BlockingAssignRange(..) | Insn::BlockingAssignRangeDyn(..) |
                Insn::BlockingAssignBitDyn(..) | Insn::NbaAssignRangeDyn(..) => {
                    unreachable!("parallel block should not contain fallback/blocking/NbaRangeDyn instructions");
                }
                Insn::Nop => {}
            }
            pc += 1;
        }
        nba_out
    }

    /// Execute a compiled bytecode block. Returns true if executed successfully.
    #[inline]
    fn exec_bytecode(&mut self, block_idx: usize) -> bool {
        // Fast path: if we JIT-compiled this block, call the native fn
        // directly. Zero-cost when the jit feature is off (jit_fns stays
        // empty; index returns None which short-circuits).
        if let Some(Some(jit_fn)) = self.jit_fns.get(block_idx).copied() {
            let self_ptr: *mut u8 = self as *mut Self as *mut u8;
            let rc = unsafe { jit_fn(self_ptr) };
            if rc == 0 {
                self.prof_insns_executed += self.compiled_edge_blocks[block_idx]
                    .as_ref().map(|cb| cb.instructions.len() as u64).unwrap_or(0);
                return true;
            }
            // Non-zero return: block asked for interpreter fallback
            // (e.g. encountered a Wide/4-state path). Mark this block
            // un-JITtable for the rest of the run and fall through.
            self.jit_fns[block_idx] = None;
        }
        // Interpreter path.
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
        let mut local_count: u64 = 0;
        while pc < len {
            local_count += 1;
            match &insns[pc] {
                Insn::LoadConst(dest, val) => {
                    // Reuse vm_regs[dest]'s buffer via copy_from — no alloc.
                    self.vm_regs[*dest as usize].copy_from(val.as_ref());
                }
                Insn::LoadSignal(dest, sig_id) => {
                    let d = *dest as usize;
                    let s = *sig_id;
                    // Reuse vm_regs[d]'s buffer; disjoint fields of self.
                    self.vm_regs[d].copy_from(&self.signal_table[s]);
                }
                Insn::LoadSignalSigned(dest, sig_id) => {
                    let d = *dest as usize;
                    let s = *sig_id;
                    self.vm_regs[d].copy_from(&self.signal_table[s]);
                    self.vm_regs[d].is_signed = true;
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
                Insn::Select(dest, cond, then_r, else_r) => {
                    let v = if self.vm_regs[*cond as usize].has_unknown() {
                        let t = self.vm_regs[*then_r as usize].clone();
                        let e = self.vm_regs[*else_r as usize].clone();
                        t.merge_unknown(&e)
                    } else if self.vm_regs[*cond as usize].is_true() {
                        self.vm_regs[*then_r as usize].clone()
                    } else {
                        self.vm_regs[*else_r as usize].clone()
                    };
                    self.vm_regs[*dest as usize] = v;
                }
                Insn::Jump(target) => {
                    pc = *target as usize;
                    continue;
                }
                Insn::NbaAssign(sig_id, val_reg, width) => {
                    let val = self.vm_regs[*val_reg as usize].resize_for_assign(*width);
                    // Update nba_fast_index too so a follow-up partial-range
                    // or bit NBA to the same signal merges into THIS new
                    // whole-value entry, not into a stale earlier partial.
                    self.nba_fast_index.insert(*sig_id, self.nba_fast.len());
                    self.nba_fast.push(NbaFast { signal_id: *sig_id, value: val });
                }
                Insn::NbaAssignRange(sig_id, hi, lo, val_reg) => {
                    // O(1) lookup via nba_fast_index instead of the prior
                    // O(N) `iter().rposition` scan. Mutate the existing
                    // entry in-place (no clone) when we find one — falls
                    // back to seeding from `signal_table[id].clone()` only
                    // for the first NBA to a given signal in this window.
                    let (low, high) = if hi >= lo { (*lo, *hi) } else { (*hi, *lo) };
                    let w = high - low + 1;
                    let val = self.vm_regs[*val_reg as usize].resize(w);
                    let id = *sig_id;
                    if let Some(&i) = self.nba_fast_index.get(&id) {
                        let target = &mut self.nba_fast[i].value;
                        for bit_pos in low..=high {
                            target.set_bit(bit_pos as usize, val.get_bit((bit_pos - low) as usize));
                        }
                    } else {
                        let mut new_val = self.signal_table[id].clone();
                        for bit_pos in low..=high {
                            new_val.set_bit(bit_pos as usize, val.get_bit((bit_pos - low) as usize));
                        }
                        self.nba_fast_index.insert(id, self.nba_fast.len());
                        self.nba_fast.push(NbaFast { signal_id: id, value: new_val });
                    }
                }
                Insn::NbaAssignRangeDyn(sig_id, hi_reg, lo_reg, val_reg) => {
                    let hi_u = self.vm_regs[*hi_reg as usize].to_u64().unwrap_or(0) as u32;
                    let lo_u = self.vm_regs[*lo_reg as usize].to_u64().unwrap_or(0) as u32;
                    let (low, high) = if hi_u >= lo_u { (lo_u, hi_u) } else { (hi_u, lo_u) };
                    let w = high - low + 1;
                    let val = self.vm_regs[*val_reg as usize].resize(w);
                    let id = *sig_id;
                    let sig_w = self.signal_widths[id];
                    let high_eff = high.min(sig_w.saturating_sub(1));
                    if let Some(&i) = self.nba_fast_index.get(&id) {
                        let target = &mut self.nba_fast[i].value;
                        for bit_pos in low..=high_eff {
                            target.set_bit(bit_pos as usize, val.get_bit((bit_pos - low) as usize));
                        }
                    } else {
                        let mut new_val = self.signal_table[id].clone();
                        for bit_pos in low..=high_eff {
                            new_val.set_bit(bit_pos as usize, val.get_bit((bit_pos - low) as usize));
                        }
                        self.nba_fast_index.insert(id, self.nba_fast.len());
                        self.nba_fast.push(NbaFast { signal_id: id, value: new_val });
                    }
                }
                Insn::NbaAssignBitDyn(sig_id, idx_reg, val_reg) => {
                    let idx = self.vm_regs[*idx_reg as usize].to_u64().unwrap_or(0) as usize;
                    let bit = self.vm_regs[*val_reg as usize].get_bit(0);
                    let id = *sig_id;
                    if let Some(&i) = self.nba_fast_index.get(&id) {
                        self.nba_fast[i].value.set_bit(idx, bit);
                    } else {
                        let mut new_val = self.signal_table[id].clone();
                        new_val.set_bit(idx, bit);
                        self.nba_fast_index.insert(id, self.nba_fast.len());
                        self.nba_fast.push(NbaFast { signal_id: id, value: new_val });
                    }
                }
                Insn::StmtFallback(payload) => {
                    let s = payload.0.clone();
                    self.prof_fallback_insns += 1;
                    let r = payload.1;
                    let t0 = std::time::Instant::now();
                    self.exec_statement(&s);
                    let elapsed = t0.elapsed().as_nanos() as u64;
                    let e = self.prof_fallback_by_reason.entry(r).or_insert((0u64, 0u64));
                    e.0 += 1;
                    e.1 += elapsed;
                }
                Insn::BlockingAssign(sig_id, val_reg, width) => {
                    let mut val = self.vm_regs[*val_reg as usize].resize(*width);
                    val.is_signed = self.signal_signed[*sig_id];
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
                Insn::BlockingAssignBitDyn(sig_id, idx_reg, val_reg) => {
                    // Hot path on c910: a single bit in some packed signal
                    // (e.g. `mem[i][7:0] = 0` after parser flatten). The
                    // previous version cloned the *entire* signal_table[id]
                    // — a wide signal can be many KB — just to flip one
                    // bit, then full-value compared to detect change. For
                    // wipe loops that touch thousands of cells per cycle
                    // this dominated allocator + memcpy time. Now: read
                    // current bit, skip the write if it matches; otherwise
                    // set in-place and mark dirty.
                    let idx = self.vm_regs[*idx_reg as usize].to_u64().unwrap_or(0) as usize;
                    let bit = self.vm_regs[*val_reg as usize].get_bit(0);
                    let id = *sig_id;
                    if idx < self.signal_widths[id] as usize {
                        let cur = self.signal_table[id].get_bit(idx);
                        if cur != bit {
                            self.signal_table[id].set_bit(idx, bit);
                            self.signal_table[id].is_signed = self.signal_signed[id];
                            if !self.dirty_signals[id] {
                                self.dirty_signals[id] = true;
                                self.dirty_list.push(id);
                            }
                            self.dirty_any = true;
                            self.table_modified = true;
                        }
                    }
                }
                Insn::BlockingAssignRange(sig_id, hi, lo, val_reg) => {
                    // Same in-place pattern as BlockingAssignBitDyn — the
                    // previous code cloned the whole signal Value to flip
                    // a few bits.
                    let (low, high) = if hi >= lo { (*lo, *hi) } else { (*hi, *lo) };
                    let w = high - low + 1;
                    let val = self.vm_regs[*val_reg as usize].resize(w);
                    let id = *sig_id;
                    let mut changed = false;
                    for bit_pos in low..=high {
                        let src_bit = val.get_bit((bit_pos - low) as usize);
                        if self.signal_table[id].get_bit(bit_pos as usize) != src_bit {
                            self.signal_table[id].set_bit(bit_pos as usize, src_bit);
                            changed = true;
                        }
                    }
                    if changed {
                        self.signal_table[id].is_signed = self.signal_signed[id];
                        if !self.dirty_signals[id] {
                            self.dirty_signals[id] = true;
                            self.dirty_list.push(id);
                        }
                        self.dirty_any = true;
                        self.table_modified = true;
                    }
                }
                Insn::BlockingAssignRangeDyn(sig_id, hi_reg, lo_reg, val_reg) => {
                    let hi = self.vm_regs[*hi_reg as usize].to_u64().unwrap_or(0) as u32;
                    let lo = self.vm_regs[*lo_reg as usize].to_u64().unwrap_or(0) as u32;
                    let (low, high) = if hi >= lo { (lo, hi) } else { (hi, lo) };
                    let w = high - low + 1;
                    let val = self.vm_regs[*val_reg as usize].resize(w);
                    let id = *sig_id;
                    let mut changed = false;
                    for bit_pos in low..=high {
                        let src_bit = val.get_bit((bit_pos - low) as usize);
                        if self.signal_table[id].get_bit(bit_pos as usize) != src_bit {
                            self.signal_table[id].set_bit(bit_pos as usize, src_bit);
                            changed = true;
                        }
                    }
                    if changed {
                        self.signal_table[id].is_signed = self.signal_signed[id];
                        if !self.dirty_signals[id] {
                            self.dirty_signals[id] = true;
                            self.dirty_list.push(id);
                        }
                        self.dirty_any = true;
                        self.table_modified = true;
                    }
                }
                Insn::LoadArrayElem(dest, array_name, idx_reg) => {
                    // Use the dense array_elem_ids Vec cache — eliminates
                    // the per-call `format!("{}[{}]", name, idx)` allocation
                    // and HashMap lookup, which alone cost ~500ns/call and
                    // dominated c910's per-cycle bytecode execution
                    // (thousands of array reads per clock). The cache is
                    // populated lazily by `get_array_elem_id` on first
                    // miss; subsequent calls are a Vec index away.
                    let idx = self.vm_regs[*idx_reg as usize].to_u64().unwrap_or(0) as i64;
                    if let Some(eid) = self.get_array_elem_id(array_name, idx) {
                        self.vm_regs[*dest as usize].copy_from(&self.signal_table[eid]);
                    } else {
                        self.vm_regs[*dest as usize] = Value::new(1);
                    }
                }
                Insn::NbaAssignArray(array_name, idx_reg, val_reg, width) => {
                    let idx = self.vm_regs[*idx_reg as usize].to_u64().unwrap_or(0) as i64;
                    if let Some(eid) = self.get_array_elem_id(array_name, idx) {
                        let val = self.vm_regs[*val_reg as usize].resize(*width);
                        self.nba_fast_index.insert(eid, self.nba_fast.len());
                        self.nba_fast.push(NbaFast { signal_id: eid, value: val });
                    }
                }
                Insn::Move(d, s) => {
                    let d = *d as usize;
                    let s = *s as usize;
                    if d != s {
                        // Split-borrow so we can copy_from in place without cloning.
                        let (lo, hi) = if d < s {
                            let (a, b) = self.vm_regs.split_at_mut(s);
                            (&mut a[d], &b[0])
                        } else {
                            let (a, b) = self.vm_regs.split_at_mut(d);
                            (&mut b[0], &a[s])
                        };
                        lo.copy_from(hi);
                    }
                }
                Insn::SetSigned(reg) => {
                    self.vm_regs[*reg as usize].is_signed = true;
                }
                Insn::Nop => {}
            }
            pc += 1;
        }
        self.prof_insns_executed += local_count;
    }

    fn build_comb_entries(&mut self) {
        let mut entries = Vec::new();

        // Continuous assigns
        for ca in &self.module.continuous_assigns {
            let mut reads = HashSet::new();
            let mut writes = HashSet::new();
            Self::collect_expr_reads(&ca.rhs, &self.module, &mut reads);
            Self::collect_lhs_writes(&ca.lhs, &self.module, &mut writes);
            // Detect identity assigns: assign dst = src (simple signal-to-signal copy)
            let direct_copy = if let (ExprKind::Ident(lhs_hier), ExprKind::Ident(rhs_hier)) = (&ca.lhs.kind, &ca.rhs.kind) {
                let dst_name = Self::resolve_hier_name_static(lhs_hier, &self.module);
                let src_name = Self::resolve_hier_name_static(rhs_hier, &self.module);
                if let (Some(&dst_id), Some(&src_id)) = (self.signal_name_to_id.get(dst_name.as_str()), self.signal_name_to_id.get(src_name.as_str())) {
                    let width = self.signal_widths[dst_id];
                    if width == self.signal_widths[src_id] {
                        Some(CombItem::DirectCopy { dst_id, src_id, width })
                    } else { None }
                } else { None }
            } else { None };

            let scope_hint = self
                .infer_contassign_scope_hint(&ca.lhs, &ca.rhs)
                .or_else(|| self.infer_scope_from_rw_sets(&writes, &reads));

            // Resolve write targets, retrying with scope_hint for bare names
            let wids: Vec<usize> = writes.iter()
                .filter_map(|w| {
                    if let Some(&id) = self.signal_name_to_id.get(w.as_str()) { return Some(id); }
                    if let Some(scope) = &scope_hint {
                        let qualified = format!("{}.{}", scope, w);
                        if let Some(&id) = self.signal_name_to_id.get(qualified.as_str()) { return Some(id); }
                    }
                    None
                })
                .collect();

            // For bare-identifier LHS, compile the RHS and use BlockingAssign
            // (whole-signal write). For BitSelect/PartSelect/Concat LHS, use
            // compile_cont_assign_lhs which routes through compile_blocking_target
            // so only the addressed sub-range is updated — essential for yosys
            // gate-level netlists whose per-bit assigns `assign d[0] = expr;`
            // would otherwise clobber the whole vector wire.
            let lhs_is_bare_ident = matches!(ca.lhs.kind, ExprKind::Ident(_));
            // Try fused-gate fast path first: recognizes yosys patterns like
            // `assign d[0] = a & b` or `assign d[0] = ~(a & b)` — executes
            // without VM dispatch, just bit reads + 4-state combinator + set_bit.
            let fused = self.try_build_fused_gate(&ca.lhs, &ca.rhs, scope_hint.as_deref());
            let item = if let Some(op) = fused {
                CombItem::FusedGate { op }
            } else if let Some(dc) = direct_copy {
                dc
            } else if wids.len() == 1 && lhs_is_bare_ident {
                let dst_id = wids[0];
                let width = self.signal_widths[dst_id];
                let mut compiler = super::bytecode::BytecodeCompiler::new(
                    &self.signal_name_to_id,
                    &self.signal_signed,
                    &self.signal_widths,
                    &self.module.arrays,
                    &self.widths,
                );
                compiler.set_scope_hint(scope_hint.clone());
                compiler.set_params(&self.module.parameters);
                if compiler.compile_cont_assign(&ca.rhs, dst_id, width) {
                    CombItem::CompiledContAssign { compiled: compiler.finish() }
                } else {
                    CombItem::ContAssign { lhs: ca.lhs.clone(), rhs: ca.rhs.clone() }
                }
            } else if !lhs_is_bare_ident {
                // Sub-range LHS: try bytecode compile so bit/range writes run
                // at VM speed instead of through the interpreted assign_value.
                let mut compiler = super::bytecode::BytecodeCompiler::new(
                    &self.signal_name_to_id,
                    &self.signal_signed,
                    &self.signal_widths,
                    &self.module.arrays,
                    &self.widths,
                );
                compiler.set_scope_hint(scope_hint.clone());
                compiler.set_params(&self.module.parameters);
                let lhs_w = compiler.infer_lhs_width_pub(&ca.lhs);
                if lhs_w > 0 && compiler.compile_cont_assign_lhs(&ca.lhs, &ca.rhs, lhs_w) {
                    CombItem::CompiledContAssign { compiled: compiler.finish() }
                } else {
                    CombItem::ContAssign { lhs: ca.lhs.clone(), rhs: ca.rhs.clone() }
                }
            } else {
                CombItem::ContAssign { lhs: ca.lhs.clone(), rhs: ca.rhs.clone() }
            };

            // Resolve reads, retrying with scope_hint prefix for bare local names.
            // Without this, references like `mem_valid` from a top-level cont_assign
            // would not match `testbench.mem_valid` in signal_name_to_id, and the
            // entry would be marked has_unresolved_reads=true and re-fired every
            // settle iteration.
            let mut rids: Vec<usize> = Vec::with_capacity(reads.len());
            let mut unresolved_count = 0usize;
            for r in &reads {
                if let Some(&id) = self.signal_name_to_id.get(r.as_str()) {
                    rids.push(id);
                    continue;
                }
                let mut found = false;
                if let Some(scope) = &scope_hint {
                    let qualified = format!("{}.{}", scope, r);
                    if let Some(&id) = self.signal_name_to_id.get(qualified.as_str()) {
                        rids.push(id);
                        found = true;
                    }
                }
                if !found { unresolved_count += 1; }
            }
            let has_unresolved_reads = unresolved_count > 0;
            if has_unresolved_reads && std::env::var("XEZIM_DUMP_UNRESOLVED").is_ok() {
                let unresolved: Vec<&String> = reads.iter()
                    .filter(|r| {
                        if self.signal_name_to_id.contains_key((*r).as_str()) { return false; }
                        if let Some(scope) = &scope_hint {
                            if self.signal_name_to_id.contains_key(format!("{}.{}", scope, r).as_str()) {
                                return false;
                            }
                        }
                        true
                    })
                    .collect();
                eprintln!("[UNRES] cont_assign scope={:?} unresolved={:?} resolved={}/{}",
                    scope_hint, unresolved, rids.len(), reads.len());
            }
            entries.push(CombEntry {
                item,
                scope_hint,
                read_signal_ids: rids,
                write_signal_ids: wids,
                has_unresolved_reads,
            });
        }

        // Always @* and always_comb blocks
        for ab in &self.module.always_blocks {
            if matches!(ab.kind, AlwaysKind::AlwaysComb | AlwaysKind::Always) {
                let is_always_comb = ab.kind == AlwaysKind::AlwaysComb;
                let mut reads = HashSet::new();
                let mut writes = HashSet::new();
                Self::collect_stmt_reads(&ab.stmt, &self.module, &mut reads, &mut writes);
                let wids: Vec<usize> = writes.iter()
                    .filter_map(|w| self.signal_name_to_id.get(w.as_str()).copied())
                    .collect();
                // For comb-sensitivity purposes, exclude signals that are written by
                // this block. Loop variables and local temps are written-then-read
                // within a single execution; external re-triggering on them would
                // cause infinite settle loops (e.g. `for (j = 0; j < N; j++)` in an
                // always @* block).
                let sens_reads: HashSet<String> = reads.difference(&writes).cloned().collect();
                let rids: Vec<usize> = sens_reads.iter()
                    .filter_map(|r| self.signal_name_to_id.get(r.as_str()).copied())
                    .collect();
                // Unresolved reads in always @* are usually parameters, genvars,
                // typedefs, or loop-local integer variables — none of which change
                // at runtime. Don't mark the block as has_unresolved_reads for
                // those cases; it causes the block to fire every settle iteration
                // and can produce infinite settle loops when the block itself
                // writes temporary variables (loop indices, scratch regs).
                let has_unresolved_reads = false;
                let scope_hint = self.infer_scope_from_rw_sets(&writes, &reads);
                // Try bytecode-compiling the comb always block. On success
                // the settle path skips exec_statement entirely and runs
                // the flat Insn stream via exec_insns.
                let item = {
                    let mut compiler = super::bytecode::BytecodeCompiler::new(
                        &self.signal_name_to_id,
                        &self.signal_signed,
                        &self.signal_widths,
                        &self.module.arrays,
                        &self.widths,
                    );
                    compiler.set_scope_hint(scope_hint.clone());
                    compiler.set_tasks(&self.module.tasks);
                    compiler.set_params(&self.module.parameters);
                    // Enable AST fallback so partially-unsupported constructs
                    // compile to StmtFallback insns instead of failing the
                    // whole block. Simple parts still benefit from bytecode
                    // signal-ID pre-resolution.
                    compiler.set_ast_fallback(true);
                    if compiler.compile_stmt(&ab.stmt) {
                        CombItem::CompiledAlwaysBlock { compiled: compiler.finish(), is_always_comb }
                    } else {
                        CombItem::AlwaysBlock { stmt: ab.stmt.clone(), is_always_comb }
                    }
                };
                entries.push(CombEntry {
                    item,
                    scope_hint,
                    read_signal_ids: rids,
                    write_signal_ids: wids,
                    has_unresolved_reads,
                });
            }
        }


        // Topologically reorder `entries` so that writers come before readers
        // where possible. This collapses feedforward chains to 1 settle iter.
        // Cycles are broken arbitrarily; feedback still needs multi-iter.
        let num_signals = self.signal_table.len();
        {
            let n = entries.len();
            let mut writers_by_sig: Vec<Vec<usize>> = vec![Vec::new(); num_signals];
            for (idx, entry) in entries.iter().enumerate() {
                for &sig_id in &entry.write_signal_ids {
                    if sig_id < num_signals {
                        writers_by_sig[sig_id].push(idx);
                    }
                }
            }
            // Build forward graph: for each entry B, predecessors = writers of its read signals.
            let mut indeg = vec![0usize; n];
            let mut succs: Vec<Vec<usize>> = vec![Vec::new(); n];
            // Use a temporary visited matrix row-by-row to avoid double-counting edges.
            let mut seen_pred: Vec<u32> = vec![u32::MAX; n];
            for b in 0..n {
                for &sid in &entries[b].read_signal_ids {
                    if sid >= num_signals { continue; }
                    for &a in &writers_by_sig[sid] {
                        if a == b { continue; }
                        if seen_pred[a] == b as u32 { continue; }
                        seen_pred[a] = b as u32;
                        succs[a].push(b);
                        indeg[b] += 1;
                    }
                }
            }
            // Kahn's: pick zero-indegree, remove. If cycle, pick lowest indegree.
            let mut new_order: Vec<usize> = Vec::with_capacity(n);
            let mut placed = vec![false; n];
            let mut ready: Vec<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
            while new_order.len() < n {
                if let Some(a) = ready.pop() {
                    if placed[a] { continue; }
                    placed[a] = true;
                    new_order.push(a);
                    for &b in &succs[a] {
                        if placed[b] { continue; }
                        if indeg[b] > 0 { indeg[b] -= 1; }
                        if indeg[b] == 0 { ready.push(b); }
                    }
                } else {
                    // Cycle: pick any remaining node with min indegree.
                    let mut best = usize::MAX;
                    let mut best_d = usize::MAX;
                    for i in 0..n {
                        if !placed[i] && indeg[i] < best_d {
                            best_d = indeg[i]; best = i;
                        }
                    }
                    if best == usize::MAX { break; }
                    placed[best] = true;
                    new_order.push(best);
                    indeg[best] = 0;
                    for &b in &succs[best] {
                        if placed[b] { continue; }
                        if indeg[b] > 0 { indeg[b] -= 1; }
                        if indeg[b] == 0 { ready.push(b); }
                    }
                }
            }
            // Apply permutation only if we got a real full permutation.
            let valid_permutation = if new_order.len() == n {
                let mut seen = vec![false; n];
                let mut ok = true;
                for &i in &new_order {
                    if i >= n || seen[i] {
                        ok = false;
                        break;
                    }
                    seen[i] = true;
                }
                ok
            } else {
                false
            };
            if valid_permutation {
                let mut permuted: Vec<CombEntry> = Vec::with_capacity(n);
                // Take entries out via swap_remove would mangle indices; rebuild by index.
                // Use an Option<CombEntry> trick.
                let mut slots: Vec<Option<CombEntry>> = entries.into_iter().map(Some).collect();
                for &i in &new_order {
                    permuted.push(slots[i].take().unwrap());
                }
                entries = permuted;
            } else if !new_order.is_empty() && n > 0 {
                eprintln!(
                    "[xezim][comb] skipping invalid reorder permutation: len={} entries={}",
                    new_order.len(),
                    n
                );
            }
        }

        // Build reverse dependency index by signal ID using final entry
        // order. CSR layout: counts → prefix-sum → fill, all in flat
        // u32 Vecs. Avoids 585K × 24 B of empty Vec headers and 585K
        // separate small allocations.
        let mut counts: Vec<u32> = vec![0u32; num_signals + 1];
        for entry in entries.iter() {
            for &sig_id in &entry.read_signal_ids {
                if sig_id < num_signals {
                    counts[sig_id + 1] += 1;
                }
            }
        }
        // In-place prefix sum: counts[i] becomes start-offset for signal i.
        let mut dep_offsets = counts;
        for i in 1..dep_offsets.len() {
            dep_offsets[i] += dep_offsets[i - 1];
        }
        let total = dep_offsets[num_signals] as usize;
        let mut dep_entries: Vec<u32> = vec![0u32; total];
        // `cursor[id]` tracks the next write position for signal `id`;
        // initialized to the start-offset and bumped per insert.
        let mut cursor: Vec<u32> = dep_offsets[..num_signals].to_vec();
        for (idx, entry) in entries.iter().enumerate() {
            for &sig_id in &entry.read_signal_ids {
                if sig_id < num_signals {
                    let pos = cursor[sig_id] as usize;
                    dep_entries[pos] = idx as u32;
                    cursor[sig_id] += 1;
                }
            }
        }
        self.comb_dep_offsets = dep_offsets;
        self.comb_dep_entries = dep_entries;
        let dc_count = entries.iter().filter(|e| matches!(&e.item, CombItem::DirectCopy { .. })).count();
        let cca_count = entries.iter().filter(|e| matches!(&e.item, CombItem::CompiledContAssign { .. })).count();
        let ca_count = entries.iter().filter(|e| matches!(&e.item, CombItem::ContAssign { .. })).count();
        let ab_count = entries.iter().filter(|e| matches!(&e.item, CombItem::AlwaysBlock { .. })).count();
        let fg_count = entries.iter().filter(|e| matches!(&e.item, CombItem::FusedGate { .. })).count();
        if dc_count > 0 || fg_count > 0 {
            sim_dbg_eprintln!("[OPT] comb entries: {} direct-copy, {} compiled-ca, {} ast-ca, {} always-block, {} fused-gate", dc_count, cca_count, ca_count, ab_count, fg_count);
            sim_dbg_eprintln!("[OPT] edge blocks: {}, event_waiters: {}", self.edge_blocks.len(), self.event_waiters.len());
        }
        self.comb_unresolved_idx = entries.iter().enumerate()
            .filter_map(|(i, e)| if e.has_unresolved_reads { Some(i) } else { None })
            .collect();
        self.comb_time0_idx = entries.iter().enumerate()
            .filter_map(|(i, e)| {
                let always_comb = matches!(&e.item,
                    CombItem::AlwaysBlock { is_always_comb: true, .. }
                    | CombItem::CompiledAlwaysBlock { is_always_comb: true, .. });
                if e.read_signal_ids.is_empty() || always_comb { Some(i) } else { None }
            })
            .collect();
        self.comb_entries = entries;
        // Drop AST storage for items we've consumed into comb_entries.
        // continuous_assigns live on in CombItem::ContAssign (fallback) or
        // as DirectCopy / FusedGate / CompiledContAssign; the source Vec in
        // the module is no longer read. Same for combinational always blocks
        // — edge-sensitive ones were moved into self.edge_blocks earlier.
        self.module.continuous_assigns = Vec::new();
        self.module.always_blocks = Vec::new();
    }

    fn collect_leaf_idents(expr: &Expression, out: &mut HashSet<String>) {
        match &expr.kind {
            ExprKind::Ident(hier) => {
                if hier.path.len() == 1 {
                    out.insert(hier.path[0].name.name.clone());
                }
            }
            ExprKind::Index { expr, index } => {
                Self::collect_leaf_idents(expr, out);
                Self::collect_leaf_idents(index, out);
            }
            ExprKind::RangeSelect { expr, left, right, .. } => {
                Self::collect_leaf_idents(expr, out);
                Self::collect_leaf_idents(left, out);
                Self::collect_leaf_idents(right, out);
            }
            ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                Self::collect_leaf_idents(operand, out);
            }
            ExprKind::Binary { left, right, .. } => {
                Self::collect_leaf_idents(left, out);
                Self::collect_leaf_idents(right, out);
            }
            ExprKind::Conditional { condition, then_expr, else_expr } => {
                Self::collect_leaf_idents(condition, out);
                Self::collect_leaf_idents(then_expr, out);
                Self::collect_leaf_idents(else_expr, out);
            }
            ExprKind::Concatenation(parts) => {
                for p in parts {
                    Self::collect_leaf_idents(p, out);
                }
            }
            ExprKind::AssignmentPattern(parts) => {
                for p in parts {
                    Self::collect_leaf_idents(p.expr(), out);
                }
            }
            ExprKind::Replication { count, exprs } => {
                Self::collect_leaf_idents(count, out);
                for e in exprs {
                    Self::collect_leaf_idents(e, out);
                }
            }
            ExprKind::Call { func, args } => {
                Self::collect_leaf_idents(func, out);
                for a in args {
                    Self::collect_leaf_idents(a, out);
                }
            }
            ExprKind::SystemCall { args, .. } => {
                for a in args {
                    Self::collect_leaf_idents(a, out);
                }
            }
            ExprKind::MemberAccess { expr, .. } => {
                Self::collect_leaf_idents(expr, out);
            }
            _ => {}
        }
    }

    fn infer_contassign_scope_hint(&self, lhs: &Expression, rhs: &Expression) -> Option<String> {
        let ident_raw = |expr: &Expression| -> Option<String> {
            if let ExprKind::Ident(hier) = &expr.kind {
                Some(
                    hier.path
                        .iter()
                        .map(|s| s.name.name.as_str())
                        .collect::<Vec<_>>()
                        .join("."),
                )
            } else {
                None
            }
        };
        let ident_leaf = |expr: &Expression| -> Option<String> {
            if let ExprKind::Ident(hier) = &expr.kind {
                if hier.path.len() == 1 {
                    let name = hier.path[0].name.name.clone();
                    if !name.contains('.') {
                        return Some(name);
                    }
                }
            }
            None
        };
        let parent_n = |name: &str, n: usize| -> Option<String> {
            let mut cur = name.rsplit_once('.').map(|(p, _)| p.to_string())?;
            for _ in 0..n {
                cur = cur.rsplit_once('.').map(|(p, _)| p.to_string())?;
            }
            Some(cur)
        };

        let lhs_raw = ident_raw(lhs);
        let rhs_raw = ident_raw(rhs);
        let lhs_leaf_opt = ident_leaf(lhs);
        let rhs_leaf_opt = ident_leaf(rhs);

        // Common port-connection form after inlining:
        //   child.input  = parent_signal;
        //   parent_signal = child.output;
        // Use parent instance scope to resolve the unqualified side.
        if lhs_leaf_opt.is_none() && rhs_leaf_opt.is_some() {
            if let Some(raw) = lhs_raw.as_deref() {
                if let Some(scope) = parent_n(raw, 1) {
                    return Some(scope);
                }
            }
        }
        if lhs_leaf_opt.is_some() && rhs_leaf_opt.is_none() {
            if let Some(raw) = rhs_raw.as_deref() {
                if let Some(scope) = parent_n(raw, 1) {
                    return Some(scope);
                }
            }
        }

        let lhs_leaf = lhs_leaf_opt?;
        let suffix = format!(".{}", lhs_leaf);
        let mut leaves = HashSet::new();
        Self::collect_leaf_idents(lhs, &mut leaves);
        Self::collect_leaf_idents(rhs, &mut leaves);
        if leaves.is_empty() {
            return None;
        }

        let mut best_parent: Option<String> = None;
        let mut best_score = 0usize;
        let mut best_depth = 0usize;
        for full_name in self.signal_name_to_id.keys() {
            let Some(parent) = full_name.strip_suffix(&suffix) else { continue };
            let mut score = 0usize;
            for leaf in &leaves {
                let candidate = format!("{}.{}", parent, leaf);
                if self.signal_name_to_id.contains_key(candidate.as_str()) {
                    score += 1;
                }
            }
            let depth = parent.split('.').count();
            if score > best_score
                || (score == best_score && depth > best_depth)
                || (score == best_score
                    && depth == best_depth
                    && best_parent.as_ref().is_none_or(|p| parent.len() > p.len()))
            {
                best_parent = Some(parent.to_string());
                best_score = score;
                best_depth = depth;
            }
        }
        if best_score == 0 {
            None
        } else {
            best_parent
        }
    }

    fn infer_scope_from_rw_sets(
        &self,
        writes: &HashSet<String>,
        reads: &HashSet<String>,
    ) -> Option<String> {
        let mut leaves = HashSet::new();
        let mut anchor: Option<String> = None;
        for name in writes {
            if !name.contains('.') && !name.contains('[') {
                if anchor.is_none() {
                    anchor = Some(name.clone());
                }
                leaves.insert(name.clone());
            }
        }
        for name in reads {
            if !name.contains('.') && !name.contains('[') {
                if anchor.is_none() {
                    anchor = Some(name.clone());
                }
                leaves.insert(name.clone());
            }
        }
        let anchor = anchor?;
        let suffix = format!(".{}", anchor);
        let mut best_parent: Option<String> = None;
        let mut best_score = 0usize;
        let mut best_depth = 0usize;
        for full_name in self.signal_name_to_id.keys() {
            let Some(parent) = full_name.strip_suffix(&suffix) else { continue };
            let mut score = 0usize;
            for leaf in &leaves {
                let candidate = format!("{}.{}", parent, leaf);
                if self.signal_name_to_id.contains_key(candidate.as_str()) {
                    score += 1;
                }
            }
            let depth = parent.split('.').count();
            if score > best_score
                || (score == best_score && depth > best_depth)
                || (score == best_score
                    && depth == best_depth
                    && best_parent.as_ref().is_none_or(|p| parent.len() > p.len()))
            {
                best_parent = Some(parent.to_string());
                best_score = score;
                best_depth = depth;
            }
        }
        if best_score == 0 { None } else { best_parent }
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
            ExprKind::Concatenation(exprs) => {
                for e in exprs { Self::collect_expr_reads(e, module, reads); }
            }
            ExprKind::AssignmentPattern(parts) => {
                for p in parts { Self::collect_expr_reads(p.expr(), module, reads); }
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
                match &base.kind {
                    ExprKind::Ident(hier) => {
                        let name = Self::resolve_hier_name_static(hier, module);
                        if let Some((lo, hi, _)) = module.arrays.get(&name) {
                            for i in *lo..=*hi { writes.insert(format!("{}[{}]", name, i)); }
                        } else { writes.insert(name); }
                    }
                    ExprKind::MemberAccess { expr, member } => {
                        if let ExprKind::Ident(hier) = &expr.kind {
                            let mut name = Self::resolve_hier_name_static(hier, module);
                            if !name.is_empty() {
                                name.push('.');
                                name.push_str(&member.name);
                                writes.insert(name);
                            }
                        }
                    }
                    _ => {}
                }
            }
            ExprKind::MemberAccess { expr, member } => {
                if let ExprKind::Ident(hier) = &expr.kind {
                    let mut name = Self::resolve_hier_name_static(hier, module);
                    if !name.is_empty() {
                        name.push('.');
                        name.push_str(&member.name);
                        writes.insert(name);
                    }
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
                        ForInit::Assign { lvalue, rvalue } => {
                            Self::collect_expr_reads(rvalue, module, reads);
                            Self::collect_lhs_writes(lvalue, module, writes);
                        }
                        ForInit::VarDecl { name, init: rvalue, .. } => {
                            Self::collect_expr_reads(rvalue, module, reads);
                            writes.insert(name.name.clone());
                        }
                    }
                }
                if let Some(c) = condition { Self::collect_expr_reads(c, module, reads); }
                // Step expressions are typically i = i + 1, parsed as
                // Binary { op: Assign, left, right }. Collect both reads and
                // LHS writes so loop variables are excluded from sensitivity.
                for s in step {
                    Self::collect_expr_reads(s, module, reads);
                    if let ExprKind::Binary { op: BinaryOp::Assign, left, .. } = &s.kind {
                        Self::collect_lhs_writes(left, module, writes);
                    }
                }
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
    fn resolve_hier_name_static(hier: &HierarchicalIdentifier, _module: &ElaboratedModule) -> String {
        hier.path.iter().map(|s| s.name.name.as_str()).collect::<Vec<_>>().join(".")
    }

    /// Resolve a hier ident to its signal_id, retrying with scope_hint prefix
    /// for bare names. Returns None if unresolved.
    fn resolve_ident_id(&self, hier: &HierarchicalIdentifier, scope_hint: Option<&str>) -> Option<usize> {
        let name = Self::resolve_hier_name_static(hier, &self.module);
        if let Some(&id) = self.signal_name_to_id.get(name.as_str()) { return Some(id); }
        if let Some(scope) = scope_hint {
            let qualified = format!("{}.{}", scope, name);
            if let Some(&id) = self.signal_name_to_id.get(qualified.as_str()) { return Some(id); }
        }
        None
    }


    /// Try to evaluate `expr` as a constant non-negative u64 (for bit-select indices).
    fn try_const_u64(expr: &Expression) -> Option<u64> {
        match &expr.kind {
            ExprKind::Paren(inner) => Self::try_const_u64(inner),
            ExprKind::Number(NumberLiteral::Integer { value, base, .. }) => {
                let r = match base { NumberBase::Binary => 2, NumberBase::Octal => 8, NumberBase::Hex => 16, NumberBase::Decimal => 10 };
                let cleaned: String = value.chars().filter(|c| *c != '_').collect();
                u64::from_str_radix(&cleaned, r).ok()
            }
            _ => None,
        }
    }

    /// Try to resolve `expr` to a single-bit reference (sig_id, bit_index).
    /// Handles bare identifiers (1-bit signals) and constant-index bit-selects.
    fn try_resolve_bit_ref(&self, expr: &Expression, scope_hint: Option<&str>) -> Option<BitRef> {
        match &expr.kind {
            ExprKind::Paren(inner) => self.try_resolve_bit_ref(inner, scope_hint),
            ExprKind::Ident(hier) => {
                let id = self.resolve_ident_id(hier, scope_hint)?;
                if self.signal_widths[id] == 1 {
                    Some(BitRef { sig_id: id as u32, bit: 0 })
                } else { None }
            }
            ExprKind::Index { expr: base, index } => {
                let hier = if let ExprKind::Ident(h) = &base.kind { h } else { return None; };
                // Reject array element access (e.g. `reg mem [3:0]; mem[0]`) —
                // this is NOT a bit-select on a packed vector. Treating it as
                // such would read bit 0 of the whole-array signal storage
                // instead of element 0.
                let name = Self::resolve_hier_name_static(hier, &self.module);
                if self.module.arrays.contains_key(&name) { return None; }
                if let Some(scope) = scope_hint {
                    if self.module.arrays.contains_key(&format!("{}.{}", scope, name)) {
                        return None;
                    }
                }
                let id = self.resolve_ident_id(hier, scope_hint)?;
                let bit = Self::try_const_u64(index)?;
                if (bit as u32) < self.signal_widths[id] {
                    Some(BitRef { sig_id: id as u32, bit: bit as u32 })
                } else { None }
            }
            _ => None,
        }
    }

    /// Attempt to recognize a yosys-style gate pattern and return a fused op.
    /// Only fires when LHS and all RHS leaves are single-bit refs with no SDF delay.
    fn try_build_fused_gate(&self, lhs: &Expression, rhs: &Expression, scope_hint: Option<&str>) -> Option<FusedGate> {
        let dst = self.try_resolve_bit_ref(lhs, scope_hint)?;
        if self.sdf_delays[dst.sig_id as usize] != 0 { return None; }
        // Strip outer parens on rhs
        fn unparen(e: &Expression) -> &Expression {
            if let ExprKind::Paren(inner) = &e.kind { unparen(inner) } else { e }
        }
        let r = unparen(rhs);
        // 1) Conditional (mux): s ? t : e
        if let ExprKind::Conditional { condition, then_expr, else_expr } = &r.kind {
            let s = self.try_resolve_bit_ref(condition, scope_hint)?;
            let t = self.try_resolve_bit_ref(then_expr, scope_hint)?;
            let e = self.try_resolve_bit_ref(else_expr, scope_hint)?;
            return Some(FusedGate::Mux2 { dst, s, t, e });
        }
        // 2) Unary BitNot: ~X where X is leaf or binary
        if let ExprKind::Unary { op: UnaryOp::BitNot, operand } = &r.kind {
            let inner = unparen(operand);
            if let ExprKind::Binary { op, left, right } = &inner.kind {
                let gop = match op {
                    BinaryOp::BitAnd => GateBin::And,
                    BinaryOp::BitOr => GateBin::Or,
                    BinaryOp::BitXor => GateBin::Xor,
                    _ => return None,
                };
                let a = self.try_resolve_bit_ref(left, scope_hint)?;
                let b = self.try_resolve_bit_ref(right, scope_hint)?;
                return Some(FusedGate::Bin2 { dst, a, b, op: gop, invert: true });
            }
            let src = self.try_resolve_bit_ref(operand, scope_hint)?;
            return Some(FusedGate::Buf1 { dst, src, invert: true });
        }
        // 3) Binary: a & b, a | b, a ^ b
        if let ExprKind::Binary { op, left, right } = &r.kind {
            let gop = match op {
                BinaryOp::BitAnd => Some(GateBin::And),
                BinaryOp::BitOr => Some(GateBin::Or),
                BinaryOp::BitXor => Some(GateBin::Xor),
                _ => None,
            };
            if let Some(gop) = gop {
                let a = self.try_resolve_bit_ref(left, scope_hint)?;
                let b = self.try_resolve_bit_ref(right, scope_hint)?;
                return Some(FusedGate::Bin2 { dst, a, b, op: gop, invert: false });
            }
        }
        // 4) Simple buf: leaf
        if let Some(src) = self.try_resolve_bit_ref(r, scope_hint) {
            return Some(FusedGate::Buf1 { dst, src, invert: false });
        }
        None
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
            EventControl::HierIdentifier(expr) => {
                if let ExprKind::Ident(h) = &expr.kind {
                    vec![Sensitivity { signal_name: self.resolve_hier_name(h), edge: EdgeKind::AnyEdge }]
                } else { Vec::new() }
            }
            _ => Vec::new(),
        }
    }

    /// Create an EventWaiter with pre-resolved sensitivity IDs for O(1) edge checking.
    fn make_event_waiter(&self, pid: usize, sens: Vec<Sensitivity>, continuation: Vec<Statement>) -> EventWaiter {
        let resolved: Vec<SensitivityId> = sens.iter().filter_map(|s| {
            self.signal_name_to_id.get(s.signal_name.as_str()).map(|&id| SensitivityId { signal_id: id, edge: s.edge })
        }).collect();
        // `sens` (Vec<Sensitivity>) is consumed for resolution and dropped;
        // EventWaiter only carries the resolved IDs from here on.
        EventWaiter { pid, resolved_sensitivities: resolved, continuation, registered_time: self.time }
    }

    /// Drain pending NBAs and repeatedly snapshot → apply_nba → settle →
    /// check_edges until a round produces neither new edges nor new NBAs.
    ///
    /// Needed when a signal transition commits via NBA whose settled effect
    /// triggers further edge-sensitive blocks. The canonical case is the c910
    /// reset chain: async_cpurst_b negedge fires the sync-chain FF → queues
    /// `cpurst_3ff <= 0` NBA → settle propagates cpurst_b X→0 → every
    /// sub-module's local cpurst_b port (driven by a port-connection cont-
    /// assign) transitions X→0 → their async-reset FFs must fire within the
    /// same iter, otherwise the next iter's snapshot captures prev=0 and the
    /// X→0 edge is lost forever (FF stays X, instructions never retire).
    ///
    /// `cascade_limit` bounds the loop at well-behaved designs; a legitimate
    /// design requiring more than this suggests a combinational loop through
    /// the sync chain that the user needs to address.
    ///
    /// Returns (t_snap, t_nba, t_settle, t_edges) deltas in ns for profiling;
    /// callers that don't need them can ignore the return value.
    fn drain_edge_cascade(&mut self, cascade_limit: u32) -> (u64, u64, u64, u64) {
        let (mut t_snap, mut t_nba, mut t_settle, mut t_edges) = (0u64, 0u64, 0u64, 0u64);
        let mut cascade_iter = 0u32;
        while cascade_iter < cascade_limit {
            if self.nba_fast.is_empty() && self.nba_queue.is_empty() { break; }
            let t0 = std::time::Instant::now();
            self.snapshot_edge_signals();
            t_snap += t0.elapsed().as_nanos() as u64;
            let t0 = std::time::Instant::now();
            self.apply_nba();
            t_nba += t0.elapsed().as_nanos() as u64;
            if self.dirty_any {
                let t0 = std::time::Instant::now();
                self.settle_combinatorial();
                t_settle += t0.elapsed().as_nanos() as u64;
            }
            let t0 = std::time::Instant::now();
            let edges_before = self.prof_edges_fired;
            self.check_edges();
            t_edges += t0.elapsed().as_nanos() as u64;
            if self.prof_edges_fired == edges_before
                && self.nba_fast.is_empty()
                && self.nba_queue.is_empty()
            {
                break;
            }
            cascade_iter += 1;
        }
        (t_snap, t_nba, t_settle, t_edges)
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
        let cascade_limit = self.cascade_limit;
        // Optional periodic progress log — set XEZIM_PROGRESS=<seconds> to
        // emit `[PROGRESS]` lines showing sim_time + wall + iters every N
        // seconds. Useful for investigating long-running designs like c910
        // where $display output is sparse.
        let progress_interval = std::env::var("XEZIM_PROGRESS")
            .ok().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        let mut next_progress = if progress_interval > 0 {
            std::time::Duration::from_secs(progress_interval)
        } else { std::time::Duration::MAX };
        while !self.finished && iters < max_iters {
            iters += 1;
            if progress_interval > 0 && sim_start.elapsed() >= next_progress {
                eprintln!("[PROGRESS] wall={:.1}s sim_time={} iters={} edges_fired={} nba_q={} waiters={}",
                    sim_start.elapsed().as_secs_f64(), self.time, iters,
                    self.prof_edges_fired, self.nba_fast.len() + self.nba_queue.len(),
                    self.event_waiters.len());
                next_progress += std::time::Duration::from_secs(progress_interval);
            }

            let has_timed = !self.event_queue.is_empty();
            let has_waiters = !self.event_waiters.is_empty();
            let has_clocks = !self.clock_generators.is_empty();

            if !has_timed && !has_waiters && !has_clocks && self.delayed_updates.is_empty() { break; }
            // Deadlock: only waiters remain but nothing can ever wake them.
            if has_waiters && !has_timed && !has_clocks && self.delayed_updates.is_empty() { break; }

            // Determine next time: minimum of event queue, clock generators, and delayed updates
            let next_eq_time = self.event_queue.next_time();
            let next_clk_time = if has_clocks {
                self.clock_generators.iter().map(|c| c.next_toggle_time).min()
            } else { None };
            let next_delayed = self.next_delayed_time();
            let next_time = [next_eq_time, next_clk_time, next_delayed].into_iter()
                .flatten().min()
                .unwrap_or_else(|| if has_waiters { self.time } else { u64::MAX });

            if next_time > self.max_time { break; }
            if next_time > self.time { self.time = next_time; }

            {
                let _t = std::time::Instant::now();
                if iters > 1 {
                    self.snapshot_edge_signals();
                }
                t_snap += _t.elapsed().as_nanos() as u64;

                if self.apply_delayed_updates() {
                    self.settle_combinatorial();
                }

                self.fire_clock_generators();

                let _t = std::time::Instant::now();
                let mut batch = self.event_queue.remove(self.time);
                t_sched += _t.elapsed().as_nanos() as u64;
                let _t = std::time::Instant::now();
                while !batch.is_empty() {
                    if self.finished { break; }
                    let (pid, stmts) = batch.remove(0);
                    let t_now = self.time;
                    for (p, s) in batch.drain(..) {
                        self.event_queue.schedule(t_now, p, s);
                    }
                    self.run_scheduled_process(pid, &stmts);
                    if !self.is_pid_suspended(pid) {
                        self.child_finished(pid);
                    }
                    if self.event_queue.next_time() == Some(self.time) {
                        batch = self.event_queue.remove(self.time);
                    } else {
                        batch.clear();
                    }
                }
                t_process += _t.elapsed().as_nanos() as u64;

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
                // Cascade: edge-sensitive blocks fired by check_edges may have
                // pushed NBAs whose settled effect triggers further edges.
                // Drain them within this iter — see drain_edge_cascade.
                let (ds, dn, dse, de) = self.drain_edge_cascade(cascade_limit);
                t_snap += ds; t_nba += dn; t_settle += dse; t_edges += de;
                // (Deleted) The end-of-iter snapshot_edge_signals was
                // redundant: the early snapshot at the top of the next iter
                // (line ~3283) captures the same signal_table values because
                // nothing between the two snapshots mutates signals. At iter
                // 1, prev_table is pre-seeded to all-X by Simulator::new, so
                // that iteration's check_edges works without an early
                // snapshot. From iter 2 onward, the early snapshot provides
                // the correct pre-state. Removing the second snapshot cuts
                // ~half the `snap` phase cost.

                self.check_monitor();
                if self.aitrace_mode { self.aitrace_write_changes(); } else { self.vcd_write_changes(); }
                self.loop_iters += 1;
            }
        }
        let sim_elapsed = sim_start.elapsed();
        eprintln!("[PROF] settle={:.1}ms edges={:.1}ms nba={:.1}ms process={:.1}ms snap={:.1}ms sched={:.1}ms",
            t_settle as f64/1e6, t_edges as f64/1e6, t_nba as f64/1e6,
            t_process as f64/1e6, t_snap as f64/1e6, t_sched as f64/1e6);
        let unresolved = self.comb_entries.iter().filter(|e| e.has_unresolved_reads).count();
        eprintln!("[PROF] edge_waiters={:.1}ms edge_cg={:.1}ms waiter_iters={}",
            self.prof_edge_waiters as f64/1e6, self.prof_edge_cg as f64/1e6, self.prof_waiter_iters);
        eprintln!("[PROF] edge_detect={:.1}ms edge_exec={:.1}ms edges_fired={} insns={} ns_per_insn={:.1} fallbacks={}",
            self.prof_edge_detect as f64/1e6, self.prof_edge_exec as f64/1e6, self.prof_edges_fired,
            self.prof_insns_executed,
            if self.prof_insns_executed > 0 { self.prof_edge_exec as f64 / self.prof_insns_executed as f64 } else { 0.0 },
            self.prof_fallback_insns);
        let mut reasons: Vec<(&'static str, u64, u64)> = self.prof_fallback_by_reason.iter()
            .map(|(k, v)| (*k, v.0, v.1)).collect();
        reasons.sort_by_key(|(_, _, ns)| std::cmp::Reverse(*ns));
        for (reason, count, ns) in reasons.iter().take(15) {
            eprintln!("[PROF] fallback_reason {:>30}: count={:>8} total={:>8.1}ms avg={:>7.1}µs",
                reason, count, *ns as f64 / 1e6, *ns as f64 / *count as f64 / 1e3);
        }
        eprintln!("[PROF] settle_dc={:.1}ms({}) settle_ca={:.1}ms({}) settle_ab={:.1}ms({})",
            self.prof_settle_dc_ns as f64/1e6, self.prof_settle_dc_count,
            self.prof_settle_ca_ns as f64/1e6, self.prof_settle_ca_count,
            self.prof_settle_ab_ns as f64/1e6, self.prof_settle_ab_count);
        eprintln!("[PROF] settle_calls={} settle_iters={} max_iters={} entry_evals={} unresolved_entries={}/{}",
            self.settle_calls, self.settle_iters, self.max_settle_iters, self.entry_evals,
            unresolved, self.comb_entries.len());
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
                    CombItem::FusedGate { op } => {
                        let id = match op {
                            FusedGate::Buf1 { dst, .. }
                            | FusedGate::Bin2 { dst, .. }
                            | FusedGate::Mux2 { dst, .. } => dst.sig_id as usize,
                        };
                        &self.id_to_name[id]
                    }
                    CombItem::ContAssign {  .. } | CombItem::CompiledContAssign { .. } => {
                        if let Some(&id) = entry.write_signal_ids.first() {
                            &self.id_to_name[id]
                        } else { continue; }
                    }
                    CombItem::AlwaysBlock { .. } | CombItem::CompiledAlwaysBlock { .. } => {
                        if let Some(&id) = entry.write_signal_ids.first() {
                            &self.id_to_name[id]
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
                let top = block_top_signal.entry(block).or_insert((name.to_string(), 0));
                if count > top.1 { *top = (name.to_string(), count); }
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

    fn snapshot_process_context(&self) -> ProcessContext {
        ProcessContext {
            this_stack: self.this_stack.clone(),
            local_stack: self.local_stack.clone(),
            class_context_stack: self.class_context_stack.clone(),
            cg_this: self.cg_this,
            return_value: self.return_value.clone(),
            break_flag: self.break_flag,
            continue_flag: self.continue_flag,
        }
    }

    fn restore_process_context(&mut self, ctx: ProcessContext) {
        self.this_stack = ctx.this_stack;
        self.local_stack = ctx.local_stack;
        self.class_context_stack = ctx.class_context_stack;
        self.cg_this = ctx.cg_this;
        self.return_value = ctx.return_value;
        self.break_flag = ctx.break_flag;
        self.continue_flag = ctx.continue_flag;
    }

    fn inherit_current_process_context(&mut self, pid: usize) {
        let ctx = self.snapshot_process_context();
        if ctx.this_stack.is_empty()
            && ctx.local_stack.is_empty()
            && ctx.class_context_stack.is_empty()
            && ctx.cg_this.is_none()
            && ctx.return_value.is_none()
            && !ctx.break_flag
            && !ctx.continue_flag
        {
            self.process_contexts.remove(&pid);
        } else {
            self.process_contexts.insert(pid, ctx);
        }
    }

    /// Drain events in the scheduler whose fire time is at or before `target`.
    /// Used when a task-internal `#delay` needs to yield the simulator so
    /// concurrent processes can advance while this task sleeps.
    fn run_events_until(&mut self, target: u64) {
        let saved_pid = self.current_pid;
        let saved_break = self.break_flag;
        loop {
            let next = self.event_queue.next_time();
            let nt = match next { Some(t) if t <= target => t, _ => break };
            if nt > self.time { self.time = nt; }
            let processes = self.event_queue.remove(self.time);
            for (pid, stmts) in processes {
                if self.finished { break; }
                self.run_scheduled_process(pid, &stmts);
                if !self.is_pid_suspended(pid) { self.child_finished(pid); }
            }
            if !self.nba_fast.is_empty() || !self.nba_queue.is_empty() { self.apply_nba(); }
            if self.dirty_any { self.settle_combinatorial(); }
        }
        self.current_pid = saved_pid;
        self.break_flag = saved_break;
    }

    fn run_scheduled_process(&mut self, pid: usize, stmts: &[Statement]) {
        // Fast path: if we have no saved process context for this pid AND
        // the caller's execution context is empty, skip the full snapshot /
        // restore dance. Forever-loop bodies like `jclk = ~jclk` that run
        // with no locals don't need context bookkeeping; each call paid
        // several `Vec<HashMap<String, Value>>`-level clones for nothing.
        let saved_ctx_needed = !self.this_stack.is_empty()
            || !self.local_stack.is_empty()
            || !self.class_context_stack.is_empty();
        let has_pid_ctx = self.process_contexts.contains_key(&pid);
        if !saved_ctx_needed && !has_pid_ctx {
            self.run_process_stmts(pid, stmts);
            if self.is_pid_suspended(pid) {
                // Only snapshot if actually suspended and has state worth saving.
                if !self.this_stack.is_empty()
                    || !self.local_stack.is_empty()
                    || !self.class_context_stack.is_empty()
                {
                    self.process_contexts.insert(pid, self.snapshot_process_context());
                }
            }
            return;
        }
        let saved = self.snapshot_process_context();
        let ctx = self.process_contexts.remove(&pid).unwrap_or_default();
        self.restore_process_context(ctx);
        self.run_process_stmts(pid, stmts);
        if self.is_pid_suspended(pid) {
            self.process_contexts.insert(pid, self.snapshot_process_context());
        } else {
            self.process_contexts.remove(&pid);
        }
        self.restore_process_context(saved);
    }

    fn run_process_stmts(&mut self, pid: usize, stmts: &[Statement]) {
        self.current_pid = pid;
        sim_dbg_eprintln!("[DEBUG] running process {} ({} stmts) at time {}", pid, stmts.len(), self.time);
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

            // While loop with event/timing waits inside: unroll one iteration,
            // re-append the while statement so the condition is re-checked
            // after suspension.
            // If-statement whose chosen branch contains blocking stmts: descend
            // into the branch via run_process_stmts so repeat/while/@event
            // inside the branch can properly suspend the process.
            if let StatementKind::If { condition, then_stmt, else_stmt, .. } = &stmt.kind {
                let chosen: Option<&Statement> = if self.eval_expr(condition).is_true() {
                    Some(then_stmt.as_ref())
                } else {
                    else_stmt.as_ref().map(|b| b.as_ref())
                };
                if let Some(branch) = chosen {
                    if self.stmt_is_blocking(branch) {
                        let branch_stmts: Vec<Statement> = match &branch.kind {
                            StatementKind::SeqBlock { stmts, .. } => stmts.clone(),
                            _ => vec![branch.clone()],
                        };
                        let mut cont = branch_stmts;
                        cont.extend_from_slice(&stmts[i+1..]);
                        self.run_process_stmts(pid, &cont);
                        return;
                    }
                }
            }

            if let StatementKind::While { condition, body } = &stmt.kind {
                if self.stmt_has_event_wait(body) {
                    let cond_val = self.eval_expr(condition).is_true();
                    if cond_val {
                        let body_stmts = match &body.kind {
                            StatementKind::SeqBlock { stmts, .. } => stmts.clone(),
                            _ => vec![*body.clone()],
                        };
                        let mut cont: Vec<Statement> = body_stmts;
                        cont.push(stmt.clone());
                        cont.extend_from_slice(&stmts[i+1..]);
                        self.run_process_stmts(pid, &cont);
                        return;
                    } else {
                        i += 1;
                        continue;
                    }
                }
            }

            // Check for ParBlock (fork...join)
            if let StatementKind::ParBlock { stmts: sub_stmts, join_type, .. } = &stmt.kind {
                let mut child_pids = HashSet::new();
                for s in sub_stmts {
                    let pid_child = self.next_pid; self.next_pid += 1;
                    self.process_parents.insert(pid_child, pid);
                    self.inherit_current_process_context(pid_child);
                    // Schedule children to run at current time
                    self.event_queue.schedule(self.time, pid_child, vec![s.clone()]);
                    child_pids.insert(pid_child);
                }
                
                if *join_type == JoinType::JoinNone {
                    // Continue immediately
                    i += 1;
                    continue;
                } else {
                    // Suspend current process and wait for children
                    let cont = stmts[i+1..].to_vec();
                    self.join_waiters.push(JoinWaiter {
                        parent_pid: pid,
                        child_pids,
                        join_type: *join_type,
                        continuation: cont,
                        finished_children: HashSet::new(),
                    });
                    return;
                }
            } else {
                self.exec_statement(stmt);
            }

            // Check for WaitFork
            if let StatementKind::WaitFork = &stmt.kind {
                let children: HashSet<usize> = self.process_parents.iter()
                    .filter(|(_, &p)| p == pid)
                    .map(|(&c, _)| c)
                    .collect();
                
                if children.is_empty() {
                    i += 1;
                    continue;
                } else {
                    let cont = stmts[i+1..].to_vec();
                    self.join_waiters.push(JoinWaiter {
                        parent_pid: pid,
                        child_pids: children,
                        join_type: JoinType::Join,
                        continuation: cont,
                        finished_children: HashSet::new(),
                    });
                    return;
                }
            }

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
    fn resolve_nba_target(&mut self, lhs: &Expression) -> Option<usize> {
        match &lhs.kind {
            ExprKind::Ident(hier) => {
                let is_ambiguous_leaf =
                    hier.path.len() == 1 && !hier.path[0].name.name.contains('.');
                // Use cached signal ID if available
                if let Some(id) = hier.cached_signal_id.get() {
                    if !is_ambiguous_leaf {
                        return Some(id);
                    }
                }
                let name = self.resolve_hier_name(hier);
                if let Some(&id) = self.signal_name_to_id.get(name.as_str()) {
                    hier.cached_signal_id.set(Some(id));
                    return Some(id);
                }
                None
            }
            ExprKind::Index { expr, index } => {
                if let ExprKind::Ident(hier) = &expr.kind {
                    let name = self.resolve_hier_name(hier);
                    if self.module.arrays.contains_key(&name) {
                        let idx = self.eval_expr(index).to_u64().unwrap_or(0);
                        // Use a small buffer to avoid allocation for common array names
                        let elem = format!("{}[{}]", name, idx);
                        return self.signal_name_to_id.get(elem.as_str()).copied();
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn apply_nba(&mut self) {
        // Reset the per-window partial-NBA index — entries we accumulated
        // during the previous exec_insns runs are about to be drained
        // into signal_table, so any new partial-range NBAs in the next
        // window should re-seed from the freshly-applied signal_table.
        self.nba_fast_index.clear();
        // Take ownership so we can move values directly into signal_table
        // without the zero-placeholder swap dance.
        let mut nba = std::mem::take(&mut self.nba_fast);
        for entry in nba.drain(..) {
            let id = entry.signal_id;
            let width = self.signal_widths[id];
            let signed = self.signal_signed[id];
            let mut val = entry.value;
            if val.width != width { val = val.resize(width); }
            // Force declared signedness so a signed RHS (e.g. `$signed(...)`)
            // doesn't corrupt later reads relying on zero-extension.
            if val.is_signed != signed { val.is_signed = signed; }
            if self.signal_table[id] != val {
                if !self.dirty_signals[id] { self.dirty_signals[id] = true; self.dirty_list.push(id); }
                self.dirty_any = true;
                self.signal_table[id] = val;
                self.table_modified = true;
            }
        }
        self.nba_fast = nba;
        for i in 0..self.nba_queue.len() {
            if let Some(ref lhs) = self.nba_queue[i].lhs {
                let lhs = lhs.clone();
                let val = self.nba_queue[i].value.clone();
                self.assign_value(&lhs, &val);
            }
        }
        self.nba_queue.clear();
    }

    /// Apply delayed signal updates that are due at or before the current time.
    /// Returns true if any updates were applied.
    fn apply_delayed_updates(&mut self) -> bool {
        if self.delayed_updates.is_empty() { return false; }
        let mut applied = false;
        let mut i = 0;
        while i < self.delayed_updates.len() {
            if self.delayed_updates[i].0 <= self.time {
                let (_, id, val) = self.delayed_updates.swap_remove(i);
                if self.signal_table[id] != val {
                    self.signal_table[id] = val;
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

    /// Get the next time a delayed update is due (for time advancement).
    fn next_delayed_time(&self) -> Option<u64> {
        self.delayed_updates.iter().map(|(t, _, _)| *t).min()
    }

    /// Schedule a delayed signal update (inertial delay model).
    fn schedule_delayed(&mut self, id: usize, val: Value) {
        let delay = self.sdf_delays[id];
        let target_time = self.time + delay;
        // Inertial delay: remove any pending update for this signal
        self.delayed_updates.retain(|(_, sid, _)| *sid != id);
        self.delayed_updates.push((target_time, id, val));
    }

    /// Get the signal ID for a simple LHS identifier expression.
    fn get_lhs_signal_id(&self, lhs: &Expression) -> Option<usize> {
        if let ExprKind::Ident(hier) = &lhs.kind {
            let is_ambiguous_leaf =
                hier.path.len() == 1 && !hier.path[0].name.name.contains('.');
            if let Some(id) = hier.cached_signal_id.get() {
                if !is_ambiguous_leaf {
                    return Some(id);
                }
            }
            let resolved = self.resolve_hier_name(hier);
            if let Some(&id) = self.signal_name_to_id.get(resolved.as_str()) {
                return Some(id);
            }
            // Fallback for legacy single-segment names.
            let leaf = hier.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
            self.signal_name_to_id.get(leaf).copied()
        } else { None }
    }

    /// Snapshot only edge-sensitive signals + event_waiter signals into
    /// the prev_val/prev_xz parallel arrays (A3 from the compression
    /// analysis). Wide signals (> 64 bits) also update `prev_wide`.
    fn snapshot_edge_signals(&mut self) {
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
        for &id in &self.edge_signal_ids {
            snap_one(id, &self.signal_table, &self.signal_widths,
                &mut self.prev_val, &mut self.prev_xz, &mut self.prev_wide);
        }
        for i in 0..self.event_waiters.len() {
            for j in 0..self.event_waiters[i].resolved_sensitivities.len() {
                let sid = self.event_waiters[i].resolved_sensitivities[j].signal_id;
                snap_one(sid, &self.signal_table, &self.signal_widths,
                    &mut self.prev_val, &mut self.prev_xz, &mut self.prev_wide);
            }
        }
        for i in 0..self.cg_event_waiters.len() {
            for j in 0..self.cg_event_waiters[i].1.len() {
                let sid = self.cg_event_waiters[i].1[j].signal_id;
                snap_one(sid, &self.signal_table, &self.signal_widths,
                    &mut self.prev_val, &mut self.prev_xz, &mut self.prev_wide);
            }
        }
    }

    /// Check edge: compare signal_table[id] vs (prev_val[id], prev_xz[id]).
    #[inline]
    fn check_edge_id(&self, id: usize, edge: EdgeKind) -> bool {
        let (cur_v, cur_x) = self.signal_table[id].raw_bits();
        let prev_v = self.prev_val[id];
        let prev_x = self.prev_xz[id];
        // LogicBit at bit 0: One iff v&1==1 && x&1==0; Zero iff v&1==0 && x&1==0.
        let cb_one  = (cur_v & 1) == 1 && (cur_x & 1) == 0;
        let cb_zero = (cur_v & 1) == 0 && (cur_x & 1) == 0;
        let pb_one  = (prev_v & 1) == 1 && (prev_x & 1) == 0;
        let pb_zero = (prev_v & 1) == 0 && (prev_x & 1) == 0;
        match edge {
            EdgeKind::Posedge => !pb_one && cb_one,
            EdgeKind::Negedge => !pb_zero && cb_zero,
            EdgeKind::AnyEdge => {
                if self.signal_widths[id] > 64 {
                    if let Some(p) = self.prev_wide.get(&id) {
                        return self.signal_table[id] != *p;
                    }
                }
                cur_v != prev_v || cur_x != prev_x
            }
        }
    }

    fn check_edges(&mut self) {
        let blocks = std::mem::take(&mut self.edge_blocks);
        self.in_edge_block = true;

        // Phase 1: detect which blocks trigger.
        //
        // Inverted iteration: walk the (usually small) list of edge-sensitive
        // signal IDs, compute whether each fired an edge this tick, then
        // dispatch to the blocks sensitive to that signal via
        // `edge_blocks_by_sig`. For c910 this turns ~20,000 check_edge_id
        // calls per tick (10k blocks × 2 sensitivities) into ~200
        // (one per unique clk/rst/enable signal).
        let t0 = std::time::Instant::now();
        let mut triggered_bitmap = vec![false; blocks.len()];
        // edge_blocks_by_sig is parallel to edge_signal_ids (position-indexed).
        for (pos, &sid) in self.edge_signal_ids.iter().enumerate() {
            let entry = match self.edge_blocks_by_sig.get(pos) {
                Some(e) if !e.is_empty() => e,
                _ => continue,
            };
            // Compute edge-fired booleans once for this signal using SoA
            // u64 pairs; falls back to full Value compare for wide signals.
            let (cur_v, cur_x) = self.signal_table[sid].raw_bits();
            let prev_v = self.prev_val[sid];
            let prev_x = self.prev_xz[sid];
            let cb_one  = (cur_v & 1) == 1 && (cur_x & 1) == 0;
            let cb_zero = (cur_v & 1) == 0 && (cur_x & 1) == 0;
            let pb_one  = (prev_v & 1) == 1 && (prev_x & 1) == 0;
            let pb_zero = (prev_v & 1) == 0 && (prev_x & 1) == 0;
            let fires_pos = !pb_one && cb_one;
            let fires_neg = !pb_zero && cb_zero;
            let fires_any = if self.signal_widths[sid] > 64 {
                self.prev_wide.get(&sid)
                    .map_or(cur_v != prev_v || cur_x != prev_x,
                        |p| self.signal_table[sid] != *p)
            } else {
                cur_v != prev_v || cur_x != prev_x
            };
            if !fires_pos && !fires_neg && !fires_any { continue; }
            // Fan out to all blocks sensitive to this signal.
            for &(block_idx, edge) in entry {
                if block_idx >= triggered_bitmap.len() || triggered_bitmap[block_idx] { continue; }
                let fired = match edge {
                    EdgeKind::Posedge => fires_pos,
                    EdgeKind::Negedge => fires_neg,
                    EdgeKind::AnyEdge => fires_any,
                };
                if fired { triggered_bitmap[block_idx] = true; }
            }
        }
        let triggered: Vec<usize> = triggered_bitmap.iter().enumerate()
            .filter_map(|(i, &t)| if t { Some(i) } else { None })
            .collect();
        self.prof_edge_detect += t0.elapsed().as_nanos() as u64;
        self.prof_edges_fired += triggered.len() as u64;

        if !triggered.is_empty() {
            let t1 = std::time::Instant::now();

            // Separate into parallel-eligible and sequential blocks
            let mut parallel_blocks: Vec<usize> = Vec::new();
            let mut sequential_blocks: Vec<usize> = Vec::new();
            for &bi in &triggered {
                if bi < self.edge_block_parallel.len() && self.edge_block_parallel[bi] {
                    parallel_blocks.push(bi);
                } else {
                    sequential_blocks.push(bi);
                }
            }

            // Phase 2a: execute parallel-eligible blocks with thread::scope
            // Only parallelize when total instruction count justifies threading
            // overhead (~5µs per spawn). Threshold: 10k+ total instructions.
            let parallel_insn_count: usize = parallel_blocks.iter()
                .filter_map(|&bi| self.compiled_edge_blocks[bi].as_ref().map(|cb| cb.instructions.len()))
                .sum();
            if parallel_blocks.len() >= 2 && parallel_insn_count >= 10_000 {
                let signal_table = &self.signal_table;
                let signal_signed = &self.signal_signed;
                let signal_name_to_id = &self.signal_name_to_id;

                // Pre-extract instruction slices as raw pointers to avoid
                // sending non-Sync CompiledBlock (contains StmtFallback with
                // Cell fields) across threads. We only access parallel-eligible
                // blocks which are guaranteed to have no StmtFallback insns.
                struct BlockSlice { ptr: *const super::bytecode::Insn, len: usize, num_regs: usize }
                unsafe impl Send for BlockSlice {}
                unsafe impl Sync for BlockSlice {}

                let block_slices: Vec<(usize, BlockSlice)> = parallel_blocks.iter()
                    .filter_map(|&bi| {
                        self.compiled_edge_blocks[bi].as_ref().map(|cb| (bi, BlockSlice {
                            ptr: cb.instructions.as_ptr(),
                            len: cb.instructions.len(),
                            num_regs: cb.num_regs as usize,
                        }))
                    })
                    .collect();

                let num_threads = std::thread::available_parallelism()
                    .map(|n| n.get().min(block_slices.len()).min(8))
                    .unwrap_or(2);
                let chunk_size = (block_slices.len() + num_threads - 1) / num_threads;

                let mut all_nba: Vec<Vec<NbaFast>> = Vec::new();
                std::thread::scope(|s| {
                    let mut handles = Vec::new();
                    for chunk in block_slices.chunks(chunk_size) {
                        let handle = s.spawn(move || {
                            let mut thread_nba: Vec<NbaFast> = Vec::new();
                            let max_regs = chunk.iter().map(|(_, bs)| bs.num_regs).max().unwrap_or(0);
                            let mut vm_regs = vec![Value::zero(1); max_regs];
                            for (_, bs) in chunk {
                                if vm_regs.len() < bs.num_regs {
                                    vm_regs.resize(bs.num_regs, Value::zero(1));
                                }
                                let insns = unsafe { std::slice::from_raw_parts(bs.ptr, bs.len) };
                                let mut nba = Self::exec_insns_isolated(
                                    insns, signal_table, signal_signed,
                                    signal_name_to_id, &mut vm_regs,
                                );
                                thread_nba.append(&mut nba);
                            }
                            thread_nba
                        });
                        handles.push(handle);
                    }
                    for h in handles {
                        if let Ok(nba) = h.join() {
                            all_nba.push(nba);
                        }
                    }
                });
                for nba_batch in all_nba {
                    // Sync nba_fast_index for each appended entry so the
                    // sequential-block path that may follow can still find
                    // the latest entry per signal_id in O(1).
                    for entry in nba_batch {
                        self.nba_fast_index.insert(entry.signal_id, self.nba_fast.len());
                        self.nba_fast.push(entry);
                    }
                }
            } else {
                // Too few blocks for threading overhead to pay off
                for &bi in &parallel_blocks {
                    self.exec_bytecode(bi);
                }
            }

            // Phase 2b: execute sequential blocks on main thread.
            // Skip the name_resolve_hint save/restore for blocks that have
            // no StmtFallback insns (common case: 99%+ of edges don't fall
            // back to AST exec, since bytecode pre-resolves signal IDs).
            for &bi in &sequential_blocks {
                let needs_hint = bi < self.edge_block_needs_hint.len()
                    && self.edge_block_needs_hint[bi];
                if needs_hint {
                    let saved_hint = self.name_resolve_hint.borrow().clone();
                    if let Some(scope) = self.edge_block_scope.get(bi).and_then(|s| s.as_ref()) {
                        *self.name_resolve_hint.borrow_mut() = Some(scope.clone());
                    }
                    if !self.exec_bytecode(bi) {
                        self.exec_statement(&blocks[bi].stmt);
                    }
                    *self.name_resolve_hint.borrow_mut() = saved_hint;
                } else {
                    self.exec_bytecode(bi);
                }
            }

            self.prof_edge_exec += t1.elapsed().as_nanos() as u64;
        }

        // Trigger covergroup sampling
        let _t_cg = std::time::Instant::now();
        for i in 0..self.cg_event_waiters.len() {
            let handle = self.cg_event_waiters[i].0;
            let mut triggered = false;
            for j in 0..self.cg_event_waiters[i].1.len() {
                let sid = &self.cg_event_waiters[i].1[j];
                if self.check_edge_id(sid.signal_id, sid.edge) {
                    triggered = true;
                    break;
                }
            }
            if triggered {
                self.sample_covergroup(handle);
            }
        }

        self.prof_edge_cg += _t_cg.elapsed().as_nanos() as u64;

        // Wake up event_waiters whose sensitivity conditions are met
        let _t_w = std::time::Instant::now();
        let waiters = std::mem::take(&mut self.event_waiters);
        self.prof_waiter_iters += waiters.len() as u64;
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
                sim_dbg_eprintln!("[DEBUG] waiter for process {} triggered at time {}", waiter.pid, self.time);
                self.event_queue.schedule(self.time, waiter.pid, waiter.continuation);
            } else {
                self.event_waiters_swap.push(waiter);
            }
        }
        std::mem::swap(&mut self.event_waiters, &mut self.event_waiters_swap);
        self.prof_edge_waiters += _t_w.elapsed().as_nanos() as u64;
        self.edge_blocks = blocks;
        self.in_edge_block = false;
    }

    fn settle_combinatorial(&mut self) {
        if self.settling { return; }
        if !self.dirty_any { return; }
        self.settling = true;
        self.settle_calls += 1;

        let entries = std::mem::take(&mut self.comb_entries);
        let dep_offsets = std::mem::take(&mut self.comb_dep_offsets);
        let dep_entries = std::mem::take(&mut self.comb_dep_entries);
        let num_entries = entries.len();

        // Resize persistent buffers if needed (only happens once)
        if self.settle_triggered.len() < num_entries {
            self.settle_triggered.resize(num_entries, false);
        }

        let mut total_iters = 0u64;
        let limit = self.settle_limit as u64;

        // One-time seed: consume the initial dirty set, mark dependents.
        // settle_triggered acts as persistent "needs evaluation" across passes.
        for &eidx in &self.settle_triggered_list {
            self.settle_triggered[eidx] = false;
        }
        self.settle_triggered_list.clear();
        for &id in &self.dirty_list {
            if self.dirty_signals[id] {
                self.dirty_signals[id] = false;
                if id + 1 < dep_offsets.len() {
                    let lo = dep_offsets[id] as usize;
                    let hi = dep_offsets[id + 1] as usize;
                    for &eidx_u32 in &dep_entries[lo..hi] {
                        let eidx = eidx_u32 as usize;
                        if !self.settle_triggered[eidx] {
                            self.settle_triggered[eidx] = true;
                            self.settle_triggered_list.push(eidx);
                        }
                    }
                }
            }
        }
        self.dirty_list.clear();
        self.dirty_any = false;

        // Unresolved entries always re-eval. Iterate the precomputed
        // index list instead of scanning `0..num_entries` (can be ~500K
        // and is called on every BlockingAssign).
        for &eidx in self.comb_unresolved_idx.iter() {
            if eidx < num_entries && !self.settle_triggered[eidx] {
                self.settle_triggered[eidx] = true;
                self.settle_triggered_list.push(eidx);
            }
        }
        // Time-0 / empty-read-set fire unconditionally — but only the FIRST
        // settle call at time=0 needs to seed them. Subsequent settle calls
        // at the same (or later) time rely on dirty propagation through the
        // worklist. The prior O(num_entries) scan per call was ~500μs on
        // c910 and dominated per-assign cost during memory-init loops.
        if self.time == 0 && !self.comb_time0_fired {
            for &eidx in self.comb_time0_idx.iter() {
                if eidx < num_entries && !self.settle_triggered[eidx] {
                    self.settle_triggered[eidx] = true;
                    self.settle_triggered_list.push(eidx);
                }
            }
            self.comb_time0_fired = true;
        }

        // Chaotic-iteration loop: drain a sorted worklist of triggered
        // entries each pass. New triggers added during a pass (via worklist
        // propagation below) land in `settle_triggered_list` for the next
        // pass. Sorting per pass keeps entries processed in topo order
        // (small k since only triggered entries appear), which is dramatically
        // faster than a 0..num_entries linear scan when num_entries is large
        // (e.g. ~467K on c910) but the triggered set is small.
        let mut cur_list: Vec<usize> = Vec::new();
        for iteration in 0..limit {
            total_iters += 1;
            let mut evaluated_any = false;

            // Take the current worklist and sort by eidx (topo order).
            std::mem::swap(&mut cur_list, &mut self.settle_triggered_list);
            cur_list.sort_unstable();

            // Hot loop: >90% of iterations hit DC / CompiledContAssign /
            // CompiledAlwaysBlock — avoid scope_hint.clone() and Instant::now()
            // on those paths. Only the AST fallback arms pay that cost.
            for &eidx in &cur_list {
                if !self.settle_triggered[eidx] { continue; }
                // Consume the trigger now. If any signal is written AGAIN
                // later this pass (or next), the entry is re-triggered by
                // the worklist propagation below.
                self.settle_triggered[eidx] = false;
                evaluated_any = true;
                let dirty_before = self.dirty_list.len();

                self.entry_evals += 1;
                if self.activity_mon {
                    if let Some(slot) = self.activity_counts.get_mut(eidx) { *slot += 1; }
                }
                match &entries[eidx].item {
                    CombItem::DirectCopy { dst_id, src_id, width } => {
                        let src_val = self.signal_table[*src_id].clone();
                        let resized = if src_val.width != *width { src_val.resize(*width) } else { src_val };
                        if self.signal_table[*dst_id] != resized {
                            let delay = self.sdf_delays[*dst_id];
                            if delay > 0 && self.time > 0 {
                                self.schedule_delayed(*dst_id, resized);
                            } else {
                                self.mark_dirty_id(*dst_id);
                                self.signal_table[*dst_id] = resized;
                                self.table_modified = true;
                            }
                        }
                        self.prof_settle_dc_count += 1;
                    }
                    CombItem::CompiledContAssign { compiled } => {
                        if self.vm_regs.len() < compiled.num_regs as usize {
                            self.vm_regs.resize(compiled.num_regs as usize, Value::zero(1));
                        }
                        let insns = unsafe {
                            std::slice::from_raw_parts(compiled.instructions.as_ptr(), compiled.instructions.len())
                        };
                        self.exec_insns(insns);
                        self.prof_settle_dc_count += 1;
                    }
                    CombItem::ContAssign { lhs, rhs } => {
                        let scope_hint = entries[eidx].scope_hint.clone();
                        let t_entry = std::time::Instant::now();
                        let saved_hint = self.name_resolve_hint.borrow().clone();
                        if let Some(hint) = &scope_hint {
                            *self.name_resolve_hint.borrow_mut() = Some(hint.clone());
                        }
                        let lhs_id = self.get_lhs_signal_id(lhs);
                        if scope_hint.is_none() {
                            if let Some(id) = lhs_id {
                                if let Some(full) = self.id_to_name.get(id) {
                                    if let Some((parent, _)) = full.rsplit_once('.') {
                                        *self.name_resolve_hint.borrow_mut() = Some(parent.to_string());
                                    }
                                }
                            }
                        }
                        let w = self.infer_lhs_width(lhs);
                        let val = self.eval_expr_ctx(rhs, w).resize(w);
                        // Check if the LHS target has an SDF delay
                        let delay = lhs_id.map(|id| self.sdf_delays[id]).unwrap_or(0);
                        if delay > 0 && self.time > 0 {
                            if let Some(id) = lhs_id {
                                if self.signal_table[id] != val {
                                    self.schedule_delayed(id, val);
                                }
                            }
                        } else {
                            if let Some(id) = lhs_id {
                                let width = self.signal_widths[id];
                                let mut resized = if self.signal_real[id] {
                                    if val.is_real { val.clone() } else { Value::from_f64(val.to_f64()) }
                                } else {
                                    if val.is_real { Value::from_u64(val.to_f64() as u64, width) } else { val.resize(width) }
                                };
                                resized.is_signed = self.signal_signed[id];
                                if self.signal_table[id] != resized {
                                    self.mark_dirty_id(id);
                                    self.signal_table[id] = resized;
                                    self.table_modified = true;
                                }
                            } else {
                                self.assign_value(lhs, &val);
                            }
                        }
                        *self.name_resolve_hint.borrow_mut() = saved_hint;
                        self.prof_settle_ca_ns += t_entry.elapsed().as_nanos() as u64;
                        self.prof_settle_ca_count += 1;
                    }
                    CombItem::AlwaysBlock { stmt, .. } => {
                        let scope_hint = entries[eidx].scope_hint.clone();
                        let t_entry = std::time::Instant::now();
                        let saved_hint = self.name_resolve_hint.borrow().clone();
                        if let Some(hint) = &scope_hint {
                            *self.name_resolve_hint.borrow_mut() = Some(hint.clone());
                        }
                        let write_ids = &entries[eidx].write_signal_ids;
                        self.settle_prev_values.clear();
                        for &id in write_ids {
                            self.settle_prev_values.push((id, self.signal_table[id].clone()));
                        }
                        self.exec_statement(stmt);
                        *self.name_resolve_hint.borrow_mut() = saved_hint;
                        for i in 0..self.settle_prev_values.len() {
                            let (id, ref old_val) = self.settle_prev_values[i];
                            if self.signal_table[id] != *old_val {
                                if !self.dirty_signals[id] { self.dirty_signals[id] = true; self.dirty_list.push(id); }
                                self.dirty_any = true;
                            }
                        }
                        self.prof_settle_ab_ns += t_entry.elapsed().as_nanos() as u64;
                        self.prof_settle_ab_count += 1;
                    }
                    CombItem::CompiledAlwaysBlock { compiled, .. } => {
                        // Bytecode path: BlockingAssign/NbaAssign insns mark
                        // dirty automatically; no pre/post value snapshot needed.
                        if self.vm_regs.len() < compiled.num_regs as usize {
                            self.vm_regs.resize(compiled.num_regs as usize, Value::zero(1));
                        }
                        let insns = unsafe {
                            std::slice::from_raw_parts(compiled.instructions.as_ptr(), compiled.instructions.len())
                        };
                        self.exec_insns(insns);
                        self.prof_settle_ab_count += 1;
                    }
                    CombItem::FusedGate { op } => {
                        let op = *op;
                        self.exec_fused_gate(op);
                        self.prof_settle_dc_count += 1;
                    }
                }

                // Worklist propagation: any signals newly dirtied by this
                // entry's evaluation trigger their dependents, which are
                // appended to `settle_triggered_list` if not already fired.
                // Topo-ordered entries → most dependents sit at higher tidx
                // and will be reached before the outer iter boundary.
                let dirty_after = self.dirty_list.len();
                if dirty_after > dirty_before {
                    for di in dirty_before..dirty_after {
                        let sig_id = self.dirty_list[di];
                        // Consume the dirty flag — we're propagating it now.
                        // If the signal gets dirtied again later, it'll be
                        // re-pushed to dirty_list with a fresh flag.
                        self.dirty_signals[sig_id] = false;
                        if sig_id + 1 < dep_offsets.len() {
                            let lo = dep_offsets[sig_id] as usize;
                            let hi = dep_offsets[sig_id + 1] as usize;
                            for &dep_eidx_u32 in &dep_entries[lo..hi] {
                                let dep_eidx = dep_eidx_u32 as usize;
                                if !self.settle_triggered[dep_eidx] {
                                    self.settle_triggered[dep_eidx] = true;
                                    self.settle_triggered_list.push(dep_eidx);
                                }
                            }
                        }
                    }
                }
            }

            // After one full topo scan: if no entry was evaluated, fixpoint.
            if !evaluated_any { break; }
            // Reset dirty bookkeeping for the next scan. (We already consumed
            // the per-signal flags via the propagation loop; clear the list.)
            self.dirty_list.clear();
            self.dirty_any = false;
            // Prepare cur_list for next pass reuse (it'll be swapped in).
            cur_list.clear();
        }

        self.comb_entries = entries;
        self.comb_dep_offsets = dep_offsets;
        self.comb_dep_entries = dep_entries;
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

    /// Look up flat signal_id for array element `name[idx]` via a lazy per-array
    /// cache. First call per array populates the cache by iterating
    /// signal_name_to_id for all elements (one-time O(N) over the array size);
    /// subsequent calls are O(1) table indexing. Avoids the `format!()` +
    /// HashMap lookup that dominates tight memory-init loops (e.g. 1.4M writes
    /// in the c910 testbench `ram0.mem[i] = 0` wipe).
    fn get_array_elem_id(&mut self, name: &str, idx: i64) -> Option<usize> {
        let (arr_lo, arr_hi) = match self.module.arrays.get(name) {
            Some(&(lo, hi, _)) => (lo, hi),
            None => return None,
        };
        if idx < arr_lo || idx > arr_hi { return None; }
        let offset = (idx - arr_lo) as usize;
        if !self.array_elem_ids.contains_key(name) {
            let size = (arr_hi - arr_lo + 1) as usize;
            let mut v = vec![-1i64; size];
            for i in 0..size {
                let elem_idx = arr_lo + i as i64;
                let elem_name = format!("{}[{}]", name, elem_idx);
                if let Some(&id) = self.signal_name_to_id.get(elem_name.as_str()) {
                    v[i] = id as i64;
                }
            }
            self.array_elem_ids.insert(name.to_string(), v);
        }
        let v = self.array_elem_ids.get(name).unwrap();
        let id = v[offset];
        if id < 0 { None } else { Some(id as usize) }
    }

    fn assign_value(&mut self, lhs: &Expression, val: &Value) -> bool {
        match &lhs.kind {
            ExprKind::Ident(hier) => {
                if hier.path.len() == 1 && hier.path[0].selects.is_empty() {
                    let name = &hier.path[0].name.name;
                    // Check local stack
                    if !self.local_stack.is_empty() {
                        let last_idx = self.local_stack.len() - 1;
                        if self.local_stack[last_idx].contains_key(name) {
                            self.local_stack[last_idx].insert(name.clone(), val.clone());
                            return true;
                        }
                    }
                    // Check 'this' properties
                    if let Some(Some(handle)) = self.this_stack.last() {
                        if let Some(Some(instance)) = self.heap.get_mut(*handle) {
                            if instance.properties.contains_key(name) {
                                instance.properties.insert(name.clone(), val.clone());
                                return true;
                            }
                        }
                    }
                } else if hier.path.len() > 1 {
                    let obj_name = &hier.path[0].name.name;
                    {
                        let sub = hier.path[1..].iter().map(|s| s.name.name.as_str()).collect::<Vec<_>>().join(".");
                        if let Some(fields) = self.module.packed_struct_fields.get(obj_name).cloned() {
                            if let Some((_, off, w)) = fields.iter().find(|(m, _, _)| m == &sub).cloned() {
                                if let Some(cur_sig) = self.get_signal_value_by_name(obj_name) {
                                    let total_w = cur_sig.width;
                                    let mut cur = cur_sig.resize(total_w);
                                    let piece = val.resize(w);
                                    for i in 0..w {
                                        let bit = piece.get_bit(i as usize);
                                        cur.set_bit((off + i) as usize, bit);
                                    }
                                    let prev = self.get_signal_value_by_name(obj_name);
                                    let changed = prev.as_ref() != Some(&cur);
                                    self.set_signal_value_by_name(obj_name, cur);
                                    return changed;
                                }
                            }
                        }
                    }
                    let obj_val = if let Some(locals) = self.local_stack.last() {
                        locals.get(obj_name).cloned()
                    } else {
                        self.get_signal_value_by_name(obj_name)
                    };
                    if let Some(v) = obj_val {
                        let mut cur_handle = v.to_u64().unwrap_or(0) as usize;
                        for i in 1..hier.path.len() {
                            if cur_handle == 0 || cur_handle >= self.heap.len() { break; }
                            let member_name = &hier.path[i].name.name;
                            if i == hier.path.len() - 1 {
                                if let Some(Some(inst)) = self.heap.get_mut(cur_handle) {
                                    if inst.properties.contains_key(member_name) {
                                        inst.properties.insert(member_name.clone(), val.clone());
                                        return true;
                                    }
                                }
                                break;
                            }
                            if let Some(Some(inst)) = self.heap.get(cur_handle) {
                                if let Some(mval) = inst.properties.get(member_name) {
                                    cur_handle = mval.to_u64().unwrap_or(0) as usize;
                                } else { break; }
                            } else { break; }
                        }
                    }
                }
                let is_ambiguous_leaf =
                    hier.path.len() == 1 && !hier.path[0].name.name.contains('.');
                if let Some(id) = hier.cached_signal_id.get() {
                    if !is_ambiguous_leaf {
                        let width = self.signal_widths[id];
                        let mut resized = if self.signal_real[id] {
                            if val.is_real { val.clone() } else { Value::from_f64(val.to_f64()) }
                        } else {
                            if val.is_real { Value::from_u64(val.to_f64() as u64, width) } else { val.resize(width) }
                        };
                        resized.is_signed = self.signal_signed[id];
                        let changed = self.signal_table[id] != resized;
                        if changed {
                            self.mark_dirty_id(id);
                            self.signal_table[id] = resized;
                            self.table_modified = true;
                        }
                        return changed;
                    }
                }
                let name = self.resolve_hier_name(hier);
                if let Some(&id) = self.signal_name_to_id.get(name.as_str()) {
                    hier.cached_signal_id.set(Some(id));
                    let width = self.signal_widths[id];
                    let mut resized = if self.signal_real[id] {
                        if val.is_real { val.clone() } else { Value::from_f64(val.to_f64()) }
                    } else {
                        if val.is_real { Value::from_u64(val.to_f64() as u64, width) } else { val.resize(width) }
                    };
                    resized.is_signed = self.signal_signed[id];
                    let changed = self.signal_table[id] != resized;
                    if changed {
                        self.mark_dirty_id(id);
                        self.signal_table[id] = resized;
                        self.table_modified = true;
                    }
                    return changed;
                }
                // Fallback: signal not in signal_name_to_id (truly unknown,
                // e.g. testbench probe wires that didn't elaborate). Write
                // directly into the legacy `signals` HashMap without first
                // forcing a 35M-entry sync from signal_table — sync was the
                // dominant cost (20s × ~40 calls = 14 minutes) of c910 time-0
                // settle. We only write here, never read from self.signals
                // before this path, so the sync was unnecessary.
                let width = self.widths.get(&name).copied().unwrap_or(val.width);
                let is_real = self.real_signals.contains(&name);
                let mut resized = if is_real {
                    if val.is_real { val.clone() } else { Value::from_f64(val.to_f64()) }
                } else {
                    if val.is_real { Value::from_u64(val.to_f64() as u64, width) } else { val.resize(width) }
                };
                resized.is_signed = self.signed_signals.contains(&name);
                let changed = self.signals.get(&name).map_or(true, |p| *p != resized);
                if changed { 
                    self.mark_dirty(&name);
                }
                self.signals.insert(name.clone(), resized.clone());

                // If this is an array or queue, and we are assigning a packed value,
                // we might want to split it into elements.
                if let Some((lo, hi, elem_width)) = self.module.arrays.get(&name).cloned() {
                    let num_elements = (resized.width / elem_width) as usize;
                    // For queues/dynamic arrays, we update the size
                    let is_dynamic = hi < lo || hi == 63 && lo == 0; // simplistic check for [lo:hi] vs []/[$]
                    if is_dynamic {
                        self.signals.insert(format!("{}.size", name), Value::from_u64(num_elements as u64, 32));
                    }
                    for i in 0..num_elements {
                        let l = (num_elements - 1 - i) * elem_width as usize + (elem_width as usize - 1);
                        let r = (num_elements - 1 - i) * elem_width as usize;
                        let elem_val = resized.range_select(l, r);
                        self.signals.insert(format!("{}[{}]", name, i), elem_val);
                    }
                }

                changed
            }
            ExprKind::Index { expr, index } => {
                // N-dimensional (N >= 3) unpacked array element assignment
                {
                    let mut cur = expr.as_ref();
                    let mut rev_idxs: Vec<&Expression> = vec![index.as_ref()];
                    while let ExprKind::Index { expr: inner_e, index: inner_i } = &cur.kind {
                        rev_idxs.push(inner_i.as_ref());
                        cur = inner_e.as_ref();
                    }
                    if let ExprKind::Ident(hier) = &cur.kind {
                        let base_name = self.resolve_hier_name(hier);
                        if let Some((shape, _w)) = self.module.arrays_nd.get(&base_name).cloned() {
                            if rev_idxs.len() == shape.len() {
                                let mut name = base_name.clone();
                                for i in (0..rev_idxs.len()).rev() {
                                    let v = self.eval_expr(rev_idxs[i]).to_u64().unwrap_or(0) as i64;
                                    name = format!("{}[{}]", name, v);
                                }
                                if let Some(&id) = self.signal_name_to_id.get(name.as_str()) {
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
                                let changed = self.signals.get(&name).map_or(true, |p| *p != *val);
                                if changed {
                                    self.signals.insert(name.clone(), val.clone());
                                    self.mark_dirty(&name);
                                }
                                return changed;
                            }
                        }
                    }
                }
                // 2D array element assignment: mem[i][j] = val
                if let ExprKind::Index { expr: inner_expr, index: inner_idx } = &expr.kind {
                    if let ExprKind::Ident(hier) = &inner_expr.kind {
                        let name = self.resolve_hier_name(hier);
                        if self.module.arrays_2d.contains_key(&name) {
                            let i = self.eval_expr(inner_idx).to_u64().unwrap_or(0) as i64;
                            let j = self.eval_expr(index).to_u64().unwrap_or(0) as i64;
                            let elem_name = format!("{}[{}][{}]", name, i, j);
                            if let Some(&id) = self.signal_name_to_id.get(elem_name.as_str()) {
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
                            let changed = self.signals.get(&elem_name).map_or(true, |p| *p != *val);
                            if changed {
                                self.signals.insert(elem_name.clone(), val.clone());
                                self.mark_dirty(&elem_name);
                            }
                            return changed;
                        }
                    }
                }
                if let ExprKind::Ident(hier) = &expr.kind {
                    let name = self.resolve_hier_name(hier);
                    let idx_val = self.eval_expr(index);
                    let idx_str = if self.is_associative_array(&name) {
                        self.assoc_key_str(&name, &idx_val)
                    } else {
                        idx_val.to_u64().unwrap_or(0).to_string()
                    };

                    // Check if this is an array element assignment
                    if self.module.arrays.contains_key(&name) || self.is_associative_array(&name) {
                        let elem_name = format!("{}[{}]", name, idx_str);
                        if let Some(&id) = self.signal_name_to_id.get(elem_name.as_str()) {
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
                        // Fallback: slow path / associative array
                        let changed = self.signals.get(&elem_name).map_or(true, |p| *p != *val);
                        if changed {
                            sim_dbg_eprintln!("[DEBUG] signal {} changed to {:?}", elem_name, val);
                            self.signals.insert(elem_name.clone(), val.clone());
                            self.mark_dirty(&elem_name);
                        }
                        return changed;
                    }
                    // Fall back to bit select assignment
                    let idx = idx_val.to_u64().unwrap_or(0) as usize;
                    // Bit select needs signal_table
                    if let Some(&id) = self.signal_name_to_id.get(name.as_str()) {
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
            ExprKind::RangeSelect { expr, left, right, kind } => {
                let l = self.eval_expr(left).to_u64().unwrap_or(0) as usize;
                let r = self.eval_expr(right).to_u64().unwrap_or(0) as usize;
                let (msb, lsb) = match kind {
                    RangeKind::Constant => (l.max(r), l.min(r)),
                    RangeKind::IndexedUp => (l + r.saturating_sub(1), l),
                    RangeKind::IndexedDown => (l, l.saturating_sub(r.saturating_sub(1))),
                };
                // Unpacked array slice assignment: copy element-by-element
                if let ExprKind::Ident(hier) = &expr.kind {
                    let name = self.resolve_hier_name(hier);
                    if let Some(&(arr_lo, arr_hi, elem_w)) = self.module.arrays.get(&name) {
                        let count = msb + 1 - lsb;
                        let descending = self.module.descending_arrays.contains(&name);
                        let mut changed = false;
                        let _ = (arr_lo, descending);
                        for i in 0..count {
                            let lhs_idx = (lsb + i) as i64;
                            if lhs_idx < arr_lo || lhs_idx > arr_hi { continue; }
                            let elem_name = format!("{}[{}]", name, lhs_idx);
                            let new_val = if (val.width as usize) == count * elem_w as usize {
                                val.range_select((i + 1) * elem_w as usize - 1, i * elem_w as usize)
                            } else {
                                val.clone()
                            };
                            if self.get_signal_value_by_name(&elem_name).as_ref() != Some(&new_val) {
                                self.set_signal_value_by_name(&elem_name, new_val);
                                changed = true;
                            }
                        }
                        return changed;
                    }
                }
                // Fast path: resolve target signal_id directly (avoids format!
                // + HashMap lookup on every call for tight memory-init loops).
                let target_id: Option<usize> = match &expr.kind {
                    ExprKind::Ident(hier) => {
                        if let Some(id) = hier.cached_signal_id.get() {
                            Some(id)
                        } else {
                            let name = self.resolve_hier_name(hier);
                            if let Some(&id) = self.signal_name_to_id.get(name.as_str()) {
                                hier.cached_signal_id.set(Some(id));
                                Some(id)
                            } else { None }
                        }
                    }
                    ExprKind::Index { expr: arr_expr, index } => {
                        if let ExprKind::Ident(hier) = &arr_expr.kind {
                            let name = self.resolve_hier_name(hier);
                            if self.module.arrays.contains_key(&name) {
                                let idx = self.eval_expr(index).to_u64().unwrap_or(0) as i64;
                                self.get_array_elem_id(&name, idx)
                            } else { None }
                        } else { None }
                    }
                    _ => None,
                };
                if let Some(id) = target_id {
                    let width = self.signal_widths[id] as usize;
                    // Whole-word fast path: if the range covers the whole
                    // signal, skip the per-bit loop.
                    if lsb == 0 && msb + 1 >= width {
                        let resized = val.resize(width as u32);
                        let changed = self.signal_table[id] != resized;
                        if changed {
                            self.mark_dirty_id(id);
                            self.signal_table[id] = resized;
                            self.table_modified = true;
                        }
                        return changed;
                    }
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
                        self.mark_dirty_id(id);
                    }
                    return changed;
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
            ExprKind::MemberAccess { expr, member } => {
                if let ExprKind::Ident(hier) = &expr.kind {
                    let name = self.resolve_hier_name(hier);
                    if let Some(fields) = self.module.packed_struct_fields.get(&name).cloned() {
                        if let Some((_, off, w)) = fields.iter().find(|(m, _, _)| m == &member.name).cloned() {
                            if let Some(cur_sig) = self.get_signal_value_by_name(&name) {
                                let total_w = cur_sig.width;
                                let mut cur = cur_sig.resize(total_w);
                                let piece = val.resize(w);
                                for i in 0..w {
                                    let bit = piece.get_bit(i as usize);
                                    cur.set_bit((off + i) as usize, bit);
                                }
                                let prev = self.get_signal_value_by_name(&name);
                                let changed = prev.as_ref() != Some(&cur);
                                self.set_signal_value_by_name(&name, cur);
                                return changed;
                            }
                        }
                    }
                }
                let base = self.eval_expr(expr);
                let handle = base.to_u64().unwrap_or(0) as usize;
                if handle != 0 && handle < self.heap.len() {
                    if let Some(instance) = &mut self.heap[handle] {
                        let changed = instance.properties.get(&member.name) != Some(val);
                        if changed {
                            instance.properties.insert(member.name.clone(), val.clone());
                        }
                        return changed;
                    }
                }
                false
            }
            _ => false,
        }
    }

    pub fn eval_expr(&mut self, expr: &Expression) -> Value {
        self.eval_expr_ctx(expr, 0)
    }

    /// Evaluate expression with a context width hint (for proper shift sizing).
    /// When ctx_width > 0, shift operators widen their left operand to ctx_width.
    pub fn eval_expr_ctx(&mut self, expr: &Expression, ctx_width: u32) -> Value {
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
            ExprKind::Ident(hier) => {
                if hier.path.len() == 1 && hier.path[0].selects.is_empty() {
                    let name = &hier.path[0].name.name;
                    if name == "UVM_ACTIVE" { return Value::from_u64(1, 32); }
                    if name == "UVM_PASSIVE" { return Value::from_u64(0, 32); }
                    if let Some(locals) = self.local_stack.last() {
                        if let Some(val) = locals.get(name) { return val.clone(); }
                    }
                    if let Some(Some(handle)) = self.this_stack.last() {
                        if let Some(Some(instance)) = self.heap.get(*handle) {
                            if let Some(val) = instance.properties.get(name) { return val.clone(); }
                        }
                    }
                } else if hier.path.len() > 1 {
                    // Check local stack for dotted names like "item.index"
                    if let Some(locals) = self.local_stack.last() {
                        let dotted = hier.path.iter().map(|s| s.name.name.as_str()).collect::<Vec<_>>().join(".");
                        if let Some(val) = locals.get(&dotted) { return val.clone(); }
                    }
                    let obj_name = &hier.path[0].name.name;
                    {
                        let sub = hier.path[1..].iter().map(|s| s.name.name.as_str()).collect::<Vec<_>>().join(".");
                        if let Some(fields) = self.module.packed_struct_fields.get(obj_name).cloned() {
                            if let Some((_, off, w)) = fields.iter().find(|(m, _, _)| m == &sub).cloned() {
                                if let Some(sig) = self.get_signal_value_by_name(obj_name) {
                                    return sig.range_select((off + w - 1) as usize, off as usize);
                                }
                            }
                        }
                    }
                    if hier.path.len() == 2 {
                        let mname = hier.path[1].name.name.as_str();
                        if self.is_associative_array(obj_name) {
                            if mname == "size" || mname == "num" {
                                let prefix = format!("{}[", obj_name);
                                let count = self.signals.keys().filter(|k| k.starts_with(&prefix)).count();
                                return Value::from_u64(count as u64, 32);
                            }
                            if mname == "delete" {
                                let prefix = format!("{}[", obj_name);
                                let keys: Vec<String> = self.signals.keys().filter(|k| k.starts_with(&prefix)).cloned().collect();
                                for k in keys { self.signals.remove(&k); }
                                return Value::zero(32);
                            }
                        }
                        if mname == "delete" && self.module.arrays.contains_key(obj_name) {
                            self.set_queue_size(obj_name, 0);
                            return Value::zero(32);
                        }
                        if (mname == "size" || mname == "num") && self.module.arrays.contains_key(obj_name) {
                            return Value::from_u64(self.get_queue_size(obj_name), 32);
                        }
                        if mname == "pop_front" && self.module.arrays.contains_key(obj_name) {
                            return self.eval_builtin_method(obj_name, "pop_front", &[]).unwrap_or(Value::zero(32));
                        }
                        if mname == "pop_back" && self.module.arrays.contains_key(obj_name) {
                            return self.eval_builtin_method(obj_name, "pop_back", &[]).unwrap_or(Value::zero(32));
                        }
                        if matches!(mname, "sort" | "rsort" | "reverse" | "sum" | "product" | "min" | "max" | "unique" | "unique_index" | "find" | "find_first" | "find_last" | "find_index" | "find_first_index" | "find_last_index" | "and" | "or" | "xor") && self.module.arrays.contains_key(obj_name) {
                            return self.eval_builtin_method(obj_name, mname, &[]).unwrap_or(Value::zero(32));
                        }
                    }
                    // Handle hierarchical ident that might be class member access: obj.prop
                    let val = if let Some(locals) = self.local_stack.last() {
                        locals.get(obj_name).cloned()
                    } else {
                        self.get_signal_value_by_name(obj_name)
                    };
                    if let Some(v) = val {
                        let mut cur_handle = v.to_u64().unwrap_or(0) as usize;
                        for i in 1..hier.path.len() {
                            if cur_handle == 0 || cur_handle >= self.heap.len() { break; }
                            if let Some(Some(inst)) = self.heap.get(cur_handle) {
                                let member_name = &hier.path[i].name.name;
                                if let Some(mval) = inst.properties.get(member_name) {
                                    if i == hier.path.len() - 1 { return mval.clone(); }
                                    cur_handle = mval.to_u64().unwrap_or(0) as usize;
                                } else { break; }
                            } else { break; }
                        }
                    }
                }
                self.fast_signal_read(hier)
            }
            ExprKind::Unary { op, operand } => {
                let v = self.eval_expr(operand);
                match op {
                    UnaryOp::Plus => v, UnaryOp::Minus => { let mut r = Value::zero(v.width).sub(&v).resize(v.width); r.is_signed = true; r },
                    UnaryOp::LogNot => v.logic_not(), UnaryOp::BitNot => v.bitwise_not(),
                    UnaryOp::BitAnd => v.reduce_and(), UnaryOp::BitOr => v.reduce_or(), UnaryOp::BitXor => v.reduce_xor(),
                    UnaryOp::BitNand => v.reduce_and().logic_not(), UnaryOp::BitNor => v.reduce_or().logic_not(), UnaryOp::BitXnor => v.reduce_xor().logic_not(),
                    UnaryOp::PreIncr => { let nv = v.add(&Value::from_u64(1, v.width)); self.assign_value(operand, &nv); nv },
                    UnaryOp::PostIncr => { let nv = v.add(&Value::from_u64(1, v.width)); self.assign_value(operand, &nv); v },
                    UnaryOp::PreDecr => { let nv = v.sub(&Value::from_u64(1, v.width)); self.assign_value(operand, &nv); nv },
                    UnaryOp::PostDecr => { let nv = v.sub(&Value::from_u64(1, v.width)); self.assign_value(operand, &nv); v },
                    UnaryOp::HashHash => Value::zero(1),
                }
            }
            ExprKind::Binary { op, left, right } => {
                // Short-circuit evaluation for logical operators (IEEE §11.3.5)
                if matches!(op, BinaryOp::LogAnd | BinaryOp::LogOr) {
                    let l = self.eval_expr_ctx(left, ctx_width);
                    match op {
                        BinaryOp::LogAnd => {
                            if l.to_u64() == Some(0) { return Value::from_u64(0, 1); }
                            let r = self.eval_expr_ctx(right, ctx_width);
                            return l.logic_and(&r);
                        }
                        BinaryOp::LogOr => {
                            if l.to_u64().map_or(false, |v| v != 0) { return Value::from_u64(1, 1); }
                            let r = self.eval_expr_ctx(right, ctx_width);
                            return l.logic_or(&r);
                        }
                        _ => unreachable!()
                    }
                }
                // Unpacked array equality/inequality
                if matches!(op, BinaryOp::Eq | BinaryOp::Neq) {
                    if let (ExprKind::Ident(lhier), ExprKind::Ident(rhier)) = (&left.kind, &right.kind) {
                        let ln = self.resolve_hier_name(lhier);
                        let rn = self.resolve_hier_name(rhier);
                        if self.module.arrays.contains_key(&ln) && self.module.arrays.contains_key(&rn) {
                            let (llo, lhi, _) = self.module.arrays[&ln];
                            let (rlo, rhi, _) = self.module.arrays[&rn];
                            let lsize = (lhi - llo + 1) as usize;
                            let rsize = (rhi - rlo + 1) as usize;
                            if lsize != rsize { return Value::from_u64(if matches!(op, BinaryOp::Eq) { 0 } else { 1 }, 1); }
                            let l_desc = self.module.descending_arrays.contains(&ln);
                            let r_desc = self.module.descending_arrays.contains(&rn);
                            let mut equal = true;
                            for i in 0..lsize {
                                let lidx = if l_desc { lhi - i as i64 } else { llo + i as i64 };
                                let ridx = if r_desc { rhi - i as i64 } else { rlo + i as i64 };
                                let lv = self.get_signal_value_by_name(&format!("{}[{}]", ln, lidx)).unwrap_or(Value::zero(1));
                                let rv = self.get_signal_value_by_name(&format!("{}[{}]", rn, ridx)).unwrap_or(Value::zero(1));
                                if lv != rv { equal = false; break; }
                            }
                            let r = if matches!(op, BinaryOp::Eq) { equal } else { !equal };
                            return Value::from_u64(if r { 1 } else { 0 }, 1);
                        }
                    }
                }
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
                if c.has_unknown() {
                    // IEEE 1800 §11.4.11 Table 11-21: per-bit merge — bit is known
                    // only where both branches agree; otherwise X.
                    let t = self.eval_expr_ctx(then_expr, ctx_width);
                    let e = self.eval_expr_ctx(else_expr, ctx_width);
                    t.merge_unknown(&e)
                }
                else if c.is_true() { self.eval_expr_ctx(then_expr, ctx_width) } else { self.eval_expr_ctx(else_expr, ctx_width) }
            }
            ExprKind::Concatenation(parts) => { let mut r = Value::zero(0); for p in parts.iter().rev() { r = self.eval_expr(p).concat_with(&r); } r }
            ExprKind::StreamOp { left_to_right, slice_size, exprs } => {
                // Concat in source order: MSB = first expr. Build by concatenating from LSB (last expr).
                let mut concat = Value::zero(0);
                for p in exprs.iter().rev() {
                    // Special case: dynamic array/queue ident → concat all elements, idx0 at MSB.
                    let piece = if let ExprKind::Ident(h) = &p.kind {
                        let n = self.resolve_hier_name(h);
                        if self.module.arrays.contains_key(&n) {
                            let ew = self.lookup_signal_width(&format!("{}[0]", n))
                                .unwrap_or_else(|| self.module.arrays.get(&n).map(|t| t.2).unwrap_or(8)).max(1);
                            let sz = self.get_queue_size(&n) as usize;
                            let mut acc = Value::zero(0);
                            for i in 0..sz {
                                let ev = self.get_signal_value_by_name(&format!("{}[{}]", n, i)).unwrap_or(Value::zero(ew));
                                acc = acc.concat_with(&ev);
                            }
                            acc
                        } else { self.eval_expr(p) }
                    } else { self.eval_expr(p) };
                    concat = piece.concat_with(&concat);
                }
                let total_w = concat.width as usize;
                let streamed = if !*left_to_right {
                    concat
                } else {
                    let slice = slice_size.as_ref().map(|e| self.eval_expr(e).to_u64().unwrap_or(1) as usize).unwrap_or(1).max(1);
                    // Reverse successive slice-sized chunks starting from the MSB side.
                    let mut out = Value::zero(total_w as u32);
                    let full_chunks = total_w / slice;
                    let remainder = total_w - full_chunks * slice;
                    // Original bits [total_w-1 .. 0], where bit (total_w-1) is MSB.
                    // Source chunk k (0-indexed from MSB): bits [total_w-1-k*slice .. total_w-k*slice-slice].
                    // With left_to_right streaming, chunk order is preserved but bits *within* each chunk stay ordered.
                    // Actually `{<<slice {x}}` emits slice-sized groups MSB-first from x with each group's bits in LSB-first order inside the stream... The classic interpretation: slice the source into slice-sized chunks from LSB, then reverse the chunk order.
                    // Standard behavior: output = reverse the order of slice-sized chunks of the source.
                    for k in 0..full_chunks {
                        for b in 0..slice {
                            let src_bit = k * slice + b;
                            let dst_bit = (full_chunks - 1 - k) * slice + b + remainder;
                            if src_bit < total_w && dst_bit < total_w {
                                out.set_bit(dst_bit, concat.get_bit(src_bit));
                            }
                        }
                    }
                    // Leftover bits at the top of the source (fewer than slice) go to the LSB of output.
                    for b in 0..remainder {
                        let src_bit = full_chunks * slice + b;
                        let dst_bit = b;
                        if src_bit < total_w {
                            out.set_bit(dst_bit, concat.get_bit(src_bit));
                        }
                    }
                    out
                };
                // When RHS stream is assigned/evaluated in a wider context,
                // pad on the LSB side (stream sits at the MSB of the target).
                if ctx_width > streamed.width {
                    let target_w = ctx_width as usize;
                    let mut padded = Value::zero(ctx_width);
                    let shift = target_w - streamed.width as usize;
                    for b in 0..streamed.width as usize {
                        padded.set_bit(b + shift, streamed.get_bit(b));
                    }
                    padded
                } else {
                    streamed
                }
            }
            ExprKind::Replication { count, exprs } => {
                // Replication evaluation. Two correctness/perf fixes vs. the
                // old `for _ in 0..n { r = inner.concat_with(&r); }` loop:
                //
                // 1) **O(n × inner.width) instead of O(n²)**: pre-allocate
                //    the result `Value::zero(n × inner.width)` and copy
                //    `inner` into each slot at offset `k × inner.width`.
                //    Mirrors iverilog's `of_REPLICATE` (`vvp_vector4_t res
                //    (val.size() * rept, BIT4_X); for (idx) res.set_vec
                //    (idx * val.size(), val);`). The old loop allocated a
                //    growing `Value` each iteration and copied the
                //    accumulator — quadratic in `n`.
                //
                // 2) **Clamp the count**. `to_u64()` on a Value with X bits
                //    returns the masked value, which can be u64::MAX for
                //    pathological signals; the resulting `Value::zero(huge)`
                //    + `for _ in 0..huge` would either OOM or run forever.
                //    IEEE 1800 says replication with X count is undefined;
                //    treat anything above MAX_REPL as 0 and let downstream
                //    resize handle it.
                const MAX_REPL: u64 = 1 << 20;
                let count_val = self.eval_expr(count);
                let n_raw = count_val.to_u64().unwrap_or(1);
                let n = if n_raw > MAX_REPL { 0 } else { n_raw };
                let mut inner = Value::zero(0);
                for e in exprs.iter().rev() {
                    inner = self.eval_expr(e).concat_with(&inner);
                }
                let inner_w = inner.width as usize;
                let total_w = (inner_w as u64).saturating_mul(n) as u32;
                let mut r = Value::zero(total_w);
                if inner_w > 0 {
                    for k in 0..n as usize {
                        let off = k * inner_w;
                        for i in 0..inner_w {
                            r.set_bit(off + i, inner.get_bit(i));
                        }
                    }
                }
                r
            }
            ExprKind::Index { expr, index } => {
                // N-dimensional (N >= 3) unpacked array element access
                {
                    let mut cur = expr.as_ref();
                    let mut rev_idxs: Vec<&Expression> = vec![index.as_ref()];
                    while let ExprKind::Index { expr: inner_e, index: inner_i } = &cur.kind {
                        rev_idxs.push(inner_i.as_ref());
                        cur = inner_e.as_ref();
                    }
                    if let ExprKind::Ident(hier) = &cur.kind {
                        let base_name = self.resolve_hier_name(hier);
                        if let Some((shape, w)) = self.module.arrays_nd.get(&base_name).cloned() {
                            if rev_idxs.len() == shape.len() {
                                let mut name = base_name.clone();
                                for i in (0..rev_idxs.len()).rev() {
                                    let v = self.eval_expr(rev_idxs[i]).to_u64().unwrap_or(0) as i64;
                                    name = format!("{}[{}]", name, v);
                                }
                                if let Some(&eid) = self.signal_name_to_id.get(name.as_str()) {
                                    let mut v = self.signal_table[eid].clone();
                                    if self.signal_signed[eid] { v.is_signed = true; }
                                    return v;
                                }
                                if let Some(sv) = self.signals.get(&name) { return sv.clone(); }
                                return Value::new(w);
                            }
                        }
                    }
                }
                // 2D array element access: mem[i][j]
                if let ExprKind::Index { expr: inner_expr, index: inner_idx } = &expr.kind {
                    if let ExprKind::Ident(hier) = &inner_expr.kind {
                        let name = self.resolve_hier_name(hier);
                        if self.module.arrays_2d.contains_key(&name) {
                            let i = self.eval_expr(inner_idx).to_u64().unwrap_or(0) as i64;
                            let j = self.eval_expr(index).to_u64().unwrap_or(0) as i64;
                            let elem_name = format!("{}[{}][{}]", name, i, j);
                            if let Some(&eid) = self.signal_name_to_id.get(elem_name.as_str()) {
                                let mut v = self.signal_table[eid].clone();
                                if self.signal_signed[eid] { v.is_signed = true; }
                                return v;
                            }
                            if let Some(sv) = self.signals.get(&elem_name) { return sv.clone(); }
                            let w = self.module.arrays_2d.get(&name).map(|t| t.2).unwrap_or(1);
                            return Value::new(w);
                        }
                    }
                }
                // Check if this is an array element access (memory[idx]) vs bit select
                if let ExprKind::Ident(hier) = &expr.kind {
                    let name = self.resolve_hier_name(hier);
                    if self.module.arrays.contains_key(&name) || self.is_associative_array(&name) {
                        // Array element access: look up signal "name[idx]"
                        if self.module.dynamic_arrays.contains(&name) {
                            self.scriptllar_bound.push((self.get_queue_size(&name) as i64) - 1);
                        }
                        let idx_val = self.eval_expr(index);
                        if self.module.dynamic_arrays.contains(&name) { self.scriptllar_bound.pop(); }
                        let idx_str = if self.is_associative_array(&name) {
                            self.assoc_key_str(&name, &idx_val)
                        } else {
                            idx_val.to_u64().unwrap_or(0).to_string()
                        };
                        let elem_name = format!("{}[{}]", name, idx_str);
                        if let Some(&eid) = self.signal_name_to_id.get(elem_name.as_str()) {
                            let mut v = self.signal_table[eid].clone();
                            if self.signal_signed[eid] { v.is_signed = true; }
                            return v;
                        }
                        let mut v = if let Some(sv) = self.signals.get(&elem_name) {
                            sv.clone()
                        } else if let Some(def_expr) = self.module.assoc_defaults.get(&name).cloned() {
                            self.eval_expr(&def_expr)
                        } else {
                            Value::new(1)
                        };
                        if self.signed_signals.contains(&elem_name) { v.is_signed = true; }
                        return v;
                    }
                }
                // Fall back to bit select
                self.eval_expr(expr).bit_select(self.eval_expr(index).to_u64().unwrap_or(0) as usize)
            }
            ExprKind::RangeSelect { expr, left, right, kind, .. } => {
                // Unpacked array slice: concatenate elements
                if let ExprKind::Ident(hier) = &expr.kind {
                    let name = self.resolve_hier_name(hier);
                    if let Some(&(arr_lo, arr_hi, elem_w)) = self.module.arrays.get(&name) {
                        let is_dyn = self.module.dynamic_arrays.contains(&name);
                        let upper_bound: i64 = if is_dyn {
                            (self.get_queue_size(&name) as i64) - 1
                        } else {
                            arr_hi
                        };
                        self.scriptllar_bound.push(upper_bound);
                        let l = self.eval_expr(left).to_i64().unwrap_or(0);
                        let r = self.eval_expr(right).to_i64().unwrap_or(0);
                        self.scriptllar_bound.pop();
                        let (lo, hi) = match kind {
                            RangeKind::Constant => (l.min(r), l.max(r)),
                            RangeKind::IndexedUp => (l, l + r - 1),
                            RangeKind::IndexedDown => (l - r + 1, l),
                        };
                        if hi < lo { return Value::zero(0); }
                        let lo = lo.max(arr_lo);
                        let hi = hi.min(if is_dyn { upper_bound } else { arr_hi });
                        if hi < lo { return Value::zero(0); }
                        let count = (hi - lo + 1) as usize;
                        let total_w = count as u32 * elem_w;
                        let mut acc = Value::zero(total_w);
                        for i in 0..count {
                            let idx = lo + i as i64;
                            if idx < arr_lo || idx > arr_hi { continue; }
                            let elem_name = format!("{}[{}]", name, idx);
                            let v = self.get_signal_value_by_name(&elem_name).unwrap_or(Value::zero(elem_w));
                            for b in 0..elem_w as usize {
                                acc.set_bit(i * elem_w as usize + b, v.get_bit(b));
                            }
                        }
                        return acc;
                    }
                }
                let base = self.eval_expr(expr); let l = self.eval_expr(left).to_u64().unwrap_or(0) as usize; let r = self.eval_expr(right).to_u64().unwrap_or(0) as usize;
                let result = match kind { RangeKind::Constant => base.range_select(l, r), RangeKind::IndexedUp => base.range_select(l+r-1, l), RangeKind::IndexedDown => base.range_select(l, l.saturating_sub(r-1)) };
                result
            }
            ExprKind::Inside { expr, ranges } => {
                let val = self.eval_expr(expr);
                for r in ranges {
                    match &r.kind {
                        ExprKind::Range(lo, hi) => {
                            let l = self.eval_expr(lo);
                            let h = self.eval_expr(hi);
                            if val.greater_equal(&l).is_true() && val.less_equal(&h).is_true() { return Value::from_u64(1, 1); }
                        }
                        _ => {
                            if val == self.eval_expr(r) { return Value::from_u64(1, 1); }
                        }
                    }
                }
                Value::zero(1)
            }
            ExprKind::Range(lo, hi) => {
                // Standalone range shouldn't really happen except inside Inside, but handle it
                let l = self.eval_expr(lo);
                let h = self.eval_expr(hi);
                l.concat_with(&h) // Dummy representation
            }
            ExprKind::Paren(inner) => self.eval_expr_ctx(inner, ctx_width),
            ExprKind::AssignExpr { lvalue, rvalue } => {
                let v = self.eval_expr(rvalue);
                self.assign_value(lvalue, &v);
                v
            }
            ExprKind::SystemCall { name, args } => match name.as_str() {
                "$clog2" => { let v = args.first().map(|a| self.eval_expr(a).to_u64().unwrap_or(0)).unwrap_or(0); Value::from_u64(if v <= 1 { 1 } else { 64 - (v-1).leading_zeros() } as u64, 32) }
                "$bits" => {
                    if let Some(arg) = args.first() {
                        if let ExprKind::Ident(hier) = &arg.kind {
                             let name = self.resolve_hier_name(hier);
                             if let Some(w) = self.module.typedefs.get(&name) {
                                 return Value::from_u64(*w as u64, 32);
                             }
                        }
                        Value::from_u64(self.eval_expr(arg).width as u64, 32)
                    } else { Value::zero(32) }
                }
                "$signed" => { let mut v = args.first().map(|a| self.eval_expr(a)).unwrap_or(Value::zero(32)); v.is_signed = true; v }
                "$unsigned" => { let mut v = args.first().map(|a| self.eval_expr(a)).unwrap_or(Value::zero(32)); v.is_signed = false; v }
                "$time" => Value::from_u64(self.time, 64),
                "$test$plusargs" => {
                    sim_dbg_eprintln!("[DEBUG] eval $test$plusargs with {} args", args.len());
                    let pat = match args.first().map(|a| &a.kind) {
                        Some(ExprKind::StringLiteral(s)) => s.clone(),
                        Some(_) => self.eval_expr(&args[0]).to_sv_string(),
                        None => String::new(),
                    };
                    Value::from_u64(self.test_plusarg(&pat) as u64, 1)
                }
                "$value$plusargs" => self.eval_value_plusargs(args),
                "$fopen" => self.open_file_handle(args),
                "$fclose" => self.close_file_handle(args),
                "$fwrite" => self.write_file_handle(args, false),
                "$fdisplay" => self.write_file_handle(args, true),
                "$ftell" => {
                    use std::io::Seek;
                    let fd = args.first().map(|a| self.eval_file_handle_arg(a)).unwrap_or(0);
                    let pos = self.file_handles.get_mut(&fd).and_then(|f| f.stream_position().ok()).unwrap_or(0);
                    Value::from_u64(pos, 32)
                }
                "$fseek" => {
                    use std::io::{Seek, SeekFrom};
                    let fd = args.first().map(|a| self.eval_file_handle_arg(a)).unwrap_or(0);
                    let off = args.get(1).map(|a| self.eval_expr(a).to_u64().unwrap_or(0) as i64).unwrap_or(0);
                    let whence = args.get(2).map(|a| self.eval_expr(a).to_u64().unwrap_or(0)).unwrap_or(0);
                    let from = match whence { 1 => SeekFrom::Current(off), 2 => SeekFrom::End(off), _ => SeekFrom::Start(off as u64) };
                    if let Some(f) = self.file_handles.get_mut(&fd) { let _ = f.seek(from); }
                    Value::zero(32)
                }
                "$rewind" => {
                    use std::io::{Seek, SeekFrom};
                    let fd = args.first().map(|a| self.eval_file_handle_arg(a)).unwrap_or(0);
                    if let Some(f) = self.file_handles.get_mut(&fd) { let _ = f.seek(SeekFrom::Start(0)); }
                    Value::zero(32)
                }
                "$ungetc" => {
                    let ch = args.first().map(|a| self.eval_expr(a).to_u64().unwrap_or(0) as u8).unwrap_or(0);
                    let fd = args.get(1).map(|a| self.eval_file_handle_arg(a)).unwrap_or(0);
                    self.ungetc_buf.entry(fd).or_default().push(ch);
                    Value::from_u64(ch as u64, 32)
                }
                "$fgetc" => {
                    use std::io::Read;
                    let fd = args.first().map(|a| self.eval_file_handle_arg(a)).unwrap_or(0);
                    if let Some(buf) = self.ungetc_buf.get_mut(&fd) {
                        if let Some(c) = buf.pop() { return Value::from_u64(c as u64, 32); }
                    }
                    if let Some(f) = self.file_handles.get_mut(&fd) {
                        let mut b = [0u8; 1];
                        if f.read(&mut b).unwrap_or(0) == 1 { return Value::from_u64(b[0] as u64, 32); }
                    }
                    Value::from_u64(u32::MAX as u64, 32)
                }
                "$readmemh" => self.read_memory_file(args, 16),
                "$readmemb" => self.read_memory_file(args, 2),
                "$random" => Value::from_u64(0, 32), // stub
                "$isunknown" => { let v = args.first().map(|a| self.eval_expr(a)).unwrap_or(Value::zero(1)); Value::from_u64(v.has_xz() as u64, 1) }
                "$realtobits" => { let v = args.first().map(|a| self.eval_expr(a)).unwrap_or(Value::zero(64)); v }
                "$bitstoreal" => { let v = args.first().map(|a| self.eval_expr(a)).unwrap_or(Value::zero(64)); v }
                "$itor" => { let v = args.first().map(|a| self.eval_expr(a)).unwrap_or(Value::zero(32)); Value::from_f64(v.to_u64().unwrap_or(0) as f64) }
                "$rtoi" => { let v = args.first().map(|a| self.eval_expr(a)).unwrap_or(Value::zero(64)); Value::from_u64(v.to_f64() as u64, 32) }
                "$ceil" => { let v = args.first().map(|a| self.eval_expr(a)).unwrap_or(Value::zero(64)); Value::from_f64(v.to_f64().ceil()) }
                "$floor" => { let v = args.first().map(|a| self.eval_expr(a)).unwrap_or(Value::zero(64)); Value::from_f64(v.to_f64().floor()) }
                "$sqrt" => { let v = args.first().map(|a| self.eval_expr(a)).unwrap_or(Value::zero(64)); Value::from_u64(v.to_f64().sqrt() as u64, 32) }
                "$pow" => { let a = args.get(0).map(|a| self.eval_expr(a)).unwrap_or(Value::zero(64)); let b = args.get(1).map(|a| self.eval_expr(a)).unwrap_or(Value::zero(64)); Value::from_f64(a.to_f64().powf(b.to_f64())) }
                "$log10" => { let v = args.first().map(|a| self.eval_expr(a)).unwrap_or(Value::zero(64)); Value::from_u64(v.to_f64().log10() as u64, 32) }
                "$exp" => { let v = args.first().map(|a| self.eval_expr(a)).unwrap_or(Value::zero(64)); Value::from_f64(v.to_f64().exp()) }
                "$ln" => { let v = args.first().map(|a| self.eval_expr(a)).unwrap_or(Value::zero(64)); Value::from_f64(v.to_f64().ln()) }
                "$log2" => { let v = args.first().map(|a| self.eval_expr(a)).unwrap_or(Value::zero(64)); Value::from_f64(v.to_f64().log2()) }
                "$clog2" => { let v = args.first().map(|a| self.eval_expr(a)).unwrap_or(Value::zero(32)); let n = v.to_u64().unwrap_or(0); Value::from_u64(if n <= 1 { 0 } else { 64 - (n - 1).leading_zeros() as u64 }, 32) }
                "$shortrealtobits" => { let v = args.first().map(|a| self.eval_expr(a)).unwrap_or(Value::zero(32)); let f = v.to_f64() as f32; Value::from_u64(f.to_bits() as u64, 32) }
                "$bitstoshortreal" => { let v = args.first().map(|a| self.eval_expr(a)).unwrap_or(Value::zero(32)); let f = f32::from_bits(v.to_u64().unwrap_or(0) as u32); Value::from_f64(f as f64) }
                "$bits" => {
                    if let Some(arg) = args.first() {
                        let v = self.eval_expr(arg);
                        Value::from_u64(v.width as u64, 32)
                    } else { Value::zero(32) }
                }
                "$dimensions" | "$unpacked_dimensions" => {
                    if let Some(arg) = args.first() {
                        if let ExprKind::Ident(hier) = &arg.kind {
                            let aname = self.resolve_hier_name(hier);
                            let has_unpacked = self.module.arrays.contains_key(&aname);
                            let packed_w = if has_unpacked {
                                self.module.arrays[&aname].2
                            } else if let Some(&id) = self.signal_name_to_id.get(aname.as_str()) {
                                self.signal_widths[id]
                            } else { 0 };
                            if name == "$unpacked_dimensions" {
                                return Value::from_u64(if has_unpacked { 1 } else { 0 }, 32);
                            }
                            let packed_dim: u64 = if packed_w > 1 { 1 } else { 0 };
                            let unpacked_dim: u64 = if has_unpacked { 1 } else { 0 };
                            let total = packed_dim + unpacked_dim;
                            return Value::from_u64(total.max(1), 32);
                        }
                    }
                    Value::from_u64(1, 32)
                }
                sn @ ("$left" | "$high" | "$right" | "$low" | "$size" | "$increment") => {
                    let sn = sn.to_string();
                    let dim = args.get(1).map(|a| self.eval_expr(a).to_u64().unwrap_or(1)).unwrap_or(1) as usize;
                    if let Some(arg) = args.first() {
                        if let ExprKind::Ident(hier) = &arg.kind {
                            let aname = self.resolve_hier_name(hier);
                            let unpacked = self.module.arrays.get(&aname).cloned();
                            let packed_w = if let Some((_,_,w)) = unpacked { w }
                                else if let Some(&id) = self.signal_name_to_id.get(aname.as_str()) {
                                    self.signal_widths[id]
                                } else { 0 };
                            let (lo, hi, descending) = if unpacked.is_some() && dim == 1 {
                                let (l, h, _) = unpacked.unwrap();
                                let desc = self.module.descending_arrays.contains(&aname);
                                (l, h, desc)
                            } else {
                                (0i64, packed_w as i64 - 1, true)
                            };
                            let left = if descending { hi } else { lo };
                            let right = if descending { lo } else { hi };
                            let size = (hi - lo + 1).max(0) as u64;
                            let result = match sn.as_str() {
                                "$left" => left as u64,
                                "$right" => right as u64,
                                "$high" => hi as u64,
                                "$low" => lo as u64,
                                "$size" => size,
                                "$increment" => if descending { 1 } else { 0u64.wrapping_sub(1) },
                                _ => 0,
                            };
                            return Value::from_u64(result, 32);
                        }
                        let v = self.eval_expr(arg);
                        match sn.as_str() {
                            "$left" | "$high" => return Value::from_u64(v.width as u64 - 1, 32),
                            "$size" => return Value::from_u64(v.width as u64, 32),
                            "$increment" => return Value::from_u64(1, 32),
                            _ => return Value::zero(32),
                        }
                    }
                    Value::zero(32)
                }
                "$typename" => {
                    if let Some(arg) = args.first() {
                        if let ExprKind::Ident(hier) = &arg.kind {
                            let name = self.resolve_hier_name(hier);
                            if let Some(&id) = self.signal_name_to_id.get(name.as_str()) {
                                let w = self.signal_widths[id];
                                let s = if w == 1 { "logic" } else { "logic" };
                                return Value::from_string(s);
                            }
                        }
                    }
                    Value::from_string("logic")
                }
                "$isunbounded" => {
                    if let Some(arg) = args.first() {
                        if let ExprKind::Dollar = &arg.kind {
                            return Value::from_u64(1, 1);
                        }
                        let v = self.eval_expr(arg);
                        // $ is stored as all-ones in the parameter's width
                        let all_ones = if v.width >= 64 { u64::MAX } else { (1u64 << v.width) - 1 };
                        Value::from_u64(if v.to_u64() == Some(all_ones) { 1 } else { 0 }, 1)
                    } else { Value::zero(1) }
                }
                "$countbits" => {
                    if args.len() >= 2 {
                        let v = self.eval_expr(&args[0]);
                        // Collect target bit values from remaining args
                        let mut targets = Vec::new();
                        for arg in &args[1..] {
                            if let ExprKind::Number(NumberLiteral::UnbasedUnsized(c)) = &arg.kind {
                                targets.push(match c {
                                    '0' => super::value::LogicBit::Zero,
                                    '1' => super::value::LogicBit::One,
                                    'x' | 'X' => super::value::LogicBit::X,
                                    'z' | 'Z' => super::value::LogicBit::Z,
                                    _ => super::value::LogicBit::One,
                                });
                            } else {
                                let bv = self.eval_expr(arg).to_u64().unwrap_or(1);
                                targets.push(if bv == 0 { super::value::LogicBit::Zero } else { super::value::LogicBit::One });
                            }
                        }
                        let mut count = 0u64;
                        for i in 0..v.width as usize {
                            let b = v.get_bit(i);
                            if targets.contains(&b) { count += 1; }
                        }
                        Value::from_u64(count, 32)
                    } else { Value::zero(32) }
                }
                "$countones" => {
                    let v = args.first().map(|a| self.eval_expr(a)).unwrap_or(Value::zero(32));
                    let mut count = 0u64;
                    for i in 0..v.width as usize {
                        if v.get_bit(i) == super::value::LogicBit::One { count += 1; }
                    }
                    Value::from_u64(count, 32)
                }
                "$onehot" => {
                    let v = args.first().map(|a| self.eval_expr(a)).unwrap_or(Value::zero(32));
                    let mut count = 0u64;
                    for i in 0..v.width as usize {
                        if v.get_bit(i) == super::value::LogicBit::One { count += 1; }
                    }
                    Value::from_u64(if count == 1 { 1 } else { 0 }, 1)
                }
                "$onehot0" => {
                    let v = args.first().map(|a| self.eval_expr(a)).unwrap_or(Value::zero(32));
                    let mut count = 0u64;
                    for i in 0..v.width as usize {
                        if v.get_bit(i) == super::value::LogicBit::One { count += 1; }
                    }
                    Value::from_u64(if count <= 1 { 1 } else { 0 }, 1)
                }
                "$sscanf" => {
                    if args.len() >= 3 {
                        if let ExprKind::StringLiteral(src) = &args[0].kind {
                            if let ExprKind::StringLiteral(fmt) = &args[1].kind {
                                if fmt.contains("%d") || fmt.contains("%i") {
                                    if let Ok(n) = src.trim().parse::<i64>() {
                                        self.assign_value(&args[2], &Value::from_u64(n as u64, 32));
                                        return Value::from_u64(1, 32);
                                    }
                                }
                            }
                        }
                        let src_val = self.eval_expr(&args[0]);
                        let src_str = src_val.to_sv_string();
                        if let ExprKind::StringLiteral(fmt) = &args[1].kind {
                            if fmt.contains("%d") || fmt.contains("%i") {
                                if let Ok(n) = src_str.trim().parse::<i64>() {
                                    self.assign_value(&args[2], &Value::from_u64(n as u64, 32));
                                    return Value::from_u64(1, 32);
                                }
                            }
                        }
                    }
                    Value::zero(32)
                }
                _ => Value::zero(32),
            },
            ExprKind::This => {
                if let Some(Some(handle)) = self.this_stack.last() {
                    Value::from_u64(*handle as u64, 32)
                } else {
                    Value::zero(32)
                }
            }
            ExprKind::MemberAccess { expr, member } => {
                // Local-stack dotted lookup: e.g. "item.index" inside a with-clause
                if let ExprKind::Ident(hier) = &expr.kind {
                    if hier.path.len() == 1 {
                        let base_name = &hier.path[0].name.name;
                        let dotted = format!("{}.{}", base_name, member.name);
                        if let Some(locals) = self.local_stack.last() {
                            if let Some(v) = locals.get(&dotted) { return v.clone(); }
                        }
                    }
                }
                if let ExprKind::Ident(hier) = &expr.kind {
                    let name = self.resolve_hier_name(hier);
                    if let Some(fields) = self.module.packed_struct_fields.get(&name).cloned() {
                        if let Some((_, off, w)) = fields.iter().find(|(m, _, _)| m == &member.name).cloned() {
                            if let Some(sig) = self.get_signal_value_by_name(&name) {
                                return sig.range_select((off + w - 1) as usize, off as usize);
                            }
                        }
                    }
                    if self.is_associative_array(&name) {
                        let mname = member.name.as_str();
                        if mname == "size" || mname == "num" {
                            let prefix = format!("{}[", name);
                            let count = self.signals.keys().filter(|k| k.starts_with(&prefix)).count();
                            return Value::from_u64(count as u64, 32);
                        }
                        if mname == "delete" {
                            let prefix = format!("{}[", name);
                            let keys: Vec<String> = self.signals.keys().filter(|k| k.starts_with(&prefix)).cloned().collect();
                            for k in keys { self.signals.remove(&k); }
                            return Value::zero(32);
                        }
                        let qualified = format!("{}.{}", name, mname);
                        if let Some(v) = self.lookup_signal_value(&qualified) {
                            return v;
                        }
                        return Value::zero(32);
                    }
                }
                let base = self.eval_expr(expr);
                let handle = base.to_u64().unwrap_or(0) as usize;
                if handle == 0 || handle >= self.heap.len() {
                    Value::zero(32)
                } else if let Some(instance) = &self.heap[handle] {
                    instance.properties.get(&member.name).cloned().unwrap_or(Value::zero(32))
                } else {
                    Value::zero(32)
                }
            }
            ExprKind::Call { func, args } => self.eval_call(func, args),
            ExprKind::Dollar => {
                if let Some(&b) = self.scriptllar_bound.last() {
                    let mut v = Value::from_u64(b as u64, 32);
                    v.is_signed = true;
                    v
                } else {
                    Value::from_u64(u64::MAX, 32)
                }
            }
            ExprKind::Null => Value::zero(32),
            ExprKind::Empty => Value::zero(1),
            ExprKind::WithClause { expr, filter } => {
                // In expression context, evaluate the inner expression (with clause is handled at assignment level)
                self.eval_expr(expr)
            }
            ExprKind::AssignmentPattern(parts) => { let mut r = Value::zero(0); for p in parts.iter().rev() { r = self.eval_expr(p.expr()).concat_with(&r); } r }
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
            NumberLiteral::Real(f) => Value::from_f64(*f),
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
                // Tagged union assignment: un = tagged Name (inner);
                if let ExprKind::Tagged { tag, inner } = &rvalue.kind {
                    if let ExprKind::Ident(lh) = &lvalue.kind {
                        let lname = self.resolve_hier_name(lh);
                        self.active_union_tag.insert(lname.clone(), tag.name.clone());
                        if let Some(inner_expr) = inner {
                            let v = self.eval_expr(inner_expr);
                            self.set_signal_value_by_name(&lname, v);
                        } else {
                            let w = self.lookup_signal_width(&lname).unwrap_or(1);
                            self.set_signal_value_by_name(&lname, Value::zero(w));
                        }
                        if !self.in_edge_block { self.settle_combinatorial(); }
                        return;
                    }
                }
                // Slice assignment for N-dimensional unpacked arrays where LHS
                // and RHS both supply fewer indices than dimensions:
                //   B[i][j] = A[p][q];   // with A, B both 3D ⇒ copy inner dim
                {
                    fn unwrap_nd<'a>(e: &'a Expression) -> (&'a Expression, Vec<&'a Expression>) {
                        let mut cur = e;
                        let mut rev_idxs: Vec<&Expression> = Vec::new();
                        while let ExprKind::Index { expr: inner_e, index: inner_i } = &cur.kind {
                            rev_idxs.push(inner_i.as_ref());
                            cur = inner_e.as_ref();
                        }
                        (cur, rev_idxs)
                    }
                    let (lbase, lrev) = unwrap_nd(lvalue);
                    let (rbase, rrev) = unwrap_nd(rvalue);
                    if let (ExprKind::Ident(lh), ExprKind::Ident(rh)) = (&lbase.kind, &rbase.kind) {
                        let lname = self.resolve_hier_name(lh);
                        let rname = self.resolve_hier_name(rh);
                        let lshape = self.module.arrays_nd.get(&lname).cloned();
                        let rshape = self.module.arrays_nd.get(&rname).cloned();
                        if let (Some((ls, _)), Some((rs, _))) = (lshape, rshape) {
                            if ls.len() == rs.len()
                                && lrev.len() < ls.len()
                                && lrev.len() == rrev.len()
                            {
                                let given = lrev.len();
                                let mut l_prefix = lname.clone();
                                let mut r_prefix = rname.clone();
                                for i in (0..given).rev() {
                                    let lv = self.eval_expr(lrev[i]).to_u64().unwrap_or(0) as i64;
                                    let rv = self.eval_expr(rrev[i]).to_u64().unwrap_or(0) as i64;
                                    l_prefix = format!("{}[{}]", l_prefix, lv);
                                    r_prefix = format!("{}[{}]", r_prefix, rv);
                                }
                                let remaining: Vec<(i64, i64)> = ls[given..].iter().copied().collect();
                                let rem_r: Vec<(i64, i64)> = rs[given..].iter().copied().collect();
                                if remaining == rem_r {
                                    fn enum_idx(dims: &[(i64, i64)], lp: String, rp: String,
                                                out: &mut Vec<(String, String)>) {
                                        if dims.is_empty() { out.push((lp, rp)); return; }
                                        let (lo, hi) = dims[0];
                                        for i in lo..=hi {
                                            enum_idx(&dims[1..],
                                                format!("{}[{}]", lp, i),
                                                format!("{}[{}]", rp, i), out);
                                        }
                                    }
                                    let mut pairs = Vec::new();
                                    enum_idx(&remaining, l_prefix, r_prefix, &mut pairs);
                                    for (lp, rp) in pairs {
                                        let v = self.get_signal_value_by_name(&rp).unwrap_or(Value::zero(1));
                                        self.set_signal_value_by_name(&lp, v);
                                    }
                                    if !self.in_edge_block { self.settle_combinatorial(); }
                                    return;
                                }
                            }
                        }
                    }
                }
                // Handle LHS streaming concat: {<<slice {a, b, c, ...}} = rhs;
                // Handle RHS streaming concat to a dynamic array/queue target:
                //   queue_of_packed = {<<slice {a, b, c, ...}};
                // Distribute the streamed bits element-by-element (MSB first).
                if let ExprKind::StreamOp { .. } = &rvalue.kind {
                    if let ExprKind::Ident(lh) = &lvalue.kind {
                        let lname = self.resolve_hier_name(lh);
                        if self.module.arrays.contains_key(&lname) {
                            let elem_w = self.lookup_signal_width(&format!("{}[0]", lname))
                                .unwrap_or_else(|| self.module.arrays.get(&lname).map(|t| t.2).unwrap_or(8)).max(1);
                            // Evaluate stream raw (no pad) by using ctx 0.
                            let sv = self.eval_expr_ctx(rvalue, 0);
                            let total = sv.width as usize;
                            let n_elems = (total + elem_w as usize - 1) / elem_w as usize;
                            self.set_queue_size(&lname, n_elems as u64);
                            for k in 0..n_elems {
                                let hi = total.saturating_sub(k * elem_w as usize).saturating_sub(1);
                                let lo_raw = total.saturating_sub((k + 1) * elem_w as usize);
                                let lo = lo_raw;
                                let piece = if hi >= lo { sv.range_select(hi, lo) } else { Value::zero(elem_w) };
                                self.signals.insert(format!("{}[{}]", lname, k), piece.clone());
                                self.widths.insert(format!("{}[{}]", lname, k), elem_w);
                            }
                            if !self.in_edge_block { self.settle_combinatorial(); }
                            return;
                        }
                    }
                }
                if let ExprKind::StreamOp { left_to_right, slice_size, exprs } = &lvalue.kind {
                    // Gather RHS bits: if it's a dynamic array/queue of a packed
                    // element type, concatenate elements with index 0 at the MSB.
                    let rhs_val = if let ExprKind::Ident(rhier) = &rvalue.kind {
                        let rname = self.resolve_hier_name(rhier);
                        if self.module.arrays.contains_key(&rname) || self.module.dynamic_arrays.contains(&rname) {
                            let n = self.get_queue_size(&rname) as usize;
                            let elem_w = self.lookup_signal_width(&format!("{}[0]", rname))
                                .unwrap_or_else(|| self.module.arrays.get(&rname).map(|t| t.2).unwrap_or(8));
                            let total_w = (n as u32) * elem_w;
                            let mut packed = Value::zero(total_w);
                            for i in 0..n {
                                let ev = self.get_signal_value_by_name(&format!("{}[{}]", rname, i))
                                    .unwrap_or(Value::zero(elem_w));
                                // pack at position [total_w-1-i*elem_w .. total_w-(i+1)*elem_w]
                                let dst_base = total_w as usize - (i + 1) * elem_w as usize;
                                for b in 0..elem_w as usize {
                                    packed.set_bit(dst_base + b, ev.get_bit(b));
                                }
                            }
                            packed
                        } else {
                            self.eval_expr(rvalue)
                        }
                    } else {
                        self.eval_expr(rvalue)
                    };
                    // Apply inverse stream op on rhs_val: the stream op is its own inverse
                    // per slice, so we re-apply it to obtain the "ordered" bits.
                    let total_w = rhs_val.width as usize;
                    let ordered = if !*left_to_right {
                        rhs_val
                    } else {
                        let slice = slice_size.as_ref().map(|e| self.eval_expr(e).to_u64().unwrap_or(1) as usize).unwrap_or(1).max(1);
                        let mut out = Value::zero(total_w as u32);
                        let full_chunks = total_w / slice;
                        let remainder = total_w - full_chunks * slice;
                        for k in 0..full_chunks {
                            for b in 0..slice {
                                let src_bit = k * slice + b;
                                let dst_bit = (full_chunks - 1 - k) * slice + b + remainder;
                                if src_bit < total_w && dst_bit < total_w {
                                    out.set_bit(dst_bit, rhs_val.get_bit(src_bit));
                                }
                            }
                        }
                        for b in 0..remainder {
                            let src_bit = full_chunks * slice + b;
                            let dst_bit = b;
                            if src_bit < total_w {
                                out.set_bit(dst_bit, rhs_val.get_bit(src_bit));
                            }
                        }
                        out
                    };
                    // Distribute `ordered` MSB-first to each target in source order.
                    let total = ordered.width as usize;
                    // Compute fixed-width targets; last target may be dynamic (remainder).
                    let mut fixed_ws: Vec<u32> = Vec::new();
                    let mut dyn_last: Option<String> = None;
                    let mut dyn_last_elem_w: u32 = 8;
                    for (i, e) in exprs.iter().enumerate() {
                        let is_last = i == exprs.len() - 1;
                        if is_last {
                            if let ExprKind::Ident(h) = &e.kind {
                                let n = self.resolve_hier_name(h);
                                if self.module.dynamic_arrays.contains(&n) || self.module.arrays.contains_key(&n) {
                                    let ew = self.lookup_signal_width(&format!("{}[0]", n))
                                        .unwrap_or_else(|| self.module.arrays.get(&n).map(|t| t.2).unwrap_or(8));
                                    dyn_last = Some(n);
                                    dyn_last_elem_w = ew;
                                    continue;
                                }
                            }
                        }
                        fixed_ws.push(self.infer_lhs_width(e));
                    }
                    let fixed_total: u32 = fixed_ws.iter().sum();
                    let mut msb = total; // bit position just above the MSB
                    for (i, e) in exprs.iter().enumerate() {
                        let is_last = i == exprs.len() - 1;
                        if is_last && dyn_last.is_some() {
                            let remaining = (total as u32).saturating_sub(fixed_total);
                            let elem_w = dyn_last_elem_w.max(1);
                            let n_elems = (remaining / elem_w) as usize;
                            let dname = dyn_last.clone().unwrap();
                            self.set_queue_size(&dname, n_elems as u64);
                            for k in 0..n_elems {
                                let hi = msb.saturating_sub(k * elem_w as usize).saturating_sub(1);
                                let lo = msb.saturating_sub((k + 1) * elem_w as usize);
                                if hi >= lo {
                                    let slice_v = ordered.range_select(hi, lo);
                                    self.set_signal_value_by_name(&format!("{}[{}]", dname, k), slice_v);
                                }
                            }
                        } else {
                            let w = self.infer_lhs_width(e) as usize;
                            if w == 0 { continue; }
                            let hi = msb.saturating_sub(1);
                            let lo = msb.saturating_sub(w);
                            let slice_v = if hi >= lo { ordered.range_select(hi, lo) } else { Value::zero(w as u32) };
                            self.assign_value(e, &slice_v);
                            msb = lo;
                        }
                    }
                    if !self.in_edge_block { self.settle_combinatorial(); }
                    return;
                }
                // Handle array locator methods with `with` clause: qs = arr.find with (filter)
                if let ExprKind::WithClause { expr: wexpr, filter } = &rvalue.kind {
                    if let ExprKind::MemberAccess { expr: arr_expr, member } = &wexpr.kind {
                        if let ExprKind::Ident(hier) = &arr_expr.kind {
                            let arr_name = self.resolve_hier_name(hier);
                            let mname = member.name.as_str();
                            if matches!(mname, "find" | "find_first" | "find_last" | "find_index" | "find_first_index" | "find_last_index" | "unique" | "unique_index" | "min" | "max") {
                                if let ExprKind::Ident(lhier) = &lvalue.kind {
                                    let lname = self.resolve_hier_name(lhier);
                                    let cur_size = self.get_queue_size(&arr_name) as usize;
                                    let mut results = Vec::new();
                                    for i in 0..cur_size {
                                        if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", arr_name, i)) {
                                            // Bind "item" and "item.index" in local stack
                                            let mut locals = HashMap::new();
                                            locals.insert("item".to_string(), v.clone());
                                            locals.insert("item.index".to_string(), Value::from_u64(i as u64, 32));
                                            self.local_stack.push(locals);
                                            let cond = self.eval_expr(filter);
                                            self.local_stack.pop();
                                            if cond.is_true() {
                                                if mname.contains("index") { results.push(Value::from_u64(i as u64, 32)); }
                                                else { results.push(v); }
                                            }
                                        }
                                    }
                                    if mname.contains("first") { results.truncate(1); }
                                    if mname.contains("last") && !results.is_empty() {
                                        let last = results.pop().unwrap();
                                        results = vec![last];
                                    }
                                    // Assign results to destination queue
                                    for (i, v) in results.iter().enumerate() {
                                        self.set_signal_value_by_name(&format!("{}[{}]", lname, i), v.clone());
                                    }
                                    self.set_queue_size(&lname, results.len() as u64);
                                }
                                if !self.in_edge_block { self.settle_combinatorial(); }
                                return;
                            }
                        }
                    }
                }
                // Handle queue = array.locator_method (no with clause)
                // Detect via MemberAccess or hierarchical ident (e.g. s.unique_index)
                let locator_info: Option<(String, &str)> = if let ExprKind::MemberAccess { expr: arr_expr, member } = &rvalue.kind {
                    let mname = member.name.as_str();
                    if matches!(mname, "min" | "max" | "unique" | "unique_index") {
                        if let ExprKind::Ident(ahier) = &arr_expr.kind {
                            Some((self.resolve_hier_name(ahier), mname))
                        } else { None }
                    } else { None }
                } else if let ExprKind::Ident(rhier) = &rvalue.kind {
                    if rhier.path.len() == 2 {
                        let arr_name = &rhier.path[0].name.name;
                        let mname = rhier.path[1].name.name.as_str();
                        if matches!(mname, "min" | "max" | "unique" | "unique_index") && self.module.arrays.contains_key(arr_name) {
                            Some((arr_name.clone(), mname))
                        } else { None }
                    } else { None }
                } else { None };
                if let Some((arr_name, mname)) = locator_info {
                    if let ExprKind::Ident(lhier) = &lvalue.kind {
                        let lname = self.resolve_hier_name(lhier);
                        if self.module.arrays.contains_key(&arr_name) {
                            let cur_size = self.get_queue_size(&arr_name) as usize;
                            let mut results: Vec<Value> = Vec::new();
                            if mname == "unique" || mname == "unique_index" {
                                let mut seen = std::collections::HashSet::new();
                                for i in 0..cur_size {
                                    if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", arr_name, i)) {
                                        let key = v.to_u64().unwrap_or(0);
                                        if seen.insert(key) {
                                            if mname == "unique_index" { results.push(Value::from_u64(i as u64, 32)); }
                                            else { results.push(v); }
                                        }
                                    }
                                }
                            } else if mname == "min" || mname == "max" {
                                let mut best: Option<Value> = None;
                                for i in 0..cur_size {
                                    if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", arr_name, i)) {
                                        let keep = match &best {
                                            None => true,
                                            Some(b) => if mname == "min" { v.to_u64().unwrap_or(u64::MAX) < b.to_u64().unwrap_or(u64::MAX) }
                                                       else { v.to_u64().unwrap_or(0) > b.to_u64().unwrap_or(0) },
                                        };
                                        if keep { best = Some(v); }
                                    }
                                }
                                if let Some(b) = best { results.push(b); }
                            }
                            for (i, v) in results.iter().enumerate() {
                                self.set_signal_value_by_name(&format!("{}[{}]", lname, i), v.clone());
                            }
                            self.set_queue_size(&lname, results.len() as u64);
                            if !self.in_edge_block { self.settle_combinatorial(); }
                            return;
                        }
                    }
                }
                let w = self.infer_lhs_width(lvalue);
                // Handle bare `x = new;` (no parens) as class instantiation
                let bare_new = if let ExprKind::Ident(hier) = &rvalue.kind {
                    hier.path.len() == 1 && hier.path[0].name.name == "new"
                } else { false };
                if bare_new {
                    let type_name = self.get_expr_type_name(lvalue);
                    if let Some(tname) = type_name {
                        if let Some(class_def) = self.module.classes.get(&tname).cloned() {
                            let lname_opt = if let ExprKind::Ident(lh) = &lvalue.kind {
                                Some(self.resolve_hier_name(lh))
                            } else { None };
                            let ta_cloned = lname_opt.as_ref().and_then(|n| self.module.class_type_args.get(n).cloned());
                            let handle = self.instantiate_class_with_type_args(&class_def, &[], ta_cloned.as_deref());
                            self.assign_value(lvalue, &handle.resize(w));
                            if !self.in_edge_block { self.settle_combinatorial(); }
                            return;
                        }
                    }
                }
                let val = if let ExprKind::Call { func, args } = &rvalue.kind {
                    if let ExprKind::Ident(hier) = &func.kind {
                        let method_name = hier.path.last().unwrap().name.name.as_str();
                        if method_name == "new" {
                            let type_name = self.get_expr_type_name(lvalue);
                            if let Some(tname) = type_name {
                                if tname == "semaphore" {
                                    let handle = self.heap.len();
                                    self.heap.push(Some(ClassInstance { class_name: tname.clone(), properties: HashMap::new() }));
                                    let initial_count = args.first().map(|a| self.eval_expr(a).to_u64().unwrap_or(0) as i64).unwrap_or(0);
                                    self.semaphores.insert(handle, initial_count);
                                    Value::from_u64(handle as u64, 32)
                                } else if tname == "mailbox" {
                                    let handle = self.heap.len();
                                    self.heap.push(Some(ClassInstance { class_name: tname.clone(), properties: HashMap::new() }));
                                    self.mailboxes.insert(handle, std::collections::VecDeque::new());
                                    Value::from_u64(handle as u64, 32)
                                } else if let Some(class_def) = self.module.classes.get(&tname).cloned() {
                                    let lname_opt = if let ExprKind::Ident(lh) = &lvalue.kind {
                                        Some(self.resolve_hier_name(lh))
                                    } else { None };
                                    let ta_cloned = lname_opt.as_ref().and_then(|n| self.module.class_type_args.get(n).cloned());
                                    self.instantiate_class_with_type_args(&class_def, args, ta_cloned.as_deref())
                                } else if let Some(cg_def) = self.module.covergroups.get(&tname).cloned() {
                                    self.instantiate_covergroup(&cg_def, args)
                                } else {
                                    // Could be dynamic array new[size]
                                    if let Some(arg) = args.first() {
                                        let size = self.eval_expr(arg);
                                        if let ExprKind::Ident(lhier) = &lvalue.kind {
                                            let name = self.resolve_hier_name(lhier);
                                            self.signals.insert(format!("{}.size", name), size.clone());
                                        }
                                        // Do not assign to lvalue (array) itself
                                        if !self.in_edge_block { self.settle_combinatorial(); }
                                        return;
                                    } else { self.eval_expr_ctx(rvalue, w) }
                                }
                            } else {
                                // Dynamic array new[size] without explicit type name
                                if let Some(arg) = args.first() {
                                    let size = self.eval_expr(arg);
                                    if let ExprKind::Ident(lhier) = &lvalue.kind {
                                        let name = self.resolve_hier_name(lhier);
                                        self.signals.insert(format!("{}.size", name), size.clone());
                                    }
                                    if !self.in_edge_block { self.settle_combinatorial(); }
                                    return;
                                } else { self.eval_expr_ctx(rvalue, w) }
                            }
                        } else { self.eval_expr_ctx(rvalue, w) }
                    } else { self.eval_expr_ctx(rvalue, w) }
                } else { self.eval_expr_ctx(rvalue, w) };
                if let (ExprKind::Ident(lhier), ExprKind::Ident(rhier)) = (&lvalue.kind, &rvalue.kind) {
                    let lname = self.resolve_hier_name(lhier);
                    let rname = self.resolve_hier_name(rhier);
                    if self.is_associative_array(&lname) && self.is_associative_array(&rname) {
                        let prefix = format!("{}[", rname);
                        let entries: Vec<(String, Value)> = self.signals.iter()
                            .filter(|(k, _)| k.starts_with(&prefix) && k.ends_with(']'))
                            .map(|(k, v)| {
                                let key = &k[prefix.len()..k.len()-1];
                                (format!("{}[{}]", lname, key), v.clone())
                            })
                            .collect();
                        for (k, v) in entries {
                            self.signals.insert(k, v);
                        }
                        if !self.in_edge_block { self.settle_combinatorial(); }
                        return;
                    }
                    if self.module.arrays.contains_key(&lname) && self.module.arrays.contains_key(&rname) {
                        let (llo, lhi, _) = self.module.arrays[&lname];
                        let (rlo, rhi, _) = self.module.arrays[&rname];
                        let lsize = (lhi - llo + 1) as usize;
                        let rsize = (rhi - rlo + 1) as usize;
                        let count = lsize.min(rsize);
                        let l_desc = self.module.descending_arrays.contains(&lname);
                        let r_desc = self.module.descending_arrays.contains(&rname);
                        for i in 0..count {
                            let ridx = if r_desc { rhi - i as i64 } else { rlo + i as i64 };
                            let lidx = if l_desc { lhi - i as i64 } else { llo + i as i64 };
                            let rval = self.get_signal_value_by_name(&format!("{}[{}]", rname, ridx)).unwrap_or(Value::zero(32));
                            self.set_signal_value_by_name(&format!("{}[{}]", lname, lidx), rval);
                        }
                        if !self.in_edge_block { self.settle_combinatorial(); }
                        return;
                    }
                }
                if let ExprKind::Ident(lhier) = &lvalue.kind {
                    let lname = self.resolve_hier_name(lhier);
                    if self.module.arrays.contains_key(&lname) {
                        // Queue/array slice assignment: lq = rq[a:b]
                        if let ExprKind::RangeSelect { expr: rbase, left, right, .. } = &rvalue.kind {
                            if let ExprKind::Ident(rhier) = &rbase.kind {
                                let rname = self.resolve_hier_name(rhier);
                                if self.module.arrays.contains_key(&rname) {
                                    let (r_lo_a, r_hi_a, _) = self.module.arrays[&rname];
                                    let r_is_dyn = self.module.dynamic_arrays.contains(&rname);
                                    let r_upper: i64 = if r_is_dyn {
                                        (self.get_queue_size(&rname) as i64) - 1
                                    } else { r_hi_a };
                                    self.scriptllar_bound.push(r_upper);
                                    let l = self.eval_expr(left).to_i64().unwrap_or(0);
                                    let r = self.eval_expr(right).to_i64().unwrap_or(0);
                                    self.scriptllar_bound.pop();
                                    // Per IEEE 7.10.1: if l > r the slice is empty.
                                    let results: Vec<Value> = if l > r {
                                        Vec::new()
                                    } else {
                                        let lo = l.max(r_lo_a);
                                        let hi = r.min(r_upper);
                                        if hi < lo { Vec::new() } else {
                                            (lo..=hi).map(|idx| {
                                                self.get_signal_value_by_name(&format!("{}[{}]", rname, idx))
                                                    .unwrap_or(Value::zero(32))
                                            }).collect()
                                        }
                                    };
                                    let (l_lo, l_hi, _) = self.module.arrays[&lname];
                                    for (i, v) in results.iter().enumerate() {
                                        let idx = l_lo + i as i64;
                                        if idx > l_hi { break; }
                                        self.set_signal_value_by_name(&format!("{}[{}]", lname, idx), v.clone());
                                    }
                                    if self.module.dynamic_arrays.contains(&lname) {
                                        self.set_queue_size(&lname, results.len() as u64);
                                    }
                                    if !self.in_edge_block { self.settle_combinatorial(); }
                                    return;
                                }
                            }
                        }
                        if let ExprKind::AssignmentPattern(items) = &rvalue.kind {
                            let (lo, hi, _w) = self.module.arrays[&lname];
                            let descending = self.module.descending_arrays.contains(&lname);
                            for (i, item) in items.iter().enumerate() {
                                let idx = if descending { hi - i as i64 } else { lo + i as i64 };
                                if idx < lo || idx > hi { break; }
                                let v = self.eval_expr(item.expr());
                                self.set_signal_value_by_name(&format!("{}[{}]", lname, idx), v);
                            }
                            if !self.in_edge_block { self.settle_combinatorial(); }
                            return;
                        }
                        if let ExprKind::Concatenation(exprs) = &rvalue.kind {
                            // Expand queue/array elements in concat (e.g. q = {q, 4})
                            let mut all_vals: Vec<Value> = Vec::new();
                            for expr in exprs.iter() {
                                if let ExprKind::Ident(ehier) = &expr.kind {
                                    let ename = self.resolve_hier_name(ehier);
                                    if self.module.arrays.contains_key(&ename) {
                                        let esize = self.get_queue_size(&ename) as usize;
                                        for j in 0..esize {
                                            if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", ename, j)) {
                                                all_vals.push(v);
                                            }
                                        }
                                        continue;
                                    }
                                }
                                // Slice of a queue: q[a:b]
                                if let ExprKind::RangeSelect { expr: rbase, left, right, .. } = &expr.kind {
                                    if let ExprKind::Ident(rhier) = &rbase.kind {
                                        let rname = self.resolve_hier_name(rhier);
                                        if self.module.arrays.contains_key(&rname) {
                                            let (r_lo_a, r_hi_a, _) = self.module.arrays[&rname];
                                            let r_is_dyn = self.module.dynamic_arrays.contains(&rname);
                                            let r_upper: i64 = if r_is_dyn {
                                                (self.get_queue_size(&rname) as i64) - 1
                                            } else { r_hi_a };
                                            self.scriptllar_bound.push(r_upper);
                                            let l = self.eval_expr(left).to_i64().unwrap_or(0);
                                            let r = self.eval_expr(right).to_i64().unwrap_or(0);
                                            self.scriptllar_bound.pop();
                                            if l <= r {
                                                let lo = l.max(r_lo_a);
                                                let hi = r.min(r_upper);
                                                if hi >= lo {
                                                    for idx in lo..=hi {
                                                        if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", rname, idx)) {
                                                            all_vals.push(v);
                                                        }
                                                    }
                                                }
                                            }
                                            continue;
                                        }
                                    }
                                }
                                all_vals.push(self.eval_expr(expr));
                            }
                            let (lo, hi, _w) = self.module.arrays[&lname];
                            for (i, v) in all_vals.iter().enumerate() {
                                let idx = lo + i as i64;
                                if idx > hi { break; }
                                self.set_signal_value_by_name(&format!("{}[{}]", lname, idx), v.clone());
                            }
                            if self.module.dynamic_arrays.contains(&lname) {
                                self.set_queue_size(&lname, all_vals.len() as u64);
                            }
                            if !self.in_edge_block { self.settle_combinatorial(); }
                            return;
                        }
                    }
                }
                // When assigning a locator/reduction method result to a queue, store as single-element queue
                if let ExprKind::Ident(lhier) = &lvalue.kind {
                    let lname = self.resolve_hier_name(lhier);
                    if self.module.dynamic_arrays.contains(&lname) && self.module.arrays.contains_key(&lname) {
                        if let ExprKind::MemberAccess { member, .. } = &rvalue.kind {
                            let mname = member.name.as_str();
                            if matches!(mname, "min" | "max" | "unique" | "find" | "find_first" | "find_last" | "find_index" | "find_first_index" | "find_last_index" | "sum" | "product") {
                                if !val.has_xz() {
                                    self.set_signal_value_by_name(&format!("{}[0]", lname), val);
                                    self.set_queue_size(&lname, 1);
                                } else {
                                    self.set_queue_size(&lname, 0);
                                }
                                if !self.in_edge_block { self.settle_combinatorial(); }
                                return;
                            }
                        }
                    }
                }
                self.assign_value(lvalue, &val);
                if !self.in_edge_block { self.settle_combinatorial(); }
            }
            StatementKind::NonblockingAssign { lvalue, delay, rvalue } => {
                let val = self.eval_expr(rvalue);
                let w = self.infer_lhs_width(lvalue);
                let d = delay.as_ref().map(|de| self.eval_expr(de).to_u64().unwrap_or(0)).unwrap_or(0);
                if d == 0 {
                    let id_opt = self.resolve_nba_target(lvalue);
                    if let Some(id) = id_opt {
                        // Track via index map so subsequent partial-NBAs
                        // (NbaAssignRange / NbaAssignBitDyn from the
                        // bytecode VM) targeting the same signal merge into
                        // this entry rather than re-seeding from
                        // signal_table — same invariant as the bytecode
                        // NbaAssign Insn.
                        self.nba_fast_index.insert(id, self.nba_fast.len());
                        self.nba_fast.push(NbaFast { signal_id: id, value: val.resize_for_assign(w) });
                    } else {
                        self.nba_queue.push(NbaEntry { lhs: Some(lvalue.clone()), value: val.resize_for_assign(w), resolved_id: None });
                    }
                } else {
                    self.nba_queue.push(NbaEntry { lhs: Some(lvalue.clone()), value: val.resize_for_assign(w), resolved_id: None });
                }
            }
            StatementKind::If { condition, then_stmt, else_stmt, .. } => {
                if self.eval_expr(condition).is_true() { self.exec_statement(then_stmt); }
                else if let Some(el) = else_stmt { self.exec_statement(el); }
            }
            StatementKind::Case { expr, items, .. } => {
                let val = self.eval_expr(expr); let mut matched = false;
                for (_iidx, item) in items.iter().enumerate() { if item.is_default { continue; } for pat in &item.patterns { if val.case_eq(&self.eval_expr(pat)).is_true() {
                    self.exec_statement(&item.stmt); matched = true; break; } } if matched { break; } }
                if !matched { for item in items { if item.is_default {
                    self.exec_statement(&item.stmt); break; } } }
            }
            StatementKind::For { init, condition, step, body } => {
                for fi in init { match fi {
                    ForInit::VarDecl { data_type, name, init: e } => { let v = self.eval_expr(e); let w = super::elaborate::resolve_type_width(data_type, Some(&self.module.parameters), Some(&self.module.typedefs)); self.widths.insert(name.name.clone(), w); self.signals.insert(name.name.clone(), v.resize(w)); }
                    ForInit::Assign { lvalue, rvalue } => { let v = self.eval_expr(rvalue); self.assign_value(lvalue, &v); }
                }}
                let loop_limit: u64 = std::env::var("XEZIM_LOOP_LIMIT").ok()
                    .and_then(|s| s.parse().ok()).unwrap_or(10_000_000);
                let mut iters: u64 = 0;
                loop {
                    if iters > loop_limit || self.finished { break; } iters += 1;
                    if let Some(c) = condition { if !self.eval_expr(c).is_true() { break; } }
                    self.break_flag = false; self.continue_flag = false; self.exec_statement(body);
                    if self.break_flag { self.break_flag = false; break; } self.continue_flag = false;
                    for s in step { self.exec_expr_stmt(s); }
                }
            }
            StatementKind::Foreach { array, vars, body } => {
                if let ExprKind::Ident(hier) = &array.kind {
                    let name = self.resolve_hier_name(hier);
                    let size = self.lookup_signal_width(&name).unwrap_or(1) as u64;
                    if let Some(var) = vars.first().and_then(|v| v.as_ref()) {
                        self.widths.insert(var.name.clone(), 32);
                        for i in 0..size { if self.finished { break; } self.signals.insert(var.name.clone(), Value::from_u64(i, 32)); self.exec_statement(body); }
                    }
                }
            }
            StatementKind::While { condition, body } => { let loop_limit: u64 = std::env::var("XEZIM_LOOP_LIMIT").ok().and_then(|s| s.parse().ok()).unwrap_or(10_000_000); let mut i: u64 = 0; loop { if i > loop_limit || self.finished { break; } i += 1; if !self.eval_expr(condition).is_true() { break; } self.break_flag = false; self.exec_statement(body); if self.break_flag { self.break_flag = false; break; } } }
            StatementKind::DoWhile { body, condition } => { let loop_limit: u64 = std::env::var("XEZIM_LOOP_LIMIT").ok().and_then(|s| s.parse().ok()).unwrap_or(10_000_000); let mut i: u64 = 0; loop { if i > loop_limit || self.finished { break; } i += 1; self.break_flag = false; self.exec_statement(body); if self.break_flag { self.break_flag = false; break; } if !self.eval_expr(condition).is_true() { break; } } }
            StatementKind::Repeat { count, body } => { let n = self.eval_expr(count).to_u64().unwrap_or(0); for _ in 0..n.min(10000) { if self.finished { break; } self.break_flag = false; self.continue_flag = false; self.exec_statement(body); if self.break_flag { self.break_flag = false; break; } self.continue_flag = false; } }
            StatementKind::Forever { body } => { let mut i = 0; loop { if i > 100000 || self.finished || self.time > self.max_time { break; } i += 1; self.exec_statement(body); } }
            StatementKind::SeqBlock { stmts, .. } => { for s in stmts { if self.finished || self.break_flag || self.continue_flag { break; } self.exec_statement(s); } }
            StatementKind::ParBlock { stmts, join_type, .. } => {
                let mut pids = Vec::new();
                for s in stmts {
                    let pid = self.next_pid; self.next_pid += 1;
                    pids.push(pid);
                    self.process_parents.insert(pid, 0); // (top-level as parent for now)
                    self.event_queue.schedule(self.time, pid, vec![s.clone()]);
                }
                match join_type {
                    JoinType::Join => {
                        let mut child_set = HashSet::new();
                        for &cp in &pids { child_set.insert(cp); }
                        self.join_waiters.push(JoinWaiter {
                            parent_pid: self.current_pid,
                            child_pids: child_set,
                            join_type: *join_type,
                            continuation: Vec::new(),
                            finished_children: HashSet::new(),
                        });
                        self.break_flag = true; // Suspend current execution
                    }
                    _ => {} // JoinAny/JoinNone: simplified support for now
                }
            }
            StatementKind::TimingControl { control, stmt } => {
                match control {
                    TimingControl::Delay(d) => {
                        let delay = self.eval_expr(d).to_u64().unwrap_or(0);
                        self.apply_nba(); self.settle_combinatorial(); self.snapshot_edge_signals();
                        let target = self.time + delay;
                        // Run scheduled continuations (other processes) whose fire
                        // time falls inside this delay window so concurrent blocks
                        // can advance while we're sleeping inside a task.
                        self.run_events_until(target);
                        if self.time < target { self.time = target; }
                        self.settle_combinatorial(); self.check_monitor();
                    }
                    TimingControl::Event(e) => {
                        let sens = self.event_to_sens(e);
                        sim_dbg_eprintln!("[DEBUG] process {} waiting for event {:?} at time {}", self.current_pid, sens, self.time);
                        // Suspension is handled by run_process_stmts for top-level timing controls.
                        // If we are here, it's a nested timing control which we don't fully support yet.
                        let mut cont = vec![*stmt.clone()];
                        let pid = self.cg_this.unwrap_or(0); // placeholder
                        self.event_waiters.push(self.make_event_waiter(pid, sens, cont));
                        self.break_flag = true;
                        return;
                    }
                }
                self.exec_statement(stmt);
                // After body executes, check for edges (e.g., clk toggled)
                // and drain any cascade — see drain_edge_cascade.
                self.settle_combinatorial();
                self.check_edges();
                let _ = self.drain_edge_cascade(self.cascade_limit);
            }
            StatementKind::Break => { self.break_flag = true; }
            StatementKind::Continue => { self.continue_flag = true; }
            StatementKind::Return(expr) => {
                if let Some(e) = expr {
                    self.return_value = Some(self.eval_expr(e));
                }
                self.break_flag = true;
            }
            StatementKind::Disable(_) | StatementKind::WaitFork => {}
            StatementKind::RsReturn => {
                self.rs_return_flag = true;
                self.break_flag = true;
            }
            StatementKind::RsAction { body } => {
                let prev = self.rs_return_flag;
                self.rs_return_flag = false;
                self.exec_statement(body);
                let triggered = self.rs_return_flag;
                self.rs_return_flag = prev;
                if triggered {
                    // Consume the break we raised for our RsReturn so the
                    // enclosing sequence continues with the next production.
                    self.break_flag = false;
                }
            }
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
                let w = super::elaborate::resolve_type_width(data_type, Some(&self.module.parameters), Some(&self.module.typedefs));
                let two_state = super::elaborate::is_type_two_state(data_type);
                let default_v = if two_state { Value::zero(w) } else { Value::new(w) };
                for d in declarators {
                    let dims = &d.dimensions;
                    let mut range: Option<(i64, i64)> = None;
                    let mut descending = false;
                    if let Some(first) = dims.first() {
                        use crate::ast::types::UnpackedDimension;
                        match first {
                            UnpackedDimension::Range { left, right, .. } => {
                                let l = super::elaborate::const_eval_i64_with_params(left, None).unwrap_or(0);
                                let r = super::elaborate::const_eval_i64_with_params(right, None).unwrap_or(0);
                                range = Some((l.min(r), l.max(r)));
                                if l > r { descending = true; }
                            }
                            UnpackedDimension::Expression { expr, .. } => {
                                let n = super::elaborate::const_eval_i64_with_params(expr, None).unwrap_or(0);
                                if n > 0 { range = Some((0, n - 1)); }
                            }
                            UnpackedDimension::Unsized(_) | UnpackedDimension::Queue { .. } => {
                                // Register as dynamic array / queue (initially empty).
                                let name = d.name.name.clone();
                                self.module.arrays.insert(name.clone(), (0, -1, w));
                                self.module.dynamic_arrays.insert(name.clone());
                                self.widths.insert(name.clone(), w);
                                self.set_queue_size(&name, 0);
                                continue;
                            }
                            _ => {}
                        }
                    }
                    if let Some((lo, hi)) = range {
                        let name = d.name.name.clone();
                        self.module.arrays.insert(name.clone(), (lo, hi, w));
                        if descending { self.module.descending_arrays.insert(name.clone()); }
                        for idx in lo..=hi {
                            let elem = format!("{}[{}]", name, idx);
                            self.signals.insert(elem.clone(), default_v.clone());
                            self.widths.insert(elem, w);
                        }
                        self.widths.insert(name.clone(), w);
                    } else {
                        if let Some(task_name) = self.current_static_task.clone() {
                            let key = format!("{}.{}", task_name, d.name.name);
                            if !self.static_task_init.insert(key) {
                                continue;
                            }
                        }
                        let class_name = if let crate::ast::types::DataType::TypeReference { name, .. } = data_type {
                            Some(name.name.name.clone())
                        } else { None };
                        let v = if let Some(init_expr) = d.init.as_ref() {
                            let mut produced: Option<Value> = None;
                            if let Some(cn) = &class_name {
                                if let Some(class_def) = self.module.classes.get(cn).cloned() {
                                    let is_new = match &init_expr.kind {
                                        ExprKind::Call { func, args } => {
                                            if let ExprKind::Ident(h) = &func.kind {
                                                if h.path.last().map_or(false, |s| s.name.name == "new") {
                                                    Some(args.clone())
                                                } else { None }
                                            } else { None }
                                        }
                                        ExprKind::Ident(h) => {
                                            if h.path.last().map_or(false, |s| s.name.name == "new") {
                                                Some(vec![])
                                            } else { None }
                                        }
                                        _ => None,
                                    };
                                    if let Some(call_args) = is_new {
                                        produced = Some(self.instantiate_class(&class_def, &call_args));
                                    }
                                }
                            }
                            produced.unwrap_or_else(|| self.eval_expr_ctx(init_expr, w).resize(w))
                        } else {
                            default_v.clone()
                        };
                        self.widths.insert(d.name.name.clone(), w);
                        self.signals.insert(d.name.name.clone(), v);
                    }
                }
            }
            StatementKind::EventTrigger { name, .. } => {
                let raw = name.name.clone();
                let trimmed = raw.trim_start_matches('.').to_string();
                let mut candidates = Vec::new();
                candidates.push(raw.clone());
                if trimmed != raw {
                    candidates.push(trimmed.clone());
                }
                let top_prefixed = format!("{}.{}", self.module.name, trimmed);
                if top_prefixed != raw && top_prefixed != trimmed {
                    candidates.push(top_prefixed);
                }
                candidates.sort();
                candidates.dedup();

                for sig_name in candidates {
                    if self.signal_name_to_id.contains_key(sig_name.as_str()) {
                        let cur = self.get_signal_value_by_name(&sig_name).unwrap_or(Value::zero(1));
                        let new_val = if cur.bits_first() == LogicBit::One { Value::zero(1) } else { Value::ones(1) };
                        sim_dbg_eprintln!("[DEBUG] firing event {} (new_val={:?}) at time {}", sig_name, new_val, self.time);
                        self.fast_signal_write(&sig_name, new_val);
                    }
                }
                // Settle combinatorial logic but defer edge-triggered blocks
                // (always @(e)) to the main event loop so the triggering
                // process sees pre-event state until its next delay/wait.
                self.settle_combinatorial();
            }
            StatementKind::Coverpoint { .. } | StatementKind::Cross { .. } => {}
        }
    }

    fn exec_expr_stmt(&mut self, expr: &Expression) {
        match &expr.kind {
            ExprKind::SystemCall { name, args } => self.exec_system_task(name, args),
            ExprKind::Ident(hier) => {
                let name = self.resolve_hier_name(hier);
                if let Some(td) = self.module.tasks.get(&name).cloned() {
                    // Execute bare task-enable only for zero-time tasks.
                    // Blocking tasks (with delay/event/wait/forever blocking) require
                    // process suspension semantics that expr-stmt fast path does not model.
                    if !td.items.iter().any(|s| self.stmt_is_blocking(s)) {
                        self.exec_task_call(&td, &[]);
                    }
                } else {
                    self.eval_expr(expr);
                }
            }
            ExprKind::AssignExpr { lvalue, rvalue } => {
                // Simple direct assign — the for-loop step produces this
                // after xezim-core 8b9c88c, and it's a tight inner-loop
                // stmt in memory-init patterns. Skip the eval_expr
                // dispatch overhead of the `_` fallback.
                let val = self.eval_expr(rvalue);
                self.assign_value(lvalue, &val);
            }
            ExprKind::Binary { op: BinaryOp::Assign, left, right } => {
                let val = if let ExprKind::Call { func, args } = &right.kind {
                    if let ExprKind::Ident(hier) = &func.kind {
                        if hier.path.last().unwrap().name.name == "new" {
                            let type_name = self.get_expr_type_name(left);
                            if let Some(tname) = type_name {
                                if let Some(class_def) = self.module.classes.get(&tname).cloned() {
                                    let lname_opt = if let ExprKind::Ident(lh) = &left.kind {
                                        Some(self.resolve_hier_name(lh))
                                    } else { None };
                                    let ta_cloned = lname_opt.as_ref().and_then(|n| self.module.class_type_args.get(n).cloned());
                                    self.instantiate_class_with_type_args(&class_def, args, ta_cloned.as_deref())
                                } else if let Some(cg_def) = self.module.covergroups.get(&tname).cloned() {
                                    self.instantiate_covergroup(&cg_def, args)
                                } else { self.eval_expr(right) }
                            } else { self.eval_expr(right) }
                        } else { self.eval_expr(right) }
                    } else { self.eval_expr(right) }
                } else {
                    self.eval_expr(right)
                };
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
            "$display" | "$displayb" | "$displayh" | "$displayo" => { let m = self.format_args(args, name); self.record_output(m.clone()); self.stdout_writeln(&m); }
            "$write" | "$writeb" | "$writeh" | "$writeo" => { let m = self.format_args(args, name); self.record_output(m.clone()); self.stdout_write(&m); }
            "$monitor" | "$monitorb" | "$monitorh" | "$monitoro" => { self.monitor = Some((name.to_string(), args.to_vec())); self.check_monitor(); }
            "$monitoroff" => { self.monitor = None; }
            "$finish" | "$stop" => { self.finished = true; }
            "$fclose" => { let _ = self.close_file_handle(args); }
            "$fwrite" => { let _ = self.write_file_handle(args, false); }
            "$fdisplay" => { let _ = self.write_file_handle(args, true); }
            "$fseek" => {
                use std::io::{Seek, SeekFrom};
                let fd = args.first().map(|a| self.eval_file_handle_arg(a)).unwrap_or(0);
                let off = args.get(1).map(|a| self.eval_expr(a).to_u64().unwrap_or(0) as i64).unwrap_or(0);
                let whence = args.get(2).map(|a| self.eval_expr(a).to_u64().unwrap_or(0)).unwrap_or(0);
                let from = match whence { 1 => SeekFrom::Current(off), 2 => SeekFrom::End(off), _ => SeekFrom::Start(off as u64) };
                if let Some(f) = self.file_handles.get_mut(&fd) { let _ = f.seek(from); }
            }
            "$rewind" => {
                use std::io::{Seek, SeekFrom};
                let fd = args.first().map(|a| self.eval_file_handle_arg(a)).unwrap_or(0);
                if let Some(f) = self.file_handles.get_mut(&fd) { let _ = f.seek(SeekFrom::Start(0)); }
            }
            "$ungetc" => {
                let ch = args.first().map(|a| self.eval_expr(a).to_u64().unwrap_or(0) as u8).unwrap_or(0);
                let fd = args.get(1).map(|a| self.eval_file_handle_arg(a)).unwrap_or(0);
                self.ungetc_buf.entry(fd).or_default().push(ch);
            }
            "$readmemh" => { let _ = self.read_memory_file(args, 16); }
            "$readmemb" => { let _ = self.read_memory_file(args, 2); }
            "$dumpfile" => {
                if let Some(arg) = args.first() {
                    if let ExprKind::StringLiteral(s) = &arg.kind {
                        if self.aitrace_mode {
                            // Replace .vcd extension with .aitrace
                            let name = if s.ends_with(".vcd") {
                                format!("{}.aitrace", &s[..s.len()-4])
                            } else {
                                format!("{}.aitrace", s)
                            };
                            self.vcd_file = Some(name);
                        } else {
                            self.vcd_file = Some(s.clone());
                        }
                    } else {
                        self.vcd_file = Some(if self.aitrace_mode { "dump.aitrace".to_string() } else { "dump.vcd".to_string() });
                    }
                }
            }
            "$dumpvars" => {
                if self.aitrace_mode {
                    self.aitrace_start_dump();
                } else {
                    self.vcd_start_dump();
                }
            }
            "$dumpoff" => { self.vcd_enabled = false; }
            "$dumpon" => { self.vcd_enabled = true; }
            "$sscanf" => {
                if args.len() >= 3 {
                    let src_str = if let ExprKind::StringLiteral(s) = &args[0].kind {
                        s.clone()
                    } else {
                        self.eval_expr(&args[0]).to_sv_string()
                    };
                    if let ExprKind::StringLiteral(fmt) = &args[1].kind {
                        if fmt.contains("%d") || fmt.contains("%i") {
                            if let Ok(n) = src_str.trim().parse::<i64>() {
                                self.assign_value(&args[2], &Value::from_u64(n as u64, 32));
                            }
                        } else if fmt.contains("%s") {
                            self.assign_value(&args[2], &Value::from_string(&src_str));
                        } else if fmt.contains("%h") || fmt.contains("%x") {
                            if let Ok(n) = u64::from_str_radix(src_str.trim().trim_start_matches("0x").trim_start_matches("0X"), 16) {
                                self.assign_value(&args[2], &Value::from_u64(n, 32));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn format_args(&mut self, args: &[Expression], tn: &str) -> String {
        if args.is_empty() { return String::new(); }
        if let ExprKind::StringLiteral(fmt) = &args[0].kind { return self.format_string(fmt, &args[1..], tn); }
        let r = if tn.ends_with('b') { 'b' } else if tn.ends_with('h') { 'h' } else { 'd' };
        let mut result = Vec::new();
        for a in args {
            let v = self.eval_expr(a);
            result.push(match r { 'b' => v.to_bin_string(), 'h' => v.to_hex_string(), _ => v.to_dec_string() });
        }
        result.join(" ")
    }

    fn format_string(&mut self, fmt: &str, args: &[Expression], _tn: &str) -> String {
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
                    'p' | 'P' => { if ai < args.len() {
                        let arg = &args[ai]; ai += 1;
                        if let ExprKind::Ident(h) = &arg.kind {
                            let name = self.resolve_hier_name(h);
                            if let Some(tag) = self.active_union_tag.get(&name).cloned() {
                                let v = self.eval_expr(arg);
                                result.push_str(&format!("'{{{}:{}}}", tag, v.to_dec_string()));
                                continue;
                            }
                        }
                        let v = self.eval_expr(arg);
                        result.push_str(&v.to_dec_string());
                    } }
                    _ => { if ai < args.len() { let v = self.eval_expr(&args[ai]); ai += 1; match spec {
                        'd' | 'D' => { let s = v.to_dec_string(); result.push_str(&pad_string(&s, pad_width, zero_pad)); }
                        'b' | 'B' => { let s = v.to_bin_string(); result.push_str(&pad_string(&s, pad_width, zero_pad)); }
                        'h' | 'H' | 'x' | 'X' => { let s = v.to_hex_string(); result.push_str(&pad_string(&s, pad_width, zero_pad)); }
                        'o' | 'O' => { let s = if let Some(u) = v.to_u64() { format!("{:o}", u) } else { "x".to_string() }; result.push_str(&pad_string(&s, pad_width, zero_pad)); }
                        'f' | 'F' => { let s = format!("{:.6}", v.to_f64()); result.push_str(&pad_string(&s, pad_width, zero_pad)); }
                        'g' | 'G' => { let s = format!("{:?}", v.to_f64()); result.push_str(&pad_string(&s, pad_width, zero_pad)); }
                        'e' | 'E' => { let s = format!("{:.6e}", v.to_f64()); result.push_str(&pad_string(&s, pad_width, zero_pad)); }
                        's' | 'S' => { if let ExprKind::StringLiteral(s) = &args[ai-1].kind { result.push_str(s); } else { let s = v.to_sv_string(); result.push_str(&pad_string(&s, pad_width, zero_pad)); } }
                        'c' | 'C' => { let b = (v.to_u64().unwrap_or(0) & 0xff) as u8; result.push(b as char); }
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
            if changed { self.record_output(m.clone()); self.stdout_writeln(&m); self.monitor_prev = self.signals.clone(); }
        }
    }

    fn resolve_hier_name(&self, hier: &HierarchicalIdentifier) -> String {
        // Per-hier cache: first call resolves and memoizes the result on the
        // AST node; every subsequent call on the same node returns in O(1)
        // without HashMap lookups, path-join allocation, or hint bookkeeping.
        // This is the dominant win for tight loops like `ram0.mem[i][7:0] = 0`
        // where the same hier objects are re-visited thousands of times.
        // The hint side-effect is deliberately skipped on the cached path:
        // hints only steer the suffix-scan fallback for ambiguous
        // unqualified names, and a resolved (cached) name no longer needs
        // that guidance.
        if let Some(cached) = hier.cached_resolved_name.get() {
            return cached.clone();
        }
        let resolved = self.resolve_hier_name_uncached(hier);
        // OnceCell::set returns Err if already set (race) — ignore; value
        // would be identical anyway.
        let _ = hier.cached_resolved_name.set(resolved.clone());
        resolved
    }

    fn resolve_hier_name_uncached(&self, hier: &HierarchicalIdentifier) -> String {
        let common_prefix_len = |a: &str, b: &str| -> usize {
            let mut n = 0usize;
            for (sa, sb) in a.split('.').zip(b.split('.')) {
                if sa == sb { n += 1; } else { break; }
            }
            n
        };
        fn parent_of(s: &str) -> &str {
            s.rsplit_once('.').map(|(p, _)| p).unwrap_or("")
        }
        let raw = hier.path.iter().map(|s| s.name.name.as_str()).collect::<Vec<_>>().join(".");
        // Exact dotted name match first.
        if self.signal_name_to_id.contains_key(raw.as_str()) {
            if raw.contains('.') {
                let parent = parent_of(&raw).to_string();
                *self.name_resolve_hint.borrow_mut() = Some(parent);
            }
            return raw;
        }
        // Fast paths to avoid the O(N) suffix scan below. Critical for tight
        // memory-init loops (e.g. c910 testbench wipe).
        //
        // 1) raw is a known array / queue / assoc / dynamic array name.
        // 2) raw starts with the top testbench module's name (e.g. `tb.`),
        //    but arrays/signals are keyed without that prefix. Try stripping
        //    the first segment and re-checking.
        let check_known_or_signal = |name: &str, this: &Self| -> bool {
            this.module.arrays.contains_key(name)
                || this.module.arrays_2d.contains_key(name)
                || this.module.arrays_nd.contains_key(name)
                || this.module.dynamic_arrays.contains(name)
        };
        if check_known_or_signal(&raw, self) {
            if raw.contains('.') {
                let parent = parent_of(&raw).to_string();
                *self.name_resolve_hint.borrow_mut() = Some(parent);
            }
            return raw;
        }
        if let Some((_head, rest)) = raw.split_once('.') {
            if self.signal_name_to_id.contains_key(rest) || check_known_or_signal(rest, self) {
                let out = rest.to_string();
                if out.contains('.') {
                    let parent = parent_of(&out).to_string();
                    *self.name_resolve_hint.borrow_mut() = Some(parent);
                }
                return out;
            }
        }
        let leaf = hier.path.last().map(|s| s.name.name.clone()).unwrap_or_default();
        if leaf.is_empty() {
            return raw;
        }

        // Heuristic fallback for unresolved single-segment names:
        // choose a suffix match guided by the most recent hierarchical hint.
        // Skip the O(N) suffix scan if the "leaf" already contains dots —
        // that means the parser flattened a deep hier path into one segment
        // (testbench probes like `assign x = x_soc.x_cpu_sub_system_axi.....fifo_full`),
        // and no key in signal_name_to_id can end with `.<full_dotted_path>`
        // since the table is keyed by the full path itself, not nested deeper.
        // On c910, this scan over 35.7M signals dominated time-0 settle
        // (575s / 11K probes).
        if hier.path.len() == 1 && !leaf.contains('.') {
            // Use leaf-name reverse index — O(1) instead of O(N) scan.
            let candidates: Vec<&str> = self.leaf_name_to_ids
                .get(leaf.as_str())
                .map(|ids| ids.iter()
                    .filter_map(|&id| self.id_to_name.get(id).map(|n| n.as_ref()))
                    .collect())
                .unwrap_or_default();
            if !candidates.is_empty() {
                let hint_owned = self.name_resolve_hint.borrow().clone().unwrap_or_default();
                let mut best: Option<&str> = None;
                let mut best_score: isize = -1;
                for key in candidates {
                    let key_parent = parent_of(key);
                    let score = common_prefix_len(&hint_owned, key_parent) as isize;
                    match best {
                        None => {
                            best = Some(key);
                            best_score = score;
                        }
                        Some(prev) => {
                            let prefer = if score != best_score {
                                score > best_score
                            } else {
                                let kd = key.split('.').count();
                                let pd = prev.split('.').count();
                                kd < pd || (kd == pd && key.len() < prev.len())
                            };
                            if prefer {
                                best = Some(key);
                                best_score = score;
                            }
                        }
                    }
                }
                if let Some(k) = best {
                    let parent = parent_of(k).to_string();
                    *self.name_resolve_hint.borrow_mut() = Some(parent);
                    return k.to_string();
                }
            }
        }

        // Multi-segment suffix fallback: for paths like "uut.picorv32_core.cpu_state",
        // look for keys ending with ".uut.picorv32_core.cpu_state", preferring the one
        // closest (by common-prefix) to the current scope hint.
        // Use leaf_name_to_ids index to narrow candidates to signals whose
        // last `.`-segment matches the leaf — turns the O(N) scan over
        // 35M signals into O(M) where M is the number of signals sharing
        // that leaf name (usually small).
        if hier.path.len() > 1 {
            let suffix = format!(".{}", raw);
            let candidates: Vec<&str> = if let Some(ids) = self.leaf_name_to_ids.get(leaf.as_str()) {
                ids.iter()
                    .filter_map(|&id| self.id_to_name.get(id).map(|n| n.as_ref()))
                    .filter(|k: &&str| k.ends_with(&suffix) || *k == raw.as_str())
                    .collect()
            } else {
                Vec::new()
            };
            if !candidates.is_empty() {
                let hint_owned = self.name_resolve_hint.borrow().clone().unwrap_or_default();
                let mut best: Option<&str> = None;
                let mut best_score: isize = -1;
                for key in candidates {
                    let key_parent = parent_of(key);
                    let score = common_prefix_len(&hint_owned, key_parent) as isize;
                    match best {
                        None => { best = Some(key); best_score = score; }
                        Some(prev) => {
                            let prefer = if score != best_score {
                                score > best_score
                            } else {
                                let kd = key.split('.').count();
                                let pd = prev.split('.').count();
                                kd < pd || (kd == pd && key.len() < prev.len())
                            };
                            if prefer { best = Some(key); best_score = score; }
                        }
                    }
                }
                if let Some(k) = best {
                    let parent = parent_of(k).to_string();
                    *self.name_resolve_hint.borrow_mut() = Some(parent);
                    return k.to_string();
                }
            }
        }

        if self.signal_name_to_id.contains_key(leaf.as_str()) {
            return leaf;
        }

        // Last-resort compatibility fallback.
        leaf
    }

    /// Fast signal read avoiding String allocation.
    /// Uses cached_signal_id to remember the signal name as &str key for HashMap lookup.
    #[inline]
    
    fn fast_signal_read(&self, hier: &HierarchicalIdentifier) -> Value {
        let is_ambiguous_leaf =
            hier.path.len() == 1 && !hier.path[0].name.name.contains('.');
        // Try cached signal ID first (O(1) Vec access)
        if let Some(id) = hier.cached_signal_id.get() {
            if !is_ambiguous_leaf {
                let mut v = self.signal_table[id].clone();
                if self.signal_signed[id] { v.is_signed = true; }
                if self.signal_real[id] { v.is_real = true; }
                return v;
            }
        }
        // First access: resolve name and cache ID
        let name = self.resolve_hier_name(hier);
        if let Some(&id) = self.signal_name_to_id.get(name.as_str()) {
            hier.cached_signal_id.set(Some(id));
            let mut v = self.signal_table[id].clone();
            if self.signal_signed[id] { v.is_signed = true; }
            if self.signal_real[id] { v.is_real = true; }
            return v;
        }
        // Fallback
        let mut v = self.signals.get(&name).cloned().unwrap_or_else(|| Value::new(1));
        if self.signed_signals.contains(&name) { v.is_signed = true; }
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

    /// Execute a fused 1-bit gate. Reads operand bits from signal_table,
    /// computes 4-state result, writes single bit back if changed.
    #[inline]
    fn exec_fused_gate(&mut self, op: FusedGate) {
        #[inline(always)]
        fn and4(a: LogicBit, b: LogicBit) -> LogicBit {
            use LogicBit::*;
            if matches!(a, Zero) || matches!(b, Zero) { return Zero; }
            if matches!(a, One) && matches!(b, One) { return One; }
            X
        }
        #[inline(always)]
        fn or4(a: LogicBit, b: LogicBit) -> LogicBit {
            use LogicBit::*;
            if matches!(a, One) || matches!(b, One) { return One; }
            if matches!(a, Zero) && matches!(b, Zero) { return Zero; }
            X
        }
        #[inline(always)]
        fn xor4(a: LogicBit, b: LogicBit) -> LogicBit {
            use LogicBit::*;
            match (a, b) {
                (Zero, Zero) | (One, One) => Zero,
                (Zero, One) | (One, Zero) => One,
                _ => X,
            }
        }
        #[inline(always)]
        fn not4(a: LogicBit) -> LogicBit {
            use LogicBit::*;
            match a { Zero => One, One => Zero, _ => X }
        }
        let (dst, new_bit) = match op {
            FusedGate::Buf1 { dst, src, invert } => {
                let s = self.signal_table[src.sig_id as usize].get_bit(src.bit as usize);
                let v = if invert { not4(s) } else {
                    // Z treated as X when used as a wire value
                    match s { LogicBit::Z => LogicBit::X, other => other }
                };
                (dst, v)
            }
            FusedGate::Bin2 { dst, a, b, op: gop, invert } => {
                let va = self.signal_table[a.sig_id as usize].get_bit(a.bit as usize);
                let vb = self.signal_table[b.sig_id as usize].get_bit(b.bit as usize);
                let r = match gop {
                    GateBin::And => and4(va, vb),
                    GateBin::Or => or4(va, vb),
                    GateBin::Xor => xor4(va, vb),
                };
                (dst, if invert { not4(r) } else { r })
            }
            FusedGate::Mux2 { dst, s, t, e } => {
                let vs = self.signal_table[s.sig_id as usize].get_bit(s.bit as usize);
                let vt = self.signal_table[t.sig_id as usize].get_bit(t.bit as usize);
                let ve = self.signal_table[e.sig_id as usize].get_bit(e.bit as usize);
                let v = match vs {
                    LogicBit::Zero => ve,
                    LogicBit::One => vt,
                    _ => if vt == ve { vt } else { LogicBit::X },
                };
                (dst, v)
            }
        };
        let id = dst.sig_id as usize;
        let cur = self.signal_table[id].get_bit(dst.bit as usize);
        if cur != new_bit {
            self.signal_table[id].set_bit(dst.bit as usize, new_bit);
            self.table_modified = true;
            self.mark_dirty_id(id);
        }
    }

    /// Batch-sync signal_table → signals HashMap.
    /// Called lazily before any code that reads from the HashMap.
    fn sync_table_to_hashmap(&mut self) {
        if !self.table_modified { return; }
        for (id, name) in self.id_to_name.iter().enumerate() {
            self.signals.insert(name.to_string(), self.signal_table[id].clone());
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

    /// JIT bridge: read `signal_table[id]` as a raw `u64` of val_bits.
    /// For 4-state (X/Z) values we still return a best-effort bit
    /// pattern (the val_bits directly) — the JIT'd block will compute
    /// on garbage but the caller re-runs the block via the interpreter
    /// afterwards if any X/Z is in play. (Phase-1 MVP: no XZ tracking
    /// in JIT code; we rely on post-execution fallback semantics which
    /// are fine for c910 post-reset where all signals are determinate.)
    #[inline]
    pub(crate) fn jit_load_signal(&self, id: usize) -> u64 {
        if id >= self.signal_table.len() { return 0; }
        self.signal_table[id].to_u64().unwrap_or(0)
    }

    /// JIT bridge: mirror `Insn::BlockingAssign` — width-mask the
    /// incoming val_bits, compare against current signal value, mark
    /// dirty + propagate if changed. Preserves `is_signed` from the
    /// signal's declared sign so readers get correct arithmetic.
    #[inline]
    pub(crate) fn jit_store_signal(&mut self, id: usize, val_bits: u64, width: u32) {
        if id >= self.signal_table.len() { return; }
        let sig_w = self.signal_widths[id];
        let w = if width == 0 { sig_w } else { width };
        let mut new_val = Value::from_u64(val_bits, w);
        if w != sig_w { new_val = new_val.resize(sig_w); }
        new_val.is_signed = self.signal_signed[id];
        if self.signal_table[id] != new_val {
            self.mark_dirty_id(id);
            self.signal_table[id] = new_val;
            self.table_modified = true;
        }
    }

    /// JIT bridge: mirror `Insn::NbaAssign` — push an `NbaFast` entry.
    /// `apply_nba` later commits to `signal_table` at the end-of-cycle.
    #[inline]
    pub(crate) fn jit_schedule_nba(&mut self, id: usize, val_bits: u64, width: u32) {
        if id >= self.signal_table.len() { return; }
        let sig_w = self.signal_widths[id];
        let w = if width == 0 { sig_w } else { width };
        let mut val = Value::from_u64(val_bits, w);
        if w != sig_w { val = val.resize(sig_w); }
        val.is_signed = self.signal_signed[id];
        self.nba_fast_index.insert(id, self.nba_fast.len());
        self.nba_fast.push(NbaFast { signal_id: id, value: val });
    }

    /// JIT bridge: mirror `Insn::NbaAssignRange` / `NbaAssignRangeDyn`
    /// — read-modify-write bits `[hi:lo]` of the target signal (or
    /// the latest in-flight NbaFast entry) with `val_bits` occupying
    /// the low `(hi-lo+1)` bits.
    #[inline]
    pub(crate) fn jit_schedule_nba_range(
        &mut self,
        id: usize,
        hi: u32,
        lo: u32,
        val_bits: u64,
    ) {
        if id >= self.signal_table.len() { return; }
        let (low, high) = if hi >= lo { (lo, hi) } else { (hi, lo) };
        let w = high - low + 1;
        let val = Value::from_u64(val_bits, w);
        // Compose onto latest nba_fast entry if any, else onto
        // signal_table's current value — exactly matches the
        // interpreter's NbaAssignRange read-modify-write pattern.
        let sig_w = self.signal_widths[id];
        let high_eff = high.min(sig_w.saturating_sub(1));
        if let Some(&i) = self.nba_fast_index.get(&id) {
            let target = &mut self.nba_fast[i].value;
            for bit_pos in low..=high_eff {
                target.set_bit(bit_pos as usize, val.get_bit((bit_pos - low) as usize));
            }
            return;
        }
        let mut new_val = self.signal_table[id].clone();
        for bit_pos in low..=high_eff {
            new_val.set_bit(bit_pos as usize, val.get_bit((bit_pos - low) as usize));
        }
        self.nba_fast_index.insert(id, self.nba_fast.len());
        self.nba_fast.push(NbaFast { signal_id: id, value: new_val });
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
                // Avoid a second signal_name_to_id lookup via mark_dirty(name)
                // — we already resolved id above.
                self.mark_dirty_id(id);
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

    /// Look up a signal value by name. Consults the indexed signal_table
    /// first (zero-clone branch returns from a Vec read), falling back to
    /// the legacy `signals` HashMap for dynamically created entries
    /// (queue elems, foreach loop vars, in-process var decls, etc.).
    /// This is the single read path that lets us drop the dual-store of
    /// static signals without breaking the fallback callers.
    #[inline]
    fn lookup_signal_value(&self, name: &str) -> Option<Value> {
        if let Some(&id) = self.signal_name_to_id.get(name) {
            return Some(self.signal_table[id].clone());
        }
        self.signals.get(name).cloned()
    }

    /// True if the named signal exists in either the indexed table or
    /// the dynamic-overflow HashMap. Use in place of
    /// `self.signals.contains_key(name)`.
    #[inline]
    fn has_signal(&self, name: &str) -> bool {
        self.signal_name_to_id.contains_key(name) || self.signals.contains_key(name)
    }

    /// Look up declared width by signal name.
    #[inline]
    fn lookup_signal_width(&self, name: &str) -> Option<u32> {
        if let Some(&id) = self.signal_name_to_id.get(name) {
            return Some(self.signal_widths[id]);
        }
        self.widths.get(name).copied()
    }

    /// Whether the named signal is signed.
    #[inline]
    fn lookup_signal_signed(&self, name: &str) -> bool {
        if let Some(&id) = self.signal_name_to_id.get(name) {
            return self.signal_signed[id];
        }
        self.signed_signals.contains(name)
    }

    /// Whether the named signal is real (`real`/`shortreal`).
    #[inline]
    fn lookup_signal_real(&self, name: &str) -> bool {
        if let Some(&id) = self.signal_name_to_id.get(name) {
            return self.signal_real[id];
        }
        self.real_signals.contains(name)
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
    fn infer_width(&mut self, expr: &Expression) -> u32 { match &expr.kind { ExprKind::Ident(h) => { let n = self.resolve_hier_name(h); self.lookup_signal_width(&n).unwrap_or(1) } ExprKind::Number(NumberLiteral::Integer { size, .. }) => size.unwrap_or(32), ExprKind::Concatenation(p) => { let mut total = 0; for x in p { total += self.infer_width(x); } total } ExprKind::Paren(inner) => self.infer_width(inner), ExprKind::AssignExpr { lvalue, .. } => self.infer_width(lvalue), ExprKind::Binary { left, right, .. } => self.infer_width(left).max(self.infer_width(right)), ExprKind::Unary { operand, .. } => self.infer_width(operand), ExprKind::Conditional { then_expr, else_expr, .. } => self.infer_width(then_expr).max(self.infer_width(else_expr)), _ => self.eval_expr(expr).width } }
    fn infer_lhs_width(&mut self, expr: &Expression) -> u32 {
        match &expr.kind {
            ExprKind::Concatenation(p) => { let mut total = 0; for x in p { total += self.infer_lhs_width(x); } total }
            ExprKind::Ident(h) => {
                let is_ambiguous_leaf =
                    h.path.len() == 1 && !h.path[0].name.name.contains('.');
                if let Some(id) = h.cached_signal_id.get() {
                    if !is_ambiguous_leaf {
                        return self.signal_widths[id];
                    }
                }
                let name = self.resolve_hier_name(h);
                if let Some(&id) = self.signal_name_to_id.get(name.as_str()) {
                    h.cached_signal_id.set(Some(id));
                    return self.signal_widths[id];
                }
                if let Some(w) = self.widths.get(&name).copied() {
                    return w;
                }
                let leaf = h.path.last().map(|s| s.name.name.as_str()).unwrap_or("");
                if let Some(&id) = self.signal_name_to_id.get(leaf) {
                    h.cached_signal_id.set(Some(id));
                    return self.signal_widths[id];
                }
                self.widths.get(leaf).copied().unwrap_or(32)
            }
            ExprKind::RangeSelect { left, right, kind, .. } => {
                let l = self.eval_expr(left).to_u64().unwrap_or(0);
                let r = self.eval_expr(right).to_u64().unwrap_or(0);
                match kind {
                    RangeKind::IndexedUp | RangeKind::IndexedDown => r as u32,
                    RangeKind::Constant => if l >= r { (l-r+1) as u32 } else { (r-l+1) as u32 },
                }
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
    pub fn get_signal(&self, name: &str) -> Option<&Value> {
        if let Some(&id) = self.signal_name_to_id.get(name) {
            return Some(&self.signal_table[id]);
        }
        self.signals.get(name)
    }
    pub fn set_signal(&mut self, name: &str, val: Value) {
        if let Some(&id) = self.signal_name_to_id.get(name) {
            let w = self.signal_widths[id];
            self.signal_table[id] = val.resize(w);
            self.table_modified = true;
            self.mark_dirty_id(id);
            return;
        }
        if let Some(w) = self.widths.get(name) { self.signals.insert(name.to_string(), val.resize(*w)); }
        else { self.widths.insert(name.to_string(), val.width); self.signals.insert(name.to_string(), val); }
    }

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
        let mut w: super::vcd_sink::VcdSink = if self.threads >= 2 {
            super::vcd_sink::VcdSink::threaded(file)
        } else {
            super::vcd_sink::VcdSink::inline(file)
        };

        // Collect and sort signal names for deterministic output
        let mut sig_names: Vec<String> = self.signals.keys().cloned().collect();
        sig_names.sort();

        // Assign VCD identifier codes
        let mut id_map = HashMap::new();
        for (idx, name) in sig_names.iter().enumerate() {
            id_map.insert(name.clone(), Self::vcd_id_code(idx));
        }

        // Write VCD header
        let _ = writeln!(w, "$date\n  Simulation generated by xezim\n$end");
        let _ = writeln!(w, "$version\n  xezim 0.1\n$end");
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
            let width = self.lookup_signal_width(name).unwrap_or(1);
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
            let val = self.lookup_signal_value(name).unwrap_or_else(|| Value::new(1));
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
            let ch = match val.bits_first() {
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
            let name_str: &str = name.as_ref();
            if let Some(vcd_id) = self.vcd_id_map.get(name_str) {
                let val = &self.signal_table[id];
                let changed = match self.vcd_prev_signals.get(name_str) {
                    Some(prev) => prev != val,
                    None => true,
                };
                if changed {
                    changes.push((vcd_id.clone(), val.clone()));
                    self.vcd_prev_signals.insert(name_str.to_string(), val.clone());
                }
            }
        }

        if changes.is_empty() { return; }

        let time_marker = if self.time != self.vcd_last_time {
            self.vcd_last_time = self.time;
            Some(self.time)
        } else {
            None
        };

        if let Some(sink) = self.vcd_writer.as_mut() {
            sink.post_vcd_changes(time_marker, changes);
        }
    }

    /// Flush and close VCD file
    fn vcd_finish(&mut self) {
        if let Some(ref mut w) = self.vcd_writer {
            let _ = w.flush();
        }
        self.vcd_writer = None;
    }

    // ═══════════════════════════════════════════════════════════════
    // AITRACE dump support (AITRACE-T text format, Level 0)
    // ═══════════════════════════════════════════════════════════════

    /// Format a Value as a hex string for AITRACE output.
    /// Uses 0x prefix. X/Z bits produce masked hex like 0xXX0A.
    fn aitrace_format_value(val: &Value) -> String {
        if val.width == 1 {
            return match val.bits_first() {
                LogicBit::Zero => "0".to_string(),
                LogicBit::One => "1".to_string(),
                LogicBit::X => "X".to_string(),
                LogicBit::Z => "Z".to_string(),
            };
        }
        // Check if any X or Z bits
        let mut has_xz = false;
        for i in 0..val.width as usize {
            match val.get_bit(i) {
                LogicBit::X | LogicBit::Z => { has_xz = true; break; }
                _ => {}
            }
        }
        if has_xz {
            // Emit nibble-by-nibble, replacing any nibble containing X/Z
            let nibbles = (val.width as usize + 3) / 4;
            let mut s = String::with_capacity(nibbles + 2);
            s.push_str("0x");
            let mut leading = true;
            for nib in (0..nibbles).rev() {
                let base = nib * 4;
                let mut nibval = 0u8;
                let mut nib_xz = false;
                for b in 0..4 {
                    let bit_idx = base + b;
                    if bit_idx < val.width as usize {
                        match val.get_bit(bit_idx) {
                            LogicBit::One => { nibval |= 1 << b; }
                            LogicBit::X | LogicBit::Z => { nib_xz = true; }
                            _ => {}
                        }
                    }
                }
                if nib_xz {
                    leading = false;
                    s.push('X');
                } else if nibval != 0 || !leading || nib == 0 {
                    leading = false;
                    s.push(char::from_digit(nibval as u32, 16).unwrap().to_ascii_uppercase());
                }
            }
            s
        } else {
            format!("0x{:X}", val.to_u64().unwrap_or(0))
        }
    }

    /// Start AITRACE dump: open file, write header + dictionary + initial snapshot
    fn aitrace_start_dump(&mut self) {
        self.sync_table_to_hashmap();
        let filename = self.vcd_file.clone().unwrap_or_else(|| "dump.aitrace".to_string());
        let file = match std::fs::File::create(&filename) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("Warning: cannot create AITRACE file '{}': {}", filename, e);
                return;
            }
        };
        let mut w: super::vcd_sink::VcdSink = if self.threads >= 2 {
            super::vcd_sink::VcdSink::threaded(file)
        } else {
            super::vcd_sink::VcdSink::inline(file)
        };

        // Collect and sort signal names
        let mut sig_names: Vec<String> = self.signals.keys().cloned().collect();
        sig_names.sort();

        // Build module hierarchy from dotted names
        // e.g. "uut.cpu.pc" → module /testbench/uut/cpu, signal pc
        let top_name = &self.module.name;
        let mut modules: Vec<(String, String)> = Vec::new(); // (module_id, path)
        let mut module_map: HashMap<String, String> = HashMap::new(); // path → module_id
        // Always add top module
        let top_path = format!("/{}", top_name);
        modules.push(("m0".to_string(), top_path.clone()));
        module_map.insert(top_path.clone(), "m0".to_string());

        // Discover all module scopes from signal names
        for name in &sig_names {
            let parts: Vec<&str> = name.split('.').collect();
            if parts.len() > 1 {
                // Build scope path incrementally
                let mut path = format!("/{}", top_name);
                for part in &parts[..parts.len()-1] {
                    path = format!("{}/{}", path, part);
                    if !module_map.contains_key(&path) {
                        let mid = format!("m{}", modules.len());
                        module_map.insert(path.clone(), mid.clone());
                        modules.push((mid, path.clone()));
                    }
                }
            }
        }

        // Assign signal IDs and build signal records
        let mut id_map = HashMap::new();
        let mut signal_records: Vec<String> = Vec::new();
        for (idx, name) in sig_names.iter().enumerate() {
            let sid = format!("s{}", idx);
            id_map.insert(name.clone(), sid.clone());

            // Determine owning module and leaf name
            let parts: Vec<&str> = name.split('.').collect();
            let (mod_id, leaf) = if parts.len() > 1 {
                let mut path = format!("/{}", top_name);
                for part in &parts[..parts.len()-1] {
                    path = format!("{}/{}", path, part);
                }
                (module_map.get(&path).cloned().unwrap_or_else(|| "m0".to_string()), parts[parts.len()-1].to_string())
            } else {
                ("m0".to_string(), name.clone())
            };

            let width = self.lookup_signal_width(name).unwrap_or(1);
            let is_signed = self.lookup_signal_signed(name);
            let type_str = if width == 1 {
                "bit".to_string()
            } else if is_signed {
                format!("s{}", width)
            } else {
                format!("u{}", width)
            };

            signal_records.push(format!("S,{},{},{},{}", sid, mod_id, leaf, type_str));
        }

        // Write AITRACE header
        let _ = writeln!(w, "@aitrace 1.0");
        let _ = writeln!(w, "@format text");
        let _ = writeln!(w, "@timescale 1ns");
        let _ = writeln!(w, "@design {}", top_name);
        let _ = writeln!(w, "@profile full_debug");
        let _ = writeln!(w, "");

        // Write dictionary section
        let _ = writeln!(w, "@section dict");
        for (mid, path) in &modules {
            let _ = writeln!(w, "M,{},{}", mid, path);
        }
        for rec in &signal_records {
            let _ = writeln!(w, "{}", rec);
        }
        let _ = writeln!(w, "");

        // Write trace section header
        let _ = writeln!(w, "@section trace");

        // Write initial time and snapshot
        let _ = writeln!(w, "T,+0");

        // Write initial snapshot (N,full,...)
        let mut snap_parts: Vec<String> = Vec::new();
        for name in &sig_names {
            let sid = &id_map[name];
            let val = self.lookup_signal_value(name).unwrap_or_else(|| Value::new(1));
            snap_parts.push(format!("{}={}", sid, Self::aitrace_format_value(&val)));
        }
        // Write snapshot in chunks to avoid excessively long lines
        if snap_parts.len() <= 20 {
            let _ = writeln!(w, "N,full,{}", snap_parts.join(","));
        } else {
            // Write as multiple packed delta records for initial state
            for chunk in snap_parts.chunks(16) {
                let _ = writeln!(w, "P,{}", chunk.join(","));
            }
        }

        // Record initial snapshot for change detection
        let vcd_prev = self.signals.clone();

        self.vcd_id_map = id_map;
        self.vcd_writer = Some(w);
        self.vcd_enabled = true;
        self.vcd_last_time = self.time;
        self.vcd_prev_signals = vcd_prev;
    }

    fn is_pid_suspended(&self, pid: usize) -> bool {
        if self.event_queue.has_pid(pid) { return true; }
        if self.event_waiters.iter().any(|w| w.pid == pid) { return true; }
        false
    }

    fn child_finished(&mut self, child_pid: usize) {
        self.process_parents.remove(&child_pid);
        let mut finished_parents = Vec::new();
        for (i, waiter) in self.join_waiters.iter_mut().enumerate() {
            if waiter.child_pids.contains(&child_pid) {
                waiter.finished_children.insert(child_pid);
                let should_wake = match waiter.join_type {
                    JoinType::Join => waiter.finished_children.len() == waiter.child_pids.len(),
                    JoinType::JoinAny => !waiter.finished_children.is_empty(),
                    JoinType::JoinNone => true,
                };
                if should_wake {
                    sim_dbg_eprintln!("[DEBUG] join waiter for parent process {} triggered at time {}", waiter.parent_pid, self.time);
                    finished_parents.push(i);
                }
            }
        }
        
        finished_parents.sort_by(|a, b| b.cmp(a));
        for i in finished_parents {
            let waiter = self.join_waiters.remove(i);
            self.event_queue.schedule(self.time, waiter.parent_pid, waiter.continuation);
        }
    }

    /// Write AITRACE signal deltas for the current timestep
    fn aitrace_write_changes(&mut self) {
        if !self.vcd_enabled || self.vcd_writer.is_none() { return; }

        // Collect changes
        let mut changes: Vec<(String, String)> = Vec::new(); // (signal_id, formatted_value)
        for (id, name) in self.id_to_name.iter().enumerate() {
            let name_str: &str = name.as_ref();
            if let Some(sid) = self.vcd_id_map.get(name_str) {
                let val = &self.signal_table[id];
                let changed = match self.vcd_prev_signals.get(name_str) {
                    Some(prev) => prev != val,
                    None => true,
                };
                if changed {
                    changes.push((sid.clone(), Self::aitrace_format_value(val)));
                }
            }
        }

        if changes.is_empty() { return; }

        let w = self.vcd_writer.as_mut().unwrap();

        // Write time delta if needed
        if self.time != self.vcd_last_time {
            let delta = self.time - self.vcd_last_time;
            let _ = writeln!(w, "T,+{}", delta);
            self.vcd_last_time = self.time;
        }

        // Use packed format (P) when multiple signals change, single delta (D) for one
        if changes.len() == 1 {
            let (sid, val) = &changes[0];
            let _ = writeln!(w, "D,{},{}", sid, val);
        } else {
            // Write packed records in chunks of 16
            for chunk in changes.chunks(16) {
                let parts: Vec<String> = chunk.iter()
                    .map(|(sid, val)| format!("{}={}", sid, val))
                    .collect();
                let _ = writeln!(w, "P,{}", parts.join(","));
            }
        }

        // Update previous snapshot
        for (id, name) in self.id_to_name.iter().enumerate() {
            self.vcd_prev_signals.insert(name.to_string(), self.signal_table[id].clone());
        }
    }

    /// Flush and close AITRACE file
    fn aitrace_finish(&mut self) {
        if let Some(ref mut w) = self.vcd_writer {
            let _ = writeln!(w, "");
            let _ = writeln!(w, "@section end");
            let _ = w.flush();
        }
        self.vcd_writer = None;
    }

    fn is_associative_array(&self, name: &str) -> bool {
        self.module.associative_arrays.contains_key(name)
    }

    fn is_string_keyed_array(&self, name: &str) -> bool {
        self.module.associative_arrays.get(name).copied().unwrap_or(false)
    }

    fn assoc_key_str(&self, name: &str, idx_val: &Value) -> String {
        if self.is_string_keyed_array(name) {
            idx_val.to_sv_string()
        } else {
            idx_val.to_u64().unwrap_or(0).to_string()
        }
    }

    fn get_signal_value_by_name(&self, name: &str) -> Option<Value> {
        if let Some(&id) = self.signal_name_to_id.get(name) {
            let mut v = self.signal_table[id].clone();
            if self.signal_signed[id] { v.is_signed = true; }
            Some(v)
        } else {
            self.signals.get(name).cloned()
        }
    }

    fn set_signal_value_by_name(&mut self, name: &str, val: Value) {
        if let Some(&id) = self.signal_name_to_id.get(name) {
            let w = self.signal_widths[id];
            let resized = val.resize(w);
            if self.signal_table[id] != resized {
                if !self.dirty_signals[id] { self.dirty_signals[id] = true; self.dirty_list.push(id); }
                self.dirty_any = true;
                self.signal_table[id] = resized;
                self.table_modified = true;
            }
        } else {
            self.signals.insert(name.to_string(), val);
        }
    }

    fn get_queue_size(&self, obj_name: &str) -> u64 {
        if let Some(v) = self.signals.get(&format!("{}.size", obj_name)) {
            return v.to_u64().unwrap_or(0);
        }
        if self.module.dynamic_arrays.contains(obj_name) {
            return 0;
        }
        if let Some((lo, hi, _)) = self.module.arrays.get(obj_name) {
            return (hi - lo + 1) as u64;
        }
        0
    }

    fn set_queue_size(&mut self, obj_name: &str, size: u64) {
        self.signals.insert(format!("{}.size", obj_name), Value::from_u64(size, 32));
    }

    fn eval_builtin_method(&mut self, obj_name: &str, mname: &str, args: &[Expression]) -> Option<Value> {
        // If obj_name is a mailbox/semaphore handle variable, don't treat it as an array.
        if matches!(mname, "num" | "put" | "get" | "peek" | "try_put" | "try_get" | "try_peek") {
            if let Some(handle_val) = self.get_signal_value_by_name(obj_name) {
                let handle = handle_val.to_u64().unwrap_or(0) as usize;
                if self.mailboxes.contains_key(&handle) || self.semaphores.contains_key(&handle) {
                    return None;
                }
            }
        }
        if mname == "size" || mname == "len" {
            if let Some(v) = self.signals.get(&format!("{}.size", obj_name)) {
                return Some(v.clone());
            }
            if let Some((lo, hi, _)) = self.module.arrays.get(obj_name) {
                return Some(Value::from_u64((hi - lo + 1) as u64, 32));
            }
            // Fallback for strings
            let base_val = self.get_signal_value_by_name(obj_name);

            if let Some(base) = base_val {
                let w = base.width;
                let bytes = w / 8;
                let mut len = 0u64;
                for b in 0..bytes {
                    let mut byte_val = 0u8;
                    for bit in 0..8 {
                        if base.get_bit((b * 8 + bit) as usize) == LogicBit::One { byte_val |= 1 << bit; }
                    }
                    if byte_val != 0 {
                        len += 1;
                    }
                }
                return Some(Value::from_u64(len, 32));
            }
            return Some(Value::zero(32));
        }
        if mname == "substr" {
             if let Some(first) = args.get(0) {
                 if let Some(second) = args.get(1) {
                     let start = self.eval_expr(first).to_u64().unwrap_or(0) as usize;
                     let end = self.eval_expr(second).to_u64().unwrap_or(0) as usize;
                     
                     let base_val = self.get_signal_value_by_name(obj_name);

                     if let Some(base) = base_val {
                         let mut highest_bit = 0;
                         for i in (0..base.width as usize).rev() {
                             if base.get_bit(i) != LogicBit::Zero {
                                 highest_bit = i;
                                 break;
                             }
                         }
                         let actual_len = (highest_bit + 7) / 8;
                         if actual_len > 0 && start < actual_len && end < actual_len && start <= end {
                             let l = (actual_len - 1 - start) * 8 + 7;
                             let r = (actual_len - 1 - end) * 8;
                             return Some(base.range_select(l, r));
                         }
                     }
                 }
             }
             return Some(Value::zero(0));
        }
        if mname == "push_back" {
             if let Some(arg) = args.first() {
                 let val = self.eval_expr(arg);
                 let cur_size = self.get_queue_size(obj_name);
                 if let Some(&max) = self.module.queue_max_sizes.get(obj_name) {
                     if cur_size >= max as u64 { return Some(Value::zero(32)); }
                 }
                 self.set_signal_value_by_name(&format!("{}[{}]", obj_name, cur_size), val);
                 self.set_queue_size(obj_name, cur_size + 1);
             }
             return Some(Value::zero(32));
        }
        if mname == "push_front" {
             if let Some(arg) = args.first() {
                 let val = self.eval_expr(arg);
                 let cur_size = self.get_queue_size(obj_name);
                 if let Some(&max) = self.module.queue_max_sizes.get(obj_name) {
                     if cur_size >= max as u64 { return Some(Value::zero(32)); }
                 }
                 for i in (0..cur_size).rev() {
                     if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", obj_name, i)) {
                         self.set_signal_value_by_name(&format!("{}[{}]", obj_name, i+1), v);
                     }
                 }
                 self.set_signal_value_by_name(&format!("{}[0]", obj_name), val);
                 self.set_queue_size(obj_name, cur_size + 1);
             }
             return Some(Value::zero(32));
        }
        if mname == "pop_front" {
             let cur_size = self.get_queue_size(obj_name);
             if cur_size > 0 {
                 let val = self.get_signal_value_by_name(&format!("{}[0]", obj_name)).unwrap_or_else(|| Value::zero(32));
                 for i in 1..cur_size {
                     if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", obj_name, i)) {
                         self.set_signal_value_by_name(&format!("{}[{}]", obj_name, i-1), v);
                     }
                 }
                 self.set_queue_size(obj_name, cur_size - 1);
                 return Some(val);
             }
             return Some(Value::zero(32));
        }
        if mname == "pop_back" {
             let cur_size = self.get_queue_size(obj_name);
             if cur_size > 0 {
                 let val = self.get_signal_value_by_name(&format!("{}[{}]", obj_name, cur_size - 1)).unwrap_or_else(|| Value::zero(32));
                 self.set_queue_size(obj_name, cur_size - 1);
                 return Some(val);
             }
             return Some(Value::zero(32));
        }
        if mname == "sort" {
            let cur_size = self.get_queue_size(obj_name) as usize;
            if cur_size > 0 {
                let mut elements = Vec::new();
                for i in 0..cur_size {
                    if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", obj_name, i)) {
                        elements.push(v);
                    }
                }
                elements.sort_by(|a, b| a.to_u64().unwrap_or(0).cmp(&b.to_u64().unwrap_or(0)));
                for (i, v) in elements.into_iter().enumerate() {
                    self.set_signal_value_by_name(&format!("{}[{}]", obj_name, i), v);
                }
            }
            return Some(Value::zero(32));
        }
        if mname == "rsort" {
            let cur_size = self.get_queue_size(obj_name) as usize;
            if cur_size > 0 {
                let mut elements = Vec::new();
                for i in 0..cur_size {
                    if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", obj_name, i)) {
                        elements.push(v);
                    }
                }
                elements.sort_by(|a, b| b.to_u64().unwrap_or(0).cmp(&a.to_u64().unwrap_or(0)));
                for (i, v) in elements.into_iter().enumerate() {
                    self.set_signal_value_by_name(&format!("{}[{}]", obj_name, i), v);
                }
            }
            return Some(Value::zero(32));
        }
        if mname == "reverse" {
            let cur_size = self.get_queue_size(obj_name) as usize;
            if cur_size > 0 {
                let mut elements = Vec::new();
                for i in 0..cur_size {
                    if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", obj_name, i)) {
                        elements.push(v);
                    }
                }
                elements.reverse();
                for (i, v) in elements.into_iter().enumerate() {
                    self.set_signal_value_by_name(&format!("{}[{}]", obj_name, i), v);
                }
            }
            return Some(Value::zero(32));
        }
        if mname == "insert" {
            if args.len() >= 2 {
                let idx = self.eval_expr(&args[0]).to_u64().unwrap_or(0);
                let val = self.eval_expr(&args[1]);
                let cur_size = self.get_queue_size(obj_name);
                for i in (idx..cur_size).rev() {
                    if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", obj_name, i)) {
                        self.set_signal_value_by_name(&format!("{}[{}]", obj_name, i+1), v);
                    }
                }
                self.set_signal_value_by_name(&format!("{}[{}]", obj_name, idx), val);
                self.set_queue_size(obj_name, cur_size + 1);
            }
            return Some(Value::zero(32));
        }
        if mname == "num" {
             let prefix = format!("{}[", obj_name);
             let count1 = self.signals.keys().filter(|k| k.starts_with(&prefix)).count();
             let count2 = self.signal_name_to_id.keys().filter(|k| k.starts_with(&prefix)).count();
             return Some(Value::from_u64((count1 + count2) as u64, 32));
        }
        if mname == "sum" {
            let cur_size = self.get_queue_size(obj_name) as usize;
            let mut total = 0u64;
            for i in 0..cur_size {
                if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", obj_name, i)) {
                    total = total.wrapping_add(v.to_u64().unwrap_or(0));
                }
            }
            return Some(Value::from_u64(total, 32));
        }
        if mname == "product" {
            let cur_size = self.get_queue_size(obj_name) as usize;
            let mut total = 1u64;
            for i in 0..cur_size {
                if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", obj_name, i)) {
                    total = total.wrapping_mul(v.to_u64().unwrap_or(0));
                }
            }
            return Some(Value::from_u64(total, 32));
        }
        if matches!(mname, "and" | "or" | "xor") {
            let cur_size = self.get_queue_size(obj_name) as usize;
            let mut acc: Option<u64> = None;
            for i in 0..cur_size {
                if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", obj_name, i)) {
                    let x = v.to_u64().unwrap_or(0);
                    acc = Some(match (acc, mname) {
                        (None, _) => x,
                        (Some(a), "and") => a & x,
                        (Some(a), "or")  => a | x,
                        (Some(a), "xor") => a ^ x,
                        (Some(a), _) => a,
                    });
                }
            }
            return Some(Value::from_u64(acc.unwrap_or(0), 32));
        }
        if mname == "min" {
            let cur_size = self.get_queue_size(obj_name) as usize;
            let mut min_val: Option<Value> = None;
            for i in 0..cur_size {
                if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", obj_name, i)) {
                    if min_val.is_none() || v.to_u64().unwrap_or(u64::MAX) < min_val.as_ref().unwrap().to_u64().unwrap_or(u64::MAX) {
                        min_val = Some(v);
                    }
                }
            }
            return Some(min_val.unwrap_or(Value::zero(32)));
        }
        if mname == "max" {
            let cur_size = self.get_queue_size(obj_name) as usize;
            let mut max_val: Option<Value> = None;
            for i in 0..cur_size {
                if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", obj_name, i)) {
                    if max_val.is_none() || v.to_u64().unwrap_or(0) > max_val.as_ref().unwrap().to_u64().unwrap_or(0) {
                        max_val = Some(v);
                    }
                }
            }
            return Some(max_val.unwrap_or(Value::zero(32)));
        }
        if mname == "unique" {
            let cur_size = self.get_queue_size(obj_name) as usize;
            let mut seen = std::collections::HashSet::new();
            let mut result = Vec::new();
            for i in 0..cur_size {
                if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", obj_name, i)) {
                    let key = v.to_u64().unwrap_or(0);
                    if seen.insert(key) {
                        result.push(v);
                    }
                }
            }
            for (i, v) in result.iter().enumerate() {
                self.set_signal_value_by_name(&format!("{}[{}]", obj_name, i), v.clone());
            }
            self.set_queue_size(obj_name, result.len() as u64);
            return Some(Value::zero(32));
        }
        if mname == "unique_index" {
            let cur_size = self.get_queue_size(obj_name) as usize;
            let mut seen = std::collections::HashSet::new();
            let mut indices = Vec::new();
            for i in 0..cur_size {
                if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", obj_name, i)) {
                    let key = v.to_u64().unwrap_or(0);
                    if seen.insert(key) {
                        indices.push(i as u64);
                    }
                }
            }
            for (i, idx) in indices.iter().enumerate() {
                self.set_signal_value_by_name(&format!("{}[{}]", obj_name, i), Value::from_u64(*idx, 32));
            }
            self.set_queue_size(obj_name, indices.len() as u64);
            return Some(Value::zero(32));
        }
        if mname == "find" || mname == "find_first" || mname == "find_last" ||
           mname == "find_index" || mname == "find_first_index" || mname == "find_last_index" {
            let cur_size = self.get_queue_size(obj_name) as usize;
            let mut results = Vec::new();
            if let Some(callback) = args.first() {
                for i in 0..cur_size {
                    if let Some(v) = self.get_signal_value_by_name(&format!("{}[{}]", obj_name, i)) {
                        let idx_match = if mname.contains("index") { Value::from_u64(i as u64, 32) } else { v.clone() };
                        if let ExprKind::Binary { op: BinaryOp::LogAnd, .. } | ExprKind::Binary { op: _, .. } = &callback.kind {
                            // Simple "with" clause: evaluate with item substituted
                            // For now just collect all non-zero
                            if v.to_u64().unwrap_or(0) != 0 {
                                results.push(idx_match);
                            }
                        } else {
                            results.push(idx_match);
                        }
                    }
                }
            }
            if mname.contains("first") { results.truncate(1); }
            if mname.contains("last") && !results.is_empty() {
                let last = results.pop().unwrap();
                results = vec![last];
            }
            return Some(if results.is_empty() { Value::zero(32) } else { results[0].clone() });
        }
        if mname == "exists" {
             if let Some(arg) = args.first() {
                 let kv = self.eval_expr(arg);
                 let key = self.assoc_key_str(obj_name, &kv);
                 let elem_name = format!("{}[{}]", obj_name, key);
                 let found = self.signals.contains_key(&elem_name) || self.signal_name_to_id.contains_key(elem_name.as_str());
                 return Some(Value::from_u64(found as u64, 1));
             }
        }
        if mname == "delete" {
            if self.is_associative_array(obj_name) {
                if let Some(arg) = args.first() {
                    let key = self.eval_expr(arg);
                    let key_str = self.assoc_key_str(obj_name, &key);
                    let elem_name = format!("{}[{}]", obj_name, key_str);
                    self.signals.remove(&elem_name);
                } else {
                    let prefix = format!("{}[", obj_name);
                    let keys: Vec<String> = self.signals.keys().filter(|k| k.starts_with(&prefix)).cloned().collect();
                    for k in keys { self.signals.remove(&k); }
                }
                return Some(Value::zero(32));
            }
            if self.module.arrays.contains_key(obj_name) {
                if let Some(arg) = args.first() {
                    let idx = self.eval_expr(arg).to_u64().unwrap_or(0) as usize;
                    let cur_size = self.get_queue_size(obj_name) as usize;
                    if idx < cur_size {
                        for i in idx..cur_size-1 {
                            let next_val = self.get_signal_value_by_name(&format!("{}[{}]", obj_name, i+1)).unwrap_or(Value::zero(32));
                            self.set_signal_value_by_name(&format!("{}[{}]", obj_name, i), next_val);
                        }
                        self.set_queue_size(obj_name, (cur_size - 1) as u64);
                    }
                } else {
                    self.set_queue_size(obj_name, 0);
                }
                return Some(Value::zero(32));
            }
        }
        if mname == "first" || mname == "last" || mname == "next" || mname == "prev" {
            if self.is_associative_array(obj_name) {
                let prefix = format!("{}[", obj_name);
                let mut keys: Vec<String> = self.signals.keys()
                    .filter(|k| k.starts_with(&prefix) && k.ends_with(']'))
                    .map(|k| k[prefix.len()..k.len()-1].to_string())
                    .collect();
                let all_numeric = keys.iter().all(|k| k.parse::<i64>().is_ok());
                if all_numeric {
                    keys.sort_by_key(|k| k.parse::<i64>().unwrap_or(0));
                } else {
                    keys.sort();
                }
                if keys.is_empty() {
                    return Some(Value::zero(32));
                }
                let result_key = if mname == "first" {
                    Some(keys[0].clone())
                } else if mname == "last" {
                    Some(keys[keys.len()-1].clone())
                } else {
                    if let Some(arg) = args.first() {
                        let cur_val = self.eval_expr(arg);
                        let cur = if all_numeric {
                            cur_val.to_u64().unwrap_or(0).to_string()
                        } else {
                            cur_val.to_sv_string()
                        };
                        if let Some(pos) = keys.iter().position(|k| *k == cur) {
                            if mname == "next" {
                                if pos + 1 < keys.len() { Some(keys[pos + 1].clone()) } else { None }
                            } else {
                                if pos > 0 { Some(keys[pos - 1].clone()) } else { None }
                            }
                        } else {
                            None
                        }
                    } else { None }
                };
                if let Some(key) = result_key {
                    if let Some(arg) = args.first() {
                        if all_numeric {
                            let key_int = key.parse::<i64>().unwrap_or(0);
                            let w = self.infer_width(arg);
                            self.assign_value(arg, &Value::from_u64(key_int as u64, w));
                        } else {
                            let key_val = Value::from_string(&key);
                            self.assign_value(arg, &key_val);
                        }
                    }
                    return Some(Value::from_u64(1, 32));
                } else {
                    return Some(Value::zero(32));
                }
            }
        }
        None
    }

    fn eval_call(&mut self, func: &Expression, args: &[Expression]) -> Value {
        // Intercept UVM method calls
        if let ExprKind::MemberAccess { expr, member } = &func.kind {
            let mname = member.name.as_str();
            
            // Check for built-in methods on identifiers
            if let ExprKind::Ident(hier) = &expr.kind {
                let name = self.resolve_hier_name(hier);
                if let Some(res) = self.eval_builtin_method(&name, mname, args) {
                    return res;
                }
                // Package scope resolution: `pkg::func(args)`. Only when LHS is an
                // explicitly known package name — not a class, signal, or handle.
                if hier.path.len() == 1 && self.module.packages.contains(&name) {
                    if let Some(fd) = self.module.functions.get(mname).cloned() {
                        return self.exec_function_call(&fd, args);
                    }
                    if let Some(td) = self.module.tasks.get(mname).cloned() {
                        self.exec_task_call(&td, args);
                        return Value::zero(32);
                    }
                }
            }

            if mname == "put" {
                let base = self.eval_expr(expr);
                let handle = base.to_u64().unwrap_or(0) as usize;
                let val = args.first().map(|a| self.eval_expr(a));
                let mut changed = false;
                if let Some(v) = val {
                    if let Some(q) = self.mailboxes.get_mut(&handle) {
                        q.push_back(v);
                        changed = true;
                    } else if let Some(count) = self.semaphores.get_mut(&handle) {
                        *count += v.to_u64().unwrap_or(1) as i64;
                        changed = true;
                    }
                }
                if changed {
                    // Waking up might be needed, but simplified: waking up is usually via events.
                    // For mailboxes/semaphores, we'd need dedicated waiter queues.
                }
                return Value::zero(32);
            }
            if mname == "get" {
                let base = self.eval_expr(expr);
                let handle = base.to_u64().unwrap_or(0) as usize;
                let arg_val = args.first().map(|a| self.eval_expr(a));
                
                if self.mailboxes.contains_key(&handle) {
                    let has_item = !self.mailboxes[&handle].is_empty();
                    if has_item {
                        let val = self.mailboxes.get_mut(&handle).unwrap().pop_front().unwrap();
                        if let Some(arg) = args.first() {
                            let w = self.infer_lhs_width(arg);
                            self.assign_value(arg, &val.resize(w));
                        }
                        return Value::zero(32);
                    } else {
                        // BLOCKING: simplified, just retry later or return for now
                        return Value::zero(32);
                    }
                } else if let Some(count) = self.semaphores.get_mut(&handle) {
                    let n = arg_val.map(|v| v.to_u64().unwrap_or(1)).unwrap_or(1) as i64;
                    if *count >= n {
                        *count -= n;
                        return Value::zero(32);
                    } else {
                        // BLOCKING: simplified
                        return Value::zero(32);
                    }
                }
                return Value::zero(32);
            }
            if mname == "try_get" {
                let base = self.eval_expr(expr);
                let handle = base.to_u64().unwrap_or(0) as usize;
                if self.mailboxes.contains_key(&handle) {
                    let val = self.mailboxes.get_mut(&handle).and_then(|q| q.pop_front());
                    if let (Some(v), Some(arg)) = (val, args.first()) {
                        let w = self.infer_lhs_width(arg);
                        self.assign_value(arg, &v.resize(w));
                        return Value::from_u64(1, 32);
                    }
                }
                return Value::zero(32);
            }

            let base = self.eval_expr(expr);

            // Fallback for non-identifier base (e.g. string literals)
            if mname == "len" || mname == "size" {
                let w = base.width;
                let bytes = w / 8;
                let mut len = 0u64;
                for b in 0..bytes {
                    let mut byte_val = 0u8;
                    for bit in 0..8 {
                        if base.get_bit((b * 8 + bit) as usize) == LogicBit::One { byte_val |= 1 << bit; }
                    }
                    if byte_val != 0 { len += 1; }
                }
                return Value::from_u64(len, 32);
            }

            if mname == "get_next_item" {
                if let Some(arg) = args.first() {
                    // Create a simple_transaction
                    if let Some(class_def) = self.module.classes.get("simple_transaction").cloned() {
                        let handle = self.instantiate_class(&class_def, &[]);
                        // Hardcode data to something
                        if let Some(Some(inst)) = self.heap.get_mut(handle.to_u64().unwrap_or(0) as usize) {
                            inst.properties.insert("data".to_string(), Value::from_u64(42, 32));
                        }
                        let w = self.infer_lhs_width(arg);
                        self.assign_value(arg, &handle.resize(w));
                    }
                }
                return Value::zero(32);
            }
            if mname == "create" {
                if let ExprKind::MemberAccess { expr: inner_expr, member: inner_member } = &expr.kind {
                    if inner_member.name.as_str() == "type_id" {
                        if let ExprKind::Ident(hier) = &inner_expr.kind {
                            let class_name = &hier.path[0].name.name;
                            if let Some(class_def) = self.module.classes.get(class_name).cloned() {
                                return self.instantiate_class(&class_def, &[]);
                            }
                        }
                    }
                }
            }
            if mname == "item_done" || mname == "connect" || mname == "raise_objection" || mname == "drop_objection" {
                return Value::zero(32);
            }
            if mname == "write" {
                // Call write on scoreboard
                let mut sb_handles = Vec::new();
                for i in 1..self.heap.len() {
                    if let Some(Some(inst)) = self.heap.get(i) {
                        if inst.class_name.contains("scoreboard") {
                            sb_handles.push(i);
                        }
                    }
                }
                for handle in sb_handles {
                    self.exec_method_call(handle, "write", args);
                }
                return Value::zero(32);
            }

            if let ExprKind::Ident(hier) = &expr.kind {
                if hier.path.last().unwrap().name.name == "super" {
                    if let Some(Some(handle)) = self.this_stack.last() {
                        return self.exec_super_method_call(*handle, &member.name, args);
                    }
                }
            }
            
            let base = self.eval_expr(expr);
            let handle = base.to_u64().unwrap_or(0) as usize;
            if handle != 0 {
                if let Some(Some(_)) = self.cg_heap.get(handle) {
                    return self.exec_cg_method_call(handle, &member.name, args);
                }
                return self.exec_method_call(handle, &member.name, args);
            }
        }
        // Handle hierarchical ident call: obj.f()
        if let ExprKind::Ident(hier) = &func.kind {
            let path = &hier.path;
            let len = path.len();
            
            // Intercept uvm_report_info and enabled
            if len == 1 {
                let name = &path[0].name.name;
                if name.starts_with('$') {
                    return match name.as_str() {
                        "$time" => Value::from_u64(self.time, 64),
                        "$test$plusargs" => {
                            let pat = match args.first().map(|a| &a.kind) {
                                Some(ExprKind::StringLiteral(s)) => s.clone(),
                                Some(_) => self.eval_expr(&args[0]).to_sv_string(),
                                None => String::new(),
                            };
                            Value::from_u64(self.test_plusarg(&pat) as u64, 1)
                        }
                        "$value$plusargs" => self.eval_value_plusargs(args),
                        "$fopen" => self.open_file_handle(args),
                        "$fclose" => self.close_file_handle(args),
                        "$fwrite" => self.write_file_handle(args, false),
                        "$fdisplay" => self.write_file_handle(args, true),
                        "$readmemh" => self.read_memory_file(args, 16),
                        "$readmemb" => self.read_memory_file(args, 2),
                        "$display" | "$displayb" | "$displayh" | "$displayo" |
                        "$write" | "$writeb" | "$writeh" | "$writeo" => {
                            self.exec_system_task(name, args);
                            Value::zero(32)
                        }
                        _ => Value::zero(32),
                    };
                }
                if name == "uvm_report_enabled" {
                    return Value::from_u64(1, 32); // Always enabled for mock
                }
                if name == "get_is_active" {
                    // UVM_ACTIVE is typically 1 in UVM
                    return Value::from_u64(1, 32);
                }
                if name == "uvm_report_info" || name == "uvm_report_warning" || name == "uvm_report_error" || name == "uvm_report_fatal" {
                    let id = if args.len() > 0 {
                        if let ExprKind::StringLiteral(s) = &args[0].kind { s.clone() } else { "UVM".to_string() }
                    } else { "".to_string() };
                    let msg = if args.len() > 1 {
                        if let ExprKind::StringLiteral(s) = &args[1].kind { s.clone() }
                        else if let ExprKind::SystemCall { name, args: sys_args } = &args[1].kind {
                            if name == "$sformatf" {
                                // Since we don't have self as mutable here in a way we can call format_args easily if it takes &mut self
                                // Actually format_args takes &mut self, eval_call takes &mut self.
                                self.format_args(sys_args, "$sformatf")
                            } else { "<expr>".to_string() }
                        } else { "<expr>".to_string() }
                    } else { "".to_string() };
                    let severity = name.replace("uvm_report_", "").to_uppercase();
                    println!("UVM_{} @ {:>3}: reporter [{}] {}", severity, self.time, id, msg);
                    return Value::zero(32);
                }
                if name == "run_test" {
                    let test_name = if let Some(arg) = args.first() {
                        if let ExprKind::StringLiteral(s) = &arg.kind { s.clone() } else { "simple_test".to_string() }
                    } else { "simple_test".to_string() };
                    println!("UVM_INFO @ {:>3}: reporter [RNTST] Running test {}...", self.time, test_name);
                    
                    if let Some(test_def) = self.module.classes.get(&test_name).cloned() {
                        let handle = self.instantiate_class(&test_def, &[]);
                        let handle_val = handle.to_u64().unwrap_or(0) as usize;
                        
                        // Bootstrapping UVM phases
                        let mut components = vec![handle_val];
                        
                        // build_phase
                        let mut i = 0;
                        while i < components.len() {
                            let c = components[i];
                            let heap_len = self.heap.len();
                            self.exec_method_call(c, "build_phase", &[]);
                            for new_h in heap_len..self.heap.len() {
                                components.push(new_h);
                            }
                            i += 1;
                        }
                        
                        // connect_phase
                        for &c in &components {
                            self.exec_method_call(c, "connect_phase", &[]);
                        }
                        
                        // run_phase
                        for &c in &components {
                            if !self.spawn_method_task_process(c, "run_phase", &[]) {
                                self.exec_method_call(c, "run_phase", &[]);
                            }
                        }
                    }
                    return Value::zero(32);
                }
            }

            // Intercept type_id::create
            if len >= 3 && path[len-1].name.name == "create" && path[len-2].name.name == "type_id" {
                let class_name = &path[len-3].name.name;
                if let Some(class_def) = self.module.classes.get(class_name).cloned() {
                    return self.instantiate_class(&class_def, &[]);
                }
            } else if len >= 2 && path[len-1].name.name == "create" {
            }

            if hier.path.len() > 1 {
                let obj_name = &hier.path[0].name.name;
                let method_name = &hier.path.last().unwrap().name.name;
                
                if let Some(res) = self.eval_builtin_method(obj_name, method_name, args) {
                    return res;
                }
                
                let obj_val = if let Some(locals) = self.local_stack.last() {
                    locals.get(obj_name).cloned()
                } else {
                    if let Some(&id) = self.signal_name_to_id.get(obj_name.as_str()) {
                        Some(self.signal_table[id].clone())
                    } else {
                        self.signals.get(obj_name).cloned()
                    }
                };
                if let Some(v) = obj_val {
                    let handle = v.to_u64().unwrap_or(0) as usize;
                    if handle != 0 {
                        if handle < self.cg_heap.len() && self.cg_heap[handle].is_some() {
                            return self.exec_cg_method_call(handle, method_name, args);
                        }
                        if handle < self.heap.len() && self.heap[handle].is_some() {
                            return self.exec_method_call(handle, method_name, args);
                        }
                    }
                }
            }
            // Handle static/constructor call: class_name::f() or new()
            let name = &hier.path.last().unwrap().name.name;
            if let Some(class_def) = self.module.classes.get(name).cloned() {
                return self.instantiate_class(&class_def, args);
            }
            if let Some(cg_def) = self.module.covergroups.get(name).cloned() {
                return self.instantiate_covergroup(&cg_def, args);
            }
            // DPI import call
            if let Some(v) = self.exec_dpi_import_call(name, args) {
                return v;
            }
            // Module-level function call
            if let Some(fd) = self.module.functions.get(name).cloned() {
                return self.exec_function_call(&fd, args);
            }
            // Module-level let call
            if let Some(ld) = self.module.lets.get(name).cloned() {
                return self.exec_let_call(&ld, args);
            }
            // Module-level task call
            if let Some(td) = self.module.tasks.get(name).cloned() {
                self.exec_task_call(&td, args);
                return Value::zero(32);
            }
        }
        Value::zero(32)
    }

    fn exec_let_call(&mut self, ld: &LetDeclaration, args: &[Expression]) -> Value {
        use crate::ast::module::PortList;
        let mut locals = HashMap::new();
        let mut arg_idx = 0usize;
        match &ld.ports {
            PortList::Ansi(ports) => {
                for p in ports {
                    let v = if arg_idx < args.len() {
                        self.eval_expr(&args[arg_idx])
                    } else {
                        Value::zero(32)
                    };
                    locals.insert(p.name.name.clone(), v);
                    arg_idx += 1;
                }
            }
            PortList::NonAnsi(names) => {
                for n in names {
                    let v = if arg_idx < args.len() {
                        self.eval_expr(&args[arg_idx])
                    } else {
                        Value::zero(32)
                    };
                    locals.insert(n.name.clone(), v);
                    arg_idx += 1;
                }
            }
            PortList::Empty => {}
        }
        self.local_stack.push(locals);
        let out = self.eval_expr(&ld.expr);
        self.local_stack.pop();
        out
    }

    /// Execute a module-level function call with arguments.
    fn exec_function_call(&mut self, fd: &FunctionDeclaration, args: &[Expression]) -> Value {
        use crate::ast::types::PortDirection;
        // Set up local scope with parameters
        let mut locals = HashMap::new();
        for (i, port) in fd.ports.iter().enumerate() {
            let val = if i < args.len() {
                self.eval_expr(&args[i])
            } else if let Some(def) = &port.default {
                self.eval_expr(def)
            } else {
                Value::zero(32)
            };
            locals.insert(port.name.name.clone(), val);
        }
        // Initialize return variable (function name)
        let ret_name = fd.name.name.name.clone();
        locals.insert(ret_name.clone(), Value::zero(32));
        self.local_stack.push(locals);
        self.return_value = None;
        // Execute function body
        for stmt in &fd.items {
            self.exec_statement(stmt);
            if self.return_value.is_some() { break; }
        }
        let result = if let Some(rv) = self.return_value.take() {
            rv
        } else {
            // Return value from function-name variable
            self.local_stack.last().and_then(|l| l.get(&ret_name).cloned()).unwrap_or(Value::zero(32))
        };
        self.local_stack.pop();
        // `return` set break_flag to short-circuit the function body — clear it
        // so the caller's enclosing loop/block isn't terminated.
        self.break_flag = false;
        result
    }

    /// Execute a module-level task call with arguments.
    fn exec_task_call(&mut self, td: &TaskDeclaration, args: &[Expression]) {
        use crate::ast::types::PortDirection;
        // Evaluate input args and collect output/ref arg expressions
        let mut locals = HashMap::new();
        let mut output_bindings: Vec<(String, Expression)> = Vec::new();
        let mut assoc_params: Vec<(String, String)> = Vec::new(); // (param_name, caller_array_name)
        let mut array_params: Vec<String> = Vec::new(); // param names with unpacked Range dim
        for (i, port) in td.ports.iter().enumerate() {
            // Unpacked array parameter (e.g. `int a [2:0]`): copy caller's
            // array elements into `param[idx]` signals so `a[0]` resolves.
            if let Some(crate::ast::types::UnpackedDimension::Range { left, right, .. }) = port.dimensions.first() {
                if i < args.len() {
                    if let ExprKind::Ident(hier) = &args[i].kind {
                        let caller_name = self.resolve_hier_name(hier);
                        let param_name = port.name.name.clone();
                        let l = super::elaborate::const_eval_i64_with_params(left, None).unwrap_or(0);
                        let r = super::elaborate::const_eval_i64_with_params(right, None).unwrap_or(0);
                        let (lo, hi) = (l.min(r), l.max(r));
                        for idx in lo..=hi {
                            let caller_elem = format!("{}[{}]", caller_name, idx);
                            let param_elem = format!("{}[{}]", param_name, idx);
                            if let Some(v) = self.get_signal_value_by_name(&caller_elem) {
                                self.signals.insert(param_elem, v);
                            }
                        }
                        let w = self.module.arrays.get(&caller_name).map(|t| t.2).unwrap_or(32);
                        self.module.arrays.insert(param_name.clone(), (lo, hi, w));
                        array_params.push(param_name);
                        continue;
                    }
                }
            }
            let is_assoc = port.dimensions.iter().any(|d| matches!(d, crate::ast::types::UnpackedDimension::Associative { .. }));
            if is_assoc && i < args.len() {
                if let ExprKind::Ident(hier) = &args[i].kind {
                    let caller_name = self.resolve_hier_name(hier);
                    let param_name = port.name.name.clone();
                    let prefix = format!("{}[", caller_name);
                    let entries: Vec<(String, Value)> = self.signals.iter()
                        .filter(|(k, _)| k.starts_with(&prefix) && k.ends_with(']'))
                        .map(|(k, v)| {
                            let key = &k[prefix.len()..k.len()-1];
                            (format!("{}[{}]", param_name, key), v.clone())
                        })
                        .collect();
                    for (k, v) in entries {
                        self.signals.insert(k, v);
                    }
                    let is_string_key = self.is_string_keyed_array(&caller_name);
                    self.module.associative_arrays.insert(param_name.clone(), is_string_key);
                    assoc_params.push((param_name, caller_name));
                }
                continue;
            }
            match port.direction {
                PortDirection::Output | PortDirection::Inout => {
                    let val = if i < args.len() { self.eval_expr(&args[i]) } else { Value::zero(32) };
                    locals.insert(port.name.name.clone(), val);
                    if i < args.len() {
                        output_bindings.push((port.name.name.clone(), args[i].clone()));
                    }
                }
                PortDirection::Ref => {
                    let val = if i < args.len() { self.eval_expr(&args[i]) } else { Value::zero(32) };
                    locals.insert(port.name.name.clone(), val);
                    if i < args.len() {
                        output_bindings.push((port.name.name.clone(), args[i].clone()));
                    }
                }
                _ => {
                    let val = if i < args.len() { self.eval_expr(&args[i]) } else { Value::zero(32) };
                    locals.insert(port.name.name.clone(), val);
                }
            }
        }
        self.local_stack.push(locals);
        self.return_value = None;
        let prev_static = self.current_static_task.take();
        if matches!(td.lifetime, Some(crate::ast::types::Lifetime::Static)) {
            self.current_static_task = Some(td.name.name.name.clone());
        }
        // Execute task body
        for stmt in &td.items {
            self.exec_statement(stmt);
            if self.return_value.is_some() { break; }
        }
        self.current_static_task = prev_static;
        // Copy output/ref values back to caller
        let locals = self.local_stack.pop().unwrap_or_default();
        for (port_name, caller_expr) in &output_bindings {
            if let Some(val) = locals.get(port_name) {
                self.assign_value(caller_expr, val);
            }
        }
        // Clean up associative array params
        for (param_name, _caller_name) in &assoc_params {
            let prefix = format!("{}[", param_name);
            let keys: Vec<String> = self.signals.keys().filter(|k| k.starts_with(&prefix)).cloned().collect();
            for k in keys { self.signals.remove(&k); }
            self.module.associative_arrays.remove(param_name);
        }
        // Clean up unpacked-array params
        for param_name in &array_params {
            let prefix = format!("{}[", param_name);
            let keys: Vec<String> = self.signals.keys().filter(|k| k.starts_with(&prefix)).cloned().collect();
            for k in keys { self.signals.remove(&k); }
            self.module.arrays.remove(param_name);
        }
        self.break_flag = false;
    }

    fn instantiate_covergroup(&mut self, cg_def: &CovergroupDeclaration, _args: &[Expression]) -> Value {
        let handle = self.cg_heap.len();
        let instance = CovergroupInstance {
            cg_name: cg_def.name.name.clone(),
            point_hits: HashMap::new(),
            cross_hits: HashMap::new(),
        };
        self.cg_heap.push(Some(instance));
        
        // Register automatic sampling if event is present
        if let Some(event) = &cg_def.event {
            let sens = self.event_to_sens(event);
            let resolved: Vec<SensitivityId> = sens.iter().filter_map(|s| {
                self.signal_name_to_id.get(s.signal_name.as_str()).map(|&id| SensitivityId { signal_id: id, edge: s.edge })
            }).collect();
            self.cg_event_waiters.push((handle, resolved));
        }
        
        Value::from_u64(handle as u64, 32)
    }

    fn exec_cg_method_call(&mut self, handle: usize, method_name: &str, _args: &[Expression]) -> Value {
        match method_name {
            "get_inst_coverage" | "get_coverage" => {
                let coverage = self.calculate_coverage(handle);
                // Return real as u64 bits (simplified: return as integer percentage for now)
                Value::from_u64(coverage as u64, 64)
            }
            "sample" => {
                self.sample_covergroup(handle);
                Value::zero(32)
            }
            _ => Value::zero(32),
        }
    }

    fn calculate_coverage(&self, handle: usize) -> f64 {
        let inst = if let Some(Some(i)) = self.cg_heap.get(handle) { i } else { return 0.0; };
        let def = if let Some(d) = self.module.covergroups.get(&inst.cg_name) { d } else { return 0.0; };
        
        let mut total_items = 0;
        let mut covered_items = 0;
        
        for item in &def.items {
            match item {
                CovergroupItem::Coverpoint(cp) => {
                    total_items += 1;
                    let cp_name = cp.name.as_ref().map(|n| n.name.clone()).unwrap_or_else(|| format!("{:?}", cp.expr));
                    if let Some(hits) = inst.point_hits.get(&cp_name) {
                        if !hits.is_empty() { covered_items += 1; }
                    }
                }
                CovergroupItem::Cross(cr) => {
                    total_items += 1;
                    let cr_name = cr.name.as_ref().map(|n| n.name.clone()).unwrap_or_else(|| cr.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join("_"));
                    if let Some(hits) = inst.cross_hits.get(&cr_name) {
                        if !hits.is_empty() { covered_items += 1; }
                    }
                }
                _ => {}
            }
        }
        
        if total_items == 0 { 100.0 }
        else { (covered_items as f64 / total_items as f64) * 100.0 }
    }

    fn sample_covergroup(&mut self, handle: usize) {
        let cg_name = if let Some(Some(inst)) = self.cg_heap.get(handle) {
            inst.cg_name.clone()
        } else { return; };
        
        let def = if let Some(d) = self.module.covergroups.get(&cg_name).cloned() { d } else { return; };
        
        for item in &def.items {
            match item {
                CovergroupItem::Coverpoint(cp) => {
                    let val = self.eval_expr(&cp.expr);
                    let cp_name = cp.name.as_ref().map(|n| n.name.clone()).unwrap_or_else(|| format!("{:?}", cp.expr));
                    if let Some(Some(inst)) = self.cg_heap.get_mut(handle) {
                        inst.point_hits.entry(cp_name).or_default().insert(val);
                    }
                }
                CovergroupItem::Cross(cr) => {
                    let mut tuple = Vec::new();
                    for id in &cr.items {
                        // Resolve each item in cross
                        let name = id.name.clone();
                        let val = self.lookup_signal_value(&name).unwrap_or(Value::zero(1));
                        tuple.push(val);
                    }
                    let cr_name = cr.name.as_ref().map(|n| n.name.clone()).unwrap_or_else(|| cr.items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join("_"));
                    if let Some(Some(inst)) = self.cg_heap.get_mut(handle) {
                        inst.cross_hits.entry(cr_name).or_default().insert(tuple);
                    }
                }
                _ => {}
            }
        }
    }

    fn instantiate_class(&mut self, class_def: &crate::compiler::elaborate::ElaboratedClass, args: &[Expression]) -> Value {
        self.instantiate_class_with_type_args(class_def, args, None)
    }

    fn instantiate_class_with_type_args(
        &mut self,
        class_def: &crate::compiler::elaborate::ElaboratedClass,
        args: &[Expression],
        type_args: Option<&[Expression]>,
    ) -> Value {
        let handle = self.heap.len();
        let mut instance = ClassInstance {

            class_name: class_def.name.clone(),
            properties: HashMap::new(),
        };
        let mut classes_to_init = vec![class_def.clone()];
        let mut cur = class_def.extends.clone();
        while let Some(cname) = cur {
            if let Some(cdef) = self.module.classes.get(&cname) {
                classes_to_init.push(cdef.clone());
                cur = cdef.extends.clone();
            } else { break; }
        }
        for cdef in classes_to_init.iter().rev() {
            for (prop_name, prop_sig) in &cdef.properties {
                instance.properties.insert(prop_name.clone(), prop_sig.value.clone());
            }
            // Bind class parameters: each param gets its default value, then
            // any positional type_args (on the leaf class only) override.
            let is_leaf = cdef.name == class_def.name;
            for (i, (pname, pdefault)) in cdef.param_defaults.iter().enumerate() {
                let expr_opt: Option<Expression> = if is_leaf {
                    type_args.and_then(|ta| ta.get(i).cloned()).or_else(|| pdefault.clone())
                } else {
                    pdefault.clone()
                };
                if let Some(e) = expr_opt {
                    let v = self.eval_expr(&e);
                    instance.properties.insert(pname.clone(), v);
                }
            }
        }
        self.heap.push(Some(instance));
        self.exec_method_call(handle, "new", args);
        Value::from_u64(handle as u64, 32)
    }

    fn exec_method_call(&mut self, handle: usize, method_name: &str, args: &[Expression]) -> Value {
        if method_name == "randomize" {
            return self.exec_randomize(handle);
        }
        // Built-in mailbox / semaphore methods
        if self.mailboxes.contains_key(&handle) {
            match method_name {
                "put" => {
                    if let Some(arg) = args.first() {
                        let v = self.eval_expr(arg);
                        self.mailboxes.get_mut(&handle).unwrap().push_back(v);
                    }
                    return Value::zero(32);
                }
                "get" | "peek" => {
                    let val = if method_name == "get" {
                        self.mailboxes.get_mut(&handle).and_then(|q| q.pop_front())
                    } else {
                        self.mailboxes.get(&handle).and_then(|q| q.front().cloned())
                    };
                    if let (Some(v), Some(arg)) = (val, args.first()) {
                        let w = self.infer_lhs_width(arg);
                        self.assign_value(arg, &v.resize(w));
                    }
                    return Value::zero(32);
                }
                "try_put" => {
                    if let Some(arg) = args.first() {
                        let v = self.eval_expr(arg);
                        self.mailboxes.get_mut(&handle).unwrap().push_back(v);
                    }
                    return Value::from_u64(1, 32);
                }
                "try_get" | "try_peek" => {
                    let val = if method_name == "try_get" {
                        self.mailboxes.get_mut(&handle).and_then(|q| q.pop_front())
                    } else {
                        self.mailboxes.get(&handle).and_then(|q| q.front().cloned())
                    };
                    if let (Some(v), Some(arg)) = (val, args.first()) {
                        let w = self.infer_lhs_width(arg);
                        self.assign_value(arg, &v.resize(w));
                        return Value::from_u64(1, 32);
                    }
                    return Value::zero(32);
                }
                "num" => {
                    let n = self.mailboxes.get(&handle).map(|q| q.len()).unwrap_or(0);
                    return Value::from_u64(n as u64, 32);
                }
                _ => {}
            }
        }
        if self.semaphores.contains_key(&handle) {
            match method_name {
                "put" => {
                    let n = args.first().map(|a| self.eval_expr(a).to_u64().unwrap_or(1)).unwrap_or(1) as i64;
                    *self.semaphores.get_mut(&handle).unwrap() += n;
                    return Value::zero(32);
                }
                "get" => {
                    let n = args.first().map(|a| self.eval_expr(a).to_u64().unwrap_or(1)).unwrap_or(1) as i64;
                    let count = self.semaphores.get_mut(&handle).unwrap();
                    if *count >= n { *count -= n; }
                    return Value::zero(32);
                }
                "try_get" => {
                    let n = args.first().map(|a| self.eval_expr(a).to_u64().unwrap_or(1)).unwrap_or(1) as i64;
                    let count = self.semaphores.get_mut(&handle).unwrap();
                    if *count >= n { *count -= n; return Value::from_u64(1, 32); }
                    return Value::zero(32);
                }
                _ => {}
            }
        }
        let class_name = if let Some(Some(inst)) = self.heap.get(handle) {
            inst.class_name.clone()
        } else { return Value::zero(32); };
        self.exec_method_in_class_hierarchy(handle, &class_name, method_name, args)
    }

    fn spawn_method_task_process(&mut self, handle: usize, method_name: &str, args: &[Expression]) -> bool {
        let mut cur_class = if let Some(Some(inst)) = self.heap.get(handle) {
            Some(inst.class_name.clone())
        } else {
            None
        };
        while let Some(cname) = cur_class {
            if let Some(class_def) = self.module.classes.get(&cname).cloned() {
                if let Some(method) = class_def.methods.get(method_name) {
                    if let crate::ast::decl::ClassMethodKind::Task(t) = &method.kind {
                        let mut locals = HashMap::new();
                        for (i, port) in t.ports.iter().enumerate() {
                            let val = if i < args.len() { self.eval_expr(&args[i]) } else { Value::zero(32) };
                            locals.insert(port.name.name.clone(), val);
                        }
                        let pid = self.next_pid;
                        self.next_pid += 1;
                        self.process_contexts.insert(pid, ProcessContext {
                            this_stack: vec![Some(handle)],
                            local_stack: vec![locals],
                            class_context_stack: vec![Some(cname.clone())],
                            cg_this: self.cg_this,
                            return_value: None,
                            break_flag: false,
                            continue_flag: false,
                        });
                        self.event_queue.schedule(self.time, pid, t.items.clone());
                        return true;
                    }
                    return false;
                }
                cur_class = class_def.extends.clone();
            } else {
                break;
            }
        }
        false
    }

    fn exec_randomize(&mut self, handle: usize) -> Value {
        use rand::Rng;
        let class_name = if let Some(Some(inst)) = self.heap.get(handle) {
            inst.class_name.clone()
        } else { return Value::zero(32); };

        let mut rand_props = Vec::new();
        let mut constraints = Vec::new();

        let mut cur = Some(class_name.clone());
        while let Some(cname) = cur {
            if let Some(class_def) = self.module.classes.get(&cname) {
                for prop in &class_def.random_properties {
                    if let Some(sig) = class_def.properties.get(prop) {
                        rand_props.push((prop.clone(), sig.width));
                    }
                }
                for con in class_def.constraints.values() {
                    constraints.push(con.clone());
                }
                cur = class_def.extends.clone();
            } else { break; }
        }

        self.this_stack.push(Some(handle));
        for trial in 0..1000 {
            let mut solved_props: HashMap<String, Value> = HashMap::new();
            let mut backup = HashMap::new();
            
            // First pass: identify simple range constraints for each property
            let mut prop_allowed_ranges: HashMap<String, Vec<(u64, u64)>> = HashMap::new();
            for con in &constraints {
                for item in &con.items {
                    let (inside_expr, inside_ranges): (Option<&Expression>, Option<Vec<(u64, u64)>>) = match item {
                        ConstraintItem::Inside { expr, range, .. } => {
                            let mut ranges = Vec::new();
                            for r in range {
                                if let ConstraintRange::Range { lo, hi } = r {
                                    let l = self.eval_expr(lo).to_u64().unwrap_or(0);
                                    let h = self.eval_expr(hi).to_u64().unwrap_or(u64::MAX);
                                    ranges.push((l, h));
                                } else if let ConstraintRange::Value(v_expr) = r {
                                    let v = self.eval_expr(v_expr).to_u64().unwrap_or(0);
                                    ranges.push((v, v));
                                }
                            }
                            (Some(expr), Some(ranges))
                        }
                        ConstraintItem::Expr(expr) => {
                            if let ExprKind::Inside { expr: inner, ranges } = &expr.kind {
                                let mut parsed = Vec::new();
                                for r in ranges {
                                    match &r.kind {
                                        ExprKind::Range(lo, hi) => {
                                            let l = self.eval_expr(lo).to_u64().unwrap_or(0);
                                            let h = self.eval_expr(hi).to_u64().unwrap_or(u64::MAX);
                                            parsed.push((l, h));
                                        }
                                        _ => {
                                            let v = self.eval_expr(r).to_u64().unwrap_or(0);
                                            parsed.push((v, v));
                                        }
                                    }
                                }
                                (Some(inner.as_ref()), Some(parsed))
                            } else {
                                (None, None)
                            }
                        }
                        _ => (None, None),
                    };
                    if let (Some(expr), Some(ranges)) = (inside_expr, inside_ranges) {
                        if let ExprKind::Ident(hier) = &expr.kind {
                            let name = hier.path.last().unwrap().name.name.clone();
                            if !ranges.is_empty() {
                                prop_allowed_ranges.entry(name).or_insert_with(Vec::new).extend(ranges);
                            }
                        }
                    }
                }
            }

            // Second pass: identify equality "assignments"
            let mut assignments: HashMap<String, Expression> = HashMap::new();
            for con in &constraints {
                for item in &con.items {
                    if let ConstraintItem::Expr(expr) = item {
                        if let ExprKind::Binary { op: BinaryOp::Eq, left, right } = &expr.kind {
                            if let ExprKind::Ident(hier) = &left.kind {
                                let name = hier.path.last().unwrap().name.name.clone();
                                // check if right-hand side doesn't contain 'name' to avoid self-reference
                                assignments.insert(name, *right.clone());
                            } else if let ExprKind::Ident(hier) = &right.kind {
                                let name = hier.path.last().unwrap().name.name.clone();
                                assignments.insert(name, *left.clone());
                            }
                        }
                    }
                }
            }

            // Local copy of properties for solving
            let mut current_props = HashMap::new();
            if let Some(Some(inst)) = self.heap.get(handle) {
                for (name, val) in &inst.properties {
                    current_props.insert(name.clone(), val.clone());
                    backup.insert(name.clone(), val.clone());
                }
            }

            // Solve properties
            let mut pids_to_solve: Vec<usize> = (0..rand_props.len()).collect();
            let mut progress = true;
            while !pids_to_solve.is_empty() && progress {
                progress = false;
                let mut i = 0;
                while i < pids_to_solve.len() {
                    let pid_idx = pids_to_solve[i];
                    let (name, width) = &rand_props[pid_idx];
                    
                    // If it has an assignment, check if we can solve it
                    if let Some(expr) = assignments.get(name) {
                        let mut idents = HashSet::new();
                        self.collect_expr_idents(expr, &mut idents);
                        let ready = idents.iter().all(|id| {
                            !rand_props.iter().any(|(n, _)| n == id) || solved_props.contains_key(id)
                        });
                        
                        if ready {
                            // Temporary set props in instance for eval_expr if needed?
                            // No, eval_expr uses self.signals and self.heap[handle].properties.
                            // We need to update the instance properties during solving if eval_expr depends on them.
                            if let Some(Some(inst)) = self.heap.get_mut(handle) {
                                for (n, v) in &solved_props { inst.properties.insert(n.clone(), v.clone()); }
                            }
                            
                            self.this_stack.push(Some(handle));
                            let val = self.eval_expr(expr);
                            self.this_stack.pop();
                            
                            solved_props.insert(name.clone(), val);
                            pids_to_solve.remove(i);
                            progress = true;
                            continue;
                        }
                    } else {
                        // No assignment, pick randomly (honoring ranges if possible)
                        let mut val = Value::zero(*width);
                        if let Some(ranges) = prop_allowed_ranges.get(name) {
                            let r_idx = self.rng.gen_range(0..ranges.len());
                            let (lo, hi) = ranges[r_idx];
                            let r_val = if hi >= lo { self.rng.gen_range(lo..=hi) } else { lo };
                            val = Value::from_u64(r_val, *width);
                        } else {
                            if *width <= 64 {
                                let r: u64 = self.rng.gen();
                                val = Value::from_u64(r, *width);
                            }
                        }
                        solved_props.insert(name.clone(), val.clone());
                        if let Some(Some(inst)) = self.heap.get_mut(handle) {
                            inst.properties.insert(name.clone(), val);
                        }
                        pids_to_solve.remove(i);
                        progress = true;
                        continue;
                    }

                    i += 1;
                }
            }
            
            // Pick truly randomly for remaining
            for pid_idx in pids_to_solve {
                let (name, width) = &rand_props[pid_idx];
                let mut val = Value::zero(*width);
                if *width <= 64 { val = Value::from_u64(self.rng.gen(), *width); }
                solved_props.insert(name.clone(), val);
            }

            // Apply solved props to instance
            if let Some(Some(inst)) = self.heap.get_mut(handle) {
                for (name, val) in &solved_props {
                    inst.properties.insert(name.clone(), val.clone());
                }
            }

            let mut all_ok = true;
            for con in &constraints {
                for item in &con.items {
                    if !self.check_constraint_item(handle, item) {
                        all_ok = false;
                        break;
                    }
                }
                if !all_ok { break; }
            }

            if all_ok {
                self.this_stack.pop();
                return Value::from_u64(1, 32);
            }

            if let Some(Some(inst)) = self.heap.get_mut(handle) {
                for (name, val) in backup {
                    inst.properties.insert(name, val);
                }
            }
        }

        self.this_stack.pop();
        Value::zero(32)
    }
    fn collect_expr_idents(&self, expr: &Expression, idents: &mut HashSet<String>) {
        match &expr.kind {
            ExprKind::Ident(hier) => {
                if let Some(seg) = hier.path.last() {
                    idents.insert(seg.name.name.clone());
                }
            }
            ExprKind::Unary { operand, .. } => self.collect_expr_idents(operand, idents),
            ExprKind::Binary { left, right, .. } => {
                self.collect_expr_idents(left, idents);
                self.collect_expr_idents(right, idents);
            }
            ExprKind::Conditional { condition, then_expr, else_expr } => {
                self.collect_expr_idents(condition, idents);
                self.collect_expr_idents(then_expr, idents);
                self.collect_expr_idents(else_expr, idents);
            }
            ExprKind::Paren(inner) => self.collect_expr_idents(inner, idents),
            ExprKind::Concatenation(exprs) => {
                for e in exprs { self.collect_expr_idents(e, idents); }
            }
            ExprKind::Call { args, .. } => {
                for a in args { self.collect_expr_idents(a, idents); }
            }
            ExprKind::Index { expr, index } => {
                self.collect_expr_idents(expr, idents);
                self.collect_expr_idents(index, idents);
            }
            ExprKind::RangeSelect { expr, left, right, .. } => {
                self.collect_expr_idents(expr, idents);
                self.collect_expr_idents(left, idents);
                self.collect_expr_idents(right, idents);
            }
            ExprKind::MemberAccess { expr, .. } => {
                self.collect_expr_idents(expr, idents);
            }
            _ => {}
        }
    }

    fn check_constraint_item(&mut self, handle: usize, item: &ConstraintItem) -> bool {
        self.this_stack.push(Some(handle));
        let ok = self.check_constraint_item_impl(item);
        self.this_stack.pop();
        ok
    }

    fn check_constraint_item_impl(&mut self, item: &ConstraintItem) -> bool {
        match item {
            ConstraintItem::Expr(expr) => {
                let res = self.eval_expr(expr);
                res.is_true()
            }
            ConstraintItem::Inside { expr, range, .. } => {
                let val = self.eval_expr(expr);
                for r in range {
                    match r {
                        ConstraintRange::Value(e) => {
                            let v = self.eval_expr(e);
                            if val == v { return true; }
                        }
                        ConstraintRange::Range { lo, hi } => {
                            let l = self.eval_expr(lo);
                            let h = self.eval_expr(hi);
                            if val.greater_equal(&l).is_true() && val.less_equal(&h).is_true() { return true; }
                        }
                    }
                }
                false
            }
            ConstraintItem::Implication { condition, constraint, .. } => {
                if self.eval_expr(condition).is_true() {
                    self.check_constraint_item_impl(constraint)
                } else { true }
            }
            ConstraintItem::IfElse { condition, then_item, else_item, .. } => {
                if self.eval_expr(condition).is_true() {
                    self.check_constraint_item_impl(then_item)
                } else if let Some(ei) = else_item {
                    self.check_constraint_item_impl(ei)
                } else { true }
            }
            ConstraintItem::Block(items) => {
                for it in items { if !self.check_constraint_item_impl(it) { return false; } }
                true
            }
            _ => true,
        }
    }

    fn exec_super_method_call(&mut self, handle: usize, method_name: &str, args: &[Expression]) -> Value {
        let class_name = if let Some(Some(ctx)) = self.class_context_stack.last() {
            ctx.clone()
        } else {
            if let Some(Some(inst)) = self.heap.get(handle) {
                inst.class_name.clone()
            } else { return Value::zero(32); }
        };
        let parent_name = if let Some(class_def) = self.module.classes.get(&class_name) {
            class_def.extends.clone()
        } else { None };
        if let Some(pname) = parent_name {
            return self.exec_method_in_class_hierarchy(handle, &pname, method_name, args);
        }
        Value::zero(32)
    }

    fn exec_method_in_class_hierarchy(&mut self, handle: usize, start_class: &str, method_name: &str, args: &[Expression]) -> Value {
        use crate::ast::decl::ClassMethodKind;
        let mut cur_class = Some(start_class.to_string());
        while let Some(cname) = cur_class {
            if let Some(class_def) = self.module.classes.get(&cname).cloned() {
                if let Some(method) = class_def.methods.get(method_name) {
                    let mut locals = HashMap::new();
                    let (ports, body) = match &method.kind {
                        ClassMethodKind::Function(f) => (&f.ports, &f.items),
                        ClassMethodKind::Task(t) => (&t.ports, &t.items),
                        _ => { cur_class = class_def.extends.clone(); continue; }
                    };
                    for (i, port) in ports.iter().enumerate() {
                        let val = if i < args.len() { self.eval_expr(&args[i]) } else { Value::zero(32) };
                        locals.insert(port.name.name.clone(), val);
                    }
                    self.this_stack.push(Some(handle));
                    self.local_stack.push(locals);
                    self.class_context_stack.push(Some(cname.clone()));
                    for stmt in body { self.exec_statement(stmt); if self.break_flag { break; } }
                    self.break_flag = false;
                    self.class_context_stack.pop();
                    let ret = self.return_value.take().unwrap_or(Value::zero(32));
                    self.local_stack.pop();
                    self.this_stack.pop();
                    return ret;
                }
                cur_class = class_def.extends.clone();
            } else { break; }
        }
        Value::zero(32)
    }

    fn resolve_expr_name(&self, expr: &Expression) -> String {
        match &expr.kind {
            ExprKind::Ident(hier) => self.resolve_hier_name(hier),
            _ => "expr".to_string(),
        }
    }

    fn get_expr_type_name(&self, expr: &Expression) -> Option<String> {
        match &expr.kind {
            ExprKind::Ident(hier) => {
                let name = self.resolve_hier_name(hier);
                self.signal_name_to_id.get(name.as_str())
                    .and_then(|id| self.signal_type_names.get(id).cloned())
            }
            _ => None,
        }
    }
}
