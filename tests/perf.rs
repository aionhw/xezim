//! Integration-test group: perf.
//!
//! Guards against PERFORMANCE regressions using deterministic work counters
//! rather than wall-clock, which would flake. See the module below.

#[path = "perf/work_counters.rs"]
mod work_counters;

#[path = "perf/bench_host_workloads.rs"]
mod bench_host_workloads;
