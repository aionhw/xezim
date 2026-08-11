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

#[path = "misc/artifact_compression_modes.rs"]
mod artifact_compression_modes;
#[path = "misc/assign_z_passthrough.rs"]
mod assign_z_passthrough;
#[path = "misc/audit_sibling_fixes.rs"]
mod audit_sibling_fixes;
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
#[path = "misc/compliance_tests.rs"]
mod compliance_tests;
#[path = "misc/cov_assertion_basic.rs"]
mod cov_assertion_basic;
#[path = "misc/dep_reg_entry_synth.rs"]
mod dep_reg_entry_synth;
#[path = "misc/deposit_task.rs"]
mod deposit_task;
#[path = "misc/duplicate_decl_locations.rs"]
mod duplicate_decl_locations;
#[path = "misc/escaped_identifier_matches_nonescaped.rs"]
mod escaped_identifier_matches_nonescaped;
#[path = "misc/force_assign_override_restore.rs"]
mod force_assign_override_restore;
#[path = "misc/force_release_semantics.rs"]
mod force_release_semantics;
#[path = "misc/forever_break_continue.rs"]
mod forever_break_continue;
#[path = "misc/function_return_bit_select.rs"]
mod function_return_bit_select;
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
#[path = "misc/inspect_types.rs"]
mod inspect_types;
#[path = "misc/issue30_runner.rs"]
mod issue30_runner;
#[path = "misc/issue_cases_runner.rs"]
mod issue_cases_runner;
#[path = "misc/ivtest_ce_cluster.rs"]
mod ivtest_ce_cluster;
#[path = "misc/ivtest_misc_cluster.rs"]
mod ivtest_misc_cluster;
#[path = "misc/ivtest_tail2_cluster.rs"]
mod ivtest_tail2_cluster;
#[path = "misc/ivtest_tail_cluster.rs"]
mod ivtest_tail_cluster;
#[path = "misc/library_flags.rs"]
mod library_flags;
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
#[path = "misc/method_call_chaining.rs"]
mod method_call_chaining;
#[path = "misc/monitor_on_change.rs"]
mod monitor_on_change;
#[path = "misc/negative_lsb_range_select.rs"]
mod negative_lsb_range_select;
#[path = "misc/nonansi_function_args.rs"]
mod nonansi_function_args;
#[path = "misc/nonzero_lsb_indexed_part_select.rs"]
mod nonzero_lsb_indexed_part_select;
#[path = "misc/operators_11_select_reduce.rs"]
mod operators_11_select_reduce;
#[path = "misc/parser_gaps2.rs"]
mod parser_gaps2;
#[path = "misc/parser_stmt_gaps.rs"]
mod parser_stmt_gaps;
#[path = "misc/obj_assocd_event_disable_fork.rs"]
mod obj_assocd_event_disable_fork;
#[path = "misc/indexed_event_roundtrip.rs"]
mod indexed_event_roundtrip;
#[path = "misc/param_class_cast_type_args.rs"]
mod param_class_cast_type_args;
#[path = "misc/param_pair_this_type_cast.rs"]
mod param_pair_this_type_cast;
#[path = "misc/assoc_class_new_stores_instance.rs"]
mod assoc_class_new_stores_instance;
#[path = "misc/bare_method_call_returns.rs"]
mod bare_method_call_returns;
#[path = "misc/bare_randomize_solver.rs"]
mod bare_randomize_solver;
#[path = "misc/uvm_agent_active_config.rs"]
mod uvm_agent_active_config;
#[path = "misc/new_ctor_vs_shallow_copy.rs"]
mod new_ctor_vs_shallow_copy;
#[path = "misc/assoc_dotted_key.rs"]
mod assoc_dotted_key;
#[path = "misc/call_returned_handle_assoc.rs"]
mod call_returned_handle_assoc;
#[path = "misc/factory_register_reentrant.rs"]
mod factory_register_reentrant;
#[path = "misc/pattern_replication_and_extends_args.rs"]
mod pattern_replication_and_extends_args;
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
#[path = "misc/static_collection_per_spec.rs"]
mod static_collection_per_spec;
#[path = "misc/static_init_sysfuncs.rs"]
mod static_init_sysfuncs;
#[path = "misc/static_local_per_spec.rs"]
mod static_local_per_spec;
#[path = "misc/static_property_access.rs"]
mod static_property_access;
#[path = "misc/stress_tests.rs"]
mod stress_tests;
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
#[path = "misc/sva_preponed_sampling.rs"]
mod sva_preponed_sampling;
#[path = "misc/timescale.rs"]
mod timescale;
#[path = "misc/struct_arrays_and_interface_members.rs"]
mod struct_arrays_and_interface_members;
#[path = "misc/unpacked_struct_in_instance.rs"]
mod unpacked_struct_in_instance;
#[path = "misc/unsized_literal_keeps_its_digits.rs"]
mod unsized_literal_keeps_its_digits;
#[path = "misc/warm_cache_diag_replay.rs"]
mod warm_cache_diag_replay;
#[path = "misc/string_formal_name_leak.rs"]
mod string_formal_name_leak;
#[path = "misc/x_warn_switch.rs"]
mod x_warn_switch;
#[path = "misc/xselect_and_concat_flatten.rs"]
mod xselect_and_concat_flatten;
#[path = "misc/static_method_via_handle.rs"]
mod static_method_via_handle;

#[path = "misc/array_elem_fast_write.rs"]
mod array_elem_fast_write;
#[path = "misc/severity_exit_status.rs"]
mod severity_exit_status;
#[path = "misc/preprocessor_directives.rs"]
mod preprocessor_directives;
#[path = "misc/package_const_fn_params.rs"]
mod package_const_fn_params;
#[path = "misc/ref_args_alias.rs"]
mod ref_args_alias;
#[path = "misc/trireg_charge_and_g_audit.rs"]
mod trireg_charge_and_g_audit;
#[path = "misc/process_status_name.rs"]
mod process_status_name;

#[path = "misc/symbol_clash_checks.rs"]
mod symbol_clash_checks;
