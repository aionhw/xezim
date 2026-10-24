//! Integration-test group: misc.
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

#[path = "misc/always_timing_lint_forms.rs"]
mod always_timing_lint_forms;
#[path = "misc/ansi_net_port_procedural_assign.rs"]
mod ansi_net_port_procedural_assign;
#[path = "misc/argument_binding.rs"]
mod argument_binding;
#[path = "misc/artifact_compression_modes.rs"]
mod artifact_compression_modes;
#[path = "misc/assign_z_passthrough.rs"]
mod assign_z_passthrough;
#[path = "misc/assignment_compatibility.rs"]
mod assignment_compatibility;
#[path = "misc/assignment_pattern_counts.rs"]
mod assignment_pattern_counts;
#[path = "misc/assoc_class_new_stores_instance.rs"]
mod assoc_class_new_stores_instance;
#[path = "misc/audit_round46_finds.rs"]
mod audit_round46_finds;
#[path = "misc/audit_sibling_fixes.rs"]
mod audit_sibling_fixes;
#[path = "misc/automatic_and_procedural_assign.rs"]
mod automatic_and_procedural_assign;
#[path = "misc/bare_method_call_returns.rs"]
mod bare_method_call_returns;
#[path = "misc/bare_randomize_solver.rs"]
mod bare_randomize_solver;
#[path = "misc/base1_packed_index.rs"]
mod base1_packed_index;
#[path = "misc/begin_keywords_noconfig.rs"]
mod begin_keywords_noconfig;
#[path = "misc/bind_upward_refs.rs"]
mod bind_upward_refs;
#[path = "misc/blocking_task_call.rs"]
mod blocking_task_call;
#[path = "misc/c910_create_en_cont_assign.rs"]
mod c910_create_en_cont_assign;
#[path = "misc/c910_create_en_full_path.rs"]
mod c910_create_en_full_path;
#[path = "misc/case_default_arm.rs"]
mod case_default_arm;
#[path = "misc/chained_member_access.rs"]
mod chained_member_access;
#[path = "misc/child_decl_init_and_wide_rand.rs"]
mod child_decl_init_and_wide_rand;
#[path = "misc/cli_files.rs"]
mod cli_files;
#[path = "misc/clocked_loop_case_nest_compiled.rs"]
mod clocked_loop_case_nest_compiled;
#[path = "misc/code_coverage.rs"]
mod code_coverage;
#[path = "misc/comb_regvar_loop_fallback.rs"]
mod comb_regvar_loop_fallback;
#[path = "misc/compliance_tests.rs"]
mod compliance_tests;
#[path = "misc/const_chain_fold.rs"]
mod const_chain_fold;
#[path = "misc/constant_expression_contexts.rs"]
mod constant_expression_contexts;
#[path = "misc/constraint_issue_regressions.rs"]
mod constraint_issue_regressions;
#[path = "misc/constraint_select_regressions.rs"]
mod constraint_select_regressions;
#[path = "misc/cont_assign_rhs_undeclared.rs"]
mod cont_assign_rhs_undeclared;
#[path = "misc/cov_assertion_basic.rs"]
mod cov_assertion_basic;
#[path = "misc/cover_sequence.rs"]
mod cover_sequence;
#[path = "misc/coverage_db.rs"]
mod coverage_db;
#[path = "misc/defparam_local_targets.rs"]
mod defparam_local_targets;
#[path = "misc/delay_precision.rs"]
mod delay_precision;
#[path = "misc/dep_reg_entry_synth.rs"]
mod dep_reg_entry_synth;
#[path = "misc/deposit_task.rs"]
mod deposit_task;
#[path = "misc/disable_task_inside_foreach.rs"]
mod disable_task_inside_foreach;
#[path = "misc/duplicate_decl_locations.rs"]
mod duplicate_decl_locations;
#[path = "misc/elaboration_runaway_guards.rs"]
mod elaboration_runaway_guards;
#[path = "misc/escaped_identifier_matches_nonescaped.rs"]
mod escaped_identifier_matches_nonescaped;
#[path = "misc/event_argument.rs"]
mod event_argument;
#[path = "misc/for_loop_var_declaration.rs"]
mod for_loop_var_declaration;
#[path = "misc/force_assign_override_restore.rs"]
mod force_assign_override_restore;
#[path = "misc/force_release_semantics.rs"]
mod force_release_semantics;
#[path = "misc/foreach_loop_variable_count.rs"]
mod foreach_loop_variable_count;
#[path = "misc/forever_break_continue.rs"]
mod forever_break_continue;
#[path = "misc/function_return_bit_select.rs"]
mod function_return_bit_select;
#[path = "misc/gate_terminal_counts.rs"]
mod gate_terminal_counts;
#[path = "misc/generate_block_name_clash.rs"]
mod generate_block_name_clash;
#[path = "misc/generate_item_restrictions.rs"]
mod generate_item_restrictions;
#[path = "misc/generate_loop_genvar.rs"]
mod generate_loop_genvar;
#[path = "misc/generate_region_contents.rs"]
mod generate_region_contents;
#[path = "misc/iface_functions_and_vif_locals.rs"]
mod iface_functions_and_vif_locals;
#[path = "misc/ifu_ibuf_casez_dispatch_c910.rs"]
mod ifu_ibuf_casez_dispatch_c910;
#[path = "misc/ifu_ibuf_create_ptr_rotate.rs"]
mod ifu_ibuf_create_ptr_rotate;
#[path = "misc/ifu_ibuf_entry_pop_c910.rs"]
mod ifu_ibuf_entry_pop_c910;
#[path = "misc/ifu_precode_c910_pc710.rs"]
mod ifu_precode_c910_pc710;
#[path = "misc/implicit_static_diagnostic.rs"]
mod implicit_static_diagnostic;
#[path = "misc/include_path_from_function_macro.rs"]
mod include_path_from_function_macro;
#[path = "misc/init_reg_seed.rs"]
mod init_reg_seed;
#[path = "misc/inspect_types.rs"]
mod inspect_types;
#[path = "misc/interconnect_and_var_ports.rs"]
mod interconnect_and_var_ports;
#[path = "misc/issue30_runner.rs"]
mod issue30_runner;
#[path = "misc/issue_cases_runner.rs"]
mod issue_cases_runner;
#[path = "misc/ivtest_ce_cluster.rs"]
mod ivtest_ce_cluster;
#[path = "misc/ivtest_misc_cluster.rs"]
mod ivtest_misc_cluster;
#[path = "misc/ivtest_round55_pins.rs"]
mod ivtest_round55_pins;
#[path = "misc/ivtest_tail2_cluster.rs"]
mod ivtest_tail2_cluster;
#[path = "misc/ivtest_tail_cluster.rs"]
mod ivtest_tail_cluster;
#[path = "misc/library_flags.rs"]
mod library_flags;
#[path = "misc/local_shadow_select.rs"]
mod local_shadow_select;
#[path = "misc/log_redirect.rs"]
mod log_redirect;
#[path = "misc/lrm_audit2_runner.rs"]
mod lrm_audit2_runner;
#[path = "misc/lrm_audit3_runner.rs"]
mod lrm_audit3_runner;
#[path = "misc/lrm_audit_runner.rs"]
mod lrm_audit_runner;
#[path = "misc/lrm_clause10_patterns.rs"]
mod lrm_clause10_patterns;
#[path = "misc/lrm_clause11_operators.rs"]
mod lrm_clause11_operators;
#[path = "misc/lrm_clause13_subroutines.rs"]
mod lrm_clause13_subroutines;
#[path = "misc/lrm_pattern_matching.rs"]
mod lrm_pattern_matching;
#[path = "misc/macro_default_after_string_actual.rs"]
mod macro_default_after_string_actual;
#[path = "misc/mailbox_method_blocking.rs"]
mod mailbox_method_blocking;
#[path = "misc/method_call_chaining.rs"]
mod method_call_chaining;
#[path = "misc/min_typ_max_delays.rs"]
mod min_typ_max_delays;
#[path = "misc/monitor_on_change.rs"]
mod monitor_on_change;
#[path = "misc/monitor_percent_m_scope.rs"]
mod monitor_percent_m_scope;
#[path = "misc/named_block_hierarchical_name.rs"]
mod named_block_hierarchical_name;
#[path = "misc/nba_array_read_x_index.rs"]
mod nba_array_read_x_index;
#[path = "misc/negative_lsb_range_select.rs"]
mod negative_lsb_range_select;
#[path = "misc/net_class_data_type.rs"]
mod net_class_data_type;
#[path = "misc/net_declaration_delay.rs"]
mod net_declaration_delay;
#[path = "misc/net_dynamic_array.rs"]
mod net_dynamic_array;
#[path = "misc/nonansi_function_args.rs"]
mod nonansi_function_args;
#[path = "misc/nonansi_ranged_subroutine_port.rs"]
mod nonansi_ranged_subroutine_port;
#[path = "misc/nonzero_lsb_indexed_part_select.rs"]
mod nonzero_lsb_indexed_part_select;
#[path = "misc/obj_assocd_event_disable_fork.rs"]
mod obj_assocd_event_disable_fork;
#[path = "misc/operators_11_select_reduce.rs"]
mod operators_11_select_reduce;
#[path = "misc/override_timescale.rs"]
mod override_timescale;
#[path = "misc/package_exports.rs"]
mod package_exports;
#[path = "misc/package_qualified_access_in_subroutine.rs"]
mod package_qualified_access_in_subroutine;
#[path = "misc/package_reexport_explicit_import.rs"]
mod package_reexport_explicit_import;
#[path = "misc/package_scope_and_call_kind.rs"]
mod package_scope_and_call_kind;
#[path = "misc/packed_mem_arena.rs"]
mod packed_mem_arena;
#[path = "misc/param_class_cast_type_args.rs"]
mod param_class_cast_type_args;
#[path = "misc/param_pair_this_type_cast.rs"]
mod param_pair_this_type_cast;
#[path = "misc/parameter_override_legality.rs"]
mod parameter_override_legality;
#[path = "misc/parameter_port_keyword.rs"]
mod parameter_port_keyword;
#[path = "misc/parameter_value_references.rs"]
mod parameter_value_references;
#[path = "misc/parser_gaps2.rs"]
mod parser_gaps2;
#[path = "misc/parser_stmt_gaps.rs"]
mod parser_stmt_gaps;
#[path = "misc/part_select_constant_bounds.rs"]
mod part_select_constant_bounds;
#[path = "misc/part_select_negative_bound_compiled.rs"]
mod part_select_negative_bound_compiled;
#[path = "misc/placeholder_parameter_replication.rs"]
mod placeholder_parameter_replication;
#[path = "misc/port_width_mismatch_explains.rs"]
mod port_width_mismatch_explains;
#[path = "misc/readmem_hierarchical_target.rs"]
mod readmem_hierarchical_target;
#[path = "misc/real_call_bytecode.rs"]
mod real_call_bytecode;
#[path = "misc/reduce_loop_unroll.rs"]
mod reduce_loop_unroll;
#[path = "misc/repeat_real_count.rs"]
mod repeat_real_count;
#[path = "misc/replication_and_cast_operands.rs"]
mod replication_and_cast_operands;
#[path = "misc/runtime_error_exit_status.rs"]
mod runtime_error_exit_status;
#[path = "misc/select_depth.rs"]
mod select_depth;
#[path = "misc/select_semantics_11_5_1.rs"]
mod select_semantics_11_5_1;
#[path = "misc/shadow_name_matrix.rs"]
mod shadow_name_matrix;
#[path = "misc/simulation_version_banner.rs"]
mod simulation_version_banner;
#[path = "misc/spec_static_and_pkg_queue.rs"]
mod spec_static_and_pkg_queue;
#[path = "misc/struct_members_and_constant_targets.rs"]
mod struct_members_and_constant_targets;
#[path = "misc/subroutine_port_redeclaration.rs"]
mod subroutine_port_redeclaration;
#[path = "misc/subroutine_port_type.rs"]
mod subroutine_port_type;
#[path = "misc/subroutine_range_identifiers.rs"]
mod subroutine_range_identifiers;
#[path = "misc/svtb_suite.rs"]
mod svtb_suite;
#[path = "misc/system_task_value.rs"]
mod system_task_value;
#[path = "misc/time0_x_defaults.rs"]
mod time0_x_defaults;
#[path = "misc/unnamed_block_end_label.rs"]
mod unnamed_block_end_label;
#[path = "misc/unsized_decimal_wrap_warning.rs"]
mod unsized_decimal_wrap_warning;
#[path = "misc/uvm_agent_active_config.rs"]
mod uvm_agent_active_config;
#[path = "misc/uvm_dir_env.rs"]
mod uvm_dir_env;
#[path = "misc/uvm_dpi_builtins.rs"]
mod uvm_dpi_builtins;
#[path = "misc/value_trace.rs"]
mod value_trace;
// The AOT backend only exists in --features jit builds; without it the
// binary ignores XEZIM_AOT and the coverage assertion can never hold.
#[path = "misc/aligned_lane_coalescing.rs"]
mod aligned_lane_coalescing;
#[cfg(feature = "jit")]
#[path = "misc/aot_native_paths.rs"]
mod aot_native_paths;
#[path = "misc/assoc_dotted_key.rs"]
mod assoc_dotted_key;
#[path = "misc/call_returned_handle_assoc.rs"]
mod call_returned_handle_assoc;
#[path = "misc/edge_and_delay_select_fixes.rs"]
mod edge_and_delay_select_fixes;
#[path = "misc/edge_task_call_process.rs"]
mod edge_task_call_process;
#[path = "misc/factory_register_reentrant.rs"]
mod factory_register_reentrant;
#[path = "misc/fallback_carried_locals.rs"]
mod fallback_carried_locals;
#[path = "misc/new_ctor_vs_shallow_copy.rs"]
mod new_ctor_vs_shallow_copy;
#[path = "misc/packed_elem_fn_inline.rs"]
mod packed_elem_fn_inline;
#[path = "misc/param_dim_array_nba.rs"]
mod param_dim_array_nba;
#[path = "misc/pattern_replication_and_extends_args.rs"]
mod pattern_replication_and_extends_args;
#[path = "misc/proc_fsm.rs"]
mod proc_fsm;
#[path = "misc/program_block_reactive.rs"]
mod program_block_reactive;
#[path = "misc/property_bare_new_initializer.rs"]
mod property_bare_new_initializer;
#[path = "misc/property_new_initializer.rs"]
mod property_new_initializer;
#[path = "misc/prtest_runner.rs"]
mod prtest_runner;
#[path = "misc/randcase_randsequence_weights.rs"]
mod randcase_randsequence_weights;
#[path = "misc/random_stability.rs"]
mod random_stability;
#[path = "misc/reg3456_pure_sv.rs"]
mod reg3456_pure_sv;
#[path = "misc/reg4712_visitor_traversal.rs"]
mod reg4712_visitor_traversal;
#[path = "misc/region_fusion.rs"]
mod region_fusion;
#[path = "misc/repeat_blocking_call.rs"]
mod repeat_blocking_call;
#[path = "misc/replicate_and_pattern.rs"]
mod replicate_and_pattern;
#[path = "misc/resource_pool_fixes.rs"]
mod resource_pool_fixes;
#[path = "misc/sanity_tests.rs"]
mod sanity_tests;
#[path = "misc/sim_tests.rs"]
mod sim_tests;
#[path = "misc/single_eval.rs"]
mod single_eval;
#[path = "misc/size_test.rs"]
mod size_test;
#[path = "misc/spec_canonical_keying.rs"]
mod spec_canonical_keying;
#[path = "misc/specparam_in_parameter_init.rs"]
mod specparam_in_parameter_init;
#[path = "misc/static_collection_per_spec.rs"]
mod static_collection_per_spec;
#[path = "misc/static_init_sysfuncs.rs"]
mod static_init_sysfuncs;
#[path = "misc/static_local_per_spec.rs"]
mod static_local_per_spec;
#[path = "misc/static_local_select_write.rs"]
mod static_local_select_write;
#[path = "misc/static_method_via_handle.rs"]
mod static_method_via_handle;
#[path = "misc/static_property_access.rs"]
mod static_property_access;
#[path = "misc/stress_tests.rs"]
mod stress_tests;
#[path = "misc/struct_arrays_and_interface_members.rs"]
mod struct_arrays_and_interface_members;
#[path = "misc/super_method_dispatch.rs"]
mod super_method_dispatch;
#[path = "misc/sv2023_compliance_runner.rs"]
mod sv2023_compliance_runner;
#[path = "misc/sv_compliance_runner.rs"]
mod sv_compliance_runner;
#[path = "misc/sv_logic_implication.rs"]
mod sv_logic_implication;
#[path = "misc/sva_action_firing.rs"]
mod sva_action_firing;
#[path = "misc/sva_named_property_in_instance.rs"]
mod sva_named_property_in_instance;
#[path = "misc/sva_preponed_sampling.rs"]
mod sva_preponed_sampling;
#[path = "misc/timescale.rs"]
mod timescale;
#[path = "misc/unpacked_struct_in_instance.rs"]
mod unpacked_struct_in_instance;
#[path = "misc/unsized_literal_keeps_its_digits.rs"]
mod unsized_literal_keeps_its_digits;
#[path = "misc/void_cast_operand.rs"]
mod void_cast_operand;
#[path = "misc/warm_cache_diag_replay.rs"]
mod warm_cache_diag_replay;
#[path = "misc/wildcard_import_conflict.rs"]
mod wildcard_import_conflict;
#[path = "misc/x_warn_switch.rs"]
mod x_warn_switch;
#[path = "misc/xselect_and_concat_flatten.rs"]
mod xselect_and_concat_flatten;

#[path = "misc/array_elem_fast_write.rs"]
mod array_elem_fast_write;
#[path = "misc/array_elem_indexed_part_select_target.rs"]
mod array_elem_indexed_part_select_target;
#[path = "misc/array_element_semantics.rs"]
mod array_element_semantics;
#[path = "misc/audit_round45_finds.rs"]
mod audit_round45_finds;
#[path = "misc/class_collection_storage.rs"]
mod class_collection_storage;
#[path = "misc/class_queue_locators.rs"]
mod class_queue_locators;
#[path = "misc/cli_compat_args.rs"]
mod cli_compat_args;
#[path = "misc/cross_checked_constructs.rs"]
mod cross_checked_constructs;
#[path = "misc/dead_giant_declaration_elision.rs"]
mod dead_giant_declaration_elision;
#[path = "misc/diagnostic_locations.rs"]
mod diagnostic_locations;
#[path = "misc/elaboration_shape_fixes.rs"]
mod elaboration_shape_fixes;
#[path = "misc/env_var_registry.rs"]
mod env_var_registry;
#[path = "misc/exit_codes.rs"]
mod exit_codes;
#[path = "misc/fresh_audit_finds.rs"]
mod fresh_audit_finds;
#[path = "misc/hier_force_holds_against_drivers.rs"]
mod hier_force_holds_against_drivers;
#[path = "misc/hier_iface_task_suspension.rs"]
mod hier_iface_task_suspension;
#[path = "misc/hier_port_drive_and_collision.rs"]
mod hier_port_drive_and_collision;
#[path = "misc/indexed_event_roundtrip.rs"]
mod indexed_event_roundtrip;
#[path = "misc/loop_body_inlines_pure_call.rs"]
mod loop_body_inlines_pure_call;
#[path = "misc/loopvar_collision_child_leaf.rs"]
mod loopvar_collision_child_leaf;
#[path = "misc/lrm_audit_force_net_select.rs"]
mod lrm_audit_force_net_select;
#[path = "misc/macro_directive_prefix_names.rs"]
mod macro_directive_prefix_names;
#[path = "misc/nested_struct_string_member_display.rs"]
mod nested_struct_string_member_display;
#[path = "misc/nettype_net_in_submodule.rs"]
mod nettype_net_in_submodule;
#[path = "misc/package_const_fn_params.rs"]
mod package_const_fn_params;
#[path = "misc/package_property_assertions.rs"]
mod package_property_assertions;
#[path = "misc/packed_mem_nba_region.rs"]
mod packed_mem_nba_region;
#[path = "misc/preprocessor_directives.rs"]
mod preprocessor_directives;
#[path = "misc/process_status_name.rs"]
mod process_status_name;
#[path = "misc/pure_fn_loops_compiled.rs"]
mod pure_fn_loops_compiled;
#[path = "misc/queue_ref_formal_shadowing.rs"]
mod queue_ref_formal_shadowing;
#[path = "misc/recursive_function_in_continuous_assign.rs"]
mod recursive_function_in_continuous_assign;
#[path = "misc/ref_args_alias.rs"]
mod ref_args_alias;
#[path = "misc/ref_struct_queue_and_local_shadow.rs"]
mod ref_struct_queue_and_local_shadow;
#[path = "misc/sched_trace_orders_a_time_slot.rs"]
mod sched_trace_orders_a_time_slot;
#[path = "misc/severity_exit_status.rs"]
mod severity_exit_status;
#[path = "misc/sigterm_ends_stuck_runs.rs"]
mod sigterm_ends_stuck_runs;
#[path = "misc/static_local_scope_isolation.rs"]
mod static_local_scope_isolation;
#[path = "misc/stdout_flush_at_sim_finish.rs"]
mod stdout_flush_at_sim_finish;
#[path = "misc/struct_local_declinit_copy.rs"]
mod struct_local_declinit_copy;
#[path = "misc/symbol_clash_checks.rs"]
mod symbol_clash_checks;
#[path = "misc/trireg_charge_and_g_audit.rs"]
mod trireg_charge_and_g_audit;
#[path = "misc/two_state_island_coverage.rs"]
mod two_state_island_coverage;
#[path = "misc/two_state_lowering_shapes.rs"]
mod two_state_lowering_shapes;
#[path = "misc/two_state_wide_reduction.rs"]
mod two_state_wide_reduction;
#[path = "misc/udn_resolver_compiled.rs"]
mod udn_resolver_compiled;
#[path = "misc/undeclared_in_subroutines.rs"]
mod undeclared_in_subroutines;
#[path = "misc/uninstantiated_module_params.rs"]
mod uninstantiated_module_params;
#[path = "misc/virtual_array_names.rs"]
mod virtual_array_names;
#[path = "misc/while_continue_final_iter.rs"]
mod while_continue_final_iter;
#[path = "misc/x_propagation_assertions.rs"]
mod x_propagation_assertions;
#[path = "misc/zero_width_select_unselected_generate.rs"]
mod zero_width_select_unselected_generate;
#[path = "misc/let_declaration_scopes.rs"]
mod let_declaration_scopes;
#[path = "misc/constraint_inside_wrap.rs"]
mod constraint_inside_wrap;
#[path = "misc/loop_var_scope_lint.rs"]
mod loop_var_scope_lint;

#[path = "misc/cli_transcript_347.rs"]
mod cli_transcript_347;

#[path = "misc/relaxed_static_initializers_351.rs"]
mod relaxed_static_initializers_351;

#[path = "misc/string_casts_391.rs"]
mod string_casts_391;
#[path = "misc/undefined_macro_recovery.rs"]
mod undefined_macro_recovery;

#[path = "misc/constant_scopes_488.rs"]
mod constant_scopes_488;
