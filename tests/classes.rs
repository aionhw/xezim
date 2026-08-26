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

#[path = "classes/collection_of_handles_new.rs"]
mod collection_of_handles_new;
#[path = "classes/static_fixed_array_storage.rs"]
mod static_fixed_array_storage;
#[path = "classes/array_equality_class.rs"]
mod array_equality_class;
#[path = "classes/class_formal_typedef_widen.rs"]
mod class_formal_typedef_widen;
#[path = "classes/assoc_typedef_element_class.rs"]
mod assoc_typedef_element_class;
#[path = "classes/bit_class_property_signedness.rs"]
mod bit_class_property_signedness;
#[path = "classes/blocking_task_super_dispatch.rs"]
mod blocking_task_super_dispatch;
#[path = "classes/class_field_named_event.rs"]
mod class_field_named_event;
#[path = "classes/this_chain_edge_sensitivity.rs"]
mod this_chain_edge_sensitivity;
#[path = "classes/class_handle_return_preservation.rs"]
mod class_handle_return_preservation;
#[path = "classes/class_output_handle_copyback.rs"]
mod class_output_handle_copyback;
#[path = "classes/typedef_extends_cast.rs"]
mod typedef_extends_cast;
#[path = "classes/class_local_typedef_aa.rs"]
mod class_local_typedef_aa;
#[path = "classes/class_local_typedef_resolution.rs"]
mod class_local_typedef_resolution;
#[path = "classes/class_method_dispatch.rs"]
mod class_method_dispatch;
#[path = "classes/method_default_this_call.rs"]
mod method_default_this_call;
#[path = "classes/class_name_method_shadow.rs"]
mod class_name_method_shadow;
#[path = "classes/class_packed_and_type_params.rs"]
mod class_packed_and_type_params;
#[path = "classes/class_param_siblings.rs"]
mod class_param_siblings;
#[path = "classes/class_program_test.rs"]
mod class_program_test;
#[path = "classes/class_property_param_width.rs"]
mod class_property_param_width;
#[path = "classes/class_scoped_enum.rs"]
mod class_scoped_enum;
#[path = "classes/class_type_param_properties.rs"]
mod class_type_param_properties;
#[path = "classes/class_value_params.rs"]
mod class_value_params;
#[path = "classes/class_width_copy_fork.rs"]
mod class_width_copy_fork;
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
#[path = "classes/constraint_funcs_aggregates.rs"]
mod constraint_funcs_aggregates;
#[path = "classes/constraint_inline_enclosing_scope.rs"]
mod constraint_inline_enclosing_scope;
#[path = "classes/constraint_logical_or.rs"]
mod constraint_logical_or;
#[path = "classes/constraint_prefixed_inline_with.rs"]
mod constraint_prefixed_inline_with;
#[path = "classes/constraint_randc_soft_local.rs"]
mod constraint_randc_soft_local;
#[path = "classes/cov_covergroup_basic.rs"]
mod cov_covergroup_basic;
#[path = "classes/coverage_auto_bins.rs"]
mod coverage_auto_bins;
#[path = "classes/covergroup_coverage_query.rs"]
mod covergroup_coverage_query;
#[path = "classes/factory_run_test.rs"]
mod factory_run_test;
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
#[path = "classes/nonvirtual_dispatch_fscanf_process.rs"]
mod nonvirtual_dispatch_fscanf_process;
#[path = "classes/out_of_class_method_shadow.rs"]
mod out_of_class_method_shadow;
#[path = "classes/param_typedef_ctor_resolution.rs"]
mod param_typedef_ctor_resolution;
#[path = "classes/process_class_9_7.rs"]
mod process_class_9_7;
#[path = "classes/randomize_inside_range.rs"]
mod randomize_inside_range;
#[path = "classes/scope_randomize_dist_and_foreach.rs"]
mod scope_randomize_dist_and_foreach;
#[path = "classes/class_unpacked_struct_properties.rs"]
mod class_unpacked_struct_properties;
#[path = "classes/nd_array_properties_and_foreach_constraints.rs"]
mod nd_array_properties_and_foreach_constraints;
#[path = "classes/randomize_member_subset.rs"]
mod randomize_member_subset;
#[path = "classes/class_property_packed_selects.rs"]
mod class_property_packed_selects;
#[path = "classes/module_scope_derived_constraints.rs"]
mod module_scope_derived_constraints;
#[path = "classes/shadowed_property_storage.rs"]
mod shadowed_property_storage;
#[path = "classes/super_property_access.rs"]
mod super_property_access;
#[path = "classes/subroutine_local_unpacked_structs.rs"]
mod subroutine_local_unpacked_structs;
#[path = "classes/std_randomize_struct.rs"]
mod std_randomize_struct;
#[path = "classes/string_method_shadows_class_method.rs"]
mod string_method_shadows_class_method;
#[path = "classes/struct_with_class_handle.rs"]
mod struct_with_class_handle;
#[path = "classes/struct_output_inout_ref_formal.rs"]
mod struct_output_inout_ref_formal;
#[path = "classes/type_param_struct_formal.rs"]
mod type_param_struct_formal;
#[path = "classes/struct_formal_and_config_foreach.rs"]
mod struct_formal_and_config_foreach;
#[path = "classes/typename_param_class.rs"]
mod typename_param_class;
#[path = "classes/unpacked_struct_class_property_whole_value.rs"]
mod unpacked_struct_class_property_whole_value;
#[path = "classes/uvm_config_db_tests.rs"]
mod uvm_config_db_tests;
#[path = "classes/uvm_factory_linkage.rs"]
mod uvm_factory_linkage;
#[path = "classes/uvm_genuine_2017.rs"]
mod uvm_genuine_2017;
#[path = "classes/uvm_integration_tests.rs"]
mod uvm_integration_tests;
#[path = "classes/pure_sv_phase_objection.rs"]
mod pure_sv_phase_objection;
#[path = "classes/uvm_objection_bridge.rs"]
mod uvm_objection_bridge;
#[path = "classes/uvm_printer_fixes.rs"]
mod uvm_printer_fixes;
#[path = "classes/virtual_iface_this_binding.rs"]
mod virtual_iface_this_binding;
#[path = "classes/class_unpacked_struct_property.rs"]
mod class_unpacked_struct_property;
#[path = "classes/struct_output_formal.rs"]
mod struct_output_formal;

#[path = "classes/class_init_cast_copy.rs"]
mod class_init_cast_copy;

#[path = "classes/fixed_member_pattern_write.rs"]
mod fixed_member_pattern_write;

#[path = "classes/nested_and_extends_spec.rs"]
mod nested_and_extends_spec;

#[path = "classes/null_deref_fatal.rs"]
mod null_deref_fatal;

#[path = "classes/ctor_dispatch_and_p_format.rs"]
mod ctor_dispatch_and_p_format;

#[path = "classes/assoc_enum_key_class_property.rs"]
mod assoc_enum_key_class_property;

#[path = "classes/module_scope_handle_access.rs"]
mod module_scope_handle_access;
#[path = "classes/struct_prop_whole_copy.rs"]
mod struct_prop_whole_copy;
#[path = "classes/randomize_obj_array_property.rs"]
mod randomize_obj_array_property;
#[path = "classes/static_assoc_struct_pool.rs"]
mod static_assoc_struct_pool;
#[path = "classes/vif_static_roundtrip.rs"]
mod vif_static_roundtrip;
#[path = "classes/vif_property_named_like_instance.rs"]
mod vif_property_named_like_instance;
#[path = "classes/nopack_member_array.rs"]
mod nopack_member_array;
#[path = "classes/class_localparam_array.rs"]
mod class_localparam_array;
#[path = "classes/member_collection_runtime_class.rs"]
mod member_collection_runtime_class;
#[path = "classes/enum_local_shadows_flat_maps.rs"]
mod enum_local_shadows_flat_maps;
