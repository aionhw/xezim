//! Registry of every `XEZIM_*` environment variable the simulator (and
//! xezim-core elaboration) reads, printed by `--show-env-avail`.
//!
//! Keep this table in sync with the source: `tests/misc/env_var_registry.rs`
//! scans both crates for `"XEZIM_..."` literals and fails if a variable is
//! read but not listed here (or listed but no longer read).

/// (name, one-line description). Sorted alphabetically.
pub static ENV_VARS: &[(&str, &str)] = &[
    (
        "XEZIM_A1_DBG",
        "Debug: trace always@(posedge) single-block compilation decisions",
    ),
    (
        "XEZIM_AB_DBG",
        "Debug: trace always-block classification during elaboration",
    ),
    (
        "XEZIM_ACTIVE_REGION",
        "Scheduling: force active-region semantics for edge continuations (0/1)",
    ),
    (
        "XEZIM_ALLOW_IMPLICIT_STATIC",
        "Elab: accept static-lifetime locals with initializers without warning",
    ),
    (
        "XEZIM_AOT",
        "AOT: compile eligible blocks to native code via generated Rust + rustc (needs --features jit build)",
    ),
    (
        "XEZIM_AOT_OPT",
        "AOT: rustc opt-level for the generated crate (0-3, default 2)",
    ),
    (
        "XEZIM_AOT_TEMPLATE",
        "AOT: dedup structurally identical blocks into shared template bodies (1 enables)",
    ),
    (
        "XEZIM_ARMED_EDGE",
        "Edge engine: force ARMED edge-detection mode on/off (0/1)",
    ),
    (
        "XEZIM_ARMED_EDGE_SHADOW",
        "Edge engine: run ARMED mode in shadow-compare mode against the scan path",
    ),
    (
        "XEZIM_ARM_CENSUS",
        "Event-edge engine: report which signals arm the most edge blocks on write",
    ),
    (
        "XEZIM_ARRAY_SOA_SHADOW",
        "Arrays: shadow-verify structure-of-arrays storage against per-element Values",
    ),
    ("XEZIM_AW_DBG", "Debug: trace assignment-width inference"),
    (
        "XEZIM_BC_DUMP",
        "Dump compiled bytecode blocks after compilation",
    ),
    (
        "XEZIM_BC_DUMP_FULL",
        "Dump compiled bytecode including full insn operands",
    ),
    (
        "XEZIM_BD_DBG",
        "Debug: trace block-local declaration binding",
    ),
    (
        "XEZIM_BIT_SENS",
        "Scheduling: bit-granular comb sensitivity (a write re-runs only readers of the changed bits); 0 disables",
    ),
    (
        "XEZIM_BM_CENSUS",
        "Census: print every builtin-method call as `[bm] <receiver> <method> this=<bool>` (aggregate with sort | uniq -c)",
    ),
    (
        "XEZIM_BSP_PAR",
        "Parallel settle: enable bulk-synchronous parallel comb settle (0/1)",
    ),
    (
        "XEZIM_BSP_PAR_THRESHOLD",
        "Parallel settle: min comb entries per level before threads are used",
    ),
    (
        "XEZIM_BSP_SETTLE",
        "Parallel settle: enable levelized BSP settle algorithm (0/1)",
    ),
    (
        "XEZIM_BSP_SHADOW",
        "Parallel settle: shadow-compare BSP settle against the canonical loop",
    ),
    (
        "XEZIM_BSP_WIDTHS",
        "Parallel settle: print per-level width/occupancy statistics",
    ),
    (
        "XEZIM_BUF_CENSUS",
        "Census: count whole-net identity continuous assigns (buffer-net collapse experiment)",
    ),
    (
        "XEZIM_BUF_COLLAPSE",
        "Default on; =0 disables: collapse whole-net identity continuous assigns (assign y = x) onto their source net (drops one delta step per buffer); force/release targets, 2-state/4-state mismatches, SDF and DPI/VPI designs are left alone",
    ),
    (
        "XEZIM_CACHE_COMPRESSION_LEVEL",
        "Elab cache: zstd compression level (default 3)",
    ),
    (
        "XEZIM_CACHE_DIR",
        "Elab cache: override cache directory (default XDG cache)",
    ),
    (
        "XEZIM_CACHE_FIT",
        "Elab cache: print cache-fit decisions per design",
    ),
    (
        "XEZIM_CACHE_STATS",
        "Elab cache: print hit/miss/size statistics",
    ),
    (
        "XEZIM_CASCADE_LIMIT",
        "Scheduling: max same-time edge cascade rounds before the runaway guard trips",
    ),
    (
        "XEZIM_CASEJUMP_LIMIT",
        "CaseJump: max table size for jump-table lowering of case statements",
    ),
    (
        "XEZIM_CASEJUMP_OFF",
        "CaseJump: disable case jump-table dispatch (fall back to chained compares)",
    ),
    (
        "XEZIM_CASEJUMP_TRACE",
        "CaseJump: trace which case statements lower to jump tables",
    ),
    (
        "XEZIM_CAST_DBG",
        "Debug: trace $cast / type-cast evaluation",
    ),
    (
        "XEZIM_CHAIN_CENSUS",
        "Phase 2: size the single-fanout chain clustering opportunity (count, lengths, dynamic coverage)",
    ),
    (
        "XEZIM_CLKTREE_PROBE",
        "Clock tree: print the detected clock-tree roots and gating structure",
    ),
    (
        "XEZIM_CLKTREE_STATS",
        "Clock tree: print dedup statistics for derived clock signals",
    ),
    (
        "XEZIM_CLOCK_TREE",
        "Cycle engine: set to 0 to disable the eager clock-tree pass",
    ),
    (
        "XEZIM_CLOCK_TREE_STATS",
        "Cycle engine: report the clock-tree conversion (roots, entries, unconverted derived clocks)",
    ),
    (
        "XEZIM_CLOCK_TREE_TRACE",
        "Cycle engine: trace the tree walk for the named derived clocks (comma-separated)",
    ),
    (
        "XEZIM_CLUSTER_CENSUS",
        "Census: form comb clusters and report how many written nets could stay local (native-clustering gate)",
    ),
    (
        "XEZIM_COACT_CENSUS",
        "Phase 2: per-dep-edge same-slot co-activation stats; sizes convex-cluster fusion (wants profiling on)",
    ),
    (
        "XEZIM_COACT_FILE",
        "Phase 2: coact census writes per-edge ratios here; region fusion reads it to gate merging",
    ),
    (
        "XEZIM_COACT_MIN",
        "Phase 2: minimum same-slot ratio for a fusion edge (default 0.9)",
    ),
    (
        "XEZIM_CODE_COVERAGE",
        "Coverage: collect code coverage, like --code-coverage=<kinds> (stmt,branch,toggle,all); the flag wins",
    ),
    (
        "XEZIM_COMB_GRAPH",
        "Profiling: write the comb operand graph (entry read/write ids, edge-block write ids) to the given path",
    ),
    (
        "XEZIM_COMB_OPS",
        "Profiling: with COMB_PATHS, emit the per-block opcode SET of each interp-bound entry (two-state closure analysis)",
    ),
    (
        "XEZIM_COMB_PATHS",
        "Profiling: count settle path (two-state/JIT/interp) per comb entry; ranked dump at end",
    ),
    (
        "XEZIM_COMB_PATHS_BY",
        "Profiling: `insns` ranks the COMB_PATHS dump by interpreted VM instructions instead of evaluations",
    ),
    (
        "XEZIM_COMB_PATHS_DUMP",
        "Profiling: print the full instruction list of the top N ranked COMB_PATHS entries",
    ),
    (
        "XEZIM_COMB_PATHS_TOP",
        "Profiling: how many ranked interp-bound entries the COMB_PATHS histograms cover (0 = all, default 40)",
    ),
    (
        "XEZIM_COMMIT_PLAN",
        "Signal commits: enable experimental per-destination sidecar specialization (1 enables)",
    ),
    (
        "XEZIM_COMPILE_FAIL_STATS",
        "Bytecode: print why statements failed to compile (bail census)",
    ),
    (
        "XEZIM_COMPILE_METHODS",
        "class-perf: compile hot class-function method bodies to bytecode and execute them (0 disables, default on; see XEZIM_METHOD_TIER)",
    ),
    (
        "XEZIM_COMPILE_PHASES",
        "Print per-phase compilation timing breakdown",
    ),
    (
        "XEZIM_CONE",
        "Debug: dump the fan-in cone of a named signal",
    ),
    (
        "XEZIM_CONST_CENSUS",
        "Census: size the constant-driven net population and the reader load sites it would fold",
    ),
    (
        "XEZIM_COV_DB",
        "Coverage: override the coverage database output path",
    ),
    (
        "XEZIM_CP_DBG",
        "Debug: trace constant propagation in elaboration",
    ),
    (
        "XEZIM_CTL_MASK",
        "Scheduling: an AND/OR gate whose other input holds the controlling value skips its high-fanout (clock) input; 0 disables",
    ),
    (
        "XEZIM_CYCLE_CENSUS",
        "Census: comb entries in combinational loops (SCCs) / self-loops and edge blocks on derived clocks — the exclusion set for single-pass cycle evaluation (=2 samples the largest SCCs)",
    ),
    (
        "XEZIM_CYCLE_MODE",
        "Engine: `event` (default, the event-driven engine) or `cycle` (cycle-based stepping after reset with event-driven fallback for ticks that need delta cycles; phase 0 = eager clock-tree pass)",
    ),
    (
        "XEZIM_DBG_ARR",
        "Elab debug: trace array dimension/type resolution",
    ),
    (
        "XEZIM_DBG_PARAM",
        "Elab debug: trace parameter overrides and resolution",
    ),
    (
        "XEZIM_DEAD_CENSUS",
        "Census: upper bound on comb entries whose outputs nothing observable reads (hanging logic)",
    ),
    ("XEZIM_DEBUG", "General verbose debug output"),
    (
        "XEZIM_DEBUG_NAMED_ARRAYS",
        "Debug: trace named-array signal-table binding",
    ),
    (
        "XEZIM_DEP_STATS",
        "Settle: print comb dependency-graph statistics",
    ),
    (
        "XEZIM_DIAG_LIMIT",
        "Diagnostics: per-kind cap for elaboration warnings (0 = unlimited)",
    ),
    (
        "XEZIM_DIRTY_EDGE",
        "Edge engine: dirty-driven edge scan instead of the full per-tick scan (0/1, default 1). Worth -5.9% host instructions on the C906 CoreMark benchmark. Shadow-validated with zero missed edges on C906 CoreMark, ibex simple_system, the C910 SoC, three UVM testbenches and the regression suite (see XEZIM_DIRTY_EDGE_SHADOW). Set to 0 to restore the full per-tick scan",
    ),
    (
        "XEZIM_DIRTY_EDGE_SHADOW",
        "Edge engine: shadow-compare dirty-driven scan vs full scan",
    ),
    (
        "XEZIM_DISPATCHER",
        "Select process dispatcher implementation (debug)",
    ),
    (
        "XEZIM_DUMP_CA_READS",
        "Dump continuous-assign read sets after elaboration",
    ),
    (
        "XEZIM_DUMP_COMB_SENS",
        "Dump computed sensitivity of comb entries",
    ),
    (
        "XEZIM_DUMP_EDGE_SENS",
        "Dump computed edge-block sensitivities",
    ),
    ("XEZIM_DUMP_ENTRY", "Dump one comb entry by index (debug)"),
    ("XEZIM_DUMP_FULL", "Dump full elaborated design tables"),
    (
        "XEZIM_DUMP_INLINE",
        "Dump inlining decisions for tasks/functions",
    ),
    (
        "XEZIM_DUMP_UNRESOLVED",
        "Dump unresolved identifiers after elaboration",
    ),
    (
        "XEZIM_EAGER_PROC_SETTLE",
        "Scheduling: settle combinationals eagerly inside processes (0/1)",
    ),
    ("XEZIM_EC_DBG", "Debug: trace event-control compilation"),
    (
        "XEZIM_EDGE_BLOCK_STATS",
        "Print per-edge-block execution statistics",
    ),
    (
        "XEZIM_EDGE_FIRE_TRACE",
        "Trace every edge-block firing (very verbose)",
    ),
    (
        "XEZIM_EDGE_FIRE_WATCH",
        "Watch a named signal's edge firings",
    ),
    (
        "XEZIM_EDGE_MERGE",
        "Merge same-sensitivity edge blocks into one compiled block (default 8; =0 disables; =census reports; =N sets the chunk cap)",
    ),
    ("XEZIM_EDGE_SCAN_STATS", "Print edge-scan cost statistics"),
    (
        "XEZIM_ELAB_STATS",
        "Elab: print elaboration statistics (instances, signals, timing)",
    ),
    (
        "XEZIM_ELIDE_STRICT",
        "Debug: =1 panics when a name lookup reaches an input-port net left out by unobserved-port elision (default: one warning)",
    ),
    (
        "XEZIM_ENABLE_CACHE",
        "Elab cache: force-enable the design cache (overrides --no-cache heuristics)",
    ),
    (
        "XEZIM_EVALS_TOP",
        "Profiling: how many entries the COMB_PATHS evaluation census lists (default 40)",
    ),
    (
        "XEZIM_EVENT_AFTER",
        "Debug: log event-queue state after each time step",
    ),
    ("XEZIM_EVENT_EDGE", "Event-edge engine: force on/off (0/1)"),
    (
        "XEZIM_EVENT_EDGE_CENSUS",
        "Event-edge engine: report why edge blocks are not gateable and how many gate through array-element arming",
    ),
    (
        "XEZIM_EVENT_EDGE_HEAL",
        "Event-edge engine: re-arm/heal mode for missed gateable flops",
    ),
    (
        "XEZIM_EVENT_EDGE_MEASURE",
        "Event-edge engine: measure gateable-flop skip effectiveness",
    ),
    (
        "XEZIM_EV_DBG",
        "Debug: trace named-event trigger/wait matching",
    ),
    (
        "XEZIM_EXIT_AFTER_COMPILE",
        "Profiling: exit right after elaboration and compile, before simulation time 0",
    ),
    (
        "XEZIM_FALLBACK_SITES",
        "Report each construct handed to the AST interpreter: reason, source byte span, scope",
    ),
    (
        "XEZIM_FAST_CALLS",
        "Compiled methods: set to 0 to disable direct VM-to-VM dispatch of compiled calls",
    ),
    (
        "XEZIM_FOLD_CONST_REGS",
        "Bytecode: set to 0 to disable register constant propagation (constant add chains, static bit indexes)",
    ),
    (
        "XEZIM_FOLD_CONST_STATS",
        "Bytecode: print per-block register constant-folding counts",
    ),
    (
        "XEZIM_FOLD_STORES",
        "Bytecode: bit mask of constant-index store folds (1 dyn range, 2 element range, 4 element whole; default 1)",
    ),
    (
        "XEZIM_FORCE_PARALLEL",
        "Parallel: force parallel dispatch even below thresholds",
    ),
    (
        "XEZIM_FOREACH_REPLAY_LIMIT",
        "Bytecode: max foreach unroll replay count",
    ),
    (
        "XEZIM_FUSE",
        "Bytecode: enable/disable insn fusion peepholes (0/1)",
    ),
    (
        "XEZIM_FUSE_ADDC2",
        "Bytecode: opt-in AddC2 superinstruction formation (1 enables)",
    ),
    (
        "XEZIM_FUSE_ARRNBA",
        "Bytecode: enable LoadSignal;LoadArrayElem;NbaAssign fusion (0/1)",
    ),
    (
        "XEZIM_FUSE_CHAINS",
        "Settle: opt-in (1) merge of single-reader comb chains into one entry; neutral on ibex, for chain-heavy designs",
    ),
    (
        "XEZIM_FUSE_CONST",
        "Bytecode: enable constant-operand fusion (0/1)",
    ),
    (
        "XEZIM_FUSE_COPYFWD",
        "Bytecode: enable forwarding of register copies into their readers (0/1)",
    ),
    (
        "XEZIM_FUSE_MOVEFWD",
        "Bytecode: enable Move-into-assign forwarding (0/1)",
    ),
    (
        "XEZIM_FUSE_SCRUBS",
        "Bytecode: enable provably-unsigned sign-scrub elision (0/1)",
    ),
    (
        "XEZIM_GIT_DATE",
        "Build metadata: git date baked into --version (build-time)",
    ),
    (
        "XEZIM_GIT_HASH",
        "Build metadata: git hash baked into --version (build-time)",
    ),
    (
        "XEZIM_GIT_TAG",
        "Build metadata: nearest release tag baked into -V (build-time)",
    ),
    (
        "XEZIM_HOT_STATS",
        "Print hottest comb entries / edge blocks at end of sim",
    ),
    (
        "XEZIM_HUGEPAGE",
        "Memory: back the signal table with huge pages (0/1)",
    ),
    (
        "XEZIM_HUGEPAGE_STATS",
        "Memory: print huge-page allocation statistics",
    ),
    (
        "XEZIM_INIT_MEM",
        "Init: initial value policy for memories (x/0/rand)",
    ),
    (
        "XEZIM_INIT_REG",
        "Init: initial value policy for registers (x/0/rand)",
    ),
    ("XEZIM_INIT_ZERO", "Init: start all state at 0 instead of x"),
    (
        "XEZIM_INIT_ZERO_PATHS",
        "Init: comma-separated hierarchical prefixes to zero-initialize",
    ),
    (
        "XEZIM_INJECT_PREFETCH",
        "Settle: prefetch the next injected entry's two-state header one pop ahead (default 1; 0 disables)",
    ),
    (
        "XEZIM_INLINE_BITS",
        "Inline-bits signal mirror: 1 forces it on, 0 off; default on only with XEZIM_JIT",
    ),
    (
        "XEZIM_INST_PROF",
        "Elab: profile per-instance elaboration cost",
    ),
    (
        "XEZIM_ISLAND_CENSUS",
        "Island phase 1: report per-clock-domain size, comb cone, boundary and observability",
    ),
    (
        "XEZIM_JIT",
        "JIT: enable native compilation of bytecode blocks (needs --features jit build)",
    ),
    ("XEZIM_JIT_BACKEND", "JIT: select backend (cranelift)"),
    (
        "XEZIM_JIT_BAIL_TRACE",
        "JIT: print pc+opcode when native codegen aborts a block mid-emit",
    ),
    (
        "XEZIM_JIT_CLIF",
        "JIT: dump cranelift IR for compiled blocks",
    ),
    (
        "XEZIM_JIT_COMB_RANGE",
        "JIT: natively compile only comb entries in [lo,hi) (bisection)",
    ),
    (
        "XEZIM_JIT_DENY",
        "JIT: comma-separated opcode names to exclude from native compilation (bisection)",
    ),
    (
        "XEZIM_JIT_DUMP_BLOCKS",
        "JIT: dump the insn stream of each compiled block",
    ),
    (
        "XEZIM_JIT_ONLY",
        "JIT: run only JIT'd blocks (bail-audit mode)",
    ),
    (
        "XEZIM_JIT_SKIP_IDX",
        "JIT: skip compiling block index N (bisection debug)",
    ),
    (
        "XEZIM_JIT_SKIP_XZ",
        "JIT: skip the X/Z runtime guard (unsafe, debug)",
    ),
    (
        "XEZIM_JIT_VERBOSE",
        "JIT: print compilation coverage summary",
    ),
    ("XEZIM_JIT_XZ_BAIL", "JIT: trace X/Z-guard bailouts"),
    (
        "XEZIM_KEEP_PORTS",
        "Elab: =1 keeps the nets of substituted input ports that nothing can reach by name (turns unobserved-port elision off)",
    ),
    (
        "XEZIM_LARGE_ARRAY_NAME_THRESHOLD",
        "Elab: element-count threshold above which per-element names are lazy",
    ),
    ("XEZIM_LAYOUT", "Print signal-table memory layout summary"),
    (
        "XEZIM_LAZY_ALWAYS",
        "Default on; =0 disables: keep instantiated `@(...)` always blocks as shared source plus instance context until they compile, instead of one materialized tree each",
    ),
    (
        "XEZIM_LAZY_PROC_SETTLE",
        "Scheduling: defer comb settle across process boundaries (0/1)",
    ),
    (
        "XEZIM_LOOP_LIMIT",
        "Runtime guard: max iterations for a zero-delay procedural loop",
    ),
    ("XEZIM_MAX_INST_DEPTH", "Elab: max instance recursion depth"),
    (
        "XEZIM_MBX_DBG",
        "Debug: trace mailbox get/put/peek operations",
    ),
    (
        "XEZIM_MEM_CENSUS",
        "Memory: per-array cell census sizing the packed word-storage opportunity",
    ),
    (
        "XEZIM_METHOD_CACHE",
        "class-perf: persistent compiled-method cache dir (1 = default location, unset = off)",
    ),
    (
        "XEZIM_METHOD_PROFILE",
        "class-perf: sample class-method execution (in-process sampling profiler)",
    ),
    (
        "XEZIM_METHOD_TIER",
        "class-perf: compile a class method only after N calls (0 = first call)",
    ),
    (
        "XEZIM_NAME_STATS",
        "Count string-keyed name lookups per site (UVM runtime-ID investigation)",
    ),
    (
        "XEZIM_NBA_DENSE",
        "NBA queue: use dense per-signal slots instead of hash map (0/1)",
    ),
    (
        "XEZIM_NO_CACHE",
        "Elab cache: disable reading/writing the design cache",
    ),
    (
        "XEZIM_NO_CLKTREE_DEDUP",
        "Clock tree: disable derived-clock deduplication",
    ),
    (
        "XEZIM_NO_DYN_RENAME",
        "Disable dynamic renaming of user identifiers colliding with internals",
    ),
    (
        "XEZIM_NO_LAZY_PREFIX",
        "Elab: disable lazy hierarchical-prefix interning",
    ),
    (
        "XEZIM_NO_MEM_WATCHDOG",
        "Disable the OOM memory watchdog thread",
    ),
    (
        "XEZIM_NO_NATIVE_CACHE",
        "AOT: disable the persistent native-library cache (~/.cache/xezim/native)",
    ),
    (
        "XEZIM_NO_PARALLEL",
        "Parallel: disable all multithreaded execution",
    ),
    (
        "XEZIM_NO_PARALLEL_RANGE",
        "Parallel: disable parallel range-partitioned apply",
    ),
    (
        "XEZIM_OOB_SELECT",
        "Semantics: `whole` makes a partially out-of-range part-select read x as a whole (some tools); default follows IEEE 1800 11.5.1, where only the out-of-range bits read x",
    ),
    (
        "XEZIM_OPCODE_CENSUS",
        "Print executed-insn opcode histogram at end of sim",
    ),
    (
        "XEZIM_PACKED_MEM",
        "Memory: packed arena storage for large integral arrays (on; 0 turns it off)",
    ),
    (
        "XEZIM_PARALLEL_SERIALIZE",
        "Parallel: run parallel plan serially (debug determinism check)",
    ),
    (
        "XEZIM_PARTITION_BY_CLOCK",
        "PDES: partition design by clock domain (0/1)",
    ),
    (
        "XEZIM_PARTITION_BY_CLOCK_MAX_K",
        "PDES: max clock-domain partitions",
    ),
    (
        "XEZIM_PARTITION_SCOPES",
        "PDES: comma-separated scope prefixes to partition by",
    ),
    (
        "XEZIM_PDES_BUCKET_NBA",
        "PDES: bucket NBA commits per partition",
    ),
    (
        "XEZIM_PDES_CHK_KINDS",
        "PDES: verify partition kind assignment (debug)",
    ),
    (
        "XEZIM_PDES_CHUNK_TARGET",
        "PDES: target entries per work chunk",
    ),
    (
        "XEZIM_PDES_DUMP_SIG",
        "PDES: dump cross-partition signal traffic",
    ),
    ("XEZIM_PDES_K", "PDES: number of logical partitions"),
    (
        "XEZIM_PDES_PAR_APPLY",
        "PDES: apply partition results in parallel (0/1)",
    ),
    (
        "XEZIM_PDES_PHASE4_DRYRUN",
        "PDES: run partition phase 4 in dry-run compare mode",
    ),
    ("XEZIM_PDES_POOL", "PDES: worker thread-pool size"),
    (
        "XEZIM_PDES_SCAN_MERGE",
        "PDES: merge partition edge scans (0/1)",
    ),
    (
        "XEZIM_PDES_SUBCHUNK",
        "PDES: sub-chunk granularity for load balancing",
    ),
    (
        "XEZIM_PDES_TICKS",
        "PDES: print per-tick partition statistics",
    ),
    (
        "XEZIM_PEND_DBG",
        "Elab debug: trace pending-module/library resolution",
    ),
    ("XEZIM_PERLP_AFTER", "Per-LP settle: activate after N ticks"),
    (
        "XEZIM_PERLP_SETTLE",
        "Per-LP settle: enable per-logical-partition settle state (0/1)",
    ),
    (
        "XEZIM_PERLP_SHADOW",
        "Per-LP settle: shadow-compare against canonical settle",
    ),
    (
        "XEZIM_PLACE_DBG",
        "Layout: print the placement pre-pass's name-graph coverage (names linked, edges)",
    ),
    (
        "XEZIM_PLACE_SIGNALS",
        "Layout: 1 orders scalar signal ids by a source-order comb walk (opt-in; measured negative on c906, kept for placement experiments)",
    ),
    (
        "XEZIM_PREFETCH_DIST",
        "Settle worklist: entry-array prefetch distance (default 8, 0 disables)",
    ),
    (
        "XEZIM_PREFETCH_MODE",
        "Settle: worklist prefetch target — 0 entry table (default), 1 two-state header, 2 header + stream",
    ),
    (
        "XEZIM_PROBE_IDENT",
        "Probe: log every resolution of a named identifier",
    ),
    (
        "XEZIM_PROBE_INLINE",
        "Probe: log task/function inlining attempts for a name",
    ),
    (
        "XEZIM_PROBE_SYSCALL",
        "Probe: log system-call dispatches by name",
    ),
    (
        "XEZIM_PROC_COND_VERIFY",
        "Audit: compare compiled process-if conditions against AST evaluation",
    ),
    (
        "XEZIM_PROC_FSM",
        "Processes: compile blocking always bodies into bytecode FSMs with wait insns (1 enables)",
    ),
    (
        "XEZIM_PROC_FSM_TS",
        "Processes: 0 keeps compiled process FSMs on the four-state VM instead of the two-state executor",
    ),
    (
        "XEZIM_PROC_LOOP_STATS",
        "Log process For-loop bytecode compiles and bail reasons",
    ),
    (
        "XEZIM_PROFILE_REPORT",
        "Profiling: print the [PROF] end-of-run report",
    ),
    (
        "XEZIM_PROFILE_TIMING",
        "Profiling: collect per-phase nanosecond timers (adds overhead)",
    ),
    (
        "XEZIM_PROGRESS",
        "Print periodic [PROGRESS] lines (wall, sim time, deltas)",
    ),
    ("XEZIM_PSDBG", "Debug: trace process scheduling decisions"),
    (
        "XEZIM_PSETTLE_STATS",
        "Print process-triggered settle statistics",
    ),
    (
        "XEZIM_PURITY_STATS",
        "Print function-purity classification statistics",
    ),
    (
        "XEZIM_RAND_DBG",
        "Debug: trace randomize()/constraint solving",
    ),
    (
        "XEZIM_RAND_DIAG",
        "Debug: on a failed randomize(), report the switched-off variables and constraints and the unsatisfied constraint items",
    ),
    (
        "XEZIM_RANGE_COPY",
        "Comb settle: lower direct constant-range assignments to slice copies (1 enables)",
    ),
    (
        "XEZIM_REF_DBG",
        "Debug: trace class-handle reference operations",
    ),
    (
        "XEZIM_REGIONS",
        "Comb settle: fuse dependency-connected compiled comb entries into region blocks (1 enables)",
    ),
    (
        "XEZIM_REGION_GATES",
        "Comb settle: also fuse 1-bit gate primitives into GateRegions (opt-in; measured net-negative on clock-tree-heavy designs)",
    ),
    (
        "XEZIM_REGION_MAX",
        "Comb settle: max members per fused region (default 8)",
    ),
    (
        "XEZIM_REGION_STATS",
        "Comb settle: print region-fusion statistics at compile time",
    ),
    ("XEZIM_REG_DBG", "Debug: trace VM register allocation"),
    (
        "XEZIM_REPORT_STATS",
        "Print summary statistics at end of run",
    ),
    (
        "XEZIM_RESIZE_ELIDE",
        "Values: elide redundant resize operations (0/1)",
    ),
    (
        "XEZIM_RSS_TRACE",
        "Memory: print resident/peak RSS at each pipeline and compile phase",
    ),
    ("XEZIM_RS_STATS", "Print resize/copy statistics for Values"),
    (
        "XEZIM_SETTLE_LEVELS",
        "Settle: print levelization depth of the comb graph",
    ),
    (
        "XEZIM_SPLIT_CENSUS",
        "Census: size the block-splitting opportunity (multi-statement comb blocks with disjoint statement read sets)",
    ),
    ("XEZIM_STACK_MB", "Main simulation thread stack size in MB"),
    (
        "XEZIM_STALL_LIMIT",
        "Stall detector: process activations without progress before aborting",
    ),
    ("XEZIM_STUCK_CLOCK", "Stuck-clock detector: enable (0/1)"),
    (
        "XEZIM_STUCK_CLOCK_EDGES",
        "Stuck-clock detector: min edges expected",
    ),
    (
        "XEZIM_STUCK_CLOCK_TICKS",
        "Stuck-clock detector: tick window",
    ),
    (
        "XEZIM_STUCK_CLOCK_WALL",
        "Stuck-clock detector: wall-clock window (seconds)",
    ),
    (
        "XEZIM_TEMPLATE_CENSUS",
        "Profiling: group compiled blocks by canonical shape; report repeated-template dynamic coverage (wants COMB_PATHS + EDGE_BLOCK_STATS)",
    ),
    (
        "XEZIM_TOPO_JOIN",
        "Default on; =0 disables: order a comb net with a large writers x readers product through one join node instead of W x R edges",
    ),
    (
        "XEZIM_TRACE_ALWAYS",
        "Trace always-block executions (very verbose)",
    ),
    ("XEZIM_TRACE_ELAB", "Elab: trace module elaboration order"),
    (
        "XEZIM_TRACE_FINISH",
        "Trace $finish / simulation-end plumbing",
    ),
    ("XEZIM_TRACE_INIT", "Elab: trace initial-value assignment"),
    (
        "XEZIM_TRACE_PARAM",
        "Elab: trace parameter value propagation",
    ),
    (
        "XEZIM_TRACE_SCHED",
        "Trace scheduler queue operations (very verbose)",
    ),
    (
        "XEZIM_TRACE_SIGNAL",
        "Trace writes to matching signal names (comma-separated)",
    ),
    (
        "XEZIM_TRACE_SPIN",
        "Diagnose time-0 livelocks: drain-guard + empty-sensitivity reports",
    ),
    (
        "XEZIM_TRACE_TYPE",
        "Elab: trace typedef-width table for matching names",
    ),
    (
        "XEZIM_TS_DBG",
        "Two-state: print lowering bail opcode per block",
    ),
    (
        "XEZIM_TS_DENY",
        "Two-state: comma-separated 4-state opcode names whose blocks bail back to the interpreter (miscompile bisection)",
    ),
    (
        "XEZIM_TS_DUMP",
        "Two-state: with XEZIM_TS_DBG, print every lowered two-state stream",
    ),
    (
        "XEZIM_TS_JIT",
        "Two-state: compile straight-line streams to native code (jit feature; exploration)",
    ),
    (
        "XEZIM_TS_JIT_CLIF",
        "Two-state: print the generated CLIF for each native stream",
    ),
    (
        "XEZIM_TS_JIT_HOT",
        "Two-state native code: compile a stream after this many evaluations (default 64)",
    ),
    (
        "XEZIM_TS_JIT_RESERVE_MB",
        "Two-state native code: size of the reserved code region (default 256)",
    ),
    (
        "XEZIM_TS_WIDE512",
        "Two-state: set to 0 to keep blocks with registers wider than 128 bits on the interpreter (default on)",
    ),
    (
        "XEZIM_TS_X",
        "Two-state: set to 0 to disable the x-plane executor (blocks that read x re-run on the four-state VM instead)",
    ),
    (
        "XEZIM_TWO_STATE",
        "Two-state u64 fast path: 0 disables (default on)",
    ),
    (
        "XEZIM_UNRESOLVED_DUMP",
        "Comb settle: name the entries that re-evaluate on every settle call",
    ),
    (
        "XEZIM_UPF_DUMP",
        "UPF: print the generated power-intent package and glue processes",
    ),
    (
        "XEZIM_UVM_DIR",
        "Path to a UVM source tree auto-added for UVM designs",
    ),
    (
        "XEZIM_VALUE_TRACE",
        "Trace Value mutations for a signal id (debug)",
    ),
    ("XEZIM_VALUE_TRACE_LIMIT", "Max Value-trace reports"),
    (
        "XEZIM_VCD_FULL",
        "VCD/FST: dump all scopes regardless of $dumpvars args",
    ),
    (
        "XEZIM_VCD_PARAM_AS_WIRE",
        "VCD/FST: emit parameters as wires for viewer compatibility",
    ),
    (
        "XEZIM_VEC_CENSUS",
        "Census: size the per-bit comb-entry population coalescable into vector ops (R1)",
    ),
    (
        "XEZIM_VEC_COALESCE",
        "Comb settle: coalesce aligned bit-sliced gates into vector operations (1 enables)",
    ),
    (
        "XEZIM_VEC_SCATTER",
        "Comb settle: batch adjacent source lanes with scalar destinations (1 enables)",
    ),
    (
        "XEZIM_VEC_SCATTER_MIN",
        "Comb settle: minimum scalar-destination batch width (2..64, default 64)",
    ),
    (
        "XEZIM_VEC_STATS",
        "Comb settle: print vector-coalescing statistics",
    ),
    (
        "XEZIM_VERBOSE",
        "Same as --verbose: version banner, [PHASE] timings, end-of-run engine counters (1)",
    ),
    (
        "XEZIM_VERIFY_INLINE_BITS",
        "JIT: shadow-verify the inline-bits mirror against Values",
    ),
    (
        "XEZIM_VIRTUAL_NAME_MIN_CELLS",
        "Names: arrays of at least this many cells get virtual element names (default 257; 0 = all)",
    ),
    (
        "XEZIM_WAITERS_FIRST",
        "Scheduling: run event waiters before edge blocks (0/1)",
    ),
    (
        "XEZIM_XZ_STATS",
        "Print X/Z-bit population statistics for the signal table",
    ),
    (
        "XEZIM_X_LITERAL_TO_ZERO",
        "Elab: treat 'x literals as 0 (two-state designs)",
    ),
    ("XEZIM_X_WARN", "Enable --x-warn X-propagation warnings (1)"),
    ("XEZIM_X_WARN_LIMIT", "Cap --x-warn reports (0 = unlimited)"),
];

/// `--show-env-avail` implementation.
pub fn print_env_avail() {
    println!("Environment variables recognized by xezim (name=value):");
    println!();
    let w = ENV_VARS.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
    for (name, desc) in ENV_VARS {
        println!("  {name:w$}  {desc}");
    }
    println!();
    println!(
        "{} variables. Most are debug/diagnostic switches: set to 1 to enable\n\
         unless the description says otherwise. Variables marked JIT need a\n\
         binary built with `--features jit`.",
        ENV_VARS.len()
    );
}
