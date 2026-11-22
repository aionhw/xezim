//! Scheduler state: NBA entries, waiters, edge-sensitive blocks, sensitivity and edge kinds.

use super::*;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SimOutput {
    pub time: u64,
    pub message: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct NbaEntry {
    /// §10.10: captured elements of a whole unpacked-array assignment.
    pub(super) elements: Option<Vec<Value>>,
    pub(super) lhs: Option<Expression>,
    pub(super) value: Value,
    pub(super) resolved_id: Option<usize>,
    /// §6.21: persistent-store key when the target is a `static` subroutine
    /// local, resolved AT SCHEDULE TIME.
    ///
    /// `assign_value` resolves such a target through `static_local_key_for`,
    /// which reads the CURRENT call frame — correct while the statement is
    /// executing, wrong in the NBA region, where the declaring frame is no
    /// longer on top. With two instances of the same interface each running a
    /// task that does `count <= count + 1`, the two applies raced over whichever
    /// frame happened to be current: one instance's counter never advanced at
    /// all while the other advanced at double rate, so its `forever` divider
    /// never reached its threshold and the clock it drove never toggled.
    pub(super) static_key: Option<String>,
    /// §10.4.2: frozen destination/source bit offsets and length for a
    /// selected persistent local. Merge at commit to preserve source order.
    pub(super) static_slice: Option<(u32, u32, u32)>,
    /// §8.3 + §4.9.4: the `this` handle in scope when the NBA was SCHEDULED.
    ///
    /// The slow path commits through `assign_value` in the NBA region, AFTER
    /// the process context has been restored to the event-loop caller — so
    /// `this_stack` is empty there and a CLASS PROPERTY target resolved to
    /// nothing. Every `prop <= val` inside a class method was silently
    /// DROPPED: scalars, vectors, static arrays, queues and dynamic arrays
    /// alike, while a blocking write in the same method worked. Captured here
    /// and re-pushed for the apply, exactly as `static_key` pins the declaring
    /// frame.
    pub(super) this_handle: Option<usize>,
    /// §10.4.2 / §4.9.4 (#569): the ROOT of a member or element target that
    /// is a subroutine formal or local holding an object handle (`tr.data[3]
    /// <= v` inside a task), captured when the NBA was SCHEDULED. Resolving
    /// the name in the NBA region would consult whichever frame the event
    /// loop still holds, which with two concurrent tasks is the wrong one.
    /// With the root's declared class, which selects a property a derived
    /// class redeclares (§8.14), when one is known.
    pub(super) root_frame: Option<(String, Value, Option<String>)>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct JoinWaiter {
    pub(super) parent_pid: usize,
    pub(super) child_pids: HashSet<usize>,
    pub(super) join_type: JoinType,
    pub(super) continuation: ProcCont,
    pub(super) finished_children: HashSet<usize>,
    /// `true` for a `wait fork` waiter. Unlike a plain `join`, its wake
    /// condition is re-evaluated against the *live* descendant tree on every
    /// child completion (LRM §9.6.1): it wakes only when the parent has zero
    /// remaining descendants, so it also waits for grandchildren spawned
    /// *after* the `wait fork` executed.
    pub(super) wait_fork: bool,
}

/// Fast-path NBA entry: compact (signal_id, value) pair for pre-resolved targets.
/// 99%+ of NBA entries use this path. Smaller struct = better cache utilization.

/// IEEE 1800-2023 §9.7: info stored when a process is explicitly suspended
/// via `process::suspend()`. The process is removed from whatever queue it
/// was parked in; `resume()` uses this to re-schedule it.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct SuspendedProc {
    /// Continuation statements to run when the process resumes.
    pub(super) continuation: ProcCont,
    /// Original scheduled expiry time if suspended while blocked on a `#delay`.
    /// `None` if blocked on an event/condition/join/mailbox. On resume, if the
    /// original delay has transpired (original_time <= self.time), the process
    /// continues immediately (LRM §9.7).
    pub(super) original_delay_expiry: Option<u64>,
}

/// §9.7 `process::await()`: a process blocked waiting for another's termination.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct AwaitWaiter {
    pub(super) target_pid: usize,
    pub(super) waiter_pid: usize,
    pub(super) continuation: ProcCont,
}

/// Header of a two-state stream in `Simulator::ts_arena`.
#[derive(Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct TsHdr {
    pub(super) off: u32,
    /// Stream length (13 bits) | executor kind (3 bits, `TS_HDR_KIND_SHIFT`);
    /// kind 0 = no arena stream. Eight bytes per entry keeps twice as many
    /// headers per cache line as the unpacked form did — the header load is
    /// the first touch of every settle evaluation.
    pub(super) len_kind: u16,
    pub(super) num_regs: u16,
}

pub(super) const TS_HDR_KIND_SHIFT: u32 = 13;

pub(super) const TS_HDR_LEN_MAX: usize = (1 << TS_HDR_KIND_SHIFT) - 1;

impl TsHdr {
    #[inline(always)]
    pub(super) fn kind(&self) -> u8 {
        (self.len_kind >> TS_HDR_KIND_SHIFT) as u8
    }
    #[inline(always)]
    pub(super) fn len(&self) -> usize {
        (self.len_kind & (TS_HDR_LEN_MAX as u16)) as usize
    }
}

/// `edge_block_armed` state bits. A gateable block skips its clock edge in
/// the scan prefilter iff its byte is ZERO: no input written since its last
/// execution (`EDGE_ARMED` clear) and nothing forcing a visit (`EDGE_HOLD`
/// clear — set while the block is not gateable, or has no valid value
/// snapshot and no wide read that lets it gate on arming alone). One byte
/// load per block visit instead of four flag arrays.
pub(super) const EDGE_ARMED: u8 = 1;

pub(super) const EDGE_HOLD: u8 = 2;

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct NbaFast {
    pub(super) signal_id: usize,
    pub(super) value: Value,
    /// Source block index. Used by the parallel-dispatch merge to
    /// recover sequential block-evaluation order before applying NBAs;
    /// without it the partition path could re-order writes across
    /// blocks and pick the wrong "last writer" for shared signals
    /// (see c910 cmark hang). Sequential / non-partition pushes leave
    /// this 0 — those paths already preserve insertion order.
    pub(super) block_index: u32,
}

#[derive(Clone, Copy)]
pub(super) struct ParallelBlockSlice {
    pub(super) ptr: *const super::bytecode::Insn,
    pub(super) len: usize,
    pub(super) num_regs: usize,
    /// CompiledBlock::nba_dup_targets, carried across the thread boundary
    /// because the raw slice loses the owning block.
    pub(super) nba_dup: bool,
}

unsafe impl Send for ParallelBlockSlice {}

unsafe impl Sync for ParallelBlockSlice {}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct EdgeSensitiveBlock {
    /// Pre-resolved signal IDs for O(1) edge checking. The unresolved
    /// `Vec<Sensitivity>` (with String name) used to live here too —
    /// dropped, since the only consumer (the init pass collecting
    /// edge_signal_ids/names) was rewritten to use this resolved
    /// list + id_to_name.
    pub(super) resolved_sensitivities: Vec<SensitivityId>,
    /// The block body. Between classification and `compile_edge_blocks` a
    /// block built from a lazy child always block holds an empty statement
    /// with the body's span here and its source in `lazy`; compilation
    /// rebuilds the body and keeps it only where a later reader needs it
    /// (see `drop_compiled_edge_block_asts`).
    pub(super) stmt: Statement,
    #[serde(skip)]
    pub(super) lazy: Option<Box<LazyAlways>>,
    pub(super) kind: AlwaysKind,
    /// Instance scope the block was inlined under, empty for top-level.
    /// Preferred over sensitivity-derived scopes (a port-connected clock's
    /// sensitivity collapses to the PARENT's signal and yields none).
    pub(super) scope: String,
}

/// A child-module always block of the `@(...) body` shape, kept as its
/// un-rewritten source (shared by every instance) plus this instance's
/// rewrite context instead of a per-instance statement tree: on a 64k-cell
/// DRAM array the 65k materialized trees were a third of the peak memory.
/// `patches` replace header event-term expressions exactly as the passes in
/// `Simulator::new` (edge-select aliases, computed edge terms) rewrote them;
/// nothing else in such a block is rewritten after elaboration.
#[derive(Debug, Clone)]
pub(super) struct LazyAlways {
    pub(super) src: super::elaborate::PendingAlways,
    pub(super) patches: Vec<(usize, Expression)>,
}

impl LazyAlways {
    /// The always block exactly as the eager pipeline would have held it.
    pub(super) fn materialize(&self) -> AlwaysBlock {
        let mut ab = Simulator::materialize_pending_always(self.src.clone());
        if !self.patches.is_empty() {
            if let StatementKind::TimingControl {
                control: TimingControl::Event(EventControl::EventExpr(exprs)),
                ..
            } = &mut ab.stmt.kind
            {
                for (i, e) in &self.patches {
                    exprs[*i].expr = e.clone();
                }
            }
        }
        ab
    }

    /// The body `classify_one_always_block` keeps for an edge block (the
    /// statement under the `@(...)` header).
    pub(super) fn materialize_body(&self) -> Statement {
        match self.materialize().stmt.kind {
            StatementKind::TimingControl { stmt, .. } => *stmt,
            _ => unreachable!("lazy always blocks have an event-control header"),
        }
    }
}

/// An always block taken out of `module.pending_always` by
/// `rewrite_edge_select_sensitivities`, in its original order: eagerly
/// materialized, or lazy (see `LazyAlways`). While `Simulator::new` runs a
/// lazy one also carries its rewritten header, which the passes there edit;
/// `patched` lists the header terms they replaced.
#[derive(Debug)]
pub(super) enum DeferredAlways {
    Eager(AlwaysBlock),
    Lazy {
        lazy: LazyAlways,
        header: Option<Box<Statement>>,
        patched: Vec<usize>,
    },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct SensitivityId {
    pub(super) signal_id: usize,
    pub(super) edge: EdgeKind,
    /// LRM §9.4.2.3 `iff` guard for this event term: the edge only counts
    /// when this expression is true at edge time. `None` for an unguarded
    /// term. Consulted by the procedural `@`-event wake path; the always-
    /// block edge paths carry it but do not yet evaluate it. Shared with
    /// the other operands of this event expression. An inline
    /// `Option<Expression>` made each term hundreds of bytes (68 MB of edge
    /// sensitivity lists on a 64k-cell DRAM).
    pub(super) iff: Option<Rc<Expression>>,
    /// §9.4.2: for a NON-TRIVIAL event expression (`@(a + b)`), the term's
    /// full expression. Operand-signal edges only COUNT when this
    /// expression's value differs from its value at arm time (stored in
    /// `EventWaiter::guard_prev`) — `a=2;b=1` leaving `a+b` at 3 must not
    /// fire. `None` for plain signal/edge terms.
    pub(super) value_of: Option<Rc<Expression>>,
    /// Scheduling depth of the NAME this term was written with (see
    /// `Simulator::sig_wake_rank`); orders same-pass waiter wakeups.
    pub(super) wake_rank: u8,
    /// §9.4.2: an unpacked-array CELL that `value_of` selects through a
    /// runtime index (`@(mem[i])`), found by evaluating the index when the
    /// waiter armed. Re-resolved whenever an operand moves without firing,
    /// so the wait follows the index to the element it selects now.
    pub(super) cell: bool,
    /// §23.3.3, §10.3.3: how the NAME this term was written with starts at
    /// time 0 when it is a port formal or an identity net (`assign y = x`)
    /// collapsed onto its source's id: 0 for a name that is no such alias,
    /// 1 for a 4-state alias (from x), 2 for a 2-state one (from 0). The
    /// alias takes the source's time-0 value as an update from that start,
    /// one the source's own readers never see (§6.8: an initializer is no
    /// event). See `Simulator::alias_t0_kind`.
    pub(super) alias_t0: u8,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct Sensitivity {
    pub(super) signal_name: String,
    pub(super) edge: EdgeKind,
    /// See `SensitivityId::iff` — carried through name→id resolution.
    pub(super) iff: Option<Rc<Expression>>,
    /// See `SensitivityId::value_of` — carried through name→id resolution.
    pub(super) value_of: Option<Rc<Expression>>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct EdgeFanout {
    pub(super) posedge: Vec<usize>,
    pub(super) negedge: Vec<usize>,
    #[serde(skip)]
    pub(super) posedge_batches: Vec<edges::EdgeBatch>,
    #[serde(skip)]
    pub(super) negedge_batches: Vec<edges::EdgeBatch>,
    pub(super) anyedge: Vec<usize>,
    /// `@(edge x)` blocks — §9.4.2 LSB edge (posedge OR negedge of bit 0),
    /// NOT the whole-vector change that `anyedge` (`@(x)` level) fires on.
    pub(super) lsbedge: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) enum EdgeKind {
    Posedge,
    Negedge,
    /// `@(x)` / `always @(x)` — level sensitivity: fires on ANY change of the
    /// whole vector (§9.2 implicit event control).
    AnyEdge,
    /// `@(edge x)` — §9.4.2 explicit any-edge: a posedge OR negedge of the
    /// LSB only (like posedge/negedge, a multi-bit vector's edge is its bit 0),
    /// NOT a whole-vector change. Distinct from AnyEdge so `@(edge wide)`
    /// doesn't fire on a change that leaves bit 0 unchanged.
    LsbEdge,
}

impl EdgeKind {
    /// Prefix used when printing a sensitivity in diagnostics
    /// (`posedge clk`, `edge bus`, …). Centralised so adding an `EdgeKind`
    /// variant touches one place, not the ~6 hang-report format sites.
    pub(super) const fn print_str(&self) -> &'static str {
        match self {
            EdgeKind::Posedge => "posedge ",
            EdgeKind::Negedge => "negedge ",
            EdgeKind::AnyEdge => "",
            EdgeKind::LsbEdge => "edge ",
        }
    }
}

/// LRM §15.4.2: a process blocked inside `mailbox.get(var)` on an empty
/// mailbox. The next `put` drains this waiter, assigns its value into
/// `lvalue`, and reschedules `cont` under `pid` at the current time.
#[derive(Debug, Clone)]
/// A process blocked in `semaphore.get(n)` because the count was below `n`
/// (IEEE 1800-2017 §15.3.3). Woken by a `put` that raises the count enough.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct SemGetWaiter {
    pub(super) pid: usize,
    /// Keys still required.
    pub(super) n: i64,
    pub(super) cont: ProcCont,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct MailboxGetWaiter {
    pub(super) pid: usize,
    pub(super) lvalue: Expression,
    pub(super) cont: ProcCont,
    /// `peek` (not `get`): the waiter reads the front WITHOUT removing it, so
    /// a `put` that wakes it must also leave the item in the mailbox for the
    /// subsequent `get`/`try_get`.
    pub(super) is_peek: bool,
}

/// §15.4.1 — a process blocked on `mailbox.put(v)` because a BOUNDED mailbox is
/// full. The value is captured at the (blocking) call; when a `get`/`try_get`
/// frees a slot the value is stored and `cont` is rescheduled.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct MailboxPutWaiter {
    pub(super) pid: usize,
    pub(super) value: Value,
    pub(super) cont: ProcCont,
}

/// See `Simulator::event_list_shape`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EventListShape {
    /// `@*`, `always_comb`, a named event, or no leading event control.
    Implicit,
    /// An explicit list whose every term is a plain identifier.
    Plain,
    /// An explicit list with a select, concatenation or expression term.
    NonPlain,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct EventWaiter {
    pub(super) pid: usize,
    /// Simulation time when this waiter parked. Drives the hang report:
    /// waiters sorted oldest-first expose "who has been stuck longest",
    /// and `arm_bits` tells whether the awaited signals ever moved since.
    pub(super) parked_time: u64,
    /// Pre-resolved signal IDs for O(1) edge checking. The unresolved
    /// `Vec<Sensitivity>` mirror was set at construction but never
    /// consulted afterwards — dropped.
    pub(super) resolved_sensitivities: Vec<SensitivityId>,
    /// Value (raw v,x low-64) of each `resolved_sensitivities` signal at the
    /// moment this waiter armed. Parallel to `resolved_sensitivities`. Used to
    /// detect a change made AFTER arming within the same snapshot generation.
    pub(super) arm_bits: Vec<(u64, u64)>,
    pub(super) continuation: ProcCont,
    /// Each sensitivity signal's value captured AT registration time
    /// (`raw_bits()` for the ≤64-bit fast path). The waiter fires when the
    /// signal changes relative to this captured baseline — NOT the global
    /// per-tick `prev_val` snapshot. This ensures NBA updates that commit
    /// after registration in the same tick wake the waiter. It also
    /// subsumes the old `snap_gen` guard: a `forever @(posedge clk)`
    /// re-registered just after consuming a posedge captures clk's now-high value,
    /// so the consumed edge cannot re-fire (IEEE 1800-2023 §9.4.2.3).
    pub(super) captured_prev: Vec<(u64, u64)>,
    /// Full captured value for >64-bit sensitivity signals (parallel to
    /// `captured_prev`); `None` for ≤64-bit signals.
    pub(super) captured_prev_wide: Vec<Option<Value>>,
    /// §9.4.2 value guard for non-trivial event expressions — parallel to
    /// `resolved_sensitivities`. `Some(v)` holds the term expression's value
    /// at ARM time; an operand edge only counts when the expression now
    /// evaluates differently (see `SensitivityId::value_of`).
    pub(super) guard_prev: Vec<Option<Value>>,
    /// Number of qualifying events still required before the continuation
    /// resumes. This collapses action-free `repeat (N) @(event);` loops into
    /// one parked waiter instead of rebuilding their AST continuation after
    /// every edge.
    pub(super) remaining_events: u64,
    /// LRM §14.13: this waiter is parked on a CLOCKING event (`@(cb)` / `##N`),
    /// not a raw `@(posedge clk)`. Its continuation must resume in the Reactive
    /// region — AFTER the same-edge NBA updates commit and the clocking input
    /// snapshot refreshes — so it reads post-edge design state and this cycle's
    /// `cb.<in>` samples. Raw edge waiters resume in the active region right
    /// after the same edge's blocks (reference-verified default; see
    /// `waiters_first` in check_edges_inner); only clocking waiters are
    /// deferred past the NBA commit.
    pub(super) is_clocking: bool,
    /// Set only for a waiter with expression terms (`SensitivityId::value_of`).
    pub(super) value_ctx: Option<Box<WaiterValueCtx>>,
}

/// §9.4.2: how a waiter re-evaluates its expression terms after it armed.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct WaiterValueCtx {
    /// The name-resolution scope the waiting process armed in. The drain runs
    /// outside that process, and a bare `mem[i]` in a sub-module instance
    /// must still read that instance's `mem` and `i`.
    pub(super) scope: Option<String>,
    /// Some term selects an unpacked-array cell (`@(mem[i])`): the cells are
    /// looked up again whenever an operand moves (see `SensitivityId::cell`).
    pub(super) cells: bool,
}

/// The name-keyed metadata a frame-local's declaration registers: its
/// typedef, class, declared type, struct members, packed layout and widths,
/// under the bare name and its `name.member` keys. Two concurrent processes
/// whose same-named locals differ in type share these tables, so the one
/// that declared last displaced the other's; `capture_local_meta` keeps a
/// displaced declaration's entries in `local_meta_cache` and
/// `reapply_local_meta` installs them again when a process whose frames
/// claim that declaration runs.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub(in crate::compiler) struct LocalMeta {
    /// A struct's: `name.member` entries belong to it.
    pub(super) members: bool,
    pub(super) typedef: Option<String>,
    pub(super) class: Option<String>,
    pub(super) container: Option<String>,
    pub(super) type_args: Option<Vec<Expression>>,
    pub(super) decl_types: Vec<(String, Rc<DataType>)>,
    pub(super) struct_members: Vec<(String, Vec<String>)>,
    pub(super) packed_fields: Vec<(String, Vec<(String, u32, u32)>)>,
    pub(super) elem_widths: Vec<(String, u32)>,
    pub(super) full_dims: Vec<(String, Vec<(i64, i64)>)>,
    pub(super) width: Option<u32>,
    pub(super) signed: bool,
    pub(super) real: bool,
    pub(super) string: bool,
}

/// Where a frame-local's declaration came from: its statement, and the class
/// and specialization it ran in. Frames running the same declaration register
/// the same metadata, so the cache of displaced metadata is keyed by it.
#[derive(Clone, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(in crate::compiler) struct MetaOrigin {
    pub(super) span: (usize, usize),
    pub(super) class: Option<String>,
    pub(super) spec: Option<(String, String)>,
}

/// A frame-local declaration a process has executed: the frame it declared
/// into, the local's name and its declaration (an index into
/// `local_meta_origins`). The list rides the process context, not the frame
/// generation: a fork child shares its parent's frames (§9.3.2), and its
/// block's `T x;` hides the parent's `U x;` from the child only.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(in crate::compiler) struct MetaClaim {
    pub(super) frame: u64,
    pub(super) name: String,
    pub(super) origin: u32,
}

/// A process parked on a CLASS-FIELD named event (`event m_event` inside a
/// class, awaited/triggered as `@m_event` / `->m_event` from a method on
/// `this`). Unlike a module-scope named event, a class `event` field has no
/// backing signal in `signal_table`, so it cannot flow through the edge-
/// sensitive `event_waiters` path. Each (owning instance handle, field name)
/// pair is a distinct synchronization object (IEEE 1800-2023 §15.5). The key is the
/// raw `(this_handle, field_name)`; `->`/`@` inside a method both resolve to
/// the same `this`, so a trigger and a wait on the same object match.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) enum InstanceEventKeys {
    One((usize, String)),
    Any(Vec<(usize, String)>),
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct InstanceEventWaiter {
    pub(super) key: InstanceEventKeys,
    pub(super) pid: usize,
    pub(super) continuation: ProcCont,
}

/// The parked `@ev` waiters on class-member events, indexed by event and by
/// pid. A process parked forever on a rarely-fired event (a reset watcher
/// left behind by `fork ... join_any`) costs nothing per trigger of another
/// event or per suspension query; entries keep their arrival order, so a
/// trigger still wakes its waiters in the order they parked.
#[derive(Default)]
pub(super) struct InstanceEventWaiters {
    pub(super) entries: BTreeMap<u64, InstanceEventWaiter>,
    by_key: HashMap<(usize, String), std::collections::BTreeSet<u64>>,
    by_pid: HashMap<usize, Vec<u64>>,
    next: u64,
}

impl InstanceEventWaiters {
    fn keys_of(w: &InstanceEventWaiter) -> &[(usize, String)] {
        match &w.key {
            InstanceEventKeys::One(k) => std::slice::from_ref(k),
            InstanceEventKeys::Any(ks) => ks,
        }
    }

    pub(super) fn push(&mut self, w: InstanceEventWaiter) {
        let seq = self.next;
        self.next += 1;
        for k in Self::keys_of(&w) {
            self.by_key.entry(k.clone()).or_default().insert(seq);
        }
        self.by_pid.entry(w.pid).or_default().push(seq);
        self.entries.insert(seq, w);
    }

    pub(super) fn contains_pid(&self, pid: usize) -> bool {
        self.by_pid.contains_key(&pid)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    fn unlink(&mut self, seq: u64, w: &InstanceEventWaiter) {
        for k in Self::keys_of(w) {
            if let Some(set) = self.by_key.get_mut(k) {
                set.remove(&seq);
                if set.is_empty() {
                    self.by_key.remove(k);
                }
            }
        }
        if let Some(seqs) = self.by_pid.get_mut(&w.pid) {
            seqs.retain(|&s| s != seq);
            if seqs.is_empty() {
                self.by_pid.remove(&w.pid);
            }
        }
    }

    /// Drop every waiter of `pid` (the process was killed or disabled).
    pub(super) fn remove_pid(&mut self, pid: usize) {
        let Some(seqs) = self.by_pid.remove(&pid) else {
            return;
        };
        for seq in seqs {
            if let Some(w) = self.entries.remove(&seq) {
                for k in Self::keys_of(&w) {
                    if let Some(set) = self.by_key.get_mut(k) {
                        set.remove(&seq);
                        if set.is_empty() {
                            self.by_key.remove(k);
                        }
                    }
                }
            }
        }
    }

    pub(super) fn remove_pids<'a>(&mut self, pids: impl IntoIterator<Item = &'a usize>) {
        for &pid in pids {
            self.remove_pid(pid);
        }
    }

    /// Park every waiter on `from` on `to` instead.
    pub(super) fn rekey(&mut self, from: &(usize, String), to: &(usize, String)) {
        let Some(seqs) = self.by_key.remove(from) else {
            return;
        };
        for seq in &seqs {
            if let Some(w) = self.entries.get_mut(seq) {
                match &mut w.key {
                    InstanceEventKeys::One(k) => *k = to.clone(),
                    InstanceEventKeys::Any(ks) => {
                        for k in ks.iter_mut().filter(|k| *k == from) {
                            *k = to.clone();
                        }
                    }
                }
            }
        }
        self.by_key.entry(to.clone()).or_default().extend(seqs);
    }

    /// Remove and return, in parking order, every waiter on `key`.
    pub(super) fn take_key(&mut self, key: &(usize, String)) -> Vec<(usize, ProcCont)> {
        let Some(seqs) = self.by_key.remove(key) else {
            return Vec::new();
        };
        let mut woken = Vec::with_capacity(seqs.len());
        for seq in seqs {
            if let Some(w) = self.entries.remove(&seq) {
                self.unlink(seq, &w);
                woken.push((w.pid, w.continuation));
            }
        }
        woken
    }
}

impl serde::Serialize for InstanceEventWaiters {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(self.entries.values())
    }
}

impl<'de> serde::Deserialize<'de> for InstanceEventWaiters {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut waiters = Self::default();
        for w in Vec::<InstanceEventWaiter>::deserialize(d)? {
            waiters.push(w);
        }
        Ok(waiters)
    }
}

/// Timing wheel for O(1) near-future event scheduling.
/// Events within WHEEL_SIZE ticks of current time use a circular array.
/// Events further out fall back to a BTreeMap.
pub(super) const WHEEL_SIZE: usize = 4096;

/// Prefix of the hidden per-clocking-input sample mirror signals (§14.13).
/// Not expressible in SV source, so it can never collide with a user name.
pub(super) const CB_SAMPLE_PREFIX: &str = "__xz_cbsample.";

/// §14.5: the hidden clocking names of the operand samples of a clockvar
/// bound to an expression (`__xz_cbop.sum.0`), and the unresolvable net name
/// the clockvar itself is registered under. Unreachable from SV source.
pub(super) const CB_OPERAND_PREFIX: &str = "__xz_cbop.";

/// IEEE 1800-2023 §14.4: `#1step` in the clocking skew maps. An input with no
/// declared skew samples with 1step too (§14.3 default), so the default-input
/// map holds only explicit non-1step skews.
pub(super) const CB_SKEW_1STEP: u64 = u64::MAX;

/// §14.4 explicit input skew `#N` (N > 0): a net's values over the last N
/// ticks. `changes` holds `(slot, value at the end of that slot)` in time
/// order, `init` the value before the first recorded slot.
#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(super) struct ClockingHist {
    pub(super) init: Option<Value>,
    pub(super) changes: VecDeque<(u64, Value)>,
    pub(super) span: u64,
}

/// Number of u64 words needed for the occupancy bitmap (256 / 64 = 4).
pub(super) const BITMAP_WORDS: usize = WHEEL_SIZE / 64;

pub(super) type EventList = VecDeque<(usize, ProcCont)>;

/// One clock-tree group's edge answer for the current detect pass.
///
/// Members of a group are exact copies of one driver, so both the fire decision
/// AND the raw value pair are shared: `fired_snap` (which re-baselines `prev`)
/// and the §9.4.2 bit-select re-test both need the values, and taking them from
/// the leader is what removes the scattered reads. `XEZIM_CLKTREE_PROBE=1`
/// verifies that equality holds rather than assuming it.
#[derive(Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct EdgeGroupMemo {
    pub(super) generation: u64,
    pub(super) fires_pos: bool,
    pub(super) fires_neg: bool,
    pub(super) fires_any: bool,
    pub(super) cur_v: u64,
    pub(super) cur_x: u64,
    pub(super) prev_v: u64,
    pub(super) prev_x: u64,
}

/// Built-in clock generator: replaces `always #N clk = ~clk` with O(1) toggle.
/// Eliminates AST cloning and traversal for the most common simulation pattern.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct ClockGen {
    pub(super) signal_id: usize,
    pub(super) half_period: u64,
    pub(super) next_toggle_time: u64,
    /// Position of `signal_id` inside `Simulator::edge_signal_ids`, or
    /// `usize::MAX` if this clock isn't in any edge sensitivity list (e.g.
    /// a `forever` generator driving only level-sensitive logic). Populated
    /// once at the end of elaboration via `seal_edge_signal_ids`; lets
    /// `fire_clock_generators` fill `toggled_clock_positions` in O(1).
    pub(super) edge_signal_position: usize,
    /// §4.4.2 FIFO rank of the next toggle among the wheel events due at the
    /// same time: how many were already queued when the generator process
    /// would have scheduled it (at its previous toggle). Those resume before
    /// the toggle, later ones after it. `None` until the time-0 drain
    /// reaches the generator's source position (`t0_rank`).
    pub(super) ahead: Option<u32>,
    /// The generator's slot in the time-0 activation order: the number of
    /// time-0 processes that run before it (see `order_time0_processes`).
    /// `usize::MAX` until that order is known.
    pub(super) t0_rank: usize,
}

/// Reusable scheduler state for `always #(expr) lhs = rhs` when the delay is
/// dynamic. The generic Forever continuation rebuilds and clones its AST on
/// every wakeup; PLL models can execute that path millions of times. Keeping
/// one owned delay/body pair preserves per-iteration delay evaluation without
/// allocating a new continuation for every clock edge.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct FastDelayAlways {
    pub(super) delay: Expression,
    pub(super) body: Statement,
    /// Pre-resolved bytecode for the retained assignment body. Unsupported
    /// constructs keep using `body` through the AST interpreter.
    pub(super) compiled_body: Option<super::bytecode::CompiledBlock>,
    /// Pre-resolved delay expression and its result register.
    pub(super) compiled_delay: Option<(super::bytecode::CompiledBlock, u16)>,
    /// Module precision relative to the global tick, cached on first arm.
    pub(super) delay_precision_diff: Option<i32>,
    /// False for the initial activation, which only arms the first delay.
    /// True once a delayed activation should execute `body` before re-arming.
    pub(super) execute_body: bool,
}

/// JIT Stage 4 Tier C Path C1 (JIT-REDESIGN-NOTES.md): one entry of
/// the side queue that JIT-generated NBA writes go into.  Layout
/// must be stable — JIT codegen writes signal_id at offset 0 (u32)
/// and val_bits at offset 8 (u64), with 4 bytes of padding between
/// (the natural alignment for u64).
///
/// `#[repr(C)]` to lock down field order across builds.  Total
/// size: 16 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct JitNbaSideEntry {
    pub(crate) signal_id: u32,
    pub(crate) _pad: u32,
    pub(crate) val_bits: u64,
}

/// Maps a signal id to its in-flight NBA queue entry index.
///
/// Named signals occupy a dense `u32` prefix, preserving the O(1), cache-local
/// path that replaced the original per-NBA HashMap lookup. Unnamed high-ID
/// array elements use a sparse retained map: memory-heavy designs can contain
/// tens of millions of elements while scheduling only a few array NBAs in any
/// one delta cycle, so a full `4 * num_signals` index is mostly cold metadata.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct NbaFastIndex {
    /// `dense_entries[id]` holds the NBA queue index for a named signal, or
    /// `u32::MAX` if no in-flight entry exists this iter.
    dense_entries: Vec<u32>,
    /// High-ID unnamed array elements touched in the current NBA window.
    sparse_entries: HashMap<usize, u32>,
    /// Dense ids set this iter. Walked by `clear()` instead of clearing the
    /// whole prefix.
    dirty_dense: Vec<usize>,
}

impl NbaFastIndex {
    pub(super) fn with_dense_prefix(dense_len: usize) -> Self {
        Self {
            dense_entries: vec![u32::MAX; dense_len],
            sparse_entries: HashMap::default(),
            dirty_dense: Vec::new(),
        }
    }

    #[inline]
    pub(super) fn get(&self, id: usize) -> Option<usize> {
        let e = if id < self.dense_entries.len() {
            self.dense_entries[id]
        } else {
            *self.sparse_entries.get(&id)?
        };
        if e == u32::MAX {
            None
        } else {
            Some(e as usize)
        }
    }

    #[inline]
    pub(super) fn insert(&mut self, id: usize, idx: usize) {
        let idx = idx as u32;
        if id < self.dense_entries.len() {
            if self.dense_entries[id] == u32::MAX {
                self.dirty_dense.push(id);
            }
            self.dense_entries[id] = idx;
        } else {
            self.sparse_entries.insert(id, idx);
        }
    }

    pub(super) fn clear(&mut self) {
        for &id in &self.dirty_dense {
            self.dense_entries[id] = u32::MAX;
        }
        self.dirty_dense.clear();
        self.sparse_entries.clear();
    }

    /// Widen the dense prefix to cover `total` signal ids. Called once the
    /// final signal-table size is known so unnamed array elements (memories)
    /// hit the O(1) dense path instead of hashing into `sparse_entries` on
    /// every NBA — on c906 memcpy the sparse hash was a measurable slice of
    /// the sim loop. 4 B/signal; freed sparse map becomes vestigial.
    pub(super) fn extend_dense(&mut self, total: usize) {
        if total > self.dense_entries.len() {
            self.dense_entries.resize(total, u32::MAX);
        }
    }
    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.dirty_dense.is_empty() && self.sparse_entries.is_empty()
    }
}

#[cfg(test)]
mod nba_fast_index_tests {
    use super::{CombEntry, CombItem, Expression, NbaFastIndex, ProcCont, Statement, TimingWheel};

    #[test]
    fn dense_prefix_and_sparse_tail_track_and_clear_entries() {
        let mut index = NbaFastIndex::with_dense_prefix(4);
        index.insert(2, 7);
        index.insert(1_000_000, 9);
        assert_eq!(index.get(2), Some(7));
        assert_eq!(index.get(1_000_000), Some(9));
        assert_eq!(index.dense_entries.len(), 4);
        assert_eq!(index.sparse_entries.len(), 1);

        index.insert(2, 11);
        index.insert(1_000_000, 13);
        assert_eq!(index.get(2), Some(11));
        assert_eq!(index.get(1_000_000), Some(13));

        index.clear();
        assert!(index.is_empty());
        assert_eq!(index.get(2), None);
        assert_eq!(index.get(1_000_000), None);
    }

    #[test]
    fn comb_entry_layout_stays_cache_compact() {
        // The interpreted Statement payload used to inflate every entry to
        // 480 bytes. Keep cold AST payloads boxed so the random-access settle
        // worklist can fit several entries per cache line/page.
        assert!(std::mem::size_of::<CombItem>() <= 64);
        assert!(std::mem::size_of::<CombEntry>() <= 160);
        assert!(std::mem::size_of::<Expression>() > std::mem::size_of::<Box<Expression>>());
        assert!(std::mem::size_of::<Statement>() > std::mem::size_of::<Box<Statement>>());
    }

    #[test]
    fn timing_wheel_pop_front_preserves_fifo_and_pending_pids() {
        let mut wheel = TimingWheel::new();
        wheel.schedule(10, 1, ProcCont::empty());
        wheel.schedule(10, 2, ProcCont::empty());
        wheel.schedule(10, 1, ProcCont::empty());

        assert_eq!(wheel.pop_front(10).map(|event| event.0), Some(1));
        assert!(wheel.has_pid(1));
        wheel.schedule(10, 3, ProcCont::empty());

        assert_eq!(wheel.pop_front(10).map(|event| event.0), Some(2));
        assert_eq!(wheel.pop_front(10).map(|event| event.0), Some(1));
        assert!(!wheel.has_pid(1));
        assert_eq!(wheel.pop_front(10).map(|event| event.0), Some(3));
        assert_eq!(wheel.next_time(), None);
    }
}

/// §9.4.2 Table 9-2 edge test on the LSB of a raw (v, x) pair — the same
/// rule `Simulator::edge_fires_prev` applies to a signal; `AnyEdge` is any
/// change of the low 64 bits.
#[inline]
pub(super) fn lsb_edge_fires(edge: EdgeKind, pv: u64, px: u64, cv: u64, cx: u64) -> bool {
    let cb_one = (cv & 1) == 1 && (cx & 1) == 0;
    let cb_zero = (cv & 1) == 0 && (cx & 1) == 0;
    let pb_one = (pv & 1) == 1 && (px & 1) == 0;
    let pb_zero = (pv & 1) == 0 && (px & 1) == 0;
    match edge {
        EdgeKind::Posedge => (pb_zero && !cb_zero) || (!pb_one && cb_one),
        EdgeKind::Negedge => (pb_one && !cb_one) || (!pb_zero && cb_zero),
        EdgeKind::AnyEdge => cv != pv || cx != px,
        EdgeKind::LsbEdge => {
            (pb_zero && !cb_zero)
                || (!pb_one && cb_one)
                || (pb_one && !cb_one)
                || (!pb_zero && cb_zero)
        }
    }
}

/// §9.4.2: did an event expression's VALUE produce `edge` going from `prev`
/// to `cur`? A level event is any change; an edge is judged on the LSB.
pub(super) fn value_event_fires(edge: EdgeKind, prev: &Value, cur: &Value) -> bool {
    if edge == EdgeKind::AnyEdge {
        return cur != prev;
    }
    let (pv, px) = prev.raw_bits();
    let (cv, cx) = cur.raw_bits();
    lsb_edge_fires(edge, pv, px, cv, cx)
}
