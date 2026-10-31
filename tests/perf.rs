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

#[path = "perf/design_shape_regression.rs"]
mod design_shape_regression;
#[path = "perf/front_end_levers.rs"]
mod front_end_levers;
#[path = "perf/loop_block_counters.rs"]
mod loop_block_counters;
#[path = "perf/packed_record_edge_loop.rs"]
mod packed_record_edge_loop;
#[path = "perf/wide_block_counters.rs"]
mod wide_block_counters;
#[path = "perf/x_plane_executor.rs"]
mod x_plane_executor;

#[path = "perf/case_fusion.rs"]
mod case_fusion;

#[path = "perf/cmp_branch_const.rs"]
mod cmp_branch_const;

#[path = "perf/named_event_intern.rs"]
mod named_event_intern;

#[path = "perf/direct_scoped_calls.rs"]
mod direct_scoped_calls;

#[path = "perf/direct_ref_formal_calls.rs"]
mod direct_ref_formal_calls;

#[path = "perf/eqc_branch.rs"]
mod eqc_branch;
