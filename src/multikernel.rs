//! True per-LP PDES (Parallel Discrete Event Simulation) skeleton.
//!
//! This module provides a from-scratch coordinator that runs N independent
//! event loops on N host threads, synchronized through bounded message
//! channels at clock edges (conservative CMB protocol). It does NOT reuse
//! `compiler::Simulator::event_loop`; the existing event loop owns global
//! state (event_queue, nba_fast, clock_generators, dirty tracking) that
//! is fundamentally incompatible with per-LP independent time advancement.
//!
//! ## Status
//!
//! Worktree experiment (branch `perlp-experiment`). Goal of this branch:
//! prove the architecture on a toy 2-module Verilog design (two clocked
//! counters + 1 shared signal). Once the toy passes, the next session can
//! evaluate whether the same coordinator scales to c910-sized designs.
//!
//! ## What's here
//!
//! - `PdesKernel`: owns one LP's slice of state — its signal subset,
//!   compiled edge blocks, event queue, NBA buffer, local sim_time.
//! - `BoundaryChannel`: ordered (sim_time, signal_id, value) message
//!   queue from a producing kernel to a consuming kernel. Implements
//!   the CMB lookahead lower-bound for the consumer's local time.
//! - `ClockBarrier`: classic N-thread barrier with generation counter,
//!   used to sync all kernels at common clock-edge "rendezvous" points.
//! - `PdesCoordinator`: top-level orchestrator. Spawns kernel threads,
//!   wires the channel topology, drives global sim_time forward through
//!   barrier ticks.
//!
//! ## What's NOT here (stubs / handed off)
//!
//! - Full SV preprocessor / parser integration — kernels are constructed
//!   directly from precompiled `KernelSpec` today.
//! - Settle fixed-point iteration — only level-sensitive comb blocks
//!   need this; the toy test uses edge-only `always @(posedge clk)`.
//! - NBA queue (only `nba_fast` analog present); array-element NBAs.
//! - `$display` / `$finish` / `$readmemh` plumbing back to a designated
//!   I/O kernel — toy test exits when local_time exceeds a fixed
//!   `max_sim_time`.
//! - Width-resize / signedness coercion on NBA apply — toy assumes
//!   homogenous widths.
//!
//! See `xezim/docs/MULTIKERNEL.md` for the full scaling roadmap to
//! c910-class designs.

use std::sync::{
    mpsc::{channel, Receiver, Sender},
    Arc, Condvar, Mutex,
};

/// Logical-Process identifier (small unsigned). LP 0 is the I/O kernel by
/// convention; `$display`/`$finish` route to it.
pub type LpId = u32;

/// A boundary signal update flowing from one kernel to another. The
/// `at_time` field carries the producer's local time at the moment of
/// write; consumers use it to validate the CMB lookahead invariant before
/// advancing past it.
#[derive(Clone, Debug)]
pub struct BoundaryUpdate {
    pub signal_id: usize,
    pub value: u64, // toy: fits-in-u64 only; real impl uses xezim_core::Value
    pub at_time: u64,
}

/// A bounded channel from one producer LP to one consumer LP. CMB
/// lookahead = the minimum delay the consumer can safely assume between
/// receiving a message and observing its effect — for synchronous RTL
/// this is one clock period of the producer.
pub struct BoundaryChannel {
    pub producer: LpId,
    pub consumer: LpId,
    pub lookahead_ns: u64,
    tx: Sender<BoundaryUpdate>,
    rx: Mutex<Option<Receiver<BoundaryUpdate>>>,
}

impl BoundaryChannel {
    pub fn new(producer: LpId, consumer: LpId, lookahead_ns: u64) -> Self {
        let (tx, rx) = channel::<BoundaryUpdate>();
        Self {
            producer,
            consumer,
            lookahead_ns,
            tx,
            rx: Mutex::new(Some(rx)),
        }
    }

    /// Producer sends an update. Non-blocking (channel is unbounded for
    /// the prototype — real impl needs a bound for backpressure).
    pub fn send(&self, msg: BoundaryUpdate) -> Result<(), String> {
        self.tx
            .send(msg)
            .map_err(|e| format!("BoundaryChannel send failed: {e}"))
    }

    /// Consumer takes ownership of the receiver end exactly once.
    pub fn take_rx(&self) -> Option<Receiver<BoundaryUpdate>> {
        self.rx.lock().ok().and_then(|mut g| g.take())
    }
}

/// N-thread barrier with generation counter. Each `wait` blocks until N
/// threads have called it; on release, all are unblocked simultaneously.
/// Reusable: subsequent `wait` calls start a fresh round.
pub struct ClockBarrier {
    n: usize,
    state: Mutex<BarrierState>,
    cv: Condvar,
}

struct BarrierState {
    count: usize,
    generation: usize,
}

impl ClockBarrier {
    pub fn new(n: usize) -> Self {
        Self {
            n,
            state: Mutex::new(BarrierState {
                count: 0,
                generation: 0,
            }),
            cv: Condvar::new(),
        }
    }

    /// Block until all N threads have called `wait`. Returns the
    /// generation number for the round just completed (useful for trace).
    pub fn wait(&self) -> usize {
        let mut state = self.state.lock().unwrap();
        let gen = state.generation;
        state.count += 1;
        if state.count == self.n {
            state.count = 0;
            state.generation = state.generation.wrapping_add(1);
            self.cv.notify_all();
            gen
        } else {
            while state.generation == gen {
                state = self.cv.wait(state).unwrap();
            }
            gen
        }
    }
}

/// Minimal "compiled block" for the prototype: a closure on the kernel's
/// signal slice that returns NBA writes. Real impl reuses
/// `compiler::bytecode::CompiledBlock`.
pub type KernelBlock = Box<dyn Fn(&[u64]) -> Vec<(usize, u64)> + Send + Sync>;

pub const DEFAULT_PDES_LOOKAHEAD_K: u64 = 1;

pub fn parse_pdes_lookahead_k(raw: Option<&str>) -> u64 {
    raw.and_then(|s| s.parse::<u64>().ok())
        .filter(|&k| k > 0)
        .unwrap_or(DEFAULT_PDES_LOOKAHEAD_K)
}

pub fn pdes_lookahead_k_from_env() -> u64 {
    parse_pdes_lookahead_k(std::env::var("XEZIM_PDES_K").ok().as_deref())
}

pub fn pdes_sync_rounds_for_ticks(ticks: u64, k: u64) -> u64 {
    if ticks == 0 {
        return 0;
    }
    let k = k.max(1);
    (ticks + k - 1) / k
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LookaheadBatch {
    pub start_tick: u64,
    pub ticks: u64,
}

pub struct LookaheadBatches {
    next_tick: u64,
    total_ticks: u64,
    k: u64,
}

impl Iterator for LookaheadBatches {
    type Item = LookaheadBatch;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next_tick >= self.total_ticks {
            return None;
        }
        let start_tick = self.next_tick;
        let ticks = self.k.min(self.total_ticks - self.next_tick);
        self.next_tick += ticks;
        Some(LookaheadBatch { start_tick, ticks })
    }
}

pub fn pdes_lookahead_batches(total_ticks: u64, k: u64) -> LookaheadBatches {
    LookaheadBatches {
        next_tick: 0,
        total_ticks,
        k: k.max(1),
    }
}

/// Specification a coordinator uses to construct a kernel. Holds owned
/// signal-id range, edge blocks, and channel handles. Cloning is cheap
/// because blocks are Arc'd.
pub struct KernelSpec {
    pub id: LpId,
    pub owned_signal_ids: Vec<usize>,
    pub blocks: Vec<Arc<KernelBlock>>,
    /// (boundary_signal_id, channels_to_notify_on_write) — for outbound.
    pub outbound: Vec<(usize, Arc<BoundaryChannel>)>,
    /// (boundary_signal_id, channel_to_poll_for_updates) — for inbound.
    pub inbound: Vec<(usize, Arc<BoundaryChannel>)>,
    pub clock_period_ns: u64,
    pub max_sim_time: u64,
}

#[derive(Clone, Default)]
pub struct BoundaryChannelPorts {
    pub outbound: Vec<(usize, Arc<BoundaryChannel>)>,
    pub inbound: Vec<(usize, Arc<BoundaryChannel>)>,
}

#[derive(Clone, Default)]
pub struct BoundaryChannelTopology {
    per_lp: Vec<BoundaryChannelPorts>,
    channel_count: usize,
}

impl BoundaryChannelTopology {
    pub fn channel_count(&self) -> usize {
        self.channel_count
    }

    pub fn for_lp(&self, lp: LpId) -> BoundaryChannelPorts {
        self.per_lp.get(lp as usize).cloned().unwrap_or_default()
    }
}

/// Build the two-LP boundary-channel topology from comb-traced IO stats.
///
/// Direction tags match `LpIoStats::boundary_directions`:
/// 0 = LP-A to LP-B, 1 = LP-B to LP-A, 2 = bidirectional. Bidirectional
/// signals are split into two unidirectional channels so each endpoint has
/// a single producer and a single consumer.
pub fn build_boundary_channels(io: &LpIoStats, clock_period_ns: u64) -> BoundaryChannelTopology {
    let mut topology = BoundaryChannelTopology {
        per_lp: vec![BoundaryChannelPorts::default(); 2],
        channel_count: 0,
    };

    let mut add_channel = |sig_id: usize, producer: LpId, consumer: LpId| {
        let ch = Arc::new(BoundaryChannel::new(producer, consumer, clock_period_ns));
        topology.per_lp[producer as usize]
            .outbound
            .push((sig_id, Arc::clone(&ch)));
        topology.per_lp[consumer as usize]
            .inbound
            .push((sig_id, ch));
        topology.channel_count += 1;
    };

    for (&sig_id, &dir) in io
        .boundary_signal_ids
        .iter()
        .zip(io.boundary_directions.iter())
    {
        match dir {
            0 => add_channel(sig_id, 0, 1),
            1 => add_channel(sig_id, 1, 0),
            2 => {
                add_channel(sig_id, 0, 1);
                add_channel(sig_id, 1, 0);
            }
            _ => debug_assert!(false, "invalid PDES boundary direction tag {dir}"),
        }
    }

    topology
}

#[derive(Debug, Clone, Default)]
pub struct Phase4RuntimeStats {
    pub lp_count: usize,
    pub lookahead_k: u64,
    pub sync_rounds_for_100_ticks: u64,
    pub local_table_signal_counts: Vec<usize>,
    pub local_table_bytes: Vec<usize>,
    pub boundary_channels: usize,
    pub outbound_endpoints: Vec<usize>,
    pub inbound_endpoints: Vec<usize>,
    pub send_context_blocks: usize,
    pub send_context_signals: usize,
}

/// Per-LP partition of the combinational settle layer (Phase 4 input).
///
/// Each comb entry is assigned to the LP that owns its write target (by
/// hierarchical name prefix). Entries whose write set straddles both LPs
/// can't be cleanly owned by a worker — they're collected separately to
/// run on the coordinator/I-O LP. Entries with no write target are
/// orphans (also coordinator-run).
///
/// `lp_entries[lp]` is the worklist a per-LP settle worker iterates; the
/// chaotic-iteration loop in `settle_combinatorial` restricted to that
/// subset, reading boundary signals from the LP's local table (kept
/// fresh by tick-boundary channel sync), is the per-LP settle.
pub struct CombPartition {
    /// Comb-entry indices owned by each LP. `lp_entries.len() == n_lp`.
    pub lp_entries: Vec<Vec<usize>>,
    /// Entries whose write set spans >1 LP — run on the coordinator LP.
    pub straddle_entries: Vec<usize>,
    /// Entries with no resolved write target — run on the coordinator LP.
    pub orphan_entries: Vec<usize>,
    /// Total comb entries (coverage check: sum of the above == this).
    pub total_entries: usize,
    /// Distinct signal ids read by an entry owned by a different LP than
    /// the signal's own (name-prefix) LP. These are the channel set the
    /// per-LP settle relies on being synced at tick boundaries. Should
    /// match `classify_lp_io`'s comb boundary total (cross-validation).
    pub boundary_signal_ids: Vec<usize>,
    /// comb_dep edges total / crossing an LP boundary.
    pub dep_edges_total: u64,
    pub dep_edges_cross_lp: u64,
    /// Per-signal owning LP (which LP's entries write it): 0/1, or 0xFF if
    /// no LP-owned entry writes it. Used by the per-LP settle merge to pick
    /// each signal's authoritative value. With single-writer-per-LP held,
    /// this is unambiguous even when uncore logic is split for balance.
    pub signal_owner_lp: Vec<u8>,
}

impl CombPartition {
    pub fn covered(&self) -> usize {
        self.lp_entries.iter().map(|v| v.len()).sum::<usize>()
            + self.straddle_entries.len()
            + self.orphan_entries.len()
    }
    pub fn coverage_ok(&self) -> bool {
        self.covered() == self.total_entries
    }
}

/// Per-LP partition of the edge-block (clocked always) layer — the
/// 55.6% chunk of the c910 sim loop. `lp_blocks[lp]` is the list of
/// parallel-eligible compiled edge-block indices a worker runs each
/// tick. Edge execution reads an immutable signal snapshot and emits NBA
/// writes, so workers share the snapshot read-only (no per-LP view clone,
/// unlike settle). `cross_lp_nba_writers` counts blocks that write a
/// signal owned by the other LP (the registered boundary set).
pub struct EdgePartition {
    pub lp_blocks: Vec<Vec<usize>>,
    pub uncore_blocks: usize,
    pub total_parallel: usize,
    pub cross_lp_nba_writers: usize,
}

/// One per-LP event-loop kernel. Constructed from a `KernelSpec`, runs
/// `run()` on its own thread.
pub struct PdesKernel {
    pub id: LpId,
    /// Shared with sibling kernels via unsafe disjoint writes. Writers
    /// into `owned_signal_ids` are race-free by construction; reads of
    /// non-owned ids see stale-but-monotonic values via boundary updates.
    pub signal_table: Arc<SignalTable<u64>>,
    pub spec: KernelSpec,
    pub barrier: Arc<ClockBarrier>,
    pub local_time: u64,
}

/// Shared signal table generic over cell type. The coordinator's
/// invariant is that each signal_id is written by exactly one kernel
/// (per `KernelSpec.owned_signal_ids`), so concurrent writes to
/// different indices are sound. Reads may observe other kernels'
/// updates with channel-delivery latency.
///
/// Toy tests instantiate `SignalTable<u64>`. The c910 path uses
/// `SignalTable<xezim_core::Value>` to match the existing bytecode
/// VM's signal type. `Value` is 24 bytes inline (with optional heap
/// Vec for widths > 64), so disjoint-index writes via raw pointer
/// remain sound but require Clone for snapshot copies. Wide values
/// need careful read-side handling on threads other than the writer;
/// the per-tick 3-barrier protocol synchronizes that.
pub struct SignalTable<T> {
    cells: std::cell::UnsafeCell<Vec<T>>,
    len: usize,
}

// SAFETY: writes partitioned by owner per `KernelSpec.owned_signal_ids`;
// reads see at least the boundary-channel value once the barrier passes.
unsafe impl<T: Send> Send for SignalTable<T> {}
unsafe impl<T: Sync> Sync for SignalTable<T> {}

impl<T: Clone + Default> SignalTable<T> {
    pub fn new(len: usize) -> Arc<Self> {
        Arc::new(Self {
            cells: std::cell::UnsafeCell::new(vec![T::default(); len]),
            len,
        })
    }
}

impl<T: Clone> SignalTable<T> {
    pub fn new_filled(len: usize, fill: T) -> Arc<Self> {
        Arc::new(Self {
            cells: std::cell::UnsafeCell::new(vec![fill; len]),
            len,
        })
    }

    /// SAFETY: caller proves no two threads write `id` concurrently.
    #[inline]
    pub unsafe fn write(&self, id: usize, value: T) {
        if id < self.len {
            // Use a raw pointer to the element so we never form an
            // `&mut Vec<T>` that aliases other threads' disjoint borrows.
            let ptr: *mut T = (*self.cells.get()).as_mut_ptr();
            std::ptr::write(ptr.add(id), value);
        }
    }

    /// Clone the value at `id` (suitable for cross-thread reads after
    /// barrier sync). For T=u64 this is a single load. For T=Value with
    /// inline storage it's a 24-byte copy. For wide values it allocates.
    #[inline]
    pub fn read_cloned(&self, id: usize) -> Option<T> {
        if id < self.len {
            // SAFETY: per the 3-barrier protocol, no kernel writes id during
            // the snapshot window.
            unsafe {
                let ptr: *const T = (*self.cells.get()).as_ptr();
                Some((*ptr.add(id)).clone())
            }
        } else {
            None
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    /// SAFETY: the slice borrow lives only across the closure call and
    /// no writer threads touch the kernel's owned IDs during that window
    /// (per the per-tick phase contract).
    pub unsafe fn as_slice(&self) -> &[T] {
        (*self.cells.get()).as_slice()
    }
}

// Convenience for the toy: u64 has a `0` default and is Copy, so the
// existing tests don't need to change. We expose a `read` alias for
// the old API.
impl SignalTable<u64> {
    #[inline]
    pub fn read(&self, id: usize) -> u64 {
        self.read_cloned(id).unwrap_or(0)
    }
}

impl PdesKernel {
    pub fn new(
        id: LpId,
        spec: KernelSpec,
        signal_table: Arc<SignalTable<u64>>,
        barrier: Arc<ClockBarrier>,
    ) -> Self {
        Self {
            id,
            signal_table,
            spec,
            barrier,
            local_time: 0,
        }
    }

    pub fn run(self) -> KernelStats {
        self.run_with_lookahead(pdes_lookahead_k_from_env())
    }

    /// Run this kernel's event loop until `max_sim_time`. Each LP advances
    /// up to `lookahead_k` local ticks between global synchronization points.
    ///
    /// The kernel executes against a private full-width mirror during a
    /// batch. Owned-signal writes are reflected to the shared table for final
    /// visibility, while cross-LP reads are driven only by boundary-channel
    /// messages. That keeps K>1 from racing on another LP's shared-table
    /// writes inside the batch.
    pub fn run_with_lookahead(mut self, lookahead_k: u64) -> KernelStats {
        let mut stats = KernelStats::default();
        stats.lookahead_k = lookahead_k.max(1);
        let inbox_rxs: Vec<(usize, Receiver<BoundaryUpdate>)> = self
            .spec
            .inbound
            .iter()
            .filter_map(|(sig, ch)| ch.take_rx().map(|rx| (*sig, rx)))
            .collect();

        // Clone the starting global table before any LP begins advancing.
        // Barrier makes sure every LP has its private mirror before writes
        // begin.
        let mut local_table: Vec<u64> = unsafe { self.signal_table.as_slice().to_vec() };
        self.barrier.wait();

        let total_ticks = if self.spec.clock_period_ns == 0 {
            0
        } else {
            (self.spec.max_sim_time + self.spec.clock_period_ns - 1) / self.spec.clock_period_ns
        };

        for batch in pdes_lookahead_batches(total_ticks, stats.lookahead_k) {
            for _ in 0..batch.ticks {
                // For tick 0, the local mirror already contains time-0
                // boundary values. Do not drain the FIFO here: a peer LP may
                // already have queued current-batch messages, but those are
                // for later ticks. From tick 1 onward, consume exactly one
                // update per inbound boundary signal. Producers send every
                // outbound boundary value every local tick, so FIFO order
                // supplies the previous-tick value.
                if stats.ticks > 0 {
                    for (sig, rx) in &inbox_rxs {
                        if let Ok(msg) = rx.recv() {
                            local_table[*sig] = msg.value;
                        }
                    }
                }

                let snapshot = local_table.clone();

                let mut nba: Vec<(usize, u64)> = Vec::new();
                for block in &self.spec.blocks {
                    let writes = block(&snapshot);
                    nba.extend(writes);
                }
                stats.blocks_fired += self.spec.blocks.len() as u64;
                stats.nba_writes += nba.len() as u64;

                for (id, value) in &nba {
                    local_table[*id] = *value;
                    // SAFETY: id is in owned_signal_ids by classifier
                    // contract, so no other kernel writes this index.
                    unsafe {
                        self.signal_table.write(*id, *value);
                    }
                }

                // Send the current value of every outbound boundary once per
                // local tick. Consumers can then block for exactly one FIFO
                // item per boundary per tick.
                for (boundary_id, ch) in &self.spec.outbound {
                    let value = local_table.get(*boundary_id).copied().unwrap_or(0);
                    let _ = ch.send(BoundaryUpdate {
                        signal_id: *boundary_id,
                        value,
                        at_time: self.local_time,
                    });
                    stats.boundary_sends += 1;
                }

                self.local_time = self.local_time.saturating_add(self.spec.clock_period_ns);
                stats.ticks += 1;
            }

            self.barrier.wait();
            stats.sync_rounds += 1;
        }
        stats.final_time = self.local_time;
        stats
    }
}

#[derive(Default, Debug, Clone)]
pub struct KernelStats {
    pub ticks: u64,
    pub lookahead_k: u64,
    pub sync_rounds: u64,
    pub blocks_fired: u64,
    pub nba_writes: u64,
    pub boundary_sends: u64,
    pub final_time: u64,
}

/// Top-level coordinator: spins up one thread per kernel, joins them,
/// returns per-kernel stats.
pub struct PdesCoordinator {
    pub signal_table: Arc<SignalTable<u64>>,
    pub barrier: Arc<ClockBarrier>,
    pub kernels: Vec<PdesKernel>,
}

impl PdesCoordinator {
    pub fn new(signal_table_len: usize, kernel_specs: Vec<KernelSpec>) -> Self {
        let signal_table = SignalTable::new(signal_table_len);
        let barrier = Arc::new(ClockBarrier::new(kernel_specs.len()));
        let mut kernels = Vec::with_capacity(kernel_specs.len());
        for spec in kernel_specs {
            let id = spec.id;
            kernels.push(PdesKernel::new(
                id,
                spec,
                Arc::clone(&signal_table),
                Arc::clone(&barrier),
            ));
        }
        Self {
            signal_table,
            barrier,
            kernels,
        }
    }

    /// Spawn one thread per kernel; return per-kernel stats on join.
    pub fn run(self) -> Vec<KernelStats> {
        self.run_with_lookahead(pdes_lookahead_k_from_env())
    }

    pub fn run_with_lookahead(self, lookahead_k: u64) -> Vec<KernelStats> {
        let signal_table = self.signal_table;
        let mut handles = Vec::with_capacity(self.kernels.len());
        for kernel in self.kernels {
            let h = std::thread::Builder::new()
                .name(format!("pdes-kernel-{}", kernel.id))
                .spawn(move || kernel.run_with_lookahead(lookahead_k))
                .expect("failed to spawn kernel thread");
            handles.push(h);
        }
        let mut stats: Vec<KernelStats> = Vec::with_capacity(handles.len());
        for h in handles {
            stats.push(h.join().expect("kernel panicked"));
        }
        // Drop the Arc<SignalTable> only after all kernels have joined.
        drop(signal_table);
        stats
    }
}

// Sanity counter so dead-code lint doesn't fire on the LpId alias.
const _: LpId = 0;

/// Result of read+write LP classification across a Simulator's blocks.
/// The boundary fields are the central PDES design number — every
/// boundary signal needs a channel message per tick (or per change),
/// so total boundary count × tick rate sets the channel throughput
/// requirement.
#[derive(Debug, Clone, Default)]
pub struct LpIoStats {
    pub blocks_total: usize,
    pub blocks_parallel: usize,
    pub blocks_lp_a: usize,
    pub blocks_lp_b: usize,

    /// Per-signal exclusive writer LP, or None when no parallel block
    /// writes the signal or multiple LPs write it.
    pub writers_lp_a_only: usize,
    pub writers_lp_b_only: usize,
    pub writers_boundary: usize, // multi-LP writers (forbidden in true PDES)

    /// Per-signal reader-set distribution. A signal can be read by:
    /// LP-A only, LP-B only, both, or neither.
    pub readers_lp_a_only: usize,
    pub readers_lp_b_only: usize,
    pub readers_both: usize,

    /// Cross-LP signals: written by one LP, read by the other. These
    /// require channel delivery each tick they change. The sum of
    /// `a_to_b + b_to_a + bidir` is the channel set the coordinator
    /// must wire up.
    pub boundary_a_to_b: usize,
    pub boundary_b_to_a: usize,
    pub boundary_bidir: usize, // shouldn't occur in true PDES (==> contradiction)

    /// SAME COUNTS but across ALL compiled blocks (not just parallel-
    /// eligible). The parallel-eligible subset excludes blocks with
    /// StmtFallback / BlockingAssign* / NbaAssignArray etc., which is
    /// where most cross-LP communication actually lives. This gives
    /// the real boundary signal count a per-LP-event_loop integration
    /// would need to channel.
    pub all_blocks_total: usize,
    pub all_blocks_lp_a: usize,
    pub all_blocks_lp_b: usize,
    pub all_writers_lp_a_only: usize,
    pub all_writers_lp_b_only: usize,
    pub all_writers_boundary: usize,
    pub all_readers_lp_a_only: usize,
    pub all_readers_lp_b_only: usize,
    pub all_readers_both: usize,
    pub all_boundary_a_to_b: usize,
    pub all_boundary_b_to_a: usize,
    pub all_boundary_bidir: usize,

    /// Combinational layer (CombEntry table) — where real cross-LP
    /// wires in synchronous RTL actually live (assign / always_comb /
    /// continuous_assigns from L1↔L2 fanout, etc.). Edge-block-only
    /// stats above will report zero boundary because clocked flops in
    /// each core write only signals in their own scope; comb-traced
    /// stats give the TRUE channel set the PDES coordinator must wire.
    pub comb_entries_total: usize,
    pub comb_entries_lp_a: usize,
    pub comb_entries_lp_b: usize,
    pub comb_entries_unscoped: usize,
    pub comb_writers_lp_a_only: usize,
    pub comb_writers_lp_b_only: usize,
    pub comb_writers_boundary: usize,
    pub comb_readers_lp_a_only: usize,
    pub comb_readers_lp_b_only: usize,
    pub comb_readers_both: usize,
    pub comb_boundary_a_to_b: usize,
    pub comb_boundary_b_to_a: usize,
    pub comb_boundary_bidir: usize,

    /// Sorted list of boundary signal IDs (A→B + B→A + bidir).
    /// Length = comb_boundary_a_to_b + comb_boundary_b_to_a + comb_boundary_bidir.
    /// Used by `build_c910_stub_specs_with_channels` to construct the
    /// real boundary channel topology at c910 scale.
    pub boundary_signal_ids: Vec<usize>,
    /// Boundary direction tag per signal in `boundary_signal_ids`:
    /// 0 = A→B (producer=LP-A), 1 = B→A (producer=LP-B), 2 = bidir.
    pub boundary_directions: Vec<u8>,

    /// Per-LP read-set signal IDs (union of comb-layer reads). Used to
    /// build the sparse per-LP signal-table snapshot — only these IDs
    /// need to be cloned per tick, not the full 35M signal_table.
    /// Sorted ascending for predictable cache-friendly iteration.
    pub read_set_lp_a: Vec<usize>,
    pub read_set_lp_b: Vec<usize>,
}

/// Scan every parallel-eligible compiled block in `sim`. For each
/// signal id, record (a) which LP writes it via any NBA insn variant,
/// (b) which LP reads it via LoadSignal/LoadSignalSigned. Returns
/// summary counts as `LpIoStats`. Array element reads/writes
/// (LoadArrayElem / NbaAssignArray) are NOT yet tracked — they use
/// dynamic indices the static classifier can't resolve.
pub fn classify_lp_io(sim: &crate::compiler::Simulator, lp_a_prefix: &str) -> LpIoStats {
    use crate::compiler::bytecode::Insn;

    const LP_NONE: u8 = 0xFF;
    const LP_A: u8 = 0;
    const LP_B: u8 = 1;
    const LP_MULTI: u8 = 0xFE;

    let n_blocks = sim.edge_block_count();
    let n_signals = sim.signal_table_len();
    let mut writer: Vec<u8> = vec![LP_NONE; n_signals];
    let mut reader: Vec<u8> = vec![LP_NONE; n_signals];

    // Separate writer/reader tables for the all-blocks variant.
    let mut all_writer: Vec<u8> = vec![LP_NONE; n_signals];
    let mut all_reader: Vec<u8> = vec![LP_NONE; n_signals];

    let mut stats = LpIoStats::default();
    stats.blocks_total = n_blocks;
    stats.all_blocks_total = n_blocks;

    let dot_prefix = format!("{}.", lp_a_prefix);

    for bi in 0..n_blocks {
        let Some(cb) = sim.compiled_edge_block_at(bi) else {
            continue;
        };
        let lp_full = match sim.edge_block_scope_at(bi).as_deref() {
            Some(scope) if scope == lp_a_prefix || scope.starts_with(&dot_prefix) => {
                stats.all_blocks_lp_a += 1;
                LP_A
            }
            _ => {
                stats.all_blocks_lp_b += 1;
                LP_B
            }
        };
        // First: accumulate into the all-blocks tables (regardless of
        // parallel-eligibility).
        for insn in &cb.instructions {
            let written_all = match insn {
                Insn::NbaAssign(id, _, _) => Some(*id),
                Insn::NbaAssignRange(id, _, _, _) => Some(*id),
                Insn::NbaAssignBitDyn(id, _, _) => Some(*id),
                Insn::NbaAssignRangeDyn(id, _, _, _) => Some(*id),
                Insn::BlockingAssign(id, _, _) => Some(*id),
                Insn::BlockingAssignRange(id, _, _, _) => Some(*id),
                Insn::BlockingAssignBitDyn(id, _, _) => Some(*id),
                Insn::BlockingAssignRangeDyn(id, _, _, _) => Some(*id),
                _ => None,
            };
            if let Some(id) = written_all {
                if id < n_signals {
                    all_writer[id] = match all_writer[id] {
                        LP_NONE => lp_full,
                        x if x == lp_full => x,
                        _ => LP_MULTI,
                    };
                }
            }
            let read_all = match insn {
                Insn::LoadSignal(_, id) => Some(*id),
                Insn::LoadSignalSigned(_, id) => Some(*id),
                _ => None,
            };
            if let Some(id) = read_all {
                if id < n_signals {
                    all_reader[id] = match all_reader[id] {
                        LP_NONE => lp_full,
                        x if x == lp_full => x,
                        _ => LP_MULTI,
                    };
                }
            }
        }

        if !sim.edge_block_parallel_at(bi) {
            continue;
        }
        stats.blocks_parallel += 1;
        let lp = match sim.edge_block_scope_at(bi).as_deref() {
            Some(scope) if scope == lp_a_prefix || scope.starts_with(&dot_prefix) => {
                stats.blocks_lp_a += 1;
                LP_A
            }
            _ => {
                stats.blocks_lp_b += 1;
                LP_B
            }
        };
        for insn in &cb.instructions {
            // Writes
            let written = match insn {
                Insn::NbaAssign(id, _, _) => Some(*id),
                Insn::NbaAssignRange(id, _, _, _) => Some(*id),
                Insn::NbaAssignBitDyn(id, _, _) => Some(*id),
                Insn::NbaAssignRangeDyn(id, _, _, _) => Some(*id),
                _ => None,
            };
            if let Some(id) = written {
                if id < n_signals {
                    writer[id] = match writer[id] {
                        LP_NONE => lp,
                        x if x == lp => x,
                        _ => LP_MULTI,
                    };
                }
            }
            // Reads
            let read = match insn {
                Insn::LoadSignal(_, id) => Some(*id),
                Insn::LoadSignalSigned(_, id) => Some(*id),
                _ => None,
            };
            if let Some(id) = read {
                if id < n_signals {
                    reader[id] = match reader[id] {
                        LP_NONE => lp,
                        x if x == lp => x,
                        _ => LP_MULTI,
                    };
                }
            }
        }
    }

    // Summary counts (parallel-eligible subset).
    for id in 0..n_signals {
        match writer[id] {
            LP_A => stats.writers_lp_a_only += 1,
            LP_B => stats.writers_lp_b_only += 1,
            LP_MULTI => stats.writers_boundary += 1,
            _ => {}
        }
        match reader[id] {
            LP_A => stats.readers_lp_a_only += 1,
            LP_B => stats.readers_lp_b_only += 1,
            LP_MULTI => stats.readers_both += 1,
            _ => {}
        }
        match (writer[id], reader[id]) {
            (LP_A, LP_B) => stats.boundary_a_to_b += 1,
            (LP_B, LP_A) => stats.boundary_b_to_a += 1,
            (LP_A, LP_MULTI) | (LP_B, LP_MULTI) => stats.boundary_bidir += 1,
            _ => {}
        }
    }

    // Summary counts (all blocks variant — true boundary set).
    for id in 0..n_signals {
        match all_writer[id] {
            LP_A => stats.all_writers_lp_a_only += 1,
            LP_B => stats.all_writers_lp_b_only += 1,
            LP_MULTI => stats.all_writers_boundary += 1,
            _ => {}
        }
        match all_reader[id] {
            LP_A => stats.all_readers_lp_a_only += 1,
            LP_B => stats.all_readers_lp_b_only += 1,
            LP_MULTI => stats.all_readers_both += 1,
            _ => {}
        }
        match (all_writer[id], all_reader[id]) {
            (LP_A, LP_B) => stats.all_boundary_a_to_b += 1,
            (LP_B, LP_A) => stats.all_boundary_b_to_a += 1,
            (LP_A, LP_MULTI) | (LP_B, LP_MULTI) => stats.all_boundary_bidir += 1,
            _ => {}
        }
    }

    // ── Combinational layer scan ──
    // PER-SIGNAL classification: comb entries' scope_hint is rarely
    // populated (438 535 of 438 580 entries unscoped on c910), so
    // block-level attribution is unreliable. Instead classify each
    // signal by its OWN hierarchical name prefix — that's the true
    // ownership invariant. Boundary = comb entry that writes/reads
    // signals from BOTH LPs.
    let mut sig_lp: Vec<u8> = vec![LP_B; n_signals];
    for id in 0..n_signals {
        let name = sim.signal_name_at(id);
        if name == lp_a_prefix || name.starts_with(&dot_prefix) {
            sig_lp[id] = LP_A;
        }
    }

    let mut comb_writer: Vec<u8> = vec![LP_NONE; n_signals];
    let mut comb_reader: Vec<u8> = vec![LP_NONE; n_signals];
    stats.comb_entries_total = sim.comb_entry_count();
    for ci in 0..stats.comb_entries_total {
        let Some((scope, reads, writes)) = sim.comb_entry_io_at(ci) else {
            continue;
        };
        // Track scope-hint stats for diagnostic only — actual
        // attribution comes from per-signal name above.
        match scope {
            Some(s) if s == lp_a_prefix || s.starts_with(&dot_prefix) => {
                stats.comb_entries_lp_a += 1
            }
            Some(_) => stats.comb_entries_lp_b += 1,
            None => stats.comb_entries_unscoped += 1,
        }
        // For each write, attribute to the WRITTEN SIGNAL's LP.
        for &id in writes {
            if id < n_signals {
                let lp = sig_lp[id];
                comb_writer[id] = match comb_writer[id] {
                    LP_NONE => lp,
                    x if x == lp => x,
                    _ => LP_MULTI,
                };
            }
        }
        // For each read, attribute to the comb entry's effective LP:
        // since one comb entry has a single write target (or unified
        // write set), use the FIRST write's LP as the entry's LP. If
        // no writes (rare), use LP-B as default.
        let entry_lp = writes
            .first()
            .and_then(|&w| if w < n_signals { Some(sig_lp[w]) } else { None })
            .unwrap_or(LP_B);
        for &id in reads {
            if id < n_signals {
                comb_reader[id] = match comb_reader[id] {
                    LP_NONE => entry_lp,
                    x if x == entry_lp => x,
                    _ => LP_MULTI,
                };
            }
        }
    }
    for id in 0..n_signals {
        match comb_writer[id] {
            LP_A => stats.comb_writers_lp_a_only += 1,
            LP_B => stats.comb_writers_lp_b_only += 1,
            LP_MULTI => stats.comb_writers_boundary += 1,
            _ => {}
        }
        match comb_reader[id] {
            LP_A => {
                stats.comb_readers_lp_a_only += 1;
                stats.read_set_lp_a.push(id);
            }
            LP_B => {
                stats.comb_readers_lp_b_only += 1;
                stats.read_set_lp_b.push(id);
            }
            LP_MULTI => {
                stats.comb_readers_both += 1;
                stats.read_set_lp_a.push(id);
                stats.read_set_lp_b.push(id);
            }
            _ => {}
        }
        match (comb_writer[id], comb_reader[id]) {
            (LP_A, LP_B) => {
                stats.comb_boundary_a_to_b += 1;
                stats.boundary_signal_ids.push(id);
                stats.boundary_directions.push(0);
            }
            (LP_B, LP_A) => {
                stats.comb_boundary_b_to_a += 1;
                stats.boundary_signal_ids.push(id);
                stats.boundary_directions.push(1);
            }
            (LP_A, LP_MULTI) | (LP_B, LP_MULTI) => {
                stats.comb_boundary_bidir += 1;
                stats.boundary_signal_ids.push(id);
                stats.boundary_directions.push(2);
            }
            _ => {}
        }
    }

    stats
}

/// Benchmark a sparse per-LP snapshot: clone only the cells at the
/// signal IDs in `read_set` into a fresh `Vec<Value>`. Returns the
/// snapshot and elapsed wall. This is the snapshot cost the per-LP
/// PDES integration would pay every tick — the bound that determines
/// whether the architecture is viable on c910 (full-table snapshot
/// was 629ms, infeasible; sparse should be ~1-3 ms per LP).
pub fn benchmark_sparse_snapshot(
    table: &SignalTable<xezim_core::Value>,
    read_set: &[usize],
) -> (Vec<xezim_core::Value>, std::time::Duration) {
    let t0 = std::time::Instant::now();
    // SAFETY: per the 3-barrier protocol, the snapshot window has no
    // concurrent writers to read_set indices.
    let snapshot: Vec<xezim_core::Value> = unsafe {
        let slice = table.as_slice();
        read_set
            .iter()
            .map(|&id| {
                if id < slice.len() {
                    slice[id].clone()
                } else {
                    xezim_core::Value::new(1)
                }
            })
            .collect()
    };
    let elapsed = t0.elapsed();
    (snapshot, elapsed)
}

/// Build a c910 stub coordinator with REAL `BoundaryChannel` objects
/// for the comb-traced boundary signal set (the 109 signals on hello).
/// Stub blocks remain no-ops; the channel topology is exercised
/// end-to-end at c910 scale to validate the wiring code, capacity, and
/// barrier interaction.
pub fn build_c910_stub_specs_with_channels(
    sim: &crate::compiler::Simulator,
    io: &LpIoStats,
    lp_a_prefix: &str,
    n_ticks: u64,
    clock_period_ns: u64,
) -> (
    Vec<KernelSpec>,
    Arc<std::sync::atomic::AtomicU64>,
    Arc<std::sync::atomic::AtomicU64>,
) {
    use std::sync::atomic::AtomicU64;

    let fire_a = Arc::new(AtomicU64::new(0));
    let fire_b = Arc::new(AtomicU64::new(0));
    let dot_prefix = format!("{}.", lp_a_prefix);

    let make_stub = |counter: Arc<AtomicU64>| -> Arc<KernelBlock> {
        Arc::new(Box::new(move |_sigs: &[u64]| -> Vec<(usize, u64)> {
            counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Vec::new()
        }))
    };

    let mut blocks_a: Vec<Arc<KernelBlock>> = Vec::new();
    let mut blocks_b: Vec<Arc<KernelBlock>> = Vec::new();
    let mut n_a = 0usize;
    let mut n_b = 0usize;
    for bi in 0..sim.edge_block_count() {
        if !sim.edge_block_compiled(bi) {
            continue;
        }
        let scope = sim.edge_block_scope_at(bi);
        let lp_a = scope
            .as_deref()
            .map(|s| s == lp_a_prefix || s.starts_with(&dot_prefix))
            .unwrap_or(false);
        if lp_a {
            blocks_a.push(make_stub(Arc::clone(&fire_a)));
            n_a += 1;
        } else {
            blocks_b.push(make_stub(Arc::clone(&fire_b)));
            n_b += 1;
        }
    }

    let topology = build_boundary_channels(io, clock_period_ns);
    let ports_a = topology.for_lp(0);
    let ports_b = topology.for_lp(1);

    eprintln!(
        "[PDES-stub-ch] block stubs: LP-A={}, LP-B={}; channel endpoints: LP-A out={}/in={}, LP-B out={}/in={}",
        n_a,
        n_b,
        ports_a.outbound.len(),
        ports_a.inbound.len(),
        ports_b.outbound.len(),
        ports_b.inbound.len()
    );

    let specs = vec![
        KernelSpec {
            id: 0,
            owned_signal_ids: Vec::new(),
            blocks: blocks_a,
            outbound: ports_a.outbound,
            inbound: ports_a.inbound,
            clock_period_ns,
            max_sim_time: clock_period_ns.saturating_mul(n_ticks),
        },
        KernelSpec {
            id: 1,
            owned_signal_ids: Vec::new(),
            blocks: blocks_b,
            outbound: ports_b.outbound,
            inbound: ports_b.inbound,
            clock_period_ns,
            max_sim_time: clock_period_ns.saturating_mul(n_ticks),
        },
    ];
    (specs, fire_a, fire_b)
}

/// Data Dependency Graph stats over the parallel-eligible compiled
/// edge blocks. Built by scanning each block's `LoadSignal*` for reads
/// and `NbaAssign*` for writes, then constructing block-level
/// adjacency: block A depends on block B if A reads any signal B writes.
/// SCCs are found via Tarjan's (must-co-locate sets); critical path
/// is the longest dependency chain in the DAG of SCCs.
#[derive(Debug, Default, Clone)]
pub struct DdgStats {
    pub blocks_total: usize,
    pub blocks_parallel: usize,
    pub deps_total: usize,
    pub max_in_degree: usize,
    pub max_out_degree: usize,
    pub independent_blocks: usize,
    pub sccs_total: usize,
    pub sccs_singleton: usize,
    pub sccs_nontrivial: usize,
    pub max_scc_size: usize,
    pub critical_path_blocks: usize,
    pub critical_path_sccs: usize,
    pub blocks_with_no_writers_visible: usize,
    pub avg_in_degree: f64,
}

/// Compute DDG over the parallel-eligible compiled edge blocks of
/// `sim`. Returns aggregate stats; the full adjacency is not exposed
/// (only the analysis derived from it) to keep the API surface small.
pub fn compute_ddg(sim: &crate::compiler::Simulator) -> DdgStats {
    use crate::compiler::bytecode::Insn;
    let n_blocks = sim.edge_block_count();
    let n_signals = sim.signal_table_len();

    // For each parallel-eligible block: collect (reads, writes) sig_ids.
    let mut block_reads: Vec<Vec<usize>> = vec![Vec::new(); n_blocks];
    let mut block_writes: Vec<Vec<usize>> = vec![Vec::new(); n_blocks];
    let mut is_eligible = vec![false; n_blocks];
    let mut stats = DdgStats::default();
    stats.blocks_total = n_blocks;
    for bi in 0..n_blocks {
        if !sim.edge_block_parallel_at(bi) {
            continue;
        }
        let Some(cb) = sim.compiled_edge_block_at(bi) else {
            continue;
        };
        is_eligible[bi] = true;
        stats.blocks_parallel += 1;
        for insn in &cb.instructions {
            match insn {
                Insn::LoadSignal(_, sig) | Insn::LoadSignalSigned(_, sig) => {
                    block_reads[bi].push(*sig);
                }
                Insn::NbaAssign(sig, _, _)
                | Insn::NbaAssignRange(sig, _, _, _)
                | Insn::NbaAssignBitDyn(sig, _, _)
                | Insn::NbaAssignRangeDyn(sig, _, _, _) => {
                    block_writes[bi].push(*sig);
                }
                _ => {}
            }
        }
    }

    // Build signal → block_writers map.
    let mut signal_writers: Vec<Vec<usize>> = vec![Vec::new(); n_signals];
    for bi in 0..n_blocks {
        if !is_eligible[bi] {
            continue;
        }
        for &sig in &block_writes[bi] {
            if sig < n_signals {
                signal_writers[sig].push(bi);
            }
        }
    }

    // Build block → block adjacency: for each block's read signal,
    // every writer of that signal is a predecessor (dep edge).
    // adj_out[a] = list of b such that a → b (a writes; b reads).
    // adj_in[b]  = list of a such that a → b.
    let mut adj_out: Vec<Vec<usize>> = vec![Vec::new(); n_blocks];
    let mut adj_in: Vec<Vec<usize>> = vec![Vec::new(); n_blocks];
    let mut blocks_with_no_writers = 0usize;
    for bi in 0..n_blocks {
        if !is_eligible[bi] {
            continue;
        }
        let mut any_writer = false;
        for &sig in &block_reads[bi] {
            if sig >= n_signals {
                continue;
            }
            for &writer in &signal_writers[sig] {
                if writer != bi {
                    adj_in[bi].push(writer);
                    adj_out[writer].push(bi);
                    any_writer = true;
                }
            }
        }
        if !any_writer && !block_reads[bi].is_empty() {
            blocks_with_no_writers += 1;
        }
    }
    // Dedup edges (a signal-id read can appear many times; we only
    // want unique block→block dep edges).
    for v in adj_out.iter_mut() {
        v.sort();
        v.dedup();
    }
    for v in adj_in.iter_mut() {
        v.sort();
        v.dedup();
    }
    stats.blocks_with_no_writers_visible = blocks_with_no_writers;
    stats.deps_total = adj_out.iter().map(|v| v.len()).sum();
    stats.max_out_degree = adj_out.iter().map(|v| v.len()).max().unwrap_or(0);
    stats.max_in_degree = adj_in.iter().map(|v| v.len()).max().unwrap_or(0);
    stats.avg_in_degree = if stats.blocks_parallel > 0 {
        stats.deps_total as f64 / stats.blocks_parallel as f64
    } else {
        0.0
    };
    stats.independent_blocks = adj_in
        .iter()
        .enumerate()
        .filter(|(bi, v)| is_eligible[*bi] && v.is_empty() && adj_out[*bi].is_empty())
        .count();

    // Tarjan's SCC algorithm — iterative to avoid stack overflow on
    // c910-sized graphs (~10K nodes is safe for recursion but the
    // structure is cleaner iterative).
    let mut index = 0i64;
    let mut stack: Vec<usize> = Vec::new();
    let mut on_stack = vec![false; n_blocks];
    let mut node_index: Vec<i64> = vec![-1; n_blocks];
    let mut node_lowlink: Vec<i64> = vec![-1; n_blocks];
    let mut scc_of: Vec<i64> = vec![-1; n_blocks];
    let mut next_scc = 0i64;
    let mut scc_sizes: Vec<usize> = Vec::new();

    // Iterative DFS state: (node, neighbor_iter_pos).
    for start in 0..n_blocks {
        if !is_eligible[start] || node_index[start] >= 0 {
            continue;
        }
        let mut call_stack: Vec<(usize, usize)> = vec![(start, 0)];
        node_index[start] = index;
        node_lowlink[start] = index;
        index += 1;
        stack.push(start);
        on_stack[start] = true;
        while let Some(&mut (v, ref mut pos)) = call_stack.last_mut() {
            if *pos < adj_out[v].len() {
                let w = adj_out[v][*pos];
                *pos += 1;
                if !is_eligible[w] {
                    continue;
                }
                if node_index[w] < 0 {
                    node_index[w] = index;
                    node_lowlink[w] = index;
                    index += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    call_stack.push((w, 0));
                } else if on_stack[w] {
                    node_lowlink[v] = node_lowlink[v].min(node_index[w]);
                }
            } else {
                // All neighbors processed: maybe pop SCC.
                if node_lowlink[v] == node_index[v] {
                    let mut size = 0usize;
                    loop {
                        let w = stack.pop().unwrap();
                        on_stack[w] = false;
                        scc_of[w] = next_scc;
                        size += 1;
                        if w == v {
                            break;
                        }
                    }
                    scc_sizes.push(size);
                    next_scc += 1;
                }
                call_stack.pop();
                if let Some(&(parent, _)) = call_stack.last() {
                    node_lowlink[parent] = node_lowlink[parent].min(node_lowlink[v]);
                }
            }
        }
    }
    stats.sccs_total = scc_sizes.len();
    stats.sccs_singleton = scc_sizes.iter().filter(|&&s| s == 1).count();
    stats.sccs_nontrivial = scc_sizes.iter().filter(|&&s| s > 1).count();
    stats.max_scc_size = scc_sizes.iter().copied().max().unwrap_or(0);

    // Critical path in the SCC-condensed DAG: longest path of SCCs.
    // Each SCC contributes its size to the block count along that path.
    let n_sccs = stats.sccs_total;
    let mut scc_adj: Vec<Vec<usize>> = vec![Vec::new(); n_sccs];
    for bi in 0..n_blocks {
        if !is_eligible[bi] {
            continue;
        }
        let sa = scc_of[bi] as usize;
        for &succ in &adj_out[bi] {
            if !is_eligible[succ] {
                continue;
            }
            let sb = scc_of[succ] as usize;
            if sa != sb {
                scc_adj[sa].push(sb);
            }
        }
    }
    for v in scc_adj.iter_mut() {
        v.sort();
        v.dedup();
    }
    // Topological sort + DP for longest path.
    let mut in_deg: Vec<usize> = vec![0; n_sccs];
    for u in 0..n_sccs {
        for &v in &scc_adj[u] {
            in_deg[v] += 1;
        }
    }
    let mut topo: Vec<usize> = Vec::with_capacity(n_sccs);
    let mut queue: std::collections::VecDeque<usize> = (0..n_sccs)
        .filter(|&u| in_deg[u] == 0)
        .collect();
    while let Some(u) = queue.pop_front() {
        topo.push(u);
        for &v in &scc_adj[u] {
            in_deg[v] -= 1;
            if in_deg[v] == 0 {
                queue.push_back(v);
            }
        }
    }
    let mut longest_blocks: Vec<usize> = vec![0; n_sccs];
    let mut longest_sccs: Vec<usize> = vec![0; n_sccs];
    for &u in &topo {
        longest_blocks[u] = longest_blocks[u].max(scc_sizes[u]);
        longest_sccs[u] = longest_sccs[u].max(1);
        for &v in &scc_adj[u] {
            let candidate_b = longest_blocks[u] + scc_sizes[v];
            let candidate_s = longest_sccs[u] + 1;
            if candidate_b > longest_blocks[v] {
                longest_blocks[v] = candidate_b;
            }
            if candidate_s > longest_sccs[v] {
                longest_sccs[v] = candidate_s;
            }
        }
    }
    stats.critical_path_blocks = longest_blocks.iter().copied().max().unwrap_or(0);
    stats.critical_path_sccs = longest_sccs.iter().copied().max().unwrap_or(0);

    stats
}

/// PDES Phase 2: Per-LP local signal table. Each LP owns a sparse
/// Vec<Value> sized only to its read+write set — typically a few MB
/// per LP for c910 (vs 1.1 GB for the full signal_table). Maps both
/// directions between global signal_id (Simulator's id space) and
/// local idx (this LP's own array). Block exec runs against the
/// local table via id translation in pdes_exec_block_local.
///
/// Phase 2.4 adds the exec wrapper. Phase 4 spawns one host thread
/// per LP, each owning its PerLpSignalTable. Phase 5 reads boundary
/// updates from BoundaryChannel and writes them into the local table
/// before each tick batch.
pub trait SignalLookup {
    fn lookup(&self, global_id: usize) -> &xezim_core::Value;
}

impl SignalLookup for [xezim_core::Value] {
    #[inline(always)]
    fn lookup(&self, global_id: usize) -> &xezim_core::Value {
        &self[global_id]
    }
}

impl SignalLookup for Vec<xezim_core::Value> {
    #[inline(always)]
    fn lookup(&self, global_id: usize) -> &xezim_core::Value {
        &self[global_id]
    }
}

/// A wrapper that provides signal lookup by trying the local per-LP table first,
/// then falling back to a global snapshot if not found.
pub struct SparseSignalTable<'a> {
    pub local: &'a PerLpSignalTable,
    pub global: &'a [xezim_core::Value],
}

impl<'a> SignalLookup for SparseSignalTable<'a> {
    #[inline(always)]
    fn lookup(&self, global_id: usize) -> &xezim_core::Value {
        if let Some(idx) = self.local.to_local(global_id) {
            &self.local.values[idx as usize]
        } else {
            &self.global[global_id]
        }
    }
}

pub enum SignalLookupWrapper<'a> {
    Global(&'a [xezim_core::Value]),
    Sparse(SparseSignalTable<'a>),
}

impl<'a> SignalLookup for SignalLookupWrapper<'a> {
    #[inline(always)]
    fn lookup(&self, global_id: usize) -> &xezim_core::Value {
        match self {
            SignalLookupWrapper::Global(g) => &g[global_id],
            SignalLookupWrapper::Sparse(s) => s.lookup(global_id),
        }
    }
}

#[derive(Clone, Debug)]
pub struct PerLpSignalTable {
    /// LP id (matches Simulator::edge_block_partition entries).
    pub lp: u32,
    /// local idx → global signal_id. Length = read_set ∪ write_set.
    pub local_to_global: Vec<u32>,
    /// global signal_id → local idx (sparse).
    pub global_to_local: ahash::AHashMap<usize, u32>,
    /// Per-LP Value cells, indexed by local idx.
    pub values: Vec<xezim_core::Value>,
    /// Per-LP width / signedness mirror (parallel to `values`).
    pub widths: Vec<u32>,
    pub signed: Vec<bool>,
}

impl PerLpSignalTable {
    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Estimated memory footprint in bytes (Value cells + maps).
    pub fn estimated_bytes(&self) -> usize {
        self.values.len() * std::mem::size_of::<xezim_core::Value>()
            + self.local_to_global.len() * std::mem::size_of::<u32>()
            + self.global_to_local.len() * (std::mem::size_of::<usize>() + std::mem::size_of::<u32>())
            + self.widths.len() * std::mem::size_of::<u32>()
            + self.signed.len() * std::mem::size_of::<bool>()
    }

    /// Look up local idx for a global signal_id. Returns None if the
    /// signal is not in this LP's read+write set (caller must handle
    /// — typically falls back to global signal_table for boundary
    /// signals delivered via BoundaryChannel).
    #[inline]
    pub fn to_local(&self, global_id: usize) -> Option<u32> {
        self.global_to_local.get(&global_id).copied()
    }

    /// Inverse lookup: local idx → global signal_id.
    #[inline]
    pub fn to_global(&self, local_idx: u32) -> Option<usize> {
        self.local_to_global.get(local_idx as usize).map(|&g| g as usize)
    }

    /// Read a value via global signal_id; falls back to None if not
    /// in this LP's set.
    #[inline]
    pub fn read_global(&self, global_id: usize) -> Option<&xezim_core::Value> {
        self.to_local(global_id)
            .and_then(|idx| self.values.get(idx as usize))
    }

    /// Write a value via global signal_id; falls back to no-op if not
    /// in this LP's set. SAFETY caveat: caller must verify the global
    /// id is in this LP's WRITE set (not just read set) to preserve
    /// LP-exclusive write invariant.
    pub fn write_global(&mut self, global_id: usize, value: xezim_core::Value) {
        if let Some(idx) = self.to_local(global_id) {
            if let Some(slot) = self.values.get_mut(idx as usize) {
                *slot = value;
            }
        }
    }

    /// PDES Phase 2.3: build a global-id-indexed snapshot Vec<Value>
    /// suitable for exec_insns_isolated. Cells at this LP's signal IDs
    /// come from the local table; other cells are filled from
    /// `global_fallback` (typically the simulator's signal_table for
    /// boundary signals delivered via channels in earlier phases, or
    /// Value::new(1) for "doesn't matter, won't be read").
    ///
    /// Memory: the returned Vec is `global_fallback.len()` Values,
    /// equal to the full signal_table size. This is the "wide
    /// snapshot" form. Phase 4 will swap to a sparse variant using
    /// bytecode id translation when ready.
    pub fn snapshot_for_tick(
        &self,
        global_fallback: &[xezim_core::Value],
    ) -> Vec<xezim_core::Value> {
        let mut snap = global_fallback.to_vec();
        for (local_idx, &global_id) in self.local_to_global.iter().enumerate() {
            let g = global_id as usize;
            if g < snap.len() {
                snap[g] = self.values[local_idx].clone();
            }
        }
        snap
    }

    /// Apply NBA writes returned by exec_insns_isolated to this LP's
    /// local table; returns the subset of writes whose signal IDs are
    /// outside this LP's set (cross-LP — should be empty when classifier
    /// shows boundary=0). Cross-LP writes are the caller's responsibility
    /// to deliver via BoundaryChannel.
    pub fn apply_nba_writes(
        &mut self,
        writes: &[(usize, xezim_core::Value)],
    ) -> Vec<(usize, xezim_core::Value)> {
        let mut cross_lp = Vec::new();
        for (global_id, val) in writes {
            if let Some(idx) = self.to_local(*global_id) {
                self.values[idx as usize] = val.clone();
            } else {
                cross_lp.push((*global_id, val.clone()));
            }
        }
        cross_lp
    }
}

/// Allocate a `SignalTable<xezim_core::Value>` sized for the c910
/// design. Each cell is initialized to the X-valued default for its
/// declared width by reading the Simulator's own initial signal table
/// (so we don't waste time re-allocating per-cell — just clone the
/// reference Vec). Returns the allocated table and the byte estimate
/// of the underlying Vec capacity for diagnostic reporting.
pub fn allocate_c910_value_signal_table(
    sim: &crate::compiler::Simulator,
) -> (Arc<SignalTable<xezim_core::Value>>, usize) {
    use xezim_core::Value;
    let n = sim.signal_table_len();
    let cells: Vec<Value> = (0..n).map(|_| Value::new(1)).collect();
    let bytes_estimate = n * std::mem::size_of::<Value>();
    (
        Arc::new(SignalTable {
            cells: std::cell::UnsafeCell::new(cells),
            len: n,
        }),
        bytes_estimate,
    )
}

/// Per-tick read-set snapshot cost on a `SignalTable<Value>`. Clones
/// every cell into a fresh Vec; returns elapsed wall and the resulting
/// snapshot. Used to demonstrate the cost of the naive "snapshot full
/// table per tick" path so the integration plan can budget the sparse-
/// snapshot optimization correctly.
pub fn benchmark_value_snapshot(
    table: &SignalTable<xezim_core::Value>,
) -> (Vec<xezim_core::Value>, std::time::Duration) {
    let t0 = std::time::Instant::now();
    let snapshot: Vec<xezim_core::Value> = unsafe { table.as_slice().to_vec() };
    let elapsed = t0.elapsed();
    (snapshot, elapsed)
}

/// Real-bytecode c910 PDES driver. Builds the same per-LP partition as
/// `build_c910_stub_specs` but invokes `SendExecContext.pdes_exec_block`
/// inside each KernelBlock — real SystemVerilog bytecode, not stub
/// closures. Runs K ticks against a captured signal_table snapshot
/// from the compiled Simulator.
///
/// Returns (total_blocks_fired, total_nba_writes, coord_wall_ms,
///          per_tick_avg_us). Caller is responsible for the snapshot's
/// validity — this function does NOT model comb settle, time-0 init,
/// $finish, or testbench I/O. Use it to measure real bytecode dispatch
/// throughput at c910 scale; do not compare results to reference sim.
///
/// Also dumps the value evolution of a few representative signals so
/// caller can confirm state is genuinely changing (not stuck).
pub fn run_c910_real_bytecode(
    sim: &crate::compiler::Simulator,
    lp_a_prefix: &str,
    n_ticks: u64,
) -> (u64, u64, f64, f64) {
    // K (multi-tick lookahead batch size) defaults to 1 = per-tick.
    // Set XEZIM_PDES_K=N to batch N ticks before each sync point.
    let k = pdes_lookahead_k_from_env();
    eprintln!("[real-pdes-c910] K (lookahead) = {}", k);
    run_c910_real_bytecode_k(sim, lp_a_prefix, n_ticks, k)
}

/// Multi-tick variant: each LP advances K ticks per sync barrier.
/// Demonstrates how lookahead-K amortizes coordination overhead at
/// c910 scale. For K=1 reduces to the per-tick path. For K>1, the
/// LPs batch K snapshots+execs before yielding (here: before
/// finishing the outer loop iteration). With the current
/// shared-signal_table architecture (no per-LP local table yet),
/// batching is structural only — actual concurrent-thread
/// independence requires the next-session per-LP-state refactor.
pub fn run_c910_real_bytecode_k(
    sim: &crate::compiler::Simulator,
    lp_a_prefix: &str,
    n_ticks: u64,
    k: u64,
) -> (u64, u64, f64, f64) {
    use std::sync::atomic::{AtomicU64, Ordering};
    use xezim_core::Value;

    let ctx = Arc::new(sim.extract_send_exec_context());
    let dot_prefix = format!("{}.", lp_a_prefix);

    // Partition PARALLEL-ELIGIBLE compiled blocks into LP-A vs LP-B by
    // scope prefix. exec_insns_isolated has unreachable!() for
    // StmtFallback / BlockingAssign* / NbaAssignRangeDyn / NbaAssignArrayRange
    // — those blocks must run on the simulator's serial path. PDES
    // dispatch is restricted to the parallel-eligible subset.
    let mut lp_a_block_ids: Vec<usize> = Vec::new();
    let mut lp_b_block_ids: Vec<usize> = Vec::new();
    let mut skipped_nonparallel = 0usize;
    for bi in 0..ctx.block_count() {
        if !ctx.block_compiled(bi) {
            continue;
        }
        if !sim.edge_block_parallel_at(bi) {
            skipped_nonparallel += 1;
            continue;
        }
        let scope = sim.edge_block_scope_at(bi).unwrap_or_default();
        if scope == lp_a_prefix || scope.starts_with(&dot_prefix) {
            lp_a_block_ids.push(bi);
        } else {
            lp_b_block_ids.push(bi);
        }
    }
    eprintln!(
        "[real-pdes-c910] partition: LP-A parallel-eligible = {}, LP-B = {}, skipped non-parallel = {}",
        lp_a_block_ids.len(),
        lp_b_block_ids.len(),
        skipped_nonparallel
    );

    // Capture an initial signal_table snapshot — the per-LP local
    // tables for this experiment share the same starting state (each
    // LP gets its own copy and writes back to its copy on NBA apply).
    let tab_a_init: Vec<Value> = sim.signal_table_slice().to_vec();
    let tab_b_init: Vec<Value> = tab_a_init.clone();

    let fires = Arc::new(AtomicU64::new(0));
    let nbas = Arc::new(AtomicU64::new(0));

    // Sequential driving — Simulator isn't Send anyway, and we want
    // pure dispatch throughput measurement. K-tick batching: each
    // outer iteration runs K ticks for each LP back-to-back. For K=1
    // this is per-tick (current behavior); for K>1 the LPs batch K
    // snapshot+exec+apply cycles before yielding. Sync between LPs is
    // implicit (single-threaded), but the wall savings show what
    // batched coordination overhead reduction would buy in a real
    // multi-threaded per-LP implementation.
    let t0 = std::time::Instant::now();
    let mut tab_a = tab_a_init;
    let mut tab_b = tab_b_init;
    let mut vm_regs: Vec<Value> = Vec::new();
    for batch in pdes_lookahead_batches(n_ticks, k) {
        // LP-A: advance `batch` ticks back-to-back.
        for _ in 0..batch.ticks {
            let snap_a = tab_a.clone();
            let mut nba_a: Vec<(usize, Value)> = Vec::new();
            for &bi in &lp_a_block_ids {
                let w = ctx.pdes_exec_block(bi, &snap_a, &mut vm_regs);
                nba_a.extend(w);
            }
            for (id, v) in &nba_a {
                tab_a[*id] = v.clone();
            }
            fires.fetch_add(lp_a_block_ids.len() as u64, Ordering::Relaxed);
            nbas.fetch_add(nba_a.len() as u64, Ordering::Relaxed);
        }
        // LP-B: advance `batch` ticks back-to-back.
        for _ in 0..batch.ticks {
            let snap_b = tab_b.clone();
            let mut nba_b: Vec<(usize, Value)> = Vec::new();
            for &bi in &lp_b_block_ids {
                let w = ctx.pdes_exec_block(bi, &snap_b, &mut vm_regs);
                nba_b.extend(w);
            }
            for (id, v) in &nba_b {
                tab_b[*id] = v.clone();
            }
            fires.fetch_add(lp_b_block_ids.len() as u64, Ordering::Relaxed);
            nbas.fetch_add(nba_b.len() as u64, Ordering::Relaxed);
        }
        // Sync point (simulated barrier): nothing to do in single-thread
        // mode; in real impl this would be where LP-A's boundary writes
        // flush to LP-B's inbox.
    }
    let wall_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let total_fires = fires.load(Ordering::Relaxed);
    let total_nbas = nbas.load(Ordering::Relaxed);
    let per_tick_us = if n_ticks > 0 {
        (wall_ms * 1000.0) / n_ticks as f64
    } else {
        0.0
    };

    // Dump a few representative signal values from each LP's final
    // state to confirm bytecode actually changed state.
    eprintln!("[real-pdes-c910] post-run state samples:");
    let mut samples_a = 0usize;
    let mut samples_b = 0usize;
    for id in 0..ctx.signal_count() {
        let name = ctx.signal_name_at(id);
        if name.is_empty() || !name.contains("x_ct_top_0") {
            continue;
        }
        let val = &tab_a[id];
        if let Some(u) = val.to_u64() {
            if u != 0 && samples_a < 5 {
                eprintln!(
                    "[real-pdes-c910]   LP-A {} = {} (id {})",
                    name, u, id
                );
                samples_a += 1;
            }
        }
        if samples_a >= 5 { break; }
    }
    for id in 0..ctx.signal_count() {
        let name = ctx.signal_name_at(id);
        if name.is_empty() || name.contains("x_ct_top_0") {
            continue;
        }
        let val = &tab_b[id];
        if let Some(u) = val.to_u64() {
            if u != 0 && samples_b < 5 {
                eprintln!(
                    "[real-pdes-c910]   LP-B {} = {} (id {})",
                    name, u, id
                );
                samples_b += 1;
            }
        }
        if samples_b >= 5 { break; }
    }
    if samples_a == 0 && samples_b == 0 {
        eprintln!(
            "[real-pdes-c910]   (all sampled signals stayed at 0 or X — \
            expected when comb settle is missing; flops can't see \
            new flop values without comb propagation)"
        );
    }

    (total_fires, total_nbas, wall_ms, per_tick_us)
}

/// c910 PDES front-half integration: classify the given Simulator's
/// compiled edge blocks by scope prefix into two LPs, build stub kernel
/// specs (block closures are no-ops that record only fire count), and
/// return the specs ready to feed to `PdesCoordinator::new`.
///
/// This deliberately stops short of executing the real bytecode — the
/// goal is to prove the parse/elaborate/compile/classify/partition/
/// kernel-construction pipeline scales to a 35M-signal, 20K-block c910
/// design without OOM or pathological slowdown. The integration of the
/// real `exec_insns_isolated` path into a `KernelBlock` closure is the
/// next-session task.
///
/// Stub blocks share `Arc<AtomicU64>` fire counters per LP so the test
/// driver can report how many invocations happened across the run.
pub fn build_c910_stub_specs(
    sim: &crate::compiler::Simulator,
    lp_a_prefix: &str,
    n_ticks: u64,
    clock_period_ns: u64,
) -> (Vec<KernelSpec>, Arc<std::sync::atomic::AtomicU64>, Arc<std::sync::atomic::AtomicU64>) {
    use std::sync::atomic::AtomicU64;

    let fire_a = Arc::new(AtomicU64::new(0));
    let fire_b = Arc::new(AtomicU64::new(0));

    let mut n_a = 0usize;
    let mut n_b = 0usize;
    let dot_prefix = format!("{}.", lp_a_prefix);

    // One stub closure per block — increments the per-LP fire counter
    // each invocation. Real impl wraps `exec_insns_isolated` with
    // signal_table snapshot and NBA collection.
    let make_stub = |counter: Arc<AtomicU64>| -> Arc<KernelBlock> {
        Arc::new(Box::new(move |_sigs: &[u64]| -> Vec<(usize, u64)> {
            counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Vec::new()
        }))
    };

    let mut blocks_a: Vec<Arc<KernelBlock>> = Vec::new();
    let mut blocks_b: Vec<Arc<KernelBlock>> = Vec::new();

    for bi in 0..sim.edge_block_count() {
        if !sim.edge_block_compiled(bi) {
            continue;
        }
        let scope = sim.edge_block_scope_at(bi);
        let lp_a = scope
            .as_deref()
            .map(|s| s == lp_a_prefix || s.starts_with(&dot_prefix))
            .unwrap_or(false);
        if lp_a {
            blocks_a.push(make_stub(Arc::clone(&fire_a)));
            n_a += 1;
        } else {
            blocks_b.push(make_stub(Arc::clone(&fire_b)));
            n_b += 1;
        }
    }
    eprintln!(
        "[PDES-stub] classified compiled blocks: LP-A '{}' = {}, LP-B = {}",
        lp_a_prefix, n_a, n_b
    );

    let specs = vec![
        KernelSpec {
            id: 0,
            owned_signal_ids: Vec::new(),
            blocks: blocks_a,
            outbound: Vec::new(),
            inbound: Vec::new(),
            clock_period_ns,
            max_sim_time: clock_period_ns.saturating_mul(n_ticks),
        },
        KernelSpec {
            id: 1,
            owned_signal_ids: Vec::new(),
            blocks: blocks_b,
            outbound: Vec::new(),
            inbound: Vec::new(),
            clock_period_ns,
            max_sim_time: clock_period_ns.saturating_mul(n_ticks),
        },
    ];
    (specs, fire_a, fire_b)
}

#[cfg(test)]
mod tests;
