//! Integration-test group: classes.
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

#[path = "classes/array_equality_class.rs"]
mod array_equality_class;
#[path = "classes/assoc_typedef_element_class.rs"]
mod assoc_typedef_element_class;
#[path = "classes/bit_class_property_signedness.rs"]
mod bit_class_property_signedness;
#[path = "classes/block_local_shadows_enclosing.rs"]
mod block_local_shadows_enclosing;
#[path = "classes/blocking_task_super_dispatch.rs"]
mod blocking_task_super_dispatch;
#[path = "classes/blocklocal_struct_named_like_member.rs"]
mod blocklocal_struct_named_like_member;
#[path = "classes/call_actual_formal_width_557.rs"]
mod call_actual_formal_width;
#[path = "classes/class_assoc_struct_string_keys.rs"]
mod class_assoc_struct_string_keys;
#[path = "classes/class_typedef_assoc_key.rs"]
mod class_typedef_assoc_key;
#[path = "classes/class_descending_array_members.rs"]
mod class_descending_array_members;
#[path = "classes/class_formal_named_like_struct.rs"]
mod class_formal_named_like_struct;
#[path = "classes/class_formal_typedef_widen.rs"]
mod class_formal_typedef_widen;
#[path = "classes/class_interface_same_name.rs"]
mod class_interface_same_name;
#[path = "classes/class_member_name_clash.rs"]
mod class_member_name_clash;
#[path = "classes/class_member_operand_width_557.rs"]
mod class_member_operand_width;
#[path = "classes/class_method_named_put_from_function.rs"]
mod class_method_named_put_from_function;
#[path = "classes/class_scope_array_const_whole_refs.rs"]
mod class_scope_array_const_whole_refs;
#[path = "classes/compiled_method_static_locals.rs"]
mod compiled_method_static_locals;
#[path = "classes/class_param_default_scope.rs"]
mod class_param_default_scope;
#[path = "classes/class_queue_struct_prop_copy.rs"]
mod class_queue_struct_prop_copy;
#[path = "classes/class_property_x_default.rs"]
mod class_property_x_default;
#[path = "classes/issue320_harness_mwe.rs"]
mod issue320_harness_mwe;
#[path = "classes/vif_specialization_keys.rs"]
mod vif_specialization_keys;
#[path = "classes/package_scoped_type_args.rs"]
mod package_scoped_type_args;
#[path = "classes/x_default_neighbours.rs"]
mod x_default_neighbours;
#[path = "classes/x_default_open.rs"]
mod x_default_open;
#[path = "classes/class_queue_struct_member_arrays.rs"]
mod class_queue_struct_member_arrays;
#[path = "classes/class_struct_collection_elements.rs"]
mod class_struct_collection_elements;
#[path = "classes/class_time_field_neg_one.rs"]
mod class_time_field_neg_one;
#[path = "classes/class_type_param_default_width.rs"]
mod class_type_param_default_width;
#[path = "classes/class_unpacked_struct_array_store.rs"]
mod class_unpacked_struct_array_store;
#[path = "classes/collection_of_handles_new.rs"]
mod collection_of_handles_new;
#[path = "classes/constraint_foreach_signed_indices.rs"]
mod constraint_foreach_signed_indices;
#[path = "classes/constructor_body_ports.rs"]
mod constructor_body_ports;
#[path = "classes/ctor_bare_member_dynarray_size.rs"]
mod ctor_bare_member_dynarray_size;
#[path = "classes/explicit_param_static_coll_read.rs"]
mod explicit_param_static_coll_read;
#[path = "classes/foreach_assoc_key_handle_write.rs"]
mod foreach_assoc_key_handle_write;
#[path = "classes/foreach_cross_elem_whole_equality.rs"]
mod foreach_cross_elem_whole_equality;
#[path = "classes/foreach_packed_struct_member_inside.rs"]
mod foreach_packed_struct_member_inside;
#[path = "classes/function_local_struct_return_shadow.rs"]
mod function_local_struct_return_shadow;
#[path = "classes/implements_typedef_scope.rs"]
mod implements_typedef_scope;
#[path = "classes/inherited_local_member.rs"]
mod inherited_local_member;
#[path = "classes/integral_prop_named_like_struct.rs"]
mod integral_prop_named_like_struct;
#[path = "classes/interface_class_inherited_names.rs"]
mod interface_class_inherited_names;
#[path = "classes/memory_tasks_fixed_property.rs"]
mod memory_tasks_fixed_property;
#[path = "classes/nested_same_named_ref_assoc_formal.rs"]
mod nested_same_named_ref_assoc_formal;
#[path = "classes/nested_static_call_named_args.rs"]
mod nested_static_call_named_args;
#[path = "classes/param_type_binding_resolves_enclosing_value_param.rs"]
mod param_type_binding_resolves_enclosing_value_param;
#[path = "classes/parameterized_class_scope.rs"]
mod parameterized_class_scope;
#[path = "classes/pre_randomize_mode_changes.rs"]
mod pre_randomize_mode_changes;
#[path = "classes/pure_constraint_implemented.rs"]
mod pure_constraint_implemented;
#[path = "classes/pure_constraint_nonvirtual.rs"]
mod pure_constraint_nonvirtual;
#[path = "classes/rand_array_elem_signedness.rs"]
mod rand_array_elem_signedness;
#[path = "classes/rand_obj_array_elem_constraints.rs"]
mod rand_obj_array_elem_constraints;
#[path = "classes/randomize_args_name_object_members.rs"]
mod randomize_args_name_object_members;
#[path = "classes/randomize_failure_diag.rs"]
mod randomize_failure_diag;
#[path = "classes/static_fixed_array_storage.rs"]
mod static_fixed_array_storage;
#[path = "classes/static_param_class_collection_reuse.rs"]
mod static_param_class_collection_reuse;
#[path = "classes/package_param_spec_args.rs"]
mod package_param_spec_args;
#[path = "classes/struct_member_bitselect_frame_local.rs"]
mod struct_member_bitselect_frame_local;
#[path = "classes/struct_member_class_handle_new.rs"]
mod struct_member_class_handle_new;
#[path = "classes/subroutine_formal_frame_type.rs"]
mod subroutine_formal_frame_type;
#[path = "classes/task_static_one_cell_per_declaration.rs"]
mod task_static_one_cell_per_declaration;
#[path = "classes/this_collection_struct_members.rs"]
mod this_collection_struct_members;
#[path = "classes/this_path_state_soft_randomize.rs"]
mod this_path_state_soft_randomize;
#[path = "classes/this_super_member.rs"]
mod this_super_member;
#[path = "classes/type_param_base_class.rs"]
mod type_param_base_class;
#[path = "classes/type_param_base_named_forward.rs"]
mod type_param_base_named_forward;
#[path = "classes/type_param_base_typedef_arg.rs"]
mod type_param_base_typedef_arg;
#[path = "classes/type_param_default_implicit_extends.rs"]
mod type_param_default_implicit_extends;
#[path = "classes/type_param_formal_stale_local.rs"]
mod type_param_formal_stale_local;
#[path = "classes/type_param_property_member_write.rs"]
mod type_param_property_member_write;
#[path = "classes/typename_p_subroutine_locals.rs"]
mod typename_p_subroutine_locals;
#[path = "classes/typename_type_param_resolves_concrete.rs"]
mod typename_type_param_resolves_concrete;
#[path = "classes/vif_output_actual_property.rs"]
mod vif_output_actual_property;
#[path = "classes/virtual_method_in_binary_is_evaluated_once.rs"]
mod virtual_method_in_binary_is_evaluated_once;
#[path = "classes/wait_level_sensitive_inactive_delta.rs"]
mod wait_level_sensitive_inactive_delta;
#[path = "classes/owner_qualified_operand_width_559.rs"]
mod owner_qualified_operand_width;

#[path = "classes/bare_parameterless_method_call.rs"]
mod bare_parameterless_method_call;
#[path = "classes/bytecode_wider_formal_width_557.rs"]
mod bytecode_wider_formal_width;
#[path = "classes/blocking_method_keyed_method_local_base.rs"]
mod blocking_method_keyed_method_local_base;
#[path = "classes/chained_member_element_access.rs"]
mod chained_member_element_access;
#[path = "classes/class_field_named_event.rs"]
mod class_field_named_event;
#[path = "classes/class_handle_return_preservation.rs"]
mod class_handle_return_preservation;
#[path = "classes/compiled_method_class_return.rs"]
mod compiled_method_class_return;
#[path = "classes/compiled_method_fast_call.rs"]
mod compiled_method_fast_call;
#[path = "classes/compiled_method_step6_surface.rs"]
mod compiled_method_step6_surface;
#[path = "classes/compiled_method_string_forms.rs"]
mod compiled_method_string_forms;

#[path = "classes/compiled_method_coll_elems.rs"]
mod compiled_method_coll_elems;
#[path = "classes/compiled_method_coll_shadow.rs"]
mod compiled_method_coll_shadow;
#[path = "classes/compiled_method_collection_calls.rs"]
mod compiled_method_collection_calls;
#[path = "classes/compiled_method_enum_formals.rs"]
mod compiled_method_enum_formals;
#[path = "classes/compiled_method_ref_formals.rs"]
mod compiled_method_ref_formals;
#[path = "classes/compiled_method_string_members.rs"]
mod compiled_method_string_members;

#[path = "classes/compiled_method_foreach.rs"]
mod compiled_method_foreach;
#[path = "classes/compiled_method_local_colls.rs"]
mod compiled_method_local_colls;

#[path = "classes/compiled_method_void.rs"]
mod compiled_method_void;

#[path = "classes/class_local_typedef_aa.rs"]
mod class_local_typedef_aa;
#[path = "classes/class_local_typedef_resolution.rs"]
mod class_local_typedef_resolution;
#[path = "classes/class_method_dispatch.rs"]
mod class_method_dispatch;
#[path = "classes/class_name_method_shadow.rs"]
mod class_name_method_shadow;
#[path = "classes/class_output_handle_copyback.rs"]
mod class_output_handle_copyback;
#[path = "classes/class_packed_and_type_params.rs"]
mod class_packed_and_type_params;
#[path = "classes/class_param_property_widths.rs"]
mod class_param_property_widths;
#[path = "classes/class_param_siblings.rs"]
mod class_param_siblings;
#[path = "classes/class_program_test.rs"]
mod class_program_test;
#[path = "classes/class_property_packed_selects.rs"]
mod class_property_packed_selects;
#[path = "classes/class_property_param_width.rs"]
mod class_property_param_width;
#[path = "classes/class_scoped_enum.rs"]
mod class_scoped_enum;
#[path = "classes/class_type_param_properties.rs"]
mod class_type_param_properties;
#[path = "classes/class_unpacked_struct_properties.rs"]
mod class_unpacked_struct_properties;
#[path = "classes/class_unpacked_struct_property.rs"]
mod class_unpacked_struct_property;
#[path = "classes/class_value_params.rs"]
mod class_value_params;
#[path = "classes/class_width_copy_fork.rs"]
mod class_width_copy_fork;
#[path = "classes/compiled_method_accessors.rs"]
mod compiled_method_accessors;
#[path = "classes/compiled_method_output_formals.rs"]
mod compiled_method_output_formals;
#[path = "classes/compiled_method_static_local_vars.rs"]
mod compiled_method_static_local_vars;
#[path = "classes/compiled_nonvirtual_dispatch.rs"]
mod compiled_nonvirtual_dispatch;
#[path = "classes/compiled_pkg_fn_scope.rs"]
mod compiled_pkg_fn_scope;
#[path = "classes/compiled_task_wait_analysis.rs"]
mod compiled_task_wait_analysis;
#[path = "classes/compiled_method_type_param_members.rs"]
mod compiled_method_type_param_members;
#[path = "classes/compiled_method_free_fn_caller_locals.rs"]
mod compiled_method_free_fn_caller_locals;
#[path = "classes/compiled_method_array_members.rs"]
mod compiled_method_array_members;
#[path = "classes/compiled_method_cast.rs"]
mod compiled_method_cast;
#[path = "classes/compiled_method_pcache.rs"]
mod compiled_method_pcache;
#[path = "classes/compiled_method_static_scope.rs"]
mod compiled_method_static_scope;
#[path = "classes/compiled_method_statics.rs"]
mod compiled_method_statics;
#[path = "classes/compiled_method_string_loop.rs"]
mod compiled_method_string_loop;
#[path = "classes/compiled_method_super.rs"]
mod compiled_method_super;
#[path = "classes/compiled_method_tiering.rs"]
mod compiled_method_tiering;
#[path = "classes/constraint_algebra_inherit.rs"]
mod constraint_algebra_inherit;
#[path = "classes/constraint_array_sum.rs"]
mod constraint_array_sum;
#[path = "classes/constraint_arrays_ordering.rs"]
mod constraint_arrays_ordering;
#[path = "classes/constraint_dyn_size_pinned_scalar.rs"]
mod constraint_dyn_size_pinned_scalar;
#[path = "classes/constraint_foreach_and_casts.rs"]
mod constraint_foreach_and_casts;
#[path = "classes/constraint_foreach_conditional.rs"]
mod constraint_foreach_conditional;
#[path = "classes/constraint_funcs_aggregates.rs"]
mod constraint_funcs_aggregates;
#[path = "classes/constraint_inline_caller_scope.rs"]
mod constraint_inline_caller_scope;
#[path = "classes/constraint_inline_enclosing_scope.rs"]
mod constraint_inline_enclosing_scope;
#[path = "classes/constraint_inside_rand_bound.rs"]
mod constraint_inside_rand_bound;
#[path = "classes/constraint_logical_or.rs"]
mod constraint_logical_or;
#[path = "classes/constraint_prefixed_inline_with.rs"]
mod constraint_prefixed_inline_with;
#[path = "classes/constraint_randc_soft_local.rs"]
mod constraint_randc_soft_local;
#[path = "classes/constraint_reversed_range.rs"]
mod constraint_reversed_range;
#[path = "classes/cov_covergroup_basic.rs"]
mod cov_covergroup_basic;
#[path = "classes/coverage_auto_bins.rs"]
mod coverage_auto_bins;
#[path = "classes/covergroup_at_least.rs"]
mod covergroup_at_least;
#[path = "classes/covergroup_bins.rs"]
mod covergroup_bins;
#[path = "classes/covergroup_coverage_query.rs"]
mod covergroup_coverage_query;
#[path = "classes/covergroup_cross_bins.rs"]
mod covergroup_cross_bins;
#[path = "classes/covergroup_illegal_bins.rs"]
mod covergroup_illegal_bins;
#[path = "classes/covergroup_implicit_names.rs"]
mod covergroup_implicit_names;
#[path = "classes/covergroup_options.rs"]
mod covergroup_options;
#[path = "classes/covergroup_placement.rs"]
mod covergroup_placement;
#[path = "classes/covergroup_queries.rs"]
mod covergroup_queries;
#[path = "classes/covergroup_sample_args.rs"]
mod covergroup_sample_args;
#[path = "classes/covergroup_sampling_event.rs"]
mod covergroup_sampling_event;
#[path = "classes/factory_run_test.rs"]
mod factory_run_test;
#[path = "classes/factory_typedef_chain.rs"]
mod factory_typedef_chain;
#[path = "classes/foreach_member_multidim.rs"]
mod foreach_member_multidim;
#[path = "classes/generate_and_class_parameters.rs"]
mod generate_and_class_parameters;
#[path = "classes/inherited_static_shared.rs"]
mod inherited_static_shared;
#[path = "classes/inspect_class.rs"]
mod inspect_class;
#[path = "classes/instance_class_comb_and_constraint_scope.rs"]
mod instance_class_comb_and_constraint_scope;
#[path = "classes/instance_struct_member_and_class_param.rs"]
mod instance_struct_member_and_class_param;
#[path = "classes/issue35_mixed_sign_constraints.rs"]
mod issue35_mixed_sign_constraints;
#[path = "classes/issue4_coupled_constraints.rs"]
mod issue4_coupled_constraints;
#[path = "classes/ivtest_class_struct_cluster.rs"]
mod ivtest_class_struct_cluster;
#[path = "classes/local_shadows_sig.rs"]
mod local_shadows_sig;
#[path = "classes/localparam_class_not_parameterized.rs"]
mod localparam_class_not_parameterized;
#[path = "classes/method_default_this_call.rs"]
mod method_default_this_call;
#[path = "classes/method_output_assoc_writeback.rs"]
mod method_output_assoc_writeback;
#[path = "classes/method_output_class_property_writeback.rs"]
mod method_output_class_property_writeback;
#[path = "classes/module_scope_derived_constraints.rs"]
mod module_scope_derived_constraints;
#[path = "classes/nba_region_event_same_slot.rs"]
mod nba_region_event_same_slot;
#[path = "classes/nd_array_properties_and_foreach_constraints.rs"]
mod nd_array_properties_and_foreach_constraints;
#[path = "classes/nested_seq_method_dispatch.rs"]
mod nested_seq_method_dispatch;
#[path = "classes/nonvirtual_dispatch_fscanf_process.rs"]
mod nonvirtual_dispatch_fscanf_process;
#[path = "classes/oor_packed_index_2state.rs"]
mod oor_packed_index_2state;
#[path = "classes/out_of_class_method_shadow.rs"]
mod out_of_class_method_shadow;
#[path = "classes/param_typedef_ctor_resolution.rs"]
mod param_typedef_ctor_resolution;
#[path = "classes/process_class_9_7.rs"]
mod process_class_9_7;
#[path = "classes/pure_sv_phase_objection.rs"]
mod pure_sv_phase_objection;
#[path = "classes/randomize_countones_count.rs"]
mod randomize_countones_count;
#[path = "classes/randomize_dist_weights.rs"]
mod randomize_dist_weights;
#[path = "classes/randomize_foreach_accept.rs"]
mod randomize_foreach_accept;
#[path = "classes/randomize_issues_295_297.rs"]
mod randomize_issues_295_297;
#[path = "classes/randomize_issues_298_299.rs"]
mod randomize_issues_298_299;
#[path = "classes/randomize_issue_302.rs"]
mod randomize_issue_302;
#[path = "classes/randomize_issues_303_305.rs"]
mod randomize_issues_303_305;
#[path = "classes/randomize_issue_301_wide2d.rs"]
mod randomize_issue_301_wide2d;
#[path = "classes/randomize_wide_sets.rs"]
mod randomize_wide_sets;
#[path = "classes/randomize_wide_open.rs"]
mod randomize_wide_open;
#[path = "classes/randomize_shift_mask_wide_peer.rs"]
mod randomize_shift_mask_wide_peer;
#[path = "classes/randomize_inside_range.rs"]
mod randomize_inside_range;
#[path = "classes/randomize_std_failed_restores_arrays.rs"]
mod randomize_std_failed_restores_arrays;
#[path = "classes/randomize_object_arrays.rs"]
mod randomize_object_arrays;
#[path = "classes/randomize_issue_255_full.rs"]
mod randomize_issue_255_full;
#[path = "classes/randomize_issue_255_shapes.rs"]
mod randomize_issue_255_shapes;
#[path = "classes/randomize_issue_261_wide.rs"]
mod randomize_issue_261_wide;
#[path = "classes/randomize_with_receiver_qualified_element.rs"]
mod randomize_with_receiver_qualified_element;
#[path = "classes/randomize_with_receiver_scope.rs"]
mod randomize_with_receiver_scope;
#[path = "classes/randomize_with_receiver_size.rs"]
mod randomize_with_receiver_size;
#[path = "classes/randomize_member_subset.rs"]
mod randomize_member_subset;
#[path = "classes/randomize_nonrand_members.rs"]
mod randomize_nonrand_members;
#[path = "classes/randomize_soft_solver_gave_up.rs"]
mod randomize_soft_solver_gave_up;
#[path = "classes/randomize_solve_before.rs"]
mod randomize_solve_before;
#[path = "classes/randomize_with_this_and_subset.rs"]
mod randomize_with_this_and_subset;
#[path = "classes/null_receiver_static_spec_544.rs"]
mod null_receiver_static_spec;
#[path = "classes/randomize_work_budget.rs"]
mod randomize_work_budget;
#[path = "classes/enum_string_valued_scope_546.rs"]
mod enum_string_valued_scope;
#[path = "classes/scope_randomize_dist_and_foreach.rs"]
mod scope_randomize_dist_and_foreach;
#[path = "classes/shadowed_property_storage.rs"]
mod shadowed_property_storage;
#[path = "classes/spec_key_class_spelling_542.rs"]
mod spec_key_class_spelling;
#[path = "classes/static_member_method_collision.rs"]
mod static_member_method_collision;
#[path = "classes/std_randomize_struct.rs"]
mod std_randomize_struct;
#[path = "classes/string_method_shadows_class_method.rs"]
mod string_method_shadows_class_method;
#[path = "classes/struct_formal_and_config_foreach.rs"]
mod struct_formal_and_config_foreach;
#[path = "classes/struct_output_formal.rs"]
mod struct_output_formal;
#[path = "classes/struct_output_inout_ref_formal.rs"]
mod struct_output_inout_ref_formal;
#[path = "classes/struct_with_class_handle.rs"]
mod struct_with_class_handle;
#[path = "classes/subroutine_local_unpacked_structs.rs"]
mod subroutine_local_unpacked_structs;
#[path = "classes/super_property_access.rs"]
mod super_property_access;
#[path = "classes/this_chain_edge_sensitivity.rs"]
mod this_chain_edge_sensitivity;
#[path = "classes/type_param_struct_formal.rs"]
mod type_param_struct_formal;
#[path = "classes/struct_formal_pattern_actual.rs"]
mod struct_formal_pattern_actual;
#[path = "classes/typedef_extends_cast.rs"]
mod typedef_extends_cast;
#[path = "classes/typedef_param_base_inherits_spec_arg.rs"]
mod typedef_param_base_inherits_spec_arg;
#[path = "classes/typename_param_class.rs"]
mod typename_param_class;

#[path = "classes/factory_vif_type_param_specialization.rs"]
mod factory_vif_type_param_specialization;

#[path = "classes/type_param_member_resolves_to_own_spec.rs"]
mod type_param_member_resolves_to_own_spec;

#[path = "classes/process_kill_via_member_handle.rs"]
mod process_kill_via_member_handle;

#[path = "classes/forward_declared_class_formal_member_write.rs"]
mod forward_declared_class_formal_member_write;

#[path = "classes/forward_class_formal_vs_caller_struct_collision.rs"]
mod forward_class_formal_vs_caller_struct_collision;

#[path = "classes/pkg_assoc_elem_read_neq.rs"]
mod pkg_assoc_elem_read_neq;
#[path = "classes/queue_elem_receiver_splice.rs"]
mod queue_elem_receiver_splice;
#[path = "classes/queue_eq_construction_path.rs"]
mod queue_eq_construction_path;
#[path = "classes/reg_field_byte_shift_stream_cast.rs"]
mod reg_field_byte_shift_stream_cast;
#[path = "classes/sibling_type_param_default_cast.rs"]
mod sibling_type_param_default_cast;
#[path = "classes/unpacked_struct_class_property_whole_value.rs"]
mod unpacked_struct_class_property_whole_value;
#[path = "classes/uvm_config_db_tests.rs"]
mod uvm_config_db_tests;
#[path = "classes/uvm_dpi_library.rs"]
mod uvm_dpi_library;
#[path = "classes/uvm_factory_linkage.rs"]
mod uvm_factory_linkage;
#[path = "classes/uvm_genuine_2017.rs"]
mod uvm_genuine_2017;
#[path = "classes/uvm_integration_tests.rs"]
mod uvm_integration_tests;
#[path = "classes/uvm_objection_bridge.rs"]
mod uvm_objection_bridge;
#[path = "classes/uvm_printer_fixes.rs"]
mod uvm_printer_fixes;
#[path = "classes/uvm_resource_db_raw_type_roundtrip.rs"]
mod uvm_resource_db_raw_type_roundtrip;
#[path = "classes/virtual_iface_this_binding.rs"]
mod virtual_iface_this_binding;

#[path = "classes/class_init_cast_copy.rs"]
mod class_init_cast_copy;

#[path = "classes/fixed_member_pattern_write.rs"]
mod fixed_member_pattern_write;

#[path = "classes/nested_and_extends_spec.rs"]
mod nested_and_extends_spec;

#[path = "classes/blocking_task_on_call_result.rs"]
mod blocking_task_on_call_result;
#[path = "classes/class_property_over_module_signal.rs"]
mod class_property_over_module_signal;
#[path = "classes/concurrent_method_local_arrays.rs"]
mod concurrent_method_local_arrays;
#[path = "classes/concurrent_method_local_queues.rs"]
mod concurrent_method_local_queues;
#[path = "classes/module_auto_task_local_array.rs"]
mod module_auto_task_local_array;
#[path = "classes/default_automatic_task_locals.rs"]
mod default_automatic_task_locals;
#[path = "classes/method_output_multidim_and_chain_property.rs"]
mod method_output_multidim_and_chain_property;
#[path = "classes/instance_class_handles_start_null.rs"]
mod instance_class_handles_start_null;
#[path = "classes/interface_array_handle_new.rs"]
mod interface_array_handle_new;
#[path = "classes/nested_rand_inline_constraints.rs"]
mod nested_rand_inline_constraints;
#[path = "classes/nested_rand_joint_solve.rs"]
mod nested_rand_joint_solve;
#[path = "classes/cast_defaulted_extends_type_argument.rs"]
mod cast_defaulted_extends_type_argument;
#[path = "classes/null_deref_fatal.rs"]
mod null_deref_fatal;
#[path = "classes/shadowed_property_initializers.rs"]
mod shadowed_property_initializers;
#[path = "classes/std_randomize_object_member.rs"]
mod std_randomize_object_member;
#[path = "classes/two_state_property_writes.rs"]
mod two_state_property_writes;
#[path = "classes/typedef_param_class_construction.rs"]
mod typedef_param_class_construction;
#[path = "classes/typename_class_operands.rs"]
mod typename_class_operands;

#[path = "classes/ctor_dispatch_and_p_format.rs"]
mod ctor_dispatch_and_p_format;

#[path = "classes/assoc_enum_key_class_property.rs"]
mod assoc_enum_key_class_property;

#[path = "classes/array_of_collections_property.rs"]
mod array_of_collections_property;
#[path = "classes/class_scoped_param_dimension.rs"]
mod class_scoped_param_dimension;
#[path = "classes/array_query_handle_qualified_member.rs"]
mod array_query_handle_qualified_member;
#[path = "classes/assoc_wide_packed_struct_key.rs"]
mod assoc_wide_packed_struct_key;
#[path = "classes/caller_local_shadows_this_cast.rs"]
mod caller_local_shadows_this_cast;
#[path = "classes/cast_dest_specialization.rs"]
mod cast_dest_specialization;
#[path = "classes/class_array_property_select.rs"]
mod class_array_property_select;
#[path = "classes/class_covergroups.rs"]
mod class_covergroups;
#[path = "classes/class_localparam_array.rs"]
mod class_localparam_array;
#[path = "classes/class_method_sibling_instance_oomr.rs"]
mod class_method_sibling_instance_oomr;
#[path = "classes/class_randomize_multidim.rs"]
mod class_randomize_multidim;
#[path = "classes/cls_agg_members_in_struct.rs"]
mod cls_agg_members_in_struct;
#[path = "classes/cls_agg_members_matrix.rs"]
mod cls_agg_members_matrix;
#[path = "classes/class_scope_call_implicit_this.rs"]
mod class_scope_call_implicit_this;
#[path = "classes/class_scope_specialization_members.rs"]
mod class_scope_specialization_members;
#[path = "classes/compiled_task_suspension_analysis.rs"]
mod compiled_task_suspension_analysis;
#[path = "classes/compiled_method_admission_regression.rs"]
mod compiled_method_admission_regression;
#[path = "classes/compiled_task_and_free_fn_chain.rs"]
mod compiled_task_and_free_fn_chain;
#[path = "classes/compiled_collection_return_store.rs"]
mod compiled_collection_return_store;
#[path = "classes/compiled_string_local_byte.rs"]
mod compiled_string_local_byte;
#[path = "classes/compiled_string_formal_byte.rs"]
mod compiled_string_formal_byte;
#[path = "classes/compiled_member_redeclared.rs"]
mod compiled_member_redeclared;
#[path = "classes/compiled_string_member_shadow_store.rs"]
mod compiled_string_member_shadow_store;
#[path = "classes/compiled_member_operand_width.rs"]
mod compiled_member_operand_width;

#[path = "classes/compiled_member_operand_signedness.rs"]
mod compiled_member_operand_signedness;

#[path = "classes/compiled_unpacked_formal.rs"]
mod compiled_unpacked_formal;

#[path = "classes/compiled_bare_member_store_width.rs"]
mod compiled_bare_member_store_width;

#[path = "classes/compiled_short_circuit.rs"]
mod compiled_short_circuit;
#[path = "classes/stale_enum_local_name.rs"]
mod stale_enum_local_name;
#[path = "classes/compiled_method_stmt_fallback.rs"]
mod compiled_method_stmt_fallback;
#[path = "classes/compiled_method_handle_carry.rs"]
mod compiled_method_handle_carry;
#[path = "classes/compiled_fallback_member_named_local.rs"]
mod compiled_fallback_member_named_local;
#[path = "classes/compiled_type_param_formals.rs"]
mod compiled_type_param_formals;
#[path = "classes/compiled_coll_elem_signed_compare.rs"]
mod compiled_coll_elem_signed_compare;
#[path = "classes/compiled_foreach_handle_member.rs"]
mod compiled_foreach_handle_member;

#[path = "classes/compiled_string_member_methods.rs"]
mod compiled_string_member_methods;

#[path = "classes/compiled_coll_elem_store_width.rs"]
mod compiled_coll_elem_store_width;
#[path = "classes/compiled_ctor_decl_init.rs"]
mod compiled_ctor_decl_init;

#[path = "classes/compiled_fallback_nested_handle.rs"]
mod compiled_fallback_nested_handle;

#[path = "classes/int_property_named_like_struct_property.rs"]
mod int_property_named_like_struct_property;
#[path = "classes/compiled_method_test_env.rs"]
mod compiled_method_test_env;
#[path = "classes/concurrent_function_struct_local_shadow.rs"]
mod concurrent_function_struct_local_shadow;
#[path = "classes/condition_waiter_yields_to_inactive.rs"]
mod condition_waiter_yields_to_inactive;
#[path = "classes/constraint_shift_bounds.rs"]
mod constraint_shift_bounds;
#[path = "classes/covergroup_bin_arithmetic.rs"]
mod covergroup_bin_arithmetic;
#[path = "classes/dyn_array_struct_copy.rs"]
mod dyn_array_struct_copy;
#[path = "classes/enum_local_shadows_flat_maps.rs"]
mod enum_local_shadows_flat_maps;
#[path = "classes/field_init_once_in_order.rs"]
mod field_init_once_in_order;
#[path = "classes/formal_class_type_frame_scoped.rs"]
mod formal_class_type_frame_scoped;
#[path = "classes/handle_chain_read_in_instance_task.rs"]
mod handle_chain_read_in_instance_task;
#[path = "classes/heap_id_collision_member_access.rs"]
mod heap_id_collision_member_access;
#[path = "classes/iff_guard_parked_context.rs"]
mod iff_guard_parked_context;
#[path = "classes/id_collision_in_heap.rs"]
mod id_collision_in_heap;
#[path = "classes/implication_joint_distribution.rs"]
mod implication_joint_distribution;
#[path = "classes/interface_class_extends_type_param.rs"]
mod interface_class_extends_type_param;
#[path = "classes/issue_246_struct_member_randomize.rs"]
mod issue_246_struct_member_randomize;
#[path = "classes/issue_249_rand_mode_receiver.rs"]
mod issue_249_rand_mode_receiver;
#[path = "classes/rand_mode_element_769.rs"]
mod rand_mode_element_769;
#[path = "classes/dist_zero_weights_770.rs"]
mod dist_zero_weights_770;
#[path = "classes/class_param_sized_formals_768.rs"]
mod class_param_sized_formals_768;
#[path = "classes/lrm_chained_handle_writes.rs"]
mod lrm_chained_handle_writes;
#[path = "classes/lrm_class_queue_locator_in_expr.rs"]
mod lrm_class_queue_locator_in_expr;
#[path = "classes/lrm_param_sized_array_member.rs"]
mod lrm_param_sized_array_member;
#[path = "classes/lrm_property_vs_struct_member_name.rs"]
mod lrm_property_vs_struct_member_name;
#[path = "classes/lrm_shadowed_property_static_type.rs"]
mod lrm_shadowed_property_static_type;
#[path = "classes/lrm_signed_class_collection_elements.rs"]
mod lrm_signed_class_collection_elements;
#[path = "classes/lrm_static_method_via_handle.rs"]
mod lrm_static_method_via_handle;
#[path = "classes/member_collection_runtime_class.rs"]
mod member_collection_runtime_class;
#[path = "classes/member_visibility_local_protected.rs"]
mod member_visibility_local_protected;
#[path = "classes/method_call_plans.rs"]
mod method_call_plans;
#[path = "classes/method_int_formal_zero_extends.rs"]
mod method_int_formal_zero_extends;
#[path = "classes/method_local_base_per_process.rs"]
mod method_local_base_per_process;
#[path = "classes/module_scope_handle_access.rs"]
mod module_scope_handle_access;
#[path = "classes/named_base_forward_matrix.rs"]
mod named_base_forward_matrix;
#[path = "classes/nopack_member_array.rs"]
mod nopack_member_array;
#[path = "classes/package_class_nested_class.rs"]
mod package_class_nested_class;
#[path = "classes/parked_task_local_vif_alias.rs"]
mod parked_task_local_vif_alias;
#[path = "classes/pkg_class_scope_types.rs"]
mod pkg_class_scope_types;
#[path = "classes/rand_collection_element_signedness.rs"]
mod rand_collection_element_signedness;
#[path = "classes/rand_width_overflow.rs"]
mod rand_width_overflow;
#[path = "classes/randomize_joint_constraints.rs"]
mod randomize_joint_constraints;
#[path = "classes/randomize_obj_array_property.rs"]
mod randomize_obj_array_property;
#[path = "classes/receiver_call_evaluated_once.rs"]
mod receiver_call_evaluated_once;
#[path = "classes/ref_formal_shadows_property.rs"]
mod ref_formal_shadows_property;
#[path = "classes/soft_constraint_compatibility.rs"]
mod soft_constraint_compatibility;
#[path = "classes/soft_constraint_matrix.rs"]
mod soft_constraint_matrix;
#[path = "classes/sqr_zero_time_loop.rs"]
mod sqr_zero_time_loop;
#[path = "classes/static_assoc_struct_pool.rs"]
mod static_assoc_struct_pool;
#[path = "classes/static_collection_qualified_access.rs"]
mod static_collection_qualified_access;
#[path = "classes/static_instance_assoc_object.rs"]
mod static_instance_assoc_object;
#[path = "classes/static_property_through_handle.rs"]
mod static_property_through_handle;
#[path = "classes/struct_array_field_read.rs"]
mod struct_array_field_read;
#[path = "classes/struct_prop_whole_copy.rs"]
mod struct_prop_whole_copy;
#[path = "classes/type_param_inherited_member_write.rs"]
mod type_param_inherited_member_write;
#[path = "classes/type_param_prop_member_write.rs"]
mod type_param_prop_member_write;
#[path = "classes/type_param_replication_localparam.rs"]
mod type_param_replication_localparam;
#[path = "classes/typedef_receiver_static_task.rs"]
mod typedef_receiver_static_task;
#[path = "classes/uvm_feature_coverage.rs"]
mod uvm_feature_coverage;
#[path = "classes/uvm_ral_coverage.rs"]
mod uvm_ral_coverage;
#[path = "classes/uvm_tlm_coverage.rs"]
mod uvm_tlm_coverage;
#[path = "classes/vif_member_read_nested_receiver.rs"]
mod vif_member_read_nested_receiver;
#[path = "classes/vif_null_compare_this_receiver.rs"]
mod vif_null_compare_this_receiver;
#[path = "classes/vif_clocking_bare_event.rs"]
mod vif_clocking_bare_event;
#[path = "classes/vif_inherited_type_param.rs"]
mod vif_inherited_type_param;
#[path = "classes/class_event_bare_triggered.rs"]
mod class_event_bare_triggered;
#[path = "classes/class_event_forever_wait.rs"]
mod class_event_forever_wait;
#[path = "classes/string_local_name_leak.rs"]
mod string_local_name_leak;
#[path = "classes/vif_null_compare_indexed_receiver.rs"]
mod vif_null_compare_indexed_receiver;
#[path = "classes/vif_return_own_definition_name.rs"]
mod vif_return_own_definition_name;
#[path = "classes/vif_property_named_like_instance.rs"]
mod vif_property_named_like_instance;
#[path = "classes/vif_receiver_neighbor_shapes.rs"]
mod vif_receiver_neighbor_shapes;
#[path = "classes/vif_receiver_operation_matrix.rs"]
mod vif_receiver_operation_matrix;
#[path = "classes/vif_resource_db_shape.rs"]
mod vif_resource_db_shape;
#[path = "classes/vif_static_roundtrip.rs"]
mod vif_static_roundtrip;
#[path = "classes/class_member_named_like_instance.rs"]
mod class_member_named_like_instance;
#[path = "classes/covergroup_class_param_bins.rs"]
mod covergroup_class_param_bins;
#[path = "classes/class_method_scalar_formal_width.rs"]
mod class_method_scalar_formal_width;
#[path = "classes/typedef_packed_return_prototype.rs"]
mod typedef_packed_return_prototype;
#[path = "classes/inherited_type_param_create.rs"]
mod inherited_type_param_create;
#[path = "classes/dynamic_array_property_new_init.rs"]
mod dynamic_array_property_new_init;
#[path = "classes/type_param_handle_not_struct.rs"]
mod type_param_handle_not_struct;
#[path = "classes/compiled_method_packed_elem_select.rs"]
mod compiled_method_packed_elem_select;

#[path = "classes/class_packed_elem_store.rs"]
mod class_packed_elem_store;

#[path = "classes/nonstatic_method_class_scope_fatal.rs"]
mod nonstatic_method_class_scope_fatal;

#[path = "classes/struct_class_prop_patterns.rs"]
mod struct_class_prop_patterns;
#[path = "classes/class_in_module_instance.rs"]
mod class_in_module_instance;
#[path = "classes/static_task_bare_call.rs"]
mod static_task_bare_call;
#[path = "classes/static_class_event.rs"]
mod static_class_event;
#[path = "classes/static_new_initializer_once.rs"]
mod static_new_initializer_once;
#[path = "classes/static_actual_caller_spec.rs"]
mod static_actual_caller_spec;
#[path = "classes/member_shadows_package_var.rs"]
mod member_shadows_package_var;

#[path = "classes/parameterized_static_constructor.rs"]
mod parameterized_static_constructor;

#[path = "classes/dynamic_outer_assoc.rs"]
mod dynamic_outer_assoc;

#[path = "classes/constraint_inside_cast_union.rs"]
mod constraint_inside_cast_union;

#[path = "classes/string_frame_teardown.rs"]
mod string_frame_teardown;

#[path = "classes/property_seed_order.rs"]
mod property_seed_order;

#[path = "classes/static_select_346.rs"]
mod static_select_346;

#[path = "classes/method_types_378.rs"]
mod next_rest_method_types_378;

#[path = "classes/randomize_hook_signatures.rs"]
mod randomize_hook_signatures;

#[path = "classes/static_class_members_382.rs"]
mod static_class_members_382;

#[path = "classes/foreach_array_of_collections_elem.rs"]
mod foreach_array_of_collections_elem;

#[path = "classes/class_assignment_types_499.rs"]
mod class_assignment_types_499;

#[path = "classes/class_param_array_reads_503.rs"]
mod class_param_array_reads_503;

#[path = "classes/randomize_std_static_subroutine_locals_513.rs"]
mod randomize_std_static_subroutine_locals;

#[path = "classes/class_field_dependency_order.rs"]
mod class_field_dependency_order;
#[path = "classes/hier_class_member_access.rs"]
mod hier_class_member_access;

#[path = "classes/review_array_constants.rs"]
mod review_array_constants;

#[path = "classes/review_randomize_rollback.rs"]
mod review_randomize_rollback;
#[path = "classes/bare_self_reference_typedef_spec.rs"]
mod bare_self_reference_typedef_spec;

#[path = "classes/compiled_call_actual_width.rs"]
mod compiled_call_actual_width;

#[path = "classes/null_handle_method_call_532.rs"]
mod null_handle_method_call_532;

#[path = "classes/alias_spec_array_const_535.rs"]
mod alias_spec_array_const_535;

#[path = "classes/local_shadows_class_array_const_534.rs"]
mod local_shadows_class_array_const_534;

#[path = "classes/class_lparam_chain_76.rs"]
mod class_lparam_chain_76;

#[path = "classes/extern_unsigned_method_scope_541.rs"]
mod extern_unsigned_method_scope_541;

#[path = "classes/class_handle_operand_width_564.rs"]
mod class_handle_operand_width_564;

#[path = "classes/subroutine_lhs_operand_width_565.rs"]
mod subroutine_lhs_operand_width_565;

#[path = "classes/nba_member_target_object_569.rs"]
mod nba_member_target_object_569;

#[path = "classes/randomize_nd_arrays_519.rs"]
mod randomize_nd_arrays_519;

#[path = "classes/method_array_write_shadows_module_array.rs"]
mod method_array_write_shadows_module_array;

#[path = "classes/constructor_default_args_2023.rs"]
mod constructor_default_args_2023;
#[path = "classes/randomize_type_param_576.rs"]
mod randomize_type_param_576;
#[path = "classes/nested_dyn_members.rs"]
mod nested_dyn_members;
#[path = "classes/randomize_fill_literal.rs"]
mod randomize_fill_literal;
#[path = "classes/randomize_nested_dyn_scope.rs"]
mod randomize_nested_dyn_scope;
#[path = "classes/shadowed_member_same_named_formal.rs"]
mod shadowed_member_same_named_formal;
#[path = "classes/member_multidim_copy.rs"]
mod member_multidim_copy;
#[path = "classes/packed_struct_pattern_class_vif_dest_587.rs"]
mod packed_struct_pattern_class_vif_dest;
#[path = "classes/rand_wide_elem_draw_sum_588.rs"]
mod rand_wide_elem_draw_sum;
#[path = "classes/member_new_owner_spec_596.rs"]
mod member_new_owner_spec_596;
#[path = "classes/super_call_formal_widths_593.rs"]
mod super_call_formal_widths_593;
#[path = "classes/randomize_foreach_empty_dyn_599.rs"]
mod randomize_foreach_empty_dyn_599;
#[path = "classes/nested_class_out_of_block_617.rs"]
mod nested_class_out_of_block_617;
#[path = "classes/nba_member_root_613.rs"]
mod nba_member_root_613;
#[path = "classes/relax_static_out_of_block_621.rs"]
mod relax_static_out_of_block_621;
#[path = "classes/class_event_array_element.rs"]
mod class_event_array_element;
#[path = "classes/local_event_wait.rs"]
mod local_event_wait;
#[path = "classes/static_spec_actual_lexical.rs"]
mod static_spec_actual_lexical;
#[path = "classes/virtual_named_class_param.rs"]
mod virtual_named_class_param;
#[path = "classes/vif_type_param_module_var.rs"]
mod vif_type_param_module_var;
#[path = "classes/queue_last_member.rs"]
mod queue_last_member;
#[path = "classes/package_constructor_scope.rs"]
mod package_constructor_scope;
#[path = "classes/class_ref_signature.rs"]
mod class_ref_signature;
#[path = "classes/package_class_constraint.rs"]
mod package_class_constraint;

#[path = "classes/nonrand_handle_constraint_state.rs"]
mod nonrand_handle_constraint_state;

#[path = "classes/conditional_enum_exclusions.rs"]
mod conditional_enum_exclusions;

#[path = "classes/default_argument_width.rs"]
mod default_argument_width;

#[path = "classes/inline_collection_caller_handle.rs"]
mod inline_collection_caller_handle;

#[path = "classes/enum_collection_domains.rs"]
mod enum_collection_domains;

#[path = "classes/state_array_size_bound.rs"]
mod state_array_size_bound;

#[path = "classes/argument_binding_selected_and_output.rs"]
mod argument_binding_selected_and_output;

#[path = "classes/compiled_method_scope_and_member_stores.rs"]
mod compiled_method_scope_and_member_stores;

#[path = "classes/randomize_no_parentheses.rs"]
mod randomize_no_parentheses;

#[path = "classes/inside_shift_width.rs"]
mod inside_shift_width;

#[path = "classes/random_mode_reenable.rs"]
mod random_mode_reenable;

#[path = "classes/parenthesized_collection_return.rs"]
mod parenthesized_collection_return;

#[path = "classes/class_concat_unpack.rs"]
mod class_concat_unpack;

#[path = "classes/compiled_member_store_carry.rs"]
mod compiled_member_store_carry;
#[path = "classes/review_pr720_followups.rs"]
mod review_pr720_followups;
#[path = "classes/class_param_local_width_681.rs"]
mod class_param_local_width_681;
#[path = "classes/randc_wide_699.rs"]
mod randc_wide_699;

#[path = "classes/class_string_queue_concatenation.rs"]
mod class_string_queue_concatenation;

#[path = "classes/class_queue_empty_pattern_through_handle.rs"]
mod class_queue_empty_pattern_through_handle;

#[path = "classes/package_type_identity_roundtrip.rs"]
mod package_type_identity_roundtrip;

#[path = "classes/review_collection_pattern_gaps.rs"]
mod review_collection_pattern_gaps;
