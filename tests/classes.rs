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
#[path = "classes/blocking_task_super_dispatch.rs"]
mod blocking_task_super_dispatch;
#[path = "classes/class_assoc_struct_string_keys.rs"]
mod class_assoc_struct_string_keys;
#[path = "classes/class_formal_typedef_widen.rs"]
mod class_formal_typedef_widen;
#[path = "classes/class_interface_same_name.rs"]
mod class_interface_same_name;
#[path = "classes/class_member_name_clash.rs"]
mod class_member_name_clash;
#[path = "classes/class_param_default_scope.rs"]
mod class_param_default_scope;
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
#[path = "classes/explicit_param_static_coll_read.rs"]
mod explicit_param_static_coll_read;
#[path = "classes/function_local_struct_return_shadow.rs"]
mod function_local_struct_return_shadow;
#[path = "classes/implements_typedef_scope.rs"]
mod implements_typedef_scope;
#[path = "classes/inherited_local_member.rs"]
mod inherited_local_member;
#[path = "classes/interface_class_inherited_names.rs"]
mod interface_class_inherited_names;
#[path = "classes/memory_tasks_fixed_property.rs"]
mod memory_tasks_fixed_property;
#[path = "classes/nested_same_named_ref_assoc_formal.rs"]
mod nested_same_named_ref_assoc_formal;
#[path = "classes/param_type_binding_resolves_enclosing_value_param.rs"]
mod param_type_binding_resolves_enclosing_value_param;
#[path = "classes/parameterized_class_scope.rs"]
mod parameterized_class_scope;
#[path = "classes/pure_constraint_implemented.rs"]
mod pure_constraint_implemented;
#[path = "classes/rand_array_elem_signedness.rs"]
mod rand_array_elem_signedness;
#[path = "classes/rand_obj_array_elem_constraints.rs"]
mod rand_obj_array_elem_constraints;
#[path = "classes/randomize_args_name_object_members.rs"]
mod randomize_args_name_object_members;
#[path = "classes/static_fixed_array_storage.rs"]
mod static_fixed_array_storage;
#[path = "classes/static_param_class_collection_reuse.rs"]
mod static_param_class_collection_reuse;
#[path = "classes/struct_member_class_handle_new.rs"]
mod struct_member_class_handle_new;
#[path = "classes/this_collection_struct_members.rs"]
mod this_collection_struct_members;
#[path = "classes/this_super_member.rs"]
mod this_super_member;
#[path = "classes/type_param_base_class.rs"]
mod type_param_base_class;
#[path = "classes/type_param_base_named_forward.rs"]
mod type_param_base_named_forward;
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

#[path = "classes/bare_parameterless_method_call.rs"]
mod bare_parameterless_method_call;
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
#[path = "classes/localparam_class_not_parameterized.rs"]
mod localparam_class_not_parameterized;
#[path = "classes/method_default_this_call.rs"]
mod method_default_this_call;
#[path = "classes/module_scope_derived_constraints.rs"]
mod module_scope_derived_constraints;
#[path = "classes/nd_array_properties_and_foreach_constraints.rs"]
mod nd_array_properties_and_foreach_constraints;
#[path = "classes/nested_seq_method_dispatch.rs"]
mod nested_seq_method_dispatch;
#[path = "classes/nonvirtual_dispatch_fscanf_process.rs"]
mod nonvirtual_dispatch_fscanf_process;
#[path = "classes/out_of_class_method_shadow.rs"]
mod out_of_class_method_shadow;
#[path = "classes/param_typedef_ctor_resolution.rs"]
mod param_typedef_ctor_resolution;
#[path = "classes/process_class_9_7.rs"]
mod process_class_9_7;
#[path = "classes/pure_sv_phase_objection.rs"]
mod pure_sv_phase_objection;
#[path = "classes/randomize_dist_weights.rs"]
mod randomize_dist_weights;
#[path = "classes/randomize_inside_range.rs"]
mod randomize_inside_range;
#[path = "classes/randomize_member_subset.rs"]
mod randomize_member_subset;
#[path = "classes/randomize_nonrand_members.rs"]
mod randomize_nonrand_members;
#[path = "classes/randomize_solve_before.rs"]
mod randomize_solve_before;
#[path = "classes/randomize_with_this_and_subset.rs"]
mod randomize_with_this_and_subset;
#[path = "classes/randomize_work_budget.rs"]
mod randomize_work_budget;
#[path = "classes/scope_randomize_dist_and_foreach.rs"]
mod scope_randomize_dist_and_foreach;
#[path = "classes/shadowed_property_storage.rs"]
mod shadowed_property_storage;
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
#[path = "classes/typedef_extends_cast.rs"]
mod typedef_extends_cast;
#[path = "classes/typedef_param_base_inherits_spec_arg.rs"]
mod typedef_param_base_inherits_spec_arg;
#[path = "classes/typename_param_class.rs"]
mod typename_param_class;
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
#[path = "classes/instance_class_handles_start_null.rs"]
mod instance_class_handles_start_null;
#[path = "classes/interface_array_handle_new.rs"]
mod interface_array_handle_new;
#[path = "classes/nested_rand_inline_constraints.rs"]
mod nested_rand_inline_constraints;
#[path = "classes/nested_rand_joint_solve.rs"]
mod nested_rand_joint_solve;
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
#[path = "classes/array_query_handle_qualified_member.rs"]
mod array_query_handle_qualified_member;
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
#[path = "classes/compiled_method_admission_regression.rs"]
mod compiled_method_admission_regression;
#[path = "classes/compiled_method_test_env.rs"]
mod compiled_method_test_env;
#[path = "classes/condition_waiter_yields_to_inactive.rs"]
mod condition_waiter_yields_to_inactive;
#[path = "classes/covergroup_bin_arithmetic.rs"]
mod covergroup_bin_arithmetic;
#[path = "classes/enum_local_shadows_flat_maps.rs"]
mod enum_local_shadows_flat_maps;
#[path = "classes/field_init_once_in_order.rs"]
mod field_init_once_in_order;
#[path = "classes/handle_chain_read_in_instance_task.rs"]
mod handle_chain_read_in_instance_task;
#[path = "classes/heap_id_collision_member_access.rs"]
mod heap_id_collision_member_access;
#[path = "classes/id_collision_in_heap.rs"]
mod id_collision_in_heap;
#[path = "classes/implication_joint_distribution.rs"]
mod implication_joint_distribution;
#[path = "classes/interface_class_extends_type_param.rs"]
mod interface_class_extends_type_param;
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
#[path = "classes/rand_collection_element_signedness.rs"]
mod rand_collection_element_signedness;
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
#[path = "classes/struct_prop_whole_copy.rs"]
mod struct_prop_whole_copy;
#[path = "classes/type_param_replication_localparam.rs"]
mod type_param_replication_localparam;
#[path = "classes/typedef_receiver_static_task.rs"]
mod typedef_receiver_static_task;
#[path = "classes/vif_member_read_nested_receiver.rs"]
mod vif_member_read_nested_receiver;
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
