//! Integration-test group: perf.
//!
//! Guards against PERFORMANCE regressions using deterministic work counters
//! rather than wall-clock, which would flake. See the module below.

#[path = "perf/work_counters.rs"]
mod work_counters;

#[path = "perf/bench_host_workloads.rs"]
mod bench_host_workloads;

#[path = "perf/report_stats.rs"]
mod report_stats;

#[path = "perf/packed_matrix_workload.rs"]
mod packed_matrix_workload;

#[path = "perf/packed_record_edge_loop.rs"]
mod packed_record_edge_loop;
#[path = "perf/design_shape_regression.rs"]
mod design_shape_regression;

#[path = "perf/threads_clamp.rs"]
mod threads_clamp;
