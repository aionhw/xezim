//! Run options set from the command line and the environment, and the hang-report handler.

use super::*;

static SIM_DEBUG_ENABLED: AtomicBool = AtomicBool::new(false);

/// `--dump-timescales`: print every module's timescale before the run starts.
static DUMP_TIMESCALES: AtomicBool = AtomicBool::new(false);

static DPI_LIB_PATHS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

pub fn set_sim_debug(enabled: bool) {
    SIM_DEBUG_ENABLED.store(enabled, Ordering::Relaxed);
}

/// `--dump-timescales`: enable the pre-run per-module timescale dump.
pub fn set_dump_timescales(enabled: bool) {
    DUMP_TIMESCALES.store(enabled, Ordering::Relaxed);
}

#[inline]
pub(super) fn dump_timescales_enabled() -> bool {
    DUMP_TIMESCALES.load(Ordering::Relaxed)
}

#[inline]
pub(super) fn sim_debug_enabled() -> bool {
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

pub(super) fn warn_x_enabled() -> bool {
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

pub(super) fn warn_x_limit() -> usize {
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
pub(super) static RUN_LENGTH_REQUESTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub fn set_run_length_requested(v: bool) {
    RUN_LENGTH_REQUESTED.store(v, std::sync::atomic::Ordering::Relaxed);
}

pub(super) fn nospecify() -> bool {
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

pub(crate) fn delay_mode() -> u8 {
    DELAY_MODE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Map a structural delay through the active `+delay_mode_*`: zero → 0,
/// unit → 1 for any nonzero delay, normal → unchanged.
pub(super) fn apply_delay_mode(d: u64) -> u64 {
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

pub(super) fn xtrace_options() -> XtraceOptions {
    xtrace_options_cell()
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default()
}

/// `--wave-suppress` / `--wave-suppress-file` / `--wave-suppress-module`: the
/// waveform EXCLUDE list, shared by the VCD, FST and XTrace dumps.
///
/// Process-global for the same reason as `set_dpi_libs`: `simulate_multi`
/// builds and runs the `Simulator` internally, and the exclusion applies to
/// every dump format at once, so it is a run-wide setting rather than a
/// per-format one. `Simulator::new` copies it into the public
/// `wave_suppress` field, which a library user may also overwrite directly.
fn wave_suppress_cell() -> &'static Mutex<Vec<String>> {
    static WAVE_SUPPRESS: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    WAVE_SUPPRESS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Set the waveform exclusion list. Each entry is either a hierarchical path
/// (`tb_top.u_blk.u_bank0`, top-module prefix optional) or a bare module name
/// (`mem_bank`), which excludes EVERY instance of that module at every depth.
/// `--wave-suppress-module` entries are passed through with a `module:` prefix,
/// which forces the module reading for an identifier that is also a path.
pub fn set_wave_suppress(entries: &[String]) {
    if let Ok(mut guard) = wave_suppress_cell().lock() {
        *guard = entries.to_vec();
    }
}

/// The exclusion list `Simulator::new` should start from.
pub(crate) fn configured_wave_suppress() -> Vec<String> {
    wave_suppress_cell()
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

pub(super) fn configured_vpi_libs() -> Vec<String> {
    vpi_lib_paths()
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default()
}

/// Open a `--dpi-lib` library with `RTLD_NOW | RTLD_GLOBAL` (see
/// `load_dpi_libraries`).
pub(super) fn open_dpi_library(path: &str) -> Result<Library, libloading::Error> {
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
/// compiler (`$CC` when set, else `clang`) producing a position-independent
/// shared object. Only portable flags: the compiler's default target is
/// already the host, and an x86-only flag such as `-m64` makes a non-x86
/// compiler reject the command (#282). `$CC` may carry its own leading
/// arguments (`ccache gcc`). On macOS the linker must be told that the
/// dispatch symbol is resolved at load time from the xezim executable.
pub(super) fn dpi_trampoline_cc_command(
    cc_env: Option<&str>,
    so: &std::path::Path,
    c: &std::path::Path,
) -> Vec<std::ffi::OsString> {
    let mut cmd: Vec<std::ffi::OsString> = cc_env
        .map(|v| v.split_whitespace().map(Into::into).collect::<Vec<_>>())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| vec!["clang".into()]);
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

pub(crate) fn foreign_libraries_configured() -> bool {
    !configured_dpi_libs().is_empty() || !configured_vpi_libs().is_empty()
}

pub(super) fn configured_dpi_libs() -> Vec<String> {
    dpi_lib_paths()
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default()
}

/// Escape a string for embedding inside a JSON `"..."` literal — covers the
/// minimum set of control / structural characters so the coverage-DB output
/// stays valid JSON for covergroup / coverpoint names that include `"` or
/// `\`.
pub(super) fn json_escape(s: &str) -> String {
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

pub(super) fn cached_env_flag(name: &'static str, slot: &'static OnceLock<bool>) -> bool {
    *slot.get_or_init(|| std::env::var(name).ok().as_deref() == Some("1"))
}

pub(super) fn cached_env_set(name: &'static str, slot: &'static OnceLock<bool>) -> bool {
    *slot.get_or_init(|| std::env::var_os(name).is_some())
}

pub(super) fn xz_copy_debug_enabled() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    *FLAG.get_or_init(|| std::env::var_os("XEZIM_CP_DBG").is_some())
}

pub(super) fn parallel_disabled_by_env() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    cached_env_flag("XEZIM_NO_PARALLEL", &FLAG)
}

pub(super) fn parallel_serialize_enabled() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    *FLAG.get_or_init(|| {
        std::env::var("XEZIM_PARALLEL_SERIALIZE")
            .map(|v| v != "0")
            .unwrap_or(true)
    })
}

/// A process waiting for a signal edge event.
/// SIGUSR1 -> on-demand hang report from a live run (`kill -USR1 <pid>`).
/// The handler only flips a flag; the simulation loop prints the report at
/// the next iteration boundary (async-signal-safe).
pub(super) static HANG_REPORT_REQUESTED: std::sync::atomic::AtomicBool =
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
        assert_eq!(a[0], "clang");
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
        assert_eq!(args(Some("  "))[0], "clang");
    }
}
