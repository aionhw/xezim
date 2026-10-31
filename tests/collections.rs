//! Integration-test group: collections.
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

#[path = "collections/struct_copy_selector.rs"]
mod struct_copy_selector;

#[path = "collections/array_element_collection.rs"]
mod array_element_collection;
#[path = "collections/array_locator_and_reduction.rs"]
mod array_locator_and_reduction;
#[path = "collections/array_locator_index.rs"]
mod array_locator_index;
#[path = "collections/array_locator_named_iterator.rs"]
mod array_locator_named_iterator;
#[path = "collections/array_of_collections.rs"]
mod array_of_collections;
#[path = "collections/array_of_queues.rs"]
mod array_of_queues;
#[path = "collections/assoc_array_reductions.rs"]
mod assoc_array_reductions;
#[path = "collections/assoc_compliance.rs"]
mod assoc_compliance;
#[path = "collections/assoc_method_dispatch.rs"]
mod assoc_method_dispatch;
#[path = "collections/assoc_missing_key_default.rs"]
mod assoc_missing_key_default;
#[path = "collections/assoc_noparen_size.rs"]
mod assoc_noparen_size;
#[path = "collections/assoc_whole_copy_targets.rs"]
mod assoc_whole_copy_targets;
#[path = "collections/collection_write_open_gaps.rs"]
mod collection_write_open_gaps;
#[path = "collections/concurrent_local_dyn_arrays.rs"]
mod concurrent_local_dyn_arrays;
#[path = "collections/constant_array_index_dependency.rs"]
mod constant_array_index_dependency;
#[path = "collections/cont_assign_2d_array_index0.rs"]
mod cont_assign_2d_array_index0;
#[path = "collections/dyn_array_loop_write_notifies.rs"]
mod dyn_array_loop_write_notifies;
#[path = "collections/dyn_array_of_mailboxes.rs"]
mod dyn_array_of_mailboxes;
#[path = "collections/fixed_array_member_initializer.rs"]
mod fixed_array_member_initializer;
#[path = "collections/fixed_array_member_pattern_forms.rs"]
mod fixed_array_member_pattern_forms;
#[path = "collections/foreach_assoc_key_types.rs"]
mod foreach_assoc_key_types;
#[path = "collections/foreach_blocking_resume.rs"]
mod foreach_blocking_resume;
#[path = "collections/foreach_negative_dims.rs"]
mod foreach_negative_dims;
#[path = "collections/locator_item_index.rs"]
mod locator_item_index;
#[path = "collections/lrm_clause7_arrays.rs"]
mod lrm_clause7_arrays;
#[path = "collections/mailbox_array_new.rs"]
mod mailbox_array_new;
#[path = "collections/mailbox_bounded.rs"]
mod mailbox_bounded;
#[path = "collections/mailbox_handle_named_like_struct.rs"]
mod mailbox_handle_named_like_struct;
#[path = "collections/member_off_handle_collection_elem.rs"]
mod member_off_handle_collection_elem;
#[path = "classes/compiled_method_test_env.rs"]
mod compiled_method_test_env;
#[path = "collections/compiled_ret_coll_by_name.rs"]
mod compiled_ret_coll_by_name;
#[path = "collections/compiled_typedef_local_queue.rs"]
mod compiled_typedef_local_queue;
#[path = "collections/call_ret_store_into_local_coll.rs"]
mod call_ret_store_into_local_coll;
#[path = "collections/local_coll_init_fast_fn_eligible.rs"]
mod local_coll_init_fast_fn_eligible;
#[path = "collections/multidim_assoc_array.rs"]
mod multidim_assoc_array;
#[path = "collections/nested_assoc_entries.rs"]
mod nested_assoc_entries;
#[path = "collections/nested_assoc_under_outer_dim.rs"]
mod nested_assoc_under_outer_dim;
#[path = "collections/noparen_array_methods.rs"]
mod noparen_array_methods;
#[path = "collections/pp_queue_slice.rs"]
mod pp_queue_slice;
#[path = "collections/property_queue_and_context.rs"]
mod property_queue_and_context;
#[path = "collections/queue_eq_unknown_elems.rs"]
mod queue_eq_unknown_elems;
#[path = "collections/queue_local_shadow_restore.rs"]
mod queue_local_shadow_restore;
#[path = "collections/queue_member_init.rs"]
mod queue_member_init;
#[path = "collections/queue_ops_and_dist.rs"]
mod queue_ops_and_dist;
#[path = "collections/queue_store_element_type.rs"]
mod queue_store_element_type;
#[path = "collections/sibling_static_collection_collision.rs"]
mod sibling_static_collection_collision;
#[path = "collections/sort_with_clause.rs"]
mod sort_with_clause;
#[path = "collections/stream_justify_assoc_default_partsel.rs"]
mod stream_justify_assoc_default_partsel;
#[path = "collections/string_foreach_content.rs"]
mod string_foreach_content;
#[path = "collections/struct_collections_across_calls.rs"]
mod struct_collections_across_calls;
#[path = "collections/void_cast_queue_ops.rs"]
mod void_cast_queue_ops;
#[path = "collections/whole_array_continuous_assign.rs"]
mod whole_array_continuous_assign;
#[path = "collections/with_clause_class_receivers.rs"]
mod with_clause_class_receivers;

#[path = "collections/queue_dyn_write_semantics.rs"]
mod queue_dyn_write_semantics;

#[path = "collections/assoc_of_struct.rs"]
mod assoc_of_struct;
#[path = "collections/builtin_class_construction_forms.rs"]
mod builtin_class_construction_forms;
#[path = "collections/callee_local_shadow_assoc_output.rs"]
mod callee_local_shadow_assoc_output;
#[path = "collections/array_actual_hier_pkg_writeback.rs"]
mod array_actual_hier_pkg_writeback;
#[path = "collections/callee_local_shadow_int_assoc.rs"]
mod callee_local_shadow_int_assoc;
#[path = "collections/output_array_formal_starts_default.rs"]
mod output_array_formal_starts_default;
#[path = "collections/class_member_semaphore_keys.rs"]
mod class_member_semaphore_keys;
#[path = "collections/count_methods_return_signed_int.rs"]
mod count_methods_return_signed_int;
#[path = "collections/foreach_live_size_bounds.rs"]
mod foreach_live_size_bounds;
#[path = "collections/foreach_packed_multidim.rs"]
mod foreach_packed_multidim;
#[path = "collections/lrm_audit_argument_passing.rs"]
mod lrm_audit_argument_passing;
#[path = "collections/mailbox_unpacked_struct.rs"]
mod mailbox_unpacked_struct;
#[path = "collections/queue_concat_index_prefilled_prefix.rs"]
mod queue_concat_index_prefilled_prefix;
#[path = "collections/std_randomize_multidim.rs"]
mod std_randomize_multidim;
#[path = "collections/queue_struct_pop.rs"]
mod queue_struct_pop;
#[path = "collections/deferred_retsnap_slow_path.rs"]
mod deferred_retsnap_slow_path;
#[path = "collections/assoc_locator_methods.rs"]
mod assoc_locator_methods;
#[path = "collections/struct_dynamic_members.rs"]
mod struct_dynamic_members;
#[path = "collections/typedef_and_fixed_collections.rs"]
mod typedef_and_fixed_collections;
#[path = "collections/block_local_shadow_module.rs"]
mod block_local_shadow_module;
#[path = "collections/pattern_equality.rs"]
mod pattern_equality;
#[path = "collections/compiled_string_queue_element_compare.rs"]
mod compiled_string_queue_element_compare;
#[path = "collections/package_refs_in_subroutines.rs"]
mod package_refs_in_subroutines;
#[path = "collections/nested_assoc_foreach.rs"]
mod nested_assoc_foreach;
#[path = "collections/assoc_formal_struct_elems.rs"]
mod assoc_formal_struct_elems;
#[path = "collections/function_collection_return.rs"]
mod function_collection_return;
#[path = "collections/struct_dynamic_members_instances.rs"]
mod struct_dynamic_members_instances;
#[path = "collections/typedef_alias_dims.rs"]
mod typedef_alias_dims;
#[path = "collections/declarator_typedef_dims.rs"]
mod declarator_typedef_dims;
#[path = "collections/package_var_collisions.rs"]
mod package_var_collisions;
#[path = "collections/foreach_typedef_neighbours_312.rs"]
mod foreach_typedef_neighbours_312;

#[path = "collections/member_stream_340.rs"]
mod member_stream_340;

#[path = "collections/declaration_patterns_365.rs"]
mod next_twelve_declaration_patterns_365;

#[path = "collections/whole_array_nba_381.rs"]
mod next_rest_whole_array_nba_381;

#[path = "collections/collection_returns_387.rs"]
mod collection_returns_387;

#[path = "collections/review_package_collection_aliases.rs"]
mod review_package_collection_aliases;

#[path = "collections/foreach_omitted_var_before_collection.rs"]
mod foreach_omitted_var_before_collection;
#[path = "collections/assoc_select_write_and_return.rs"]
mod assoc_select_write_and_return;
#[path = "collections/foreach_subarray.rs"]
mod foreach_subarray;
#[path = "collections/assoc_coll_rows.rs"]
mod assoc_coll_rows;
#[path = "collections/dyn_rows_copy_595.rs"]
mod dyn_rows_copy_595;
#[path = "collections/dyn_row_print_order_597.rs"]
mod dyn_row_print_order_597;

#[path = "collections/row_of_2d_array_assign.rs"]
mod row_of_2d_array_assign;

#[path = "collections/conditional_aggregate_659.rs"]
mod conditional_aggregate_659;

#[path = "collections/assoc_key_enumeration_cache.rs"]
mod assoc_key_enumeration_cache;
#[path = "collections/hierarchical_whole_array_assign.rs"]
mod hierarchical_whole_array_assign;
#[path = "collections/inside_associative_values.rs"]
mod inside_associative_values;

#[path = "collections/foreach_indexed_receiver.rs"]
mod foreach_indexed_receiver;

#[path = "collections/enclosing_value_parameter_specialization.rs"]
mod enclosing_value_parameter_specialization;

#[path = "collections/lexical_value_parameter_specialization.rs"]
mod lexical_value_parameter_specialization;

#[path = "collections/coll_rows_typed_by_declaration.rs"]
mod coll_rows_typed_by_declaration;
