//! Simulator profiling.

use super::*;
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

impl Simulator {
    /// Phase-1 multicore exploration: dump the edge-block dependency
    /// graph as an hMETIS hypergraph (vertex = parallel-eligible edge
    /// block, hyperedge = signal connecting all blocks that read or
    /// NBA-write it). Vertex weight = bytecode-instruction count
    /// (static cost estimate). Hyperedge weight = signal width.
    ///
    /// Only blocks marked `edge_block_parallel[i] = true` participate;
    /// non-pure blocks (StmtFallback / blocking writes) cannot
    /// safely cross-core anyway and stay on the sequential path.
    /// The vertex-id space is dense over those blocks, with mapping
    /// `mt_kahypar_vid → original edge_block_idx` written to a sibling
    /// file `<path>.vmap` so the partition file can be inverted back.
    /// Phase-2 profile snapshot: per-block runtime (ns) and per-signal
    /// toggle counts collected during a stats-enabled run. Serialized
    /// as a tiny line-oriented text format (no JSON dep). Symmetric
    /// with `--write-profile` / `--profile-input`.
    pub fn write_phase2_profile(&self, path: &str) -> Result<(), String> {
        use std::io::Write;
        if !self.edge_block_stats_enabled {
            return Err(
                "profile collection requires XEZIM_EDGE_BLOCK_STATS=1 at run time".to_string(),
            );
        }
        let mut out = std::io::BufWriter::new(
            std::fs::File::create(path).map_err(|e| format!("create {}: {}", path, e))?,
        );
        writeln!(out, "# xezim phase2/4 profile v2").map_err(|e| e.to_string())?;
        writeln!(
            out,
            "# blocks={} signals={}",
            self.edge_block_exec_ns.len(),
            self.signal_toggle_count.len()
        )
        .map_err(|e| e.to_string())?;
        writeln!(out, "[edge_block_exec]").map_err(|e| e.to_string())?;
        for (bi, ns) in self.edge_block_exec_ns.iter().enumerate() {
            if *ns == 0 {
                continue;
            }
            let count = self.edge_block_exec_counts.get(bi).copied().unwrap_or(0);
            writeln!(out, "{} {} {}", bi, *ns, count).map_err(|e| e.to_string())?;
        }
        writeln!(out, "[signal_toggle]").map_err(|e| e.to_string())?;
        for (sid, t) in self.signal_toggle_count.iter().enumerate() {
            if *t == 0 {
                continue;
            }
            writeln!(out, "{} {}", sid, *t).map_err(|e| e.to_string())?;
        }
        // Phase-4 sections.
        writeln!(out, "[nba_count_per_block]").map_err(|e| e.to_string())?;
        for (bi, n) in self.nba_count_per_block.iter().enumerate() {
            if *n == 0 {
                continue;
            }
            writeln!(out, "{} {}", bi, *n).map_err(|e| e.to_string())?;
        }
        writeln!(out, "[sensitivity_trigger]").map_err(|e| e.to_string())?;
        for (sid, t) in self.sensitivity_trigger_count.iter().enumerate() {
            if *t == 0 {
                continue;
            }
            writeln!(out, "{} {}", sid, *t).map_err(|e| e.to_string())?;
        }
        writeln!(out, "[cross_block_wakeup]").map_err(|e| e.to_string())?;
        for (bi, w) in self.cross_block_wakeup_count.iter().enumerate() {
            if *w == 0 {
                continue;
            }
            writeln!(out, "{} {}", bi, *w).map_err(|e| e.to_string())?;
        }
        // Boundary packet count + local quiescence iters: derived from
        // the current partition if one is loaded. Boundary packets are
        // signals NBA-written by partition P that are read by some
        // other partition. Local quiescence is the count of settle
        // iterations whose NBAs landed entirely within one partition.
        if self.edge_block_partition_count > 0 {
            let k = self.edge_block_partition_count as usize;
            let mut boundary = vec![0u64; k];
            // Build sets: writers by partition × readers by partition.
            for (sid, _) in self.signal_toggle_count.iter().enumerate() {
                let writers: ahash::AHashSet<u32> = (0..self.edge_blocks.len())
                    .filter(|&bi| self.edge_block_parallel.get(bi).copied().unwrap_or(false))
                    .filter(|&bi| {
                        self.compiled_edge_blocks
                            .get(bi)
                            .and_then(|c| c.as_ref())
                            .is_some_and(|cb| {
                                cb.instructions.iter().any(|i| match i {
                                    super::bytecode::Insn::NbaAssign(s, _, _)
                                    | super::bytecode::Insn::NbaAssignConst(s, _, _)
                                    | super::bytecode::Insn::NbaAssignArrayRead(s, _, _, _)
                                    | super::bytecode::Insn::NbaAssignRange(s, _, _, _) => {
                                        let s = &(*s as usize);
                                        *s == sid
                                    }
                                    _ => false,
                                })
                            })
                    })
                    .map(|bi| self.edge_block_partition.get(bi).copied().unwrap_or(0))
                    .collect();
                let readers: ahash::AHashSet<u32> = (0..self.edge_blocks.len())
                    .filter(|&bi| self.edge_block_parallel.get(bi).copied().unwrap_or(false))
                    .filter(|&bi| {
                        self.compiled_edge_blocks
                            .get(bi)
                            .and_then(|c| c.as_ref())
                            .is_some_and(|cb| {
                                cb.instructions.iter().any(|i| {
                                    matches!(i,
                                    super::bytecode::Insn::LoadSignal(_, s)
                                    | super::bytecode::Insn::LoadSignalSigned(_, s)
                                    | super::bytecode::Insn::LoadSignalRange(_, s, _, _)
                                    | super::bytecode::Insn::LoadSignalBit(_, s, _)
                                    | super::bytecode::Insn::NbaAssignArrayRead(_, _, s, _)
                                    | super::bytecode::Insn::BranchIfSignalFalse(s, _, _) if *s as usize == sid)
                                })
                            })
                    })
                    .map(|bi| self.edge_block_partition.get(bi).copied().unwrap_or(0))
                    .collect();
                // For each writer partition, if any READER is in a
                // different partition, count one boundary packet.
                for &wp in &writers {
                    if readers.iter().any(|&rp| rp != wp) && (wp as usize) < boundary.len() {
                        boundary[wp as usize] += 1;
                    }
                }
            }
            writeln!(out, "[boundary_packet]").map_err(|e| e.to_string())?;
            for (p, b) in boundary.iter().enumerate() {
                writeln!(out, "{} {}", p, *b).map_err(|e| e.to_string())?;
            }
            writeln!(out, "[local_quiescence]").map_err(|e| e.to_string())?;
            for (p, q) in self.local_quiescence_iterations.iter().enumerate() {
                writeln!(out, "{} {}", p, *q).map_err(|e| e.to_string())?;
            }
        }
        out.flush().map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Phase-3 event-island analysis: detect groups of edge blocks
    /// that MUST be co-located on the same partition because splitting
    /// them across cores would require an illegal or expensive
    /// zero-time barrier mid-cycle. Returns Union-Find roots: per-block
    /// representative id (canonical block_idx). Two blocks are in the
    /// same island iff `roots[a] == roots[b]`.
    ///
    /// What's collapsed:
    /// - **Async-reset cones**: any block whose sensitivity list has
    ///   ≥2 distinct edge signals (e.g. `posedge clk or negedge rst`)
    ///   merges with every other parallel-eligible block driven by
    ///   the SAME secondary signal. This catches the classic FF-with-
    ///   async-reset family — splitting them risks the reset edge
    ///   firing on one core while the dependent FFs are mid-update on
    ///   another.
    /// - **Combinational SCCs**: builds a directed graph
    ///   `A → B` when A NBA-writes a signal B reads. Tarjan SCCs of
    ///   size ≥2 (mutual recursion through NBA) are collapsed.
    /// - Clock-gating cones and latch cones are detected indirectly:
    ///   a gated clock arrives via a comb assign which currently
    ///   marks its driven block non-pure (already sequential), and
    ///   latches are SystemVerilog `always @*` with conditional NBA
    ///   that we similarly treat as non-pure. Phase 3 doesn't need
    ///   to handle them explicitly — they don't enter the parallel
    ///   set in the first place.
    pub fn compute_phase3_islands(&self) -> Vec<usize> {
        let n = self.edge_blocks.len();
        // Union-Find over block_idx. Path compression + union-by-rank.
        let mut parent: Vec<usize> = (0..n).collect();
        let mut rank: Vec<u8> = vec![0; n];
        fn find(parent: &mut [usize], mut x: usize) -> usize {
            while parent[x] != x {
                parent[x] = parent[parent[x]];
                x = parent[x];
            }
            x
        }
        fn union(parent: &mut [usize], rank: &mut [u8], a: usize, b: usize) {
            let (ra, rb) = (find(parent, a), find(parent, b));
            if ra == rb {
                return;
            }
            if rank[ra] < rank[rb] {
                parent[ra] = rb;
            } else if rank[ra] > rank[rb] {
                parent[rb] = ra;
            } else {
                parent[rb] = ra;
                rank[ra] += 1;
            }
        }

        // --- 3b: async-reset cones ---
        // Build map sig_id → list of parallel block_idxs that have it
        // in their sensitivity list. Then for any block with multiple
        // sensitivities, union all blocks sharing each of those sigs.
        let mut sens_to_blocks: ahash::AHashMap<usize, Vec<usize>> = ahash::AHashMap::default();
        for (bi, blk) in self.edge_blocks.iter().enumerate() {
            if !self.edge_block_parallel.get(bi).copied().unwrap_or(false) {
                continue;
            }
            for s in &blk.resolved_sensitivities {
                sens_to_blocks.entry(s.signal_id).or_default().push(bi);
            }
        }
        let mut cone_unions = 0usize;
        for (bi, blk) in self.edge_blocks.iter().enumerate() {
            if !self.edge_block_parallel.get(bi).copied().unwrap_or(false) {
                continue;
            }
            // Distinct sensitivity signals (a clock or reset, etc).
            let mut sids: Vec<usize> = blk
                .resolved_sensitivities
                .iter()
                .map(|s| s.signal_id)
                .collect();
            sids.sort();
            sids.dedup();
            if sids.len() < 2 {
                continue; // single-signal sensitivity = clean clock domain
            }
            // For each sensitivity signal, merge bi with every other
            // parallel block also driven by that signal — that's the
            // "cone" for this trigger source.
            for &sid in &sids {
                if let Some(peers) = sens_to_blocks.get(&sid) {
                    for &peer in peers {
                        if peer != bi {
                            union(&mut parent, &mut rank, bi, peer);
                            cone_unions += 1;
                        }
                    }
                }
            }
        }

        // --- 3a: combinational SCCs over the NBA dataflow graph ---
        // Edge bi → bj when `bi` NBA-writes a signal `bj` reads.
        // For parallel-eligible blocks only.
        let mut writes_of: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut reads_of: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut writers_of_sig: ahash::AHashMap<usize, Vec<usize>> = ahash::AHashMap::default();
        let mut readers_of_sig: ahash::AHashMap<usize, Vec<usize>> = ahash::AHashMap::default();
        for bi in 0..n {
            if !self.edge_block_parallel.get(bi).copied().unwrap_or(false) {
                continue;
            }
            let cb = match self.compiled_edge_blocks.get(bi).and_then(|c| c.as_ref()) {
                Some(c) => c,
                None => continue,
            };
            for insn in cb.instructions.iter() {
                match insn {
                    // The fused array-read NBA both READS its index signal and
                    // WRITES its destination, so it has to land in BOTH sets —
                    // the alternations below are mutually exclusive per insn.
                    super::bytecode::Insn::NbaAssignArrayRead(wsid, _, rsid, _) => {
                        let (w, r) = (*wsid as usize, *rsid as usize);
                        if !reads_of[bi].contains(&r) {
                            reads_of[bi].push(r);
                            readers_of_sig.entry(r).or_default().push(bi);
                        }
                        if !writes_of[bi].contains(&w) {
                            writes_of[bi].push(w);
                            writers_of_sig.entry(w).or_default().push(bi);
                        }
                    }
                    super::bytecode::Insn::LoadSignal(_, sid)
                    | super::bytecode::Insn::LoadSignalSigned(_, sid)
                    | super::bytecode::Insn::LoadSignalRange(_, sid, _, _)
                    | super::bytecode::Insn::LoadSignalBit(_, sid, _)
                    | super::bytecode::Insn::BranchIfSignalFalse(sid, _, _)
                        if !reads_of[bi].contains(&(*sid as usize)) =>
                    {
                        reads_of[bi].push(*sid as usize);
                        readers_of_sig.entry(*sid as usize).or_default().push(bi);
                    }
                    super::bytecode::Insn::NbaAssign(sid, _, _)
                    | super::bytecode::Insn::NbaAssignConst(sid, _, _)
                    | super::bytecode::Insn::NbaAssignRange(sid, _, _, _)
                        if !writes_of[bi].contains(&(*sid as usize)) =>
                    {
                        writes_of[bi].push(*sid as usize);
                        writers_of_sig.entry(*sid as usize).or_default().push(bi);
                    }
                    _ => {}
                }
            }
        }
        // Build adjacency `succ[bi]` = blocks that read something bi writes.
        let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n];
        for bi in 0..n {
            for &sid in &writes_of[bi] {
                if let Some(readers) = readers_of_sig.get(&sid) {
                    for &r in readers {
                        if r != bi && !succ[bi].contains(&r) {
                            succ[bi].push(r);
                        }
                    }
                }
            }
        }
        // Tarjan iterative SCC. Skips non-parallel blocks (no out-edges).
        let mut index = 0i64;
        let mut stack: Vec<usize> = Vec::new();
        let mut on_stack = vec![false; n];
        let mut idx_of: Vec<i64> = vec![-1; n];
        let mut low: Vec<i64> = vec![-1; n];
        let mut scc_unions = 0usize;
        // Recursion-as-stack: (node, child_iter_pos)
        for start in 0..n {
            if idx_of[start] != -1 {
                continue;
            }
            if !self
                .edge_block_parallel
                .get(start)
                .copied()
                .unwrap_or(false)
            {
                continue;
            }
            let mut work: Vec<(usize, usize)> = Vec::new();
            idx_of[start] = index;
            low[start] = index;
            index += 1;
            stack.push(start);
            on_stack[start] = true;
            work.push((start, 0));
            while let Some(&(v, ci)) = work.last() {
                if ci < succ[v].len() {
                    let w = succ[v][ci];
                    work.last_mut().unwrap().1 += 1;
                    if idx_of[w] == -1 {
                        idx_of[w] = index;
                        low[w] = index;
                        index += 1;
                        stack.push(w);
                        on_stack[w] = true;
                        work.push((w, 0));
                    } else if on_stack[w] && idx_of[w] < low[v] {
                        low[v] = idx_of[w];
                    }
                } else {
                    // Done with v: maybe pop SCC.
                    if low[v] == idx_of[v] {
                        let mut comp: Vec<usize> = Vec::new();
                        loop {
                            let x = stack.pop().expect("stack underflow");
                            on_stack[x] = false;
                            comp.push(x);
                            if x == v {
                                break;
                            }
                        }
                        if comp.len() > 1 {
                            // Union all members.
                            let head = comp[0];
                            for &m in &comp[1..] {
                                union(&mut parent, &mut rank, head, m);
                                scc_unions += 1;
                            }
                        }
                    }
                    work.pop();
                    if let Some(&(p, _)) = work.last() {
                        if low[v] < low[p] {
                            low[p] = low[v];
                        }
                    }
                }
            }
        }

        // Path-compress so callers can read `parent[bi]` directly.
        for i in 0..n {
            let r = find(&mut parent, i);
            parent[i] = r;
        }
        // Stats line — useful for confirming the analysis fired.
        let mut roots: ahash::AHashSet<usize> = ahash::AHashSet::default();
        for &p in &parent {
            roots.insert(p);
        }
        eprintln!(
            "[PART] phase-3 islands: {} blocks → {} super-vertices (cone_unions={}, scc_unions={})",
            n,
            roots.len(),
            cone_unions,
            scc_unions
        );
        parent
    }

    pub fn emit_edge_block_hypergraph(&self, path: &str) -> Result<(usize, usize), String> {
        self.emit_edge_block_hypergraph_with_profile(path, None)
    }

    /// Phase-2 profile-guided variant of `emit_edge_block_hypergraph`.
    /// When `profile` is `Some`, vertex weights come from measured
    /// runtime (ns) × fire-count and hyperedge weights from
    /// signal toggle count × signal width — both are real-workload
    /// proxies, much better than the static insn-count / width
    /// estimates the unprofiled path uses.
    ///
    /// The profile is the JSON written by a prior run with
    /// `--write-profile`. Vertices missing from the profile fall
    /// back to the static estimate so a partial profile still works.
    pub fn emit_edge_block_hypergraph_with_profile(
        &self,
        path: &str,
        profile: Option<&Phase2Profile>,
    ) -> Result<(usize, usize), String> {
        self.emit_edge_block_hypergraph_full(path, profile, None)
    }

    /// Phase-3 hypergraph emit: when `islands` is `Some`, blocks
    /// sharing an island root collapse into one super-vertex with
    /// summed weights. The vmap then records the canonical block_idx
    /// for each super-vertex; load_partition_file already inherits
    /// partition assignments via the vmap so all members of a
    /// super-vertex get the same partition automatically.
    pub fn emit_edge_block_hypergraph_full(
        &self,
        path: &str,
        profile: Option<&Phase2Profile>,
        islands: Option<&[usize]>,
    ) -> Result<(usize, usize), String> {
        use std::io::Write;
        use xezim_core::hasher::HashMap;

        // Step 1: collect parallel-eligible block ids. With Phase 3
        // islands, dedup by island root so each super-vertex (group
        // of must-co-locate blocks) takes exactly one vid. Members
        // of the same island map to the same vid.
        let mut all_eligible: Vec<usize> = Vec::new();
        for (idx, &is_parallel) in self.edge_block_parallel.iter().enumerate() {
            if is_parallel
                && self
                    .compiled_edge_blocks
                    .get(idx)
                    .and_then(|cb| cb.as_ref())
                    .is_some()
            {
                all_eligible.push(idx);
            }
        }
        if all_eligible.is_empty() {
            return Err("no parallel-eligible edge blocks to partition".to_string());
        }

        // `blocks` = canonical representative per super-vertex.
        // `island_of[bi]` = the canonical block_idx for bi's island.
        // When `islands` is None each block is its own island.
        let n = self.edge_blocks.len();
        let island_of: Vec<usize> = match islands {
            Some(roots) if roots.len() == n => roots.to_vec(),
            _ => (0..n).collect(),
        };
        let mut blocks: Vec<usize> = Vec::new();
        let mut seen_root: ahash::AHashSet<usize> = ahash::AHashSet::default();
        for &bi in &all_eligible {
            let r = island_of[bi];
            if seen_root.insert(r) {
                blocks.push(r);
            }
        }
        let vid_of: HashMap<usize, usize> = blocks
            .iter()
            .enumerate()
            .map(|(i, &root)| (root, i + 1))
            .collect();
        // Map every eligible block to its super-vertex's vid via root.
        let block_vid = |bi: usize| -> Option<usize> { vid_of.get(&island_of[bi]).copied() };
        // Members per island (for vmap output and weight summing).
        let mut members_of: HashMap<usize, Vec<usize>> = HashMap::default();
        for &bi in &all_eligible {
            members_of.entry(island_of[bi]).or_default().push(bi);
        }

        // Step 2: walk each ELIGIBLE block's bytecode (not just the
        // representative) and collect signal ids. Map each (sid,
        // super-vertex_vid) pair so the hyperedge captures the
        // COMBINED read/write set of the island. Dedup so each vid
        // appears at most once per signal.
        let mut sig_to_vids: HashMap<usize, Vec<usize>> = HashMap::default();
        for &bi in &all_eligible {
            let vid = match block_vid(bi) {
                Some(v) => v,
                None => continue,
            };
            let cb = self.compiled_edge_blocks[bi].as_ref().unwrap();
            let mut seen: Vec<usize> = Vec::with_capacity(16);
            for insn in cb.instructions.iter() {
                let sid = match insn {
                    super::bytecode::Insn::LoadSignal(_, sid)
                    | super::bytecode::Insn::LoadSignalSigned(_, sid)
                    | super::bytecode::Insn::LoadSignalRange(_, sid, _, _)
                    | super::bytecode::Insn::LoadSignalBit(_, sid, _)
                    | super::bytecode::Insn::BranchIfSignalFalse(sid, _, _) => *sid,
                    super::bytecode::Insn::NbaAssign(sid, _, _)
                    | super::bytecode::Insn::NbaAssignConst(sid, _, _)
                    | super::bytecode::Insn::NbaAssignRange(sid, _, _, _) => *sid,
                    // Fused array-read NBA touches TWO signals: the index it
                    // reads and the destination it writes.
                    super::bytecode::Insn::NbaAssignArrayRead(wsid, _, rsid, _) => {
                        for s in [*wsid as usize, *rsid as usize] {
                            if !seen.contains(&s) {
                                seen.push(s);
                            }
                        }
                        continue;
                    }
                    _ => continue,
                };
                let sid = sid as usize;
                if !seen.contains(&sid) {
                    seen.push(sid);
                }
            }
            for sid in seen {
                let vs = sig_to_vids.entry(sid).or_default();
                if !vs.contains(&vid) {
                    vs.push(vid);
                }
            }
        }

        // Step 3: drop singleton-edges (only one block touches the
        // signal — they contribute zero to the cut and just bloat the
        // file). Sort the remaining hyperedges by smallest first vid
        // for deterministic output.
        let mut edges: Vec<(usize, Vec<usize>)> = sig_to_vids
            .into_iter()
            .filter(|(_, vs)| vs.len() >= 2)
            .collect();
        edges.sort_by_key(|(sid, _)| *sid);

        // Step 4: emit hMETIS format.
        // Header: "<num_edges> <num_vertices> <fmt>"
        //   fmt = 11 → both edge weights AND vertex weights present.
        // Each edge line: "<weight> v1 v2 ... vk"
        // Each vertex line (after edges): "<weight>"
        let mut out = std::io::BufWriter::new(
            std::fs::File::create(path).map_err(|e| format!("create {}: {}", path, e))?,
        );
        writeln!(out, "{} {} 11", edges.len(), blocks.len()).map_err(|e| e.to_string())?;
        for (sid, vs) in &edges {
            // Edge weight: phase-4 prefers MEASURED COST over hierarchy.
            // The user spec says "repartition based on measured cost,
            // not hierarchy", so when a profile is present we use:
            //   sensitivity_trigger_count × signal_width
            // (signals that actually woke blocks × bytes per packet).
            // Falls back to phase-2's toggle×width when sensitivity
            // counter wasn't populated, and finally to static width.
            let static_w = if *sid < self.signal_widths.len() {
                self.signal_widths[*sid].max(1)
            } else {
                1
            };
            let w = if let Some(p) = profile {
                let triggers = p.sensitivity_trigger_count.get(sid).copied();
                let toggles = p.signal_toggle_count.get(sid).copied().unwrap_or(0);
                let measured = triggers.unwrap_or(toggles);
                let raw = (measured.saturating_mul(static_w as u64)).max(1);
                raw.min(u32::MAX as u64 - 1) as u32
            } else {
                static_w
            };
            write!(out, "{}", w).map_err(|e| e.to_string())?;
            for v in vs {
                write!(out, " {}", v).map_err(|e| e.to_string())?;
            }
            writeln!(out).map_err(|e| e.to_string())?;
        }
        for &root in &blocks {
            // Vertex weight: phase-4 measured cost = runtime_ns
            // + (NBA_count × fire_count × NBA_FIXED_COST_NS).
            // The static NBA term gives every block a non-zero floor
            // (fixes phase 2's 1868-cold-blocks-bunched problem) while
            // still letting hot blocks dominate.
            const NBA_FIXED_COST_NS: u64 = 50; // empirical: ~50 ns/NBA
            let empty: Vec<usize> = Vec::new();
            let members = members_of.get(&root).unwrap_or(&empty);
            let w = if let Some(p) = profile {
                let ns: u64 = members
                    .iter()
                    .map(|&bi| {
                        let exec = p.edge_block_exec_ns.get(&bi).copied().unwrap_or(0);
                        let nba = p.nba_count_per_block.get(&bi).copied().unwrap_or(0);
                        let fires = p.edge_block_exec_count.get(&bi).copied().unwrap_or(0);
                        let nba_cost = nba.saturating_mul(fires).saturating_mul(NBA_FIXED_COST_NS);
                        exec.saturating_add(nba_cost)
                    })
                    .sum();
                let us = (ns / 1000).max(1);
                us.min(u32::MAX as u64 - 1) as u32
            } else {
                let total: u32 = members
                    .iter()
                    .map(|&bi| {
                        self.compiled_edge_blocks[bi]
                            .as_ref()
                            .map(|cb| cb.instructions.len() as u32)
                            .unwrap_or(0)
                    })
                    .sum();
                total.max(1)
            };
            writeln!(out, "{}", w).map_err(|e| e.to_string())?;
        }
        out.flush().map_err(|e| e.to_string())?;
        drop(out);

        // Step 5: emit a sibling vmap file. With phase-3, each vid
        // maps to MULTIPLE original block_idxs (the island's members).
        // Format: `vid root_block_idx member1,member2,... total_insns`.
        // load_partition_file parses the comma-list and propagates
        // the partition assignment to every member.
        let vmap_path = format!("{}.vmap", path);
        let mut vmap = std::io::BufWriter::new(
            std::fs::File::create(&vmap_path)
                .map_err(|e| format!("create {}: {}", vmap_path, e))?,
        );
        writeln!(vmap, "# vid root_block_idx members(comma) total_insns")
            .map_err(|e| e.to_string())?;
        for (i, &root) in blocks.iter().enumerate() {
            let empty: Vec<usize> = Vec::new();
            let members = members_of.get(&root).unwrap_or(&empty);
            let n_insns: usize = members
                .iter()
                .map(|&bi| {
                    self.compiled_edge_blocks[bi]
                        .as_ref()
                        .map(|cb| cb.instructions.len())
                        .unwrap_or(0)
                })
                .sum();
            let mlist = members
                .iter()
                .map(|&bi| bi.to_string())
                .collect::<Vec<_>>()
                .join(",");
            writeln!(vmap, "{} {} {} {}", i + 1, root, mlist, n_insns)
                .map_err(|e| e.to_string())?;
        }
        vmap.flush().map_err(|e| e.to_string())?;

        Ok((blocks.len(), edges.len()))
    }

    /// Load a partition file in mt-kahypar's standard format: one
    /// line per vertex, each line is the partition id (0..k-1).
    /// The vertex order matches the hypergraph emitted by
    /// `emit_edge_block_hypergraph`. Pairs the partition with the
    /// `.vmap` sibling so we can route back to original block ids.
    ///
    /// On success populates `edge_block_partition` (indexed by
    /// edge_block_idx) and returns `(num_assignments, num_partitions)`.
    pub fn load_partition_file(&mut self, path: &str) -> Result<(usize, u32), String> {
        let part_text =
            std::fs::read_to_string(path).map_err(|e| format!("read {}: {}", path, e))?;
        let parts: Vec<u32> = part_text
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .map(|l| l.trim().parse::<u32>().unwrap_or(0))
            .collect();

        // Try to read the .vmap (preferred — anchors vid → block_idx
        // even if the simulator state has shifted). Fall back to
        // assuming the partition file's i-th line maps to the i-th
        // parallel-eligible block in current order.
        let vmap_path = path
            .trim_end_matches(".part")
            .trim_end_matches(".partition");
        let vmap_path = format!(
            "{}.vmap",
            vmap_path.trim_end_matches(&format!(".part.{}", parts.len()).as_str())
        );
        let vmap = std::fs::read_to_string(&vmap_path).ok();

        // Default-sized to the full edge_blocks Vec so the dispatcher
        // can index by the original block id.
        let n_blocks = self.edge_blocks.len();
        self.edge_block_partition = vec![0u32; n_blocks];
        let mut assigned = 0usize;

        if let Some(vmap) = vmap {
            // vmap formats supported:
            //   Phase 1/2: "vid block_idx insn_count"      — 3 fields
            //   Phase 3:   "vid root member,member,... ic" — 4 fields with members at idx 2
            for line in vmap.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let mut it = line.split_whitespace();
                let vid: usize = match it.next().and_then(|s| s.parse().ok()) {
                    Some(v) => v,
                    None => continue,
                };
                let _root: usize = match it.next().and_then(|s| s.parse().ok()) {
                    Some(v) => v,
                    None => continue,
                };
                if vid == 0 || vid > parts.len() {
                    continue;
                }
                let part = parts[vid - 1];
                // Third token is either insn_count (phase 1/2) or
                // a comma-separated members list (phase 3). Tell them
                // apart by presence of a comma.
                let third = match it.next() {
                    Some(s) => s,
                    None => continue,
                };
                if third.contains(',') {
                    for m in third.split(',') {
                        if let Ok(bi) = m.parse::<usize>() {
                            if bi < n_blocks {
                                self.edge_block_partition[bi] = part;
                                assigned += 1;
                            }
                        }
                    }
                } else {
                    // Phase 1/2: third was insn_count → root_block_idx
                    // is the only member. But we already consumed the
                    // root field; re-route partition assignment there.
                    if _root < n_blocks {
                        self.edge_block_partition[_root] = part;
                        assigned += 1;
                    }
                }
            }
        } else {
            // Fall back: i-th parallel block gets the i-th part id.
            let mut pidx = 0usize;
            for (bi, &is_p) in self.edge_block_parallel.iter().enumerate() {
                if !is_p {
                    continue;
                }
                if pidx < parts.len() {
                    self.edge_block_partition[bi] = parts[pidx];
                    assigned += 1;
                    pidx += 1;
                }
            }
        }
        let k = parts.iter().copied().max().map(|m| m + 1).unwrap_or(0);
        self.edge_block_partition_count = k;
        Ok((assigned, k))
    }

    /// XEZIM_PARTITION_BY_CLOCK=1 auto-bucket parallel-eligible edge blocks
    /// by their primary posedge sensitivity. Each unique clock signal gets
    /// its own partition (modulo XEZIM_PARTITION_BY_CLOCK_MAX_K, default 8).
    /// Blocks within the same clock domain are tightly coupled and run in
    /// the same thread; blocks across domains are mostly independent and
    /// can run concurrently. Skipped if --load-partition was already used.
    pub(super) fn auto_partition_by_clock(&mut self) {
        if std::env::var("XEZIM_PARTITION_BY_CLOCK").as_deref() != Ok("1") {
            return;
        }
        if !self.edge_block_partition.is_empty() && self.edge_block_partition_count > 0 {
            // External partition file was loaded — respect it.
            return;
        }
        let max_k: u32 = std::env::var("XEZIM_PARTITION_BY_CLOCK_MAX_K")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(8);
        let n_blocks = self.edge_blocks.len();
        self.edge_block_partition = vec![0u32; n_blocks];
        let mut clock_to_part: HashMap<usize, u32> = HashMap::default();
        let mut next_part: u32 = 0;
        let mut max_used: u32 = 0;
        for (bi, block) in self.edge_blocks.iter().enumerate() {
            // Primary clock = first Posedge sensitivity. Fall back to first
            // any-edge / negedge if no posedge.
            let primary = block
                .resolved_sensitivities
                .iter()
                .find(|s| s.edge == EdgeKind::Posedge)
                .or_else(|| block.resolved_sensitivities.first())
                .map(|s| s.signal_id);
            let part = if let Some(clk) = primary {
                *clock_to_part.entry(clk).or_insert_with(|| {
                    let p = next_part;
                    next_part = (next_part + 1) % max_k;
                    p
                })
            } else {
                0
            };
            self.edge_block_partition[bi] = part;
            if part > max_used {
                max_used = part;
            }
        }
        self.edge_block_partition_count = max_used + 1;
        eprintln!(
            "[OPT] auto-partition-by-clock: {} blocks, {} unique clock domains, k={}",
            n_blocks,
            clock_to_part.len(),
            self.edge_block_partition_count
        );
    }

    /// Manual scope-based edge-block partition, selected by
    /// `XEZIM_PARTITION_SCOPES="<scope0>;<scope1>;..."`. Each edge block whose
    /// hierarchical scope equals or sits under prefix `i` is assigned partition
    /// `i`; blocks under none land in a final "rest" partition. This aligns the
    /// parallel edge dispatch with the design's structure (e.g. c910:
    /// core0 | core1 | rest) so each worker thread touches a disjoint region of
    /// the signal table — better cache locality than the generic block chunk.
    pub(super) fn auto_partition_by_scope(&mut self) {
        let spec = match std::env::var("XEZIM_PARTITION_SCOPES") {
            Ok(s) if !s.is_empty() => s,
            _ => return,
        };
        if !self.edge_block_partition.is_empty() && self.edge_block_partition_count > 0 {
            return; // already partitioned (clock auto-part or external file)
        }
        let prefixes: Vec<(String, String)> = spec
            .split(';')
            .filter(|s| !s.is_empty())
            .map(|s| (s.to_string(), format!("{}.", s)))
            .collect();
        if prefixes.is_empty() {
            return;
        }
        let rest_part = prefixes.len() as u32;
        let n_blocks = self.edge_blocks.len();
        self.edge_block_partition = vec![rest_part; n_blocks];
        let mut counts = vec![0usize; prefixes.len() + 1];
        for bi in 0..n_blocks {
            let part = match self.edge_block_scope.get(bi).and_then(|s| s.as_ref()) {
                Some(sc) => prefixes
                    .iter()
                    .position(|(p, pdot)| {
                        sc.as_str() == p.as_str() || sc.starts_with(pdot.as_str())
                    })
                    .map(|i| i as u32)
                    .unwrap_or(rest_part),
                None => rest_part,
            };
            self.edge_block_partition[bi] = part;
            counts[part as usize] += 1;
        }
        self.edge_block_partition_count = prefixes.len() as u32 + 1;
        eprintln!(
            "[OPT] auto-partition-by-scope: k={} buckets, block counts {:?}",
            self.edge_block_partition_count, counts
        );
    }

    /// Feasibility probe for levelized / bulk-synchronous parallel settle
    /// (XEZIM_SETTLE_LEVELS=1). Assigns each comb entry a static level =
    /// longest producer chain (entries in stored topo order; cyclic back-edges
    /// ignored), then reports the per-level width distribution. WIDE levels =
    /// many mutually-independent entries that could evaluate in parallel behind
    /// a barrier; NARROW levels (long dependency chains) mean little exploitable
    /// parallelism. Static (all entries) — an upper bound on the dynamic
    /// per-pass parallelism (settle only fires triggered entries).
    pub(super) fn report_comb_levels(&mut self) {
        let n = self.comb_entries.len();
        if n == 0 {
            return;
        }
        // signal_id -> highest producer level seen so far (topo order).
        let mut sig_prod_level: HashMap<usize, u32> = HashMap::default();
        let mut unresolved = 0usize;
        self.comb_level = vec![0u32; n];
        self.comb_has_nba = vec![false; n];
        self.comb_par_safe = vec![false; n];
        // Writer-count pre-pass: a signal written by >1 comb entry is
        // multi-driven; entries touching such a signal CANNOT run in the
        // parallel level dispatch (two threads writing the same Value would
        // race). Only single-writer entries are par_safe → disjoint writes
        // within any level are then guaranteed.
        let mut comb_writer_count: HashMap<usize, u32> = HashMap::default();
        for e in &self.comb_entries {
            for &w in &e.cold.write_signal_ids {
                *comb_writer_count.entry(w).or_insert(0) += 1;
            }
        }
        for i in 0..n {
            let e = &self.comb_entries[i];
            let single_writer = e
                .cold
                .write_signal_ids
                .iter()
                .all(|w| comb_writer_count.get(w).copied().unwrap_or(0) <= 1);
            use super::bytecode::Insn;
            match &e.item {
                // Simple isolated-safe items (no &mut self, no NBA).
                CombItem::Noop
                | CombItem::FastDirectCopy { .. }
                | CombItem::FastDirectFanout { .. }
                | CombItem::DirectCopy { .. }
                | CombItem::FusedGate { .. }
                | CombItem::FusedBufFanout { .. }
                | CombItem::FusedAndFanout { .. } => {
                    self.comb_par_safe[i] = single_writer;
                }
                CombItem::CompiledContAssign { compiled, .. }
                | CombItem::CompiledAlwaysBlock { compiled, .. } => {
                    let mut has_nba = false;
                    let mut unsupported = false;
                    for ins in &compiled.instructions {
                        match ins {
                            Insn::NbaAssign(..)
                            | Insn::NbaAssignConst(..)
                            | Insn::NbaAssignRange(..)
                            | Insn::NbaAssignRangeDyn(..)
                            | Insn::NbaAssignBitDyn(..)
                            | Insn::NbaAssignArray(..)
                            | Insn::NbaAssignArrayRange(..)
                            | Insn::NbaAssignArrayRead(..) => has_nba = true,
                            // Insns the isolated evaluator can't handle (see the
                            // catch-all in exec_comb_block_isolated): array /
                            // dynamic-range blocking writes and AST fallback.
                            Insn::StmtFallback(..)
                            | Insn::CovHit(..)
                            | Insn::EvalExprFallback(..)
                            | Insn::BlockingAssignArray(..)
                            | Insn::BlockingAssignArrayRange(..)
                            | Insn::BlockingAssignRangeDyn(..) => unsupported = true,
                            // A packed-arena memory has no slot in the
                            // isolated evaluator's signal view.
                            Insn::LoadArrayElem(_, a, _) if self.array_operand_is_packed(a) => {
                                unsupported = true
                            }
                            _ => {}
                        }
                    }
                    self.comb_has_nba[i] = has_nba;
                    // par_safe only if the isolated path fully handles it, it
                    // has no deferred NBA, and all its writes are single-driver
                    // (disjoint-write safety for parallel level eval).
                    self.comb_par_safe[i] = !has_nba && !unsupported && single_writer;
                }
                // AST entries always need &mut self.
                CombItem::ContAssign { .. } | CombItem::AlwaysBlock { .. } => {}
                // §29 UDPs mutate per-instance state — never parallel-safe.
                // GateRegion: serial-only in v1 (no isolated batch arm yet).
                CombItem::Udp { .. }
                | CombItem::UdpBatch { .. }
                | CombItem::TimingCheck { .. }
                | CombItem::GateRegion { .. }
                | CombItem::VectorGate { .. }
                | CombItem::ScatterGate { .. }
                | CombItem::CtlGate { .. } => {}
            }
            if e.has_unresolved_reads {
                unresolved += 1;
            }
            let mut lvl = 0u32;
            for &rs in &e.cold.read_signal_ids {
                if let Some(&pl) = sig_prod_level.get(&rs) {
                    if pl + 1 > lvl {
                        lvl = pl + 1;
                    }
                }
            }
            for &ws in &e.cold.write_signal_ids {
                let cur = sig_prod_level.entry(ws).or_insert(0);
                if lvl > *cur {
                    *cur = lvl;
                }
            }
            self.comb_level[i] = lvl;
        }
        if std::env::var_os("XEZIM_SETTLE_LEVELS").is_none() {
            return; // levels computed/stored; skip the diagnostic print
        }
        // Diagnostic: comb entries whose compiled body contains NBA insns.
        // The isolated settle eval drops these (deferred) → BSP divergence
        // candidate. If c910 has them and c906/static don't, that's the bug.
        {
            use super::bytecode::Insn;
            let mut nba_blocks = 0usize;
            for e in &self.comb_entries {
                let cb = match &e.item {
                    CombItem::CompiledContAssign { compiled, .. }
                    | CombItem::CompiledAlwaysBlock { compiled, .. } => compiled,
                    _ => continue,
                };
                if cb.instructions.iter().any(|i| {
                    matches!(
                        i,
                        Insn::NbaAssign(..)
                            | Insn::NbaAssignConst(..)
                            | Insn::NbaAssignRange(..)
                            | Insn::NbaAssignRangeDyn(..)
                            | Insn::NbaAssignBitDyn(..)
                            | Insn::NbaAssignArray(..)
                            | Insn::NbaAssignArrayRange(..)
                            | Insn::NbaAssignArrayRead(..)
                    )
                }) {
                    nba_blocks += 1;
                }
            }
            eprintln!(
                "[SETTLE-LEVELS] comb entries with NBA insns (dropped by isolated eval): {}",
                nba_blocks
            );
        }
        let level = &self.comb_level;
        let max_level = level.iter().copied().max().unwrap_or(0);
        let num_levels = max_level as usize + 1;
        let mut width = vec![0usize; num_levels];
        for &l in level.iter() {
            width[l as usize] += 1;
        }
        let max_width = width.iter().copied().max().unwrap_or(0);
        let avg_width = n as f64 / num_levels as f64;
        // Parallelizable mass: entries in levels wide enough to amortize a
        // per-level barrier.
        let mass_ge = |t: usize| -> usize { width.iter().filter(|&&w| w >= t).sum() };
        eprintln!(
            "[SETTLE-LEVELS] {} entries, {} levels, max_width={}, avg_width={:.1}, unresolved(always-fire)={} ({:.0}%)",
            n,
            num_levels,
            max_width,
            avg_width,
            unresolved,
            100.0 * unresolved as f64 / n as f64
        );
        eprintln!(
            "[SETTLE-LEVELS] entries in levels of width >=16: {} ({:.0}%), >=64: {} ({:.0}%), >=256: {} ({:.0}%)",
            mass_ge(16),
            100.0 * mass_ge(16) as f64 / n as f64,
            mass_ge(64),
            100.0 * mass_ge(64) as f64 / n as f64,
            mass_ge(256),
            100.0 * mass_ge(256) as f64 / n as f64,
        );
    }

    /// Scope an edge block's fallback statements run under: the block's own
    /// inlining scope, else the instance prefix of its first sensitivity.
    /// Shared by the `edge_block_scope` table and the native compile of the
    /// block, so `%m`, bare names and `%t` agree between the two paths.
    pub(super) fn edge_block_scope_for(&self, idx: usize) -> Option<String> {
        self.edge_blocks.get(idx).and_then(|b| {
            if !b.scope.is_empty() {
                Some(b.scope.clone())
            } else {
                b.resolved_sensitivities
                    .first()
                    .and_then(|sid| self.name_opt(sid.signal_id))
                    .map(|full| {
                        full.rsplit_once('.')
                            .map(|(p, _)| p.to_string())
                            .unwrap_or_else(|| self.module.name.clone())
                    })
            }
        })
    }

    pub(super) fn dump_cycle_census(&self) {
        let mode = match std::env::var("XEZIM_CYCLE_CENSUS") {
            Ok(v) => v,
            Err(_) => return,
        };
        use super::bytecode::Insn;
        let n = self.comb_entries.len();
        type Rng = (usize, u32, u32); // (signal, lo, hi) inclusive; whole = (0, u32::MAX)
        let whole = |sig: usize| -> Rng { (sig, 0, u32::MAX) };
        let bref = |r: &BitRef| -> Rng { (r.sig_id as usize, r.bit, r.bit) };
        // per entry: (reads, writes) with bit ranges where the item knows them
        let mut reads: Vec<Vec<Rng>> = Vec::with_capacity(n);
        let mut writes: Vec<Vec<Rng>> = Vec::with_capacity(n);
        let mut ranged_entries = 0usize;
        for ent in &self.comb_entries {
            let mut r: Vec<Rng> = Vec::new();
            let mut w: Vec<Rng> = Vec::new();
            let mut ranged = false;
            match &ent.item {
                CombItem::FusedGate { op } => {
                    ranged = true;
                    match op {
                        FusedGate::Buf1 { dst, src, .. } => {
                            w.push(bref(dst));
                            r.push(bref(src));
                        }
                        FusedGate::Bin2 { dst, a, b, .. } => {
                            w.push(bref(dst));
                            r.push(bref(a));
                            r.push(bref(b));
                        }
                        FusedGate::Mux2 { dst, s, t, e, .. } => {
                            w.push(bref(dst));
                            r.push(bref(s));
                            r.push(bref(t));
                            r.push(bref(e));
                        }
                        FusedGate::UdpLut3 {
                            dst,
                            inputs,
                            input_count,
                            ..
                        } => {
                            w.push(bref(dst));
                            for i in 0..(*input_count as usize).min(3) {
                                r.push(bref(&inputs[i]));
                            }
                        }
                        #[allow(unreachable_patterns)]
                        _ => {
                            ranged = false;
                        }
                    }
                }
                CombItem::Udp { idx } => {
                    ranged = true;
                    let u = &self.udp_runtime[*idx];
                    w.push(bref(&u.out_ref));
                    for i in &u.in_refs {
                        r.push(bref(i));
                    }
                }
                CombItem::CompiledContAssign { compiled }
                | CombItem::CompiledAlwaysBlock { compiled, .. } => {
                    ranged = true;
                    // whole-signal loads/stores of a signal override its ranged forms
                    for insn in &compiled.instructions {
                        match insn {
                            Insn::LoadSignal(_, s)
                            | Insn::LoadSignalSigned(_, s)
                            | Insn::BranchIfSignalFalse(s, _, _) => r.push(whole(*s as usize)),
                            Insn::LoadSignalRange(_, s, l, rr) => {
                                r.push((*s as usize, (*l).min(*rr), (*l).max(*rr)))
                            }
                            Insn::LoadSignalBit(_, s, bit) => r.push((*s as usize, *bit, *bit)),
                            Insn::NbaAssignArrayRead(dst, _, src, _) => {
                                w.push(whole(*dst as usize));
                                r.push(whole(*src as usize));
                            }
                            Insn::BlockingAssign(s, _, _)
                            | Insn::NbaAssign(s, _, _)
                            | Insn::NbaAssignConst(s, _, _)
                            | Insn::BlockingAssignString(s, _)
                            | Insn::NbaAssignRangeDyn(s, _, _, _)
                            | Insn::NbaAssignBitDyn(s, _, _)
                            | Insn::BlockingAssignRangeDyn(s, _, _, _)
                            | Insn::BlockingAssignBitDyn(s, _, _) => w.push(whole(*s as usize)),
                            Insn::BlockingAssignRange(s, hi, lo, _)
                            | Insn::NbaAssignRange(s, hi, lo, _) => {
                                w.push((*s as usize, (*lo).min(*hi), (*lo).max(*hi)))
                            }
                            _ => {}
                        }
                    }
                    // any signal in the cold sets that the scan did not see at all: keep whole
                    for &sid in &ent.cold.read_signal_ids {
                        if !r.iter().any(|x| x.0 == sid) {
                            r.push(whole(sid));
                        }
                    }
                    for &sid in &ent.cold.write_signal_ids {
                        if !w.iter().any(|x| x.0 == sid) {
                            w.push(whole(sid));
                        }
                    }
                }
                _ => {}
            }
            if !ranged {
                r = ent.cold.read_signal_ids.iter().map(|&s| whole(s)).collect();
                w = ent
                    .cold
                    .write_signal_ids
                    .iter()
                    .map(|&s| whole(s))
                    .collect();
            } else {
                ranged_entries += 1;
            }
            reads.push(r);
            writes.push(w);
        }
        // readers index: signal -> entries reading it (from the settle CSR)
        let dependents = |sig: usize| -> &[u32] {
            if sig + 1 < self.comb_dep_offsets.len() {
                &self.comb_dep_entries
                    [self.comb_dep_offsets[sig] as usize..self.comb_dep_offsets[sig + 1] as usize]
            } else {
                &[]
            }
        };
        let overlaps = |a: &Rng, b: &Rng| -> bool { a.0 == b.0 && a.1 <= b.2 && b.1 <= a.2 };
        let succ = |e: usize| -> Vec<usize> {
            let mut out: Vec<usize> = Vec::new();
            for wr in &writes[e] {
                for &d in dependents(wr.0) {
                    let d = d as usize;
                    if reads[d].iter().any(|rd| overlaps(wr, rd)) {
                        out.push(d);
                    }
                }
            }
            out.sort_unstable();
            out.dedup();
            out
        };
        // iterative Tarjan
        let mut index = vec![u32::MAX; n];
        let mut low = vec![0u32; n];
        let mut on_stack = vec![false; n];
        let mut stack: Vec<usize> = Vec::new();
        let mut next_index = 0u32;
        let mut scc_sizes: Vec<usize> = Vec::new();
        let mut in_scc = vec![false; n];
        let mut self_loop = vec![false; n];
        for root in 0..n {
            if index[root] != u32::MAX {
                continue;
            }
            let mut work: Vec<(usize, Vec<usize>, usize)> = vec![(root, succ(root), 0)];
            index[root] = next_index;
            low[root] = next_index;
            next_index += 1;
            stack.push(root);
            on_stack[root] = true;
            while let Some((v, succs, pos)) = work.last_mut() {
                let v = *v;
                if *pos < succs.len() {
                    let w = succs[*pos];
                    *pos += 1;
                    if w == v {
                        self_loop[v] = true;
                        continue;
                    }
                    if index[w] == u32::MAX {
                        index[w] = next_index;
                        low[w] = next_index;
                        next_index += 1;
                        stack.push(w);
                        on_stack[w] = true;
                        let sw = succ(w);
                        work.push((w, sw, 0));
                    } else if on_stack[w] {
                        low[v] = low[v].min(index[w]);
                    }
                } else {
                    work.pop();
                    if let Some(&(u, _, _)) = work.last() {
                        low[u] = low[u].min(low[v]);
                    }
                    if low[v] == index[v] {
                        let mut members: Vec<usize> = Vec::new();
                        loop {
                            let w = stack.pop().unwrap();
                            on_stack[w] = false;
                            members.push(w);
                            if w == v {
                                break;
                            }
                        }
                        if members.len() > 1 {
                            scc_sizes.push(members.len());
                            if members.len() >= 32 && mode == "2" {
                                let mut wsigs: HashMap<usize, usize> = HashMap::default();
                                for &m in &members {
                                    for wr in &writes[m] {
                                        *wsigs.entry(wr.0).or_insert(0) += 1;
                                    }
                                }
                                let mut ws: Vec<(usize, usize)> = wsigs.into_iter().collect();
                                ws.sort_by(|a, b| b.1.cmp(&a.1));
                                let names: Vec<String> = ws
                                    .iter()
                                    .take(3)
                                    .map(|(id, c)| format!("{}x{}", c, self.name_for_id(*id)))
                                    .collect();
                                eprintln!(
                                    "[CYCLE]   SCC size {}: distinct written signals {}; top {:?}",
                                    members.len(),
                                    ws.len(),
                                    names
                                );
                                if members.len() == 64 {
                                    let m = members[0];
                                    let kind = match &self.comb_entries[m].item {
                                        CombItem::ContAssign { .. } => "ContAssign",
                                        CombItem::CompiledContAssign { .. } => "CompiledContAssign",
                                        CombItem::AlwaysBlock { .. } => "AlwaysBlock",
                                        CombItem::CompiledAlwaysBlock { .. } => {
                                            "CompiledAlwaysBlock"
                                        }
                                        CombItem::FusedGate { .. } => "FusedGate",
                                        CombItem::VectorGate { .. } => "VectorGate",
                                        CombItem::ScatterGate { .. } => "ScatterGate",
                                        CombItem::Udp { .. } => "Udp",
                                        _ => "other",
                                    };
                                    let insns: Vec<String> = match &self.comb_entries[m].item {
                                        CombItem::CompiledContAssign { compiled }
                                        | CombItem::CompiledAlwaysBlock { compiled, .. } => {
                                            compiled
                                                .instructions
                                                .iter()
                                                .take(40)
                                                .map(|i| format!("{:?}", i))
                                                .collect()
                                        }
                                        _ => Vec::new(),
                                    };
                                    let rn: Vec<String> = reads[m]
                                        .iter()
                                        .map(|r| {
                                            format!("{}[{}:{}]", self.name_for_id(r.0), r.2, r.1)
                                        })
                                        .collect();
                                    let wn: Vec<String> = writes[m]
                                        .iter()
                                        .map(|r| {
                                            format!("{}[{}:{}]", self.name_for_id(r.0), r.2, r.1)
                                        })
                                        .collect();
                                    eprintln!(
                                        "[CYCLE]     member kind={} reads={:?} writes={:?}",
                                        kind, rn, wn
                                    );
                                    for i in insns {
                                        eprintln!("[CYCLE]       {}", &i[..i.len().min(110)]);
                                    }
                                }
                            }
                            for m in members {
                                in_scc[m] = true;
                            }
                        }
                    }
                }
            }
        }
        let latches = self_loop.iter().filter(|&&b| b).count();
        let in_scc_n = in_scc.iter().filter(|&&b| b).count();
        let excl = (0..n).filter(|&e| self_loop[e] || in_scc[e]).count();
        scc_sizes.sort_unstable_by(|a, b| b.cmp(a));
        eprintln!(
            "[CYCLE] comb entries={} ranged-tracked={} ({:.1}%) non-trivial SCCs={} entries-in-SCC={} ({:.2}%) largest={:?}",
            n,
            ranged_entries,
            100.0 * ranged_entries as f64 / n.max(1) as f64,
            scc_sizes.len(),
            in_scc_n,
            100.0 * in_scc_n as f64 / n.max(1) as f64,
            &scc_sizes[..scc_sizes.len().min(8)]
        );
        eprintln!(
            "[CYCLE] self-looping entries={} ; single-pass exclusion set (SCC ∪ self-loop) = {} entries ({:.2}%)",
            latches,
            excl,
            100.0 * excl as f64 / n.max(1) as f64
        );
        let mut written = vec![false; self.signal_table.len()];
        for ent in &self.comb_entries {
            for &w in &ent.cold.write_signal_ids {
                if w < written.len() {
                    written[w] = true;
                }
            }
        }
        let derived = self
            .edge_blocks
            .iter()
            .filter(|eb| {
                eb.resolved_sensitivities
                    .first()
                    .is_some_and(|s| written.get(s.signal_id).copied().unwrap_or(false))
            })
            .count();
        eprintln!(
            "[CYCLE] edge blocks={} clocked by a comb-written (derived/gated) signal={} ({:.1}%)",
            self.edge_blocks.len(),
            derived,
            100.0 * derived as f64 / self.edge_blocks.len().max(1) as f64
        );
    }

    pub(super) fn dump_island_census(&self) {
        if std::env::var("XEZIM_ISLAND_CENSUS").is_err() {
            return;
        }
        use std::collections::HashMap;
        // 1. Group edge blocks by their first edge-sensitive signal (the
        // clock). Multi-sensitivity blocks (async reset) count under their
        // clock terminal.
        let mut domains: HashMap<u32, Vec<usize>> = HashMap::new();
        for (i, eb) in self.edge_blocks.iter().enumerate() {
            if let Some(s) = eb.resolved_sensitivities.first() {
                domains.entry(s.signal_id as u32).or_default().push(i);
            }
        }
        // Gated-clock conversion, census form: resolve every derived clock
        // back to its ROOT by walking single-writer comb entries (buffers,
        // ICG cells, clock muxes). The parent of a derived clock is the read
        // that is itself a clock (used as an edge sensitivity or another
        // step on the path); the rest of the reads are ENABLES — data, in
        // the island's terms. This is what the reference's optimizer does
        // implicitly: its merged process runs on the root clock. First
        // census on c906 showed why this phase is REQUIRED: 760 raw
        // domains, largest only 180 blocks — the ICG fabric fragments
        // everything until clocks are rooted.
        let is_clock: std::collections::HashSet<u32> = domains.keys().copied().collect();
        let mut writer_of: HashMap<u32, usize> = HashMap::new();
        for (e, ent) in self.comb_entries.iter().enumerate() {
            for &w in &ent.cold.write_signal_ids {
                writer_of.entry(w as u32).or_insert(e);
            }
        }
        let resolve_root = |mut c: u32| -> u32 {
            for _ in 0..64 {
                let Some(&e) = writer_of.get(&c) else {
                    return c;
                };
                let ent = &self.comb_entries[e];
                // Parent = a read that is itself a clock; otherwise the sole
                // read (pure buffer/inverter); otherwise stop (c is a root
                // that happens to be comb-driven).
                let mut parent = None;
                for &r in &ent.cold.read_signal_ids {
                    if is_clock.contains(&(r as u32)) && r as u32 != c {
                        parent = Some(r as u32);
                        break;
                    }
                }
                if parent.is_none() && ent.cold.read_signal_ids.len() == 1 {
                    parent = Some(ent.cold.read_signal_ids[0] as u32);
                }
                match parent {
                    Some(p) if p != c => c = p,
                    _ => return c,
                }
            }
            c
        };
        let mut rooted: HashMap<u32, usize> = HashMap::new();
        for (&clk, blocks) in &domains {
            *rooted.entry(resolve_root(clk)).or_default() += blocks.len();
        }
        let mut rdom: Vec<(u32, usize)> = rooted.into_iter().collect();
        rdom.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
        eprintln!(
            "[ISLAND] rooted domains: {} (from {} raw); top:",
            rdom.len(),
            domains.len()
        );
        for &(sig, n) in rdom.iter().take(5) {
            eprintln!(
                "[ISLAND]   root {} ({}): {} edge blocks",
                sig,
                self.name_opt(sig as usize).unwrap_or("?"),
                n
            );
        }
        let mut dom: Vec<(u32, usize)> = domains.iter().map(|(k, v)| (*k, v.len())).collect();
        dom.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
        eprintln!("[ISLAND] {} raw clock domain(s); top:", dom.len());
        for &(sig, n) in dom.iter().take(4) {
            eprintln!(
                "[ISLAND]   clock {} ({}): {} edge blocks",
                sig,
                self.name_opt(sig as usize).unwrap_or("?"),
                n
            );
        }
        // Cone analysis at ROOTED scope: members = every edge block whose
        // raw clock resolves to the dominant root. (The raw-domain cone was
        // 1.9% on c906; the rooted domain covers 94% of edge blocks.)
        let Some(&(top_root, _)) = rdom.first() else {
            return;
        };
        let members: Vec<usize> = domains
            .iter()
            .filter(|(clk, _)| resolve_root(**clk) == top_root)
            .flat_map(|(_, v)| v.iter().copied())
            .collect();
        let members = &members;
        // 2. Domain flop outputs: write targets of the domain's COMPILED
        // edge blocks (scan insns; uncompiled blocks counted separately).
        let num_signals = self.signal_table.len();
        let mut flop_out: Vec<bool> = vec![false; num_signals];
        let mut uncompiled = 0usize;
        for &i in members {
            let Some(Some(cb)) = self.compiled_edge_blocks.get(i) else {
                uncompiled += 1;
                continue;
            };
            use super::bytecode::Insn as I;
            for insn in cb.instructions.iter() {
                match insn {
                    I::BlockingAssign(s, ..)
                    | I::NbaAssign(s, ..)
                    | I::NbaAssignConst(s, ..)
                    | I::BlockingAssignRange(s, ..)
                    | I::NbaAssignRange(s, ..)
                    | I::BlockingAssignRangeDyn(s, ..)
                    | I::NbaAssignRangeDyn(s, ..)
                    | I::BlockingAssignBitDyn(s, ..)
                    | I::NbaAssignBitDyn(s, ..) => {
                        if (*s as usize) < num_signals {
                            flop_out[*s as usize] = true;
                        }
                    }
                    _ => {}
                }
            }
        }
        // 3. Comb cone: closure of comb entries triggered (transitively) by
        // domain flop outputs, via the dependency CSR.
        let mut in_cone: Vec<bool> = vec![false; self.comb_entries.len()];
        let mut frontier: Vec<usize> = Vec::new();
        let dep_off = &self.comb_dep_offsets;
        let dep_ent = &self.comb_dep_entries;
        let push_readers = |sig: usize, frontier: &mut Vec<usize>, in_cone: &mut Vec<bool>| {
            if sig + 1 < dep_off.len() {
                for &e in &dep_ent[dep_off[sig] as usize..dep_off[sig + 1] as usize] {
                    let e = e as usize;
                    if !in_cone[e] {
                        in_cone[e] = true;
                        frontier.push(e);
                    }
                }
            }
        };
        for sig in 0..num_signals {
            if flop_out[sig] {
                push_readers(sig, &mut frontier, &mut in_cone);
            }
        }
        let mut cone_writes: Vec<bool> = vec![false; num_signals];
        while let Some(e) = frontier.pop() {
            for &w in &self.comb_entries[e].cold.write_signal_ids {
                if w < num_signals && !cone_writes[w] {
                    cone_writes[w] = true;
                    push_readers(w, &mut frontier, &mut in_cone);
                }
            }
        }
        let cone_n = in_cone.iter().filter(|&&b| b).count();
        // 4. Boundary inputs: read by cone entries, not written by cone or
        // domain flops. Internal: written inside, and named vs unnamed
        // (named = observed under a dump).
        let mut reads_outside = 0usize;
        let mut internal_named = 0usize;
        let mut internal_unnamed = 0usize;
        let mut width_excluded = 0usize;
        let mut seen_read: Vec<bool> = vec![false; num_signals];
        for (e, &inc) in in_cone.iter().enumerate() {
            if !inc {
                continue;
            }
            let ent = &self.comb_entries[e];
            let mut wide = false;
            for &r in &ent.cold.read_signal_ids {
                if r < num_signals {
                    if self.signal_widths[r] > 64 {
                        wide = true;
                    }
                    if !seen_read[r] {
                        seen_read[r] = true;
                        if !cone_writes[r] && !flop_out[r] {
                            reads_outside += 1;
                        }
                    }
                }
            }
            for &w in &ent.cold.write_signal_ids {
                if w < num_signals && self.signal_widths[w] > 64 {
                    wide = true;
                }
            }
            if wide {
                width_excluded += 1;
            }
        }
        for sig in 0..num_signals {
            if cone_writes[sig] {
                if self.name_opt(sig).map(|n| !n.is_empty()).unwrap_or(false) {
                    internal_named += 1;
                } else {
                    internal_unnamed += 1;
                }
            }
        }
        eprintln!(
            "[ISLAND] top domain: {} edge blocks ({} uncompiled/unanalyzed), {} flop-out signals",
            members.len(),
            uncompiled,
            flop_out.iter().filter(|&&b| b).count()
        );
        eprintln!(
            "[ISLAND] comb cone: {} of {} entries ({:.1}%); {} width-excluded (>64-bit) ({:.1}%)",
            cone_n,
            self.comb_entries.len(),
            100.0 * cone_n as f64 / self.comb_entries.len().max(1) as f64,
            width_excluded,
            100.0 * width_excluded as f64 / cone_n.max(1) as f64
        );
        eprintln!(
            "[ISLAND] boundary inputs: {}; cone-internal signals: {} named (dump-observed) + {} unnamed",
            reads_outside, internal_named, internal_unnamed
        );
    }

    /// Phase-2 census (`XEZIM_CHAIN_CENSUS=1`): size the single-fanout
    /// chain opportunity. A chain link is a comb entry ALL of whose written
    /// signals are read by exactly one entry (the next link) — the whole
    /// chain co-activates by construction, so fusing it into one compiled
    /// body has zero recompute waste at ANY activity factor (the defect
    /// that killed region fusion, twice). Weighted by dynamic eval counts
    /// when `XEZIM_COMB_PATHS`/profiling counters are populated.
    /// Co-activation census report: per entry, the BEST predecessor edge's
    /// same-slot ratio (co-evals / entry evals), weighted by dynamic evals.
    /// The ">=0.9 coverage" number is the fraction of all evaluation work
    /// that convex clustering can absorb with <=10% waste — the go/no-go
    /// for fanout-inclusive supernode fusion.
    pub(super) fn dump_coact_census(&self) {
        if !self.coact_enabled || self.prof_entry_counts.is_empty() {
            return;
        }
        let n = self.comb_entries.len();
        let mut buckets = [0u64; 6]; // <0.5, 0.5-0.7, 0.7-0.9, 0.9-0.99, >=0.99, no-pred
        let mut bucket_evals = [0u64; 6];
        let tot: u64 = self.prof_entry_counts.iter().sum();
        for e in 0..n {
            let evals = self.prof_entry_counts.get(e).copied().unwrap_or(0);
            if evals == 0 {
                continue;
            }
            let lo = self.coact_pred_off[e] as usize;
            let hi = self.coact_pred_off[e + 1] as usize;
            let best = self.coact_edge_count[lo..hi]
                .iter()
                .max()
                .copied()
                .unwrap_or(0);
            let b = if hi == lo {
                5
            } else {
                let r = best as f64 / evals as f64;
                if r >= 0.99 {
                    4
                } else if r >= 0.9 {
                    3
                } else if r >= 0.7 {
                    2
                } else if r >= 0.5 {
                    1
                } else {
                    0
                }
            };
            buckets[b] += 1;
            bucket_evals[b] += evals;
        }
        eprintln!(
            "[COACT] best-pred same-slot ratio, entries: <0.5:{} 0.5-0.7:{} 0.7-0.9:{} 0.9-0.99:{} >=0.99:{} no-pred:{}",
            buckets[0], buckets[1], buckets[2], buckets[3], buckets[4], buckets[5]
        );
        let pct = |x: u64| 100.0 * x as f64 / tot.max(1) as f64;
        eprintln!(
            "[COACT] dynamic-eval share: <0.5:{:.1}% 0.5-0.7:{:.1}% 0.7-0.9:{:.1}% 0.9-0.99:{:.1}% >=0.99:{:.1}% no-pred:{:.1}%",
            pct(bucket_evals[0]),
            pct(bucket_evals[1]),
            pct(bucket_evals[2]),
            pct(bucket_evals[3]),
            pct(bucket_evals[4]),
            pct(bucket_evals[5])
        );
        eprintln!(
            "[COACT] clusterable (ratio>=0.9): {:.1}% of dynamic evals; with 0.7+: {:.1}%",
            pct(bucket_evals[3] + bucket_evals[4]),
            pct(bucket_evals[2] + bucket_evals[3] + bucket_evals[4])
        );
        // PGO handoff: dump per-edge ratios so a later run can drive region
        // fusion from measured co-activation instead of blind adjacency.
        if let Ok(path) = std::env::var("XEZIM_COACT_FILE") {
            use std::fmt::Write as _;
            let mut out = String::new();
            let _ = writeln!(out, "entries {n}");
            for e in 0..n {
                let evals = self.prof_entry_counts.get(e).copied().unwrap_or(0);
                if evals == 0 {
                    continue;
                }
                let lo = self.coact_pred_off[e] as usize;
                let hi = self.coact_pred_off[e + 1] as usize;
                for k in lo..hi {
                    let r = self.coact_edge_count[k] as f64 / evals as f64;
                    if r >= 0.5 {
                        let _ = writeln!(out, "{e} {} {:.3}", self.coact_pred_ent[k], r);
                    }
                }
            }
            match std::fs::write(&path, out) {
                Ok(()) => eprintln!("[COACT] edge file written: {path}"),
                Err(e) => eprintln!("[COACT] edge file write FAILED ({path}): {e}"),
            }
        }
    }

    pub(super) fn dump_chain_census(&self) {
        if std::env::var("XEZIM_CHAIN_CENSUS").is_err() {
            return;
        }
        let n = self.comb_entries.len();
        let num_signals = self.signal_table.len();
        // reader count per signal from the dep CSR; the unique reader when
        // count == 1.
        let dep_off = &self.comb_dep_offsets;
        let dep_ent = &self.comb_dep_entries;
        let reader_of = |sig: usize| -> Option<usize> {
            if sig + 1 >= dep_off.len() {
                return None;
            }
            let lo = dep_off[sig] as usize;
            let hi = dep_off[sig + 1] as usize;
            if hi - lo == 1 {
                Some(dep_ent[lo] as usize)
            } else {
                None
            }
        };
        // next[e] = the single successor entry when EVERY signal e writes is
        // read by exactly that one entry (and nothing else). pred-count to
        // find chain heads.
        let mut next: Vec<u32> = vec![u32::MAX; n];
        let mut npred: Vec<u32> = vec![0; n];
        for (e, ent) in self.comb_entries.iter().enumerate() {
            if ent.cold.write_signal_ids.is_empty() {
                continue;
            }
            let mut succ: Option<usize> = None;
            let mut ok = true;
            for &w in &ent.cold.write_signal_ids {
                if w >= num_signals {
                    ok = false;
                    break;
                }
                match (reader_of(w), succ) {
                    (Some(r), None) if r != e => succ = Some(r),
                    (Some(r), Some(p)) if r == p => {}
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                if let Some(r) = succ {
                    next[e] = r as u32;
                    npred[r] += 1;
                }
            }
        }
        // Walk chains from heads (a linked entry whose predecessor count via
        // `next` is 0 or ambiguous). Only single-pred nodes extend a chain.
        let mut in_chain: Vec<bool> = vec![false; n];
        let mut lens: Vec<usize> = Vec::new();
        let mut linked = 0usize;
        for e in 0..n {
            if next[e] != u32::MAX {
                linked += 1;
            }
        }
        for e in 0..n {
            if next[e] == u32::MAX || in_chain[e] {
                continue;
            }
            // head: nobody chains INTO e, or the predecessor is shared
            if npred[e] == 1 {
                continue; // interior; will be covered from its head
            }
            let mut len = 0usize;
            let mut cur = e;
            while cur < n && next[cur] != u32::MAX && !in_chain[cur] {
                in_chain[cur] = true;
                len += 1;
                let nx = next[cur] as usize;
                if npred[nx] != 1 {
                    break; // next node has other chain-predecessors
                }
                cur = nx;
            }
            if len >= 2 {
                lens.push(len);
            }
        }
        let covered: usize = lens.iter().sum();
        let mut hist = [0usize; 6]; // 2-3, 4-7, 8-15, 16-31, 32-63, 64+
        for &l in &lens {
            let b = match l {
                0..=3 => 0,
                4..=7 => 1,
                8..=15 => 2,
                16..=31 => 3,
                32..=63 => 4,
                _ => 5,
            };
            hist[b] += 1;
        }
        // Dynamic coverage when eval counters exist.
        let dyn_cov = if self.prof_entry_counts.len() == n {
            let tot: u64 = self.prof_entry_counts.iter().sum();
            let cov: u64 = (0..n)
                .filter(|&e| in_chain[e])
                .map(|e| self.prof_entry_counts[e])
                .sum();
            Some((cov, tot))
        } else {
            None
        };
        eprintln!(
            "[CHAIN] entries={} single-successor-linked={} in-chains(len>=2)={} ({:.1}%) chains={}",
            n,
            linked,
            covered,
            100.0 * covered as f64 / n.max(1) as f64,
            lens.len()
        );
        eprintln!(
            "[CHAIN] length histogram 2-3:{} 4-7:{} 8-15:{} 16-31:{} 32-63:{} 64+:{} max={}",
            hist[0],
            hist[1],
            hist[2],
            hist[3],
            hist[4],
            hist[5],
            lens.iter().max().copied().unwrap_or(0)
        );
        if let Some((cov, tot)) = dyn_cov {
            eprintln!(
                "[CHAIN] dynamic eval coverage: {} of {} ({:.1}%)",
                cov,
                tot,
                100.0 * cov as f64 / tot.max(1) as f64
            );
        }
    }

    pub(super) fn dump_template_census(&self) {
        if std::env::var("XEZIM_TEMPLATE_CENSUS").is_err() {
            return;
        }
        // (hash) -> (block count, total dyn execs, total insns exec'd, sample scope, insn len)
        struct T {
            blocks: u64,
            execs: u64,
            insn_execs: u64,
            len: usize,
            sample: String,
        }
        let mut map: HashMap<u64, T> = HashMap::default();
        let mut total_execs: u64 = 0;
        let mut total_insn_execs: u64 = 0;
        let mut add =
            |map: &mut HashMap<u64, T>, hash: u64, execs: u64, len: usize, sample: &str| {
                total_execs += execs;
                total_insn_execs += execs * len as u64;
                let e = map.entry(hash).or_insert_with(|| T {
                    blocks: 0,
                    execs: 0,
                    insn_execs: 0,
                    len,
                    sample: sample.to_string(),
                });
                e.blocks += 1;
                e.execs += execs;
                e.insn_execs += execs * len as u64;
            };
        for (eidx, entry) in self.comb_entries.iter().enumerate() {
            let cb = match &entry.item {
                CombItem::CompiledContAssign { compiled, .. }
                | CombItem::CompiledAlwaysBlock { compiled, .. } => compiled,
                _ => continue,
            };
            let execs: u64 = self
                .comb_path_counts
                .get(eidx)
                .map(|c| c.iter().sum())
                .unwrap_or(0);
            if execs == 0 {
                continue;
            }
            let scope = entry.cold.scope_hint.as_deref().unwrap_or("?");
            add(
                &mut map,
                self.canon_block_hash(cb),
                execs,
                cb.instructions.len(),
                scope,
            );
        }
        for (bi, cb_opt) in self.compiled_edge_blocks.iter().enumerate() {
            let Some(cb) = cb_opt else { continue };
            let execs = self.edge_block_exec_counts.get(bi).copied().unwrap_or(0);
            if execs == 0 {
                continue;
            }
            add(
                &mut map,
                self.canon_block_hash(cb),
                execs,
                cb.instructions.len(),
                "edge",
            );
        }
        let mut ts: Vec<(&u64, &T)> = map.iter().collect();
        ts.sort_by_key(|(_, t)| std::cmp::Reverse(t.insn_execs));
        let n_blocks: u64 = ts.iter().map(|(_, t)| t.blocks).sum();
        let repeated_execs: u64 = ts
            .iter()
            .filter(|(_, t)| t.blocks > 1)
            .map(|(_, t)| t.execs)
            .sum();
        let repeated_insn_execs: u64 = ts
            .iter()
            .filter(|(_, t)| t.blocks > 1)
            .map(|(_, t)| t.insn_execs)
            .sum();
        let top100_insn: u64 = ts.iter().take(100).map(|(_, t)| t.insn_execs).sum();
        let max_mult = ts.iter().map(|(_, t)| t.blocks).max().unwrap_or(0);
        eprintln!(
            "[TEMPLATE] blocks={} templates={} max_multiplicity={}",
            n_blocks,
            ts.len(),
            max_mult
        );
        eprintln!(
            "[TEMPLATE] dyn evals: total={} in-repeated-templates={} ({:.1}%)",
            total_execs,
            repeated_execs,
            100.0 * repeated_execs as f64 / total_execs.max(1) as f64
        );
        eprintln!(
            "[TEMPLATE] dyn insns: total={} in-repeated-templates={} ({:.1}%) top100-templates={:.1}%",
            total_insn_execs,
            repeated_insn_execs,
            100.0 * repeated_insn_execs as f64 / total_insn_execs.max(1) as f64,
            100.0 * top100_insn as f64 / total_insn_execs.max(1) as f64
        );
        eprintln!("[TEMPLATE] top templates by dyn insns (mult x evals, len, sample):");
        for (hash, t) in ts.iter().take(20) {
            eprintln!(
                "[TEMPLATE]   {:016x} mult={:<4} evals={:<10} len={:<5} insn_execs={:<12} {}",
                hash, t.blocks, t.execs, t.len, t.insn_execs, t.sample
            );
        }
    }

    /// Which bits of each signal comb entry `eidx` reads, as
    /// `(signal, sig_span_mask)` pairs — None keeps every dependency edge of
    /// the entry whole-signal. Only entries whose outputs are a pure
    /// function of the bits they read qualify: fused gates (their operand
    /// bits), and lowered continuous assignments and `always_comb` /
    /// `always_latch` blocks whose stream reads no signal it also writes
    /// (no state carried between evaluations) — a block additionally
    /// storing each signal at most once, so skipping an evaluation cannot
    /// drop an intermediate value. A plain `always` block never qualifies:
    /// its variables may have other writers (an `initial` block, a
    /// testbench task), and a skipped re-evaluation would leave such a
    /// write in place where the block used to overwrite it. A signal the
    /// entry depends on but the stream never reads keeps a whole-signal
    /// edge.
    pub(super) fn comb_entry_read_masks(&self, eidx: usize) -> Option<Vec<(u32, u64)>> {
        let entry = self.comb_entries.get(eidx)?;
        if entry.has_unresolved_reads {
            return None;
        }
        let widths = &self.signal_widths;
        let mut out: Vec<(u32, u64)> = Vec::new();
        let add_bit = |out: &mut Vec<(u32, u64)>, b: BitRef| {
            let w = widths.get(b.sig_id as usize).copied().unwrap_or(0);
            let m = super::bytecode::sig_span_mask(w, b.bit, 1);
            if let Some(e) = out.iter_mut().find(|e| e.0 == b.sig_id) {
                e.1 |= m;
            } else {
                out.push((b.sig_id, m));
            }
        };
        let add_gate = |out: &mut Vec<(u32, u64)>, g: &FusedGate| match g {
            FusedGate::Buf1 { src, .. } => add_bit(out, *src),
            FusedGate::Bin2 { a, b, .. } => {
                add_bit(out, *a);
                add_bit(out, *b);
            }
            FusedGate::Mux2 { s, t, e, .. } => {
                add_bit(out, *s);
                add_bit(out, *t);
                add_bit(out, *e);
            }
            FusedGate::UdpLut3 {
                inputs,
                input_count,
                ..
            } => {
                for b in inputs.iter().take(*input_count as usize) {
                    add_bit(out, *b);
                }
            }
        };
        // A delayed (SDF) gate re-schedules its output on every evaluation;
        // keep such designs on whole-signal edges.
        let gates_ok = self.sdf_delays.is_empty();
        match &entry.item {
            CombItem::FusedGate { op } if gates_ok => add_gate(&mut out, op),
            CombItem::CtlGate { slot } if gates_ok => {
                add_gate(&mut out, &self.ctl_gates.get(*slot as usize)?.op)
            }
            CombItem::GateRegion { gates } if gates_ok => {
                for g in gates.iter() {
                    add_gate(&mut out, g);
                }
            }
            CombItem::CompiledAlwaysBlock {
                is_always_comb: false,
                ..
            } => return None,
            CombItem::CompiledContAssign { .. } | CombItem::CompiledAlwaysBlock { .. } => {
                let TsSlot::Yes(ts) = self.ts_comb.get(eidx)? else {
                    return None;
                };
                let (reads, mut writes) = super::bytecode::ts_read_masks(&ts.insns, widths)?;
                if reads.iter().any(|(s, _)| writes.contains(s)) {
                    return None;
                }
                if matches!(entry.item, CombItem::CompiledAlwaysBlock { .. }) {
                    writes.sort_unstable();
                    if writes.windows(2).any(|w| w[0] == w[1]) {
                        return None;
                    }
                }
                out = reads;
            }
            _ => return None,
        }
        // Every signal the entry reads must also wake it. An
        // `always @(s) y = s[0] ^ x;` reads `x` without being sensitive to
        // it: a change of `s[7:1]` must still re-run it so `y` picks up the
        // current `x`.
        let rids = &entry.cold.read_signal_ids;
        if out.iter().any(|(s, _)| !rids.contains(&(*s as usize))) {
            return None;
        }
        Some(out)
    }

    /// Narrow the dependency edges of entry `eidx` to the bits it reads.
    /// A signal's dependents are stored in ascending entry order.
    pub(super) fn bit_sens_patch(&mut self, eidx: usize) {
        let Some(list) = self.comb_entry_read_masks(eidx) else {
            return;
        };
        let e = eidx as u32;
        for (sig, m) in list {
            if m == u64::MAX {
                continue;
            }
            let s = sig as usize;
            if s + 1 >= self.comb_dep_offsets.len() {
                continue;
            }
            let lo = self.comb_dep_offsets[s] as usize;
            let hi = self.comb_dep_offsets[s + 1] as usize;
            let deps = &self.comb_dep_entries[lo..hi];
            let mut k = lo + deps.partition_point(|&d| d < e);
            while k < hi && self.comb_dep_entries[k] == e {
                self.comb_dep_edges[k].mask = m;
                k += 1;
            }
        }
    }

    /// Bring `comb_dep_edges` up to date with the dependency table and the
    /// entries lowered since the last settle. Runs outside the settle loop
    /// (which moves the tables into locals).
    pub(super) fn bit_sens_refresh(&mut self) {
        if self.comb_dep_edges.len() != self.comb_dep_entries.len() {
            // The edge positions and static masks the gates hold are about
            // to be rebuilt: turn them back into plain gates first.
            self.ctl_revert();
            self.comb_dep_edges = self
                .comb_dep_entries
                .iter()
                .map(|&e| DepEdge {
                    mask: u64::MAX,
                    entry: e,
                })
                .collect();
            self.bit_sens_pending.clear();
            if self.bit_sens_on {
                for eidx in 0..self.comb_entries.len() {
                    self.bit_sens_patch(eidx);
                }
            }
            self.ctl_build();
        } else if !self.bit_sens_pending.is_empty() {
            let pending = std::mem::take(&mut self.bit_sens_pending);
            if self.bit_sens_on {
                for &eidx in pending.iter() {
                    self.bit_sens_patch(eidx as usize);
                }
            }
            self.bit_sens_pending = pending;
            self.bit_sens_pending.clear();
        }
    }

    /// Turn every `CombItem::CtlGate` back into its `FusedGate` and restore
    /// the static masks of the edges it held (when they are still in place).
    pub(super) fn ctl_revert(&mut self) {
        if self.ctl_gates.is_empty() {
            return;
        }
        for e in self.comb_entries.iter_mut() {
            if let CombItem::CtlGate { slot } = e.item {
                e.item = CombItem::FusedGate {
                    op: self.ctl_gates[slot as usize].op,
                };
            }
        }
        if self.comb_dep_edges.len() == self.comb_dep_entries.len() {
            for g in &self.ctl_gates {
                let (lo, hi) = (g.pos as usize, (g.pos + g.npos) as usize);
                if let Some(edges) = self.comb_dep_edges.get_mut(lo..hi) {
                    for e in edges {
                        e.mask = g.smask;
                    }
                }
            }
        }
        self.ctl_gates.clear();
        self.ctl_sig.clear();
        self.ctl_dsts.clear();
        self.ctl_rearm_pending.clear();
        self.gate_lane_valid = false;
    }

    /// Pick the controlling-value gates (`CtlGate`) once the dependency
    /// edges carry their static masks: every two-input AND/OR fused gate
    /// (NAND/NOR included) with two distinct input signals, one of which
    /// fans out to at least `CTL_MIN_FANOUT` comb entries (a clock or other
    /// broadcast net: the input whose toggles are worth skipping).
    ///
    /// Not with SDF delays (a delayed gate re-schedules on every
    /// evaluation) or under the settle modes that evaluate entries outside
    /// the canonical loop (`ctl_on`).
    pub(super) fn ctl_build(&mut self) {
        const CTL_MIN_FANOUT: u32 = 64;
        debug_assert!(self.ctl_gates.is_empty());
        if !self.ctl_on || !self.sdf_delays.is_empty() {
            return;
        }
        let _n = self.comb_entries.len();
        let nsig = self.comb_dep_offsets.len().saturating_sub(1);
        let offs = &self.comb_dep_offsets;
        let fanout = |s: u32| -> u32 {
            let s = s as usize;
            if s < nsig { offs[s + 1] - offs[s] } else { 0 }
        };
        // A hot signal's dirty seed treats a zero mask as "masked", so none
        // of its edges may carry a static zero.
        let mut hot_ok: HashMap<u32, bool> = HashMap::default();
        let mut picked: Vec<(usize, CtlGate)> = Vec::new();
        for (eidx, entry) in self.comb_entries.iter().enumerate() {
            let CombItem::FusedGate { op } = &entry.item else {
                continue;
            };
            let FusedGate::Bin2 {
                dst, a, b, op: gop, ..
            } = *op
            else {
                continue;
            };
            let ctl = match gop {
                GateBin::And => 0u8,
                GateBin::Or => 1u8,
                GateBin::Xor => continue,
            };
            if a.sig_id == b.sig_id || dst.sig_id == a.sig_id || dst.sig_id == b.sig_id {
                continue;
            }
            let (fa, fb) = (fanout(a.sig_id), fanout(b.sig_id));
            if fa.max(fb) < CTL_MIN_FANOUT {
                continue;
            }
            // Only a declared net: nothing but its drivers, a force or a
            // deposit (all re-arming, see `ctl_note_external_write`) can
            // write it. A variable could also take a procedural write that
            // a masked gate would no longer overwrite.
            if !self
                .name_opt(dst.sig_id as usize)
                .is_some_and(|n| self.module.nets.contains(n))
            {
                continue;
            }
            let (hot, other) = if fa >= fb { (a, b) } else { (b, a) };
            // A clock-tree controlling input re-evaluates the gate only
            // through the eager tree pass, never through its own edge.
            if self
                .is_clock_tree_signal
                .get(other.sig_id as usize)
                .copied()
                .unwrap_or(false)
            {
                continue;
            }
            let e = eidx as u32;
            let edge_range = |sig: u32| -> (usize, usize) {
                let s = sig as usize;
                let (lo, hi) = (offs[s] as usize, offs[s + 1] as usize);
                let deps = &self.comb_dep_entries[lo..hi];
                let k = lo + deps.partition_point(|&d| d < e);
                let mut k2 = k;
                while k2 < hi && self.comb_dep_entries[k2] == e {
                    k2 += 1;
                }
                (k, k2)
            };
            let (k, k2) = edge_range(hot.sig_id);
            // The controlling input must wake the gate (its edge is never
            // masked): that evaluation is what restores the mask.
            let (ok_lo, ok_hi) = edge_range(other.sig_id);
            if k == k2 || ok_lo == ok_hi || fanout(other.sig_id) == 0 {
                continue;
            }
            let smask = self.comb_dep_edges[k].mask;
            if smask == 0 || self.comb_dep_edges[k..k2].iter().any(|d| d.mask != smask) {
                continue;
            }
            let ok = *hot_ok.entry(hot.sig_id).or_insert_with(|| {
                let s = hot.sig_id as usize;
                self.comb_dep_edges[offs[s] as usize..offs[s + 1] as usize]
                    .iter()
                    .all(|d| d.mask != 0)
            });
            if !ok {
                continue;
            }
            picked.push((
                eidx,
                CtlGate {
                    op: *op,
                    other,
                    hot: hot.sig_id,
                    pos: k as u32,
                    npos: (k2 - k) as u32,
                    smask,
                    ctl,
                },
            ));
        }
        if picked.is_empty() {
            return;
        }
        self.ctl_sig = vec![false; nsig];
        for (eidx, g) in picked {
            let slot = self.ctl_gates.len() as u32;
            self.ctl_sig[g.hot as usize] = true;
            let dst = match g.op {
                FusedGate::Bin2 { dst, .. } => dst.sig_id,
                _ => unreachable!(),
            };
            self.ctl_dsts.push((dst, slot));
            self.ctl_gates.push(g);
            self.comb_entries[eidx].item = CombItem::CtlGate { slot };
        }
        self.ctl_dsts.sort_unstable();
        self.gate_lane_valid = false;
        sim_dbg_eprintln!(
            "[CTL-MASK] {} gates skip their high-fanout input while controlled",
            self.ctl_gates.len()
        );
    }

    /// Evaluate `CtlGate` `slot` and set the mask of its high-fanout edges
    /// from the controlling input it just read. `edges` is the dependency
    /// edge table (the settle's local, or `comb_dep_edges`). Returns the
    /// destination when the commit changed it.
    /// `in_settle`: record the change for the settle loop's post-entry
    /// propagation (`ts_direct_dirty`) instead of marking it dirty.
    #[inline(never)]
    pub(super) fn ctl_gate_exec(
        &mut self,
        slot: u32,
        edges: &mut [DepEdge],
        in_settle: bool,
    ) -> Option<BitRef> {
        let g = self.ctl_gates[slot as usize];
        let (dst, new_bit) = self.fused_gate_eval(&g.op);
        let o = self.signal_table[g.other.sig_id as usize].get_bit_code(g.other.bit as usize);
        let m = if o == g.ctl { 0 } else { g.smask };
        let (lo, hi) = (g.pos as usize, (g.pos + g.npos) as usize);
        if let Some(es) = edges.get_mut(lo..hi) {
            for e in es {
                e.mask = m;
            }
        }
        let sdf_any = !self.sdf_delays.is_empty();
        if self.fused_bit_commit(dst, new_bit, sdf_any) {
            if in_settle {
                let chg = self.bit_chg_mask(dst.sig_id, dst.bit);
                self.ts_direct_dirty(dst.sig_id as usize, chg);
            }
            Some(dst)
        } else {
            None
        }
    }

    /// `ctl_gate_exec` outside the settle loop (which holds the edges).
    pub(super) fn ctl_gate_exec_self(&mut self, slot: u32) -> Option<BitRef> {
        let mut edges = std::mem::take(&mut self.comb_dep_edges);
        let r = if edges.len() == self.comb_dep_entries.len() {
            self.ctl_gate_exec(slot, &mut edges, false)
        } else {
            // No edges to update (none yet, or the settle holds them). The
            // mask stays as the last settle evaluation left it: a change of
            // the controlling input since then has woken the gate through
            // its own (never masked) edge, and that evaluation fixes it.
            self.ctl_gate_exec(slot, &mut [], false)
        };
        self.comb_dep_edges = edges;
        r
    }

    /// An external write (deposit) to signal `id`: a `CtlGate` driving it
    /// must see its high-fanout input again, so the next toggle re-evaluates
    /// the gate and overwrites the deposit exactly as an unmasked gate does.
    pub(super) fn ctl_note_external_write(&mut self, id: usize) {
        if self.ctl_dsts.is_empty() {
            return;
        }
        let id = id as u32;
        let start = self.ctl_dsts.partition_point(|&(d, _)| d < id);
        let slots: Vec<u32> = self.ctl_dsts[start..]
            .iter()
            .take_while(|&&(d, _)| d == id)
            .map(|&(_, s)| s)
            .collect();
        if self.comb_dep_edges.len() == self.comb_dep_entries.len() && !self.settling {
            for s in slots {
                self.ctl_rearm(s);
            }
        } else {
            self.ctl_rearm_pending.extend(slots);
        }
    }

    /// `$deposit(target, ...)`: re-arm the gates driving the target's
    /// signal, or every gate when the target does not resolve statically.
    pub(super) fn ctl_note_deposit(&mut self, target: &Expression) {
        if self.ctl_dsts.is_empty() {
            return;
        }
        let mut root = target;
        while let ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } = &root.kind {
            root = expr;
        }
        match self.get_lhs_signal_id(root) {
            Some(id) => self.ctl_note_external_write(id),
            None => {
                let all: Vec<u32> = self.ctl_dsts.iter().map(|&(d, _)| d).collect();
                for d in all {
                    self.ctl_note_external_write(d as usize);
                }
            }
        }
    }

    pub(super) fn ctl_rearm(&mut self, slot: u32) {
        let g = self.ctl_gates[slot as usize];
        let (lo, hi) = (g.pos as usize, (g.pos + g.npos) as usize);
        if let Some(es) = self.comb_dep_edges.get_mut(lo..hi) {
            for e in es {
                e.mask = g.smask;
            }
        }
    }

    /// Resolve the dispatch plan for a comb entry: two-state island if the
    /// block lowers, else the compiled native fn, else the interpreter.
    /// Mirrors the decision order of the unplanned dispatch exactly.
    pub(super) fn resolve_comb_plan(
        &mut self,
        eidx: usize,
        compiled: &super::bytecode::CompiledBlock,
    ) -> CombPlan {
        // Reuse try_two_state's lazy lowering slot (also keeps XEZIM_TWO_STATE
        // and the XEZIM_TS_DBG bail reporting behavior identical).
        use std::sync::OnceLock;
        static ON: OnceLock<bool> = OnceLock::new();
        let ts_on =
            *ON.get_or_init(|| !matches!(std::env::var("XEZIM_TWO_STATE").as_deref(), Ok("0")));
        if ts_on {
            if eidx >= self.ts_comb.len() {
                self.ts_comb.resize(eidx + 1, TsSlot::Untried);
            }
            if matches!(self.ts_comb[eidx], TsSlot::Untried) {
                let lowered = super::bytecode::lower_two_state(
                    compiled,
                    &self.signal_widths,
                    &self.signal_signed,
                    &self.signal_real,
                    &self.array_first_id,
                );
                self.ts_comb[eidx] = match lowered {
                    Some(ts) => {
                        if self.ts_hdr.len() <= eidx {
                            self.ts_hdr.resize(eidx + 1, TsHdr::default());
                        }
                        if ts.insns.len() <= TS_HDR_LEN_MAX
                            && ts.num_regs <= u16::MAX as u32
                            && self.ts_arena.len() < (u32::MAX as usize - ts.insns.len())
                        {
                            let off = self.ts_arena.len() as u32;
                            self.ts_arena.extend(ts.insns.iter().cloned());
                            self.ts_hdr[eidx] = TsHdr {
                                off,
                                len_kind: (ts.insns.len() as u16)
                                    | ((Self::ts_kind_of(&ts) as u16) << TS_HDR_KIND_SHIFT),
                                num_regs: ts.num_regs as u16,
                            };
                            if self.ts_hdr_tsx.len() <= eidx {
                                self.ts_hdr_tsx.resize(eidx + 1, 0);
                            }
                            self.ts_hdr_tsx[eidx] = ts.tsx as u8;
                        }
                        self.bit_sens_pending.push(eidx as u32);
                        TsSlot::Yes(std::sync::Arc::new(ts))
                    }
                    None => {
                        if std::env::var("XEZIM_TS_DBG").is_ok() {
                            let (bi, bop) = super::bytecode::ts_last_bail();
                            eprintln!(
                                "[TS-NO] comb eidx={} bail at #{} opcode={} insns={}",
                                eidx,
                                bi,
                                bop,
                                compiled.instructions.len()
                            );
                        }
                        TsSlot::No
                    }
                };
            }
            if let TsSlot::Yes(ts) = &self.ts_comb[eidx] {
                return CombPlan::Ts(ts.clone());
            }
        }
        #[cfg(feature = "jit")]
        if matches!(self.comb_jit_fns.get(eidx), Some(Some(_))) {
            return CombPlan::Jit;
        }
        #[cfg(feature = "jit")]
        if matches!(self.comb_tpl_fns.get(eidx), Some(Some(_))) {
            return CombPlan::Tpl;
        }
        CombPlan::Interp
    }

    /// XEZIM_COMB_PATHS=1 end-of-sim report for EDGE blocks that ran on the
    /// interpreter: ranked by VM instructions, with the two-state bail.
    /// XEZIM_COMB_PATHS=1: comb entries ranked by evaluations, with the
    /// share repeated inside one time step (several delta cycles).
    pub(super) fn dump_comb_evals(&mut self) {
        if !self.trace_comb_paths || self.comb_eval_counts.is_empty() {
            return;
        }
        let tot: u64 = self.comb_eval_counts.iter().map(|c| c[0]).sum();
        let same: u64 = self.comb_eval_counts.iter().map(|c| c[1]).sum();
        let mut ranked: Vec<(usize, [u64; 2])> = self
            .comb_eval_counts
            .iter()
            .enumerate()
            .map(|(i, c)| (i, *c))
            .filter(|(_, c)| c[0] > 0)
            .collect();
        ranked.sort_by_key(|(_, c)| std::cmp::Reverse(c[0]));
        eprintln!(
            "[EVALS] entries={} evals={} same_time_repeats={} ({:.1}%)",
            ranked.len(),
            tot,
            same,
            100.0 * same as f64 / tot.max(1) as f64
        );
        // Cumulative share by rank buckets.
        let mut acc = 0u64;
        for (rank, (_, c)) in ranked.iter().enumerate() {
            acc += c[0];
            if [10usize, 100, 300, 1000, 3000, 10000].contains(&(rank + 1)) {
                eprintln!(
                    "[EVALS] top {:>5} entries: {:.1}% of evals",
                    rank + 1,
                    100.0 * acc as f64 / tot.max(1) as f64
                );
            }
        }
        let mut kind_hist: std::collections::HashMap<&'static str, u64> =
            std::collections::HashMap::new();
        for (eidx, c) in ranked.iter() {
            let kind = match self.comb_entries.get(*eidx).map(|e| &e.item) {
                Some(CombItem::Noop) => "Noop",
                Some(CombItem::ContAssign { .. }) => "ContAssign",
                Some(CombItem::DirectCopy { .. }) => "DirectCopy",
                Some(CombItem::FastDirectCopy { .. }) => "FastDirectCopy",
                Some(CombItem::FastDirectFanout { .. }) => "FastDirectFanout",
                Some(CombItem::FusedBufFanout { .. }) => "FusedBufFanout",
                Some(CombItem::FusedAndFanout { .. }) => "FusedAndFanout",
                Some(CombItem::CompiledContAssign { .. }) => "CompiledContAssign",
                Some(CombItem::AlwaysBlock { .. }) => "AlwaysBlock",
                Some(CombItem::CompiledAlwaysBlock { .. }) => "CompiledAlwaysBlock",
                Some(CombItem::FusedGate { .. }) => "FusedGate",
                Some(CombItem::VectorGate { .. }) => "VectorGate",
                Some(_) => "other",
                None => "?",
            };
            *kind_hist.entry(kind).or_insert(0) += c[0];
        }
        let mut kh: Vec<_> = kind_hist.into_iter().collect();
        kh.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        for (k, n) in kh {
            eprintln!(
                "[EVALS] kind {:<20} {:>12} ({:.1}%)",
                k,
                n,
                100.0 * n as f64 / tot.max(1) as f64
            );
        }
        // Fused single-bit gates: where does their output go? Weighted by
        // evals. A gate whose only comb consumer is one two-state entry can
        // be absorbed into it.
        {
            let mut h: std::collections::HashMap<&'static str, u64> =
                std::collections::HashMap::new();
            let mut dst_bits_multi = 0u64;
            for (eidx, c) in ranked.iter() {
                let Some(CombItem::FusedGate { op }) =
                    self.comb_entries.get(*eidx).map(|e| &e.item)
                else {
                    continue;
                };
                let dst = match op {
                    FusedGate::Buf1 { dst, .. }
                    | FusedGate::Bin2 { dst, .. }
                    | FusedGate::Mux2 { dst, .. }
                    | FusedGate::UdpLut3 { dst, .. } => *dst,
                };
                let id = dst.sig_id as usize;
                if self.signal_widths.get(id).copied().unwrap_or(1) > 1 {
                    dst_bits_multi += c[0];
                }
                let (lo, hi) = if id + 1 < self.comb_dep_offsets.len() {
                    (
                        self.comb_dep_offsets[id] as usize,
                        self.comb_dep_offsets[id + 1] as usize,
                    )
                } else {
                    (0, 0)
                };
                let cons: Vec<usize> = self.comb_dep_entries[lo..hi]
                    .iter()
                    .map(|&e| e as usize)
                    .collect();
                let key: &'static str = match cons.len() {
                    0 => "fanout0 (edge/process readers only)",
                    1 => {
                        let ce = cons[0];
                        let ts = self.ts_hdr.get(ce).map_or(0, |h| h.kind()) != 0;
                        match self.comb_entries.get(ce).map(|e| &e.item) {
                            Some(CombItem::FusedGate { .. }) => "fanout1 -> gate",
                            Some(CombItem::CompiledContAssign { .. }) => {
                                if ts {
                                    "fanout1 -> TS contassign"
                                } else {
                                    "fanout1 -> interp contassign"
                                }
                            }
                            Some(CombItem::CompiledAlwaysBlock { .. }) => {
                                if ts {
                                    "fanout1 -> TS always"
                                } else {
                                    "fanout1 -> interp always"
                                }
                            }
                            _ => "fanout1 -> other",
                        }
                    }
                    2 => "fanout2",
                    3..=4 => "fanout3-4",
                    _ => "fanout5+",
                };
                *h.entry(key).or_insert(0) += c[0];
            }
            let mut hv: Vec<_> = h.into_iter().collect();
            hv.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
            for (k, n) in hv {
                eprintln!(
                    "[EVALS] gate {:<40} {:>12} ({:.1}% of all evals)",
                    k,
                    n,
                    100.0 * n as f64 / tot.max(1) as f64
                );
            }
            eprintln!(
                "[EVALS] gate dst in multi-bit signal: {} evals",
                dst_bits_multi
            );
        }
        // Entries that evaluate at (nearly) every clock edge.
        let max = ranked.first().map(|(_, c)| c[0]).unwrap_or(0);
        for frac in [1.0f64, 0.66, 0.5, 0.33] {
            let thr = (max as f64 * frac) as u64;
            let n = ranked.iter().filter(|(_, c)| c[0] >= thr).count();
            let sum: u64 = ranked
                .iter()
                .filter(|(_, c)| c[0] >= thr)
                .map(|(_, c)| c[0])
                .sum();
            eprintln!(
                "[EVALS] entries with evals >= {:.2} x max: {} entries, {:.1}% of evals",
                frac,
                n,
                100.0 * sum as f64 / tot.max(1) as f64
            );
        }
        let top_n: usize = std::env::var("XEZIM_EVALS_TOP")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(40);
        for (rank, (eidx, c)) in ranked.iter().take(top_n).enumerate() {
            let (scope, ninsn, nreads) = match self.comb_entries.get(*eidx) {
                Some(e) => (
                    e.cold.scope_hint.as_deref().unwrap_or("?"),
                    match &e.item {
                        CombItem::CompiledContAssign { compiled, .. }
                        | CombItem::CompiledAlwaysBlock { compiled, .. } => {
                            compiled.instructions.len()
                        }
                        _ => 0,
                    },
                    e.cold.read_signal_ids.len(),
                ),
                None => ("?", 0, 0),
            };
            let ts = self.comb_path_counts.get(*eidx).map(|c| c[0]).unwrap_or(0);
            eprintln!(
                "[EVALS] #{rank} eidx={eidx} evals={} same_time={} ts={} insns={} reads={} scope={}",
                c[0], c[1], ts, ninsn, nreads, scope
            );
        }
    }

    pub(super) fn dump_edge_paths(&mut self) {
        if !self.trace_comb_paths || self.edge_path_counts.is_empty() {
            return;
        }
        let mut ranked: Vec<(usize, [u64; 2])> = self
            .edge_path_counts
            .iter()
            .enumerate()
            .filter(|(_, c)| c[1] > 0)
            .map(|(i, c)| (i, *c))
            .collect();
        ranked.sort_by_key(|(_, c)| std::cmp::Reverse(c[1]));
        let tot: u64 = ranked.iter().map(|(_, c)| c[1]).sum();
        eprintln!(
            "[EDGEPATH] interp edge blocks={} vm_insns={}",
            ranked.len(),
            tot
        );
        for (rank, (bi, c)) in ranked.iter().take(40).enumerate() {
            let Some(Some(cb)) = self.compiled_edge_blocks.get(*bi) else {
                continue;
            };
            let lowered = super::bytecode::lower_two_state(
                cb,
                &self.signal_widths,
                &self.signal_signed,
                &self.signal_real,
                &self.array_first_id,
            );
            let (bail, bail_i) = if lowered.is_some() {
                ("(lowers; guard/abort)", usize::MAX)
            } else {
                let (i, op) = super::bytecode::ts_last_bail();
                (op, i)
            };
            let scope = self
                .edge_block_scope
                .get(*bi)
                .and_then(|s| s.as_deref())
                .unwrap_or("?");
            eprintln!(
                "[EDGEPATH] #{rank} block={bi} fires={} vm_insns={} insns={} bail={}@{} why={} scope={}",
                c[0],
                c[1],
                cb.instructions.len(),
                bail,
                bail_i,
                super::bytecode::ts_last_gate(),
                scope
            );
            if rank < 12 {
                let ops: Vec<&'static str> = cb
                    .instructions
                    .iter()
                    .map(super::bytecode::insn_opcode_name)
                    .collect();
                eprintln!("[EDGEPATH]     {}", ops.join(" "));
            }
        }
    }

    /// End-of-sim assertion + functional coverage report.
    ///
    /// Always emits a one-line `[COV]` summary when at least one assertion
    /// site fired or one covergroup instance was sampled. When the env var
    /// `XEZIM_COV_DB` is set (or the default `xezim_cov.json` path is
    /// writable AND there is data to record), writes a JSON coverage
    /// database with per-site assertion counts and per-coverpoint /
    /// per-cross hit sets + sample totals.
    pub(super) fn emit_coverage_summary(&self) {
        let n_assert_sites = self.assertion_stats.len();
        let mut tot_pass = 0u64;
        let mut tot_fail = 0u64;
        let mut k_assert_sites = 0usize;
        let mut k_assume_sites = 0usize;
        let mut k_cover_sites = 0usize;
        for s in self.assertion_stats.values() {
            tot_pass += s.pass_count;
            tot_fail += s.fail_count;
            match s.kind {
                0 => k_assert_sites += 1,
                1 => k_assume_sites += 1,
                _ => k_cover_sites += 1,
            }
        }
        let n_cg_instances = self
            .cg_heap
            .iter()
            .filter(|x| x.is_some())
            .count()
            .saturating_sub(0);
        let mut n_cp_bins_hit = 0usize;
        let mut n_cross_tuples_hit = 0usize;
        let mut tot_samples = 0u64;
        for inst in self.cg_heap.iter().flatten() {
            tot_samples += inst.sample_count;
            for set in inst.point_hits.values() {
                n_cp_bins_hit += set.len();
            }
            for set in inst.cross_hits.values() {
                n_cross_tuples_hit += set.len();
            }
        }

        if n_assert_sites == 0 && n_cg_instances == 0 && self.code_cov.is_none() {
            return;
        }
        chatter!(
            "[COV] assertions: {} sites (assert={}, assume={}, cover={}) — \
             {} passes, {} fails",
            n_assert_sites,
            k_assert_sites,
            k_assume_sites,
            k_cover_sites,
            tot_pass,
            tot_fail
        );
        chatter!(
            "[COV] coverage: {} covergroup instances, {} samples, \
             {} unique coverpoint values, {} unique cross tuples",
            n_cg_instances,
            tot_samples,
            n_cp_bins_hit,
            n_cross_tuples_hit
        );

        // Coverage DB: skip if no data; default path used only when there IS
        // data (so unrelated runs don't drop files).
        let db_path =
            std::env::var("XEZIM_COV_DB").unwrap_or_else(|_| "xezim_cov.json".to_string());
        let mut json = String::new();
        json.push_str("{\n");
        json.push_str(&format!("  \"sim_time\": {},\n", self.time));
        json.push_str(&format!("  \"assertion_sites\": {},\n", n_assert_sites));
        json.push_str(&format!("  \"assertion_pass_total\": {},\n", tot_pass));
        json.push_str(&format!("  \"assertion_fail_total\": {},\n", tot_fail));
        json.push_str("  \"assertions\": [\n");
        let mut sites: Vec<_> = self.assertion_stats.iter().collect();
        sites.sort_by_key(|(k, _)| **k);
        for (i, (key, st)) in sites.iter().enumerate() {
            // §16.5 / §16.14: a labelled site carries its label; an
            // unlabelled one has no `name` key.
            let name = self
                .assertion_labels
                .get(*key)
                .map(|l| format!("\"name\": \"{}\", ", json_escape(l)))
                .unwrap_or_default();
            let kind = match st.kind {
                0 => "assert",
                1 => "assume",
                _ => "cover",
            };
            let (off, place) = match st.loc {
                Some((src, off)) => (
                    off,
                    self.span_file_and_line(crate::ast::Span::new(off, off + 1), src)
                        .map(|(f, l)| {
                            format!("\"file\": \"{}\", \"line\": {}, ", json_escape(&f), l)
                        })
                        .unwrap_or_default(),
                ),
                None => (0, String::new()),
            };
            json.push_str(&format!(
                "    {{{}{}\"span_start\": {}, \"kind\": \"{}\", \"pass\": {}, \"fail\": {}}}{}\n",
                name,
                place,
                off,
                kind,
                st.pass_count,
                st.fail_count,
                if i + 1 < sites.len() { "," } else { "" }
            ));
        }
        json.push_str("  ],\n");
        json.push_str("  \"covergroups\": [\n");
        let mut cg_items: Vec<&CovergroupInstance> = self.cg_heap.iter().flatten().collect();
        // Keep input order; the cg_heap index IS the instance handle.
        let _ = &mut cg_items;
        for (i, inst) in cg_items.iter().enumerate() {
            json.push_str(&format!(
                "    {{\"name\": \"{}\", \"samples\": {}, \"coverpoints\": {{",
                json_escape(&inst.cg_name),
                inst.sample_count
            ));
            let mut cp_iter = inst.point_hits.iter().peekable();
            while let Some((name, set)) = cp_iter.next() {
                json.push_str(&format!(
                    "\"{}\": {}{}",
                    json_escape(name),
                    set.len(),
                    if cp_iter.peek().is_some() { ", " } else { "" }
                ));
            }
            json.push_str("}, \"crosses\": {");
            let mut cr_iter = inst.cross_hits.iter().peekable();
            while let Some((name, set)) = cr_iter.next() {
                json.push_str(&format!(
                    "\"{}\": {}{}",
                    json_escape(name),
                    set.len(),
                    if cr_iter.peek().is_some() { ", " } else { "" }
                ));
            }
            // LRM §19.5 per-explicit-bin hit counts. Empty for coverpoints
            // that didn't declare bins.
            json.push_str("}, \"bins\": {");
            let mut b_iter = inst
                .bin_hits
                .iter()
                .chain(inst.cross_bin_hits.iter())
                .peekable();
            while let Some((name, count)) = b_iter.next() {
                json.push_str(&format!(
                    "\"{}\": {}{}",
                    json_escape(name),
                    count,
                    if b_iter.peek().is_some() { ", " } else { "" }
                ));
            }
            json.push_str("}}");
            if i + 1 < cg_items.len() {
                json.push(',');
            }
            json.push('\n');
        }
        json.push_str("  ]");
        if let Some(code) = self.code_coverage_json() {
            json.push_str(",\n");
            json.push_str(&code);
        }
        json.push_str("\n}\n");
        match std::fs::write(&db_path, &json) {
            Ok(()) => chatter!("[COV] wrote coverage DB to {}", db_path),
            Err(e) => eprintln!("[COV] warning: could not write {}: {}", db_path, e),
        }
    }

    pub(super) fn print_edge_block_stats(&self) {
        let mut opcode_counts: std::collections::HashMap<&'static str, u64> =
            std::collections::HashMap::new();
        let mut pair_counts: std::collections::HashMap<(&'static str, &'static str), u64> =
            std::collections::HashMap::new();
        let mut blocks: Vec<(u64, u64, usize, usize, u32, String)> = Vec::new();

        for (idx, count) in self.edge_block_exec_counts.iter().copied().enumerate() {
            if count == 0 {
                continue;
            }
            let Some(Some(cb)) = self.compiled_edge_blocks.get(idx) else {
                continue;
            };
            let mut max_width = 0u32;
            for insn in &cb.instructions {
                *opcode_counts
                    .entry(Self::edge_opcode_name(insn))
                    .or_insert(0) += count;
                if let Some(id) = Self::edge_insn_signal_id(insn) {
                    max_width = max_width.max(self.signal_widths.get(id).copied().unwrap_or(0));
                }
            }
            // Adjacent-pair census for fusion targeting: what consumes each
            // LoadConst, and what feeds each BranchIfFalse.
            for w in cb.instructions.windows(2) {
                use super::bytecode::Insn;
                if matches!(w[0], Insn::LoadConst(..)) {
                    *pair_counts
                        .entry(("LoadConst>", Self::edge_opcode_name(&w[1])))
                        .or_insert(0) += count;
                }
                if matches!(w[1], Insn::BranchIfFalse(..)) {
                    *pair_counts
                        .entry((">BranchIfFalse", Self::edge_opcode_name(&w[0])))
                        .or_insert(0) += count;
                }
            }
            let scope = self
                .edge_blocks
                .get(idx)
                .and_then(|b| b.resolved_sensitivities.first())
                .and_then(|sid| self.name_opt(sid.signal_id))
                .map(|s| s.to_string())
                .unwrap_or_else(|| "<no-sens>".to_string());
            blocks.push((
                count.saturating_mul(cb.instructions.len() as u64),
                count,
                idx,
                cb.instructions.len(),
                max_width,
                scope,
            ));
        }

        let mut op_vec: Vec<_> = opcode_counts.into_iter().collect();
        op_vec.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
        eprintln!(
            "[EDGE_STATS] top opcodes by static estimate (block insns × fires; branches not followed):"
        );
        for (name, count) in op_vec.into_iter().take(24) {
            eprintln!("[EDGE_STATS]   {:>24}: {}", name, count);
        }

        let mut pair_vec: Vec<_> = pair_counts.into_iter().collect();
        pair_vec.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
        eprintln!("[EDGE_STATS] top fusion-candidate pairs by dynamic count:");
        for ((tag, other), count) in pair_vec.into_iter().take(20) {
            eprintln!("[EDGE_STATS]   {:>16} {:<24}: {}", tag, other, count);
        }

        blocks.sort_by_key(|(dyn_insns, _, _, _, _, _)| std::cmp::Reverse(*dyn_insns));
        eprintln!("[EDGE_STATS] top blocks by dynamic bytecode count:");
        for (dyn_insns, count, idx, len, max_width, scope) in blocks.into_iter().take(20) {
            eprintln!(
                "[EDGE_STATS]   block={} execs={} len={} dyn_insns={} max_sig_width={} sens={}",
                idx, count, len, dyn_insns, max_width, scope
            );
        }
    }

    pub(super) fn edge_opcode_name(insn: &super::bytecode::Insn) -> &'static str {
        use super::bytecode::Insn;
        match insn {
            Insn::LoadSignalRangeDyn(..) => "LoadRngDyn",
            Insn::WaitDelayReg(..) => "WaitDly",
            Insn::WaitEdge(..) => "WaitEdge",
            Insn::CovHit(..) => "CovHit",
            Insn::CaseLut(..) => "CaseLut",
            Insn::CaseJump(..) => "CaseJump",
            Insn::CaseMaskJump(..) => "CaseMaskJump",
            Insn::CmpBranch(..) => "CmpBranch",
            Insn::CmpBranchConst(..) => "CmpBranchConst",
            Insn::MoveResize(..) => "MoveResize",
            Insn::Format(..) => "Format",
            Insn::StrOp(..) => "StrOp",
            Insn::BlockingAssignString(..) => "BlockingAssignString",
            Insn::LoadClassMember(..) => "LoadClassMember",
            Insn::StoreClassMember(..) => "StoreClassMember",
            Insn::CallMethod(..) => "CallMethod",
            Insn::CallScopedMethod(..) => "CallScopedMethod",
            Insn::LoadClassStatic(..) => "LoadClassStatic",
            Insn::StoreClassStatic(..) => "StoreClassStatic",
            Insn::ConstructObject(..) => "ConstructObject",
            Insn::CallStaticScoped(..) => "CallStaticScoped",
            Insn::CallFreeFunction(..) => "CallFreeFunction",
            Insn::CallCollMethod(..) => "CallCollMethod",
            Insn::LoadCollElem(..) => "LoadCollElem",
            Insn::StoreCollElem(..) => "StoreCollElem",
            Insn::Cast(..) => "Cast",
            Insn::ForeachKeys(..) => "ForeachKeys",
            Insn::ForeachNext(..) => "ForeachNext",
            Insn::RetCollection(..) => "RetCollection",
            Insn::AssignPendingColl(..) => "AssignPendingColl",
            Insn::LoadConst(..) => "LoadConst",
            Insn::LoadSignal(..) => "LoadSignal",
            Insn::LoadSignalSigned(..) => "LoadSignalSigned",
            Insn::LoadProcessLocal(..) => "LoadProcessLocal",
            Insn::Resize(..) => "Resize",
            Insn::Add(..) => "Add",
            Insn::Sub(..) => "Sub",
            Insn::Mul(..) => "Mul",
            Insn::Div(..) => "Div",
            Insn::Mod(..) => "Mod",
            Insn::BitAnd(..) => "BitAnd",
            Insn::BitOr(..) => "BitOr",
            Insn::BitXor(..) => "BitXor",
            Insn::BitXnor(..) => "BitXnor",
            Insn::LogAnd(..) => "LogAnd",
            Insn::LogOr(..) => "LogOr",
            Insn::Eq(..) => "Eq",
            Insn::Neq(..) => "Neq",
            Insn::CaseEq(..) => "CaseEq",
            Insn::CasezEq(..) => "CasezEq",
            Insn::CasexEq(..) => "CasexEq",
            Insn::Lt(..) => "Lt",
            Insn::Leq(..) => "Leq",
            Insn::Gt(..) => "Gt",
            Insn::Geq(..) => "Geq",
            Insn::Shl(..) => "Shl",
            Insn::Shr(..) => "Shr",
            Insn::AShr(..) => "AShr",
            Insn::BitNot(..) => "BitNot",
            Insn::LogNot(..) => "LogNot",
            Insn::Negate(..) => "Negate",
            Insn::ReduceAnd(..) => "ReduceAnd",
            Insn::ReduceOr(..) => "ReduceOr",
            Insn::ReduceXor(..) => "ReduceXor",
            Insn::BitSelect(..) => "BitSelect",
            Insn::BitSelectConst(..) => "BitSelectConst",
            Insn::RangeSelect(..) => "RangeSelect",
            Insn::RangeSelectW(..) => "RangeSelectW",
            Insn::RangeSelectConst(..) => "RangeSelectConst",
            Insn::Concat(..) => "Concat",
            Insn::Replicate(..) => "Replicate",
            Insn::BranchIfFalse(..) => "BranchIfFalse",
            Insn::Select(..) => "Select",
            Insn::Jump(..) => "Jump",
            Insn::NbaAssign(..) => "NbaAssign",
            Insn::NbaAssignConst(..) => "NbaAssignConst",
            Insn::BranchUnlessZero(..) => "BranchUnlessZero",
            Insn::BranchIfSignalFalse(..) => "BranchIfSignalFalse",
            Insn::NbaAssignRange(..) => "NbaAssignRange",
            Insn::NbaAssignRangeDyn(..) => "NbaAssignRangeDyn",
            Insn::NbaAssignBitDyn(..) => "NbaAssignBitDyn",
            Insn::BlockingAssign(..) => "BlockingAssign",
            Insn::BlockingAssignRange(..) => "BlockingAssignRange",
            Insn::BlockingAssignRangeDyn(..) => "BlockingAssignRangeDyn",
            Insn::BlockingAssignBitDyn(..) => "BlockingAssignBitDyn",
            Insn::LoadArrayElem(..) => "LoadArrayElem",
            Insn::NbaAssignArray(..) => "NbaAssignArray",
            Insn::BlockingAssignArray(..) => "BlockingAssignArray",
            Insn::NbaAssignArrayRange(..) => "NbaAssignArrayRange",
            Insn::BlockingAssignArrayRange(..) => "BlockingAssignArrayRange",
            Insn::Move(..) => "Move",
            Insn::StmtFallback(..) => "StmtFallback",
            Insn::InitLocalColl(..) => "InitLocalColl",
            Insn::EvalExprFallback(..) => "EvalExprFallback",
            Insn::ClearSigned(..) => "ClearSigned",
            Insn::Pow(..) => "Pow",
            Insn::SetSigned(..) => "SetSigned",
            Insn::DropXz(..) => "DropXz",
            Insn::Nop => "Nop",
            Insn::LoadSignalRange(..) => "LoadSignalRange",
            Insn::LoadSignalBit(..) => "LoadSignalBit",
            Insn::NbaAssignArrayRead(..) => "NbaAssignArrayRead",
            // Split by kind: the pair census is only useful if the three
            // fused ALU ops stay distinguishable.
            Insn::BinOpConstAdd2(..) => "AddC2",
            Insn::BinOpConst(_, _, _, k) => match k {
                super::bytecode::BinOpConstKind::Add => "BinOpConstAdd",
                super::bytecode::BinOpConstKind::Eq => "BinOpConstEq",
                super::bytecode::BinOpConstKind::CaseEq => "BinOpConstCaseEq",
                super::bytecode::BinOpConstKind::Xor => "BinOpConstXor",
                super::bytecode::BinOpConstKind::And => "BinOpConstAnd",
                super::bytecode::BinOpConstKind::Or => "BinOpConstOr",
                super::bytecode::BinOpConstKind::Mul => "BinOpConstMul",
                super::bytecode::BinOpConstKind::Sub => "BinOpConstSub",
            },
        }
    }

    pub(super) fn edge_insn_signal_id(insn: &super::bytecode::Insn) -> Option<usize> {
        use super::bytecode::Insn;
        match insn {
            Insn::LoadSignal(_, id)
            | Insn::LoadSignalSigned(_, id)
            | Insn::LoadSignalRange(_, id, _, _)
            | Insn::LoadSignalBit(_, id, _)
            | Insn::NbaAssign(id, _, _)
            | Insn::NbaAssignConst(id, _, _)
            | Insn::BranchIfSignalFalse(id, _, _)
            | Insn::NbaAssignRange(id, _, _, _)
            | Insn::NbaAssignRangeDyn(id, _, _, _)
            | Insn::NbaAssignBitDyn(id, _, _)
            | Insn::BlockingAssign(id, _, _)
            | Insn::BlockingAssignRange(id, _, _, _)
            | Insn::BlockingAssignRangeDyn(id, _, _, _)
            | Insn::BlockingAssignBitDyn(id, _, _)
            | Insn::NbaAssignArrayRead(id, _, _, _) => Some(*id as usize),
            _ => None,
        }
    }

    #[inline(always)]
    pub(super) fn compose_inline_range_bits(
        base_v: u64,
        base_x: u64,
        src_v: u64,
        src_x: u64,
        low: u32,
        high: u32,
    ) -> (u64, u64) {
        // A DEGENERATE range (`high < low`) reaches here when a declared width
        // was clamped below the range a select uses — e.g. a `[N*W-1:0]` wire
        // wider than SANE_MAX_PACKED_WIDTH is collapsed to 1 bit, then
        // `flat[i*W +: W]` still asks for high bits. `high - low` then
        // underflows: a debug build panics, and a release build wraps to a
        // huge width and silently corrupts neighbouring bits. Treat it as a
        // no-op write instead — callers elide when the value is unchanged.
        if high < low {
            return (base_v, base_x);
        }
        let width = high - low + 1;
        let src_mask = if width >= 64 {
            u64::MAX
        } else {
            (1u64 << width) - 1
        };
        let dst_mask = src_mask << low;
        let new_v = (base_v & !dst_mask) | ((src_v & src_mask) << low);
        let new_x = (base_x & !dst_mask) | ((src_x & src_mask) << low);
        (new_v, new_x)
    }

    pub(super) fn print_activity_report(&self) {
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
            if parts.len() <= 1 {
                return "(top)".to_string();
            }
            // parts: ["uut", "_dff_0_", "CLK"] → block = "uut"
            // parts: ["uut", "_n879_"] → block = "uut"
            // parts: ["uut", "sub", "_mux2_1_", "X"] → block = "uut.sub"
            let mut end = parts.len() - 1; // skip leaf (signal name / port)
            // If the second-to-last segment looks like a gate instance, skip it too
            if end >= 1 && is_gate_instance(parts[end - 1]) {
                end -= 1;
            }
            if end == 0 {
                return parts[0].to_string();
            }
            parts[..end].join(".")
        };

        // Aggregate comb entry triggers by block
        if !self.activity_counts.is_empty() {
            // Always-active entries (>= 0.99 evaluations per clock cycle): what are they?
            {
                let cycles = (self.loop_iters.max(2) / 2) as f64;
                let mut kinds: HashMap<&'static str, (usize, u64)> = HashMap::default();
                let mut names: Vec<(u64, String)> = Vec::new();
                let mut tree_hits = 0usize;
                for (eidx, &c) in self.activity_counts.iter().enumerate() {
                    if (c as f64) / cycles < 0.99 {
                        continue;
                    }
                    let ent = &self.comb_entries[eidx];
                    let k = match &ent.item {
                        CombItem::FusedGate { .. } => "FusedGate",
                        CombItem::CompiledContAssign { .. } => "CompiledContAssign",
                        CombItem::CompiledAlwaysBlock { .. } => "CompiledAlwaysBlock",
                        CombItem::ContAssign { .. } => "ContAssign",
                        CombItem::AlwaysBlock { .. } => "AlwaysBlock",
                        CombItem::DirectCopy { .. } | CombItem::FastDirectCopy { .. } => {
                            "DirectCopy"
                        }
                        CombItem::FastDirectFanout { .. }
                        | CombItem::FusedBufFanout { .. }
                        | CombItem::FusedAndFanout { .. } => "Fanout",
                        CombItem::VectorGate { .. } => "VectorGate",
                        CombItem::ScatterGate { .. } => "ScatterGate",
                        CombItem::GateRegion { .. } => "GateRegion",
                        CombItem::Udp { .. } | CombItem::UdpBatch { .. } => "Udp",
                        _ => "other",
                    };
                    let e = kinds.entry(k).or_insert((0, 0));
                    e.0 += 1;
                    e.1 += c;
                    if self.is_clock_tree_entry.get(eidx).copied().unwrap_or(false) {
                        tree_hits += 1;
                    }
                    if let Some(&w) = ent.cold.write_signal_ids.first() {
                        let reads: Vec<String> = ent
                            .cold
                            .read_signal_ids
                            .iter()
                            .take(3)
                            .map(|&r| {
                                self.name_for_id(r)
                                    .rsplit('.')
                                    .next()
                                    .unwrap_or("")
                                    .to_string()
                            })
                            .collect();
                        names.push((c, format!("{} <- {:?}", self.name_for_id(w), reads)));
                    }
                }
                names.sort_by(|a, b| b.0.cmp(&a.0));
                eprintln!(
                    "[ACTIVE] entries with activity >= 0.99: kinds {:?}; in clock tree: {}",
                    kinds, tree_hits
                );
                for (c, n) in names.iter().take(12) {
                    eprintln!("[ACTIVE]   {:>10} {}", c, &n[..n.len().min(160)]);
                }
            }
            let mut block_triggers: HashMap<String, u64> = HashMap::default();
            let mut block_entry_count: HashMap<String, usize> = HashMap::default();
            for (eidx, &count) in self.activity_counts.iter().enumerate() {
                if count == 0 {
                    continue;
                }
                let entry = &self.comb_entries[eidx];
                // Get destination signal name to determine block
                let dst_name = match &entry.item {
                    CombItem::Noop => "",
                    CombItem::Udp { idx } => {
                        let id = self.udp_runtime[*idx].out_ref.sig_id as usize;
                        self.name_for_id(id)
                    }
                    CombItem::UdpBatch { indices, .. } => {
                        let Some(&idx) = indices.first() else {
                            continue;
                        };
                        let id = self.udp_runtime[idx].out_ref.sig_id as usize;
                        self.name_for_id(id)
                    }
                    CombItem::TimingCheck { .. } => "",
                    CombItem::DirectCopy { dst_id, .. }
                    | CombItem::FastDirectCopy { dst_id, .. } => self.name_for_id(*dst_id),
                    CombItem::FastDirectFanout { dst_ids, .. } => {
                        let Some(&dst_id) = dst_ids.first() else {
                            continue;
                        };
                        self.name_for_id(dst_id)
                    }
                    CombItem::GateRegion { gates } => {
                        let id = match gates.first() {
                            Some(
                                FusedGate::Buf1 { dst, .. }
                                | FusedGate::Bin2 { dst, .. }
                                | FusedGate::Mux2 { dst, .. }
                                | FusedGate::UdpLut3 { dst, .. },
                            ) => dst.sig_id as usize,
                            None => 0,
                        };
                        self.name_for_id(id)
                    }
                    CombItem::FusedGate { op } => {
                        let id = match op {
                            FusedGate::Buf1 { dst, .. }
                            | FusedGate::Bin2 { dst, .. }
                            | FusedGate::Mux2 { dst, .. }
                            | FusedGate::UdpLut3 { dst, .. } => dst.sig_id as usize,
                        };
                        self.name_for_id(id)
                    }
                    CombItem::VectorGate { op } => {
                        let id = match op {
                            VectorGate::Buf { dst, .. }
                            | VectorGate::Bin { dst, .. }
                            | VectorGate::Mux { dst, .. } => dst.sig_id as usize,
                        };
                        self.name_for_id(id)
                    }
                    CombItem::ScatterGate { op } => {
                        let dsts = match op {
                            ScatterGate::Buf { dsts, .. } | ScatterGate::Bin { dsts, .. } => dsts,
                        };
                        let Some(dst) = dsts.first() else { continue };
                        self.name_for_id(*dst as usize)
                    }
                    CombItem::FusedBufFanout { dsts, .. } => {
                        let Some(dst) = dsts.first() else {
                            continue;
                        };
                        self.name_for_id(dst.sig_id as usize)
                    }
                    CombItem::FusedAndFanout { branches, .. } => {
                        let Some(branch) = branches.first() else {
                            continue;
                        };
                        self.name_for_id(branch.dst.sig_id as usize)
                    }
                    CombItem::ContAssign { .. }
                    | CombItem::CompiledContAssign { .. }
                    | CombItem::CtlGate { .. } => {
                        if let Some(&id) = entry.cold.write_signal_ids.first() {
                            self.name_for_id(id)
                        } else {
                            continue;
                        }
                    }
                    CombItem::AlwaysBlock { .. } | CombItem::CompiledAlwaysBlock { .. } => {
                        if let Some(&id) = entry.cold.write_signal_ids.first() {
                            self.name_for_id(id)
                        } else {
                            continue;
                        }
                    }
                };
                // Skip clock signals
                if is_clock(dst_name) {
                    continue;
                }
                let block = block_prefix(dst_name);
                *block_triggers.entry(block.clone()).or_insert(0) += count;
                *block_entry_count.entry(block).or_insert(0) += 1;
            }

            let mut sorted: Vec<_> = block_triggers.into_iter().collect();
            sorted.sort_by(|a, b| b.1.cmp(&a.1));
            sorted.truncate(10);

            eprintln!();
            eprintln!("  Top 10 most active blocks (comb triggers, excl. clocks):");
            eprintln!("  {:>10}  {:>6}  block", "triggers", "entries");
            eprintln!("  {:>10}  {:>6}  -----", "----------", "------");
            for (block, count) in &sorted {
                let entries = block_entry_count.get(block.as_str()).copied().unwrap_or(0);
                eprintln!("  {:>10}  {:>6}  {}", count, entries, block);
            }
        }

        // Aggregate signal toggles by block, exclude clocks
        {
            let mut block_toggles: HashMap<String, u64> = HashMap::default();
            let mut block_sig_count: HashMap<String, usize> = HashMap::default();
            let mut block_top_signal: HashMap<String, (String, u64)> = HashMap::default();
            for (id, &count) in self.signal_toggle_counts.iter().enumerate() {
                if count == 0 {
                    continue;
                }
                let name = self.name_for_id(id);
                if is_clock(name) {
                    continue;
                }
                let block = block_prefix(name);
                *block_toggles.entry(block.clone()).or_insert(0) += count;
                *block_sig_count.entry(block.clone()).or_insert(0) += 1;
                let top = block_top_signal
                    .entry(block)
                    .or_insert((name.to_string(), 0));
                if count > top.1 {
                    *top = (name.to_string(), count);
                }
            }

            // Working-set summary: how many DISTINCT signals ever toggle
            // (the true hot set that must live in cache) and how spread out
            // their ids are (spatial-locality proxy). Compare vs L2 (1 MiB) /
            // L3 (16.5 MiB) to judge cache fit.
            {
                let mut hot = 0usize;
                let mut min_id = usize::MAX;
                let mut max_id = 0usize;
                for (id, &count) in self.signal_toggle_counts.iter().enumerate() {
                    if count == 0 {
                        continue;
                    }
                    hot += 1;
                    min_id = min_id.min(id);
                    max_id = max_id.max(id);
                }
                let vsz = std::mem::size_of::<Value>();
                let hot_bytes = hot * vsz;
                let span = if hot > 0 { max_id - min_id + 1 } else { 0 };
                let mib = |b: usize| b as f64 / (1024.0 * 1024.0);
                eprintln!();
                eprintln!(
                    "  [WORKING-SET] hot(toggling) signals = {} of {} ({:.1}%)",
                    hot,
                    self.signal_toggle_counts.len(),
                    100.0 * hot as f64 / self.signal_toggle_counts.len().max(1) as f64
                );
                eprintln!(
                    "  [WORKING-SET] hot Value bytes = {:.2} MiB  (=> {:.1}x L2 / {:.2}x L3)",
                    mib(hot_bytes),
                    hot_bytes as f64 / (1024.0 * 1024.0),
                    hot_bytes as f64 / (16.5 * 1024.0 * 1024.0)
                );
                eprintln!(
                    "  [WORKING-SET] id span = {} (ids {}..{}); density = {:.1}% of the span is hot",
                    span,
                    min_id,
                    max_id,
                    100.0 * hot as f64 / span.max(1) as f64
                );
            }

            let mut sorted: Vec<_> = block_toggles.into_iter().collect();
            sorted.sort_by(|a, b| b.1.cmp(&a.1));
            sorted.truncate(10);

            eprintln!();
            eprintln!("  Top 10 most toggling blocks (signal changes, excl. clocks):");
            eprintln!(
                "  {:>10}  {:>5}  {:40}  hottest signal",
                "toggles", "sigs", "block"
            );
            eprintln!(
                "  {:>10}  {:>5}  {:40}  --------------",
                "----------", "-----", "-----"
            );
            for (block, count) in &sorted {
                let sigs = block_sig_count.get(block.as_str()).copied().unwrap_or(0);
                let (hot_name, hot_count) = block_top_signal
                    .get(block.as_str())
                    .map(|(n, c)| (n.as_str(), *c))
                    .unwrap_or(("?", 0));
                let hot_short = hot_name
                    .strip_prefix(block.as_str())
                    .and_then(|s| s.strip_prefix('.'))
                    .unwrap_or(hot_name);
                eprintln!(
                    "  {:>10}  {:>5}  {:40}  {} ({})",
                    count, sigs, block, hot_short, hot_count
                );
            }
        }
        eprintln!();
    }
}
