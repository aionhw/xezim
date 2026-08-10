//! Integration-test group: scheduling.
//!
//! Every `tests/*.rs` used to build its own ~66 MB binary that statically
//! links the whole simulator; 374 of them cost 24 GB and dominated
//! `cargo test` wall-clock (the tests themselves run in milliseconds).
//! The cases now live one directory down and are included here as
//! modules, so this group links ONCE. Tests, names and assertions are
//! unchanged — only the link unit is.
//!
//! The explicit module paths below are required: a crate root resolves a
//! plain `mod x;` beside itself, not into `tests/<group>/`. To add a test,
//! drop the file in this group's directory and add one entry here.

#[path = "scheduling/wait_fork_immediate_children.rs"]
mod wait_fork_immediate_children;
#[path = "scheduling/comb_collection_element_sensitivity.rs"]
mod comb_collection_element_sensitivity;
#[path = "scheduling/computed_edge_expressions.rs"]
mod computed_edge_expressions;
#[path = "scheduling/active_region_fifo.rs"]
mod active_region_fifo;
#[path = "scheduling/adaptive_edge_skip.rs"]
mod adaptive_edge_skip;
#[path = "scheduling/always_comb_sensitivity_audit.rs"]
mod always_comb_sensitivity_audit;
#[path = "scheduling/always_iff_guard.rs"]
mod always_iff_guard;
#[path = "scheduling/always_level_delay.rs"]
mod always_level_delay;
#[path = "scheduling/audit_ports_disable_drivers.rs"]
mod audit_ports_disable_drivers;
#[path = "scheduling/bare_clocking_event.rs"]
mod bare_clocking_event;
#[path = "scheduling/blocking_loop_break_continue.rs"]
mod blocking_loop_break_continue;
#[path = "scheduling/struct_member_read_sensitivity.rs"]
mod struct_member_read_sensitivity;
#[path = "scheduling/c910_biu_bresp_capture_race.rs"]
mod c910_biu_bresp_capture_race;
#[path = "scheduling/c910_settle_miri.rs"]
mod c910_settle_miri;
#[path = "scheduling/case_wait_in_task.rs"]
mod case_wait_in_task;
#[path = "scheduling/clock_gate_fanout.rs"]
mod clock_gate_fanout;
#[path = "scheduling/clock_t0_variable_delay_phase.rs"]
mod clock_t0_variable_delay_phase;
#[path = "scheduling/clocking_event_cycle_delay.rs"]
mod clocking_event_cycle_delay;
#[path = "scheduling/comb_result_clobbered_by_process.rs"]
mod comb_result_clobbered_by_process;
#[path = "scheduling/dead_clock_watchdog.rs"]
mod dead_clock_watchdog;
#[path = "scheduling/delay_spike_warning.rs"]
mod delay_spike_warning;
#[path = "scheduling/determinism_and_stall.rs"]
mod determinism_and_stall;
#[path = "scheduling/disable_loop_label.rs"]
mod disable_loop_label;
#[path = "scheduling/edge_delivery.rs"]
mod edge_delivery;
#[path = "scheduling/edge_event_lsb.rs"]
mod edge_event_lsb;
#[path = "scheduling/event_control_iff_guard.rs"]
mod event_control_iff_guard;
#[path = "scheduling/event_features_15_5.rs"]
mod event_features_15_5;
#[path = "scheduling/event_triggered_in_event_control.rs"]
mod event_triggered_in_event_control;
#[path = "scheduling/event_wait_same_time.rs"]
mod event_wait_same_time;
#[path = "scheduling/explicit_sensitivity_and_delay_task.rs"]
mod explicit_sensitivity_and_delay_task;
#[path = "scheduling/forever_break_and_disable_fork_label.rs"]
mod forever_break_and_disable_fork_label;
#[path = "scheduling/fork_automatic_capture.rs"]
mod fork_automatic_capture;
#[path = "scheduling/fork_join_edge.rs"]
mod fork_join_edge;
#[path = "scheduling/fork_var_sharing.rs"]
mod fork_var_sharing;
#[path = "scheduling/gap_fixes_scoping_and_nba.rs"]
mod gap_fixes_scoping_and_nba;
#[path = "scheduling/gate_rise_fall_delay.rs"]
mod gate_rise_fall_delay;
#[path = "scheduling/intra_assignment_delay.rs"]
mod intra_assignment_delay;
#[path = "scheduling/ivtest_always_cluster.rs"]
mod ivtest_always_cluster;
#[path = "scheduling/local_arrays_and_edge_always.rs"]
mod local_arrays_and_edge_always;
#[path = "scheduling/loop_control_and_oob_index.rs"]
mod loop_control_and_oob_index;
#[path = "scheduling/lrm_disable.rs"]
mod lrm_disable;
#[path = "scheduling/lrm_name_collapse_and_delay.rs"]
mod lrm_name_collapse_and_delay;
#[path = "scheduling/mailbox_blocking_processes.rs"]
mod mailbox_blocking_processes;
#[path = "scheduling/mailbox_fork_phase_hop.rs"]
mod mailbox_fork_phase_hop;
#[path = "scheduling/named_event_identity.rs"]
mod named_event_identity;
#[path = "scheduling/nba_after_zero_delay.rs"]
mod nba_after_zero_delay;
#[path = "scheduling/nba_fast_vs_queue_order.rs"]
mod nba_fast_vs_queue_order;
#[path = "scheduling/nba_index_freeze_and_elem_selects.rs"]
mod nba_index_freeze_and_elem_selects;
#[path = "scheduling/nba_intra_event_control.rs"]
mod nba_intra_event_control;
#[path = "scheduling/nba_leak_waiter_active_region.rs"]
mod nba_leak_waiter_active_region;
#[path = "scheduling/nba_region_not_flushed_mid_edge.rs"]
mod nba_region_not_flushed_mid_edge;
#[path = "scheduling/null_mailbox_and_stall_location.rs"]
mod null_mailbox_and_stall_location;
#[path = "scheduling/oracle_timing_semantics.rs"]
mod oracle_timing_semantics;
#[path = "scheduling/preprocessor_diagnostic_location.rs"]
mod preprocessor_diagnostic_location;
#[path = "scheduling/preprocessor_github_issues.rs"]
mod preprocessor_github_issues;
#[path = "scheduling/process_block_local_shadowing.rs"]
mod process_block_local_shadowing;
#[path = "scheduling/procedural_loop_stall.rs"]
mod procedural_loop_stall;
#[path = "scheduling/pure_inline_loop_body.rs"]
mod pure_inline_loop_body;
#[path = "scheduling/ranged_port_connections_and_nba_freeze.rs"]
mod ranged_port_connections_and_nba_freeze;
#[path = "scheduling/parallel_dispatch_expr_fallback.rs"]
mod parallel_dispatch_expr_fallback;
#[path = "scheduling/sampled_value_inferred_clock.rs"]
mod sampled_value_inferred_clock;
#[path = "scheduling/sequential_event_waits.rs"]
mod sequential_event_waits;
#[path = "scheduling/specify_delays_and_event_list.rs"]
mod specify_delays_and_event_list;
#[path = "scheduling/star_sensitivity_and_implicit_port_net.rs"]
mod star_sensitivity_and_implicit_port_net;
#[path = "scheduling/suspending_loop_depth_and_continue.rs"]
mod suspending_loop_depth_and_continue;
#[path = "scheduling/task_body_delay_scaling.rs"]
mod task_body_delay_scaling;
#[path = "scheduling/timing_check_delayed_nets_singlelimit.rs"]
mod timing_check_delayed_nets_singlelimit;
#[path = "scheduling/unbased_unsized_fill.rs"]
mod unbased_unsized_fill;
#[path = "scheduling/vardelay_clock_period.rs"]
mod vardelay_clock_period;
#[path = "scheduling/wait_in_foreach_blocks.rs"]
mod wait_in_foreach_blocks;
#[path = "scheduling/waiter_edge_ordering.rs"]
mod waiter_edge_ordering;
#[path = "scheduling/xtrace_conformance.rs"]
mod xtrace_conformance;

#[path = "scheduling/compiled_for_loops.rs"]
mod compiled_for_loops;
