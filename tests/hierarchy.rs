//! Integration-test group: hierarchy.
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

#[path = "hierarchy/absolute_path_writes_in_subroutines.rs"]
mod absolute_path_writes_in_subroutines;
#[path = "hierarchy/array_of_module_instances.rs"]
mod array_of_module_instances;
#[path = "hierarchy/bind_directive_basic.rs"]
mod bind_directive_basic;
#[path = "hierarchy/bind_static_init_syscall_scope.rs"]
mod bind_static_init_syscall_scope;
#[path = "hierarchy/bind_in_module.rs"]
mod bind_in_module;
#[path = "hierarchy/bind_into_interface.rs"]
mod bind_into_interface;
#[path = "hierarchy/bind_path_through_library_module.rs"]
mod bind_path_through_library_module;
#[path = "hierarchy/bind_through_nested_part_select.rs"]
mod bind_through_nested_part_select;
#[path = "hierarchy/bind_upward_refs.rs"]
mod bind_upward_refs;
#[path = "hierarchy/c910_scoped_cont_assign.rs"]
mod c910_scoped_cont_assign;
#[path = "hierarchy/class_method_hier_subroutine_calls.rs"]
mod class_method_hier_subroutine_calls;
#[path = "hierarchy/colliding_actual_child_body.rs"]
mod colliding_actual_child_body;
#[path = "hierarchy/defparam_upward_paths.rs"]
mod defparam_upward_paths;
#[path = "hierarchy/downward_hier_refs_in_siblings.rs"]
mod downward_hier_refs_in_siblings;
#[path = "hierarchy/downward_write_with_live_objects.rs"]
mod downward_write_with_live_objects;
#[path = "hierarchy/dump_merged_sv_adopted_primary.rs"]
mod dump_merged_sv_adopted_primary;
#[path = "hierarchy/dump_merged_sv_library_dedup.rs"]
mod dump_merged_sv_library_dedup;
#[path = "hierarchy/dump_merged_sv_top_pruning.rs"]
mod dump_merged_sv_top_pruning;
#[path = "hierarchy/expression_port_actuals.rs"]
mod expression_port_actuals;
#[path = "hierarchy/final_blocks_hierarchy.rs"]
mod final_blocks_hierarchy;
#[path = "hierarchy/flat_actual_struct_port.rs"]
mod flat_actual_struct_port;
#[path = "hierarchy/foreach_in_inlined_child.rs"]
mod foreach_in_inlined_child;
#[path = "hierarchy/foreach_over_submodule_array.rs"]
mod foreach_over_submodule_array;
#[path = "hierarchy/genblk_dotted_ref_nettype_none.rs"]
mod genblk_dotted_ref_nettype_none;
#[path = "hierarchy/genblk_cont_assign.rs"]
mod genblk_cont_assign;
#[path = "hierarchy/generate_label_hier_roots.rs"]
mod generate_label_hier_roots;
#[path = "hierarchy/generate_scope_names.rs"]
mod generate_scope_names;
#[path = "hierarchy/generate_scope_subroutine_calls.rs"]
mod generate_scope_subroutine_calls;
#[path = "hierarchy/generic_interface_ports.rs"]
mod generic_interface_ports;
#[path = "hierarchy/ifu_ibuf_32_instances_c910.rs"]
mod ifu_ibuf_32_instances_c910;
#[path = "hierarchy/implicit_net_gate_port.rs"]
mod implicit_net_gate_port;
#[path = "hierarchy/implicit_net_name_collision.rs"]
mod implicit_net_name_collision;
#[path = "hierarchy/implicit_net_submodule.rs"]
mod implicit_net_submodule;
#[path = "hierarchy/imported_name_hier_ref.rs"]
mod imported_name_hier_ref;
#[path = "hierarchy/inout_port_default.rs"]
mod inout_port_default;
#[path = "hierarchy/instance_var_init_signedness.rs"]
mod instance_var_init_signedness;
#[path = "hierarchy/interface_array_element_ports.rs"]
mod interface_array_element_ports;
#[path = "hierarchy/interface_default_clocking.rs"]
mod interface_default_clocking;
#[path = "hierarchy/ivtest_port_cluster.rs"]
mod ivtest_port_cluster;
#[path = "hierarchy/local_method_static_binding.rs"]
mod local_method_static_binding;
#[path = "hierarchy/member_access_roots.rs"]
mod member_access_roots;
#[path = "hierarchy/mixed_port_connections.rs"]
mod mixed_port_connections;
#[path = "hierarchy/module_class_bare_names.rs"]
mod module_class_bare_names;
#[path = "hierarchy/module_timescale_cli.rs"]
mod module_timescale_cli;
#[path = "hierarchy/multi_instance_scope.rs"]
mod multi_instance_scope;
#[path = "hierarchy/multi_top_paths.rs"]
mod multi_top_paths;
#[path = "hierarchy/multiple_top_modules.rs"]
mod multiple_top_modules;
#[path = "hierarchy/nested_cross_module_call.rs"]
mod nested_cross_module_call;
#[path = "hierarchy/nested_interface_struct_ports.rs"]
mod nested_interface_struct_ports;
#[path = "hierarchy/nested_member_in_instance.rs"]
mod nested_member_in_instance;
#[path = "hierarchy/nonansi_port_completion.rs"]
mod nonansi_port_completion;
#[path = "hierarchy/nonansi_port_declarations.rs"]
mod nonansi_port_declarations;
#[path = "hierarchy/nonansi_port_expressions.rs"]
mod nonansi_port_expressions;
#[path = "hierarchy/nonansi_port_redeclaration.rs"]
mod nonansi_port_redeclaration;
#[path = "hierarchy/nonansi_ports_and_vif_accessors.rs"]
mod nonansi_ports_and_vif_accessors;
#[path = "hierarchy/nonzero_lsb_port_select.rs"]
mod nonzero_lsb_port_select;
#[path = "hierarchy/null_ports_lib_defines_nowarn.rs"]
mod null_ports_lib_defines_nowarn;
#[path = "hierarchy/package_scoped_vars_and_methods.rs"]
mod package_scoped_vars_and_methods;
#[path = "hierarchy/packed_port_formal_type.rs"]
mod packed_port_formal_type;
#[path = "hierarchy/packed_record_output_chain.rs"]
mod packed_record_output_chain;
#[path = "hierarchy/param_typedef_instance_isolation.rs"]
mod param_typedef_instance_isolation;
#[path = "hierarchy/parameter_override_identifiers.rs"]
mod parameter_override_identifiers;
#[path = "hierarchy/parenless_task_enables.rs"]
mod parenless_task_enables;
#[path = "hierarchy/pct_m_block_scope.rs"]
mod pct_m_block_scope;
#[path = "hierarchy/percent_m_declaring_scope.rs"]
mod percent_m_declaring_scope;
#[path = "hierarchy/percent_m_module_init_scope.rs"]
mod percent_m_module_init_scope;
#[path = "hierarchy/percent_m_scope.rs"]
mod percent_m_scope;
#[path = "hierarchy/pkg_subroutines_and_unit_scope.rs"]
mod pkg_subroutines_and_unit_scope;
#[path = "hierarchy/port_actual_legality.rs"]
mod port_actual_legality;
#[path = "hierarchy/port_default_child_scope.rs"]
mod port_default_child_scope;
#[path = "hierarchy/port_net_driven_from_inside.rs"]
mod port_net_driven_from_inside;
#[path = "hierarchy/port_net_pull_resolution.rs"]
mod port_net_pull_resolution;
#[path = "hierarchy/repro_import.rs"]
mod repro_import;
#[path = "hierarchy/resource_pool_scope_lookup.rs"]
mod resource_pool_scope_lookup;
#[path = "hierarchy/same_name_port_hop.rs"]
mod same_name_port_hop;
#[path = "hierarchy/submodule_array_element_assigns.rs"]
mod submodule_array_element_assigns;
#[path = "hierarchy/submodule_net_array_shapes.rs"]
mod submodule_net_array_shapes;
#[path = "hierarchy/submodule_two_dim_array_element.rs"]
mod submodule_two_dim_array_element;
#[path = "hierarchy/t_bound_generic_value_param_argument.rs"]
mod t_bound_generic_value_param_argument;
#[path = "hierarchy/t_bound_member_collection_registration.rs"]
mod t_bound_member_collection_registration;
#[path = "hierarchy/t_bound_queue_member_storage.rs"]
mod t_bound_queue_member_storage;
#[path = "hierarchy/t_bound_queue_value_param_chain.rs"]
mod t_bound_queue_value_param_chain;
#[path = "hierarchy/t_nested_value_param_in_type_arg.rs"]
mod t_nested_value_param_in_type_arg;
#[path = "hierarchy/t_no_parens_static_method.rs"]
mod t_no_parens_static_method;
#[path = "hierarchy/typedef_array_ports_in_children.rs"]
mod typedef_array_ports_in_children;
#[path = "hierarchy/undefined_call_in_instance.rs"]
mod undefined_call_in_instance;
#[path = "hierarchy/undriven_implicit_output_is_z.rs"]
mod undriven_implicit_output_is_z;
#[path = "hierarchy/unpacked_struct_port.rs"]
mod unpacked_struct_port;
#[path = "hierarchy/unresolved_hier_refs.rs"]
mod unresolved_hier_refs;
#[path = "hierarchy/vif_arrays_and_null_default.rs"]
mod vif_arrays_and_null_default;
#[path = "hierarchy/vif_clocking_in_methods.rs"]
mod vif_clocking_in_methods;
#[path = "hierarchy/vif_hierarchical_instance_path.rs"]
mod vif_hierarchical_instance_path;
#[path = "hierarchy/vif_in_subroutines.rs"]
mod vif_in_subroutines;
#[path = "hierarchy/vif_typedef_null_default.rs"]
mod vif_typedef_null_default;
#[path = "hierarchy/wildcard_import_shadow.rs"]
mod wildcard_import_shadow;

#[path = "hierarchy/nested_modules_and_zpad.rs"]
mod nested_modules_and_zpad;

#[path = "hierarchy/bind_grid_module_name_refs.rs"]
mod bind_grid_module_name_refs;
#[path = "hierarchy/bind_with_parameters.rs"]
mod bind_with_parameters;
#[path = "hierarchy/formal_shadows_instance_signal.rs"]
mod formal_shadows_instance_signal;
#[path = "hierarchy/iface_port_params.rs"]
mod iface_port_params;
#[path = "hierarchy/inlined_interface_array_member_refs.rs"]
mod inlined_interface_array_member_refs;
#[path = "hierarchy/instance_block_locals_shadow_module_names.rs"]
mod instance_block_locals_shadow_module_names;
#[path = "hierarchy/instance_clocking_block_event.rs"]
mod instance_clocking_block_event;
#[path = "hierarchy/instance_collections_resolve_by_scope.rs"]
mod instance_collections_resolve_by_scope;
#[path = "hierarchy/instance_loop_local_shadows_module_var.rs"]
mod instance_loop_local_shadows_module_var;
#[path = "hierarchy/instance_task_in_forever_loop.rs"]
mod instance_task_in_forever_loop;
#[path = "hierarchy/interface_port_inside.rs"]
mod interface_port_inside;
#[path = "hierarchy/interface_task_lexical_scope.rs"]
mod interface_task_lexical_scope;
#[path = "hierarchy/library_dir_on_demand.rs"]
mod library_dir_on_demand;
#[path = "hierarchy/lrm_package_variable_scoping.rs"]
mod lrm_package_variable_scoping;
#[path = "hierarchy/lrm_audit_time0_port_event.rs"]
mod lrm_audit_time0_port_event;
#[path = "hierarchy/modport_expression_ports.rs"]
mod modport_expression_ports;
#[path = "hierarchy/modport_shape_battery.rs"]
mod modport_shape_battery;
#[path = "hierarchy/multi_top_typedef_packed_dims.rs"]
mod multi_top_typedef_packed_dims;
#[path = "hierarchy/nonansi_modport_port_decl.rs"]
mod nonansi_modport_port_decl;
#[path = "hierarchy/ref_formal_same_name.rs"]
mod ref_formal_same_name;
#[path = "hierarchy/single_element_instance_array.rs"]
mod single_element_instance_array;
#[path = "hierarchy/unit_scope_library_subroutine.rs"]
mod unit_scope_library_subroutine;
#[path = "hierarchy/unobserved_port_elision.rs"]
mod unobserved_port_elision;
#[path = "hierarchy/vif_packed_array_elem_nba.rs"]
mod vif_packed_array_elem_nba;
#[path = "hierarchy/named_block_hier_names.rs"]
mod named_block_hier_names;

#[path = "hierarchy/instance_array_342.rs"]
mod instance_array_342;

#[path = "hierarchy/positional_ports_358.rs"]
mod next_twelve_positional_ports_358;

#[path = "hierarchy/generate_names_380.rs"]
mod next_rest_generate_names_380;

#[path = "hierarchy/package_virtual_interfaces.rs"]
mod package_virtual_interfaces;

#[path = "hierarchy/vif_associative_bindings.rs"]
mod vif_associative_bindings;

#[path = "hierarchy/vif_returned_receivers.rs"]
mod vif_returned_receivers;

#[path = "hierarchy/vif_specialized_collections.rs"]
mod vif_specialized_collections;

#[path = "hierarchy/vif_null_binding_copies.rs"]
mod vif_null_binding_copies;

#[path = "hierarchy/bind_generate_paths.rs"]
mod bind_generate_paths;

#[path = "hierarchy/signed_generate_instances.rs"]
mod signed_generate_instances;

#[path = "hierarchy/vif_sibling_regressions.rs"]
mod vif_sibling_regressions;

#[path = "hierarchy/bind_sibling_regressions.rs"]
mod bind_sibling_regressions;

#[path = "hierarchy/vif_sibling_neighbors.rs"]
mod vif_sibling_neighbors;

#[path = "hierarchy/bind_sibling_neighbors.rs"]
mod bind_sibling_neighbors;

#[path = "hierarchy/interface_static_initializers.rs"]
mod interface_static_initializers;

#[path = "hierarchy/bare_bind_targets.rs"]
mod bare_bind_targets;

#[path = "hierarchy/pr_core_followups.rs"]
mod pr_core_followups;

#[path = "hierarchy/interface_array_slices.rs"]
mod interface_array_slices;

#[path = "hierarchy/static_vif_specialization_audit.rs"]
mod static_vif_specialization_audit;

#[path = "hierarchy/static_vif_scope_audit.rs"]
mod static_vif_scope_audit;

#[path = "hierarchy/interface_port_hierarchy.rs"]
mod interface_port_hierarchy;

#[path = "hierarchy/package_array_params.rs"]
mod package_array_params;

#[path = "hierarchy/vif_defaults_469.rs"]
mod vif_defaults_469;

#[path = "hierarchy/static_init_dependency_order.rs"]
mod static_init_dependency_order;
#[path = "hierarchy/review_static_init_dependencies.rs"]
mod review_static_init_dependencies;
