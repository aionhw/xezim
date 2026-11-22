//! Compiled-method tiering, fast calls, the method cache directory, the sampled method
//! profiler and phase profiles.

use super::*;

/// ---- Sampled method profiler (in-process, no perf_event access) ----
///
/// The simulator's hot path is class-method dispatch + body execution
/// (`exec_method_in_class_hierarchy`), and UVM makes it by far the dominant
/// cost of a testbench run. To target optimizations we need an exact
/// wall-time-by-method histogram. `perf` is unavailable in this sandbox
/// (perf_event_paranoid) and gdb sampling is fragile under optimization, so
/// this tiny sampler lives in-process: the hot thread publishes the FNV hash
/// of the currently-running method name into `CUR_METHOD_HASH` (a single
/// relaxed store — no lock), and a sampler thread reads it every ~5 ms and
/// bumps a per-hash counter. First-seen hashes are resolved to full names
/// at report time via a name registry keyed by hash. Collisions in a 64-bit
/// FNV over a handful of method names are impossible in practice.
/// NB: this MUST be a process-global atomic (not a thread-local): the
/// background sampler thread reads it while the hot thread writes it. And to
/// keep the interpreter's critical path zero-cost when profiling is off, the
/// per-dispatch store below is gated by `SAMPLER_READY` (set once when the
/// sampler thread is spawned), which stays `false` in normal runs so the hot
/// path takes at most one predicted branch.
pub(super) static CUR_METHOD_HASH: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

pub(super) static SAMPLER_READY: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Planner split: 1 while the innermost method runs its COMPILED block,
/// 0 while it runs the AST interpreter. Sampled alongside the hash so the
/// histogram can answer "of the time spent in method X, how much is already
/// bytecode" — the input that ranks admission-expansion rows by heat.
pub(super) static CUR_METHOD_COMPILED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

static METHOD_SAMPLES: std::sync::OnceLock<Mutex<std::collections::HashMap<u64, u64>>> =
    std::sync::OnceLock::new();

static METHOD_SAMPLES_VM: std::sync::OnceLock<Mutex<std::collections::HashMap<u64, u64>>> =
    std::sync::OnceLock::new();

static METHOD_NAMES: std::sync::OnceLock<Mutex<std::collections::HashMap<u64, String>>> =
    std::sync::OnceLock::new();

/// FNV-1a 64-bit over the method name (cheap; the store below is the only
/// cross-thread write on the hot path).
#[inline]
pub(super) fn fnv_name(s: &str) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for &b in s.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// class-perf gate: class-function body bytecode compilation+execution is
/// ON by default; set `XEZIM_COMPILE_METHODS=0` to force every run onto
/// the proven AST interpreter. Tiering (see `method_tier_threshold`)
/// keeps cold methods on the interpreter, so the default path only
/// differs from the AST funnel for hot methods — and for those the
/// compiled block is byte-identical by construction (all-or-nothing
/// admission, no fallback mid-block).
pub(super) fn compiled_methods_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("XEZIM_COMPILE_METHODS")
            .map(|v| v != "0")
            .unwrap_or(true)
    })
}

/// Method diagnostics follow the same process-lifetime policy as compilation.
/// Cached calls must not search the environment on every method invocation.
pub(super) fn method_trace_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("XEZIM_FALLBACK_SITES")
            .map(|v| v != "0" && !v.is_empty())
            .unwrap_or(false)
    })
}

/// class-perf ADAPTIVE tiering: with the gate on, a method body is compiled
/// only once it has been CALLED *(cid, mid)* this many times
/// (`XEZIM_METHOD_TIER`). "Adaptive" here is two structural properties, not
/// a single number:
///
/// 1. **Skip cache gates the counter.** The tier counter is only reached by
///    methods that pass the per-(cid, mid) skip cache (see
///    `try_run_compiled_method`), i.e. methods that are *plausibly*
///    compilable. The ~90% of UVM methods that are permanently skip-cached
///    (task form, string/ref-formal, non-scalar return, ...) return before
///    the counter, so they pay zero tier bookkeeping — a cold/never-
///    compiling method contributes nothing to the hot path.
///
/// 2. **A hot threshold for actual compilation.** Once a non-skip method has
///    been CALLED `HOT_CALLS` times it is, by definition, no longer a
///    one-shot — the compile cost is repaid by its remaining calls on any
///    body shape. The original flat default of 100 000 was calibrated for
///    UVM-suite compile ECONOMY, but benchmarks showed it leaves genuinely
///    hot methods on the AST interpreter far too long: a scalar-bodied
///    method called 300k× under the 100k default spent its first 100k calls
///    (a third of the run) on AST and ran ~65% slower than with a lower
///    threshold. `HOT_CALLS` = 1000 is a measured middle ground:
///
///      * a method called ≥ 1000× compiles almost immediately and captures
///        the full bytecode win (300k-call benchmark: 0.8s vs 1.2s at 100k);
///      * methods called fewer times, and the whole short-run, compile-heavy
///        population the original author guarded, never reach the threshold
///        and stay on the reference-identical AST interpreter — no compile
///        cost is paid for them.
///
/// `0` restores compile-on-first-call (the pilot behavior the compiled
/// method_* tests pin). The interpreter frames below the threshold are
/// bit-identical to a gate-OFF run, so tiering cannot change observable
/// behavior.
pub(super) fn method_tier_threshold() -> u32 {
    // Hot-compile threshold: after this many genuine calls a method is
    // treated as hot and its body is compiled (already repaid on any body
    // shape). The tier counter bumps at most this many times while the
    // method is still sub-scalar before the compiled path takes over
    // counter-free.
    const HOT_CALLS: u32 = 1_000;
    static T: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *T.get_or_init(|| {
        std::env::var("XEZIM_METHOD_TIER")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(HOT_CALLS)
    })
}

/// Explicit `XEZIM_METHOD_TIER` enables measured loop heat in addition to
/// call-count tiering. AST loops publish heat at exit; the next method call
/// attempts compilation after that heat reaches the threshold. Unsupported
/// bodies remain interpreted, and an unset tier retains call-count admission.
#[inline]
/// The tier threshold was left at its default (no `XEZIM_METHOD_TIER`).
pub(super) fn method_tier_default() -> bool {
    !method_boost_enabled()
}

pub(super) fn method_boost_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("XEZIM_METHOD_TIER").is_some())
}

/// The subroutine shape `try_run_compiled_method` plans for. Borrowed, so
/// the per-call hooks (class methods, wait-free tasks, free functions) never
/// `Simulator::type_param_binding_kind`: a type parameter bound to `string`
/// or to a class.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum TypeParamKind {
    Str,
    Class,
}

/// What `Simulator::method_string_frame_begin` added to `string_signals`.
pub(super) struct MethodStringFrame {
    pub(super) signals: Vec<String>,
    pub(super) props: std::rc::Rc<Vec<String>>,
    pub(super) prop_mask: u64,
    pub(super) prop_more: Vec<usize>,
}

/// clone a declaration.
#[derive(Clone, Copy)]
pub(super) enum CompiledBodyKind<'a> {
    Function(&'a crate::ast::decl::FunctionDeclaration),
    Task(&'a TaskDeclaration),
    /// Extern / pure prototypes: never compiled.
    Other,
}

impl<'a> CompiledBodyKind<'a> {
    pub(super) fn of(kind: &'a crate::ast::decl::ClassMethodKind) -> Self {
        use crate::ast::decl::ClassMethodKind as K;
        match kind {
            K::Function(f) => Self::Function(f),
            K::Task(t) => Self::Task(t),
            K::Extern(_) | K::PureVirtual(_) => Self::Other,
        }
    }
}

/// class-perf P3: direct-call eligibility for a compiled (cid, mid).
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) enum FastCallState {
    /// Not yet decided (no map entry): the plan or compiled block may not
    /// exist yet — the call runs the interpreter route, which builds them;
    /// re-probed on the next direct-call attempt. The variant exists to
    /// document the tri-state; insertion sites use absence instead.
    #[allow(dead_code)]
    Pending,
    /// Never direct-callable (shape or body reasons); permanent.
    Never,
    /// Direct-callable — run `entry.block` in place.
    Entry(std::rc::Rc<FastCallEntry>),
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum FastAccessor {
    None,
    Getter(Box<str>),
    Setter(Box<str>),
}

/// class-perf P3: everything a direct VM->VM `CallMethod` needs to run a
/// compiled callee without materializing args or re-entering the
/// interpreter's binding loop. Mirrors `try_run_compiled_method`'s seeding
/// and result clamping, but sources formals from the CALLER's argument
/// registers.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct FastCallEntry {
    pub(super) block: std::rc::Rc<super::bytecode::CompiledBlock>,
    pub(super) this_reg: super::bytecode::RegId,
    pub(super) result_reg: super::bytecode::RegId,
    /// Per formal: (name, resize width (0 = keep source), optional
    /// signedness stamp) — replicates the interpreter binding loop's
    /// coercion. The frame copy keeps interpreter re-entries (nested call
    /// write-back temps, `\0vmargN` seeding) seeing the same values the
    /// AST path would have bound.
    pub(super) coerce: Vec<(Box<str>, u32, Option<bool>)>,
    /// Seed for a void/class result cell the body never writes (the
    /// interpreter's implicit-return init, computed from the return type).
    pub(super) seed: Value,
    pub(super) result_width: u32,
    pub(super) result_signed: bool,
    pub(super) is_class_result: bool,
    pub(super) is_void_result: bool,
    pub(super) accessor: FastAccessor,
    /// `ref` / `inout` scalar formal positions: their final register value
    /// is copied back into the caller's argument slot, as the interpreter
    /// route's write-back temps (`vm_drain_arg_temps`) would.
    pub(super) writeback: Box<[usize]>,
}

/// class-perf P3: is the direct VM->VM dispatch enabled? (XEZIM_FAST_CALLS=0
/// disables — an A/B switch for debugging.)
pub(super) fn fast_calls_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("XEZIM_FAST_CALLS")
            .map(|v| v != "0" && !v.is_empty())
            .unwrap_or(true)
    })
}

/// Cache directory for the persistent compiled-method cache, from
/// XEZIM_METHOD_CACHE. Unset: disabled. "0" or empty: disabled. "1":
/// <XDG_CACHE_HOME or ~/.cache>/xezim/method-cache. Any other value: that
/// literal path. The directory is created lazily on the first WRITE (a
/// read-only run must not fail or litter just because the env var is set).
pub(super) fn method_cache_dir() -> Option<&'static std::path::Path> {
    static DIR: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let raw = std::env::var("XEZIM_METHOD_CACHE").ok()?;
        if raw.is_empty() || raw == "0" {
            return None;
        }
        if raw != "1" {
            return Some(std::path::PathBuf::from(raw));
        }
        let base = std::env::var_os("XDG_CACHE_HOME")
            .filter(|v| !v.is_empty())
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache"))
            })?;
        Some(base.join("xezim").join("method-cache"))
    })
    .as_deref()
}

/// Sampler thread: read the hot thread's current hash and bump.
pub fn sampler_sample() {
    let h = CUR_METHOD_HASH.load(Ordering::Relaxed);
    if h == 0 {
        return;
    }
    let bucket = if CUR_METHOD_COMPILED.load(Ordering::Relaxed) {
        METHOD_SAMPLES_VM.get_or_init(Default::default)
    } else {
        METHOD_SAMPLES.get_or_init(Default::default)
    };
    *bucket.lock().unwrap().entry(h).or_insert(0) += 1;
}

/// After the run, resolve hashes to names and print the histogram.
pub fn sampler_report(top: usize) {
    let samples = METHOD_SAMPLES.get_or_init(Default::default).lock().unwrap();
    let vm = METHOD_SAMPLES_VM
        .get_or_init(Default::default)
        .lock()
        .unwrap();
    let names = METHOD_NAMES.get_or_init(Default::default).lock().unwrap();
    let mut v: Vec<u64> = samples.keys().chain(vm.keys()).copied().collect();
    v.sort_by_key(|&h| {
        std::cmp::Reverse(samples.get(&h).copied().unwrap_or(0) + vm.get(&h).copied().unwrap_or(0))
    });
    eprintln!("===== METHOD SAMPLE HISTOGRAM (top {}) =====", top);
    for k in v.iter().take(top) {
        let ast = samples.get(k).copied().unwrap_or(0);
        let bytecode = vm.get(k).copied().unwrap_or(0);
        let name = names.get(k).map(|s| s.as_str()).unwrap_or("?");
        eprintln!("{:>8} AST {:>8} VM  {}", ast, bytecode, name);
    }
}

/// Report-time registry: record that `hash` corresponds to `name`. Only called
/// for the handful of distinct methods a testbench actually runs (off hot path).
#[inline]
pub fn sampler_register_name(hash: u64, name: &str) {
    let mut n = METHOD_NAMES.get_or_init(Default::default).lock().unwrap();
    if !n.contains_key(&hash) {
        n.insert(hash, name.to_string());
    }
}

/// Clear state between simulator runs in the same process.
pub fn sampler_reset() {
    SAMPLER_READY.store(false, Ordering::Relaxed);
    CUR_METHOD_COMPILED.store(false, Ordering::Relaxed);
    METHOD_SAMPLES
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .clear();
    METHOD_SAMPLES_VM
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .clear();
    METHOD_NAMES
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .clear();
    CUR_METHOD_HASH.store(0, Ordering::Relaxed);
}

/// Phase-2/4 profile snapshot loaded from a `--write-profile` text file.
/// Sparse: only blocks that fired and signals that toggled appear.
#[derive(Default, Debug, Clone)]
pub struct Phase2Profile {
    /// edge_block_idx → accumulated runtime in ns.
    pub edge_block_exec_ns: ahash::AHashMap<usize, u64>,
    /// edge_block_idx → fire count.
    pub edge_block_exec_count: ahash::AHashMap<usize, u64>,
    /// signal_id → toggle count.
    pub signal_toggle_count: ahash::AHashMap<usize, u64>,
    /// Phase-4: edge_block_idx → static NBA insn count (per fire).
    pub nba_count_per_block: ahash::AHashMap<usize, u64>,
    /// Phase-4: signal_id → count of triggers that woke at least one
    /// fanout block.
    pub sensitivity_trigger_count: ahash::AHashMap<usize, u64>,
    /// Phase-4: edge_block_idx → count of times this block was woken
    /// by another block's NBA-driven edge fire.
    pub cross_block_wakeup_count: ahash::AHashMap<usize, u64>,
    /// Phase-4: partition_id → number of signals it NBA-writes that
    /// are read by another partition. Only present if the profile
    /// was collected with a partition loaded.
    pub boundary_packet_count: ahash::AHashMap<usize, u64>,
    /// Phase-4: partition_id → settle iterations local to it.
    pub local_quiescence_iterations: ahash::AHashMap<usize, u64>,
}

impl Phase2Profile {
    /// Parse the `--write-profile` text format. Tolerates missing
    /// sections (returns an empty subgroup) so a partial profile from
    /// an aborted run is still usable.
    pub fn load_from_file(path: &str) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("read {}: {}", path, e))?;
        let mut p = Phase2Profile::default();
        let mut section = "";
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                section = match &line[1..line.len() - 1] {
                    "edge_block_exec" => "edge_block_exec",
                    "signal_toggle" => "signal_toggle",
                    "nba_count_per_block" => "nba_count_per_block",
                    "sensitivity_trigger" => "sensitivity_trigger",
                    "cross_block_wakeup" => "cross_block_wakeup",
                    "boundary_packet" => "boundary_packet",
                    "local_quiescence" => "local_quiescence",
                    _ => "",
                };
                continue;
            }
            let mut it = line.split_whitespace();
            match section {
                "edge_block_exec" => {
                    let bi: usize = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                    let ns: u64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                    let c: u64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                    p.edge_block_exec_ns.insert(bi, ns);
                    p.edge_block_exec_count.insert(bi, c);
                }
                "signal_toggle" => {
                    let sid: usize = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                    let t: u64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                    p.signal_toggle_count.insert(sid, t);
                }
                "nba_count_per_block" => {
                    let bi: usize = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                    let n: u64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                    p.nba_count_per_block.insert(bi, n);
                }
                "sensitivity_trigger" => {
                    let sid: usize = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                    let t: u64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                    p.sensitivity_trigger_count.insert(sid, t);
                }
                "cross_block_wakeup" => {
                    let bi: usize = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                    let w: u64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                    p.cross_block_wakeup_count.insert(bi, w);
                }
                "boundary_packet" => {
                    let pid: usize = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                    let b: u64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                    p.boundary_packet_count.insert(pid, b);
                }
                "local_quiescence" => {
                    let pid: usize = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                    let q: u64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                    p.local_quiescence_iterations.insert(pid, q);
                }
                _ => {}
            }
        }
        Ok(p)
    }
}
