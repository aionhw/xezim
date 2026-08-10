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
#[path = "classes/class_formal_typedef_widen.rs"]
mod class_formal_typedef_widen;
#[path = "classes/assoc_typedef_element_class.rs"]
mod assoc_typedef_element_class;
#[path = "classes/bit_class_property_signedness.rs"]
mod bit_class_property_signedness;
#[path = "classes/class_field_named_event.rs"]
mod class_field_named_event;
#[path = "classes/class_handle_return_preservation.rs"]
mod class_handle_return_preservation;
#[path = "classes/class_local_typedef_aa.rs"]
mod class_local_typedef_aa;
#[path = "classes/class_local_typedef_resolution.rs"]
mod class_local_typedef_resolution;
#[path = "classes/class_method_dispatch.rs"]
mod class_method_dispatch;
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
#[path = "classes/constraint_foreach_and_casts.rs"]
mod constraint_foreach_and_casts;
#[path = "classes/constraint_funcs_aggregates.rs"]
mod constraint_funcs_aggregates;
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
#[path = "classes/typename_param_class.rs"]
mod typename_param_class;
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
#[path = "classes/class_dynarray_property_elem_new.rs"]
mod class_dynarray_property_elem_new;
#[path = "classes/class_dynarray_property_fixed_copy.rs"]
mod class_dynarray_property_fixed_copy;
#[path = "classes/class_dynarray_property_size.rs"]
mod class_dynarray_property_size;
#[path = "classes/class_unpacked_struct_property.rs"]
mod class_unpacked_struct_property;
#[path = "classes/dollar_bits_class_member_shadow.rs"]
mod dollar_bits_class_member_shadow;
#[path = "classes/foreach_dynarray_property_elem.rs"]
mod foreach_dynarray_property_elem;
#[path = "classes/randomize_obj_array_property.rs"]
mod randomize_obj_array_property;
#[path = "classes/struct_output_formal.rs"]
mod struct_output_formal;

#[path = "classes/class_init_cast_copy.rs"]
mod class_init_cast_copy;

#[path = "classes/nested_and_extends_spec.rs"]
mod nested_and_extends_spec;
