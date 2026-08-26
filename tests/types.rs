//! Integration-test group: types.
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

#[path = "types/packed_struct_formal_member.rs"]
mod packed_struct_formal_member;
#[path = "types/block_local_width_in_index_shift.rs"]
mod block_local_width_in_index_shift;
#[path = "types/array_read_unknown_index_is_x.rs"]
mod array_read_unknown_index_is_x;
#[path = "types/ansi_port_name_with_unpacked_dim.rs"]
mod ansi_port_name_with_unpacked_dim;
#[path = "types/array_element_declared_signedness.rs"]
mod array_element_declared_signedness;
#[path = "types/array_parameter.rs"]
mod array_parameter;
#[path = "types/array_query_multidim_packed.rs"]
mod array_query_multidim_packed;
#[path = "types/assoc_of_queue_enumeration.rs"]
mod assoc_of_queue_enumeration;
#[path = "types/bits_of_signal_in_const_expr.rs"]
mod bits_of_signal_in_const_expr;
#[path = "types/bits_package_scoped_type_width.rs"]
mod bits_package_scoped_type_width;
#[path = "types/callback_typedef_static.rs"]
mod callback_typedef_static;
#[path = "types/cast_typeparam.rs"]
mod cast_typeparam;
#[path = "types/cast_valparam_typeparam_static.rs"]
mod cast_valparam_typeparam_static;
#[path = "types/cast_value_param_specs.rs"]
mod cast_value_param_specs;
#[path = "types/circular_typedef_diagnostics.rs"]
mod circular_typedef_diagnostics;
#[path = "types/conditional_real_x_select.rs"]
mod conditional_real_x_select;
#[path = "types/const_function_params.rs"]
mod const_function_params;
#[path = "types/constructor_new_not_static.rs"]
mod constructor_new_not_static;
#[path = "types/constructor_new_property_type.rs"]
mod constructor_new_property_type;
#[path = "types/constructor_typedef_dispatch.rs"]
mod constructor_typedef_dispatch;
#[path = "types/copy_constructor_decl_init.rs"]
mod copy_constructor_decl_init;
#[path = "types/copy_ctor_type_mismatch.rs"]
mod copy_ctor_type_mismatch;
#[path = "types/declared_signedness_wins.rs"]
mod declared_signedness_wins;
#[path = "types/default_assignment_pattern_packed.rs"]
mod default_assignment_pattern_packed;
#[path = "types/defparam_override.rs"]
mod defparam_override;
#[path = "types/dollar_lvalue_and_assoc_width.rs"]
mod dollar_lvalue_and_assoc_width;
#[path = "types/enum_name_formal_param.rs"]
mod enum_name_formal_param;
#[path = "types/enum_next_prev_count.rs"]
mod enum_next_prev_count;
#[path = "types/enum_signedness_and_pkg_scope.rs"]
mod enum_signedness_and_pkg_scope;
#[path = "types/enum_xz_and_package_shapes.rs"]
mod enum_xz_and_package_shapes;
#[path = "types/formal_type_metadata_and_typedef_packed_array.rs"]
mod formal_type_metadata_and_typedef_packed_array;
#[path = "types/forward_referenced_parameter.rs"]
mod forward_referenced_parameter;
#[path = "types/forward_typedef_class_handle_width.rs"]
mod forward_typedef_class_handle_width;
#[path = "types/function_return_width.rs"]
mod function_return_width;
#[path = "types/implicit_port_net_and_packed_typedef_array.rs"]
mod implicit_port_net_and_packed_typedef_array;
#[path = "types/instance_queue_structs_and_loop_scope.rs"]
mod instance_queue_structs_and_loop_scope;
#[path = "types/ivtest_cast_cluster.rs"]
mod ivtest_cast_cluster;
#[path = "types/local_localparam_width.rs"]
mod local_localparam_width;
#[path = "types/local_packed_struct_alias.rs"]
mod local_packed_struct_alias;
#[path = "types/lrm_dynamic_cast.rs"]
mod lrm_dynamic_cast;
#[path = "types/lrm_real_cast_case_inside.rs"]
mod lrm_real_cast_case_inside;
#[path = "types/lrm_struct_union_dims.rs"]
mod lrm_struct_union_dims;
#[path = "types/module_type_param_behavioral.rs"]
mod module_type_param_behavioral;
#[path = "types/multi_dim_elem_read_and_param_dims.rs"]
mod multi_dim_elem_read_and_param_dims;
#[path = "types/ascending_indexed_part_select.rs"]
mod ascending_indexed_part_select;
#[path = "types/nonzero_based_vector_writes.rs"]
mod nonzero_based_vector_writes;
#[path = "types/nonzero_based_vector_reads.rs"]
mod nonzero_based_vector_reads;
#[path = "types/nested_packed_typedefs.rs"]
mod nested_packed_typedefs;
#[path = "types/packed_2d_element_in_size_cast.rs"]
mod packed_2d_element_in_size_cast;
#[path = "types/packed_2d_net_element_assign.rs"]
mod packed_2d_net_element_assign;
#[path = "types/packed_array_typedef_element_width.rs"]
mod packed_array_typedef_element_width;
#[path = "types/packed_assignment_patterns.rs"]
mod packed_assignment_patterns;
#[path = "types/packed_element_range_select.rs"]
mod packed_element_range_select;
#[path = "types/packed_multidim_unpacked_select.rs"]
mod packed_multidim_unpacked_select;
#[path = "types/param_cb_isolation.rs"]
mod param_cb_isolation;
#[path = "types/param_signedness_and_generate_scope.rs"]
mod param_signedness_and_generate_scope;
#[path = "types/param_sized_array.rs"]
mod param_sized_array;
#[path = "types/param_struct_widths_and_nba_patterns.rs"]
mod param_struct_widths_and_nba_patterns;
#[path = "types/parameter_const_eval_corners.rs"]
mod parameter_const_eval_corners;
#[path = "types/pattern_params_and_call_member.rs"]
mod pattern_params_and_call_member;
#[path = "types/power_operator_signedness.rs"]
mod power_operator_signedness;
#[path = "types/signed_mixed_equality.rs"]
mod signed_mixed_equality;
#[path = "types/property_value_param_binding.rs"]
mod property_value_param_binding;
#[path = "types/range_select_param_arith.rs"]
mod range_select_param_arith;
#[path = "types/real_literal_comb_eval.rs"]
mod real_literal_comb_eval;
#[path = "types/real_valued_delay.rs"]
mod real_valued_delay;
#[path = "types/shift_context_width.rs"]
mod shift_context_width;
#[path = "types/shift_width_and_scope_hint.rs"]
mod shift_width_and_scope_hint;
#[path = "types/size_cast_context_and_fn_return_default.rs"]
mod size_cast_context_and_fn_return_default;
#[path = "types/static_typedef_singleton.rs"]
mod static_typedef_singleton;
#[path = "types/streaming_and_typedef_array_width.rs"]
mod streaming_and_typedef_array_width;
#[path = "types/struct_copy_and_queue_ops.rs"]
mod struct_copy_and_queue_ops;
#[path = "types/struct_named_patterns.rs"]
mod struct_named_patterns;
#[path = "types/struct_unpacked_array_member.rs"]
mod struct_unpacked_array_member;
#[path = "types/submodule_packed_struct_member_contassign.rs"]
mod submodule_packed_struct_member_contassign;
#[path = "types/tagged_union_matches.rs"]
mod tagged_union_matches;
#[path = "types/tagged_union_pattern_literal.rs"]
mod tagged_union_pattern_literal;
#[path = "types/task_formal_param_width.rs"]
mod task_formal_param_width;
#[path = "types/type_param_bound_to_specialization.rs"]
mod type_param_bound_to_specialization;
#[path = "types/type_param_static_property.rs"]
mod type_param_static_property;
#[path = "types/typedef_declaring_scope.rs"]
mod typedef_declaring_scope;
#[path = "types/typedef_specialization_dispatch.rs"]
mod typedef_specialization_dispatch;
#[path = "types/typedef_two_state_and_untyped_param_sign.rs"]
mod typedef_two_state_and_untyped_param_sign;
#[path = "types/typeparam_default_resolution.rs"]
mod typeparam_default_resolution;
#[path = "types/typeparam_typeid_create.rs"]
mod typeparam_typeid_create;
#[path = "types/typeref_param_static_local.rs"]
mod typeref_param_static_local;
#[path = "types/union_shared_storage.rs"]
mod union_shared_storage;
#[path = "types/unit_scope_user_type_var.rs"]
mod unit_scope_user_type_var;
#[path = "types/unpacked_array_ports_and_vif_arrays.rs"]
mod unpacked_array_ports_and_vif_arrays;
#[path = "types/unpacked_struct_array_members.rs"]
mod unpacked_struct_array_members;
#[path = "types/unpacked_struct_func_arg.rs"]
mod unpacked_struct_func_arg;
#[path = "types/valparam_spec_cycle.rs"]
mod valparam_spec_cycle;
#[path = "types/value_param_specialization.rs"]
mod value_param_specialization;
#[path = "types/packed_elem_shift_context_width.rs"]
mod packed_elem_shift_context_width;
#[path = "types/vcd_param_as_wire.rs"]
mod vcd_param_as_wire;
#[path = "types/wide_signed_arith_and_power.rs"]
mod wide_signed_arith_and_power;
#[path = "types/byte_local_narrow.rs"]
mod byte_local_narrow;
#[path = "types/packed_struct_array_elem_write.rs"]
mod packed_struct_array_elem_write;
#[path = "types/bits_of_type_operands.rs"]
mod bits_of_type_operands;
#[path = "types/packed_3d_chained_select.rs"]
mod packed_3d_chained_select;
#[path = "types/unary_context_width.rs"]
mod unary_context_width;
#[path = "types/nested_packed_struct_array_access.rs"]
mod nested_packed_struct_array_access;
#[path = "types/package_typedef_scoping.rs"]
mod package_typedef_scoping;
#[path = "types/diag_kind_limit_env.rs"]
mod diag_kind_limit_env;
#[path = "types/package_scope_resolution.rs"]
mod package_scope_resolution;
#[path = "types/preprocessor_include_fatal.rs"]
mod preprocessor_include_fatal;
#[path = "types/wire_typedef_declarations.rs"]
mod wire_typedef_declarations;
#[path = "types/generate_scope_struct_metadata.rs"]
mod generate_scope_struct_metadata;
#[path = "types/signedness_and_power_context.rs"]
mod signedness_and_power_context;
#[path = "types/width_context_discipline.rs"]
mod width_context_discipline;
#[path = "types/hierarchy_and_type_overrides.rs"]
mod hierarchy_and_type_overrides;

#[path = "types/array_reduction_element_type.rs"]
mod array_reduction_element_type;

#[path = "types/lexical_string_literal_semantics.rs"]
mod lexical_string_literal_semantics;

#[path = "types/procedural_storage_semantics.rs"]
mod procedural_storage_semantics;

#[path = "types/expression_event_controls.rs"]
mod expression_event_controls;

#[path = "types/packed_multilevel_and_collision.rs"]
mod packed_multilevel_and_collision;

#[path = "types/star_vs_always_comb.rs"]
mod star_vs_always_comb;

#[path = "types/force_expression_tracks.rs"]
mod force_expression_tracks;

#[path = "types/procedural_loop_and_static_fn.rs"]
mod procedural_loop_and_static_fn;

#[path = "types/inside_defaults_and_foreach_dir.rs"]
mod inside_defaults_and_foreach_dir;

#[path = "types/param_const_eval_contexts.rs"]
mod param_const_eval_contexts;

#[path = "types/struct_port_pattern_ca.rs"]
mod struct_port_pattern_ca;

#[path = "types/packed2d_slice_ca_offset.rs"]
mod packed2d_slice_ca_offset;

#[path = "types/typedef_bits_of_signal_dims.rs"]
mod typedef_bits_of_signal_dims;

#[path = "types/nested_dynamic_members.rs"]
mod nested_dynamic_members;

#[path = "types/assoc_enum_key_name.rs"]
mod assoc_enum_key_name;
#[path = "types/struct_real_member_roundtrip.rs"]
mod struct_real_member_roundtrip;
#[path = "types/assoc_keys_and_handles.rs"]
mod assoc_keys_and_handles;
#[path = "types/zero_mask_call_elision.rs"]
mod zero_mask_call_elision;
#[path = "types/formal_metadata_shadow_roundtrip.rs"]
mod formal_metadata_shadow_roundtrip;
#[path = "types/decode_helper_assign_compiles.rs"]
mod decode_helper_assign_compiles;
#[path = "types/unpacked_elem_compare_width.rs"]
mod unpacked_elem_compare_width;
#[path = "types/tf_port_direction_inheritance.rs"]
mod tf_port_direction_inheritance;
#[path = "types/inside_const_members.rs"]
mod inside_const_members;
#[path = "types/packed_struct_pattern_compile.rs"]
mod packed_struct_pattern_compile;
#[path = "types/task_fsm_compile.rs"]
mod task_fsm_compile;
#[path = "types/case_jump_dispatch.rs"]
mod case_jump_dispatch;
#[path = "types/case_wildcard_signed_extension.rs"]
mod case_wildcard_signed_extension;
#[path = "types/case_mask_jump_dispatch.rs"]
mod case_mask_jump_dispatch;
#[path = "types/packed_member_self_determined_width.rs"]
mod packed_member_self_determined_width;
#[path = "types/zero_width_select_confidence.rs"]
mod zero_width_select_confidence;
#[path = "types/typedef_chain_local_namespace.rs"]
mod typedef_chain_local_namespace;
