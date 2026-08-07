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
#[path = "collections/assoc_compliance.rs"]
mod assoc_compliance;
#[path = "collections/assoc_method_dispatch.rs"]
mod assoc_method_dispatch;
#[path = "collections/concurrent_local_dyn_arrays.rs"]
mod concurrent_local_dyn_arrays;
#[path = "collections/constant_array_index_dependency.rs"]
mod constant_array_index_dependency;
#[path = "collections/cont_assign_2d_array_index0.rs"]
mod cont_assign_2d_array_index0;
#[path = "collections/dyn_array_of_mailboxes.rs"]
mod dyn_array_of_mailboxes;
#[path = "collections/fixed_array_member_initializer.rs"]
mod fixed_array_member_initializer;
#[path = "collections/fixed_array_member_pattern_forms.rs"]
mod fixed_array_member_pattern_forms;
#[path = "collections/foreach_blocking_resume.rs"]
mod foreach_blocking_resume;
#[path = "collections/foreach_negative_dims.rs"]
mod foreach_negative_dims;
#[path = "collections/lrm_clause7_arrays.rs"]
mod lrm_clause7_arrays;
#[path = "collections/mailbox_bounded.rs"]
mod mailbox_bounded;
#[path = "collections/multidim_assoc_array.rs"]
mod multidim_assoc_array;
#[path = "collections/noparen_array_methods.rs"]
mod noparen_array_methods;
#[path = "collections/pp_queue_slice.rs"]
mod pp_queue_slice;
#[path = "collections/property_queue_and_context.rs"]
mod property_queue_and_context;
#[path = "collections/queue_local_shadow_restore.rs"]
mod queue_local_shadow_restore;
#[path = "collections/queue_member_init.rs"]
mod queue_member_init;
#[path = "collections/queue_ops_and_dist.rs"]
mod queue_ops_and_dist;
#[path = "collections/sort_with_clause.rs"]
mod sort_with_clause;
#[path = "collections/struct_collections_across_calls.rs"]
mod struct_collections_across_calls;
#[path = "collections/stream_justify_assoc_default_partsel.rs"]
mod stream_justify_assoc_default_partsel;
#[path = "collections/whole_array_continuous_assign.rs"]
mod whole_array_continuous_assign;

#[path = "collections/multidim_assoc_nested_exists.rs"]
mod multidim_assoc_nested_exists;
#[path = "collections/aa_index_type_narrowing.rs"]
mod aa_index_type_narrowing;
#[path = "collections/whole_queue_copy.rs"]
mod whole_queue_copy;
#[path = "collections/struct_coll_element.rs"]
mod struct_coll_element;
#[path = "collections/class_coll_bits_and_pattern.rs"]
mod class_coll_bits_and_pattern;
#[path = "collections/queue_indexed_grow.rs"]
mod queue_indexed_grow;
#[path = "collections/string_foreach_content.rs"]
mod string_foreach_content;
