//! Integration-test group: perf.
//!
//! Guards against PERFORMANCE regressions using deterministic work counters
//! rather than wall-clock, which would flake. See the module below.
//!
//! Also hosts the statistics-footer tests, which exercise the CLI binary's
//! `--report-stats` output (deterministic text/JSON assertions, no wall-clock
//! timing), so they share the link unit instead of adding new top-level
//! binaries.

#[path = "perf/work_counters.rs"]
mod work_counters;

#[path = "perf/report_flag.rs"]
mod report_flag;

#[path = "perf/report_data.rs"]
mod report_data;

#[path = "perf/report_default_footer.rs"]
mod report_default_footer;
