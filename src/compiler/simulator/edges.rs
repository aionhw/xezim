//! Simulator edges.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    /// Group edge-sensitive signals that are exact copies of the same driver.
    ///
    /// A flattened netlist replicates the clock tree into per-module buffered
    /// copies. They all carry the same value and therefore all fire the same
    /// edge, but the detect scan visits each one separately and pays its own
    /// three scattered reads (`signal_table`, `prev_val`, `prev_xz`) to
    /// recompute an identical answer. Resolving each edge signal through the
    /// pure-copy relation to its driver lets the scan compute the edge ONCE per
    /// group (see `edge_group_memo`).
    ///
    /// Only these `CombItem`s count as a copy, and only under conditions that
    /// make the destination bit-for-bit equal to its source:
    /// `DirectCopy` / `FastDirectCopy` / `FastDirectFanout` / non-inverting
    /// `FusedBufFanout` / non-inverting `FusedGate::Buf1`. A computed clock
    /// (`Bin2`, `Mux2` — a clock gate or mux) is NOT a copy, but is a perfectly
    /// good shared ROOT for the copies hanging off it.
    ///
    /// Constraints, each of which would otherwise produce a wrong edge:
    /// * **Single writer.** A multiply-driven net does not track any one source.
    /// * **Equal width, <= 64 bits.** A resize changes the value; a wide signal
    ///   compares through `prev_wide`, not the inline `prev_val`/`prev_xz` pair.
    /// * **Matching 4-state-ness, not real.** A 2-state destination drops X/Z on
    ///   every write (§6.11.1), so it is not equal to a 4-state source.
    /// * **No SDF delay.** A delayed copy lags its source in time.
    /// * **Bit 0 on both sides** for the bit-addressed fused forms — bit 0 is
    ///   what posedge/negedge detection reads.
    /// * **Reached through >= 1 copy edge.** An independently driven ROOT stays
    ///   its own singleton. Folding it in would give it a spurious t=0 edge:
    ///   `prev` is baselined after the source initialises but before its copies
    ///   settle, so root and copy legitimately disagree for exactly one detect.
    pub(super) fn build_edge_sig_groups(&mut self) {
        self.edge_group_of.clear();
        self.edge_group_memo.clear();
        self.edge_group_count = 0;
        if std::env::var_os("XEZIM_NO_CLKTREE_DEDUP").is_some() || self.edge_signal_ids.is_empty() {
            return;
        }
        let nsig = self.signal_table.len();

        let mut writer_count: Vec<u32> = vec![0u32; nsig];
        for entry in &self.comb_entries {
            for &d in &entry.cold.write_signal_ids {
                if d < nsig {
                    writer_count[d] = writer_count[d].saturating_add(1);
                }
            }
        }

        // `copy_src[d]` = the signal `d` is an exact copy of, or -1.
        let mut copy_src: Vec<i64> = vec![-1i64; nsig];
        let eligible = |s: &Self, dst: usize, src: usize| -> bool {
            dst < nsig
                && src < nsig
                && dst != src
                && writer_count[dst] == 1
                && s.signal_widths[dst] <= 64
                && s.signal_widths[dst] == s.signal_widths[src]
                && !s.signal_real[dst]
                && !s.signal_real[src]
                && s.signal_two_state.get(dst).copied().unwrap_or(false)
                    == s.signal_two_state.get(src).copied().unwrap_or(false)
                && s.sdf_delays.get(dst).copied().unwrap_or(0) == 0
        };
        for entry in &self.comb_entries {
            match &entry.item {
                CombItem::DirectCopy { dst_id, src_id, .. }
                | CombItem::FastDirectCopy { dst_id, src_id } => {
                    if eligible(self, *dst_id, *src_id) {
                        copy_src[*dst_id] = *src_id as i64;
                    }
                }
                CombItem::FastDirectFanout { src_id, dst_ids } => {
                    for &d in dst_ids.iter() {
                        if eligible(self, d, *src_id) {
                            copy_src[d] = *src_id as i64;
                        }
                    }
                }
                CombItem::FusedBufFanout { src, dsts, invert } => {
                    if *invert || src.bit != 0 {
                        continue;
                    }
                    for d in dsts.iter() {
                        if d.bit == 0 && eligible(self, d.sig_id as usize, src.sig_id as usize) {
                            copy_src[d.sig_id as usize] = src.sig_id as i64;
                        }
                    }
                }
                CombItem::FusedGate {
                    op: FusedGate::Buf1 { dst, src, invert },
                } => {
                    if !*invert
                        && src.bit == 0
                        && dst.bit == 0
                        && eligible(self, dst.sig_id as usize, src.sig_id as usize)
                    {
                        copy_src[dst.sig_id as usize] = src.sig_id as i64;
                    }
                }
                _ => {}
            }
        }

        // Resolve each edge signal to its root, counting copy hops. Bounded so
        // a copy CYCLE (illegal RTL, but reachable through a mis-elaborated
        // generate loop) cannot spin here.
        const MAX_COPY_HOPS: usize = 64;
        let mut root_group: HashMap<usize, u32> = HashMap::default();
        let mut group_of: Vec<u32> = vec![u32::MAX; self.edge_signal_ids.len()];
        let mut next_group = 0u32;
        for (pos, &sid) in self.edge_signal_ids.iter().enumerate() {
            let mut cur = sid;
            let mut hops = 0usize;
            while hops < MAX_COPY_HOPS {
                match copy_src.get(cur).copied().unwrap_or(-1) {
                    -1 => break,
                    s => {
                        cur = s as usize;
                        hops += 1;
                    }
                }
            }
            // Reached through no copy at all -> independently driven root.
            if hops == 0 || hops >= MAX_COPY_HOPS {
                continue;
            }
            let g = *root_group.entry(cur).or_insert_with(|| {
                let g = next_group;
                next_group += 1;
                g
            });
            group_of[pos] = g;
        }

        // A group with a single member saves nothing and costs a memo slot.
        let mut members = vec![0u32; next_group as usize];
        for &g in &group_of {
            if g != u32::MAX {
                members[g as usize] += 1;
            }
        }
        for g in group_of.iter_mut() {
            if *g != u32::MAX && members[*g as usize] < 2 {
                *g = u32::MAX;
            }
        }

        self.edge_group_of = group_of;
        self.edge_group_count = next_group as usize;
        self.edge_group_memo = vec![EdgeGroupMemo::default(); next_group as usize];
        self.edge_group_generation = 0;
        if std::env::var_os("XEZIM_CLKTREE_STATS").is_some() {
            let grouped = self
                .edge_group_of
                .iter()
                .filter(|&&g| g != u32::MAX)
                .count();
            let live = members.iter().filter(|&&m| m >= 2).count();
            eprintln!(
                "[CLKTREE] edge signals={} grouped={} ({:.1}%) into {} groups; \
                 largest={} (probe with XEZIM_CLKTREE_PROBE=1)",
                self.edge_signal_ids.len(),
                grouped,
                100.0 * grouped as f64 / self.edge_signal_ids.len().max(1) as f64,
                live,
                members.iter().copied().max().unwrap_or(0),
            );
        }
    }

    /// Fast-path edge-detect when the only signals that could have an edge
    /// this iter are the clocks that just toggled in `fire_clock_generators`.
    /// The caller (event_loop) is responsible for proving this — i.e. that
    /// no apply_nba / settle / process / delayed-update wrote any non-clock
    /// signal since the last snap.  Reduces detect scan from
    /// O(|edge_signal_ids|) ~10k to O(|toggled_clocks|) ~a handful on c910.
    pub(super) fn check_edges_clocks_only(&mut self) {
        // Clone the tiny scratch buffer so the borrow checker is happy when
        // we hand the slice into `check_edges_inner` (which needs &mut self).
        let positions = self.toggled_clock_positions.clone();
        self.check_edges_inner(Some(&positions), false);
        self.prof_clocks_only_detect += 1;
        self.drain_edge_exec_rescan();
    }

    /// Edge-detect + dispatch core.  When `detect_subset` is `Some`, scans
    /// only those positions in `edge_signal_ids` instead of the full range —
    /// see `check_edges_clocks_only`.  The dispatch / exec / cascade-prep
    /// path is identical either way.
    /// §19.5 event-driven covergroup sampling for the edges fired this
    /// pass. Split out of check_edges_inner so waiters-first mode can run it
    /// before process continuations (and classic mode after edge blocks).
    pub(super) fn sample_edge_covergroups(&mut self) {
        let _t_cg = self.profile_timing.then(std::time::Instant::now);
        for i in 0..self.cg_event_waiters.len() {
            let handle = self.cg_event_waiters[i].0;
            let mut triggered = None;
            for j in 0..self.cg_event_waiters[i].1.len() {
                let sid = &self.cg_event_waiters[i].1[j];
                if self.check_edge_id(sid.signal_id, sid.edge) {
                    triggered = Some(sid.iff.clone());
                    break;
                }
            }
            // §9.4.2.3: `@(posedge clk iff en)` samples only while `en` holds.
            if let Some(iff) = triggered {
                self.sample_covergroup_guarded(handle, iff.as_deref());
            }
        }
        if let Some(t) = _t_cg {
            self.prof_edge_cg += t.elapsed().as_nanos() as u64;
        }
    }

    /// Fill `sig_wake_rank` and `name_wake_rank`: the scheduling depth that
    /// orders the waiters one drain wakes. §4.7 leaves the order open; this
    /// is the reference simulator's, measured. Waiters on a procedurally
    /// written signal run first. A continuous assignment's update is its own
    /// active event (§10.3), so its target's waiters run one step later. An
    /// input port whose actual is procedurally written adds a step (two for
    /// an interface port); one whose actual is itself a net, or an output
    /// port, shares its actual's list and adds none. Only comb-driven
    /// signals and collapsed port names are stored; the rest are depth 0.
    pub(super) fn build_sig_wake_rank(&mut self) {
        self.wake_rank_built = true;
        let top_prefix = format!("{}.", self.module.name);
        let iface_insts: HashSet<&str> = self
            .module
            .instances
            .iter()
            .filter(|i| self.module.interfaces.contains(&i.def_name))
            .map(|i| i.path.as_str())
            .collect();
        let port_step = |formal: &str| -> u8 {
            let f = formal.strip_prefix(top_prefix.as_str()).unwrap_or(formal);
            let inst = f.rsplit_once('.').map_or("", |(p, _)| p);
            if iface_insts.contains(inst) { 2 } else { 1 }
        };
        // Whole-net identity connections not collapsed onto one id, keyed by
        // the driven side: dst -> (src, step). An input steps from its
        // actual to the formal, an output from the formal to its actual (no
        // step). Formals collapsed onto their actual are ranked by name below.
        const OUT: u8 = u8::MAX;
        let mut port_dst: HashMap<usize, (usize, u8)> = HashMap::default();
        let mut collapsed: Vec<(&str, &str)> = Vec::new();
        for (formal, actual) in &self.module.port_aliases {
            let (Some(&f), Some(&a)) = (
                self.signal_name_to_id.get(formal.as_str()),
                self.signal_name_to_id.get(actual.as_str()),
            ) else {
                continue;
            };
            if f == a {
                // Only the input-port pass collapses a FORMAL onto its actual.
                if self.collapsed_port_children.contains(formal.as_str()) {
                    collapsed.push((formal.as_str(), actual.as_str()));
                }
            } else {
                port_dst.insert(f, (a, port_step(formal)));
                port_dst.insert(a, (f, OUT));
            }
        }
        let step_of = |s: usize, rs: u8, w: usize| -> u8 {
            match port_dst.get(&w) {
                Some(&(src, OUT)) if src == s => 0,
                Some(&(src, st)) if src == s => {
                    if rs == 0 {
                        st
                    } else {
                        0
                    }
                }
                _ => 1,
            }
        };
        // Comb-driven signals start unreached; everything else is depth 0.
        let entries = &self.comb_entries;
        let mut rank: HashMap<usize, u8> = HashMap::default();
        for e in entries.iter() {
            for &w in &e.cold.write_signal_ids {
                rank.insert(w, u8::MAX);
            }
        }
        // Shortest weighted distance from the undriven signals, one bucket
        // per depth. Weights are 0..=2 and never negative, so an entry is
        // final the first time its shallowest input reaches it.
        const CAP: usize = 250;
        let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); CAP + 1];
        let mut entry_done = vec![false; entries.len()];
        let relax = |ei: usize,
                     s: usize,
                     rs: u8,
                     rank: &mut HashMap<usize, u8>,
                     buckets: &mut Vec<Vec<usize>>| {
            for &w in &entries[ei].cold.write_signal_ids {
                let nr = (rs as usize + step_of(s, rs, w) as usize).min(CAP);
                if let Some(cur) = rank.get_mut(&w) {
                    if nr < *cur as usize {
                        *cur = nr as u8;
                        buckets[nr].push(w);
                    }
                }
            }
        };
        // Depth 0: entries with an undriven (or no) input.
        for (ei, e) in entries.iter().enumerate() {
            let free = if e.cold.read_signal_ids.is_empty() {
                Some(usize::MAX)
            } else {
                e.cold
                    .read_signal_ids
                    .iter()
                    .copied()
                    .find(|r| !rank.contains_key(r))
            };
            if let Some(s) = free {
                entry_done[ei] = true;
                relax(ei, s, 0, &mut rank, &mut buckets);
            }
        }
        let offs = &self.comb_dep_offsets;
        let deps = &self.comb_dep_entries;
        for r in 0..=CAP {
            let mut i = 0;
            while i < buckets[r].len() {
                let s = buckets[r][i];
                i += 1;
                if rank.get(&s).copied() != Some(r as u8) || s + 1 >= offs.len() {
                    continue;
                }
                for &ei in &deps[offs[s] as usize..offs[s + 1] as usize] {
                    let ei = ei as usize;
                    if ei < entry_done.len() && !entry_done[ei] {
                        entry_done[ei] = true;
                        relax(ei, s, r as u8, &mut rank, &mut buckets);
                    }
                }
            }
        }
        // Driven only from a feedback loop with no free input: one step.
        for r in rank.values_mut() {
            if *r == u8::MAX {
                *r = 1;
            }
        }
        rank.retain(|_, r| *r != 0);
        // Collapsed input ports: the actual's depth, plus the port's step
        // when that actual is procedurally written. Chains resolve through
        // the actual's own entry.
        let mut names: HashMap<String, u8> = HashMap::default();
        let links: HashMap<&str, &str> = collapsed.iter().copied().collect();
        for &(formal, _) in &collapsed {
            // Up the chain to the first name whose depth is known.
            let mut chain: Vec<&str> = Vec::new();
            let mut cur = formal;
            let mut r = loop {
                if let Some(&r) = names.get(cur) {
                    break r;
                }
                match links.get(cur) {
                    Some(&a) if !chain.contains(&cur) && chain.len() < 64 => {
                        chain.push(cur);
                        cur = a;
                    }
                    _ => {
                        break self
                            .signal_name_to_id
                            .get(cur)
                            .and_then(|id| rank.get(id).copied())
                            .unwrap_or(0);
                    }
                }
            };
            for &f in chain.iter().rev() {
                if r == 0 {
                    r = port_step(f);
                }
                names.insert(f.to_string(), r);
            }
        }
        self.sig_wake_rank = rank;
        self.name_wake_rank = names;
    }

    /// Detect which parked event waiters' sensitivities fired (against
    /// their own captured_prev baselines) and remove them from the parked
    /// list, refreshing the baseline of everyone left. Returns the (pid,
    /// continuation) pairs to run. Shared by the classic post-edge-block
    /// drain and the commercial-order pre-edge-block drain (see
    /// XEZIM_WAITERS_FIRST in check_edges_inner).
    pub(super) fn drain_triggered_event_waiters(&mut self) -> Vec<(usize, ProcCont)> {
        if self.event_waiters.is_empty() {
            return Vec::new();
        }
        let mut waiters = std::mem::take(&mut self.event_waiters);
        self.prof_waiter_iters += waiters.len() as u64;
        let mut triggered_conts = std::mem::take(&mut self.triggered_conts_buf);
        triggered_conts.clear();
        // Wake rank of each entry of `triggered_conts`, kept only from the
        // first nonzero rank on: empty means every wakeup is depth 0.
        let mut ranks: Vec<u8> = Vec::new();
        // In place: a parked waiter that does not fire stays where it is
        // (the old drain moved every waiter through a swap vector each tick).
        waiters.retain_mut(|waiter| {
            let mut triggered = false;
            let mut rank = 0u8;
            let mut moved = false;
            let mut deferred_ix = usize::MAX;
            let saved_hint = waiter
                .value_ctx
                .as_ref()
                .map(|c| self.name_resolve_hint.replace(c.scope.clone()));
            for (i, sid) in waiter.resolved_sensitivities.iter().enumerate() {
                let (pv, px) = waiter.captured_prev[i];
                let pw = waiter.captured_prev_wide[i].as_ref();
                if !self.term_fires(sid, pv, px, pw) {
                    continue;
                }
                moved = true;
                // LRM §9.4.2.3: `@(posedge clk iff g)` only fires when the
                // guard `g` holds at edge time. A false guard re-arms the
                // waiter (it stays in event_waiters for the next edge)
                // rather than resuming the process. The guard is evaluated
                // in the WAITING PROCESS's context (`eval_waiter_condition`
                // swaps in that pid's parked class frames + scope): a
                // class-task guard like `vif.sig_grant[master_id] === 1`
                // reads its virtual interface through the object's `this`
                // frame and `master_id` from the parked locals — a bare
                // module-scope eval cannot resolve either name and reads X,
                // so the waiter would never fire (ubus arbitration).
                let guard_ok = match sid.iff.as_deref() {
                    // §14.13: a CLOCKING event (`@(cb iff g)`) occurs after
                    // the block samples, so its guard must read this edge's
                    // clockvars — judged in the Reactive region instead
                    // (`resolve_deferred_clocking_guards`). Evaluated here it
                    // read the previous edge's sample.
                    Some(_) if waiter.is_clocking => true,
                    // Fast path: a waiter with no parked context, drained
                    // from the top level, sees the module scope directly.
                    Some(g)
                        if !self.process_contexts.contains_key(&waiter.pid)
                            && self.process_context_is_default() =>
                    {
                        self.eval_expr(g).is_true()
                    }
                    Some(g) => self.eval_waiter_condition(waiter.pid, g),
                    None => true,
                };
                // §9.4.2: a non-trivial event expression fires only when its
                // VALUE changed since arm — `a=2;b=1` leaving `a+b` at 3 is
                // not an event, however many operands moved — and an edge
                // (`@(posedge v[3])`) is judged on that value's LSB.
                let value_ok = match (&sid.value_of, waiter.guard_prev.get(i)) {
                    (Some(e), Some(Some(prev))) => {
                        let e = e.clone();
                        let prev = prev.clone();
                        let cur = self.eval_expr(&e);
                        value_event_fires(sid.edge, &prev, &cur)
                    }
                    _ => true,
                };
                if guard_ok && value_ok {
                    triggered = true;
                    rank = sid.wake_rank;
                    if waiter.is_clocking && sid.iff.is_some() {
                        deferred_ix = i;
                    }
                    break;
                }
            }
            if deferred_ix != usize::MAX {
                self.hold_clocking_guard(waiter, deferred_ix);
                if let Some(h) = saved_hint {
                    *self.name_resolve_hint.borrow_mut() = h;
                }
                return false;
            }
            if triggered && waiter.remaining_events > 1 {
                waiter.remaining_events -= 1;
                triggered = false;
            }
            let keep = if triggered {
                sim_dbg_eprintln!(
                    "[DEBUG] waiter for process {} triggered at time {}",
                    waiter.pid,
                    self.time
                );
                if waiter.is_clocking {
                    // §14.13: resume in the Reactive region, not here in the
                    // Active region — defer the continuation past apply_nba +
                    // tick_clocking_blocks so it reads post-edge state and this
                    // cycle's clocking samples.
                    let cont = std::mem::replace(&mut waiter.continuation, ProcCont::empty());
                    self.deferred_clocking_conts.push((waiter.pid, cont));
                } else {
                    let cont = std::mem::replace(&mut waiter.continuation, ProcCont::empty());
                    if rank != 0 || !ranks.is_empty() {
                        if ranks.capacity() == 0 {
                            ranks = std::mem::take(&mut self.wake_rank_buf);
                            ranks.clear();
                        }
                        ranks.resize(triggered_conts.len(), 0);
                        ranks.push(rank);
                    }
                    triggered_conts.push((waiter.pid, cont));
                }
                false
            } else {
                // Refresh this waiter's `captured_prev` baseline to each
                // sensitivity signal's CURRENT value so that a qualifying
                // edge occurring over multiple steps is detected.
                //
                // Without this, a waiter armed while its signal sits at the
                // edge-target level can never see the NEXT edge: e.g. a
                // process resumes on the first `@(posedge clk)` at t=5
                // (clk=1) and immediately re-arms on the next line; its
                // captured_prev is frozen at 1, so when clk later goes
                // 1→0→1 the Posedge test `!pb_one && cb_one` stays false
                // (pb_one clings to the stale 1) and the second posedge is
                // lost — the process strands (see
                // tests/bound_module / sequential_event_waits). Tracking the
                // running level here preserves the same-tick NBA-region
                // semantics captured_prev was added for (a waiter still
                // won't fire on an edge that completed BEFORE it armed, since
                // at arm time captured_prev already equals current), while
                // catching cross-tick transitions through the target level.
                for (i, sid) in waiter.resolved_sensitivities.iter().enumerate() {
                    waiter.captured_prev[i] = self.term_raw_bits(sid.signal_id);
                    if let Some(v) = self.term_wide_value(sid.signal_id) {
                        waiter.captured_prev_wide[i] = Some(v);
                    }
                }
                // An operand moved but no expression term fired: re-baseline
                // the expression values (an edge may arrive in steps), and
                // follow a moved index to the cell it selects now.
                if moved && waiter.value_ctx.is_some() {
                    self.rebase_value_terms(waiter);
                }
                true
            };
            if let Some(h) = saved_hint {
                *self.name_resolve_hint.borrow_mut() = h;
            }
            keep
        });
        self.event_waiters = waiters;
        // Within-region process resumption order is LRM-indeterminate
        // (§4.7), but the reference simulator wakes the LAST-armed waiter
        // first — and this campaign matches the reference's observable
        // ordering so differential runs stay comparable. Registration order
        // is FIFO; reverse to LIFO at the single hand-off point.
        triggered_conts.reverse();
        // §4.4/§10.3: a continuous assignment's update is its own active
        // event, so a net it drives changes — and wakes its waiters — only
        // after every waiter of its source has been scheduled. Order the
        // wakeups by scheduling depth (`build_sig_wake_rank`), keeping LIFO
        // within a depth.
        if !ranks.is_empty() {
            self.order_wakeups_by_rank(&mut triggered_conts, ranks);
        }
        triggered_conts
    }

    /// §9.4.2: an operand of `w`'s expression terms moved without an event.
    /// Take the expressions' current values as the new baseline (an edge on
    /// the value may arrive in several steps) and, when they select array
    /// cells through an index, watch the cells the indices name now — the
    /// index expression is re-evaluated, so `@(mem[i])` follows `i`.
    /// Runs with the waiter's scope as the resolution hint.
    pub(super) fn rebase_value_terms(&mut self, w: &mut EventWaiter) {
        if w.value_ctx.as_ref().is_some_and(|c| c.cells) {
            let mut terms = std::mem::take(&mut w.resolved_sensitivities);
            // `resolve_cell_terms` keeps the other terms in place and puts
            // the cells after them.
            let fixed = terms.iter().take_while(|t| !t.cell).count();
            self.resolve_cell_terms(&mut terms);
            w.captured_prev.truncate(fixed);
            w.captured_prev_wide.truncate(fixed);
            w.arm_bits.truncate(fixed);
            w.guard_prev.truncate(fixed);
            for t in &terms[fixed..] {
                let bits = self.term_raw_bits(t.signal_id);
                w.captured_prev.push(bits);
                w.arm_bits.push(bits);
                w.captured_prev_wide.push(self.term_wide_value(t.signal_id));
                w.guard_prev.push(None);
            }
            w.resolved_sensitivities = terms;
        }
        let mut last: Option<(crate::ast::Span, Value)> = None;
        for i in 0..w.resolved_sensitivities.len() {
            let Some(e) = w.resolved_sensitivities[i].value_of.as_ref() else {
                continue;
            };
            let v = match &last {
                Some((sp, v)) if *sp == e.span => v.clone(),
                _ => {
                    let v = self.eval_expr(e);
                    last = Some((e.span, v.clone()));
                    v
                }
            };
            w.guard_prev[i] = Some(v);
        }
    }

    /// Stable sort of one drain's wakeups (already LIFO) by `ranks`, their
    /// registration-order wake ranks. Out of line: most drains wake depth-0
    /// waiters only and never get here.
    #[cold]
    #[inline(never)]
    pub(super) fn order_wakeups_by_rank(
        &mut self,
        conts: &mut Vec<(usize, ProcCont)>,
        mut ranks: Vec<u8>,
    ) {
        if ranks.len() > 1 && ranks.iter().any(|&r| r != ranks[0]) {
            ranks.reverse();
            let mut keyed: Vec<(u8, (usize, ProcCont))> =
                ranks.iter().copied().zip(conts.drain(..)).collect();
            keyed.sort_by_key(|k| k.0);
            conts.extend(keyed.into_iter().map(|k| k.1));
        }
        self.wake_rank_buf = ranks;
    }

    /// Hand a consumed `drain_triggered_event_waiters` vector back for reuse.
    #[inline]
    pub(super) fn recycle_conts(&mut self, mut v: Vec<(usize, ProcCont)>) {
        if v.capacity() > self.triggered_conts_buf.capacity() {
            v.clear();
            self.triggered_conts_buf = v;
        }
    }

    /// §16.9.3 `$rose`/`$fell`/`$stable`/`$changed` from the current and the
    /// previous tick's sampled values. `$rose` is a least-significant bit
    /// that is 1 now and was anything else (0, x or z) before, `$fell` the
    /// same for 0; `$stable` compares the whole values with `===` (an x/z bit
    /// must match exactly). With no previous sample nothing has changed.
    pub(super) fn sampled_change_fn(name: &str, cur: &Value, prev: Option<&Value>) -> bool {
        let Some(prev) = prev else {
            return name == "$stable";
        };
        match name {
            "$rose" => cur.get_bit(0) == LogicBit::One && prev.get_bit(0) != LogicBit::One,
            "$fell" => cur.get_bit(0) == LogicBit::Zero && prev.get_bit(0) != LogicBit::Zero,
            "$stable" => cur.case_eq(prev).to_u64() == Some(1),
            _ => cur.case_eq(prev).to_u64() != Some(1),
        }
    }

    /// The activation scope a sampled-value call site's history belongs to:
    /// the executing block's instance scope ("" at top level). One source
    /// call in a child module is one site per INSTANCE — keyed by span alone,
    /// every instance shared the first one's clock and history, so a
    /// `$rose(x)` in a second instance clocked on another net never fired.
    pub(super) fn sampled_scope(&self) -> String {
        self.activation_scope.borrow().clone().unwrap_or_default()
    }

    /// The entry recorded for `site` under `scope`, else — with `fallback`,
    /// for a site seen from a single scope only — that one entry (a block
    /// run outside its own activation scope). Registration passes no
    /// fallback: there, a missing entry means another instance.
    pub(super) fn sampled_site_lookup<T: Copy>(
        map: &HashMap<usize, Vec<(Box<str>, T)>>,
        site: usize,
        scope: &str,
        fallback: bool,
    ) -> Option<T> {
        let v = map.get(&site)?;
        v.iter()
            .find(|(s, _)| **s == *scope)
            .map(|(_, t)| *t)
            .or_else(|| (fallback && v.len() == 1).then(|| v[0].1))
    }

    /// §16.9.3 sampled-value function with an EXPLICIT clocking argument
    /// (`$rose(x, @(posedge clk))`) or, in a procedural context, the
    /// module's `default clocking`. Returns `(sample_at_latest_edge,
    /// sample_n_edges_back)`; None -> caller uses its legacy (inferred/SVA)
    /// path.
    pub(super) fn sampled_fn_via_watch(
        &mut self,
        expr: &Expression,
        args: &[Expression],
        n: usize,
    ) -> Option<(Value, Option<Value>)> {
        // Explicit `@(...)` argument anywhere in the arg list.
        let mut edge_code: Option<u8> = None;
        let mut clk_id: Option<usize> = None;
        for a in args.iter().skip(1) {
            if let Some((ec, clk)) = Self::clocking_arg_marker(a) {
                edge_code = Some(ec);
                clk_id = self.get_lhs_signal_id(clk);
                break;
            }
        }
        let implicit_site = edge_code
            .is_none()
            .then_some(self.active_sva_site)
            .flatten();
        if let Some(si) = implicit_site {
            let site = &self.sva_sites[si];
            edge_code = Some(match site.edge {
                1 => 2,
                2 => 0,
                _ => 1,
            });
            clk_id = self
                .signal_name_to_id
                .get(site.clock_signal.as_str())
                .copied()
                .or(Some(usize::MAX));
        }
        if edge_code.is_none() {
            // Procedural context + default clocking: infer from it.
            // No default clocking: fall back to the clock inferred from the
            // enclosing always block. Without this, procedural `$rose(a)`
            // inside `always @(posedge clk)` had no history at all — `prev`
            // defaulted to `cur`, so $rose/$fell were ALWAYS 0 and $stable
            // ALWAYS 1, silently.
            if !self.clocking_meta.contains_key("__xz_default_clocking") {
                let (ec, cid) = Self::sampled_site_lookup(
                    &self.sampled_site_clock,
                    expr.span.start,
                    self.activation_scope.borrow().as_deref().unwrap_or(""),
                    !self.sampled_registering,
                )?;
                edge_code = Some(ec);
                clk_id = Some(cid);
            }
            if edge_code.is_none() {
                let (clk_str, _) = self.clocking_meta.get("__xz_default_clocking")?.clone();
                // clocking_meta stores the bare clock SIGNAL name (the parser
                // drops the event's edge); default clocking events are
                // overwhelmingly posedge — same assumption the `##N` cycle-delay
                // machinery makes.
                edge_code = Some(1);
                clk_id = self.signal_name_to_id.get(clk_str.trim()).copied();
            }
        }
        let clk_id = clk_id?;
        let operand = args.first()?.clone();
        let site = expr.span.start;
        let widx = self.sampled_watch_for(
            site,
            &operand,
            edge_code.unwrap_or(0),
            clk_id,
            n,
            implicit_site,
        )?;
        if let ExprKind::SystemCall { name, .. } = &expr.kind {
            if name == "$past" {
                self.sampled_watches[widx].gate = args
                    .get(2)
                    .filter(|a| !matches!(a.kind, ExprKind::Null | ExprKind::Empty))
                    .cloned();
                if self.sampled_registering
                    && let Some(g) = args.get(2)
                {
                    let mut ids = Vec::new();
                    self.collect_sva_signal_ids(g, &mut ids);
                    self.add_sampled_watch_ids(widx, &ids);
                }
            }
        }
        Some(self.sampled_cur_and_past(widx, &operand, n))
    }

    /// (current sample, `n` edges back) for a sampled-value call. The per-slot
    /// sampler runs after the Active processes and before the SVA sites, so
    /// when the reader runs BEFORE this slot's sample the front of the
    /// history is the PREVIOUS edge: the current value is read live and the
    /// past index shifts by one. A procedural `$past(v)` at the second edge
    /// used to index past that single sample and report "no history", and
    /// from the third edge on returned the value from two edges back.
    pub(super) fn sampled_cur_and_past(
        &mut self,
        widx: usize,
        operand: &Expression,
        n: usize,
    ) -> (Value, Option<Value>) {
        let sampled_this_slot = self.sampled_watches[widx].last_sample_time == Some(self.time);
        // Off the clock's tick the current sample is still the operand's
        // PREPONED value in this slot (§16.5.1), not its live one: an
        // operand written earlier in the slot must not count yet.
        let w = &self.sampled_watches[widx];
        let cur = match w.hist.front() {
            Some(v) if sampled_this_slot => v.clone(),
            _ => match w.sig_id {
                Some(_) if w.slot_time == Some(self.time) && w.slot_val.is_some() => {
                    w.slot_val.clone().unwrap()
                }
                Some(sid) => self.slot_preponed(sid),
                None => {
                    let op = operand.clone();
                    self.eval_expr(&op)
                }
            },
        };
        let idx = if sampled_this_slot {
            n
        } else {
            n.saturating_sub(1)
        };
        let past = if n == 0 {
            Some(cur.clone())
        } else {
            // §16.9.3: a tick before the first one samples the operand's
            // default value (its declaration value, else its type's).
            let w = &self.sampled_watches[widx];
            w.hist.get(idx).or(w.default.as_ref()).cloned()
        };
        (cur, past)
    }

    /// Fill `sampled_defaults` before the time-0 settle, while every variable
    /// still holds its declaration value or its type's default (§16.5.1). A
    /// concurrent assertion registers after the settle, and one in a
    /// procedure only when the procedure reaches it, so its operands are
    /// found here by name: every signal whose last name segment a concurrent
    /// assertion (its action blocks included) or a named property or
    /// sequence spells. That needs no scope resolution, and an extra match
    /// only costs its entry.
    pub(super) fn capture_sampled_defaults(&mut self) {
        let mut names: HashSet<String> = HashSet::default();
        for b in &self.module.initial_blocks {
            Self::sva_stmt_names(&b.stmt, false, &mut names);
        }
        for p in &self.module.pending_initial {
            Self::sva_stmt_names(&p.source, false, &mut names);
        }
        for b in self.edge_blocks.iter() {
            Self::sva_stmt_names(&b.stmt, false, &mut names);
        }
        for (st, ..) in &self.proc_sampled_blocks {
            Self::sva_stmt_names(st, false, &mut names);
        }
        for body in self.module.property_decls.values() {
            Self::sva_expr_names(body, &mut names);
        }
        if names.is_empty() {
            return;
        }
        for (name, &id) in self.signal_name_to_id.iter() {
            // An array cell is named `mem[3]`; an assertion spells `mem`. The
            // cells of a memory too large for per-element names (the packed
            // arena) are not in this map and keep their current value: only
            // an `always_comb` or continuous assignment writing one at time
            // 0 tells the two apart, and copying such a memory is not free.
            let leaf = name.rsplit(['.', ':']).next().unwrap_or_default();
            if !names.contains(leaf.split('[').next().unwrap_or_default()) {
                continue;
            }
            if let Some(v) = self.signal_table.get(id) {
                self.sampled_defaults.insert(id, v.clone());
            }
        }
    }

    /// The name segments the concurrent assertions in `st` spell, in every
    /// statement that can hold one. With `all` (inside a concurrent
    /// assertion's action block, whose sampled-value calls the assertion
    /// clocks) the names of the statements' expressions count too.
    pub(super) fn sva_stmt_names(st: &Statement, all: bool, out: &mut HashSet<String>) {
        use crate::ast::stmt::StatementKind as SK;
        match &st.kind {
            SK::Assertion(a) => {
                let sva = a.is_property || a.is_sequence;
                if sva || all {
                    Self::sva_expr_names(&a.expr, out);
                }
                for s in [&a.action, &a.else_action].into_iter().flatten() {
                    Self::sva_stmt_names(s, all || sva, out);
                }
            }
            SK::Expr(e) if all => Self::sva_expr_names(e, out),
            SK::BlockingAssign { rvalue, .. } | SK::NonblockingAssign { rvalue, .. } if all => {
                Self::sva_expr_names(rvalue, out)
            }
            SK::SeqBlock { stmts, .. } | SK::ParBlock { stmts, .. } => {
                stmts.iter().for_each(|s| Self::sva_stmt_names(s, all, out))
            }
            SK::If {
                condition,
                then_stmt,
                else_stmt,
                ..
            } => {
                if all {
                    Self::sva_expr_names(condition, out);
                }
                Self::sva_stmt_names(then_stmt, all, out);
                if let Some(e) = else_stmt {
                    Self::sva_stmt_names(e, all, out);
                }
            }
            SK::Case { expr, items, .. } => {
                if all {
                    Self::sva_expr_names(expr, out);
                }
                items
                    .iter()
                    .for_each(|it| Self::sva_stmt_names(&it.stmt, all, out))
            }
            SK::RandCase { items } => items
                .iter()
                .for_each(|(_, s)| Self::sva_stmt_names(s, all, out)),
            SK::TimingControl { stmt, .. } | SK::Wait { stmt, .. } => {
                Self::sva_stmt_names(stmt, all, out)
            }
            SK::WaitOrder { pass, fail, .. } => {
                for s in [pass, fail].into_iter().flatten() {
                    Self::sva_stmt_names(s, all, out);
                }
            }
            SK::For { body, .. }
            | SK::While { body, .. }
            | SK::DoWhile { body, .. }
            | SK::Repeat { body, .. }
            | SK::Forever { body }
            | SK::ForeverTail { body }
            | SK::Foreach { body, .. }
            | SK::ForeachTail { body, .. }
            | SK::RsAction { body } => Self::sva_stmt_names(body, all, out),
            _ => {}
        }
    }

    /// Every name segment `e` spells, clocked bodies included.
    pub(super) fn sva_expr_names(e: &Expression, out: &mut HashSet<String>) {
        Self::walk_operands(e, &mut |x| match &x.kind {
            ExprKind::Ident(h) => out.extend(h.path.iter().map(|s| s.name.name.clone())),
            ExprKind::SvaClocked { events, body } => {
                Self::sva_expr_names(body, out);
                for event in events {
                    Self::sva_expr_names(&event.expr, out);
                    if let Some(g) = &event.iff {
                        Self::sva_expr_names(g, out);
                    }
                }
            }
            _ => {}
        });
    }

    /// §16.5.1: a declaration initializer the elaborator could not fold runs
    /// as a time-0 process: a static initializer, or at the top level a
    /// synthesized initial block (dummy spans, as `cov_synthesized_initial`
    /// tells them apart). What it assigns is still the default sampled
    /// value, so end each such process with `SAMPLED_DEFAULT_MARKER`.
    pub(super) fn mark_deferred_inits(&mut self) {
        let dummy = |sp: crate::ast::Span| sp.start == 0 && sp.end == 0;
        let blocks = self.module.static_init_blocks.iter_mut().map(|b| (b, true));
        let synthesized = self.module.initial_blocks.iter_mut().filter_map(|b| {
            let s = &b.stmt;
            let ok = dummy(s.span)
                || matches!(&s.kind, StatementKind::BlockingAssign { lvalue, .. } if dummy(lvalue.span));
            ok.then_some((b, false))
        });
        for (b, any) in blocks.chain(synthesized) {
            let mut targets = Vec::new();
            if !Self::init_targets(&b.stmt, any, &mut targets) || targets.is_empty() {
                continue;
            }
            let span = crate::ast::Span::dummy();
            let marker = Statement::new(
                StatementKind::Expr(Expression::new(
                    ExprKind::SystemCall {
                        name: SAMPLED_DEFAULT_MARKER.to_string(),
                        args: targets,
                    },
                    span,
                )),
                span,
            );
            if let StatementKind::SeqBlock { stmts, .. } = &mut b.stmt.kind {
                stmts.push(marker);
                continue;
            }
            let st = std::mem::replace(&mut b.stmt, Statement::new(StatementKind::Null, span));
            b.stmt = Statement::new(
                StatementKind::SeqBlock {
                    name: None,
                    stmts: vec![st, marker],
                },
                span,
            );
        }
    }

    /// The targets of the blocking assignments in a deferred declaration
    /// initializer. With `any` false (a synthesized initial block), false
    /// unless the block holds nothing else.
    pub(super) fn init_targets(st: &Statement, any: bool, out: &mut Vec<Expression>) -> bool {
        match &st.kind {
            StatementKind::BlockingAssign { lvalue, .. } => {
                out.push(lvalue.clone());
                true
            }
            StatementKind::SeqBlock { stmts, .. } => {
                // Every statement is visited: `all` would stop at the first
                // miss and leave the later targets out.
                stmts
                    .iter()
                    .fold(true, |ok, s| Self::init_targets(s, any, out) && ok)
            }
            _ => any,
        }
    }

    /// `SAMPLED_DEFAULT_MARKER`: a deferred declaration initializer has run,
    /// so its targets now hold their default sampled values (§16.5.1).
    /// Update the ones a concurrent assertion or a procedural watch keeps.
    pub(super) fn record_sampled_defaults(&mut self, targets: &[Expression]) {
        let mut ids = Vec::new();
        for t in targets {
            let Some(id) = self.get_lhs_signal_id(t) else {
                continue;
            };
            let Some(v) = self.signal_table.get(id).cloned() else {
                continue;
            };
            self.set_sampled_default(id, v);
            ids.push(id);
        }
        self.refresh_expr_defaults(&ids);
    }

    /// Make `v` the default sampled value of signal `id` (§16.5.1): for the
    /// watches that read it before their first tick, and as its time-0
    /// sample for the concurrent assertions.
    pub(super) fn set_sampled_default(&mut self, id: usize, v: Value) {
        for w in &mut self.sampled_watches {
            if w.sig_id == Some(id) && w.default.is_some() {
                w.default = Some(v.clone());
            }
        }
        if self.time == 0
            && self.sva_preponed_slot == Some(0)
            && let Some(p) = self.sva_preponed.get_mut(&id)
        {
            *p = v.clone();
        }
        if let Some(d) = self.sampled_defaults.get_mut(&id) {
            *d = v;
        }
    }

    /// Re-evaluate the default of each expression watch (`$past(a | b)`)
    /// that reads one of `ids`, whose default sampled values changed, from
    /// the default sampled values of its operands.
    pub(super) fn refresh_expr_defaults(&mut self, ids: &[usize]) {
        if ids.is_empty() {
            return;
        }
        for i in 0..self.sampled_watches.len() {
            let w = &self.sampled_watches[i];
            // Every operand needs its default: a live one would mix values
            // from after the time-0 settle into a default.
            if w.sig_id.is_some()
                || w.default.is_none()
                || !w.sample_ids.iter().any(|id| ids.contains(id))
                || !w
                    .sample_ids
                    .iter()
                    .all(|id| self.sampled_defaults.contains_key(id))
            {
                continue;
            }
            let e = self.sampled_watches[i].expr.clone();
            let ops = self.sampled_watches[i].sample_ids.clone();
            let v = self.eval_with_defaults(&e, &ops);
            self.sampled_watches[i].default = Some(v);
        }
    }

    /// `e` evaluated with each signal of `ids` that has a default sampled
    /// value holding it.
    pub(super) fn eval_with_defaults(&mut self, e: &Expression, ids: &[usize]) -> Value {
        let mut saved: Vec<(usize, Value)> = ids
            .iter()
            .filter(|&&id| id < self.signal_table.len())
            .filter_map(|&id| Some((id, self.sampled_defaults.get(&id)?.clone())))
            .collect();
        for (id, v) in &mut saved {
            std::mem::swap(&mut self.signal_table[*id], v);
        }
        let v = self.eval_expr(e);
        for (id, v) in saved.into_iter().rev() {
            self.signal_table[id] = v;
        }
        v
    }

    /// Pre-register sampled-value watches for every `$rose/$fell/$stable/
    /// $changed/$past` call in procedural code, so their clock-edge history
    /// accumulates from time 0 (lazy registration at first eval would miss
    /// every earlier edge).
    pub(super) fn register_sampled_watches(&mut self) {
        if self.clocking_meta.is_empty() {
            // Explicit @(...) args can still appear; scan anyway.
        }
        // Each site registers under the activation scope it will run in
        // (see `sampled_scope`), so install that scope around its scan.
        let saved_activation_scope = self.activation_scope.borrow().clone();
        self.sampled_registering = true;
        let mut stmts: Vec<(Statement, String)> = Vec::new();
        for ib in &self.module.initial_blocks {
            stmts.push((ib.stmt.clone(), ib.scope.clone()));
        }
        for (st, scope) in stmts {
            *self.activation_scope.borrow_mut() = Some(scope);
            self.scan_sampled_stmt(&st);
        }
        // §16.9.3: an edge-sensitive always block supplies the inferred
        // clocking event for every sampled-value call in its body. Sourced
        // from `edge_blocks` rather than `module.always_blocks` — by the time
        // this runs the always blocks have already been compiled into edge
        // blocks and the module vec is empty, which is why procedural
        // `$rose`/`$fell` had no clock and silently never fired.
        // Read through a second handle on the list instead of cloning every
        // block's statement (the registration below needs `&mut self`).
        let blocks = Arc::clone(&self.edge_blocks);
        for b in blocks.iter() {
            let clk = b.resolved_sensitivities.iter().find_map(|s| match s.edge {
                EdgeKind::Posedge => Some((1u8, s.signal_id)),
                EdgeKind::Negedge => Some((2u8, s.signal_id)),
                _ => None,
            });
            if let Some((ec, cid)) = clk {
                *self.activation_scope.borrow_mut() = Some(b.scope.clone());
                self.record_sampled_sites(&b.stmt, ec, cid);
                self.scan_sampled_stmt(&b.stmt);
            }
        }
        for (st, scope, ec, cid) in std::mem::take(&mut self.proc_sampled_blocks) {
            *self.activation_scope.borrow_mut() = Some(scope);
            self.record_sampled_sites(&st, ec, cid);
            self.scan_sampled_stmt(&st);
        }
        self.sampled_registering = false;
        *self.activation_scope.borrow_mut() = saved_activation_scope;
    }

    /// Tag every sampled-value call site inside `st` with the block's clock.
    pub(super) fn record_sampled_sites(&mut self, st: &Statement, ec: u8, cid: usize) {
        let mut sites: Vec<usize> = Vec::new();
        Self::collect_sampled_sites_stmt(st, &mut sites);
        for site in sites {
            let scope = self.sampled_scope();
            let v = self.sampled_site_clock.entry(site).or_default();
            if !v.iter().any(|(s, _)| **s == *scope) {
                v.push((scope.into(), (ec, cid)));
            }
        }
    }

    /// Span starts of every `$rose`/`$fell`/`$stable`/`$changed`/`$past` call
    /// inside a statement — the key `sampled_site_clock` and
    /// `sampled_watch_site` both use.
    pub(super) fn collect_sampled_sites_stmt(st: &Statement, out: &mut Vec<usize>) {
        use crate::ast::stmt::StatementKind as SK;
        match &st.kind {
            SK::SeqBlock { stmts, .. } | SK::ParBlock { stmts, .. } => {
                for s in stmts {
                    Self::collect_sampled_sites_stmt(s, out);
                }
            }
            SK::If {
                condition,
                then_stmt,
                else_stmt,
                ..
            } => {
                Self::collect_sampled_sites_expr(condition, out);
                Self::collect_sampled_sites_stmt(then_stmt, out);
                if let Some(e) = else_stmt {
                    Self::collect_sampled_sites_stmt(e, out);
                }
            }
            SK::TimingControl { stmt, .. } => Self::collect_sampled_sites_stmt(stmt, out),
            SK::For { body, .. }
            | SK::While { body, .. }
            | SK::DoWhile { body, .. }
            | SK::Repeat { body, .. }
            | SK::Forever { body }
            | SK::Foreach { body, .. } => Self::collect_sampled_sites_stmt(body, out),
            SK::Case { expr, items, .. } => {
                Self::collect_sampled_sites_expr(expr, out);
                for it in items {
                    Self::collect_sampled_sites_stmt(&it.stmt, out);
                }
            }
            SK::BlockingAssign { rvalue, .. } => Self::collect_sampled_sites_expr(rvalue, out),
            SK::NonblockingAssign { rvalue, .. } => Self::collect_sampled_sites_expr(rvalue, out),
            SK::Expr(e) => Self::collect_sampled_sites_expr(e, out),
            _ => {}
        }
    }

    pub(super) fn collect_sampled_sites_expr(e: &Expression, out: &mut Vec<usize>) {
        match &e.kind {
            ExprKind::SystemCall { name, args }
                if matches!(
                    name.as_str(),
                    "$rose" | "$fell" | "$stable" | "$changed" | "$past"
                ) =>
            {
                out.push(e.span.start);
                for a in args {
                    Self::collect_sampled_sites_expr(a, out);
                }
            }
            ExprKind::SystemCall { args, .. } | ExprKind::Call { args, .. } => {
                for a in args {
                    Self::collect_sampled_sites_expr(a, out);
                }
            }
            ExprKind::Binary { left, right, .. } => {
                Self::collect_sampled_sites_expr(left, out);
                Self::collect_sampled_sites_expr(right, out);
            }
            ExprKind::Unary { operand, .. } => Self::collect_sampled_sites_expr(operand, out),
            ExprKind::Paren(inner) => Self::collect_sampled_sites_expr(inner, out),
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                Self::collect_sampled_sites_expr(condition, out);
                Self::collect_sampled_sites_expr(then_expr, out);
                Self::collect_sampled_sites_expr(else_expr, out);
            }
            _ => {}
        }
    }

    pub(super) fn scan_sampled_stmt(&mut self, st: &Statement) {
        use crate::ast::stmt::StatementKind as SK;
        match &st.kind {
            SK::SeqBlock { stmts, .. } | SK::ParBlock { stmts, .. } => {
                for s in stmts {
                    self.scan_sampled_stmt(s);
                }
            }
            SK::If {
                condition,
                then_stmt,
                else_stmt,
                ..
            } => {
                self.scan_sampled_expr(condition);
                self.scan_sampled_stmt(then_stmt);
                if let Some(e) = else_stmt {
                    self.scan_sampled_stmt(e);
                }
            }
            SK::TimingControl { stmt, .. } => self.scan_sampled_stmt(stmt),
            SK::For { body, .. }
            | SK::While { body, .. }
            | SK::DoWhile { body, .. }
            | SK::Repeat { body, .. }
            | SK::Forever { body }
            | SK::Foreach { body, .. } => self.scan_sampled_stmt(body),
            SK::Case { expr, items, .. } => {
                self.scan_sampled_expr(expr);
                for it in items {
                    self.scan_sampled_stmt(&it.stmt);
                }
            }
            SK::BlockingAssign { rvalue, .. } => self.scan_sampled_expr(rvalue),
            SK::NonblockingAssign { rvalue, .. } => self.scan_sampled_expr(rvalue),
            SK::Expr(e) => self.scan_sampled_expr(e),
            _ => {}
        }
    }

    pub(super) fn scan_sampled_expr(&mut self, e: &Expression) {
        match &e.kind {
            ExprKind::SystemCall { name, args }
                if matches!(
                    name.as_str(),
                    "$rose" | "$fell" | "$stable" | "$changed" | "$past"
                ) =>
            {
                // Registration mirrors the eval-time dispatcher; ignore
                // failures (no clock resolvable) — eval falls back.
                let ex = e.clone();
                let ar = args.clone();
                let depth = if name == "$past" {
                    ar.get(1)
                        .filter(|a| !matches!(a.kind, ExprKind::Null | ExprKind::Empty))
                        .map(|a| self.eval_expr(a).to_u64().unwrap_or(1) as usize)
                        .unwrap_or(1)
                } else {
                    1
                };
                let _ = self.sampled_fn_via_watch(&ex, &ar, depth);
            }
            ExprKind::SystemCall { args, .. } | ExprKind::Call { args, .. } => {
                for a in args {
                    self.scan_sampled_expr(a);
                }
            }
            ExprKind::Binary { left, right, .. } => {
                self.scan_sampled_expr(left);
                self.scan_sampled_expr(right);
            }
            ExprKind::Unary { operand, .. } | ExprKind::Paren(operand) => {
                self.scan_sampled_expr(operand)
            }
            ExprKind::SvaClocked { body, .. } => self.scan_sampled_expr(body),
            ExprKind::RangeSelect {
                expr, left, right, ..
            } => {
                self.scan_sampled_expr(expr);
                self.scan_sampled_expr(left);
                self.scan_sampled_expr(right);
            }
            ExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.scan_sampled_expr(condition);
                self.scan_sampled_expr(then_expr);
                self.scan_sampled_expr(else_expr);
            }
            ExprKind::Concatenation(items) => {
                for i in items {
                    self.scan_sampled_expr(i);
                }
            }
            ExprKind::Index { expr, index } => {
                self.scan_sampled_expr(expr);
                self.scan_sampled_expr(index);
            }
            _ => {}
        }
    }

    /// `$__xz_clocking_arg(edge_code, clk)` marker from the parser (the
    /// explicit clocking argument of a §16.9.3 sampled-value function).
    pub(super) fn clocking_arg_marker(e: &Expression) -> Option<(u8, &Expression)> {
        if let ExprKind::SystemCall { name, args } = &e.kind {
            if name == "$__xz_clocking_arg" && args.len() == 2 {
                if let ExprKind::Number(NumberLiteral::Integer { value, .. }) = &args[0].kind {
                    return Some((value.parse().unwrap_or(0), &args[1]));
                }
            }
        }
        None
    }

    /// Reconstruct a signal's PREPONED (slot-entry) value from the edge
    /// snapshot arrays.
    /// Copy a signal's current value into its preponed snapshot (the
    /// per-slot pass does this for every edge signal after a slot that wrote
    /// something; a signal registered mid-run needs it once, immediately).
    pub(super) fn seed_prev_from_current(&mut self, id: usize) {
        if id >= self.signal_table.len() || id >= self.prev_val.len() || id >= self.prev_xz.len() {
            return;
        }
        let (v, x) = self.signal_table[id].raw_bits();
        self.prev_val[id] = v;
        self.prev_xz[id] = x;
        if self.signal_widths.get(id).copied().unwrap_or(0) > 64 {
            let cur = self.signal_table[id].clone();
            self.prev_wide.insert(id, cur);
        }
    }

    /// §16.5.1: the sampled value of signal `id` in this slot's Preponed
    /// region. In time slot 0 that is its default sampled value: the
    /// preponed snapshot is taken after the time-0 settle has run the
    /// continuous assignments, so it would read what they drive.
    #[inline]
    pub(super) fn slot_preponed(&self, id: usize) -> Value {
        if self.time == 0
            && let Some(v) = self.sampled_defaults.get(&id)
        {
            return v.clone();
        }
        self.preponed_value_of(id)
    }

    /// §16.5.1: in time slot 0, the default sampled value of signal `id`
    /// (`sampled_defaults`), when it has one.
    pub(super) fn slot0_default(&self, id: usize) -> Option<Value> {
        if self.time != 0 {
            return None;
        }
        self.sampled_defaults.get(&id).cloned()
    }

    pub(super) fn preponed_value_of(&self, id: usize) -> Value {
        if let Some(w) = self.prev_wide.get(&id) {
            return w.clone();
        }
        let width = self.signal_widths.get(id).copied().unwrap_or(1).min(64);
        let mut v = Value::from_inline(self.prev_val[id], self.prev_xz[id], width);
        v.is_signed = self.signal_signed.get(id).copied().unwrap_or(false);
        v
    }

    pub(super) fn add_sampled_watch_ids(&mut self, idx: usize, ids: &[usize]) {
        for &sid in ids {
            if !self.sampled_watches[idx].sample_ids.contains(&sid) {
                self.sampled_watches[idx].sample_ids.push(sid);
            }
            if !self.preponed_ids.contains(&sid) {
                self.preponed_ids.push(sid);
            }
            if !self.edge_signal_ids.contains(&sid) {
                self.edge_signal_ids.push(sid);
                self.seed_prev_from_current(sid);
            }
        }
    }

    /// Get-or-create the sampled watch for a call site. Registers the clock
    /// (and a simple-signal operand) into `edge_signal_ids` so the snapshot
    /// pass keeps their preponed values fresh.
    pub(super) fn sampled_watch_for(
        &mut self,
        site: usize,
        operand: &Expression,
        edge_code: u8,
        clk_id: usize,
        depth: usize,
        clock_site: Option<usize>,
    ) -> Option<usize> {
        if let Some(i) = Self::sampled_site_lookup(
            &self.sampled_watch_site,
            site,
            self.activation_scope.borrow().as_deref().unwrap_or(""),
            !self.sampled_registering,
        ) {
            if depth + 1 > self.sampled_watches[i].depth {
                self.sampled_watches[i].depth = depth + 1;
            }
            return Some(i);
        }
        let edge = match edge_code {
            1 => EdgeKind::Posedge,
            2 => EdgeKind::Negedge,
            _ => EdgeKind::AnyEdge,
        };
        let sig_id = match &operand.kind {
            ExprKind::Ident(h) if h.path.iter().all(|p| p.selects.is_empty()) => {
                self.get_lhs_signal_id(operand)
            }
            _ => None,
        };
        // A signal registered here at run time has no preponed snapshot yet,
        // and the snapshot pass only runs for a slot that wrote something —
        // a declaration-initialised operand (`logic [7:0] v = 8'hA5;`) is
        // never written, so its first sample read a stale x. Seed the
        // snapshot from the current value on registration.
        if clk_id != usize::MAX && !self.edge_signal_ids.contains(&clk_id) {
            self.edge_signal_ids.push(clk_id);
            self.seed_prev_from_current(clk_id);
        }
        if clk_id != usize::MAX && !self.preponed_ids.contains(&clk_id) {
            self.preponed_ids.push(clk_id);
        }
        if let Some(sid) = sig_id {
            if !self.preponed_ids.contains(&sid) {
                self.preponed_ids.push(sid);
            }
            if !self.edge_signal_ids.contains(&sid) {
                self.edge_signal_ids.push(sid);
                self.seed_prev_from_current(sid);
            }
        }
        let default = if !self.sampled_registering {
            None
        } else {
            match sig_id {
                // §16.5.1: the value from before time 0, which
                // `capture_sampled_defaults` kept. A concurrent assertion
                // registers its watches after the time-0 settle (a net reads
                // its driver's value by then), and one in a procedure after
                // the procedure's earlier writes.
                Some(sid) => self
                    .sampled_defaults
                    .get(&sid)
                    .or(self.signal_table.get(sid))
                    .cloned(),
                // An expression's default is computed from its operands'
                // defaults, kept for every operand so a later change to one
                // (`refresh_expr_defaults`) can recompute it.
                None => {
                    let mut ids = Vec::new();
                    self.collect_sva_signal_ids(operand, &mut ids);
                    if !self.time0_settled {
                        for &id in &ids {
                            if !self.sampled_defaults.contains_key(&id)
                                && let Some(v) = self.signal_table.get(id)
                            {
                                self.sampled_defaults.insert(id, v.clone());
                            }
                        }
                    }
                    Some(self.eval_with_defaults(operand, &ids))
                }
            }
        };
        let clock_events = clock_site
            .map(|si| &self.sva_sites[si].events)
            .filter(|events| {
                // §16.9.3: a site can retain a resolvable clock expression
                // even when its descriptive clock name has no physical ID
                // (for example a clock reached through an interface port).
                // Preserve that event instead of indexing the sentinel ID.
                clk_id == usize::MAX
                    || events.len() != 1
                    || events[0].signal.is_none()
                    || events[0].event.iff.is_some()
            })
            .cloned()
            .unwrap_or_default();
        let i = self.sampled_watches.len();
        self.sampled_watches.push(SampledWatch {
            clk_id,
            edge,
            clock_events,
            sig_id,
            expr: operand.clone(),
            hist: std::collections::VecDeque::new(),
            gate: None,
            action_gate: false,
            pending_sample: None,
            sample_ids: Vec::new(),
            slot_values: Vec::new(),
            depth: depth + 1,
            last_sample_time: None,
            slot_val: None,
            slot_time: None,
            default,
        });
        let mut sample_ids = Vec::new();
        self.collect_sva_signal_ids(operand, &mut sample_ids);
        self.add_sampled_watch_ids(i, &sample_ids);
        let scope = self.sampled_scope();
        self.sampled_watch_site
            .entry(site)
            .or_default()
            .push((scope.into(), i));
        Some(i)
    }

    /// Advance each sampled watch whose clock edged this delta: push the
    /// PREPONED sample of the operand onto its history.
    pub(super) fn tick_sampled_watches(&mut self) {
        for i in 0..self.sampled_watches.len() {
            if self.sampled_watches[i].slot_time != Some(self.time) {
                let ids = self.sampled_watches[i].sample_ids.clone();
                let vals = ids
                    .into_iter()
                    .map(|sid| (sid, self.slot_preponed(sid)))
                    .collect();
                let slot_val = self.sampled_watches[i]
                    .sig_id
                    .map(|sid| self.slot_preponed(sid));
                let w = &mut self.sampled_watches[i];
                w.slot_values = vals;
                w.slot_val = slot_val;
                w.slot_time = Some(self.time);
            }
            let (clk_id, edge) = {
                let w = &self.sampled_watches[i];
                (w.clk_id, w.edge)
            };
            let fired = if self.sampled_watches[i].clock_events.is_empty() && clk_id == usize::MAX {
                // An unbound inferred clock has no ticks to sample.
                false
            } else if self.sampled_watches[i].clock_events.is_empty() {
                self.check_edge_id(clk_id, edge)
            } else {
                let mut events = std::mem::take(&mut self.sampled_watches[i].clock_events);
                let fired = self.sva_events_fired(&mut events);
                self.sampled_watches[i].clock_events = events;
                fired
            };
            if !fired {
                continue;
            }
            let mut saved = self.sampled_watches[i].slot_values.clone();
            for (sid, v) in &mut saved {
                std::mem::swap(&mut self.signal_table[*sid], v);
            }
            let gate = self.sampled_watches[i].gate.clone();
            let deferred_gate = self.sampled_watches[i].action_gate && gate.is_some();
            let qualifies =
                deferred_gate || gate.as_ref().is_none_or(|g| self.eval_expr(g).is_true());
            let sample = if qualifies {
                let e = self.sampled_watches[i].expr.clone();
                Some(self.eval_expr(&e))
            } else {
                None
            };
            for (sid, v) in saved.into_iter().rev() {
                self.signal_table[sid] = v;
            }
            if let Some(sample) = sample {
                let w = &mut self.sampled_watches[i];
                if deferred_gate {
                    w.pending_sample = Some((self.time, sample));
                    continue;
                }
                w.last_sample_time = Some(self.time);
                w.hist.push_front(sample);
                while w.hist.len() > w.depth + 1 {
                    w.hist.pop_back();
                }
            }
        }
    }

    /// §16.9.3 / §16.14.1: the operand is sampled in Preponed, but an
    /// action block's unsampled gate is evaluated after NBA, before actions.
    pub(super) fn finish_action_sampled_watches(&mut self) {
        for i in 0..self.sampled_watches.len() {
            let Some((time, sample)) = self.sampled_watches[i].pending_sample.take() else {
                continue;
            };
            let gate = self.sampled_watches[i].gate.clone();
            if gate.as_ref().is_none_or(|g| self.eval_expr(g).is_true()) {
                let w = &mut self.sampled_watches[i];
                w.last_sample_time = Some(time);
                w.hist.push_front(sample);
                while w.hist.len() > w.depth + 1 {
                    w.hist.pop_back();
                }
            }
        }
    }

    pub(super) fn check_edges_inner(
        &mut self,
        detect_subset: Option<&[usize]>,
        validate_coverage: bool,
    ) {
        if self.finished {
            return;
        }
        // A `#delay` in an edge block's body re-enters here through the
        // synchronous `TimingControl::Delay` arm. That is legitimate — the
        // delay's own region flush must deliver edges made before it — but a
        // block that keeps re-triggering itself across its delay would recurse
        // forever, so bound the nesting. At the cap we stop descending; the
        // outer pass's post-exec rescan still delivers what is pending.
        const EDGE_PASS_DEPTH_LIMIT: u32 = 8;
        if self.edge_pass_depth >= EDGE_PASS_DEPTH_LIMIT {
            return;
        }
        self.edge_pass_depth += 1;
        let outermost_pass = !self.in_edge_block;
        if self.event_measure {
            self.event_phase += 1; // flop SAMPLE phase
        }
        // §16.9.3 sampled-value histories advance FIRST — a waiter
        // continuation resumed on this same clock edge reads the
        // just-sampled cycle.
        if !self.sampled_watches.is_empty() {
            self.tick_sampled_watches();
        }
        // Commercial-simulator process ordering (reference-verified on three
        // same-module micro-shapes: NBA FF racing a driver-style waiter that
        // writes its inputs; a blocking-write always block racing a waiter
        // that reads; a waiter reading a same-edge NBA target): the edge's
        // always blocks execute FIRST, then parked `@(posedge clk)` waiters
        // resume in the same active region reading pre-NBA state. The
        // OpenRAM-style "check reads the pre-blast value" case is NOT a
        // waiters-first ordering — it resolves through port-boundary edge
        // delivery — and holds under blocks-first too (pinned by
        // waiter_edge_ordering::parked_waiter_reads_pre_edge_block_value).
        // XEZIM_WAITERS_FIRST=1 restores the legacy pre-block resumption.
        static WAITERS_FIRST: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let waiters_first = *WAITERS_FIRST.get_or_init(|| {
            std::env::var("XEZIM_WAITERS_FIRST")
                .map(|v| v == "1")
                .unwrap_or(false)
        });
        if waiters_first {
            // §19.5: event-driven covergroups sample the values current at
            // the clocking event — BEFORE same-edge process continuations
            // (which may immediately drive the next stimulus value).
            self.sample_edge_covergroups();
        }
        if waiters_first && !self.event_waiters.is_empty() {
            let _t_w = self.profile_timing.then(std::time::Instant::now);
            // Drain and run triggered waiters, then RE-DRAIN: a resumed
            // continuation may `-> ev` or write a signal that triggers ANOTHER
            // parked waiter in the SAME time step (thread A resumes on a clock
            // edge and fires `ev`; thread B parked on `@(ev)` must wake now,
            // not next tick — §14.13 clocking-block synchronization). Running
            // inline swallows the follow-up pass the old event-queue path gave,
            // so loop until no waiter fires. Bounded against #0 event ping-pong.
            let mut settle_guard = 0u32;
            loop {
                let mut conts = self.drain_triggered_event_waiters();
                if conts.is_empty() {
                    self.recycle_conts(conts);
                    break;
                }
                for (pid, stmts) in conts.drain(..) {
                    if self.finished {
                        break;
                    }
                    self.run_scheduled_process(pid, &stmts);
                    if !self.is_pid_suspended(pid) {
                        self.child_finished(pid);
                    }
                    // A continuation that ended on a break/return leaves its
                    // control flags set; everything downstream of this edge
                    // pass (edge blocks' fallback exec, SVA actions) runs
                    // through exec_statement, which no-ops while they're up.
                    self.break_flag = false;
                    self.continue_flag = false;
                    self.return_flag = false;
                }
                self.recycle_conts(conts);
                settle_guard += 1;
                if self.finished || settle_guard > 10_000 {
                    break;
                }
            }
            if let Some(t) = _t_w {
                self.prof_edge_waiters += t.elapsed().as_nanos() as u64;
            }
        }
        // Cheap handle; `self.edge_blocks` stays populated so nested passes and
        // `edge_blocks.get(bi)` probes still see the real list.
        let blocks = Arc::clone(&self.edge_blocks);
        self.in_edge_block = true;

        // Honor `iff` guards on edge-sensitive always blocks (LRM §9.4.2.3).
        // Evaluate each guarded block's terms up front — here `eval_expr` can
        // borrow `&mut self` freely, whereas the detection loop below holds
        // the trigger bitmap borrow and cannot. A block is DENIED this
        // delta-cycle when every sensitivity term that fired an edge has a
        // guard that is currently false; the hot loop then skips it. The
        // whole pass is skipped when no edge block carries a guard.
        let iff_active = !self.edge_blocks_with_iff.is_empty();
        let mut iff_denied = std::mem::take(&mut self.edge_iff_denied);
        iff_denied.clear();
        if iff_active {
            iff_denied.resize(blocks.len(), false);
            let guarded = std::mem::take(&mut self.edge_blocks_with_iff);
            for &bi in &guarded {
                if bi >= blocks.len() {
                    continue;
                }
                let mut any_fire = false;
                let mut allow = false;
                for s in &blocks[bi].resolved_sensitivities {
                    if !self.check_edge_id(s.signal_id, s.edge) {
                        continue;
                    }
                    any_fire = true;
                    match &s.iff {
                        Some(g) => {
                            if self.eval_expr(g).is_true() {
                                allow = true;
                                break;
                            }
                        }
                        None => {
                            allow = true;
                            break;
                        }
                    }
                }
                if any_fire && !allow {
                    iff_denied[bi] = true;
                }
            }
            self.edge_blocks_with_iff = guarded;
        }

        // Phase 1: detect which blocks trigger.
        //
        // Inverted iteration: walk the (usually small) list of edge-sensitive
        // signal IDs, compute whether each fired an edge this tick, then
        // dispatch to the blocks sensitive to that signal via
        // `edge_blocks_by_sig`. For c910 this turns ~20,000 check_edge_id
        // calls per tick (10k blocks × 2 sensitivities) into ~200
        // (one per unique clk/rst/enable signal).
        let t0 = self.profile_timing.then(std::time::Instant::now);
        // Reuse hoisted bitmap/list. Bits are cleared selectively at the end
        // from edge_triggered_list, avoiding an O(edge_blocks) bitmap fill and
        // a second bitmap scan on every check.
        if self.edge_triggered_bitmap.len() < blocks.len() {
            self.edge_triggered_bitmap.resize(blocks.len(), false);
        }
        let mut triggered = std::mem::take(&mut self.edge_triggered_list);
        triggered.clear();
        let triggered_bitmap = &mut self.edge_triggered_bitmap[..blocks.len()];
        let bitsel_active = !self.bitsel_edge_sens.is_empty();
        let armed_prefilter = self.armed_edge
            && !self.armed_edge_shadow
            && self.event_skip
            && self.time >= self.event_after;
        if armed_prefilter {
            if self.edge_prefilter_seen.len() < blocks.len() {
                self.edge_prefilter_seen.resize(blocks.len(), 0);
            }
            self.edge_prefilter_generation = self.edge_prefilter_generation.wrapping_add(1);
            if self.edge_prefilter_generation == 0 {
                self.edge_prefilter_seen.fill(0);
                self.edge_prefilter_generation = 1;
            }
        }
        let prefilter_generation = self.edge_prefilter_generation;
        let mut prefiltered = 0u64;
        // edge_blocks_by_sig is parallel to edge_signal_ids (position-indexed).
        // Two scan modes: full range (default) or a caller-supplied subset of
        // positions (clocks-only / dirty-driven fast paths).
        //
        // This used to be a small `Iterator` enum, but the mode is constant
        // for the whole pass while the enum re-dispatched on its discriminant
        // through `Iterator::next` on EVERY position — LLVM would not unswitch
        // it out of a body this large. A loop-invariant `subset_scan` flag and
        // a plain index give the same two modes with one predictable compare
        // per position and no iterator state to advance.
        let subset_positions: &[usize] = detect_subset.unwrap_or(&[]);
        let subset_scan = detect_subset.is_some();
        let scan_len = if subset_scan {
            subset_positions.len()
        } else {
            self.edge_signal_ids.len()
        };
        // Every position that fires an edge this pass, with its value AT
        // DETECT TIME (pre-exec). After dispatch + waiter wakeup, prev is
        // refreshed to these values so (a) the consumed edge cannot re-fire
        // in a later same-time detect, and (b) a blocking write made DURING
        // exec reads as a NEW edge relative to the refreshed prev — which
        // drain_edge_exec_rescan then delivers (§9.2).
        let mut fired_snap = std::mem::take(&mut self.edge_fired_snap);
        let mut fired_wide = std::mem::take(&mut self.edge_fired_wide);
        fired_snap.clear();
        fired_wide.clear();
        // One generation per detect pass: a memo slot stamped with an older
        // generation is stale, so the table never has to be cleared.
        self.edge_group_generation = self.edge_group_generation.wrapping_add(1);
        let group_generation = self.edge_group_generation;
        let group_dedup_active = !self.edge_group_of.is_empty() && self.forced_signals.is_empty();
        // Hoisted views of every table the scan touches per position.
        //
        // The loop stores through `self` (memo slots, the trigger bitmap, the
        // stats counters), and LLVM cannot prove those stores miss the OTHER
        // vectors' headers, so each iteration re-materialized every base
        // pointer and length out of `self` before it could index. Borrowing
        // them once as local slices pins the bases in registers and turns each
        // access back into a single indexed load. Every store in the body
        // targets a DISJOINT field of `self`, which is what lets these borrows
        // coexist with them.
        let edge_sig_ids: &[usize] = &self.edge_signal_ids;
        let by_sig: &[EdgeFanout] = &self.edge_blocks_by_sig;
        let group_of: &[u32] = &self.edge_group_of;
        let signal_table: &[Value] = &self.signal_table;
        let prev_val_t: &[u64] = &self.prev_val;
        let prev_xz_t: &[u64] = &self.prev_xz;
        let widths_t: &[u32] = &self.signal_widths;
        let prev_wide_t: &HashMap<usize, Value> = &self.prev_wide;
        let group_memo: &mut [EdgeGroupMemo] = &mut self.edge_group_memo;
        // Same treatment for the tables the DISPATCH half reads — the armed
        // prefilter alone consults three of them on every one of ~189 M
        // fanout events.
        let blk_armed: &[u8] = &self.edge_block_armed;
        let bitsel_sid_bits: &[u64] = &self.bitsel_sid_bits;
        let bitsel_edge_sens = &self.bitsel_edge_sens;
        let prefilter_seen: &mut [u32] = &mut self.edge_prefilter_seen;
        let stats_on = self.edge_block_stats_enabled;
        let scan_stats = self.edge_scan_stats;
        let clktree_probe = self.clktree_probe;
        // Accumulated in registers and folded back after the loop — a `self`
        // counter incremented in the body is another store LLVM has to order
        // against the table loads.
        let mut scanned_delta = 0u64;
        let mut changed_delta = 0u64;
        for scan_i in 0..scan_len {
            let pos = if subset_scan {
                subset_positions[scan_i]
            } else {
                scan_i
            };
            // Fanout gate first. It needs only `pos`, and a position with no
            // sensitive block contributes nothing, so testing it before the
            // `edge_signal_ids` load costs nothing and saves that load.
            // (`edge_signal_ids` may be LONGER than `edge_blocks_by_sig` —
            // `sampled_watch_for` appends to it at run time — and those extra
            // positions were skipped by the fanout arm before this swap too.)
            let fanout = match by_sig.get(pos) {
                Some(e)
                    if !e.posedge.is_empty()
                        || !e.negedge.is_empty()
                        || !e.anyedge.is_empty()
                        || !e.lsbedge.is_empty() =>
                {
                    e
                }
                _ => continue,
            };
            // Clock-tree dedup: on a flattened netlist most edge signals are
            // per-module copies of one clock net, so the first member of a
            // group scanned this pass computes the answer and the rest read it
            // back — trading three scattered loads for one memo slot. Disabled
            // while anything is FORCED: §9.3.1 lets a force hold a copy away
            // from its driver, which is exactly the case the grouping assumes
            // cannot happen. `forced_signals` is empty in the common case.
            let group = if group_dedup_active {
                group_of.get(pos).copied().unwrap_or(u32::MAX)
            } else {
                u32::MAX
            };
            let memo_hit =
                group != u32::MAX && group_memo[group as usize].generation == group_generation;
            let (sid, cur_v, cur_x, prev_v, prev_x, fires_pos, fires_neg, fires_any) =
                if memo_hit && !clktree_probe {
                    let m = group_memo[group as usize];
                    // The bulk of the scan is clock-tree members that did NOT
                    // fire. Leave here, before reading `edge_signal_ids` and
                    // before unpacking the memo's four value words — only the
                    // three fired flags are needed to decide.
                    if !m.fires_pos && !m.fires_neg && !m.fires_any {
                        if scan_stats {
                            scanned_delta += 1;
                        }
                        continue;
                    }
                    let sid = match edge_sig_ids.get(pos) {
                        Some(&s) => s,
                        None => continue,
                    };
                    (
                        sid,
                        m.cur_v,
                        m.cur_x,
                        m.prev_v,
                        m.prev_x,
                        m.fires_pos,
                        m.fires_neg,
                        m.fires_any,
                    )
                } else {
                    let sid = match edge_sig_ids.get(pos) {
                        Some(&s) => s,
                        None => continue,
                    };
                    // Compute edge-fired booleans for this signal using SoA u64
                    // pairs; falls back to full Value compare for wide signals.
                    let (cur_v, cur_x) = signal_table[sid].raw_bits();
                    let prev_v = prev_val_t[sid];
                    let prev_x = prev_xz_t[sid];
                    let (fires_pos, fires_neg) = if (cur_x | prev_x) & 1 == 0 {
                        // Known clock levels need only a change and polarity.
                        let changed = (cur_v ^ prev_v) & 1 != 0;
                        (changed && cur_v & 1 != 0, changed && cur_v & 1 == 0)
                    } else {
                        // §9.4.2 includes known-to-X/Z transitions, not only
                        // transitions ending at a known zero or one.
                        let cb_one = (cur_v & 1) == 1 && (cur_x & 1) == 0;
                        let cb_zero = (cur_v & 1) == 0 && (cur_x & 1) == 0;
                        let pb_one = (prev_v & 1) == 1 && (prev_x & 1) == 0;
                        let pb_zero = (prev_v & 1) == 0 && (prev_x & 1) == 0;
                        (
                            (pb_zero && !cb_zero) || (!pb_one && cb_one),
                            (pb_one && !cb_one) || (!pb_zero && cb_zero),
                        )
                    };
                    let fires_any = if widths_t[sid] > 64 {
                        prev_wide_t
                            .get(&sid)
                            .map_or(cur_v != prev_v || cur_x != prev_x, |p| {
                                signal_table[sid] != *p
                            })
                    } else {
                        cur_v != prev_v || cur_x != prev_x
                    };
                    if memo_hit {
                        // Probe mode: the memo was available but we recomputed.
                        // Any disagreement means the grouping is unsound.
                        let m = group_memo[group as usize];
                        self.clktree_probe_hits += 1;
                        if m.cur_v != cur_v
                            || m.cur_x != cur_x
                            || m.prev_v != prev_v
                            || m.prev_x != prev_x
                            || m.fires_pos != fires_pos
                            || m.fires_neg != fires_neg
                            || m.fires_any != fires_any
                        {
                            self.clktree_probe_mismatch += 1;
                            if self.clktree_probe_mismatch <= 8 {
                                // No `name_for_id` here — it borrows `self`
                                // immutably while the trigger bitmap is held
                                // mutably. The summary resolves names afterwards.
                                self.clktree_probe_sids.push(sid);
                                eprintln!(
                                    "[CLKTREE-PROBE] MISMATCH t={} sid={} group={} \
                                 leader(cur={:x}/{:x} prev={:x}/{:x} p{}n{}a{}) \
                                 member(cur={:x}/{:x} prev={:x}/{:x} p{}n{}a{})",
                                    self.time,
                                    sid,
                                    group,
                                    m.cur_v,
                                    m.cur_x,
                                    m.prev_v,
                                    m.prev_x,
                                    m.fires_pos as u8,
                                    m.fires_neg as u8,
                                    m.fires_any as u8,
                                    cur_v,
                                    cur_x,
                                    prev_v,
                                    prev_x,
                                    fires_pos as u8,
                                    fires_neg as u8,
                                    fires_any as u8,
                                );
                            }
                        }
                    } else if group != u32::MAX {
                        group_memo[group as usize] = EdgeGroupMemo {
                            generation: group_generation,
                            fires_pos,
                            fires_neg,
                            fires_any,
                            cur_v,
                            cur_x,
                            prev_v,
                            prev_x,
                        };
                    }
                    (
                        sid, cur_v, cur_x, prev_v, prev_x, fires_pos, fires_neg, fires_any,
                    )
                };
            if scan_stats {
                scanned_delta += 1;
            }
            if !fires_pos && !fires_neg && !fires_any {
                continue;
            }
            fired_snap.push((sid, cur_v, cur_x));
            if widths_t[sid] > 64 {
                fired_wide.push((sid, signal_table[sid].clone()));
            }
            if scan_stats {
                changed_delta += 1;
            }
            // Shadow coverage check: a firing position MUST have been captured
            // in the dirty set (clocks + write hooks). If not, a write path
            // isn't hooked → dirty-driven detect would miss this edge. Abort
            // loudly rather than risk silent wrong sim.
            if validate_coverage && pos < self.edge_pos_seen.len() && !self.edge_pos_seen[pos] {
                eprintln!(
                    "[DIRTY-EDGE-SHADOW] time={} MISSED edge at pos={} sid={} \
                     (write path not hooked) — aborting",
                    self.time, pos, sid
                );
                self.abort_run();
                self.edge_scan_scanned += scanned_delta;
                self.edge_scan_changed += changed_delta;
                self.edge_pass_depth -= 1;
                self.in_edge_block = !outermost_pass;
                return;
            }
            // Phase-2 toggle counter: any edge means this signal
            // changed value this delta-cycle. Bumped only when
            // stats are enabled (XEZIM_EDGE_BLOCK_STATS=1) so the
            // production hot path stays branch-free.
            if stats_on && sid < self.signal_toggle_count.len() {
                self.signal_toggle_count[sid] += 1;
            }
            // Phase-4: track wakeups per fanout member + which signals
            // are sensitivity sources. We're inside the trigger loop;
            // the writer of `sid` isn't known here (the NBA queue is
            // already drained), so cross_block_wakeup_count gets
            // attributed to each WOKEN block — that's the inverse-
            // direction packet-flow count (B was waited on N times)
            // which is symmetric for hyperedge weight purposes.
            let mut woke_any = false;
            // Per-sid prefilter: only signals with a registered bit-select
            // sensitivity pay the (sid, block) tuple-hash below.
            let sid_bitsel = bitsel_active
                && bitsel_sid_bits
                    .get(sid >> 6)
                    .is_some_and(|w| w & (1u64 << (sid & 63)) != 0);
            macro_rules! dispatch_block {
                ($block_idx:expr_2021, $kind:expr_2021) => {{
                    let block_idx = $block_idx;
                    // §9.4.2 bit/part-select level term (`@(v[3])`): detection
                    // above is per SIGNAL, so re-test the bits this block
                    // actually watches and drop the wake when only a sibling
                    // bit moved. Edge terms never carry a mask.
                    let bitsel_ok = !sid_bitsel
                        || !matches!($kind, EdgeKind::AnyEdge)
                        || match bitsel_edge_sens.get(&(sid, block_idx)) {
                            None => true,
                            Some(event_bits::EventMask::Low(m)) => {
                                ((cur_v ^ prev_v) | (cur_x ^ prev_x)) & m != 0
                            }
                            Some(event_bits::EventMask::Wide(bits)) => {
                                event_bits::wide_select_changed(
                                    bits,
                                    &signal_table[sid],
                                    prev_wide_t.get(&sid),
                                )
                            }
                        };
                    if bitsel_ok
                        && block_idx < triggered_bitmap.len()
                        && !(iff_active && iff_denied[block_idx])
                    {
                        let skip_early = armed_prefilter && blk_armed[block_idx] == 0;
                        if skip_early {
                            // `event_gateable_total` / `event_would_skip` /
                            // `armed_fast_skips` all advance in lockstep
                            // with `prefiltered` here, so they are folded
                            // in once after the loop instead of paying
                            // three read-modify-writes through `self` on
                            // each of ~110 M armed skips. The per-block
                            // generation stamp that deduplicated the count
                            // (a block sensitive to two changed signals in
                            // one pass) is kept only for the opt-in stats;
                            // the reported skip count may otherwise count
                            // such a block once per signal.
                            if !stats_on {
                                prefiltered += 1;
                                woke_any = true;
                            } else if prefilter_seen[block_idx] != prefilter_generation {
                                prefilter_seen[block_idx] = prefilter_generation;
                                prefiltered += 1;
                                woke_any = true;
                                if block_idx < self.cross_block_wakeup_count.len() {
                                    self.cross_block_wakeup_count[block_idx] += 1;
                                }
                            }
                        } else if !triggered_bitmap[block_idx] {
                            triggered_bitmap[block_idx] = true;
                            triggered.push(block_idx);
                            woke_any = true;
                            if stats_on && block_idx < self.cross_block_wakeup_count.len() {
                                self.cross_block_wakeup_count[block_idx] += 1;
                            }
                        }
                    }
                }};
            }
            if fires_pos {
                for &block_idx in &fanout.posedge {
                    dispatch_block!(block_idx, EdgeKind::Posedge);
                }
            }
            if fires_neg {
                for &block_idx in &fanout.negedge {
                    dispatch_block!(block_idx, EdgeKind::Negedge);
                }
            }
            if fires_any {
                for &block_idx in &fanout.anyedge {
                    dispatch_block!(block_idx, EdgeKind::AnyEdge);
                }
            }
            // §9.4.2 `@(edge x)`: an LSB posedge OR negedge (not any change).
            if fires_pos || fires_neg {
                for &block_idx in &fanout.lsbedge {
                    dispatch_block!(block_idx, EdgeKind::LsbEdge);
                }
            }
            if woke_any && stats_on && sid < self.sensitivity_trigger_count.len() {
                self.sensitivity_trigger_count[sid] += 1;
            }
        }
        if let Some(t) = t0 {
            self.prof_edge_detect += t.elapsed().as_nanos() as u64;
        }
        // Fold back the counters the scan accumulated in registers.
        self.edge_scan_scanned += scanned_delta;
        self.edge_scan_changed += changed_delta;
        self.event_gateable_total += prefiltered;
        self.event_would_skip += prefiltered;
        self.armed_fast_skips += prefiltered;
        self.prof_edges_fired += triggered.len() as u64 + prefiltered;
        // Return the iff-denial scratch buffer for reuse next delta-cycle.
        self.edge_iff_denied = iff_denied;
        // O1 MEASUREMENT (timestamp-based, gating-agnostic): for every gateable
        // flop firing this tick, it would be SKIPPABLE iff none of its data
        // inputs changed since the flop's previous fire. No behavior change.
        if self.event_skip && self.time >= self.event_after {
            // ARMED mode avoids snapshot reads until an input write marks the
            // block. The legacy path retains adaptive snapshot/epoch filtering
            // and remains available with XEZIM_ARMED_EDGE=0.
            const CHANGE_STREAK_TO_EPOCH: u8 = 8;
            const EPOCH_PROBE_WINDOW: u8 = 64;
            let now = self.event_phase;
            let mut w = 0usize;
            // Same treatment as the detect loop: these counters advance on
            // nearly every iteration, and a read-modify-write through `self`
            // in the body forces the block tables' bases to be re-derived
            // afterwards. Accumulate in registers, fold in once at the end.
            let mut gateable_delta = 0u64;
            let mut would_skip_delta = 0u64;
            let mut fast_skip_delta = 0u64;
            let mut snap_check_delta = 0u64;
            for r in 0..triggered.len() {
                let bi = triggered[r];
                let gate = self.edge_block_gateable.get(bi).copied().unwrap_or(false);
                if gate {
                    gateable_delta += 1;
                }
                let mut snapshot_result = None;
                let mut epoch_fast = false;
                let keep = if gate && self.armed_edge {
                    let was_armed = self.edge_block_armed[bi] & EDGE_ARMED != 0;
                    // Clear before execution. Any write performed by this or
                    // another block later in the delta cycle re-arms through
                    // the canonical write hooks.
                    self.edge_block_armed[bi] &= !EDGE_ARMED;
                    if !was_armed {
                        if self.armed_edge_shadow {
                            // `start`/`end` are read HERE rather than above:
                            // the dominant arm is the un-armed fast skip
                            // below, which never needs the block's read span,
                            // and hoisting them cost it two loads plus their
                            // bounds checks on every skipped fire.
                            let start = self.edge_block_off[bi] as usize;
                            let end = self.edge_block_off[bi + 1] as usize;
                            self.armed_shadow_checks += 1;
                            snap_check_delta += 1;
                            let mut differ = false;
                            for k in start..end {
                                let sid = self.edge_block_reads_flat[k] as usize;
                                let (lo, w) = self.edge_block_reads_meta[k];
                                let (v, x) = Self::raw_bits_slice(&self.signal_table[sid], lo, w);
                                let (sv, sx) = self.edge_block_snap_flat[k];
                                if v != sv || x != sx {
                                    differ = true;
                                    break;
                                }
                            }
                            if differ {
                                eprintln!(
                                    "[EVENT-EDGE] ARMED shadow miss at time={} block={} span={:?}",
                                    self.time, bi, blocks[bi].stmt.span
                                );
                                // `abort_run`, field by field: `triggered_bitmap` still
                                // borrows `self` mutably here.
                                self.saw_fatal = true;
                                self.finished = true;
                            }
                            snapshot_result = Some(differ);
                            differ
                        } else {
                            fast_skip_delta += 1;
                            false
                        }
                    } else if self.edge_block_arm_only[bi]
                        || !self.edge_block_snap_valid[bi]
                        || !self.armed_edge_shadow
                    {
                        // Armed means an input was WRITTEN since the last
                        // execution. Comparing its value against the snapshot
                        // elided only ~20% of these executions (C906: 38M of
                        // 187M compares) at ~200 host instructions each, more
                        // than the elided executions cost — so an armed block
                        // simply executes. The snapshot is kept only for the
                        // shadow mode's cross-check.
                        true
                    } else {
                        let start = self.edge_block_off[bi] as usize;
                        let end = self.edge_block_off[bi + 1] as usize;
                        snap_check_delta += 1;
                        let st: &[Value] = &self.signal_table;
                        let reads: &[u32] = &self.edge_block_reads_flat;
                        let mets: &[(u16, u16)] = &self.edge_block_reads_meta;
                        let snaps: &[(u64, u64)] = &self.edge_block_snap_flat;
                        let mut differ = false;
                        for k in start..end {
                            let sid = reads[k] as usize;
                            let (lo, w) = mets[k];
                            let (v, x) = Self::raw_bits_slice(&st[sid], lo, w);
                            let (sv, sx) = snaps[k];
                            if v != sv || x != sx {
                                differ = true;
                                break;
                            }
                        }
                        snapshot_result = Some(differ);
                        differ
                    }
                } else if gate {
                    let start = self.edge_block_off[bi] as usize;
                    let end = self.edge_block_off[bi + 1] as usize;
                    let reads = &self.edge_block_reads_flat[start..end];
                    let epoch_active = self.edge_block_epoch_probe_left[bi] != 0;
                    let epoch_changed = epoch_active
                        && reads.iter().any(|&s| {
                            // NBA/blocking writes made by the prior edge block
                            // share its sample phase. Equality is therefore a
                            // conservative "changed" result; false positives
                            // only execute a block and cannot create a bad skip.
                            self.sig_last_change[s as usize] >= self.flop_last_fire[bi]
                        });
                    if epoch_changed {
                        self.edge_block_epoch_probe_left[bi] -= 1;
                        self.event_epoch_fast_exec += 1;
                        epoch_fast = true;
                        true
                    } else if !self.edge_block_snap_valid[bi] {
                        // An epoch-fast execution deliberately invalidates its
                        // snapshot instead of refreshing every input. Rebuild
                        // it with one unconditional execution before a future
                        // value-based skip is permitted.
                        true
                    } else {
                        snap_check_delta += 1;
                        let snap = &self.edge_block_snap_flat[start..end];
                        let mets = &self.edge_block_reads_meta[start..end];
                        let st = &self.signal_table;
                        let mut differ = false;
                        for k in 0..reads.len() {
                            let s = unsafe { *reads.get_unchecked(k) } as usize;
                            let (lo, w) = unsafe { *mets.get_unchecked(k) };
                            let (v, x) = Self::raw_bits_slice(&st[s], lo, w);
                            let (sv, sx) = unsafe { *snap.get_unchecked(k) };
                            if v != sv || x != sx {
                                differ = true;
                                break;
                            }
                        }
                        snapshot_result = Some(differ);
                        differ
                    }
                } else {
                    true
                };
                if gate && !self.armed_edge {
                    self.flop_last_fire[bi] = now;
                    match snapshot_result {
                        Some(true) => {
                            let streak = self.edge_block_change_streak[bi].saturating_add(1);
                            self.edge_block_change_streak[bi] = streak;
                            if streak >= CHANGE_STREAK_TO_EPOCH {
                                self.edge_block_epoch_probe_left[bi] = EPOCH_PROBE_WINDOW;
                            }
                        }
                        Some(false) => {
                            self.edge_block_change_streak[bi] = 0;
                            self.edge_block_epoch_probe_left[bi] = 0;
                        }
                        None => {}
                    }
                }
                // Skip self-heal (see `edge_block_skip_streak`): after
                // `event_heal` consecutive skipped edges, force one real
                // execution + snapshot rebuild. Invisible for correctly-idle
                // flops; bounds any skip-bookkeeping wedge.
                let keep = if !keep && gate && self.event_heal != 0 {
                    let streak = self.edge_block_skip_streak[bi].saturating_add(1);
                    if streak >= self.event_heal {
                        self.edge_block_skip_streak[bi] = 0;
                        self.event_healed += 1;
                        // Invalidate the snapshot so the keep path below
                        // rebuilds it from current values after this forced
                        // execution.
                        self.edge_block_snap_valid[bi] = false;
                        if bi < self.edge_block_armed.len()
                            && !self.edge_block_wide_read.get(bi).copied().unwrap_or(false)
                        {
                            self.edge_block_armed[bi] |= EDGE_HOLD;
                        }
                        true
                    } else {
                        self.edge_block_skip_streak[bi] = streak;
                        false
                    }
                } else {
                    if keep && gate {
                        self.edge_block_skip_streak[bi] = 0;
                    }
                    keep
                };
                if keep {
                    if gate {
                        if epoch_fast {
                            self.edge_block_snap_valid[bi] = false;
                            if bi < self.edge_block_armed.len()
                                && !self.edge_block_wide_read.get(bi).copied().unwrap_or(false)
                            {
                                self.edge_block_armed[bi] |= EDGE_HOLD;
                            }
                        } else if self.armed_edge && !self.armed_edge_shadow {
                            // Write-armed and not cross-checking: the value
                            // snapshot is never compared, so do not rebuild
                            // it — just mark the block skippable while
                            // unarmed.
                            self.edge_block_snap_valid[bi] = true;
                            if bi < self.edge_block_armed.len() {
                                self.edge_block_armed[bi] &= !EDGE_HOLD;
                            }
                        } else {
                            let start = self.edge_block_off[bi] as usize;
                            let end = self.edge_block_off[bi + 1] as usize;
                            let has_wide = self.edge_block_reads_meta[start..end]
                                .iter()
                                .any(|&(_, width)| width > 64);
                            if has_wide {
                                self.edge_block_snap_valid[bi] = false;
                                if bi < self.edge_block_armed.len()
                                    && !self.edge_block_wide_read.get(bi).copied().unwrap_or(false)
                                {
                                    self.edge_block_armed[bi] |= EDGE_HOLD;
                                }
                            } else {
                                // Keep compact snapshots for narrow reads.
                                let st: &[Value] = &self.signal_table;
                                let reads: &[u32] = &self.edge_block_reads_flat;
                                let mets: &[(u16, u16)] = &self.edge_block_reads_meta;
                                let snaps: &mut [(u64, u64)] = &mut self.edge_block_snap_flat;
                                for k in start..end {
                                    let s = reads[k] as usize;
                                    let (lo, w) = mets[k];
                                    snaps[k] = Self::raw_bits_slice(&st[s], lo, w);
                                }
                                self.edge_block_snap_valid[bi] = true;
                                if bi < self.edge_block_armed.len() {
                                    self.edge_block_armed[bi] &= !EDGE_HOLD;
                                }
                            }
                        }
                    }
                    triggered[w] = bi;
                    w += 1;
                } else {
                    would_skip_delta += 1;
                    if bi < triggered_bitmap.len() {
                        triggered_bitmap[bi] = false;
                    }
                }
            }
            // Fold the register-accumulated counters back.
            self.event_gateable_total += gateable_delta;
            self.event_would_skip += would_skip_delta;
            self.armed_fast_skips += fast_skip_delta;
            self.event_snapshot_checks += snap_check_delta;
            triggered.truncate(w);
        } else if self.event_measure {
            let now = self.event_phase;
            for &bi in &triggered {
                if self.edge_block_gateable.get(bi).copied().unwrap_or(false) {
                    self.event_gateable_total += 1;
                    let last_fire = self.flop_last_fire[bi];
                    // Skippable iff no data input changed since this flop's
                    // previous SAMPLE phase. Writes from the edge execution
                    // itself may share that phase, so equality is treated as
                    // changed conservatively.
                    let changed = self.edge_block_data_reads[bi]
                        .iter()
                        .any(|&s| self.sig_last_change[s as usize] >= last_fire);
                    if !changed {
                        self.event_would_skip += 1;
                    }
                    self.flop_last_fire[bi] = now;
                }
            }
        }

        // Stall attribution: while the exec-write rescan drain is looping,
        // count executions per block so a livelock report can name the
        // ping-ponging always blocks.
        if self.in_edge_rescan && !triggered.is_empty() {
            for &bi in &triggered {
                *self.edge_rescan_block_hits.entry(bi).or_insert(0) += 1;
            }
        }

        // Attribute edge work that spans multiple scheduler delta cycles at
        // one timestamp. Normal timed clock ticks have stall_iters == 0, so
        // they do not pay for these diagnostic hash-map updates.
        if self.stall_limit > 0 && self.stall_iters > 0 {
            for &bi in &triggered {
                *self.stall_edge_block_hits.entry(bi).or_insert(0) += 1;
            }
        }

        if !triggered.is_empty() {
            if let Some(pat) = self.edge_fire_trace.clone() {
                // One-time: precompute which blocks match, and resolve the
                // watch list. Per-dispatch cost is then a bitmap index per
                // fired block.
                if self.edge_fire_match.is_none() {
                    let m: Vec<bool> = self
                        .edge_blocks
                        .iter()
                        .map(|b| {
                            b.scope.contains(pat.as_str())
                                || b.resolved_sensitivities
                                    .iter()
                                    .any(|si| self.name_for_id(si.signal_id).contains(pat.as_str()))
                        })
                        .collect();
                    self.edge_fire_match = Some(m);
                }
                if !self.edge_fire_watch_init {
                    self.edge_fire_watch_init = true;
                    // A bare `EDGE_FIRE_WATCH` (missing prefix) was silently
                    // ignored — both prior customer traces ran with watch=[]
                    // because of exactly this. Accept the alias and always
                    // report the resolved configuration.
                    let w_env = std::env::var("XEZIM_EDGE_FIRE_WATCH")
                        .or_else(|_| std::env::var("XEZIM_EDGE_FIRE_WATCH"));
                    if w_env.is_err() {
                        eprintln!(
                            "[EDGE-FIRE] trace active but XEZIM_EDGE_FIRE_WATCH is not set — \
                             per-fire values will not be printed; set \
                             XEZIM_EDGE_FIRE_WATCH=<sig1,sig2,...>"
                        );
                    }
                    if let Ok(w) = w_env {
                        for patt in w.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                            let mut hit: Option<(String, usize)> = None;
                            self.signal_name_to_id.for_each_name(|n, id| {
                                if n.contains(patt)
                                    && hit.as_ref().is_none_or(|(h, _)| n.len() < h.len())
                                {
                                    hit = Some((n.to_string(), id));
                                }
                                true
                            });
                            match hit {
                                Some((n, id)) => self.edge_fire_watch.push((n, id)),
                                None => eprintln!(
                                    "[EDGE-FIRE] watch pattern '{}' matched no signal",
                                    patt
                                ),
                            }
                        }
                        eprintln!(
                            "[EDGE-FIRE] watching {} signal(s): {:?}",
                            self.edge_fire_watch.len(),
                            self.edge_fire_watch
                                .iter()
                                .map(|(n, _)| n.as_str())
                                .collect::<Vec<_>>()
                        );
                    }
                }
                let matched = self.edge_fire_match.as_ref().unwrap();
                for &bi in &triggered {
                    if !matched.get(bi).copied().unwrap_or(false) {
                        continue;
                    }
                    let Some(b) = self.edge_blocks.get(bi) else {
                        continue;
                    };
                    let sens: Vec<String> = b
                        .resolved_sensitivities
                        .iter()
                        .map(|si| {
                            format!("{}{}", si.edge.print_str(), self.name_for_id(si.signal_id))
                        })
                        .collect();
                    let reads: Vec<String> = self
                        .edge_block_data_reads
                        .get(bi)
                        .map(|rs| {
                            rs.iter()
                                .map(|&sid| {
                                    let sid = sid as usize;
                                    let v = self
                                        .signal_table
                                        .get(sid)
                                        .map(|v| v.to_hex_string())
                                        .unwrap_or_else(|| "?".into());
                                    format!("{}='h{}", self.name_for_id(sid), v)
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let watch: Vec<String> = self
                        .edge_fire_watch
                        .iter()
                        .map(|(n, id)| {
                            let v = self
                                .signal_table
                                .get(*id)
                                .map(|v| v.to_hex_string())
                                .unwrap_or_else(|| "?".into());
                            format!("{}='h{}", n, v)
                        })
                        .collect();
                    eprintln!(
                        "[EDGE-FIRE] t={} window={} bi={} scope={} @({}) reads=[{}] watch=[{}]",
                        self.time,
                        self.edge_dispatch_phase,
                        bi,
                        if b.scope.is_empty() {
                            "<top>"
                        } else {
                            &b.scope
                        },
                        sens.join(","),
                        reads.join(", "),
                        watch.join(", ")
                    );
                }
            }
            let t1 = self.profile_timing.then(std::time::Instant::now);
            if self.edge_block_stats_enabled {
                for &bi in &triggered {
                    if let Some(count) = self.edge_block_exec_counts.get_mut(bi) {
                        *count += 1;
                    }
                }
            }

            // Separate into parallel-eligible and sequential blocks
            let mut parallel_blocks = std::mem::take(&mut self.edge_parallel_work);
            let mut sequential_blocks = std::mem::take(&mut self.edge_sequential_work);
            parallel_blocks.clear();
            sequential_blocks.clear();
            let mut parallel_insn_count = 0usize;
            for &bi in &triggered {
                if bi < self.edge_block_parallel.len() && self.edge_block_parallel[bi] {
                    parallel_insn_count +=
                        self.edge_block_insn_len.get(bi).copied().unwrap_or(0) as usize;
                    parallel_blocks.push(bi);
                } else {
                    sequential_blocks.push(bi);
                }
            }

            // Phase 2a: execute parallel-eligible blocks with thread::scope.
            //
            // Whether threading pays is NOT a property of the instruction count
            // alone — it depends on the machine (thread handoff + NBA merge cost
            // vs core count) and on the block SHAPE: 512 blocks of 40 insns each
            // clear a 10k total easily, yet each block is far too small to
            // amortize a hand-off, and the old fixed gate made such designs ~6x
            // SLOWER than sequential. So instead of guessing a constant, measure
            // it on this machine: spend the first `PAR_CAL_TICKS` qualifying ticks
            // sequential, the next `PAR_CAL_TICKS` parallel, compare ns/insn, and
            // keep the winner for the rest of the run.
            //
            // XEZIM_NO_PARALLEL=1 forces sequential; XEZIM_FORCE_PARALLEL=1 skips
            // calibration and always threads (for A/B measurement).
            const PAR_CAL_TICKS: u32 = 64;
            let parallel_disabled = parallel_disabled_by_env();
            let qualifies = parallel_blocks.len() >= 2 && parallel_insn_count >= 10_000;
            let use_parallel = if parallel_disabled {
                false
            } else if !qualifies {
                false
            } else if env_is_one_cached!("XEZIM_FORCE_PARALLEL") {
                true
            } else {
                match self.par_cal_phase {
                    0 => false, // measuring the sequential baseline
                    1 => true,  // measuring the parallel path
                    _ => self.par_cal_use_parallel,
                }
            };
            let cal_t0 = if qualifies && !parallel_disabled && self.par_cal_phase < 2 {
                Some(std::time::Instant::now())
            } else {
                None
            };
            if use_parallel {
                let signal_table = &self.signal_table;
                let signal_signed = &self.signal_signed;
                let signal_name_to_id = &self.signal_name_to_id;
                let array_first_id = &self.array_first_id;
                let packed = &self.packed;

                // Phase-1 partition-aware grouping: if a partition file
                // has been loaded (via --load-partition), bucket blocks
                // by their partition id so each thread processes one
                // partition worth of work. Falls back to the legacy
                // chunk-by-range when no partition is present.
                //
                // The partition path REQUIRES parallel_blocks to be
                // sorted by block_index — chunks bucket by partition id
                // (independent of block index), and the post-dispatch
                // k-way merge assumes each chunk's NBAs come out in
                // block-index order. Sorting AFTER bucketing would still
                // not satisfy that invariant. Without this sort, k=4 on
                // c910 hangs after ~50K cycles.
                //
                // The chunk-by-range path must NOT sort: chunk-by-range
                // chunks are contiguous slices of parallel_blocks, so
                // flattening them back preserves whatever order
                // `triggered` collected blocks in (sensitivity-fanout
                // order). c910 sequential dispatch has block-order
                // dependencies that break under index-sorted order
                // (c910 t=1 k=0 hangs at iters=200040 if we sort here).
                let use_partition =
                    self.edge_block_partition_count > 0 && !self.edge_block_partition.is_empty();
                if use_partition {
                    parallel_blocks.sort_unstable();
                }
                let mut block_slices: Vec<(usize, ParallelBlockSlice)> =
                    Vec::with_capacity(parallel_blocks.len());
                for &bi in &parallel_blocks {
                    if let Some(cb) = self.compiled_edge_blocks[bi].as_ref() {
                        block_slices.push((
                            bi,
                            ParallelBlockSlice {
                                ptr: cb.instructions.as_ptr(),
                                len: cb.instructions.len(),
                                num_regs: cb.num_regs as usize,
                                nba_dup: cb.nba_dup_targets,
                            },
                        ));
                    }
                }

                let mut chunks: Vec<Vec<(usize, ParallelBlockSlice)>>;
                if use_partition {
                    self.prof_par_dispatch_partition += 1;
                    let k = self.edge_block_partition_count as usize;
                    chunks = (0..k).map(|_| Vec::new()).collect();
                    for (bi, bs) in block_slices.into_iter() {
                        let p = self.edge_block_partition.get(bi).copied().unwrap_or(0) as usize;
                        let p = p.min(k - 1);
                        chunks[p].push((bi, bs));
                    }
                    chunks.retain(|c| !c.is_empty());
                } else {
                    self.prof_par_dispatch_legacy += 1;
                    let num_threads = std::thread::available_parallelism()
                        .map(|n| n.get().min(block_slices.len()).min(8))
                        .unwrap_or(2);
                    let chunk_size = block_slices.len().div_ceil(num_threads);
                    chunks = block_slices
                        .chunks(chunk_size)
                        .map(|c| c.to_vec())
                        .collect();
                }

                let mut all_nba: Vec<Vec<NbaFast>> = Vec::new();
                // XEZIM_PARALLEL_SERIALIZE=1: run exec_insns_isolated
                // sequentially in the main thread (chunks processed in
                // spawn order). Diagnoses whether the parallel-mode
                // c910 stall is in exec_insns_isolated's behavior vs
                // multi-thread scheduling. If it still fails with this
                // set, the bug is in exec_insns_isolated itself.
                let serialize = parallel_serialize_enabled();
                if serialize {
                    for chunk in chunks.iter() {
                        let chunk = chunk.as_slice();
                        let mut thread_nba: Vec<NbaFast> = Vec::new();
                        let max_regs = chunk.iter().map(|(_, bs)| bs.num_regs).max().unwrap_or(0);
                        let mut vm_regs = vec![Value::zero(1); max_regs];
                        for (bi, bs) in chunk {
                            if vm_regs.len() < bs.num_regs {
                                vm_regs.resize(bs.num_regs, Value::zero(1));
                            }
                            let insns = unsafe { std::slice::from_raw_parts(bs.ptr, bs.len) };
                            let mut nba = Self::exec_insns_isolated(
                                insns,
                                signal_table,
                                signal_signed,
                                signal_name_to_id,
                                array_first_id,
                                Some(packed),
                                &mut vm_regs,
                                *bi as u32,
                                bs.nba_dup,
                            );
                            thread_nba.append(&mut nba);
                        }
                        all_nba.push(thread_nba);
                    }
                } else {
                    std::thread::scope(|s| {
                        let mut handles = Vec::new();
                        for chunk in chunks.iter() {
                            let chunk = chunk.as_slice();
                            let handle = s.spawn(move || {
                                let mut thread_nba: Vec<NbaFast> = Vec::new();
                                let max_regs =
                                    chunk.iter().map(|(_, bs)| bs.num_regs).max().unwrap_or(0);
                                let mut vm_regs = vec![Value::zero(1); max_regs];
                                for (bi, bs) in chunk {
                                    if vm_regs.len() < bs.num_regs {
                                        vm_regs.resize(bs.num_regs, Value::zero(1));
                                    }
                                    let insns =
                                        unsafe { std::slice::from_raw_parts(bs.ptr, bs.len) };
                                    let mut nba = Self::exec_insns_isolated(
                                        insns,
                                        signal_table,
                                        signal_signed,
                                        signal_name_to_id,
                                        array_first_id,
                                        Some(packed),
                                        &mut vm_regs,
                                        *bi as u32,
                                        bs.nba_dup,
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
                }
                // Apply NBAs.
                //
                // Chunk-by-range path: chunks are contiguous slices of
                // parallel_blocks (which is in `triggered` order), so a
                // simple flatten reproduces the order legacy sequential
                // dispatch would have used. c910 sequential block-order
                // dependencies require this exact order — k-way merging
                // by block_index here would change behavior and hang.
                //
                // Partition path: chunks are bucketed by partition id
                // (independent of block index), so flattening loses
                // global ordering and picks the wrong "last writer" for
                // signals written from different partitions. Each chunk
                // is already block-index sorted (parallel_blocks sorted
                // upstream when use_partition is true), so a k-way
                // min-heap merge reconstructs global block-index order
                // in O(N log k).
                if use_partition {
                    let total: usize = all_nba.iter().map(Vec::len).sum();
                    if total > 0 {
                        self.nba_fast.reserve(total);
                        let mut iters: Vec<std::vec::IntoIter<NbaFast>> =
                            all_nba.into_iter().map(|v| v.into_iter()).collect();
                        let mut head: Vec<Option<NbaFast>> =
                            (0..iters.len()).map(|_| None).collect();
                        for (i, it) in iters.iter_mut().enumerate() {
                            if let Some(n) = it.next() {
                                head[i] = Some(n);
                            }
                        }
                        use std::cmp::Reverse;
                        use std::collections::BinaryHeap;
                        let mut heap: BinaryHeap<(Reverse<u32>, usize)> =
                            BinaryHeap::with_capacity(iters.len());
                        for (ci, entry) in head.iter().enumerate() {
                            if let Some(entry) = entry {
                                heap.push((Reverse(entry.block_index), ci));
                            }
                        }
                        while let Some((_, ci)) = heap.pop() {
                            if let Some(entry) = head[ci].take() {
                                self.nba_fast_index
                                    .insert(entry.signal_id, self.nba_fast.len());
                                self.nba_fast.push(entry);
                            }
                            if let Some(n) = iters[ci].next() {
                                heap.push((Reverse(n.block_index), ci));
                                head[ci] = Some(n);
                            }
                        }
                    }
                } else {
                    for nba_batch in all_nba {
                        for entry in nba_batch {
                            self.nba_fast_index
                                .insert(entry.signal_id, self.nba_fast.len());
                            self.nba_fast.push(entry);
                        }
                    }
                }
            } else {
                // Sequential: either the tick is too small to be worth threading,
                // or calibration found threading to be a net loss on this machine.
                for &bi in &parallel_blocks {
                    let t = if self.edge_block_stats_enabled {
                        Some(std::time::Instant::now())
                    } else {
                        None
                    };
                    if self.profile_report {
                        self.prof_cur.store(
                            crate::compiler::prof_sampler::KIND_EDGE | bi as u64,
                            std::sync::atomic::Ordering::Relaxed,
                        );
                    }
                    self.exec_bytecode(bi);
                    if self.profile_report {
                        self.prof_cur.store(
                            crate::compiler::prof_sampler::KIND_OTHER,
                            std::sync::atomic::Ordering::Relaxed,
                        );
                    }
                    if let Some(t) = t {
                        if let Some(slot) = self.edge_block_exec_ns.get_mut(bi) {
                            *slot += t.elapsed().as_nanos() as u64;
                        }
                    }
                }
            }

            // The parallel path executes bytecode on worker threads, which
            // cannot touch `prof_insns_executed` — so `insns=0`/`ns_per_insn=0`
            // used to be reported for the very path that was running, and a
            // profile that reads zero is worse than one that reads nothing.
            // The workers' EXECUTED count is not recoverable without threading
            // a counter through the whole worker protocol (and a block's static
            // length is NOT its executed length — a `case` runs one arm), so
            // record the dispatch itself and let the PROF line say so.
            if use_parallel {
                self.prof_par_ticks += 1;
                self.prof_par_blocks += parallel_blocks.len() as u64;
            }

            // Calibration bookkeeping: fold this tick's cost into whichever
            // phase we are measuring, and lock in the winner once both are
            // sampled. ns/insn is the comparison metric so an uneven tick size
            // between the two phases cannot skew the decision.
            if let Some(t0) = cal_t0 {
                let ns = t0.elapsed().as_nanos();
                match self.par_cal_phase {
                    0 => {
                        self.par_cal_seq_ns += ns;
                        self.par_cal_seq_insns += parallel_insn_count as u64;
                    }
                    1 => {
                        self.par_cal_par_ns += ns;
                        self.par_cal_par_insns += parallel_insn_count as u64;
                    }
                    _ => {}
                }
                self.par_cal_ticks += 1;
                if self.par_cal_ticks >= PAR_CAL_TICKS {
                    self.par_cal_ticks = 0;
                    if self.par_cal_phase == 0 {
                        self.par_cal_phase = 1;
                    } else if self.par_cal_phase == 1 {
                        let seq = if self.par_cal_seq_insns > 0 {
                            self.par_cal_seq_ns as f64 / self.par_cal_seq_insns as f64
                        } else {
                            f64::INFINITY
                        };
                        let par = if self.par_cal_par_insns > 0 {
                            self.par_cal_par_ns as f64 / self.par_cal_par_insns as f64
                        } else {
                            f64::INFINITY
                        };
                        // Require a real margin (>10%) before paying the
                        // threading cost — a coin-flip difference is not worth
                        // the nondeterminism.
                        self.par_cal_use_parallel = par * 1.10 < seq;
                        self.par_cal_phase = 2;
                        sim_dbg_eprintln!(
                            "[OPT] parallel edge dispatch calibrated: seq={:.1} ns/insn par={:.1} ns/insn -> {}",
                            seq,
                            par,
                            if self.par_cal_use_parallel {
                                "PARALLEL"
                            } else {
                                "SEQUENTIAL"
                            }
                        );
                    }
                }
            }

            // Phase 2b: execute sequential blocks on main thread.
            // Skip the name_resolve_hint save/restore for blocks that have
            // no StmtFallback insns (common case: 99%+ of edges don't fall
            // back to AST exec, since bytecode pre-resolves signal IDs).
            for &bi in &sequential_blocks {
                if self.finished {
                    break;
                }
                let needs_hint =
                    bi < self.edge_block_needs_hint.len() && self.edge_block_needs_hint[bi];
                let t_seq = if self.edge_block_stats_enabled {
                    Some(std::time::Instant::now())
                } else {
                    None
                };
                if self.profile_report {
                    self.prof_cur.store(
                        crate::compiler::prof_sampler::KIND_EDGE | bi as u64,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                }
                if needs_hint {
                    // §6.21: an AST fallback belongs to this always process,
                    // not to a suspended task that caused edge dispatch.
                    let saved_context = if self.process_context_is_default() {
                        None
                    } else {
                        Some(self.take_process_context())
                    };
                    let saved_hint = self.name_resolve_hint.borrow().clone();
                    // Same activation-scope install as run_process_stmts:
                    // the fallback stmts execute as THIS block's activation
                    // ("" for a top block — its own names are the
                    // bare keys), ratchet-proof against a mid-block
                    // hierarchical reference redirecting the block's bare
                    // names downward into an instance.
                    let saved_activation_scope = self.activation_scope.replace(Some(
                        self.edge_block_scope
                            .get(bi)
                            .cloned()
                            .flatten()
                            .unwrap_or_default(),
                    ));
                    if let Some(scope) = self.edge_block_scope.get(bi).and_then(|s| s.as_ref()) {
                        *self.name_resolve_hint.borrow_mut() = Some(scope.clone());
                        // $time/%t in the fallback body must scale to THIS
                        // block's module timescale (see current_module_def).
                        self.timescale_scope_override = Some(scope.clone());
                    }
                    if !self.exec_bytecode(bi) {
                        self.exec_statement(&blocks[bi].stmt);
                    }
                    self.timescale_scope_override = None;
                    *self.name_resolve_hint.borrow_mut() = saved_hint;
                    *self.activation_scope.borrow_mut() = saved_activation_scope;
                    if let Some(ctx) = saved_context {
                        self.restore_process_context(ctx);
                    }
                } else {
                    self.exec_bytecode(bi);
                }
                if self.profile_report {
                    self.prof_cur.store(
                        crate::compiler::prof_sampler::KIND_OTHER,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                }
                if let Some(t) = t_seq {
                    if let Some(slot) = self.edge_block_exec_ns.get_mut(bi) {
                        *slot += t.elapsed().as_nanos() as u64;
                    }
                }
            }

            if let Some(t) = t1 {
                self.prof_edge_exec += t.elapsed().as_nanos() as u64;
            }
            parallel_blocks.clear();
            sequential_blocks.clear();
            self.edge_parallel_work = parallel_blocks;
            self.edge_sequential_work = sequential_blocks;
        }

        for &bi in &triggered {
            if bi < self.edge_triggered_bitmap.len() {
                self.edge_triggered_bitmap[bi] = false;
            }
        }
        triggered.clear();
        self.edge_triggered_list = triggered;

        // Trigger covergroup sampling (already done pre-continuation in
        // waiters-first mode — §19.5 samples must see the values current AT
        // the event, before a same-edge process continuation mutates them).
        if !waiters_first {
            self.sample_edge_covergroups();
        }

        // Wake up event_waiters whose sensitivity conditions are met.
        // IEEE 1800-2023 §4.4.5: triggered continuations run in the
        // active region of the same time-step (before NBA region).
        // Collect their continuations locally and run them INLINE
        // below — DON'T schedule into event_queue, because event_queue
        // is also used for #0-delay events that must NOT run in the
        // active region. Inline execution restricts the drain to only
        // event_waiter continuations.
        let _t_w = self.profile_timing.then(std::time::Instant::now);
        let triggered_conts = if waiters_first {
            Vec::new() // already drained and run before the edge blocks
        } else {
            self.drain_triggered_event_waiters()
        };
        if let Some(t) = _t_w {
            self.prof_edge_waiters += t.elapsed().as_nanos() as u64;
        }
        // Consume the edges dispatched this pass: refresh prev for every
        // FIRED position to its pre-exec value (captured at detect time,
        // AFTER the cg/waiter checks above so those still saw the original
        // prev). A blocking write made by a triggered block during exec now
        // reads as a fresh edge against this prev, so the rescan drain can
        // re-trigger sensitive blocks without re-firing the edge already
        // consumed here.
        for &(sid, v, x) in &fired_snap {
            self.prev_val[sid] = v;
            self.prev_xz[sid] = x;
        }
        for (sid, val) in fired_wide.drain(..) {
            if let Some(p) = self.prev_wide.get_mut(&sid) {
                *p = val;
            }
        }
        self.edge_fired_snap = fired_snap;
        self.edge_fired_wide = fired_wide;
        drop(blocks);
        self.edge_pass_depth -= 1;
        // A nested pass must not clear the flag out from under its caller.
        self.in_edge_block = !outermost_pass;

        // Run the triggered waiter continuations IMMEDIATELY (active
        // region of current time-step, before apply_nba commits) so
        // their NBA pushes join the same nba_fast queue as the
        // edge_blocks just fired — IEEE 1800-2023 §4.4.5 semantics.
        //
        // Default is now inline active-region execution — reference-verified
        // (a waiter resumed by an edge reads the PRE-NBA value of a reg that
        // same edge updates). XEZIM_ACTIVE_REGION=0 restores the legacy
        // schedule-into-event_queue path (waiter runs in the NEXT event_loop
        // iter after apply_nba, reading post-NBA values).
        let active_region = {
            static FLAG: OnceLock<bool> = OnceLock::new();
            *FLAG.get_or_init(|| std::env::var("XEZIM_ACTIVE_REGION").ok().as_deref() != Some("0"))
        };
        // Record continuation writes for the §9.2 rescan drain (see
        // `in_edge_cont`): these run AFTER this pass's edge detection, so
        // without recording, an AnyEdge block sensitive to a signal a
        // continuation writes never re-fires (change swallowed by the next
        // prev refresh). SAVE/RESTORE, not set/clear: a continuation whose
        // body hits `#0` re-enters check_edges synchronously (nested pass),
        // and an unconditional clear there stripped the flag before the
        // post-#0 write executed — losing exactly the event this exists for.
        let __prev_edge_cont = self.in_edge_cont;
        self.in_edge_cont = true;
        let mut triggered_conts = triggered_conts;
        for (pid, stmts) in triggered_conts.drain(..) {
            if self.finished {
                break;
            }
            if active_region {
                // Inline execution — see tests/nba_leak_waiter_active_region.rs
                self.run_scheduled_process(pid, &stmts);
                if !self.is_pid_suspended(pid) {
                    self.child_finished(pid);
                }
                self.break_flag = false;
                self.continue_flag = false;
                self.return_flag = false;
            } else {
                // Legacy: schedule for next event_loop iter
                self.event_queue.schedule(self.time, pid, stmts);
            }
        }
        self.recycle_conts(triggered_conts);
        if waiters_first || active_region {
            // Waiters whose trigger was produced DURING this pass — by the
            // blocks just executed (`-> ev` inside an always_ff) or, in
            // blocks-first mode, by a continuation the inline loop above just
            // resumed (`-> ev` right after `@(posedge clk)`) — cannot have
            // been in the earlier drain. §15.5.1: they resume in the SAME
            // active region — before this slot's NBA region — but the next
            // check_edges that would notice them sits in the cascade AFTER
            // apply_nba (or never comes, if no further event wakes this
            // slot). Drain-and-run inline, re-draining because a resumed
            // continuation may fire further events.
            let mut settle_guard = 0u32;
            loop {
                let mut conts = self.drain_triggered_event_waiters();
                if conts.is_empty() {
                    self.recycle_conts(conts);
                    break;
                }
                for (pid, stmts) in conts.drain(..) {
                    if self.finished {
                        break;
                    }
                    self.run_scheduled_process(pid, &stmts);
                    if !self.is_pid_suspended(pid) {
                        self.child_finished(pid);
                    }
                    self.break_flag = false;
                    self.continue_flag = false;
                    self.return_flag = false;
                }
                self.recycle_conts(conts);
                settle_guard += 1;
                if self.finished || settle_guard > 10_000 {
                    if env_set_cached!("XEZIM_TRACE_SPIN") && !self.finished {
                        eprintln!(
                            "[DRAINGUARD] t={} exhausted; {} waiters:",
                            self.time,
                            self.event_waiters.len()
                        );
                        for w in self.event_waiters.iter().take(6) {
                            let names: Vec<String> = w
                                .resolved_sensitivities
                                .iter()
                                .map(|sid| {
                                    let nm = self
                                        .name_opt(sid.signal_id)
                                        .map(|s| s.to_string())
                                        .unwrap_or_else(|| format!("id{}", sid.signal_id));
                                    let cur =
                                        self.signal_table.get(sid.signal_id).map(|v| v.to_u64());
                                    format!(
                                        "{nm}:{:?} cur={:?} prev={:?}",
                                        sid.edge,
                                        cur,
                                        w.captured_prev.first()
                                    )
                                })
                                .collect();
                            eprintln!("  pid={} sens={:?}", w.pid, names);
                        }
                    }
                    break;
                }
            }
        }
        self.in_edge_cont = __prev_edge_cont;
    }
}
