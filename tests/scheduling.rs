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

#[path = "scheduling/concurrent_struct_formal_geometry.rs"]
mod concurrent_struct_formal_geometry;

#[path = "scheduling/named_branch_frames.rs"]
mod named_branch_frames;

#[path = "scheduling/loop_frame_bail_rollback.rs"]
mod loop_frame_bail_rollback;
#[path = "scheduling/nba_region_waiter_order.rs"]
mod nba_region_waiter_order;
#[path = "scheduling/property_wait_shapes.rs"]
mod property_wait_shapes;
#[path = "scheduling/replicated_clock_edges.rs"]
mod replicated_clock_edges;
#[path = "scheduling/replicated_edges_forced.rs"]
mod replicated_edges_forced;

#[path = "scheduling/active_region_fifo.rs"]
mod active_region_fifo;
#[path = "scheduling/adaptive_edge_skip.rs"]
mod adaptive_edge_skip;
#[path = "scheduling/always_comb_sensitivity_audit.rs"]
mod always_comb_sensitivity_audit;
#[path = "scheduling/always_iff_guard.rs"]
mod always_iff_guard;
#[path = "scheduling/t0_comb_defer_start_batch_550.rs"]
mod t0_comb_defer_start_batch;
#[path = "scheduling/always_level_delay.rs"]
mod always_level_delay;
#[path = "scheduling/armed_array_element_inputs.rs"]
mod armed_array_element_inputs;
#[path = "scheduling/assign_wake_order.rs"]
mod assign_wake_order;
#[path = "scheduling/assoc_bracket_keys.rs"]
mod assoc_bracket_keys;
#[path = "scheduling/audit_ports_disable_drivers.rs"]
mod audit_ports_disable_drivers;
#[path = "scheduling/bare_clocking_event.rs"]
mod bare_clocking_event;
#[path = "scheduling/bit_sensitivity.rs"]
mod bit_sensitivity;
#[path = "scheduling/block_local_decl_ast_fallback.rs"]
mod block_local_decl_ast_fallback;
#[path = "scheduling/blocking_loop_break_continue.rs"]
mod blocking_loop_break_continue;
#[path = "scheduling/c910_biu_bresp_capture_race.rs"]
mod c910_biu_bresp_capture_race;
#[path = "scheduling/c910_settle_miri.rs"]
mod c910_settle_miri;
#[path = "scheduling/case_wait_in_task.rs"]
mod case_wait_in_task;
#[path = "scheduling/class_method_delay_timeunit.rs"]
mod class_method_delay_timeunit;
#[path = "scheduling/clock_gate_fanout.rs"]
mod clock_gate_fanout;
#[path = "scheduling/clock_t0_variable_delay_phase.rs"]
mod clock_t0_variable_delay_phase;
#[path = "scheduling/clockgen_x_clock_stays_x.rs"]
mod clockgen_x_clock_stays_x;
#[path = "scheduling/clocking_event_cycle_delay.rs"]
mod clocking_event_cycle_delay;
#[path = "scheduling/clocking_iff_samples_this_edge.rs"]
mod clocking_iff_samples_this_edge;
#[path = "scheduling/clocking_task_context.rs"]
mod clocking_task_context;
#[path = "scheduling/comb_collection_element_sensitivity.rs"]
mod comb_collection_element_sensitivity;
#[path = "scheduling/comb_member_write_sensitivity.rs"]
mod comb_member_write_sensitivity;
#[path = "scheduling/comb_result_clobbered_by_process.rs"]
mod comb_result_clobbered_by_process;
#[path = "scheduling/computed_edge_expressions.rs"]
mod computed_edge_expressions;
#[path = "scheduling/condition_waiter_name_gate.rs"]
mod condition_waiter_name_gate;
#[path = "scheduling/dead_clock_watchdog.rs"]
mod dead_clock_watchdog;
#[path = "scheduling/decl_init_time_literal_scaling.rs"]
mod decl_init_time_literal_scaling;
#[path = "scheduling/assertion_control.rs"]
mod assertion_control;
#[path = "scheduling/deferred_immediate_assertions.rs"]
mod deferred_immediate_assertions;
#[path = "scheduling/delay_lands_on_clock_edge.rs"]
mod delay_lands_on_clock_edge;
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
#[path = "scheduling/element_sensitivity_after_large_array.rs"]
mod element_sensitivity_after_large_array;
#[path = "scheduling/event_control_iff_guard.rs"]
mod event_control_iff_guard;
#[path = "scheduling/event_features_15_5.rs"]
mod event_features_15_5;
#[path = "scheduling/event_triggered_in_event_control.rs"]
mod event_triggered_in_event_control;
#[path = "scheduling/event_wait_same_time.rs"]
mod event_wait_same_time;
#[path = "scheduling/expect_statement_blocks.rs"]
mod expect_statement_blocks;
#[path = "scheduling/explicit_level_suspend.rs"]
mod explicit_level_suspend;
#[path = "scheduling/explicit_sensitivity_and_delay_task.rs"]
mod explicit_sensitivity_and_delay_task;
#[path = "scheduling/for_decl_signed_across_wait.rs"]
mod for_decl_signed_across_wait;
#[path = "scheduling/forever_break_and_disable_fork_label.rs"]
mod forever_break_and_disable_fork_label;
#[path = "scheduling/fork_automatic_capture.rs"]
mod fork_automatic_capture;
#[path = "scheduling/fork_children_start_in_spawn_slot.rs"]
mod fork_children_start_in_spawn_slot;
#[path = "scheduling/fork_join_edge.rs"]
mod fork_join_edge;
#[path = "scheduling/fork_join_none_await_context.rs"]
mod fork_join_none_await_context;
#[path = "scheduling/fork_var_activation_isolation.rs"]
mod fork_var_activation_isolation;
#[path = "scheduling/fork_var_sharing.rs"]
mod fork_var_sharing;
#[path = "scheduling/forked_package_task_suspends.rs"]
mod forked_package_task_suspends;
#[path = "scheduling/gap_fixes_scoping_and_nba.rs"]
mod gap_fixes_scoping_and_nba;
#[path = "scheduling/gate_rise_fall_delay.rs"]
mod gate_rise_fall_delay;
#[path = "scheduling/iff_guard_module_scope.rs"]
mod iff_guard_module_scope;
#[path = "scheduling/intra_assignment_delay.rs"]
mod intra_assignment_delay;
#[path = "scheduling/intra_assignment_event_identifier.rs"]
mod intra_assignment_event_identifier;
#[path = "scheduling/issue_237_mixed_clock_configs.rs"]
mod issue_237_mixed_clock_configs;
#[path = "scheduling/ivtest_always_cluster.rs"]
mod ivtest_always_cluster;
#[path = "scheduling/large_array_element_events.rs"]
mod large_array_element_events;
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
#[path = "scheduling/multidim_assoc_struct_copy.rs"]
mod multidim_assoc_struct_copy;
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
#[path = "scheduling/nba_region_join_none_order.rs"]
mod nba_region_join_none_order;
#[path = "scheduling/nba_region_not_flushed_mid_edge.rs"]
mod nba_region_not_flushed_mid_edge;
#[path = "scheduling/null_mailbox_and_stall_location.rs"]
mod null_mailbox_and_stall_location;
#[path = "scheduling/oracle_timing_semantics.rs"]
mod oracle_timing_semantics;
#[path = "scheduling/parallel_dispatch_expr_fallback.rs"]
mod parallel_dispatch_expr_fallback;
#[path = "scheduling/preprocessor_diagnostic_location.rs"]
mod preprocessor_diagnostic_location;
#[path = "scheduling/preprocessor_github_issues.rs"]
mod preprocessor_github_issues;
#[path = "scheduling/procedural_loop_stall.rs"]
mod procedural_loop_stall;
#[path = "scheduling/process_block_local_shadowing.rs"]
mod process_block_local_shadowing;
#[path = "scheduling/pure_inline_loop_body.rs"]
mod pure_inline_loop_body;
#[path = "scheduling/randcase_blocking_branch.rs"]
mod randcase_blocking_branch;
#[path = "scheduling/ranged_port_connections_and_nba_freeze.rs"]
mod ranged_port_connections_and_nba_freeze;
#[path = "scheduling/sampled_value_inferred_clock.rs"]
mod sampled_value_inferred_clock;
#[path = "scheduling/sampled_value_per_instance.rs"]
mod sampled_value_per_instance;
#[path = "scheduling/select_event_terms.rs"]
mod select_event_terms;
#[path = "scheduling/sequential_event_waits.rs"]
mod sequential_event_waits;
#[path = "scheduling/shared_edge_outputs.rs"]
mod shared_edge_outputs;
#[path = "scheduling/specify_delays_and_event_list.rs"]
mod specify_delays_and_event_list;
#[path = "scheduling/star_sensitivity_and_implicit_port_net.rs"]
mod star_sensitivity_and_implicit_port_net;
#[path = "scheduling/static_local_nba_per_instance.rs"]
mod static_local_nba_per_instance;
#[path = "scheduling/struct_member_read_sensitivity.rs"]
mod struct_member_read_sensitivity;
#[path = "scheduling/suspending_loop_depth_and_continue.rs"]
mod suspending_loop_depth_and_continue;
#[path = "scheduling/sva_time0_default_samples.rs"]
mod sva_time0_default_samples;
#[path = "scheduling/task_body_delay_scaling.rs"]
mod task_body_delay_scaling;
#[path = "scheduling/time0_process_order.rs"]
mod time0_process_order;
#[path = "scheduling/timing_check_delayed_nets_singlelimit.rs"]
mod timing_check_delayed_nets_singlelimit;
#[path = "scheduling/typeparam_pool_wait.rs"]
mod typeparam_pool_wait;
#[path = "scheduling/unbased_unsized_fill.rs"]
mod unbased_unsized_fill;
#[path = "scheduling/vardelay_clock_period.rs"]
mod vardelay_clock_period;
#[path = "scheduling/wait_fork_immediate_children.rs"]
mod wait_fork_immediate_children;
#[path = "scheduling/wait_in_foreach_blocks.rs"]
mod wait_in_foreach_blocks;
#[path = "scheduling/wait_named_event_retrigger.rs"]
mod wait_named_event_retrigger;
#[path = "scheduling/wait_satisfaction_order.rs"]
mod wait_satisfaction_order;
#[path = "scheduling/waiter_edge_ordering.rs"]
mod waiter_edge_ordering;
#[path = "scheduling/xtrace_conformance.rs"]
mod xtrace_conformance;
#[path = "scheduling/zero_delay_inactive_region.rs"]
mod zero_delay_inactive_region;

#[path = "scheduling/compiled_for_loops.rs"]
mod compiled_for_loops;

#[path = "scheduling/always_value_change_after_edge_continuation.rs"]
mod always_value_change_after_edge_continuation;
#[path = "scheduling/bare_randomize_in_method.rs"]
mod bare_randomize_in_method;
#[path = "scheduling/class_event_member_wait.rs"]
mod class_event_member_wait;
#[path = "scheduling/class_event_parked_watchers.rs"]
mod class_event_parked_watchers;
#[path = "scheduling/class_nonevent_prop_wait.rs"]
mod class_nonevent_prop_wait;
#[path = "scheduling/clock_gen_dotted_target.rs"]
mod clock_gen_dotted_target;
#[path = "scheduling/clock_gen_fifo_rank.rs"]
mod clock_gen_fifo_rank;
#[path = "scheduling/clocking_cont_trigger_same_step.rs"]
mod clocking_cont_trigger_same_step;
#[path = "scheduling/comb_self_member_sensitivity.rs"]
mod comb_self_member_sensitivity;
#[path = "scheduling/delay_always_compound_body.rs"]
mod delay_always_compound_body;
#[path = "scheduling/delayed_write_pending_semantics.rs"]
mod delayed_write_pending_semantics;
#[path = "scheduling/edge_skip_shared_output.rs"]
mod edge_skip_shared_output;
#[path = "scheduling/finish_with_live_fork_child.rs"]
mod finish_with_live_fork_child;
#[path = "scheduling/hierarchical_event_wait.rs"]
mod hierarchical_event_wait;
#[path = "scheduling/inlined_call_rollback.rs"]
mod inlined_call_rollback;
#[path = "scheduling/intra_assignment_delay_in_always.rs"]
mod intra_assignment_delay_in_always;
#[path = "scheduling/labeled_block_local_hier_ref.rs"]
mod labeled_block_local_hier_ref;
#[path = "scheduling/lrm_audit_clocking_skew.rs"]
mod lrm_audit_clocking_skew;
#[path = "scheduling/lrm_audit_disable_from_fork.rs"]
mod lrm_audit_disable_from_fork;
#[path = "scheduling/lrm_audit_sva.rs"]
mod lrm_audit_sva;
#[path = "scheduling/nba_array_elem_last_write_wins.rs"]
mod nba_array_elem_last_write_wins;
#[path = "scheduling/seq_grant_nba_region_order.rs"]
mod seq_grant_nba_region_order;
#[path = "scheduling/nba_wait_for_region_yield.rs"]
mod nba_wait_for_region_yield;
#[path = "scheduling/nested_delay_slot_servicing.rs"]
mod nested_delay_slot_servicing;
#[path = "scheduling/object_event_paths.rs"]
mod object_event_paths;
#[path = "scheduling/package_and_unit_delays_scale_by_timescale.rs"]
mod package_and_unit_delays_scale_by_timescale;
#[path = "scheduling/package_cli_timescale_delay.rs"]
mod package_cli_timescale_delay;
#[path = "scheduling/phase_jump_static_latch.rs"]
mod phase_jump_static_latch;
#[path = "scheduling/release_from_level_sensitive_block.rs"]
mod release_from_level_sensitive_block;
#[path = "scheduling/static_recursion_shared_cell.rs"]
mod static_recursion_shared_cell;
#[path = "scheduling/sva_context_width.rs"]
mod sva_context_width;
#[path = "scheduling/sva_instances_and_sequences.rs"]
mod sva_instances_and_sequences;
#[path = "scheduling/sva_local_diagnostics.rs"]
mod sva_local_diagnostics;
#[path = "scheduling/sva_past_first_tick.rs"]
mod sva_past_first_tick;
#[path = "scheduling/sva_semantics_reference.rs"]
mod sva_semantics_reference;
#[path = "scheduling/waiter_cont_anyedge_wake.rs"]
mod waiter_cont_anyedge_wake;

#[path = "scheduling/clocking_delivery_356.rs"]
mod clocking_delivery_356;

#[path = "scheduling/sampled_history_363.rs"]
mod next_twelve_sampled_history_363;

#[path = "scheduling/sequence_counts_367.rs"]
mod next_twelve_sequence_counts_367;

#[path = "scheduling/finish_barrier_371.rs"]
mod next_twelve_finish_barrier_371;

#[path = "scheduling/generated_assertions_374.rs"]
mod next_rest_generated_assertions_374;

#[path = "scheduling/followed_by_375.rs"]
mod next_rest_followed_by_375;

#[path = "scheduling/loop_scope_379.rs"]
mod next_rest_loop_scope_379;

#[path = "scheduling/clock_edges_385.rs"]
mod clock_edges_385;

#[path = "scheduling/action_scopes_386.rs"]
mod action_scopes_386;

#[path = "scheduling/pending_negation_393.rs"]
mod pending_negation_393;
#[path = "scheduling/or_end_of_sim_compose_420.rs"]
mod or_end_of_sim_compose_420;
#[path = "scheduling/sampled_preponed_421.rs"]
mod sampled_preponed_421;
#[path = "scheduling/assertion_m_scope_419.rs"]
mod assertion_m_scope_419;
#[path = "scheduling/followed_vacuity_395.rs"]
mod followed_vacuity_395;

#[path = "scheduling/sva_array_sampling.rs"]
mod sva_array_sampling;

#[path = "scheduling/sva_named_property_hier_sampling.rs"]
mod sva_named_property_hier_sampling;

#[path = "scheduling/array_port_connection_timing.rs"]
mod array_port_connection_timing;
#[path = "scheduling/sva_event_lists.rs"]
mod sva_event_lists;

#[path = "scheduling/nested_vacuity_445.rs"]
mod nested_vacuity_445;

#[path = "scheduling/sampled_action_468.rs"]
mod sampled_action_468;

#[path = "scheduling/sampled_action_array_509.rs"]
mod sampled_action_array_509;

#[path = "scheduling/sva_until_properties.rs"]
mod sva_until_properties;

#[path = "scheduling/or_vacuity_508.rs"]
mod or_vacuity_508;

#[path = "scheduling/or_settles_early_533.rs"]
mod or_settles_early_533;

#[path = "scheduling/until_folding_510.rs"]
mod until_folding_510;

#[path = "scheduling/review_assertion_vacuity.rs"]
mod review_assertion_vacuity;

#[path = "scheduling/review_action_sampling.rs"]
mod review_action_sampling;

#[path = "scheduling/review_package_event_scopes.rs"]
mod review_package_event_scopes;
#[path = "scheduling/function_formal_not_in_sensitivity.rs"]
mod function_formal_not_in_sensitivity;

#[path = "scheduling/port_handshake_t0_550.rs"]
mod port_handshake_t0_550;
#[path = "scheduling/armed_edge_dynamic_offset_read.rs"]
mod armed_edge_dynamic_offset_read;
#[path = "scheduling/clocking_output_clockvar_read.rs"]
mod clocking_output_clockvar_read;
#[path = "scheduling/comb_sensitivity_591_592.rs"]
mod comb_sensitivity_591_592;
#[path = "scheduling/one_step_delay_615.rs"]
mod one_step_delay_615;
#[path = "scheduling/clocking_select_clockvars.rs"]
mod clocking_select_clockvars;

#[path = "scheduling/always_property.rs"]
mod always_property;
#[path = "scheduling/class_collection_named_wait.rs"]
mod class_collection_named_wait;

#[path = "scheduling/class_or_value_change.rs"]
mod class_or_value_change;

#[path = "scheduling/preprocessor_linear_join.rs"]
mod preprocessor_linear_join;
