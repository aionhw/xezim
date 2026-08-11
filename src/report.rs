//! Opt-in end-of-run statistics footer (`--report-stats` / XEZIM_REPORT_STATS).
//!
//! OFF by default and fully inert when off: main.rs resolves the mode once
//! after argument parsing and calls the collectors here only when a footer
//! was requested, so a normal run's output and cost are unchanged. The
//! footer goes to stderr, leaving the stdout lines scripts already grep
//! (`Simulation finished at time ...`) alone.
//!
//! Rendering is split from collection: each format is a pure function of a
//! `RunStats` value, so the exact text is unit-testable without running a
//! simulation (tests/perf/report_stats.rs includes this file directly —
//! the module lives in the CLI binary, not the library).

/// Footer selection: off (default), human-readable text, or one JSON line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReportMode {
    Off,
    Human,
    Json,
}

/// Map the `XEZIM_REPORT_STATS` value to a mode: `1` = human text, `json` =
/// JSON, anything else (or unset) stays off. Pure so the mapping is testable;
/// the CLI flag wins over the environment (main.rs consults this only when no
/// `--report-stats` flag was given).
pub(crate) fn mode_from_env_value(value: Option<&str>) -> ReportMode {
    match value {
        Some("json") => ReportMode::Json,
        Some("1") => ReportMode::Human,
        _ => ReportMode::Off,
    }
}

/// The values a footer reports. Only fields that are actually measured are
/// present; `None` means "could not be collected here" and the field is
/// omitted from both renderings rather than reported as a fabricated zero.
pub(crate) struct RunStats {
    /// CARGO_PKG_VERSION of the running binary.
    pub version: String,
    /// XEZIM_GIT_HASH of the running binary (same constant as the banner).
    pub git_rev: String,
    /// Wall-clock for the whole process, from the `Instant` main() takes
    /// before doing anything else.
    pub wall_ms: u64,
    /// getrusage(RUSAGE_SELF) user CPU time.
    pub cpu_user_ms: u64,
    /// getrusage(RUSAGE_SELF) system CPU time.
    pub cpu_sys_ms: u64,
    /// VmHWM from /proc/self/status; None off Linux or if unreadable.
    pub peak_rss_kb: Option<u64>,
    /// /proc/sys/kernel/hostname; None if unreadable.
    pub hostname: Option<String>,
    /// Final simulation time; None for runs that never simulate (--compile).
    pub sim_time_ns: Option<u64>,
}

/// Render the human-readable footer: a short generic block, one
/// `key : value` per line. Deliberately not modeled on any other tool's
/// summary block, and no line starts with "Simulation finished" (that
/// phrase is stdout's, and scripts grep for it).
pub(crate) fn render_human(stats: &RunStats) -> String {
    let mut out = String::from("--- xezim run statistics ---\n");
    out.push_str(&format!("version     : {}\n", stats.version));
    out.push_str(&format!("git_rev     : {}\n", stats.git_rev));
    out.push_str(&format!("wall_ms     : {}\n", stats.wall_ms));
    out.push_str(&format!("cpu_user_ms : {}\n", stats.cpu_user_ms));
    out.push_str(&format!("cpu_sys_ms  : {}\n", stats.cpu_sys_ms));
    if let Some(kb) = stats.peak_rss_kb {
        out.push_str(&format!("peak_rss_kb : {}\n", kb));
    }
    if let Some(ref host) = stats.hostname {
        out.push_str(&format!("hostname    : {}\n", host));
    }
    if let Some(t) = stats.sim_time_ns {
        out.push_str(&format!("sim_time_ns : {}\n", t));
    }
    out
}

/// Render the footer as a single flat JSON line (newline-terminated).
/// Emitted with plain `format!` on purpose — the schema is flat and tiny, so
/// a JSON dependency would buy nothing. `schema_version` lets CI consumers
/// notice if the set of keys ever changes.
pub(crate) fn render_json(stats: &RunStats) -> String {
    let mut out = String::from("{\"schema_version\":1");
    out.push_str(&format!(",\"version\":\"{}\"", escape_json(&stats.version)));
    out.push_str(&format!(",\"git_rev\":\"{}\"", escape_json(&stats.git_rev)));
    out.push_str(&format!(",\"wall_ms\":{}", stats.wall_ms));
    out.push_str(&format!(",\"cpu_user_ms\":{}", stats.cpu_user_ms));
    out.push_str(&format!(",\"cpu_sys_ms\":{}", stats.cpu_sys_ms));
    if let Some(kb) = stats.peak_rss_kb {
        out.push_str(&format!(",\"peak_rss_kb\":{}", kb));
    }
    if let Some(ref host) = stats.hostname {
        out.push_str(&format!(",\"hostname\":\"{}\"", escape_json(host)));
    }
    if let Some(t) = stats.sim_time_ns {
        out.push_str(&format!(",\"sim_time_ns\":{}", t));
    }
    out.push_str("}\n");
    out
}

/// Minimal JSON string escaping: backslash, quote, and control characters.
/// The only free-form string in the schema is the hostname (version/git_rev
/// are build constants), but every string field goes through here anyway.
pub(crate) fn escape_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// (user, system) CPU time of this process in milliseconds, via
/// getrusage(RUSAGE_SELF) — the same pattern as `print_resource_usage`.
#[cfg(unix)]
pub(crate) fn cpu_times_ms() -> (u64, u64) {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) } != 0 {
        return (0, 0);
    }
    let ms = |tv: libc::timeval| tv.tv_sec as u64 * 1000 + tv.tv_usec as u64 / 1000;
    (ms(ru.ru_utime), ms(ru.ru_stime))
}

#[cfg(not(unix))]
pub(crate) fn cpu_times_ms() -> (u64, u64) {
    (0, 0)
}

/// Peak resident set size in kB (VmHWM from /proc/self/status). None where
/// there is no procfs — the caller then omits the field.
pub(crate) fn peak_rss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmHWM:") {
            // Line format is "VmHWM:   123456 kB" — keep the numeric field.
            return rest.split_whitespace().next()?.parse().ok();
        }
    }
    None
}

/// Hostname from /proc/sys/kernel/hostname; None if unreadable so the
/// footer omits the field instead of inventing one.
pub(crate) fn hostname() -> Option<String> {
    let name = std::fs::read_to_string("/proc/sys/kernel/hostname").ok()?;
    let name = name.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}
