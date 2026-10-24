use std::env;
use std::path::{Path, PathBuf};

// Opt-in end-of-run statistics footer (`--report-stats` / XEZIM_REPORT_STATS).
// CLI-only plumbing, so it lives in the binary, not the library.
mod report;

// Other simulators' command-line spellings (`-do`, `-g`, `-sv_seed`, ...).
mod cli_compat;
mod cli_files;
mod uvm_dir;

/// The library's `chatter!` for the CLI: internal lines (run banner, `[PHASE]`
/// timings) print only under `--verbose`/`--profile`/`--sim-debug`.
macro_rules! chatter {
    ($($arg:tt)*) => {
        if xezim::verbose() {
            eprintln!($($arg)*);
        }
    };
}

/// Stdout twin of `chatter!`, for the banner lines that have always gone to
/// stdout.
macro_rules! chatter_out {
    ($($arg:tt)*) => {
        if xezim::verbose() {
            println!($($arg)*);
        }
    };
}

/// `run -all`: no time cap short of $finish or an empty event queue. Large,
/// yet small enough that the simulator's conversion to ticks at a 1 fs
/// precision still leaves room above it.
const RUN_ALL_NS: u64 = 9_000_000_000_000;

/// The one line every simulating run ends with. Scripts grep its
/// `Simulation finished at time N` prefix.
fn print_finish_line(time: u64, finished: bool) {
    if finished {
        println!("Simulation finished at time {} ($finish called)", time);
    } else {
        println!("Simulation finished at time {}", time);
    }
}

// The `#[global_allocator]` lives in `xezim-core/src/lib.rs`, not here: Rust
// allows only one per binary, and declaring it in the shared library covers the
// test binaries and xezim-b as well as this CLI.

fn default_design_cache_dir() -> PathBuf {
    if let Some(path) = env::var_os("XEZIM_CACHE_DIR").filter(|p| !p.is_empty()) {
        return PathBuf::from(path);
    }
    if let Some(path) = env::var_os("XDG_CACHE_HOME").filter(|p| !p.is_empty()) {
        return PathBuf::from(path).join("xezim").join("designs");
    }
    if let Some(home) = env::var_os("HOME").filter(|p| !p.is_empty()) {
        return PathBuf::from(home)
            .join(".cache")
            .join("xezim")
            .join("designs");
    }
    PathBuf::from(".xezim-cache")
}

fn design_dependency_files(
    lib_files: &[String],
    lib_dirs: &[String],
    lib_exts: Option<&[String]>,
) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = lib_files.iter().map(PathBuf::from).collect();
    let default_exts = ["v".to_string(), "sv".to_string(), "V".to_string()];
    let exts = lib_exts.unwrap_or(&default_exts);
    for dir in lib_dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
                continue;
            };
            if path.is_file() && exts.iter().any(|candidate| candidate == ext) {
                files.push(path);
            }
        }
    }
    files.sort();
    files.dedup();
    files
}

/// Read a u64 from /proc/<pid|self>/status or /proc/meminfo by key (kB units).
fn proc_kb(path: &str, key: &str) -> Option<u64> {
    let s = std::fs::read_to_string(path).ok()?;
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix(key) {
            // expect "<key>: <num> kB"
            return rest
                .trim_start_matches(|c: char| c == ':' || c.is_whitespace())
                .split_whitespace()
                .next()
                .and_then(|n| n.parse::<u64>().ok());
        }
    }
    None
}

/// Spawn a watchdog that polls /proc/self/status every second. If RSS exceeds
/// 3/4 of MemTotal, print a warning to stderr and kill the process. Disable by
/// setting XEZIM_NO_MEM_WATCHDOG=1.
fn spawn_memory_watchdog() {
    if std::env::var("XEZIM_NO_MEM_WATCHDOG").ok().as_deref() == Some("1") {
        return;
    }
    let total_kb = match proc_kb("/proc/meminfo", "MemTotal") {
        Some(t) if t > 0 => t,
        _ => return, // /proc unavailable (non-Linux); skip silently
    };
    let limit_kb = total_kb / 4 * 3;
    std::thread::spawn(move || {
        let pid = std::process::id();
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            if let Some(rss_kb) = proc_kb("/proc/self/status", "VmRSS") {
                if rss_kb > limit_kb {
                    eprintln!(
                        "[xezim][mem-watchdog] RSS {} MiB exceeds 3/4 of system memory ({} MiB of {} MiB) — killing pid {} to prevent OOM. Set XEZIM_NO_MEM_WATCHDOG=1 to disable.",
                        rss_kb / 1024,
                        limit_kb / 1024,
                        total_kb / 1024,
                        pid,
                    );
                    // SIGKILL self — bypasses panic handlers, no Drop runs,
                    // but ensures the process actually exits even if a thread
                    // is stuck inside a long allocation.
                    unsafe {
                        libc::kill(pid as i32, libc::SIGKILL);
                    }
                    // Fallback if libc isn't available somehow.
                    std::process::exit(137);
                }
            }
        }
    });
}

/// `-xezim_env <file>`: set (or with `unsetenv`, remove) the `XEZIM_*`
/// variables a file lists, overriding the shell environment. Files apply in
/// command-line order; a later setting wins.
fn apply_xezim_env_files() {
    let args: Vec<String> = env::args().skip(1).collect();
    let files = match cli_files::env_files_in_args(&args) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };
    for f in files {
        let text = match std::fs::read_to_string(&f) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("Error: cannot read -xezim_env file '{}': {}", f, e);
                std::process::exit(1);
            }
        };
        let settings = match cli_files::parse_env_file(&text) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Error: -xezim_env file '{}': {}", f, e);
                std::process::exit(1);
            }
        };
        for (name, value) in settings {
            // SAFETY: called first thing in `main`, before any other thread
            // exists, so nothing reads the environment concurrently.
            unsafe {
                match value {
                    Some(v) => env::set_var(&name, v),
                    None => env::remove_var(&name),
                }
            }
        }
    }
}

fn print_usage() {
    eprintln!("Usage: xezim [mode] [options] <source_files> [plusargs]");
    eprintln!("Modes (pick one; default is 'simulate'):");
    eprintln!("  --parse          Lex + parse only, report diagnostics");
    eprintln!("  --compile        Parse + elaborate, report diagnostics (no simulation)");
    eprintln!("  --simulate       Parse + elaborate + simulate (default)");
    eprintln!("Options:");
    eprintln!("  -V               Print version and exit");
    eprintln!("  -I <dir>         Add directory to include search path");
    eprintln!("  -D <name>[=val]  Define a macro");
    eprintln!("  -uvm, --uvm      Add the UVM library named by XEZIM_UVM_DIR (its src include");
    eprintln!("                   dir and uvm_pkg.sv); XEZIM_UVM_VERSION picks a release");
    eprintln!("  -s <topmodule>   Specify the top-level module to elaborate");
    eprintln!("  --no-sim         Alias for --compile (deprecated)");
    eprintln!("  --preprocess     Run the preprocessor only; emit expanded text");
    eprintln!("  --dump-tokens    With --parse, print the token stream");
    eprintln!("  --dump-ast       With --parse, print the AST");
    eprintln!(
        "  --max-time <n>[ps|ns|us|ms|s]   Maximum simulation time; bare <n> is ns (default: 100ms)"
    );
    eprintln!("  --sim-debug      Enable simulator [DEBUG]/[OPT] output (alias: --sim_debug);");
    eprintln!("                   implies the --verbose engine lines");
    eprintln!("  --strict-top     Error out if -s names a module that does not exist (default)");
    eprintln!("  --no-strict-top  Warn and auto-detect the design root instead when -s names");
    eprintln!("                   a module that does not exist (for generated corpora whose");
    eprintln!("                   recorded top names are known-stale)");
    eprintln!("  --profile        Print the [PROF] end-of-run profile report (edge-block, settle");
    eprintln!("                   and timing counters) plus the --verbose engine lines ([PHASE]");
    eprintln!("                   timings etc.). Same as XEZIM_PROFILE_REPORT=1. Adds overhead.");
    eprintln!(
        "  --error-exit     Exit nonzero if any $error was reported ($fatal always does)
  --relax-implicit-static  Accept `int x = ...;` inside a static subroutine
                   (§6.21) with a warning instead of an error. Also enabled by
                   XEZIM_ALLOW_IMPLICIT_STATIC=1.
  --relax-driver-conflicts  Report a variable written by conflicting drivers
                   (two continuous assigns, continuous plus procedural writes,
                   an always_comb/always_latch/always_ff variable written by
                   another process; §6.5, §9.2.2) as a warning instead of an
                   error. Also enabled by XEZIM_RELAX_DRIVER_CONFLICTS=1."
    );
    eprintln!("  --code-coverage[=<kinds>]  Collect code coverage: <kinds> is a comma list of");
    eprintln!("                   stmt, branch, toggle (or all, the default). Counts go to");
    eprintln!("                   xezim_cov.json (XEZIM_COV_DB) with the functional coverage;");
    eprintln!("                   --verbose prints a summary. XEZIM_CODE_COVERAGE=<kinds> too.");
    eprintln!("  --code-coverage-scope <path>[,<path>...]  Only instrument these instance");
    eprintln!("                   subtrees (tb.dut) and packages. Repeatable.");
    eprintln!("  --verbose        Internal engine lines, off by default: the version banner,");
    eprintln!("                   [PHASE] timings, end-of-run engine counters ([PROF]/[FUSE]/");
    eprintln!("                   [EVENT-EDGE]/[COV]), compile-time optimisation notes and");
    eprintln!("                   --compile's design summary; plus per-file compile progress");
    eprintln!("                   (each file as it is parsed and the definitions it contributed).");
    eprintln!("                   Same as XEZIM_VERBOSE=1.");
    eprintln!("  --dump-files-list  Print the full resolved file list (after -f expansion):");
    eprintln!("                     sources in parse order, -v library files, -y library dirs");
    eprintln!(
        "  --upf <file>             Load IEEE 1801 power intent (repeatable): supply nets, power"
    );
    eprintln!(
        "                           switches, domain corruption, isolation; UPF package functions"
    );
    eprintln!(
        "  --upf-top <path>         Instance the UPF scope applies to (default: first instance of"
    );
    eprintln!("                           the set_design_top module)");
    eprintln!(
        "  --work=<lib>             VHDL library for the .vhd/.vhdl files that follow (default work)"
    );
    eprintln!("  --std=08                 VHDL standard (VHDL-2008, the default and only one)");
    eprintln!("  --dump-merged-sv <file>  Write the sources, fully preprocessed (`ifdef");
    eprintln!("                     resolved, macros expanded, `includes inlined), into one");
    eprintln!("                     self-contained .sv file — a standalone repro for");
    eprintln!("                     debugging parse/elaboration problems in -f builds.");
    eprintln!("                     With -s <top>, keeps only the files reachable from that");
    eprintln!("                     top (conservative: may keep one extra, never one fewer);");
    eprintln!("                     without -s, writes every input file.");
    eprintln!("  --dump-timescales  Print each module's timescale before the run (no source");
    eprintln!("                     $printtimescale needed); flags modules with no `timescale.");
    eprintln!("  --dpi-lib <so>   Load a DPI shared library (.so/.dylib/.dll)");
    eprintln!("  --show-env-avail Print every XEZIM_* environment variable with a description");
    eprintln!("  --vpi-lib <so>   Load a VPI module and run its vlog_startup_routines (-m)");
    eprintln!("  --x-warn         Warn when a signal holding a valid 0/1 value takes an x bit");
    eprintln!("                   after time 0, naming the signal, its instance/module and its");
    eprintln!("                   drivers. Signals x/z from the start are never reported.");
    eprintln!("                   Also enabled by +X_WARN or XEZIM_X_WARN=1 in the environment.");
    eprintln!("  --x-warn-limit N Cap --x-warn reports at N (default 50, 0 = unlimited).");
    eprintln!("                   Elaboration diagnostics (port width mismatches, implicit");
    eprintln!("                   nets, ...) are capped per KIND at 5; raise with");
    eprintln!("                   XEZIM_DIAG_LIMIT=N (0 = unlimited).");
    eprintln!("                   XEZIM_TRACE_SIGNAL=name[,name...]  Trace elaboration");
    eprintln!("                   signal-table writes for matching names (debug).");
    eprintln!("                   XEZIM_TRACE_TYPE=name[,name...]  Trace typedef-width table");
    eprintln!(
        "                   writes and type-width resolutions for matching type names (debug)."
    );
    eprintln!("                   Also settable as X_WARN_LIMIT=N.");
    eprintln!("  --module-timescale <unit>/<prec>            Timescale for every module with no");
    eprintln!(
        "                     [mod1,mod2=]<unit>/<prec>   explicit source-level timescale (the"
    );
    eprintln!(
        "                     named form limits it to the listed modules). Repeatable. Never"
    );
    eprintln!(
        "                     overrides a `timeunit`/`timeprecision` decl or an active `timescale."
    );
    eprintln!("  --timescale <unit>/<prec>  Alias for the un-named --module-timescale form,");
    eprintln!("  -timescale <unit>/<prec>     spelled as other simulators spell it. Same rule:");
    eprintln!("                     it is a DEFAULT for design elements with no timescale");
    eprintln!("                     directive, and never overrides an explicit one.");
    eprintln!("  -override_timescale <unit>/<prec>  One timescale for EVERY design element,");
    eprintln!("                     package and compilation unit: every `timescale directive,");
    eprintln!("                     `timeunit`/`timeprecision` decl and --module-timescale is");
    eprintln!("                     ignored (e.g. 1ns/1ns, 1ps/1ps). Unit-suffixed literals");
    eprintln!("                     such as #3ns keep their absolute value.");
    eprintln!("  --report-stats[=json]  Print an end-of-run statistics footer on stderr");
    eprintln!("                   (human text; '=json' emits one JSON line instead). Off by");
    eprintln!("                   default. XEZIM_REPORT_STATS=1|json enables it too; the");
    eprintln!("                   flag wins over the environment.");
    eprintln!(
        "  --cache          Enable the EXPERIMENTAL warm-start design cache (off by default;"
    );
    eprintln!("                   also enabled by XEZIM_ENABLE_CACHE=1 or --cache-dir).");
    eprintln!(
        "  --cache-dir <dir> Store/reuse content-addressed elaborated designs (implies --cache)"
    );
    eprintln!("                    (default: $XEZIM_CACHE_DIR or $XDG_CACHE_HOME/xezim/designs).");
    eprintln!("  --no-cache       Force-disable the design cache (default; XEZIM_NO_CACHE=1 too).");
    eprintln!("  --artifact-compression <none|1-22>  -o artifact compression: 'none' writes raw");
    eprintln!("                   bincode (larger file, fastest load); 1-22 sets the zstd level");
    eprintln!("                   (default 3). Both kinds are auto-detected when loading.");
    eprintln!("  --cache-compression-level <1-22>  Set zstd compression level for cache files");
    eprintln!("                   (default: 3). Higher = better compression but slower.");
    eprintln!("                   Can also be set via XEZIM_CACHE_COMPRESSION_LEVEL=N.");
    eprintln!("  --cache-stats    Print compression statistics when reading/writing cache files.");
    eprintln!("                   Can also be set via XEZIM_CACHE_STATS=1.");
    eprintln!(
        "  -l, --log <file> Redirect all stdout/stderr (including DPI output) to <file>
  -v <file>        Library file: modules compiled only to resolve instantiations
  --primitive-verbose  Show parse/adoption diagnostics for explicit -v files
  -y <dir>         Library directory: <module>.<ext> loaded on demand
  +libext+<ext>+.. Extension list for -y search (replaces default .v/.sv/.V)
  +nospecify       Suppress specify-block path delays and timing checks (zero-delay gate sim)
  +delay_mode_zero Force all structural (specify/SDF) delays to 0 (fast functional GLS)
  +delay_mode_unit Collapse every nonzero structural delay to 1 time unit
  +mindelays/+typdelays/+maxdelays  min:typ:max selection (specify + SDF; default typ)
  +notimingcheck   Disable specify timing checks ($setup, $hold, $width, ...)
  +no_notifier     Report timing violations without toggling notifiers
  +no_tchk_msg     Toggle notifiers without printing timing violations"
    );
    eprintln!("  --xtrace <file>  Emit an XTrace dump to <file> (compliance Level 0:");
    eprintln!("                   dictionary + time + signal deltas + event records).");
    eprintln!("                   A '.zst'/'.zstd' suffix zstd-compresses the stream.");
    eprintln!("  --xtrace-scope <hier>  Restrict the XTrace dump to signals under <hier>");
    eprintln!("                   (exact name or '<hier>.' prefix). Repeatable.");
    eprintln!("  --xtrace-from <ns>  Only dump XTrace changes at/after this time (ns).");
    eprintln!("  --xtrace-to <ns>    Stop the XTrace dump after this time (ns).");
    eprintln!("  --xtrace-level <0>  XTrace compliance level (1-4 reserved: semantic,");
    eprintln!("                   transactional, AI-native, retrieval layers).");
    eprintln!("  --xtrace-format <text>  XTrace output format (binary reserved).");
    eprintln!("  --xtrace-profile <name>  @profile header value (default: minimal).");
    eprintln!("  --xtrace-compress <none|zstd>  Compress the XTrace stream (declared in");
    eprintln!("                   the @compression header; forces a '.zst' file name).");
    eprintln!("  --tee <file>     Copy stdout/stderr to <file> and retain console output");
    eprintln!("  -vcd [dumpon]    Enable source-driven VCD dumping (same as --wave)");
    eprintln!("  --wave           Compile the model WITH waveform support, enabling the");
    eprintln!("                   `$dumpfile`/`$dumpvars` VCD tasks. Off by default: an");
    eprintln!("                   active dump forces loops onto the slower AST path and");
    eprintln!("                   builds a per-signal trace table, so a run that never");
    eprintln!("                   dumps should not pay for it. `--fst`/`--xtrace` imply it.");
    eprintln!("  --fst <file>     Emit an FST (GTKWave binary) waveform dump to <file>.");
    eprintln!("  --fst-scope <hier>  Restrict the FST dump to signals under <hier>");
    eprintln!("                   (exact name or '<hier>.' prefix). Repeatable.");
    eprintln!("  -fst_scope_file <file>  (--fst-scope-file) FST scopes from a file: one or");
    eprintln!("                   more per line, '#' or '//' comments. Adds to --fst-scope.");
    eprintln!("  -xezim_env <file>  (--xezim-env) Set XEZIM_* variables from a file before");
    eprintln!("                   anything reads them: NAME=value, export NAME=value,");
    eprintln!("                   setenv NAME value, unsetenv NAME. Overrides the shell.");
    eprintln!("  --sv2017         Parse as IEEE 1800-2017 (default is 1800-2023)");
    eprintln!("  --sv2023         Parse as IEEE 1800-2023 (default; kept for back-compat)");
    eprintln!("  --no-strict      Disable strict negative-test diagnostics (accept LRM-illegal");
    eprintln!("                   constructs instead of erroring; default is strict/on)");
    eprintln!("Compatibility:");
    eprintln!("  -Ifoo, -DNAME=V  Accepted");
    eprintln!("  +incdir+dir1+dir2 / +define+FOO=1+BAR Accepted");
    eprintln!("  +NAME / +NAME=VALUE passed to $test$plusargs/$value$plusargs");
    eprintln!("  +seed=<n>        Seed the RNG (default: 1, so runs are reproducible)");
    eprintln!("  +seed=random     Draw a seed from entropy; the seed is printed so the");
    eprintln!("                   run can be replayed with +seed=<that value>");
    eprintln!("                   (same seed -> byte-identical run; affects e.g. the");
    eprintln!("                   number of packets a random UVM test collects)");
    eprintln!("  -f/-c filelist   Recursive; options inside filelist are supported");
    eprintln!("Other simulators' spellings (also accepted inside -f files):");
    eprintln!("  <top> ...        A bare design-unit name (or work.<top>) that is not a file");
    eprintln!("                   names a top module, like -s; repeat for several tops");
    eprintln!("  -F <file>        Args file; +incdir+ paths resolve against the args file's");
    eprintln!("                   directory, else as given. -file = -f");
    eprintln!(
        "  -do \"<cmds>\"     Also -do <file.do>. Subset: run -all (until $finish), run <n><unit>"
    );
    eprintln!(
        "                   (summed; --max-time still caps), quit [-f], exit [-f], do <file>."
    );
    eprintln!(
        "                   Other commands are an error. No run before quit = elaborate only"
    );
    eprintln!(
        "  -gNAME=VAL       Parameter default for every module declaring an overridable NAME"
    );
    eprintln!("                   (instance values still win); -g/<top>/NAME=VAL: one module only");
    eprintln!("  -GNAME=VAL       Same, and it also replaces instance and defparam values");
    eprintln!("  -sv_seed <n|random>  Same as +seed=<n|random>");
    eprintln!("  -sv_lib <name>, -sv_root <dir>  DPI library <dir>/<name>.so (see --dpi-lib)");
    eprintln!("  -logfile <file>  Same as -l (redirects; the terminal gets nothing)");
    eprintln!("  -c               With no file after it: accepted (batch mode is the only mode)");
    eprintln!(
        "  -work/-L/-Lf/-lib <lib>  Ignored with one warning: every run compiles from source"
    );
    eprintln!("  -sv12compat, -sv17compat  Same as --sv2017 (-sv05compat/-sv09compat warn)");
    eprintln!("  +cover[=<letters>], -coverage  Code coverage (see --code-coverage): s, b, t");
    eprintln!("                   = stmt, branch, toggle; bare = all three. Other letters are");
    eprintln!("                   ignored with a warning");
    eprintln!("  -sv, -mfcu, -quiet, -64, -batch, -nologo, +acc[=..], -<step>args=..,");
    eprintln!("  -suppress <ids>, +fcover, -sva,");
    eprintln!("  -assertdebug     Accepted, no effect");
    eprintln!("  -sfcu, -t <res>, -wlf <file>  Accepted with a warning (one compilation unit;");
    eprintln!("                   finest precision; waveforms come from --fst/--wave)");
}

fn print_version() {
    // Crate version first and alone on its line: `-V` output is commonly
    // parsed by scripts. The build provenance follows in the same shape the
    // run banner uses, so a log and a `-V` can be matched by eye.
    println!("xezim version {}", env!("CARGO_PKG_VERSION"));
    println!(
        "git {} ({})",
        env!("XEZIM_GIT_HASH"),
        env!("XEZIM_GIT_DATE")
    );
    println!("tag {}", env!("XEZIM_GIT_TAG"));
}

fn print_simulation_banner() {
    println!("=== xezim {} ===", env!("CARGO_PKG_VERSION"));
    println!(
        "git {} ({})",
        env!("XEZIM_GIT_HASH"),
        env!("XEZIM_GIT_DATE")
    );
}

/// Parse a SystemVerilog time literal (`1ns`, `10ns`, `100ps`) to a power-of-
/// ten seconds exponent. Rejects an illegal mantissa or unit.
fn parse_time_literal(s: &str) -> Result<i32, String> {
    let s = s.trim();
    let split = s.find(|c: char| c.is_alphabetic()).unwrap_or(s.len());
    let (digits, unit) = s.split_at(split);
    let mantissa_exp = match digits.trim() {
        "1" => 0,
        "10" => 1,
        "100" => 2,
        other => {
            return Err(format!(
                "invalid time mantissa '{}' (must be 1, 10, or 100)",
                other
            ));
        }
    };
    let unit_exp = match unit.trim() {
        "s" => 0,
        "ms" => -3,
        "us" => -6,
        "ns" => -9,
        "ps" => -12,
        "fs" => -15,
        other => return Err(format!("invalid time unit '{}'", other)),
    };
    Ok(mantissa_exp + unit_exp)
}

/// Parse a `<unit>/<precision>` timescale value, checking precision <= unit.
fn parse_timescale_value(d: &str) -> Result<(i32, i32), String> {
    let (u, p) = d.split_once('/').ok_or_else(|| {
        format!(
            "invalid --module-timescale value '{}' (expected <unit>/<precision>)",
            d
        )
    })?;
    let ue = parse_time_literal(u)?;
    let pe = parse_time_literal(p)?;
    // A larger precision exponent means coarser precision; precision must be
    // equal to or finer (<=) the unit.
    if pe > ue {
        return Err(format!(
            "invalid --module-timescale '{}': precision {} is larger than unit {}",
            d,
            p.trim(),
            u.trim()
        ));
    }
    Ok((ue, pe))
}

/// Build the `--module-timescale` configuration from raw option strings,
/// validating units and detecting conflicting named assignments.
/// Parse a `--max-time` value: a bare number is NANOSECONDS (historical
/// default), an attached suffix selects the unit: `30000000ps`, `30us`,
/// `1ms`, `2s` (case-insensitive; `us` or `µs`). Returns nanoseconds.
/// A customer run set `--max-time 30000000` intending picoseconds under a
/// 1ps timescale and got 30 ms — the unit was invisible and unspellable.
fn parse_max_time(raw: &str) -> Result<u64, String> {
    let t = raw.trim();
    let lower = t.to_ascii_lowercase();
    let (num_str, factor_ns): (&str, f64) = if let Some(n) = lower.strip_suffix("ps") {
        (n, 1e-3)
    } else if let Some(n) = lower.strip_suffix("ns") {
        (n, 1.0)
    } else if let Some(n) = lower.strip_suffix("µs") {
        (n, 1e3)
    } else if let Some(n) = lower.strip_suffix("us") {
        (n, 1e3)
    } else if let Some(n) = lower.strip_suffix("ms") {
        (n, 1e6)
    } else if let Some(n) = lower.strip_suffix('s') {
        (n, 1e9)
    } else {
        (lower.as_str(), 1.0)
    };
    let num: f64 = num_str.trim().parse().map_err(|_| {
        format!(
            "invalid --max-time value '{}' (expected <n>[ps|ns|us|ms|s])",
            raw
        )
    })?;
    if !(num > 0.0) {
        return Err(format!("--max-time must be positive, got '{}'", raw));
    }
    let ns = num * factor_ns;
    let rounded = ns.round();
    if rounded < 1.0 {
        return Err(format!(
            "--max-time '{}' is below 1 ns; the cap is tracked in whole nanoseconds",
            raw
        ));
    }
    Ok(rounded as u64)
}

/// Marks an `-override_timescale` value inside the `--module-timescale`
/// argument list (a module name cannot start with `+`).
const OVERRIDE_TIMESCALE_TAG: &str = "+force+";

/// The `<unit>/<prec>` value of an `-override_timescale` spelling at `t`
/// (`-override_timescale`, `--override_timescale`, `-override-timescale`,
/// `--override-timescale`, each with a following value or `=value`), as
/// `Some(Some(value))`; `Some(None)` when the value is the next token; `None`
/// when `t` is not one of these spellings.
fn override_timescale_flag(t: &str) -> Option<Option<&str>> {
    let body = t.strip_prefix("--").or_else(|| t.strip_prefix('-'))?;
    let (name, value) = match body.split_once('=') {
        Some((n, v)) => (n, Some(v)),
        None => (body, None),
    };
    matches!(name, "override_timescale" | "override-timescale").then_some(value)
}

fn build_module_timescale_cli(raw: &[String]) -> Result<xezim::ModuleTimescaleCli, String> {
    let mut cli = xezim::ModuleTimescaleCli::default();
    for spec in raw {
        // `-override_timescale`: the last one given wins.
        if let Some(v) = spec.strip_prefix(OVERRIDE_TIMESCALE_TAG) {
            cli.force = Some(
                parse_timescale_value(v)
                    .map_err(|e| e.replace("--module-timescale", "-override_timescale"))?,
            );
            continue;
        }
        let (modules, value) = match spec.split_once('=') {
            Some((m, v)) => (Some(m), v),
            None => (None, spec.as_str()),
        };
        let ts = parse_timescale_value(value)?;
        match modules {
            None => cli.global = Some(ts),
            Some(list) => {
                for m in list.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    if let Some(&existing) = cli.named.get(m) {
                        if existing != ts {
                            return Err(format!(
                                "conflicting --module-timescale assignments for module '{}'",
                                m
                            ));
                        }
                    }
                    cli.named.insert(m.to_string(), ts);
                }
            }
        }
    }
    Ok(cli)
}

fn push_define_token(tok: &str, defines: &mut Vec<(String, Option<String>)>) {
    if tok.is_empty() {
        return;
    }
    if let Some(pos) = tok.find('=') {
        defines.push((tok[..pos].to_string(), Some(tok[pos + 1..].to_string())));
    } else {
        defines.push((tok.to_string(), None));
    }
}

fn push_plus_incdir(arg: &str, include_dirs: &mut Vec<String>) {
    if !arg.starts_with("+incdir+") {
        return;
    }
    let payload = &arg[8..];
    for dir in payload.split('+').filter(|s| !s.is_empty()) {
        include_dirs.push(dir.to_string());
    }
}

/// `+libext+.sv+.vlib` — extension list for `-y` library-directory search.
/// Entries may be given with or without the leading dot; the list REPLACES
/// the default (.v/.sv/.V), matching commercial tools.
fn push_plus_libext(arg: &str, lib_exts: &mut Option<Vec<String>>) {
    let list = lib_exts.get_or_insert_with(Vec::new);
    for e in arg["+libext+".len()..].split('+') {
        let e = e.trim().trim_start_matches('.');
        if !e.is_empty() {
            list.push(e.to_string());
        }
    }
}

/// Recognize the commercial gate-level-simulation (GLS) delay/timing flag
/// family so it never falls silently into the generic
/// plusarg bucket. Returns true if `flag` was consumed here.
///
/// - `+delay_mode_zero` / `+delay_mode_unit` are MODELED (force structural
///   delays to 0, or every nonzero one to 1 tick).
/// - `+delay_mode_path` maps to xezim's default (specify path delays apply) —
///   recognized, no effect.
/// - Flags whose effect xezim cannot model (`+delay_mode_distributed`, pulse
///   control, transport/multisource interconnect delays) warn ONCE so the user
///   knows the timing is approximated — never silent.
/// - `+no_notifier` / `+no_tchk_msg` split a timing violation's two effects
///   (notifier toggle, message). The other timing-check controls (`+neg_tchk`,
///   …) are recognized no-ops.
fn handle_gls_flag(flag: &str) -> bool {
    // `+pulse_e/0`, `+pulse_r/95` etc. carry a trailing value.
    let head = flag.split('/').next().unwrap_or(flag);
    match head {
        "+delay_mode_zero" | "-delay_mode_zero" => {
            xezim::compiler::simulator::set_delay_mode(1);
        }
        "+delay_mode_unit" | "-delay_mode_unit" => {
            xezim::compiler::simulator::set_delay_mode(2);
        }
        // Path delays are what xezim already uses when a specify block is
        // present — recognized, no behavior change.
        "+delay_mode_path" | "-delay_mode_path" => {}
        "+no_notifier" => xezim::compiler::simulator::set_no_notifier(true),
        "+no_tchk_msg" => xezim::compiler::simulator::set_no_tchk_msg(true),
        // Negative limits are always honored; the rest have no xezim analogue.
        "+neg_tchk"
        | "+nonegdelay"
        | "+old_ntc"
        | "+ntc_warn"
        | "+nosdferror"
        | "+nocelldefinepragma"
        | "+sdf_verbose"
        | "+sdfverbose" => {}
        // Behavior xezim cannot model — warn once, don't pretend.
        "+delay_mode_distributed" | "-delay_mode_distributed" => {
            eprintln!(
                "Warning: {} requests distributed (gate/net) delays, which xezim does not model \
                 — structural timing is approximated (functional results are unaffected in the \
                 typical case).",
                flag
            );
        }
        "+pulse_e"
        | "+pulse_r"
        | "+pulse_int_e"
        | "+pulse_int_r"
        | "+transport_int_delays"
        | "+transport_path_delays"
        | "+multisource_int_delays" => {
            eprintln!(
                "Warning: {} (pulse/transport/multisource delay control) is not modeled by xezim; \
                 delays are treated as simple inertial.",
                flag
            );
        }
        _ => return false,
    }
    true
}

fn push_plus_define(arg: &str, defines: &mut Vec<(String, Option<String>)>) {
    if !arg.starts_with("+define+") {
        return;
    }
    let payload = &arg[8..];
    for d in payload.split('+').filter(|s| !s.is_empty()) {
        push_define_token(d, defines);
    }
}

/// `-F`'s `+incdir+` rule: a relative directory names one next to the args
/// file first, and only falls back to the path as given when there is none.
/// resolve_rel()'s as-given-first order cannot do this: `.` always exists, so
/// `+incdir+.` in an args file elsewhere meant the working directory, never
/// the file's own.
fn resolve_rel_file_first(base: &Path, p: &str) -> String {
    let pp = Path::new(p);
    if pp.is_absolute() {
        return p.to_string();
    }
    let joined = base.join(pp);
    if joined.exists() || !pp.exists() {
        joined.to_string_lossy().to_string()
    } else {
        p.to_string()
    }
}

fn resolve_rel(base: &Path, p: &str) -> String {
    let pp = Path::new(p);
    if pp.is_absolute() {
        p.to_string()
    } else if pp.exists() {
        p.to_string()
    } else {
        base.join(pp).to_string_lossy().to_string()
    }
}

/// Per source: where each line of its preprocessed text came from.
type LineMaps = Vec<Option<xezim::sv_parser::source_map::LineMap>>;

/// Preprocessed text of each source and its line map. On failure, the
/// rendered preprocessor diagnostics.
fn preprocess_sources(
    sources: &[String],
    source_files: &[String],
    include_dirs: &[String],
    defines: &[(String, Option<String>)],
) -> Result<(Vec<String>, LineMaps, xezim_core::PreprocessedSources), Vec<String>> {
    // The same pass elaboration runs; its result is handed to the simulate
    // path (see `xezim::stash_preprocessed`) so the design is not
    // preprocessed a second time.
    let mut pre = xezim_core::preprocess_design(sources, source_files, include_dirs, defines);
    // §22 strict-mode directive errors (`\`line`/`\`pragma`/`\`resetall`/…)
    // and failed `include`s; a non-empty list fails the run.
    if !pre.errors.is_empty() {
        return Err(std::mem::take(&mut pre.errors));
    }
    let texts = std::mem::take(&mut pre.texts);
    let maps = std::mem::take(&mut pre.line_maps);
    Ok((texts, maps, pre))
}

/// Expand `$VAR` and `${VAR}` style references against the process
/// environment. Unknown variables expand to empty (matching the typical
/// behaviour of other simulators on `-f` filelists). Used so that
/// command files like core-v-verif's `${DV_UVML_HRTBT_PATH}/pkg.flist`
/// resolve without requiring callers to pre-substitute.
fn expand_env_vars(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() {
            // ${NAME}
            if bytes[i + 1] == b'{' {
                if let Some(end) = s[i + 2..].find('}') {
                    let name = &s[i + 2..i + 2 + end];
                    if let Ok(v) = std::env::var(name) {
                        out.push_str(&v);
                    }
                    i = i + 2 + end + 1;
                    continue;
                }
            }
            // $NAME (alphanumeric / underscore)
            let mut j = i + 1;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }
            if j > i + 1 {
                let name = &s[i + 1..j];
                if let Ok(v) = std::env::var(name) {
                    out.push_str(&v);
                }
                i = j;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn split_filelist_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for ch in line.chars() {
        match quote {
            Some(q) => {
                if ch == q {
                    quote = None;
                } else {
                    cur.push(ch);
                }
            }
            None => {
                if ch == '"' || ch == '\'' {
                    quote = Some(ch);
                } else if ch.is_whitespace() {
                    if !cur.is_empty() {
                        out.push(cur.clone());
                        cur.clear();
                    }
                } else {
                    cur.push(ch);
                }
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn process_command_file(
    path: &str,
    source_files: &mut Vec<String>,
    top_modules: &mut Vec<String>,
    include_dirs: &mut Vec<String>,
    defines: &mut Vec<(String, Option<String>)>,
    lib_dirs: &mut Vec<String>,
    plusargs: &mut Vec<String>,
    lib_files: &mut Vec<String>,
    lib_exts: &mut Option<Vec<String>>,
    nospecify: &mut bool,
    primitive_verbose: &mut bool,
    module_timescale_args: &mut Vec<String>,
    compat: &mut cli_compat::CompatArgs,
    incdir_rel: bool,
) -> Result<(), String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Cannot read command file '{}': {}", path, e))?;
    let base = Path::new(path).parent().unwrap_or_else(|| Path::new("."));
    let mut in_block_comment = false;

    for raw in content.lines() {
        let mut line = raw.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
            continue;
        }
        if in_block_comment {
            if let Some((_prefix, _)) = line.split_once("*/") {
                in_block_comment = false;
            }
            continue;
        }
        if line.starts_with("/*") {
            if !line.contains("*/") {
                in_block_comment = true;
            }
            continue;
        }
        if let Some((prefix, _)) = line.split_once("//") {
            line = prefix.trim();
            if line.is_empty() {
                continue;
            }
        }
        let toks: Vec<String> = split_filelist_line(line)
            .into_iter()
            .map(|t| expand_env_vars(&t))
            .collect();
        if toks.is_empty() {
            continue;
        }

        let mut i = 0usize;
        while i < toks.len() {
            let t = toks[i].as_str();
            let used = cli_compat::handle_flag(&toks, i, compat, plusargs)
                .map_err(|e| format!("{} (in args file '{}')", e, path))?;
            if used > 0 {
                i += used;
                continue;
            }
            match t {
                "-s" => {
                    i += 1;
                    let top = toks
                        .get(i)
                        .filter(|top| !top.is_empty() && !top.starts_with('-'))
                        .ok_or_else(|| {
                            format!("-s requires a top module (in args file '{}')", path)
                        })?;
                    top_modules.push(top.clone());
                }
                // `-c` with no file after it: batch-mode switch, nothing to do.
                "-c" if !cli_compat::c_takes_file(
                    toks.get(i + 1).map(|s| s.as_str()),
                    &compat.libs,
                ) => {}
                "-I" => {
                    i += 1;
                    if i < toks.len() {
                        include_dirs.push(resolve_rel(base, &toks[i]));
                    }
                }
                "-D" => {
                    i += 1;
                    if i < toks.len() {
                        push_define_token(&toks[i], defines);
                    }
                }
                "-y" | "--lib" => {
                    i += 1;
                    if i < toks.len() {
                        let d = resolve_rel(base, &toks[i]);
                        lib_dirs.push(d.clone());
                        include_dirs.push(d);
                    }
                }
                "-v" => {
                    i += 1;
                    if i < toks.len() {
                        lib_files.push(resolve_rel(base, &toks[i]));
                    }
                }
                _ if t.starts_with("+libext+") => {
                    push_plus_libext(t, lib_exts);
                }
                "+nospecify" | "-nospecify" => {
                    *nospecify = true;
                }
                "+notimingcheck" | "+notimingchecks" | "-notimingchecks" => {
                    xezim::compiler::simulator::set_no_timing_checks(true);
                }
                // `-F` differs from `-f` only in resolving `+incdir+` paths
                // against the args file's own directory, else as given (see
                // resolve_rel_file_first). `-file` is a long spelling of `-f`.
                // `--fst-scope-file <file>` (path relative to this args
                // file); ahead of the `-f<file>` prefix form below.
                _ if cli_files::option_value(t, cli_files::FST_SCOPE_FILE_NAMES).is_some() => {
                    let v = match cli_files::option_value(t, cli_files::FST_SCOPE_FILE_NAMES) {
                        Some(Some(v)) => Some(v.to_string()),
                        _ => {
                            i += 1;
                            toks.get(i).map(|s| s.to_string())
                        }
                    };
                    if let Some(v) = v {
                        compat.fst_scope_files.push(resolve_rel(base, &v));
                    }
                }
                // `-xezim_env` must be on the command line: the environment
                // is set before any args file is read.
                _ if cli_files::option_value(t, cli_files::XEZIM_ENV_NAMES).is_some() => {
                    if cli_files::option_value(t, cli_files::XEZIM_ENV_NAMES) == Some(None) {
                        i += 1;
                    }
                    eprintln!(
                        "Warning: -xezim_env in args file '{}' is ignored; give it on the command line",
                        path
                    );
                }
                "-f" | "-c" | "-F" | "-file" => {
                    let nested_rel = t == "-F";
                    i += 1;
                    if i < toks.len() {
                        let nested = resolve_rel(base, &toks[i]);
                        process_command_file(
                            &nested,
                            source_files,
                            top_modules,
                            include_dirs,
                            defines,
                            lib_dirs,
                            plusargs,
                            lib_files,
                            lib_exts,
                            nospecify,
                            primitive_verbose,
                            module_timescale_args,
                            compat,
                            nested_rel,
                        )?;
                    }
                }
                _ if t.starts_with("-I") && t.len() > 2 => {
                    include_dirs.push(resolve_rel(base, &t[2..]));
                }
                _ if t.starts_with("-D") && t.len() > 2 => {
                    push_define_token(&t[2..], defines);
                }
                _ if t.starts_with("-y") && t.len() > 2 => {
                    let d = resolve_rel(base, &t[2..]);
                    lib_dirs.push(d.clone());
                    include_dirs.push(d);
                }
                _ if t.starts_with("-f") && t.len() > 2 => {
                    let nested = resolve_rel(base, &t[2..]);
                    process_command_file(
                        &nested,
                        source_files,
                        top_modules,
                        include_dirs,
                        defines,
                        lib_dirs,
                        plusargs,
                        lib_files,
                        lib_exts,
                        nospecify,
                        primitive_verbose,
                        module_timescale_args,
                        compat,
                        false,
                    )?;
                }
                _ if t.starts_with("+incdir+") => {
                    let first = include_dirs.len();
                    push_plus_incdir(t, include_dirs);
                    if incdir_rel {
                        for d in &mut include_dirs[first..] {
                            *d = resolve_rel_file_first(base, d);
                        }
                    }
                }
                "--primitive-verbose" => {
                    *primitive_verbose = true;
                }
                "-xenowarn" => {
                    xezim::set_implicit_net_warn(false);
                }
                "--strict-top" => {
                    xezim::set_strict_top(true);
                }
                "--no-strict-top" => {
                    xezim::set_strict_top(false);
                }
                "--relax-driver-conflicts" => {
                    xezim_core::elaborate::set_relax_driver_conflicts(true);
                }
                _ if t.starts_with("+define+") => {
                    push_plus_define(t, defines);
                }
                _ if handle_gls_flag(t) => {}
                // `--module-timescale <v>` / `--module-timescale=<v>` inside an
                // args file (a customer args file used the `=` form; it was
                // silently ignored and every no-`timescale module fell back to
                // the default — see the warn arm below).
                "--module-timescale" => {
                    if i + 1 < toks.len() {
                        i += 1;
                        module_timescale_args.push(toks[i].to_string());
                    }
                }
                _ if t.starts_with("--module-timescale=") => {
                    module_timescale_args.push(t["--module-timescale=".len()..].to_string());
                }
                // commercial-simulator-compatible spelling of the same thing:
                // `-timescale <unit>/<prec>` supplies the DEFAULT for design
                // elements that carry no timescale directive, and leaves an
                // explicit one alone. Both dash forms, both separators.
                "-timescale" | "--timescale" => {
                    if i + 1 < toks.len() {
                        i += 1;
                        module_timescale_args.push(toks[i].to_string());
                    }
                }
                _ if t.starts_with("-timescale=") => {
                    module_timescale_args.push(t["-timescale=".len()..].to_string());
                }
                _ if t.starts_with("--timescale=") => {
                    module_timescale_args.push(t["--timescale=".len()..].to_string());
                }
                // `-override_timescale <unit>/<prec>`: one timescale for every
                // design element, replacing all source-level timescales.
                _ if override_timescale_flag(t).is_some() => match override_timescale_flag(t) {
                    Some(Some(v)) => {
                        module_timescale_args.push(format!("{}{}", OVERRIDE_TIMESCALE_TAG, v))
                    }
                    _ => {
                        if i + 1 < toks.len() {
                            i += 1;
                            module_timescale_args
                                .push(format!("{}{}", OVERRIDE_TIMESCALE_TAG, toks[i]));
                        }
                    }
                },
                // commercial-simulator-compatible seed aliases: `-svseed <n>` / `-svseed=<n>`
                // (and `-seed` likewise) lower onto the `+seed=` plusarg the
                // simulator already consumes.
                "-svseed" | "-seed" => {
                    if i + 1 < toks.len() {
                        i += 1;
                        plusargs.push(format!("+seed={}", toks[i]));
                    }
                }
                _ if t.starts_with("-svseed=") => {
                    plusargs.push(format!("+seed={}", &t["-svseed=".len()..]));
                }
                _ if t.starts_with("-seed=") => {
                    plusargs.push(format!("+seed={}", &t["-seed=".len()..]));
                }
                _ if t.starts_with("-s") && t.len() > 2 => {
                    top_modules.push(t[2..].to_string());
                }
                _ if t.starts_with('+') => {
                    plusargs.push(t.to_string());
                }
                _ if t.starts_with('-') => {
                    // An option this parser does not understand. NEVER swallow
                    // it silently — a customer args file lost its
                    // `--module-timescale=` (timescale silently defaulted) and
                    // nearly its seed to exactly that.
                    eprintln!(
                        "[xezim][warning] ignored unrecognized option '{}' in args file '{}'",
                        t, path
                    );
                }
                _ => {
                    source_files.push(resolve_rel(base, t));
                }
            }
            i += 1;
        }
    }
    Ok(())
}

/// `-l <file>` / `--log <file>`: send everything the run prints to `file`.
///
/// Done at the file-descriptor level rather than by swapping in a Rust writer,
/// because that is the only thing that catches ALL of it: the simulator prints
/// through `println!`, and a DPI/VPI C model's `printf()` writes straight to
/// fd 1 — a writer-based logger would silently miss both.
#[cfg(unix)]
fn redirect_stdio_to_log(path: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::io::AsRawFd;

    let f = std::fs::File::create(path)?;
    // Flush first, so anything already buffered goes to the real terminal
    // rather than turning up at the head of the log.
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    unsafe {
        if libc::dup2(f.as_raw_fd(), libc::STDOUT_FILENO) < 0
            || libc::dup2(f.as_raw_fd(), libc::STDERR_FILENO) < 0
        {
            return Err(std::io::Error::last_os_error());
        }
    }
    // fds 1 and 2 now own the file; dropping `f` would close it under them.
    std::mem::forget(f);
    Ok(())
}

#[cfg(not(unix))]
fn redirect_stdio_to_log(path: &str) -> std::io::Result<()> {
    let _ = path;
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "-l/--log is only supported on unix",
    ))
}

/// Mirror both descriptors through a helper process. The helper survives
/// simulator exit/abort and drains C stdio written during exit handlers.
#[cfg(unix)]
static LOG_TEE_PID: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

#[cfg(unix)]
extern "C" fn finish_log_tee() {
    use std::io::Write;
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    unsafe {
        libc::fflush(std::ptr::null_mut());
        libc::close(libc::STDOUT_FILENO);
        libc::close(libc::STDERR_FILENO);
        let pid = LOG_TEE_PID.load(std::sync::atomic::Ordering::Relaxed);
        // An inherited descriptor in a background child must not hang exit.
        for _ in 0..100 {
            if libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) != 0 {
                break;
            }
            let delay = libc::timespec {
                tv_sec: 0,
                tv_nsec: 10_000_000,
            };
            libc::nanosleep(&delay, std::ptr::null_mut());
        }
    }
}

#[cfg(unix)]
fn tee_stdio_to_log(path: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    fn pair() -> std::io::Result<[OwnedFd; 2]> {
        let mut fds = [-1; 2];
        if unsafe { libc::pipe(fds.as_mut_ptr()) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let result = unsafe { [OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])] };
        for fd in &result {
            if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        Ok(result)
    }
    unsafe fn copy_bytes(fd: libc::c_int, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            let n = unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) };
            if n > 0 {
                bytes = &bytes[n as usize..];
            } else if n < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            } else {
                break;
            }
        }
    }
    let log = std::fs::File::create(path)?;
    let stdout_pipe = pair()?;
    let stderr_pipe = pair()?;
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    unsafe {
        libc::fflush(std::ptr::null_mut());
    }
    let parent = unsafe { libc::getpid() };
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err(std::io::Error::last_os_error());
    }
    if pid == 0 {
        // Only descriptor syscalls after fork: do not touch inherited Rust
        // locks or run destructors in the helper.
        unsafe {
            libc::close(stdout_pipe[1].as_raw_fd());
            libc::close(stderr_pipe[1].as_raw_fd());
            libc::signal(libc::SIGPIPE, libc::SIG_IGN);
            // Terminal/group termination belongs to the simulator. Keep the
            // helper alive long enough to drain the simulator's final output.
            libc::signal(libc::SIGINT, libc::SIG_IGN);
            libc::signal(libc::SIGTERM, libc::SIG_IGN);
            libc::signal(libc::SIGHUP, libc::SIG_IGN);
            let mut polls = [
                libc::pollfd {
                    fd: stdout_pipe[0].as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: stderr_pipe[0].as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            let mut buf = [0u8; 8192];
            let mut open = 2;
            while open != 0 {
                let ready = libc::poll(polls.as_mut_ptr(), 2, 250);
                if ready < 0 {
                    if std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                        continue;
                    }
                    break;
                }
                if ready == 0 && libc::getppid() != parent {
                    break;
                }
                for (i, p) in polls.iter_mut().enumerate() {
                    if p.fd < 0 || p.revents == 0 {
                        continue;
                    }
                    let n = libc::read(p.fd, buf.as_mut_ptr().cast(), buf.len());
                    if n > 0 {
                        copy_bytes(log.as_raw_fd(), &buf[..n as usize]);
                        copy_bytes(
                            if i == 0 {
                                libc::STDOUT_FILENO
                            } else {
                                libc::STDERR_FILENO
                            },
                            &buf[..n as usize],
                        );
                    } else if n == 0 {
                        libc::close(p.fd);
                        p.fd = -1;
                        open -= 1;
                    } else if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                        libc::close(p.fd);
                        p.fd = -1;
                        open -= 1;
                    }
                }
            }
            libc::_exit(0);
        }
    }
    if unsafe { libc::dup2(stdout_pipe[1].as_raw_fd(), libc::STDOUT_FILENO) } < 0
        || unsafe { libc::dup2(stderr_pipe[1].as_raw_fd(), libc::STDERR_FILENO) } < 0
    {
        return Err(std::io::Error::last_os_error());
    }
    // C output must already be in the pipe when a fatal signal bypasses exit.
    unsafe {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            unsafe extern "C" {
                static mut stdout: *mut libc::FILE;
                static mut stderr: *mut libc::FILE;
            }
            libc::setvbuf(stdout, std::ptr::null_mut(), libc::_IONBF, 0);
            libc::setvbuf(stderr, std::ptr::null_mut(), libc::_IONBF, 0);
        }
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        {
            unsafe extern "C" {
                static mut __stdoutp: *mut libc::FILE;
                static mut __stderrp: *mut libc::FILE;
            }
            libc::setvbuf(__stdoutp, std::ptr::null_mut(), libc::_IONBF, 0);
            libc::setvbuf(__stderrp, std::ptr::null_mut(), libc::_IONBF, 0);
        }
        LOG_TEE_PID.store(pid, std::sync::atomic::Ordering::Relaxed);
        libc::atexit(finish_log_tee);
    }
    Ok(())
}

#[cfg(not(unix))]
fn tee_stdio_to_log(_path: &str) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "--tee is only supported on unix",
    ))
}

/// Post-elaboration design summary (vendor-elaborator style). xezim inlines
/// the hierarchy into one flat module, so the left column counts FLATTENED
/// runtime objects and the right column counts unique parsed definitions —
/// a sanity check that the whole design was analyzed.
fn print_design_summary(
    defs: &std::collections::HashMap<String, xezim::SourceDefinition, impl std::hash::BuildHasher>,
    elab: &xezim::compiler::ElaboratedModule,
) {
    use xezim::SourceDefinition as SD;
    let uniq = |f: &dyn Fn(&SD) -> bool| defs.values().filter(|d| f(d)).count();
    let u_mod = uniq(&|d| matches!(d, SD::Module(_)));
    let u_ifc = uniq(&|d| matches!(d, SD::Interface(_)));
    let u_prog = uniq(&|d| matches!(d, SD::Program(_)));
    let u_pkg = uniq(&|d| matches!(d, SD::Package(_)));
    let u_cls = uniq(&|d| matches!(d, SD::Class(_)));
    let u_udp = uniq(&|d| matches!(d, SD::Udp(_)));
    let arr_elems: i64 = elab
        .arrays
        .values()
        .map(|&(lo, hi, _)| (hi - lo + 1).max(0))
        .sum();
    println!("Design summary (flattened / unique definitions):");
    let row = |label: &str, inst: usize, uniqn: usize| {
        println!("  {:<28}{:>12}  {:>8}", label, inst, uniqn);
    };
    println!("  {:<28}{:>12}  {:>8}", "", "flattened", "unique");
    row("Modules:", elab.src_file_of_module.len(), u_mod);
    row("Interfaces:", elab.interfaces.len(), u_ifc);
    row("Programs:", u_prog, u_prog);
    row("Packages:", elab.packages.len(), u_pkg);
    row("UDP instances:", elab.udp_instances.len(), u_udp);
    row("Classes:", elab.classes.len(), u_cls);
    // Distinct hierarchical scopes among the flattened signal names — the
    // closest analogue of a vendor elaborator's "instances" count.
    let scopes: std::collections::HashSet<&str> = elab
        .signals
        .keys()
        .filter_map(|n| n.rfind('.').map(|i| &n[..i]))
        .collect();
    println!("  {:<28}{:>12}", "Instance scopes:", scopes.len());
    println!("  {:<28}{:>12}", "Signals:", elab.signals.len());
    println!(
        "  {:<28}{:>12}  ({} elements)",
        "Unpacked arrays:",
        elab.arrays.len(),
        arr_elems
    );
    println!("  {:<28}{:>12}", "Named events:", elab.events.len());
    // Inlined-instance blocks live in the lazily-materialized pending_* vecs
    // until the bytecode compiler drains them — count BOTH, or a big design
    // reports "1 always block" while 66k sit pending.
    println!(
        "  {:<28}{:>12}",
        "Always blocks:",
        elab.always_blocks.len() + elab.pending_always.len()
    );
    println!(
        "  {:<28}{:>12}",
        "Initial blocks:",
        elab.initial_blocks.len() + elab.pending_initial.len()
    );
    println!("  {:<28}{:>12}", "Final blocks:", elab.final_blocks.len());
    println!(
        "  {:<28}{:>12}",
        "Cont. assignments:",
        elab.continuous_assigns.len() + elab.pending_cont_assign.len()
    );
    println!("  {:<28}{:>12}", "Functions:", elab.functions.len());
    println!("  {:<28}{:>12}", "Tasks:", elab.tasks.len());
    println!(
        "  {:<28}{:>9} ns",
        "Simulation time unit:",
        elab.tick_s * 1e9
    );
}

/// Peak/current memory and CPU usage, vendor-tool style, so long builds can
/// be compared against other tools' footers.
fn print_resource_usage(wall_start: std::time::Instant) {
    let mut peak = String::new();
    let mut cur = String::new();
    if let Ok(st) = std::fs::read_to_string("/proc/self/status") {
        for line in st.lines() {
            if let Some(v) = line.strip_prefix("VmHWM:") {
                peak = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("VmRSS:") {
                cur = v.trim().to_string();
            }
        }
    }
    #[cfg(unix)]
    {
        let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) } == 0 {
            let user = ru.ru_utime.tv_sec as f64 + ru.ru_utime.tv_usec as f64 / 1e6;
            let sys = ru.ru_stime.tv_sec as f64 + ru.ru_stime.tv_usec as f64 / 1e6;
            println!(
                "xezim: CPU Usage - {:.1}s system + {:.1}s user = {:.1}s total (wall {:.1}s)",
                sys,
                user,
                sys + user,
                wall_start.elapsed().as_secs_f64()
            );
        }
    }
    if !peak.is_empty() || !cur.is_empty() {
        println!("xezim: Memory Usage - Current: {}, Peak: {}", cur, peak);
    }
}

/// Opt-in end-of-run statistics footer (`--report-stats` / XEZIM_REPORT_STATS).
/// A no-op when the mode is Off, so a default run's output is byte-identical;
/// when enabled the footer goes to stderr, leaving stdout (including the
/// `Simulation finished at time ...` line scripts grep) untouched.
/// `sim_time_ns` is None for runs that never simulate (--compile).
fn emit_run_stats(
    mode: report::ReportMode,
    wall_start: std::time::Instant,
    sim_time_ns: Option<u64>,
) {
    if mode == report::ReportMode::Off {
        return;
    }
    let (cpu_user_ms, cpu_sys_ms) = report::cpu_times_ms();
    let stats = report::RunStats {
        version: env!("CARGO_PKG_VERSION").to_string(),
        git_rev: env!("XEZIM_GIT_HASH").to_string(),
        wall_ms: wall_start.elapsed().as_millis() as u64,
        cpu_user_ms,
        cpu_sys_ms,
        peak_rss_kb: report::peak_rss_kb(),
        hostname: report::hostname(),
        sim_time_ns,
    };
    match mode {
        report::ReportMode::Off => {}
        report::ReportMode::Human => eprint!("{}", report::render_human(&stats)),
        report::ReportMode::Json => eprint!("{}", report::render_json(&stats)),
    }
}

/// Design units DECLARED at the top level of one preprocessed file, plus every
/// identifier the file mentions. Both come from one lexer pass, so comments and
/// string literals can never contribute a false name.
///
/// A unit counts as declared here only at nesting depth 0 (or inside nothing
/// but `package`s, which is where classes normally live) — otherwise an ANSI
/// port list's `interface foo_if.mp p` would register `foo_if` as *defined* by
/// the instantiating file and misroute every reference to it.
fn scan_units_and_refs(text: &str) -> (Vec<String>, std::collections::HashSet<String>, bool) {
    use xezim::lexer::TokenKind as TK;
    let toks = xezim::lexer::Lexer::new(text).tokenize();
    let mut declared = Vec::new();
    let mut refs = std::collections::HashSet::new();
    // §23.11: a top-level `bind` attaches instances to a module named
    // elsewhere. Nothing REFERENCES the bind file by name, so reachability
    // alone would always drop it — silently removing checkers from the repro.
    let mut top_level_bind = false;
    // Open unit keywords, innermost last.
    let mut stack: Vec<&str> = Vec::new();
    let mut prev: &str = "";
    let mut i = 0usize;
    while i < toks.len() {
        let t = &toks[i];
        let text_s = t.text.as_str();
        if matches!(t.kind, TK::Identifier | TK::EscapedIdentifier) {
            refs.insert(text_s.to_string());
            prev = text_s;
            i += 1;
            continue;
        }
        let kind = match text_s {
            "module" | "macromodule" => Some("module"),
            "interface" => Some("interface"),
            "program" => Some("program"),
            "package" => Some("package"),
            "primitive" => Some("primitive"),
            "checker" => Some("checker"),
            "class" => Some("class"),
            _ => None,
        };
        if let Some(kind) = kind {
            // `interface class C` is a CLASS declaration — don't also open an
            // interface scope for it, or the missing `endinterface` unbalances
            // everything that follows.
            let iface_class =
                kind == "interface" && toks.get(i + 1).is_some_and(|n| n.text == "class");
            // `typedef class C;` is a §6.18 forward declaration, not a
            // definition; the real one may live in another file entirely.
            let fwd = kind == "class" && prev == "typedef";
            if !iface_class && !fwd {
                let top_level = stack.iter().all(|s| *s == "package");
                if top_level {
                    // Skip the modifiers that may sit between the keyword and
                    // the name (`module automatic m`, `class static C`).
                    let mut j = i + 1;
                    while toks.get(j).is_some_and(|n| {
                        matches!(n.text.as_str(), "static" | "automatic" | "virtual")
                    }) {
                        j += 1;
                    }
                    if let Some(n) = toks.get(j) {
                        if matches!(n.kind, TK::Identifier | TK::EscapedIdentifier) {
                            declared.push(n.text.clone());
                        }
                    }
                }
                stack.push(kind);
            }
        } else if text_s.starts_with("end")
            && matches!(
                text_s,
                "endmodule"
                    | "endinterface"
                    | "endprogram"
                    | "endpackage"
                    | "endprimitive"
                    | "endchecker"
                    | "endclass"
            )
        {
            stack.pop();
        } else if text_s == "bind" && stack.is_empty() {
            top_level_bind = true;
        }
        prev = text_s;
        i += 1;
    }
    (declared, refs, top_level_bind)
}

/// `--dump-merged-sv` with `-s <top>`: the indices of the files actually needed
/// to elaborate `top`, in original order.
///
/// The closure is LEXICAL and runs before parsing, so the dump still works when
/// the design does not elaborate — which is the situation the flag exists for.
/// A file is pulled in when it declares a unit some already-included file
/// mentions by name. That is deliberately conservative: it can keep more than
/// strictly necessary (a file's *other* units drag their own dependencies
/// along), never less. Returns None when `top` is not declared by any input
/// file — it may come from a `-v`/`-y` library, and dumping everything is the
/// safe answer.
fn merged_sv_files_for_top(top: &str, texts: &[String]) -> Option<Vec<usize>> {
    let scanned: Vec<_> = texts.iter().map(|t| scan_units_and_refs(t)).collect();
    // First declaration wins, matching the elaborator's own resolution.
    let mut owner: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (fi, (declared, _, _)) in scanned.iter().enumerate() {
        for name in declared {
            owner.entry(name.as_str()).or_insert(fi);
        }
    }
    let start = *owner.get(top)?;
    let mut keep = vec![false; texts.len()];
    let mut queue = vec![start];
    keep[start] = true;
    // A file that declares no design unit at all is never REFERENCED by name,
    // so reachability alone would always drop it — yet it may hold §3.12
    // compilation-unit declarations (a `typedef`/function/parameter at file
    // scope) or a top-level `bind`, both of which the rest of the design uses
    // without naming the file. Dropping those does not merely fail to compile:
    // it can silently CHANGE the answer, which is the one thing a reduction
    // tool must never do. Seed them all, then let the walk expand from them
    // (that is what pulls in a checker reachable only through a bind).
    for (fi, (declared, _, top_bind)) in scanned.iter().enumerate() {
        if (declared.is_empty() || *top_bind) && !keep[fi] {
            keep[fi] = true;
            queue.push(fi);
        }
    }
    while let Some(fi) = queue.pop() {
        for name in &scanned[fi].1 {
            if let Some(&dep) = owner.get(name.as_str()) {
                if !keep[dep] {
                    keep[dep] = true;
                    queue.push(dep);
                }
            }
        }
    }
    Some(
        keep.iter()
            .enumerate()
            .filter(|(_, k)| **k)
            .map(|(i, _)| i)
            .collect(),
    )
}

/// Record — and, for a repeat, STRIP — compilation-unit-scope (`$unit`)
/// `task`/`function` definitions in `text`.
///
/// `--dump-merged-sv` concatenates whole adopted library files, but a `-v`/`-y`
/// library only contributes the definitions an instantiation actually needed:
/// two libraries can each carry their own copy of a `$unit` helper task and the
/// original run never sees both. The merged file does, and §26.2 makes a repeat
/// declaration in the same scope an error, so the dump would not re-compile.
///
/// Only DUPLICATES are removed and only at depth 0, so a misparse degrades to
/// today's behaviour (text appended verbatim) rather than dropping code. The
/// first definition wins, matching the elaborator's own resolution order.
fn strip_duplicate_unit_subroutines(
    text: &str,
    seen: &mut std::collections::HashSet<String>,
) -> (String, usize) {
    // The identifier a `task`/`function` header declares: everything up to the
    // first `(` or `;` minus qualifiers and the return type, i.e. the LAST
    // identifier token ("automatic logic [7:0] foo" -> "foo").
    fn header_name(rest: &str) -> Option<String> {
        let head = rest.split(['(', ';']).next().unwrap_or("");
        head.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
            .filter(|t| !t.is_empty() && !t.chars().next().is_some_and(|c| c.is_ascii_digit()))
            .next_back()
            .map(|t| t.to_string())
    }
    const OPENERS: [&str; 8] = [
        "module",
        "macromodule",
        "interface",
        "package",
        "program",
        "class",
        "checker",
        "primitive",
    ];
    const CLOSERS: [&str; 8] = [
        "endmodule",
        "endinterface",
        "endpackage",
        "endprogram",
        "endclass",
        "endchecker",
        "endprimitive",
        "endconfig",
    ];
    // `endtask`/`endfunction` as a standalone TOKEN anywhere in the line: a
    // one-line definition (`task automatic f(...); ...; endtask`) closes on the
    // header line itself, and a scanner that only looked at the first token
    // swallowed everything after it.
    fn closes_here(line: &str, end_kw: &str) -> bool {
        line.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
            .any(|t| t == end_kw)
    }
    let mut out = String::with_capacity(text.len());
    let mut depth = 0i32;
    let mut skipping: Option<&'static str> = None;
    let mut stripped = 0usize;
    for line in text.lines() {
        let t = line.trim_start();
        let toks: Vec<&str> = t.split_whitespace().collect();
        if let Some(end_kw) = skipping {
            if closes_here(t, end_kw) {
                skipping = None;
            }
            continue;
        }
        let first = toks.first().copied().unwrap_or("");
        // `typedef class c;` is a forward declaration with no `endclass`.
        if first != "typedef" {
            // `virtual class`, `static task`, … — look past one qualifier.
            let kw = if OPENERS.contains(&first) {
                Some(first)
            } else {
                toks.get(1).copied().filter(|w| OPENERS.contains(w))
            };
            if kw.is_some() {
                depth += 1;
            }
            if CLOSERS.contains(&first) {
                depth -= 1;
            }
            if depth == 0
                && (first == "task" || first == "function")
                // `extern`/`pure virtual` prototypes have no body to strip.
                && !t.starts_with("extern")
                && !t.starts_with("pure")
            {
                if let Some(name) = header_name(&t[first.len()..]) {
                    if !seen.insert(name.clone()) {
                        out.push_str(&format!(
                            "// [xezim] duplicate $unit {first} '{name}' suppressed; first definition kept\n"
                        ));
                        let end_kw = if first == "task" {
                            "endtask"
                        } else {
                            "endfunction"
                        };
                        // A one-liner closes on this very line; only a
                        // multi-line body needs the skip state.
                        if !closes_here(t, end_kw) {
                            skipping = Some(end_kw);
                        }
                        stripped += 1;
                        continue;
                    }
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    (out, stripped)
}

/// `--dump-merged-sv` phase 2: after elaboration, append the `-v`/`-y`
/// library files whose definitions were actually ADOPTED — with them inlined
/// the merged file rebuilds standalone, with no -v/-y flags. Whole files are
/// appended (a cell library groups related primitives); if one also defines a
/// name the primary sources already define, the re-compile will say so.
fn append_adopted_libs_to_merged(
    merged_out: &str,
    primary_texts: &[String],
    primary_labels: &[String],
    kept: Option<&[usize]>,
) {
    let adopted = xezim::adopted_lib_files();
    if adopted.is_empty() {
        return;
    }
    let mut extra = String::new();
    let mut nfiles = 0usize;
    let mut nmods = 0usize;
    let mut nstripped = 0usize;
    // References made INSIDE the adopted library texts. The `-s <top>` file
    // closure is lexical over the PRIMARY sources only, so a primary file
    // whose module is instantiated solely from a library file was dropped
    // from the merged output — the dump then contained the instantiation but
    // not the definition and did not re-run standalone. Collect the library
    // refs here and re-add such primaries below.
    let mut lib_refs: Vec<String> = Vec::new();
    // Seed from the primary sources already in the file, so a library copy of
    // a task the design itself defines is suppressed too.
    let mut seen_subs: std::collections::HashSet<String> = std::collections::HashSet::new();
    if let Ok(primary) = std::fs::read_to_string(merged_out) {
        let _ = strip_duplicate_unit_subroutines(&primary, &mut seen_subs);
    }
    for (path, mods) in &adopted {
        // Preprocess with the SAME context the -v/-y indexing pass used
        // (fresh preprocessor, run include dirs, post-primary macro
        // snapshot) so the appended text has its includes inlined and
        // macros expanded. Raw bytes broke the standalone re-compile the
        // moment merged.sv left the original directory: the library's
        // relative include paths no longer resolved and its macros came
        // out undefined.
        let (text, how) = match xezim::preprocess_adopted_lib(path) {
            Some(t) => (t, "preprocessed"),
            None => match std::fs::read(path) {
                Ok(bytes) => {
                    eprintln!(
                        "Warning: --dump-merged-sv: preprocessing library '{}' failed; appending raw text (the merged file may not re-compile standalone)",
                        path.display()
                    );
                    (String::from_utf8_lossy(&bytes).into_owned(), "raw")
                }
                Err(e) => {
                    eprintln!(
                        "Warning: --dump-merged-sv could not append library '{}': {}",
                        path.display(),
                        e
                    );
                    continue;
                }
            },
        };
        nfiles += 1;
        nmods += mods.len();
        {
            let (_decl, refs, _bind) = scan_units_and_refs(&text);
            lib_refs.extend(refs);
        }
        let (text, nstrip) = strip_duplicate_unit_subroutines(&text, &mut seen_subs);
        nstripped += nstrip;
        extra.push_str(&format!(
            "\n// ===== adopted library file: {} ({}; needed for: {}) =====\n",
            path.display(),
            how,
            mods.join(", ")
        ));
        extra.push_str(&text);
        if !extra.ends_with('\n') {
            extra.push('\n');
        }
    }
    if nfiles == 0 {
        return;
    }
    // Re-add primary files the `-s` closure dropped but the adopted library
    // text references (transitively: a re-added primary may itself pull in
    // further dropped primaries). Mirrors elaboration order — a name declared
    // by a primary resolves there before any library fallback.
    if let Some(kept) = kept {
        let scanned: Vec<_> = primary_texts
            .iter()
            .map(|t| scan_units_and_refs(t))
            .collect();
        let mut owner: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for (fi, (declared, _, _)) in scanned.iter().enumerate() {
            for name in declared {
                owner.entry(name.as_str()).or_insert(fi);
            }
        }
        let mut included = vec![false; primary_texts.len()];
        for &fi in kept {
            if fi < included.len() {
                included[fi] = true;
            }
        }
        let mut queue = lib_refs;
        let mut added: Vec<usize> = Vec::new();
        while let Some(name) = queue.pop() {
            if let Some(&fi) = owner.get(name.as_str()) {
                if !included[fi] {
                    included[fi] = true;
                    added.push(fi);
                    queue.extend(scanned[fi].1.iter().cloned());
                }
            }
        }
        added.sort_unstable();
        for fi in &added {
            let (text, nstrip) =
                strip_duplicate_unit_subroutines(&primary_texts[*fi], &mut seen_subs);
            nstripped += nstrip;
            extra.push_str(&format!(
                "
// ===== primary file re-added: {} (referenced only from an adopted library) =====
",
                primary_labels[*fi]
            ));
            extra.push_str(&text);
            if !extra.ends_with('\n') {
                extra.push('\n');
            }
        }
        if !added.is_empty() {
            println!(
                "Re-added {} primary file(s) referenced only from adopted libraries",
                added.len()
            );
        }
    }
    if let Err(e) = std::fs::OpenOptions::new()
        .append(true)
        .open(merged_out)
        .and_then(|mut f| std::io::Write::write_all(&mut f, extra.as_bytes()))
    {
        eprintln!(
            "Warning: cannot append libraries to '{}': {}",
            merged_out, e
        );
        return;
    }
    println!(
        "Appended {} adopted library file(s) ({} definition(s)) to {}{}",
        nfiles,
        nmods,
        merged_out,
        if nstripped > 0 {
            format!(" (suppressed {nstripped} duplicate $unit task/function definition(s))")
        } else {
            String::new()
        }
    );
}

fn main() {
    // `-xezim_env <file>` before anything reads an `XEZIM_*` variable (this
    // very function reads XEZIM_HUGEPAGE and XEZIM_STACK_MB). The process is
    // still single-threaded here, so setting the environment is sound.
    apply_xezim_env_files();
    // FIRST thing, before any large allocation: a parent can disable transparent
    // huge pages for its whole descendant tree with prctl(PR_SET_THP_DISABLE, 1),
    // and the flag is inherited across fork+exec. While set, madvise(MADV_HUGEPAGE)
    // still returns 0 and sets VM_HUGEPAGE but the fault handler never attempts a
    // huge page, and MADV_COLLAPSE fails EINVAL — the simulator's whole
    // `advise_hugepages()` path becomes a silent no-op. Some launchers (CI
    // runners, agent harnesses, container supervisors) set it.
    //
    // This MUST happen here rather than in `advise_hugepages()`: by the time the
    // simulator runs, `compile()` has already faulted the ~1.3 GB of per-signal
    // arrays in as 4 KiB pages, leaving only a partial MADV_COLLAPSE to recover.
    // Measured on c906 memcpy x50 — cleared at process start:
    // dTLB-load-misses 161.1M -> 65.2M, cycles -3.7%, wall -3.8%. Cleared late
    // instead: only 93.7M and no cycle win. Clearing it needs no privilege and
    // affects only this process. XEZIM_HUGEPAGE=0 opts out.
    #[cfg(target_os = "linux")]
    if std::env::var("XEZIM_HUGEPAGE").ok().as_deref() != Some("0") {
        const PR_SET_THP_DISABLE: libc::c_int = 41;
        unsafe {
            libc::prctl(PR_SET_THP_DISABLE, 0, 0, 0, 0);
        }
    }
    spawn_memory_watchdog();
    // Install the SIGUSR1 hang-report handler before compile: a user poking a
    // seemingly-hung run during a long elaboration must not kill it (the
    // default SIGUSR1 action is termination).
    xezim::compiler::simulator::install_hang_report_handler();

    // The parser, elaborator, and statement interpreter are all deeply
    // recursive; a debug build's unoptimized frames overflow the default
    // 8 MiB main-thread stack on large designs (a full UVM elaboration
    // aborts with "thread 'main' has overflowed its stack"). Do what rustc
    // does: run the whole compile+simulate on a worker thread with a large
    // stack. The memory is virtual — pages commit only if actually used —
    // so the big default costs nothing. XEZIM_STACK_MB overrides.
    let stack_mb: usize = std::env::var("XEZIM_STACK_MB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1024);
    if stack_mb > 0 {
        let code = std::thread::Builder::new()
            .name("xezim-main".to_string())
            .stack_size(stack_mb * 1024 * 1024)
            .spawn(run_main)
            .expect("spawn simulation thread")
            .join()
            .unwrap_or_else(|_| 101);
        std::process::exit(code);
    }
    let code = run_main();
    std::process::exit(code);
}

fn run_main() -> i32 {
    let compile_wall_start = std::time::Instant::now();

    // Default to IEEE 1800-2023 mode. SV-2023 is additive over -2017, so
    // valid -2017 code stays valid; pass `--sv2017` to opt back to the
    // older grammar where a new keyword or syntax form gets in the way.
    sv_parser::set_sv2023(true);

    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        print_usage();
        std::process::exit(1);
    }

    let mut source_files: Vec<String> = Vec::new();
    // All `-s <top>` modules, in order. UVM testbenches commonly declare two
    // unconnected roots (e.g. `hdl_top` + `hvl_top`); when more than one is
    // given we synthesize a wrapper module that instantiates them all and
    // elaborate that instead (a single root reaching every requested top).
    let mut top_modules: Vec<String> = Vec::new();
    // Default cap: 100 ms of simulated time, in ns. Long enough for UVM tests
    // that run tens of milliseconds; a design that never calls `$finish`
    // still stops.
    let mut max_time: u64 = 100_000_000;
    let mut dump_tokens = false;
    let mut dump_ast = false;
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mode {
        Preprocess,
        Parse,
        Compile,
        Simulate,
    }
    let mut mode: Mode = Mode::Simulate;
    let mut mode_explicit = false;
    // §20.10 / issue #107: opt-in promotion of `$error` occurrences to a
    // failing exit status. `$fatal` always fails regardless of this flag.
    let mut error_exit = false;
    let mut strict_checks = true;
    let mut source_delay_select: u8 = 1;
    // Warm-start design cache is EXPERIMENTAL and OFF by default — every run
    // does a full cold elaboration. Opt in with `XEZIM_ENABLE_CACHE=1`, the
    // `--cache` flag, or `--cache-dir <dir>`. `XEZIM_NO_CACHE=1` still force-
    // disables (kept so existing scripts that set it keep working).
    let mut design_cache_enabled = env::var("XEZIM_ENABLE_CACHE").ok().as_deref() == Some("1")
        && env::var("XEZIM_NO_CACHE").ok().as_deref() != Some("1");
    let mut design_cache_dir: Option<PathBuf> = None;
    // Cache compression settings
    let mut cache_compression_level: Option<i32> = None;
    let mut cache_stats = false;

    // Check environment variables for cache compression settings
    if let Ok(level_str) = env::var("XEZIM_CACHE_COMPRESSION_LEVEL") {
        if let Ok(level) = level_str.parse::<i32>() {
            cache_compression_level = Some(level);
        }
    }
    if env::var("XEZIM_CACHE_STATS").ok().as_deref() == Some("1") {
        cache_stats = true;
    }
    let mut verbose = false;
    let mut _output_file: Option<String> = None;
    let mut lib_dirs: Vec<String> = Vec::new();
    let mut lib_files: Vec<String> = Vec::new();
    let mut lib_exts: Option<Vec<String>> = None;
    let mut primitive_verbose = false;
    let mut nospecify = false;
    let mut log_file: Option<String> = None;
    let mut log_tee = false;
    let mut settle_limit: Option<u32> = None;
    let mut activity_mon = false;
    let mut dump_timescales = false;
    let mut sdf_file: Option<String> = None;
    let mut sdf_select: Option<xezim::compiler::sdf::DelaySelect> = None;
    let mut xtrace_file: Option<String> = None;
    let mut xtrace_scopes: Vec<String> = Vec::new();
    let mut xtrace_from_ns: u64 = 0;
    let mut xtrace_to_ns: u64 = u64::MAX;
    // XTrace compliance level (§24). We emit Level 0 (dictionary + time +
    // signal deltas, plus the §10.4 event record); levels 1-4 add the semantic,
    // transactional and retrieval layers and are RESERVED — asking for one is a
    // warning, not a silent lie in the header.
    let mut xtrace_level: u8 = 0;
    let mut xtrace_format = "text".to_string();
    let mut xtrace_profile: Option<String> = None;
    let mut xtrace_compress: Option<String> = None;
    let mut wave = false;
    // `--code-coverage[=<kinds>]` / `--code-coverage-scope`.
    let mut code_cov_kinds: Option<u8> = None;
    let mut code_cov_scopes: Vec<String> = Vec::new();
    let mut fst_file: Option<String> = None;
    let mut fst_scopes: Vec<String> = Vec::new();
    let mut sim_debug = false;
    let mut dump_files_list = false;
    let mut dump_merged_sv: Option<String> = None;
    let mut upf_files: Vec<String> = Vec::new();
    // VHDL design library of the VHDL files that follow (`--work=<lib>`).
    let mut vhdl_work = String::from("work");
    let mut vhdl_file_libs: Vec<(String, String)> = Vec::new();
    let mut dpi_libs: Vec<String> = Vec::new();
    let mut vpi_libs: Vec<String> = Vec::new();
    let mut module_timescale_args: Vec<String> = Vec::new();
    let mut plusargs: Vec<String> = Vec::new();
    let mut emit_hypergraph: Option<String> = None;
    let mut load_partition: Option<String> = None;
    let mut write_profile: Option<String> = None;
    let mut profile_input: Option<String> = None;
    let mut collapse_islands: bool = false;
    // `--report-stats[=json]`; None = no flag given, fall back to the
    // XEZIM_REPORT_STATS environment switch (resolved after the loop).
    let mut report_stats_cli: Option<report::ReportMode> = None;

    let mut include_dirs: Vec<String> = Vec::new();
    let mut defines: Vec<(String, Option<String>)> = Vec::new();

    let mut compat = cli_compat::CompatArgs::default();
    let mut max_time_explicit = false;

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        // Other simulators' spellings first: several (`-sv_seed`, `-suppress`,
        // `-lib`) would otherwise read as the glued `-s<top>` / `-l<file>`.
        match cli_compat::handle_flag(&args, i, &mut compat, &mut plusargs) {
            Ok(0) => {}
            Ok(n) => {
                i += n;
                continue;
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        match arg.as_str() {
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            "--show-env-avail" => {
                xezim::env_vars::print_env_avail();
                std::process::exit(0);
            }
            "-I" => {
                i += 1;
                if i < args.len() {
                    include_dirs.push(args[i].clone());
                }
            }
            _ if arg.starts_with("-I") && arg.len() > 2 => {
                include_dirs.push(arg[2..].to_string());
            }
            "-D" => {
                i += 1;
                if i < args.len() {
                    push_define_token(&args[i], &mut defines);
                }
            }
            _ if arg.starts_with("-D") && arg.len() > 2 => {
                push_define_token(&arg[2..], &mut defines);
            }
            // `-override_timescale <unit>/<prec>` (ahead of the `-o<file>` prefix
            // form, which would take it): one timescale for every
            // design element, replacing every `timescale directive and
            // `timeunit`/`timeprecision` declaration.
            _ if override_timescale_flag(arg).is_some() => match override_timescale_flag(arg) {
                Some(Some(v)) => {
                    module_timescale_args.push(format!("{}{}", OVERRIDE_TIMESCALE_TAG, v))
                }
                _ => {
                    i += 1;
                    if i < args.len() {
                        module_timescale_args
                            .push(format!("{}{}", OVERRIDE_TIMESCALE_TAG, args[i]));
                    } else {
                        eprintln!("Error: -override_timescale requires <unit>/<precision>");
                        std::process::exit(1);
                    }
                }
            },
            "-o" => {
                i += 1;
                if i < args.len() {
                    _output_file = Some(args[i].clone());
                }
            }
            _ if arg.starts_with("-o") && arg.len() > 2 => {
                _output_file = Some(arg[2..].to_string());
            }
            "--tee" => {
                i += 1;
                if let Some(path) = args.get(i) {
                    log_file = Some(path.clone());
                    log_tee = true;
                } else {
                    eprintln!("Error: --tee requires <file>");
                    std::process::exit(1);
                }
            }
            _ if arg.starts_with("--tee=") => {
                log_file = Some(arg["--tee=".len()..].to_string());
                log_tee = true;
            }
            "-l" | "--log" | "-logfile" => {
                i += 1;
                if i < args.len() {
                    log_file = Some(args[i].clone());
                }
            }
            _ if arg.starts_with("--log=") => {
                log_file = Some(arg["--log=".len()..].to_string());
            }
            _ if arg.starts_with("-l") && arg.len() > 2 => {
                log_file = Some(arg[2..].to_string());
            }
            "-s" => {
                i += 1;
                if i < args.len() {
                    top_modules.push(args[i].clone());
                }
            }
            _ if arg.starts_with("-s") && arg.len() > 2 => {
                top_modules.push(arg[2..].to_string());
            }
            // `-c` with no file after it: batch-mode switch, nothing to do.
            "-c" if !cli_compat::c_takes_file(
                args.get(i + 1).map(|s| s.as_str()),
                &compat.libs,
            ) => {}
            // `-F` also resolves `+incdir+` paths against the args file's
            // directory first (resolve_rel_file_first); `-file` is `-f`.
            // `--fst-scope-file <file>`: one `--fst-scope` per listed scope.
            // Ahead of the `-f<file>` args-file prefix form, which would
            // otherwise take `-fst_scope_file`.
            _ if cli_files::option_value(arg, cli_files::FST_SCOPE_FILE_NAMES).is_some() => {
                let path = match cli_files::option_value(arg, cli_files::FST_SCOPE_FILE_NAMES) {
                    Some(Some(v)) => v.to_string(),
                    _ => {
                        i += 1;
                        match args.get(i) {
                            Some(v) => v.clone(),
                            None => {
                                eprintln!("Error: -fst_scope_file requires a file");
                                std::process::exit(1);
                            }
                        }
                    }
                };
                match std::fs::read_to_string(&path) {
                    Ok(text) => fst_scopes.extend(cli_files::parse_scope_file(&text)),
                    Err(e) => {
                        eprintln!("Error: cannot read -fst_scope_file '{}': {}", path, e);
                        std::process::exit(1);
                    }
                }
            }
            // `-xezim_env <file>` was applied at start-up (apply_xezim_env_files).
            _ if cli_files::option_value(arg, cli_files::XEZIM_ENV_NAMES).is_some() => {
                if cli_files::option_value(arg, cli_files::XEZIM_ENV_NAMES) == Some(None) {
                    i += 1;
                }
            }
            "-c" | "-f" | "-F" | "-file" => {
                let incdir_rel = arg == "-F";
                i += 1;
                if i < args.len() {
                    match process_command_file(
                        &args[i],
                        &mut source_files,
                        &mut top_modules,
                        &mut include_dirs,
                        &mut defines,
                        &mut lib_dirs,
                        &mut plusargs,
                        &mut lib_files,
                        &mut lib_exts,
                        &mut nospecify,
                        &mut primitive_verbose,
                        &mut module_timescale_args,
                        &mut compat,
                        incdir_rel,
                    ) {
                        Ok(()) => {}
                        Err(e) => {
                            eprintln!("{}", e);
                            std::process::exit(1);
                        }
                    }
                }
            }
            _ if arg.starts_with("-f") && arg.len() > 2 => {
                match process_command_file(
                    &arg[2..],
                    &mut source_files,
                    &mut top_modules,
                    &mut include_dirs,
                    &mut defines,
                    &mut lib_dirs,
                    &mut plusargs,
                    &mut lib_files,
                    &mut lib_exts,
                    &mut nospecify,
                    &mut primitive_verbose,
                    &mut module_timescale_args,
                    &mut compat,
                    false,
                ) {
                    Ok(()) => {}
                    Err(e) => {
                        eprintln!("{}", e);
                        std::process::exit(1);
                    }
                }
            }
            "-y" => {
                i += 1;
                if i < args.len() {
                    lib_dirs.push(args[i].clone());
                    include_dirs.push(args[i].clone());
                }
            }
            _ if arg.starts_with("-y") && arg.len() > 2 => {
                lib_dirs.push(arg[2..].to_string());
                include_dirs.push(arg[2..].to_string());
            }
            "--lib" => {
                i += 1;
                if i < args.len() {
                    lib_dirs.push(args[i].clone());
                    include_dirs.push(args[i].clone());
                }
            }
            _ if arg.starts_with("+incdir+") => {
                push_plus_incdir(arg, &mut include_dirs);
            }
            _ if arg.starts_with("+define+") => {
                push_plus_define(arg, &mut defines);
            }
            _ if arg.starts_with("+libext+") => {
                push_plus_libext(arg, &mut lib_exts);
            }
            // Commercial GLS flags. `+nospecify` suppresses specify-block path
            // delays (zero-delay gate sim) and timing checks; `+notimingcheck(s)`
            // only the timing checks. The `-` spellings are accepted too.
            "+nospecify" | "-nospecify" => {
                nospecify = true;
            }
            // min:typ:max selection — governs specify-path triplets and, when
            // no --sdf-min/typ/max was given, the SDF annotation too.
            "+mindelays" | "-mindelays" => {
                xezim::sv_parser::set_delay_select(0);
                source_delay_select = 0;
                if sdf_select.is_none() {
                    sdf_select = Some(xezim::compiler::sdf::DelaySelect::Min);
                }
            }
            "+typdelays" | "-typdelays" => {
                xezim::sv_parser::set_delay_select(1);
                source_delay_select = 1;
                if sdf_select.is_none() {
                    sdf_select = Some(xezim::compiler::sdf::DelaySelect::Typ);
                }
            }
            "+maxdelays" | "-maxdelays" => {
                xezim::sv_parser::set_delay_select(2);
                source_delay_select = 2;
                if sdf_select.is_none() {
                    sdf_select = Some(xezim::compiler::sdf::DelaySelect::Max);
                }
            }
            "+notimingcheck" | "+notimingchecks" | "-notimingchecks" => {
                xezim::compiler::simulator::set_no_timing_checks(true);
            }
            _ if handle_gls_flag(arg) => {}
            _ if arg.starts_with('+') => {
                // `+X_WARN` is both a plusarg (so `$test$plusargs` can see it)
                // and the switch itself — this arm runs before the explicit
                // match below, so handle it here too or the flag is swallowed.
                if arg.eq_ignore_ascii_case("+X_WARN") {
                    xezim::compiler::simulator::set_warn_x(true);
                }
                if let Some(n) = arg
                    .strip_prefix("+X_WARN_LIMIT=")
                    .or_else(|| arg.strip_prefix("+x_warn_limit="))
                    .and_then(|v| v.parse::<usize>().ok())
                {
                    xezim::compiler::simulator::set_warn_x(true);
                    xezim::compiler::simulator::set_warn_x_limit(n);
                }
                plusargs.push(arg.clone());
            }
            // `-v <file>` — a library FILE (traditional library semantics): its
            // modules are compiled on demand to satisfy unresolved
            // instantiations and are never top-module candidates. The old
            // "verbose" meaning of -v (which controlled nothing) moved to
            // --verbose.
            "-v" => {
                i += 1;
                if i < args.len() {
                    lib_files.push(args[i].clone());
                } else {
                    eprintln!("Error: -v requires a library file name");
                    std::process::exit(1);
                }
            }
            "--error-exit" => {
                error_exit = true;
            }
            // §6.21: downgrade the implicitly-static-initializer error to a
            // warning. Real designs carry the pattern and other simulators
            // let it be suppressed, so a user hitting it mid-flow needs a way
            // forward that is not "edit a vendor's source tree".
            "--relax-implicit-static" => {
                xezim_core::elaborate::set_relax_implicit_static(true);
            }
            // §6.5/§9.2.2: downgrade a conflicting variable driver (two
            // continuous assigns, continuous plus procedural, an always_comb/
            // always_latch/always_ff variable written by another process) to
            // a warning, for existing designs written against tools that
            // accept them.
            "--relax-driver-conflicts" => {
                xezim_core::elaborate::set_relax_driver_conflicts(true);
            }
            // The end-of-run [PROF] report (edge-block, settle and timing
            // counters). Every reader consults the environment switch, so
            // the flag sets it before the simulator is constructed.
            "--profile" => {
                // SAFETY: argument parsing runs single-threaded, before any
                // simulator thread exists.
                unsafe { std::env::set_var("XEZIM_PROFILE_REPORT", "1") };
            }
            "--verbose" => {
                verbose = true;
            }
            "--primitive-verbose" => {
                primitive_verbose = true;
            }
            // Suppress §6.10 implicit 1-bit net warnings (gate-level designs
            // with unresolved vendor cells can emit thousands).
            "-xenowarn" => {
                xezim::set_implicit_net_warn(false);
            }
            // §107: `-s <name>` naming no known module is an ERROR rather than
            // a warning plus auto-detection, so a scripted flow that keys off
            // the exit status catches a typo'd or stale top.
            "--strict-top" => {
                xezim::set_strict_top(true);
            }
            "--no-strict-top" => {
                xezim::set_strict_top(false);
            }
            "-V" => {
                print_version();
                std::process::exit(0);
            }
            "--preprocess" => {
                if mode_explicit && mode != Mode::Preprocess {
                    eprintln!("Error: --preprocess conflicts with previously set mode");
                    std::process::exit(1);
                }
                mode = Mode::Preprocess;
                mode_explicit = true;
            }
            "--parse" => {
                if mode_explicit && mode != Mode::Parse {
                    eprintln!("Error: --parse conflicts with previously set mode");
                    std::process::exit(1);
                }
                mode = Mode::Parse;
                mode_explicit = true;
            }
            "--compile" | "--no-sim" => {
                if mode_explicit && mode != Mode::Compile {
                    eprintln!("Error: --compile conflicts with previously set mode");
                    std::process::exit(1);
                }
                mode = Mode::Compile;
                mode_explicit = true;
            }
            "--simulate" => {
                if mode_explicit && mode != Mode::Simulate {
                    eprintln!("Error: --simulate conflicts with previously set mode");
                    std::process::exit(1);
                }
                mode = Mode::Simulate;
                mode_explicit = true;
            }
            "--sv2023" => {
                // No-op now (default), kept for back-compat with existing scripts.
                sv_parser::set_sv2023(true);
            }
            "--sv2017" => {
                sv_parser::set_sv2023(false);
            }
            // Strict negative-test diagnostics (reject LRM-illegal constructs).
            // ON by default; `--no-strict` (alias `--lenient`) turns it off.
            "--strict" => {
                sv_parser::set_strict_checks(true);
                strict_checks = true;
            }
            // X_WARN: warn the first time each signal takes an x bit after
            // time 0, naming the signal, its instance/module and its drivers.
            // Accepted as a flag, as `+X_WARN` (plusarg), and as `X_WARN=1`
            // in the environment — same switch, three spellings, because run
            // scripts reach for different ones.
            "--x-warn" | "--X_WARN" | "-X_WARN" | "+X_WARN" => {
                xezim::compiler::simulator::set_warn_x(true);
            }
            "--x-warn-limit" | "--X_WARN_LIMIT" => {
                i += 1;
                if i < args.len() {
                    match args[i].parse::<usize>() {
                        Ok(n) => xezim::compiler::simulator::set_warn_x_limit(n),
                        Err(_) => {
                            eprintln!("Error: --x-warn-limit requires a number (0 = unlimited)");
                            std::process::exit(1);
                        }
                    }
                }
            }
            _ if arg.starts_with("--x-warn-limit=") => {
                match arg["--x-warn-limit=".len()..].parse::<usize>() {
                    Ok(n) => xezim::compiler::simulator::set_warn_x_limit(n),
                    Err(_) => {
                        eprintln!("Error: --x-warn-limit requires a number (0 = unlimited)");
                        std::process::exit(1);
                    }
                }
            }
            "--no-strict" | "--lenient" => {
                sv_parser::set_strict_checks(false);
                strict_checks = false;
            }
            "--dump-tokens" => {
                dump_tokens = true;
                if !mode_explicit {
                    mode = Mode::Parse;
                }
            }
            "--dump-ast" => {
                dump_ast = true;
                if !mode_explicit {
                    mode = Mode::Parse;
                }
            }
            "--max-time" => {
                i += 1;
                if i < args.len() {
                    match parse_max_time(&args[i]) {
                        Ok(v) => {
                            max_time = v;
                            max_time_explicit = true;
                        }
                        Err(e) => {
                            eprintln!("{}", e);
                            std::process::exit(1);
                        }
                    }
                }
            }
            _ if arg.starts_with("--max-time=") => {
                match parse_max_time(&arg["--max-time=".len()..]) {
                    Ok(v) => {
                        max_time = v;
                        max_time_explicit = true;
                    }
                    Err(e) => {
                        eprintln!("{}", e);
                        std::process::exit(1);
                    }
                }
            }
            "--settle-limit" => {
                i += 1;
                if i < args.len() {
                    settle_limit = Some(args[i].parse().unwrap_or(1000));
                }
            }
            "--activity-mon" => {
                activity_mon = true;
            }
            "--dump-timescales" | "--dump-timescale" => {
                dump_timescales = true;
            }
            "--sdf" => {
                i += 1;
                if i < args.len() {
                    sdf_file = Some(args[i].clone());
                }
            }
            "--sdf-min" => {
                sdf_select = Some(xezim::compiler::sdf::DelaySelect::Min);
            }
            "--sdf-typ" => {
                sdf_select = Some(xezim::compiler::sdf::DelaySelect::Typ);
            }
            "--sdf-max" => {
                sdf_select = Some(xezim::compiler::sdf::DelaySelect::Max);
            }
            "--xtrace" => {
                i += 1;
                if i < args.len() {
                    xtrace_file = Some(args[i].clone());
                }
            }
            _ if arg.starts_with("--xtrace=") => {
                xtrace_file = Some(arg["--xtrace=".len()..].to_string());
            }
            "--xtrace-scope" => {
                i += 1;
                if i < args.len() {
                    xtrace_scopes.push(args[i].clone());
                }
            }
            _ if arg.starts_with("--xtrace-scope=") => {
                xtrace_scopes.push(arg["--xtrace-scope=".len()..].to_string());
            }
            "--xtrace-from" => {
                i += 1;
                if i < args.len() {
                    xtrace_from_ns = args[i].parse().unwrap_or(0);
                }
            }
            _ if arg.starts_with("--xtrace-from=") => {
                xtrace_from_ns = arg["--xtrace-from=".len()..].parse().unwrap_or(0);
            }
            "--xtrace-to" => {
                i += 1;
                if i < args.len() {
                    xtrace_to_ns = args[i].parse().unwrap_or(u64::MAX);
                }
            }
            _ if arg.starts_with("--xtrace-to=") => {
                xtrace_to_ns = arg["--xtrace-to=".len()..].parse().unwrap_or(u64::MAX);
            }
            "--xtrace-level" => {
                i += 1;
                if i < args.len() {
                    xtrace_level = args[i].parse().unwrap_or(0);
                }
            }
            _ if arg.starts_with("--xtrace-level=") => {
                xtrace_level = arg["--xtrace-level=".len()..].parse().unwrap_or(0);
            }
            "--xtrace-format" => {
                i += 1;
                if i < args.len() {
                    xtrace_format = args[i].clone();
                }
            }
            _ if arg.starts_with("--xtrace-format=") => {
                xtrace_format = arg["--xtrace-format=".len()..].to_string();
            }
            "--xtrace-profile" => {
                i += 1;
                if i < args.len() {
                    xtrace_profile = Some(args[i].clone());
                }
            }
            _ if arg.starts_with("--xtrace-profile=") => {
                xtrace_profile = Some(arg["--xtrace-profile=".len()..].to_string());
            }
            "--xtrace-compress" => {
                i += 1;
                if i < args.len() {
                    xtrace_compress = Some(args[i].clone());
                }
            }
            _ if arg.starts_with("--xtrace-compress=") => {
                xtrace_compress = Some(arg["--xtrace-compress=".len()..].to_string());
            }
            "--wave" => {
                wave = true;
            }
            "-vcd" => {
                wave = true;
                if args.get(i + 1).is_some_and(|a| a == "dumpon") {
                    i += 1;
                }
            }
            "--code-coverage" => {
                code_cov_kinds = Some(
                    xezim::compiler::simulator::KIND_STATEMENT
                        | xezim::compiler::simulator::KIND_BRANCH
                        | xezim::compiler::simulator::KIND_TOGGLE,
                );
            }
            _ if arg.starts_with("--code-coverage=") => {
                match xezim::compiler::simulator::parse_code_coverage_kinds(
                    &arg["--code-coverage=".len()..],
                ) {
                    Ok(k) => code_cov_kinds = Some(k),
                    Err(e) => {
                        eprintln!("Error: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            "--code-coverage-scope" => {
                i += 1;
                match args.get(i) {
                    Some(v) => code_cov_scopes.extend(
                        v.split(',')
                            .map(str::trim)
                            .filter(|p| !p.is_empty())
                            .map(String::from),
                    ),
                    None => {
                        eprintln!("Error: --code-coverage-scope requires an instance path");
                        std::process::exit(1);
                    }
                }
            }
            _ if arg.starts_with("--code-coverage-scope=") => {
                code_cov_scopes.extend(
                    arg["--code-coverage-scope=".len()..]
                        .split(',')
                        .map(str::trim)
                        .filter(|p| !p.is_empty())
                        .map(String::from),
                );
            }
            "--fst" => {
                i += 1;
                if i < args.len() {
                    fst_file = Some(args[i].clone());
                }
            }
            _ if arg.starts_with("--fst=") => {
                fst_file = Some(arg["--fst=".len()..].to_string());
            }
            "--fst-scope" => {
                i += 1;
                if i < args.len() {
                    fst_scopes.push(args[i].clone());
                }
            }
            _ if arg.starts_with("--fst-scope=") => {
                fst_scopes.push(arg["--fst-scope=".len()..].to_string());
            }
            // `--sim_debug` kept as a compatibility alias for existing scripts.
            "--sim-debug" | "--sim_debug" => {
                sim_debug = true;
            }
            "--dump-files-list" => {
                dump_files_list = true;
            }
            "--upf" => {
                i += 1;
                if i < args.len() {
                    upf_files.push(args[i].clone());
                    xezim_core::upf::add_upf_file(args[i].clone());
                } else {
                    eprintln!("Error: --upf requires a UPF file name");
                    std::process::exit(1);
                }
            }
            _ if arg.starts_with("--work=") => {
                vhdl_work = arg["--work=".len()..].to_ascii_lowercase();
                if vhdl_work.is_empty() {
                    eprintln!("Error: --work= requires a VHDL library name");
                    std::process::exit(1);
                }
            }
            _ if arg.starts_with("--std=") => {
                let v = &arg["--std=".len()..];
                if !matches!(v, "08" | "2008") {
                    eprintln!(
                        "Error: --std={}: only VHDL-2008 (--std=08) is supported",
                        v
                    );
                    std::process::exit(1);
                }
            }
            "--upf-top" => {
                i += 1;
                if i < args.len() {
                    xezim_core::upf::set_upf_top(args[i].clone());
                } else {
                    eprintln!("Error: --upf-top requires an instance path");
                    std::process::exit(1);
                }
            }
            "--dump-merged-sv" => {
                i += 1;
                if i < args.len() {
                    dump_merged_sv = Some(args[i].clone());
                } else {
                    eprintln!("Error: --dump-merged-sv requires an output file name");
                    std::process::exit(1);
                }
            }
            _ if arg.starts_with("--dump-merged-sv=") => {
                dump_merged_sv = Some(arg["--dump-merged-sv=".len()..].to_string());
            }
            "--threads" => {
                // Removed. Consume the count so it isn't taken for a source file.
                i += 1;
                eprintln!("Warning: --threads has been removed (ignored)");
            }
            "--report-stats" => {
                report_stats_cli = Some(report::ReportMode::Human);
            }
            _ if arg.starts_with("--report-stats=") => {
                let fmt = &arg["--report-stats=".len()..];
                if fmt == "json" {
                    report_stats_cli = Some(report::ReportMode::Json);
                } else {
                    eprintln!(
                        "Error: --report-stats={}: unknown format (expected 'json')",
                        fmt
                    );
                    std::process::exit(1);
                }
            }
            "--cache-dir" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: --cache-dir requires a directory");
                    std::process::exit(1);
                }
                design_cache_dir = Some(PathBuf::from(&args[i]));
                design_cache_enabled = true;
            }
            _ if arg.starts_with("--cache-dir=") => {
                design_cache_dir = Some(PathBuf::from(&arg["--cache-dir=".len()..]));
                design_cache_enabled = true;
            }
            "--no-cache" => {
                design_cache_enabled = false;
            }
            "--cache" => {
                // Explicit opt-in to the experimental warm-start cache.
                design_cache_enabled = true;
            }
            // Artifact (-o) compression: `none` writes raw bincode (larger,
            // instant load); a number is a zstd level. Default: zstd level 3.
            "--artifact-compression" => {
                i += 1;
                let v = args.get(i).cloned().unwrap_or_default();
                match v.as_str() {
                    "none" | "off" | "0" => xezim_core::set_artifact_uncompressed(true),
                    _ => match v.parse::<i32>() {
                        Ok(n) if (1..=22).contains(&n) => xezim_core::set_zstd_level(n),
                        _ => {
                            eprintln!(
                                "Error: --artifact-compression takes 'none' or a zstd level 1-22"
                            );
                            std::process::exit(1);
                        }
                    },
                }
            }
            _ if arg.starts_with("--artifact-compression=") => {
                let v = &arg["--artifact-compression=".len()..];
                match v {
                    "none" | "off" | "0" => xezim_core::set_artifact_uncompressed(true),
                    _ => match v.parse::<i32>() {
                        Ok(n) if (1..=22).contains(&n) => xezim_core::set_zstd_level(n),
                        _ => {
                            eprintln!(
                                "Error: --artifact-compression takes 'none' or a zstd level 1-22"
                            );
                            std::process::exit(1);
                        }
                    },
                }
            }
            "--cache-compression-level" => {
                i += 1;
                if i < args.len() {
                    if let Ok(level) = args[i].parse::<i32>() {
                        cache_compression_level = Some(level);
                    } else {
                        eprintln!(
                            "Error: --cache-compression-level requires a number between 1 and 22"
                        );
                        std::process::exit(1);
                    }
                } else {
                    eprintln!("Error: --cache-compression-level requires a number");
                    std::process::exit(1);
                }
            }
            _ if arg.starts_with("--cache-compression-level=") => {
                if let Ok(level) = arg["--cache-compression-level=".len()..].parse::<i32>() {
                    cache_compression_level = Some(level);
                } else {
                    eprintln!(
                        "Error: --cache-compression-level requires a number between 1 and 22"
                    );
                    std::process::exit(1);
                }
            }
            "--cache-stats" => {
                cache_stats = true;
            }
            "--emit-hypergraph" => {
                i += 1;
                if i < args.len() {
                    emit_hypergraph = Some(args[i].clone());
                }
            }
            _ if arg.starts_with("--emit-hypergraph=") => {
                emit_hypergraph = Some(arg["--emit-hypergraph=".len()..].to_string());
            }
            "--load-partition" => {
                i += 1;
                if i < args.len() {
                    load_partition = Some(args[i].clone());
                }
            }
            _ if arg.starts_with("--load-partition=") => {
                load_partition = Some(arg["--load-partition=".len()..].to_string());
            }
            "--write-profile" => {
                i += 1;
                if i < args.len() {
                    write_profile = Some(args[i].clone());
                }
            }
            _ if arg.starts_with("--write-profile=") => {
                write_profile = Some(arg["--write-profile=".len()..].to_string());
            }
            "--profile-input" => {
                i += 1;
                if i < args.len() {
                    profile_input = Some(args[i].clone());
                }
            }
            _ if arg.starts_with("--profile-input=") => {
                profile_input = Some(arg["--profile-input=".len()..].to_string());
            }
            "--collapse-islands" => {
                collapse_islands = true;
            }
            "--dpi-lib" => {
                i += 1;
                if i < args.len() {
                    dpi_libs.push(args[i].clone());
                }
            }
            // A VPI module: loaded, then its `vlog_startup_routines` runs so
            // it can register system tasks (IEEE 1800-2017 §38.2).
            "--vpi-lib" | "-m" => {
                i += 1;
                if i < args.len() {
                    vpi_libs.push(args[i].clone());
                }
            }
            // Implementation-defined extension: assign a time unit/precision to
            // module definitions that have no explicit source-level timescale.
            "--module-timescale" => {
                i += 1;
                if i < args.len() {
                    module_timescale_args.push(args[i].clone());
                }
            }
            _ if arg.starts_with("--module-timescale=") => {
                module_timescale_args.push(arg["--module-timescale=".len()..].to_string());
            }
            // commercial-simulator-compatible spelling of the same thing:
            // `-timescale <unit>/<prec>` supplies the DEFAULT for design
            // elements that carry no timescale directive, and does NOT
            // override an explicit one — matching the switch it mirrors.
            "-timescale" | "--timescale" => {
                i += 1;
                if i < args.len() {
                    module_timescale_args.push(args[i].clone());
                }
            }
            _ if arg.starts_with("-timescale=") => {
                module_timescale_args.push(arg["-timescale=".len()..].to_string());
            }
            _ if arg.starts_with("--timescale=") => {
                module_timescale_args.push(arg["--timescale=".len()..].to_string());
            }
            // commercial-simulator-compatible seed aliases (undocumented): lower onto `+seed=`.
            "-svseed" | "-seed" => {
                i += 1;
                if i < args.len() {
                    plusargs.push(format!("+seed={}", args[i]));
                }
            }
            _ if arg.starts_with("-svseed=") => {
                plusargs.push(format!("+seed={}", &arg["-svseed=".len()..]));
            }
            _ if arg.starts_with("-seed=") => {
                plusargs.push(format!("+seed={}", &arg["-seed=".len()..]));
            }
            _ if arg.starts_with('-') => {
                eprintln!("Warning: unknown flag '{}' (ignored)", arg);
            }
            // A design-unit name that is not a file names a top, as `-s` does.
            _ => match cli_compat::bare_top_name(arg, &compat.libs) {
                Some(top) => {
                    top_modules.push(top);
                }
                None => {
                    if xezim_core::vhdl::is_vhdl_path(arg) {
                        vhdl_file_libs.push((arg.clone(), vhdl_work.clone()));
                    }
                    source_files.push(arg.clone())
                }
            },
        }
        i += 1;
    }
    // `--sv2017`/`--sv2023`, or a `-sv*compat` flag here or in an args file.
    let sv2023_mode = sv_parser::is_sv2023();
    dpi_libs.extend(cli_compat::resolve_sv_libs(&compat));
    if !compat.param_overrides.is_empty() {
        xezim_core::set_param_overrides(
            compat
                .param_overrides
                .iter()
                .map(|(module, name, value, force)| xezim_core::ParamOverride {
                    module: module.clone(),
                    name: name.clone(),
                    value: value.clone(),
                    force: *force,
                })
                .collect(),
        );
    }
    // `-do`: its `run` commands set how long the run goes. An explicit
    // `--max-time` stays a hard cap on top of them.
    if !compat.do_scripts.is_empty() {
        let plan = cli_compat::plan_do_scripts(&compat.do_scripts).map(|p| {
            for w in &p.warnings {
                eprintln!("Warning: {}", w);
            }
            p.run
        });
        match plan {
            Ok(cli_compat::DoRun::All) => {
                if !max_time_explicit {
                    max_time = RUN_ALL_NS;
                }
            }
            Ok(cli_compat::DoRun::For(0)) => {
                eprintln!("Error: -do: a total run time of 0 is not supported");
                std::process::exit(1);
            }
            Ok(cli_compat::DoRun::For(ns)) => {
                if !max_time_explicit || ns <= max_time {
                    max_time = ns;
                    xezim::compiler::simulator::set_run_length_requested(true);
                }
            }
            // Loaded and quit without a `run`: elaborate only.
            Ok(cli_compat::DoRun::Load) => {
                if !mode_explicit {
                    mode = Mode::Compile;
                }
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
    }

    // Code coverage: `--code-coverage` wins over XEZIM_CODE_COVERAGE, which
    // wins over other simulators' `+cover` / `-coverage`.
    let cover_request = cli_compat::cover_request(&compat);
    if let (Some(spec), Some((kinds, unsupported))) = (&compat.cover, &cover_request) {
        if !unsupported.is_empty() {
            let names: Vec<String> = unsupported
                .chars()
                .map(|c| match c {
                    'c' => "condition (c)".to_string(),
                    'e' => "expression (e)".to_string(),
                    'f' => "FSM (f)".to_string(),
                    'x' => "extended toggle (x)".to_string(),
                    _ => format!("'{}'", c),
                })
                .collect();
            let kept: Vec<&str> = [
                (xezim::compiler::simulator::KIND_STATEMENT, "statement"),
                (xezim::compiler::simulator::KIND_BRANCH, "branch"),
                (xezim::compiler::simulator::KIND_TOGGLE, "toggle"),
            ]
            .iter()
            .filter(|(k, _)| kinds & k != 0)
            .map(|(_, n)| *n)
            .collect();
            eprintln!(
                "Warning: +cover={}: xezim has no {} coverage; collecting {}",
                spec,
                names.join(", "),
                if kept.is_empty() {
                    "no code coverage".to_string()
                } else {
                    format!("{} coverage", kept.join(", "))
                }
            );
        }
    }
    let env_cov = match env::var("XEZIM_CODE_COVERAGE") {
        Ok(v) if !v.is_empty() && v != "0" => {
            match xezim::compiler::simulator::parse_code_coverage_kinds(&v) {
                Ok(k) => Some(k),
                Err(e) => {
                    eprintln!("Error: XEZIM_CODE_COVERAGE: {}", e);
                    std::process::exit(1);
                }
            }
        }
        _ => None,
    };
    let code_cov = code_cov_kinds
        .or(env_cov)
        .or(cover_request.map(|(k, _)| k))
        .unwrap_or(0);
    if code_cov != 0 {
        xezim::compiler::simulator::set_code_coverage(Some(
            xezim::compiler::simulator::CodeCoverage {
                kinds: code_cov,
                scopes: code_cov_scopes,
            },
        ));
    } else if !code_cov_scopes.is_empty() {
        eprintln!("Warning: --code-coverage-scope has no effect without --code-coverage");
    }

    // Opt-in statistics footer: the CLI flag wins over XEZIM_REPORT_STATS.
    // Off (the default) is fully inert — nothing is collected or printed.
    let report_mode = report_stats_cli.unwrap_or_else(|| {
        report::mode_from_env_value(env::var("XEZIM_REPORT_STATS").ok().as_deref())
    });

    // XEZIM_VERBOSE=1 is `--verbose` for scripts that cannot add a flag.
    if env::var("XEZIM_VERBOSE").ok().as_deref() == Some("1") {
        verbose = true;
    }
    if verbose {
        xezim::set_compile_verbose(true);
    }
    // Internal engine chatter (run banner, [PHASE] timings, end-of-run engine
    // counters) is off by default: a plain run prints the design's own
    // output, warnings/errors and the final result line. `--profile` sets
    // XEZIM_PROFILE_REPORT while parsing, so both spellings turn it on, and
    // XEZIM_PROFILE_TIMING's timers are only ever read from those lines.
    let env_on = |k: &str| env::var(k).ok().as_deref() == Some("1");
    xezim::set_verbose(
        verbose || sim_debug || env_on("XEZIM_PROFILE_REPORT") || env_on("XEZIM_PROFILE_TIMING"),
    );

    // `--fst-scope-file` given inside an args file.
    for f in std::mem::take(&mut compat.fst_scope_files) {
        match std::fs::read_to_string(&f) {
            Ok(text) => fst_scopes.extend(cli_files::parse_scope_file(&text)),
            Err(e) => {
                eprintln!("Error: cannot read -fst_scope_file '{}': {}", f, e);
                std::process::exit(1);
            }
        }
    }

    // `-uvm`: add the UVM library that `XEZIM_UVM_DIR` names (its `src`
    // include directory and `uvm_pkg.sv`; src/uvm_dir.rs). Before
    // `--dump-files-list`, so the list shows what was added.
    match uvm_dir::apply_uvm_dir(
        compat.uvm,
        env::var("XEZIM_UVM_DIR").ok().as_deref(),
        env::var("XEZIM_UVM_VERSION").ok().as_deref(),
        &mut source_files,
        &mut include_dirs,
    ) {
        uvm_dir::UvmDirAction::Added(src) if verbose => {
            eprintln!(
                "[xezim] -uvm: added {}/uvm_pkg.sv and its include directory",
                src.display()
            );
        }
        uvm_dir::UvmDirAction::Error(why) => {
            eprintln!("Error: {}", why);
            std::process::exit(1);
        }
        _ => {}
    }

    // `--dump-files-list`: the fully resolved compilation file set, after every
    // `-f` args file has been expanded. Printed BEFORE the files are read so
    // the list still appears when a file is missing or fails to parse — that
    // is exactly the situation the flag exists to debug.
    if dump_files_list {
        println!("=== files list: {} source file(s) ===", source_files.len());
        for (i, sf) in source_files.iter().enumerate() {
            let exists = Path::new(sf).exists();
            println!(
                "  {:>4}. {}{}",
                i + 1,
                sf,
                if exists { "" } else { "   [NOT FOUND]" }
            );
        }
        if !lib_files.is_empty() {
            println!("--- -v library file(s): {} ---", lib_files.len());
            for lf in &lib_files {
                println!("  {}", lf);
            }
        }
        if !lib_dirs.is_empty() {
            println!("--- -y library dir(s): {} ---", lib_dirs.len());
            for ld in &lib_dirs {
                println!("  {}", ld);
            }
        }
        if !include_dirs.is_empty() {
            println!("--- include dir(s): {} ---", include_dirs.len());
            for id in &include_dirs {
                println!("  {}", id);
            }
        }
        println!("=== end files list ===");
    }

    if source_files.is_empty() {
        eprintln!("Error: no source files specified");
        print_usage();
        std::process::exit(1);
    }
    // VHDL sources go to the VHDL front end, each into the library the
    // last `--work=` before it named (args-file entries: `work`).
    let (vhdl_sources, sv_sources): (Vec<String>, Vec<String>) = std::mem::take(&mut source_files)
        .into_iter()
        .partition(|f| xezim_core::vhdl::is_vhdl_path(f));
    source_files = sv_sources;
    xezim_core::vhdl::configure(
        vhdl_sources
            .into_iter()
            .map(|path| {
                let library = vhdl_file_libs
                    .iter()
                    .find(|(p, _)| *p == path)
                    .map_or_else(|| "work".to_string(), |(_, l)| l.clone());
                xezim_core::vhdl::VhdlFile { path, library }
            })
            .collect(),
    );
    // A VHDL simulation runs until no events are left (IEEE 1076-2008
    // §14.7.5), as GHDL does: no default run-length limit.
    if xezim_core::vhdl::configured() && !max_time_explicit {
        max_time = RUN_ALL_NS;
    }

    // XTrace option validation. Every one of these degrades to what we really
    // emit and SAYS SO — the header must never claim a level, a format or a
    // transport the file does not carry (XTrace §6, §24).
    if xtrace_level != 0 {
        eprintln!(
            "Warning: --xtrace-level {} is reserved; emitting level 0 signal deltas",
            xtrace_level
        );
        xtrace_level = 0;
    }
    let _ = xtrace_level;
    if xtrace_format != "text" {
        eprintln!(
            "Warning: --xtrace-format '{}' is reserved; emitting text",
            xtrace_format
        );
    }
    if xtrace_compress.as_deref() == Some("none") {
        xtrace_compress = None;
    }
    if let Some(ref c) = xtrace_compress {
        if c != "zstd" {
            eprintln!(
                "Warning: --xtrace-compress '{}' is unknown; writing uncompressed text",
                c
            );
            xtrace_compress = None;
        }
    }
    xezim::compiler::simulator::set_xtrace_options(xtrace_profile.clone(), xtrace_compress.clone());
    // `--fst`/`--xtrace` are explicit dump requests, so they enable waveform
    // support on their own; `--wave` is what a source-driven `$dumpvars` needs.
    // Installed BEFORE elaboration: the model is built differently when a dump
    // can run (see `for_loop_needs_ast`).
    xezim::compiler::simulator::set_wave_enabled(
        wave || fst_file.is_some() || xtrace_file.is_some(),
    );

    // Install the --module-timescale configuration before any elaboration.
    if !module_timescale_args.is_empty() {
        match build_module_timescale_cli(&module_timescale_args) {
            Ok(cli) => xezim::set_module_timescale_cli(cli),
            Err(e) => {
                eprintln!("error: {}", e);
                std::process::exit(1);
            }
        }
    }

    // Library search config (`-v` files, `+libext+` extensions) — consumed by
    // the core resolver that satisfies unresolved instantiations from `-y`
    // directories and `-v` files.
    if nospecify {
        xezim::compiler::simulator::set_nospecify(true);
        if sdf_file.is_some() {
            eprintln!(
                "Warning: +nospecify combined with --sdf — specify path delays are \
suppressed but the explicit SDF annotation still applies."
            );
        }
    }
    if !lib_dirs.is_empty() || !lib_files.is_empty() || lib_exts.is_some() || primitive_verbose {
        xezim::set_library_cli(xezim::LibraryCli {
            lib_files: lib_files.clone(),
            lib_dirs: lib_dirs.clone(),
            lib_exts: lib_exts.clone(),
            primitive_verbose,
        });
    }

    if let Some(ref path) = log_file {
        let result = if log_tee {
            tee_stdio_to_log(path)
        } else {
            redirect_stdio_to_log(path)
        };
        if let Err(e) = result {
            eprintln!("Error: cannot open log file '{}': {}", path, e);
            std::process::exit(1);
        }
    }

    // Print build identity before loading, compiling, or running the design so
    // every simulation log identifies the exact binary that produced it.
    if mode == Mode::Simulate {
        print_simulation_banner();
    }

    if design_cache_enabled && mode == Mode::Simulate {
        let directory = design_cache_dir
            .clone()
            .unwrap_or_else(default_design_cache_dir);
        let dependency_files = design_dependency_files(&lib_files, &lib_dirs, lib_exts.as_deref());
        let semantic_salt = format!(
            "sv2023={};strict={};delay_select={};module_timescale={:?};lib_dirs={:?};lib_files={:?};lib_exts={:?};nospecify={};param_overrides={:?};vhdl={:?}",
            sv2023_mode,
            strict_checks,
            source_delay_select,
            module_timescale_args,
            lib_dirs,
            lib_files,
            lib_exts,
            nospecify,
            compat.param_overrides,
            xezim_core::vhdl::cache_digest(),
        );
        // Set cache compression settings before cache is used
        if let Some(level) = cache_compression_level {
            xezim_core::set_zstd_level(level);
        }
        if cache_stats {
            xezim_core::set_compression_stats(true);
        }

        xezim::set_design_cache(Some(xezim::DesignCacheConfig {
            directory,
            semantic_salt,
            dependency_files,
        }));
    } else {
        xezim::set_design_cache(None);
    }

    // Fast path: if the only source file is a xezim compiled artifact, load
    // it and jump straight to simulation (skip parse + elaborate).
    if source_files.len() == 1 && mode == Mode::Simulate {
        let sf = &source_files[0];
        if let Ok(head) = std::fs::read(sf)
            .as_ref()
            .map(|v| v.iter().take(8).copied().collect::<Vec<u8>>())
        {
            // Recognize the artifact family before checking its version.
            // The loader must diagnose an older/newer layout rather than
            // send binary payload bytes to the source parser.
            if head.len() == 8 && head[..7] == xezim::XEZIM_BYTECODE_MAGIC[..7] {
                match xezim::read_compiled(sf) {
                    Ok(Some(elab)) => {
                        chatter_out!("Loaded compiled: {}", sf);
                        chatter_out!("Max time: {} ns", max_time);
                        chatter_out!("------------------------------");
                        let total_start = xezim::WallTimer::now();
                        xezim::compiler::simulator::set_sim_debug(sim_debug);
                        xezim::compiler::simulator::set_dump_timescales(dump_timescales);
                        xezim::compiler::simulator::set_dpi_libs(&dpi_libs);
                        xezim::compiler::simulator::set_vpi_libs(&vpi_libs);
                        let mut sim = xezim::compiler::Simulator::new(elab, max_time);
                        if let Some(limit) = settle_limit {
                            sim.settle_limit = limit;
                        }
                        sim.activity_mon = activity_mon;
                        sim.xtrace_file = xtrace_file.clone();
                        sim.xtrace_scopes = xtrace_scopes.clone();
                        sim.xtrace_from_ns = xtrace_from_ns;
                        sim.xtrace_to_ns = xtrace_to_ns;
                        sim.fst_file = fst_file.clone();
                        sim.fst_scopes = fst_scopes.clone();
                        sim.set_plusargs(&plusargs);
                        // Pass the full CLI invocation (binary name +
                        // all args + plusargs) so vpi_get_vlog_info
                        // can hand the same argv back to UVM.
                        sim.set_args(&args);
                        let compilation_start = xezim::WallTimer::now();
                        sim.compile();
                        if !sim.compile_errors.is_empty() {
                            for e in &sim.compile_errors {
                                eprintln!("Error: {}", e);
                            }
                            std::process::exit(1);
                        }
                        chatter!(
                            "[PHASE] compilation: {:.1}ms",
                            compilation_start.elapsed().as_secs_f64() * 1000.0
                        );
                        let simulation_start = xezim::WallTimer::now();
                        sim.simulate();
                        chatter!(
                            "[PHASE] simulation: {:.1}ms",
                            simulation_start.elapsed().as_secs_f64() * 1000.0
                        );
                        chatter!(
                            "[PHASE] total: {:.1}ms",
                            total_start.elapsed().as_secs_f64() * 1000.0
                        );
                        chatter_out!("------------------------------");
                        {
                            let (hits, last_t) = sim.settle_limit_report();
                            if hits > 0 {
                                eprintln!(
                                    "[WARN] settle limit was exhausted {} time(s) during this run (last at time {}) — results in those slots may not have converged; raise --settle-limit.",
                                    hits, last_t
                                );
                            }
                        }
                        print_finish_line(sim.time, sim.finished);
                        // Footer before the exit-status checks so it also
                        // appears for runs that end with a nonzero status.
                        emit_run_stats(report_mode, compile_wall_start, Some(sim.time));
                        if sim.stuck_clock_aborted {
                            std::process::exit(3);
                        }
                        let code = exit_status_for_severities(&sim, error_exit);
                        if code != 0 {
                            std::process::exit(code);
                        }
                        return 0;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        eprintln!("Error loading compiled artifact '{}': {}", sf, e);
                        std::process::exit(1);
                    }
                }
            }
        }
    }

    let mut sources: Vec<String> = Vec::new();
    let mut file_labels: Vec<String> = Vec::new();
    for sf in &source_files {
        let path = Path::new(sf);
        if !path.exists() {
            eprintln!("Error: file '{}' not found", sf);
            std::process::exit(1);
        }
        // Lossy decode: some real RTL files contain stray non-UTF-8 bytes
        // (e.g. latin-1 in a comment — scr1_pipe_hdu.sv). Read raw bytes and
        // replace invalid sequences with U+FFFD instead of failing the whole run.
        match std::fs::read(path) {
            Ok(bytes) => {
                file_labels.push(sf.clone());
                sources.push(
                    String::from_utf8(bytes)
                        .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
                );
            }
            Err(e) => {
                eprintln!("Error: cannot read '{}': {}", sf, e);
                std::process::exit(1);
            }
        }
    }

    let mut top_module = top_modules.last().cloned();
    // Multi-top: synthesize a single wrapper root that instantiates every
    // requested `-s` top, so all of them elaborate (UVM hdl_top + hvl_top etc.).
    // Appended after the real sources so the instantiated modules are already
    // declared; the wrapper has no macros/includes, so preprocessing is a no-op.
    if top_modules.len() > 1 {
        // The core's own multi-top root name, so every printed path drops it.
        let wrap_name = xezim_core::MULTI_TOP_WRAPPER;
        let mut body = format!("module {wrap_name};\n");
        // Instance name = module name (legal — separate namespaces), so each
        // top keeps its identity in hierarchical paths: `tb.u_m.u_i` from a
        // sibling top (a $dumpvars scope, a monitor's cross reference)
        // resolves, and dumped scopes match the reference simulator's
        // naming. The old `__xz_top_inst_<i>` alias made every such path
        // unmatchable.
        for t in top_modules.iter() {
            body.push_str(&format!("  {} {}();\n", t, t));
        }
        body.push_str("endmodule\n");
        sources.push(body);
        source_files.push("<xz_multitop>".to_string());
        file_labels.push("<xz_multitop>".to_string());
        top_module = Some(wrap_name.to_string());
    }

    let (mut preprocessed_sources, mut line_maps, mut pp_state) =
        match preprocess_sources(&sources, &source_files, &include_dirs, &defines) {
            Ok(v) => v,
            Err(errors) => {
                for e in &errors {
                    eprintln!("{}", e);
                }
                eprintln!("Error: preprocessing failed ({} error(s))", errors.len());
                std::process::exit(1);
            }
        };

    // `--dump-merged-sv <file>`: one self-contained .sv with every source in
    // parse order, fully preprocessed — `ifdef branches resolved, macros
    // expanded, `includes inlined. A 125-file `-f` build becomes a single
    // re-runnable repro for parse/elaboration debugging. Blank lines left by
    // the preprocessor are kept so line numbers inside each section still
    // match the per-file diagnostics.
    let mut merged_kept: Option<Vec<usize>> = None;
    if let Some(ref merged_out) = dump_merged_sv {
        // With `-s <top>`, keep only the files needed to elaborate that top —
        // the whole point of the flag is cutting a 125-file build down to a
        // re-runnable repro, and for a shared `-f` list most of those files
        // belong to some other top.
        let keep: Vec<usize> = match top_module.as_deref() {
            Some(top) => match merged_sv_files_for_top(top, &preprocessed_sources) {
                Some(sel) => sel,
                None => {
                    eprintln!(
                        "Warning: --dump-merged-sv: top module '{}' is not declared by any \
                         input file (a -v/-y library?); dumping all files",
                        top
                    );
                    (0..preprocessed_sources.len()).collect()
                }
            },
            None => (0..preprocessed_sources.len()).collect(),
        };
        let pruned = keep.len() < preprocessed_sources.len();
        if pruned {
            merged_kept = Some(keep.clone());
        }
        let mut out = String::new();
        out.push_str(&format!(
            "// Merged preprocessed sources — xezim {} ({} file(s))\n\
             // Defines, `ifdef selection and `include expansion already applied.\n\
             // NOTE: `timescale directives are consumed by the preprocessor and\n\
             // not re-emitted; pass --module-timescale when re-running this file.\n",
            env!("CARGO_PKG_VERSION"),
            keep.len()
        ));
        if pruned {
            out.push_str(&format!(
                "// Reduced to the files reachable from top '{}' ({} of {} inputs).\n\
                 // The closure is lexical and conservative: it may keep a file more\n\
                 // than strictly needed, never one fewer. Omit -s to dump everything.\n",
                top_module.as_deref().unwrap_or(""),
                keep.len(),
                preprocessed_sources.len()
            ));
        }
        for (n, &i) in keep.iter().enumerate() {
            out.push_str(&format!(
                "\n// ===== file {}/{}: {} =====\n",
                n + 1,
                keep.len(),
                file_labels[i]
            ));
            let text = &preprocessed_sources[i];
            out.push_str(text);
            if !text.ends_with('\n') {
                out.push('\n');
            }
        }
        match std::fs::write(merged_out, &out) {
            Ok(()) => println!(
                "Wrote merged preprocessed SV to {} ({} files{}, {} bytes)",
                merged_out,
                keep.len(),
                if pruned {
                    format!(
                        " of {}, reachable from '{}'",
                        preprocessed_sources.len(),
                        top_module.as_deref().unwrap_or("")
                    )
                } else {
                    String::new()
                },
                out.len()
            ),
            Err(e) => {
                eprintln!("Error: cannot write '{}': {}", merged_out, e);
                std::process::exit(1);
            }
        }
    }

    // Version banner for every mode except --preprocess, whose stdout must
    // stay pure source text. The simulate path adds its own Max-time lines
    // below. Build identity in --compile/--parse logs matters for exactly the
    // situation those modes are used in: debugging with a specific build.
    if mode != Mode::Preprocess && mode != Mode::Simulate {
        chatter_out!("=== xezim {} ===", env!("CARGO_PKG_VERSION"));
        chatter_out!(
            "git {} ({})",
            env!("XEZIM_GIT_HASH"),
            env!("XEZIM_GIT_DATE")
        );
    }

    if mode == Mode::Preprocess {
        // IEEE 1800-2017 §22: preprocess-only mode. The preprocessor has
        // already run (expanding macros and `\`include`s, evaluating
        // `\`ifdef`/`\`begin_keywords`, etc.); emit the expanded text. A
        // preprocessing-mode sv-test passes on a clean exit — `preprocess_sources`
        // exits 1 above if a directive genuinely failed, so reaching here means
        // success.
        for (label, source) in file_labels.iter().zip(preprocessed_sources.iter()) {
            println!("// === Preprocessed: {} ===", label);
            print!("{}", source);
        }
        return 0;
    }

    if mode == Mode::Parse {
        if dump_tokens {
            for (_i, (label, source)) in file_labels
                .iter()
                .zip(preprocessed_sources.iter())
                .enumerate()
            {
                println!("=== Tokens: {} ===", label);
                let tokens = xezim::tokenize_file(source, None);
                for tok in &tokens {
                    println!(
                        "{:?} '{}' @ {}..{}",
                        tok.kind, tok.text, tok.span.start, tok.span.end
                    );
                }
            }
        }
        let mut total_desc = 0;
        let mut total_err = 0;
        let mut total_warn = 0;
        for (fi, (label, source)) in file_labels
            .iter()
            .zip(preprocessed_sources.iter())
            .enumerate()
        {
            xezim::progress_status(&format!(
                "[{}] parsing {}/{}: {}",
                if mode == Mode::Parse {
                    "parse"
                } else {
                    "compile"
                },
                fi + 1,
                file_labels.len(),
                label.rsplit('/').next().unwrap_or(label)
            ));
            let tokens = xezim::lexer::Lexer::new(source).tokenize();
            let mut parser = sv_parser::parse::Parser::new(tokens);
            let source_ast = parser.parse_source_text();
            let diags = parser.diagnostics().to_vec();
            report_parse_diagnostics(&diags, source, line_maps[fi].as_ref(), label);
            total_desc += source_ast.descriptions.len();
            total_err += diags
                .iter()
                .filter(|d| d.severity == xezim::diagnostics::Severity::Error)
                .count();
            total_warn += diags
                .iter()
                .filter(|d| d.severity == xezim::diagnostics::Severity::Warning)
                .count();
            if dump_ast {
                println!("=== AST: {} ===", label);
                println!("{:#?}", source_ast);
            }
        }
        xezim::progress_clear();
        println!(
            "Parsed {} file(s): {} descriptions, {} errors, {} warnings",
            preprocessed_sources.len(),
            total_desc,
            total_err,
            total_warn
        );
        if total_err > 0 {
            std::process::exit(1);
        }
        return 0;
    }

    if mode == Mode::Compile {
        let mut total_desc = 0;
        let mut total_err = 0;
        let mut total_warn = 0;

        for (fi, (label, source)) in file_labels
            .iter()
            .zip(preprocessed_sources.iter())
            .enumerate()
        {
            xezim::progress_status(&format!(
                "[{}] parsing {}/{}: {}",
                if mode == Mode::Parse {
                    "parse"
                } else {
                    "compile"
                },
                fi + 1,
                file_labels.len(),
                label.rsplit('/').next().unwrap_or(label)
            ));
            let tokens = xezim::lexer::Lexer::new(source).tokenize();
            let mut parser = sv_parser::parse::Parser::new(tokens);
            let source_ast = parser.parse_source_text();
            let diags = parser.diagnostics().to_vec();
            report_parse_diagnostics(&diags, source, line_maps[fi].as_ref(), label);
            total_desc += source_ast.descriptions.len();
            total_err += diags
                .iter()
                .filter(|d| d.severity == xezim::diagnostics::Severity::Error)
                .count();
            total_warn += diags
                .iter()
                .filter(|d| d.severity == xezim::diagnostics::Severity::Warning)
                .count();
        }
        xezim::progress_clear();
        println!(
            "Parsed {} file(s): {} descriptions, {} errors, {} warnings",
            preprocessed_sources.len(),
            total_desc,
            total_err,
            total_warn
        );
        if total_err > 0 {
            std::process::exit(1);
        }

        // §9.4.5 intra-assignment delay canonicalization — keep the compiled
        // artifact consistent with the simulate path (see xezim::intra_delay).
        let sources: Vec<String> = sources
            .iter()
            .map(|s| xezim::intra_delay::rewrite_intra_assignment_delays(s))
            .collect();
        match xezim::parse_and_elaborate_multi(
            &sources,
            top_module.as_deref(),
            &include_dirs,
            &source_files,
            &defines,
        ) {
            Ok((_defs, mut elab)) => {
                // Second-pass `should_fail` lint (additive — does not alter the
                // elaboration above): reject illegal SV the main path accepts.
                let dv: Vec<&xezim::SourceDefinition> = _defs.values().collect();
                let lint = xezim::should_fail_lint::lint_should_fail(&dv, &elab);
                if !lint.is_empty() {
                    for e in &lint {
                        eprintln!("error: {}", e);
                    }
                    std::process::exit(1);
                }
                println!("Elaboration successful");
                if std::env::var("XEZIM_INST_PROF").is_ok() {
                    // Per-instantiation elaboration section timings (also
                    // prints one [IPROF] line per instance).
                    xezim::compiler::elaborate::iprof_dump();
                }
                if let Some(ref mo) = dump_merged_sv {
                    append_adopted_libs_to_merged(
                        mo,
                        &preprocessed_sources,
                        &file_labels,
                        merged_kept.as_deref(),
                    );
                }
                // The flattened-design table and CPU/memory lines are
                // engine detail; the verdict above is the mode's result.
                if xezim::verbose() {
                    print_design_summary(&_defs, &elab);
                    print_resource_usage(compile_wall_start);
                }
                emit_run_stats(report_mode, compile_wall_start, None);
                // §6.21: keep compiled artifacts consistent with the simulate
                // path — re-issue static initializers that call simulation-time
                // system functions as time-0 assignments (issue #26).
                xezim::defer_static_syscall_inits(&_defs, &mut elab);
                if let Some(ref out) = _output_file {
                    // The serialized artifact format flattens always_blocks /
                    // initial_blocks / continuous_assigns; pending_* are
                    // `#[serde(skip)]` and would be silently dropped.
                    // Materialize before serialize so the artifact is complete.
                    elab.materialize_pending();
                    match xezim::write_compiled(&elab, out) {
                        Ok(()) => println!("Wrote compiled artifact to {}", out),
                        Err(e) => {
                            eprintln!("Error writing '{}': {}", out, e);
                            std::process::exit(1);
                        }
                    }
                }
            }
            Err(e) => {
                report_fatal_error(&e);
                std::process::exit(1);
            }
        }
        return 0;
    }

    chatter_out!("Max time: {} ns", max_time);
    chatter_out!("------------------------------");
    // Hand the preprocessing above to elaboration instead of redoing it. Kept
    // local when --dump-merged-sv still needs the texts after the run.
    if dump_merged_sv.is_none() {
        pp_state.texts = std::mem::take(&mut preprocessed_sources);
        pp_state.line_maps = std::mem::take(&mut line_maps);
        xezim::stash_preprocessed(pp_state);
    }
    xezim::compiler::simulator::set_sim_debug(sim_debug);
    xezim::compiler::simulator::set_dump_timescales(dump_timescales);
    xezim::compiler::simulator::set_dpi_libs(&dpi_libs);
    xezim::compiler::simulator::set_vpi_libs(&vpi_libs);

    // The front-end pass's preprocessed texts are needed after the run only
    // for `--dump-merged-sv`; the simulator preprocesses on its own. Don't
    // keep a spare copy of the whole design resident through simulation.
    let preprocessed_sources = if dump_merged_sv.is_some() {
        preprocessed_sources
    } else {
        Vec::new()
    };
    drop(line_maps);

    // Unobserved-port elision (xezim_core::elaborate::set_port_elision): the
    // elaborator may leave out the nets of substituted input ports that the
    // SOURCES cannot reach by name. Whatever reaches nets by name from
    // outside the sources is ruled out here: waveform dumps (--wave, --fst,
    // --xtrace), VPI and DPI libraries, SDF and UPF annotation, code
    // coverage, x-warnings (they name the net that went x), the
    // partition tooling, and XEZIM_KEEP_PORTS=1. Library callers never turn
    // it on: they may look any signal up after the run.
    xezim_core::elaborate::set_port_elision(
        !xezim::compiler::simulator::wave_enabled()
            && fst_file.is_none()
            && xtrace_file.is_none()
            && vpi_libs.is_empty()
            && dpi_libs.is_empty()
            && sdf_file.is_none()
            && upf_files.is_empty()
            && !xezim_core::upf::upf_configured()
            && code_cov == 0
            && !xezim::compiler::simulator::warn_x_active()
            && emit_hypergraph.is_none()
            && load_partition.is_none()
            && write_profile.is_none()
            && profile_input.is_none()
            && std::env::var("XEZIM_KEEP_PORTS").ok().as_deref() != Some("1"),
    );

    match xezim::simulate_multi(
        &sources,
        max_time,
        top_module.as_deref(),
        &include_dirs,
        &source_files,
        settle_limit,
        activity_mon,
        sdf_file.as_deref(),
        sdf_select,
        &defines,
        &plusargs,
        xtrace_file.as_deref(),
        &xtrace_scopes,
        xtrace_from_ns,
        xtrace_to_ns,
        fst_file.as_deref(),
        &fst_scopes,
        emit_hypergraph.as_deref(),
        load_partition.as_deref(),
        write_profile.as_deref(),
        profile_input.as_deref(),
        collapse_islands,
    ) {
        Ok(sim) => {
            chatter_out!("------------------------------");
            if let Some(ref mo) = dump_merged_sv {
                append_adopted_libs_to_merged(
                    mo,
                    &preprocessed_sources,
                    &file_labels,
                    merged_kept.as_deref(),
                );
            }
            {
                let (hits, last_t) = sim.settle_limit_report();
                if hits > 0 {
                    eprintln!(
                        "[WARN] settle limit was exhausted {} time(s) during this run (last at time {}) — results in those slots may not have converged; raise --settle-limit.",
                        hits, last_t
                    );
                }
            }
            print_finish_line(sim.time, sim.finished);
            // Footer before the exit-status checks so it also appears for
            // runs that end with a nonzero status.
            emit_run_stats(report_mode, compile_wall_start, Some(sim.time));
            if sim.stuck_clock_aborted {
                // Dead-clock watchdog aborted (XEZIM_STUCK_CLOCK=abort): fail
                // fast so CI/regressions surface the hang instead of timing out.
                std::process::exit(3);
            }
            let code = exit_status_for_severities(&sim, error_exit);
            if code != 0 {
                std::process::exit(code);
            }
            // Exit here rather than returning: `main` exits with the code
            // anyway, and returning would first drop the simulator (tens of
            // millions of signal values, every side table, the elaborated
            // module). On c910 that teardown is ~1.6 s after the last line of
            // output. Everything with an observable side effect is already
            // done: waves, traces and the `$display` writer are flushed and
            // synced at the end of `simulate()`, the stats footer is printed,
            // and `process::exit` flushes Rust's own stdout buffer.
            std::process::exit(0);
        }
        Err(e) => {
            report_fatal_error(&e);
            std::process::exit(1);
        }
    }
}

/// §20.10: translate end-of-run severity state into a process exit status.
/// `$fatal` must never exit 0 — its finish_number is a diagnostics level, not
/// a success code, so any `$fatal` fails the run. `$error` only fails when the
/// user opts in with `--error-exit` (matching the "promote errors" switch other
/// simulators provide), so existing flows that tolerate errors keep working.
fn exit_status_for_severities(sim: &xezim::compiler::Simulator, error_exit: bool) -> i32 {
    if sim.saw_fatal {
        return 1;
    }
    if error_exit && sim.error_count > 0 {
        return 1;
    }
    0
}

/// Print one file's parser errors and warnings (and, under --verbose, its
/// informational notes), each at its original `file:line:col` with the
/// source line (identical repeats dropped).
fn report_parse_diagnostics(
    diags: &[xezim::diagnostics::Diagnostic],
    source: &str,
    map: Option<&xezim::sv_parser::source_map::LineMap>,
    label: &str,
) {
    use xezim::diagnostics::Severity;
    let notes = if xezim::verbose() {
        Some(Severity::Info)
    } else {
        None
    };
    for sev in [Severity::Error, Severity::Warning]
        .into_iter()
        .chain(notes)
    {
        for d in xezim::render_parse_diagnostics(diags, sev, source, map, label) {
            xezim::progress_clear();
            eprintln!("{}", d);
        }
    }
}

/// Print a fatal parse/elaboration error. A message that is already a
/// located diagnostic (`file:line:col: error: ...`, or a "Parse errors in"
/// block of them) stands on its own; anything else gets the generic prefix.
fn report_fatal_error(e: &str) {
    let first = e.lines().next().unwrap_or("");
    let located = first.starts_with("In file included from")
        || first.starts_with("Parse errors in")
        || first.starts_with("Strict check failed in")
        || first.starts_with("Preprocessing failed in")
        || first.contains(": error: ");
    if located {
        eprintln!("{}", e);
    } else {
        eprintln!("Simulation error: {}", e);
    }
}
