//! Integration-test group: perf.
//!
//! Guards against PERFORMANCE regressions using deterministic work counters
//! rather than wall-clock, which would flake. See the module below.

#[path = "perf/work_counters.rs"]
mod work_counters;

// CLI `--threads` clamp behaviour (deterministic stderr assertions).
#[path = "perf/threads_clamp.rs"]
mod threads_clamp;
