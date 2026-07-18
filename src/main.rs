use std::env;
use std::path::Path;

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

fn print_usage() {
    eprintln!("Usage: xezim [mode] [options] <source_files> [plusargs]");
    eprintln!("Modes (pick one; default is 'simulate'):");
    eprintln!("  --parse          Lex + parse only, report diagnostics");
    eprintln!("  --compile        Parse + elaborate, report diagnostics (no simulation)");
    eprintln!("  --simulate       Parse + elaborate + simulate (default)");
    eprintln!("Options:");
    eprintln!("  -v               Verbose output");
    eprintln!("  -V               Print version and exit");
    eprintln!("  -I <dir>         Add directory to include search path");
    eprintln!("  -D <name>[=val]  Define a macro");
    eprintln!("  -s <topmodule>   Specify the top-level module to elaborate");
    eprintln!("  --no-sim         Alias for --compile (deprecated)");
    eprintln!("  --preprocess     Run the preprocessor only; emit expanded text");
    eprintln!("  --dump-tokens    With --parse, print the token stream");
    eprintln!("  --dump-ast       With --parse, print the AST");
    eprintln!("  --max-time <n>   Set maximum simulation time (default: 100000)");
    eprintln!("  --sim_debug      Enable simulator [DEBUG]/[OPT] output");
    eprintln!("  --dpi-lib <so>   Load a DPI shared library (.so/.dylib/.dll)");
    eprintln!("  --vpi-lib <so>   Load a VPI module and run its vlog_startup_routines (-m)");
    eprintln!("  --module-timescale <unit>/<prec>            Timescale for every module with no");
    eprintln!("                     [mod1,mod2=]<unit>/<prec>   explicit source-level timescale (the");
    eprintln!("                     named form limits it to the listed modules). Repeatable. Never");
    eprintln!("                     overrides a `timeunit`/`timeprecision` decl or an active `timescale.");
    eprintln!("  --threads <n>    Worker threads (default: 1 = single-thread).");
    eprintln!("                   n>=2 offloads stdout writes to a background thread.");
    eprintln!("  -l, --log <file> Redirect all stdout/stderr (including DPI output) to <file>
  -v <file>        Library file: modules compiled only to resolve instantiations
  -y <dir>         Library directory: <module>.<ext> loaded on demand
  +libext+<ext>+.. Extension list for -y search (replaces default .v/.sv/.V)
  +nospecify       Suppress specify-block path delays (zero-delay gate sim)
  +delay_mode_zero Force all structural (specify/SDF) delays to 0 (fast functional GLS)
  +delay_mode_unit Collapse every nonzero structural delay to 1 time unit
  +mindelays/+typdelays/+maxdelays  min:typ:max selection (specify + SDF; default typ)
  +notimingcheck   Accepted no-op (specify timing checks are not modeled)");
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
    eprintln!("  --fst <file>     Emit an FST (GTKWave binary) waveform dump to <file>.");
    eprintln!("  --fst-scope <hier>  Restrict the FST dump to signals under <hier>");
    eprintln!("                   (exact name or '<hier>.' prefix). Repeatable.");
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
}

fn print_version() {
    println!("xezim version {}", env!("CARGO_PKG_VERSION"));
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
        other => return Err(format!("invalid time mantissa '{}' (must be 1, 10, or 100)", other)),
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
        format!("invalid --module-timescale value '{}' (expected <unit>/<precision>)", d)
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
fn build_module_timescale_cli(raw: &[String]) -> Result<xezim::ModuleTimescaleCli, String> {
    let mut cli = xezim::ModuleTimescaleCli::default();
    for spec in raw {
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

fn push_plus_incdir(arg: &str, include_dirs: &mut Vec<String>, lib_dirs: &mut Vec<String>) {
    if !arg.starts_with("+incdir+") {
        return;
    }
    let payload = &arg[8..];
    for dir in payload.split('+').filter(|s| !s.is_empty()) {
        include_dirs.push(dir.to_string());
        lib_dirs.push(dir.to_string());
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
/// family (VCS / Questa / Xcelium) so it never falls silently into the generic
/// plusarg bucket. Returns true if `flag` was consumed here.
///
/// - `+delay_mode_zero` / `+delay_mode_unit` are MODELED (force structural
///   delays to 0, or every nonzero one to 1 tick).
/// - `+delay_mode_path` maps to xezim's default (specify path delays apply) —
///   recognized, no effect.
/// - Flags whose effect xezim cannot model (`+delay_mode_distributed`, pulse
///   control, transport/multisource interconnect delays) warn ONCE so the user
///   knows the timing is approximated — never silent.
/// - Timing-check controls (`+no_notifier`, `+neg_tchk`, …) are recognized
///   no-ops: xezim does not model specify timing checks, so there is nothing to
///   toggle (same rationale as `+notimingcheck`).
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
        // Timing-check control: nothing to disable (checks aren't modeled).
        "+no_notifier" | "+no_tchk_msg" | "+neg_tchk" | "+nonegdelay"
        | "+old_ntc" | "+ntc_warn" | "+nosdferror" | "+nocelldefinepragma"
        | "+sdf_verbose" | "+sdfverbose" => {}
        // Behavior xezim cannot model — warn once, don't pretend.
        "+delay_mode_distributed" | "-delay_mode_distributed" => {
            eprintln!(
                "Warning: {} requests distributed (gate/net) delays, which xezim does not model \
                 — structural timing is approximated (functional results are unaffected in the \
                 typical case).",
                flag
            );
        }
        "+pulse_e" | "+pulse_r" | "+pulse_int_e" | "+pulse_int_r"
        | "+transport_int_delays" | "+transport_path_delays"
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

fn preprocess_sources(
    sources: &[String],
    source_files: &[String],
    include_dirs: &[String],
    defines: &[(String, Option<String>)],
) -> Result<Vec<String>, String> {
    let mut pp = xezim::preprocessor::Preprocessor::new();
    for dir in include_dirs {
        pp.add_include_dir(std::path::PathBuf::from(dir));
    }
    for (name, val) in defines {
        pp.define(
            name.clone(),
            xezim::preprocessor::MacroDef {
                name: name.clone(),
                params: None,
                body: val.clone().unwrap_or_default(),
            },
        );
    }

    let mut preprocessed = Vec::with_capacity(sources.len());
    for (i, source) in sources.iter().enumerate() {
        let source_path = source_files.get(i).map(|p| std::path::PathBuf::from(p));
        preprocessed.push(pp.preprocess_file(source, source_path.as_deref()));
    }
    // §22 strict-mode directive errors (`\`line`/`\`pragma`/`\`resetall`/…).
    // Collected only when strict checks are on; a non-empty list fails the run.
    if !pp.errors().is_empty() {
        return Err(pp.errors().join("; "));
    }
    Ok(preprocessed)
}

/// Expand `$VAR` and `${VAR}` style references against the process
/// environment. Unknown variables expand to empty (matching the typical
/// VCS / Xcelium / Verilator behaviour on `-f` filelists). Used so that
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
    include_dirs: &mut Vec<String>,
    defines: &mut Vec<(String, Option<String>)>,
    lib_dirs: &mut Vec<String>,
    plusargs: &mut Vec<String>,
    lib_files: &mut Vec<String>,
    lib_exts: &mut Option<Vec<String>>,
    nospecify: &mut bool,
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
            match t {
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
                "+notimingcheck" | "+notimingchecks" | "-notimingchecks" => {}
                "-f" | "-c" => {
                    i += 1;
                    if i < toks.len() {
                        let nested = resolve_rel(base, &toks[i]);
                        process_command_file(
                            &nested,
                            source_files,
                            include_dirs,
                            defines,
                            lib_dirs,
                            plusargs,
                            lib_files,
                            lib_exts,
                            nospecify,
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
                        include_dirs,
                        defines,
                        lib_dirs,
                        plusargs,
                        lib_files,
                        lib_exts,
                        nospecify,
                    )?;
                }
                _ if t.starts_with("+incdir+") => {
                    push_plus_incdir(t, include_dirs, lib_dirs);
                }
                _ if t.starts_with("+define+") => {
                    push_plus_define(t, defines);
                }
                _ if handle_gls_flag(t) => {}
                _ if t.starts_with('+') => {
                    plusargs.push(t.to_string());
                }
                _ if t.starts_with('-') => {}
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

fn main() {
    spawn_memory_watchdog();

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
    let mut top_module: Option<String> = None;
    // All `-s <top>` modules, in order. UVM testbenches commonly declare two
    // unconnected roots (e.g. `hdl_top` + `hvl_top`); when more than one is
    // given we synthesize a wrapper module that instantiates them all and
    // elaborate that instead (a single root reaching every requested top).
    let mut top_modules: Vec<String> = Vec::new();
    let mut max_time: u64 = 100_000;
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
    let mut verbose = false;
    let mut _output_file: Option<String> = None;
    let mut lib_dirs: Vec<String> = Vec::new();
    let mut lib_files: Vec<String> = Vec::new();
    let mut lib_exts: Option<Vec<String>> = None;
    let mut nospecify = false;
    let mut log_file: Option<String> = None;
    let mut settle_limit: Option<u32> = None;
    let mut activity_mon = false;
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
    let mut fst_file: Option<String> = None;
    let mut fst_scopes: Vec<String> = Vec::new();
    let mut sim_debug = false;
    let mut dpi_libs: Vec<String> = Vec::new();
    let mut vpi_libs: Vec<String> = Vec::new();
    let mut module_timescale_args: Vec<String> = Vec::new();
    let mut plusargs: Vec<String> = Vec::new();
    let mut threads: usize = 1;
    let mut emit_hypergraph: Option<String> = None;
    let mut load_partition: Option<String> = None;
    let mut write_profile: Option<String> = None;
    let mut profile_input: Option<String> = None;
    let mut collapse_islands: bool = false;
    let mut pdes_c910_stub: Option<String> = None;
    let mut pdes_c910_ticks: u64 = 100;
    let mut multikernel_scope: Option<String> = None;

    let mut include_dirs: Vec<String> = Vec::new();
    let mut defines: Vec<(String, Option<String>)> = Vec::new();

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-h" | "--help" => {
                print_usage();
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
            "-o" => {
                i += 1;
                if i < args.len() {
                    _output_file = Some(args[i].clone());
                }
            }
            _ if arg.starts_with("-o") && arg.len() > 2 => {
                _output_file = Some(arg[2..].to_string());
            }
            "-l" | "--log" => {
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
                    top_module = Some(args[i].clone());
                    top_modules.push(args[i].clone());
                }
            }
            _ if arg.starts_with("-s") && arg.len() > 2 => {
                top_module = Some(arg[2..].to_string());
                top_modules.push(arg[2..].to_string());
            }
            "-c" | "-f" => {
                i += 1;
                if i < args.len() {
                    match process_command_file(
                        &args[i],
                        &mut source_files,
                        &mut include_dirs,
                        &mut defines,
                        &mut lib_dirs,
                        &mut plusargs,
                        &mut lib_files,
                        &mut lib_exts,
                        &mut nospecify,
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
                    &mut include_dirs,
                    &mut defines,
                    &mut lib_dirs,
                    &mut plusargs,
                    &mut lib_files,
                    &mut lib_exts,
                    &mut nospecify,
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
                push_plus_incdir(arg, &mut include_dirs, &mut lib_dirs);
            }
            _ if arg.starts_with("+define+") => {
                push_plus_define(arg, &mut defines);
            }
            _ if arg.starts_with("+libext+") => {
                push_plus_libext(arg, &mut lib_exts);
            }
            // Commercial GLS flags. `+nospecify` suppresses specify-block path
            // delays (zero-delay gate sim). `+notimingcheck(s)` is accepted as a
            // documented no-op: xezim does not model specify timing checks, so
            // they are permanently "disabled" already. Xcelium's `-` spellings
            // are accepted too.
            "+nospecify" | "-nospecify" => {
                nospecify = true;
            }
            // min:typ:max selection — governs specify-path triplets and, when
            // no --sdf-min/typ/max was given, the SDF annotation too.
            "+mindelays" | "-mindelays" => {
                xezim::sv_parser::set_delay_select(0);
                if sdf_select.is_none() {
                    sdf_select = Some(xezim::compiler::sdf::DelaySelect::Min);
                }
            }
            "+typdelays" | "-typdelays" => {
                xezim::sv_parser::set_delay_select(1);
                if sdf_select.is_none() {
                    sdf_select = Some(xezim::compiler::sdf::DelaySelect::Typ);
                }
            }
            "+maxdelays" | "-maxdelays" => {
                xezim::sv_parser::set_delay_select(2);
                if sdf_select.is_none() {
                    sdf_select = Some(xezim::compiler::sdf::DelaySelect::Max);
                }
            }
            "+notimingcheck" | "+notimingchecks" | "-notimingchecks" => {
                // no-op by design; recognized so flows don't carry a mystery plusarg
            }
            _ if handle_gls_flag(arg) => {}
            _ if arg.starts_with('+') => {
                plusargs.push(arg.clone());
            }
            // `-v <file>` — a library FILE (Verilog-XL/VCS semantics): its
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
            "--verbose" => {
                verbose = true;
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
            }
            "--no-strict" | "--lenient" => {
                sv_parser::set_strict_checks(false);
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
                    max_time = args[i].parse().unwrap_or(100_000);
                }
            }
            "--settle-limit" => {
                i += 1;
                if i < args.len() {
                    settle_limit = Some(args[i].parse().unwrap_or(100));
                }
            }
            "--activity-mon" => {
                activity_mon = true;
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
            "--sim_debug" => {
                sim_debug = true;
            }
            "--threads" => {
                i += 1;
                if i < args.len() {
                    threads = args[i].parse().unwrap_or(1).max(1);
                }
            }
            _ if arg.starts_with("--threads=") => {
                threads = arg["--threads=".len()..].parse().unwrap_or(1).max(1);
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
            "--pdes-c910-stub" => {
                i += 1;
                if i < args.len() {
                    pdes_c910_stub = Some(args[i].clone());
                }
            }
            _ if arg.starts_with("--pdes-c910-stub=") => {
                pdes_c910_stub = Some(arg["--pdes-c910-stub=".len()..].to_string());
            }
            "--pdes-c910-ticks" => {
                i += 1;
                if i < args.len() {
                    if let Ok(n) = args[i].parse::<u64>() {
                        pdes_c910_ticks = n;
                    }
                }
            }
            _ if arg.starts_with("--pdes-c910-ticks=") => {
                if let Ok(n) = arg["--pdes-c910-ticks=".len()..].parse::<u64>() {
                    pdes_c910_ticks = n;
                }
            }
            "--multikernel-scope" => {
                i += 1;
                if i < args.len() {
                    multikernel_scope = Some(args[i].clone());
                }
            }
            _ if arg.starts_with("--multikernel-scope=") => {
                multikernel_scope = Some(arg["--multikernel-scope=".len()..].to_string());
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
            _ if arg.starts_with('-') => {
                eprintln!("Warning: unknown flag '{}' (ignored)", arg);
            }
            _ => {
                source_files.push(arg.clone());
            }
        }
        i += 1;
    }

    if source_files.is_empty() {
        eprintln!("Error: no source files specified");
        print_usage();
        std::process::exit(1);
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
    if !lib_files.is_empty() || lib_exts.is_some() {
        xezim::set_library_cli(xezim::LibraryCli {
            lib_files: lib_files.clone(),
            lib_exts: lib_exts.clone(),
        });
    }

    if let Some(ref path) = log_file {
        if let Err(e) = redirect_stdio_to_log(path) {
            eprintln!("Error: cannot open log file '{}': {}", path, e);
            std::process::exit(1);
        }
    }

    // Fast path: if the only source file is a xezim compiled artifact, load
    // it and jump straight to simulation (skip parse + elaborate).
    if source_files.len() == 1 && mode == Mode::Simulate {
        let sf = &source_files[0];
        if let Ok(head) = std::fs::read(sf)
            .as_ref()
            .map(|v| v.iter().take(8).copied().collect::<Vec<u8>>())
        {
            if head.len() == 8 && &head[..] == xezim::XEZIM_BYTECODE_MAGIC {
                match xezim::read_compiled(sf) {
                    Ok(Some(elab)) => {
                        println!("=== xezim {} ===", env!("CARGO_PKG_VERSION"));
                        println!("Loaded compiled: {}", sf);
                        println!("Max time: {}", max_time);
                        println!("------------------------------");
                        let total_start = std::time::Instant::now();
                        xezim::compiler::simulator::set_sim_debug(sim_debug);
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
                        sim.set_threads(threads);
                        // Pass the full CLI invocation (binary name +
                        // all args + plusargs) so vpi_get_vlog_info
                        // can hand the same argv back to UVM.
                        sim.set_args(&args);
                        let compilation_start = std::time::Instant::now();
                        sim.compile();
                        eprintln!(
                            "[PHASE] compilation: {:.1}ms",
                            compilation_start.elapsed().as_secs_f64() * 1000.0
                        );
                        let simulation_start = std::time::Instant::now();
                        sim.simulate();
                        eprintln!(
                            "[PHASE] simulation: {:.1}ms",
                            simulation_start.elapsed().as_secs_f64() * 1000.0
                        );
                        eprintln!(
                            "[PHASE] total: {:.1}ms",
                            total_start.elapsed().as_secs_f64() * 1000.0
                        );
                        println!("------------------------------");
                        println!("Simulation finished at time {}", sim.time);
                        if sim.finished {
                            println!("($finish called)");
                        }
                        return;
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
                sources.push(String::from_utf8_lossy(&bytes).into_owned());
            }
            Err(e) => {
                eprintln!("Error: cannot read '{}': {}", sf, e);
                std::process::exit(1);
            }
        }
    }

    // Multi-top: synthesize a single wrapper root that instantiates every
    // requested `-s` top, so all of them elaborate (UVM hdl_top + hvl_top etc.).
    // Appended after the real sources so the instantiated modules are already
    // declared; the wrapper has no macros/includes, so preprocessing is a no-op.
    if top_modules.len() > 1 {
        let wrap_name = "__xz_multitop__";
        let mut body = format!("module {wrap_name};\n");
        for (i, t) in top_modules.iter().enumerate() {
            body.push_str(&format!("  {} __xz_top_inst_{}();\n", t, i));
        }
        body.push_str("endmodule\n");
        sources.push(body);
        source_files.push("<xz_multitop>".to_string());
        file_labels.push("<xz_multitop>".to_string());
        top_module = Some(wrap_name.to_string());
    }

    let preprocessed_sources =
        match preprocess_sources(&sources, &source_files, &include_dirs, &defines) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Error: preprocessing failed: {}", e);
                std::process::exit(1);
            }
        };

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
        return;
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
        for (label, source) in file_labels.iter().zip(preprocessed_sources.iter()) {
            let tokens = xezim::lexer::Lexer::new(source).tokenize();
            let mut parser = sv_parser::parse::Parser::new(tokens);
            let source_ast = parser.parse_source_text();
            let diags = parser.diagnostics().to_vec();
            for err in diags
                .iter()
                .filter(|d| d.severity == xezim::diagnostics::Severity::Error)
            {
                let (line, col) = byte_to_line_col(source, err.span.start);
                eprintln!("[{}] {}:{}: error: {}", label, line, col, err.message);
            }
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
        return;
    }

    if mode == Mode::Compile {
        let mut total_desc = 0;
        let mut total_err = 0;
        let mut total_warn = 0;

        for (label, source) in file_labels.iter().zip(preprocessed_sources.iter()) {
            let tokens = xezim::lexer::Lexer::new(source).tokenize();
            let mut parser = sv_parser::parse::Parser::new(tokens);
            let source_ast = parser.parse_source_text();
            let diags = parser.diagnostics().to_vec();
            for err in diags
                .iter()
                .filter(|d| d.severity == xezim::diagnostics::Severity::Error)
            {
                let (line, col) = byte_to_line_col(source, err.span.start);
                eprintln!("[{}] {}:{}: error: {}", label, line, col, err.message);
            }
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
                eprintln!("Simulation error: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }

    println!("=== xezim {} ===", env!("CARGO_PKG_VERSION"));
    println!("Max time: {}", max_time);
    println!("------------------------------");
    xezim::compiler::simulator::set_sim_debug(sim_debug);
    xezim::compiler::simulator::set_dpi_libs(&dpi_libs);
    xezim::compiler::simulator::set_vpi_libs(&vpi_libs);

    // PDES c910 stub mode: parse + elaborate + compile, then run the
    // PdesCoordinator with stub blocks for `pdes_c910_ticks` ticks.
    // Skips the regular event_loop. Front-half integration test for
    // the worktree perlp-experiment branch.
    if let Some(lp_a_prefix) = &pdes_c910_stub {
        match xezim::pdes_c910_stub_multi(
            &sources,
            top_module.as_deref(),
            &include_dirs,
            &source_files,
            &defines,
            lp_a_prefix,
            pdes_c910_ticks,
        ) {
            Ok(()) => {
                println!("------------------------------");
                println!("PDES c910 stub complete");
            }
            Err(e) => {
                eprintln!("PDES stub error: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }

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
        threads,
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
        multikernel_scope.as_deref(),
    ) {
        Ok(sim) => {
            println!("------------------------------");
            println!("Simulation finished at time {}", sim.time);
            if sim.finished {
                println!("($finish called)");
            }
        }
        Err(e) => {
            eprintln!("Simulation error: {}", e);
            std::process::exit(1);
        }
    }
}

fn byte_to_line_col(source: &str, byte_offset: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    for (i, ch) in source.char_indices() {
        if i >= byte_offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}
