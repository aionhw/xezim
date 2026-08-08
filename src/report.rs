//! Statistics report module
//!
//! Provides data structures and emitters for vendor-style simulation
//! statistics footers (modeled after commercial tool summary blocks).

use serde::Serialize;
use std::time::SystemTime;

/// Report mode for statistics output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReportMode {
    Off,
    Human,
    Json,
    File(std::path::PathBuf),
}

impl Default for ReportMode {
    fn default() -> Self {
        ReportMode::Off
    }
}

/// Timing phases in milliseconds.
///
/// `parse_ms`/`elaborate_ms` are deliberately absent: parse and elaborate run
/// together in one call (`parse_and_elaborate_multi`), so they cannot be
/// measured separately today. Reporting fabricated zeros would read as
/// "parsing was instantaneous"; the measured phases are compile, sim, total.
#[derive(Debug, Clone, Serialize)]
pub struct Phases {
    pub compile_ms: f64,
    pub sim_ms: f64,
    pub total_ms: f64,
}

/// CPU time breakdown in seconds.
#[derive(Debug, Clone, Serialize)]
pub struct Cpu {
    pub user_s: f64,
    pub sys_s: f64,
    pub total_s: f64,
}

/// Memory usage in kilobytes.
#[derive(Debug, Clone, Serialize)]
pub struct Mem {
    pub peak_rss_kb: u64,
    pub cur_rss_kb: u64,
}

/// Workload counters.
#[derive(Debug, Clone, Serialize)]
pub struct Workload {
    pub sim_time_ns: u64,
    pub insns: u64,
    pub delta_cycles: u64,
    pub nba_events: u64,
    pub edge_fires: u64,
    pub signal_count: usize,
}

/// Best-effort memory attribution in megabytes.
#[derive(Debug, Clone, Serialize)]
pub struct MemAttribution {
    pub signal_table_mb: f64,
    pub class_heap_mb: f64,
    pub bytecode_mb: f64,
}

/// Complete simulation report.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub version: String,
    pub host: String,
    pub cores: usize,
    pub threads_requested: usize,
    pub threads_used: usize,
    pub phases: Phases,
    pub cpu: Cpu,
    pub mem: Mem,
    pub workload: Workload,
    pub memory_attribution: Option<MemAttribution>,
}

impl Report {
    /// Build a Report from collected data.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        version: String,
        host: String,
        cores: usize,
        threads_requested: usize,
        threads_used: usize,
        phases: Phases,
        cpu: Cpu,
        mem: Mem,
        workload: Workload,
        memory_attribution: Option<MemAttribution>,
    ) -> Self {
        Self {
            version,
            host,
            cores,
            threads_requested,
            threads_used,
            phases,
            cpu,
            mem,
            workload,
            memory_attribution,
        }
    }
}

/// Read a u64 from /proc/<pid|self>/status or /proc/meminfo by key (kB units).
fn proc_kb(path: &str, key: &str) -> Option<u64> {
    let s = std::fs::read_to_string(path).ok()?;
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix(key) {
            return rest
                .trim_start_matches(|c: char| c == ':' || c.is_whitespace())
                .split_whitespace()
                .next()
                .and_then(|n| n.parse::<u64>().ok());
        }
    }
    None
}

/// Collect current RSS and peak RSS (VmHWM) in kB.
pub fn collect_memory() -> (u64, u64) {
    let mut peak = 0u64;
    let mut cur = 0u64;
    if let Ok(st) = std::fs::read_to_string("/proc/self/status") {
        for line in st.lines() {
            if let Some(v) = line.strip_prefix("VmHWM:") {
                // Format is "<n> kB" — keep only the numeric part.
                peak = v
                    .split_whitespace()
                    .next()
                    .and_then(|n| n.parse::<u64>().ok())
                    .unwrap_or(0);
            } else if let Some(v) = line.strip_prefix("VmRSS:") {
                cur = v
                    .split_whitespace()
                    .next()
                    .and_then(|n| n.parse::<u64>().ok())
                    .unwrap_or(0);
            }
        }
    }
    (cur, peak)
}

/// Collect CPU time via getrusage (returns (user_s, sys_s)).
#[cfg(unix)]
pub fn collect_cpu() -> (f64, f64) {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) } == 0 {
        let user = ru.ru_utime.tv_sec as f64 + ru.ru_utime.tv_usec as f64 / 1e6;
        let sys = ru.ru_stime.tv_sec as f64 + ru.ru_stime.tv_usec as f64 / 1e6;
        return (user, sys);
    }
    (0.0, 0.0)
}

#[cfg(not(unix))]
pub fn collect_cpu() -> (f64, f64) {
    (0.0, 0.0)
}

/// Major page faults counted by the kernel (getrusage `ru_majflt`).
#[cfg(unix)]
pub fn major_page_faults() -> u64 {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) } == 0 {
        ru.ru_majflt.max(0) as u64
    } else {
        0
    }
}

#[cfg(not(unix))]
pub fn major_page_faults() -> u64 {
    0
}

/// Get hostname.
pub fn get_hostname() -> String {
    if let Ok(name) = std::env::var("HOSTNAME") {
        if !name.is_empty() {
            return name;
        }
    }
    if let Ok(name) = std::fs::read_to_string("/proc/sys/kernel/hostname") {
        let name = name.trim().to_string();
        if !name.is_empty() {
            return name;
        }
    }
    "unknown".to_string()
}

/// Convert days since 1970-01-01 to a (year, month, day) civil date
/// (Howard Hinnant's `civil_from_days` algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Format the current time as an RFC2822 date string in UTC, computed
/// directly from the system clock (no external date-time crate).
pub fn format_rfc2822_utc() -> String {
    let now = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let days = now.div_euclid(86400);
    let secs_of_day = now.rem_euclid(86400);
    let (year, month, day) = civil_from_days(days);
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day / 60) % 60;
    let second = secs_of_day % 60;
    // 1970-01-01 was a Thursday. weekday index 0 = Sunday.
    let weekday = ((days.rem_euclid(7)) as usize + 4) % 7;
    const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} UTC",
        WEEKDAYS[weekday],
        day,
        MONTHS[(month - 1) as usize],
        year,
        hour,
        minute,
        second
    )
}

/// The dashed separator used between header and footer sections.
const SEPARATOR: &str = "--------------------------------------------------------";

/// Render the compilation performance summary block.
fn render_compilation_block(report: &Report, out: &mut String) {
    out.push_str("Compilation Performance Summary\n");
    out.push_str(SEPARATOR);
    out.push('\n');
    out.push_str(&format!("xezim started at        :  {}\n", format_rfc2822_utc()));
    out.push_str(&format!(
        "Elapsed time            :  {:.2} sec\n",
        report.phases.total_ms / 1000.0
    ));
    out.push_str(&format!("CPU Time                :  {:.2} sec\n", report.cpu.total_s));
    let vm_size_kb = proc_kb("/proc/self/status", "VmSize:").unwrap_or(0);
    let vm_size_gb = vm_size_kb as f64 / (1024.0 * 1024.0);
    out.push_str(&format!("Virtual memory size     :  {:.2} GB\n", vm_size_gb));
    out.push_str(&format!(
        "Resident set size       :  {:.2} GB\n",
        report.mem.cur_rss_kb as f64 / (1024.0 * 1024.0)
    ));
    let shared_kb = proc_kb("/proc/self/status", "VmShm:").unwrap_or(0);
    let shared_mb = shared_kb as f64 / 1024.0;
    out.push_str(&format!("Shared memory size      :  {:.0} MB\n", shared_mb));
    let private_mb = report.mem.cur_rss_kb.saturating_sub(shared_kb) as f64 / 1024.0;
    out.push_str(&format!("Private memory size     :  {:.0} MB\n", private_mb));
    out.push_str(&format!("Major page faults       :  {}\n", major_page_faults()));
    out.push_str(&format!("Machine name            :  {}\n", report.host));
    out.push_str(SEPARATOR);
    out.push_str("\n\n");
}

/// Render report in human-readable format (vendor-tool style).
pub fn render_human(report: &Report) -> String {
    let mut out = String::new();
    render_compilation_block(report, &mut out);
    if report.workload.sim_time_ns > 0 || report.phases.sim_ms > 0.0 {
        // Simulation block.
        out.push_str("Simulation Performance Summary\n");
        out.push_str(SEPARATOR);
        out.push('\n');
        out.push_str(&format!(
            "Simulation started at   :  {}\n",
            format_rfc2822_utc()
        ));
        out.push_str(&format!(
            "Elapsed Time            :  {:.2} sec\n",
            report.phases.sim_ms / 1000.0
        ));
        out.push_str(&format!("CPU Time                :  {:.2} sec\n", report.cpu.total_s));
        let vm_size_kb = proc_kb("/proc/self/status", "VmSize:").unwrap_or(0);
        let vm_size_gb = vm_size_kb as f64 / (1024.0 * 1024.0);
        out.push_str(&format!("Virtual memory size     :  {:.2} GB\n", vm_size_gb));
        out.push_str(&format!(
            "Resident set size       :  {:.2} GB\n",
            report.mem.cur_rss_kb as f64 / (1024.0 * 1024.0)
        ));
        let shared_kb = proc_kb("/proc/self/status", "VmShm:").unwrap_or(0);
        let shared_mb = shared_kb as f64 / 1024.0;
        out.push_str(&format!("Shared memory size      :  {:.0} MB\n", shared_mb));
        let private_mb = report.mem.cur_rss_kb.saturating_sub(shared_kb) as f64 / 1024.0;
        out.push_str(&format!("Private memory size     :  {:.0} MB\n", private_mb));
        out.push_str(&format!("Major page faults       :  {}\n", major_page_faults()));
        out.push_str(&format!("Machine name            :  {}\n", report.host));
        out.push_str(SEPARATOR);
        out.push_str("\n\n");
        // Simulation finished line.
        let sim_time_ns = report.workload.sim_time_ns;
        let sim_time_ms = sim_time_ns as f64 / 1_000_000.0;
        out.push_str(&format!(
            "Simulation finished at  {} ns ({:.2} ms)\n",
            sim_time_ns, sim_time_ms
        ));
    }
    out
}

/// Render report as a single JSON document (for CI diffing).
pub fn render_json(report: &Report) -> String {
    serde_json::to_string_pretty(report).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_report() -> Report {
        Report::new(
            "0.9.7".to_string(),
            "test-host".to_string(),
            8,
            4,
            4,
            Phases {
                compile_ms: 3.0,
                sim_ms: 4.0,
                total_ms: 10.0,
            },
            Cpu {
                user_s: 0.5,
                sys_s: 0.3,
                total_s: 0.8,
            },
            Mem {
                peak_rss_kb: 102400,
                cur_rss_kb: 51200,
            },
            Workload {
                sim_time_ns: 1000,
                insns: 100,
                delta_cycles: 5,
                nba_events: 10,
                edge_fires: 2,
                signal_count: 50,
            },
            None,
        )
    }

    #[test]
    fn json_emits_version() {
        let json = render_json(&sample_report());
        assert!(json.contains("\"version\""));
        assert!(json.contains("0.9.7"));
    }

    #[test]
    fn human_has_separators() {
        let human = render_human(&sample_report());
        assert!(human.contains(SEPARATOR));
        assert!(human.contains("Compilation Performance Summary"));
    }

    #[test]
    fn civil_date_is_correct() {
        // 1970-01-01 -> (1970, 1, 1)
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2000-03-01 (a leap-year day)
        assert_eq!(civil_from_days(11017), (2000, 3, 1));
        // 2024-02-29
        assert_eq!(civil_from_days(19782), (2024, 2, 29));
        assert_eq!(civil_from_days(19783), (2024, 3, 1));
    }
}
