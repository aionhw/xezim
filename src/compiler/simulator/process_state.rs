//! Process continuations, process-id vectors, the timing wheel, process FSMs and process
//! contexts.

use super::*;

thread_local! {
    /// Current `run_process_stmts` recursion depth. The suspend-aware loop
    /// handlers trampoline through the event queue (instead of recursing) once
    /// this exceeds `RPS_TRAMPOLINE_DEPTH`, bounding the stack for long
    /// synchronous loops that would otherwise recurse until stack overflow.
    pub(super) static RPS_DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// A list of `(pid, item)` entries that also counts each pid's entries, so
/// "does this process wait here?" is O(1) however many processes wait.
#[derive(Debug)]
pub(crate) struct PidVec<T> {
    pub(super) items: Vec<(usize, T)>,
    counts: HashMap<usize, u32>,
}

impl<T> Default for PidVec<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            counts: HashMap::default(),
        }
    }
}

impl<T> PidVec<T> {
    pub(crate) fn push(&mut self, entry: (usize, T)) {
        *self.counts.entry(entry.0).or_insert(0) += 1;
        self.items.push(entry);
    }

    pub(crate) fn contains_pid(&self, pid: usize) -> bool {
        self.counts.contains_key(&pid)
    }

    pub(super) fn forget(counts: &mut HashMap<usize, u32>, pid: usize) {
        if let Some(c) = counts.get_mut(&pid) {
            *c -= 1;
            if *c == 0 {
                counts.remove(&pid);
            }
        }
    }

    pub(crate) fn remove(&mut self, index: usize) -> (usize, T) {
        let entry = self.items.remove(index);
        Self::forget(&mut self.counts, entry.0);
        entry
    }

    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&(usize, T)) -> bool) {
        let counts = &mut self.counts;
        self.items.retain(|entry| {
            let k = keep(entry);
            if !k {
                Self::forget(counts, entry.0);
            }
            k
        });
    }

    /// Remove and return, in order, the entries whose `marked` flag is set.
    pub(crate) fn take_marked(&mut self, marked: &[bool]) -> Vec<(usize, T)> {
        if !marked.iter().any(|&m| m) {
            return Vec::new();
        }
        let mut taken = Vec::new();
        let items = std::mem::take(&mut self.items);
        self.items.reserve(items.len());
        for (entry, &m) in items.into_iter().zip(marked) {
            if m {
                Self::forget(&mut self.counts, entry.0);
                taken.push(entry);
            } else {
                self.items.push(entry);
            }
        }
        taken
    }

    pub(crate) fn extend(&mut self, entries: impl IntoIterator<Item = (usize, T)>) {
        for entry in entries {
            self.push(entry);
        }
    }

    pub(crate) fn iter(&self) -> std::slice::Iter<'_, (usize, T)> {
        self.items.iter()
    }

    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl<T> std::ops::Index<usize> for PidVec<T> {
    type Output = (usize, T);
    fn index(&self, index: usize) -> &(usize, T) {
        &self.items[index]
    }
}

impl<T> IntoIterator for PidVec<T> {
    type Item = (usize, T);
    type IntoIter = std::vec::IntoIter<(usize, T)>;
    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

impl<T: serde::Serialize> serde::Serialize for PidVec<T> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.items.serialize(s)
    }
}

impl<'de, T: serde::Deserialize<'de>> serde::Deserialize<'de> for PidVec<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut v = Self::default();
        for entry in Vec::<(usize, T)>::deserialize(d)? {
            v.push(entry);
        }
        Ok(v)
    }
}

/// A suspended process's remaining work: a shared statement list plus a cursor,
/// chained to whatever runs after that list is exhausted.
///
/// It is NOT an index into the AST, and it cannot be. `run_process_stmts`
/// SYNTHESIZES the list it executes — a blocking `begin/end` is flattened into
/// the caller's stream, a blocking task call is spliced in front of the caller's
/// tail — so the statements a parked process still owes are routinely a
/// concatenation that exists nowhere in the source. What CAN be named is a
/// shared handle to that synthesized list plus an offset, and that is what
/// removes the copying:
///
/// * parking used to deep-clone the tail of the statement list — everything the
///   process had left — on every `#delay` and every `@(posedge)`;
/// * splicing used to clone the body and then copy the caller's whole tail onto
///   the end of it, every time a blocking block or task was entered.
///
/// `next` is the frame chain: a splice pushes `body` with `next` = the caller's
/// tail, and the executor follows the chain when a frame runs out. Frames are
/// `Arc`, so capturing a continuation costs O(depth) pointer bumps instead of
/// O(work remaining) statement clones.
///
/// Measured on `bench/run_uvm_bench.sh`: -3.6% median across the UVM examples,
/// which spend ~98% of the loop on this path. It is a TRADE, not a free win —
/// `Arc::from` per splice costs a tight `forever` re-splicing a large block
/// (+9% on the `cont_post_100` synthetic).
#[derive(Clone, Debug)]
pub(super) struct ProcCont {
    pub(super) stmts: Arc<[Statement]>,
    pub(super) start: usize,
    pub(super) next: Option<Arc<ProcCont>>,
    // Absolute index into `stmts` of a synthesized `While`/`Repeat` re-entry
    // tail, if this frame carries one (see bug 28's PR #174 follow-up).
    // Those two kinds are reused for BOTH a loop's own continuation
    // (re-appended after unrolling one iteration, which must consume a
    // pending `break`/`continue`) and an ordinary fresh occurrence of the
    // same kind reached for the first time while a flag from an ENCLOSING
    // loop happens to be pending (which must NOT consume it — it has to be
    // skipped like any other statement, §9.3.3/§12.7). The two are
    // structurally identical (same `StatementKind` variant), so the
    // run_process_stmts top guard cannot tell them apart by kind alone; this
    // field names the ONE index in this frame that is a genuine re-entry.
    // `While` and `Repeat` are the only kinds ever re-appended: a blocking
    // `for` lowers to `while (cond) { body; step }` and a `do…while` to
    // `body; while (cond) body`, so `For`/`DoWhile` are only ever reached
    // fresh. `Foreach`/`Forever` re-enter through the distinct
    // `ForeachTail`/`ForeverTail` kinds, so their base kind is never
    // ambiguous and is never a flag consumer.
    pub(super) loop_tail_at: Option<usize>,
    /// Remaining iterations of a suspended repeat at loop_tail_at. The body
    /// frame stays immutable and shared instead of being cloned per iteration.
    pub(super) repeat_remaining: Option<u64>,
}

/// Defense in depth for the chain length: the derived drop for a linked list
/// recurses once per link, so a long chain aborts the process instead of
/// returning memory. Unlink iteratively, stopping as soon as a node is still
/// shared (someone else owns the rest).
impl Drop for ProcCont {
    fn drop(&mut self) {
        let mut cur = self.next.take();
        while let Some(arc) = cur {
            match Arc::try_unwrap(arc) {
                Ok(mut node) => cur = node.next.take(),
                Err(_) => break,
            }
        }
    }
}

impl ProcCont {
    /// A continuation over a freshly synthesized statement list.
    pub(super) fn from_vec(stmts: Vec<Statement>) -> Self {
        ProcCont {
            stmts: Arc::from(stmts),
            start: 0,
            next: None,
            loop_tail_at: None,
            repeat_remaining: None,
        }
    }

    /// A continuation over a shared statement slice.
    pub(super) fn from_frame(stmts: Arc<[Statement]>) -> Self {
        ProcCont {
            stmts,
            start: 0,
            next: None,
            loop_tail_at: None,
            repeat_remaining: None,
        }
    }

    /// Nothing to run.
    pub(super) fn empty() -> Self {
        thread_local! {
            static EMPTY: Arc<[Statement]> = Arc::from(Vec::new());
        }
        ProcCont {
            stmts: EMPTY.with(Arc::clone),
            start: 0,
            next: None,
            loop_tail_at: None,
            repeat_remaining: None,
        }
    }

    /// Resume at `idx` of THIS frame, keeping the rest of the chain. This is the
    /// operation that used to be a deep clone. `loop_tail_at` names a position
    /// in THIS SAME `stmts` array, so it carries over unchanged — moving the
    /// cursor doesn't move the tail.
    pub(super) fn resume_at(&self, idx: usize) -> Self {
        ProcCont {
            stmts: Arc::clone(&self.stmts),
            start: idx.min(self.stmts.len()),
            next: self.next.clone(),
            loop_tail_at: self.loop_tail_at,
            repeat_remaining: self.repeat_remaining,
        }
    }

    /// Run `stmts` first, then this continuation from `resume_at`. Replaces
    /// "copy the caller's tail onto the end of the spliced body".
    pub(super) fn pushed(&self, stmts: Vec<Statement>, resume_at: usize) -> Self {
        // A frame with nothing left contributes NOTHING but a link. Splice the
        // rest of the chain in directly instead of wrapping it.
        //
        // This is what made suspend-aware `while`/`for` unbounded: the loop
        // re-pushes its continuation from the LAST statement of its own frame,
        // so every iteration wrapped an already-exhausted frame and the chain
        // grew by one link per iteration — O(N) memory, and a recursive `Drop`
        // of that list overflowed the stack at ~2000 iterations
        // (`for (int i=0;i<2000;i++) @(posedge clk);`). `repeat`/`forever` were
        // unaffected only because they have their own counted-waiter path.
        let next = if resume_at >= self.stmts.len() {
            self.next.clone()
        } else {
            Some(Arc::new(self.resume_at(resume_at)))
        };
        ProcCont {
            stmts: Arc::from(stmts),
            start: 0,
            next,
            loop_tail_at: None,
            repeat_remaining: None,
        }
    }

    /// Like `pushed(vec![body.clone()], resume_at)`, but when `body` is
    /// `StatementKind::Null` (a standalone `#delay;` or `@(ev);` with no
    /// attached statement), skips allocating the 1-element `[Null]` wrapper
    /// frame and `Arc<ProcCont>` link and resumes directly at `resume_at`.
    pub(super) fn pushed_body(&self, body: &Statement, resume_at: usize) -> Self {
        if matches!(body.kind, StatementKind::Null) {
            if resume_at < self.stmts.len() {
                return self.resume_at(resume_at);
            }
            if let Some(next) = &self.next {
                return (**next).clone();
            }
            thread_local! {
                static NULL_ONE: Arc<[Statement]> = Arc::from(vec![Statement::new(
                    StatementKind::Null,
                    crate::ast::Span::dummy(),
                )]);
            }
            return ProcCont {
                stmts: NULL_ONE.with(Arc::clone),
                start: 0,
                next: None,
                loop_tail_at: None,
                repeat_remaining: None,
            };
        }
        self.pushed(vec![body.clone()], resume_at)
    }

    /// Like `pushed`, but `stmts[tail_idx]` is a synthesized `While`/`Repeat`
    /// re-entry — the ONE occurrence of that kind in this frame allowed to
    /// consume a pending `break`/`continue` (see `loop_tail_at`'s doc
    /// comment on `ProcCont`).
    pub(super) fn pushed_with_tail(
        &self,
        stmts: Vec<Statement>,
        resume_at: usize,
        tail_idx: usize,
    ) -> Self {
        let mut c = self.pushed(stmts, resume_at);
        c.loop_tail_at = Some(tail_idx);
        c
    }

    /// Run a previously prepared statement frame, then resume this chain.
    pub(super) fn pushed_frame(&self, stmts: Arc<[Statement]>, resume_at: usize) -> Self {
        let next = if resume_at >= self.stmts.len() {
            self.next.clone()
        } else {
            Some(Arc::new(self.resume_at(resume_at)))
        };
        ProcCont {
            stmts,
            start: 0,
            next,
            loop_tail_at: None,
            repeat_remaining: None,
        }
    }

    /// Like `pushed_frame`, but `stmts[tail_idx]` is a synthesized loop
    /// re-entry (see `pushed_with_tail`).
    pub(super) fn pushed_frame_with_tail(
        &self,
        stmts: Arc<[Statement]>,
        resume_at: usize,
        tail_idx: usize,
    ) -> Self {
        let mut c = self.pushed_frame(stmts, resume_at);
        c.loop_tail_at = Some(tail_idx);
        c
    }

    /// This frame only, from the cursor.
    pub(super) fn frame(&self) -> &[Statement] {
        &self.stmts[self.start.min(self.stmts.len())..]
    }

    /// Nothing left here or behind.
    pub(super) fn is_exhausted(&self) -> bool {
        self.start >= self.stmts.len() && self.next.is_none()
    }

    /// Statements still owed across the whole chain. Walks the chain — keep it
    /// off hot paths; it exists for diagnostics and for the few callers that
    /// must hand a flat list to code outside the process executor.
    pub(super) fn len(&self) -> usize {
        let mut n = self.frame().len();
        let mut cur = self.next.as_ref();
        while let Some(f) = cur {
            n += f.frame().len();
            cur = f.next.as_ref();
        }
        n
    }

    pub(super) fn is_empty(&self) -> bool {
        self.is_exhausted()
    }

    /// The next statement this continuation would execute.
    pub(super) fn first(&self) -> Option<&Statement> {
        if let Some(s) = self.frame().first() {
            return Some(s);
        }
        let mut cur = self.next.as_ref();
        while let Some(f) = cur {
            if let Some(s) = f.frame().first() {
                return Some(s);
            }
            cur = f.next.as_ref();
        }
        None
    }

    /// Flatten the chain. Diagnostics / snapshot paths only. Materialize
    /// repeat counters that live outside the shared AST while executing.
    pub(super) fn to_vec(&self) -> Vec<Statement> {
        fn append(frame: &ProcCont, out: &mut Vec<Statement>) {
            let offset = out.len();
            out.extend_from_slice(frame.frame());
            if let (Some(tail), Some(remaining)) =
                (frame.loop_tail_at, frame.repeat_remaining)
                && tail >= frame.start
                && let Some(stmt) = out.get_mut(offset + tail - frame.start)
                && let StatementKind::Repeat { count, .. } = &mut stmt.kind
            {
                *count = Expression::new(
                    ExprKind::Number(NumberLiteral::Integer {
                        size: Some(64),
                        signed: false,
                        base: NumberBase::Decimal,
                        value: remaining.to_string(),
                        cached_val: crate::ast::cache::LiteralCache::new(None),
                    }),
                    count.span,
                );
            }
        }
        let mut out = Vec::new();
        append(self, &mut out);
        let mut cur = self.next.as_ref();
        while let Some(frame) = cur {
            append(frame, &mut out);
            cur = frame.next.as_ref();
        }
        out
    }

}

impl From<Vec<Statement>> for ProcCont {
    fn from(v: Vec<Statement>) -> Self {
        if v.is_empty() {
            return ProcCont::empty();
        }
        ProcCont::from_vec(v)
    }
}

/// Per-tick wall-time accumulators that the event_loop driver carries
/// across iterations.  Extracted from `event_loop` so `run_one_tick`
/// can be called as a unit.
#[derive(Default, Debug)]
pub(super) struct PerTickAccum {
    pub(super) t_settle: u64,
    pub(super) t_edges: u64,
    pub(super) t_nba: u64,
    pub(super) t_process: u64,
    pub(super) t_snap: u64,
    pub(super) t_sched: u64,
    pub(super) t_clocking: u64,
    pub(super) t_cond: u64,
    pub(super) t_late: u64,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct TimingWheel {
    pub(super) wheel: Vec<EventList>, // circular array of WHEEL_SIZE slots
    #[serde(with = "serde_big_array")]
    bitmap: [u64; BITMAP_WORDS], // occupancy bitmap: bit set = slot non-empty
    pub(super) overflow: BTreeMap<u64, EventList>, // far-future events
    /// Number of non-zero bitmap words, so `is_empty` is O(1) instead of a
    /// 64-word compare per tick.
    occupied_words: u32,
    /// Memoized `next_time()`: `None` = unknown (recompute). Invalidated by
    /// every mutator that can remove an event; `schedule` narrows it in
    /// place. The scan it replaces walked up to every bitmap word on each
    /// of the ~8 calls per tick.
    next_cache: std::cell::Cell<Option<Option<u64>>>,
    current_time: u64, // last known simulation time
    /// Multiset of pids currently scheduled somewhere in the wheel or
    /// overflow. Lets `has_pid` return O(1) instead of scanning all 256
    /// slots (and all overflow) on every `is_pid_suspended` check —
    /// which fires on every run_scheduled_process call (~115K times on c910).
    /// Count-based rather than set-based because the same pid can have
    /// multiple scheduled events at different times.
    ///
    /// Kept as a `HashMap` deliberately. Replacing it with a `Vec<u32>` indexed
    /// by pid (pids are consecutive small integers, so the hashing looked like
    /// pure overhead) measured NEUTRAL on both a 3-process and a 400-process
    /// load — and pids are never reused, so the dense form grows to the
    /// high-water pid and trades memory for nothing. Do not re-litigate without
    /// a benchmark that shows this map is hot.
    pid_counts: HashMap<usize, u32>,
    /// Entries `schedule_front` put at the head of `front_time`'s slot and
    /// not yet popped; `last_pop_front` says whether the latest `pop_front`
    /// returned one. A clock generator's FIFO rank counts only the events
    /// that were queued when it scheduled its toggle, never fork children
    /// inserted ahead of them later.
    front_time: u64,
    front_pending: u32,
    pub(super) last_pop_front: bool,
}

impl TimingWheel {
    pub(super) fn new() -> Self {
        let mut wheel = Vec::with_capacity(WHEEL_SIZE);
        for _ in 0..WHEEL_SIZE {
            wheel.push(VecDeque::new());
        }
        TimingWheel {
            wheel,
            bitmap: [0u64; BITMAP_WORDS],
            overflow: BTreeMap::new(),
            occupied_words: 0,
            next_cache: std::cell::Cell::new(None),
            current_time: 0,
            pid_counts: HashMap::default(),
            front_time: 0,
            front_pending: 0,
            last_pop_front: false,
        }
    }

    /// Pids queued in the wheel slot of `time`, in queue order.
    pub(super) fn slot_pids(&self, time: u64) -> Vec<usize> {
        self.wheel[Self::slot(time)].iter().map(|e| e.0).collect()
    }

    /// Stable-sort the wheel slot of `time` by a per-pid rank (pids without
    /// one keep their place ahead of the ranked ones).
    pub(super) fn reorder_slot(&mut self, time: u64, rank: &HashMap<usize, usize>) {
        self.wheel[Self::slot(time)]
            .make_contiguous()
            .sort_by_key(|e| rank.get(&e.0).map_or(0, |r| r + 1));
    }

    /// Number of events queued for exactly `time`.
    pub(super) fn len_at(&self, time: u64) -> usize {
        let mut n = self.overflow.get(&time).map_or(0, |l| l.len());
        if time >= self.current_time && time < self.current_time + WHEEL_SIZE as u64 {
            n += self.wheel[Self::slot(time)].len();
        }
        n
    }

    #[inline(always)]
    pub(super) fn slot(time: u64) -> usize {
        (time as usize) & (WHEEL_SIZE - 1)
    }

    /// Set bitmap bit for a slot.
    #[inline(always)]
    fn bitmap_set(&mut self, slot: usize) {
        let w = &mut self.bitmap[slot >> 6];
        if *w == 0 {
            self.occupied_words += 1;
        }
        *w |= 1u64 << (slot & 63);
    }

    /// Clear bitmap bit for a slot.
    #[inline(always)]
    fn bitmap_clear(&mut self, slot: usize) {
        let w = &mut self.bitmap[slot >> 6];
        if *w != 0 {
            *w &= !(1u64 << (slot & 63));
            if *w == 0 {
                self.occupied_words -= 1;
            }
        }
        self.next_cache.set(None);
    }

    /// Schedule an event at the given time.
    pub(super) fn schedule(&mut self, time: u64, pid: usize, stmts: ProcCont) {
        sim_dbg_eprintln!("[DEBUG] scheduling process {} at time {}", pid, time);
        *self.pid_counts.entry(pid).or_insert(0) += 1;
        self.narrow_next_cache(time);
        if time < self.current_time + WHEEL_SIZE as u64 {
            let s = Self::slot(time);
            self.wheel[s].push_back((pid, stmts));
            self.bitmap_set(s);
        } else {
            self.overflow
                .entry(time)
                .or_default()
                .push_back((pid, stmts));
        }
    }

    /// Schedule at the FRONT of `time`'s slot, before entries already queued
    /// there. §9.3.2: `fork` children start executing immediately, in the
    /// spawning time slot — queueing them behind events already pending for
    /// this timestamp (a clock toggle, say) let those run first, so a child
    /// whose first act is `@(posedge clk)` armed AFTER the edge it was meant
    /// to catch and slept a whole period.
    ///
    /// Children are inserted in reverse so the group keeps source order.
    pub(super) fn schedule_front(&mut self, time: u64, pid: usize, stmts: ProcCont) {
        *self.pid_counts.entry(pid).or_insert(0) += 1;
        if self.front_time != time {
            self.front_time = time;
            self.front_pending = 0;
        }
        self.front_pending += 1;
        self.narrow_next_cache(time);
        if time < self.current_time + WHEEL_SIZE as u64 {
            let s = Self::slot(time);
            self.wheel[s].push_front((pid, stmts));
            self.bitmap_set(s);
        } else {
            self.overflow
                .entry(time)
                .or_default()
                .push_front((pid, stmts));
        }
    }

    /// Put back `rest`, in order and ahead of anything queued at `time`
    /// since: events a batch took for `time` but has not run yet.
    pub(super) fn requeue_front(&mut self, time: u64, rest: EventList) {
        self.narrow_next_cache(time);
        for (pid, stmts) in rest.into_iter().rev() {
            *self.pid_counts.entry(pid).or_insert(0) += 1;
            if time < self.current_time + WHEEL_SIZE as u64 {
                let s = Self::slot(time);
                self.wheel[s].push_front((pid, stmts));
                self.bitmap_set(s);
            } else {
                self.overflow
                    .entry(time)
                    .or_default()
                    .push_front((pid, stmts));
            }
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.occupied_words == 0 && self.overflow.is_empty()
    }

    /// A new event at `time` can only move the next time earlier.
    #[inline(always)]
    fn narrow_next_cache(&self, time: u64) {
        if let Some(c) = self.next_cache.get() {
            let nt = c.map_or(time, |t| t.min(time));
            self.next_cache.set(Some(Some(nt)));
        }
    }

    /// Get the next scheduled time (minimum) using bitmap scan.
    /// Uses trailing_zeros to find the next occupied slot in O(1) per word.
    pub(super) fn next_time(&self) -> Option<u64> {
        if let Some(c) = self.next_cache.get() {
            // A cached time behind the wheel's clock means an event was left
            // behind (deferred drain); fall through to the exact scan.
            if c.map_or(true, |t| t >= self.current_time) {
                return c;
            }
        }
        let r = self.next_time_scan();
        self.next_cache.set(Some(r));
        r
    }

    fn next_time_scan(&self) -> Option<u64> {
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
            let delta = if slot >= start_slot {
                slot - start_slot
            } else {
                slot + WHEEL_SIZE - start_slot
            };
            return Some(self.current_time + delta as u64);
        }
        // Scan remaining words after start_word
        for w in 1..BITMAP_WORDS {
            let word_idx = (start_word + w) & (BITMAP_WORDS - 1);
            if self.bitmap[word_idx] != 0 {
                let bit = self.bitmap[word_idx].trailing_zeros() as usize;
                let slot = (word_idx << 6) | bit;
                let delta = if slot >= start_slot {
                    slot - start_slot
                } else {
                    slot + WHEEL_SIZE - start_slot
                };
                return Some(self.current_time + delta as u64);
            }
        }
        // Phase 3: wrap back to start_word's LOW bits (< start_bit). These are
        // the furthest-ahead slots in the wheel window — a nearly-full wrap
        // around — and belong AFTER every other word in scan order. Phase 1's
        // `!0 << start_bit` mask hid them and Phase 2 stops before returning to
        // start_word, so without this pass a lone event in this range is
        // INVISIBLE: e.g. current_time=250 (slot 250, word 3 bit 58) with an
        // event at time 500 (slot 244, word 3 bit 52) — same word, lower bit.
        // next_time() would then return None and the scheduler would end the
        // run early, silently dropping the event. (next_time_after scans all
        // 256 slots and never had this hole.)
        if start_bit != 0 {
            let last_masked = self.bitmap[start_word] & !(!0u64 << start_bit);
            if last_masked != 0 {
                let bit = last_masked.trailing_zeros() as usize;
                let slot = (start_word << 6) | bit;
                // slot < start_slot here, so the wrap delta is always positive.
                let delta = slot + WHEEL_SIZE - start_slot;
                return Some(self.current_time + delta as u64);
            }
        }
        // Check overflow
        self.overflow.keys().next().copied()
    }

    /// Earliest scheduled time STRICTLY greater than `after`, if any. Used to
    /// break a zero-delay (#0) livelock: when the current time slot can't make
    /// progress but real events exist ahead, jump to them.
    pub(super) fn next_time_after(&self, after: u64) -> Option<u64> {
        let base_slot = Self::slot(self.current_time);
        let mut best: Option<u64> = None;
        // Wheel events all live in [current_time, current_time+WHEEL_SIZE); each
        // occupied slot maps to exactly one time in that window.
        for slot in 0..WHEEL_SIZE {
            if self.bitmap[slot >> 6] & (1u64 << (slot & 63)) == 0 {
                continue;
            }
            let delta = (slot + WHEEL_SIZE - base_slot) % WHEEL_SIZE;
            let t = self.current_time + delta as u64;
            if t > after {
                best = Some(best.map_or(t, |b| b.min(t)));
            }
        }
        if let Some((&t, _)) = self.overflow.range((after + 1)..).next() {
            best = Some(best.map_or(t, |b| b.min(t)));
        }
        best
    }

    fn move_overflow_into_wheel(&mut self, time: u64) {
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
    }

    #[inline]
    fn decrement_pid_count(&mut self, pid: usize) {
        if let Some(c) = self.pid_counts.get_mut(&pid) {
            *c = c.saturating_sub(1);
            if *c == 0 {
                self.pid_counts.remove(&pid);
            }
        }
    }

    /// Remove and return the next FIFO event at `time`, leaving the remaining
    /// same-time events in the wheel. This avoids repeatedly draining and
    /// reinserting a large active-region batch for every process activation.
    pub(super) fn pop_front(&mut self, time: u64) -> Option<(usize, ProcCont)> {
        self.current_time = time;
        self.move_overflow_into_wheel(time);
        let s = Self::slot(time);
        let event = self.wheel[s].pop_front();
        if self.wheel[s].is_empty() {
            self.bitmap_clear(s);
        }
        self.last_pop_front = false;
        if let Some((pid, _)) = event.as_ref() {
            self.decrement_pid_count(*pid);
            if self.front_pending > 0 && self.front_time == time {
                self.front_pending -= 1;
                self.last_pop_front = true;
            }
        }
        event
    }

    /// Events currently queued at `time` (wheel slot + overflow). Captured
    /// once, before the first pop, to size the time-0 start batch for
    /// `t0_starts_remaining`.
    pub(super) fn slot_len(&self, time: u64) -> usize {
        self.wheel[Self::slot(time)].len() + self.overflow.get(&time).map_or(0, |v| v.len())
    }

    /// Remove and return all events at the given time.
    pub(super) fn remove(&mut self, time: u64) -> EventList {
        self.current_time = time;
        if self.front_time == time {
            self.front_pending = 0;
        }
        self.move_overflow_into_wheel(time);
        let s = Self::slot(time);
        let events = std::mem::take(&mut self.wheel[s]);
        if !events.is_empty() {
            // Only clear bitmap if slot is truly empty
            self.bitmap_clear(s);
            // Decrement pid_counts for each removed event.
            for (p, _) in &events {
                self.decrement_pid_count(*p);
            }
        }
        events
    }

    /// O(1) check: is any event scheduled for this pid anywhere in the queue?
    pub(super) fn has_pid(&self, pid: usize) -> bool {
        self.pid_counts.contains_key(&pid)
    }

    /// The pending continuation scheduled for `pid`, if any. Linear scan of
    /// every wheel slot + the overflow map — used ONLY by the terminal
    /// zero-delay stall report, never on the scheduling hot path.
    /// Every pending (pid, absolute_time) in the wheel + overflow. Read-only,
    /// used ONLY by the terminal stall report to show where non-spinning
    /// processes are parked (e.g. the initializer the spinner is waiting for,
    /// scheduled at a future time the livelock never reaches).
    pub(super) fn pending_pid_times(&self) -> Vec<(usize, u64)> {
        let mut out: Vec<(usize, u64)> = Vec::new();
        let cur_slot = Self::slot(self.current_time);
        for s in 0..WHEEL_SIZE {
            if self.wheel[s].is_empty() {
                continue;
            }
            let delta = ((s + WHEEL_SIZE - cur_slot) % WHEEL_SIZE) as u64;
            let abs_time = self.current_time + delta;
            for (p, _) in &self.wheel[s] {
                out.push((*p, abs_time));
            }
        }
        for (t, list) in &self.overflow {
            for (p, _) in list {
                out.push((*p, *t));
            }
        }
        out
    }

    pub(super) fn pending_stmts_for(&self, pid: usize) -> Option<&ProcCont> {
        if !self.has_pid(pid) {
            return None;
        }
        for slot in &self.wheel {
            for (p, s) in slot {
                if *p == pid {
                    return Some(s);
                }
            }
        }
        for list in self.overflow.values() {
            for (p, s) in list {
                if *p == pid {
                    return Some(s);
                }
            }
        }
        None
    }

    /// §9.7: remove ALL events for `pid` from both the wheel and overflow.
    /// Returns the earliest (scheduled_time, continuation) if found.
    /// Used by `process::suspend()` and `process::kill()` to desensitize a
    /// process from its delay.
    pub(super) fn remove_pid(&mut self, pid: usize) -> Option<(u64, ProcCont)> {
        if !self.pid_counts.contains_key(&pid) {
            return None;
        }
        self.next_cache.set(None);
        let mut found: Option<(u64, ProcCont)> = None;
        let cur_slot = Self::slot(self.current_time);

        // 1) Scan the timing wheel: extract matching entries, keep the rest.
        for s in 0..WHEEL_SIZE {
            let v = std::mem::take(&mut self.wheel[s]);
            if v.is_empty() {
                continue;
            }
            let delta = ((s + WHEEL_SIZE - cur_slot) % WHEEL_SIZE) as u64;
            let abs_time = self.current_time + delta;
            let mut kept = VecDeque::new();
            for (p, stmts) in v {
                if p == pid {
                    // Capture the earliest match.
                    if found.is_none() || abs_time < found.as_ref().unwrap().0 {
                        found = Some((abs_time, stmts));
                    }
                } else {
                    kept.push_back((p, stmts));
                }
            }
            if kept.is_empty() {
                self.bitmap_clear(s);
            } else {
                self.wheel[s] = kept;
            }
        }

        // 2) Scan overflow (BTreeMap is time-sorted, so first match is earliest).
        if found.is_none() {
            let times: Vec<u64> = self.overflow.keys().copied().collect();
            for t in times {
                if let Some(v) = self.overflow.get_mut(&t) {
                    if let Some(idx) = v.iter().position(|(p, _)| *p == pid) {
                        let stmts = v.remove(idx).expect("matching event disappeared").1;
                        found = Some((t, stmts));
                        if v.is_empty() {
                            self.overflow.remove(&t);
                        }
                        break;
                    }
                }
            }
        } else {
            // Also clean overflow of any remaining entries for this pid.
            for (_, v) in self.overflow.iter_mut() {
                v.retain(|(p, _)| *p != pid);
            }
            self.overflow.retain(|_, v| !v.is_empty());
        }
        self.pid_counts.remove(&pid);
        found
    }
}

/// Roadmap steps 11-12: a process compiled to bytecode WITH wait points.
/// The scheduler keeps only (pid -> frame); a resume re-enters `exec_insns`
/// at `pc` with the frame's registers swapped in, instead of re-walking the
/// AST continuation chain. Registered only for `always` bodies whose whole
/// statement tree compiles fallback-free (see `try_register_proc_fsm`).
/// Two-state form of a process FSM body (see `TwoStateBlock::has_wait`).
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct ProcFsmTs {
    pub(super) block: super::bytecode::TwoStateBlock,
    /// The FSM's own two-state register file, swapped into `ts_regs` for a
    /// segment; `valid` says it holds the current values (a VM segment
    /// makes the VM registers the truth again).
    pub(super) regs: Vec<u64>,
    pub(super) valid: bool,
    /// Copy taken at segment start: a bail re-runs the segment on the VM
    /// from these values.
    pub(super) snap: Vec<u64>,
    /// Back-off after repeated bails (reset-phase x): skip two-state
    /// attempts until `resumes` reaches `skip_until`.
    pub(super) fails: u32,
    pub(super) resumes: u32,
    pub(super) skip_until: u32,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct ProcFsm {
    pub(super) compiled: super::bytecode::CompiledBlock,
    pub(super) ts: Option<Box<ProcFsmTs>>,
    /// Signal ids the body can write (sorted), or None when it has array
    /// stores or AST fallbacks; lets `note_comb_ran_in_process` skip the
    /// snapshot of comb outputs this process can never clobber.
    pub(super) writes: Option<Arc<[u32]>>,
    /// Event-control specs for WaitEdge insns, with the lazily resolved
    /// sensitivity and its signal-id terms cached after the first suspension.
    pub(super) waits: Vec<(
        crate::ast::stmt::EventControl,
        Option<Vec<Sensitivity>>,
        Option<Vec<SensitivityId>>,
    )>,
    /// Per wait: a clocking event (`@(cb)`), whose waiter resumes in the
    /// Reactive region after the block samples (§14.13).
    pub(super) wait_is_clocking: Vec<bool>,
    pub(super) pc: u32,
    pub(super) regs: Vec<Value>,
    pub(super) scope: String,
    /// Module-precision diff for delay quantization (§3.14.3), resolved on
    /// first activation when the process scope is installed.
    pub(super) precision_diff: Option<i32>,
    /// `always` bodies restart at pc 0 on completion; `initial` bodies are
    /// done — the frame is dropped and the pid never reactivates.
    pub(super) is_always: bool,
    /// Roadmap step 13: rustc-native form of the body (XEZIM_AOT=1). The
    /// native fn loads/spills its registers from `native_frame` around each
    /// suspension; bytecode execution is the fallback when absent. A
    /// checkpoint leaves it out; a restore reattaches the one setup built.
    #[cfg(feature = "jit")]
    #[serde(skip)]
    pub(super) native: Option<super::aot::AotFsmFn>,
    #[cfg(feature = "jit")]
    pub(super) native_frame: Vec<u64>,
}

/// What a Wait insn suspended on: (payload). Paired with the resume pc in
/// `Simulator::fsm_suspend`.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) enum FsmWait {
    /// Delay in global ticks (already quantized).
    Delay(u64),
    /// Index into the frame's `waits` table.
    Edge(u32),
}

pub(super) type ClockingOutputDrive = (String, String, Value, Option<Expression>);

pub(super) type ClockingCycleDrive = (u64, String, String, Value, Option<Expression>);

/// §14.5: an expression a clockvar is bound to and its operand nets, each
/// with the hidden clocking name of its sample.
pub(super) type ClockingBoundExpr = (Expression, Vec<(String, String)>);

pub(super) type ClockingEdgeDrive = (String, Value, EdgeKind, u64, Option<Expression>);

/// One clocking block's per-tick poll state: its clock resolved to a
/// signal-table id once (None: fall back to the by-name read), the edge it
/// advances on, and the level seen on the previous tick.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct ClockingTick {
    pub(super) cb: String,
    pub(super) clk: String,
    pub(super) id: Option<usize>,
    pub(super) edge: EdgeKind,
    pub(super) prev: u8,
}

/// A callee's rename frames held aside while its actuals are evaluated
/// (`begin_formal_dyn` / `commit_formal_dyn`).
pub(super) type FormalDynSetup = (usize, Vec<(String, String)>, Vec<(String, String)>);

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct ProcessContext {
    pub(super) this_stack: Vec<Option<usize>>,
    pub(super) local_stack: Vec<HashMap<String, Value>>,
    /// Open block scopes of blocking `begin…end` blocks that declare
    /// variables (§6.21), innermost last; see `BlockScopeSave`. Rides process
    /// suspension like `local_stack`, since the block ends after a resume.
    pub(super) block_scope_saves: Vec<BlockScopeSave>,
    /// Parallel to `local_stack`: the frame generation each slot was pushed
    /// with. Two different method activations never share a generation even
    /// when they occupy the same slot index, so a fork child can tell whether
    /// a parent slot still holds the activation it forked from (see
    /// `merge_fork_writes`).
    pub(super) local_gen_stack: Vec<u64>,
    pub(super) local_type_stack: Vec<(HashMap<String, String>, HashMap<String, String>)>,
    pub(super) class_context_stack: Vec<Option<String>>,
    /// §8.25: a suspended static task retains its specialization and the
    /// enclosing bindings, independently of other processes running next.
    pub(super) current_spec: Option<(String, String)>,
    pub(super) spec_scope_stack: Vec<(String, String)>,
    pub(super) cg_this: Option<usize>,
    pub(super) return_value: Option<Value>,
    pub(super) break_flag: bool,
    pub(super) continue_flag: bool,
    pub(super) return_flag: bool,
    // Carried so a task that suspends mid-body keeps its full call context
    // (needed once blocking task/method calls are inlined into the process
    // statement stream — see `task_cleanup` and `StatementKind::ScopePop`).
    pub(super) local_iface_aliases: Vec<HashMap<String, String>>,
    pub(super) ref_binding_stack: Vec<HashMap<String, Expression>>,
    pub(super) ref_alias_stack: Vec<HashMap<String, String>>,
    /// §13.5.2 `ref` formals bound by IDENTITY (aggregates with no storage of
    /// their own); element accesses on them are rewritten onto the actual.
    /// Pushed/popped in lockstep with `ref_binding_stack`.
    pub(super) ref_identity_stack: Vec<Vec<String>>,
    pub(super) queue_frame_saves: Vec<Vec<(String, QueueLocalSave)>>,
    pub(super) task_cleanup: Vec<TaskCleanup>,
    // Per-call-frame map of a local dynamic-array/queue/assoc LOCAL's bare
    // name to a process-unique storage key (e.g. `edges` -> `@edges#7`).
    // SystemVerilog automatic locals are per-invocation: two concurrent
    // task calls each declaring `int edges[$]` must NOT share the global
    // `signals` keys `edges.size` / `edges[i]`. By renaming at the VarDecl
    // and resolving the bare name through this map (see `dyn_name_lookup`
    // + the `resolve_hier_name` early return), each invocation's data lives
    // under a distinct key. Stack of frames pushed/popped in sync with
    // `push_queue_frame`/`pop_and_restore_queue_frame`.
    pub(super) local_dyn: Vec<Vec<(String, String)>>,
    pub(super) formal_dyn: Vec<Vec<(String, String)>>,
    pub(super) static_local_syncs: Vec<(String, Vec<(String, String)>)>,
    /// See `Simulator::method_local_base`. This is per-PROCESS state: the
    /// base indexes into `local_stack`, which is swapped out with the rest
    /// of the context when a process parks. A process suspended inside an
    /// inlined BLOCKING class method used to leave its base behind, so
    /// another process's frame-local lookups (`local_class_type_of`,
    /// `local_typedef_type_of`, `get_expr_type_name`'s `in_any_frame`) were
    /// bounded by an index into a `local_stack` that was no longer there —
    /// a method-local shadowing a module-scope net then resolved to the net
    /// and `local = new()` constructed the wrong class.
    pub(super) method_local_base: Vec<usize>,
    /// §6.21: the lifetime of the subroutine this process is running (see
    /// `Simulator::current_static_task` / `current_auto_task`). A task that
    /// parks mid-body must not leave its lifetime to the process that runs
    /// next, nor lose it when it resumes.
    pub(super) current_static_task: Option<String>,
    pub(super) current_auto_task: Option<String>,
    /// See `Simulator::task_string_formals`.
    pub(super) task_string_formals: Vec<TaskFrameStrings>,
    /// See `Simulator::local_meta_claims`.
    pub(super) local_meta_claims: Vec<MetaClaim>,
}

/// Origin tag shared by fork creation and terminal metadata retirement.
pub(super) const FORK_CHILD_ORIGIN: &str = "fork child";
