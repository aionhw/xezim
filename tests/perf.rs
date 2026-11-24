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
#[path = "perf/packed_member_path_store.rs"]
mod packed_member_path_store;
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
#[path = "perf/condition_waiter_gate.rs"]
mod condition_waiter_gate;
#[path = "perf/delay_loop_compiled.rs"]
mod delay_loop_compiled;
#[path = "perf/impure_call_delegated.rs"]
mod impure_call_delegated;
#[path = "perf/review_impure_call_single_assign.rs"]
mod review_impure_call_single_assign;

#[path = "perf/eqc_branch.rs"]
mod eqc_branch;

#[path = "perf/struct_resolver_compiled.rs"]
mod struct_resolver_compiled;

#[path = "perf/displaced_local_meta_reapply.rs"]
mod displaced_local_meta_reapply;

#[path = "perf/same_name_live_frames_943.rs"]
mod same_name_live_frames_943;

#[path = "perf/const_index_array_arming.rs"]
mod const_index_array_arming;

#[path = "perf/class_aggregate_resolution.rs"]
mod class_aggregate_resolution;

#[path = "perf/parked_waiter_index.rs"]
mod parked_waiter_index;
#[path = "perf/queue_shift_counters.rs"]
mod queue_shift_counters;
#[path = "perf/select_store_counters.rs"]
mod select_store_counters;
#[path = "perf/timing_check_conditions.rs"]
mod timing_check_conditions;
#[path = "perf/struct_formal_member_calls.rs"]
mod struct_formal_member_calls;
#[path = "perf/static_local_select_cover.rs"]
mod static_local_select_cover;
#[path = "perf/local_select_width_chain.rs"]
mod local_select_width_chain;

#[path = "perf/fallback_hotspots.rs"]
mod fallback_hotspots;
#[path = "perf/process_memory_paths.rs"]
mod process_memory_paths;
#[path = "perf/packed_member_path_read.rs"]
mod packed_member_path_read;
#[path = "perf/decl_param_scope_scaling.rs"]
mod decl_param_scope_scaling;
#[path = "perf/scoped_package_calls.rs"]
mod scoped_package_calls;
