//! Event-driven simulator for SystemVerilog combinatorial and sequential logic.
//!
//! Implements a simplified IEEE 1800 scheduling model:
//!   Active region:  blocking assigns, continuous assigns, always_comb
//!   NBA region:     non-blocking assign updates
//!   Reactive:       edge-triggered always_ff/always_latch blocks
//!
//! The simulator state and shared types live here. Execution methods are
//! grouped by responsibility in the `simulator/` implementation modules.

// Compiler modules shared by the implementation submodules.
#[cfg(feature = "jit")]
use super::{aot, ts_jit};
use super::{
    bytecode, dispatch, elaborate, fst_sink, interrupt, jit, sdf, stdout_sink, value, vcd_sink,
};

use super::elaborate::{AlwaysBlock, DpiImportSpec, ElaboratedModule, Signal};
use super::value::{LogicBit, Value};
use crate::ast::decl::{
    AlwaysKind, ClassConstraint, ConstraintItem, ConstraintRange, CovergroupDeclaration,
    CovergroupItem, FunctionDeclaration, LetDeclaration, TaskDeclaration,
};
use crate::ast::expr::*;
use crate::ast::stmt::*;
use crate::ast::types::{DataType, IntegerAtomType, PortDirection};
#[allow(unused_imports)]
use crate::{log_eprintln as eprintln, log_println as println};

use super::interrupt::{
    INTERRUPT_SIGNAL, acknowledge_interrupt, interrupt_requested, loop_stop_requested,
    publish_sim_time,
};
use fst_writer::{
    FstHeaderWriter, FstScopeType, FstSignalId, FstSignalType, FstVarDirection, FstVarType,
};
use libffi::middle::{Arg, Cif, CodePtr, Type};
use libloading::Library;
use rand::SeedableRng;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::ffi::{CStr, CString};
use std::fs::OpenOptions;
use std::io::Write;
use std::os::raw::c_void;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use xezim_core::elaborate::resolve_type_width;
use xezim_core::hasher::{HashMap, HashSet};

static SIM_DEBUG_ENABLED: AtomicBool = AtomicBool::new(false);

/// First activations of a cached process-`if` condition also run the AST
/// evaluator and compare (see `try_eval_process_cond_bytecode`). Small: each
/// audit costs one redundant AST evaluation; divergence pins the span to the
/// AST forever, so the window only needs to catch shape-level miscompiles,
/// which show on the first data that exercises them.
const COND_AUDIT_EVALS: u8 = 4;
/// `--dump-timescales`: print every module's timescale before the run starts.
static DUMP_TIMESCALES: AtomicBool = AtomicBool::new(false);
static DPI_LIB_PATHS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

/// Opaque handle base for `process::self()` (IEEE 1800-2023 §9.7). The token is
/// `PROCESS_HANDLE_BASE + pid`, chosen far above any real heap index so a
/// process handle can never be mistaken for a class-object handle.
const PROCESS_HANDLE_BASE: u64 = 0x7000_0000;

/// Interpreted loops poll the Ctrl-C flag once every this-many-plus-one
/// iterations: a long loop inside one time slot never reaches the event
/// loop's slot-boundary poll, and the backstop would otherwise kill it with
/// the dumps unfinalized.
const LOOP_INTERRUPT_POLL_MASK: u64 = 0x3ff;

/// A random stream (IEEE 1800-2023 §18.14 "Random stability").
///
/// §18.14 requires every *object* and every *process* to own an independent,
/// seedable stream whose state can be captured (`get_randstate`) and restored
/// (`set_randstate`) verbatim — a restored stream must replay the identical
/// sequence. `StdRng` (ChaCha) has no stable, round-trippable textual state, so
/// the streams use an explicit SplitMix64 generator: the entire state is one
/// `u64`, which serialises to 16 hex digits and restores exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SvRng {
    state: u64,
}

impl SvRng {
    /// The seed a run uses when it does not ask for another one. Fixed, not
    /// drawn from entropy, so that two runs of the same design are identical
    /// and a failing random test can be re-run and debugged. `+seed=<n>`
    /// picks a different stream; `+seed=random` opts into entropy.
    const DEFAULT_SEED: u64 = 1;

    /// §18.14: seeding is deterministic — equal seeds yield equal sequences.
    fn from_seed(seed: u64) -> Self {
        SvRng {
            state: seed ^ 0x9E37_79B9_7F4A_7C15,
        }
    }

    /// §18.14.2 `get_randstate`: the full state as an implementation-defined
    /// string (here: 16 hex digits).
    fn to_state_string(&self) -> String {
        format!("{:016x}", self.state)
    }

    /// §18.14.2 `set_randstate`: restore a state produced by `to_state_string`.
    /// A foreign string is implementation-defined; hash it into a seed rather
    /// than ignore it.
    fn from_state_string(s: &str) -> Self {
        let t = s.trim();
        if let Ok(v) = u64::from_str_radix(t, 16) {
            SvRng { state: v }
        } else {
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for b in t.as_bytes() {
                h ^= *b as u64;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
            SvRng { state: h }
        }
    }

    /// SplitMix64 step.
    fn next_state(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

impl rand::RngCore for SvRng {
    fn next_u32(&mut self) -> u32 {
        (self.next_state() >> 32) as u32
    }
    fn next_u64(&mut self) -> u64 {
        self.next_state()
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        let mut i = 0;
        while i < dest.len() {
            let chunk = self.next_state().to_le_bytes();
            let n = (dest.len() - i).min(8);
            dest[i..i + n].copy_from_slice(&chunk[..n]);
            i += n;
        }
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

thread_local! {}
thread_local! {
    /// Current `run_process_stmts` recursion depth. The suspend-aware loop
    /// handlers trampoline through the event queue (instead of recursing) once
    /// this exceeds `RPS_TRAMPOLINE_DEPTH`, bounding the stack for long
    /// synchronous loops that would otherwise recurse until stack overflow.
    static RPS_DEPTH: Cell<usize> = const { Cell::new(0) };
}

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
static CUR_METHOD_HASH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static SAMPLER_READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Planner split: 1 while the innermost method runs its COMPILED block,
/// 0 while it runs the AST interpreter. Sampled alongside the hash so the
/// histogram can answer "of the time spent in method X, how much is already
/// bytecode" — the input that ranks admission-expansion rows by heat.
static CUR_METHOD_COMPILED: std::sync::atomic::AtomicBool =
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
fn fnv_name(s: &str) -> u64 {
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
fn compiled_methods_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("XEZIM_COMPILE_METHODS")
            .map(|v| v != "0")
            .unwrap_or(true)
    })
}

/// Method diagnostics follow the same process-lifetime policy as compilation.
/// Cached calls must not search the environment on every method invocation.
fn method_trace_enabled() -> bool {
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
fn method_tier_threshold() -> u32 {
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
fn method_boost_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("XEZIM_METHOD_TIER").is_some())
}

/// Compiler salt for the persistent compiled-method cache (XEZIM_METHOD_CACHE).
/// Bump on ANY change that can alter the compiled instructions or the
/// admit/decline decision for the same source text — admission inputs,
/// lowering, register allocation, block layout. Stale entries with an old
/// salt hash to different keys and are simply recompiled; the safety net is
/// the suite-level ON == OFF parity run against a warm cache.
// §6.16 / §6.24.3: string casts emit conversion rather than width resizing.
const METHOD_CACHE_COMPILER_SALT: u64 = 0x6370_6666_3046;

/// The subroutine shape `try_run_compiled_method` plans for. Borrowed, so
/// the per-call hooks (class methods, wait-free tasks, free functions) never
/// clone a declaration.
#[derive(Clone, Copy)]
enum CompiledBodyKind<'a> {
    Function(&'a crate::ast::decl::FunctionDeclaration),
    Task(&'a TaskDeclaration),
    /// Extern / pure prototypes: never compiled.
    Other,
}

impl<'a> CompiledBodyKind<'a> {
    fn of(kind: &'a crate::ast::decl::ClassMethodKind) -> Self {
        use crate::ast::decl::ClassMethodKind as K;
        match kind {
            K::Function(f) => Self::Function(f),
            K::Task(t) => Self::Task(t),
            K::Extern(_) | K::PureVirtual(_) => Self::Other,
        }
    }
}

/// class-perf P3: direct-call eligibility for a compiled (cid, mid).
#[derive(Clone)]
enum FastCallState {
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

#[derive(Clone, Debug, PartialEq, Eq)]
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
struct FastCallEntry {
    block: std::rc::Rc<super::bytecode::CompiledBlock>,
    this_reg: super::bytecode::RegId,
    result_reg: super::bytecode::RegId,
    /// Per formal: (name, resize width (0 = keep source), optional
    /// signedness stamp) — replicates the interpreter binding loop's
    /// coercion. The frame copy keeps interpreter re-entries (nested call
    /// write-back temps, `\0vmargN` seeding) seeing the same values the
    /// AST path would have bound.
    coerce: Vec<(Box<str>, u32, Option<bool>)>,
    /// Seed for a void/class result cell the body never writes (the
    /// interpreter's implicit-return init, computed from the return type).
    seed: Value,
    result_width: u32,
    result_signed: bool,
    is_class_result: bool,
    is_void_result: bool,
    accessor: FastAccessor,
}

/// class-perf P3: is the direct VM->VM dispatch enabled? (XEZIM_FAST_CALLS=0
/// disables — an A/B switch for debugging.)
fn fast_calls_enabled() -> bool {
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
fn method_cache_dir() -> Option<&'static std::path::Path> {
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

/// Depth at which `run_process_stmts` loop handlers switch from recursing to
/// trampolining via the event queue. ~2.9KB/frame overflows an 8MB stack near
/// ~2800; 300 leaves a wide margin while keeping normal (shallow) loops on the
/// fast recursive path.
const RPS_TRAMPOLINE_DEPTH: usize = 300;

/// Internal marker retrieving a §9.4.5 intra-assignment RHS value captured in
/// `Simulator::intra_saved` (see `make_intra_saved_expr`). Never user-visible.
const INTRA_SAVED_MARKER: &str = "$__xz_intra_saved";
/// Internal `$__xz_vc_changed(idx, expr)`: true once `expr` differs from the
/// value stashed in `Simulator::vc_saved[idx]` when a value-change event
/// control (`@prop` on a non-event class property) was armed. Never
/// user-visible.
const VC_CHANGED_MARKER: &str = "$__xz_vc_changed";
/// Ends a deferred declaration initializer: `(<target>, ...)` records the
/// values it gave as their default sampled values (see `sampled_defaults`).
const SAMPLED_DEFAULT_MARKER: &str = "$__xz_sampled_default";
/// A signal of a concurrent assertion resolved at registration: `(<id>)`
/// reads `signal_table[id]` (see `sva_resolve_signals`).
const SVA_SIG_MARKER: &str = "$__xz_sva_sig";
/// §16.3: the message of the `$error` a failed assertion with no `else`
/// clause reports (the reference simulator's wording).
const ASSERT_DEFAULT_ERROR: &str = "Assertion error.";
/// Internal task a waiter resumes to report that a wait made from the
/// nested (C-called) path is over; see `run_nested_until`.
const SYNC_WAKE_MARKER: &str = "$__xz_sync_wake";

/// §21.2.1.7 `%m` bookkeeping: an `m_scope_stack` entry starting with
/// `M_ROOT_MARK` carries the absolute declaring path of the running
/// subroutine (`pk`, `tb.u`); one starting with `M_SUBR_MARK` is a subroutine
/// named from the caller's instance. Plain entries are named blocks.
pub(crate) const M_ROOT_MARK: char = '\u{1}';
const M_SUBR_MARK: char = '\u{2}';

pub fn set_sim_debug(enabled: bool) {
    SIM_DEBUG_ENABLED.store(enabled, Ordering::Relaxed);
}

/// `--dump-timescales`: enable the pre-run per-module timescale dump.
pub fn set_dump_timescales(enabled: bool) {
    DUMP_TIMESCALES.store(enabled, Ordering::Relaxed);
}

#[inline]
fn dump_timescales_enabled() -> bool {
    DUMP_TIMESCALES.load(Ordering::Relaxed)
}

#[inline]
fn sim_debug_enabled() -> bool {
    SIM_DEBUG_ENABLED.load(Ordering::Relaxed)
}

/// XTrace header knobs that only the CLI sets: `--xtrace-profile` (the §6.5
/// `@profile` token) and `--xtrace-compress` (the §6.8 `@compression` token).
///
/// Process-global, like `set_dpi_libs`, because `simulate_multi` constructs and
/// runs the `Simulator` internally — there is no other channel from `main` into
/// the dump, and threading two more arguments through a 24-argument public
/// signature would churn every caller in the tree.
/// `+nospecify` (commercial GLS flag): suppress specify-block module path
/// delays — zero-delay gate simulation — and, as in the reference simulator,
/// the §31 timing checks (`timing_checks::set_no_timing_checks` disables only
/// the checks).
static NOSPECIFY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `--warn-x`: report the FIRST time each signal takes an X bit after time 0,
/// naming the signal, its instance/module, and everything that drives it.
/// Off by default — X during initialization is normal and would drown the log.
static WARN_X: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// `--warn-x-limit N`: stop after N reports (0 = unlimited).
static WARN_X_LIMIT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(50);

pub fn set_warn_x(v: bool) {
    WARN_X.store(v, std::sync::atomic::Ordering::Relaxed);
}

pub fn set_warn_x_limit(n: usize) {
    WARN_X_LIMIT.store(n, std::sync::atomic::Ordering::Relaxed);
}

/// Whether x-warnings are on (`--x-warn`, `+X_WARN`, `XEZIM_X_WARN`).
pub fn warn_x_active() -> bool {
    warn_x_enabled()
}

fn warn_x_enabled() -> bool {
    if WARN_X.load(std::sync::atomic::Ordering::Relaxed) {
        return true;
    }
    // `X_WARN=1` / `X_WARN=on`: same switch, settable without editing the
    // command line. Any value other than 0/off/no/false enables it.
    match std::env::var("XEZIM_X_WARN") {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            !v.is_empty() && v != "0" && v != "off" && v != "no" && v != "false"
        }
        Err(_) => false,
    }
}

fn warn_x_limit() -> usize {
    if let Ok(v) = std::env::var("XEZIM_X_WARN_LIMIT") {
        if let Ok(n) = v.trim().parse::<usize>() {
            return n;
        }
    }
    WARN_X_LIMIT.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_nospecify(v: bool) {
    NOSPECIFY.store(v, std::sync::atomic::Ordering::Relaxed);
}

/// The time limit is a requested run length (`-do "run 100ns"`), so reaching
/// it is the expected end of the run, not a possible hang to report.
static RUN_LENGTH_REQUESTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub fn set_run_length_requested(v: bool) {
    RUN_LENGTH_REQUESTED.store(v, std::sync::atomic::Ordering::Relaxed);
}

fn nospecify() -> bool {
    NOSPECIFY.load(std::sync::atomic::Ordering::Relaxed)
}

/// Structural-delay mode (commercial-simulator GLS convention): 0 = normal (specify/SDF path
/// delays apply), 1 = `+delay_mode_zero` (all structural delays forced to 0 —
/// fast functional GLS), 2 = `+delay_mode_unit` (every nonzero structural delay
/// becomes 1 time unit). Affects specify path delays and SDF back-annotation;
/// procedural `#` delays are unaffected, per the LRM's delay_mode scope.
static DELAY_MODE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

pub fn set_delay_mode(mode: u8) {
    DELAY_MODE.store(mode, std::sync::atomic::Ordering::Relaxed);
}

fn delay_mode() -> u8 {
    DELAY_MODE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Map a structural delay through the active `+delay_mode_*`: zero → 0,
/// unit → 1 for any nonzero delay, normal → unchanged.
fn apply_delay_mode(d: u64) -> u64 {
    match delay_mode() {
        1 => 0,
        2 => {
            if d > 0 {
                1
            } else {
                0
            }
        }
        _ => d,
    }
}

#[derive(Clone, Default)]
pub struct XtraceOptions {
    pub profile: Option<String>,
    pub compress: Option<String>,
}

/// §21.7: waveform dumping (VCD via `$dumpvars`, FST, XTrace) is OPT-IN at
/// model-compile time via `--wave`. It is not free: an active dump forces
/// loops that would otherwise compile onto the AST path so every iteration is
/// visible, and it builds a per-traced-signal table. A run that never dumps
/// should not pay for either, and a design that calls `$dumpvars` should not
/// silently start dumping.
///
/// Set from the CLI before elaboration, so `Simulator::new` can read it while
/// building the model. `--fst`/`--xtrace` are explicit dump requests and turn
/// it on by themselves.
static WAVE_ENABLED: AtomicBool = AtomicBool::new(false);

/// Enable waveform support for models compiled after this call.
pub fn set_wave_enabled(on: bool) {
    WAVE_ENABLED.store(on, Ordering::Relaxed);
}

/// Whether waveform support was requested for this run.
pub fn wave_enabled() -> bool {
    WAVE_ENABLED.load(Ordering::Relaxed)
}

fn xtrace_options_cell() -> &'static Mutex<XtraceOptions> {
    static XTRACE_OPTIONS: OnceLock<Mutex<XtraceOptions>> = OnceLock::new();
    XTRACE_OPTIONS.get_or_init(|| Mutex::new(XtraceOptions::default()))
}

pub fn set_xtrace_options(profile: Option<String>, compress: Option<String>) {
    if let Ok(mut guard) = xtrace_options_cell().lock() {
        *guard = XtraceOptions { profile, compress };
    }
}

fn xtrace_options() -> XtraceOptions {
    xtrace_options_cell()
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default()
}

fn dpi_lib_paths() -> &'static Mutex<Vec<String>> {
    DPI_LIB_PATHS.get_or_init(|| Mutex::new(Vec::new()))
}

pub fn set_dpi_libs(paths: &[String]) {
    if let Ok(mut guard) = dpi_lib_paths().lock() {
        *guard = paths.to_vec();
    }
}

fn vpi_lib_paths() -> &'static Mutex<Vec<String>> {
    static VPI_LIB_PATHS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    VPI_LIB_PATHS.get_or_init(|| Mutex::new(Vec::new()))
}

/// `--vpi-lib`: shared objects loaded as VPI modules. Each one's
/// `vlog_startup_routines` runs once, before simulation, so it can
/// register system tasks with `vpi_register_systf`.
pub fn set_vpi_libs(paths: &[String]) {
    if let Ok(mut guard) = vpi_lib_paths().lock() {
        *guard = paths.to_vec();
    }
}

fn configured_vpi_libs() -> Vec<String> {
    vpi_lib_paths()
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default()
}

/// Open a `--dpi-lib` library with `RTLD_NOW | RTLD_GLOBAL` (see
/// `load_dpi_libraries`).
fn open_dpi_library(path: &str) -> Result<Library, libloading::Error> {
    // SAFETY: loading a dynamic library runs its initializers; the handle is
    // kept alive for the simulator's lifetime.
    #[cfg(unix)]
    {
        use libloading::os::unix::{Library as UnixLibrary, RTLD_GLOBAL, RTLD_NOW};
        unsafe { UnixLibrary::open(Some(path), RTLD_NOW | RTLD_GLOBAL) }.map(Library::from)
    }
    #[cfg(not(unix))]
    {
        unsafe { Library::new(path) }
    }
}

/// The command that builds the DPI export trampoline (§35.5.4): the host C
/// compiler (`$CC` when set, else `cc`) producing a position-independent
/// shared object. Only portable flags: the compiler's default target is
/// already the host, and an x86-only flag such as `-m64` makes a non-x86
/// compiler reject the command (#282). `$CC` may carry its own leading
/// arguments (`ccache gcc`). On macOS the linker must be told that the
/// dispatch symbol is resolved at load time from the xezim executable.
fn dpi_trampoline_cc_command(
    cc_env: Option<&str>,
    so: &std::path::Path,
    c: &std::path::Path,
) -> Vec<std::ffi::OsString> {
    let mut cmd: Vec<std::ffi::OsString> = cc_env
        .map(|v| v.split_whitespace().map(Into::into).collect::<Vec<_>>())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| vec!["cc".into()]);
    for a in ["-fPIC", "-shared", "-O1"] {
        cmd.push(a.into());
    }
    if cfg!(target_os = "macos") {
        cmd.push("-Wl,-U,___xezim_dpi_export_dispatch".into());
    }
    cmd.push("-o".into());
    cmd.push(so.into());
    cmd.push(c.into());
    cmd
}

fn configured_dpi_libs() -> Vec<String> {
    dpi_lib_paths()
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default()
}

/// Escape a string for embedding inside a JSON `"..."` literal — covers the
/// minimum set of control / structural characters so the coverage-DB output
/// stays valid JSON for covergroup / coverpoint names that include `"` or
/// `\`.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            _ => out.push(c),
        }
    }
    out
}

fn cached_env_flag(name: &'static str, slot: &'static OnceLock<bool>) -> bool {
    *slot.get_or_init(|| std::env::var(name).ok().as_deref() == Some("1"))
}

fn cached_env_set(name: &'static str, slot: &'static OnceLock<bool>) -> bool {
    *slot.get_or_init(|| std::env::var_os(name).is_some())
}

macro_rules! env_set_cached {
    ($name:literal) => {{
        static FLAG: OnceLock<bool> = OnceLock::new();
        cached_env_set($name, &FLAG)
    }};
}

macro_rules! env_is_one_cached {
    ($name:literal) => {{
        static FLAG: OnceLock<bool> = OnceLock::new();
        cached_env_flag($name, &FLAG)
    }};
}

fn xz_copy_debug_enabled() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    *FLAG.get_or_init(|| std::env::var_os("XEZIM_CP_DBG").is_some())
}

fn parallel_disabled_by_env() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    cached_env_flag("XEZIM_NO_PARALLEL", &FLAG)
}

fn parallel_serialize_enabled() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    cached_env_flag("XEZIM_PARALLEL_SERIALIZE", &FLAG)
}

macro_rules! sim_dbg_eprintln {
    ($($arg:tt)*) => {
        if sim_debug_enabled() {
            eprintln!($($arg)*);
        }
    };
}

/// Write `signal_table[id]` and update the parallel `signal_has_xz`
/// byte that the JIT inline X/Z prelude reads. Must be used in place
/// of bare `write_sig!(self, id, val);` so the JIT bitmap stays
/// consistent. Both fields are simple `Vec`s on `Self`, so the macro
/// expands to a single pair of indexed writes — the borrow checker
/// sees disjoint field borrows just like the bare assignment did.
/// Mark a signal id dirty for the next incremental VCD flush. A macro (not a
/// method) so it expands to disjoint FIELD accesses — a `&mut self` method call
/// would conflict with call sites that already hold a mutable borrow of another
/// field (e.g. `for cg in &mut self.clock_generators`). Cheap: one bounds check
/// + one epoch compare when the dump is active, a single branch otherwise. The
/// rolling epoch dedups repeat writes within a flush.
macro_rules! vcd_mark {
    ($self:ident, $id:expr_2021) => {{
        if $self.dump_dirty_active {
            let __vm_id = $id;
            if __vm_id < $self.dump_dirty_mark.len()
                && $self.dump_dirty_mark[__vm_id] != $self.dump_dirty_epoch
            {
                $self.dump_dirty_mark[__vm_id] = $self.dump_dirty_epoch;
                $self.dump_dirty.push(__vm_id as u32);
            }
        }
    }};
}

macro_rules! write_sig {
    ($self:ident, $id:expr_2021, $val:expr_2021) => {{
        let __wsig_id = $id;
        let mut __wsig_val = $val;
        // Invariant: a table slot's stored signedness is the signal's DECLARED
        // signedness (`signal_signed[id]`), never the rvalue's. An unsized
        // decimal literal is signed (§5.7.1), so without this `barr[i] = 9`
        // stamped is_signed onto a `bit [3:0]` element and every later read
        // sign-extended it (-7). Real values are exempt (is_signed unused) and
        // strings never reach a sized table slot with it consulted.
        if !__wsig_val.is_real && __wsig_id < $self.signal_signed.len() {
            __wsig_val.is_signed = $self.signal_signed[__wsig_id];
        }
        // LRM §9.3.1: skip writes to forced signals. The `is_empty` guard
        // keeps the common no-force case to one branch — `contains_key`
        // hashes the id even when the map is empty, and this macro runs on
        // EVERY signal write (measured ~2% of c906 memcpy cycles).
        if $self.forced_signals.is_empty() || !$self.forced_signals.contains_key(&__wsig_id) {
            let __wsig_changed = $self.signal_table[__wsig_id] != __wsig_val;
            if __wsig_id < $self.signal_has_xz.len() {
                $self.signal_has_xz[__wsig_id] = if __wsig_val.raw_bits().1 != 0 {
                    1u8
                } else {
                    0u8
                };
            }
            // JIT-redesign Stage 1 maintenance: keep signal_inline_bits in
            // sync with signal_table for the future JIT codegen path that
            // reads values by direct pointer offset.  Zero overhead when
            // signal_inline_bits is empty (default; allocated only when
            // XEZIM_INLINE_BITS=1).
            if __wsig_id < $self.signal_inline_bits.len() {
                let (__wsig_v, __wsig_x) = __wsig_val.raw_bits();
                $self.signal_inline_bits[__wsig_id] = [__wsig_v, __wsig_x];
            }
            // `--warn-x`: record a signal taking a NEW x bit after time 0.
            // One bool load when disabled. Placed here because the old value is
            // still in the table, so "became x" is distinguishable from "was
            // already x". Written inline rather than as a call: this macro
            // expands inside bodies that already hold a `&mut` borrow of another
            // field, so only disjoint field access is possible here.
            if $self.warn_x && $self.time > 0 {
                let (__xov, __xox) = $self.signal_table[__wsig_id].raw_bits();
                let (__xnv, __xnx) = __wsig_val.raw_bits();
                let _ = __xov;
                // x is (xz & !val). Report only a VALID 0/1 bit (old xz clear)
                // going x: a bit that has been x or z all along — an undriven
                // input, an uninitialized reg first materializing — is not a
                // valid-value corruption and stays silent.
                let __fresh = (__xnx & !__xnv) & !__xox;
                let __wide_new_x = __wsig_val.width > 64
                    && __wsig_val.has_xz()
                    && !$self.signal_table[__wsig_id].has_xz();
                if (__fresh != 0 || __wide_new_x)
                    && $self.warn_x_seen.borrow_mut().insert(__wsig_id)
                {
                    let __xt = $self.time;
                    let __xp = $self.current_pid;
                    $self.warn_x_pending.borrow_mut().push((
                        __wsig_id,
                        __xt,
                        __wsig_val.clone(),
                        __xp,
                        None,
                    ));
                }
            }
            // XEZIM_VALUE_TRACE: queue a changed watched write (old value is
            // still in the table here). Disjoint field accesses only — this
            // macro expands under live &mut borrows of other fields.
            if let Some(__vt_ids) = &$self.value_trace_ids {
                if __vt_ids.contains(&__wsig_id) && $self.signal_table[__wsig_id] != __wsig_val {
                    $self.value_trace_pending.borrow_mut().push((
                        __wsig_id,
                        $self.time,
                        $self.signal_table[__wsig_id].clone(),
                        __wsig_val.clone(),
                        $self.current_pid,
                        $self.edge_dispatch_phase,
                    ));
                }
            }
            $self.signal_table[__wsig_id] = __wsig_val;
            // Expression-backed procedural assignments use per-signal
            // generations as their sensitivity set. Keep the usual no-active-
            // assignment path to one branch; only real value changes advance
            // the generation.
            if __wsig_changed
                && !$self.active_force_exprs.is_empty()
                && __wsig_id < $self.active_force_signal_epochs.len()
            {
                $self.active_force_signal_epochs[__wsig_id] =
                    $self.active_force_signal_epochs[__wsig_id].wrapping_add(1);
                $self.force_epoch_gen = $self.force_epoch_gen.wrapping_add(1);
            }
            // Incremental VCD: mark this write dirty (no-op when the dump is off
            // or the full scan is forced). Superset-safe — the flush re-checks
            // prev vs cur, so marking an unchanged write costs only a skip.
            vcd_mark!($self, __wsig_id);
            // O1 measurement: stamp the signal's last-change phase.
            if $self.event_measure && !$self.armed_edge && __wsig_id < $self.sig_last_change.len() {
                $self.sig_last_change[__wsig_id] = $self.event_phase;
            }
            // Arm only gateable edge blocks that actually read this signal.
            // The dense bitmap keeps unrelated writes a single cheap branch;
            // positive hits use the sparse CSR fanout built after compile.
            if $self.armed_edge
                && ((__wsig_id >> 6) < $self.armed_input_bitmap.len())
                && ($self.armed_input_bitmap[__wsig_id >> 6] >> (__wsig_id & 63)) & 1 != 0
            {
                let (lo, hi) = $self.armed_input_ranges[__wsig_id];
                for __k in lo as usize..hi as usize {
                    let __bi = $self.armed_input_blocks[__k] as usize;
                    $self.edge_block_armed[__bi] |= EDGE_ARMED;
                }
            }
            // Dirty-driven edge detect: record edge-sensitive writes (no-op unless
            // XEZIM_DIRTY_EDGE/_SHADOW). Inlined here rather than a method call so
            // the borrow stays a direct field access in macro-expanded contexts.
            if ($self.dirty_edge || $self.dirty_edge_shadow)
                && (__wsig_id as usize) < $self.sig_to_edge_pos.len()
            {
                let __ep = $self.sig_to_edge_pos[__wsig_id as usize];
                if __ep >= 0 && !$self.edge_pos_seen[__ep as usize] {
                    $self.edge_pos_seen[__ep as usize] = true;
                    $self.changed_edge_pos.push(__ep as usize);
                }
            }
            // §9.2 re-trigger: a blocking write to an edge-sensitive signal
            // made WHILE an edge block executes must re-run edge detection
            // for that signal (drain_edge_exec_rescan) — record its position.
            // One bool load when not inside an edge block.
            if ($self.in_edge_block || $self.in_edge_cont)
                && (__wsig_id as usize) < $self.sig_to_edge_pos.len()
            {
                let __ep = $self.sig_to_edge_pos[__wsig_id as usize];
                if __ep >= 0 && !$self.edge_exec_seen[__ep as usize] {
                    $self.edge_exec_seen[__ep as usize] = true;
                    $self.edge_exec_wrote.push(__ep as usize);
                }
            }
            // §38.36.1 cbValueChange: report the change right away, in the
            // region of the write. One emptiness check while nothing is
            // watched. Writes that bypass this macro are caught by the poll
            // at the end of each settle and delta (`vpi_cb::vpi_checkpoint`).
            // The dispatcher takes the raw self-pointer: this macro expands
            // under live borrows of other fields.
            if !$self.dpi_value_change_cbs.is_empty()
                && $self.dpi_value_change_cbs.contains_key(&__wsig_id)
            {
                vpi_cb::vpi_vc_written($self.vpi_self_ptr, __wsig_id);
            }
        }
    }};
}

/// Lazy two-state lowering state for one comb entry (see P5 lowering in
/// bytecode.rs). `Arc` so the eval site can clone the handle out of `self`
/// before running with `&mut self`.
#[derive(Clone)]
enum TsSlot {
    Untried,
    No,
    Yes(std::sync::Arc<super::bytecode::TwoStateBlock>),
}

/// A combinatorial item (continuous assign or always @*/always_comb block)
/// with pre-computed sensitivity set for efficient evaluation.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
enum CombItem {
    Noop,
    ContAssign {
        // Interpreted fallbacks are cold. Keep their large ASTs out of the
        // enum so direct-copy/fused/compiled entries stay cache compact.
        lhs: Box<Expression>,
        rhs: Box<Expression>,
        delay: u64,
    },
    /// Fast path: direct signal-to-signal copy (assign b = a) with pre-resolved IDs.
    DirectCopy {
        dst_id: usize,
        src_id: usize,
        width: u32,
    },
    FastDirectCopy {
        dst_id: usize,
        src_id: usize,
    },
    /// Several exact-width, zero-delay copies with one common source. Port
    /// arrays frequently elaborate into dozens of these (notably replicated
    /// PLL clock inputs); one entry avoids repeated worklist dispatch while
    /// retaining an individual signal-table cell for every destination.
    FastDirectFanout {
        src_id: usize,
        dst_ids: Box<[usize]>,
    },
    /// Batched one-bit buffers sharing the same source bit and inversion.
    FusedBufFanout {
        src: BitRef,
        dsts: Box<[BitRef]>,
        invert: bool,
    },
    /// Batched one-bit AND/NAND gates sharing one input. Clock trees commonly
    /// instantiate many `gclk = parent_clk & enable` cells; one dependency
    /// entry avoids repeated worklist dispatch while every enable remains a
    /// sensitivity and every destination remains independent.
    FusedAndFanout {
        common: BitRef,
        branches: Box<[FusedAndBranch]>,
        invert: bool,
    },
    /// Bytecode-compiled cont_assign: RHS compiled to VM instructions,
    /// result written to pre-resolved dst_id via BlockingAssign insn.
    CompiledContAssign {
        compiled: super::bytecode::CompiledBlock,
    },
    AlwaysBlock {
        stmt: Box<Statement>,
        is_always_comb: bool,
    },
    /// Bytecode-compiled comb always block. Skips the AST interpreter per
    /// settle iteration — dominant hot path in RTL-heavy designs.
    CompiledAlwaysBlock {
        compiled: super::bytecode::CompiledBlock,
        is_always_comb: bool,
    },
    /// Fused 1-bit gate: recognizes yosys-generated patterns
    /// `assign d[i] = a op b`, `assign d[i] = ~a`, `assign d[i] = s ? t : e`.
    /// Skips bytecode VM entirely — reads operand bits from signal_table,
    /// computes 4-state logic, writes single bit back.
    FusedGate {
        op: FusedGate,
    },
    /// Aligned slices of independent one-bit gates represented as one plane
    /// operation and one destination commit.
    VectorGate {
        op: VectorGate,
    },
    /// Adjacent source lanes whose destinations remain separate scalar cells.
    /// The vector operation is shared while each destination keeps its own
    /// change detection and dependency dispatch.
    ScatterGate {
        op: ScatterGate,
    },
    /// A coact-clustered batch of fused gates executed in one worklist
    /// dispatch, in topological member order, through the SAME per-gate
    /// evaluator as `FusedGate` — zero semantic novelty, machinery amortized
    /// over the batch. Formed by the region pass (class 3) from
    /// profile-measured co-activation edges. c906: gates are 49.3% of all
    /// comb evals and were untouched by bytecode fusion.
    GateRegion {
        gates: Box<[FusedGate]>,
    },
    /// IEEE 1800-2017 §29 User-Defined Primitive instance. Evaluated by
    /// `eval_udp(idx)` against `self.udp_runtime[idx]` (truth table + per-
    /// instance sequential state). Always evaluated on the serial settle path.
    Udp {
        idx: usize,
    },
    /// Edge-filtered sequential UDPs sharing one event terminal. Their data
    /// terminals are proven state-holding, so one dependency dispatch can
    /// evaluate the whole clock-domain batch.
    UdpBatch {
        event_ref: BitRef,
        indices: Box<[usize]>,
    },
    /// §31 timing checks watching one signal: `eval_timing_watch(idx)`
    /// against `self.timing_watches[idx]`. Reads that signal and writes
    /// nothing directly (a notifier toggles through the NBA region).
    TimingCheck {
        idx: usize,
    },
    /// A `FusedGate` AND/OR (NAND/NOR) whose high-fanout input is ignored
    /// while the other input holds the controlling value: the gate is
    /// `ctl_gates[slot]` (see `CtlGate`). Made from `FusedGate` entries at
    /// the first settle, never before the prepared-comb cache is written.
    CtlGate {
        slot: u32,
    },
}

/// Controlling-value skip for one two-input AND/OR gate (`CombItem::CtlGate`).
///
/// `wl = clk & rs` cannot change while `rs` is 0, however `clk` toggles, so
/// the gate's dependency edges from `clk` (the high-fanout input) carry mask
/// 0 while `rs` sits at the controlling value (0 for AND/NAND, 1 for
/// OR/NOR) and the static mask otherwise. Every evaluation of the gate
/// recomputes that mask from the controlling input's value it read, so:
/// - a zero mask always means the controlling input held the controlling
///   value at the gate's last evaluation, and the output already equals the
///   forced result (x AND 0 = 0; x and z never control);
/// - the controlling input's own edge is never masked, so its change always
///   re-evaluates the gate, which restores the mask with the current value
///   of the masked input in hand (same-delta changes of both inputs are
///   read together).
#[derive(Clone, Copy, Debug)]
struct CtlGate {
    /// The gate (`FusedGate::Bin2` with `GateBin::And` / `GateBin::Or`).
    op: FusedGate,
    /// The input whose controlling value masks the other one.
    other: BitRef,
    /// The masked (high-fanout) input's signal.
    hot: u32,
    /// `comb_dep_edges[pos..pos + npos]`: the edges from `hot` to the gate.
    pos: u32,
    npos: u32,
    /// Their static (bit-sensitivity) mask, restored when not controlled.
    smask: u64,
    /// Controlling bit code: 0 for AND/NAND, 1 for OR/NOR.
    ctl: u8,
}

/// Per-instance runtime state for a §29 UDP. Terminals are resolved to single
/// bit refs; sequential state and previous-input levels drive edge detection.
#[derive(Clone)]
struct UdpRuntime {
    udp_name: String,
    inst_path: String,
    out_ref: BitRef,
    in_refs: Vec<BitRef>,
    is_sequential: bool,
    delay: u64,
    /// Shared by every instance of the same UDP definition. The elaborator
    /// attaches rows to each flattened instance, but retaining those clones
    /// wastes substantial memory on cell fabrics.
    rows: Arc<[crate::ast::decl::UdpTableRow]>,
    /// Precomputed `(state, previous inputs, current inputs) -> next state`
    /// table for small UDPs. Values are 0, 1, or 2 (=x).
    transition_lut: Option<Arc<[u8]>>,
    /// Number of ternary input vectors (`3 ^ input_count`) in the LUT.
    lut_input_states: usize,
    /// Cached ternary encoding of the last input vector. The LUT path can
    /// compare and index this directly instead of rebuilding previous inputs.
    input_state_idx: usize,
    /// When only one input can cause a state transition, identify that port
    /// and the previous/current trit pairs worth evaluating. Other pairs can
    /// update the cached trit and return without reading the remaining ports.
    event_input: Option<u8>,
    event_transition_mask: u16,
    /// Current output/state level: 0,1,2(=x).
    state: u8,
    /// Previous input levels (0,1,2); 255 = uninitialised (first eval).
    prev_inputs: Vec<u8>,
    initialized: bool,
    /// True once we've warned about a no-match with a delayed multibit output.
    warned: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct BitRef {
    sig_id: u32,
    bit: u32,
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct FusedAndBranch {
    dst: BitRef,
    other: BitRef,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
enum GateBin {
    And,
    Or,
    Xor,
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
enum FusedGate {
    /// dst = src, or dst = ~src when invert
    Buf1 {
        dst: BitRef,
        src: BitRef,
        invert: bool,
    },
    /// dst = a op b, or dst = ~(a op b) when invert
    Bin2 {
        dst: BitRef,
        a: BitRef,
        b: BitRef,
        op: GateBin,
        invert: bool,
    },
    /// dst = s ? t : e
    Mux2 {
        dst: BitRef,
        s: BitRef,
        t: BitRef,
        e: BitRef,
    },
    /// Zero-delay combinational UDP lowered to a compact current-input LUT.
    /// Three ternary inputs fit in 54 packed bits, and the immutable node can use
    /// the same direct dependency dispatch as built-in fused gates.
    UdpLut3 {
        dst: BitRef,
        inputs: [BitRef; 3],
        input_count: u8,
        table: u64,
    },
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
struct VectorRef {
    sig_id: u32,
    lo: u32,
}

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
enum VectorGate {
    Buf {
        dst: VectorRef,
        src: VectorRef,
        width: u8,
        invert: bool,
    },
    Bin {
        dst: VectorRef,
        a: VectorRef,
        b: VectorRef,
        width: u8,
        op: GateBin,
        invert: bool,
    },
    Mux {
        dst: VectorRef,
        s: VectorRef,
        t: VectorRef,
        e: VectorRef,
        width: u8,
    },
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
enum ScatterGate {
    Buf {
        dsts: Box<[u32]>,
        src: VectorRef,
        width: u8,
        invert: bool,
    },
    Bin {
        dsts: Box<[u32]>,
        a: VectorRef,
        b: VectorRef,
        width: u8,
        op: GateBin,
        invert: bool,
    },
}

// See the §9.2.2.2 note in Simulator::collect_expr_reads' Call arm: set
// while collecting reads for a PLAIN `always` (star or explicit list),
// whose sensitivity excludes called-function bodies (only
// always_comb/always_latch descend into them).
/// §27.6: hierarchical refs through an UNNAMED generate block's implicit
/// `genblk<N>` name. Unnamed-branch declarations keep bare names (the
/// historical leakage other resolution depends on), so `genblk1.x`
/// resolves by stripping the implicit segments and retrying bare.
fn strip_genblk_segments(name: &str) -> Option<String> {
    if !name.starts_with("genblk") && !name.contains(".genblk") {
        return None;
    }
    let stripped: Vec<&str> = name
        .split('.')
        .filter(|seg| {
            !(seg.starts_with("genblk")
                && seg[6..].chars().all(|c| c.is_ascii_digit())
                && seg.len() > 6)
        })
        .collect();
    if stripped.is_empty() || stripped.len() == name.split('.').count() {
        return None;
    }
    Some(stripped.join("."))
}

/// Continuous-assignment targets indexed by leaf name, for the edge-select
/// alias sharing in `rewrite_edge_select_sensitivities` (built once, on the
/// first edge term that selects a bit of a vector).
mod edgesel_writers {
    use super::*;

    pub(super) fn base_ident(e: &Expression) -> Option<String> {
        match &e.kind {
            ExprKind::Paren(inner) => base_ident(inner),
            ExprKind::Index { expr, .. } | ExprKind::RangeSelect { expr, .. } => base_ident(expr),
            ExprKind::Ident(h) => Some(
                h.path
                    .iter()
                    .map(|s| s.name.name.as_str())
                    .collect::<Vec<_>>()
                    .join("."),
            ),
            _ => None,
        }
    }
    pub(super) fn leaf(n: &str) -> &str {
        n.rsplit('.').next().unwrap_or(n)
    }

    #[derive(Default)]
    pub(super) struct WriterIndex {
        /// Leaf name -> indices into `continuous_assigns`.
        pub cont: HashMap<String, Vec<usize>>,
        /// Leaf name (of the target, or of the parent net a port target
        /// connects to) -> indices into `pending_cont_assign`.
        pub pend: HashMap<String, Vec<usize>>,
        /// Leaf names some procedural `force` or `assign` (§10.6) targets:
        /// its bits need not follow their continuous drivers.
        pub forced: HashSet<String>,
    }

    /// The targets of `force` / procedural `assign` statements in `s`.
    fn proc_cont_targets<'a>(s: &'a Statement, out: &mut Vec<&'a Expression>) {
        use crate::ast::stmt::ProceduralContinuous as PC;
        match &s.kind {
            StatementKind::ProceduralContinuous(PC::Force { lvalue, .. })
            | StatementKind::ProceduralContinuous(PC::Assign { lvalue, .. }) => {
                match &lvalue.kind {
                    ExprKind::Concatenation(parts) => out.extend(parts.iter()),
                    _ => out.push(lvalue),
                }
            }
            StatementKind::If {
                then_stmt,
                else_stmt,
                ..
            } => {
                proc_cont_targets(then_stmt, out);
                if let Some(e) = else_stmt {
                    proc_cont_targets(e, out);
                }
            }
            StatementKind::Case { items, .. } => {
                for it in items {
                    proc_cont_targets(&it.stmt, out);
                }
            }
            StatementKind::For { body, .. }
            | StatementKind::Foreach { body, .. }
            | StatementKind::While { body, .. }
            | StatementKind::DoWhile { body, .. }
            | StatementKind::Repeat { body, .. }
            | StatementKind::ForeachTail { body, .. }
            | StatementKind::Forever { body }
            | StatementKind::ForeverTail { body }
            | StatementKind::RsAction { body } => proc_cont_targets(body, out),
            StatementKind::SeqBlock { stmts, .. } | StatementKind::ParBlock { stmts, .. } => {
                for x in stmts {
                    proc_cont_targets(x, out);
                }
            }
            StatementKind::TimingControl { stmt, .. } | StatementKind::Wait { stmt, .. } => {
                proc_cont_targets(stmt, out)
            }
            StatementKind::RandCase { items } => {
                for (_, x) in items {
                    proc_cont_targets(x, out);
                }
            }
            StatementKind::WaitOrder { pass, fail, .. } => {
                if let Some(p) = pass {
                    proc_cont_targets(p, out);
                }
                if let Some(f) = fail {
                    proc_cont_targets(f, out);
                }
            }
            _ => {}
        }
    }

    impl WriterIndex {
        pub(super) fn build(module: &ElaboratedModule) -> Self {
            let mut ix = WriterIndex::default();
            fn targets(e: &Expression) -> Vec<&Expression> {
                match &e.kind {
                    ExprKind::Concatenation(parts) => parts.iter().collect(),
                    _ => vec![e],
                }
            }
            for (i, ca) in module.continuous_assigns.iter().enumerate() {
                for t in targets(&ca.lhs) {
                    if let Some(b) = base_ident(t) {
                        ix.cont.entry(leaf(&b).to_string()).or_default().push(i);
                    }
                }
            }
            let mut forced: Vec<&Expression> = Vec::new();
            for b in &module.always_blocks {
                proc_cont_targets(&b.stmt, &mut forced);
            }
            for b in module
                .initial_blocks
                .iter()
                .chain(module.final_blocks.iter())
            {
                proc_cont_targets(&b.stmt, &mut forced);
            }
            for t in module.tasks.values() {
                for x in &t.items {
                    proc_cont_targets(x, &mut forced);
                }
            }
            for f in module.functions.values() {
                for x in &f.items {
                    proc_cont_targets(x, &mut forced);
                }
            }
            for e in forced {
                if let Some(b) = base_ident(e) {
                    ix.forced.insert(leaf(&b).to_string());
                }
            }
            let pend_sources = module
                .pending_always
                .iter()
                .map(|p| (&*p.source, &p.ctx))
                .chain(module.pending_initial.iter().map(|p| (&*p.source, &p.ctx)));
            for (src, ctx) in pend_sources {
                let mut f: Vec<&Expression> = Vec::new();
                proc_cont_targets(src, &mut f);
                for e in f {
                    if let Some(b) = base_ident(e) {
                        ix.forced.insert(leaf(&b).to_string());
                        if let Some(mb) = ctx.port_map.get(&b).and_then(base_ident) {
                            ix.forced.insert(leaf(&mb).to_string());
                        }
                    }
                }
            }
            for (i, p) in module.pending_cont_assign.iter().enumerate() {
                for t in targets(&p.lhs_source) {
                    let Some(b) = base_ident(t) else {
                        continue;
                    };
                    let mut leaves = vec![leaf(&b).to_string()];
                    if let Some(mb) = p.ctx.port_map.get(&b).and_then(base_ident) {
                        leaves.push(leaf(&mb).to_string());
                    }
                    leaves.dedup();
                    for l in leaves {
                        let v = ix.pend.entry(l).or_default();
                        if v.last() != Some(&i) {
                            v.push(i);
                        }
                    }
                }
            }
            ix
        }
    }
}

thread_local! {
    static SKIP_FN_BODY_READS: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct CombEntry {
    item: CombItem,
    /// Everything the settle HOT LOOP does not read, behind one pointer.
    ///
    /// The eval loop's `entries[eidx]` is an indirect access; at 144 bytes an
    /// entry straddled 2–3 cache lines and every evaluation paid the extra
    /// misses (4.5k evals per simulated cycle on the C906 SoC). With the
    /// three cold containers boxed the entry is 64 bytes — the match on
    /// `item` plus the two flags touch one line. `cold` is dereferenced only
    /// during entry construction, diagnostics, and rebuild passes.
    cold: Box<CombEntryCold>,
    /// True when dependency extraction could not resolve all read signals.
    /// Such entries are conservatively re-evaluated each settle pass.
    has_unresolved_reads: bool,
    /// True for a plain `always @(sensitivity_list)` block whose body can
    /// reach `$finish` / `$stop`. xezim routes level-only `always @(...)`
    /// blocks through the comb-settle path, which fires them at time 0
    /// with their (typically X) initial values — but per IEEE 1800
    /// §9.2.2.1 such a block actually suspends at its event control and
    /// only runs when a sensitivity input changes. Firing it at time 0
    /// is usually harmless (the block computes garbage that gets
    /// recomputed later), but if the body hits `$finish` the whole
    /// simulation aborts before the testbench has initialised its
    /// stimulus. So: don't seed these from the time-0 dirty set; let the
    /// first real sensitivity transition trigger them.
    defer_at_time0: bool,
}

/// Cold half of a `CombEntry` — see `CombEntry::cold`.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct CombEntryCold {
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
    /// Source span of the originating construct (cont-assign LHS or always
    /// block statement) so diagnostics — notably the settle-limit warning —
    /// can name file:line for a non-converging driver. `Span::dummy()` when
    /// the origin has no source (synthesized entries).
    span: crate::ast::Span,
    /// Code coverage counter of an interpreted continuous assignment (the
    /// compiled form counts with an `Insn::CovHit`). Never cached.
    #[serde(skip)]
    cov: Option<u32>,
}

const PREPARED_COMB_MAGIC: &[u8; 8] = b"XZCMB020";

/// One `comb_dep_edges` element: a dependent entry and the bits of the
/// signal it reads (`bytecode::sig_span_mask` chunks; all ones = any bit).
#[derive(Clone, Copy)]
struct DepEdge {
    mask: u64,
    entry: u32,
}

#[derive(serde::Serialize)]
struct PreparedCombCacheRef<'a> {
    signal_count: usize,
    entries: &'a [CombEntry],
    dep_offsets: &'a [u32],
    dep_entries: &'a [u32],
    unresolved_idx: &'a [usize],
    time0_idx: &'a [usize],
    time0_deferred: &'a [CombEntry],
    cont_driven: &'a HashSet<usize>,
}

#[derive(serde::Deserialize)]
struct PreparedCombCache {
    signal_count: usize,
    entries: Vec<CombEntry>,
    dep_offsets: Vec<u32>,
    dep_entries: Vec<u32>,
    unresolved_idx: Vec<usize>,
    time0_idx: Vec<usize>,
    time0_deferred: Vec<CombEntry>,
    cont_driven: HashSet<usize>,
}

#[derive(Debug, Clone)]
pub struct SimOutput {
    pub time: u64,
    pub message: String,
}

#[derive(Debug, Clone)]
struct NbaEntry {
    lhs: Option<Expression>,
    value: Value,
    resolved_id: Option<usize>,
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
    static_key: Option<String>,
    /// §10.4.2: frozen destination/source bit offsets and length for a
    /// selected persistent local. Merge at commit to preserve source order.
    static_slice: Option<(u32, u32, u32)>,
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
    this_handle: Option<usize>,
}

#[derive(Debug, Clone)]
struct JoinWaiter {
    parent_pid: usize,
    child_pids: HashSet<usize>,
    join_type: JoinType,
    continuation: ProcCont,
    finished_children: HashSet<usize>,
    /// `true` for a `wait fork` waiter. Unlike a plain `join`, its wake
    /// condition is re-evaluated against the *live* descendant tree on every
    /// child completion (LRM §9.6.1): it wakes only when the parent has zero
    /// remaining descendants, so it also waits for grandchildren spawned
    /// *after* the `wait fork` executed.
    wait_fork: bool,
}

/// Fast-path NBA entry: compact (signal_id, value) pair for pre-resolved targets.
/// 99%+ of NBA entries use this path. Smaller struct = better cache utilization.

/// IEEE 1800-2023 §9.7: info stored when a process is explicitly suspended
/// via `process::suspend()`. The process is removed from whatever queue it
/// was parked in; `resume()` uses this to re-schedule it.
#[derive(Clone)]
struct SuspendedProc {
    /// Continuation statements to run when the process resumes.
    continuation: ProcCont,
    /// Original scheduled expiry time if suspended while blocked on a `#delay`.
    /// `None` if blocked on an event/condition/join/mailbox. On resume, if the
    /// original delay has transpired (original_time <= self.time), the process
    /// continues immediately (LRM §9.7).
    original_delay_expiry: Option<u64>,
}

/// §9.7 `process::await()`: a process blocked waiting for another's termination.
struct AwaitWaiter {
    target_pid: usize,
    waiter_pid: usize,
    continuation: ProcCont,
}

/// Header of a two-state stream in `Simulator::ts_arena`.
#[derive(Clone, Copy, Default)]
struct TsHdr {
    off: u32,
    /// Stream length (13 bits) | executor kind (3 bits, `TS_HDR_KIND_SHIFT`);
    /// kind 0 = no arena stream. Eight bytes per entry keeps twice as many
    /// headers per cache line as the unpacked form did — the header load is
    /// the first touch of every settle evaluation.
    len_kind: u16,
    num_regs: u16,
}

const TS_HDR_KIND_SHIFT: u32 = 13;
const TS_HDR_LEN_MAX: usize = (1 << TS_HDR_KIND_SHIFT) - 1;

impl TsHdr {
    #[inline(always)]
    fn kind(&self) -> u8 {
        (self.len_kind >> TS_HDR_KIND_SHIFT) as u8
    }
    #[inline(always)]
    fn len(&self) -> usize {
        (self.len_kind & (TS_HDR_LEN_MAX as u16)) as usize
    }
}

/// `edge_block_armed` state bits. A gateable block skips its clock edge in
/// the scan prefilter iff its byte is ZERO: no input written since its last
/// execution (`EDGE_ARMED` clear) and nothing forcing a visit (`EDGE_HOLD`
/// clear — set while the block is not gateable, or has no valid value
/// snapshot and no wide read that lets it gate on arming alone). One byte
/// load per block visit instead of four flag arrays.
const EDGE_ARMED: u8 = 1;
const EDGE_HOLD: u8 = 2;

struct NbaFast {
    signal_id: usize,
    value: Value,
    /// Source block index. Used by the parallel-dispatch merge to
    /// recover sequential block-evaluation order before applying NBAs;
    /// without it the partition path could re-order writes across
    /// blocks and pick the wrong "last writer" for shared signals
    /// (see c910 cmark hang). Sequential / non-partition pushes leave
    /// this 0 — those paths already preserve insertion order.
    block_index: u32,
}

#[derive(Clone, Copy)]
struct ParallelBlockSlice {
    ptr: *const super::bytecode::Insn,
    len: usize,
    num_regs: usize,
    /// CompiledBlock::nba_dup_targets, carried across the thread boundary
    /// because the raw slice loses the owning block.
    nba_dup: bool,
}

unsafe impl Send for ParallelBlockSlice {}
unsafe impl Sync for ParallelBlockSlice {}

#[derive(Debug, Clone)]
struct EdgeSensitiveBlock {
    /// Pre-resolved signal IDs for O(1) edge checking. The unresolved
    /// `Vec<Sensitivity>` (with String name) used to live here too —
    /// dropped, since the only consumer (the init pass collecting
    /// edge_signal_ids/names) was rewritten to use this resolved
    /// list + id_to_name.
    resolved_sensitivities: Vec<SensitivityId>,
    /// The block body. Between classification and `compile_edge_blocks` a
    /// block built from a lazy child always block holds an empty statement
    /// with the body's span here and its source in `lazy`; compilation
    /// rebuilds the body and keeps it only where a later reader needs it
    /// (see `drop_compiled_edge_block_asts`).
    stmt: Statement,
    lazy: Option<Box<LazyAlways>>,
    kind: AlwaysKind,
    /// Instance scope the block was inlined under, empty for top-level.
    /// Preferred over sensitivity-derived scopes (a port-connected clock's
    /// sensitivity collapses to the PARENT's signal and yields none).
    scope: String,
}

/// A child-module always block of the `@(...) body` shape, kept as its
/// un-rewritten source (shared by every instance) plus this instance's
/// rewrite context instead of a per-instance statement tree: on a 64k-cell
/// DRAM array the 65k materialized trees were a third of the peak memory.
/// `patches` replace header event-term expressions exactly as the passes in
/// `Simulator::new` (edge-select aliases, computed edge terms) rewrote them;
/// nothing else in such a block is rewritten after elaboration.
#[derive(Debug, Clone)]
struct LazyAlways {
    src: super::elaborate::PendingAlways,
    patches: Vec<(usize, Expression)>,
}

impl LazyAlways {
    /// The always block exactly as the eager pipeline would have held it.
    fn materialize(&self) -> AlwaysBlock {
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
    fn materialize_body(&self) -> Statement {
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
enum DeferredAlways {
    Eager(AlwaysBlock),
    Lazy {
        lazy: LazyAlways,
        header: Option<Box<Statement>>,
        patched: Vec<usize>,
    },
}

#[derive(Debug, Clone)]
struct SensitivityId {
    signal_id: usize,
    edge: EdgeKind,
    /// LRM §9.4.2.3 `iff` guard for this event term: the edge only counts
    /// when this expression is true at edge time. `None` for an unguarded
    /// term. Consulted by the procedural `@`-event wake path; the always-
    /// block edge paths carry it but do not yet evaluate it. Boxed, like
    /// `value_of`: almost every term has neither, and an inline
    /// `Option<Expression>` made each term hundreds of bytes (68 MB of edge
    /// sensitivity lists on a 64k-cell DRAM).
    iff: Option<Box<Expression>>,
    /// §9.4.2: for a NON-TRIVIAL event expression (`@(a + b)`), the term's
    /// full expression. Operand-signal edges only COUNT when this
    /// expression's value differs from its value at arm time (stored in
    /// `EventWaiter::guard_prev`) — `a=2;b=1` leaving `a+b` at 3 must not
    /// fire. `None` for plain signal/edge terms.
    value_of: Option<Box<Expression>>,
    /// Scheduling depth of the NAME this term was written with (see
    /// `Simulator::sig_wake_rank`); orders same-pass waiter wakeups.
    wake_rank: u8,
    /// §9.4.2: an unpacked-array CELL that `value_of` selects through a
    /// runtime index (`@(mem[i])`), found by evaluating the index when the
    /// waiter armed. Re-resolved whenever an operand moves without firing,
    /// so the wait follows the index to the element it selects now.
    cell: bool,
}

#[derive(Debug, Clone)]
struct Sensitivity {
    signal_name: String,
    edge: EdgeKind,
    /// See `SensitivityId::iff` — carried through name→id resolution.
    iff: Option<Box<Expression>>,
    /// See `SensitivityId::value_of` — carried through name→id resolution.
    value_of: Option<Box<Expression>>,
}

#[derive(Debug, Clone, Default)]
struct EdgeFanout {
    posedge: Vec<usize>,
    negedge: Vec<usize>,
    anyedge: Vec<usize>,
    /// `@(edge x)` blocks — §9.4.2 LSB edge (posedge OR negedge of bit 0),
    /// NOT the whole-vector change that `anyedge` (`@(x)` level) fires on.
    lsbedge: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum EdgeKind {
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
    const fn print_str(&self) -> &'static str {
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
struct SemGetWaiter {
    pid: usize,
    /// Keys still required.
    n: i64,
    cont: ProcCont,
}

struct MailboxGetWaiter {
    pid: usize,
    lvalue: Expression,
    cont: ProcCont,
    /// `peek` (not `get`): the waiter reads the front WITHOUT removing it, so
    /// a `put` that wakes it must also leave the item in the mailbox for the
    /// subsequent `get`/`try_get`.
    is_peek: bool,
}

/// §15.4.1 — a process blocked on `mailbox.put(v)` because a BOUNDED mailbox is
/// full. The value is captured at the (blocking) call; when a `get`/`try_get`
/// frees a slot the value is stored and `cont` is rescheduled.
struct MailboxPutWaiter {
    pid: usize,
    value: Value,
    cont: ProcCont,
}

/// A process waiting for a signal edge event.
/// SIGUSR1 -> on-demand hang report from a live run (`kill -USR1 <pid>`).
/// The handler only flips a flag; the simulation loop prints the report at
/// the next iteration boundary (async-signal-safe).
static HANG_REPORT_REQUESTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn hang_report_signal_handler(_sig: libc::c_int) {
    HANG_REPORT_REQUESTED.store(true, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(unix)]
pub fn install_hang_report_handler() {
    unsafe {
        libc::signal(
            libc::SIGUSR1,
            hang_report_signal_handler as *const () as usize,
        );
    }
}

#[cfg(not(unix))]
pub fn install_hang_report_handler() {}

/// See `Simulator::event_list_shape`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventListShape {
    /// `@*`, `always_comb`, a named event, or no leading event control.
    Implicit,
    /// An explicit list whose every term is a plain identifier.
    Plain,
    /// An explicit list with a select, concatenation or expression term.
    NonPlain,
}

#[derive(Debug, Clone)]
struct EventWaiter {
    pid: usize,
    /// Simulation time when this waiter parked. Drives the hang report:
    /// waiters sorted oldest-first expose "who has been stuck longest",
    /// and `arm_bits` tells whether the awaited signals ever moved since.
    parked_time: u64,
    /// Pre-resolved signal IDs for O(1) edge checking. The unresolved
    /// `Vec<Sensitivity>` mirror was set at construction but never
    /// consulted afterwards — dropped.
    resolved_sensitivities: Vec<SensitivityId>,
    /// Value (raw v,x low-64) of each `resolved_sensitivities` signal at the
    /// moment this waiter armed. Parallel to `resolved_sensitivities`. Used to
    /// detect a change made AFTER arming within the same snapshot generation.
    arm_bits: Vec<(u64, u64)>,
    continuation: ProcCont,
    /// Each sensitivity signal's value captured AT registration time
    /// (`raw_bits()` for the ≤64-bit fast path). The waiter fires when the
    /// signal changes relative to this captured baseline — NOT the global
    /// per-tick `prev_val` snapshot. This ensures NBA updates that commit
    /// after registration in the same tick wake the waiter. It also
    /// subsumes the old `snap_gen` guard: a `forever @(posedge clk)`
    /// re-registered just after consuming a posedge captures clk's now-high value,
    /// so the consumed edge cannot re-fire (IEEE 1800-2023 §9.4.2.3).
    captured_prev: Vec<(u64, u64)>,
    /// Full captured value for >64-bit sensitivity signals (parallel to
    /// `captured_prev`); `None` for ≤64-bit signals.
    captured_prev_wide: Vec<Option<Value>>,
    /// §9.4.2 value guard for non-trivial event expressions — parallel to
    /// `resolved_sensitivities`. `Some(v)` holds the term expression's value
    /// at ARM time; an operand edge only counts when the expression now
    /// evaluates differently (see `SensitivityId::value_of`).
    guard_prev: Vec<Option<Value>>,
    /// Number of qualifying events still required before the continuation
    /// resumes. This collapses action-free `repeat (N) @(event);` loops into
    /// one parked waiter instead of rebuilding their AST continuation after
    /// every edge.
    remaining_events: u64,
    /// LRM §14.13: this waiter is parked on a CLOCKING event (`@(cb)` / `##N`),
    /// not a raw `@(posedge clk)`. Its continuation must resume in the Reactive
    /// region — AFTER the same-edge NBA updates commit and the clocking input
    /// snapshot refreshes — so it reads post-edge design state and this cycle's
    /// `cb.<in>` samples. Raw edge waiters resume in the active region right
    /// after the same edge's blocks (reference-verified default; see
    /// `waiters_first` in check_edges_inner); only clocking waiters are
    /// deferred past the NBA commit.
    is_clocking: bool,
    /// Set only for a waiter with expression terms (`SensitivityId::value_of`).
    value_ctx: Option<Box<WaiterValueCtx>>,
}

/// §9.4.2: how a waiter re-evaluates its expression terms after it armed.
#[derive(Debug, Clone)]
struct WaiterValueCtx {
    /// The name-resolution scope the waiting process armed in. The drain runs
    /// outside that process, and a bare `mem[i]` in a sub-module instance
    /// must still read that instance's `mem` and `i`.
    scope: Option<String>,
    /// Some term selects an unpacked-array cell (`@(mem[i])`): the cells are
    /// looked up again whenever an operand moves (see `SensitivityId::cell`).
    cells: bool,
}

/// A process parked on a CLASS-FIELD named event (`event m_event` inside a
/// class, awaited/triggered as `@m_event` / `->m_event` from a method on
/// `this`). Unlike a module-scope named event, a class `event` field has no
/// backing signal in `signal_table`, so it cannot flow through the edge-
/// sensitive `event_waiters` path. Each (owning instance handle, field name)
/// pair is a distinct synchronization object (IEEE 1800-2023 §15.5). The key is the
/// raw `(this_handle, field_name)`; `->`/`@` inside a method both resolve to
/// the same `this`, so a trigger and a wait on the same object match.
#[derive(Clone)]
enum InstanceEventKeys {
    One((usize, String)),
    Any(Vec<(usize, String)>),
}

impl InstanceEventKeys {
    fn contains(&self, key: &(usize, String)) -> bool {
        match self {
            Self::One(own) => own == key,
            Self::Any(keys) => keys.contains(key),
        }
    }
}

#[derive(Clone)]
struct InstanceEventWaiter {
    key: InstanceEventKeys,
    pid: usize,
    continuation: ProcCont,
}

/// Timing wheel for O(1) near-future event scheduling.
/// Events within WHEEL_SIZE ticks of current time use a circular array.
/// Events further out fall back to a BTreeMap.
const WHEEL_SIZE: usize = 4096;

/// Prefix of the hidden per-clocking-input sample mirror signals (§14.13).
/// Not expressible in SV source, so it can never collide with a user name.
const CB_SAMPLE_PREFIX: &str = "__xz_cbsample.";

/// IEEE 1800-2023 §14.4: `#1step` in the clocking skew maps. An input with no
/// declared skew samples with 1step too (§14.3 default), so the default-input
/// map holds only explicit non-1step skews.
const CB_SKEW_1STEP: u64 = u64::MAX;

/// §14.4 explicit input skew `#N` (N > 0): a net's values over the last N
/// ticks. `changes` holds `(slot, value at the end of that slot)` in time
/// order, `init` the value before the first recorded slot.
#[derive(Default)]
struct ClockingHist {
    init: Option<Value>,
    changes: VecDeque<(u64, Value)>,
    span: u64,
}
/// Number of u64 words needed for the occupancy bitmap (256 / 64 = 4).
const BITMAP_WORDS: usize = WHEEL_SIZE / 64;

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
struct ProcCont {
    stmts: Arc<[Statement]>,
    start: usize,
    next: Option<Arc<ProcCont>>,
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
    loop_tail_at: Option<usize>,
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
    fn from_vec(stmts: Vec<Statement>) -> Self {
        ProcCont {
            stmts: Arc::from(stmts),
            start: 0,
            next: None,
            loop_tail_at: None,
        }
    }

    /// A continuation over a shared statement slice.
    fn from_frame(stmts: Arc<[Statement]>) -> Self {
        ProcCont {
            stmts,
            start: 0,
            next: None,
            loop_tail_at: None,
        }
    }

    /// Nothing to run.
    fn empty() -> Self {
        thread_local! {
            static EMPTY: Arc<[Statement]> = Arc::from(Vec::new());
        }
        ProcCont {
            stmts: EMPTY.with(Arc::clone),
            start: 0,
            next: None,
            loop_tail_at: None,
        }
    }

    /// Resume at `idx` of THIS frame, keeping the rest of the chain. This is the
    /// operation that used to be a deep clone. `loop_tail_at` names a position
    /// in THIS SAME `stmts` array, so it carries over unchanged — moving the
    /// cursor doesn't move the tail.
    fn resume_at(&self, idx: usize) -> Self {
        ProcCont {
            stmts: Arc::clone(&self.stmts),
            start: idx.min(self.stmts.len()),
            next: self.next.clone(),
            loop_tail_at: self.loop_tail_at,
        }
    }

    /// Run `stmts` first, then this continuation from `resume_at`. Replaces
    /// "copy the caller's tail onto the end of the spliced body".
    fn pushed(&self, stmts: Vec<Statement>, resume_at: usize) -> Self {
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
        }
    }

    /// Like `pushed(vec![body.clone()], resume_at)`, but when `body` is
    /// `StatementKind::Null` (a standalone `#delay;` or `@(ev);` with no
    /// attached statement), skips allocating the 1-element `[Null]` wrapper
    /// frame and `Arc<ProcCont>` link and resumes directly at `resume_at`.
    fn pushed_body(&self, body: &Statement, resume_at: usize) -> Self {
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
            };
        }
        self.pushed(vec![body.clone()], resume_at)
    }

    /// Like `pushed`, but `stmts[tail_idx]` is a synthesized `While`/`Repeat`
    /// re-entry — the ONE occurrence of that kind in this frame allowed to
    /// consume a pending `break`/`continue` (see `loop_tail_at`'s doc
    /// comment on `ProcCont`).
    fn pushed_with_tail(&self, stmts: Vec<Statement>, resume_at: usize, tail_idx: usize) -> Self {
        let mut c = self.pushed(stmts, resume_at);
        c.loop_tail_at = Some(tail_idx);
        c
    }

    /// Run a previously prepared statement frame, then resume this chain.
    fn pushed_frame(&self, stmts: Arc<[Statement]>, resume_at: usize) -> Self {
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
        }
    }

    /// Like `pushed_frame`, but `stmts[tail_idx]` is a synthesized loop
    /// re-entry (see `pushed_with_tail`).
    fn pushed_frame_with_tail(
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
    fn frame(&self) -> &[Statement] {
        &self.stmts[self.start.min(self.stmts.len())..]
    }

    /// Nothing left here or behind.
    fn is_exhausted(&self) -> bool {
        self.start >= self.stmts.len() && self.next.is_none()
    }

    /// Statements still owed across the whole chain. Walks the chain — keep it
    /// off hot paths; it exists for diagnostics and for the few callers that
    /// must hand a flat list to code outside the process executor.
    fn len(&self) -> usize {
        let mut n = self.frame().len();
        let mut cur = self.next.as_ref();
        while let Some(f) = cur {
            n += f.frame().len();
            cur = f.next.as_ref();
        }
        n
    }

    fn is_empty(&self) -> bool {
        self.is_exhausted()
    }

    /// The next statement this continuation would execute.
    fn first(&self) -> Option<&Statement> {
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

    /// Flatten the chain. Diagnostics / snapshot paths only.
    fn to_vec(&self) -> Vec<Statement> {
        let mut out: Vec<Statement> = self.frame().to_vec();
        let mut cur = self.next.clone();
        while let Some(f) = cur {
            out.extend_from_slice(f.frame());
            cur = f.next.clone();
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

type EventList = VecDeque<(usize, ProcCont)>;

/// One clock-tree group's edge answer for the current detect pass.
///
/// Members of a group are exact copies of one driver, so both the fire decision
/// AND the raw value pair are shared: `fired_snap` (which re-baselines `prev`)
/// and the §9.4.2 bit-select re-test both need the values, and taking them from
/// the leader is what removes the scattered reads. `XEZIM_CLKTREE_PROBE=1`
/// verifies that equality holds rather than assuming it.
#[derive(Clone, Copy, Default)]
struct EdgeGroupMemo {
    generation: u64,
    fires_pos: bool,
    fires_neg: bool,
    fires_any: bool,
    cur_v: u64,
    cur_x: u64,
    prev_v: u64,
    prev_x: u64,
}

/// Built-in clock generator: replaces `always #N clk = ~clk` with O(1) toggle.
/// Eliminates AST cloning and traversal for the most common simulation pattern.
struct ClockGen {
    signal_id: usize,
    half_period: u64,
    next_toggle_time: u64,
    /// Position of `signal_id` inside `Simulator::edge_signal_ids`, or
    /// `usize::MAX` if this clock isn't in any edge sensitivity list (e.g.
    /// a `forever` generator driving only level-sensitive logic). Populated
    /// once at the end of elaboration via `seal_edge_signal_ids`; lets
    /// `fire_clock_generators` fill `toggled_clock_positions` in O(1).
    edge_signal_position: usize,
    /// §4.4.2 FIFO rank of the next toggle among the wheel events due at the
    /// same time: how many were already queued when the generator process
    /// would have scheduled it (at its previous toggle). Those resume before
    /// the toggle, later ones after it. `None` until the time-0 drain
    /// reaches the generator's source position (`t0_rank`).
    ahead: Option<u32>,
    /// The generator's slot in the time-0 activation order: the number of
    /// time-0 processes that run before it (see `order_time0_processes`).
    /// `usize::MAX` until that order is known.
    t0_rank: usize,
}

/// Reusable scheduler state for `always #(expr) lhs = rhs` when the delay is
/// dynamic. The generic Forever continuation rebuilds and clones its AST on
/// every wakeup; PLL models can execute that path millions of times. Keeping
/// one owned delay/body pair preserves per-iteration delay evaluation without
/// allocating a new continuation for every clock edge.
struct FastDelayAlways {
    delay: Expression,
    body: Statement,
    /// Pre-resolved bytecode for the retained assignment body. Unsupported
    /// constructs keep using `body` through the AST interpreter.
    compiled_body: Option<super::bytecode::CompiledBlock>,
    /// Pre-resolved delay expression and its result register.
    compiled_delay: Option<(super::bytecode::CompiledBlock, u16)>,
    /// Module precision relative to the global tick, cached on first arm.
    delay_precision_diff: Option<i32>,
    /// False for the initial activation, which only arms the first delay.
    /// True once a delayed activation should execute `body` before re-arming.
    execute_body: bool,
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
#[derive(Clone, Copy, Debug)]
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
struct NbaFastIndex {
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
    fn with_dense_prefix(dense_len: usize) -> Self {
        Self {
            dense_entries: vec![u32::MAX; dense_len],
            sparse_entries: HashMap::default(),
            dirty_dense: Vec::new(),
        }
    }

    #[inline]
    fn get(&self, id: usize) -> Option<usize> {
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
    fn insert(&mut self, id: usize, idx: usize) {
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

    fn clear(&mut self) {
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
    fn extend_dense(&mut self, total: usize) {
        if total > self.dense_entries.len() {
            self.dense_entries.resize(total, u32::MAX);
        }
    }
    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.dirty_dense.is_empty() && self.sparse_entries.is_empty()
    }
}

mod aggregate_assignments;
mod aggregate_expressions;
mod aggregate_members;
mod arguments;
mod assertions;
mod sva_until;
mod assignments;
mod bytecode_runtime;
mod calls;
mod class_access;
mod class_members;
mod class_methods;
mod class_specialization;
mod class_array_constants;
mod property_initialization;
mod clocking;
mod collection_methods;
mod collections;
mod comb_compilation;
mod compilation;
mod compiled_methods;
mod constraint_domains;
mod constraint_solver;
mod constraint_targets;
mod construction;
mod coverage;
mod dpi_imports;
mod edges;
mod event_loop;
mod expression_types;
mod expressions;
mod file_io;
mod formatting;
mod functions;
mod initialization;
mod inline_constraints;
mod inspection;
mod isolated_gates;
mod jit_runtime;
mod mailboxes;
mod method_types;
mod named_events;
mod process_control;
mod process_scopes;
mod processes;
mod profiling;
mod sensitivity;
mod settling;
mod setup;
mod signal_commits;
mod statement_assignments;
mod statements;
mod system_tasks;
mod two_state;
mod type_metadata;
mod udp;
mod value_display;
mod virtual_interfaces;
mod waveforms;
mod assert_ctl;
mod code_cov;
mod dpi_export;
mod dpi_task;
mod event_bits;
mod event_stamps;
mod module_paths;
mod names;
mod rand_csp;
mod rand_diag;

/// Marks a `dist` target recorded by MEMBER path in `rand_order_sensitive`.
const MEMBER_DIST_MARK: &str = "\u{1}member:";
mod rand_scope;
mod sv_file;
mod task_suspend;
mod timing_checks;
mod ts_x;
mod typename;
mod uvm_dpi;
mod vpi_api;
mod vpi_cb;
mod vpi_model;
use assert_ctl::{AssertCtl, CtlIdent, CtlSite};
pub(crate) use code_cov::{
    COND_FN as COV_COND_FN, HIT_TASK as COV_HIT_TASK, marker_id as cov_marker_id,
};
pub use code_cov::{
    CodeCoverage, KIND_BRANCH, KIND_STATEMENT, KIND_TOGGLE,
    parse_kinds as parse_code_coverage_kinds, set_code_coverage,
};
use event_stamps::{EventId, EventStampStore, NamedEventPlan, NamedEventTarget};
pub(crate) use names::{IdNames, NameMap};
pub use timing_checks::{set_no_notifier, set_no_tchk_msg, set_no_timing_checks};

#[cfg(test)]
mod store_write_mask_tests {
    use super::{name_bit, note_store_write, take_store_writes};

    /// Two simulations in one process each run on their own thread. A drain
    /// on one thread must not clear the writes the other noted: with one
    /// process-wide mask it did, and the other's parked `wait(cond)` never
    /// re-checked (UVM runs hung under `cargo test`, which runs tests
    /// concurrently).
    #[test]
    fn a_drain_on_one_thread_keeps_another_threads_writes() {
        let _ = take_store_writes();
        note_store_write(name_bit("pending"));
        std::thread::spawn(|| {
            let _ = take_store_writes();
            note_store_write(name_bit("other"));
            let _ = take_store_writes();
        })
        .join()
        .unwrap();
        assert_eq!(
            take_store_writes() & name_bit("pending"),
            name_bit("pending")
        );
        assert_eq!(take_store_writes(), 0);
    }

    /// A thread's first drain sees every bit: writes made before it existed
    /// cannot be known, so its first look must re-check everything.
    #[test]
    fn a_new_threads_first_drain_sees_every_bit() {
        let first = std::thread::spawn(take_store_writes).join().unwrap();
        assert_eq!(first, u64::MAX);
    }

    #[test]
    fn simultaneous_drains_preserve_each_threads_mask() {
        let barrier = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            for name in ["worker_left", "worker_right"] {
                let barrier = &barrier;
                scope.spawn(move || {
                    let _ = take_store_writes();
                    let mut mismatches = 0;
                    for _ in 0..64 {
                        note_store_write(name_bit(name));
                        barrier.wait();
                        let observed = take_store_writes();
                        barrier.wait();
                        mismatches += usize::from(observed != name_bit(name));
                        mismatches += usize::from(take_store_writes() != 0);
                    }
                    assert_eq!(mismatches, 0);
                });
            }
        });
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

/// Per-tick wall-time accumulators that the event_loop driver carries
/// across iterations.  Extracted from `event_loop` so `run_one_tick`
/// can be called as a unit.
#[derive(Default, Debug)]
struct PerTickAccum {
    t_settle: u64,
    t_edges: u64,
    t_nba: u64,
    t_process: u64,
    t_snap: u64,
    t_sched: u64,
}

struct TimingWheel {
    wheel: Vec<EventList>,              // circular array of WHEEL_SIZE slots
    bitmap: [u64; BITMAP_WORDS],        // occupancy bitmap: bit set = slot non-empty
    overflow: BTreeMap<u64, EventList>, // far-future events
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
    last_pop_front: bool,
}

impl TimingWheel {
    fn new() -> Self {
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
    fn slot_pids(&self, time: u64) -> Vec<usize> {
        self.wheel[Self::slot(time)].iter().map(|e| e.0).collect()
    }

    /// Stable-sort the wheel slot of `time` by a per-pid rank (pids without
    /// one keep their place ahead of the ranked ones).
    fn reorder_slot(&mut self, time: u64, rank: &HashMap<usize, usize>) {
        self.wheel[Self::slot(time)]
            .make_contiguous()
            .sort_by_key(|e| rank.get(&e.0).map_or(0, |r| r + 1));
    }

    /// Number of events queued for exactly `time`.
    fn len_at(&self, time: u64) -> usize {
        let mut n = self.overflow.get(&time).map_or(0, |l| l.len());
        if time >= self.current_time && time < self.current_time + WHEEL_SIZE as u64 {
            n += self.wheel[Self::slot(time)].len();
        }
        n
    }

    #[inline(always)]
    fn slot(time: u64) -> usize {
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
    fn schedule(&mut self, time: u64, pid: usize, stmts: ProcCont) {
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
    fn schedule_front(&mut self, time: u64, pid: usize, stmts: ProcCont) {
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
    fn requeue_front(&mut self, time: u64, rest: EventList) {
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

    fn is_empty(&self) -> bool {
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
    fn next_time(&self) -> Option<u64> {
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
    fn next_time_after(&self, after: u64) -> Option<u64> {
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
    fn pop_front(&mut self, time: u64) -> Option<(usize, ProcCont)> {
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

    /// Remove and return all events at the given time.
    fn remove(&mut self, time: u64) -> EventList {
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
    fn has_pid(&self, pid: usize) -> bool {
        self.pid_counts.contains_key(&pid)
    }

    /// The pending continuation scheduled for `pid`, if any. Linear scan of
    /// every wheel slot + the overflow map — used ONLY by the terminal
    /// zero-delay stall report, never on the scheduling hot path.
    /// Every pending (pid, absolute_time) in the wheel + overflow. Read-only,
    /// used ONLY by the terminal stall report to show where non-spinning
    /// processes are parked (e.g. the initializer the spinner is waiting for,
    /// scheduled at a future time the livelock never reaches).
    fn pending_pid_times(&self) -> Vec<(usize, u64)> {
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

    fn pending_stmts_for(&self, pid: usize) -> Option<&ProcCont> {
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
    fn remove_pid(&mut self, pid: usize) -> Option<(u64, ProcCont)> {
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

thread_local! {
    /// Names written since the condition-waiter drain last looked, as a
    /// 64-bit filter over `name_bit`. Set by every mutation of the
    /// STRING-KEYED stores a parked `wait(cond)` can depend on: class
    /// properties (`PropMap`), class statics (`StaticMap`), the runtime signal
    /// map (`SignalMap`), and every procedural / name-based NBA assignment. A
    /// write that does not name its target sets every bit. The id-based RTL
    /// signal table is deliberately NOT counted: a clock toggle must not
    /// re-arm the UVM housekeeping waits (see `drain_condition_waiters`).
    ///
    /// Per THREAD, not per process: a simulation runs on one thread
    /// (`simulate_multi` gives it its own), and two simulations in one
    /// process — the test harness runs them concurrently — shared the old
    /// process-wide mask, so one's drain cleared the bits the other had just
    /// set; that one's parked `wait(cond)` then never re-checked and its run
    /// hung. Starts full, so a thread's first drain re-checks everything.
    static STORE_WRITE_MASK: std::cell::Cell<u64> = const { std::cell::Cell::new(u64::MAX) };
}

/// The `STORE_WRITE_MASK` bits of a property / variable name: two bits of
/// an FNV-1a hash, so two short names rarely share both.
#[inline]
fn name_bit(n: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in n.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    (1u64 << (h & 63)) | (1u64 << ((h >> 6) & 63))
}

/// Two filter bits of a formal-metadata key's base name (the text before the
/// first `.`), from its first two bytes: constant time on every write (see
/// `Simulator::meta_note`).
#[inline]
fn meta_key_bits(key: &str) -> u64 {
    let b = key.as_bytes();
    let b0 = b.first().copied().unwrap_or(0);
    let b1 = match b.get(1) {
        Some(&c) if c != b'.' => c,
        _ => 0,
    };
    (1u64 << (b0 & 63)) | (1u64 << (b1 & 63))
}

#[inline]
fn note_store_write(bits: u64) {
    STORE_WRITE_MASK.with(|m| m.set(m.get() | bits));
}

/// A write to the string-keyed stores that does not name its target.
#[inline]
fn bump_store_gen() {
    note_store_write(u64::MAX);
}

/// The names written since the last call.
fn take_store_writes() -> u64 {
    STORE_WRITE_MASK.with(|m| m.replace(0))
}

/// A class object's property map. Reads deref to the plain map; every
/// mutating entry point notes the property name in `STORE_WRITE_MASK`, so no
/// write site can be missed by the condition-waiter gate.
#[derive(Debug, Clone, Default)]
struct PropMap(HashMap<String, Value>);

impl PropMap {
    #[inline]
    fn insert(&mut self, k: String, v: Value) -> Option<Value> {
        note_store_write(name_bit(&k));
        self.0.insert(k, v)
    }
    #[inline]
    fn get_mut(&mut self, k: &str) -> Option<&mut Value> {
        note_store_write(name_bit(k));
        self.0.get_mut(k)
    }
    #[inline]
    fn remove(&mut self, k: &str) -> Option<Value> {
        note_store_write(name_bit(k));
        self.0.remove(k)
    }
}

/// Class static properties (`Class::prop` keys), noted like `PropMap`.
#[derive(Debug, Clone, Default)]
struct StaticMap(HashMap<String, Value>);

impl StaticMap {
    #[inline]
    fn insert(&mut self, k: String, v: Value) -> Option<Value> {
        note_store_write(name_bit(k.rsplit("::").next().unwrap_or(&k)));
        self.0.insert(k, v)
    }
}

/// §8.25/§6.20.2 (issue #423): the materialized elements of one class-scope
/// ARRAY constant of one specialization — a header VALUE parameter declared
/// with unpacked dimensions (`class C #(int unsigned TBL [4] = '{...})`,
/// read `C#(..)::TBL[i]`) or a class-body `localparam` array (`LP[i]`).
/// Lazily built on first element read by
/// `Simulator::materialize_class_param_array`; `lo`/`hi` preserve bounds;
/// `default` carries the declared element type for invalid/unknown indices.
#[derive(Debug, Clone)]
struct ClassParamArray {
    lo: i64,
    hi: i64,
    descending: bool,
    default: Value,
    elems: Vec<Value>,
}

/// §8.3.2/§6.20.2 (issue #388): a class-scope ARRAY constant resolved for a
/// WHOLE-COLLECTION context — `$size`, `inside`, `foreach`, whole-array copy —
/// spelled `pkg::Cls::T`, `Cls::T` from a foreign class, or bare `T` inside a
/// method of the declaring class. Holds the bounds plus the element source:
/// `param_cell` constants (class-body `localparam` arrays, header value
/// parameters with unpacked dims) read the materialized `class_param_arrays`
/// cell named by `key`; a static fixed-array property reads its per-element
/// signals `Cls::name[i]` (also named by `key`).
struct ClassScopeArrayConst {
    key: String,
    /// Normalized bounds (min, max) — the indexing domain.
    lo: i64,
    hi: i64,
    /// Declared bounds in source order (`[5:4]` gives (5, 4)) — drives
    /// `$left`/`$right`/`$increment` and the `foreach` visit order. Falls
    /// back to the normalized pair when no declared-dimension row exists.
    left: i64,
    right: i64,
    param_cell: bool,
}

impl std::ops::Deref for StaticMap {
    type Target = HashMap<String, Value>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::Deref for PropMap {
    type Target = HashMap<String, Value>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a> IntoIterator for &'a PropMap {
    type Item = (&'a String, &'a Value);
    type IntoIter = <&'a HashMap<String, Value> as IntoIterator>::IntoIter;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

/// Property -> (class-table key, definition) of the first class on a class's
/// `extends` chain that declares it (see `Simulator::prop_owners`).
type PropOwners = HashMap<String, (String, Arc<super::elaborate::ElaboratedClass>)>;

/// Property names some class declares with a given property kind; a name in
/// none of them skips the per-class checks for that kind.
struct ClassMemberNames {
    statics: HashSet<String>,
    /// Cheap prefilter for selected static-property accesses.
    static_first: [bool; 256],
    vif_props: HashSet<String>,
    /// First bytes of the `vif_props` names: a cheap prefilter for the
    /// member-chain checks that ask whether a name can be a vif property.
    vif_first: [bool; 256],
    /// Every declared method name, of any kind.
    methods: HashSet<String>,
    /// Every declared task name.
    tasks: HashSet<String>,
    /// Every property declared as a packed vector (`bit [3:0][7:0] p`).
    packed_vec_props: HashSet<String>,
    /// Every method name some class declares as a function returning
    /// `string`.
    string_methods: HashSet<String>,
    static_methods: HashSet<String>,
    /// Every function/task key and each of its suffixes after a `.`.
    subroutine_suffixes: HashSet<String>,
    /// Typedef names (module or class scope) whose target a specialization
    /// key spells by type (`typedef_alias_frag`): a virtual interface, a
    /// one-keyword built-in, or a plain alias of another name.
    alias_typedefs: HashSet<String>,
    /// Typedef names (module or class scope) whose target is a class-scoped
    /// type (`typedef R::elem_t et_t;`).
    scoped_typedefs: HashSet<String>,
    /// Names that may prefix a class-scope reference without being a
    /// class: class parameters and typedef names.
    scope_prefixes: HashSet<String>,
    /// First bytes of the `params` names: a cheap prefilter.
    param_first: [bool; 256],
    /// Every class type/value parameter name.
    params: HashSet<String>,
    /// Every associative-array property name.
    assoc_props: HashSet<String>,
    /// Every `string` property name.
    string_props: HashSet<String>,
    /// Every static property and class `localparam` name: the only names
    /// `static_prop_key` can resolve.
    static_or_param: HashSet<String>,
    /// Names of class constant arrays and fixed static arrays.
    constant_arrays: HashSet<String>,
    /// Names `plain_ident_read` leaves to the full path: static, vif and
    /// struct-capable properties, and the UVM activity constants.
    ident_slow: HashSet<String>,
}

/// A foreach index variable's outer state, restored when the loop exits:
/// (name, value, lived in a frame, typedef binding, trusted mark, packed
/// struct layout).
type LoopVarSnapshot = (
    String,
    Option<Value>,
    bool,
    Option<String>,
    bool,
    Option<Vec<(String, u32, u32)>>,
    (Option<u32>, bool),
);

/// `instance_assoc_member`'s class-only verdict for a bare name inside a
/// method of a given runtime class (class tables are fixed at run time).
enum MemberCollKind {
    /// A collection member: per-instance (`None`) or a static fixed array of
    /// the named owner class.
    Coll(Option<String>),
    /// A static collection somewhere in the chain.
    StaticColl,
    /// Neither: a per-instance collection only through a declared type that
    /// carries a collection dimension. `raws` are the raw declared type names
    /// met on the chain walk (a type-parameter binding for one of them
    /// changes the answer); `unbound` is the verdict with no binding applied.
    Plain { raws: Vec<String>, unbound: bool },
}

#[derive(Debug, Clone)]
struct ClassInstance {
    class_name: String,
    properties: PropMap,
    /// Maps a class TYPE-parameter name (e.g. `T` in
    /// `class Mk #(type T=Base)`) to the concrete class name it was
    /// specialized with (e.g. `Base`). Populated by
    /// `instantiate_class_with_type_args`. Used so an unqualified `obj = new()`
    /// unqualified `obj = new()` whose declared type is a type parameter
    /// constructs the bound concrete class (running its real `new`). Empty
    /// for classes without type parameters / non-specialized instances.
    type_bindings: HashMap<String, String>,
    /// The active specialization (`(base_class, sig)`) the instance was
    /// constructed under, captured so a later VIRTUAL method call on the
    /// instance can restore it (current_spec is otherwise lost once the
    /// constructing static call returns). Used by the dispatch override in
    /// `exec_method_in_class_hierarchy`. `None` for placeholder/stub
    /// instances and unspecialized constructions.
    spec: Option<(String, String)>,
    /// §23.8 / §23.10.1: the INSTANCE scope whose process ran `new()`.
    /// Hierarchical names in this object's methods resolve upward from
    /// here, not from the caller's scope — a checker class constructed
    /// inside a bound module must see its binder's host even when the
    /// method is invoked from the top level (e.g. through a handle held
    /// in an associative array). Empty for objects built at top scope.
    creation_scope: String,
}

/// §16.4 a deferred immediate assertion's pending report: the action block
/// to run when it matures, unless its process resumes first (a flush).
#[derive(Debug, Clone)]
struct DeferredReport {
    pid: usize,
    kind: crate::ast::stmt::DeferredAssertion,
    /// The assertion statement, which identifies it for the flush of a
    /// re-executed `always_comb`.
    span: crate::ast::Span,
    passed: bool,
    action: Option<Statement>,
    /// §16.3: a failed assertion with no `else` clause reports the default
    /// `$error`, naming this scope (`%m` where the assertion executed).
    default_error_scope: Option<String>,
    /// The scope of the comb-entry `always_comb` that queued the report
    /// (`pid` is then `usize::MAX`).
    comb_scope: Option<String>,
    this_handle: Option<usize>,
    class_context: Option<String>,
    /// §20.12: what an assertion control selects the assertion by.
    ctl: CtlIdent,
}

/// LRM §4.4 / §16 observed-region probe entry. A concurrent property's
/// per-cycle re-evaluation registers one of these into
/// `Simulator::pending_observed` from its sensitivity trigger (e.g. a
/// `@(posedge clk)` event). When the observed-region drain runs, the
/// runtime evaluates `predicate` and records the boolean outcome under
/// `name` in `assertion_stats`. Today only the storage + drain are in
/// place; no producer wires probes up. M9 (SVA elaboration) is the
/// consumer-side follow-up.
#[derive(Debug, Clone)]
struct ObservedProbe {
    name: String,
    predicate: Expression,
    /// Optional `this` handle so a property declared in a class context can
    /// resolve member references at evaluation time.
    this_handle: Option<usize>,
}

/// §9.4.2: one term in an assertion clocking event, including its last
/// value. A simple signal keeps its resolved name for the common fast path.
#[derive(Debug, Clone)]
struct SvaClockEvent {
    event: EventExpr,
    signal: Option<String>,
    previous: Value,
}

/// LRM §16.5 SVA clocked property site. Registered once when an
/// `assert property (@clk body)` statement first executes; thereafter
/// any term in its clocking event drives per-cycle evaluation of `body`.
/// Every clock tick starts one attempt of `node`; `attempts` holds the ones
/// still in flight (multi-cycle sequences, deferred consequents).
#[derive(Debug, Clone)]
struct SvaClockedSite {
    /// Unique identifier (hash of span position) so dedup across
    /// re-executions is cheap.
    span_key: usize,
    /// Key of the site's `assertion_stats` tally, and its location.
    stat_key: usize,
    stat_loc: (Option<u32>, usize),
    /// Instance scope of the registering process: every instance of a
    /// module or interface shares the assertion's span.
    dedup_scope: String,
    /// 0 assert, 1 assume, 2 cover (§16.5): a cover miss is not a failure
    /// and fires no else-action; a cover match fires the pass action.
    kind: u8,
    /// §16.14.3 `cover sequence`: an attempt counts every match of the
    /// sequence and lives on while it can still match.
    all_matches: bool,
    /// The first term's simple signal, if any, for sampled calls using the
    /// single-signal history path. `events` drives assertion evaluation.
    clock_signal: String,
    /// The first term's edge: 0 posedge, 1 negedge, 2 any change.
    edge: u8,
    /// §16.5: a tick is the union of the per-term edge/iff events.
    events: Vec<SvaClockEvent>,
    /// The bare signal names `$past` reads in the body (sorted, unique), found
    /// once at registration: the walk ran on every clock fire.
    past_names: Vec<String>,
    /// The body compiled to an evaluable property tree (see `SvaNode`).
    node: std::sync::Arc<SvaNode>,
    /// §16.12 `disable iff (guard)`: while the guard holds, every attempt in
    /// flight is cancelled and no new one starts. Evaluated with the current
    /// (unsampled) values, as the LRM's asynchronous reset semantics require.
    disable: Option<Expression>,
    /// The clock event happened and the attempts it starts have not been
    /// evaluated yet (`sva_note_edges`).
    fired: bool,
    /// Attempts still in flight, oldest first.
    attempts: Vec<SvaState>,
    /// LRM §16.9.3 `$past(<signal>, N)` per-signal value history.
    /// Each entry is `signal_name → ring of sampled values (index 0 =
    /// this cycle, index N = N cycles ago)`. Refreshed at every clock fire
    /// BEFORE any attempt advances.
    past_snapshots: HashMap<String, std::collections::VecDeque<Value>>,
    /// LRM §16.5 pass/fail action blocks (`assert property (...) <pass>
    /// else <fail>;`). Run when the property tallies a non-vacuous
    /// pass / a fail at a clock fire — so a failing concurrent assertion
    /// actually reports (e.g. its `else $error(...)`), not just tallies.
    pass_action: Option<Statement>,
    fail_action: Option<Statement>,
    /// Instance scope the assertion is declared in, and the pid its action
    /// blocks run under, so `%m` and severity messages name that scope
    /// rather than whichever process ran last.
    scope: String,
    action_pid: usize,
    /// §21.2.1.7: the lexical chain below `scope` — enclosing named blocks
    /// and the assertion's own label — that `%m` and severity messages in
    /// the action blocks name (`tb.ap`).
    m_chain: Vec<String>,
    /// LRM §16.5.1 — signal ids referenced anywhere in `body`. Their
    /// slot-entry (Preponed) values are cached in `Simulator::sva_preponed`
    /// and swapped in while this site's predicate is evaluated, so the
    /// property samples pre-edge values through the sampled-value overlay.
    sampled_ids: Vec<usize>,
    /// §16.5.1: the selects of unpacked arrays in `body` whose indices read
    /// signals (`a[k]`), each with its array's (first cell id, low index,
    /// high index) when it selects an element of a 1-D array. The cells they
    /// name are renamed in each Preponed region and kept in `sampled_ids`
    /// after its first `fixed` entries.
    sels: Vec<(Expression, Option<(usize, i64, i64)>)>,
    fixed: usize,
    /// §16.10: the initial values of the attempt's local variable slots
    /// (empty when the property has none).
    lv_init: SvaEnv,
    /// §16.14.6: a procedural concurrent assertion. An attempt starts only
    /// for an execution of the statement (`matured`), not at every tick.
    procedural: bool,
    /// §16.14.6.1: the automatic variables the property reads, captured
    /// into these slots when the statement executes.
    captured: Vec<(usize, String)>,
    /// §16.14.6.2: matured instances, each the slot values it captured,
    /// waiting for the next tick of the leading clock.
    matured: Vec<SvaEnv>,
    /// The property has its own `disable iff` (§16.15: the scope's
    /// `default disable iff` does not apply).
    explicit_disable: bool,
    /// The full path of the scope the assertion executes in, which selects
    /// the `default disable iff` that governs it.
    disable_scope: String,
    /// §20.12: the site's identity for assertion control, and its state.
    ctl: CtlSite,
}

/// A sequence (§16.9) as parsed, before it is turned into an automaton.
#[derive(Debug, Clone)]
enum SvaSeq {
    /// A boolean: holds at one cycle.
    Bool(Expression),
    /// `##[min:max]` (`max == u64::MAX` unbounded).
    Delay(u64, u64),
    Concat(Vec<SvaSeq>),
    Or(Box<SvaSeq>, Box<SvaSeq>),
    And(Box<SvaSeq>, Box<SvaSeq>),
    Intersect(Box<SvaSeq>, Box<SvaSeq>),
    Throughout(Expression, Box<SvaSeq>),
    Within(Box<SvaSeq>, Box<SvaSeq>),
    FirstMatch(Box<SvaSeq>),
    /// `s[*min:max]`.
    Rep {
        seq: Box<SvaSeq>,
        min: u64,
        max: u64,
    },
    /// `b[->min:max]`.
    Goto {
        cond: Expression,
        min: u64,
        max: u64,
    },
    /// `b[=min:max]`.
    NonCon {
        cond: Expression,
        min: u64,
        max: u64,
    },
    /// §16.12.12 `left until right` / `left s_until right`: matches the
    /// tick `right` first matches, provided `left` held at every earlier
    /// tick.
    Until(Box<SvaSeq>, Box<SvaSeq>),
    /// §16.10 / §16.11 match items, run within the cycle: local variable
    /// assignments and subroutine calls (no cycle of its own).
    Assign(Vec<SvaMatchItem>),
}

/// The local variable values of one thread of an attempt (§16.10), indexed
/// by the slots the site's locals were bound to; empty for a site with no
/// locals.
type SvaEnv = Vec<Value>;

/// A §16.10 / §16.11 sequence match item.
#[derive(Debug, Clone)]
struct SvaMatchItem {
    /// The assigned local `(slot, width, signed)`; `None` for a subroutine
    /// call run for its side effect.
    slot: Option<(usize, u32, bool)>,
    expr: Expression,
}

/// Sequence automaton: a cycle advances over `Tick` edges; every other edge
/// is followed within the cycle. The accept state is reached at the cycle a
/// match ends.
#[derive(Debug, Clone)]
struct SvaNfa {
    states: Vec<Vec<SvaEdge>>,
    start: usize,
    accept: usize,
}

#[derive(Debug, Clone)]
enum SvaEdge {
    Eps(usize),
    /// Follow when the boolean holds at this cycle.
    Check(Expression, usize),
    /// Follow at the next cycle.
    Tick(usize),
    /// A compound sub-sequence: followed at every cycle it reports a match.
    Sub(Box<SvaCompound>, usize),
    /// Run the match items, then follow (§16.10 / §16.11).
    Assign(Vec<SvaMatchItem>, usize),
}

#[derive(Debug, Clone)]
enum SvaCompound {
    /// Both match; ends when the later one ends (§16.9.5).
    And(SvaNfa, SvaNfa),
    /// Both match and end at the same cycle (§16.9.6).
    Intersect(SvaNfa, SvaNfa),
    /// The boolean holds at every cycle of the sequence (§16.9.9).
    Throughout(Expression, SvaNfa),
    /// Only the earliest match (§16.9.7).
    FirstMatch(SvaNfa),
    /// §16.7: a counted cycle delay, with no state unrolling or clipping.
    Delay(u64, u64),
    /// §16.9.2: counted copies of the unit, one cycle apart.
    Repeat(SvaNfa, u64, u64),
    /// §16.12.12 `left until right` / `left s_until right`: matches the
    /// tick `right` first matches, provided `left` held at every earlier
    /// tick.
    Until { left: SvaNfa, right: SvaNfa },
}

/// A sequence in flight: `pcs` are the states waiting for the next cycle,
/// `subs` the compound edges being followed.
#[derive(Debug, Clone, Default)]
struct SvaRun {
    started: bool,
    /// The states waiting for the next cycle, each with its thread's local
    /// variables (§16.10).
    pcs: Vec<(usize, SvaEnv)>,
    subs: Vec<SvaSubRun>,
    /// The local variables the run starts with.
    init_env: SvaEnv,
}

impl SvaRun {
    fn with_env(init_env: SvaEnv) -> Self {
        SvaRun {
            init_env,
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone)]
struct SvaSubRun {
    /// `(state, edge)` of the `Sub` edge in the enclosing automaton.
    at: (usize, usize),
    target: usize,
    runs: Vec<SvaRun>,
    /// Which operand of an `and` has matched so far.
    ever: [bool; 2],
    /// The local variables the compound was entered with, and those of each
    /// operand's latest match (§16.10: an operand's assignments flow out).
    env: SvaEnv,
    ever_env: [SvaEnv; 2],
    ticks: u64,
    repeated: Vec<(u64, SvaRun)>,
}

/// A property body compiled for evaluation (§16.12).
#[derive(Debug, Clone)]
enum SvaNode {
    /// A sequence used as a property: holds iff it has a match. `strong`
    /// (§16.12.2): an attempt still in flight at the end of simulation fails.
    Seq(SvaNfa, bool),
    /// `not p`.
    Not(Box<SvaNode>),
    /// `ante |-> cons` (overlap) / `ante |=> cons`: every match of the
    /// antecedent starts one evaluation of the consequent.
    Impl {
        ante: SvaNfa,
        overlap: bool,
        cons: Box<SvaNode>,
    },
    /// §16.12.9: at least one antecedent match has a successful consequent.
    Followed {
        ante: SvaNfa,
        overlap: bool,
        cons: Box<SvaNode>,
    },
    And(Box<SvaNode>, Box<SvaNode>),
    Or(Box<SvaNode>, Box<SvaNode>),
    Iff(Box<SvaNode>, Box<SvaNode>),
    /// §16.12.12: arbitrary properties are started at each candidate tick.
    Until {
        left: Box<SvaNode>,
        right: Box<SvaNode>,
        strong: bool,
    },
    /// §16.12.6 `s_eventually e`: holds once `e` samples true.
    Eventually(Expression),
    /// §16.12.6 `s_always e`: fails the first tick `e` samples false.
    Always(Expression),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SvaOutcome {
    Pass,
    /// A vacuous success (§16.14.8), including an implication whose
    /// antecedent never matched or whose consequents all passed vacuously.
    Vacuous,
    Fail,
}

/// Per-attempt evaluation state mirroring `SvaNode`.
#[derive(Debug, Clone)]
enum SvaState {
    Seq(SvaRun),
    Not(Box<SvaState>),
    Impl {
        ante: SvaRun,
        ante_alive: bool,
        nonvacuous: bool,
        /// Consequent evaluations `(ticks until they start, state)`.
        cons: Vec<(u32, SvaState)>,
    },
    Followed {
        ante: SvaRun,
        ante_alive: bool,
        nonvacuous: bool,
        cons: Vec<(u32, SvaState)>,
    },
    And {
        a: Box<SvaState>,
        b: Box<SvaState>,
        ra: Option<SvaOutcome>,
        rb: Option<SvaOutcome>,
    },
    Or {
        a: Box<SvaState>,
        b: Box<SvaState>,
        ra: Option<SvaOutcome>,
        rb: Option<SvaOutcome>,
    },
    Until {
        env: SvaEnv,
        /// Only ticks with an evaluation still pending, or whose failure
        /// (*left* failed) settled the attempt, are retained. Ticks whose
        /// left held and right has failed are folded into `nonvacuous` and
        /// dropped, so the list stays bounded by the pending evaluations
        /// (issue #510); the folded ticks keep the prefix-holds invariant.
        ticks: VecDeque<SvaUntilTick>,
        /// Nonvacuity contributed by folded ticks (and live ones, see
        /// `sva_advance_until`): §16.14.8, either operand can be nonvacuous.
        nonvacuous: bool,
        closed: bool,
    },
    /// `s_eventually` / `s_always`, with the attempt's local variables.
    Leaf(SvaEnv),
}

/// One candidate start point of a property-valued until operand.
#[derive(Debug, Clone)]
struct SvaUntilTick {
    left: SvaState,
    right: SvaState,
    left_result: Option<SvaOutcome>,
    right_result: Option<SvaOutcome>,
}

/// Thompson-style construction of `SvaNfa` fragments.
struct SvaNfaBuilder {
    states: Vec<Vec<SvaEdge>>,
}

enum SvaRepUnit<'a> {
    Seq(&'a SvaSeq),
    Goto(&'a Expression),
}

impl SvaNfaBuilder {
    fn state(&mut self) -> usize {
        self.states.push(Vec::new());
        self.states.len() - 1
    }

    fn add(&mut self, from: usize, edge: SvaEdge) {
        self.states[from].push(edge);
    }

    fn build(seq: &SvaSeq) -> SvaNfa {
        let mut b = SvaNfaBuilder { states: Vec::new() };
        let (start, accept) = b.frag(seq);
        SvaNfa {
            states: b.states,
            start,
            accept,
        }
    }

    fn not_of(e: &Expression) -> Expression {
        Expression::new(
            ExprKind::Unary {
                op: crate::ast::expr::UnaryOp::LogNot,
                operand: Box::new(e.clone()),
            },
            e.span,
        )
    }

    fn true_lit(span: crate::ast::Span) -> Expression {
        Expression::new(
            ExprKind::Number(crate::ast::expr::NumberLiteral::Integer {
                size: None,
                signed: false,
                base: crate::ast::expr::NumberBase::Decimal,
                value: "1".to_string(),
                cached_val: std::cell::Cell::new(None),
            }),
            span,
        )
    }

    /// One unit of a repetition: the repeated sequence, or one `b[->1]`
    /// occurrence (`!b` cycles then a `b` cycle).
    fn unit(&mut self, u: &SvaRepUnit<'_>) -> (usize, usize) {
        match u {
            SvaRepUnit::Seq(s) => self.frag(s),
            SvaRepUnit::Goto(cond) => {
                let s = self.state();
                let x = self.state();
                let e = self.state();
                self.add(s, SvaEdge::Check(Self::not_of(cond), x));
                self.add(x, SvaEdge::Tick(s));
                self.add(s, SvaEdge::Check((*cond).clone(), e));
                (s, e)
            }
        }
    }

    /// `unit[*min:max]`: consecutive copies one cycle apart.
    fn rep(&mut self, u: &SvaRepUnit<'_>, min: u64, max: u64) -> (usize, usize) {
        // §16.9.2: large counts use run-time counters, preserving the
        // complete unit (including multi-cycle matches and local variables).
        if min > 256 || (max != u64::MAX && max > 256) {
            let unit = match u {
                SvaRepUnit::Seq(seq) => Self::build(seq),
                SvaRepUnit::Goto(cond) => Self::build(&SvaSeq::Goto {
                    cond: (*cond).clone(),
                    min: 1,
                    max: 1,
                }),
            };
            return self.sub(SvaCompound::Repeat(unit, min, max));
        }
        let s = self.state();
        let t = self.state();
        if min == 0 {
            self.add(s, SvaEdge::Eps(t));
        }
        let mut prev_end: Option<usize> = None;
        for _ in 0..min {
            let (cs, ce) = self.unit(u);
            match prev_end {
                None => self.add(s, SvaEdge::Eps(cs)),
                Some(pe) => self.add(pe, SvaEdge::Tick(cs)),
            }
            prev_end = Some(ce);
        }
        if let Some(pe) = prev_end {
            self.add(pe, SvaEdge::Eps(t));
        }
        if max == u64::MAX {
            let (cs, ce) = self.unit(u);
            match prev_end {
                None => self.add(s, SvaEdge::Eps(cs)),
                Some(pe) => self.add(pe, SvaEdge::Tick(cs)),
            }
            self.add(ce, SvaEdge::Eps(t));
            self.add(ce, SvaEdge::Tick(cs));
        } else {
            let max = max.max(min);
            for _ in min..max {
                let (cs, ce) = self.unit(u);
                match prev_end {
                    None => self.add(s, SvaEdge::Eps(cs)),
                    Some(pe) => self.add(pe, SvaEdge::Tick(cs)),
                }
                self.add(ce, SvaEdge::Eps(t));
                prev_end = Some(ce);
            }
        }
        (s, t)
    }

    fn frag(&mut self, seq: &SvaSeq) -> (usize, usize) {
        match seq {
            SvaSeq::Bool(e) => {
                let s = self.state();
                let t = self.state();
                self.add(s, SvaEdge::Check(e.clone(), t));
                (s, t)
            }
            SvaSeq::Delay(min, max) => {
                if *min > 4096 || (*max != u64::MAX && *max > 4096) {
                    return self.sub(SvaCompound::Delay(*min, *max));
                }
                let s = self.state();
                let mut cur = s;
                for _ in 0..*min {
                    let n = self.state();
                    self.add(cur, SvaEdge::Tick(n));
                    cur = n;
                }
                if *max == u64::MAX {
                    self.add(cur, SvaEdge::Tick(cur));
                    (s, cur)
                } else {
                    let t = self.state();
                    self.add(cur, SvaEdge::Eps(t));
                    for _ in *min..(*max).max(*min) {
                        let n = self.state();
                        self.add(cur, SvaEdge::Tick(n));
                        self.add(n, SvaEdge::Eps(t));
                        cur = n;
                    }
                    (s, t)
                }
            }
            SvaSeq::Concat(items) => {
                if items.is_empty() {
                    let s = self.state();
                    return (s, s);
                }
                let (s0, mut prev_end) = self.frag(&items[0]);
                for it in &items[1..] {
                    let (si, ei) = self.frag(it);
                    self.add(prev_end, SvaEdge::Eps(si));
                    prev_end = ei;
                }
                (s0, prev_end)
            }
            SvaSeq::Or(a, b) => {
                let s = self.state();
                let t = self.state();
                let (sa, ea) = self.frag(a);
                let (sb, eb) = self.frag(b);
                self.add(s, SvaEdge::Eps(sa));
                self.add(s, SvaEdge::Eps(sb));
                self.add(ea, SvaEdge::Eps(t));
                self.add(eb, SvaEdge::Eps(t));
                (s, t)
            }
            SvaSeq::And(a, b) => self.sub(SvaCompound::And(Self::build(a), Self::build(b))),
            SvaSeq::Intersect(a, b) => {
                self.sub(SvaCompound::Intersect(Self::build(a), Self::build(b)))
            }
            SvaSeq::Throughout(e, a) => {
                self.sub(SvaCompound::Throughout(e.clone(), Self::build(a)))
            }
            SvaSeq::Until(a, b) => self.sub(SvaCompound::Until {
                left: Self::build(a),
                right: Self::build(b),
            }),
            SvaSeq::Within(a, b) => {
                // §16.9.10: `(1[*0:$] ##1 a ##1 1[*0:$]) intersect b`.
                let span = crate::ast::Span::dummy();
                let padded = SvaSeq::Concat(vec![
                    SvaSeq::Delay(0, u64::MAX),
                    (**a).clone(),
                    SvaSeq::Delay(0, u64::MAX),
                    SvaSeq::Bool(Self::true_lit(span)),
                ]);
                self.sub(SvaCompound::Intersect(Self::build(&padded), Self::build(b)))
            }
            SvaSeq::FirstMatch(a) => self.sub(SvaCompound::FirstMatch(Self::build(a))),
            SvaSeq::Rep { seq, min, max } => self.rep(&SvaRepUnit::Seq(seq), *min, *max),
            SvaSeq::Goto { cond, min, max } => self.rep(&SvaRepUnit::Goto(cond), *min, *max),
            SvaSeq::Assign(items) => {
                let s = self.state();
                let t = self.state();
                self.add(s, SvaEdge::Assign(items.clone(), t));
                (s, t)
            }
            SvaSeq::NonCon { cond, min, max } => {
                // §16.9.2: `b[->min:max] ##1 !b[*0:$]`.
                let (s, e) = self.rep(&SvaRepUnit::Goto(cond), *min, *max);
                let t = self.state();
                let f = self.state();
                self.add(e, SvaEdge::Eps(t));
                self.add(e, SvaEdge::Tick(f));
                self.add(f, SvaEdge::Check(Self::not_of(cond), e));
                (s, t)
            }
        }
    }

    fn sub(&mut self, c: SvaCompound) -> (usize, usize) {
        let s = self.state();
        let t = self.state();
        self.add(s, SvaEdge::Sub(Box::new(c), t));
        (s, t)
    }
}

/// Covergroup handles live above this tag; class handles are plain heap indices.
const CG_HANDLE_TAG: usize = 1 << 30;

#[derive(Debug, Clone)]
struct CovergroupInstance {
    cg_name: String,
    /// The object whose constructor ran `cg = new` (a class-body
    /// covergroup): coverpoint expressions read ITS properties, so sampling
    /// from outside the class (`e.cg.sample()`) evaluates with this `this`.
    owner: Option<usize>,
    /// §19.3 constructor formals bound at `new(...)`: visible to coverpoint
    /// expressions, bins and options of this instance.
    ctor_args: Vec<(String, Value)>,
    /// §19.3 `ref` constructor formals and their actual expressions: read
    /// again at every sample rather than captured at `new`.
    ctor_refs: Vec<(String, Expression)>,
    /// §8.25/§19.5: the value parameters of the owning object's class
    /// specialization (`N` of `group_n #(2)`), for bins and options of a
    /// class-body covergroup. A constructor formal of the same name hides one.
    class_params: Vec<(String, Value)>,
    /// Hits: coverpoint_name -> observed value -> times sampled
    point_hits: HashMap<String, HashMap<Value, u64>>,
    /// Cross hits: cross_name -> observed value tuple -> times sampled
    cross_hits: HashMap<String, HashMap<Vec<Value>, u64>>,
    /// §19.6.1 hit counts of the bins of a cross body. Key:
    /// `cross_name.bin_name`. A sample counts once for each bin selecting a
    /// product of the bins its values fall in.
    cross_bin_hits: HashMap<String, u64>,
    /// Per-explicit-bin hit counts (LRM §19.5). Key: `coverpoint.bin`. Only
    /// populated for coverpoints that declared `bins`. Sampling increments
    /// the counter when the value falls in any of the bin's ranges; an
    /// `illegal_bins` hit also fires an immediate error.
    #[allow(dead_code)]
    bin_hits: HashMap<String, u64>,
    /// §19.5.2 in-flight attempts of each transition sequence, by bin key
    /// (`cp.bin#<n>` for the n-th sequence of a scalar bin): (step, count).
    trans_pos: HashMap<String, HashSet<(u32, u32)>>,
    /// Total `sample()` invocations on this instance.
    sample_count: u64,
    /// §19.8 `stop()`: sampling is off until `start()`.
    stopped: bool,
    /// Coverpoints and crosses stopped with `cg.item.stop()`.
    stopped_items: HashSet<String>,
    /// §19.7 `cg.option.<name> = v` written at run time.
    options: HashMap<String, Value>,
}

/// One bin of a coverpoint as coverage counts it (§19.5).
struct CgBin {
    /// Key of its hit counter in `bin_hits`; None for an automatic bin,
    /// whose hits come from the sampled values.
    key: Option<String>,
    /// The declared bin name (`auto[k]` for an automatic bin).
    name: String,
    /// Constant value ranges; empty for a transition bin or non-constant
    /// values.
    ranges: Vec<(i64, i64)>,
}

/// Which bins own one product of a cross's bins (§19.6.1).
enum CrossOwner {
    /// Removed by the ignore_bins or illegal_bins at this index of the
    /// cross body.
    Excluded(usize),
    /// An automatic cross bin of its own.
    Auto,
    /// Held by these user bins (indices among the body's `bins`).
    User(Vec<usize>),
}

/// One resolved step of a §19.5.2 transition: its values and repetition
/// count range (`max` None = unbounded).
#[derive(Clone)]
struct TransAtom {
    ranges: Vec<(i64, i64)>,
    kind: crate::ast::decl::TransRepeatKind,
    min: u32,
    max: Option<u32>,
}

/// The bins of a coverpoint, or of a crossed variable.
struct CpShape {
    bins: Vec<CgBin>,
    /// Automatic bins: (value space, values per bin, bin count).
    auto: Option<(u64, u64, u64)>,
    /// Values removed by `ignore_bins` / `illegal_bins`.
    excl: Vec<(i64, i64)>,
}

/// Per-source-location running tally for `assert` / `assume` / `cover`,
/// keyed by `Simulator::assert_stat_key` (source file and span start) so
/// each distinct source position is tracked once across all executions.
#[derive(Debug, Clone, Copy, Default)]
struct AssertionStat {
    /// "assert" / "assume" / "cover" — one of three kinds
    kind: u8,
    /// Where the statement is: its source file index (when known) and the
    /// span start in that file's preprocessed text.
    loc: Option<(Option<u32>, usize)>,
    /// Times the predicate evaluated true (`cover` calls these "hits").
    pass_count: u64,
    /// Times the predicate evaluated false (`cover` ignores).
    fail_count: u64,
}

/// Roadmap steps 11-12: a process compiled to bytecode WITH wait points.
/// The scheduler keeps only (pid -> frame); a resume re-enters `exec_insns`
/// at `pc` with the frame's registers swapped in, instead of re-walking the
/// AST continuation chain. Registered only for `always` bodies whose whole
/// statement tree compiles fallback-free (see `try_register_proc_fsm`).
/// Two-state form of a process FSM body (see `TwoStateBlock::has_wait`).
struct ProcFsmTs {
    block: super::bytecode::TwoStateBlock,
    /// The FSM's own two-state register file, swapped into `ts_regs` for a
    /// segment; `valid` says it holds the current values (a VM segment
    /// makes the VM registers the truth again).
    regs: Vec<u64>,
    valid: bool,
    /// Copy taken at segment start: a bail re-runs the segment on the VM
    /// from these values.
    snap: Vec<u64>,
    /// Back-off after repeated bails (reset-phase x): skip two-state
    /// attempts until `resumes` reaches `skip_until`.
    fails: u32,
    resumes: u32,
    skip_until: u32,
}

struct ProcFsm {
    compiled: super::bytecode::CompiledBlock,
    ts: Option<Box<ProcFsmTs>>,
    /// Signal ids the body can write (sorted), or None when it has array
    /// stores or AST fallbacks; lets `note_comb_ran_in_process` skip the
    /// snapshot of comb outputs this process can never clobber.
    writes: Option<Arc<[u32]>>,
    /// Event-control specs for WaitEdge insns, with the lazily resolved
    /// sensitivity and its signal-id terms cached after the first suspension.
    waits: Vec<(
        crate::ast::stmt::EventControl,
        Option<Vec<Sensitivity>>,
        Option<Vec<SensitivityId>>,
    )>,
    /// Per wait: a clocking event (`@(cb)`), whose waiter resumes in the
    /// Reactive region after the block samples (§14.13).
    wait_is_clocking: Vec<bool>,
    pc: u32,
    regs: Vec<Value>,
    scope: String,
    /// Module-precision diff for delay quantization (§3.14.3), resolved on
    /// first activation when the process scope is installed.
    precision_diff: Option<i32>,
    /// `always` bodies restart at pc 0 on completion; `initial` bodies are
    /// done — the frame is dropped and the pid never reactivates.
    is_always: bool,
    /// Roadmap step 13: rustc-native form of the body (XEZIM_AOT=1). The
    /// native fn loads/spills its registers from `native_frame` around each
    /// suspension; bytecode execution is the fallback when absent.
    #[cfg(feature = "jit")]
    native: Option<super::aot::AotFsmFn>,
    #[cfg(feature = "jit")]
    native_frame: Vec<u64>,
}

/// What a Wait insn suspended on: (payload). Paired with the resume pc in
/// `Simulator::fsm_suspend`.
enum FsmWait {
    /// Delay in global ticks (already quantized).
    Delay(u64),
    /// Index into the frame's `waits` table.
    Edge(u32),
}

/// One clocking block's per-tick poll state: its clock resolved to a
/// signal-table id once (None: fall back to the by-name read), the edge it
/// advances on, and the level seen on the previous tick.
struct ClockingTick {
    cb: String,
    clk: String,
    id: Option<usize>,
    edge: EdgeKind,
    prev: u8,
}

/// A callee's rename frames held aside while its actuals are evaluated
/// (`begin_formal_dyn` / `commit_formal_dyn`).
type FormalDynSetup = (usize, Vec<(String, String)>, Vec<(String, String)>);

#[derive(Debug, Clone, Default)]
struct ProcessContext {
    this_stack: Vec<Option<usize>>,
    local_stack: Vec<HashMap<String, Value>>,
    /// Open block scopes of blocking `begin…end` blocks that declare
    /// variables (§6.21), innermost last; see `BlockScopeSave`. Rides process
    /// suspension like `local_stack`, since the block ends after a resume.
    block_scope_saves: Vec<BlockScopeSave>,
    /// Parallel to `local_stack`: the frame generation each slot was pushed
    /// with. Two different method activations never share a generation even
    /// when they occupy the same slot index, so a fork child can tell whether
    /// a parent slot still holds the activation it forked from (see
    /// `merge_fork_writes`).
    local_gen_stack: Vec<u64>,
    local_type_stack: Vec<(HashMap<String, String>, HashMap<String, String>)>,
    class_context_stack: Vec<Option<String>>,
    /// §8.25: a suspended static task retains its specialization and the
    /// enclosing bindings, independently of other processes running next.
    current_spec: Option<(String, String)>,
    spec_scope_stack: Vec<(String, String)>,
    cg_this: Option<usize>,
    return_value: Option<Value>,
    break_flag: bool,
    continue_flag: bool,
    return_flag: bool,
    // Carried so a task that suspends mid-body keeps its full call context
    // (needed once blocking task/method calls are inlined into the process
    // statement stream — see `task_cleanup` and `StatementKind::ScopePop`).
    local_iface_aliases: Vec<HashMap<String, String>>,
    ref_binding_stack: Vec<HashMap<String, Expression>>,
    ref_alias_stack: Vec<HashMap<String, String>>,
    /// §13.5.2 `ref` formals bound by IDENTITY (aggregates with no storage of
    /// their own); element accesses on them are rewritten onto the actual.
    /// Pushed/popped in lockstep with `ref_binding_stack`.
    ref_identity_stack: Vec<Vec<String>>,
    queue_frame_saves: Vec<Vec<(String, QueueLocalSave)>>,
    task_cleanup: Vec<TaskCleanup>,
    // Per-call-frame map of a local dynamic-array/queue/assoc LOCAL's bare
    // name to a process-unique storage key (e.g. `edges` -> `@edges#7`).
    // SystemVerilog automatic locals are per-invocation: two concurrent
    // task calls each declaring `int edges[$]` must NOT share the global
    // `signals` keys `edges.size` / `edges[i]`. By renaming at the VarDecl
    // and resolving the bare name through this map (see `dyn_name_lookup`
    // + the `resolve_hier_name` early return), each invocation's data lives
    // under a distinct key. Stack of frames pushed/popped in sync with
    // `push_queue_frame`/`pop_and_restore_queue_frame`.
    local_dyn: Vec<Vec<(String, String)>>,
    formal_dyn: Vec<Vec<(String, String)>>,
    static_local_syncs: Vec<(String, Vec<(String, String)>)>,
    /// See `Simulator::method_local_base`. This is per-PROCESS state: the
    /// base indexes into `local_stack`, which is swapped out with the rest
    /// of the context when a process parks. A process suspended inside an
    /// inlined BLOCKING class method used to leave its base behind, so
    /// another process's frame-local lookups (`local_class_type_of`,
    /// `local_typedef_type_of`, `get_expr_type_name`'s `in_any_frame`) were
    /// bounded by an index into a `local_stack` that was no longer there —
    /// a method-local shadowing a module-scope net then resolved to the net
    /// and `local = new()` constructed the wrong class.
    method_local_base: Vec<usize>,
    /// §6.21: the lifetime of the subroutine this process is running (see
    /// `Simulator::current_static_task` / `current_auto_task`). A task that
    /// parks mid-body must not leave its lifetime to the process that runs
    /// next, nor lose it when it resumes.
    current_static_task: Option<String>,
    current_auto_task: Option<String>,
    /// See `Simulator::task_string_formals`.
    task_string_formals: Vec<TaskFrameStrings>,
}

/// Frame-local string classifications for task formals and scalar locals.
/// They remain authoritative while other processes change global metadata.
#[derive(Debug, Clone)]
struct TaskFrameStrings {
    /// `local_gen_stack` generation of the task's frame.
    fgen: u64,
    /// Its `string` formals.
    strings: Vec<String>,
    /// Its non-string formals whose name `string_signals` holds for another
    /// scope.
    others: Vec<String>,
}

#[derive(Debug, Clone)]
struct FormalMetadataSnapshot {
    name: String,
    parts: FormalMetaParts,
}

/// The name-keyed type metadata of one formal or local (see
/// `snapshot_formal_parts`).
#[derive(Debug, Clone, Default)]
struct FormalMetaParts {
    declared_type: Option<DataType>,
    packed_fields: Option<Vec<(String, u32, u32)>>,
    packed_element_widths: Vec<(String, u32)>,
    packed_dimensions: Option<Vec<(i64, i64)>>,
    class_type: Option<String>,
    typedef_type: Option<String>,
}

/// Declaration facts of one class method's formals and return value, built
/// once per method (see `Simulator::method_call_plan`). The facts marked
/// "type tables" read `typedefs` / `typedef_types` / `parameters` /
/// `enum_members` / `plain_class_types`, which a procedural `typedef` or an
/// inline-enum local can change at run time: they hold while
/// `Simulator::type_tables_gen` equals `type_gen`, and every use re-derives them
/// otherwise.
struct MethodCallPlan {
    /// Keeps the method alive, and with it the address the plan is keyed by.
    _method: Arc<crate::ast::decl::ClassMethod>,
    type_gen: u64,
    ports: Vec<PortPlan>,
    ret: Option<RetPlan>,
    /// Every formal is a plain scalar or plain class: none has an actual to
    /// resolve as a virtual interface (type tables).
    all_plain: bool,
    /// The `formal_meta_name` positions: the names whose metadata a call
    /// saves and restores (declaration only).
    meta_ks: Vec<usize>,
    /// `meta_key_bits` of every one of those names, OR-ed.
    meta_bloom: u64,
    /// `meta_key_bits` of each of those names, by position.
    meta_bits: Vec<u64>,
    /// `(vif_local_names.len(), no formal name is in vif_local_names)` at the
    /// last look; the set only grows, so an equal length is the same set.
    vif_absent: std::cell::Cell<(usize, bool)>,
}

/// A memoized `method_defining_class` answer: the defining class, the
/// method, its inherited string property names, and its call plan once built
/// (see `MethodCallPlan`).
#[derive(Clone)]
struct MethodDef {
    class: std::rc::Rc<String>,
    method: Arc<crate::ast::decl::ClassMethod>,
    string_props: std::rc::Rc<Vec<String>>,
    plan: std::rc::Rc<std::cell::RefCell<Option<std::rc::Rc<MethodCallPlan>>>>,
}

/// Per leaf class: the facts of its `extends` chain that instantiation used
/// to re-derive on every `new` (see `Simulator::inst_template`). The class
/// tables are fixed at run time; the template also records the chain it was
/// built from and is only used for that same chain (compared by pointer).
/// `InstTemplate::seed` flag: seed the property with x (`prop_seeds_x`).
const SEED_X: u8 = 4;

struct InstTemplate {
    chain: Vec<Arc<crate::compiler::elaborate::ElaboratedClass>>,
    /// Pre-seeded scalar/handle property map for the chain (cloned in one
    /// allocation per `new()` instead of inserting property-by-property).
    base_props: HashMap<String, Value>,
    /// Per-class keys into `base_props` when a descendant property hides an
    /// ancestor parameter. Such chains need property/parameter interleaving.
    property_seed_keys: Option<Vec<Vec<String>>>,
    /// Property names redeclared along the chain (`seed & 3 == 2`).
    shadowed_props: Vec<String>,
    /// Whether any class in `chain` has `param_defaults` (value parameters).
    has_value_params: bool,
    /// The `(chain index, property)` pairs, in the registration loop's
    /// order, that the type-parameter-bound collection registration can
    /// claim: a declared type name and no queue, associative, fixed,
    /// multi-dimensional or static collection property of the class.
    coll_candidates: Vec<(usize, String)>,
    /// §7.2.2 member defaults of the chain's UNPACKED-struct properties
    /// (`dflt_t d;` with `typedef struct { int a = 3; } dflt_t`): the cell
    /// key `<prop>.<member>`, the default expression and the member's type,
    /// root class first so a derived class's property wins.
    struct_defaults: Vec<(String, Expression, DataType)>,
    /// §6.8 / §7.2: the initial member cells of the chain's UNPACKED-struct
    /// properties (`st_t st;`), keyed `<prop>.<member>`: x for a 4-state
    /// member, 0 for a 2-state one. `struct_defaults` applies on top.
    struct_cells: Vec<(String, Value)>,
}

impl MethodCallPlan {
    /// The `k`-th saved name (see `meta_ks`).
    fn meta_name(&self, k: usize) -> &str {
        use crate::ast::decl::ClassMethodKind;
        let (ports, ret) = match &self._method.kind {
            ClassMethodKind::Function(f) => (&f.ports[..], Some(f.name.name.name.as_str())),
            ClassMethodKind::Task(t) => (&t.ports[..], None),
            _ => (&[][..], None),
        };
        match ports.get(k) {
            Some(p) => p.name.name.as_str(),
            None => ret.unwrap_or_default(),
        }
    }
}

/// An active class-method activation's formal-metadata save. The snapshot
/// of each formal and return-variable name (see `snapshot_formal_parts`) is
/// taken when that name's metadata is first written — up to then the tables
/// hold the entry state for it, which is what an eager snapshot would have
/// saved — and a name never written has nothing to restore. Every writer of
/// the six tables calls `meta_note` first.
struct MetaFrame {
    plan: std::rc::Rc<MethodCallPlan>,
    /// Positions in `plan.meta_ks` already saved (all of them once `full`).
    saved: u64,
    full: bool,
    parts: Vec<(usize, FormalMetaParts)>,
    /// `meta_bloom` of this save and of every save below it.
    cum_bloom: u64,
    /// `meta_bits` of the names not saved yet, OR-ed.
    unsaved_bloom: u64,
}

/// See `MethodCallPlan`.
struct PortPlan {
    /// `port_is_plain_scalar` (declaration only).
    plain_scalar: bool,
    /// `port_class_ref` (type tables).
    class_ref: Option<bool>,
    /// Width of a built-in integer or literal-range vector formal (type
    /// tables); `None` for any other type.
    fixed_width: Option<u32>,
    /// `is_type_signed` of the declared type (declaration only).
    signed: bool,
    /// A direct scalar two-state declaration; independent of specialization.
    two_state: bool,
    /// `is_virtual_iface_type` is false whatever the specialization: neither
    /// an interface nor a type reference, or a plain class (type tables).
    never_vif: bool,
}

/// §13.4.1: the conversion a function's value takes to its declared return
/// type (see `Simulator::ret_convert`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RetConv {
    /// Not a numeric scalar: the value passes through untouched.
    Keep,
    /// `real` / `shortreal` / `realtime`.
    Real,
    /// An integral type; `two_state` for bit, byte, int, ... (§6.11.1).
    Int { two_state: bool },
}

/// See `MethodCallPlan` (functions only).
struct RetPlan {
    /// `method_return_is_string`, when no specialization can change it (type
    /// tables).
    is_string: Option<bool>,
    /// The declared type is `string` (declaration only).
    decl_string: bool,
    /// `dyn_ret_type` of the call path (declaration only).
    dyn_ret: Option<DataType>,
    /// `unpacked_struct_of` (type tables).
    unpacked: Option<crate::ast::types::StructUnionType>,
    /// `resolve_type_width` of the declared type (type tables).
    width: u32,
    signed: bool,
    real: bool,
    two_state: bool,
    /// A declared class-type return (the class tables are fixed).
    class: Option<String>,
    /// `fn_returns_collection` (type tables).
    collection: bool,
}

/// The name-keyed type metadata a subroutine-local declaration displaced
/// (see `decl_shadow_log`).
#[derive(Debug, Clone)]
struct DeclShadow {
    width: Option<u32>,
    signed: bool,
    real: bool,
    string: bool,
    meta: FormalMetadataSnapshot,
}

/// One selected region of a declared packed value (§7.4.1 / §11.5.1).
struct DeclaredPackedAccess {
    storage: String,
    is_signed: bool,
    static_target: Option<(String, Option<(String, String)>, String)>,
    offset: u32,
    width: u32,
    dimensions: Vec<(i64, i64)>,
    element_type: DataType,
    unpacked: usize,
}

/// A known static root either supports packed access or retains its own path.
enum DeclaredStaticAccess {
    Packed(DeclaredPackedAccess),
    Other,
}

/// One subroutine activation's displaced declarations, restored when it
/// returns. `frame_gen` is the activation's local-frame generation; `epoch` the
/// `nested_run_epoch` at entry.
#[derive(Debug, Clone)]
struct DeclShadowFrame {
    frame_gen: u64,
    epoch: u64,
    saves: Vec<DeclShadow>,
}

/// §6.21: the scope of a `begin…end` block that declares variables, opened
/// when the block is entered with a call frame on the stack. Block-locals
/// live in the enclosing frame under their bare names, so on exit the names
/// the block declared are removed again and any enclosing variable they
/// shadowed is put back. Only shadowed names cost a copy.
#[derive(Debug, Clone, Default)]
struct BlockScopeSave {
    /// `frame_key` of the flattened block's frame (blocking path); 0 for a
    /// synchronous block.
    frame_key: usize,
    /// Generation of the frame the block declares into: exit leaves a frame
    /// of another activation alone.
    frame_gen: u64,
    /// Enclosing frame entries (bare names and unpacked-struct leaves) the
    /// block's declarations shadow.
    shadowed: Vec<(String, Value)>,
    /// §6.21: declared layouts and scalar types displaced by this block.
    declarations: Vec<DeclShadow>,
    /// The enclosing frame's string-kind overlay, not the global name table.
    string_kinds: Vec<(String, Option<bool>)>,
    /// Virtual-interface aliases and auxiliary slots hidden by this block.
    vif_aliases: Vec<(String, Option<String>)>,
    vif_slots: Vec<(String, Option<Value>)>,
    /// A suspend-aware for loop closes its variable scope at the same sentinel.
    loop_vars: Vec<LoopVarSnapshot>,
    owns_loop_frame: bool,
    /// Enclosing collection renames (`local_dyn`) the block's collection
    /// declarations shadow, moved aside so the inner declaration cannot
    /// clean the outer storage up.
    dyn_shadowed: Vec<(String, String)>,
    /// `(depth, len)` of `static_local_syncs` at entry when the block
    /// declares a `static` variable.
    static_mark: Option<(usize, usize)>,
    /// The subroutine's `static` locals that a non-static block variable of
    /// the same name hides, as `static_local_syncs` depth and the
    /// `(name, key)` entries taken out of that frame until the block ends.
    static_hidden: Option<(usize, Vec<(String, String)>)>,
    /// `next_pid` at entry: a block that forked keeps its collection storage
    /// (a child may still use it).
    pid_mark: usize,
}

/// Deferred teardown for an inlined blocking task/method call: the work
/// `exec_task_call` would normally run synchronously after the body, replayed
/// at the body's `ScopePop` sentinel so the process can suspend in between.
#[derive(Debug, Clone, Default)]
struct TaskCleanup {
    /// Instance scope an inlined HIERARCHICAL/interface task body resolves
    /// bare names under (`bus.snoop()` → "bus"). Installed at bind and on
    /// every process RESUME while this frame is on top (a suspend inside the
    /// body would otherwise lose it), restored to `prev_hint` at unwind.
    frame_scope_hint: Option<String>,
    prev_hint: Option<String>,
    /// §21.2.1.7 `%m`: the m_scope_stack to restore on task return (a task
    /// call RESETS the lexical scope to just the task, since tasks are
    /// declared at module/package level — the CALLER's scope is not part of
    /// the callee's `%m`).
    saved_m_scope: Vec<String>,
    /// Name of the inlined task, for §9.6.2 `disable <task>` bookkeeping
    /// (`active_task_pids`). None for frames without a stable name.
    task_name: Option<String>,
    output_bindings: Vec<(String, crate::ast::expr::Expression)>,
    /// §13.5.3: omitted outputs copy back in the declaration's scope.
    default_output_bindings: Vec<(String, crate::ast::expr::Expression)>,
    /// §25.9: flat binding slots shadowed by this task's formals.
    saved_vif_formals: Vec<(String, Option<Value>)>,
    assoc_params: Vec<(String, String, bool)>,
    array_params: Vec<String>,
    /// `(formal, actual, lo, hi)` for an `output`/`inout`/`ref` ARRAY formal:
    /// its elements are copied back to the caller's array on return.
    array_writebacks: Vec<(String, String, i64, i64)>,
    /// `(formal, actual)` for an `output`/`inout`/`ref` QUEUE / dynamic-array
    /// formal: the callee's collection is copied back to the caller's on
    /// return. Functions always did this; tasks bound no collection formal at
    /// all, so a `task push(ref int q[$])` pushed into storage named after the
    /// FORMAL and the caller's queue never changed.
    queue_writebacks: Vec<(String, String)>,
    /// This frame pushed an entry on `task_string_formals` (popped on unwind).
    pushed_string_formals: bool,
    saved_break: bool,
    saved_continue: bool,
    saved_return: bool,
    prev_static: Option<String>,
    /// The caller's `current_auto_task` to restore on unwind (paired with
    /// `prev_static`).
    prev_auto: Option<String>,
    /// Set when a blocking-method inline (run_process_stmts) pushed a receiver
    /// `this`/class-context for an `obj.method()` call; pop them on unwind.
    /// Default false (free-task/same-`this` inlines don't touch those stacks).
    pushed_method_this: bool,
    saved_spec: Option<(String, String)>,
    formal_metadata: Vec<FormalMetadataSnapshot>,
    /// The method task of a class declared inside a module runs in its
    /// object's instance (see `Simulator::modclass_depth`): the process's
    /// activation scope while this frame is on top.
    modclass: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DpiRetKind {
    Void,
    Int32,
    /// §35.5.5 / H.7.4: a scalar `logic` result is an `svLogic` — an
    /// `unsigned char` holding sv_0 / sv_1 / sv_z (2) / sv_x (3).
    Logic,
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
    /// §35.5.6.1 / H.7.4: a scalar 4-state formal (`logic`, `reg`) is an
    /// `svLogic` (`unsigned char`, sv_z = 2, sv_x = 3) by value, or an
    /// `svLogic*` for output and inout.
    LogicIn,
    LogicOut,
    /// §35.5.6.1: a scalar two-state `bit` is an `svBit` (`unsigned char`)
    /// by value, or an `svBit*` for output and inout. Unlike `int`/`byte`/
    /// `shortint` (pass-by-value i32 whose low byte the C side reads) the
    /// value must be reduced to its single meaningful bit (0/1) before
    /// crossing the boundary — a 1-bit formal is not the low 8 bits.
    BitIn,
    BitOut,
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
    /// An open-array formal (`int a[]`, `bit [7:0] a[]`): passed as an
    /// Annex H open-array handle (see `DpiOpenArrayHeader`).
    OpenArrayIn(DpiOaElem),
    OpenArrayOut(DpiOaElem),
    VecLogicIn(u32),
    VecLogicOut(u32),
    /// 2-state packed vector (`bit [N]`) passed as plain `uint32_t*`
    /// (svBitVecVal*). ABI differs from VecLogicIn: no bval field.
    VecBitIn(u32),
    VecBitOut(u32),
    /// §35.5.6 / H.7.6: an unpacked struct (or a fixed-size array of
    /// them), passed as a pointer to the C struct of its members; its C
    /// layout comes from the formal's declared type (`dpi_c_type`).
    StructIn,
    StructOut,
}

/// The canonical C element of a DPI open array (IEEE 1800-2017 H.7):
/// `char`, `short`, `int`, `long long`, `float`, `double`, or a packed
/// vector as `svBitVecVal` / `svLogicVecVal` words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DpiOaElem {
    I8,
    I16,
    I32,
    I64,
    F32,
    F64,
    Bit(u32),
    Logic(u32),
    /// A scalar `bit` / `logic` element: one `svBit` / `svLogic` byte
    /// (H.7.4), not a one-word vector.
    SBit,
    SLogic,
}

impl DpiOaElem {
    fn bytes(self) -> usize {
        match self {
            DpiOaElem::I8 | DpiOaElem::SBit | DpiOaElem::SLogic => 1,
            DpiOaElem::I16 => 2,
            DpiOaElem::I32 | DpiOaElem::F32 => 4,
            DpiOaElem::I64 | DpiOaElem::F64 => 8,
            DpiOaElem::Bit(w) => 4 * w.div_ceil(32).max(1) as usize,
            DpiOaElem::Logic(w) => 8 * w.div_ceil(32).max(1) as usize,
        }
    }

    /// The SV width of one element.
    fn width(self) -> u32 {
        match self {
            DpiOaElem::I8 => 8,
            DpiOaElem::I16 => 16,
            DpiOaElem::I32 | DpiOaElem::F32 => 32,
            DpiOaElem::I64 | DpiOaElem::F64 => 64,
            DpiOaElem::Bit(w) | DpiOaElem::Logic(w) => w.max(1),
            DpiOaElem::SBit | DpiOaElem::SLogic => 1,
        }
    }
}

/// The header placed immediately BEFORE the element data of a DPI open
/// array; the handle C receives points at the data, so code indexing it as
/// a plain C array keeps working while `svSize`, `svLeft`,
/// `svGetArrElemPtr1`, ... (src/svdpi_shim.c, which mirrors this layout as
/// `xz_oa_hdr`) read the shape from here.
#[repr(C)]
#[derive(Clone, Copy)]
struct DpiOpenArrayHeader {
    magic: u32,
    elem_bytes: u32,
    ndims: i32,
    packed: i32,
    packed_left: i32,
    packed_right: i32,
    left: [i32; 4],
    right: [i32; 4],
    total_bytes: u32,
    elem_count: u32,
}

const DPI_OA_MAGIC: u32 = 0x414f_5a58; // "XZOA"
const DPI_OA_HDR_BYTES: usize = std::mem::size_of::<DpiOpenArrayHeader>();
const _: () = assert!(DPI_OA_HDR_BYTES == 64);

/// One open-array argument: its buffer (header + data, 8-byte aligned) and
/// shape, kept alive for the call and decoded again for an output.
struct DpiOpenArrayArg {
    buf: Vec<u64>,
    elem: DpiOaElem,
    count: usize,
}

impl DpiOpenArrayArg {
    /// `shape` is each unpacked dimension's `(left, right)`, outermost
    /// first; `values` holds the elements row-major, every dimension in
    /// ascending index order (the order `svGetArrElemPtr*` addresses).
    fn new(elem: DpiOaElem, values: &[Value], shape: &[(i64, i64)]) -> Self {
        let count = values.len();
        let mut left = [0i32; 4];
        let mut right = [0i32; 4];
        for (k, &(l, r)) in shape.iter().take(4).enumerate() {
            left[k] = l as i32;
            right[k] = r as i32;
        }
        let eb = elem.bytes();
        let data_bytes = eb * count;
        let mut buf = vec![0u64; (DPI_OA_HDR_BYTES + data_bytes).div_ceil(8)];
        let packed = matches!(elem, DpiOaElem::Bit(_) | DpiOaElem::Logic(_));
        let hdr = DpiOpenArrayHeader {
            magic: DPI_OA_MAGIC,
            elem_bytes: eb as u32,
            ndims: shape.len().clamp(1, 4) as i32,
            packed: packed as i32,
            packed_left: if packed { elem.width() as i32 - 1 } else { 0 },
            packed_right: 0,
            left,
            right,
            total_bytes: data_bytes as u32,
            elem_count: count as u32,
        };
        let bytes: &mut [u8] =
            unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast::<u8>(), buf.len() * 8) };
        bytes[..DPI_OA_HDR_BYTES].copy_from_slice(unsafe {
            std::slice::from_raw_parts(
                (&hdr as *const DpiOpenArrayHeader).cast::<u8>(),
                DPI_OA_HDR_BYTES,
            )
        });
        for (i, v) in values.iter().enumerate() {
            let at = DPI_OA_HDR_BYTES + i * eb;
            let dst = &mut bytes[at..at + eb];
            match elem {
                DpiOaElem::I8 | DpiOaElem::I16 | DpiOaElem::I32 | DpiOaElem::I64 => {
                    let x = if v.is_real {
                        v.to_f64() as i64 as u64
                    } else {
                        v.to_u64().unwrap_or(0)
                    };
                    dst.copy_from_slice(&x.to_le_bytes()[..eb]);
                }
                DpiOaElem::F32 => dst.copy_from_slice(&(Self::real_of(v) as f32).to_le_bytes()),
                DpiOaElem::F64 => dst.copy_from_slice(&Self::real_of(v).to_le_bytes()),
                DpiOaElem::SBit => dst[0] = (v.get_bit(0) == LogicBit::One) as u8,
                DpiOaElem::SLogic => dst[0] = Simulator::dpi_svlogic_code(v),
                DpiOaElem::Bit(w) => {
                    let (aval, _) = Simulator::dpi_value_to_logic_words(v, w);
                    for (k, word) in aval.iter().enumerate().take(eb / 4) {
                        dst[k * 4..k * 4 + 4].copy_from_slice(&word.to_le_bytes());
                    }
                }
                DpiOaElem::Logic(w) => {
                    let il = Simulator::dpi_value_to_logic_interleaved(v, w);
                    for (k, word) in il.iter().enumerate().take(eb / 4) {
                        dst[k * 4..k * 4 + 4].copy_from_slice(&word.to_le_bytes());
                    }
                }
            }
        }
        DpiOpenArrayArg { buf, elem, count }
    }

    fn real_of(v: &Value) -> f64 {
        if v.is_real {
            v.to_f64()
        } else {
            v.to_u64().unwrap_or(0) as i64 as f64
        }
    }

    /// The handle C receives: the start of the element data.
    fn handle(&mut self) -> *mut std::ffi::c_void {
        unsafe {
            self.buf
                .as_mut_ptr()
                .cast::<u8>()
                .add(DPI_OA_HDR_BYTES)
                .cast()
        }
    }

    /// The elements as C left them.
    fn values(&self) -> Vec<Value> {
        let eb = self.elem.bytes();
        let bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(self.buf.as_ptr().cast::<u8>(), self.buf.len() * 8)
        };
        (0..self.count)
            .map(|i| {
                let at = DPI_OA_HDR_BYTES + i * eb;
                let src = &bytes[at..at + eb];
                let word = |k: usize| u32::from_le_bytes(src[k * 4..k * 4 + 4].try_into().unwrap());
                match self.elem {
                    DpiOaElem::I8 => Value::from_u64(src[0] as i8 as i64 as u64, 8),
                    DpiOaElem::I16 => {
                        Value::from_u64(i16::from_le_bytes([src[0], src[1]]) as i64 as u64, 16)
                    }
                    DpiOaElem::I32 => Value::from_u64(word(0) as u64, 32),
                    DpiOaElem::I64 => {
                        Value::from_u64(u64::from_le_bytes(src.try_into().unwrap()), 64)
                    }
                    DpiOaElem::F32 => {
                        Value::from_f64(f32::from_le_bytes(src.try_into().unwrap()) as f64)
                    }
                    DpiOaElem::F64 => Value::from_f64(f64::from_le_bytes(src.try_into().unwrap())),
                    DpiOaElem::SBit => Value::from_u64((src[0] & 1) as u64, 1),
                    DpiOaElem::SLogic => Simulator::dpi_svlogic_value(src[0]),
                    DpiOaElem::Bit(w) => {
                        let aval: Vec<u32> = (0..eb / 4).map(word).collect();
                        Simulator::dpi_logic_words_to_value(&aval, &[], w)
                    }
                    DpiOaElem::Logic(w) => {
                        let il: Vec<u32> = (0..eb / 4).map(word).collect();
                        Simulator::dpi_logic_interleaved_to_value(&il, w)
                    }
                }
            })
            .collect()
    }
}

struct DpiBinding {
    ret: DpiRetKind,
    arg_kinds: Vec<DpiArgKind>,
    cif: Cif,
    fn_ptr: CodePtr,
}

/// Width of a spec fragment naming a DIMENSIONED vector type —
/// `bit[7:0]`, `logic signed[3:0][7:0]` — as `data_type_to_spec_fragment`
/// renders it for a `P#(bit [7:0])` specialization. Keyword-only fragments
/// are `atom_type_keyword_width`'s job; anything else declines.
fn vector_fragment_width(frag: &str) -> Option<u32> {
    let rest = frag
        .strip_prefix("bit")
        .or_else(|| frag.strip_prefix("logic"))
        .or_else(|| frag.strip_prefix("reg"))?;
    let rest = rest
        .trim_start()
        .strip_prefix("unsigned")
        .or_else(|| rest.trim_start().strip_prefix("signed"))
        .unwrap_or(rest)
        .trim();
    if !rest.starts_with('[') {
        return None;
    }
    let mut width: u64 = 1;
    for dim in rest.split(']').filter(|d| !d.trim().is_empty()) {
        let inner = dim.trim().strip_prefix('[')?;
        let (l, r) = inner.split_once(':')?;
        let (l, r) = (l.trim().parse::<i64>().ok()?, r.trim().parse::<i64>().ok()?);
        width = width.checked_mul((l - r).unsigned_abs() + 1)?;
    }
    u32::try_from(width).ok()
}

/// Parse `<base>[<idx>]` and resolve via the compact 1D-array map.
/// Free function so it can be shared between the `&self` resolver on
/// Simulator and the `&[Value]`-borrowed `exec_insns_isolated` path that
/// runs on separate threads.
/// Self-determined bit width of a built-in type keyword, for `$bits(<type>)`
/// where the type parses as a bare identifier (§6.11, §20.6.2). Returns None
/// for non-type names so a same-named signal/typedef path is used instead.
fn atom_type_keyword_width(name: &str) -> Option<u32> {
    let name = name
        .trim()
        .strip_suffix(" unsigned")
        .or_else(|| name.trim().strip_suffix(" signed"))
        .unwrap_or(name.trim())
        .trim_end();
    Some(match name {
        "integer" | "int" => 32,
        "shortint" => 16,
        "longint" | "time" | "real" | "realtime" => 64,
        "shortreal" => 32,
        "byte" => 8,
        "bit" | "logic" | "reg" => 1,
        _ => return None,
    })
}

/// `h` without its last segment. The derived `Clone` copies the resolution
/// caches, which belong to the full path; left in place, the prefix resolved
/// as the whole name (`u.sig` evaluated `u` as the signal `u.sig`).
fn hier_prefix(
    h: &crate::ast::expr::HierarchicalIdentifier,
) -> crate::ast::expr::HierarchicalIdentifier {
    let mut p = h.clone();
    p.path.pop();
    p.cached_signal_id = Cell::new(None);
    p.cached_resolved_name = std::cell::OnceCell::new();
    p
}

#[inline]
fn resolve_array_elem_id(
    name: &str,
    array_first_id: &HashMap<Arc<str>, (usize, i64, i64)>,
) -> Option<usize> {
    let bytes = name.as_bytes();
    if bytes.last() != Some(&b']') {
        return None;
    }
    let brack = name.rfind('[')?;
    let base = &name[..brack];
    let idx_str = &name[brack + 1..name.len() - 1];
    let idx: i64 = idx_str.parse().ok()?;
    let &(first_id, lo, hi) = array_first_id.get(base)?;
    if idx < lo || idx > hi {
        return None;
    }
    Some(first_id + (idx - lo) as usize)
}

/// §9.4.2 Table 9-2 edge test on the LSB of a raw (v, x) pair — the same
/// rule `Simulator::edge_fires_prev` applies to a signal; `AnyEdge` is any
/// change of the low 64 bits.
#[inline]
fn lsb_edge_fires(edge: EdgeKind, pv: u64, px: u64, cv: u64, cx: u64) -> bool {
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
fn value_event_fires(edge: EdgeKind, prev: &Value, cur: &Value) -> bool {
    if edge == EdgeKind::AnyEdge {
        return cur != prev;
    }
    let (pv, px) = prev.raw_bits();
    let (cv, cx) = cur.raw_bits();
    lsb_edge_fires(edge, pv, px, cv, cx)
}

/// Packed word storage for large memories (on by default;
/// `XEZIM_PACKED_MEM=0` turns it off).
///
/// c906 census: 35.06 M of its 35.11 M signals are array cells, and the big
/// ones are all BYTE-wide RAMs — 16 separate 8-bit `ram*.mem` arrays standing
/// in for one 128-bit word. As ordinary signals each cell costs a 32 B
/// `Value` plus ~24 B of per-signal side tables (width/signed/real/
/// inline_bits/has_xz/dirty) to carry 8 bits. In the arena a cell is its
/// value plane plus its x/z plane, each rounded up to 1, 2, 4 or 8 bytes and
/// stored side by side, so one access touches one cache line; a two-state
/// element has no x/z plane at all. Declared element metadata (width,
/// signedness, two-state) is kept once per ARRAY, not per cell.
///
/// ADMISSION: an integral element of at most 64 bits in an array big enough
/// to have skipped per-element name registration (see `packed_admissible`).
///
/// IDS: a cell id is `PACKED_BASE | segment << SEG_SHIFT | offset`. Within
/// one array the ids are consecutive, so the `first_id + (idx - lo)` cell
/// arithmetic every caller already performs stays correct with no change.
/// Every id at or above `PACKED_BASE` indexes far past the end of
/// `signal_table`, so a site that still indexes the table with one fails
/// loudly (out of bounds) rather than silently reading the wrong cell.
pub(crate) const PACKED_BASE: usize = 1usize << 48;
const PACKED_SEG_SHIFT: u32 = 32;
const PACKED_OFF_MASK: usize = (1usize << PACKED_SEG_SHIFT) - 1;
/// Segment numbers fit in the 16 bits between the offset and `PACKED_BASE`.
const PACKED_MAX_SEGS: usize = 1 << (48 - PACKED_SEG_SHIFT);

#[inline]
pub(crate) fn is_packed_id(id: usize) -> bool {
    id >= PACKED_BASE
}

fn packed_mem_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var("XEZIM_PACKED_MEM")
            .map(|v| v != "0")
            .unwrap_or(true)
    })
}

/// `XEZIM_MEM_CENSUS` helper: serialized size of any serde value behind a
/// `dyn` reference, so the census can list heterogeneous fields in one table.
mod erased_size {
    pub(super) trait Ser {
        fn size(&self) -> usize;
    }
    impl<T: serde::Serialize> Ser for T {
        fn size(&self) -> usize {
            bincode::serialized_size(self).unwrap_or(0) as usize
        }
    }
}

/// One admitted array.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PackedSeg {
    /// Byte offset of cell 0 in `PackedMem::data`.
    base: usize,
    /// Number of cells.
    count: usize,
    /// Element-width mask of the value plane.
    vmask: u64,
    /// Mask of the x/z plane: `vmask`, or 0 for a two-state element (which
    /// stores no x/z plane; its "x/z" load reads a neighbour and is masked
    /// to nothing).
    xmask: u64,
    /// The bytes of one plane (`plane` bytes of ones), for the 8-byte
    /// read-modify-write stores.
    pmask: u64,
    /// Bytes per plane: 1, 2, 4 or 8.
    plane: u8,
    /// Bytes per cell: `plane` (two-state) or `2 * plane` (value + x/z).
    cell: u8,
    /// Declared element width in bits (1..=64).
    width: u8,
    /// `SEG_SIGNED` / `SEG_TWO_STATE` / `SEG_FORCED`.
    flags: u8,
}

const SEG_SIGNED: u8 = 1;
const SEG_TWO_STATE: u8 = 2;
/// At least one cell of the segment is under `force` (§10.6.2); stores then
/// consult `PackedMem::forced` before landing.
const SEG_FORCED: u8 = 4;

/// Zero bytes after every segment, so an 8-byte load or read-modify-write
/// store at any plane start stays inside `data`.
const PACKED_PAD: usize = 16;

/// Byte arena backing every admitted array (see `PACKED_BASE`).
#[derive(Default)]
pub(crate) struct PackedMem {
    data: Vec<u8>,
    segs: Vec<PackedSeg>,
    /// Cells currently under `force` (only consulted for a segment carrying
    /// `SEG_FORCED`).
    forced: HashSet<usize>,
}

impl PackedMem {
    /// Admit an array of `count` cells of `width` bits, every cell holding
    /// `init`. Returns the id of cell 0, or None when the arena cannot take
    /// it (width 0 or over 64, or out of segment numbers).
    fn admit(
        &mut self,
        count: usize,
        width: u32,
        signed: bool,
        two_state: bool,
        init: (u64, u64),
    ) -> Option<usize> {
        if width == 0 || width > 64 || count == 0 || self.segs.len() >= PACKED_MAX_SEGS {
            return None;
        }
        let plane: u8 = match width {
            1..=8 => 1,
            9..=16 => 2,
            17..=32 => 4,
            _ => 8,
        };
        let cell = if two_state { plane } else { 2 * plane };
        let vmask = if width >= 64 {
            u64::MAX
        } else {
            (1u64 << width) - 1
        };
        let seg = PackedSeg {
            base: self.data.len(),
            count,
            vmask,
            xmask: if two_state { 0 } else { vmask },
            pmask: if plane == 8 {
                u64::MAX
            } else {
                (1u64 << (8 * plane as u32)) - 1
            },
            plane,
            cell,
            width: width as u8,
            flags: if signed { SEG_SIGNED } else { 0 } | if two_state { SEG_TWO_STATE } else { 0 },
        };
        let (iv, ix) = (init.0 & seg.vmask, init.1 & seg.xmask);
        let bytes = count.checked_mul(cell as usize)?;
        if iv == 0 && ix == 0 {
            self.data.resize(self.data.len() + bytes, 0);
        } else {
            let mut pattern = [0u8; 16];
            pattern[..plane as usize].copy_from_slice(&iv.to_le_bytes()[..plane as usize]);
            if !two_state {
                pattern[plane as usize..cell as usize]
                    .copy_from_slice(&ix.to_le_bytes()[..plane as usize]);
            }
            self.data.reserve(bytes + PACKED_PAD);
            for _ in 0..count {
                self.data.extend_from_slice(&pattern[..cell as usize]);
            }
        }
        self.data.resize(self.data.len() + PACKED_PAD, 0);
        let first = PACKED_BASE | (self.segs.len() << PACKED_SEG_SHIFT);
        self.segs.push(seg);
        Some(first)
    }

    fn reserve(&mut self, bytes: usize) {
        self.data.reserve(bytes);
    }

    /// Bytes held by the arena.
    fn bytes(&self) -> usize {
        self.data.len()
    }

    /// The cell bytes (for huge-page advice).
    fn data(&self) -> &[u8] {
        &self.data
    }

    #[inline(always)]
    fn locate(&self, id: usize) -> Option<(&PackedSeg, usize)> {
        let seg = self.segs.get((id - PACKED_BASE) >> PACKED_SEG_SHIFT)?;
        let off = id & PACKED_OFF_MASK;
        if off >= seg.count {
            return None;
        }
        Some((seg, seg.base + off * seg.cell as usize))
    }

    #[inline(always)]
    fn load8(&self, at: usize) -> u64 {
        u64::from_le_bytes(self.data[at..at + 8].try_into().unwrap())
    }

    /// Raw cell contents — `(val_bits, xz_bits, width)` with NO `Value`
    /// constructed. The RAM hot path moves whole words between the arena and
    /// a VM register; building a `Value` per access showed up as ~4% in
    /// `cell_read` plus ~3.5% in the allocator on a c906 cmark profile.
    #[inline]
    pub(crate) fn raw(&self, id: usize) -> (u64, u64, u32) {
        match self.locate(id) {
            Some((seg, at)) => (
                self.load8(at) & seg.vmask,
                self.load8(at + seg.plane as usize) & seg.xmask,
                seg.width as u32,
            ),
            None => (0, 1, 1),
        }
    }

    /// `raw` plus the element's declared signedness.
    #[inline]
    pub(crate) fn raw_signed(&self, id: usize) -> (u64, u64, u32, bool) {
        match self.locate(id) {
            Some((seg, at)) => (
                self.load8(at) & seg.vmask,
                self.load8(at + seg.plane as usize) & seg.xmask,
                seg.width as u32,
                seg.flags & SEG_SIGNED != 0,
            ),
            None => (0, 1, 1, false),
        }
    }

    /// Store raw bits into a cell (masked to the element width; a two-state
    /// element drops x/z to 0). A cell under `force` keeps its forced value.
    /// True when the cell changed.
    #[inline]
    pub(crate) fn set_raw(&mut self, id: usize, v: u64, x: u64) -> bool {
        let Some((seg, at)) = self.locate(id) else {
            return false;
        };
        let seg = *seg;
        if seg.flags & SEG_FORCED != 0 && self.forced.contains(&id) {
            return false;
        }
        self.store_at(&seg, at, v, x)
    }

    #[inline(always)]
    fn store_at(&mut self, seg: &PackedSeg, at: usize, v: u64, x: u64) -> bool {
        let nv = v & seg.vmask;
        let nx = x & seg.xmask;
        let xat = at + seg.plane as usize;
        let ov = self.load8(at);
        let ox = self.load8(xat);
        if ov & seg.vmask == nv && ox & seg.xmask == nx {
            return false;
        }
        // Read-modify-write of whole 8-byte words: the bytes past this
        // plane are written back unchanged, so `ox` stays current.
        self.data[at..at + 8].copy_from_slice(&((ov & !seg.pmask) | nv).to_le_bytes());
        if seg.xmask != 0 {
            self.data[xat..xat + 8].copy_from_slice(&((ox & !seg.pmask) | nx).to_le_bytes());
        }
        true
    }

    /// Land `(v, x)` in the cell even if it is forced, leaving the force as
    /// it is: for a swap undone before anything else reads the cell (a
    /// concurrent assertion's sampled values, §16.5.1).
    fn put_raw(&mut self, id: usize, v: u64, x: u64) {
        if let Some((seg, at)) = self.locate(id) {
            let seg = *seg;
            self.store_at(&seg, at, v, x);
        }
    }

    /// §10.6.2 `force`: land `(v, x)` in the cell even if it is already
    /// forced, and pin it there until `release_forced`.
    pub(crate) fn force_raw(&mut self, id: usize, v: u64, x: u64) -> bool {
        let Some((seg, at)) = self.locate(id) else {
            return false;
        };
        let seg_idx = (id - PACKED_BASE) >> PACKED_SEG_SHIFT;
        let seg_copy = *seg;
        let changed = self.store_at(&seg_copy, at, v, x);
        self.segs[seg_idx].flags |= SEG_FORCED;
        self.forced.insert(id);
        changed
    }

    /// §10.6.2 `release`: the cell keeps its value (a variable) and takes
    /// ordinary stores again.
    pub(crate) fn release_forced(&mut self, id: usize) {
        self.forced.remove(&id);
    }

    #[inline]
    pub(crate) fn width(&self, id: usize) -> u32 {
        self.locate(id)
            .map(|(seg, _)| seg.width as u32)
            .unwrap_or(1)
    }
    #[inline]
    pub(crate) fn is_signed(&self, id: usize) -> bool {
        self.locate(id)
            .is_some_and(|(seg, _)| seg.flags & SEG_SIGNED != 0)
    }
    #[inline]
    pub(crate) fn read(&self, id: usize) -> Value {
        if self.locate(id).is_none() {
            return Value::new(1);
        }
        let (v, x, w, signed) = self.raw_signed(id);
        let mut value = Value::from_inline(v, x, w);
        value.is_signed = signed;
        value
    }
    /// Returns true when the cell actually changed.
    #[inline]
    pub(crate) fn write(&mut self, id: usize, val: &Value) -> bool {
        let w = self.width(id);
        let (v, x) = if val.width == w && !val.is_real && !val.is_fill {
            val.raw_bits()
        } else {
            val.resize(w).raw_bits()
        };
        self.set_raw(id, v, x)
    }
}

/// Bits `lo..=hi` of `dst` replaced by the low bits of `src` (a part-select
/// store into a packed-arena cell; `hi` < 64, empty when `lo > hi`).
#[inline(always)]
fn splice_bits(dst: u64, src: u64, lo: u32, hi: u32) -> u64 {
    if lo > hi {
        return dst;
    }
    let len = hi - lo + 1;
    let m = if len >= 64 {
        u64::MAX
    } else {
        ((1u64 << len) - 1) << lo
    };
    (dst & !m) | ((src << lo) & m)
}

#[inline(always)]
/// Element width of a bytecode array operand, so an out-of-range or x/z-index
/// READ can yield x at the ELEMENT width. A 1-bit x zero-extends into a wider
/// element (`0000000x`), which silently turns "unknown" into "almost all
/// zero" — an AES S-box indexed by an uninitialized input must stay all-x.
fn bytecode_array_elem_width(
    array: &super::bytecode::ArrayOperand,
    array_first_id: &HashMap<Arc<str>, (usize, i64, i64)>,
    signal_table: &[Value],
) -> u32 {
    bytecode_array_first_id(array, array_first_id)
        .and_then(|f| signal_table.get(f))
        .map(|v| v.width)
        .unwrap_or(1)
        .max(1)
}

/// Id of element `lo` of a bytecode array operand, when it resolves.
fn bytecode_array_first_id(
    array: &super::bytecode::ArrayOperand,
    array_first_id: &HashMap<Arc<str>, (usize, i64, i64)>,
) -> Option<usize> {
    match array {
        super::bytecode::ArrayOperand::Dense { first_id, .. } => Some(*first_id),
        super::bytecode::ArrayOperand::Named(name) => {
            array_first_id.get(name.as_str()).map(|&(f, _, _)| f)
        }
    }
}

/// §11.5.1: the effective index of a DYNAMIC bit-select. `None` when the index
/// contains x or z, in which case a WRITE must be discarded ("no bits shall be
/// modified") and a read yields x.
///
/// `Value::to_u64` masks x bits to ZERO and returns `Some(0)` rather than
/// `None`, so it cannot answer this question. Every dynamic bit-store used
/// `to_u64().unwrap_or(0)` and therefore wrote BIT 0 whenever the index was
/// unknown. Out-of-range indices need no help here: `Value::set_bit` already
/// drops them. Same defect, and same fix, as the `LoadArrayElem` x-index
/// sentinel on the read side.

/// §11.5.1 part-select whose physical range `[hi:lo]` may leave `[0, w)`.
/// Out-of-range positions read x, and the in-range positions read their
/// bits (IEEE 1800 §11.5.1; differential-verified against a reference
/// simulator in `tests/misc/operators_11_select_reduce.rs`). Some tools
/// read the WHOLE select x instead when any position is out of range;
/// `XEZIM_OOB_SELECT=whole` selects that form.
/// `XEZIM_OOB_SELECT=whole`: a partially out-of-range select reads x as a
/// whole and a partially out-of-range WRITE is discarded (some tools);
/// default is the §11.5.1 per-bit form for both.
// ---- Wide two-state register word helpers (little-endian `[u64; N]`). ----

/// Top word index of a `w`-bit value and the mask of its live bits.
#[inline(always)]
fn wtop(w: u16) -> (usize, u64) {
    let top = ((w as usize).max(1) - 1) >> 6;
    let r = w as u32 - (top as u32) * 64;
    (top, if r >= 64 { u64::MAX } else { (1u64 << r) - 1 })
}

/// Truncate `x` to `w` bits in place.
#[inline(always)]
fn wmask_top<const N: usize>(x: &mut [u64; N], w: u16) {
    let (top, m) = wtop(w);
    x[top] &= m;
    for i in top + 1..N {
        x[i] = 0;
    }
}

#[inline(always)]
fn repeated_word(part: u64, width: u8) -> Option<u64> {
    match width {
        1 => Some(0u64.wrapping_sub(part & 1)),
        2 => Some((part & 0x3).wrapping_mul(0x5555_5555_5555_5555)),
        4 => Some((part & 0xf).wrapping_mul(0x1111_1111_1111_1111)),
        8 => Some((part & 0xff).wrapping_mul(0x0101_0101_0101_0101)),
        16 => Some((part & 0xffff).wrapping_mul(0x0001_0001_0001_0001)),
        32 => Some((part & 0xffff_ffff).wrapping_mul(0x0000_0001_0000_0001)),
        64 => Some(part),
        _ => None,
    }
}

#[inline(always)]
fn replicate_narrow(part: u64, width: u8, count: u8) -> u64 {
    if count == 0 {
        return 0;
    }
    if count == 1 {
        return part;
    }
    let total = width as u16 * count as u16;
    if let Some(word) = repeated_word(part, width) {
        return if total >= 64 {
            word
        } else {
            word & ((1u64 << total) - 1)
        };
    }

    let mut acc = 0u64;
    let mut chunk = part;
    let mut chunk_width = width as u32;
    let mut copies = count;
    while copies != 0 {
        if copies & 1 != 0 {
            acc = (acc << chunk_width) | chunk;
        }
        copies >>= 1;
        if copies != 0 {
            chunk |= chunk << chunk_width;
            chunk_width *= 2;
        }
    }
    acc
}

#[inline(always)]
fn replicate_wide<const N: usize>(part: u64, width: u8, count: u16) -> [u64; N] {
    if count == 0 {
        return [0u64; N];
    }
    if count == 1 {
        let mut acc = [0u64; N];
        acc[0] = part;
        return acc;
    }
    let total = width as u16 * count;
    if let Some(word) = repeated_word(part, width) {
        let mut acc = [word; N];
        wmask_top(&mut acc, total);
        return acc;
    }

    let mut acc = [0u64; N];
    let mut chunk = [0u64; N];
    chunk[0] = part;
    let mut chunk_width = width as u32;
    let mut copies = count;
    while copies != 0 {
        if copies & 1 != 0 {
            acc = wshl(acc, chunk_width);
            for i in 0..N {
                acc[i] |= chunk[i];
            }
        }
        copies >>= 1;
        if copies != 0 {
            let shifted = wshl(chunk, chunk_width);
            for i in 0..N {
                chunk[i] |= shifted[i];
            }
            chunk_width *= 2;
        }
    }
    acc
}

/// `x << sh` across N words (bits shifted past the top word are dropped).
#[inline(always)]
fn wshl<const N: usize>(x: [u64; N], sh: u32) -> [u64; N] {
    if sh == 0 {
        return x;
    }
    let ws = (sh >> 6) as usize;
    let bs = sh & 63;
    let mut r = [0u64; N];
    for i in ws..N {
        let mut v = x[i - ws] << bs;
        if bs != 0 && i > ws {
            v |= x[i - ws - 1] >> (64 - bs);
        }
        r[i] = v;
    }
    r
}

/// `x >> sh` across N words.
#[inline(always)]
fn wshr<const N: usize>(x: [u64; N], sh: u32) -> [u64; N] {
    if sh == 0 {
        return x;
    }
    let ws = (sh >> 6) as usize;
    let bs = sh & 63;
    let mut r = [0u64; N];
    for i in 0..N.saturating_sub(ws) {
        let mut v = x[i + ws] >> bs;
        if bs != 0 && i + ws + 1 < N {
            v |= x[i + ws + 1] << (64 - bs);
        }
        r[i] = v;
    }
    r
}

/// The 64 bits of `x` starting at bit `lo`.
#[inline(always)]
fn wget64<const N: usize>(x: &[u64; N], lo: u32) -> u64 {
    let ws = (lo >> 6) as usize;
    let bs = lo & 63;
    if ws >= N {
        return 0;
    }
    let mut v = x[ws] >> bs;
    if bs != 0 && ws + 1 < N {
        v |= x[ws + 1] << (64 - bs);
    }
    v
}

pub(crate) fn oob_select_whole() -> bool {
    static WHOLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *WHOLE.get_or_init(|| std::env::var("XEZIM_OOB_SELECT").ok().as_deref() == Some("whole"))
}

/// §11.5.1 partial-write rule: `true` when a write through a select whose
/// physical range `[hi:lo]` leaves `[0, w)` must be discarded entirely
/// (`XEZIM_OOB_SELECT=whole`); by default only the in-range bits are stored.
#[inline]
pub(crate) fn oob_write_discard(hi: i64, lo: i64, w: u32) -> bool {
    oob_select_whole() && (lo < 0 || hi >= w as i64)
}

/// Outcome of a two-state dynamic-offset slice read.
enum TsDyn {
    Ok(u64),
    Oob,
    X,
}

#[inline]
pub(crate) fn oob_range_select(base: &Value, hi: i64, lo: i64) -> Value {
    let per_bit = !oob_select_whole();
    if hi < lo {
        return Value::new(1);
    }
    let width = (hi - lo + 1) as u32;
    let w = base.width as i64;
    if lo >= 0 && hi < w {
        return base.range_select(hi as usize, lo as usize);
    }
    if !per_bit && !base.is_fill {
        return Value::new(width);
    }
    base.range_select_signed(hi, lo)
}

#[inline]
fn dyn_bit_index(v: &Value) -> Option<usize> {
    if v.has_xz() {
        return None;
    }
    Some(v.to_u64().unwrap_or(0) as usize)
}

/// Every array access in a compiled block lands here; on a CPU core all of
/// them are `Dense` (a bounds check and an add), so that arm is inlined
/// at the thirteen call sites and only the name-keyed lookup stays out of
/// line.
#[inline(always)]
fn resolve_bytecode_array_elem(
    array: &super::bytecode::ArrayOperand,
    idx: i64,
    array_first_id: &HashMap<Arc<str>, (usize, i64, i64)>,
    signal_name_to_id: &NameMap,
) -> Option<usize> {
    if let super::bytecode::ArrayOperand::Dense {
        first_id, lo, hi, ..
    } = array
    {
        return if idx >= *lo && idx <= *hi {
            Some(*first_id + (idx - *lo) as usize)
        } else {
            None
        };
    }
    resolve_bytecode_array_elem_named(array, idx, array_first_id, signal_name_to_id)
}

#[inline(never)]
fn resolve_bytecode_array_elem_named(
    array: &super::bytecode::ArrayOperand,
    idx: i64,
    array_first_id: &HashMap<Arc<str>, (usize, i64, i64)>,
    signal_name_to_id: &NameMap,
) -> Option<usize> {
    match array {
        super::bytecode::ArrayOperand::Dense { .. } => None,
        super::bytecode::ArrayOperand::Named(name) => array_first_id
            .get(name.as_str())
            .and_then(|&(first, lo, hi)| {
                if idx >= lo && idx <= hi {
                    Some(first + (idx - lo) as usize)
                } else {
                    None
                }
            })
            .or_else(|| {
                // Diagnostic for the slow by-name fallback (format! + String
                // hash per access — measured ~9% of c906 memcpy cycles when a
                // hot array lands here). XEZIM_DEBUG_NAMED_ARRAYS=1 prints
                // each distinct array name once.
                if std::env::var_os("XEZIM_DEBUG_NAMED_ARRAYS").is_some() {
                    use std::sync::{Mutex, OnceLock};
                    static SEEN: OnceLock<Mutex<std::collections::HashSet<String>>> =
                        OnceLock::new();
                    let seen = SEEN.get_or_init(|| Mutex::new(Default::default()));
                    if seen.lock().unwrap().insert(name.clone()) {
                        eprintln!("[NAMED-ARRAY] slow-path array operand: {}", name);
                    }
                }
                let elem_name = format!("{}[{}]", name, idx);
                signal_name_to_id.get(elem_name.as_str()).copied()
            }),
    }
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

/// IEEE 1800-2017 §18.4: where a member of an aggregate (struct/union) CLASS
/// property actually lives. Class properties are cells in `heap[h].properties`,
/// not signals, so member access has to be routed here rather than through the
/// flat-signal struct paths.
#[derive(Debug, Clone)]
enum ClassAggRef {
    /// Member of a PACKED struct/union — a bit slice `[off +: w]` of the
    /// property's raw `total`-bit integral value. Union members all sit at
    /// `off == 0` and therefore alias one another (§7.3.1).
    Packed {
        handle: usize,
        prop: String,
        off: u32,
        w: u32,
        total: u32,
        /// §7.2.1: a 2-state member of a 4-state struct — its reads and
        /// writes convert x/z to 0.
        two_state: bool,
    },
    /// Member of an UNPACKED struct — its own cell, keyed `<prop>.<member>`.
    Unpacked { handle: usize, key: String, w: u32 },
    /// A bit SLICE of one per-instance cell of an unpacked-struct property
    /// member: `[off +: w]` of the cell's `cell_w` bits. This is how a
    /// PACKED-ARRAY member of an unpacked struct is addressed (the whole
    /// array is one cell, §7.4.1 slot math picks the element) and how a
    /// part-select slices an element of an unpacked-array member.
    UnpackedSlice {
        handle: usize,
        key: String,
        off: u32,
        w: u32,
        cell_w: u32,
    },
}

/// Where a `class_packed_operand` value lives: the whole property, or an
/// element of its unpacked dimension(s) under the per-instance key
/// `<h>#<prop>[k]...` — the write counterpart splices back there.
#[derive(Clone, Debug)]
enum ClassPackedStore {
    Prop,
    ElemSuffix(String),
}

/// One step of a member/select chain being resolved into class aggregate
/// storage: a named member, a constant index, or a part-select.
#[derive(Debug, Clone)]
enum MemberSel {
    Member(String),
    Index(i64),
    Range { kind: RangeKind, l: i64, r: i64 },
}

/// One step of a member/select chain, UNEVALUATED — [`class_member_select_ref`]
/// resolves the receiver/property boundary before any index expression is
/// evaluated, so a chain rooted at no struct-valued class property never
/// pays for its selects.
enum PendingSel<'a> {
    Member(&'a str),
    Index(&'a Expression),
    Range {
        kind: RangeKind,
        left: &'a Expression,
        right: &'a Expression,
    },
}

/// How one select of a chain addresses the packed value it is applied to
/// (see [`Simulator::packed_hops`]), before any index is evaluated.
#[derive(Debug, Clone, Copy)]
enum PackedHop {
    /// A packed dimension declared `[l:r]` of elements `ew` bits wide.
    /// §7.4.1: the left bound labels the most significant element.
    Dim { l: i64, r: i64, ew: u32 },
    /// A member of a packed struct or union, `off` bits above its LSB and
    /// `w` bits wide.
    Field { off: u32, w: u32 },
}

/// The §7.8 collection kind of an unpacked-struct member whose declarator
/// carries an unsized `[]`, queue `[$]` or associative `[k]` dimension.
/// (Distinct from the rand-collection [`CollKind`]: this one separates the
/// queue and dynamic registrations.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StructCollKind {
    Dynamic,
    Queue,
    Assoc,
}

/// IEEE 1800-2017 §18.4: a constraint target that is not a bare rand property
/// of the object being solved.
#[derive(Debug, Clone)]
enum RandMemberTarget {
    /// A member of an aggregate (struct/union) rand property.
    Agg(ClassAggRef),
    /// A property of a `rand` OBJECT HANDLE — solved concurrently with the
    /// enclosing object (the handle itself is never written).
    Sub(usize, String),
}

/// IEEE 1800-2017 §21.7.2.1 `var_type` of a dumped object. Every `$var` used to
/// be hardcoded `wire`, which mislabels every variable in the design and hides
/// `event` semantics from the viewer entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VcdVarKind {
    Wire,
    Reg,
    Integer,
    Time,
    Real,
    Event,
    Parameter,
}

/// One expression-backed procedural continuous assignment/force.
///
/// Resolved signal reads carry the generation observed at the previous RHS
/// evaluation. Reads that cannot be mapped to compact signal storage retain a
/// conservative fallback so class/runtime-map expressions keep their prior
/// semantics.
#[derive(Clone)]
struct ActiveForceExpr {
    key: String,
    lvalue: Expression,
    rvalue: Expression,
    hint: Option<String>,
    read_signal_ids: Vec<usize>,
    read_epochs: Vec<u64>,
    has_unresolved_reads: bool,
    /// §10.6.2 bit/part-select force on a net (`partial_forces`): the reads
    /// are the net's drivers' and the forcing expressions'; `lvalue` is the
    /// whole net.
    partial: bool,
}

/// IEEE 1800-2023 §10.6.2: the forced bits of a net under bit/part-select
/// `force`. `bits[i]` names the piece (forcing expression) and the bit
/// within it that bit `i` takes; `None` bits follow the net's drivers.
struct PartialForce {
    bits: Vec<Option<(usize, u32)>>,
    /// (rvalue, select width, resolution hint) per `force` statement.
    pieces: Vec<(Expression, u32, Option<String>)>,
}

impl VcdVarKind {
    /// The §21.7.2.1 `var_type` keyword.
    fn keyword(self) -> &'static str {
        match self {
            VcdVarKind::Wire => "wire",
            VcdVarKind::Reg => "reg",
            VcdVarKind::Integer => "integer",
            VcdVarKind::Time => "time",
            VcdVarKind::Real => "real",
            VcdVarKind::Event => "event",
            VcdVarKind::Parameter => "parameter",
        }
    }
}

/// Name-keyed overflow signal store with a maintained ELEMENT INDEX:
/// every key containing `[` is also recorded under its base name (the text
/// before the first bracket), so collection machinery (queue/dynamic/assoc
/// locals stored as `name[i]` entries) can enumerate a name's elements
/// without scanning the whole map. The `starts_with` full-map scans in the
/// queue-frame save/restore were 25%+ of a UVM AVIP run.
///
/// Reads go through `Deref<Target = HashMap<..>>`; every mutation that can
/// add or drop a KEY must go through the inherent `insert`/`remove` below
/// (inherent methods shadow the deref'd HashMap ones), keeping the index
/// exact. `get_mut`-style value edits need no index work.
pub struct SignalMap {
    map: HashMap<String, Value>,
    elems: HashMap<String, std::collections::HashSet<String>>,
}

impl SignalMap {
    fn base_of(k: &str) -> Option<&str> {
        k.find('[').map(|i| &k[..i])
    }

    pub fn insert(&mut self, k: String, v: Value) -> Option<Value> {
        bump_store_gen();
        if let Some(b) = Self::base_of(&k) {
            if !self.map.contains_key(&k) {
                self.elems
                    .entry(b.to_string())
                    .or_default()
                    .insert(k.clone());
            }
        }
        self.map.insert(k, v)
    }

    pub fn remove(&mut self, k: &str) -> Option<Value> {
        bump_store_gen();
        let r = self.map.remove(k);
        if r.is_some() {
            if let Some(b) = Self::base_of(k) {
                if let Some(set) = self.elems.get_mut(b) {
                    set.remove(k);
                    if set.is_empty() {
                        self.elems.remove(b);
                    }
                }
            }
        }
        r
    }

    /// All stored keys beginning with `prefix`, which must end with `[`.
    /// Served from the element index: only the owning base name's keys are
    /// touched, never the whole map. Handles nested-bracket prefixes
    /// (`q[0].f[`) by indexing on the OUTERMOST base and filtering.
    pub fn keys_with_elem_prefix(&self, prefix: &str) -> Vec<String> {
        debug_assert!(prefix.ends_with('['));
        let base = &prefix[..prefix.len() - 1];
        let outer = Self::base_of(base).unwrap_or(base);
        self.elems
            .get(outer)
            .map(|set| {
                set.iter()
                    .filter(|k| k.starts_with(prefix))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Keys of every `base[...]` element currently stored, unordered.
    pub fn elem_keys(&self, base: &str) -> Vec<String> {
        self.elems
            .get(base)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Borrowing form of `keys_with_elem_prefix`: the element keys under
    /// `prefix` (which must end in `[`) without cloning them.
    pub fn elem_keys_with_prefix<'a>(
        &'a self,
        prefix: &'a str,
    ) -> impl Iterator<Item = &'a str> + 'a {
        let base = &prefix[..prefix.len().saturating_sub(1)];
        let outer = Self::base_of(base).unwrap_or(base);
        self.elems
            .get(outer)
            .into_iter()
            .flat_map(|set| set.iter().map(|k| k.as_str()))
            .filter(move |k| k.starts_with(prefix))
    }

    /// Is any key under the outermost base of `prefix` prefixed by it?
    /// Every key that contains `[` is indexed under the text before its
    /// FIRST `[`, so `obj[k][j]` and `obj[k].field` are both found under
    /// `obj`; this replaces a scan of the whole map.
    pub fn any_key_with_prefix(&self, prefix: &str) -> bool {
        let Some(outer) = Self::base_of(prefix) else {
            return self.map.keys().any(|k| k.starts_with(prefix));
        };
        self.elems
            .get(outer)
            .is_some_and(|set| set.iter().any(|k| k.starts_with(prefix)))
    }
}

impl std::ops::Deref for SignalMap {
    type Target = HashMap<String, Value>;
    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

impl std::ops::DerefMut for SignalMap {
    fn deref_mut(&mut self) -> &mut Self::Target {
        bump_store_gen();
        &mut self.map
    }
}

/// Mid-exec ts aborts tolerated per block before demotion.
const TS_ABORT_DEMOTE: u16 = 8;
/// Flag bit in a `ts_save_list` id marking a wide save entry (signal ids
/// stay below it).
const TS_SAVE_WIDE: u32 = 1 << 31;
/// Flag bit in a `ts_save_list` id marking a code coverage count to take back
/// when the block bails (the four-state re-run counts it again).
const TS_SAVE_COV: u32 = 1 << 30;
/// Consecutive x-read bails before a block is put to sleep.
const TS_XBAIL_STREAK: u8 = 8;
/// First sleep, in successful two-state evaluations design-wide (about half
/// a clock tick on a CPU SoC); doubles per further streak, up to 2^12 units.
const TS_XBAIL_SLEEP_UNIT: u64 = 256;
/// A gap this long between two bails of one block counts as a clean stretch
/// and restarts its streak.
const TS_XBAIL_FORGET: u64 = 1 << 16;

/// A sleeping two-state block: what was swapped out, and where.
struct TsSleepRec {
    kind: u8,
    idx: u32,
    len_kind: u16,
    slept_at: u64,
    plan: Option<CombPlan>,
    slot: Option<TsSlot>,
}

#[derive(Clone)]
enum CombPlan {
    Unresolved,
    /// Two-state island. The per-eval guard can still bail dynamically
    /// (forces, X inputs); the plan then falls through JIT-then-interp,
    /// exactly like the unplanned dispatch.
    Ts(std::sync::Arc<super::bytecode::TwoStateBlock>),
    /// Native slot; its function is stored in `comb_jit_fns`.
    #[cfg(feature = "jit")]
    Jit,
    /// Template slot; its function and mapping window are in `comb_tpl_fns`.
    #[cfg(feature = "jit")]
    Tpl,
    Interp,
}

pub struct Simulator {
    pub signals: SignalMap,
    vpi_port_directions: HashMap<Arc<str>, PortDirection>,
    /// Signals currently under force/release control (LRM §9.3.1).
    forced_signals: HashMap<usize, Value>,
    /// §10.6.2 bit/part-select forces on nets, by signal id. Each id is
    /// also in `forced_signals` (so ordinary writes drop) and has a
    /// `partial` `active_force_exprs` entry that re-merges the drivers.
    partial_forces: HashMap<usize, PartialForce>,
    /// Nets released while a settle pass was running (the entry table is
    /// taken out then); re-driven when the pass ends.
    pending_release_ids: Vec<usize>,
    /// Memoization for `prop_bound_collection`: (class, member) → resolved
    /// raw type name (hierarchy walk), and type name → is-collection verdict
    /// (which otherwise LINEAR-SCANS every class's local typedefs — 5% of an
    /// AVIP run). Class/typedef tables are elaboration-static, and per-handle
    /// type bindings are applied AFTER the cached raw lookup, so both maps
    /// are sound to memoize for the life of the run.
    prop_raw_ty_cache: std::cell::RefCell<HashMap<String, HashMap<String, Option<String>>>>,
    typedef_target_cache:
        std::cell::RefCell<HashMap<String, HashMap<String, Option<crate::ast::types::DataType>>>>,
    collection_dim_cache: std::cell::RefCell<HashMap<String, bool>>,
    /// Sticky: set the first time a `<base>.` frame marker is created (an
    /// unpacked-struct formal binding, or a struct-return variable via
    /// `unpacked_struct_leaf_keys`); gates `frame_leaf_key_of`'s per-eval
    /// probe.
    any_struct_formal_markers: bool,
    /// Recycled subroutine-frame maps: every call built a fresh
    /// `HashMap<String, Value>` and dropped it on return — at UVM call rates
    /// the table alloc/free was pure allocator churn. `clear()` keeps the
    /// table's capacity, so a pooled frame re-binds formals with zero
    /// rehashing growth.
    /// Bumped whenever any `active_force_signal_epochs` entry advances;
    /// `refresh_active_forces` early-exits when nothing bumped since its
    /// last completed pass (the post-settle refresh otherwise re-scanned
    /// every armed force per settle — 1.6% of an axi4Lite run).
    force_epoch_gen: u64,
    force_refresh_done_gen: u64,
    /// XEZIM_NAME_STATS counters for the string-keyed read path (the UVM
    /// "runtime IDs" investigation). 0=get_signal_value_by_name,
    /// 1=resolve_hier_name, 2=subroutine frames pushed, 3=total locals in
    /// them — the last pair sizes the frame maps, which measured a mean of
    /// 1.8 entries and so would be better served by a linear-scan Vec than
    /// by hashing a String key.
    /// Flattened per-class collection-property lookup for
    /// `instance_assoc_member`: class -> (member -> kind), built once by
    /// walking the ancestry, so the 22.3M-call hot path costs two hash
    /// lookups instead of four map probes plus a linear scan PER ANCESTOR
    /// LEVEL. `true` = per-instance collection (`<handle>#<member>`);
    /// `Some(class)` = static fixed array owned by that class.
    #[allow(clippy::type_complexity)]
    class_coll_index: std::cell::RefCell<HashMap<String, HashMap<String, Option<String>>>>,
    /// Per class: bare names that NO class in its inheritance chain declares
    /// as a property (locals, module signals, method formals). Every bare
    /// identifier evaluated inside a class method asked
    /// `instance_assoc_member`, which walked the chain and probed the
    /// collection tables per level, on every evaluation; such a name can
    /// never become a collection member, so the verdict is cached.
    class_non_member_cache: std::cell::RefCell<HashMap<String, HashSet<String>>>,
    /// Runtime class -> bare name -> `instance_assoc_member` verdict.
    member_coll_cache: std::cell::RefCell<HashMap<String, HashMap<String, MemberCollKind>>>,
    /// Per class: properties that are NOT structs (see `class_prop_struct`).
    class_prop_struct_neg: std::cell::RefCell<HashMap<String, HashSet<String>>>,
    name_stats: [std::cell::Cell<u64>; 5],
    name_stats_on: bool,
    frame_pool: Vec<HashMap<String, Value>>,
    /// Name-keyed type registrations (`widths`, `signed_signals`,
    /// `real_signals`, `string_signals`) that a fallback carrying locals
    /// replaced for its duration: (name, width, signed, real, string) as they
    /// were before. See `fb_frame_push`.
    fb_meta_saves: Vec<(String, Option<u32>, bool, bool, bool)>,
    /// `local_stack` depth below each active fallback frame (see
    /// `fb_frame_push`): a write inside one leaves 2-state fitting to the
    /// copy-back, since the carried locals' types are not in
    /// `var_decl_types`.
    fb_frame_depths: Vec<usize>,
    /// Lazily-built leaf index over `module.parameters` for the bytecode
    /// compiler's suffix-match fallback (see `lookup_param_value`). Built
    /// once — parameters are final before any bytecode compilation runs.
    param_leaf_index_cell: std::cell::OnceCell<HashMap<String, Vec<String>>>,
    /// Lazily-built leaf (`::`-suffix) index over `module.var_decl_types`
    /// for `declared_collection_elem_class`'s package-qualified fallback.
    /// The id-collision-in-heap gates consult that fallback on EVERY
    /// member/indexed access, and the old whole-map `rsplit(':')`-per-key
    /// scan was ~5.5ms per access on a 370K-signal SoC — 96% of a
    /// production run's CPU (a 152s TB ballooned past 26 minutes). Index
    /// entries are CANDIDATE KEYS only; lookups stay authoritative against
    /// the live map, so runtime insertions (formal/foreach metadata,
    /// always keyed by the exact bare name the direct probe already
    /// covers) can neither be missed nor stale-hit.
    var_decl_leaf_index_cell: std::cell::OnceCell<HashMap<String, Vec<String>>>,
    /// Flattened names currently under an active `force` (§10.6) or
    /// procedural continuous `assign` (§10.6.1) whose storage lives only
    /// in the runtime `signals` map (no compact signal-table id). Mirrors
    /// `forced_signals` for those targets so the name-keyed write
    /// fallbacks can drop ordinary writes while the override is active.
    forced_names: HashSet<String>,
    /// §13.3.1: last value of each STATIC function's implicit return
    /// variable (key: pkg-scope + fn name). Seeds the next call's return
    /// slot so an unassigned exit path returns the previous value.
    static_fn_ret: HashMap<String, Value>,
    /// §10.6.2: an active force/procedural-assign whose RHS is an
    /// EXPRESSION acts as a continuous assignment — it re-evaluates when
    /// its operands change until release. (target key, lvalue, rvalue,
    /// scope hint at arm time). One-shot snapshots diverged: `force n =
    /// src + 1` held the arm-time value forever.
    active_force_exprs: Vec<ActiveForceExpr>,
    /// Per-signal write generation used by `active_force_exprs`. Allocated
    /// alongside the signal table and touched only while an expression-backed
    /// override is active.
    active_force_signal_epochs: Vec<u64>,
    /// Deterministic work counters for regression tests: RHS evaluations and
    /// clean dependency checks skipped.
    active_force_evals: u64,
    active_force_skips: u64,
    /// Fast signal table: indexed by signal_id for O(1) access.
    signal_table: Vec<Value>,
    /// Packed byte storage for admitted large narrow arrays; cells addressed
    /// by ids at or above `PACKED_BASE` (see `PackedMem`). Empty unless
    /// `XEZIM_PACKED_MEM=1`.
    packed: PackedMem,
    /// Parallel array: 1 byte per signal_id, non-zero iff that signal
    /// currently has any X/Z bits. Maintained by `write_sig!` and a few
    /// in-place mutators. Used by the JIT's inline X/Z prelude (avoids
    /// a C-bridge call per block dispatch). Sized once in `compile()`
    /// to match `signal_table.len()` and never resized — the address of
    /// `signal_has_xz.as_ptr()` is JIT-baked at compile time and must
    /// stay valid for the simulator's lifetime.
    pub(crate) signal_has_xz: Vec<u8>,
    /// Per-destination sidecar roles used by direct commit paths. Bit 0 marks
    /// edge-history participation; bit 1 marks armed-input fanout.
    signal_commit_plan: Vec<u8>,
    /// Parallel storage to `signal_table`: `[val_bits, xz_bits]` per
    /// signal in a JIT-friendly flat layout.  Future JIT codegen can
    /// read signal values by direct pointer offset (base + id * 16)
    /// instead of crossing the FFI boundary to call
    /// `xezim_jit_load_signal`.  Allocated only when
    /// `XEZIM_INLINE_BITS=1`; default is empty (zero overhead).
    /// Maintained synchronously by `write_sig!` macro and the
    /// canonical `after_signal_write` helper — see
    /// JIT-REDESIGN-NOTES.md for the audit context.
    pub(crate) signal_inline_bits: Vec<[u64; 2]>,
    /// JIT Stage 4 Path C1: side queue for inline JIT NBA writes.
    /// JIT-generated code writes `(signal_id, val_bits)` u32+u64 pairs
    /// directly via a baked-in pointer + the length counter below.
    /// Before `apply_nba` runs, `drain_jit_nba_side_queue` copies the
    /// entries into the canonical `nba_fast` Vec via the existing
    /// elision-and-resize logic.  Pre-allocated to 1M slots × 16
    /// bytes = 16 MB; never resized so the JIT's baked pointer
    /// stays valid for the simulator's lifetime.  Allocated only
    /// when `XEZIM_INLINE_BITS=1`.
    pub(crate) jit_nba_side_queue: Vec<JitNbaSideEntry>,
    /// Length counter for `jit_nba_side_queue`.  JIT writes increment
    /// this directly.  `drain_jit_nba_side_queue` resets to 0 after
    /// transferring entries to nba_fast.
    pub(crate) jit_nba_side_len: u32,
    /// Map signal name → signal_id for fast lookup. `Arc<str>` keys are
    /// shared with `id_to_name` so each signal name lives on the heap
    /// once instead of twice (saves ~25 MB on c910-scale). The elements of
    /// named 1-D arrays are VIRTUAL entries (see `NameMap`).
    signal_name_to_id: NameMap,
    /// Compile-time scope resolver: bare leaf name -> enclosing scopes that
    /// declare that leaf. Avoids rescanning the complete flattened namespace
    /// for every continuous assignment and combinational block.
    scope_parents_by_leaf: HashMap<Arc<str>, Vec<Arc<str>>>,
    /// Lazy per-array element-ID cache: array_name → Vec where index is the
    /// element index and the value is the flat signal_id (None if missing).
    /// Populated on first miss in the hot write/read path; avoids per-access
    /// `format!("{}[{}]", name, i)` + HashMap lookup on tight loops like
    /// memory wipe/load in testbenches.
    array_elem_ids: HashMap<String, Vec<i64>>,
    /// Compact resolver for 1D unpacked-array elements: `base_name` →
    /// (`first_id`, `lo`, `hi`). The N elements of a 1D array occupy
    /// contiguous signal_ids `first_id..=first_id + (hi-lo)`, so name-style
    /// lookups like `"tb.foo.mem[5]"` can be resolved by parsing the suffix
    /// and computing `first_id + (idx - lo)` instead of materialising
    /// `signal_name_to_id` and `id_to_name` entries for every element.
    /// On E902 this saves ~107 MB (~852K array cells; HashMap entries +
    /// per-element Arc<str> strings) and on c910 / large-memory designs
    /// scales linearly with memory depth.
    array_first_id: HashMap<Arc<str>, (usize, i64, i64)>,
    /// Reverse index: leaf-name (last `.`-segment of a signal's full path) →
    /// signal_ids that share that leaf. Built once for the "real" signal set
    /// (excludes per-element array IDs which all end with `]`). Keeps the
    /// `path.len()==1 && bare leaf` resolution path O(1) instead of an
    /// O(N) scan over signal_name_to_id, which on c910 scales to 35M entries
    /// and was the dominant cost (575s) of time-0 settle for testbench-probe
    /// continuous assigns like `assign x = bare_leaf;`.
    leaf_name_to_ids: HashMap<Arc<str>, Vec<usize>>,
    /// Signal id → registered name; array cells without a stored name are
    /// gaps (see `IdNames`), resolved through `name_for_id`.
    id_to_name: IdNames,
    /// Element names of virtual-name arrays, built on first request by
    /// `name_for_id` (append-only: entries are never replaced or removed,
    /// which is what lets it hand out `&str` borrowed from `&self`).
    elem_name_cache: RefCell<HashMap<usize, Box<str>>>,
    /// Indices into `module.instances`, sorted by instance path. Hierarchical
    /// upward-name resolution probes instance paths repeatedly; binary search
    /// avoids a full instance-list scan without duplicating the long paths.
    instance_path_order: Vec<usize>,
    /// Map signal_id → width (for fast width lookup).
    signal_widths: Vec<u32>,
    /// Set of signal IDs that are signed.
    signal_signed: Vec<bool>,
    signal_real: Vec<bool>,
    /// Signal ids whose declared type is 2-state (`bit`/`byte`/`int`/…). Per
    /// §6.11.1/§10.7 an implicit conversion of a 4-state RHS into such a target
    /// maps X/Z to 0 — enforced in `fit_value_to_signal`.
    signal_two_state: Vec<bool>,
    /// Signal ids driven by a lowered GATE PRIMITIVE. §28.4's `buf` truth table
    /// maps a `z` input to an `x` output, but §10.3 continuous assignment passes
    /// the value through unchanged — and a gate lowers to an ordinary continuous
    /// assign, so by the time one-bit copies are fused into a buffer op the two
    /// are indistinguishable. Applying the gate rule to both made every
    /// `assign y = x;` eat `z`. This says which ids the gate rule applies to.
    signal_gate_driven: Vec<bool>,
    /// Signal ids whose declared type is `string`. §6.16 makes a string a
    /// DYNAMIC type with no length limit, but it is stored in the fixed-width
    /// signal table like everything else — `resolve_type_width` hands it 1024
    /// bits, i.e. 128 characters. Fitting a value to that width silently
    /// truncated every longer string (a 130-char concat came back 128, and the
    /// dropped end took a `{base, "TAIL"}` suffix with it). These ids skip the
    /// width fit and keep whatever length they are given.
    signal_is_string: Vec<bool>,
    /// `--warn-x` state. `warn_x` is a plain field (not the atomic) so the
    /// write hot path costs one bool load; everything else is touched only
    /// when a report actually fires.
    warn_x: bool,
    /// Signals already recorded, so a register that re-loads an X input every
    /// cycle is named once rather than ten thousand times. `RefCell` because
    /// `write_sig!` expands in contexts that already hold a `&mut` borrow of
    /// another field (the clock-generator loop), so the hook can only touch
    /// disjoint fields — never `&mut self` as a whole.
    warn_x_seen: std::cell::RefCell<HashSet<usize>>,
    /// Captured at the moment of the write — id, time, and the value — and
    /// formatted later, once a `&mut self` is available again.
    /// id, time, value, and the process that performed the write. The pid is
    /// the only attribution available for a procedural write: `initial`/`always`
    /// bodies are drained into processes at compile time, so they are not in
    /// `module.initial_blocks` any more, but `process_origin` still knows what
    /// each pid came from and where.
    warn_x_pending: std::cell::RefCell<Vec<(usize, u64, Value, usize, Option<String>)>>,
    /// Dedup for entries identified by NAME rather than id — associative and
    /// other elements that live in the untyped `signals` map and so have no
    /// entry in the signal table.
    warn_x_seen_named: std::cell::RefCell<HashSet<String>>,
    warn_x_count: usize,
    /// Lazily built on the first report: signal id -> one line per driver.
    /// Derived from the comb entries and edge blocks on demand, so a run
    /// without `--warn-x` pays nothing for it.
    warn_x_drivers: Option<HashMap<usize, Vec<String>>>,
    /// Signal ids that carry a CONTINUOUS driver — the whole-name LHS of an
    /// `assign` (including the continuous-assigns that inlining synthesizes for
    /// instance port connections). §6.5 forbids mixing continuous and procedural
    /// drivers on a variable, so this is exactly "has no procedural driver", and
    /// §21.7.2.1's `var_type` for such an object is `wire` — what a reference simulator emits
    /// and what makes a viewer colour it as a net. `module.continuous_assigns` is
    /// drained into bytecode at compile time, so the fact is recorded here while
    /// it is still known.
    cont_driven: HashSet<usize>,
    /// Names whose sole continuous driver was an identity port connect that
    /// collapsed into a shared id. The dropped connect made them wire-typed
    /// for a dump, and the shared id can't carry per-name typing.
    collapsed_port_children: HashSet<String>,
    /// Set by a `collapse_identity_port_nets` run that reached its fixed
    /// point: the name-map generation it left, the continuous-assign list
    /// (length, buffer) and port-alias count it read, and whether its buffer
    /// pass ran. A later run over the same inputs has nothing to re-point.
    collapse_settled: Option<(u64, usize, usize, usize, bool)>,
    /// Instantiated always blocks awaiting classification, in order (see
    /// `DeferredAlways`); empty once `classify_always_blocks` has run.
    deferred_always: Vec<DeferredAlways>,
    /// Sparse: signal_id → declared user type name (e.g. class/struct
    /// type for `MyClass h;`). Only populated for signals where the
    /// elaborator recorded a non-None `type_name` on the source
    /// `Signal` struct — most regular bit/logic signals have None
    /// here. Populated once at construction so `module.signals` can
    /// be freed afterwards.
    signal_type_names: HashMap<usize, String>,
    pub widths: HashMap<String, u32>,
    pub signed_signals: HashSet<String>,
    /// Names of ARRAY locals whose packed element geometry
    /// `exec_stmt_var_decl` registered in `packed_full_dims` (so a later
    /// same-named array local can drop it).
    array_local_packed_dims: HashSet<String>,
    /// Field layout of the packed-struct element of the array local being
    /// declared, for `declare_local_dyn` to register under the per-call key.
    /// The bare name's previous entry is restored once the layout moves to
    /// that key: (layout, bare name, previous entry).
    #[allow(clippy::type_complexity)]
    local_elem_layout: Option<(
        Vec<(String, u32, u32)>,
        String,
        Option<Vec<(String, u32, u32)>>,
    )>,
    pub real_signals: HashSet<String>,
    /// Base names of 2D/ND UNPACKED arrays (union of `module.arrays_2d` and
    /// `module.arrays_nd` keys). The bytecode compiler consults this so a
    /// continuous-assign LHS `m[0][j]` on a genuine 2D array is NOT mistaken
    /// for a flattened-signal bit-select (`flattened_outer_zero_signal_id`) —
    /// it must bail to the interpreter's element-aware path like `m[1][j]`
    /// already does. Computed once at construction.
    pub multi_dim_array_names: HashSet<String>,
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
    /// Snapshot generation counter: bumped every `snapshot_edge_signals`.
    /// (Previously read by `EventWaiter::registered_snap_gen` to guard
    /// pre-registration edges; that mechanism was replaced by per-signal
    /// `captured_prev` baselines captured at arm time — see `EventWaiter` —
    /// which handles the NBA-region / same-tick cases directly. The counter
    /// is retained for diagnostics/future use but is currently write-only.)
    snap_gen: u64,
    edge_signal_names: HashSet<String>,
    /// Edge sensitivity resolved to signal IDs.
    edge_signal_ids: Vec<usize>,
    /// Signals registered by sampled-value watches at run time (`$rose`,
    /// `$past`, …): their preponed value lives in `prev_*` and is read by
    /// `preponed_value_of`, so the incremental snapshot always walks them.
    preponed_ids: Vec<usize>,
    /// Reverse index signal_id → edge_block_idx lists by edge kind, built once after
    /// edge_blocks are classified. Lets `check_edges` iterate edge-sensitive
    /// signals (typically ~100s on c910: clk, rst_b, a few enables) instead
    /// of all edge_blocks (10K+), yielding 50-100× faster edge detection.
    edge_blocks_by_sig: Vec<EdgeFanout>,
    /// Indices (into `edge_blocks`) of edge-sensitive always blocks that
    /// carry at least one `iff` guard on a sensitivity term (LRM §9.4.2.3).
    /// Built alongside `edge_blocks_by_sig`; empty for the common
    /// guard-free design, so `check_edges` skips the guard pass entirely.
    edge_blocks_with_iff: Vec<usize>,
    /// Per-block scratch bitmap (indexed by edge-block index) marking blocks
    /// whose `iff` guard denies firing this delta-cycle. Rebuilt each
    /// `check_edges` before the detection loop so the hot loop can skip a
    /// denied block with a single array read.
    edge_iff_denied: Vec<bool>,
    pub time: u64,
    pub output: Vec<SimOutput>,
    capture_output: bool,
    pub finished: bool,
    /// §20.10: a `$fatal` executed, or the run ended on a run-time error
    /// (`abort_run`) — the run must not exit 0.
    pub saw_fatal: bool,
    /// `$fatal`'s finish_number (0/1/2), for the exit status.
    pub fatal_finish_number: Option<i32>,
    /// Count of `$error` occurrences (see `--error-exit`).
    pub error_count: u64,
    /// Compile-time diagnostics that must prevent simulation (e.g. an illegal
    /// §6.18 assignment of a non-class value to a class handle). The CLI and
    /// library `simulate()` report these and stop; simulation never runs.
    pub compile_errors: Vec<String>,
    /// Set when the dead-clock watchdog aborted the run (XEZIM_STUCK_CLOCK=abort).
    /// The CLI reports a non-zero exit so CI/regressions fail fast.
    pub stuck_clock_aborted: bool,
    compiled: bool,
    pub monitor: Option<(String, Vec<Expression>)>,
    /// `$monitoroff` pauses (not destroys) the monitor; `$monitoron` resumes.
    pub monitor_paused: bool,
    /// System-task names already warned about this run — both the generic
    /// "unknown system task" diagnostic and the recognized-but-unsupported
    /// stubs ($save, $asserton, …) emit ONCE per distinct name so a task in
    /// a clocked loop cannot flood stderr.
    warned_system_tasks: HashSet<String>,
    /// `$strobe` queue — (task name, args, arming instance scope, arming
    /// `%m` lexical suffix): formatted+printed at the end of the current
    /// event-loop iteration, after `apply_nba` has committed scheduled
    /// non-blocking writes. Each entry is `(task_name, args)` and is
    /// re-evaluated at drain time so the printed values reflect the
    /// post-NBA state per IEEE 1800 §15.3.4 (postponed region).
    pending_strobes: Vec<(String, Vec<Expression>, String, String)>,
    /// IEEE 1800-2017 §4.4 "observed region" — drained AFTER the NBA region
    /// has settled and BEFORE the postponed region runs. Concurrent
    /// property/sequence checkers (LRM §16) register a probe expression
    /// here; the runtime evaluates each probe and records the boolean
    /// outcome. Probe = (property-name, predicate-expr, current-instance
    /// `this` handle).
    ///
    /// Today this queue is plumbing only — no producer is wired up.
    /// Concurrent assertions land in M9, which depends on this region
    /// existing. Drain is a no-op when empty so behavior is unchanged.
    pending_observed: Vec<ObservedProbe>,
    /// §16.4 deferred immediate assertion reports awaiting maturity.
    deferred_asserts: Vec<DeferredReport>,
    /// §20.12: an assertion control is in force (the control log is not
    /// empty); every assertion path tests only this until then.
    assert_ctl_on: bool,
    assert_ctl: Option<Box<AssertCtl>>,
    /// `typedef_layout_member`'s unpacked-struct typedef layouts by total
    /// width; dropped when a block-local typedef is registered.
    typedef_layouts_by_width: Option<HashMap<u32, Vec<Vec<(String, u32, u32, bool)>>>>,
    /// Class packed ranges needing label translation, keyed by (class,
    /// property); None for descending ranges ending at zero.
    class_prop_dim_cache: std::cell::RefCell<HashMap<(String, String), Option<(i64, i64)>>>,
    /// LRM §16.5 SVA clocked-assertion sites. Populated lazily on
    /// the first time the AssertionStatement{is_property,
    /// expr:SvaClocked} executes. Each site's saved event values let us
    /// edge-detect the clock signal each loop iteration; `pending`
    /// queues consequents deferred by `|=>` or `##N`.
    sva_sites: Vec<SvaClockedSite>,
    /// §16.10: the local variables of the attempt thread being evaluated,
    /// read through slot markers (see `sva_bind_locals`).
    sva_lv: Vec<Value>,
    /// `(width, signed, two-state)` of each local variable slot of the site
    /// being registered (see `sva_bind_locals`).
    sva_lv_types: Vec<(u32, bool, bool)>,
    /// §16.14.6.2: procedural concurrent assertion instances queued this time
    /// step, not yet matured: `(site, process, captured slot values)`. A
    /// flush point of the process drops its own.
    sva_proc_pending: Vec<(usize, usize, Vec<Value>)>,
    /// §16.15 `default disable iff`, by the scope that declares it.
    sva_default_disable: HashMap<String, Expression>,
    /// LRM §16.9.3: index of the SVA site whose body is currently
    /// being evaluated. Set by `tick_sva_sites` around each body
    /// eval; consulted by `eval_expr`'s `$past` handler to find the
    /// site's snapshot map. `None` when no SVA body is active —
    /// `$past` then returns 0.
    active_sva_site: Option<usize>,
    /// LRM §16.5.1 Preponed-region sample cache for clocked concurrent
    /// assertions. Keyed by signal id, holds each SVA-referenced signal's
    /// value as of the START of the current time slot (captured at slot
    /// entry, before the active/NBA regions run). When a clocked property
    /// fires on its clock edge, its predicate is evaluated against these
    /// sampled values rather than the live post-NBA values, so a signal
    /// that changes on the SAME edge is seen with its old (preponed) value
    /// — matching the §16.5.1/§16.9 sampled-value semantics instead of
    /// being one clock cycle early. Only populated when `sva_sites` is
    /// non-empty; the referenced ids are those collected per site.
    sva_preponed: HashMap<usize, Value>,
    /// The time slot `sva_preponed` was captured for. The Preponed region
    /// runs once per slot (§4.4.2.1), so the later deltas of a slot (a VPI
    /// write from cbReadWriteSynch reopening the Active region) keep it.
    sva_preponed_slot: Option<u64>,
    /// A site's clock event is recorded (`fired`) and not evaluated yet.
    sva_owed: bool,
    /// `sva_post_nba` is running the slot's due cbReadWriteSynch batches.
    sva_post_nba_busy: bool,
    /// LRM §14.3 clocking-block per-cycle input snapshots. Each entry
    /// is `cb_name → (signal_name → pre-edge value)`. Updated in
    /// `tick_clocking_blocks` just before the clock signal's posedge
    /// is consumed by edge-triggered always blocks. Reads of
    /// `cb.<signal>` (where the signal is direction `input`) return
    /// from this snapshot — the `#1step` input skew of LRM §14.3.
    /// Without the snapshot, `cb.sig` would return the post-edge
    /// value, defeating monitor patterns that rely on stable
    /// pre-edge sampling.
    clocking_snapshots: HashMap<String, HashMap<String, Value>>,
    /// LRM §14.3: per-cb metadata. `(clock_signal_name, [(signal_name, is_input)])`.
    /// Populated at compile() from `module.clocking_blocks`.
    clocking_meta: HashMap<String, (String, Vec<(String, bool)>)>,
    /// LRM §14.4: pending clocking-output drives. `cb_name → [(signal,
    /// value)]`. `cb.<out_sig> <= val` queues here instead of driving
    /// immediately; `tick_clocking_blocks` applies them at the next
    /// posedge of the cb's clock (output skew).
    clocking_output_pending: HashMap<String, Vec<(String, Value)>>,
    /// Per-clocking-block previous clock bit (for edge detection).
    clocking_prev_clock: HashMap<String, u8>,
    /// Per-tick clock poll table for `tick_clocking_blocks` (see there).
    clocking_tick_cache: Vec<ClockingTick>,
    /// Buffer-collapse exclusions (override-target leaves, procedural write
    /// names), collected once while the process bodies are still present.
    buf_collapse_write_sets: Option<(HashSet<String>, HashSet<String>)>,
    /// Last source value of constant packed-memory element connections.
    /// Unresolved dependency polling must not overwrite a VPI deposit when
    /// the connection's source has not changed (§23.3.3.3, §38.34).
    packed_array_copy_values: HashMap<(usize, usize), Value>,
    /// LRM §14.3: each clocking block's clock EDGE (`@(posedge/negedge clk)`).
    /// `tick_clocking_blocks` and `@(cb)` resolution consult this so a
    /// `@(negedge clk)` block samples/advances on the negedge, not the posedge.
    /// Absent ⇒ posedge (the default and by far the common case).
    clocking_edge: HashMap<String, EdgeKind>,
    /// Set when `tick_clocking_blocks` published a changed sample mirror, so
    /// the caller runs one more edge pass — the publish happens after this
    /// slot's edge detection, and the next iteration's snapshot would
    /// otherwise absorb the change and swallow the `@(cb.sig)` event.
    clocking_mirror_dirty: bool,
    /// §14.16 `cb.out <= ##N v` drives awaiting N further clocking events:
    /// cb → [(remaining_cycles, net, value)]. Decremented at each edge of that
    /// block; applied (with output skew) when it reaches zero.
    clocking_cycle_pending: HashMap<String, Vec<(u64, String, Value)>>,
    /// §14.3 signal RENAMING (`input alias = net;`): cb → (clocking name →
    /// resolved net). The snapshot/meta store the NET; reads and drives use
    /// the clocking name, so they translate through this first.
    clocking_sig_alias: HashMap<String, HashMap<String, String>>,
    /// §14.4 per-cb default OUTPUT skew in ticks (absent = #0).
    clocking_out_skew: HashMap<String, u64>,
    /// §14.4 per-signal output-skew overrides: cb → (resolved net → ticks).
    clocking_sig_out_skew: HashMap<String, HashMap<String, u64>>,
    /// §14.4 per-cb default INPUT skew in ticks (absent = #0).
    clocking_in_skew: HashMap<String, u64>,
    /// §14.4 per-signal input-skew overrides: cb → (resolved net → ticks).
    clocking_sig_in_skew: HashMap<String, HashMap<String, u64>>,
    /// Sim time of each cb's most recent clock edge — a `cb.out <= v` executed
    /// AT the edge time drives at edge+skew (§14.16.1); between edges it waits
    /// for the next edge.
    clocking_last_edge: HashMap<String, u64>,
    /// The `default clocking` block's real name — `##0` consults its
    /// `clocking_last_edge` entry to decide "already at the clocking event".
    default_clocking_cb: Option<String>,
    /// LRM §14.4 `#1step` input skew — Preponed (slot-entry) value of every
    /// clocking-block INPUT signal, keyed by signal name. Captured at the top
    /// of each time slot BEFORE callback writes and active/NBA regions run,
    /// so when a clock posedge fires later in the slot the
    /// input snapshot is the value from *before* the edge, not the same-edge
    /// NBA update. Without this, `cb.<in>` reads the post-edge value (off by
    /// one clock relative to a reference simulator).
    clocking_preponed: HashMap<String, Value>,
    /// Slot whose Preponed clocking-input values have already been captured.
    clocking_preponed_slot: Option<u64>,
    /// §14.4 `input #N` history per sampled net (see `ClockingHist`). Empty
    /// unless some clocking input has an explicit positive skew.
    clocking_hist: HashMap<String, ClockingHist>,
    /// Time of the slot `refresh_clocking_preponed` last ran in.
    clocking_hist_slot: Option<u64>,
    /// §14.3 clocking-visible name of each `clocking_meta` signal, in the
    /// same order. Snapshots are keyed by it, so `input d; input #3 dl = d;`
    /// keeps two samples of one net.
    clocking_vis: HashMap<String, Vec<String>>,
    /// §14.16.2 inout clockvars: cb → nets that are BOTH sampled and driven.
    clocking_inouts: HashMap<String, HashSet<String>>,
    /// Continuations of clocking-event waiters (`@(cb)` / `##N`) that fired this
    /// slot, held for resumption in the Reactive region — after `apply_nba` and
    /// `tick_clocking_blocks` — instead of running in the Active region with the
    /// raw edge. See `EventWaiter::is_clocking`.
    deferred_clocking_conts: Vec<(usize, ProcCont)>,
    /// §9.4.2.3/§14.13: clocking-event waiters (`@(cb iff g)`) whose edge
    /// fired this slot but whose `iff` guard is judged only once the block
    /// has sampled — the clocking event, and so its guard, occurs after the
    /// clockvars update. Each entry holds the waiter and the guard of the
    /// term that fired; see `resolve_deferred_clocking_guards`.
    deferred_clocking_guarded: Vec<(EventWaiter, Box<Expression>)>,
    /// LRM §4.4 "reactive region" — drained AFTER the observed region
    /// has fired and BEFORE the postponed region runs. In a strict LRM
    /// implementation this hosts `program`-block procedural code so a
    /// testbench observes design state only after design NBAs have
    /// settled. Today the queue is framework-only (no producer is
    /// wired up); drain is a no-op when empty, so behavior is unchanged.
    /// Each entry is one program `initial` block: its statements and its
    /// instance scope (`""` for a top-level program).
    pending_reactive: Vec<(Vec<crate::ast::stmt::Statement>, String)>,
    /// §24.3 / §4.4: the processes of `program` blocks (their initial
    /// blocks and every fork child they spawn). They resume in the Reactive
    /// region, after the slot's NBA updates and clocking events.
    program_pids: HashSet<usize>,
    /// §24.3: program-process continuations woken in the Active region (an
    /// edge, a delay, a join), held for this slot's Reactive region.
    reactive_conts: Vec<(usize, ProcCont)>,
    /// Set while `drain_reactive_region` runs program processes.
    in_reactive_drain: bool,
    /// §7.9.11: the `default:` value of a class associative-array member
    /// (`<handle>#<member>`) that an assignment pattern gave it, read for
    /// every key the array does not hold.
    class_assoc_defaults: HashMap<String, Value>,
    /// §7.2.1: variables of a 4-state packed struct type with 2-state
    /// members (`struct packed {bit [3:0] b; logic [3:0] l;}`). Such a
    /// member converts x/z to 0 on every read and write, which the compiled
    /// member slices do not do, so their field layouts are withheld from the
    /// bytecode compiler (`bc_packed_fields`) and the interpreter serves them.
    mixed_packed_vars: HashSet<String>,
    /// §8.9: the storage object of each STATIC unpacked-struct property,
    /// keyed by its static key (`Cls::prop`): a heap object of the declaring
    /// class that holds the member cells (`<prop>.<member>`) every access —
    /// `Cls::prop.m`, `obj.prop.m`, `prop.m` in a method — shares.
    static_struct_holders: HashMap<String, usize>,
    /// Names of the static unpacked-struct properties any class declares
    /// (gates the `static_struct_holders` lookups).
    static_struct_names: HashSet<String>,
    /// `module.packed_struct_fields` without the `mixed_packed_vars` entries;
    /// `None` when there are none.
    bc_packed_fields: Option<HashMap<String, Vec<(String, u32, u32)>>>,
    pub monitor_prev: HashMap<String, Value>,
    /// Previously-printed values of the active `$monitor`'s non-`$time`
    /// arguments. `$monitor` re-prints only when one of these changes
    /// (LRM §21.2.3); `None` forces a print (fresh registration).
    monitor_arg_prev: Option<Vec<Value>>,
    /// Instance scope in force when `$monitor`/`$fmonitor` was ARMED.
    ///
    /// `check_monitor` re-formats the saved format string at the end of every
    /// time slot, long after the arming statement returned — so a `%m` in it
    /// used whatever scope happened to be executing then (usually none, which
    /// degrades to the top module). §21.2.1.7 ties `%m` to the scope of the
    /// statement that CONTAINS it, so a `$fmonitor(fd, "%m")` inside a bound
    /// instance must keep reporting that instance. Captured here at arm time
    /// and reinstalled around the re-format.
    monitor_scope: String,
    /// The LEXICAL half of that `%m` (process block label + task / function /
    /// named-block chain), captured at the same moment. `monitor_scope` alone
    /// only pins the INSTANCE: a slot-end check that fires while some
    /// unrelated task is mid-flight would otherwise append that task's name.
    monitor_m_suffix: String,
    /// Set only while `check_monitor` re-formats, so the `%m` arm uses the
    /// captured suffix instead of the live scope stack.
    monitor_m_active: bool,
    /// Active tag for a tagged union variable: signal name → tag name.
    pub active_union_tag: HashMap<String, String>,
    pub max_time: u64,
    /// Seconds per simulation tick (finest timescale precision; 1e-9 default).
    pub tick_s: f64,
    /// $timeformat state (§21.3.5): (units_exp, fraction_digits, suffix,
    /// min_field_width). `None` until `$timeformat` is called; %t then uses
    /// the design's finest precision, 0 fraction digits, no suffix.
    time_format: Option<(i32, usize, String, usize)>,
    /// Maximum iterations for combinatorial settling per cycle.
    pub settle_limit: u32,
    /// §5 diagnosability: how many settles exhausted the cap, and when last.
    settle_limit_hits: u64,
    settle_limit_last_time: u64,
    /// §5.5/§10.3 (#35): when set, the running settle DEFERS cont-assign
    /// family entries (the writing PROCESS must not observe its own write's
    /// propagation until it yields); UDP/gate/always entries stay eager.
    proc_settle_defer: bool,
    /// READ signal ids of the entries skipped while deferring. The next FULL
    /// settle re-dirties them so the skipped entries re-trigger through the
    /// normal dependency machinery — entry INDICES would go stale across the
    /// topo reorder / lazy materialization of `comb_entries`.
    deferred_proc_entries: Vec<u32>,
    /// Maximum snapshot→apply_nba→settle→check_edges rounds per event-loop
    /// iter (see drain_edge_cascade). Cached at sim init; override via
    /// XEZIM_CASCADE_LIMIT env var.
    cascade_limit: u32,
    /// SDF delay annotation (None if no SDF loaded).
    pub sdf_annotation: Option<super::sdf::SdfAnnotation>,
    /// min/typ/max selection from the CLI (--sdf-min/typ/max or the
    /// +mindelays/+typdelays/+maxdelays plusargs). A runtime `$sdf_annotate`
    /// honors this; `None` means Typ (the CLI default).
    pub sdf_select: Option<super::sdf::DelaySelect>,
    /// Per-signal delay in sim ticks (0 = no delay). Indexed by signal_id.
    sdf_delays: Vec<u64>,
    /// Pending delayed signal updates: (time, signal_id, value)
    delayed_updates: Vec<(u64, usize, Value)>,
    /// §4.9.4: future-time NBA updates (`a <= #d v`). Kept SEPARATE from
    /// `delayed_updates` (gate/CA transport, an ACTIVE-region event at
    /// maturity): these commit in the target slot's NBA region, after that
    /// slot's Active reads.
    delayed_nba: Vec<(u64, usize, Value)>,
    /// Future NBA writes whose targets require captured declaration context.
    delayed_nba_queue: Vec<(u64, NbaEntry)>,
    /// NBA writes into PACKED-ARENA cells (issue #147). A packed cell has no
    /// `signal_table` slot, so it cannot ride `nba_fast` (apply indexes the
    /// per-signal tables) — and committing immediately gave NBAs into big
    /// RAMs blocking semantics: a same-timestep reader in another edge block
    /// saw the new value a delta early. (eid, value plane, xz plane); tiny —
    /// entries exist only between an edge exec and the same tick's NBA
    /// region. Last-write-wins by linear scan.
    packed_nba: Vec<(usize, u64, u64)>,
    /// §29 per-instance UDP runtime state, indexed by `CombItem::Udp{idx}`.
    udp_runtime: Vec<UdpRuntime>,
    /// True when the design contains any UDP instance — forces the serial
    /// settle path (sequential UDPs need `&mut self` to update their state,
    /// which the parallel/BSP isolated eval cannot provide).
    has_udp: bool,
    /// §31 timing checks, and the signals their terminals watch (indexed
    /// by `CombItem::TimingCheck{idx}`).
    timing_checks: Vec<timing_checks::TimingCheckRt>,
    timing_watches: Vec<timing_checks::TcWatch>,
    /// Like `has_udp`: timing checks keep per-check state, so their comb
    /// entries force the serial settle.
    has_timing_checks: bool,
    /// §30.4 module paths (see `module_paths`); `None` without any, so a
    /// design with no specify paths carries one null pointer.
    module_paths: Option<Box<module_paths::ModulePaths>>,
    /// Timing-check events seen by a settle inside a running process,
    /// evaluated once it suspends: (check, event role).
    timing_pending: Vec<(u32, u8)>,
    /// Armed time-based `$timeskew`/`$fullskew` deadlines: (time, check).
    timing_timers: Vec<(u64, usize)>,
    module: ElaboratedModule,
    dpi_libraries: Vec<Library>,
    dpi_bindings: HashMap<String, DpiBinding>,
    dpi_unsupported: HashSet<String>,
    dpi_unresolved: HashSet<String>,
    /// §8.10 non-static-method-through-`::` diagnostics already emitted
    /// (`(class, method)` pairs) — a given pair is reported once.
    nonstatic_scope_reported: HashSet<(String, String)>,
    /// Imports whose library binding has been attempted.
    dpi_bind_tried: HashSet<String>,
    /// State of the built-in UVM DPI-C helpers (`uvm_dpi.rs`).
    uvm_dpi: uvm_dpi::UvmDpiState,
    /// Class instance heap (index 0 is null).
    heap: Vec<Option<ClassInstance>>,
    /// LRM §25.8 virtual-interface bindings.
    /// Key: `(class_instance_handle, property_name)`.
    /// Value: `(bound_iface_inst_name, modport_opt)`. The modport is
    /// carried from the class property's declared type (e.g. `virtual
    /// bus_if.master vif`) so the rewrite path can also emit a direction
    /// warning when writing to a modport-input member.
    virtual_iface_bindings: HashMap<(usize, String), (String, Option<String>)>,
    /// §25.9/§8.9: the interface instance a STATIC class property holding a
    /// virtual interface is bound to, keyed by the property's static storage
    /// key (`S::spv`, `G#virtual pif::one`) — one binding per class (per
    /// specialization), shared by every object and static method.
    static_vif_bindings: HashMap<String, String>,
    /// The property names `static_vif_bindings` holds a binding of (the
    /// cheap gate `vif_rhs_possible` checks).
    static_vif_names: HashSet<String>,
    /// Every static property name some class declares as a virtual
    /// interface or through a type parameter (built on first use): the gate
    /// before `bind_static_vif` looks at an assignment.
    vif_capable_statics: std::cell::OnceCell<HashSet<String>>,
    /// False once start-up found no such static (`vif_capable_statics`
    /// empty): the one-byte test every blocking assignment makes.
    may_bind_static_vif: bool,
    /// The names of static fixed arrays of virtual interfaces some class
    /// declares (§25.10, §8.9): the gate before `static_vif_array_elem`.
    static_vif_array_names: HashSet<String>,
    /// §13.4/§8.5: the CALL in receiver position of a member/select chain
    /// being evaluated (`me()` in `me().o.x`, `q.pop_front()` in
    /// `q.pop_front().addr`), with its one value: `(node address, call-frame
    /// depth, value)`. The chain's paths re-evaluate their receiver freely;
    /// while an entry is live, the same node at the same depth returns the
    /// value instead of running the call again. See `recv_call_once_eval`.
    recv_call_memo: Vec<(usize, usize, Value)>,
    /// A virtual-interface handle token has been stored in a collection
    /// element (see `queue_eval_arg`): receivers may then resolve BY VALUE.
    vif_tokens_stored: bool,
    /// Memo for transitive blocking-task detection (pure-LRM mode only): maps a
    /// subroutine name to whether its body — following calls — eventually hits a
    /// blocking construct (`#`/`@`/`wait`/`fork…join[_any]`). Lets a task that
    /// blocks only via nested calls route through the suspend-aware runner
    /// instead of the name whitelist. Keyed by bare name (over-approx across a
    /// free task / same-named method). See `subroutine_name_blocks`.
    tb_cache: task_suspend::CycleMemo,
    /// The wait-free analysis that gates compiling a class task (see
    /// `task_suspend`).
    ts: task_suspend::TsState,
    /// LRM §25.9: stack of per-call virtual-interface formal-arg
    /// aliases. When a task or function takes `virtual <iface> <name>`,
    /// the call hooks add a frame mapping `<name>` to the caller's
    /// interface-instance identifier (e.g. `bus`). `resolve_hier_name`
    /// consults the top frame so `vif.member` resolves to `bus.member`.
    /// Popped at task return.
    local_iface_aliases: Vec<HashMap<String, String>>,
    /// LRM §25.9: name aliases for PLAIN variables declared `virtual
    /// <iface>` (block-local or module-scope, not class properties and
    /// not subroutine formals). `vif = bus;` records `vif -> bus` here;
    /// the same resolve sites that consult the top
    /// `local_iface_aliases` frame fall back to this map, so
    /// `vif.member` reads/writes and `vif.f()` calls dispatch to the
    /// bound instance. `vif = null` removes the entry. Keyed by bare
    /// variable name (consistent with the simulator's global-by-bare-
    /// name treatment of subroutine locals).
    viface_var_aliases: HashMap<String, String>,
    /// §25.9: the interface instance behind the vif a function RETURN just
    /// produced (`return req_vif;`) — consumed by a `vd = call();` assignment
    /// so accessor-style getters keep the binding. Cleared before each such
    /// call.
    last_vif_return: Option<String>,
    /// While a `with (item...)` filter is being evaluated over a queue whose
    /// elements are unpacked STRUCTS, `item` names no value — the element's
    /// members live in separate signals. Bind it to the element's flat name
    /// (`q[3]`) so `item.field` reads `q[3].field`. Checked at the eval sites,
    /// before `resolve_hier_name`, whose per-node cache would freeze the first
    /// element's binding for the whole sort.
    item_alias: Option<String>,
    /// Custom iterator name bound inside a locator/`with` filter (e.g. `i` in
    /// `q.find(i) with (i.field)`). `item_alias` resolves `item.field`; with a
    /// custom name the struct-member lambda reads `i.field`, which must also
    /// dereference through the aliased element path. Defaults to `item` when
    /// the user wrote no explicit iterator.
    locator_iter: String,
    /// LRM §18.5.4 dist-pick-once tracking (per-randomize scope). Records
    /// `(handle, prop_name)` pairs for which a weighted `dist` constraint
    /// has already drawn one sample in the current randomize call. The
    /// outer 16-pass fixpoint loop would otherwise re-pick on every
    /// iteration, biasing the empirical distribution toward higher-
    /// weighted values (9:1 weights produced ~98:2 instead of 90:10).
    /// Cleared by `exec_randomize` at entry.
    dist_picked_once: HashSet<(usize, String)>,
    /// IEEE 1800-2017 §18.3/§18.5.12 — the `inside`-range domain of each rand
    /// property of the object currently being solved (exact, signed). The
    /// fixpoint consults it so an affine solve or a `!=` re-pick lands the
    /// target inside its DECLARED range rather than merely satisfying the one
    /// item in hand (an `unique {a, arr[0]}` re-pick used to blow `a inside
    /// {[1:10]}` wide open). Rebuilt at the top of every solver trial.
    rand_ranges: HashMap<String, Vec<(i128, i128)>>,
    /// The same for the ELEMENTS of each rand array / collection property:
    /// the hard `inside` domain its elements are constrained to (signed,
    /// exact). Refreshed at the top of every repair pass, so a conditional
    /// domain follows the current values of its condition.
    rand_elem_ranges: HashMap<String, Vec<(i128, i128)>>,
    /// Static class properties — one shared cell per `Class::prop`,
    /// keyed `"ClassName::propname"` where ClassName is the class that
    /// declared the static property.
    class_statics: StaticMap,
    /// §8.25/§6.20.2 (issue #423): lazily materialized per-specialization
    /// elements of class-scope ARRAY constants (header value parameters
    /// with unpacked dims, class localparam arrays), keyed
    /// `DeclClass#canon_sig::name` — or `DeclClass::name`-shaped default
    /// bindings when no specialization is active. See
    /// `materialize_class_param_array`.
    class_param_arrays: HashMap<String, ClassParamArray>,
    /// Per-frame overlay for class/typedef types of frame-locals. The
    /// global var_class_types/var_typedef_types are keyed by BARE name and
    /// shared across every process, so a class-typed local declared in one
    /// subroutine (or another process) silently overwrote the entry a
    /// different frame relied on — `$cast`/`new`-resolution then read a
    /// stale class and a valid cast reported a definite mismatch. Each
    /// local frame carries its own (class, typedef) maps, consulted
    /// innermost-first before the globals; the stack mirrors `local_stack`
    /// (push/pop helpers + ProcessContext swaps), so process interleaving
    /// keeps each process's bindings intact.
    local_type_stack: Vec<(HashMap<String, String>, HashMap<String, String>)>,
    /// Active parameterized-class specialization for per-spec static storage:
    /// `(base_class, sig)` where `sig` is the canonical `#(...)` param text. Set
    /// while a `C#(params)::...` static method runs or a `C#(params)::prop` is
    /// accessed; `static_prop_key` folds `sig` into the key ONLY for statics
    /// declared in `base_class` (so unrelated/inherited statics stay shared),
    /// giving each specialization its own cell (§8.25). None = shared cell.
    current_spec: Option<(String, String)>,
    /// Active specialization scope CHAIN (innermost last): a parameterized
    /// `C#(params)::method()` call (`eval_call`) sets `current_spec` to the
    /// callee for the body, so a value-parameter reference in an ACTUAL
    /// argument that belongs to an ENCLOSING specialization (e.g. `string
    /// FIELD` of `uvm_utils` passed into `uvm_config_db#(uvm_object)::get`)
    /// can no longer resolve through `current_spec` alone. This stack keeps
    /// the CALLER specializations alive so value-param resolution can walk up
    /// to them (§13.5.1: args evaluate in the caller's scope).
    spec_scope_stack: Vec<(String, String)>,
    /// §28.11 gate FALL delays by signal id, from
    /// `ElaboratedModule::gate_fall_delays`. Empty unless some gate used the
    /// two-delay `#(rise, fall)` form with differing values.
    gate_fall_delay_by_id: HashMap<usize, u64>,
    /// `assign #(rise, fall, turnoff)` (§10.3.3): per-net turn-off delay,
    /// used for a transition to z. Empty unless some assign used the
    /// three-delay form.
    gate_off_delay_by_id: HashMap<usize, u64>,
    /// Declaring class → property → does this property's packed range
    /// depend on a parameter? `fit_class_prop` runs on every property store,
    /// so the common answer (`false`) is memoized to one hash lookup.
    spec_prop_is_dyn: std::cell::RefCell<HashMap<String, HashMap<String, bool>>>,
    /// Memo for `respec_packed_width`: `(declaring_class, property,
    /// parameter-binding fingerprint)` → re-resolved width, so the packed
    /// range is const-eval'd once per specialization instead of per store.
    spec_prop_width_cache: std::cell::RefCell<HashMap<(String, String, String), Option<u32>>>,
    /// `static_cell_width` per static cell key (class tables are fixed at
    /// run time, and a key names one specialization).
    static_cell_width_cache: std::cell::RefCell<HashMap<String, Option<(u32, bool)>>>,
    /// Tracks class names currently being constructed via the `type_id::create()`
    /// shortcut, to prevent infinite recursion when a parameterized
    /// class's constructor or static initializers re-enter `type_id::create()`
    /// for the same class.
    type_id_create_in_progress: HashSet<String>,
    /// Tracks which `(base, sig)` specializations have already had their
    /// static-call initializers run. Per §8.25/§6.21, each parameterized-class
    /// specialization has its own set of static properties, and side-effecting
    /// static initializers (e.g. `local static bit m__initialized =
    /// __deferred_init()`) must run once per specialization — not just once
    /// for the generic class with default params.
    initialized_spec_statics: std::collections::HashSet<(String, String)>,
    /// Concrete module/package typedef references, excluding aliases hoisted
    /// from generic class definitions without an active specialization.
    referenced_class_typedefs: Vec<DataType>,
    /// Object handles currently mid-`register()` in a factory. Guards
    /// against RE-ENTRANT registration: a factory `register(obj)` whose
    /// first statement `obj.get_type_name()` triggers a parameterized
    /// class specialization's lazy static init, which itself calls
    /// `factory.register(obj)` for the SAME object. The re-entrant call is
    /// skipped so the OUTER call performs the single registration — without
    /// it, the outer call resumes and finds the type already stored, emitting
    /// a spurious UVM `TPRGED` warning.
    factory_reg_in_progress: std::collections::HashSet<usize>,
    /// (handle, class) whose constructor body is currently executing —
    /// unqualified calls on `this` bind within that class's chain (§8.7).
    ctor_class_stack: Vec<(usize, String)>,
    /// (var.prop) null-deref reads already reported (in-method soft errors).
    reported_null_derefs: std::collections::HashSet<String>,
    /// Queue of specializations pending static-init, processed iteratively
    /// by `ensure_spec_statics` to avoid deep Rust stack recursion.
    pending_spec_inits: Vec<(String, String)>,
    /// Recursion guard for `ensure_spec_statics`.
    spec_init_depth: u32,
    /// Recursion guard for evaluating call-bearing instance-property
    /// initializers at construction (see `evaluate_call_init_at_construct`).
    construct_init_depth: u32,
    /// Classes awaiting their §8.7 initialization for each constructing object.
    pending_class_initializers:
        HashMap<usize, Vec<std::sync::Arc<crate::compiler::elaborate::ElaboratedClass>>>,
    /// Class-typed procedural locals — variable name -> class type name.
    /// Lets a later `name = new();` know which class to construct.
    var_class_types: HashMap<String, String>,
    /// Parameterized-class procedural locals — variable name -> the declared
    /// `#(...)` type-parameter args (as expressions). Lets a SEPARATE-statement
    /// `name = new(...)` (which can't see the decl's data_type) construct the
    /// right specialization, so the instance records its type bindings. This is
    /// right specialization, so the instance records its type bindings (IEEE 1800-2023 §8.25).
    /// Needed for per-spec static resolution (get_type vs get_type_handle → GET).
    var_type_args: HashMap<String, Vec<Expression>>,
    /// §15.3/§15.4: built-in container local variable → "mailbox"/"semaphore".
    /// Populated at VarDecl exec time when the declared base type is a
    /// (possibly parameterized) mailbox/semaphore, so a later separate
    /// `name = new(bound)` allocates the right container. get_expr_type_name
    /// can't serve this: for a class property it returns the enclosing class
    /// name, and for an untracked local it returns None.
    var_container_types: HashMap<String, String>,
    /// LRM §6.19.6: typedef-typed local variable → typedef name. So
    /// `<local>.next()/.first()/.last()/.num()/.prev()` resolves the
    /// enum-member list via `module.enum_members[typedef]`. Mirrors
    /// `var_class_types`; populated at VarDecl exec time when the
    /// data type is a `TypeReference` whose name is in `module.typedefs`.
    var_typedef_types: HashMap<String, String>,
    /// Names of `string`-typed variables (locals, params, package globals) so
    /// `{a, b}` concatenation involving them is treated as string concat.
    string_signals: HashSet<String>,
    /// Per-call-frame save area for queue/dynamic-array LOCALS. Queue storage
    /// lives globally in `self.signals` (`name.size`, `name[i]`) keyed by bare
    /// name, so a nested method whose local/param queue shares a name with a
    /// caller's local would clobber it. On entry to a method/function/task we
    /// push a frame; whenever a queue param is bound or a queue local is
    /// (re)declared, we snapshot the caller's `name.*`/`name[*]` signals into
    /// the top frame; on exit we restore them.
    queue_frame_saves: Vec<Vec<(String, QueueLocalSave)>>,
    /// Per-call-frame rename map for local dynamic arrays/queues/assoc
    /// locals (bare name -> process-unique storage key). See
    /// `ProcessContext::local_dyn`.
    local_dyn: Vec<Vec<(String, String)>>,
    /// Parallel to `local_dyn`: each call frame's queue / dynamic-array /
    /// associative FORMALS, bare name -> storage key (a per-call copy
    /// `@q#N`, or for `ref` the actual's own storage). The frame's
    /// `local_dyn` table holds an identity entry `(name, name)` for each, so
    /// it shadows an enclosing same-named local and reads as its bare name
    /// from a nested call (§13.3); `dyn_name_lookup` maps the identity hit
    /// in the innermost frame through this table.
    formal_dyn: Vec<Vec<(String, String)>>,
    /// Monotonic counter backing the process-unique keys in `local_dyn`.
    next_dyn_id: u64,
    /// Built-in mailboxes (handle -> queue of values)
    mailboxes: HashMap<usize, std::collections::VecDeque<Value>>,
    /// §15.4 aggregate messages. A queue slot holds one `Value`, which cannot
    /// carry an UNPACKED struct — its members are separate leaf signals. Such
    /// a message is copied member-wise into a shadow variable and the slot
    /// holds the shadow's id instead. Ids are recycled as messages are
    /// consumed, so the shadows are bounded by the deepest the box ever got
    /// rather than growing with every message put.
    mbx_msg_seq: u64,
    mbx_free_ids: Vec<u64>,
    /// §15.4.1 bounded-mailbox capacity (handle -> bound). Absent or 0 means
    /// UNBOUNDED — `try_put` never fails and `put` never blocks. `new(N)` with
    /// N>0 records N here; a full box rejects `try_put` and parks `put`.
    mailbox_bound: HashMap<usize, usize>,
    /// Producers parked on `put` to a full bounded mailbox (handle -> queue).
    mailbox_put_waiters: HashMap<usize, std::collections::VecDeque<MailboxPutWaiter>>,
    /// §18.13 rand_mode: per-instance set of rand properties whose
    /// randomization is currently disabled. `obj.x.rand_mode(0)` adds "x".
    /// `obj.rand_mode(0)` disables ALL rand props.
    rand_mode_disabled: HashMap<usize, HashSet<String>>,
    /// §18.13 constraint_mode: per-instance set of constraint names that
    /// are currently disabled. `obj.cb.constraint_mode(0)` adds "cb".
    constraint_mode_disabled: HashMap<usize, HashSet<String>>,
    /// §18.5.9/§18.6 recursion guard: depth of nested `randomize()` calls made
    /// for `rand` class-handle members (a cyclic object graph would otherwise
    /// recurse forever).
    randomize_depth: usize,
    /// Failed randomize() calls reported so far, per class or
    /// `std::randomize` call site (`rand_fail_warn`).
    rand_fail_reports: HashMap<String, u32>,
    /// Whether the `XEZIM_RAND_DIAG` hint has been printed.
    rand_fail_hinted: bool,
    /// One work/time budget shared by an outer randomize call and every
    /// recursive rand-object solve it starts.
    randomize_budget_depth: usize,
    randomize_budget_remaining: u64,
    randomize_budget_deadline: Option<std::time::Instant>,
    randomize_budget_exhausted: bool,
    /// Active enclosing solves retain only their reachable children's rand state.
    rand_child_snapshots: Vec<rand_scope::RandChildTransaction>,
    /// IEEE 1800-2017 §18.5.11 — a `static constraint` block is SHARED by every
    /// instance of the class: `constraint_mode()` on it through ANY instance
    /// (or through the class scope) sets one class-wide flag. Keyed by
    /// (declaring class, constraint name); membership == disabled.
    static_constraint_disabled: HashSet<(String, String)>,
    /// §18.4.3 randc cycles: per-(instance, prop) set of values already
    /// drawn in the current cycle. Each randomize call must pick a value
    /// not in this set; once every value in the prop's range has been
    /// drawn the set resets and a new cycle begins.
    randc_used: HashMap<(usize, String), HashSet<u64>>,
    /// §18.4 randc: values drawn TENTATIVELY by the randomize() call that is
    /// currently in flight, keyed like `randc_used`. The solver retries a
    /// failed solution up to 1000 times; without this each retry would burn
    /// another value out of the permutation cycle, so a 2-bit randc could
    /// visit only two of its four values across four randomize() calls.
    /// A pick is rolled back (removed from `randc_used`) when its trial is
    /// retried or abandoned, so exactly ONE value leaves the cycle per
    /// SUCCESSFUL randomize(). `Some(prev)` records the pre-reset used-set of
    /// a pick that wrapped the cycle, so the rollback can restore it.
    randc_pending: HashMap<(usize, String), (u64, Option<HashSet<u64>>)>,
    /// §18.5.4/§18.10 dist schedules, keyed by (instance handle, variable):
    /// one (weight signature, item schedule, next slot) per weight set. See
    /// `pick_dist_value`.
    dist_decks: HashMap<(usize, String), Vec<(u64, Vec<usize>, usize)>>,
    /// Built-in semaphores (handle -> current count)
    semaphores: HashMap<usize, i64>,
    /// Covergroup instance heap (index 0 is null).
    cg_heap: Vec<Option<CovergroupInstance>>,
    /// Running tallies for every distinct `assert`/`assume`/`cover` source
    /// location. Keyed by the statement's `span.start` byte offset so a
    /// single source line aggregates across all dynamic executions.
    /// Emitted at end-of-sim under `[COV] assertion summary` + (when
    /// `XEZIM_COV_DB` is set) the JSON coverage database file.
    assertion_stats: HashMap<usize, AssertionStat>,
    /// Code coverage sites and toggle state (`--code-coverage`); `None` when
    /// the run collects none. See `code_cov`.
    code_cov: Option<Box<code_cov::CodeCov>>,
    /// Code coverage counters, indexed by the id a `$__xz_cov` marker or an
    /// `Insn::CovHit` carries. Empty when the run collects none.
    code_cov_hits: Vec<u64>,
    /// Toggle coverage candidates, named while `new` still has the
    /// elaboration signal table; resolved to ids in `compile`.
    code_cov_toggle_names: Vec<String>,
    /// Call stack for tracking 'this' and local variables.
    this_stack: Vec<Option<usize>>,
    local_stack: Vec<HashMap<String, Value>>,
    /// Open block scopes; see `ProcessContext::block_scope_saves`.
    block_scope_saves: Vec<BlockScopeSave>,
    /// `(body address, frame generation)` of the synchronous loop body whose
    /// block scope `loop_scope_open` holds open across iterations.
    hoisted_scope: (usize, u64),
    /// Pids whose continuation `continue_stmts_or_trampoline` rescheduled
    /// with their context left live on the simulator.
    trampolined_pids: HashSet<usize>,
    /// Parallel to `local_stack`: the frame generation each slot was pushed
    /// with (see `ProcessContext::local_gen_stack`). Kept on the active
    /// `Simulator` exactly like `local_stack`, and moved into/out of a
    /// `ProcessContext` by snapshot/take/restore.
    local_gen_stack: Vec<u64>,
    /// Context for 'super' resolution: stack of (current_class_name).
    class_context_stack: Vec<Option<String>>,
    /// For each class-method call entered via `exec_method_in_class_hierarchy`,
    /// the `local_stack` depth at entry (before the method's own frame is
    /// pushed). Used by `get_expr_type_name` to decide whether a bare name is
    /// a local of the CURRENT method vs a stale flat-map entry leaked from a
    /// caller's scope — without this, `var_class_types` (never cleared on
    /// method exit) lets a caller's same-named local shadow a class property's
    /// declared type, so `prop = new(...)` constructs the wrong class (and can
    /// recurse infinitely when the wrong class is the enclosing class itself).
    method_local_base: Vec<usize>,
    /// Immutable method bodies cached after their first dispatch. Runtime
    /// execution mutates object state, never declarations, so sharing the
    /// parsed body avoids rebuilding a deep AST on every call.
    class_method_cache: std::cell::RefCell<
        HashMap<String, HashMap<String, Option<Arc<crate::ast::decl::ClassMethod>>>>,
    >,
    /// class-perf Step 7: memoized `class_has_method` (whole-chain,
    /// declaration-presence only — includes PureVirtual/Extern prototypes,
    /// unlike `cached_class_method`). Same freeze argument as the
    /// defining-class memo (`method_defining_class`).
    class_has_method_memo: std::cell::RefCell<HashMap<String, HashMap<String, bool>>>,
    /// class-perf Step 7: memoized `is_static_method` (whole-chain — an
    /// ancestor's static declaration wins, exactly like the walk it
    /// replaces).
    is_static_method_memo: std::cell::RefCell<HashMap<String, HashMap<String, bool>>>,
    /// Current covergroup instance if in sampling context.
    cg_this: Option<usize>,
    /// Processes waiting for join
    join_waiters: Vec<JoinWaiter>,
    /// Map from child PID -> parent PID
    process_parents: HashMap<usize, usize>,
    /// Per-process execution context for scheduled class/task processes.
    process_contexts: HashMap<usize, ProcessContext>,
    /// Instance scope (e.g. `"TB.p1"`) for a scheduled process, set from the
    /// originating initial block. `run_scheduled_process` installs it as the
    /// name-resolution hint so AST-evaluated bare names in a multiply-
    /// instantiated module resolve to THIS instance's signals.
    process_scope_hint: HashMap<usize, String>,
    /// §21.2.1.7 `%m`: each process's own lexical scope chain, parked here
    /// while it is suspended. `m_scope_stack` is a single global stack, so a
    /// task that suspends at a `#delay` used to leave its own name installed
    /// for whatever process ran next — every later `%m`, including a
    /// `$monitor` arming in an unrelated instance, inherited that name.
    process_m_scope: HashMap<usize, Vec<String>>,
    /// §21.2.1.7 `%m`: label of a named `initial begin : lbl` / `always
    /// begin : lbl`, per pid. The scheduler FLATTENS a process's outermost
    /// `begin`/`end` into its statement stream (see `run_process_stmts`), so
    /// the `SeqBlock` arm that would push the label onto `m_scope_stack`
    /// never runs for it and `%m` dropped the block name — `top.u_a` where
    /// it should read `top.u_a.lbl`. Nested (non-flattened) blocks still go
    /// through `m_scope_stack` as usual.
    process_m_label: HashMap<usize, String>,
    /// Where each process came from: the creating construct's source span
    /// and its kind ("initial block", "always block", "fork child", …).
    /// One insert per process CREATION (never on the activation hot path);
    /// read only by `report_zero_delay_stall` so a spinning pid can be
    /// traced back to the RTL construct that spawned it.
    process_origin: HashMap<usize, (crate::ast::Span, &'static str)>,
    /// XEZIM_TRACE_SCHED=1|all|<t>|<t0>:<t1> — an ORDERED log of what runs
    /// inside each time slot: the tick header, every clock-generator toggle,
    /// and every process as it is popped from the timing wheel. A race is a
    /// question about intra-timestamp ORDER, which the value/edge traces
    /// cannot answer — they say what changed, never who went first. Windowed
    /// by time because a whole run is unreadable.
    trace_sched: Option<(u64, u64)>,
    /// Instance scope of the edge block currently executing (set around
    /// dispatch, cleared after). `current_module_def` prefers it over the
    /// per-pid hint so $time/%t scale to the BLOCK's module timescale, not
    /// the last scheduled process's.
    timescale_scope_override: Option<String>,
    /// Collection name a `return <collection>` recorded (§13.4: functions may
    /// return arrays/queues). The caller's assign-from-call consumes it with
    /// a whole-collection copy; a plain value return leaves it None.
    pending_ret_collection: Option<String>,
    /// The collection the last compiled call instruction got back through
    /// `pending_ret_collection` (None for a plain value return). Only the
    /// `AssignPendingColl` that may follow the call reads it.
    vm_ret_coll: Option<String>,
    /// Flat element values a compiled collection-returning call produced
    /// (`Insn::RetCollection`), stashed straight out of the callee's register
    /// file. `Insn::AssignPendingColl` prefers these over the string-keyed
    /// `vm_ret_coll`/`__xz_ret_coll__` snapshot+copy round trip, which
    /// allocates a formatted key, a HashMap insert, a class-legacy scan and
    /// per-element lookups per returned element. `None` = the AST/string path
    /// produced the return and the historic signal-snapshot copy applies.
    vm_ret_vals: Option<Vec<Value>>,
    /// Innermost-first stack of whether each currently-active function returns
    /// a collECTION (queue / unsized dynamic array). The `Return` statement
    /// reads the top to decide whether a LITERAL return value (`return '{...}`,
    /// which is not an Ident, so the bare-name snapshot path can't fire) must be
    /// materialised into a whole-collection snapshot and handed to the caller
    /// through `pending_ret_collection`. Without it a function whose `return`
    /// expression is a queue literal collapsed to a single-element scalar
    /// value when the caller assigned it to a queue variable — an EMPTY `'{}`
    /// returned size 1 (one phantom element), and `'{a,b}` kept only `a`.
    fn_ret_collection_stack: Vec<bool>,
    /// Parallel to `fn_ret_collection_stack`: the running function returns
    /// an associative array (§13.4).
    fn_ret_assoc_stack: Vec<bool>,
    /// Stable instance scope of the process currently running (for `%m`).
    /// Unlike `name_resolve_hint`, this is not mutated by name resolution.
    current_scope: String,
    /// Recycled scope-hint strings for the process wake-up path: every
    /// wake-up installed a fresh clone of the process scope into
    /// `name_resolve_hint` and `current_scope` (four allocations for a
    /// compiled-FSM wake-up) and freed them on exit.
    hint_string_pool: Vec<String>,
    /// Gated-clock conversion (cycle design phase 0). A derived clock is a
    /// fused-gate output (ICG `clk & en`, buffer, inverter) whose input
    /// clock chains back to a clock GENERATOR. Those gate entries form the
    /// clock tree: at a generator toggle they are evaluated EAGERLY in
    /// topological order and any derived clock that toggled joins the same
    /// edge pass, so the tree never goes through the settle worklist
    /// (c906: 666 always-active entries, 38% of all comb evaluations, were
    /// the clock tree). `trigger_deps` skips a tree entry when the written
    /// signal is a tree clock; an ENABLE change still reaches it through
    /// the enable's own dependents.
    /// `XEZIM_CYCLE_MODE`: `event` (the event-driven engine, default) or
    /// `cycle`. Everything under the cycle design (the eager clock tree,
    /// later the cycle scheduler and its delta-cycle fallback) is gated on
    /// this so `event` is exactly the engine as it was.
    cycle_mode: bool,
    clock_tree_by_root: HashMap<usize, Vec<usize>>,
    is_clock_tree_entry: Vec<bool>,
    /// Gate lane (see `build_gate_lane`): per comb entry, the index of its
    /// compact gate op when the entry is a fused single-bit gate whose
    /// output no comb entry reads (c906: 47% of all settle evaluations),
    /// else `u32::MAX`. Such gates are queued when triggered and evaluated
    /// in one tight loop at the end of the pass instead of through the
    /// generic entry path.
    gate_lane: Vec<u32>,
    gate_ops: Vec<FusedGate>,
    /// Per lane op: bit0 = destination is an armed edge-block input, bit1 =
    /// destination is edge-sensitive. Decides which write notes the fast
    /// commit must make.
    gate_obs: Vec<u8>,
    gate_queue: Vec<u32>,
    gate_queued: Vec<bool>,
    gate_lane_valid: bool,
    /// Paths of instances whose definition is an interface (see
    /// `is_interface_instance`); filled on first use.
    iface_instance_paths: std::cell::OnceCell<std::collections::HashSet<String>>,
    /// Interface instance path by its virtual-interface handle token (see
    /// `iface_handle_token`); filled on first use.
    iface_by_token: std::cell::OnceCell<HashMap<u64, String>>,
    /// See `iface_path_parts`.
    #[allow(clippy::type_complexity)]
    iface_path_parts: std::cell::OnceCell<(HashSet<String>, HashSet<String>)>,
    /// Quiet signals: neither an armed edge-block input nor edge-sensitive,
    /// so a write to one has no side-table work at all. One bit per signal;
    /// rebuilt when either table changes. A quiet store skips the write
    /// observer with one load (c906: most two-state stores are internal
    /// nets).
    quiet_bits: Vec<u64>,
    quiet_valid: bool,
    prof_quiet_stores: u64,
    prof_observed_stores: u64,
    /// Native two-state streams (XEZIM_TS_JIT=1, jit feature): per comb
    /// entry, the compiled straight-line function, consulted before the
    /// arena executor.
    #[cfg(feature = "jit")]
    ts_jit: Option<Box<super::ts_jit::enabled::TsJitModule>>,
    #[cfg(feature = "jit")]
    ts_jit_fns: Vec<Option<super::ts_jit::enabled::TsJitFn>>,
    #[cfg(feature = "jit")]
    ts_jit_on: bool,
    /// Per comb entry: two-state evaluations so far (native path only);
    /// a stream is compiled once it crosses `ts_jit_hot`, so only the hot
    /// set turns into machine code and the code stays cache-resident.
    #[cfg(feature = "jit")]
    ts_jit_evals: Vec<u32>,
    #[cfg(feature = "jit")]
    ts_jit_hot: u32,
    is_clock_tree_signal: Vec<bool>,
    prof_clock_tree_evals: u64,
    /// Instance scope of the comb/edge BLOCK currently being evaluated, for
    /// `%m` only — deliberately separate from `current_scope`, which also
    /// steers name resolution (`resolve_hier_name`) and must not start
    /// tracking the settle loop.
    ///
    /// §21.2.1.7: `%m` is the hierarchical name of the scope holding the
    /// format string. `current_scope` is installed by `run_process`, which
    /// only ever runs `initial`/`fork`/delay-`always` processes — a
    /// sensitivity-driven block (`always_comb`, `always @(edge)`) is a
    /// `CombEntry` or a compiled edge block and never passes through there.
    /// So `%m` in the body of any inlined `always_comb` reported the bare TOP
    /// module name, identically for every instance, which makes per-instance
    /// log correlation in a multiply-instantiated design impossible.
    ///
    /// Updated in place (`clear` + `push_str`) rather than assigned, so the
    /// settle hot loop reuses one allocation and pays only a short memcmp.
    m_block_scope: String,
    /// A comb/edge block (not a process) is executing, so `%m` names
    /// `m_block_scope` — empty for a block of the root — and no process
    /// block label. Cleared by `run_process`.
    m_block_active: bool,
    /// Interned scope id per edge block, parallel to `edge_blocks`. Two blocks
    /// in the same instance share an id, so the `%m` bookkeeping in
    /// `exec_bytecode` is a `u32` compare against a contiguous array instead
    /// of a `String` compare that also had to deref the `edge_blocks` `Arc`
    /// and reach into a 3607-entry struct vector. That comparison runs on
    /// every one of ~189 M block fires and measured 3.5% of retired
    /// instructions on the c906 memcpy workload.
    edge_block_scope_id: Vec<u32>,
    /// Which interned scope `m_block_scope` currently holds, or `u32::MAX`
    /// when its contents did not come from `edge_block_scope_id` (any
    /// non-edge-block writer sets it back to `MAX`, so the next block fire
    /// re-syncs rather than trusting a stale id).
    m_block_scope_id: u32,
    /// Stack of the names of the functions/tasks currently executing (innermost
    /// last), so `%m` inside a package subroutine resolves to `<pkg>.<name>`
    /// (matching real simulators) rather than the top-module name. Pushed in
    /// `exec_function_call` / `exec_task_call` around the body and popped after.
    func_call_stack: Vec<String>,
    /// §26.3: the package whose subroutine body is running, innermost last
    /// (`None` for a module-level subroutine). A package subroutine resolves
    /// its free names in ITS OWN package, but package-scope declarations are
    /// hoisted into the shared tables under their bare name — last writer wins
    /// — so a name two packages both declare has to be read back through
    /// `<pkg>::<name>`. See `pkg_ambiguous_names` and `resolve_hier_name`.
    pkg_scope_stack: Vec<Option<String>>,
    /// Package scope for the NEXT `exec_function_call` / `exec_task_call`, set
    /// by a scoped `pkg::f(...)` dispatch (which is the only site that knows
    /// which package was named). Taken on entry, before argument evaluation, so
    /// a call nested in an argument cannot inherit it.
    pending_pkg_scope: Option<String>,
    /// §26.3: package-scope names that MORE THAN ONE package declares — the
    /// only ones whose bare hoisted entry is ambiguous. Empty for a design
    /// without such a collision, which short-circuits the qualified lookup.
    pkg_ambiguous_names: HashSet<String>,
    /// §26.3: the package VARIABLE names among `pkg_ambiguous_names` (see
    /// `split_contested_package_vars`): only these resolve through a design
    /// unit's imports or a package class's package.
    contested_pkg_vars: HashSet<String>,
    /// §21.2.1.7 `%m` lexical-scope hierarchy WITHIN the current instance:
    /// task / function / named-block / fork-block names, innermost last.
    /// Pushed on subroutine/named-block entry, popped on exit.
    m_scope_stack: Vec<String>,
    /// IEEE 1800-2017 §6.21 / §13.3.1: `static` variables declared inside a
    /// (possibly automatic) subroutine keep ONE persistent storage across all
    /// runtime calls, and their declaration initializer runs only once. Keyed
    /// `"<subroutine>::<var>"`. Only scalar integral/real statics are tracked;
    /// arrays/queues fall back to per-call storage.
    static_local_vars: HashMap<String, Value>,
    /// Bare names of `static` subroutine locals that are scalar (tracked in
    /// `static_local_vars`). Used as a CHEAP gate before the per-subroutine
    /// `static_local_keys` check, so a bare identifier that is never a static
    /// anywhere skips the key-building cost. (`static_local_vars` cells are
    /// still read/written route to each cell's shared value only when the
    /// SUBROUTINE-PRECISE key below also matches.)
    static_local_names: HashSet<String>,
    /// Module/package subroutine statics allocated before procedural code.
    hoisted_sub_static_names: HashSet<String>,
    hoisted_static_subroutines: HashSet<String>,
    /// First-byte membership filter over `static_local_names`. The eval
    /// preamble tests EVERY bare identifier against that set, so a designs
    /// with any static subroutine local paid a string hash per expression
    /// node; this rejects the overwhelming majority with one array load.
    static_local_first: [bool; 256],
    /// Persistent-cell keys ("`<cls>::<sub>::<nm>`"/"`<sub>::<nm>`") of all
    /// scalar `static` subroutine locals. A name into `static_local_names`
    /// only routes to the live shared cell when this key — which embeds the
    /// declaring subroutine/class/specialization — matches, so a DIFFERENT
    /// subroutine's same-named (plain/automatic) local is never wrongly
    /// shared.
    static_local_keys: HashSet<String>,
    /// True while evaluating elaboration-time constant (local)parameter
    /// initializers that call user functions. Each such constant-function call
    /// is independent (§13.4.3), so `static` locals must NOT share the runtime
    /// persistent store during this window.
    in_const_param_eval: bool,
    /// Per-subroutine-call stack of `(local_name, persistent_key)` recording
    /// which locals in the current call are `static` so their final values are
    /// written back to `static_local_vars` on return. The `String` head is the
    /// subroutine name (used to build the persistent key).
    static_local_syncs: Vec<(String, Vec<(String, String)>)>,
    /// Return value from last function call.
    return_value: Option<Value>,
    /// "string_typed-name" flag snapshots for each active subroutine frame
    /// (free function / method / task). `string_signals` is a flat global set
    /// keyed by bare name; a nested frame's NON-STRING local (e.g. `int m`)
    /// runs `string_signals.remove("m")` (the block-local decl clears stale
    /// string flags), which would permanently clobber an ENCLOSING scope's
    /// `string m` — so a later `m = <long string>` there is no longer treated
    /// as a string and gets resized to the 1024-bit placeholder (128 chars).
    /// Snapshot at frame entry, restore at frame exit, so a
    /// frame can never leak an add OR a remove to its caller.
    string_signals_removed: Vec<Vec<String>>,
    /// §6.21/§13.5/§6.16: string classifications of task formals and scalar
    /// locals, keyed by their frame generation (`local_gen_stack`).
    /// A task can suspend, so its formals must not enter the global
    /// `string_signals` set: while it waits, or after `disable fork` kills
    /// it, another subroutine's same-named packed formal would read as a
    /// string and lose its bit writes. Per-process (moved with
    /// `ProcessContext`), and the frame's non-string formals override a
    /// global string flag of the same name; see `frame_name_is_string`.
    task_string_formals: Vec<TaskFrameStrings>,
    /// §13.4/§6.21: width, signedness and declared-type tables are keyed by
    /// bare name, so a callee's local (`logic [7:0] r;`) overwrote the
    /// entries of a caller's same-named variable, which then kept the
    /// callee's type after the return — a signed `int r` read back unsigned
    /// or truncated, including through an `inout`/`output` copy-back. Each
    /// function, class-method and synchronous task activation logs what its
    /// declarations displaced and puts it back on exit.
    decl_shadow_log: Vec<DeclShadowFrame>,
    /// Bumped whenever other processes may run inside a subroutine call (the
    /// nested scheduler of a synchronous `#delay`). A shadow frame whose
    /// activation saw that happen is dropped, not restored: an interleaved
    /// activation may own the entries by then.
    nested_run_epoch: u64,
    /// Union of every parked `wait(cond)` waiter's read names, rebuilt lazily
    /// (see `cond_read_union_dirty`) so a blocking write can reject in O(1)
    /// without allocating — `check_condition_waiters_for_write` sits on the
    /// interpreter's hottest path.
    cond_read_union: HashSet<String>,
    cond_read_union_dirty: bool,
    /// Static-collection bare names declared by two SIBLING classes (§8.9),
    /// computed once at startup (`colliding_names` in the static-collection
    /// registration loop). `static_coll_name_collides` used to rescan every
    /// class on every static-collection access.
    colliding_static_colls: HashSet<String>,
    colliding_static_colls_ready: bool,
    /// Default random number generator — the stream used by any object or
    /// process that has not been given a private one (§18.14).
    rng: SvRng,
    /// §18.14.1 per-PROCESS random stream. Populated lazily: a process only
    /// gets a private stream once `process::self().srandom()` /
    /// `.set_randstate()` / `.get_randstate()` is called on it. Until then it
    /// draws from `rng`, preserving the historical (entropy) behaviour.
    /// Keyed by pid, so forked threads are independent of each other and of
    /// their parent — their `$urandom` sequence no longer depends on the
    /// scheduling order of the other threads.
    proc_rng: HashMap<usize, SvRng>,
    /// §18.14.1/§18.14.2 per-OBJECT random stream, keyed by class handle.
    /// Populated lazily by `obj.srandom(seed)` / `obj.set_randstate(s)` /
    /// `obj.get_randstate()`; `randomize()` on such an object draws from it, so
    /// the object's sequence is unaffected by any thread reseeding and vice
    /// versa.
    obj_rng: HashMap<usize, SvRng>,
    /// Handles of the objects whose `randomize()` is currently executing
    /// (a stack — a constraint/post_randomize may randomize another object).
    /// `cur_rng` consults the top of this stack first.
    obj_rng_stack: Vec<usize>,
    /// §18.7 — the receiver NAME (a bare handle variable) of the in-flight
    /// `obj.randomize() with {…}` call. Inside the randomized frame that
    /// local is out of scope (eval of it returns 0), so the member-forcing
    /// solver recognises `recv.member` as the object's own `member` when
    /// `recv` is this receiver AND `member` is a rand property.
    rand_receiver: Option<String>,
    /// `method_local_base` entry of the solve frame `rand_receiver` belongs
    /// to; see `active_rand_receiver`.
    rand_receiver_base: usize,
    /// §8.10: property names declared by MORE than one class of some object's
    /// inheritance chain. Non-leaf declarers' copies are stored under
    /// `"<Class>::<name>"`; the set is the cheap gate that keeps every
    /// unshadowed access on the bare-name fast path.
    shadowed_prop_names: HashSet<String>,
    /// `meta_key_bits` of every name in `shadowed_prop_names`: a prefilter that
    /// spares the hot member-access paths a hash per access.
    shadowed_prop_mask: u64,
    /// §6.11.1 / §7.5: instance-scoped storage names (`<handle>#<member>`) of
    /// queue / dynamic-array class properties whose element type is a SIGNED
    /// integral type (`byte`, `shortint`, `logic signed [7:0]`, ...). Their
    /// elements live in the runtime map with no declared signedness of their
    /// own, so a store stamps it from here (`p.dyn[1] = 8'hff` reads -1).
    signed_class_colls: HashSet<String>,
    /// §8.25 / §8.3: classes whose base class depends on their own
    /// parameters — `class W #(type B = base) extends B;` (the base IS a type
    /// parameter) or `class W2 #(type B) extends W #(B);` (the base is such a
    /// class specialized with this class's parameters). Maps the class to its
    /// DECLARED `extends` name and argument fragments, before linking
    /// rewrote them. Each specialization whose base CLASS differs from the
    /// default's gets its own class entry (see `ensure_spec_class`).
    dep_base: HashMap<String, (String, Vec<String>)>,
    /// The default specialization's base class ENTRY of each `dep_base`
    /// class; the class entry itself carries this base. A specialization
    /// binding a base of another class gets its own entry; one binding the
    /// same base class with other arguments shares this one, the arguments
    /// flowing through the specialization (see `spec_projection`).
    dep_default: HashMap<String, String>,
    /// The `dep_base` keys as a short list: the hot paths (every `new`,
    /// `$cast`, static key and type-parameter lookup) ask whether a class is
    /// one, and a design has a handful at most.
    dep_names: Vec<String>,
    /// `spec_entry` answers for `dep_base` classes, keyed `"<base>\0<sig>"`.
    spec_entry_cache: std::cell::RefCell<HashMap<String, String>>,
    /// Specialized class entry name -> the class it was specialized from
    /// (`"wrap_c<derived_c>"` -> `"wrap_c"`).
    spec_clone_origin: HashMap<String, String>,
    /// Typedef names whose target is a specialization of a `dep_base` class
    /// (`typedef wrap_c #(derived_c) wd_t;`): `$cast` checks a destination
    /// declared through one against that specialization.
    dep_typedefs: HashSet<String>,
    /// Scratch key for the property-initializer loop of instantiation.
    inst_key_scratch: String,
    /// Instantiation templates by leaf class name (see `InstTemplate`).
    inst_templates: std::cell::RefCell<HashMap<String, std::rc::Rc<InstTemplate>>>,
    /// §18.11: when `obj.randomize(a, b)` names a MEMBER SUBSET, only those
    /// properties are solved; every other rand member keeps its current value
    /// and acts as state. Empty/None means the ordinary whole-object form.
    randomize_subset: Option<HashSet<String>>,
    settling: bool,
    in_edge_block: bool,
    /// True while waiter CONTINUATIONS resumed inline in the active region
    /// (blocks-first mode) execute. Their writes happen AFTER this pass's
    /// edge detection, so — exactly like writes made inside an edge block —
    /// they must be recorded into `edge_exec_wrote` for the §9.2 rescan
    /// drain, or the change is silently swallowed when the next snapshot
    /// refreshes `prev` (an AnyEdge comb block sensitive to a TB-driven
    /// signal then never fires again: the write-data-stops-flowing bug).
    in_edge_cont: bool,
    /// Nesting depth of `check_edges_inner`. A `#delay` inside an edge block's
    /// body re-enters the edge machinery through the synchronous
    /// `TimingControl::Delay` arm; now that a nested pass sees the real block
    /// list (see `edge_blocks`) it can genuinely fire blocks, so a block that
    /// re-triggers itself through its own delay could recurse without bound.
    /// Bounded by `EDGE_PASS_DEPTH_LIMIT`.
    edge_pass_depth: u32,
    /// §9.4.2 bit- and part-select level event terms: `(signal_id,
    /// block_idx) -> mask` of the PHYSICAL bits the block's terms on that
    /// signal watch, for `always @(v[3] or v[7:5])`. Edge sensitivity is
    /// otherwise tracked per SIGNAL, so a block watching `v[0]` also woke on
    /// a change to `v[3]`. Applies to the any-change fanout only; an entry
    /// exists only when every term of the block on that signal is such a
    /// select (see `narrow_event_mask`). Kept as a sparse side map rather
    /// than a field on `SensitivityId` because select terms are rare and the
    /// dispatch loop is the hottest path in the simulator: when this map is
    /// empty the extra work is a single already-hot bool test.
    bitsel_edge_sens: HashMap<(usize, usize), event_bits::EventMask>,
    /// Bitset over signal ids: bit set ⇔ `bitsel_edge_sens` has ≥1 entry for
    /// that sid. Lets the edge dispatch loop skip the per-(sid, block) tuple
    /// hash for the overwhelmingly common signals with no bit-select
    /// sensitivity (one such entry anywhere used to put a HashMap get on
    /// every block wake — ~1.5% of c906 memcpy cycles).
    bitsel_sid_bits: Vec<u64>,
    /// Reusable bitmap for `check_edges`. Hoisted out of the per-iteration
    /// `vec![false; blocks.len()]` to avoid ~10 K-byte alloc/drop on every
    /// settle iter (8938 iters × ~10 K blocks = 89 MB churn on c910 hello).
    edge_triggered_bitmap: Vec<bool>,
    /// Reusable work buffers for `check_edges`; populated directly while
    /// walking fired edge signals so we do not scan the whole bitmap to
    /// discover triggered blocks.
    edge_triggered_list: Vec<usize>,
    /// Reusable `check_edges` buffers for the fired positions' detect-time
    /// values (narrow planes / wide values); a fresh pair grew on every pass.
    edge_fired_snap: Vec<(usize, u64, u64)>,
    edge_fired_wide: Vec<(usize, Value)>,
    /// Generation marker for gateable blocks rejected by ARMED filtering
    /// during fanout dispatch. This deduplicates multi-sensitivity blocks
    /// without appending skipped work to `edge_triggered_list`.
    edge_prefilter_seen: Vec<u32>,
    edge_prefilter_generation: u32,
    edge_parallel_work: Vec<usize>,
    edge_sequential_work: Vec<usize>,
    nba_queue: Vec<NbaEntry>,
    /// §15.5.2 `->> e` nonblocking event triggers: fired when the NBA region
    /// flushes, not in the active region (a same-slot `.triggered` read in an
    /// active/inactive process must still see 0).
    pending_nba_triggers: Vec<String>,
    /// NBA-deferred `->>m_event` triggers on class-field events, captured as
    /// the resolved `(this_handle, field_name)` key at trigger time (the NBA
    /// drain runs outside the method, with no `this` to reconstruct it).
    pending_nba_instance_triggers: Vec<(usize, String)>,
    /// §9.6.2 `disable <task_name>`: which processes are currently executing
    /// an invocation of each named task. Filled by bind_task_frame, cleared
    /// by unwind_task_frame; a cross-process disable kills the listed pids.
    active_task_pids: HashMap<String, HashSet<usize>>,
    /// §15.5.5 event variables are HANDLES to synchronization objects:
    /// `ev_alias = ev_b` makes both names one object. Maps an event variable
    /// (or element key `arr[i]`) to the event it currently aliases;
    /// `EVENT_NULL_KEY` marks a null handle. Trigger/`.triggered`/wait_order
    /// all chase this map to the canonical key first.
    event_aliases: HashMap<String, String>,
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
    nba_fast_index: NbaFastIndex,
    /// Edge-sensitive always blocks. Behind an `Arc` so an edge pass can hold a
    /// cheap handle to the list while `self` stays fully usable: the pass used
    /// to `mem::take` this Vec, which left it EMPTY for the duration. Any
    /// re-entrant lookup — a nested `check_edges` from a `#delay` in an edge
    /// block's body, or the `self.edge_blocks.get(bi)` probes elsewhere — then
    /// saw nothing and silently dropped every block (see `check_edges_inner`).
    edge_blocks: Arc<Vec<EdgeSensitiveBlock>>,
    /// Bytecode-compiled edge blocks (for blocks that compiled successfully).
    /// Index matches edge_blocks. None = fallback to AST interpreter.
    compiled_edge_blocks: Vec<Option<super::bytecode::CompiledBlock>>,
    /// Parallel array to `compiled_edge_blocks`: when feature=jit is on
    /// and `JitModule::try_compile` succeeded at elaboration time, this
    /// holds the native function pointer for the block. `exec_bytecode`
    /// calls it in place of the interpreter loop. None = use interpreter.
    jit_fns: Vec<Option<super::jit::JitFn>>,
    /// Native code for COMB (settle) entries, by comb-entry index. Settle is
    /// ~70% of the loop on DUT-heavy RTL while edge blocks are ~25%, so this
    /// is where the JIT pays. cfg-gated: an earlier runtime-checked version
    /// cost +2.0% in the DEFAULT build from the hot-loop check alone.
    #[cfg(feature = "jit")]
    comb_jit_fns: Vec<Option<super::jit::JitFn>>,
    /// Template-mode native comb fns: `(body, offset into comb_tpl_map_blob,
    /// map len)` per entry. The mapping arrays are HOST-owned (one flat blob)
    /// so the generated crate carries only the bodies — see
    /// `gen_templated_blocks`.
    #[cfg(feature = "jit")]
    comb_tpl_fns: Vec<Option<(super::aot::AotTplFn, u32, u32)>>,
    #[cfg(feature = "jit")]
    comb_tpl_map_blob: Vec<u32>,
    /// Template-mode native EDGE-block fns, same layout as `comb_tpl_fns`.
    /// Checked before `jit_fns` in the edge fast path.
    #[cfg(feature = "jit")]
    edge_tpl_fns: Vec<Option<(super::aot::AotTplFn, u32, u32)>>,
    #[cfg(feature = "jit")]
    edge_tpl_map_blob: Vec<u32>,
    /// Consecutive X/Z-bail strikes per entry (retires the fn past a limit).
    #[cfg(feature = "jit")]
    comb_jit_strikes: Vec<u8>,
    /// (ran natively, bailed on X/Z) under XEZIM_PROFILE_TIMING.
    #[cfg(feature = "jit")]
    prof_comb_jit: [u64; 2],
    /// The `JitModule` owns the JIT'd code's memory — it must outlive
    /// all compiled function pointers. Kept on Simulator so the mmapped
    /// code pages stay mapped for the life of the simulation.
    jit_module: Option<super::jit::JitModule>,
    /// True for blocks eligible for parallel execution (no StmtFallback).
    edge_block_parallel: Vec<bool>,
    /// Per-edge-block partition / core assignment. Indexed by edge_block
    /// idx; value is the partition (worker thread) that owns the block.
    /// Empty when no partition has been loaded — falls back to the legacy
    /// chunk-by-range scheduling. Loaded from a partition file emitted by
    /// mt-kahypar (or any compatible hMETIS partitioner).
    pub(crate) edge_block_partition: Vec<u32>,
    /// Number of distinct partitions (k) when `edge_block_partition` is
    /// populated. Drives how many worker threads the parallel-edge
    /// dispatcher spawns.
    pub(crate) edge_block_partition_count: u32,
    /// Per-edge-block execution counts. Cheap enough to maintain on the hot
    /// path, and used only for optional end-of-run attribution.
    edge_block_exec_counts: Vec<u64>,
    edge_block_stats_enabled: bool,
    /// Phase-2 profile-guided partitioning: accumulated wall time
    /// (nanoseconds) spent executing each edge block. Populated
    /// only when `edge_block_stats_enabled` is set. Drives the
    /// vertex weight in profile-guided hypergraph emission so
    /// mt-kahypar balances *real* per-thread work, not insn count.
    edge_block_exec_ns: Vec<u64>,
    /// Phase-2 profile-guided partitioning: per-signal toggle count
    /// for signals in the edge-sensitivity universe. Updated lazily
    /// in `check_edge_id` whenever a sampled signal changes. Used as
    /// the hyperedge weight in profile-guided emission — busy
    /// signals are more expensive to share across cores.
    signal_toggle_count: Vec<u64>,
    /// Phase-4 counter: number of NBA writes performed per edge
    /// block during a stats-enabled run. NBAs cost a queue insert +
    /// a later apply pass, both of which are heavier than reads.
    /// Better vertex-weight signal than insn count for highly
    /// asymmetric blocks (small body, many NBAs vs big body, few NBAs).
    nba_count_per_block: Vec<u64>,
    /// Phase-4 counter: per-sensitivity-signal trigger count
    /// (sid → fires). Subset of signal_toggle_count restricted to
    /// signals that actually appear in some block's sensitivity list.
    /// Filters out toggles on signals that never wake anyone.
    sensitivity_trigger_count: Vec<u64>,
    /// Phase-4 counter: per-block cross-block wakeup count. When
    /// block A's NBA write changes a signal that triggers block B,
    /// `cross_block_wakeup_count[A]` increments. Captures the actual
    /// "packet flow" between blocks — much sharper hyperedge weight
    /// than static fanout because it counts only the signals that
    /// CAUSED a wakeup, not all readers.
    cross_block_wakeup_count: Vec<u64>,
    /// Phase-4 counter: per-partition local-quiescence iteration
    /// count — how many settle iterations were resolved within the
    /// partition before any cross-partition signal had to be
    /// merged. Higher = better partition (more local work). Same
    /// indexing as boundary_packet_count.
    local_quiescence_iterations: Vec<u64>,
    /// Per-edge-block: cached scope hint (parent module name from first
    /// sensitivity signal) so sequential-exec path can skip per-call rsplit.
    edge_block_scope: Vec<Option<String>>,
    /// Per-edge-block: true if block contains StmtFallback insns (and thus
    /// needs `name_resolve_hint` to be set for AST-exec). When false, the
    /// save/restore clone of the RefCell string can be skipped.
    edge_block_needs_hint: Vec<bool>,
    /// Per edge block: no fallback, no `Format`, compiled — the block can
    /// neither resolve a bare name nor print `%m`, so the per-firing scope
    /// bookkeeping in `exec_bytecode` (a String clone into
    /// `name_resolve_hint` + the `%m` buffer refresh) is skipped. At ~580
    /// fired blocks/tick the clone alone was allocator-visible in perf.
    edge_block_light: Vec<bool>,
    /// Flat copy of each compiled edge block's instruction count so the hot
    /// dispatch loop doesn't chase the `Option<CompiledBlock>` header.
    edge_block_insn_len: Vec<u32>,
    /// VM register file (reusable across executions to avoid allocation).
    vm_regs: Vec<Value>,
    /// class-perf Step 9d: foreach key arenas for compiled methods — slot
    /// i holds the materialized key `Value`s plus the per-slot position
    /// cursor of the i-th (compile-time numbered) `foreach` in the
    /// currently-executing block. Swapped in `try_run_compiled_method`
    /// alongside `vm_regs` so a nested compiled call never clobbers an
    /// outer loop's keys; grown lazily to the block's `foreach_slots`.
    /// The cursor is a `u32` (never `usize::MAX`-sentinel — the executor
    /// compares against `keys.len()`).
    foreach_arena: Vec<(Vec<Value>, u32)>,
    /// Built-in clock generators (optimized always #N clk = ~clk)
    clock_generators: Vec<ClockGen>,
    /// Source position of every clock generator, for its time-0 rank:
    /// `(index into clock_generators, instance scope, source offset)`.
    clock_gen_src: Vec<(usize, String, usize)>,
    /// Pids of `initial` blocks that cannot suspend (no timing control,
    /// no blocking call); see `order_time0_processes`.
    t0_timing_free: HashSet<usize>,
    /// Time-0 activation index of each process queued at time 0, consulted
    /// by `clock_rank_on_pop` until the first drain settles the generators.
    t0_pid_ord: HashMap<usize, usize>,
    /// Dynamic-delay simple assignments, keyed by their scheduled process id.
    fast_delay_always: HashMap<usize, FastDelayAlways>,
    /// Compiled process FSMs by pid (roadmap 11-12, opt-in XEZIM_PROC_FSM=1).
    proc_fsm: HashMap<usize, Box<ProcFsm>>,
    /// Resume pc for the next `exec_insns` call (consumed at entry; 0 for
    /// ordinary block executions).
    fsm_start_pc: u32,
    /// Set by the Wait insn arms: (resume pc, what to wait on). `exec_insns`
    /// breaks its loop right after setting this.
    fsm_suspend: Option<(u32, FsmWait)>,
    /// Precision diff of the CURRENTLY EXECUTING process FSM (the Wait arm
    /// needs it to quantize the delay register).
    fsm_precision_diff: i32,
    event_queue: TimingWheel,
    next_pid: usize,
    current_pid: usize,
    /// Monotonic counter assigning each pushed local call-frame a unique
    /// generation (see `local_gen_stack`). Used to tell apart two different
    /// method activations that reuse the same `local_stack` slot index — e.g.
    /// a fork child spawned under method-<i>A</i> that keeps running after
    /// <i>A</i> returns must not merge its (stale) locals into the frame slot
    /// that method-<i>B</i> now owns (regression 3627).
    frame_gen: u64,
    /// Value that `$` resolves to in the current evaluation scope
    /// (e.g. queue upper bound during `q[a:$]`). Stack of overrides.
    dollar_bound: Vec<i64>,
    break_flag: bool,
    continue_flag: bool,
    /// Set by a `return` (alongside break_flag). Distinguishes a task/function
    /// `return` from a loop `break`: a return must unwind to the enclosing
    /// task's `ScopePop` boundary (which clears it via `saved_return`), whereas
    /// a `break` only exits the nearest loop. Without this, an inlined blocking
    /// task's `return` leaked break_flag past its ScopePop into the caller's
    /// continuation.
    return_flag: bool,
    rs_return_flag: bool,
    /// Set when `exec_statement`'s Wait handler parks the process via
    /// `exec_park_cont`. Must propagate through method-frame save/restore.
    parked_from_exec: bool,
    /// When `exec_statement` encounters a blocking `wait(cond)` with a
    /// FALSE condition inside a loop body (foreach/while/for), it cannot park
    /// (it has no continuation). `run_process_stmts` sets this to the full
    /// continuation `[stmt, stmts[i+1..]]` before calling `exec_statement`, and
    /// `exec_statement`'s Wait handler reads it to park the process.
    exec_park_cont: Option<ProcCont>,
    /// Depth of DPI-exported subroutines running for a C caller. Their
    /// statements run on the synchronous path and cannot park, so a wait
    /// there runs the scheduler nested instead (`run_nested_until`).
    dpi_export_depth: u32,
    /// §35.5.2/§35.9: imported tasks running on their own stacks
    /// (`dpi_task.rs`): the SV names of the imported tasks a loaded library
    /// implements (empty: none, and the mechanism costs nothing), the live
    /// calls by id, the one running now, and whether that one is unwinding
    /// after a disable.
    dpi_task_names: HashSet<String>,
    dpi_tasks: HashMap<u64, Box<dpi_task::DpiTask>>,
    /// Strings an exported subroutine handed to C (a `string` result or
    /// output), kept until its next call (§35.5.6).
    dpi_export_cstrings: HashMap<usize, Vec<std::ffi::CString>>,
    /// Numbers the hidden variables of each call from C (`dpi_export.rs`).
    dpi_export_seq: u64,
    /// The time slot batches `run_events_until` is running, outermost first:
    /// their unrun processes (`requeue_slot_batches`).
    slot_batches: Vec<(u64, EventList)>,
    dpi_cur_task: Option<u64>,
    dpi_unwinding: bool,
    next_dpi_task: u64,
    /// Wake markers fired by waiters registered from that nested path.
    sync_wakes: HashSet<u64>,
    /// The run ended while a task entered from C was still waiting: nothing
    /// was left to wake it, or `--max-time` came first (`run_nested_until`).
    /// `finished` is raised to stop it, but no `$finish` was called.
    ended_waiting_in_c: bool,
    next_sync_wake: u64,
    /// File names of open multichannel descriptors (`vpi_mcd_name`).
    mcd_names: HashMap<i32, String>,
    /// No-progress guard for the `foreach` replay fallback: how many times
    /// a given `(pid, statement offset)` has parked without the loop ever
    /// completing. That fallback restarts at index 0 on every resume, so a
    /// run of parks means the loop is stuck rather than iterating.
    foreach_replay_parks: HashMap<(usize, usize), u32>,
    /// Consecutive parks tolerated before the guard fires. 0 disables it.
    /// Override with `XEZIM_FOREACH_REPLAY_LIMIT`.
    foreach_replay_limit: u32,
    /// SV-2023: target named block for `disable <name>` propagation.
    disable_target: Option<String>,
    /// §9.6.2: a `disable` just executed whose target has not been located
    /// yet — the process loop checks its own continuation first, then the
    /// processes the block may belong to (`disable_remote_block`).
    disable_check_pending: bool,
    /// §9.6.2: address of each flattened NAMED block frame → the block name,
    /// so a `disable` can find where the block ends in a continuation chain.
    named_block_frames: HashMap<usize, String>,
    /// §9.6.2: the forking process's continuation at each run-loop fork
    /// child's spawn — the named blocks the child was forked inside.
    fork_spawn_cont: HashMap<usize, ProcCont>,
    fork_spawn_prune_at: usize,
    /// §9.6.2: processes unwound out of a named block or task by another
    /// process's `disable`, applied when each next runs.
    pending_block_disable: HashMap<usize, String>,
    /// Label of a process-level named block (`initial begin : worker`) -> its
    /// pid, so `disable worker` from another process can terminate it.
    disable_labels: HashMap<String, usize>,
    /// A named `fork : blk ... join*` maps to the processes it spawned, so
    /// `disable blk` can terminate them (§9.6.2). Without this, disabling a
    /// join_none fork block — which has already returned — unwound the
    /// DISABLING process instead, dropping its continuation.
    fork_block_children: HashMap<String, HashSet<usize>>,
    /// Associative arrays already warned about an x/z index.
    warned_assoc_index: HashSet<String>,
    /// `#delay` sites already warned about a pathological (non-finite / absurdly
    /// large) real delay value, keyed by the delay expression's source offset —
    /// warn once per site so a spiking clock names itself without spamming.
    warned_delay_spikes: HashSet<usize>,
    /// PIDs whose first-at-time-0 variable `#(expr)` delay evaluated to 0 and
    /// was deferred once past the time-0 settle (so the period signal — e.g. a
    /// real cont-assign feeding `always #(p/2) clk=~clk` — settles before the
    /// clock's phase is fixed). Guards the one-shot re-park against a genuine
    /// zero-period loop re-parking forever at t=0.
    t0_delay_deferred: HashSet<usize>,
    /// Prepared frames for loops that suspend once per iteration.
    suspended_loop_frames: HashMap<usize, HashMap<(usize, usize), Arc<[Statement]>>>,
    /// Bytecode cached for non-suspending process loops after their enclosing
    /// local frame is known.
    process_loop_bytecode: HashMap<(usize, usize, usize), Option<super::bytecode::CompiledBlock>>,
    /// Bytecode cached for the CONDITION of a per-cycle self-check `if` whose
    /// statement as a whole cannot compile (e.g. its fail branch touches a
    /// name-keyed initial-scope variable). The condition — typically a wide
    /// compare against an inlinable helper call — is the hot part; the chosen
    /// branch still executes through the AST path.
    /// The u8 is the SELF-AUDIT countdown: the first activations evaluate the
    /// condition through BOTH the bytecode and the AST and compare; any
    /// disagreement permanently pins the span to the AST (None). Only
    /// side-effect-free conditions are cached, so the double evaluation is
    /// unobservable.
    process_cond_bytecode: HashMap<
        (usize, usize, usize),
        Option<(super::bytecode::CompiledBlock, super::bytecode::RegId, u8)>,
    >,
    /// SV-2023: PIDs killed by `disable fork` — skip dispatch on these.
    killed_pids: HashSet<usize>,
    /// §9.7 `process` class: PIDs that have completed execution (child_finished).
    finished_pids: HashSet<usize>,
    /// §9.7 `process` class: PIDs explicitly suspended via `suspend()`. The
    /// continuation + original delay info is in `suspended_proc_info`.
    suspended_pids: HashSet<usize>,
    /// §9.7: detailed resumption info for each suspended pid.
    suspended_proc_info: HashMap<usize, SuspendedProc>,
    /// §9.7 `process::await()`: processes waiting for another's termination.
    await_waiters: Vec<AwaitWaiter>,
    /// Names of init-declared (automatic) `for`-loop variables that — because
    /// the enclosing process had no local frame — currently live in the signal
    /// table. A `fork` child must capture these BY VALUE at fork time so the
    /// canonical `for (int i…) fork … join_none` idiom sees its per-iteration
    /// `i` instead of the loop's final value (LRM §9.3.2). Pushed/popped as a
    /// stack by the `for` executor to handle nested loops.
    auto_loop_vars: Vec<String>,
    /// SV-2023: redirected lvalues for `ref` formal parameters during a call.
    /// Top-of-stack scope is consulted when assigning to a known formal name.
    ref_binding_stack: Vec<HashMap<String, Expression>>,
    /// §13.5.2 TRUE ALIASING for the subset of `ref` actuals whose storage is
    /// module-visible (a plain variable, resolvable independent of the caller
    /// frame). Reads of the formal re-read the actual and writes write through
    /// IMMEDIATELY — a parallel process observing the actual mid-call sees the
    /// callee's writes, and no return copy-out clobbers writes the observer
    /// made during the call. Actuals that are caller-frame locals or aggregate
    /// elements keep the legacy copy-in/copy-out path (entry absent here).
    /// Pushed/popped in lockstep with `ref_binding_stack`.
    ref_alias_stack: Vec<HashMap<String, String>>,
    /// §13.5.2 `ref` formals bound by IDENTITY (aggregates with no storage of
    /// their own); element accesses on them are rewritten onto the actual.
    /// Pushed/popped in lockstep with `ref_binding_stack`.
    ref_identity_stack: Vec<Vec<String>>,
    /// Rename-table entries of the queue / associative formals bound so far
    /// in the call being set up (see `bind_queue_param`). They enter the
    /// callee's frame only once every actual is evaluated: an actual must
    /// not resolve to a formal of the call it is being passed to.
    pending_formal_dyn: Vec<(String, String)>,
    /// True while the top ref frame has an aliased or identity-bound formal —
    /// the only state in which `ref_formal_redirect_*` can rewrite anything.
    /// Maintained on push/pop/take/restore so the hot paths test one bool.
    ref_redirect_hot: bool,
    /// §19.7.1 `cg::type_option.<field>` per covergroup TYPE (weight, goal,
    /// comment, strobe, merge_instances), seeded from the body's
    /// `type_option.f = v;` items on first access.
    cg_type_options: HashMap<String, HashMap<String, Value>>,
    /// Per-call-site receiver expressions for `obj.coll.method()` dispatch.
    method_receiver_cache: HashMap<(usize, usize, usize, u32, u32), std::rc::Rc<Expression>>,
    /// Reused buffer for the `<hint>.<name>` probe in `get_signal_value_by_name` (a `&self` path).
    hint_key_scratch: std::cell::RefCell<String>,
    /// Reused buffer for the `__vif_local__` key probes (`&self` paths).
    vif_key_scratch: std::cell::RefCell<String>,
    /// §8.25: specialization-argument texts already folded to their value
    /// (`2 + 2` -> `4`, `'h4` -> `4`), or None when the text is not a
    /// constant integer expression (see `fold_const_spec_frag`).
    spec_frag_fold_cache: RefCell<HashMap<String, Option<String>>>,
    /// The leaf name of every design parameter key (`LP` of `tb.LP`), built
    /// on first use: the gate before a scoped parameter lookup.
    param_leaf_names: std::cell::OnceCell<HashSet<String>>,
    /// §6.22.1: canonical type text of each design-level unpacked typedef
    /// (`int[$]`) -> the typedef that stands for that type in
    /// specialization keys (built on first use; see `unpacked_typedef_rep`).
    unpacked_typedef_reps: std::cell::OnceCell<HashMap<String, String>>,
    /// Every typedef name (design or class scope, bare and qualified) that
    /// carries unpacked dimensions or aliases one that does: the cheap gate
    /// of `unpacked_typedef_rep`.
    unpacked_typedef_names: std::cell::OnceCell<HashSet<String>>,
    /// Reused buffer for `frame_leaf_key_of`'s marker probe.
    marker_key_scratch: String,
    /// See `lvalue_root_is_unpacked_struct_prop`.
    unpacked_struct_prop_names: std::cell::OnceCell<HashSet<String>>,
    /// See `struct_root_possible`.
    struct_root_names: HashSet<String>,
    struct_root_names_built: bool,
    /// See `multidim_members_possible`.
    multidim_members: Option<bool>,
    /// See `struct_prop_name_possible`.
    struct_capable_prop_names: std::cell::OnceCell<HashSet<String>>,
    /// Member names that appear as dynamic/queue/associative members of a
    /// struct type a class property (or a type-parameter binding, which can
    /// only name a typedef) can resolve to — the cheap gate in front of
    /// `class_struct_coll_name`'s receiver walk.
    struct_coll_member_prop_names: std::cell::OnceCell<HashSet<String>>,
    /// §7.4.5: names whose ELEMENTS are collections — variables declared as
    /// a fixed array of queues / dynamic / associative arrays (`int q3[3][$]`)
    /// and struct members declared that way (`T sp[4][$]`). Gates the
    /// whole-collection assignment of a selected element (`q3[0] = q3[1]`).
    indexed_coll_roots: std::cell::OnceCell<HashSet<String>>,
    /// Bit `len % 64` per name in `indexed_coll_roots` (0: none), so the
    /// common miss costs no hashing.
    indexed_coll_mask: std::cell::Cell<u64>,
    /// Bit `len % 64` set for the length of every name in
    /// `struct_coll_member_prop_names`: rejects most names without hashing.
    struct_coll_member_len_mask: std::cell::Cell<u64>,
    /// Struct-member collections that live inside an associative-array
    /// ELEMENT (`aa[k].q`): growing one creates the element (§7.8.6), see
    /// `mark_struct_coll_owner_present`.
    assoc_elem_colls: HashSet<String>,
    /// Element collections (`@v#3[0]`, ...) of each automatic local array of
    /// collections, keyed by the local's storage; dropped with it (see
    /// `declare_local_coll_array`, `cleanup_dyn_storage`).
    local_coll_elems: HashMap<String, Vec<String>>,
    /// Declared associative tails of dynamic roots and their scoped rows.
    dynamic_assoc_dims: HashMap<String, Vec<crate::ast::types::UnpackedDimension>>,
    /// `(callee member, caller member)` collection copy-backs of the dynamic
    /// members of an output/inout/ref struct formal, drained into the call's
    /// queue writebacks (see `bind_unpacked_struct_formal`).
    pending_struct_coll_wb: Vec<(String, String)>,
    /// The staged dynamic members of a struct a `return` handed back (see
    /// `stage_ret_struct_colls`), for the caller's assignment to copy.
    pending_ret_struct: Option<String>,
    /// See `class_member_names`.
    class_member_names_cell: std::cell::OnceCell<ClassMemberNames>,
    /// Every bare name a `__vif_local__` key was ever created for (never
    /// shrinks): a name outside it has no such key.
    vif_local_names: HashSet<String>,
    /// See `nonvirtual_target_class`.
    #[allow(clippy::type_complexity)]
    nonvirtual_target_cache: std::cell::RefCell<HashMap<String, HashMap<String, Option<String>>>>,
    /// `class_has_method` answers by class then method (see there).
    has_method_cache: std::cell::RefCell<HashMap<String, HashMap<String, bool>>>,
    /// See `member_string_keyed_unbound`.
    #[allow(clippy::type_complexity)]
    string_keyed_member_cache: std::cell::RefCell<HashMap<String, HashMap<String, Option<bool>>>>,
    /// See `method_defining_class`.
    #[allow(clippy::type_complexity)]
    method_def_cache: std::cell::RefCell<HashMap<String, HashMap<String, Option<MethodDef>>>>,
    /// Every covergroup key and its leaf after the last `::` or `.` (see
    /// `covergroup_def_for`).
    covergroup_leaf_names: std::cell::OnceCell<HashSet<String>>,
    /// Class -> the `string_properties` of it and its ancestors, in chain
    /// order (a method call marks them in `string_signals`).
    class_string_props: std::cell::RefCell<HashMap<String, std::rc::Rc<Vec<String>>>>,
    /// Class -> `class_enclosing_module` answer.
    class_enclosing_cache: std::cell::RefCell<HashMap<String, Option<String>>>,
    /// See `plain_class_ref`.
    plain_class_types: std::cell::RefCell<Option<HashMap<String, bool>>>,
    /// See `coll_member_possible`.
    coll_member_names: std::cell::RefCell<Option<HashSet<String>>>,
    /// Reused `<name>.size` key buffer of the queue-frame save/restore.
    queue_key_scratch: String,
    /// See `prop_owners`.
    #[allow(clippy::type_complexity)]
    prop_owner_index: std::cell::RefCell<HashMap<String, std::rc::Rc<PropOwners>>>,
    /// `class_prop_width_impl` answers that no instance can change, by class
    /// then property; `None` marks a width the instance's bindings decide.
    #[allow(clippy::type_complexity)]
    prop_width_memo: std::cell::RefCell<HashMap<String, HashMap<String, Option<Option<u32>>>>>,
    /// Emptied formal-metadata snapshot vectors of the class-method call path,
    /// reused across calls.
    formal_parts_pool: Vec<Vec<(usize, FormalMetaParts)>>,
    /// Emptied `queue_frame_saves` / `local_dyn` frames and saved-name
    /// buffers, reused by the next subroutine entry.
    queue_save_pool: Vec<Vec<(String, QueueLocalSave)>>,
    local_dyn_pool: Vec<Vec<(String, String)>>,
    queue_name_pool: Vec<String>,
    /// Emptied `current_spec` save buffers (see `clone_spec_pooled`).
    spec_buf_pool: Vec<(String, String)>,
    /// Open class-method metadata saves, innermost last (see `MetaFrame`).
    meta_frames: Vec<MetaFrame>,
    /// How many of `meta_frames` have not taken their snapshot yet.
    meta_pending: usize,
    /// Call plans by `Arc<ClassMethod>` address (see `MethodCallPlan`).
    method_plans: std::cell::RefCell<HashMap<usize, std::rc::Rc<MethodCallPlan>>>,
    /// Moves whenever a procedural `typedef` or an inline-enum local edits the
    /// type tables at run time (see `MethodCallPlan`).
    type_tables_gen: u64,
    /// Emptied `local_type_stack` overlay pairs, reused by `push_local_frame`
    /// (capacity kept, contents cleared) instead of allocating a map pair for
    /// every activation that records a class/typedef-typed name.
    local_type_pool: Vec<(HashMap<String, String>, HashMap<String, String>)>,
    /// Emptied subroutine names of popped `static_local_syncs` frames, reused
    /// by the next frame push instead of allocating a copy of the name.
    sync_name_pool: Vec<String>,
    /// Scratch key for `cleanup_dyn_storage`.
    dyn_key_scratch: String,
    /// Interned scope hints and head identifiers for `method_receiver_cache` keys (see there).
    method_receiver_hint_ids: HashMap<String, u32>,
    /// class-perf Step 4b: cache of fully-lowered class-FUNCTION `CompiledBlock`s,
    /// keyed by an INTERNED (class, method) id pair PLUS the resolved width
    /// signature (formal widths + result width). The width signature captures
    /// all instance-dependent compile inputs (parameterized formals / return),
    /// so a cached block is always byte-identical to a fresh compile of the
    /// same instance. String keys are never cached (HashMaps intern to u32).
    compiled_class_method_ids: HashMap<String, u32>,
    compiled_method_block_cache: HashMap<
        (u32, u32, u32, std::rc::Rc<Vec<u32>>, u32),
        super::bytecode::CompiledMethodOutcome,
    >,
    compiled_method_admission: Option<std::rc::Rc<super::bytecode::MethodAdmission>>,
    persisted_method_context_hash: Option<u64>,
    /// class-perf Step 7b: prebound compiled-method gates, keyed (cid, mid).
    /// Present only for methods that PASSED the compile gate; the per-call
    /// type-resolution work (return kind, formal seed widths) runs once at
    /// plan creation instead of per call.
    compiled_method_plans:
        HashMap<(u32, u32, u32), std::rc::Rc<super::bytecode::PreboundCompiledMethod>>,
    /// class-perf decision cache: interned (class, method) id pairs whose
    /// method can NEVER run as bytecode (task-form, string return, void, a
    /// ref/output formal, or a non-scalar return). These decisions are
    /// deterministic per method and instance-independent (`scalar_formal_integral`
    /// resolves widths from module scope), so once decided they never flip;
    /// memoizing them skips the gate + type-resolution re-validation on every
    /// one of the (mostly negative-cached) UVM calls.
    compiled_method_skip: HashSet<(u32, u32, u32)>,
    /// class-perf adaptive tiering: per-interned-(cid, mid) call counters.
    /// Bumped ONLY by methods that passed the skip cache (plausibly
    /// compilable) and only while below the tier threshold; once a method
    /// crosses, it compiles and the counter is never touched again (the
    /// compiled path above the threshold is counter-free). Keyed by the
    /// (cid, mid) ids the block cache uses, so interning is done once.
    compiled_method_call_counts: HashMap<(u32, u32, u32), u32>,
    /// class-perf loop-heat boost (MEMOIZATION / pass B): whether a method
    /// is a wait-free loop candidate, keyed (cid, mid, spec_id). Computing it
    /// runs a structural statement walk (`body_has_loop` + a wait-free probe),
    /// which the maintainer flagged as a per-call cost; it is a pure function
    /// of the declaration (instance-independent), so it is memoized once and
    /// never re-walked. `None` = not yet decided; `Some(true)` = candidate.
    compiled_method_boost_qual: HashMap<(u32, u32, u32), bool>,
    /// class-perf loop-heat boost (MEASURED HEAT / pass C): stack of interned
    /// (cid, mid, spec_id) keys for the wait-free loop-candidate methods
    /// currently interpreting on the AST function path. `exec_statement`'s
    /// loop arms add their back-edge count to the innermost key, so heat is
    /// attributed to the actual method running the loop (nested calls
    /// attribute to the innermost active one). Empty when the boost is off,
    /// keeping the default path flat.
    compiled_method_heat_stack: Vec<(u32, u32, u32)>,
    /// class-perf loop-heat boost (MEASURED HEAT / pass C): accumulated loop
    /// back-edges (iterations) a method's body actually ran while on the AST
    /// interpreter, keyed (cid, mid, spec_id). A body crosses the tier here
    /// -> treated as loop-hot (compiles) instead of compiling any short loop
    /// on first call.
    compiled_method_loop_heat: HashMap<(u32, u32, u32), u64>,
    /// class-perf P3: direct VM->VM dispatch entries for `CallMethod`,
    /// keyed (cid, mid) of the DEFINING class. `Pending` re-probes after
    /// the plan / block caches fill; `Never` is a permanent decline
    /// (param-dependent shape, writeback formals, fallback-heavy body);
    /// `Entry` runs the callee's block in-place with a fresh register
    /// file, skipping arg materialization + interpreter binding.
    compiled_fast_calls: HashMap<(u32, u32, u32), FastCallState>,
    /// Static-method twin of `compiled_fast_calls` (class-perf Change 3):
    /// memoized direct-call entries for no-arg static class functions, keyed
    /// (cid, mid, 0). A separate map because `vm_fast_entry_for` permanently
    /// declines static methods in the instance map, which would poison this
    /// path's memoization.
    compiled_fast_static_calls: HashMap<(u32, u32, u32), FastCallState>,
    /// Phase 2 (class-perf): intern a `(base_class, spec_sig)` specialization to a
    /// u32 id (0 = non-specialized / no spec), so the plan / skip / fast-call caches
    /// can be keyed by `(spec_id, cid, mid)` instead of `(cid, mid)`. Without this, a
    /// parameterized class's method (e.g. `Cache#(string)::getv`) shared one plan
    /// across ALL specializations, forcing it to the AST interpreter whenever a
    /// type-param formal/return wasn't module-scope-resolvable. Interned lazily from
    /// `inst.spec` at the plan-lookup point.
    compiled_spec_intern: HashMap<(String, String), u32>,
    /// `resolve_typeref_class_name` memo: name -> scope -> class ctx -> (class-table size, answer).
    #[allow(clippy::type_complexity)]
    typeref_class_memo: std::cell::RefCell<
        HashMap<String, HashMap<String, HashMap<String, (usize, Option<String>)>>>,
    >,
    /// Deferred teardown for inlined blocking task/method calls (LIFO). Each
    /// `ScopePop` sentinel pops and replays the top entry. Per-process (carried
    /// in ProcessContext across suspension).
    task_cleanup: Vec<TaskCleanup>,
    /// Names carrying a TRUSTED `var_typedef_types` binding — created by an
    /// active `foreach` key-type binding (§6.19.6/§12.7.3) and restored by
    /// `restore_loop_vars`. Consulted before every other type source.
    fe_trusted_types: HashSet<String>,
    /// `wait(expr)` whose condition references no signals — it depends on class
    /// members / locals another same-time process mutates.
    /// Such a process parks here
    /// (pid + continuation whose first stmt is the Wait) and is re-checked in a
    /// fixpoint after the same-time batch drains, suspending until the
    /// condition genuinely flips instead of falsely proceeding.
    condition_waiters: Vec<(usize, ProcCont)>,
    /// Waiters from `condition_waiters` whose conditions became true upon
    /// blocking writes, recorded in write-satisfaction order (§9.7.4).
    ready_condition_waiters: Vec<(usize, ProcCont)>,
    /// Ident/member names read by the wait condition of each parked waiter.
    cond_waiter_reads: HashMap<usize, HashSet<String>>,
    /// Wait condition expression for each parked waiter.
    cond_waiter_conditions: HashMap<usize, Expression>,
    /// Per parked condition waiter: `Some(bits)` when everything its
    /// condition reads lives in the string-keyed stores (see
    /// `cond_gate_for`), with the `name_bit`s of each name it reads. Such a
    /// waiter is skipped by the bulk re-schedule until a write notes all the
    /// bits of one of those names — nothing it can observe has changed; a
    /// constant condition (`wait(0)`) reads no name and is never re-checked.
    /// `None` keeps the unconditional re-schedule (RTL signals, calls).
    cond_waiter_gate: HashMap<usize, Option<Box<[u64]>>>,
    /// Timestamp of each UNGATED parked waiter's last unconditional re-check
    /// by a re-pass drain (`drain_condition_waiters(_, from_repass: true)`);
    /// see that method for why one re-check per timestamp is enough.
    cond_repass_recheck: HashMap<usize, u64>,
    /// IEEE 1800-2017 §4.4.2.3 Inactive region: continuations of `#0`
    /// delays park here instead of in the event_queue. The event_queue's
    /// batch drain in `run_one_tick` re-fetches same-time entries into the
    /// SAME active pass (before `apply_nba`), so scheduling a `#0`
    /// continuation there made it resume BEFORE the NBA region committed —
    /// an NBA posted before the `#0` was invisible after it. Commercial
    /// simulators all make an NBA posted before a
    /// `#0` visible after it, so entries parked here are promoted back into
    /// the event queue only at the END of the current tick, after
    /// `apply_nba` has run (see `promote_inactive_to_active`).
    inactive_queue: Vec<(usize, ProcCont)>,
    /// Processes waiting for the NBA region to commit (e.g.
    /// `uvm_wait_for_nba_region`: `nba <= next_nba; @(nba)` where `nba` is a
    /// procedural variable). Resumed in `run_one_tick` after all active +
    /// inactive (`#0`) deltas have settled and `apply_nba` has committed.
    nba_region_waiters: Vec<(usize, ProcCont)>,
    /// Bumped whenever a `wait(expr)` condition evaluates true (a parked waiter
    /// proceeds). The condition-waiter fixpoint uses it to detect "no progress
    /// this round" and stop (genuine stall).
    cond_progress: u64,
    /// SV-2023: blocked mailbox `get`/`peek` proxies keyed by mailbox handle.
    /// Each waiter records the suspended process pid, the destination lvalue
    /// expression to receive the value, and the post-get continuation
    /// statements. `put` drains a waiter, assigns the value into its lvalue,
    /// and re-schedules the continuation at the current time.
    mailbox_get_waiters: HashMap<usize, std::collections::VecDeque<MailboxGetWaiter>>,
    /// IEEE 1800-2023 §9.3.2: a fork child's WRITES to the parent's automatic
    /// variables must be visible to the parent after the child finishes. xezim
    /// gives each child a COPY of the parent's locals (see
    /// `inherit_fork_child_context`), then `propagate_fork_locals_to_parent`
    /// merges the child's frames back. To avoid clobbering a parent variable
    /// that the child only INHERITED (never wrote) — which the parent may have
    /// child CHANGED relative to this baseline.
    fork_baselines: HashMap<usize, Vec<HashMap<String, Value>>>,
    /// IEEE 1800-2023 §6.21/§9.3.2: automatic variables declared in a
    /// procedural block (`initial`/`always`) with NO enclosing call frame live
    /// in the global `self.signals` map (not a `local_stack` frame). At fork
    /// time `inherit_fork_child_context` copies them (via `auto_loop_vars`)
    /// INTO the child's `local_stack` frame so each child gets its own
    /// snapshot. That creates a storage MISMATCH: the child's writes land in
    /// its private frame, while the parent reads from `self.signals`. The
    /// `local_stack` merge alone cannot bridge this — the parent has no frame
    /// for these vars. This map records, per child, the bare names that were
    /// captured FROM `self.signals`, so the propagate path can write the
    /// child's changed values back INTO `self.signals` (true shared storage,
    /// per §6.21's enclosing-scope rule).
    fork_signal_captures: HashMap<usize, Vec<String>>,
    /// Processes blocked in `semaphore.get()` on an under-full semaphore.
    semaphore_get_waiters: HashMap<usize, std::collections::VecDeque<SemGetWaiter>>,
    /// IEEE 1800-2017 §9.4.5 intra-assignment delay (`lhs = #d rhs`): RHS
    /// values captured at statement execution, keyed by the slot index baked
    /// into the synthesized `$__xz_intra_saved(idx)` continuation. Taken
    /// (removed) when the delayed assignment fires.
    intra_saved: HashMap<u64, Value>,
    intra_saved_next: u64,
    /// Armed values of value-change waits on class properties (see
    /// `VC_CHANGED_MARKER`); removed when the wait resumes.
    vc_saved: HashMap<u64, (usize, Value)>,
    /// LRM §15.5.3: per-named-event last-trigger time. `e.triggered`
    /// returns 1 iff its dense stamp matches the current simulation time
    /// (i.e. the event has been triggered in the same time slot as the
    /// query). EventTrigger handler stamps this.
    event_triggered_time: EventStampStore,
    /// Processes waiting for signal edge events (@(posedge clk), etc.)
    event_waiters: Vec<EventWaiter>,
    /// Processes parked on a class-field named event (`@m_event` inside a
    /// method on `this`). Woken directly by `fire_instance_event` when the
    /// matching `->m_event` runs; see `InstanceEventWaiter`.
    instance_event_waiters: Vec<InstanceEventWaiter>,
    /// Monotonic epoch bumped each time a LATE-region resume (Re-NBA waiters,
    /// reactive initials, deferred clocking continuations) runs a continuation
    /// inline. The run_one_tick tail watches it to know those resumes produced
    /// activity that may need another edge/waiter pass in the SAME time slot
    /// (a bare signal flip or `-> e` schedules nothing by itself).
    late_resume_epoch: u64,
    /// Source file of each process's module, for assertion tallies.
    pid_src_file: HashMap<usize, Option<u32>>,
    /// Covergroups waiting for sampling events
    cg_event_waiters: Vec<(usize, Vec<SensitivityId>)>,
    /// Covergroups sampled on a named event (`@(ev)`), by the event's
    /// signal id: (instance, the event term's `iff`). Sampled when the
    /// event fires; a trigger right after a process wakes on an edge was
    /// missed by the edge scan.
    cg_named_event_waiters: HashMap<usize, Vec<(usize, Option<Expression>)>>,
    /// Class-embedded covergroups sampled on an event property of their
    /// object, by (object, event name).
    cg_instance_event_waiters: HashMap<(usize, String), Vec<(usize, Option<Expression>)>>,
    /// Reused per tick: the triggered continuations of
    /// `drain_triggered_event_waiters` and the clock generators' tree
    /// roots were fresh Vecs on every tick, and `drain_deferred_comb`
    /// hands its snapshot vectors back here for `note_comb_ran_in_process`.
    triggered_conts_buf: Vec<(usize, ProcCont)>,
    /// Scheduling depth of the comb-driven signals (see
    /// `build_sig_wake_rank`): one more per continuous assignment on the way
    /// from a procedurally written signal, which is depth 0 and not stored.
    /// Orders the waiters one drain wakes (`drain_triggered_event_waiters`).
    sig_wake_rank: HashMap<usize, u8>,
    wake_rank_built: bool,
    /// Depth of the input-port names collapsed onto their actual's id
    /// (`collapse_identity_port_nets`), which the id alone cannot carry.
    name_wake_rank: HashMap<String, u8>,
    /// Scratch: wake rank of each continuation in `triggered_conts_buf`.
    wake_rank_buf: Vec<u8>,
    cg_tree_roots_buf: Vec<usize>,
    deferred_comb_pool: Vec<Vec<(usize, Value)>>,
    /// Write set of the process FSM currently executing (see `ProcFsm::writes`).
    cur_fsm_writes: Option<Arc<[u32]>>,
    /// Per comb entry: (identity of the FSM write set last checked, verdict
    /// = the FSM can clobber this entry). The same pair recurs every cycle.
    fsm_clobber_cache: Vec<(usize, bool)>,
    /// VCD dump state
    /// `--wave`: waveform dumping was requested for this run. Latched from the
    /// global at construction so the model is built consistently — an active
    /// dump changes which loops can compile (see `for_loop_needs_ast`).
    wave: bool,
    vcd_file: Option<String>,
    vcd_writer: Option<super::vcd_sink::VcdSink>,
    /// Per-traced-signal table: (signal_table index, output id code). Built
    /// once at dump start; the per-cycle loop walks just this vector instead
    /// of iterating the full id_to_name (36M entries on c910) and hashing
    /// every name. The identifier code is an `Arc<str>` so the VCD per-change
    /// clone is a refcount bump, not a heap allocation.
    vcd_trace: Vec<(usize, Arc<str>)>,
    vcd_enabled: bool,
    vcd_last_time: u64,
    /// Optional size limit for the VCD dump ($dumplimit, §21.7.1.5).
    /// Previous emitted value per entry in `vcd_trace` (parallel vector).
    /// A plain Vec<Value> — no duplicated name strings, no hashing.
    vcd_prev_signals: Vec<Value>,
    /// Optional scope/signal-name filters captured from `$dumpvars(level, sig...)`.
    /// When non-empty, only signals whose hierarchical name matches one of these
    /// scopes (or is a signal name given exactly) are emitted to the VCD. The
    /// strings are ABSOLUTE (they include the top module); `dump_filter_prefixes`
    /// rewrites them to the top-RELATIVE form the signal table uses.
    vcd_filter_scopes: Vec<String>,
    /// §21.7.1.4 `$dumpvars` depth argument: 0 = every level below the given
    /// scope, N = N levels starting at it.
    vcd_dump_depth: u32,
    /// §21.7.2.1 var type per `vcd_trace` entry (parallel vector). Drives both
    /// the `$var` declaration and the runtime record shape (an `event` emits a
    /// bare `1<id>` pulse, a `real` an `r<decimal>` record).
    vcd_var_kinds: Vec<VcdVarKind>,
    /// Last sim time at which each `vcd_trace` entry that is an `event` emitted
    /// its trigger pulse — dedups repeat triggers inside one time slot.
    vcd_event_last: Vec<u64>,
    /// Incremental (dirty-set) change detection, SHARED by every dump writer
    /// (VCD, XTrace, FST). The full scan is O(all dumped signals) per timestep;
    /// for a large dump where few signals move per step (e.g. a 1.5M-signal
    /// flattened netlist) that dominates wall time — measured at ~19x the cost
    /// of the simulation itself on c906. Instead every real signal write marks
    /// the signal dirty (a SUPERSET of actual changes — each writer's
    /// prev-value compare still guarantees only genuine changes are emitted),
    /// so a flush visits O(written signals) not O(all). Coverage: both value
    /// write choke points (the `write_sig!` macro and `after_signal_write`)
    /// call `vcd_mark!`; SVA preponed swaps deliberately do not (they restore).
    /// `dump_dirty` is the per-flush list of dirty signal ids; `dump_dirty_mark`
    /// dedups within a flush via a rolling epoch (mark==epoch ⇒ already listed).
    /// The list is consumed by every active writer in `dump_write_changes` and
    /// cleared once, after the last of them — so the writers must READ it and
    /// never clear it themselves.
    /// `vcd_id_to_trace` / `xtrace_id_to_trace` / `fst_id_to_trace` map a signal
    /// id to that writer's (possibly aliased) trace slots.
    /// `vcd_event_indices` are the event trace slots, always checked (events
    /// fire via event_triggered_time, not signal writes). Disabled — falling
    /// back to the full scan — via XEZIM_VCD_FULL=1 / XEZIM_DUMP_FULL=1 or when
    /// the maps are unbuilt.
    dump_dirty: Vec<u32>,
    dump_dirty_mark: Vec<u64>,
    dump_dirty_epoch: u64,
    vcd_id_to_trace: Vec<Vec<u32>>,
    vcd_event_indices: Vec<usize>,
    dump_dirty_active: bool,
    /// A dump writer or toggle coverage is open under a forced full scan
    /// (`XEZIM_VCD_FULL=1` / `XEZIM_DUMP_FULL=1`), so `dump_dirty_active` is
    /// off with something still to flush.
    dump_full: bool,
    /// §21.7.1.8 `$dumplimit`: byte budget for the dump, and the running
    /// (approximate) count of bytes handed to the sink. `vcd_limit_hit` latches
    /// once the budget is spent and permanently stops the dump.
    vcd_limit: Option<u64>,
    vcd_bytes: u64,
    vcd_limit_hit: bool,
    /// XEZIM_BSP_SHADOW=1: number of time>0 settles compared so far.
    bsp_shadow_checks: u64,
    /// Buffered stdout sink for $display/$write. Lazily initialized on first
    /// write; runs its own background writer unless XEZIM_DUMP_INLINE=1.
    stdout_sink: Option<super::stdout_sink::StdoutSink>,
    /// XTrace dump state. Independent from VCD — can run alongside.
    /// Set `xtrace_file` to enable. Emits the XTrace v1.0 "minimal" profile:
    /// dictionary + per-cycle signal deltas (VCD-equivalent payload). When the
    /// filename ends in `.zst`/`.zstd` the byte stream is zstd-compressed.
    pub xtrace_file: Option<String>,
    /// Optional hierarchical scope filters for XTrace. When non-empty, only
    /// signals matching one of these (exact, or `<scope>.` prefix) are dumped.
    pub xtrace_scopes: Vec<String>,
    xtrace_writer: Option<super::vcd_sink::VcdSink>,
    /// Per-traced-NET table: (signal_table index, XTrace signal id). One entry
    /// per backing net — an aliased name (§9.2 `alias=`) has its own id in the
    /// dictionary but emits no deltas of its own.
    xtrace_trace: Vec<(usize, Arc<str>)>,
    /// Signal id → `xtrace_trace` slots, for the shared incremental dirty-set
    /// change detection (see the `dump_dirty` field docs). Empty ⇒ full scan.
    xtrace_id_to_trace: Vec<Vec<u32>>,
    /// Previous emitted value per entry in `xtrace_trace` (parallel vector).
    xtrace_prev_signals: Vec<Value>,
    /// Whether entry `i` of `xtrace_trace` is a `real` (parallel vector): its
    /// value token is a decimal number, not a hex bit pattern (§15.1).
    xtrace_real: Vec<bool>,
    /// Whether entry `i` of `xtrace_trace` is a `string` (parallel vector): its
    /// value token is a §15.4 quoted, escaped string rather than a bit pattern.
    xtrace_string: Vec<bool>,
    /// The t=0 `N,full` snapshot is emitted lazily, on the first in-window
    /// `xtrace_write_changes` call — by then the time-0 initial blocks have run
    /// and settled, so the checkpoint carries real values (matching the
    /// reference producer) instead of the pre-init all-X image.
    xtrace_snapshot_pending: bool,
    /// SV `event` objects (signal_table index, XTrace signal id). An event has
    /// no level, so it is NOT in `xtrace_trace`: it emits an §10.4 `X` record
    /// per trigger instead of a value delta.
    xtrace_events: Vec<(usize, Arc<str>)>,
    /// Time of the last `X` record emitted for each `xtrace_events` entry, so a
    /// 0→1→0 toggle inside one time slot still counts as exactly one trigger
    /// (mirrors `vcd_event_last`).
    xtrace_event_last: Vec<u64>,
    xtrace_last_time: u64,
    /// Change-emitting steps since the last durable flush of the XTrace
    /// writer. Bounds how much trailing trace a crash/SIGKILL can lose
    /// (the binary builds with `panic = "abort"`, so `Drop` does not run on
    /// panic, and the memory watchdog SIGKILLs outright).
    xtrace_dirty_steps: u32,
    /// XTrace time window (#8) in nanoseconds, from `--xtrace-from`/`--xtrace-to`.
    /// `xtrace_to_ns == u64::MAX` means no upper bound. Set by the CLI.
    pub xtrace_from_ns: u64,
    pub xtrace_to_ns: u64,
    /// The same window converted to simulation ticks (tick_s units), cached at
    /// dump start. `xtrace_to_t == u64::MAX` means unbounded.
    xtrace_from_t: u64,
    xtrace_to_t: u64,
    /// FST dump state (GTKWave binary format via the `fst-writer` crate).
    /// Independent of VCD/XTrace — enabled by `--fst <file>`.
    pub fst_file: Option<String>,
    /// Optional hierarchical scope filters for FST (same semantics as
    /// `--xtrace-scope`): keep signals whose name equals or sits under a scope.
    pub fst_scopes: Vec<String>,
    /// The FST body writer (post-header phase). `None` until `fst_start_dump`.
    /// Value packing, block compression and I/O run on a background thread
    /// (`XEZIM_DUMP_INLINE=1` keeps them here); see `fst_sink::FstSink`.
    fst_writer: Option<super::fst_sink::FstSink>,
    /// Named-cast targets resolvable at COMPILE time: name -> (width,
    /// signed). Built once at construction; handed to every bytecode
    /// compiler so `type'(expr)` compiles instead of falling back.
    cast_widths: HashMap<String, (u32, bool)>,
    /// `module.functions` holds each `FunctionDeclaration` by value, so the
    /// obvious `get(name).cloned()` deep-copies a whole function AST — body,
    /// statements and all — on EVERY call. On a design that calls a function
    /// from a continuous assign that is the dominant cost: cloning the decl
    /// and the allocator traffic behind it accounted for roughly a third of
    /// the run. Hand out a refcounted handle instead; the deep copy happens
    /// once per function, then it is a refcount bump.
    fn_decl_cache: HashMap<String, std::rc::Rc<FunctionDeclaration>>,
    /// class-perf P2: one-shot Rc snapshot of `module.functions` for the
    /// method compiler (free-function calls). Built lazily on the first
    /// class-method compile; cloned per call site by Rc only.
    free_fn_table: Option<std::rc::Rc<HashMap<String, crate::ast::decl::FunctionDeclaration>>>,
    /// DPI import function prototypes by SV name, built once on the first
    /// method compile (see `dpi_import_fd_table`).
    dpi_import_fds: Option<std::rc::Rc<HashMap<String, crate::ast::decl::FunctionDeclaration>>>,
    /// Package name ("" for none) -> interned `__free_fn__<pkg>` class name.
    free_fn_class_names: HashMap<String, std::rc::Rc<str>>,
    /// Call name -> package its callee runs in (see `free_call_pkg`).
    free_call_pkgs: HashMap<String, Option<std::rc::Rc<str>>>,
    /// The same for `module.tasks`: every interpreted task call cloned the
    /// whole `TaskDeclaration`, and the clone plus its drop was about a
    /// quarter of a behavioural memory model's run (a device calling a
    /// handful of timing-check tasks every clock).
    task_decl_cache: HashMap<String, std::rc::Rc<TaskDeclaration>>,

    /// Memoized answer to "is this module-scope function side-effect free?",
    /// used to decide whether a call may be skipped when its result provably
    /// cannot affect the expression. Conservative: anything the walker does
    /// not positively recognise as safe is recorded impure.
    fn_pure_cache: HashMap<String, bool>,
    /// Lazily-built index over `module.packed_signal_elem_widths`: every
    /// dot-terminated prefix of every key, i.e. exactly the set of names for
    /// which some `name.<rest>` key exists. Formal-metadata bookkeeping runs
    /// on EVERY function/task call and used to answer that question by
    /// linear-scanning the whole map — three times per call (snapshot,
    /// clear, and clear again inside restore). The map scales with the
    /// DESIGN (packed arrays and struct members everywhere), so on a
    /// 55k-signal design a single assign with four function calls on its
    /// RHS cost ~10ms per evaluation. With the index, a formal with no
    /// dotted keys — every non-struct formal — is O(1).
    ///
    /// Invariant: MAY over-approximate (stale entries only cost a scan),
    /// must NEVER under-approximate — every insertion of a dotted key goes
    /// through `note_elem_width_key`. Built on first use so every
    /// construction path (fresh elaboration, artifact load) is covered.
    elem_dotted_bases: RefCell<Option<HashSet<String>>>,
    /// Per-net trace: (signal-table index, (FST handle, immutable encoding width)).
    /// A zero encoding width denotes a real; fixed vectors use at least one.
    fst_trace: Vec<(usize, (FstSignalId, u32))>,
    /// Signal id → `fst_trace` slots, for the shared incremental dirty-set
    /// change detection (see the `dump_dirty` field docs). Empty ⇒ full scan.
    fst_id_to_trace: Vec<Vec<u32>>,
    /// Previous emitted value per `fst_trace` entry (parallel vector).
    fst_prev_signals: Vec<Value>,
    /// SV event objects are pulses rather than levels, so they are kept out of
    /// `fst_trace` and emitted once for each trigger.
    fst_events: Vec<(usize, FstSignalId)>,
    /// Last emitted trigger time for each `fst_events` entry.
    fst_event_last: Vec<u64>,
    /// Pre-computed combinatorial entries with sensitivity sets.
    comb_entries: Vec<CombEntry>,
    /// Optional content-addressed cache for the compiled combinational
    /// worklist. The elaborated-design cache key is embedded in this path.
    prepared_comb_cache_path: Option<PathBuf>,
    /// Static topological level per comb entry (longest producer chain).
    /// Populated when the level-BSP settle path is enabled; entries within one
    /// level write disjoint signals (no read-after-write), so they can be
    /// evaluated in parallel behind a per-level barrier. Empty = not built.
    comb_level: Vec<u32>,
    /// Per comb entry: true if its compiled body contains NBA insns. The
    /// isolated settle eval (`eval_comb_entry_isolated`) collects deferred NBAs
    /// into a local and DROPS them, whereas canonical `exec_insns` schedules
    /// them into the real NBA queue. So the BSP driver must route these through
    /// the full `&mut self` path. Built alongside comb_level.
    comb_has_nba: Vec<bool>,
    /// Per comb entry: evaluable by the static isolated path (no `&mut self`,
    /// disjoint writes) — not AST, no NBA, no unsupported insn. Only par_safe
    /// entries run on worker threads in the parallel BSP level dispatch.
    comb_par_safe: Vec<bool>,
    /// Reusable per-level buckets for the BSP settle driver (one Vec per static
    /// level). Taken out/restored each settle call so capacity is retained —
    /// avoids re-allocating ~num_levels Vecs on every call. Inner Vecs are
    /// cleared as each level is drained.
    bsp_buckets: Vec<Vec<usize>>,
    /// Reusable "re-trigger for next sweep" list for the BSP driver.
    bsp_next_seed: Vec<usize>,
    /// XEZIM_BSP_WIDTHS=1 diagnostic: histogram of per-level evaluated-entry
    /// counts during time>0 (steady-state) settles, bucketed by width
    /// [1, 2-15, 16-63, 64-255, 256-1023, 1024+]. Decides whether intra-level
    /// parallelism has enough width to amortize dispatch in steady state.
    bsp_width_buckets: [u64; 6],
    bsp_widths_on: bool,
    /// XEZIM_BSP_SETTLE=1: route combinational settle through the level-BSP
    /// wavefront driver instead of the canonical chaotic-iteration worklist.
    bsp_settle: bool,
    /// XEZIM_BSP_PAR=1: evaluate each level's par_safe entries across worker
    /// threads (intra-level parallelism). Requires bsp_settle.
    bsp_par: bool,
    /// Minimum par_safe entries in a level before the parallel dispatch fires
    /// (smaller levels run serial — dispatch wouldn't amortize). XEZIM_BSP_PAR_THRESHOLD.
    bsp_par_threshold: usize,
    /// XEZIM_BSP_SHADOW=1: per-settle compare BSP vs canonical, abort on diff.
    bsp_shadow: bool,
    /// Precomputed indices of comb_entries with has_unresolved_reads=true.
    /// Built once alongside comb_entries; scanning this short list every
    /// settle call is vastly cheaper than iterating all num_entries bits
    /// (can be ~500K on designs like c910).
    comb_unresolved_idx: Vec<usize>,
    /// Co-activation census (`XEZIM_COACT_CENSUS=1`): per-entry predecessor
    /// CSR over the dep graph, a same-slot counter per (entry, pred) edge,
    /// and the last simulated time each entry evaluated. Sizing data for
    /// convex-cluster fusion: an entry whose evals are >=90% same-slot with
    /// one predecessor can join that predecessor's cluster with bounded
    /// waste — even across fanout. Empty unless the env is set.
    coact_pred_off: Vec<u32>,
    coact_pred_ent: Vec<u32>,
    coact_edge_count: Vec<u64>,
    coact_epoch: Vec<u64>,
    coact_enabled: bool,
    /// Precomputed indices of comb_entries that should fire unconditionally
    /// at time=0: entries with empty read_signal_ids, or always_comb blocks.
    /// Avoids an O(num_entries) scan on every settle call while time=0.
    comb_time0_idx: Vec<usize>,
    /// Latches once `comb_time0_idx` has been seeded into settle_triggered.
    /// Every subsequent settle call at time=0 can skip the re-seed; the
    /// worklist and dirty propagation already keep things moving.
    comb_time0_fired: bool,
    /// §9.2.2.2: always_comb blocks with an EMPTY inferred sensitivity list
    /// (no inputs, e.g. `always_comb y = 1'b0;`). The LRM says these trigger
    /// exactly once at time zero, AFTER all initial/always procedures have
    /// STARTED (run to their first delay). Firing them in the pre-initial
    /// settle would let an initial block read the output as already-settled;
    /// a reference simulator (and the standard) has that same-time pre-delay read return X.
    /// So we hold these out of the normal time-0 settle and fire them once at
    /// the END of the time-0 active region (see run_one_tick). Cloned entries,
    /// evaluated directly.
    comb_time0_deferred: Vec<CombEntry>,
    /// Latches once the deferred empty-sensitivity always_comb blocks fired.
    comb_time0_deferred_done: bool,
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
    /// Lazily-lowered two-state bodies for comb entries (P5): indexed by
    /// entry, `Untried` until first evaluation. Cleared whenever the entry
    /// list is rebuilt.
    ts_comb: Vec<TsSlot>,
    /// Per-comb-entry settle dispatch plan, resolved on first evaluation.
    /// Collapses the per-eval chain (env OnceLock, TsSlot bounds+match,
    /// comb_jit_fns bounds+copy, strike bookkeeping) — measured 15% of a
    /// JIT'd ibex run as settle-loop self time — into one enum load.
    /// `ts_comb`/`comb_jit_fns` stay the sources of truth at resolution;
    /// a JIT strike-out downgrades the plan in place.
    comb_plan: Vec<CombPlan>,
    /// Two-state code arena: every lowered comb stream appended contiguously
    /// (in first-evaluation order, which tracks the worklist order), plus a
    /// dense per-entry header. `kind == 0` means "no arena stream".
    ts_arena: Vec<super::bytecode::TsInsn>,
    ts_hdr: Vec<TsHdr>,
    /// Same, for edge blocks (indexed like `compiled_edge_blocks`).
    ts_edge: Vec<TsSlot>,
    /// Two-state scratch register file (u64 words).
    ts_regs: Vec<u64>,
    /// x/z planes of `ts_regs` for the x-plane executor.
    ts_xregs: Vec<u64>,
    /// Per arena header: the stream is x-plane runnable (`TwoStateBlock::tsx`).
    ts_hdr_tsx: Vec<u8>,
    /// x-plane executor enabled (`XEZIM_TS_X=0` disables).
    tsx_on: bool,
    prof_tsx_evals: u64,
    /// Wide (65..=128-bit) two-state register bank; same index space as
    /// `ts_regs` (a register's bank is static, decided at lower time).
    /// Wide two-state banks, flat words viewed as `[u64; N]` registers:
    /// N = 2 (widths ≤ 128) and N = 8 (≤ 512).
    ts_wregs: Vec<u64>,
    ts_wregs8: Vec<u64>,
    /// Two-state eval hits, for the profile report.
    prof_ts_evals: u64,
    /// Why the two-state engine declined an eval — reported so a profile
    /// from a real design says WHY islands are inert instead of just "0".
    prof_ts_bail_warnx: u64,
    prof_ts_bail_forced: u64,
    prof_ts_bail_xread: u64,
    prof_ts_bail_abort: u64,
    /// Set by `ts_guard_and_exec`: the last `false` came from a MID-EXEC
    /// abort (partial ts work + full 4-state replay), not a cheap guard
    /// bail. Callers use it to DEMOTE chronically aborting blocks — the TS
    /// NBA admission exposed blocks whose guards pass but whose bodies hit
    /// an abortable op (blocking reads of X RAM data) on most evals: 67 M
    /// aborts/run, each paying guard + partial exec + replay, a net LOSS vs
    /// never admitting. Demotion returns such a block to exactly its
    /// pre-admission engine; clean runners keep the fast path.
    ts_exec_aborted: bool,
    /// Set by a two-state executor that read an x/z bit; the guard reports
    /// it as an x-read bail (no demotion), not as an abort.
    ts_xread_bail: bool,
    /// `TsInsn::SaveSig` records (signal, value bits, x bits) for the
    /// evaluation in progress; restored on a bail, truncated on success.
    ts_save_list: Vec<(u32, u64, u64)>,
    /// Comb entry whose two-state stream is executing (u32::MAX for an
    /// edge block, whose runner sets the scope itself): a `TsInsn::Fallback`
    /// sets the `%m` buffer and the resolve hint from its scope, as the
    /// four-state comb path does before evaluating an entry.
    ts_cur_eidx: u32,
    /// The settle takes `comb_entries` out of `self` for its duration; this
    /// is a view of the taken vector so `ts_fallback_scope` can still read
    /// an entry's scope hint. Null outside the settle.
    settle_entries_view: (*const CombEntry, usize),
    /// Census: x-read bails per comb entry (count, executor kind).
    #[cfg(feature = "opcode-census")]
    ts_xbail_by_entry: HashMap<usize, (u64, u8)>,
    /// Per-slot mid-exec abort counts; at `TS_ABORT_DEMOTE` the slot flips
    /// to No / Interp for the rest of the run. (Re-promotion after init
    /// settles is future work.)
    ts_edge_abortn: Vec<u16>,
    /// x-read bail backoff. A block that bails on an x read every time (an
    /// unwritten memory element it keeps reading) paid the executor entry
    /// and the guard on every evaluation before running interpreted anyway.
    /// After `TS_XBAIL_STREAK` bails in a row it is put to SLEEP: its plan
    /// or slot is swapped for the demoted form, so the hot path pays
    /// nothing for it, and a record on this heap (keyed by the successful
    /// two-state evaluation count) wakes it later, sleeping twice as long
    /// each time the bails continue. Per-entry streak state is touched only
    /// on a bail; a long clean stretch resets it.
    ts_sleep: std::collections::BinaryHeap<std::cmp::Reverse<(u64, u32)>>,
    ts_sleep_recs: Vec<Option<TsSleepRec>>,
    ts_sleep_free: Vec<u32>,
    /// `(streak, last bail at, probe until)` per comb entry / edge block.
    /// `probe until` is set at wake-up to one evaluation cadence of the
    /// block (measured while it slept): a bail before it means the first
    /// attempt after waking still read x, and the streak escalates; a bail
    /// after it means the block ran clean in between, and the streak
    /// restarts.
    ts_comb_xb: Vec<(u8, u64, u64)>,
    ts_edge_xb: Vec<(u8, u64, u64)>,
    /// Interpreted evaluations of a SLEEPING block, plus one (0 = awake):
    /// counted on the interpreter paths, which a sleeping block takes.
    ts_asleep_comb: Vec<u32>,
    ts_asleep_edge: Vec<u32>,
    /// Set by `ts_guard_and_exec` when the failure was an x read.
    ts_last_xbail: bool,
    ts_comb_abortn: Vec<u16>,
    comb_plan_abortn: Vec<u16>,
    /// XEZIM_PROFILE_REPORT=1: reference-style ranked profile (by design
    /// unit / instance / construct). Implies per-entry and per-edge-block
    /// nanosecond attribution.
    profile_report: bool,
    /// XEZIM_PROC_FSM: 1 = every compilable initial body is a process FSM,
    /// 0 = none, unset (2) = the plain-stimulus shape only. Read per
    /// simulator so a test can opt out before it builds one.
    proc_fsm_mode: u8,
    comb_dep_entries: Vec<u32>,
    /// `comb_dep_entries` with bit-granular sensitivity, for the settle
    /// loop: each dependent with the bits of the signal it reads. A write
    /// inside the settle loop whose changed bits miss the mask does not
    /// trigger that dependent — `assign lo = bus[3:0]` stays idle while
    /// `bus[7:4]` moves. Empty until the first settle; entries narrow as
    /// they lower two-state.
    comb_dep_edges: Vec<DepEdge>,
    /// Entries lowered since the masks were last refined.
    bit_sens_pending: Vec<u32>,
    /// XEZIM_BIT_SENS=0 turns the masks off (every dependent triggers).
    bit_sens_on: bool,
    /// Controlling-value skip (`CtlGate`): the gates, indexed by the
    /// `CombItem::CtlGate` slot. Rebuilt with the dependency edges.
    ctl_gates: Vec<CtlGate>,
    /// Per signal: some dependency edge from it may be masked by a
    /// `CtlGate`, so the settle's dirty seed must test the edge masks.
    /// Empty when no gate qualifies.
    ctl_sig: Vec<bool>,
    /// `(destination signal, slot)` of every `CtlGate`, sorted: an external
    /// write to a destination (a deposit) must re-arm its gate.
    ctl_dsts: Vec<(u32, u32)>,
    /// Slots re-armed while the settle held the dependency edges.
    ctl_rearm_pending: Vec<u32>,
    /// XEZIM_CTL_MASK=0, or a settle mode that evaluates entries outside the
    /// canonical loop (partitioned / level-BSP), turns the skip off.
    ctl_on: bool,
    /// Dirty records of the stores a two-state entry makes in the settle
    /// loop (direct mode), with each store's changed bits (`u64::MAX` when
    /// unknown): `ts_rec_n` records, the first in `ts_rec0` (most entries
    /// store one signal), the rest in `ts_recs`. Kept apart from
    /// `dirty_list` so every record carries its own mask; the settle loop
    /// consumes them right after the entry.
    ts_rec0: (usize, u64),
    ts_rec_n: usize,
    ts_recs: Vec<(usize, u64)>,
    /// Bitvec: dirty_signals[signal_id] = true if signal changed since last settle.
    dirty_signals: Vec<bool>,
    /// Explicit list of dirty signal IDs (maintained alongside dirty_signals bitvec)
    /// This avoids O(num_signals) scan in settle_combinatorial.
    dirty_list: Vec<usize>,
    dirty_any: bool,
    /// Set while the settle loop runs a two-state entry: its stores append
    /// to `dirty_list` directly, without the `dirty_signals` dedup flag (a
    /// random access into a 35M-entry table per store on c906). The loop
    /// re-reads and clears that span right after the entry, and
    /// `trigger_deps!` dedups by entry, so duplicates are harmless.
    ts_direct_writes: bool,
    /// When true, signal_table has been modified and signals HashMap is stale.
    table_modified: bool,
    /// Scratch buffer reused across event_loop iters: positions (into
    /// `edge_signal_ids`) of clock signals that toggled during the current
    /// iter's `fire_clock_generators`. When the iter ends up doing no other
    /// work (no NBA apply, no settle, no process, no delayed update), the
    /// edge-detect scan can be restricted to just these positions — cuts
    /// per-iter edge_detect work from O(|edge_signal_ids|) ~10k to ~|clocks|.
    /// See `event_loop` for the dispatch.
    toggled_clock_positions: Vec<usize>,
    /// XEZIM_EDGE_SCAN_STATS=1 diagnostic: edge-detect positions scanned vs
    /// positions that actually changed (fired). Measures the dirty-driven
    /// detect ceiling before building it.
    edge_scan_scanned: u64,
    edge_scan_changed: u64,
    edge_scan_stats: bool,
    /// XEZIM_ARRAY_SOA_SHADOW=1 (with XEZIM_INLINE_BITS=1): redirect array-cell
    /// READS to the compact signal_inline_bits store and assert they match the
    /// canonical signal_table — Stage-0 validation toward dropping the 32-byte
    /// Value array cells. signal_table stays canonical (safe).
    soa_shadow: bool,
    /// Per-signal eligibility for the SoA read fast path: width ≤ 64, not
    /// real, not string. Built alongside `signal_inline_bits` (empty ⇒ the
    /// path is off and every read falls back to `signal_table`).
    soa_read_ok: Vec<bool>,
    /// XEZIM_OPCODE_CENSUS=1: dynamic opcode + adjacent-pair execution counts
    /// for fusion planning. `census_counts[op]`, `census_pairs[prev*64+op]`.
    census_enabled: bool,
    census_counts: Vec<u64>,
    census_pairs: Vec<u64>,
    /// Two-state executor census (opcode-census builds only): adjacent
    /// opcode pairs and singles by variant name.
    #[cfg(feature = "opcode-census")]
    ts_census: std::collections::HashMap<(&'static str, &'static str), u64>,
    /// Clock-tree dedup (see `build_edge_sig_groups`). `edge_group_of` is
    /// parallel to `edge_signal_ids`: the group a POSITION belongs to, or
    /// `u32::MAX` for a signal that must be checked on its own. Members of one
    /// group are exact copies of a common driver, so the first member scanned in
    /// a pass computes the edge and the rest read it out of `edge_group_memo`.
    /// The memo is stamped with `edge_group_generation` (bumped once per detect
    /// pass) so it never needs clearing.
    edge_group_of: Vec<u32>,
    edge_group_memo: Vec<EdgeGroupMemo>,
    edge_group_generation: u64,
    edge_group_count: usize,
    /// `XEZIM_CLKTREE_PROBE=1`: instead of trusting the memo, recompute every
    /// member and count how often the leader's answer disagreed. The dedup is
    /// only sound if this is zero, and it is cheaper to measure than to argue.
    clktree_probe: bool,
    clktree_probe_hits: u64,
    clktree_probe_mismatch: u64,
    /// Signal ids of the first few probe mismatches, resolved to names in the
    /// summary (the detect loop holds a mutable borrow and cannot name them).
    clktree_probe_sids: Vec<usize>,
    /// Dirty-driven edge detect (XEZIM_DIRTY_EDGE=1). `sig_to_edge_pos[id]` =
    /// position of signal `id` in `edge_signal_ids`, or -1 if not edge-sensitive.
    /// Writes to edge-sensitive signals (via write_sig!/after_signal_write)
    /// push their position into `changed_edge_pos` (deduped by `edge_pos_seen`),
    /// so check_edges can scan only changed positions instead of all of them.
    sig_to_edge_pos: Vec<i32>,
    changed_edge_pos: Vec<usize>,
    /// Retained buffer that becomes `changed_edge_pos` while a scan runs, so
    /// positions written during the scan do not grow a fresh Vec every pass.
    edge_pos_scratch: Vec<usize>,
    edge_pos_seen: Vec<bool>,
    dirty_edge: bool,
    dirty_edge_shadow: bool,
    /// Edge positions written by a BLOCKING assign while an edge-triggered
    /// block was executing (`in_edge_block`), deduped via `edge_exec_seen`.
    /// §9.2: such a change must (re-)trigger blocks sensitive to it —
    /// `drain_edge_exec_rescan` re-detects exactly these positions in a
    /// follow-up delta pass instead of silently coalescing them away.
    edge_exec_wrote: Vec<usize>,
    edge_exec_seen: Vec<bool>,
    /// Nesting depth of a scheduled PROCESS activation. §4.5: a blocking write
    /// schedules the blocks sensitive to it in the ACTIVE region; the writing
    /// process keeps running until it suspends, and only then do they execute.
    /// The blocking-assign arm used to settle INLINE, so an `always @(*)` ran
    /// in the middle of another process's statement sequence — and if that
    /// process later wrote the comb block's target, the comb result was
    /// clobbered and never recomputed (`initial begin a=0; b=1; g=0; end`
    /// against `always @(*) if (a^b) g=…` left g at 0 instead of the value).
    /// While this is non-zero, procedural comb entries triggered mid-process
    /// are parked in `deferred_comb` and run at the process's exit instead.
    proc_depth: u32,
    /// (comb entry index, outputs it produced) for procedural entries that ran
    /// while a process was mid-flight. Checked for clobbering at process exit.
    deferred_comb: Vec<(usize, Vec<(usize, Value)>)>,
    /// True while `drain_edge_exec_rescan` is re-running check_edges for
    /// exec-written positions; makes check_edges_inner count block
    /// executions into `edge_rescan_block_hits` for stall attribution.
    in_edge_rescan: bool,
    /// block index -> executions during the current rescan drain. Read by
    /// `report_edge_rescan_stall` to name the ping-ponging always blocks.
    edge_rescan_block_hits: HashMap<usize, u64>,
    /// Edge-block executions accumulated while simulated time is not
    /// advancing. Unlike `edge_rescan_block_hits`, this spans outer scheduler
    /// iterations so progress/recovery diagnostics can identify a delta-cycle
    /// loop whose writes are separated by NBA or process scheduling phases.
    stall_edge_block_hits: HashMap<usize, u64>,
    /// Counter — number of iters whose check_edges ran over just the
    /// toggled clock subset instead of the full edge_signal_ids scan.
    prof_clocks_only_detect: u64,
    /// Lookup parallel to `signal_table`: true iff this signal id is in
    /// `edge_signal_ids` AND is NOT a clock generator's output.  Used by the
    /// clocks-only fast-path to decide whether an NBA write actually moved
    /// a signal whose change could be missed if check_edges scans only the
    /// toggled clocks.  Built once at the end of elaboration.
    is_edge_signal_non_clock: Vec<bool>,
    /// Per-iter scratch flag set by `write_sig!` whenever a write targets a
    /// non-clock edge signal — regardless of whether the value actually
    /// changes (most callers gate write_sig! on a prior equality check, so
    /// no-op writes don't reach the macro).  Also set in the parallel
    /// apply_nba path (which bypasses the macro).  Cleared at the top of
    /// each `event_loop` iter; read just before check_edges to decide
    /// between the full scan and the clocks-only fast path.
    nba_touched_edge_non_clock: bool,
    /// Counter — number of `Insn::NbaAssign` evaluations whose value
    /// already matched `signal_table[sig_id]` and were therefore not
    /// pushed to `nba_fast`.  Captures the no-op fraction of NBA writes
    /// (~70% on c910 flop-heavy workloads) before they incur queue/apply
    /// overhead.  Counted in the main interpreter path only; the
    /// parallel `exec_insns_isolated` path also elides but doesn't bump.
    prof_nba_elided: u64,
    // O1 event-driven-edge MEASUREMENT (XEZIM_EVENT_EDGE_MEASURE=1, no
    // behavior change): per-edge-block static DATA-read set (LoadSignal sids
    // minus sensitivity), whether the block is gateable (no dynamic reads),
    // a "changed since last main-clock posedge" accumulator, and counters for
    // how many main-clock flops WOULD be skipped (no data input changed).
    event_measure: bool,
    // event_skip: the REAL skip (XEZIM_EVENT_EDGE=1) — actually drop gateable
    // flop fires whose data didn't change. event_measure just counts.
    event_skip: bool,
    // Skip only at sim_time >= this (XEZIM_EVENT_AFTER). Boot/load uses write
    // paths ($readmemh, testbench drives, DPI) that may bypass the change
    // hook; running boot WITHOUT skip avoids that false-skip window. The 99%
    // waste is in steady state.
    event_after: u64,
    edge_block_data_reads: Vec<Vec<u32>>,
    edge_block_gateable: Vec<bool>,
    /// Gateable only through write-arming: the block reads/writes a dense
    /// unpacked array with a dynamic index, and every element of that array
    /// is registered as an arm-only input (no value snapshot). Once armed it
    /// always executes; the snapshot compare cannot see element changes.
    edge_block_arm_only: Vec<bool>,
    /// Gateable block with a >64-bit data read: no compact value snapshot
    /// exists, so under write-arming an UNARMED block skips on arming alone
    /// (no input write since its last execution ⇒ no change).
    edge_block_wide_read: Vec<bool>,
    /// XEZIM_ARM_CENSUS=1: per-signal (writes, block-arms) through note_armed_write.
    arm_census: Option<Vec<(u64, u64)>>,
    // Timestamp-based change tracking (gating-agnostic): per-signal iteration
    // at which it last changed, and per-edge-block iteration at which it last
    // fired. A flop is skippable iff none of its data inputs changed since the
    // flop's previous fire (max data last-change <= flop last-fire).
    sig_last_change: Vec<u64>,
    flop_last_fire: Vec<u64>,
    // Snapshot-compare skip (correct-by-construction, no change-hook dependency):
    // per gateable edge-block, the data-input signal VALUES captured at its last
    // snapshot-verified fire. Epoch-fast executions invalidate this snapshot;
    // one unconditional execution rebuilds it before skipping is possible. On
    // the next verified edge we compare CURRENT values directly and skip iff
    // bit-identical (Q provably unchanged). Reads ground truth, so it cannot
    // false-skip regardless of which write paths exist (unlike a hook-only
    // timestamp scheme, which silently corrupts if any write bypasses the hook).
    // Inline (v,x) raw_bits snapshots: gateable blocks restrict to <=64-bit
    // reads (see build_event_measure_state), so two u64 compares replace a
    // full Value::eq — orders of magnitude cheaper at 555M compares/run.
    // Flat CSR layout (one allocation each) keeps the per-fire compare in
    // contiguous memory: edge_block_off[bi]..edge_block_off[bi+1] indexes
    // both reads_flat and snap_flat for block bi.
    edge_block_reads_flat: Vec<u32>,
    /// Parallel to `edge_block_reads_flat`: the (lo, width) BIT SLICE of the
    /// signal that the block actually reads. A full read of a narrow signal
    /// is (0, width). A constant bit/part select of a WIDE bus records just
    /// the selected slice, which is what lets a 1-bit synchronizer tap of a
    /// 1000+-bit bus stay gateable (whole-signal raw_bits would alias).
    edge_block_reads_meta: Vec<(u16, u16)>,
    edge_block_snap_flat: Vec<(u64, u64)>,
    edge_block_off: Vec<u32>,
    edge_block_snap_valid: Vec<bool>,
    // Blocks whose inputs change on nearly every fire temporarily use epochs
    // to bypass the redundant snapshot compare. Epochs may only force an
    // execution; every skip is still verified against the value snapshot.
    edge_block_change_streak: Vec<u8>,
    edge_block_epoch_probe_left: Vec<u8>,
    /// Self-heal for the flop-fire skip: consecutive SKIPPED clock edges per
    /// gateable block. After XEZIM_EVENT_EDGE_HEAL consecutive skips (default
    /// 16, 0 disables) the block is force-executed once and its snapshot
    /// rebuilt. For a correctly-idle flop the forced execution changes nothing
    /// (inputs unchanged ⇒ same outputs, NBA elided), so this is semantically
    /// invisible — but it BOUNDS the damage of any skip-bookkeeping wedge
    /// (e.g. a divider flop frozen for a dozen cycles while its clock toggles,
    /// as seen on a customer PLL model) to at most HEAL clock edges.
    edge_block_skip_streak: Vec<u16>,
    /// Heal threshold (consecutive skipped edges before a forced execution).
    event_heal: u16,
    /// Count of forced heal executions — nonzero means a skip streak hit the
    /// threshold; a LARGE count with wrong waveforms points at a skip bug.
    event_healed: u64,
    /// ARMED event-edge prefilter. Writes first test a compact signal bitmap;
    /// only data-input hits walk the sparse signal -> gateable-block fanout.
    /// An unarmed block can skip without reading its operand snapshots.
    armed_edge: bool,
    armed_edge_shadow: bool,
    /// XEZIM_EDGE_FIRE_TRACE=<substr>: log every edge-block fire whose
    /// instance scope or sensitivity-signal name contains <substr> — with
    /// the SCHEDULER WINDOW it fired from (active / cascade / clocking-mirror
    /// / rescan) and its gateable data-input values at fire time. Built to
    /// attribute one-cycle-early flop captures (a fire in any window other
    /// than `active` samples post-NBA state).
    edge_fire_trace: Option<String>,
    /// Lazily-built per-block match bitmap for `edge_fire_trace` — the
    /// string scan runs ONCE per block, not once per fire (the naive scan
    /// measurably slowed a customer run with thousands of fires per tick).
    edge_fire_match: Option<Vec<bool>>,
    /// XEZIM_EDGE_FIRE_WATCH=<name1,name2>: signals whose CURRENT values
    /// are appended to every traced fire line. Resolved lazily by substring
    /// match against the signal table (first match per pattern).
    edge_fire_watch: Vec<(String, usize)>,
    edge_fire_watch_init: bool,
    /// Label of the dispatch window the current check_edges belongs to.
    edge_dispatch_phase: &'static str,
    /// XEZIM_VALUE_TRACE=<substr>[,<substr>...] — print every committed
    /// change of any signal whose hierarchical name contains a pattern:
    /// time, name, old→new value, dispatch phase, and the WRITING process's
    /// origin (file:line). The tool for "where does the data stop flowing
    /// and who wrote it" on a design we cannot see. The write macro runs
    /// under disjoint-field borrow constraints, so hits are queued in a
    /// RefCell (same shape as warn_x_pending) and formatted at the drain.
    /// None = disabled, Some(ids) = resolved watch set.
    value_trace_ids: Option<std::collections::HashSet<usize>>,
    value_trace_pending: std::cell::RefCell<Vec<(usize, u64, Value, Value, usize, &'static str)>>,
    value_trace_emitted: u64,
    value_trace_limit: u64,
    /// One bit per signal id (u64 words): is it a data input of a gateable
    /// edge block? A bitset keeps the whole table cache-resident where one
    /// byte per signal (1.4 MB with array elements) did not.
    armed_input_bitmap: Vec<u64>,
    armed_input_ranges: Vec<(u32, u32)>,
    armed_input_blocks: Vec<u32>,
    edge_block_armed: Vec<u8>,
    /// Edge blocks whose work was folded into a merged block: their
    /// sensitivities are cleared, so they never fire again. The skip census
    /// must ignore them wholesale — counting their ORIGINAL statement's
    /// writes made every merged target look multiply driven, and a
    /// multiply-driven output disqualifies the block that absorbed them.
    edge_block_retired: Vec<bool>,
    armed_fast_skips: u64,
    armed_shadow_checks: u64,
    // Phase-granular monotonic counter: bumped at each check_edges entry AND
    // each settle entry, so a flop's SAMPLE phase (check_edges) and the
    // comb-SETTLE phase that produces its next data have distinct timestamps
    // (loop_iters is per-tick — too coarse to separate them).
    event_phase: u64,
    event_would_skip: u64,
    event_gateable_total: u64,
    event_epoch_fast_exec: u64,
    event_snapshot_checks: u64,
    settle_calls: u64,
    /// Settle census: entry evaluations that actually changed a signal
    /// (i.e. propagated), and dependency edges walked doing so. The ratio
    /// against `entry_evals` says whether the lever is fewer triggers or
    /// cheaper dispatch.
    prof_settle_writes: u64,
    prof_settle_dep_edges: u64,
    /// Dynamic executions of the packed blocking-fill collapse. Reported with
    /// profiling so regression tests can prove the fast path was exercised.
    packed_blocking_fill_collapses: u64,
    // Profiling accumulators (nanoseconds)
    prof_edge_detect: u64,
    prof_edge_exec: u64,
    /// Phase-1 multicore observability: counts how many delta-cycles
    /// the parallel dispatcher took the partition-aware path vs the
    /// legacy chunk-by-range path. Both are bumped on every parallel
    /// dispatch; sum tells you total dispatcher cycles.
    prof_par_dispatch_partition: u64,
    prof_par_dispatch_legacy: u64,
    prof_edge_waiters: u64,
    prof_edge_cg: u64,
    prof_waiter_iters: u64,
    prof_edges_fired: u64,
    prof_insns_executed: u64,
    /// Auto-calibration of the parallel edge-dispatch path (see the gate in
    /// `check_edges_inner`). The old gate assumed threading always pays once a
    /// tick carries >=10k insns, but whether it pays at all depends on the
    /// machine (thread handoff + NBA merge cost vs core count) and on the
    /// block SHAPE (many tiny blocks amortize nothing). Rather than guess a
    /// constant, measure: run the first qualifying ticks each way and keep
    /// whichever is faster. 0 = still calibrating sequentially, 1 = calibrating
    /// in parallel, 2 = locked.
    par_cal_phase: u8,
    par_cal_ticks: u32,
    par_cal_seq_ns: u128,
    par_cal_seq_insns: u64,
    par_cal_par_ns: u128,
    par_cal_par_insns: u64,
    par_cal_use_parallel: bool,
    /// Ticks dispatched through the parallel edge path, and blocks so
    /// dispatched. Reported in `[PROF]` because those blocks' instructions are
    /// executed on worker threads and are NOT included in `insns=`.
    prof_par_ticks: u64,
    prof_par_blocks: u64,
    /// Zero-delay (delta) livelock detection: how many event-loop iterations
    /// have run at the CURRENT simulation time without it advancing. A design
    /// whose processes keep re-arming at the same timestamp (`forever #0;`, a
    /// zero-delay event ping-pong, a `wait` on an already-true condition inside
    /// a `forever`) can never advance, and xezim used to spin silently forever
    /// — the user just saw the sim "stuck at time 0".
    /// Re-entry depth of `exec_forever_sched`'s suspend-aware path. A `forever`
    /// whose leading statement only SOMETIMES blocks (`wait (cond)` with cond
    /// already true) would otherwise recurse once per iteration and overflow the
    /// Rust stack; past this depth the continuation goes back to the scheduler.
    forever_depth: u32,
    /// Continuation frames built by `exec_forever_sched`, keyed by the
    /// process, the loop body's source span, the index of the statement it
    /// parked on, and the site kind. The span identifies the LOOP: the
    /// `ForeverTail` inside each continuation carries a clone of the body,
    /// so an address key would change on every iteration (the first cut of
    /// this cache missed on every wake-up and grew without bound). The
    /// process id keeps two inlined instances of the same source loop —
    /// same span, different rewritten names — apart. Before this cache
    /// every wake-up of a `forever @(posedge clk)` / `always` process
    /// deep-cloned its statement list and its whole body into a fresh
    /// frame: 5 % of the c906 memcpy steady state was that cloning.
    /// Resolved sensitivity ids of a `forever` loop's wait, keyed by (pid,
    /// wait-statement span start): the loop re-parks on the same wait every
    /// iteration, and resolving it meant rebuilding signal-name strings and
    /// hashing them per wake-up (a clocked testbench monitor routed to the
    /// process path did this every cycle).
    forever_sens_cache: HashMap<(usize, u32), (Vec<SensitivityId>, bool)>,
    forever_cont_cache: HashMap<(usize, usize, usize, usize, u8), Arc<[Statement]>>,
    /// Pending-injection bitset for `settle_combinatorial_inner` (kept to avoid reallocation).
    settle_inject_bits: Vec<u64>,
    /// Summary of `settle_inject_bits` (one bit per word); sized from the
    /// entry count like the bitset itself. A fixed 32-word array here
    /// overflowed on c910 (218k entries).
    settle_inject_sum: Vec<u64>,
    /// A `return <virtual interface>` left `__vif_return__` for the next assignment.
    vif_return_pending: bool,
    /// Static lvalue widths of bare-leaf identifiers, per identifier span (see infer_lhs_width).
    lhs_leaf_width_cache: HashMap<(usize, usize), u32>,
    /// See `blocking_cont_frame`. Keyed by `(node_addr, kind)` and retains
    /// the owning `Arc<[Statement]>` alongside the prepared continuation frame
    /// so the AST node address stays valid and cannot be recycled, while all
    /// processes (`pid`s) executing the same shared AST node reuse one frame.
    #[allow(clippy::type_complexity)]
    blocking_cont_cache: HashMap<(usize, u8), (Arc<[Statement]>, Arc<[Statement]>)>,
    /// See `class_task_body_frame`.
    #[allow(clippy::type_complexity)]
    class_task_frame_cache:
        HashMap<(usize, usize, usize), (Arc<crate::ast::decl::ClassMethod>, Arc<[Statement]>)>,
    /// §11.8.1: while narrowing relational constraint bounds for a target whose
    /// comparison context is UNSIGNED (the target, or the other operand, is
    /// unsigned), a bound literal must be read by its unsigned value — e.g. the
    /// unsized decimal `4294967290` (0xFFFFFFFA), which `to_i64` would otherwise
    /// sign-extend to -6, collapsing `u >= 4294967290` to `u >= 0`. Scoped to
    /// the `narrow_relational_bounds` / `affine_*` subtree.
    constraint_cmp_unsigned: bool,
    stall_iters: u64,
    stall_time: u64,
    /// Process activations at the current timestamp — the accurate signal for
    /// "who is spinning" (the event queue's own pid_counts is reset per drain).
    stall_pid_hits: HashMap<usize, u64>,
    /// Delta cycles allowed at one timestamp before the run is declared a
    /// zero-delay livelock. Generous by default (real designs settle in a
    /// handful); tune with XEZIM_STALL_LIMIT, 0 disables the check.
    stall_limit: u64,
    /// Set by the per-pid stall check inside `run_process_stmts` when a single
    /// process spins past `stall_limit` at one timestamp (e.g. an
    /// `always #(period) clk=~clk` whose real period glitched to 0). Instead of
    /// aborting there, it flags the outer loop to attempt a defer-and-advance
    /// to the next scheduled event (commercial-sim parity) — a #0 spinner whose
    /// driving value recovers once time moves resumes; only a livelock with
    /// nothing scheduled ahead is fatal. See `try_break_zero_delay_livelock`.
    zero_delay_defer_pending: bool,
    prof_fallback_insns: u64,
    /// Fine-grained phase timing calls `Instant::now` several times per
    /// simulation tick. Keep it opt-in so normal runs do not pay that cost.
    profile_timing: bool,
    /// XEZIM_COMB_PATHS=1: per-comb-entry settle-path counters
    /// [two_state, jit, interpreter], dumped ranked at end of sim.
    trace_comb_paths: bool,
    /// Per entry: [two-state execs, jit execs, interp execs, interp insns].
    comb_path_counts: Vec<[u64; 4]>,
    /// Last interp-path entry noted and the VM insn counter at that point
    /// (the delta at the next note is charged to it).
    comb_path_last: Option<(usize, u64)>,
    /// XEZIM_COMB_PATHS: per edge block [interp fires, interp VM insns].
    edge_path_counts: Vec<[u64; 2]>,
    /// XEZIM_COMB_PATHS: per comb entry [evals, evals repeated within one
    /// time step], plus the last evaluation time.
    comb_eval_counts: Vec<[u64; 2]>,
    comb_eval_last_time: Vec<u64>,
    prof_fallback_by_reason: HashMap<Arc<str>, (u64, u64)>,
    prof_settle_dc_ns: u64,
    prof_settle_ca_ns: u64,
    prof_settle_ab_ns: u64,
    prof_settle_dc_count: u64,
    /// Entry evaluations made in a settle pass after the first.
    prof_settle_repass_evals: u64,
    /// Per-CombItem-variant evaluation histogram (XEZIM_ENTRY_HIST=1).
    prof_entry_hist: [u64; 12],
    /// Per-entry eval counts (XEZIM_PROFILE_TIMING) for shape attribution.
    prof_entry_counts: Vec<u64>,
    /// `--profile` sampler thread (see `prof_sampler`); `prof_cur` is the
    /// slot the simulation thread publishes its current construct into.
    prof_sampler: Option<crate::compiler::prof_sampler::ProfSampler>,
    prof_cur: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// XEZIM_CONE: entry is a member of a mergeable chain (static analysis).
    cone_chain_member: Vec<bool>,
    /// XEZIM_CONE: the chains themselves, so the end-of-run report can weigh
    /// the dispatches a merge would SAVE against the recomputation it would
    /// FORCE. A merged kernel fires when any member's inputs change and then
    /// recomputes every member, so its best-case work is max(evals)*len while
    /// the unmerged chain costs sum(evals). Equal-firing members make that
    /// ratio 1.0 (merging is pure win); skewed members make it >1.0 and the
    /// added compute has to be paid for out of the saved dispatch.
    cone_chains: Vec<Vec<u32>>,
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
    entry_evals: u64,
    settle_iters: u64,
    max_settle_iters: u64,
    /// Per-comb-entry trigger count (for --activity-mon).
    activity_counts: Vec<u64>,
    /// Per-signal change count (for --activity-mon).
    signal_toggle_counts: Vec<u64>,
    /// Whether activity monitoring is enabled.
    pub activity_mon: bool,
    /// XEZIM_TRACE_ALWAYS: per-fire trace of always blocks. None = off;
    /// Some("") = trace every block; Some(s) = only blocks whose label
    /// (kind, scope, written signal names) contains `s`.
    trace_always: Option<String>,
    /// Runtime plusargs passed from CLI/filelists (e.g. +FOO, +BAR=1).
    plusargs: Vec<String>,
    /// The seed the run chose with `+seed=<n>` / `+seed=random` / `-sv_seed`
    /// (None: the default stream). `XEZIM_INIT_REG=random` mixes it in, so
    /// register X-init values change with the seed.
    run_seed: Option<u64>,

    /// Owned copies of every CLI arg as `CString`s. Required so that
    /// `vpi_argv` (raw `*mut c_char` pointers into these buffers) stays
    /// valid for the lifetime of the simulator — `vpi_get_vlog_info` hands
    /// these pointers back to external C/DPI code which may store them past
    /// the duration of the VPI call.
    vpi_arg_cstrings: Vec<CString>,
    /// Raw C-string pointers parallel to `vpi_arg_cstrings`. The first
    /// entry is the binary name (xezim), the rest are CLI args + plusargs
    /// concatenated. Pointers stay stable across the simulator lifetime.
    vpi_argv: Vec<*mut libc::c_char>,
    /// Cache of DPI scope handles keyed by hierarchical scope name.
    /// Each value is a `Box<DpiScope>` raw pointer handed out to C code
    /// via `svGetScopeFromName` and recovered in `svGetNameFromScope`.
    dpi_scopes: HashMap<String, *mut libc::c_void>,
    prof_psettle_calls: u64,
    prof_psettle_deferred: u64,
    /// cbValueChange index (§38.36.1): watched signal id -> ids of the
    /// callbacks watching it. Consulted on every `write_sig!`, so it is a
    /// flat field (one emptiness check while nothing is watched); the rest of
    /// the callback state lives in `vpi_cb`.
    dpi_value_change_cbs: HashMap<usize, Vec<u64>>,
    /// Every collection/array member name declared by ANY class — the cheap
    /// pre-filter for chained-handle member resolution (see
    /// `instance_assoc_member`). Built once on first use.
    chained_coll_member_names: std::cell::OnceCell<HashSet<String>>,
    /// §18.5.7 foreach solving: the `inside {…}` ranges the current body
    /// imposes on the element being solved (indices bound), so a relational
    /// repick (`arr[i] > arr[i-1]`) lands INSIDE the declared range instead of
    /// anywhere in the element's full domain.
    elem_inside_hint: Vec<(i64, i64)>,
    /// Set after several trials failed a strict foreach check: relational
    /// repicks take the BOUND value (minimal/maximal feasible) instead of a
    /// uniform draw, so a globally coupled monotone chain (`g[i][j][k] >
    /// g[i-1][j][k]` over a 4x4x4 grid in [1:2000]) is solved greedily in
    /// index order rather than exhausting the range by luck.
    rand_tight_mode: bool,
    /// VPI callback registry (§38.36, `vpi_cb.rs`). Untouched unless a VPI
    /// client registers callbacks.
    vpi_cb: Box<vpi_cb::VpiCbState>,
    /// Bit `r` set while a callback of reason `r` is registered (plus
    /// `vpi_cb::MASK_PENDING`): the gate every callback hook tests first, so
    /// a run without VPI callbacks pays one load and branch per hook site.
    vpi_cb_mask: u64,
    /// Time of the last slot whose start (cbNextSimTime, cbAtStartOfSimTime)
    /// has been processed; u64::MAX before the first.
    vpi_slot_started: u64,
    /// `$display` output written since the stdout sink was last synced, so a
    /// VPI callback's own output can be ordered after it cheaply.
    stdout_unsynced: bool,
    /// Self-pointer, installed once at the top of `simulate()`. The
    /// `write_sig!` macro needs a `*mut Simulator` to hand the VPI
    /// callback dispatcher, but it expands in contexts that already hold
    /// a mutable borrow of another field, so it cannot re-borrow `self`.
    /// Reading this field is disjoint from those borrows. Without it the
    /// macro used to install a NULL `ACTIVE_SIMULATOR`, so a callback that
    /// called back into VPI aborted the process.
    vpi_self_ptr: *mut Simulator,
    /// Open file handles for $fopen/$fwrite/$fclose, keyed by an internal
    /// channel id derived from the MCD/FD value (see `resolve_channel_key`):
    /// MCD channels use their bit number (1..=30), FD channels use the raw FD
    /// value (0x8000_0000+). FD STDOUT/STDERR/STDIN and MCD bit 0 are never
    /// inserted — they are handled inline at write time.
    file_handles: HashMap<i32, sv_file::SvFile>,
    /// Some handle holds buffered `$fwrite` output (see `flush_file_writes`).
    file_writes_pending: bool,
    /// §21.3.5 (measured): `$feof` is a STICKY flag set when a read
    /// operation actually hits end-of-file — not a position probe. A read
    /// that consumes exactly up to the final newline leaves it clear.
    /// Cleared by `$fseek`/`$rewind`/`$ungetc` (C stdio semantics).
    file_eof: HashSet<i32>,
    /// Per-fd ungetc pushback buffer (LIFO).
    ungetc_buf: HashMap<i32, Vec<u8>>,
    /// §20.15 stochastic-analysis queues, keyed by q_id. Created by
    /// `$q_initialize`, mutated by `$q_add`/`$q_remove`, queried by
    /// `$q_full`/`$q_exam`.
    queues: HashMap<i64, StochasticQueue>,
    static_task_init: HashSet<String>,
    /// §6.21: the static locals of `initial`/`always`/`final` blocks whose
    /// declaration already ran, by (declarator span, storage name). Such a
    /// variable is created and initialized once; its block's later
    /// activations (and loop iterations) keep its value.
    static_block_decls: HashMap<(usize, usize), Vec<String>>,
    /// Generations of the frames a `for`/`foreach` loop pushes for its
    /// loop variables when its process has no call frame: a block local
    /// declared under one is still a process-block local (§6.21).
    loop_frame_gens: HashSet<u64>,
    current_static_task: Option<String>,
    /// Name of the enclosing subroutine when it is declared `automatic`
    /// (or a class method — automatic by default, §8.6). Mirrors
    /// `current_static_task`, but records the AUTOMATIC half of a
    /// subroutine's lifetime so per-invocation local-array isolation can
    /// be applied to module-scope automatic tasks/functions too, not just
    /// class methods. `None` at top level, and for class methods (whose
    /// locals already get isolated via the `in_method` path).
    current_auto_task: Option<String>,
    /// Best-effort hierarchical context for resolving ambiguous leaf identifiers.
    name_resolve_hint: RefCell<Option<String>>,
    /// The scope of the code CURRENTLY executing as a process/block
    /// activation: `Some(scope)` for an instance-scoped activation,
    /// `Some("")` for a top-scope one (its own names are the bare keys),
    /// `None` outside any activation (build/detection evals). Installed
    /// at activation entry (run_process_stmts for process
    /// activations, the edge-block AST-fallback branch for edge blocks)
    /// and restored on exit. Unlike `name_resolve_hint` it is NEVER
    /// advanced by the resolution ratchet, so a bare name the
    /// activation's own scope declares always wins over a mid-block
    /// hierarchical reference's ratcheted hint (§23.6: an
    /// unqualified name never resolves DOWNWARD into an instance).
    /// Outside activations it stays None so compile-time evals keep
    /// their deliberate hint-first order (clock-gen detection installs
    /// the block's own scope; a `rhs_parent_scoped` port connect
    /// installs the parent's).
    activation_scope: RefCell<Option<String>>,
    /// Depth of running methods of objects of a class declared inside a
    /// (non-top) module (§8, §23.9: each instance holds its own class, whose
    /// methods name that instance's items): such a method runs with the
    /// object's instance (`modclass_instance`) as its activation scope and
    /// resolve hint, and that instance's collections in its rename frame.
    modclass_depth: std::cell::Cell<u32>,
    /// The unpacked arrays, queues, dynamic and associative arrays,
    /// unpacked structs and sub-instances an instance declares (instance
    /// path -> bare names), for the rename frame a module-class method gets
    /// (see `modclass_depth`).
    modclass_colls: RefCell<HashMap<String, std::rc::Rc<Vec<String>>>>,
    /// Per class declared inside a (non-top) module with SEVERAL instances:
    /// those instances' paths (§8, §23.9: each holds its own class, with its
    /// own statics); None for every other class. See `modclass_static_inst`.
    modclass_insts: RefCell<HashMap<String, Option<std::rc::Rc<Vec<String>>>>>,
    /// Whether any such class exists (see `any_multi_modclass`).
    multi_modclass: std::cell::OnceCell<bool>,
    /// Whether any class is declared inside a module other than the top
    /// (computed on first use): without one, `modclass_instance` is None.
    has_modclass: std::cell::Cell<Option<bool>>,
    /// Memoized `signal_name_to_id` keys under an assoc-array prefix.
    /// That map is populated only while the signal table is built and
    /// during `collapse_identity_port_nets` — both elaboration-time —
    /// so the set of its keys under a given prefix is FIXED for the
    /// run. `assoc_top_level_keys` used to re-scan the whole map on
    /// every call, `starts_with` per key; on the axi4 UVM AVIP that
    /// single scan was 5.1% of the run (memcmp attributed to it by a
    /// call-graph profile). The live half of the enumeration (elements
    /// in `self.signals`) is NOT cached — that map mutates at runtime.
    assoc_static_keys_cache: RefCell<HashMap<String, Arc<Vec<Arc<str>>>>>,
    /// §16.9.3 sampled-value function watches: per call site, a history of
    /// the operand sampled (preponed) at each matching clock edge. hist[0]
    /// is the sample at the most recent edge, hist[1] one edge earlier, …
    sampled_watches: Vec<SampledWatch>,
    /// §16.5.1 default sampled values by signal id: what each signal a
    /// concurrent assertion can name held before time 0 (its declaration
    /// initializer, else its type's default). §16.9.3 reads it for a clock
    /// tick before the first one. Filled by `capture_sampled_defaults`.
    sampled_defaults: HashMap<usize, Value>,
    /// The pre-process time-0 settle has run: a value read now is no longer
    /// a default sampled value.
    time0_settled: bool,
    /// §9.2.2.2 timing: `-> ev` fired from a comb block during the PRE-process
    /// time-0 settle (compile) happens before any `always @(ev)` process has
    /// registered its waiter — the trigger would be lost. Queue them and
    /// re-fire at the end of the time-0 active region.
    pending_t0_triggers: Vec<String>,
    /// True while the compile-time (pre-process) time-0 settle runs.
    in_pre_process_settle: bool,
    /// Call-site (expression span start) → per activation scope (see
    /// `sampled_scope`) → index into `sampled_watches`. Every instance of a
    /// child module shares the call's span, so the scope tells them apart.
    sampled_watch_site: HashMap<usize, Vec<(Box<str>, usize)>>,
    /// §16.9.3: for a `$rose`/`$fell`/`$stable`/`$changed`/`$past` call with no
    /// explicit `@(...)` argument and no `default clocking`, the clocking event
    /// is inferred from the ENCLOSING procedural block's event control. Filled
    /// in by `register_sampled_watches` (call-site span → (edge code, clock
    /// signal id)) so the history accumulates from time 0. Keyed per
    /// activation scope like `sampled_watch_site`.
    sampled_site_clock: HashMap<usize, Vec<(Box<str>, (u8, usize))>>,
    /// Edge-controlled always blocks that run as PROCESSES (blocking body)
    /// and call a sampled-value function: (body, scope, edge code, clock id),
    /// consumed by `register_sampled_watches` like the edge blocks.
    proc_sampled_blocks: Vec<(Statement, String, u8, usize)>,
    /// True while `register_sampled_watches` pre-registers the sites.
    sampled_registering: bool,
}

/// One `$rose/$fell/$stable/$changed/$past(x, @(edge clk))` call site.
struct SampledWatch {
    /// A simple clock id, or usize::MAX when `clock_events` supplies it.
    clk_id: usize,
    edge: EdgeKind,
    /// A union, guarded, or expression-valued implicit assertion clock.
    clock_events: Vec<SvaClockEvent>,
    /// Simple-signal operand: sampled PREPONED via the prev-value snapshot.
    sig_id: Option<usize>,
    /// General operand, evaluated using the slot-entry snapshot.
    expr: Expression,
    hist: std::collections::VecDeque<Value>,
    /// §16.9.3: only qualifying sampled clock ticks enter this history.
    gate: Option<Expression>,
    /// An action's gating expression is evaluated in its Reactive context.
    action_gate: bool,
    pending_sample: Option<(u64, Value)>,
    sample_ids: Vec<usize>,
    slot_values: Vec<(usize, Value)>,
    depth: usize,
    /// Time of the most recent sample. A PROCEDURAL `$past` runs before this
    /// slot's sampler (an SVA site after it), so the reader checks whether the
    /// current edge is already at the front of `hist` before indexing.
    last_sample_time: Option<u64>,
    /// §16.5.1: a simple-signal operand's value at the START of the time
    /// slot (`slot_val` taken at `slot_time`'s first sampler pass, when the
    /// delta snapshot still holds the previous slot's final values). A clock
    /// that edges in a LATER delta of the slot than an operand change (a
    /// port clock driven through a continuous assignment, `.ck(a & en)`)
    /// must still sample the operand's preponed value, not the
    /// previous delta's.
    slot_val: Option<Value>,
    slot_time: Option<u64>,
    /// §16.9.3 default sampled value — what the operand held before time 0
    /// (its declaration initializer, else its type's default) — used for a
    /// tick before the first one. Known only for a watch registered before
    /// the run starts.
    default: Option<Value>,
}

/// Empty static used as fallback name for unnamed array-element ids
/// (large 1-D arrays have per-element entries skipped to save memory).
static EMPTY_NAME: &str = "";

/// §20.15 stochastic-analysis queue state for `$q_initialize`/`$q_add`/
/// `$q_remove`/`$q_full`/`$q_exam`.
///
/// `q_type`: 1 = FIFO (§20.15.1 Table 20-9), 2 = LIFO. Each entry records the
/// user `job_id`, `inform_id`, and the sim-time tick at which it was added so
/// `$q_exam` can compute the wait-time statistics of Table 20-10. Statistics
/// accumulate across the queue's lifetime: `max_len_seen` is the high-water
/// mark (code 3); inter-arrival samples feed code 2; per-job waits feed codes
/// 4/5/6. `i64::MIN`/sentinels encode "no sample yet" (the LRM leaves these
/// stats implementation-defined; many tools report -1 in that case).
#[derive(Clone, Debug, Default)]
struct StochasticQueue {
    q_type: i64,
    max_length: i64,
    entries: VecDeque<(i64, i64, i64)>, // (job_id, inform_id, arrival_tick)
    // lifetime statistics
    arrivals: u64,
    last_arrival_tick: Option<i64>,
    sum_interarrival: i128,
    max_len_seen: usize,
    shortest_wait_ever: Option<i64>,
    longest_wait_queued: Option<i64>,
    sum_wait_removed: i128,
    count_wait_removed: u64,
}

/// Evaluate the single real argument of a §20.8.2 one-operand real math
/// function (`$sin`, `$sqrt`, …). Defaults to 0.0 when absent.
#[inline]
fn real1<F>(args: &[Expression], eval: &mut F) -> f64
where
    F: FnMut(&Expression) -> Value,
{
    args.first().map(|a| eval(a).to_f64()).unwrap_or(0.0)
}

/// Evaluate the two real arguments of a §20.8.2 two-operand real math function
/// (`$atan2(y,x)`, `$hypot(x,y)`), defaulting missing operands to 0.0.
#[inline]
fn real2<F>(args: &[Expression], eval: &mut F) -> (f64, f64)
where
    F: FnMut(&Expression) -> Value,
{
    (
        args.first().map(|a| eval(a).to_f64()).unwrap_or(0.0),
        args.get(1).map(|a| eval(a).to_f64()).unwrap_or(0.0),
    )
}

/// Caller-frame state for a queue/dynamic-array LOCAL that a nested call may
/// clobber. Bare-name locals share the global `signals` and `module.*`
/// registration tables, so a nested method declaring a local of the same name
/// (even a non-queue, e.g. its own `int p`) overwrites the caller's
/// bookkeeping. We snapshot the caller's `name.size` / `name[i]` signals AND
/// its `module.arrays`/`dynamic_arrays`/`descending_arrays`/`widths`/
/// queue-limit registration, restoring both on return. Without the
/// registration restore, the caller's queue stays unregistered after the
/// nested call returns, so its later `delete()` and element reads become
/// silent no-ops (IEEE 1800-2023 §6.21 automatic-variable per-invocation storage).
#[derive(Clone, Default, Debug)]
struct QueueLocalSave {
    signals: Vec<(String, Value)>,
    arrays_entry: Option<(i64, i64, u32)>,
    was_dynamic: bool,
    was_descending: bool,
    queue_max: Option<u32>,
    width: Option<u32>,
    /// The caller's `module.associative_arrays` entry (`true` = string
    /// keys): a same-named callee local drops it at its declaration, and an
    /// int-keyed array re-registered as string-keyed reads its keys as text.
    assoc_entry: Option<bool>,
}

/// §6.16.9 string→number conversion: scan LEADING digit/underscore
/// characters and stop at the first other character. No whitespace skip —
/// `"  512".atoi()` is 0 (the C-atoi trim diverged from the reference) —
/// and `'+'` is not consumed. `"12_34"` scans to 1234. A single leading
/// `'-'` negates. For the non-decimal conversions x/z/? are legal 4-STATE
/// digits: `"3fZ".atohex()` is 12'h3fZ (a partially-z integer), matching
/// the reference, not a scan-stop. Result is the signed 32-bit `integer`
/// the LRM specifies.
fn sv_ato_value(text: &str, radix: u32) -> Value {
    let mut it = text.chars().peekable();
    let neg = it.peek() == Some(&'-');
    if neg {
        it.next();
    }
    let mut digits = String::new();
    for c in it {
        if c == '_' {
            continue;
        }
        let is_digit = c.to_digit(radix).is_some()
            || (radix != 10 && matches!(c, 'x' | 'X' | 'z' | 'Z' | '?'));
        if !is_digit {
            break;
        }
        digits.push(c);
    }
    let mut v = if digits.is_empty() {
        Value::zero(32)
    } else {
        Value::from_str_radix(&digits, radix, 32)
    };
    if neg && !v.has_xz() {
        let n = v.to_u64().unwrap_or(0) as i64;
        v = Value::from_u64((-n) as u64, 32);
    }
    v.is_signed = true;
    v
}

/// The consequent of a class-constraint implication: a constraint item
/// (`ConstraintItem::Implication`) or the right operand of a `->` expression.
enum ImpCons<'a> {
    Item(&'a ConstraintItem),
    Expr(&'a Expression),
}

impl Drop for Simulator {
    /// Release heap allocations that aren't tracked by Rust's lifetime
    /// machinery: the `Box<DpiScope>` pointers `svGetScopeFromName` hands out.
    ///
    /// `dpi_scopes` owns `Box`-es that were leaked to the C side. Drop them
    /// here so the simulator's exit doesn't leak them per-test. (Callback
    /// handles belong to the client, which frees them with `vpi_remove_cb`
    /// or `vpi_free_object`; the registry itself is plain owned data.)
    fn drop(&mut self) {
        // Scope handles
        for (_name, ptr) in self.dpi_scopes.drain() {
            if !ptr.is_null() {
                unsafe {
                    drop(Box::from_raw(ptr as *mut DpiScope));
                }
            }
        }
    }
}

// Per-thread path of value-parameter names currently being resolved by
// `Simulator::resolve_value_param_from_spec`. Used to break cyclic value-param
// references (a specialization argument that names a value param of the same
// class, which would otherwise recurse infinitely and overflow the stack).
thread_local! {
    static VALPARAM_RESOLVING: std::cell::RefCell<Vec<String>> =
        std::cell::RefCell::new(Vec::new());
}

/// Drop guard that pops the last entry from `VALPARAM_RESOLVING` when it goes
/// out of scope, ensuring the path is unwound on every return path (including
/// `?` early returns) from `resolve_value_param_from_spec`.
struct ValparamResolveGuard;
impl Drop for ValparamResolveGuard {
    fn drop(&mut self) {
        VALPARAM_RESOLVING.with(|r| r.borrow_mut().pop());
    }
}

/// Builtin collection / string / enum methods `eval_builtin_method`
/// implements, classified once per call from the method name.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BuiltinM {
    First,
    Last,
    Num,
    Next,
    Prev,
    Size,
    Len,
    Getc,
    Substr,
    PushBack,
    PushFront,
    PopFront,
    PopBack,
    Sort,
    Rsort,
    Reverse,
    Shuffle,
    Insert,
    Sum,
    Product,
    And,
    Or,
    Xor,
    Min,
    Max,
    Unique,
    UniqueIndex,
    Find,
    FindFirst,
    FindLast,
    FindIndex,
    FindFirstIndex,
    FindLastIndex,
    Exists,
    Delete,
    Other,
}

impl BuiltinM {
    #[inline]
    fn classify(mname: &str) -> BuiltinM {
        match mname {
            "first" => BuiltinM::First,
            "last" => BuiltinM::Last,
            "num" => BuiltinM::Num,
            "next" => BuiltinM::Next,
            "prev" => BuiltinM::Prev,
            "size" => BuiltinM::Size,
            "len" => BuiltinM::Len,
            "getc" => BuiltinM::Getc,
            "substr" => BuiltinM::Substr,
            "push_back" => BuiltinM::PushBack,
            "push_front" => BuiltinM::PushFront,
            "pop_front" => BuiltinM::PopFront,
            "pop_back" => BuiltinM::PopBack,
            "sort" => BuiltinM::Sort,
            "rsort" => BuiltinM::Rsort,
            "reverse" => BuiltinM::Reverse,
            "shuffle" => BuiltinM::Shuffle,
            "insert" => BuiltinM::Insert,
            "sum" => BuiltinM::Sum,
            "product" => BuiltinM::Product,
            "and" => BuiltinM::And,
            "or" => BuiltinM::Or,
            "xor" => BuiltinM::Xor,
            "min" => BuiltinM::Min,
            "max" => BuiltinM::Max,
            "unique" => BuiltinM::Unique,
            "unique_index" => BuiltinM::UniqueIndex,
            "find" => BuiltinM::Find,
            "find_first" => BuiltinM::FindFirst,
            "find_last" => BuiltinM::FindLast,
            "find_index" => BuiltinM::FindIndex,
            "find_first_index" => BuiltinM::FindFirstIndex,
            "find_last_index" => BuiltinM::FindLastIndex,
            "exists" => BuiltinM::Exists,
            "delete" => BuiltinM::Delete,
            _ => BuiltinM::Other,
        }
    }
}

// ============================================================================
// VPI (Verification Procedural Interface) Support
// ============================================================================

// VPI constants. Every value is the one IEEE 1800-2017 Annex K assigns
// it, and matches `include/vpi_user.h` symbol for symbol. These are not
// free parameters: a C file compiled against a vendor `vpi_user.h` and
// linked to xezim agrees with the numbers below or it silently takes the
// wrong branch.
mod vpi {
    use libc::c_int;

    // Value formats (Table 38-44).
    pub const BIN_STR_VAL: c_int = 1;
    pub const OCT_STR_VAL: c_int = 2;
    pub const DEC_STR_VAL: c_int = 3;
    pub const HEX_STR_VAL: c_int = 4;
    pub const SCALAR_VAL: c_int = 5;
    pub const INT_VAL: c_int = 6;
    pub const REAL_VAL: c_int = 7;
    pub const STRING_VAL: c_int = 8;
    pub const VECTOR_VAL: c_int = 9;
    pub const TIME_VAL: c_int = 11;
    pub const OBJ_TYPE_VAL: c_int = 12;
    /// Set into `value_p->format` when a value cannot be supplied
    /// (§38.16). This is the only failure channel `vpi_get_value` has.
    pub const SUPPRESS_VAL: c_int = 13;

    // vpiScalarVal codes.
    pub const SCALAR_0: c_int = 0;
    pub const SCALAR_1: c_int = 1;
    pub const SCALAR_Z: c_int = 2;
    pub const SCALAR_X: c_int = 3;

    // vpi_put_value flags.
    pub const FORCE_FLAG: c_int = 5;
    pub const RELEASE_FLAG: c_int = 6;

    // vpi_get properties.
    pub const UNDEFINED: c_int = -1;
    pub const TYPE: c_int = 1;
    pub const NAME: c_int = 2;
    pub const FULL_NAME: c_int = 3;
    pub const SIZE: c_int = 4;
    /// §38.35 Table 38-4: simulation time unit / precision as a base-10
    /// exponent of seconds. Both are GLOBAL properties — the standard permits
    /// (and cocotb uses) a NULL object handle to ask for the simulation-wide
    /// value.
    pub const TIME_UNIT: c_int = 11;
    pub const TIME_PRECISION: c_int = 12;
    pub const DEF_NAME: c_int = 9;
    pub const SCALAR: c_int = 17;
    pub const VECTOR: c_int = 18;
    pub const SIGNED: c_int = 65;
    pub const DIRECTION: c_int = 20;
    pub const INPUT: c_int = 1;
    pub const OUTPUT: c_int = 2;
    pub const INOUT: c_int = 3;
    #[allow(dead_code)] // Reserved VPI ABI value; retained alongside supported values.
    pub const MIXED_IO: c_int = 4;
    pub const NO_DIRECTION: c_int = 5;

    // vpi_control operations.
    pub const STOP: c_int = 66;
    pub const FINISH: c_int = 67;
    pub const RESET: c_int = 68;

    // vpi_chk_error levels and states.
    #[allow(dead_code)] // Reserved VPI ABI value; retained alongside supported values.
    pub const NOTICE: c_int = 1;
    pub const WARNING: c_int = 2;
    pub const ERROR: c_int = 3;
    pub const RUN_STATE: c_int = 3;

    // System-function return kinds (`s_vpi_systf_data.sysfunctype`).
    #[allow(dead_code)] // Reserved VPI ABI value; retained alongside supported values.
    pub const INT_FUNC: c_int = 1;
    pub const REAL_FUNC: c_int = 2;
    pub const TIME_FUNC: c_int = 3;
    pub const SIZED_FUNC: c_int = 4;
    pub const SIZED_SIGNED_FUNC: c_int = 5;

    // Object types returned by vpi_get(vpiType), and traversal relations.
    pub const CONSTANT: c_int = 7;
    pub const ITERATOR: c_int = 27;
    pub const FUNC_TYPE: c_int = 44;
    pub const SYS_FUNC_CALL: c_int = 56;
    pub const SYS_TASK_CALL: c_int = 57;
    pub const SYS_TF_CALL: c_int = 85;
    pub const ARGUMENT: c_int = 89;
    pub const MEMORY: c_int = 29;
    pub const MODULE: c_int = 32;
    /// The generic "instance" class (module/program/interface/package). It is
    /// a SystemVerilog extension, so it lives in `sv_vpi_user.h` at 745 — NOT
    /// in `vpi_user.h`. cocotb asks for this, not vpiModule, when discovering
    /// the design root.
    pub const INSTANCE: c_int = 745;
    pub const PARAMETER: c_int = 41;
    pub const PART_SELECT: c_int = 42;
    pub const SCOPE: c_int = 84;
    pub const INTERNAL_SCOPE: c_int = 92;
    pub const VARIABLES: c_int = 100;
    pub const INTEGER_VAR: c_int = 25;
    pub const NET: c_int = 36;
    pub const PORT: c_int = 44;
    pub const PORT_BIT: c_int = 45;
    pub const REAL_VAR: c_int = 47;
    /// Also `vpiLogicVar` — the standard aliases them.
    pub const REG: c_int = 48;
    pub const TIME_VAR: c_int = 63;
    pub const LONG_INT_VAR: c_int = 610;
    pub const SHORT_INT_VAR: c_int = 611;
    pub const INT_VAR: c_int = 612;
    pub const SHORT_REAL_VAR: c_int = 613;
    pub const BYTE_VAR: c_int = 614;
    pub const STRING_VAR: c_int = 616;
    pub const ENUM_VAR: c_int = 617;
    pub const STRUCT_VAR: c_int = 618;
    pub const UNION_VAR: c_int = 619;
    pub const BIT_VAR: c_int = 620;

    // Time types.
    pub const SCALED_REAL_TIME: c_int = 1;
    pub const SIM_TIME: c_int = 2;
    pub const SUPPRESS_TIME: c_int = 3;

    // s_vpi_systf_data.type
    pub const SYS_TASK: c_int = 1;
    pub const SYS_FUNC: c_int = 2;

    // Callback reasons (Table 38-49).
    #[allow(dead_code)] // Reserved VPI ABI value; retained alongside supported values.
    pub const CB_VALUE_CHANGE: c_int = 1;
    #[allow(dead_code)] // Reserved VPI ABI value; retained alongside supported values.
    pub const CB_NEXT_SIM_TIME: c_int = 8;
    #[allow(dead_code)] // Reserved VPI ABI value; retained alongside supported values.
    pub const CB_START_OF_SIMULATION: c_int = 11;
    #[allow(dead_code)] // Reserved VPI ABI value; retained alongside supported values.
    pub const CB_END_OF_SIMULATION: c_int = 12;
    #[allow(dead_code)] // Reserved VPI ABI value; retained alongside supported values.
    pub const CB_READ_WRITE_SYNCH: c_int = 6;
    #[allow(dead_code)] // Reserved VPI ABI value; retained alongside supported values.
    pub const CB_READ_ONLY_SYNCH: c_int = 7;
    #[allow(dead_code)] // Reserved VPI ABI value; retained alongside supported values.
    pub const CB_AFTER_DELAY: c_int = 9;
    #[allow(dead_code)] // Reserved VPI ABI value; retained alongside supported values.
    pub const CB_START_OF_RESET: c_int = 19;
}

// VPI types matching vpi_user.h. `aval`/`bval` are PLI_INT32 (signed) in
// the standard header, so mirror that exactly — the bit patterns are what
// matter, but the sizes must agree.
//
// Per-bit encoding (§38.10.1):
//     aval bval   value
//       0    0      0
//       1    0      1
//       0    1      Z
//       1    1      X
#[repr(C)]
#[derive(Clone, Copy)]
struct s_vpi_vecval {
    aval: i32,
    bval: i32,
}

#[repr(C)]
union s_vpi_value_union {
    str: *mut libc::c_char,
    scalar: libc::c_int,
    integer: libc::c_int,
    real: libc::c_double,
    time: *mut s_vpi_time,
    vector: *mut s_vpi_vecval,
    misc: *mut libc::c_char,
}

#[repr(C)]
pub struct s_vpi_value {
    format: libc::c_int,
    value: s_vpi_value_union,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct s_vpi_time {
    type_: libc::c_int,
    high: u32,
    low: u32,
    real: libc::c_double,
}

// Scratch storage handed back to C by `vpi_get_value` for the formats
// whose value is a pointer (vectors, strings, time). IEEE 1800-2017
// §38.16 lets the simulator own this memory and only guarantees it
// until the next `vpi_get_value` call, which is exactly what these
// give: each call overwrites the previous contents.
thread_local! {
    static VPI_VEC_SCRATCH: std::cell::RefCell<Vec<s_vpi_vecval>> =
        const { std::cell::RefCell::new(Vec::new()) };
    static VPI_STR_SCRATCH: std::cell::RefCell<Vec<u8>> =
        const { std::cell::RefCell::new(Vec::new()) };
    // vpi_get_str() rotates through a pool rather than reusing one buffer.
    // `vpi_printf("%s %s", vpi_get_str(vpiName, h), vpi_get_str(vpiDefName, h))`
    // is idiomatic VPI: both pointers are live at once, and C leaves the
    // argument evaluation order unspecified. A single shared buffer makes
    // both strings read as whichever call happened to run last.
    static VPI_STR2_SCRATCH: std::cell::RefCell<(usize, Vec<Vec<u8>>)> =
        std::cell::RefCell::new((0, vec![Vec::new(); 8]));
    static VPI_TIME_SCRATCH: std::cell::RefCell<s_vpi_time> =
        const { std::cell::RefCell::new(s_vpi_time { type_: 2, high: 0, low: 0, real: 0.0 }) };
}

/// `Value` bit code (0=0, 1=1, 2=X, 3=Z) -> one `(aval, bval)` bit pair.
#[inline]
fn bit_code_to_ab(code: u8) -> (u32, u32) {
    match code {
        0 => (0, 0),
        1 => (1, 0),
        2 => (1, 1), // X
        _ => (0, 1), // Z
    }
}

/// One `(aval, bval)` bit pair -> `Value` bit code.
#[inline]
fn ab_to_bit_code(a: u32, b: u32) -> u8 {
    match (a, b) {
        (0, 0) => 0,
        (1, 0) => 1,
        (1, 1) => 2, // X
        _ => 3,      // Z
    }
}

/// Render a `Value` into `s_vpi_vecval` words, preserving X and Z.
fn value_to_vecval(v: &Value, out: &mut Vec<s_vpi_vecval>) {
    let w = v.width.max(1) as usize;
    let words = w.div_ceil(32);
    out.clear();
    out.resize(words, s_vpi_vecval { aval: 0, bval: 0 });
    let mut aval = vec![0u32; words];
    let mut bval = vec![0u32; words];
    for i in 0..w {
        let (a, b) = bit_code_to_ab(v.get_bit_code(i));
        if a != 0 {
            aval[i / 32] |= 1u32 << (i % 32);
        }
        if b != 0 {
            bval[i / 32] |= 1u32 << (i % 32);
        }
    }
    for i in 0..words {
        out[i] = s_vpi_vecval {
            aval: aval[i] as i32,
            bval: bval[i] as i32,
        };
    }
}

/// Build a `Value` of `width` bits from `s_vpi_vecval` words, preserving
/// X and Z. Words beyond the slice read as zero.
fn vecval_to_value(vec: &[s_vpi_vecval], width: u32, is_signed: bool) -> Value {
    let mut v = Value::zero(width);
    for i in 0..width as usize {
        let w = i / 32;
        if w >= vec.len() {
            break;
        }
        let a = (vec[w].aval as u32 >> (i % 32)) & 1;
        let b = (vec[w].bval as u32 >> (i % 32)) & 1;
        v.set_bit_code(i, ab_to_bit_code(a, b));
    }
    v.is_signed = is_signed;
    v
}

/// Radix string with 4-state digits, `bits_per_digit` of 1, 3 or 4.
/// A digit is `z` when every bit in it is Z, otherwise `x` when any bit
/// is unknown — the Verilog display rule.
/// Inverse of [`vpi_radix_string`]: decode a `vpi*StrVal` payload into a
/// `w`-bit `Value`.
///
/// `x`/`z` (and `?`, which the LRM accepts for z) cover a WHOLE digit, so one
/// `x` in a hex string is four x bits. Digits are read LSB-first from the right;
/// a string shorter than the signal left-extends with 0, longer is truncated.
/// Underscores and surrounding whitespace are ignored (§38.16).
fn vpi_value_from_radix_str(s: &str, bits_per_digit: u32, w: u32) -> Value {
    let mut v = Value::zero(w);
    let mut bit: u32 = 0;
    for ch in s.chars().rev() {
        if bit >= w {
            break;
        }
        if ch == '_' || ch.is_whitespace() {
            continue;
        }
        let lc = ch.to_ascii_lowercase();
        let per_bit_code = match lc {
            'x' => Some(2u8),
            'z' | '?' => Some(3u8),
            _ => None,
        };
        match per_bit_code {
            Some(code) => {
                for k in 0..bits_per_digit {
                    if bit + k < w {
                        v.set_bit_code((bit + k) as usize, code);
                    }
                }
            }
            None => {
                // An unparsable digit is not silently a zero — leave the run
                // as x so a malformed payload is visible rather than wrong.
                let d = lc.to_digit(1u32 << bits_per_digit);
                for k in 0..bits_per_digit {
                    if bit + k >= w {
                        break;
                    }
                    let code = match d {
                        Some(dv) => ((dv >> k) & 1) as u8,
                        None => 2u8,
                    };
                    v.set_bit_code((bit + k) as usize, code);
                }
            }
        }
        bit += bits_per_digit;
    }
    v
}

fn vpi_radix_string(v: &Value, bits_per_digit: usize) -> String {
    let w = v.width.max(1) as usize;
    let ndigits = w.div_ceil(bits_per_digit);
    let mut s = String::with_capacity(ndigits);
    for d in (0..ndigits).rev() {
        let mut digit = 0u32;
        let (mut xs, mut zs, mut n) = (0usize, 0usize, 0usize);
        for b in 0..bits_per_digit {
            let idx = d * bits_per_digit + b;
            if idx >= w {
                continue;
            }
            n += 1;
            match v.get_bit_code(idx) {
                1 => digit |= 1 << b,
                2 => xs += 1,
                3 => zs += 1,
                _ => {}
            }
        }
        if n > 0 && zs == n {
            s.push('z');
        } else if xs + zs > 0 {
            s.push('x');
        } else {
            s.push(char::from_digit(digit, 1u32 << bits_per_digit).unwrap_or('0'));
        }
    }
    if s.is_empty() {
        s.push('0');
    }
    s
}

// Thread-local pointer to the current simulator instance.
// Set during DPI import calls so VPI C callbacks can access it.
thread_local! {
    static ACTIVE_SIMULATOR: std::cell::Cell<*mut Simulator> = const { std::cell::Cell::new(std::ptr::null_mut()) };
    /// Current DPI scope set by `svSetScope` and read by `svGetScope`.
    /// Cached as a raw pointer into the simulator's `dpi_scopes` map.
    /// Points to a `DpiScope` struct, or null if no scope is active.
    static ACTIVE_SCOPE: std::cell::Cell<*mut libc::c_void> = const { std::cell::Cell::new(std::ptr::null_mut()) };
}

/// Process-wide fallback for VPI callers that execute on a different thread
/// than the one that installed `ACTIVE_SIMULATOR` in TLS.
static GLOBAL_ACTIVE_SIMULATOR: std::sync::atomic::AtomicPtr<Simulator> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// What a `vpiHandle` points at. `vpiHandle` is `void *` to C, so this is
/// entirely private; every entry point checks `kind` before using the
/// fields that only some kinds populate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum VpiKind {
    /// A module port (§37.16): its own object, with `vpiDirection`; the
    /// connected signal keeps its `vpiNet`/`vpiReg` type. `signal_id` is the
    /// port's signal so value reads work through the port handle.
    Port,
    /// A whole signal in the flat signal table.
    Signal,
    /// A bit range of a signal — a packed-struct member or part-select.
    Slice,
    /// A module instance. `inst_idx == -1` is the top module.
    Module,
    /// An unpacked array. `vpi_handle_by_index` selects a word.
    Memory,
    /// The result of `vpi_iterate`, consumed by `vpi_scan`.
    Iterator,
    /// A read-only value: a `$systf` argument that is not signal-backed
    /// (a literal, or any computed expression).
    Constant,
    /// The `$systf` call currently executing. `vpi_iterate(vpiArgument, ..)`
    /// walks its arguments; `vpi_put_value` on it sets a system function's
    /// return value.
    SysTfCall,
    /// A system task or function registered with `vpi_register_systf`
    /// (`vpiUserSystf`). `name` is its `$name`.
    UserSystf,
    /// A module path (`vpiModPath`): `signal_id` is its output net, `lsb`
    /// the path's position among that net's paths.
    ModPath,
    /// A timing check (`vpiTchk`): `signal_id` indexes the simulator's
    /// timing checks.
    Tchk,
    /// An intermodule path (`vpiInterModPath`): `signal_id` is the input
    /// port's signal, `inst_idx` the output port's.
    InterModPath,
    /// A `vpi_register_cb` handle; `signal_id` holds the callback's id in
    /// `Simulator::vpi_cb` (see `vpi_cb.rs`).
    Callback,
    /// A scope of the object model that is not an instance: a package, a
    /// generate scope, a named block, a task or a function. `signal_id` is
    /// its index in the model (see `vpi_model.rs`).
    Scope,
    /// Any other object of the object model: processes, continuous
    /// assignments, primitives, ports without storage, expressions, ranges,
    /// typespecs, ... `signal_id` is its index in the model; `usize::MAX`
    /// marks a constant made on the fly (value in `value`, `vpiConstType`
    /// in `lsb`).
    Obj,
}

/// Wrapper for VPI handles stored as raw pointers.
#[derive(Clone)]
struct VpiHandle {
    kind: VpiKind,
    /// Signal / Slice / Memory: index into the signal table. For Memory this
    /// is the id of element 0.
    signal_id: usize,
    /// `vpi_get(vpiType)` answer, resolved from the DECLARED type when the
    /// handle is created. It must not be derived from the current value:
    /// a `logic` signal is a `vpiReg` whether or not it happens to hold X.
    type_code: libc::c_int,
    direction: libc::c_int,
    /// Slice only: bit range within `signal_id`.
    lsb: u32,
    width: u32,
    /// Module only: index into `module.instances`, or -1 for the top module.
    inst_idx: isize,
    /// Local name (`data_bus`) and hierarchical name (`tb_top.u_sub.data_bus`).
    name: String,
    full_name: String,
    /// Module only: the definition this instantiates (`vpiDefName`).
    def_name: String,
    /// Iterator only: the objects still to be handed out by `vpi_scan`.
    items: Vec<VpiHandle>,
    pos: usize,
    /// Constant only: the argument's value, captured at call time.
    value: Option<Value>,
    /// SysTfCall only: depth of the frame in `VPI_SYSTF_STACK`, so a handle
    /// held across a nested `$systf` call still names its own frame.
    frame: usize,
}

impl VpiHandle {
    fn signal(id: usize, type_code: libc::c_int, name: &str, full: &str) -> Self {
        VpiHandle {
            kind: VpiKind::Signal,
            signal_id: id,
            type_code,
            direction: vpi::NO_DIRECTION,
            lsb: 0,
            width: 0,
            inst_idx: -1,
            name: name.to_string(),
            full_name: full.to_string(),
            def_name: String::new(),
            items: Vec::new(),
            pos: 0,
            value: None,
            frame: 0,
        }
    }

    fn port(id: usize, name: &str, full: &str, direction: libc::c_int, width: u32) -> Self {
        let mut h = VpiHandle::signal(id, vpi::PORT, name, full);
        h.kind = VpiKind::Port;
        h.direction = direction;
        h.width = width;
        h
    }

    fn into_raw(self) -> *mut libc::c_void {
        Box::into_raw(Box::new(self)) as *mut libc::c_void
    }

    /// A read-only `$systf` argument that is not signal-backed.
    fn constant(v: Value) -> Self {
        VpiHandle {
            kind: VpiKind::Constant,
            signal_id: 0,
            type_code: vpi::CONSTANT,
            direction: vpi::NO_DIRECTION,
            lsb: 0,
            width: v.width,
            inst_idx: -1,
            name: String::new(),
            full_name: String::new(),
            def_name: String::new(),
            items: Vec::new(),
            pos: 0,
            value: Some(v),
            frame: 0,
        }
    }

    /// The `$systf` call at `frame` (an index into `VPI_SYSTF_STACK`).
    fn systf_call(frame: usize, name: &str, is_func: bool, ret_width: u32) -> Self {
        VpiHandle {
            kind: VpiKind::SysTfCall,
            signal_id: 0,
            type_code: if is_func {
                vpi::SYS_FUNC_CALL
            } else {
                vpi::SYS_TASK_CALL
            },
            direction: vpi::NO_DIRECTION,
            lsb: 0,
            width: ret_width,
            inst_idx: -1,
            name: name.to_string(),
            full_name: name.to_string(),
            def_name: String::new(),
            items: Vec::new(),
            pos: 0,
            value: None,
            frame,
        }
    }
}

/// Borrow a handle, returning None for NULL.
///
/// # Safety
/// `handle` must be NULL or a pointer returned by one of the VPI handle
/// constructors and not yet freed.
unsafe fn vpi_deref<'a>(handle: *mut libc::c_void) -> Option<&'a VpiHandle> {
    unsafe {
        if handle.is_null() {
            None
        } else {
            Some(&*(handle as *const VpiHandle))
        }
    }
}

/// The `$systf` invocation currently on the stack. `vpi_handle(vpiSysTfCall,
/// NULL)` names the innermost one; `vpi_iterate(vpiArgument, ..)` walks its
/// arguments; `vpi_put_value` on it sets a system function's return value.
struct VpiSystfFrame {
    /// The `$name`, as `vpi_get_str(vpiName, ..)` reports it.
    name: String,
    /// Argument objects, resolved once at call time. A signal-backed argument
    /// is writable (an `output` argument); the rest are `Constant`.
    args: Vec<VpiHandle>,
    /// Return value deposited by `vpi_put_value` on the call handle.
    ret: Option<Value>,
    /// Width the return value is resized to.
    ret_width: u32,
    /// vpiSysTask or vpiSysFunc.
    tf_type: libc::c_int,
    /// `sysfunctype`, for a function.
    func_type: libc::c_int,
    /// The call site (see `vpi_api::systf_site`): what `vpi_put_userdata`
    /// attaches to, carried by the call handle as its `signal_id`.
    site: usize,
}

// Innermost frame last: a `$systf` may invoke another.
thread_local! {
    static VPI_SYSTF_STACK: std::cell::RefCell<Vec<VpiSystfFrame>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// The last diagnostic, for `vpi_chk_error`. Cleared once reported.
    static VPI_LAST_ERROR: std::cell::RefCell<Option<(libc::c_int, String)>> =
        const { std::cell::RefCell::new(None) };
}

/// Record a diagnostic so `vpi_chk_error` can report it, and echo it to stderr.
///
/// Every VPI failure path routes through here. Without it `vpi_chk_error`
/// would always answer "no error", and a caller that checks it — which is the
/// only reason to call it — would sail straight past a failed call.
fn vpi_error(level: libc::c_int, msg: String) {
    eprintln!("[VPI] {}", msg);
    VPI_LAST_ERROR.with(|c| *c.borrow_mut() = Some((level, msg)));
    // §38.36.3 cbPLIError.
    if level >= vpi::ERROR {
        vpi_cb::vpi_notify_pli_error();
    }
}

/// A `$systf` registered from a VPI module via `vpi_register_systf`.
#[derive(Clone, Copy)]
struct VpiSystf {
    /// `PLI_INT32 (*calltf)(PLI_BYTE8 *user_data)`, as a `usize` so the
    /// struct stays `Copy` and can live in a thread-local map.
    calltf: usize,
    compiletf: usize,
    /// Called once per invocation of a `vpiSizedFunc` to learn its return width.
    sizetf: usize,
    user_data: usize,
    /// vpiSysTask (1) or vpiSysFunc (2).
    tf_type: libc::c_int,
    /// `sysfunctype`: vpiIntFunc / vpiRealFunc / vpiTimeFunc / vpiSizedFunc /
    /// vpiSizedSignedFunc. Meaningless for a task.
    func_type: libc::c_int,
}

// System tasks and functions registered by a VPI module's
// `vlog_startup_routines`. Registration happens before the Simulator is
// built, so this cannot hang off it. Single-threaded, like the rest of the
// VPI surface.
thread_local! {
    static VPI_SYSTFS: std::cell::RefCell<HashMap<String, VpiSystf>> =
        std::cell::RefCell::new(HashMap::default());
}

/// Mirror of `s_vpi_systf_data` from `vpi_user.h` (IEEE 1800-2017 §38.36).
#[repr(C)]
pub struct s_vpi_systf_data {
    type_: libc::c_int,
    sysfunctype: libc::c_int,
    tfname: *mut libc::c_char,
    calltf: *mut libc::c_void,
    compiletf: *mut libc::c_void,
    sizetf: *mut libc::c_void,
    user_data: *mut libc::c_char,
}

/// Mirror of `s_cb_data` from `vpi_user.h` — the value-change dispatcher
/// builds one of these on the stack and hands its address to the C
/// callback. Fields match the IEEE 1800-2017 §38.7 layout exactly.
///
/// `index` sits between `value` and `user_data` in the standard struct.
/// It was missing here, so every field after it was read from the wrong
/// offset by any C code compiled against a conforming `vpi_user.h` —
/// `user_data` in particular.
#[repr(C)]
pub struct s_cb_data {
    reason: libc::c_int,
    /// C function pointer (`PLI_INT32 (*cb_rtn)(p_cb_data)`).
    cb_rtn: *mut libc::c_void,
    /// Object the callback was registered against (the signal handle for
    /// cbValueChange). Passed back to the callback on each fire.
    obj: *mut libc::c_void,
    time: *mut s_vpi_time,
    value: *mut s_vpi_value,
    index: libc::c_int,
    user_data: *mut libc::c_void,
}

/// Mirror of `s_vpi_vlog_info` from `vpi_user.h`. Returned by
/// `vpi_get_vlog_info` to provide binary name and version info.
#[repr(C)]
pub struct s_vpi_vlog_info {
    argc: libc::c_int,
    argv: *mut *mut libc::c_char,
    product: *mut libc::c_char,
    version: *mut libc::c_char,
}

/// Opaque DPI scope handle. The `name` field carries the hierarchical
/// scope name; `svGetNameFromScope` recovers it by casting the pointer
/// back to a `DpiScope`.
#[repr(C)]
struct DpiScope {
    name: [u8; 256],
    name_len: usize,
}

/// Execute a closure with access to the active simulator.
///
/// Returns `None` when there is no active simulator — a VPI call from
/// outside a DPI context. This used to `panic!`, which unwinds across an
/// `extern "C"` boundary and therefore aborts the process; every VPI
/// entry point now degrades to its documented failure value instead.
fn try_active_sim<F, R>(who: &str, f: F) -> Option<R>
where
    F: FnOnce(&mut Simulator) -> R,
{
    let mut sim_ptr = ACTIVE_SIMULATOR.with(|cell| cell.get());
    if sim_ptr.is_null() {
        sim_ptr = GLOBAL_ACTIVE_SIMULATOR.load(std::sync::atomic::Ordering::Acquire);
    }
    if sim_ptr.is_null() {
        eprintln!(
            "[VPI] {}: no active simulator (call from outside a DPI context)",
            who
        );
        return None;
    }
    Some(f(unsafe { &mut *sim_ptr }))
}

/// Execute a closure with access to the active simulator, for the DPI
/// scope primitives, which have no way to report failure.
fn with_active_sim<F, R>(f: F) -> R
where
    F: FnOnce(&mut Simulator) -> R,
{
    let mut sim_ptr = ACTIVE_SIMULATOR.with(|cell| cell.get());
    if sim_ptr.is_null() {
        sim_ptr = GLOBAL_ACTIVE_SIMULATOR.load(std::sync::atomic::Ordering::Acquire);
    }
    if sim_ptr.is_null() {
        panic!("[VPI] No active simulator - VPI call outside DPI context?");
    }
    f(unsafe { &mut *sim_ptr })
}

/// The `vpi_get(vpiType)` code for a signal, from its DECLARED type.
///
/// Falls back to `vpiReg` — a 4-state vector variable — for anything whose
/// declaration we cannot see, which is the safe answer: a caller that
/// believes a signal may hold X will read it with a 4-state format.
fn vpi_type_of(sim: &Simulator, name: &str, id: usize) -> libc::c_int {
    use xezim_core::ast::types::{
        DataType, IntegerAtomType, IntegerVectorType, RealType, SimpleType, StructUnionKind,
    };

    // A parameter is also stored as a signal, so this must come first or
    // every parameter reports vpiReg.
    if sim.module.parameters.contains_key(name) {
        return vpi::PARAMETER;
    }
    if sim.module.nets.contains(name) {
        return vpi::NET;
    }
    // The signal table's `is_real` covers reals reached through a path we
    // have no declaration for.
    let looks_real = sim.signal_table.get(id).map(|v| v.is_real).unwrap_or(false);

    let dt = sim.module.var_decl_types.get(name).or_else(|| {
        name.rsplit('.')
            .next()
            .and_then(|leaf| sim.module.var_decl_types.get(leaf))
    });

    match dt {
        Some(DataType::IntegerVector { kind, .. }) => match kind {
            IntegerVectorType::Bit => vpi::BIT_VAR,
            IntegerVectorType::Logic | IntegerVectorType::Reg => vpi::REG,
        },
        Some(DataType::IntegerAtom { kind, .. }) => match kind {
            IntegerAtomType::Byte => vpi::BYTE_VAR,
            IntegerAtomType::ShortInt => vpi::SHORT_INT_VAR,
            IntegerAtomType::Int => vpi::INT_VAR,
            IntegerAtomType::LongInt => vpi::LONG_INT_VAR,
            IntegerAtomType::Integer => vpi::INTEGER_VAR,
            IntegerAtomType::Time => vpi::TIME_VAR,
        },
        Some(DataType::Real { kind, .. }) => match kind {
            RealType::ShortReal => vpi::SHORT_REAL_VAR,
            _ => vpi::REAL_VAR,
        },
        Some(DataType::Simple {
            kind: SimpleType::String,
            ..
        }) => vpi::STRING_VAR,
        Some(DataType::Enum(_)) => vpi::ENUM_VAR,
        Some(DataType::Struct(su)) => match su.kind {
            StructUnionKind::Union => vpi::UNION_VAR,
            _ => vpi::STRUCT_VAR,
        },
        _ if looks_real => vpi::REAL_VAR,
        _ => vpi::REG,
    }
}

fn vpi_direction_code(dir: PortDirection) -> libc::c_int {
    match dir {
        PortDirection::Input => vpi::INPUT,
        PortDirection::Output => vpi::OUTPUT,
        PortDirection::Inout => vpi::INOUT,
        PortDirection::Ref => vpi::NO_DIRECTION,
    }
}

/// Direction of the port a signal belongs to, `vpiNoDirection` for a plain
/// signal. The standard defines `vpiDirection` on port objects; answering
/// it on the port's signal too is a convenience that costs nothing.
fn vpi_direction_of(sim: &Simulator, name: &str) -> libc::c_int {
    sim.vpi_port_directions
        .get(name)
        .map(|d| vpi_direction_code(*d))
        .unwrap_or(vpi::NO_DIRECTION)
}

fn new_vpi_handle(sim: &Simulator, name: &str, id: usize) -> *mut libc::c_void {
    let type_code = vpi_type_of(sim, name, id);
    let leaf = name.rsplit('.').next().unwrap_or(name);
    let mut handle = VpiHandle::signal(id, type_code, leaf, &vpi_full_name(sim, name));
    handle.direction = vpi_direction_of(sim, name);
    handle.into_raw()
}

/// Prefix a design-relative name with the top module, the way
/// `vpi_get_str(vpiFullName, ..)` must report it.
fn vpi_full_name(sim: &Simulator, name: &str) -> String {
    sim.hier_path(name)
}

/// Strip a leading `<top>.` from a hierarchical name. Signal-table names
/// are stored relative to the top module, but VPI callers write the full
/// path.
fn vpi_strip_top<'a>(sim: &Simulator, name: &'a str) -> &'a str {
    let top = sim.module.name.as_str();
    if let Some(rest) = name.strip_prefix(top) {
        if let Some(rest) = rest.strip_prefix('.') {
            return rest;
        }
        if rest.is_empty() {
            return "";
        }
    }
    name
}

/// A module handle for `inst_idx` (-1 = the top module).
fn vpi_module_handle(sim: &Simulator, inst_idx: isize) -> *mut libc::c_void {
    let (name, full, def) = if inst_idx < 0 {
        (
            sim.module.name.clone(),
            sim.module.name.clone(),
            sim.module.name.clone(),
        )
    } else {
        let inst = &sim.module.instances[inst_idx as usize];
        let leaf = inst
            .path
            .rsplit('.')
            .next()
            .unwrap_or(&inst.path)
            .to_string();
        (leaf, vpi_full_name(sim, &inst.path), inst.def_name.clone())
    };
    VpiHandle {
        kind: VpiKind::Module,
        signal_id: 0,
        type_code: vpi::MODULE,
        direction: vpi::NO_DIRECTION,
        lsb: 0,
        width: 0,
        inst_idx,
        name,
        full_name: full,
        def_name: def,
        items: Vec::new(),
        pos: 0,
        value: None,
        frame: 0,
    }
    .into_raw()
}

/// The design-relative path of a module handle ("" for the top module).
fn vpi_scope_path(sim: &Simulator, h: &VpiHandle) -> String {
    if h.inst_idx < 0 {
        String::new()
    } else {
        sim.module.instances[h.inst_idx as usize].path.clone()
    }
}

/// True when `name` is a signal the elaborator invented for an instance
/// (a 1-bit placeholder so validation recognises the name). Those must not
/// appear as VPI signal objects — they are module instances.
fn vpi_is_instance_name(sim: &Simulator, name: &str) -> bool {
    sim.module.instances.iter().any(|i| i.path == name)
}

/// Resolve a packed-struct member path (`pixel_data.r`) to its bit range.
fn vpi_slice_of(sim: &Simulator, name: &str) -> Option<VpiHandle> {
    let (base, member) = name.rsplit_once('.')?;
    let fields = sim.module.packed_struct_fields.get(base)?;
    let &(_, lsb, width) = fields.iter().find(|(m, _, _)| m == member)?;
    let &id = sim.signal_name_to_id.get(base)?;
    Some(VpiHandle {
        kind: VpiKind::Slice,
        signal_id: id,
        type_code: vpi::PART_SELECT,
        direction: vpi::NO_DIRECTION,
        lsb,
        width,
        inst_idx: -1,
        name: member.to_string(),
        full_name: vpi_full_name(sim, name),
        def_name: String::new(),
        items: Vec::new(),
        pos: 0,
        value: None,
        frame: 0,
    })
}

/// Read a slice out of its parent signal.
fn vpi_slice_read(sim: &Simulator, h: &VpiHandle) -> Option<Value> {
    let parent = &vpi_api::cell_value(sim, h.signal_id)?;
    let mut v = Value::zero(h.width);
    for i in 0..h.width as usize {
        v.set_bit_code(i, parent.get_bit_code(h.lsb as usize + i));
    }
    Some(v)
}

/// Write a slice back into its parent signal.
fn vpi_slice_write(sim: &Simulator, h: &VpiHandle, val: &Value) -> Option<Value> {
    let mut parent = vpi_api::cell_value(sim, h.signal_id)?;
    for i in 0..h.width as usize {
        parent.set_bit_code(h.lsb as usize + i, val.get_bit_code(i));
    }
    Some(parent)
}

/// Get signal ID by hierarchical name.
///
/// Tries the full name, then each successively shorter suffix, so
/// `top.dut.sig`, `dut.sig` and `sig` all resolve. Member paths like
/// `top.dut.some_struct.val_i` are registered as `some_struct.val_i`, so
/// the suffix walk is what finds them.
///
/// The previous loop never advanced its cursor: it only ever tested the
/// name with its first segment stripped, and when that missed it spun
/// forever. Any unresolvable dotted name hung the simulator when checking
/// paths that may not exist.
/// §35.5.4 DPI export dispatch. The generated trampoline shared object calls
/// this from C to run an exported SystemVerilog subroutine. `id` is the
/// subroutine's declaration-order index; `n` args are 64-bit integer slots.
#[unsafe(no_mangle)]
pub extern "C" fn __xezim_dpi_export_dispatch(
    id: libc::c_longlong,
    n: libc::c_longlong,
    a: *const libc::c_longlong,
) -> libc::c_longlong {
    try_active_sim("__xezim_dpi_export_dispatch", |sim| {
        if n < 0 {
            // An export C cannot call (`load_dpi_export_trampoline`).
            return sim.run_dpi_export_unmodelled(id as usize) as libc::c_longlong;
        }
        sim.run_dpi_export(id as usize, n as usize, a as *const i64) as libc::c_longlong
    })
    .unwrap_or(0)
}

#[unsafe(no_mangle)]
pub extern "C" fn vpi_handle_by_name(
    name: *mut libc::c_char,
    scope: *mut libc::c_void,
) -> *mut libc::c_void {
    if name.is_null() {
        return std::ptr::null_mut();
    }
    let full_name = unsafe { std::ffi::CStr::from_ptr(name) }
        .to_string_lossy()
        .trim()
        .to_string();

    try_active_sim("vpi_handle_by_name", |sim| {
        // VPI object model (vpi_model.rs): every declared object, scope and
        // generate block by its full name, or relative to `scope`.
        if let Some(h) = vpi_model::handle_by_name(sim, &full_name, unsafe { vpi_deref(scope) }) {
            return h;
        }
        // The top module itself, and any instance in the hierarchy.
        let rel = vpi_strip_top(sim, &full_name);
        if rel.is_empty() {
            return vpi_module_handle(sim, -1);
        }
        if let Some(i) = sim.module.instances.iter().position(|i| i.path == rel) {
            return vpi_module_handle(sim, i as isize);
        }

        let mut rest: &str = full_name.as_str();
        loop {
            // An instance's 1-bit placeholder signal must not be handed out
            // as a signal object.
            if !vpi_is_instance_name(sim, rest) {
                // Arrays first: a continuous assignment that reads an element
                // leaves a 1-bit placeholder signal under the array's own
                // name, which must not shadow the array.
                if let Some(h) = vpi_memory_of(sim, rest) {
                    return h.into_raw();
                }
                // Multi-dimensional arrays, their sub-arrays, and elements
                // of arrays too large to have named elements.
                if let Some(h) = vpi_api::array_object_of(sim, rest) {
                    return h.into_raw();
                }
                if let Some(&id) = sim.signal_name_to_id.get(rest) {
                    return new_vpi_handle(sim, rest, id);
                }
                if let Some(h) = vpi_slice_of(sim, rest) {
                    return h.into_raw();
                }
            }
            match rest.find('.') {
                Some(dot) => rest = &rest[dot + 1..],
                None => return std::ptr::null_mut(),
            }
        }
    })
    .unwrap_or(std::ptr::null_mut())
}

/// An unpacked array (`int mem [0:3]`) as a `vpiMemory` object. Its words
/// are separate signals named `mem[0]`..`mem[n-1]`; `vpi_handle_by_index`
/// selects one.
fn vpi_memory_of(sim: &Simulator, name: &str) -> Option<VpiHandle> {
    let &(lo, hi, _w) = sim.module.arrays.get(name)?;
    let count = (hi - lo).unsigned_abs() as u32 + 1;
    let id = *sim
        .signal_name_to_id
        .get(format!("{}[{}]", name, lo).as_str())?;
    Some(VpiHandle {
        kind: VpiKind::Memory,
        signal_id: id,
        type_code: vpi::MEMORY,
        direction: vpi::NO_DIRECTION,
        lsb: lo.min(hi) as u32,
        width: count,
        inst_idx: -1,
        name: name.rsplit('.').next().unwrap_or(name).to_string(),
        full_name: vpi_full_name(sim, name),
        def_name: String::new(),
        items: Vec::new(),
        pos: 0,
        value: None,
        frame: 0,
    })
}

/// Select one word of a `vpiMemory`.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_handle_by_index(
    handle: *mut libc::c_void,
    index: libc::c_int,
) -> *mut libc::c_void {
    let Some(h) = (unsafe { vpi_deref(handle) }) else {
        return std::ptr::null_mut();
    };
    // VPI object model (vpi_model.rs): arrays, sub-arrays and generate
    // scope arrays.
    if let Some(r) = try_active_sim("vpi_handle_by_index", |sim| {
        vpi_model::handle_by_index(sim, h, index as i64)
    })
    .flatten()
    {
        return r;
    }
    if h.kind != VpiKind::Memory {
        return std::ptr::null_mut();
    }
    // A multi-dimensional array (or a sub-array) indexes its leftmost
    // remaining dimension, exactly as one index of vpi_handle_by_multi_index.
    if let Some(r) = vpi_api::index_array_handle(h, index) {
        return r;
    }
    let full = h.full_name.clone();
    try_active_sim("vpi_handle_by_index", |sim| {
        let rel = vpi_strip_top(sim, &full).to_string();
        let elem = format!("{}[{}]", rel, index);
        match sim.signal_name_to_id.get(elem.as_str()) {
            Some(&id) => {
                let ty = vpi_type_of(sim, &elem, id);
                VpiHandle::signal(id, ty, &elem, &vpi_full_name(sim, &elem)).into_raw()
            }
            None => std::ptr::null_mut(),
        }
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Traverse a one-to-one relationship. Only `vpiScope` is modelled.
///
/// `vpi_handle(vpiScope, NULL)` returns the top module. That is an xezim
/// extension — the standard route to the top is
/// `vpi_scan(vpi_iterate(vpiModule, NULL))` — but enough VPI code in the
/// wild spells it this way that supporting it is worth more than the
/// purity of returning NULL.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_handle(type_: libc::c_int, refh: *mut libc::c_void) -> *mut libc::c_void {
    // The currently executing $systf. A `NULL` reference is the idiomatic
    // spelling; the standard also allows passing the call handle itself.
    if type_ == vpi::SYS_TF_CALL {
        return VPI_SYSTF_STACK.with(|st| {
            let st = st.borrow();
            match st.last() {
                Some(f) => {
                    let mut h = VpiHandle::systf_call(
                        st.len() - 1,
                        &f.name,
                        f.tf_type == vpi::SYS_FUNC,
                        f.ret_width,
                    );
                    h.signal_id = f.site;
                    h.into_raw()
                }
                None => {
                    vpi_error(
                        vpi::ERROR,
                        "vpi_handle(vpiSysTfCall): no $systf is executing".to_string(),
                    );
                    std::ptr::null_mut()
                }
            }
        });
    }
    if type_ == vpi_api::vc::USER_SYSTF {
        return vpi_api::user_systf_of_call(refh);
    }
    try_active_sim("vpi_handle", |sim| {
        // VPI object model (vpi_model.rs): the one-to-one relations.
        if let Some(r) = vpi_model::handle(sim, type_, unsafe { vpi_deref(refh) }) {
            return r;
        }
        if type_ != vpi::SCOPE {
            return std::ptr::null_mut();
        }
        let Some(h) = (unsafe { vpi_deref(refh) }) else {
            return vpi_module_handle(sim, -1);
        };
        match h.kind {
            // The scope containing a module is its parent.
            VpiKind::Module => {
                if h.inst_idx < 0 {
                    return std::ptr::null_mut(); // the top has no parent
                }
                let parent = sim.module.instances[h.inst_idx as usize].parent.clone();
                if parent.is_empty() && sim.root_is_multi_top() {
                    std::ptr::null_mut() // a top under the multi-top wrapper
                } else if parent.is_empty() {
                    vpi_module_handle(sim, -1)
                } else {
                    match sim.module.instances.iter().position(|i| i.path == parent) {
                        Some(i) => vpi_module_handle(sim, i as isize),
                        None => std::ptr::null_mut(),
                    }
                }
            }
            // The scope containing an object is the instance it is declared in.
            _ => {
                let rel = vpi_strip_top(sim, &h.full_name).to_string();
                match rel.rsplit_once('.') {
                    Some((scope, _)) => {
                        match sim.module.instances.iter().position(|i| i.path == scope) {
                            Some(i) => vpi_module_handle(sim, i as isize),
                            None => vpi_module_handle(sim, -1),
                        }
                    }
                    None => vpi_module_handle(sim, -1),
                }
            }
        }
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Objects declared DIRECTLY in `scope` (no deeper dotted component), in
/// sorted order so iteration is deterministic.
fn vpi_scope_members(sim: &Simulator, scope: &str) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = Vec::new();
    for (name, &id) in sim.signal_name_to_id.iter() {
        let name: &str = name;
        let leaf = if scope.is_empty() {
            if name.contains('.') {
                continue;
            }
            name
        } else {
            match name.strip_prefix(scope).and_then(|r| r.strip_prefix('.')) {
                Some(r) if !r.contains('.') => r,
                _ => continue,
            }
        };
        // Array words (`mem[0]`) are reached through their vpiMemory parent,
        // and an instance placeholder is not a signal.
        if leaf.contains('[')
            || vpi_is_instance_name(sim, name)
            || xezim_core::elaborate::is_delay_stage_net(name)
            || event_bits::is_edge_alias_net(name)
        {
            continue;
        }
        out.push((name.to_string(), id));
    }
    // Unpacked arrays have no whole-array signal; surface them as vpiMemory.
    for name in sim.module.arrays.keys() {
        let matches_scope = if scope.is_empty() {
            !name.contains('.')
        } else {
            name.strip_prefix(scope)
                .and_then(|r| r.strip_prefix('.'))
                .map(|r| !r.contains('.'))
                .unwrap_or(false)
        };
        if matches_scope && !out.iter().any(|(n, _)| n == name) {
            out.push((name.clone(), usize::MAX));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Iterate a one-to-many relationship. Returns NULL when the relationship
/// yields nothing, as the standard requires (callers test for it).
#[unsafe(no_mangle)]
pub extern "C" fn vpi_iterate(type_: libc::c_int, refh: *mut libc::c_void) -> *mut libc::c_void {
    // Module paths and timing checks come from the object model when it has
    // the sources; `vpi_api`'s view of the simulator's tables is the fallback
    // (a design loaded from a compiled artifact). The delay routines accept
    // either kind of handle.
    const MOD_PATH: libc::c_int = 31;
    const TCHK: libc::c_int = 61;
    if type_ == MOD_PATH || type_ == TCHK {
        if let Some(r) = try_active_sim("vpi_iterate", |sim| {
            vpi_model::iterate(sim, type_, unsafe { vpi_deref(refh) })
        })
        .flatten()
        {
            return r;
        }
    }
    if let Some(it) = vpi_api::iterate_extra(type_, refh) {
        return it;
    }
    // The arguments of a $systf call. Handled before the design traversal
    // below because it needs no simulator, only the call frame.
    if type_ == vpi::ARGUMENT {
        let Some(h) = (unsafe { vpi_deref(refh) }) else {
            return std::ptr::null_mut();
        };
        if h.kind != VpiKind::SysTfCall {
            return std::ptr::null_mut();
        }
        let items = VPI_SYSTF_STACK.with(|st| {
            st.borrow()
                .get(h.frame)
                .map(|f| f.args.clone())
                .unwrap_or_default()
        });
        return vpi_make_iterator(items);
    }
    try_active_sim("vpi_iterate", |sim| {
        let scope_h = unsafe { vpi_deref(refh) };
        // VPI object model (vpi_model.rs): the one-to-many relations.
        if let Some(r) = vpi_model::iterate(sim, type_, scope_h) {
            return r;
        }

        // A NULL reference means "the whole design": the only top-level
        // module is the one we elaborated.
        let scope_path = match scope_h {
            None => {
                if type_ == vpi::MODULE || type_ == vpi::INSTANCE {
                    // Under the multi-top wrapper the tops are its children.
                    let tops: Vec<isize> = if sim.root_is_multi_top() {
                        (0..sim.module.instances.len())
                            .filter(|&i| sim.module.instances[i].parent.is_empty())
                            .map(|i| i as isize)
                            .collect()
                    } else {
                        vec![-1]
                    };
                    let items = tops
                        .into_iter()
                        .map(|i| unsafe {
                            *Box::from_raw(vpi_module_handle(sim, i) as *mut VpiHandle)
                        })
                        .collect();
                    return vpi_make_iterator(items);
                }
                return std::ptr::null_mut();
            }
            Some(h) if h.kind == VpiKind::Module => vpi_scope_path(sim, h),
            Some(_) => return std::ptr::null_mut(),
        };

        // Child scopes / instances.
        if type_ == vpi::MODULE || type_ == vpi::INSTANCE || type_ == vpi::INTERNAL_SCOPE {
            let items: Vec<VpiHandle> = sim
                .module
                .instances
                .iter()
                .enumerate()
                .filter(|(_, i)| i.parent == scope_path)
                .map(|(idx, _)| unsafe {
                    *Box::from_raw(vpi_module_handle(sim, idx as isize) as *mut VpiHandle)
                })
                .collect();
            return vpi_make_iterator(items);
        }

        // Declared objects, filtered by what the caller asked for.
        if type_ == vpi::PORT {
            // §37.16: ports are their own objects, one per direction-bearing
            // member of the scope, in name order.
            let items: Vec<VpiHandle> = vpi_scope_members(sim, &scope_path)
                .into_iter()
                .filter(|(_, id)| *id != usize::MAX)
                .filter_map(|(name, id)| {
                    let dir = sim.vpi_port_directions.get(name.as_str())?;
                    let leaf = name.rsplit('.').next().unwrap_or(&name).to_string();
                    let width = sim.signal_widths.get(id).copied().unwrap_or(1) as u32;
                    Some(VpiHandle::port(
                        id,
                        &leaf,
                        &vpi_full_name(sim, &name),
                        vpi_direction_code(*dir),
                        width,
                    ))
                })
                .collect();
            return vpi_make_iterator(items);
        }

        let want = |ty: libc::c_int| -> bool {
            match type_ {
                vpi::NET => ty == vpi::NET,
                vpi::REG => ty == vpi::REG || ty == vpi::BIT_VAR,
                vpi::PARAMETER => ty == vpi::PARAMETER,
                vpi::MEMORY => ty == vpi::MEMORY,
                vpi::VARIABLES => ty != vpi::NET && ty != vpi::PARAMETER && ty != vpi::MEMORY,
                _ => false,
            }
        };

        let mut items: Vec<VpiHandle> = Vec::new();
        for (name, id) in vpi_scope_members(sim, &scope_path) {
            if id == usize::MAX {
                if let Some(m) = vpi_memory_of(sim, &name) {
                    if want(vpi::MEMORY) {
                        items.push(m);
                    }
                }
                continue;
            }
            let ty = vpi_type_of(sim, &name, id);
            if want(ty) {
                let leaf = name.rsplit('.').next().unwrap_or(&name);
                let mut handle = VpiHandle::signal(id, ty, leaf, &vpi_full_name(sim, &name));
                handle.direction = vpi_direction_of(sim, &name);
                items.push(handle);
            }
        }
        vpi_make_iterator(items)
    })
    .unwrap_or(std::ptr::null_mut())
}

fn vpi_make_iterator(items: Vec<VpiHandle>) -> *mut libc::c_void {
    if items.is_empty() {
        return std::ptr::null_mut();
    }
    VpiHandle {
        kind: VpiKind::Iterator,
        signal_id: 0,
        type_code: vpi::ITERATOR,
        direction: vpi::NO_DIRECTION,
        lsb: 0,
        width: 0,
        inst_idx: -1,
        name: String::new(),
        full_name: String::new(),
        def_name: String::new(),
        items,
        pos: 0,
        value: None,
        frame: 0,
    }
    .into_raw()
}

/// Hand out the next object of an iterator. When the iterator is exhausted
/// it is FREED and NULL returned — the caller must not free it itself, per
/// IEEE 1800-2017 §38.32.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_scan(iter: *mut libc::c_void) -> *mut libc::c_void {
    if iter.is_null() {
        return std::ptr::null_mut();
    }
    let h = unsafe { &mut *(iter as *mut VpiHandle) };
    if h.kind != VpiKind::Iterator {
        return std::ptr::null_mut();
    }
    if h.pos >= h.items.len() {
        drop(unsafe { Box::from_raw(iter as *mut VpiHandle) });
        return std::ptr::null_mut();
    }
    let next = h.items[h.pos].clone();
    h.pos += 1;
    next.into_raw()
}

/// String-valued properties. The returned pointer addresses simulator-owned
/// storage valid until the next `vpi_get_str` call on this thread.
/// Spelling of a `vpiType` code, for `vpi_get_str(vpiType, h)`. Kept beside
/// the `vpi` constant block it mirrors so the two cannot drift.
fn vpi_type_name(code: libc::c_int) -> Option<&'static str> {
    Some(match code {
        vpi::CONSTANT => "vpiConstant",
        vpi::ITERATOR => "vpiIterator",
        vpi::INTEGER_VAR => "vpiIntegerVar",
        vpi::MEMORY => "vpiMemory",
        vpi::MODULE => "vpiModule",
        vpi::NET => "vpiNet",
        vpi::PORT => "vpiPort",
        vpi::PORT_BIT => "vpiPortBit",
        vpi::PARAMETER => "vpiParameter",
        vpi::PART_SELECT => "vpiPartSelect",
        vpi::REAL_VAR => "vpiRealVar",
        vpi::REG => "vpiReg",
        vpi::SCOPE => "vpiScope",
        vpi::SYS_FUNC_CALL => "vpiSysFuncCall",
        vpi::SYS_TASK_CALL => "vpiSysTaskCall",
        vpi::TIME_VAR => "vpiTimeVar",
        vpi::INTERNAL_SCOPE => "vpiInternalScope",
        vpi::LONG_INT_VAR => "vpiLongIntVar",
        vpi::SHORT_INT_VAR => "vpiShortIntVar",
        vpi::INT_VAR => "vpiIntVar",
        vpi::SHORT_REAL_VAR => "vpiShortRealVar",
        vpi::BYTE_VAR => "vpiByteVar",
        vpi::STRING_VAR => "vpiStringVar",
        vpi::ENUM_VAR => "vpiEnumVar",
        vpi::STRUCT_VAR => "vpiStructVar",
        vpi::UNION_VAR => "vpiUnionVar",
        vpi::BIT_VAR => "vpiBitVar",
        vpi_cb::VPI_CALLBACK => "vpiCallback",
        _ => return None,
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn vpi_get_str(
    property: libc::c_int,
    handle: *mut libc::c_void,
) -> *mut libc::c_char {
    let Some(h) = (unsafe { vpi_deref(handle) }) else {
        return std::ptr::null_mut();
    };
    // VPI object model (vpi_model.rs): names, files and type names.
    let model = match vpi_model::get_str_static(property, h) {
        Some(r) => Some(r),
        None if !matches!(
            h.kind,
            VpiKind::SysTfCall | VpiKind::Constant | VpiKind::Iterator
        ) =>
        {
            try_active_sim("vpi_get_str", |sim| vpi_model::get_str(sim, property, h)).flatten()
        }
        None => None,
    };
    if let Some(r) = model {
        return match r {
            Some(s) => vpi_model::str_result(&s),
            None => std::ptr::null_mut(),
        };
    }
    let s = match property {
        vpi::NAME => h.name.clone(),
        vpi::FULL_NAME => h.full_name.clone(),
        vpi::DEF_NAME if h.kind == VpiKind::Module => h.def_name.clone(),
        // §38.38: the STRING spelling of the object's type, which must agree
        // with the code `vpi_get(vpiType, h)` reports. Clients print this in
        // diagnostics — cocotb logs a warning for every object whose type it
        // cannot name.
        vpi::TYPE => match vpi_type_name(h.type_code) {
            Some(n) => n.to_string(),
            None => {
                vpi_error(
                    vpi::WARNING,
                    format!("vpi_get_str: no name for vpiType {}", h.type_code),
                );
                return std::ptr::null_mut();
            }
        },
        _ => {
            vpi_error(
                vpi::WARNING,
                format!("vpi_get_str: property {} not modelled", property),
            );
            return std::ptr::null_mut();
        }
    };
    VPI_STR2_SCRATCH.with(|cell| {
        let mut pool = cell.borrow_mut();
        let slot = pool.0;
        pool.0 = (slot + 1) % 8;
        let buf = &mut pool.1[slot];
        buf.clear();
        buf.extend_from_slice(s.as_bytes());
        buf.push(0);
        buf.as_mut_ptr() as *mut libc::c_char
    })
}

/// Register a system task or function implemented in a VPI module.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_register_systf(data: *mut s_vpi_systf_data) -> *mut libc::c_void {
    if data.is_null() {
        return std::ptr::null_mut();
    }
    let d = unsafe { &*data };
    if d.tfname.is_null() || d.calltf.is_null() {
        vpi_error(
            vpi::ERROR,
            "vpi_register_systf: tfname and calltf are required".into(),
        );
        return std::ptr::null_mut();
    }
    let name = unsafe { std::ffi::CStr::from_ptr(d.tfname) }
        .to_string_lossy()
        .trim()
        .to_string();
    if !name.starts_with('$') {
        vpi_error(
            vpi::ERROR,
            format!("vpi_register_systf: tfname '{}' must start with '$'", name),
        );
        return std::ptr::null_mut();
    }
    if d.type_ != vpi::SYS_TASK && d.type_ != vpi::SYS_FUNC {
        vpi_error(
            vpi::ERROR,
            format!(
                "vpi_register_systf: '{}' has type {}, expected vpiSysTask or vpiSysFunc",
                name, d.type_
            ),
        );
        return std::ptr::null_mut();
    }
    let entry = VpiSystf {
        calltf: d.calltf as usize,
        compiletf: d.compiletf as usize,
        sizetf: d.sizetf as usize,
        user_data: d.user_data as usize,
        tf_type: d.type_,
        func_type: d.sysfunctype,
    };
    VPI_SYSTFS.with(|m| m.borrow_mut().insert(name.clone(), entry));
    // The `vpiUserSystf` object: `vpi_get_systf_info` reads it back.
    vpi_api::note_registered_systf(&name);
    vpi_api::user_systf_handle(&name).into_raw()
}

/// Call a registered `$systf`. Returns false when `name` is not registered,
/// so the caller can fall through to its own diagnostic.
/// Width of a system function's return value, per its `sysfunctype`.
/// `vpiSizedFunc` asks the module via `sizetf`.
fn vpi_sysfunc_width(entry: &VpiSystf) -> u32 {
    type TfFn = extern "C" fn(*mut libc::c_char) -> libc::c_int;
    match entry.func_type {
        vpi::REAL_FUNC => 64,
        vpi::TIME_FUNC => 64,
        vpi::SIZED_FUNC | vpi::SIZED_SIGNED_FUNC if entry.sizetf != 0 => {
            let f: TfFn = unsafe { std::mem::transmute(entry.sizetf as *const ()) };
            let w = f(entry.user_data as *mut libc::c_char);
            if w > 0 { w as u32 } else { 32 }
        }
        _ => 32,
    }
}

/// Invoke a registered `$systf` with a frame carrying its arguments, so the
/// C side can reach them through `vpi_handle(vpiSysTfCall, NULL)` and
/// `vpi_iterate(vpiArgument, ..)`, and deposit a return value.
///
/// Returns the function's return value; `None` for a task, or when `name` is
/// not registered (so the caller can fall through to its own diagnostic).
fn vpi_call_systf(
    sim: *mut Simulator,
    name: &str,
    args: &[Expression],
    span: crate::ast::Span,
) -> Option<Value> {
    let entry = VPI_SYSTFS.with(|m| m.borrow().get(name).copied())?;
    // The call instance this invocation belongs to (its user data lives
    // there, and its compiletf runs once).
    let (site, first_call) = vpi_api::systf_site(unsafe { &*sim }, name, span);

    let is_func = entry.tf_type == vpi::SYS_FUNC;
    let ret_width = if is_func {
        vpi_sysfunc_width(&entry)
    } else {
        0
    };

    // Resolve the arguments BEFORE the frame is pushed: building a handle
    // evaluates expressions, which can itself invoke a nested $systf.
    let arg_handles: Vec<VpiHandle> = unsafe {
        let s = &mut *sim;
        args.iter().map(|a| s.vpi_arg_handle(a)).collect()
    };

    VPI_SYSTF_STACK.with(|st| {
        st.borrow_mut().push(VpiSystfFrame {
            name: name.to_string(),
            args: arg_handles,
            ret: None,
            ret_width,
            tf_type: entry.tf_type,
            func_type: entry.func_type,
            site,
        })
    });

    ACTIVE_SIMULATOR.with(|cell| cell.set(sim));
    // Foreign code may read what the design wrote to a file.
    unsafe { (*sim).flush_file_writes() };
    type TfFn = extern "C" fn(*mut libc::c_char) -> libc::c_int;
    // compiletf runs once per call instance, just before that instance's
    // first calltf: xezim has no separate compile phase to run it in.
    if entry.compiletf != 0 && first_call {
        let f: TfFn = unsafe { std::mem::transmute(entry.compiletf as *const ()) };
        f(entry.user_data as *mut libc::c_char);
    }
    let f: TfFn = unsafe { std::mem::transmute(entry.calltf as *const ()) };
    f(entry.user_data as *mut libc::c_char);

    let frame = VPI_SYSTF_STACK.with(|st| st.borrow_mut().pop());
    if !is_func {
        return None;
    }
    // A function that never deposited a value returns 0, as the LRM's own
    // system functions do.
    let mut v = frame
        .and_then(|f| f.ret)
        .unwrap_or_else(|| Value::zero(ret_width.max(1)));
    if entry.func_type == vpi::REAL_FUNC {
        v.is_real = true;
    } else if entry.func_type == vpi::SIZED_SIGNED_FUNC {
        v.is_signed = true;
    }
    Some(v)
}

/// True when a VPI module registered `name`. Lets the caller distinguish a
/// user-defined `$task` from a genuinely unknown one.
pub fn vpi_systf_registered(name: &str) -> bool {
    VPI_SYSTFS.with(|m| m.borrow().contains_key(name))
}

/// Return width of a registered system function, without invoking it.
pub fn vpi_sysfunc_ret_width(name: &str) -> u32 {
    VPI_SYSTFS
        .with(|m| m.borrow().get(name).map(|e| vpi_sysfunc_width(e)))
        .unwrap_or(32)
}

/// True when `name` was registered as a system FUNCTION (`vpiSysFunc`).
pub fn vpi_systf_is_func(name: &str) -> bool {
    VPI_SYSTFS.with(|m| {
        m.borrow()
            .get(name)
            .is_some_and(|e| e.tf_type == vpi::SYS_FUNC)
    })
}

/// Mirror of `s_vpi_error_info` (IEEE 1800-2017 §38.4).
#[repr(C)]
pub struct s_vpi_error_info {
    state: libc::c_int,
    level: libc::c_int,
    message: *mut libc::c_char,
    product: *mut libc::c_char,
    code: *mut libc::c_char,
    file: *mut libc::c_char,
    line: libc::c_int,
}

/// Report the last VPI diagnostic, and clear it. Returns the severity level,
/// or 0 when nothing has gone wrong since the previous call.
///
/// `error_info_p` may be NULL — callers often use `vpi_chk_error(NULL)` purely
/// as a "did anything fail?" probe.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_chk_error(error_info_p: *mut s_vpi_error_info) -> libc::c_int {
    let taken = VPI_LAST_ERROR.with(|c| c.borrow_mut().take());
    let Some((level, msg)) = taken else { return 0 };
    if !error_info_p.is_null() {
        let info = unsafe { &mut *error_info_p };
        info.state = vpi::RUN_STATE;
        info.level = level;
        info.message = vpi_err_scratch(&msg);
        info.product = b"xezim\0".as_ptr() as *mut libc::c_char;
        info.code = b"\0".as_ptr() as *mut libc::c_char;
        info.file = std::ptr::null_mut();
        info.line = 0;
    }
    level
}

fn vpi_err_scratch(s: &str) -> *mut libc::c_char {
    thread_local! {
        static BUF: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    BUF.with(|cell| {
        let mut b = cell.borrow_mut();
        b.clear();
        b.extend_from_slice(s.as_bytes());
        b.push(0);
        b.as_mut_ptr() as *mut libc::c_char
    })
}

/// Load each `--vpi-lib` and run its `vlog_startup_routines`, the standard
/// entry point for a VPI module (IEEE 1800-2017 §38.2).
pub fn vpi_run_startup_routines(libs: &mut Vec<Library>, paths: &[String]) {
    for path in paths {
        // SAFETY: loading a dynamic library is inherently unsafe; the handle
        // is kept alive for the simulator's lifetime.
        let lib = match unsafe { Library::new(path) } {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[VPI] failed to load '{}': {}", path, e);
                continue;
            }
        };
        type StartupFn = unsafe extern "C" fn();
        // `vlog_startup_routines` is a NULL-terminated array of function
        // pointers, not a function.
        let routines: *const Option<StartupFn> =
            match unsafe { lib.get::<*const Option<StartupFn>>(b"vlog_startup_routines\0") } {
                Ok(sym) => *sym,
                Err(_) => {
                    eprintln!(
                        "[VPI] '{}' has no vlog_startup_routines; nothing to register",
                        path
                    );
                    libs.push(lib);
                    continue;
                }
            };
        if routines.is_null() {
            libs.push(lib);
            continue;
        }
        let mut i = 0isize;
        loop {
            let slot = unsafe { *routines.offset(i) };
            match slot {
                Some(f) => unsafe { f() },
                None => break,
            }
            i += 1;
        }
        libs.push(lib);
    }
}

/// Free a VPI handle.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_free_object(handle: *mut libc::c_void) -> libc::c_int {
    // §38.24: 1 on SUCCESS, 0 on failure. This returned 0 unconditionally, so
    // every free reported failure even though the handle was released — which
    // is what made cocotb log "Attempting to free root iterator failed!" after
    // a perfectly good root scan.
    if handle.is_null() {
        return 0;
    }
    drop(unsafe { Box::from_raw(handle as *mut VpiHandle) });
    1
}

/// Get a VPI property value. Returns `vpiUndefined` (-1) for a property
/// xezim does not model, rather than a plausible-looking 0.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_get(property: libc::c_int, handle: *mut libc::c_void) -> libc::c_int {
    // vpiTimeUnit / vpiTimePrecision are answered BEFORE the handle is
    // dereferenced: they are simulation-wide, and a NULL `obj` is the standard
    // way to ask for the whole simulation's value. Deref-first returned
    // `vpiUndefined` (-1) for them, and a consumer that reads the result as a
    // base-10 exponent then believes the timebase is 1e-1 s — which is exactly
    // how cocotb reported "simulator precision of 1e-1" and made every `Timer`
    // unrepresentable.
    if property == vpi::TIME_UNIT || property == vpi::TIME_PRECISION {
        // A module handle answers with that module's own timeunit /
        // timeprecision (#206); NULL, or any other object, with the
        // simulation's — the finest precision in the design.
        let module_def = unsafe { vpi_deref(handle) }
            .filter(|h| h.kind == VpiKind::Module)
            .map(|h| h.def_name.clone());
        let exp = try_active_sim("vpi_get", |sim| match &module_def {
            Some(def) => {
                let (unit, prec) = sim.reported_timescale_exp(def);
                if property == vpi::TIME_UNIT {
                    unit
                } else {
                    prec
                }
            }
            None => secs_to_exp(sim.module.tick_s),
        });
        return exp.unwrap_or(vpi::UNDEFINED);
    }
    let Some(h) = (unsafe { vpi_deref(handle) }) else {
        return vpi::UNDEFINED;
    };
    // VPI object model (vpi_model.rs): the properties of every object it
    // knows, including the declared type.
    let model = match vpi_model::get_static(property, h) {
        Some(v) => Some(v),
        None if !matches!(
            h.kind,
            VpiKind::SysTfCall | VpiKind::Constant | VpiKind::Iterator
        ) =>
        {
            try_active_sim("vpi_get", |sim| vpi_model::get(sim, property, h)).flatten()
        }
        None => None,
    };
    if let Some(v) = model {
        return v;
    }
    if property == vpi::TYPE {
        return h.type_code;
    }
    if property == vpi::DIRECTION {
        return h.direction;
    }
    // Properties of the objects `vpi_api` adds (systf, timing objects,
    // packed-arena memory words).
    if let Some(v) = vpi_api::get_property(property, h) {
        return v;
    }
    // vpiFuncType of a system function call is its `sysfunctype`.
    if h.kind == VpiKind::SysTfCall {
        return match property {
            vpi::SIZE => h.width as libc::c_int,
            vpi::FUNC_TYPE => VPI_SYSTF_STACK
                .with(|st| st.borrow().get(h.frame).map(|f| f.func_type))
                .unwrap_or(vpi::UNDEFINED),
            _ => vpi::UNDEFINED,
        };
    }
    if h.kind == VpiKind::Constant {
        let w = h.width as libc::c_int;
        return match property {
            vpi::SIZE => w,
            vpi::SIGNED => i32::from(h.value.as_ref().is_some_and(|v| v.is_signed)),
            vpi::SCALAR => i32::from(w == 1),
            vpi::VECTOR => i32::from(w > 1),
            _ => vpi::UNDEFINED,
        };
    }
    // A module or an iterator has no width, sign or scalar-ness.
    if property == vpi::DIRECTION {
        return match h.kind {
            VpiKind::Port | VpiKind::Signal => h.direction,
            _ => vpi::UNDEFINED,
        };
    }
    match h.kind {
        VpiKind::Module | VpiKind::Iterator | VpiKind::Callback => return vpi::UNDEFINED,
        // vpiSize of a memory is its number of words; of a slice, its bits.
        VpiKind::Memory | VpiKind::Slice if property == vpi::SIZE => return h.width as libc::c_int,
        _ => {}
    }
    let sig_id = h.signal_id;
    try_active_sim("vpi_get", |sim| {
        let width = sim.signal_widths.get(sig_id).copied().unwrap_or(32) as libc::c_int;
        match property {
            vpi::SIZE => width,
            vpi::SIGNED => i32::from(sim.signal_signed.get(sig_id).copied().unwrap_or(false)),
            vpi::SCALAR => i32::from(width == 1),
            vpi::VECTOR => i32::from(width > 1),
            _ => vpi::UNDEFINED,
        }
    })
    .unwrap_or(vpi::UNDEFINED)
}

/// Base-10 exponent of a seconds value, as VPI reports time unit/precision.
/// Mirrors `Simulator::secs_to_exp`, which is a private associated fn.
fn secs_to_exp(s: f64) -> libc::c_int {
    if s <= 0.0 {
        -9
    } else {
        s.log10().round() as libc::c_int
    }
}

/// Store `s` into the thread-local string scratch as a NUL-terminated C
/// string and return a pointer to it.
fn vpi_str_scratch(s: &str) -> *mut libc::c_char {
    VPI_STR_SCRATCH.with(|cell| {
        let mut buf = cell.borrow_mut();
        buf.clear();
        buf.extend_from_slice(s.as_bytes());
        buf.push(0);
        buf.as_mut_ptr() as *mut libc::c_char
    })
}

/// Read a signal value via VPI.
///
/// On any failure — bad handle, out-of-range signal, or a format xezim
/// cannot supply — sets `value_p->format` to `vpiSuppressVal` and writes
/// nothing else. IEEE 1800-2017 §38.16 makes that the caller's only way
/// to detect the failure, and `vpi_get_value` returns `void`, so there is
/// nowhere else to say so.
///
/// This used to handle exactly two formats and silently ignore the rest.
/// `vpiVectorVal` was among the ignored ones, which is the format
/// HDL backdoor read routines use: `vpi_get_value` returned success having
/// written nothing into the caller's buffer.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_get_value(handle: *mut libc::c_void, value_p: *mut s_vpi_value) {
    if value_p.is_null() {
        return;
    }
    let vp = unsafe { &mut *value_p };
    let Some(h) = (unsafe { vpi_deref(handle) }) else {
        vpi_error(vpi::ERROR, "vpi_get_value: null handle".into());
        vp.format = vpi::SUPPRESS_VAL;
        return;
    };
    // VPI object model (vpi_model.rs): expressions, constants, and the
    // objects that have no value.
    if matches!(h.kind, VpiKind::Obj | VpiKind::Scope) || h.type_code == vpi_model::c::NAMED_EVENT {
        let _ = try_active_sim("vpi_get_value", |sim| vpi_model::get_value(sim, h, vp));
        return;
    }
    // A constant argument carries its own value and needs no simulator.
    if h.kind == VpiKind::Constant {
        let ok = match &h.value {
            Some(v) => fill_vpi_value(v, 0, Some(h.type_code), vp),
            None => false,
        };
        if !ok {
            vpi_error(
                vpi::ERROR,
                format!(
                    "vpi_get_value: cannot supply format {} for a constant",
                    vp.format
                ),
            );
            vp.format = vpi::SUPPRESS_VAL;
        }
        return;
    }
    // Modules, memories, iterators and call handles have no readable value.
    if matches!(
        h.kind,
        VpiKind::Module
            | VpiKind::Iterator
            | VpiKind::Memory
            | VpiKind::SysTfCall
            | VpiKind::UserSystf
            | VpiKind::ModPath
            | VpiKind::Tchk
            | VpiKind::InterModPath
            | VpiKind::Callback
    ) {
        vpi_error(
            vpi::ERROR,
            "vpi_get_value: this object has no readable value".into(),
        );
        vp.format = vpi::SUPPRESS_VAL;
        return;
    }
    let sig_id = h.signal_id;

    let ok = try_active_sim("vpi_get_value", |sim| {
        let val = if h.kind == VpiKind::Slice {
            match vpi_slice_read(sim, h) {
                Some(v) => v,
                None => return false,
            }
        } else {
            match vpi_api::cell_value(sim, sig_id) {
                Some(v) => v,
                None => return false,
            }
        };
        let current_time = sim.time;
        // vpiStrengthVal reports the driving strength of a net.
        vpi_api::set_strength_hint(sim, sig_id, h.type_code, vp.format);
        let ok = fill_vpi_value(&val, current_time, Some(h.type_code), vp);
        vpi_api::clear_strength_hint();
        ok
    })
    .unwrap_or(false);

    if !ok {
        // vpiSuppressVal is the only failure channel this function has, and a
        // caller that never checks `format` needs vpi_chk_error to notice.
        vpi_error(
            vpi::ERROR,
            format!("vpi_get_value: cannot supply format {}", vp.format),
        );
        vp.format = vpi::SUPPRESS_VAL;
    }
}

/// Render `val` into `vp` using `vp.format`. Returns false when the
/// format cannot be supplied; the caller then sets `vpiSuppressVal`.
///
/// Shared by `vpi_get_value` and the value-change dispatcher, so a
/// callback sees its trigger object's value in exactly the format a
/// direct read would have produced.
fn fill_vpi_value(
    val: &Value,
    _current_time: u64,
    obj_type_code: Option<libc::c_int>,
    vp: &mut s_vpi_value,
) -> bool {
    {
        // vpiObjTypeVal: the simulator picks the object's natural format
        // and reports which one it chose in `format` (§38.15).
        let mut format = vp.format;
        if format == vpi::OBJ_TYPE_VAL {
            format = vpi_api::obj_type_format(val, obj_type_code);
            vp.format = format;
        }
        // §38.15: a real object is converted to an integer, rounded as
        // §6.12.2 specifies, for every format but vpiRealVal and vpiStringVal.
        let rounded;
        let val = if val.is_real && !vpi_api::format_keeps_real(format) {
            rounded = vpi_api::real_to_integer(val);
            &rounded
        } else {
            val
        };

        match format {
            vpi::INT_VAL => {
                // PLI_INT32: the low 32 bits, X/Z read as 0, a signed value
                // sign-extended. Callers that need the full width must use
                // vpiVectorVal; vpiSize tells them whether they do.
                let n = if val.is_signed {
                    val.to_i64().unwrap_or(0) as u64
                } else {
                    val.to_u64().unwrap_or(0)
                };
                vp.value.integer = (n & 0xFFFF_FFFF) as u32 as libc::c_int;
            }
            vpi::REAL_VAL => vp.value.real = val.to_f64(),
            vpi::SCALAR_VAL => {
                vp.value.scalar = match val.get_bit_code(0) {
                    0 => vpi::SCALAR_0,
                    1 => vpi::SCALAR_1,
                    2 => vpi::SCALAR_X,
                    _ => vpi::SCALAR_Z,
                };
            }
            vpi::VECTOR_VAL => {
                let ptr = VPI_VEC_SCRATCH.with(|cell| {
                    let mut buf = cell.borrow_mut();
                    value_to_vecval(val, &mut buf);
                    buf.as_mut_ptr()
                });
                vp.value.vector = ptr;
            }
            vpi::BIN_STR_VAL => vp.value.str = vpi_str_scratch(&vpi_radix_string(val, 1)),
            vpi::OCT_STR_VAL => vp.value.str = vpi_str_scratch(&vpi_api::radix_string_xz(val, 3)),
            vpi::HEX_STR_VAL => vp.value.str = vpi_str_scratch(&vpi_api::radix_string_xz(val, 4)),
            vpi::DEC_STR_VAL => vp.value.str = vpi_str_scratch(&val.to_dec_string()),
            vpi::STRING_VAL if val.is_real => {
                vp.value.str = vpi_str_scratch(&vpi_api::real_string(val.to_f64()))
            }
            vpi::STRING_VAL => vp.value.str = vpi_str_scratch(&val.to_sv_string()),
            vpi::TIME_VAL => {
                let t = val.to_u64().unwrap_or(0);
                let ptr = VPI_TIME_SCRATCH.with(|cell| {
                    let mut slot = cell.borrow_mut();
                    *slot = s_vpi_time {
                        type_: vpi::SIM_TIME,
                        high: (t >> 32) as u32,
                        low: (t & 0xFFFF_FFFF) as u32,
                        // §37 vpiTimeVal: `.real` must describe the SAME value
                        // as high/low (the value being read, `t`), not the sim
                        // clock. Using `current_time` made the record
                        // self-inconsistent (a `time` var holding 42 read at
                        // tick 5 reported real=5).
                        real: t as f64,
                    };
                    cell.as_ptr()
                });
                vp.value.time = ptr;
            }
            vpi::SUPPRESS_VAL => {} // caller explicitly wants no value
            // vpiStrengthVal, the short/long/shortreal formats and the raw
            // formats; false for anything unknown.
            other => return vpi_api::fill_extra_format(val, obj_type_code, other, vp),
        }
        true
    }
}

/// Write a signal value via VPI (supports force/release).
#[unsafe(no_mangle)]
pub extern "C" fn vpi_put_value(
    handle: *mut libc::c_void,
    value_p: *mut s_vpi_value,
    _time_p: *mut s_vpi_time,
    flags: libc::c_int,
) -> *mut libc::c_void {
    if handle.is_null() {
        return std::ptr::null_mut();
    }

    // Depositing into the call handle sets a system function's return value
    // (IEEE 1800-2017 §38.33). Handled before the simulator lookup because it
    // touches only the call frame.
    {
        let Some(h) = (unsafe { vpi_deref(handle) }) else {
            return std::ptr::null_mut();
        };
        if h.kind == VpiKind::SysTfCall {
            if value_p.is_null() {
                vpi_error(
                    vpi::ERROR,
                    "vpi_put_value: null value on a $systf call".into(),
                );
                return std::ptr::null_mut();
            }
            let w = h.width.max(1);
            let vp = unsafe { &*value_p };
            let v = match vp.format {
                vpi::INT_VAL => Value::from_u64(unsafe { vp.value.integer } as i64 as u64, w),
                vpi::REAL_VAL => Value::from_f64(unsafe { vp.value.real }),
                vpi::VECTOR_VAL => {
                    let ptr = unsafe { vp.value.vector };
                    if ptr.is_null() {
                        vpi_error(
                            vpi::ERROR,
                            "vpi_put_value: null vector on a $systf call".into(),
                        );
                        return std::ptr::null_mut();
                    }
                    let words = (w as usize).div_ceil(32);
                    vecval_to_value(unsafe { std::slice::from_raw_parts(ptr, words) }, w, false)
                }
                other => {
                    vpi_error(
                        vpi::ERROR,
                        format!(
                            "vpi_put_value: format {} unsupported on a $systf call",
                            other
                        ),
                    );
                    return std::ptr::null_mut();
                }
            };
            let frame = h.frame;
            VPI_SYSTF_STACK.with(|st| {
                if let Some(f) = st.borrow_mut().get_mut(frame) {
                    if f.tf_type == vpi::SYS_FUNC {
                        f.ret = Some(v);
                    } else {
                        vpi_error(
                            vpi::ERROR,
                            "vpi_put_value: a $systf TASK has no return value".into(),
                        );
                    }
                }
            });
            return std::ptr::null_mut();
        }
        if h.kind == VpiKind::Constant {
            vpi_error(
                vpi::ERROR,
                "vpi_put_value: a vpiConstant argument is read-only".into(),
            );
            return std::ptr::null_mut();
        }
        // VPI object model (vpi_model.rs): objects with no writable value.
        if vpi_model::put_value_rejected(h) {
            return std::ptr::null_mut();
        }
    }

    try_active_sim("vpi_put_value", |sim| {
        let h = unsafe { &*(handle as *const VpiHandle) };
        if matches!(
            h.kind,
            VpiKind::Module | VpiKind::Iterator | VpiKind::Memory | VpiKind::Callback
        ) {
            vpi_error(
                vpi::ERROR,
                "vpi_put_value: this object has no value; ignored".into(),
            );
            return std::ptr::null_mut();
        }
        // §38.36.2: nothing may be written from a cbReadOnlySynch routine.
        if sim.vpi_in_read_only() {
            vpi_error(
                vpi::ERROR,
                "vpi_put_value: writes are not allowed in the read-only synch region; ignored"
                    .into(),
            );
            return std::ptr::null_mut();
        }
        let sig_id = h.signal_id;
        if matches!(
            h.kind,
            VpiKind::UserSystf | VpiKind::ModPath | VpiKind::Tchk | VpiKind::InterModPath
        ) {
            vpi_error(
                vpi::ERROR,
                "vpi_put_value: this object has no value; ignored".into(),
            );
            return std::ptr::null_mut();
        }

        if flags == vpi::RELEASE_FLAG && is_packed_id(sig_id) {
            // Nothing can force a packed-arena word, so there is nothing to release.
            return std::ptr::null_mut();
        }
        if flags == vpi::RELEASE_FLAG {
            // §38.36.1 cbRelease, once the net has been re-driven.
            if sim.vpi_cb_mask != 0 {
                sim.vpi_queue_release(vpi_cb::CB_RELEASE, Some(sig_id));
            }
            // Release: remove from forced_signals and trigger settle
            sim.forced_signals.remove(&sig_id);
            // Re-evaluate the continuous drivers of the released net.
            sim.redrive_released_signal(sig_id);
            // Release invalidates the event-edge assumption that a flop's
            // output still reflects its last sampled inputs.
            sim.edge_block_snap_valid
                .iter_mut()
                .for_each(|v| *v = false);
            sim.edge_block_armed
                .iter_mut()
                .for_each(|v| *v |= EDGE_ARMED);
            sim.dirty_any = true;
            return std::ptr::null_mut();
        }

        if value_p.is_null() {
            vpi_error(
                vpi::ERROR,
                "vpi_put_value: null value_p with a write flag; ignored".into(),
            );
            return std::ptr::null_mut();
        }

        // A slice decodes at its own width, then splices back into its parent.
        let w = if h.kind == VpiKind::Slice {
            h.width
        } else {
            vpi_api::cell_width(sim, sig_id).unwrap_or(32)
        };
        let is_signed = vpi_api::cell_signed(sim, sig_id);
        let vp = unsafe { &*value_p };

        // A format we cannot decode writes NOTHING. The old code fell
        // through to "read the current value and write it back", which
        // looks like a successful no-op and hides the unsupported format.
        let value = match vp.format {
            vpi::INT_VAL => {
                // PLI_INT32 is signed; widen through i64 so a negative
                // integer sign-extends into a wider signal.
                let ival = unsafe { vp.value.integer } as i64;
                let mut v = Value::from_u64(ival as u64, w);
                v.is_signed = is_signed;
                v
            }
            vpi::REAL_VAL => Value::from_f64(unsafe { vp.value.real }),
            vpi::SCALAR_VAL => {
                let mut v = Value::zero(w);
                let code = match unsafe { vp.value.scalar } {
                    // vpiL / vpiH are the weak 0 / 1 levels.
                    vpi::SCALAR_0 | vpi_api::vc::SCALAR_L => 0,
                    vpi::SCALAR_1 | vpi_api::vc::SCALAR_H => 1,
                    vpi::SCALAR_X => 2,
                    vpi::SCALAR_Z => 3,
                    other => {
                        vpi_error(
                            vpi::ERROR,
                            format!("vpi_put_value: unsupported vpiScalarVal code {}", other),
                        );
                        return std::ptr::null_mut();
                    }
                };
                v.set_bit_code(0, code);
                v.is_signed = is_signed;
                v
            }
            vpi::VECTOR_VAL => {
                // Every word of the signal's width, and both aval and
                // bval, so X and Z survive a deposit. The old code read
                // only `vec[0].aval` for a signal of 64 bits or fewer —
                // dropping the upper word of anything wider than 32 —
                // masked X/Z away as `aval & !bval`, and capped the wide
                // path at four words (128 bits).
                let vec_ptr = unsafe { vp.value.vector };
                if vec_ptr.is_null() {
                    vpi_error(
                        vpi::ERROR,
                        "vpi_put_value: vpiVectorVal with a null vector; ignored".into(),
                    );
                    return std::ptr::null_mut();
                }
                let num_words = (w as usize).div_ceil(32);
                let vec = unsafe { std::slice::from_raw_parts(vec_ptr, num_words) };
                vecval_to_value(vec, w, is_signed)
            }
            // §38.16 string payloads. cocotb writes 4-state values this way
            // (a `LogicArray` carrying x/z has no lossless integer form), so
            // without these every cocotb drive of a clock or reset was
            // rejected here and silently never reached the signal.
            vpi::BIN_STR_VAL
            | vpi::OCT_STR_VAL
            | vpi::HEX_STR_VAL
            | vpi::DEC_STR_VAL
            | vpi::STRING_VAL => {
                let sp = unsafe { vp.value.str };
                if sp.is_null() {
                    vpi_error(
                        vpi::ERROR,
                        format!(
                            "vpi_put_value: null string for format {}; ignored",
                            vp.format
                        ),
                    );
                    return std::ptr::null_mut();
                }
                let text = match unsafe { std::ffi::CStr::from_ptr(sp) }.to_str() {
                    Ok(t) => t.trim(),
                    Err(_) => {
                        vpi_error(
                            vpi::ERROR,
                            "vpi_put_value: string payload is not valid UTF-8; ignored".into(),
                        );
                        return std::ptr::null_mut();
                    }
                };
                let mut v = match vp.format {
                    vpi::BIN_STR_VAL => vpi_value_from_radix_str(text, 1, w),
                    vpi::OCT_STR_VAL => vpi_value_from_radix_str(text, 3, w),
                    vpi::HEX_STR_VAL => vpi_value_from_radix_str(text, 4, w),
                    vpi::DEC_STR_VAL => match text.parse::<i64>() {
                        Ok(n) => Value::from_u64(n as u64, w),
                        Err(_) => {
                            vpi_error(
                                vpi::ERROR,
                                format!("vpi_put_value: '{}' is not a decimal; ignored", text),
                            );
                            return std::ptr::null_mut();
                        }
                    },
                    // vpiStringVal: the payload is CHARACTERS, packed
                    // right-aligned one byte per 8 bits.
                    _ => {
                        let mut sv = Value::zero(w);
                        for (i, byte) in text.as_bytes().iter().rev().enumerate() {
                            for k in 0..8u32 {
                                let bit = i as u32 * 8 + k;
                                if bit >= w {
                                    break;
                                }
                                sv.set_bit_code(bit as usize, ((byte >> k) & 1) as u8);
                            }
                        }
                        sv
                    }
                };
                v.is_signed = is_signed;
                v
            }
            // vpiTimeVal, vpiStrengthVal, the short/long/shortreal formats and
            // the raw formats.
            other => match vpi_api::decode_extra_put(vp, other, w, is_signed) {
                Ok(v) => v,
                Err(msg) => {
                    vpi_error(
                        vpi::ERROR,
                        format!("vpi_put_value: {} (nothing written)", msg),
                    );
                    return std::ptr::null_mut();
                }
            },
        };
        // §6.12.2: an integral value put to a real object converts to real, a
        // real one put to an integral object rounds to an integer.
        let value = vpi_api::fit_put_value(sim, h, value, w, is_signed);

        // Splice a slice back into its parent before it hits the signal table.
        let value = if h.kind == VpiKind::Slice {
            match vpi_slice_write(sim, h, &value) {
                Some(v) => v,
                None => return std::ptr::null_mut(),
            }
        } else {
            value
        };

        // A word of a packed-arena memory has no signal-table slot.
        if is_packed_id(sig_id) {
            if flags == vpi::FORCE_FLAG {
                vpi_error(
                    vpi::ERROR,
                    "vpi_put_value: an element of a packed-arena memory cannot be forced".into(),
                );
            } else {
                sim.cell_write(sig_id, &value);
            }
            return std::ptr::null_mut();
        }

        // For force: write value first, THEN add to forced_signals
        if flags == vpi::FORCE_FLAG {
            // Write directly to signal_table (bypass write_sig! which would skip)
            if sig_id < sim.signal_table.len() {
                sim.signal_table[sig_id] = value.clone();
                sim.sync_mirror(sig_id);
                if sig_id < sim.signal_has_xz.len() {
                    sim.signal_has_xz[sig_id] = if value.raw_bits().1 != 0 { 1u8 } else { 0u8 };
                }
                if sig_id < sim.signal_inline_bits.len() {
                    let (v, x) = value.raw_bits();
                    sim.signal_inline_bits[sig_id] = [v, x];
                }
                sim.after_signal_write(sig_id);
            }
            sim.ctl_note_external_write(sig_id);
            // Now mark as forced (after the write succeeded)
            sim.forced_signals.insert(sig_id, value);
            // §38.36.1 cbForce.
            if sim.vpi_cb_mask != 0 {
                sim.vpi_notify_override(vpi_cb::CB_FORCE, Some(sig_id));
            }
        } else {
            // Normal write - write_sig! will skip if signal is forced
            write_sig!(sim, sig_id, value);
            sim.ctl_note_external_write(sig_id);
            // A VPI deposit is an EXTERNAL write: no HDL driver stands behind
            // it, so nothing else marks the signal dirty or re-snapshots it for
            // edge detection. Without this a cocotb-driven clock landed in the
            // signal table and propagated nowhere — `always @(posedge clk)` and
            // even `always @(clk)` never fired, so the DUT sat at x while the
            // Python side saw its own writes read back correctly.
            sim.after_signal_write(sig_id);
            if sig_id < sim.dirty_signals.len() && !sim.dirty_signals[sig_id] {
                sim.dirty_signals[sig_id] = true;
                sim.dirty_list.push(sig_id);
            }
            sim.dirty_any = true;
        }

        std::ptr::null_mut()
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Read the current simulation time.
///
/// `object` is accepted for IEEE compatibility and currently ignored.
/// `time_p->type` chooses the requested representation:
/// - `vpiSimTime`: fill `high`/`low` and also mirror into `real`
/// - `vpiScaledRealTime`: fill `real` and also mirror into `high`/`low`
#[unsafe(no_mangle)]
pub extern "C" fn vpi_get_time(_object: *mut libc::c_void, time_p: *mut s_vpi_time) {
    if time_p.is_null() {
        return;
    }

    let now = try_active_sim("vpi_get_time", |sim| sim.time).unwrap_or(0);
    let tp = unsafe { &mut *time_p };
    match tp.type_ {
        vpi::SCALED_REAL_TIME | vpi::SIM_TIME | vpi::SUPPRESS_TIME => {}
        _ => tp.type_ = vpi::SIM_TIME,
    }

    tp.high = (now >> 32) as u32;
    tp.low = (now & 0xFFFF_FFFF) as u32;
    tp.real = now as f64;
}

// ============================================================================
// VPI/DPI surface
// ============================================================================
// The functions below are the minimum subset of IEEE 1800 §35 (DPI) and
// §38 (VPI) needed for standard DPI/VPI libraries to compile and link
// against xezim. They are intentionally simple — each does exactly what
// the VPI/DPI interface specifies. Symbols not exercised are stubbed.
//
// Naming matches the C standard exactly; every function is `#[no_mangle]
// pub extern "C"` so it lands in the simulator's symbol table and is
// resolvable by `dlsym` when DPI imports fire.

// --- vpi_get_vlog_info ------------------------------------------------------
//
// VPI callers call this once at startup to discover the tool name + version.
// We hand back the argv passed via Simulator::set_args (the CString pointers
// stay valid for the simulator lifetime) and a static product/version string.
//
// `info_p` must be allocated by the caller; we fill it in place.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_get_vlog_info(info_p: *mut s_vpi_vlog_info) -> libc::c_int {
    if info_p.is_null() {
        return 0;
    }
    let info = unsafe { &mut *info_p };
    info.argc = 0;
    info.argv = std::ptr::null_mut();
    info.product = b"xezim\0".as_ptr() as *mut libc::c_char;
    // The crate version, so this never drifts from the binary again. The
    // NUL is appended by concat! since env! is a &'static str.
    info.version = concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr() as *mut libc::c_char;

    try_active_sim("vpi_get_vlog_info", |sim| {
        info.argc = sim.vpi_argv.len() as libc::c_int;
        if !sim.vpi_argv.is_empty() {
            info.argv = sim.vpi_argv.as_mut_ptr();
        }
        1
    })
    .unwrap_or(0)
}

// --- vpi_release_handle -----------------------------------------------------
//
// Used by VPI code paths; semantically equivalent to `vpi_free_object`.
// We just delegate to the existing free function.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_release_handle(handle: *mut libc::c_void) -> libc::c_int {
    vpi_free_object(handle)
}

// --- svDpiVersion -----------------------------------------------------------
//
// Returns the DPI version string. Used for log banners and to
// decide which DPI features are available.
#[unsafe(no_mangle)]
pub extern "C" fn svDpiVersion() -> *const libc::c_char {
    // Static C string — no lifetime concerns.
    b"1800-2017\0".as_ptr() as *const libc::c_char
}

// --- svGetScopeFromName -----------------------------------------------------
//
// Returns an opaque handle to the named scope (a module, package, or
// interface instance). DPI code calls this with scope names (e.g. `pkg`,
// `top.dut`). We cache the handles in a HashMap so the same scope always
// returns the same pointer (svSetScope+svGetScope rely on this for equality).
//
// If the scope name is not found we still return a non-null handle
// pointing at a new `DpiScope` — synthetic scopes work for round-tripping
// through `svGetNameFromScope`.
#[unsafe(no_mangle)]
pub extern "C" fn svGetScopeFromName(name: *const libc::c_char) -> *mut libc::c_void {
    if name.is_null() {
        return std::ptr::null_mut();
    }
    let name_str = unsafe { std::ffi::CStr::from_ptr(name) }
        .to_string_lossy()
        .trim()
        .to_string();

    with_active_sim(|sim| {
        if let Some(&existing) = sim.dpi_scopes.get(&name_str) {
            return existing;
        }
        let mut scope = DpiScope {
            name: [0u8; 256],
            name_len: name_str.len().min(255),
        };
        let bytes = name_str.as_bytes();
        scope.name[..scope.name_len].copy_from_slice(&bytes[..scope.name_len]);
        let ptr = Box::into_raw(Box::new(scope)) as *mut libc::c_void;
        sim.dpi_scopes.insert(name_str, ptr);
        ptr
    })
}

// --- svGetNameFromScope -----------------------------------------------------
//
// Recovers the scope name from a handle. DPI code round-trips scope handles
// through this when emitting diagnostics.
#[unsafe(no_mangle)]
pub extern "C" fn svGetNameFromScope(scope: *mut libc::c_void) -> *const libc::c_char {
    if scope.is_null() {
        return std::ptr::null();
    }
    unsafe {
        let s = &*(scope as *const DpiScope);
        // The name is stored as fixed-size [u8; 256] with a length prefix, so
        // it is not NUL-terminated in our storage and we must hand back a
        // fresh buffer. The CALLER FREES IT WITH libc `free` (see
        // tests/dpi/uvm_dpi_test.c), so the buffer MUST come from libc
        // `malloc`. Returning a Rust-allocated `CString::into_raw()` only
        // worked while Rust's global allocator happened to BE glibc malloc;
        // installing any other global allocator turned it into an invalid
        // free. UVM calls this sparingly, so the copy is negligible.
        let bytes = &s.name[..s.name_len];
        // Preserve the previous `CString::new(..).unwrap_or_default()`
        // behaviour: an interior NUL yields an empty string.
        let n = if bytes.contains(&0) { 0 } else { bytes.len() };
        let buf = libc::malloc(n + 1) as *mut u8;
        if buf.is_null() {
            return std::ptr::null();
        }
        if n > 0 {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, n);
        }
        *buf.add(n) = 0;
        buf as *const libc::c_char
    }
}

// --- svGetScope -------------------------------------------------------------
//
// Returns the currently-active scope set by `svSetScope`. Used when an import
// is called and the DPI runtime needs to know "where am I?".
#[unsafe(no_mangle)]
pub extern "C" fn svGetScope() -> *mut libc::c_void {
    ACTIVE_SCOPE.with(|cell| cell.get())
}

// --- svSetScope -------------------------------------------------------------
//
// Updates the currently-active scope. xezim's exec_dpi_import_call
// sets the scope to the import's declaration site before invoking the
// C function, then restores the previous value on the way out.
#[unsafe(no_mangle)]
pub extern "C" fn svSetScope(scope: *mut libc::c_void) -> *mut libc::c_void {
    let prev = ACTIVE_SCOPE.with(|cell| cell.get());
    ACTIVE_SCOPE.with(|cell| cell.set(scope));
    prev
}

// --- svGetTime / svGetTimeUnit / svGetTimePrecision -------------------------
//
// The time queries for DPI code (#206). A scope naming a module definition or
// an instance path answers with that module's timescale; NULL answers with the
// simulation's (its finest precision). Exponents are powers of ten in seconds
// (-9 = 1 ns), as `vpi_get(vpiTimeUnit, ...)` reports them.

fn dpi_scope_timescale(sim: &Simulator, scope: *mut libc::c_void) -> (i32, i32) {
    if scope.is_null() {
        let exp = secs_to_exp(sim.module.tick_s);
        return (exp, exp);
    }
    let s = unsafe { &*(scope as *const DpiScope) };
    let name = String::from_utf8_lossy(&s.name[..s.name_len]).into_owned();
    let rel = vpi_strip_top(sim, &name);
    let def = if rel.is_empty() || name == sim.module.name {
        sim.module.name.clone()
    } else if let Some(inst) = sim.module.instances.iter().find(|i| i.path == rel) {
        inst.def_name.clone()
    } else if sim.module.module_timescale_exp.contains_key(name.as_str()) {
        name
    } else {
        sim.module.name.clone()
    };
    sim.reported_timescale_exp(&def)
}

#[unsafe(no_mangle)]
pub extern "C" fn svGetTimeUnit(scope: *mut libc::c_void, time_unit: *mut i32) -> libc::c_int {
    if time_unit.is_null() {
        return -1;
    }
    match try_active_sim("svGetTimeUnit", |sim| dpi_scope_timescale(sim, scope).0) {
        Some(u) => {
            unsafe { *time_unit = u };
            0
        }
        None => -1,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn svGetTimePrecision(
    scope: *mut libc::c_void,
    time_precision: *mut i32,
) -> libc::c_int {
    if time_precision.is_null() {
        return -1;
    }
    match try_active_sim("svGetTimePrecision", |sim| {
        dpi_scope_timescale(sim, scope).1
    }) {
        Some(p) => {
            unsafe { *time_precision = p };
            0
        }
        None => -1,
    }
}

/// Current simulation time. `time->type` selects the form: `vpiSimTime`
/// fills `high`/`low` with simulation ticks, `vpiScaledRealTime` fills
/// `real` in the scope's time unit.
#[unsafe(no_mangle)]
pub extern "C" fn svGetTime(scope: *mut libc::c_void, time: *mut s_vpi_time) -> libc::c_int {
    if time.is_null() {
        return -1;
    }
    let Some((ticks, tick_s, unit)) = try_active_sim("svGetTime", |sim| {
        (
            sim.time,
            sim.module.tick_s,
            dpi_scope_timescale(sim, scope).0,
        )
    }) else {
        return -1;
    };
    let t = unsafe { &mut *time };
    if t.type_ == vpi::SCALED_REAL_TIME {
        t.real = ticks as f64 * tick_s / 10f64.powi(unit);
    } else {
        t.type_ = vpi::SIM_TIME;
        t.high = (ticks >> 32) as u32;
        t.low = (ticks & 0xFFFF_FFFF) as u32;
    }
    0
}

// --- Multichannel descriptors and output (#205) -----------------------------
//
// vpi_mcd_open/close/flush/name share the channel table `$fopen` uses, so a
// descriptor opened from C can be written from SystemVerilog and the other way
// round. vpi_mcd_printf is C-variadic, so it lives in vpi_printf_shim.c, which
// formats the text and hands the file channels to xezim_vpi_mcd_write.

#[unsafe(no_mangle)]
pub extern "C" fn vpi_mcd_open(name: *const libc::c_char) -> u32 {
    if name.is_null() {
        return 0;
    }
    let path = unsafe { std::ffi::CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned();
    try_active_sim("vpi_mcd_open", |sim| {
        sim.open_file_path(&path, true, "w").to_u64().unwrap_or(0) as u32
    })
    .unwrap_or(0)
}

/// Closes the file channels in `mcd`; bit 0 (stdout) stays open. Returns 0.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_mcd_close(mcd: u32) -> u32 {
    let files = mcd & !1;
    if files == 0 {
        return 0;
    }
    try_active_sim("vpi_mcd_close", |sim| {
        sim.close_channels(files as i32);
        0
    })
    .unwrap_or(mcd)
}

#[unsafe(no_mangle)]
pub extern "C" fn vpi_mcd_flush(mcd: u32) -> libc::c_int {
    if mcd == 0 {
        return 0;
    }
    try_active_sim("vpi_mcd_flush", |sim| {
        sim.flush_channels(mcd as i32);
        0
    })
    .unwrap_or(1)
}

thread_local! {
    static VPI_MCD_NAME: RefCell<std::ffi::CString> = RefCell::new(std::ffi::CString::default());
}

/// The file name behind the lowest channel set in `cd` ("stdout" for bit 0),
/// or NULL for a channel that is not open. Valid until the next call.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_mcd_name(cd: u32) -> *mut libc::c_char {
    if cd == 0 {
        return std::ptr::null_mut();
    }
    let bit = cd.trailing_zeros() as i32;
    let name = if bit == 0 {
        Some("stdout".to_string())
    } else {
        try_active_sim("vpi_mcd_name", |sim| sim.mcd_names.get(&bit).cloned()).flatten()
    };
    match name {
        Some(n) => VPI_MCD_NAME.with(|c| {
            *c.borrow_mut() = std::ffi::CString::new(n).unwrap_or_default();
            c.borrow().as_ptr() as *mut libc::c_char
        }),
        None => std::ptr::null_mut(),
    }
}

/// Backend of `vpi_mcd_printf` for the file channels in `mcd` (the shim
/// prints bit 0 itself, next to `vpi_printf`).
#[unsafe(no_mangle)]
pub extern "C" fn xezim_vpi_mcd_write(
    mcd: u32,
    buf: *const libc::c_char,
    len: libc::c_int,
) -> libc::c_int {
    let files = mcd & !1;
    if buf.is_null() || len <= 0 || files == 0 {
        return 0;
    }
    let bytes = unsafe { std::slice::from_raw_parts(buf as *const u8, len as usize) };
    let text = String::from_utf8_lossy(bytes).into_owned();
    try_active_sim("vpi_mcd_printf", |sim| {
        sim.write_channels(files as i32, &text);
        len
    })
    .unwrap_or(0)
}

/// Flushes the simulator's output: stdout and every open file. Returns 0.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_flush() -> libc::c_int {
    try_active_sim("vpi_flush", |sim| {
        sim.flush_channels(0);
        0
    })
    .unwrap_or(0)
}

/// 1 when both handles refer to the same object, else 0.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_compare_objects(a: *mut libc::c_void, b: *mut libc::c_void) -> libc::c_int {
    if a.is_null() || b.is_null() {
        return 0;
    }
    if a == b {
        return 1;
    }
    let (Some(x), Some(y)) = (unsafe { vpi_deref(a) }, unsafe { vpi_deref(b) }) else {
        return 0;
    };
    if x.kind == VpiKind::Iterator {
        return 0;
    }
    i32::from(
        x.kind == y.kind
            && x.signal_id == y.signal_id
            && x.inst_idx == y.inst_idx
            && x.lsb == y.lsb
            && x.width == y.width
            && x.full_name == y.full_name,
    )
}

/// `vpi_get` with a 64-bit result.
#[unsafe(no_mangle)]
pub extern "C" fn vpi_get64(property: libc::c_int, handle: *mut libc::c_void) -> i64 {
    i64::from(vpi_get(property, handle))
}

// =========================================================================
// IEEE 1800-2017 §18.4 / §18.5.9 — rand COLLECTION members.
// =========================================================================

/// The three storage shapes a rand collection member can have.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum CollKind {
    /// `bit [7:0] a[4]` — fixed unpacked array, bounds known at elaboration.
    Fixed,
    /// `bit [7:0] a[]` / `a[$]` — dynamic array or queue (length is solved).
    Dyn,
    /// `bit [7:0] a[int]` — associative array (sparse; keys are pre-seeded).
    Assoc,
}

/// One rand collection member of the object being randomized.
#[derive(Clone, Debug)]
struct RandColl {
    /// Bare member name (`dyn_arr`).
    prop: String,
    /// Instance-scoped storage prefix (`<handle>#dyn_arr`).
    scoped: String,
    kind: CollKind,
    /// Element width in bits.
    width: u32,
    /// Fixed-array bounds (unused for Dyn/Assoc).
    lo: i64,
    hi: i64,
    /// Dynamic array of dynamic arrays (`bit [7:0] m[][]`).
    nested: bool,
    /// Element type is a CLASS (`rand obj arr[]`). Such elements are object
    /// handles randomized RECURSIVELY (§18.4.1), never drawn as scalars —
    /// a fresh random draw would overwrite the handle with junk, destroying
    /// the object reference and any aliasing (`arr[1]==arr[2]`).
    is_object_elem: bool,
}

/// A `.size()` constraint on a rand dynamic array (§18.4): the array must be
/// SIZED before its elements can be solved.
enum SizeCon {
    /// `a.size() == E` — E already evaluated.
    Eq(u64),
    /// `a.size() inside {…}` — the ranges already evaluated.
    In(Vec<(u64, u64)>),
    /// `a.size() < E`, `>= E`, … — E already evaluated.
    Rel(crate::ast::expr::BinaryOp, u64),
}

// ===========================================================================
// Inline (<=64-bit) fast paths for the bytecode VM's hot ALU arms.
//
// `exec_insns` used to write EVERY result as
//     self.vm_regs[d] = self.vm_regs[l].op(&self.vm_regs[r]);
// which is a call that CONSTRUCTS a fresh 32-byte `Value`, a 32-byte move
// into the destination register, and a `Drop` of the destination's previous
// contents (a branch on the storage discriminant plus `free` when it was
// `Wide`). For the overwhelmingly common case — both operands in `Inline`
// storage — the whole result is two `u64`s plus a width and a sign flag, so
// the destination can simply be overwritten in place.
//
// Each `vm_*` helper below mirrors the corresponding `Value` method EXACTLY
// for the operand shapes it accepts, and returns `None` for every other shape
// so the caller falls back to that same method. That keeps the 4-state
// (0/1/X/Z) rules in one place: anything not reproduced bit-for-bit here is
// simply not handled here. `vm_fastpath_tests` below checks the equivalence
// exhaustively over all 4-state patterns of widths 1..=5.
// ===========================================================================

/// `Value::mask` — bit mask for the valid bits of an inline value.
#[inline(always)]
fn vm_mask(width: u32) -> u64 {
    if width >= 64 {
        u64::MAX
    } else {
        (1u64 << width) - 1
    }
}

/// `(val_bits, xz_bits, width, is_signed)` for a value eligible for an inline
/// fast path: `Inline` storage, not a real, not a §5.7.1 fill, width <= 64.
/// Rejecting a shape is always safe — the caller then runs the original
/// `Value` method, which is where every rejected shape was handled before.
#[inline(always)]
fn vm_hdr(v: &Value) -> Option<(u64, u64, u32, bool)> {
    if v.is_real || v.is_fill || v.width > 64 {
        return None;
    }
    match v.inline_bits() {
        Some((val, xz)) => Some((val, xz, v.width, v.is_signed)),
        None => None,
    }
}

/// Overwrite `dst` in place with an inline result — the whole point of this
/// module. Only a destination that currently holds `Wide` storage needs the
/// old construct/move/drop, and that path also frees the heap buffer.
#[inline(always)]
fn vm_store(dst: &mut Value, val: u64, xz: u64, width: u32, signed: bool) {
    if dst.set_inline_bits(val, xz) {
        dst.width = width;
        dst.is_signed = signed;
        dst.is_real = false;
        dst.is_fill = false;
    } else {
        let mut v = Value::from_inline(val, xz, width);
        v.is_signed = signed;
        *dst = v;
    }
}

/// `Value::to_u64().unwrap_or(0)` on inline bits.
#[inline(always)]
fn vm_to_u64(val: u64, xz: u64) -> u64 {
    val & !xz
}

/// `Value::to_i64().unwrap_or(0) as u64` on inline bits.
#[inline(always)]
fn vm_to_i64(val: u64, xz: u64, width: u32, signed: bool) -> u64 {
    let raw = val & !xz;
    if signed && width > 0 && width < 64 && (raw >> (width - 1)) & 1 == 1 {
        raw | !vm_mask(width)
    } else {
        raw
    }
}

/// `Value::operand_bits_u64`.
#[inline(always)]
fn vm_operand_bits(
    val: u64,
    xz: u64,
    w_self: u32,
    signed_self: bool,
    signed_expr: bool,
    w: u32,
) -> u64 {
    if signed_expr && w_self < w && w_self < 64 {
        vm_to_i64(val, xz, w_self, signed_self)
    } else {
        vm_to_u64(val, xz)
    }
}

/// `Value::resize`'s inline head, as raw bits. `None` for the shapes that
/// head sends to `resize_slow`.
#[inline(always)]
fn vm_resize_bits(val: u64, xz: u64, width: u32, signed: bool, target: u32) -> Option<(u64, u64)> {
    if target == 0 || target > 64 {
        return None;
    }
    if target == width {
        return Some((val, xz));
    }
    let mask = vm_mask(target);
    if target < width {
        return Some((val & mask, xz & mask));
    }
    // Widen: sign-extend only for a signed source whose MSB is a KNOWN 1
    // (an X/Z MSB does not replicate here — that is `resize_for_assign`'s job).
    let mut v = val;
    if signed
        && width > 0
        && width <= 64
        && (xz >> (width - 1)) & 1 == 0
        && (val >> (width - 1)) & 1 == 1
    {
        v |= mask & !vm_mask(width);
    }
    Some((v, xz))
}

/// `Value::add` (`sub == false`) / `Value::sub` (`sub == true`).
#[inline(always)]
fn vm_add_sub(a: &Value, b: &Value, sub: bool) -> Option<(u64, u64, u32, bool)> {
    let (av, ax, aw, asg) = vm_hdr(a)?;
    let (bv, bx, bw, bsg) = vm_hdr(b)?;
    let w = aw.max(bw);
    if ax != 0 || bx != 0 {
        // `Value::new(w)`: all-X, unsigned.
        return Some((0, vm_mask(w), w, false));
    }
    let signed = asg && bsg;
    let x = vm_operand_bits(av, ax, aw, asg, signed, w);
    let y = vm_operand_bits(bv, bx, bw, bsg, signed, w);
    let r = if sub {
        x.wrapping_sub(y)
    } else {
        x.wrapping_add(y)
    };
    Some((r & vm_mask(w), 0, w, signed))
}

/// `Value::bitwise_and`.
#[inline(always)]
fn vm_bitand(a: &Value, b: &Value) -> Option<(u64, u64, u32, bool)> {
    let (av, ax, aw, _) = vm_hdr(a)?;
    let (bv, bx, bw, _) = vm_hdr(b)?;
    let w = aw.max(bw);
    if ax == 0 && bx == 0 {
        Some((av & bv, 0, w, false))
    } else {
        let any_xz = ax | bx;
        let result_val = av & bv & !any_xz;
        let result_xz = any_xz & !((!av & !ax) | (!bv & !bx)); // known 0 kills X
        Some((result_val, result_xz & vm_mask(w), w, false))
    }
}

/// `Value::bitwise_or`.
#[inline(always)]
fn vm_bitor(a: &Value, b: &Value) -> Option<(u64, u64, u32, bool)> {
    let (av, ax, aw, _) = vm_hdr(a)?;
    let (bv, bx, bw, _) = vm_hdr(b)?;
    let w = aw.max(bw);
    if ax == 0 && bx == 0 {
        Some((av | bv, 0, w, false))
    } else {
        let any_xz = ax | bx;
        let result_val = (av | bv) & !any_xz;
        let known_one = (av & !ax) | (bv & !bx);
        let result_xz = any_xz & !known_one; // known 1 kills X
        Some((result_val | known_one, result_xz & vm_mask(w), w, false))
    }
}

/// `Value::bitwise_xor`.
#[inline(always)]
fn vm_bitxor(a: &Value, b: &Value) -> Option<(u64, u64, u32, bool)> {
    let (av, ax, aw, _) = vm_hdr(a)?;
    let (bv, bx, bw, _) = vm_hdr(b)?;
    let w = aw.max(bw);
    let any_xz = ax | bx;
    Some(((av ^ bv) & !any_xz, any_xz & vm_mask(w), w, false))
}

/// `Value::bitwise_not` (keeps `is_signed`, unlike the binary bitwise ops).
#[inline(always)]
fn vm_bitnot(a: &Value) -> Option<(u64, u64, u32, bool)> {
    let (av, ax, aw, asg) = vm_hdr(a)?;
    Some(((!av & !ax) & vm_mask(aw), ax, aw, asg))
}

/// `Value::bitwise_xor(..).bitwise_not()` — the `BitXnor` arm's composition.
#[inline(always)]
fn vm_bitxnor(a: &Value, b: &Value) -> Option<(u64, u64, u32, bool)> {
    let (v, x, w, s) = vm_bitxor(a, b)?;
    Some(((!v & !x) & vm_mask(w), x, w, s))
}

/// `Value::is_equal` restricted to X-free operands. The X case returns `None`
/// so the LRM's bit-by-bit ambiguity rule stays in the original method.
#[inline(always)]
fn vm_is_equal(a: &Value, b: &Value) -> Option<bool> {
    let (av, ax, aw, asg) = vm_hdr(a)?;
    let (bv, bx, bw, bsg) = vm_hdr(b)?;
    if ax != 0 || bx != 0 {
        return None;
    }
    // §11.6.1/§11.8.2: the comparison is SIGNED only when BOTH operands are
    // signed; if either is unsigned the propagated type is unsigned and the
    // narrower operand is ZERO-extended. Mirrors `Value::is_equal` — this fast
    // path is asserted equal to it by `binary_ops_match_value_methods`, which
    // is what caught the two implementations drifting apart.
    if aw != bw {
        let w = aw.max(bw);
        let both_signed = asg && bsg;
        let (ra, rax) = vm_resize_bits(av, ax, aw, both_signed, w)?;
        let (rb, rbx) = vm_resize_bits(bv, bx, bw, both_signed, w)?;
        Some(vm_to_u64(ra, rax) == vm_to_u64(rb, rbx))
    } else {
        Some(vm_to_u64(av, ax) == vm_to_u64(bv, bx))
    }
}

/// `Value::case_eq`'s own inline fast path, reproduced verbatim so the result
/// can be stored in place instead of materialised as a `Value`.
#[inline(always)]
fn vm_case_eq(a: &Value, b: &Value) -> Option<bool> {
    if a.is_fill || b.is_fill {
        return None;
    }
    let (mut av, mut ax) = a.inline_bits()?;
    let (mut bv, mut bx) = b.inline_bits()?;
    let (aw, bw) = (a.width, b.width);
    let w = aw.max(bw);
    let mask = vm_mask(w);
    if a.is_signed && b.is_signed {
        if aw > 0 && aw < w {
            let ext = mask & !vm_mask(aw);
            let sign = 1u64 << (aw - 1);
            if av & sign != 0 {
                av |= ext;
            }
            if ax & sign != 0 {
                ax |= ext;
            }
        }
        if bw > 0 && bw < w {
            let ext = mask & !vm_mask(bw);
            let sign = 1u64 << (bw - 1);
            if bv & sign != 0 {
                bv |= ext;
            }
            if bx & sign != 0 {
                bx |= ext;
            }
        }
    }
    Some((av & mask) == (bv & mask) && (ax & mask) == (bx & mask))
}

/// `Value::less_than` (`or_equal == false`) / `Value::less_equal`.
/// Returns the (val_bits, xz_bits) of the 1-bit result.
#[inline(always)]
fn vm_less(a: &Value, b: &Value, or_equal: bool) -> Option<(u64, u64)> {
    let (av, ax, aw, asg) = vm_hdr(a)?;
    let (bv, bx, bw, bsg) = vm_hdr(b)?;
    if ax != 0 || bx != 0 {
        return Some((0, 1)); // `Value::new(1)`
    }
    // §5.5.1: signed compare only when BOTH operands are signed.
    let r = if asg && bsg {
        let x = vm_to_i64(av, ax, aw, asg) as i64;
        let y = vm_to_i64(bv, bx, bw, bsg) as i64;
        if or_equal { x <= y } else { x < y }
    } else {
        let x = vm_to_u64(av, ax);
        let y = vm_to_u64(bv, bx);
        if or_equal { x <= y } else { x < y }
    };
    Some((r as u64, 0))
}

/// `Value::is_nonzero` on inline bits: 0 = false, 1 = true, 2 = unknown.
#[inline(always)]
fn vm_nonzero(val: u64, xz: u64) -> u8 {
    if val & !xz != 0 {
        1
    } else if xz != 0 {
        2
    } else {
        0
    }
}

/// `Value::logic_and` / `Value::logic_or`, as the 1-bit result's raw bits.
#[inline(always)]
fn vm_logic2(a: &Value, b: &Value, is_or: bool) -> Option<(u64, u64)> {
    let (av, ax, _, _) = vm_hdr(a)?;
    let (bv, bx, _, _) = vm_hdr(b)?;
    let p = vm_nonzero(av, ax);
    let q = vm_nonzero(bv, bx);
    Some(if is_or {
        if p == 1 || q == 1 {
            (1, 0)
        } else if p == 0 && q == 0 {
            (0, 0)
        } else {
            (0, 1)
        }
    } else {
        if p == 1 && q == 1 {
            (1, 0)
        } else if p == 0 || q == 0 {
            (0, 0)
        } else {
            (0, 1)
        }
    })
}

/// `Value::logic_not`, as the 1-bit result's raw bits.
#[inline(always)]
fn vm_logic_not(a: &Value) -> Option<(u64, u64)> {
    let (av, ax, _, _) = vm_hdr(a)?;
    Some(match vm_nonzero(av, ax) {
        1 => (0, 0),
        0 => (1, 0),
        _ => (0, 1),
    })
}

/// `Value::bit_select`'s inline head, as the 1-bit result's raw bits.
/// Deliberately does NOT test `is_fill`/`is_real`: neither does that head,
/// and a divergence there would change §5.7.1 fill behaviour.
#[inline(always)]
fn vm_bit_select(v: &Value, index: usize) -> Option<(u64, u64)> {
    let (val, xz) = v.inline_bits()?;
    if index < v.width as usize && index < 64 {
        Some(((val >> index) & 1, (xz >> index) & 1))
    } else {
        None
    }
}

/// `Value::concat_refs(repeat_n(src, n))` restricted to a result that fits
/// inline. Every part is the SAME value, so the width sum `concat_refs` walks
/// its iterator to compute is one multiply here, and the fold needs no
/// iterator. `None` (result wider than 64 bits, or a width multiply that
/// overflows) falls back to `concat_refs` itself.
#[inline(always)]
fn vm_replicate(src: &Value, n: usize) -> Option<(u64, u64, u32)> {
    let w = src.width;
    let total = w.checked_mul(n as u32)?;
    if total > 64 {
        return None;
    }
    let (v, x) = src.raw_bits();
    let mask = vm_mask(w);
    let (mut ov, mut ox, mut off) = (0u64, 0u64, 0u32);
    // A zero-width part is SKIPPED by `concat_refs`, leaving the accumulator
    // untouched — so `total == 0` yields a zero-width, all-clear value.
    if w != 0 {
        for _ in 0..n {
            ov |= (v & mask) << off;
            ox |= (x & mask) << off;
            off += w;
        }
    }
    Some((ov, ox, total))
}

/// `Value::range_select`'s inline head, as (val_bits, xz_bits, width).
#[inline(always)]
fn vm_range_select(v: &Value, left: usize, right: usize) -> Option<(u64, u64, u32)> {
    if v.is_fill {
        return None;
    }
    let (val, xz) = v.inline_bits()?;
    let (lo, hi) = if left >= right {
        (right, left)
    } else {
        (left, right)
    };
    if hi < 64 && hi < v.width as usize {
        let width = hi - lo + 1;
        let mask = if width == 64 {
            u64::MAX
        } else {
            (1u64 << width) - 1
        };
        Some((((val >> lo) & mask), ((xz >> lo) & mask), width as u32))
    } else {
        None
    }
}

#[cfg(test)]
mod constraint_set_tests;

#[cfg(test)]
mod two_state_dynamic_tests;

#[cfg(test)]
mod vm_fastpath_tests {
    use super::*;

    /// §9.6.2: terminating a parked process releases its captured value.
    #[test]
    fn canceled_property_waits_release_values() {
        let sim = crate::simulate(
            r#"
class watch_cell; bit flag; endclass
module top;
  watch_cell watched = new();
  initial begin
    repeat (16) begin
      fork begin @(watched.flag); $fatal(1, "unexpected wake"); end join_none
      #1;
      disable fork;
    end
    $finish;
  end
endmodule
"#,
            100,
        )
        .expect("simulate");
        assert!(sim.vc_saved.is_empty(), "canceled value captures remain");
    }

    /// Every 4-state pattern of `width` bits, as (val_bits, xz_bits).
    fn patterns(width: u32) -> Vec<(u64, u64)> {
        let mut out = Vec::new();
        let n = width as usize;
        for code in 0..4usize.pow(n as u32) {
            let (mut v, mut x) = (0u64, 0u64);
            for i in 0..n {
                match (code >> (2 * i)) & 3 {
                    0 => {}           // 0
                    1 => v |= 1 << i, // 1
                    2 => x |= 1 << i, // X
                    _ => {
                        v |= 1 << i;
                        x |= 1 << i;
                    } // Z
                }
            }
            out.push((v, x));
        }
        out
    }

    fn mk(v: u64, x: u64, w: u32, signed: bool) -> Value {
        let mut val = Value::from_inline(v, x, w);
        val.is_signed = signed;
        val
    }

    /// All (value, signedness) operands over widths 1..=5.
    fn operands() -> Vec<Value> {
        let mut out = Vec::new();
        for w in 1..=5u32 {
            for (v, x) in patterns(w) {
                out.push(mk(v, x, w, false));
                out.push(mk(v, x, w, true));
            }
        }
        out
    }

    /// A fast-path result and the reference `Value` must agree on the bits AND
    /// on every header field the VM later reads back.
    fn same(fast: (u64, u64, u32, bool), reference: &Value) {
        let (fv, fx, fw, fs) = fast;
        let (rv, rx) = reference.raw_bits();
        let m = vm_mask(reference.width);
        assert_eq!(fw, reference.width, "width");
        assert_eq!(fs, reference.is_signed, "is_signed");
        assert!(!reference.is_real && !reference.is_fill);
        assert_eq!(fv & m, rv & m, "val_bits");
        assert_eq!(fx & m, rx & m, "xz_bits");
        // The unmasked words must match too — `to_u64`/`is_nonzero`/`has_xz`
        // all read them without masking.
        assert_eq!(fv, rv, "val_bits (unmasked)");
        assert_eq!(fx, rx, "xz_bits (unmasked)");
    }

    #[test]
    fn binary_ops_match_value_methods() {
        let ops = operands();
        for a in &ops {
            for b in &ops {
                if let Some(r) = vm_add_sub(a, b, false) {
                    same(r, &a.add(b));
                }
                if let Some(r) = vm_add_sub(a, b, true) {
                    same(r, &a.sub(b));
                }
                if let Some(r) = vm_bitand(a, b) {
                    same(r, &a.bitwise_and(b));
                }
                if let Some(r) = vm_bitor(a, b) {
                    same(r, &a.bitwise_or(b));
                }
                if let Some(r) = vm_bitxor(a, b) {
                    same(r, &a.bitwise_xor(b));
                }
                if let Some(r) = vm_bitxnor(a, b) {
                    same(r, &a.bitwise_xor(b).bitwise_not());
                }
                if let Some(e) = vm_is_equal(a, b) {
                    same((e as u64, 0, 1, false), &a.is_equal(b));
                    same((!e as u64, 0, 1, false), &a.is_not_equal(b));
                }
                if let Some(e) = vm_case_eq(a, b) {
                    same((e as u64, 0, 1, false), &a.case_eq(b));
                }
                if let Some((v, x)) = vm_less(a, b, false) {
                    same((v, x, 1, false), &a.less_than(b));
                    same((v, x, 1, false), &b.greater_than(a));
                }
                if let Some((v, x)) = vm_less(a, b, true) {
                    same((v, x, 1, false), &a.less_equal(b));
                    same((v, x, 1, false), &b.greater_equal(a));
                }
                if let Some((v, x)) = vm_logic2(a, b, false) {
                    same((v, x, 1, false), &a.logic_and(b));
                }
                if let Some((v, x)) = vm_logic2(a, b, true) {
                    same((v, x, 1, false), &a.logic_or(b));
                }
            }
        }
    }

    #[test]
    fn unary_ops_and_resize_match_value_methods() {
        for a in operands() {
            if let Some(r) = vm_bitnot(&a) {
                same(r, &a.bitwise_not());
            }
            if let Some((v, x)) = vm_logic_not(&a) {
                same((v, x, 1, false), &a.logic_not());
            }
            let (av, ax, aw, asg) = vm_hdr(&a).unwrap();
            for target in 0..=9u32 {
                if let Some((rv, rx)) = vm_resize_bits(av, ax, aw, asg, target) {
                    let reference = a.resize(target);
                    same((rv, rx, target, a.is_signed), &reference);
                }
            }
        }
    }

    #[test]
    fn selects_match_value_methods() {
        for a in operands() {
            for i in 0..8usize {
                if let Some((v, x)) = vm_bit_select(&a, i) {
                    same((v, x, 1, false), &a.bit_select(i));
                    // The `BranchIfSignalFalse` bit path replaces
                    // `bit_select(..).is_true()` with this test.
                    assert_eq!(v & !x != 0, a.bit_select(i).is_true());
                }
                for j in 0..8usize {
                    if let Some((v, x, w)) = vm_range_select(&a, i, j) {
                        same((v, x, w, false), &a.range_select(i, j));
                    }
                }
            }
        }
    }

    #[test]
    fn replicate_matches_concat_refs() {
        for a in operands() {
            for n in 2..=9usize {
                if let Some((v, x, w)) = vm_replicate(&a, n) {
                    let reference = Value::concat_refs(std::iter::repeat_n(&a, n));
                    same((v, x, w, false), &reference);
                }
            }
        }
    }

    #[test]
    fn repetition_helpers_match_shift_reference() {
        for width in 1u8..=64 {
            let mask = if width == 64 {
                u64::MAX
            } else {
                (1u64 << width) - 1
            };
            let part = 0xd6a5_39c7_b84e_102f & mask;

            for count in 0u8..=64 / width {
                let mut reference = 0u64;
                if width == 64 {
                    if count != 0 {
                        reference = part;
                    }
                } else {
                    for _ in 0..count {
                        reference = (reference << width) | part;
                    }
                }
                assert_eq!(
                    replicate_narrow(part, width, count),
                    reference,
                    "narrow width={width} count={count}"
                );
            }

            let max_count = 512u16 / width as u16;
            for count in [0u16, 1, 2, 3, max_count] {
                if count > max_count {
                    continue;
                }
                let mut reference = [0u64; 8];
                for _ in 0..count {
                    reference = wshl(reference, width as u32);
                    reference[0] |= part;
                }
                assert_eq!(
                    replicate_wide::<8>(part, width, count),
                    reference,
                    "wide width={width} count={count}"
                );
            }
        }
    }

    /// `vm_store` must leave the register in exactly the state a full
    /// assignment would have, for both an `Inline` and a `Wide` destination.
    #[test]
    fn store_overwrites_in_place() {
        for &(v, x, w, s) in &[(0u64, 0u64, 1u32, false), (0xff, 0x0f, 8, true)] {
            let mut dst = Value::from_inline(0xdead, 0xbeef, 3);
            dst.is_signed = true;
            vm_store(&mut dst, v, x, w, s);
            assert_eq!(dst.raw_bits(), (v, x));
            assert_eq!(dst.width, w);
            assert_eq!(dst.is_signed, s);
            assert!(!dst.is_real && !dst.is_fill);

            let mut wide = Value::zero(96); // Wide storage
            vm_store(&mut wide, v, x, w, s);
            assert_eq!(wide.raw_bits(), (v, x));
            assert_eq!(wide.width, w);
            assert_eq!(wide.is_signed, s);
        }
    }
}

/// `Insn::DropXz`: map a register's X/Z bits to 0 (§6.11.1). Out of line so
/// the VM dispatch loops carry one small arm for it.
#[cold]
#[inline(never)]
fn drop_xz(r: &mut Value) {
    if r.has_xz() {
        *r = r.to_two_state();
    }
}

/// The last `.`-separated segment of a hierarchical name and that segment
/// cut at its first `[` — what `rsplit('.')` / `split('[')` give, in one
/// pass over the bytes.
fn leaf_and_base(name: &str) -> (&str, &str) {
    let leaf = match name.bytes().rposition(|b| b == b'.') {
        Some(dot) => &name[dot + 1..],
        None => name,
    };
    let base = match leaf.bytes().position(|b| b == b'[') {
        Some(br) => &leaf[..br],
        None => leaf,
    };
    (leaf, base)
}

/// Exact membership prefilter for a string set: a bitmap over (length,
/// last byte). A string whose shape no member has is certainly absent; one
/// that matches a shape still needs the real lookup.
struct ShapeFilter {
    bits: Vec<u64>,
}

impl ShapeFilter {
    const LENS: usize = 1024;

    fn key(s: &str) -> usize {
        let len = s.len().min(Self::LENS - 1);
        let last = s.as_bytes().last().copied().unwrap_or(0) as usize;
        len * 256 + last
    }

    fn new<'a>(members: impl Iterator<Item = &'a str>) -> Self {
        let mut bits = vec![0u64; Self::LENS * 256 / 64];
        for m in members {
            let k = Self::key(m);
            bits[k >> 6] |= 1u64 << (k & 63);
        }
        Self { bits }
    }

    fn may_contain(&self, s: &str) -> bool {
        let k = Self::key(s);
        self.bits[k >> 6] & (1u64 << (k & 63)) != 0
    }
}

#[cfg(test)]
mod named_event_plan_tests {
    use std::rc::Rc;

    #[test]
    fn cached_targets_preserve_spelling_order_and_shared_signal_identity() {
        let mut sim =
            crate::simulate("module tb; event pulse; endmodule", 0).expect("elaborate event");
        let signal_id = *sim.signal_name_to_id.get("pulse").expect("event signal");
        // Two names may address one event signal. Both writes and their
        // covergroup callbacks must survive candidate deduplication.
        for name in [".pulse", "tb.pulse"] {
            sim.signal_name_to_id.insert(name.into(), signal_id);
        }
        let event = sim.event_triggered_time.intern(".pulse");
        let plan = sim.named_event_plan(event);
        let names: Vec<_> = plan
            .targets
            .iter()
            .map(|target| target.name.as_ref())
            .collect();
        assert_eq!(names, [".pulse", "pulse", "tb.pulse"]);
        for target in &plan.targets {
            assert_eq!(target.signal_id, signal_id);
            assert_eq!(sim.event_triggered_time.get(&target.name), None);
        }
        assert!(Rc::ptr_eq(&plan, &sim.named_event_plan(event)));

        let bare = sim.event_triggered_time.intern("pulse");
        let bare_plan = sim.named_event_plan(bare);
        let bare_names: Vec<_> = bare_plan
            .targets
            .iter()
            .map(|target| target.name.as_ref())
            .collect();
        assert_eq!(bare_names, ["pulse", "tb.pulse"]);
        assert!(!Rc::ptr_eq(&plan, &bare_plan));
        let missing = sim.event_triggered_time.intern("unbacked_event");
        let empty = sim.named_event_plan(missing);
        assert!(empty.targets.is_empty());
        assert!(Rc::ptr_eq(&empty, &sim.named_event_plan(missing)));
        assert!(!sim.event_triggered_time.contains_key("unbacked_event"));
    }
}

#[cfg(test)]
mod name_table_helper_tests {
    use super::{ShapeFilter, leaf_and_base};

    #[test]
    fn leaf_and_base_match_the_split_forms() {
        for name in [
            "",
            "a",
            "a.b",
            "top.u1.mem[3]",
            "x[2].y",
            "a.b[1][2]",
            "a..b",
            "a.",
            ".a",
            "[3]",
        ] {
            let leaf = name.rsplit('.').next().unwrap_or(name);
            let base = leaf.split('[').next().unwrap_or(leaf);
            assert_eq!(leaf_and_base(name), (leaf, base), "{name:?}");
        }
    }

    #[test]
    fn shape_filter_never_rejects_a_member() {
        let long = "x".repeat(2000);
        let members = ["a", "top.sig", "mem[3]", "", long.as_str()];
        let f = ShapeFilter::new(members.iter().copied());
        for m in members {
            assert!(f.may_contain(m), "{m:?}");
        }
        assert!(f.may_contain(&"y".repeat(1500).replace('y', "x")));
        assert!(!f.may_contain("b"));
        assert!(!f.may_contain("top.sig2"));
    }
}

#[cfg(test)]
mod dpi_trampoline_cc_tests {
    use super::dpi_trampoline_cc_command;
    use std::path::Path;

    fn args(cc: Option<&str>) -> Vec<String> {
        dpi_trampoline_cc_command(cc, Path::new("/t/x.so"), Path::new("/t/x.c"))
            .into_iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    /// #282: the trampoline build passes no target-specific flag (`-m64` is
    /// rejected by a non-x86 compiler) and takes the compiler from `$CC`.
    #[test]
    fn trampoline_command_is_portable() {
        let a = args(None);
        assert_eq!(a[0], "cc");
        assert!(
            !a.iter()
                .any(|x| x == "-m64" || x == "-m32" || x.starts_with("-march"))
        );
        for f in ["-fPIC", "-shared", "-o", "/t/x.so", "/t/x.c"] {
            assert!(a.iter().any(|x| x == f), "missing {f} in {a:?}");
        }
        assert_eq!(a.last().map(String::as_str), Some("/t/x.c"));
        let b = args(Some("ccache  aarch64-linux-gnu-gcc"));
        assert_eq!(&b[..2], ["ccache", "aarch64-linux-gnu-gcc"]);
        assert!(!b.iter().any(|x| x == "-m64"));
        assert_eq!(args(Some("  "))[0], "cc");
    }
}

/// The outcome of comparing an aggregate with an assignment pattern
/// (`Simulator::pattern_cmp`, IEEE 1800-2017 §11.4.5).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PatCmp {
    Equal,
    Unequal,
    /// An x/z struct member: the comparison is x.
    Unknown,
    /// An x/z array element: neither `==` nor `!=` holds.
    Neither,
}

impl PatCmp {
    /// Fold the next element's outcome in: any unequal element decides;
    /// otherwise an unknown one makes the result unknown (x for a struct
    /// member, `Neither` for an array element).
    fn and(self, next: PatCmp, member: bool) -> PatCmp {
        match (self, next) {
            (PatCmp::Unequal, _) | (_, PatCmp::Unequal) => PatCmp::Unequal,
            (PatCmp::Equal, PatCmp::Equal) => PatCmp::Equal,
            (PatCmp::Unknown, _) | (_, PatCmp::Unknown) if member => PatCmp::Unknown,
            _ => PatCmp::Neither,
        }
    }
}

#[cfg(test)]
mod event_name_tests;
