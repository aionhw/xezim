//! xezim — SystemVerilog bytecode interpreter.
//!
//! Parsing, elaboration, and shared runtime primitives live in the
//! sibling `xezim-core` crate. This crate adds the event-driven
//! interpreter (`simulator`) and bytecode IR (`bytecode`).
//!
//! For ahead-of-time native compilation, use the `xezim-b` crate.

pub mod compiler;
pub mod multikernel;
pub mod tbb;

// Re-export xezim-core surface so existing `xezim::...` paths keep working.
pub use xezim_core::{
    ast, diagnostics, lexer, log_eprintln, log_println, parse, parse_and_elaborate_multi,
    parse_str, preprocessor, read_compiled, set_log_file, sv_parser, tokenize_file, write_compiled,
    ParseResult, SourceDefinition, XEZIM_BYTECODE_MAGIC,
};

/// Simulate a single source string.
pub fn simulate(source: &str, max_time: u64) -> Result<compiler::Simulator, String> {
    simulate_multi(
        &[source.to_string()],
        max_time,
        None,
        &[],
        &[],
        None,
        false,
        None,
        None,
        &[],
        false,
        &[],
        1,
        None,
        &[],
        None,
        None,
        None,
        None,
        false,
        None,
    )
}

pub fn simulate_multi(
    sources: &[String],
    max_time: u64,
    top_module_name: Option<&str>,
    include_dirs: &[String],
    source_paths: &[String],
    settle_limit: Option<u32>,
    activity_mon: bool,
    sdf_file: Option<&str>,
    sdf_select: Option<xezim_core::sdf::DelaySelect>,
    defines: &[(String, Option<String>)],
    aitrace: bool,
    plusargs: &[String],
    threads: usize,
    xtrace_file: Option<&str>,
    xtrace_scopes: &[String],
    emit_hypergraph: Option<&str>,
    load_partition: Option<&str>,
    write_profile: Option<&str>,
    profile_input: Option<&str>,
    collapse_islands: bool,
    multikernel_scope: Option<&str>,
) -> Result<compiler::Simulator, String> {
    let total_start = std::time::Instant::now();
    let mut sim = compile_multi(
        sources,
        max_time,
        top_module_name,
        include_dirs,
        source_paths,
        defines,
    )?;
    if let Some(limit) = settle_limit {
        sim.settle_limit = limit;
    }
    sim.activity_mon = activity_mon;
    sim.aitrace_mode = aitrace;
    sim.xtrace_file = xtrace_file.map(|s| s.to_string());
    sim.xtrace_scopes = xtrace_scopes.to_vec();
    sim.set_plusargs(plusargs);
    sim.set_threads(threads);

    if let Some(sdf_path) = sdf_file {
        let sdf_content = std::fs::read_to_string(sdf_path)
            .map_err(|e| format!("Cannot read SDF file '{}': {}", sdf_path, e))?;
        let sdf = xezim_core::sdf::parse_sdf(&sdf_content)
            .map_err(|e| format!("SDF parse error: {}", e))?;
        let select = sdf_select.unwrap_or(xezim_core::sdf::DelaySelect::Typ);
        let sim_timescale = 1e-9;
        let annotation = xezim_core::sdf::annotate_sdf(&sdf, sim_timescale, select);
        sim.sdf_annotation = Some(annotation);
    }

    if let Some(path) = emit_hypergraph {
        let t = std::time::Instant::now();
        // Phase-2 profile-guided emission: --profile-input takes
        // precedence; falls back to static weights without it.
        let prof = if let Some(pp) = profile_input {
            match compiler::simulator::Phase2Profile::load_from_file(pp) {
                Ok(p) => {
                    eprintln!(
                        "[PART] using profile {} ({} blocks, {} signals)",
                        pp,
                        p.edge_block_exec_ns.len(),
                        p.signal_toggle_count.len()
                    );
                    Some(p)
                }
                Err(e) => {
                    eprintln!(
                        "[PART] failed to load profile {}: {} — falling back to static",
                        pp, e
                    );
                    None
                }
            }
        } else {
            None
        };
        // Phase-3 island analysis (optional). Computes which blocks
        // MUST be co-located across cores (async-reset cones, comb
        // SCCs) and collapses them into super-vertices in the emitted
        // hypergraph. Without --collapse-islands, every block is its
        // own vertex (Phase 1/2 behavior).
        let islands = if collapse_islands {
            Some(sim.compute_phase3_islands())
        } else {
            None
        };
        let result = sim.emit_edge_block_hypergraph_full(path, prof.as_ref(), islands.as_deref());
        match result {
            Ok((nv, ne)) => eprintln!(
                "[PART] hypergraph written to {} ({} vertices, {} hyperedges, weights={}, islands={}) in {:.1}ms",
                path,
                nv,
                ne,
                if prof.is_some() { "profile" } else { "static" },
                if islands.is_some() { "phase3" } else { "off" },
                t.elapsed().as_secs_f64() * 1000.0
            ),
            Err(e) => eprintln!("[PART] failed to write hypergraph to {}: {}", path, e),
        }
    }
    if let Some(path) = load_partition {
        let t = std::time::Instant::now();
        match sim.load_partition_file(path) {
            Ok((n, parts)) => eprintln!(
                "[PART] loaded partition from {} ({} assignments, k={}) in {:.1}ms",
                path,
                n,
                parts,
                t.elapsed().as_secs_f64() * 1000.0
            ),
            Err(e) => eprintln!("[PART] failed to load partition from {}: {}", path, e),
        }
    }

    // Multikernel-scope partition: split parallel-eligible blocks
    // into 2 LPs by scope prefix. Sets edge_block_partition so the
    // dispatcher consumes the per-LP chunks. PDES dispatcher (when
    // added) reads the same partition data.
    if let Some(prefix) = multikernel_scope {
        let t = std::time::Instant::now();
        let (n_a, n_b) = sim.apply_multikernel_scope_partition(prefix);
        eprintln!(
            "[PART] multikernel-scope applied (LP-A={}, LP-B={}) in {:.1}ms",
            n_a,
            n_b,
            t.elapsed().as_secs_f64() * 1000.0
        );
        if std::env::var("XEZIM_PDES_PHASE4_DRYRUN").ok().as_deref() == Some("1") {
            let t = std::time::Instant::now();
            let stats = sim.pdes_phase4_runtime_dry_run(prefix, 10);
            eprintln!(
                "[PDES-Phase4] dry-run: LPs={}, K={}, sync_rounds_100ticks={}, local_signals={:?}, local_table_mb={:?}, boundary_channels={}, outbound={:?}, inbound={:?}, send_ctx_blocks={}, send_ctx_signals={} ({:.1}ms)",
                stats.lp_count,
                stats.lookahead_k,
                stats.sync_rounds_for_100_ticks,
                stats.local_table_signal_counts,
                stats
                    .local_table_bytes
                    .iter()
                    .map(|b| (*b as f64) / (1024.0 * 1024.0))
                    .collect::<Vec<_>>(),
                stats.boundary_channels,
                stats.outbound_endpoints,
                stats.inbound_endpoints,
                stats.send_context_blocks,
                stats.send_context_signals,
                t.elapsed().as_secs_f64() * 1000.0
            );
        }
    }

    let simulation_start = std::time::Instant::now();
    sim.simulate();
    eprintln!(
        "[PHASE] simulation: {:.1}ms",
        simulation_start.elapsed().as_secs_f64() * 1000.0
    );

    if let Some(path) = write_profile {
        let t = std::time::Instant::now();
        match sim.write_phase2_profile(path) {
            Ok(()) => eprintln!(
                "[PART] profile written to {} in {:.1}ms (set XEZIM_EDGE_BLOCK_STATS=1 to populate)",
                path,
                t.elapsed().as_secs_f64() * 1000.0
            ),
            Err(e) => eprintln!("[PART] failed to write profile to {}: {}", path, e),
        }
    }

    let total_elapsed = total_start.elapsed();
    eprintln!(
        "[PHASE] total: {:.1}ms",
        total_elapsed.as_secs_f64() * 1000.0
    );
    eprintln!("------------------------------");
    eprintln!("Simulation finished at time {}", sim.time);
    Ok(sim)
}

pub fn compile_multi(
    sources: &[String],
    max_time: u64,
    top_module_name: Option<&str>,
    include_dirs: &[String],
    source_paths: &[String],
    defines: &[(String, Option<String>)],
) -> Result<compiler::Simulator, String> {
    let compilation_start = std::time::Instant::now();
    let (definitions, elab) = parse_and_elaborate_multi(
        sources,
        top_module_name,
        include_dirs,
        source_paths,
        defines,
    )?;

    // Drop the parsed-AST table now that elaborate has produced ElaboratedModule.
    // Nothing downstream (Simulator::new, sim.run, SDF parse) needs it. Without
    // this the AHashMap<String, SourceDefinition> (Rc<ModuleDeclaration> for
    // ~hundreds of c910 modules) sits in RSS for the entire 3-min simulation.
    // Measured: c910 hello peak 9.98 GB → ~8 GB after explicit drop.
    drop(definitions);

    let mut sim = compiler::Simulator::new(elab, max_time);
    sim.compile();
    eprintln!(
        "[PHASE] compilation: {:.1}ms",
        compilation_start.elapsed().as_secs_f64() * 1000.0
    );
    Ok(sim)
}

/// PDES c910 stub: parse + elaborate + compile a design, then run the
/// `multikernel::PdesCoordinator` with stub blocks instead of the real
/// `event_loop`. Stops short of executing real bytecode — proves the
/// parse/compile/classify/kernel-construction pipeline scales to c910
/// (35M signals, 20K blocks) and the coordinator's tick loop runs
/// against c910-sized partitions without OOM or pathological slowdown.
pub fn pdes_c910_stub_multi(
    sources: &[String],
    top_module_name: Option<&str>,
    include_dirs: &[String],
    source_paths: &[String],
    defines: &[(String, Option<String>)],
    lp_a_prefix: &str,
    n_ticks: u64,
) -> Result<(), String> {
    let total_start = std::time::Instant::now();
    let parse_start = std::time::Instant::now();
    let (definitions, elab) = parse_and_elaborate_multi(
        sources,
        top_module_name,
        include_dirs,
        source_paths,
        defines,
    )?;
    drop(definitions);
    eprintln!(
        "[PHASE] parse+elaborate: {:.1}ms",
        parse_start.elapsed().as_secs_f64() * 1000.0
    );

    let compile_start = std::time::Instant::now();
    let mut sim = compiler::Simulator::new(elab, 0);
    sim.compile();
    eprintln!(
        "[PHASE] compilation: {:.1}ms",
        compile_start.elapsed().as_secs_f64() * 1000.0
    );
    eprintln!(
        "[PDES-stub] design: {} signals, {} compiled edge blocks",
        sim.signal_table_len(),
        sim.edge_block_count()
    );

    let classify_start = std::time::Instant::now();
    let io = multikernel::classify_lp_io(&sim, lp_a_prefix);
    eprintln!(
        "[PHASE] PDES classify_lp_io: {:.1}ms",
        classify_start.elapsed().as_secs_f64() * 1000.0
    );
    eprintln!(
        "[PDES-IO] blocks: total={}, parallel={}, LP-A={}, LP-B={}",
        io.blocks_total, io.blocks_parallel, io.blocks_lp_a, io.blocks_lp_b
    );
    eprintln!(
        "[PDES-IO] writers: LP-A-only={}, LP-B-only={}, multi(boundary-violation)={}",
        io.writers_lp_a_only, io.writers_lp_b_only, io.writers_boundary
    );
    eprintln!(
        "[PDES-IO] readers: LP-A-only={}, LP-B-only={}, both={}",
        io.readers_lp_a_only, io.readers_lp_b_only, io.readers_both
    );
    eprintln!(
        "[PDES-IO] boundary signals (need per-tick channel updates): A→B={}, B→A={}, bidir={}",
        io.boundary_a_to_b, io.boundary_b_to_a, io.boundary_bidir
    );
    eprintln!(
        "[PDES-IO] PARALLEL-ELIGIBLE-only total boundary channel set = {} signals",
        io.boundary_a_to_b + io.boundary_b_to_a + io.boundary_bidir
    );
    eprintln!(
        "[PDES-IO] ALL-BLOCKS blocks: total={}, LP-A={}, LP-B={}",
        io.all_blocks_total, io.all_blocks_lp_a, io.all_blocks_lp_b
    );
    eprintln!(
        "[PDES-IO] ALL-BLOCKS writers: LP-A-only={}, LP-B-only={}, multi={}",
        io.all_writers_lp_a_only, io.all_writers_lp_b_only, io.all_writers_boundary
    );
    eprintln!(
        "[PDES-IO] ALL-BLOCKS readers: LP-A-only={}, LP-B-only={}, both={}",
        io.all_readers_lp_a_only, io.all_readers_lp_b_only, io.all_readers_both
    );
    eprintln!(
        "[PDES-IO] ALL-BLOCKS boundary signals: A→B={}, B→A={}, bidir={}, TOTAL={}",
        io.all_boundary_a_to_b,
        io.all_boundary_b_to_a,
        io.all_boundary_bidir,
        io.all_boundary_a_to_b + io.all_boundary_b_to_a + io.all_boundary_bidir
    );
    eprintln!(
        "[PDES-IO] COMB entries: total={}, LP-A={}, LP-B={}, unscoped→LP-B={}",
        io.comb_entries_total, io.comb_entries_lp_a, io.comb_entries_lp_b, io.comb_entries_unscoped
    );
    eprintln!(
        "[PDES-IO] COMB writers: LP-A-only={}, LP-B-only={}, multi={}",
        io.comb_writers_lp_a_only, io.comb_writers_lp_b_only, io.comb_writers_boundary
    );
    eprintln!(
        "[PDES-IO] COMB readers: LP-A-only={}, LP-B-only={}, both={}",
        io.comb_readers_lp_a_only, io.comb_readers_lp_b_only, io.comb_readers_both
    );
    eprintln!(
        "[PDES-IO] COMB boundary signals (TRUE CHANNEL SET): A→B={}, B→A={}, bidir={}, TOTAL={}",
        io.comb_boundary_a_to_b,
        io.comb_boundary_b_to_a,
        io.comb_boundary_bidir,
        io.comb_boundary_a_to_b + io.comb_boundary_b_to_a + io.comb_boundary_bidir
    );

    // Dump the first 30 boundary signal names so we can verify they
    // match expected cross-core paths (BIU master interface, interrupt
    // lines, AXI fabric, etc.).
    let dump_n = io.boundary_signal_ids.len().min(30);
    if dump_n > 0 {
        eprintln!("[PDES-IO] First {} boundary signal names:", dump_n);
        for (sig_id, dir) in io
            .boundary_signal_ids
            .iter()
            .take(dump_n)
            .zip(io.boundary_directions.iter())
        {
            let dir_str = match dir {
                0 => "A→B",
                1 => "B→A",
                2 => "bidir",
                _ => "?",
            };
            eprintln!(
                "[PDES-IO]   id={:7} {}  {}",
                sig_id,
                dir_str,
                sim.signal_name_at(*sig_id)
            );
        }
    }

    // Phase 4: partition the combinational settle layer across LPs.
    // The per-LP settle worker iterates lp_entries[lp]; straddle/orphan
    // entries run on the coordinator. Cross-validate the boundary set
    // against classify_lp_io's comb boundary total.
    let part_start = std::time::Instant::now();
    let part = sim.pdes_build_comb_partition(lp_a_prefix);
    eprintln!(
        "[PHASE] PDES comb partition: {:.1}ms",
        part_start.elapsed().as_secs_f64() * 1000.0
    );
    eprintln!(
        "[PDES-PART] entries: total={}, LP-A={}, LP-B={}, straddle={}, orphan={} (coverage_ok={})",
        part.total_entries,
        part.lp_entries[0].len(),
        part.lp_entries[1].len(),
        part.straddle_entries.len(),
        part.orphan_entries.len(),
        part.coverage_ok(),
    );
    eprintln!(
        "[PDES-PART] comb_dep edges: total={}, cross-LP={} ({:.4}%)",
        part.dep_edges_total,
        part.dep_edges_cross_lp,
        if part.dep_edges_total > 0 {
            100.0 * part.dep_edges_cross_lp as f64 / part.dep_edges_total as f64
        } else {
            0.0
        },
    );
    eprintln!(
        "[PDES-PART] boundary signals (partition view) = {} (classify comb total = {})",
        part.boundary_signal_ids.len(),
        io.comb_boundary_a_to_b + io.comb_boundary_b_to_a + io.comb_boundary_bidir,
    );

    // Validate the isolated comb evaluator against the fixpoint invariant:
    // re-running each compiled comb block at the settled state must produce
    // no write and no deferred NBA. mismatched/unsupported must be 0 for the
    // per-LP settle to be sound.
    let chk_start = std::time::Instant::now();
    let (checked, bits_mismatch, repr_diff, unsupported, deferred) =
        sim.pdes_check_comb_isolated();
    eprintln!(
        "[PHASE] PDES isolated-comb check: {:.1}ms",
        chk_start.elapsed().as_secs_f64() * 1000.0
    );
    eprintln!(
        "[PDES-CHK] exec_comb_block_isolated: checked={}, bits_mismatch={}, repr_diff={}, unsupported={}, deferred_nba={}",
        checked, bits_mismatch, repr_diff, unsupported, deferred
    );

    // Per-LP settle driver validation: settle each LP's subset independently
    // (boundary frozen at the global fixpoint) and confirm the merged result
    // reconverges to the global settle. mismatches ~0 == driver correct.
    let pls_start = std::time::Instant::now();
    let (e_a, e_b, it_a, it_b, mismatches, unsup) = sim.pdes_validate_perlp_settle(lp_a_prefix);
    eprintln!(
        "[PHASE] PDES per-LP settle validation: {:.1}ms",
        pls_start.elapsed().as_secs_f64() * 1000.0
    );
    eprintln!(
        "[PDES-SETTLE] LP-A entries={} (iters={}), LP-B entries={} (iters={}), mismatches={}, unsupported_evals={}",
        e_a, it_a, e_b, it_b, mismatches, unsup
    );

    // Threaded per-LP settle: run the two LP settles concurrently on worker
    // threads via the Send-able CombSettleCtx; compare seq vs parallel wall
    // time and confirm the merged result still matches the global settle.
    // (1) semantic core0-vs-rest partition (imbalanced).
    let (t_mismatch, t_unsup, seq_ms, par_ms, clone_ms) =
        sim.pdes_validate_perlp_settle_threaded(&part);
    eprintln!(
        "[PDES-SETTLE-MT semantic] LP-A={}/LP-B={} entries, mismatches={}, unsupported={}, settle_seq={:.1}ms, settle_par={:.1}ms, speedup={:.2}x, view_clone={:.1}ms",
        part.lp_entries[0].len(),
        part.lp_entries[1].len(),
        t_mismatch, t_unsup, seq_ms, par_ms,
        if par_ms > 0.0 { seq_ms / par_ms } else { 0.0 },
        clone_ms
    );

    // (2) balanced core0 || core1 partition with uncore distributed by
    // read-affinity. lp_b_prefix = the core1 sibling of lp_a_prefix.
    let lp_b_prefix = if lp_a_prefix.ends_with("x_ct_top_0") {
        lp_a_prefix.replace("x_ct_top_0", "x_ct_top_1")
    } else {
        // Fallback: no known sibling — reuse the semantic partition.
        String::new()
    };
    if !lp_b_prefix.is_empty() {
        // Edge-block (clocked always) per-LP partition + threaded exec.
        // This is the 55.6% chunk of the sim loop. Workers share the
        // snapshot read-only — no per-LP view clone.
        // Phase A de-risk: boundary signal lookahead (registered vs comb).
        // comb==0 => lookahead-1 channel is functionally sound (GO).
        let (nb, comb_consumed, prod_comb, prod_both, prod_undr, comb_names) =
            sim.pdes_boundary_lookahead_report(lp_a_prefix);
        eprintln!(
            "[PDES-LOOKAHEAD] boundary={}, comb_consumed(TRUE blocker)={} [producer: comb={} both={} undriven={}] -> {}",
            nb, comb_consumed, prod_comb, prod_both, prod_undr,
            if comb_consumed == 0 { "GO: lookahead-1 sound" } else { "co-locate these cones" }
        );
        if !comb_names.is_empty() {
            eprintln!("[PDES-LOOKAHEAD] comb-consumed-across-cut signals (sample):");
            for n in comb_names.iter().take(8) {
                eprintln!("[PDES-LOOKAHEAD]   {}", n);
            }
        }

        // Phase A.2: cycle-vs-feedforward analysis of the cross-LP coupling.
        let ca_start = std::time::Instant::now();
        let (a2b, b2a, max_cross, rounds, converged) =
            sim.pdes_crosslp_cycle_analysis(lp_a_prefix);
        eprintln!(
            "[PDES-CYCLE] cross-LP comb edges: A->B={}, B->A={} ({}) | wavefront max_crossings={}, rounds={}, converged={} ({})",
            a2b, b2a,
            if a2b == 0 || b2a == 0 { "UNIDIRECTIONAL/feedforward" } else { "BIDIRECTIONAL" },
            max_cross, rounds, converged,
            if !converged {
                "COMB CYCLE (or depth>cap) — iteration may not terminate"
            } else if max_cross <= 1 {
                "feedforward: 1 producer-ordered pass suffices"
            } else {
                "needs max_crossings iterated exchange rounds (DAG, converges)"
            }
        );
        eprintln!(
            "[PDES-CYCLE] analysis took {:.1}ms",
            ca_start.elapsed().as_secs_f64() * 1000.0
        );

        let ep = sim.pdes_build_edge_partition(lp_a_prefix, &lp_b_prefix);
        eprintln!(
            "[PDES-EDGE-PART] parallel blocks={}: LP-0={} LP-1={} (uncore={}), cross-LP NBA writers={}",
            ep.total_parallel,
            ep.lp_blocks[0].len(),
            ep.lp_blocks[1].len(),
            ep.uncore_blocks,
            ep.cross_lp_nba_writers,
        );
        let (e_nba, e_mismatch, e_seq, e_par) = sim.pdes_validate_perlp_edge_threaded(&ep);
        eprintln!(
            "[PDES-EDGE-MT] nba_writes={}, mismatches={}, exec_seq={:.1}ms, exec_par={:.1}ms, speedup={:.2}x (shared snapshot, no clone)",
            e_nba, e_mismatch, e_seq, e_par,
            if e_par > 0.0 { e_seq / e_par } else { 0.0 }
        );

        // Edge-exec thread scaling (embarrassingly parallel against the
        // shared snapshot). Best of 2 trials per thread count to damp noise.
        for &nt in &[1usize, 2, 4] {
            let mut best_seq = f64::MAX;
            let mut best_par = f64::MAX;
            let (mut nba, mut mm) = (0usize, 0usize);
            for _ in 0..2 {
                let (n, m, sq, pr) = sim.pdes_validate_edge_nthreads(nt);
                nba = n;
                mm = m;
                best_seq = best_seq.min(sq);
                best_par = best_par.min(pr);
            }
            eprintln!(
                "[PDES-EDGE-SCALE] threads={}: nba={}, mismatches={}, seq={:.2}ms, par={:.2}ms, speedup={:.2}x",
                nt, nba, mm, best_seq, best_par,
                if best_par > 0.0 { best_seq / best_par } else { 0.0 }
            );
        }

        let bal = sim.pdes_build_comb_partition_balanced(lp_a_prefix, &lp_b_prefix);
        eprintln!(
            "[PDES-PART balanced] LP-0={} LP-1={} straddle={} orphan={} (coverage_ok={}), cross-LP edges={} ({:.4}%), boundary={}",
            bal.lp_entries[0].len(),
            bal.lp_entries[1].len(),
            bal.straddle_entries.len(),
            bal.orphan_entries.len(),
            bal.coverage_ok(),
            bal.dep_edges_cross_lp,
            if bal.dep_edges_total > 0 {
                100.0 * bal.dep_edges_cross_lp as f64 / bal.dep_edges_total as f64
            } else { 0.0 },
            bal.boundary_signal_ids.len(),
        );
        let (b_mismatch, b_unsup, b_seq, b_par, b_clone) =
            sim.pdes_validate_perlp_settle_threaded(&bal);
        eprintln!(
            "[PDES-SETTLE-MT balanced] LP-0={}/LP-1={} entries, mismatches={}, unsupported={}, settle_seq={:.1}ms, settle_par={:.1}ms, speedup={:.2}x, view_clone={:.1}ms",
            bal.lp_entries[0].len(),
            bal.lp_entries[1].len(),
            b_mismatch, b_unsup, b_seq, b_par,
            if b_par > 0.0 { b_seq / b_par } else { 0.0 },
            b_clone
        );

        // Combined single-tick pipeline (edge exec -> NBA apply -> seeded
        // settle), parallel vs sequential. Run with BOTH comb partitions:
        // semantic (sound, few cross-LP edges) and balanced (fast, many
        // cross-LP edges) — a seeded tick reveals whether balanced's cross-
        // LP edges break within-tick correctness (unlike the fixpoint test).
        for (label, cp) in [("semantic", &part), ("balanced", &bal)] {
            let (m, e_nba, t_seq, t_par) = sim.pdes_validate_perlp_tick(cp, &ep);
            eprintln!(
                "[PDES-TICK {}] edge_nba={}, mismatches={}, tick_seq={:.1}ms, tick_par={:.1}ms, speedup={:.2}x",
                label, e_nba, m, t_seq, t_par,
                if t_par > 0.0 { t_seq / t_par } else { 0.0 }
            );
        }

        // Real multi-tick parallel event loop (clock-toggled ticks, parallel
        // edge exec + boundary channel + parallel settle), validated tick-by-
        // tick vs sequential. Uses the semantic (sound) comb cut.
        let n_ticks = std::env::var("XEZIM_PDES_TICKS")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(8);
        for &et in &[2usize, 4] {
            let (per_tick_mm, mt_seq, mt_par) =
                sim.pdes_validate_parallel_multitick(&part, &ep, n_ticks, et);
            eprintln!(
                "[PDES-MULTITICK et={}] {} ticks: seq={:.1}ms par={:.1}ms speedup={:.2}x, final_mismatch={} (settle 2-way, edge {}-way, boundary channel, clone-free/tick)",
                et, n_ticks, mt_seq, mt_par,
                if mt_par > 0.0 { mt_seq / mt_par } else { 0.0 },
                per_tick_mm.last().copied().unwrap_or(0),
                et
            );
        }
    }

    // Allocate a Value-backed signal table at full c910 scale —
    // measures the memory + per-tick snapshot cost of the naive
    // (snapshot-everything) per-LP exec model.
    let alloc_start = std::time::Instant::now();
    let (value_table, value_bytes) = multikernel::allocate_c910_value_signal_table(&sim);
    let alloc_ms = alloc_start.elapsed().as_secs_f64() * 1000.0;
    eprintln!(
        "[PHASE] PDES Value-table allocation: {:.1}ms ({:.1} MB for {} cells × {} B)",
        alloc_ms,
        value_bytes as f64 / 1_048_576.0,
        value_table.len(),
        std::mem::size_of::<xezim_core::Value>()
    );

    // Benchmark a single full-table snapshot — the naive cost the
    // sparse-snapshot optimization needs to beat. Repeat 3 times so
    // the allocator's first-touch cost amortizes.
    let mut snap_total_ms = 0.0f64;
    for i in 0..3 {
        let (snap, dt) = multikernel::benchmark_value_snapshot(&value_table);
        let dt_ms = dt.as_secs_f64() * 1000.0;
        snap_total_ms += dt_ms;
        eprintln!(
            "[PDES-IO] Value-table full snapshot (iter {}): {:.1}ms, {} cells, allocation peak {:.1} MB",
            i,
            dt_ms,
            snap.len(),
            (snap.capacity() * std::mem::size_of::<xezim_core::Value>()) as f64 / 1_048_576.0
        );
        // Drop the snapshot so the next iter's peak is independent.
        drop(snap);
    }
    eprintln!(
        "[PDES-IO] Value-table snapshot avg: {:.1}ms (full-table; sparse will be much smaller)",
        snap_total_ms / 3.0
    );

    // Sparse-snapshot benchmark: clone only the cells in each LP's
    // comb-traced read set. This is the per-tick cost the production
    // per-LP architecture would pay.
    eprintln!(
        "[PDES-IO] LP read-set sizes: LP-A={} signals ({:.1} MB), LP-B={} signals ({:.1} MB)",
        io.read_set_lp_a.len(),
        (io.read_set_lp_a.len() * std::mem::size_of::<xezim_core::Value>()) as f64 / 1_048_576.0,
        io.read_set_lp_b.len(),
        (io.read_set_lp_b.len() * std::mem::size_of::<xezim_core::Value>()) as f64 / 1_048_576.0,
    );
    let mut sparse_a_total = 0.0f64;
    let mut sparse_b_total = 0.0f64;
    for i in 0..3 {
        let (snap_a, dt_a) =
            multikernel::benchmark_sparse_snapshot(&value_table, &io.read_set_lp_a);
        let (snap_b, dt_b) =
            multikernel::benchmark_sparse_snapshot(&value_table, &io.read_set_lp_b);
        let ms_a = dt_a.as_secs_f64() * 1000.0;
        let ms_b = dt_b.as_secs_f64() * 1000.0;
        sparse_a_total += ms_a;
        sparse_b_total += ms_b;
        eprintln!(
            "[PDES-IO] sparse snapshot iter {}: LP-A {:.2}ms ({} cells), LP-B {:.2}ms ({} cells)",
            i,
            ms_a,
            snap_a.len(),
            ms_b,
            snap_b.len()
        );
        drop(snap_a);
        drop(snap_b);
    }
    let avg_a = sparse_a_total / 3.0;
    let avg_b = sparse_b_total / 3.0;
    eprintln!(
        "[PDES-IO] sparse snapshot avg: LP-A {:.2}ms, LP-B {:.2}ms, TOTAL {:.2}ms per tick",
        avg_a,
        avg_b,
        avg_a + avg_b
    );
    eprintln!(
        "[PDES-IO] sparse-vs-full speedup: {:.0}× (full {:.1}ms → sparse {:.2}ms)",
        (snap_total_ms / 3.0) / (avg_a + avg_b),
        snap_total_ms / 3.0,
        avg_a + avg_b
    );
    drop(value_table);

    // Data Dependency Graph analysis — block-level adjacency over the
    // parallel-eligible compiled blocks. SCCs identify must-co-locate
    // sets; critical path gives the lower bound on serial execution.
    let ddg_start = std::time::Instant::now();
    let ddg = multikernel::compute_ddg(&sim);
    let ddg_ms = ddg_start.elapsed().as_secs_f64() * 1000.0;
    eprintln!("[PHASE] DDG analysis: {:.1}ms", ddg_ms);
    eprintln!(
        "[PDES-DDG] {} blocks total, {} parallel-eligible, {} independent (no in/out edges)",
        ddg.blocks_total, ddg.blocks_parallel, ddg.independent_blocks
    );
    eprintln!(
        "[PDES-DDG] dependency edges: {} total, avg-in={:.2}, max-in={}, max-out={}",
        ddg.deps_total, ddg.avg_in_degree, ddg.max_in_degree, ddg.max_out_degree
    );
    eprintln!(
        "[PDES-DDG] SCCs: {} total, {} singletons, {} non-trivial (size>1), max size = {}",
        ddg.sccs_total, ddg.sccs_singleton, ddg.sccs_nontrivial, ddg.max_scc_size
    );
    eprintln!(
        "[PDES-DDG] critical path: {} blocks across {} SCCs (lower bound on serial work; PDES speedup ≤ blocks_parallel / critical_path)",
        ddg.critical_path_blocks, ddg.critical_path_sccs
    );
    eprintln!(
        "[PDES-DDG] theoretical PDES speedup ceiling: {:.2}× (parallel_blocks / critical_path_blocks)",
        if ddg.critical_path_blocks > 0 {
            ddg.blocks_parallel as f64 / ddg.critical_path_blocks as f64
        } else {
            0.0
        }
    );

    // PDES Phase 2: per-LP local signal tables. Allocates a sparse
    // Vec<Value> per LP sized to its read+write set. Reports memory
    // footprint vs the full Value table. The pdes_c910_stub_multi
    // flow doesn't normally apply the partition (that's the simulate
    // flow's job), so apply it here for Phase 2's sake — measurement
    // mode only.
    if sim.edge_block_partition_count == 0 {
        sim.apply_multikernel_scope_partition(lp_a_prefix);
    }
    let plp_start = std::time::Instant::now();
    let per_lp_tables = sim.build_per_lp_signal_tables(2);
    let plp_ms = plp_start.elapsed().as_secs_f64() * 1000.0;
    let total_lp_bytes: usize = per_lp_tables.iter().map(|t| t.estimated_bytes()).sum();
    let full_table_bytes = sim.signal_table_len() * std::mem::size_of::<xezim_core::Value>();
    eprintln!("[PHASE] Phase 2 per-LP signal_table build: {:.1}ms", plp_ms);
    for t in &per_lp_tables {
        eprintln!(
            "[PDES-PerLP] LP-{}: {} signals, {:.2} MB local table (read+write set)",
            t.lp,
            t.len(),
            t.estimated_bytes() as f64 / 1_048_576.0
        );
    }
    eprintln!(
        "[PDES-PerLP] total per-LP memory: {:.2} MB vs full Value table {:.2} MB → {:.0}× smaller",
        total_lp_bytes as f64 / 1_048_576.0,
        full_table_bytes as f64 / 1_048_576.0,
        if total_lp_bytes > 0 {
            full_table_bytes as f64 / total_lp_bytes as f64
        } else {
            0.0
        }
    );

    // SendExecContext extract benchmark: measures the wall+memory cost
    // of cloning the Send-safe simulator subset for cross-thread sharing
    // at c910 scale. The parallel-threads PDES path needs this clone
    // before spawning worker threads.
    let ctx_start = std::time::Instant::now();
    let ctx = sim.extract_send_exec_context();
    let ctx_ms = ctx_start.elapsed().as_secs_f64() * 1000.0;
    let ctx_bytes_blocks: usize = ctx
        .compiled_edge_blocks
        .iter()
        .filter_map(|c| c.as_ref())
        .map(|cb| cb.instructions.len() * std::mem::size_of::<crate::compiler::bytecode::Insn>())
        .sum();
    eprintln!(
        "[PDES-IO] SendExecContext extract: {:.1}ms; \
        compiled_blocks ≈ {:.1} MB ({} blocks, {} signals), \
        signal_widths {:.1} MB, signal_signed {:.1} MB, \
        id_to_name {:.1} MB ({} entries)",
        ctx_ms,
        ctx_bytes_blocks as f64 / 1_048_576.0,
        ctx.compiled_edge_blocks
            .iter()
            .filter(|c| c.is_some())
            .count(),
        ctx.signal_widths.len(),
        (ctx.signal_widths.len() * std::mem::size_of::<u32>()) as f64 / 1_048_576.0,
        (ctx.signal_signed.len() * std::mem::size_of::<bool>()) as f64 / 1_048_576.0,
        (ctx.id_to_name.len() * std::mem::size_of::<std::sync::Arc<str>>()) as f64 / 1_048_576.0,
        ctx.id_to_name.len(),
    );
    // Sample: pick the first compiled block and exec it once to prove
    // SendExecContext.pdes_exec_block works at this scale.
    let mut sample_vm: Vec<xezim_core::Value> = Vec::new();
    let sample_snapshot: Vec<xezim_core::Value> = sim.signal_table_slice().to_vec();
    let mut sample_bi = None;
    for bi in 0..ctx.block_count() {
        if ctx.block_compiled(bi) {
            sample_bi = Some(bi);
            break;
        }
    }
    if let Some(bi) = sample_bi {
        let sample_start = std::time::Instant::now();
        let writes = ctx.pdes_exec_block(bi, &sample_snapshot, &mut sample_vm);
        let sample_us = sample_start.elapsed().as_secs_f64() * 1_000_000.0;
        eprintln!(
            "[PDES-IO] SendExecContext.pdes_exec_block(bi={}) sample: {:.1}µs, {} NBA writes",
            bi,
            sample_us,
            writes.len()
        );
    }
    drop(ctx);

    let stub_start = std::time::Instant::now();
    // Use the channels-wired variant: builds real BoundaryChannel
    // objects covering the 109 boundary signals (on hello), so the
    // coordinator exercises the full channel topology end-to-end.
    let (specs, fire_a, fire_b) =
        multikernel::build_c910_stub_specs_with_channels(&sim, &io, lp_a_prefix, n_ticks, 10);
    eprintln!(
        "[PHASE] PDES stub-specs (with channels): {:.1}ms",
        stub_start.elapsed().as_secs_f64() * 1000.0
    );

    // Use a tiny signal_table — stub blocks don't actually read signals.
    // A real integration would allocate sim.signal_table_len() and
    // snapshot only its boundary slice; for the stub, len=2 suffices.
    let coord = multikernel::PdesCoordinator::new(2, specs);

    let run_start = std::time::Instant::now();
    let stats = coord.run();
    let run_ms = run_start.elapsed().as_secs_f64() * 1000.0;
    eprintln!(
        "[PHASE] PDES coordinator run: {:.1}ms ({} ticks × {} kernels)",
        run_ms,
        n_ticks,
        stats.len()
    );

    for (i, s) in stats.iter().enumerate() {
        eprintln!(
            "[PDES-stub] kernel {}: ticks={}, blocks_fired={}, final_time={}",
            i, s.ticks, s.blocks_fired, s.final_time
        );
    }
    eprintln!(
        "[PDES-stub] fire-counter LP-A={}, LP-B={} (sanity: should equal blocks×ticks per LP)",
        fire_a.load(std::sync::atomic::Ordering::Relaxed),
        fire_b.load(std::sync::atomic::Ordering::Relaxed),
    );

    // REAL bytecode through PDES at c910 scale — replaces stub closures
    // with actual `pdes_exec_block` calls. Sequential driving (LP-A
    // then LP-B per tick); produces real NBA writes. Does NOT model
    // comb settle or testbench I/O — measures dispatch throughput.
    let real_n_ticks = n_ticks.min(100); // limit so the demo finishes
    eprintln!("[PDES-real] === REAL BYTECODE through PDES dispatcher at c910 scale ===");
    let (real_fires, real_nbas, real_ms, real_per_tick_us) =
        multikernel::run_c910_real_bytecode(&sim, lp_a_prefix, real_n_ticks);
    eprintln!(
        "[PDES-real] {} ticks: {} block invocations, {} NBA writes produced",
        real_n_ticks, real_fires, real_nbas
    );
    eprintln!(
        "[PDES-real] wall: {:.1}ms total, {:.1}µs avg per tick",
        real_ms, real_per_tick_us
    );
    eprintln!(
        "[PDES-real] block invocation throughput: {:.2} M/sec (real SV bytecode)",
        real_fires as f64 / (real_ms / 1000.0) / 1_000_000.0
    );

    eprintln!(
        "[PHASE] total: {:.1}ms",
        total_start.elapsed().as_secs_f64() * 1000.0
    );
    Ok(())
}
