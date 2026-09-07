//! Integration-test group: strings.
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

#[path = "strings/local_string_array_init.rs"]
mod local_string_array_init;
#[path = "strings/assoc_in_always_ff_and_string_element_methods.rs"]
mod assoc_in_always_ff_and_string_element_methods;
#[path = "strings/call_time_defaults_and_string_queries.rs"]
mod call_time_defaults_and_string_queries;
#[path = "strings/class_member_string_queue_element_via_path.rs"]
mod class_member_string_queue_element_via_path;
#[path = "strings/display_only_always.rs"]
mod display_only_always;
#[path = "strings/dpi_integration_tests.rs"]
mod dpi_integration_tests;
#[path = "strings/dpi_child_module_import.rs"]
mod dpi_child_module_import;
#[path = "strings/format_lrm_compliance.rs"]
mod format_lrm_compliance;
#[path = "strings/format_sibling_fixes.rs"]
mod format_sibling_fixes;
#[path = "strings/fwrite_mcd_fd.rs"]
mod fwrite_mcd_fd;
#[path = "strings/hierarchical_string_method.rs"]
mod hierarchical_string_method;
#[path = "strings/interface_event_and_submodule_string.rs"]
mod interface_event_and_submodule_string;
#[path = "strings/local_string_dynamic.rs"]
mod local_string_dynamic;
#[path = "strings/lrm_string_methods.rs"]
mod lrm_string_methods;
#[path = "strings/nba_last_write_wins_elision.rs"]
mod nba_last_write_wins_elision;
#[path = "strings/nested_fork_shared_write.rs"]
mod nested_fork_shared_write;
#[path = "strings/nonzero_lsb_part_select_write.rs"]
mod nonzero_lsb_part_select_write;
#[path = "strings/p_format_assoc.rs"]
mod p_format_assoc;
#[path = "strings/p_format_named.rs"]
mod p_format_named;
#[path = "strings/p_format_recursive.rs"]
mod p_format_recursive;
#[path = "strings/ref_arg_assoc_writeback.rs"]
mod ref_arg_assoc_writeback;
#[path = "strings/ref_arg_collection_writeback.rs"]
mod ref_arg_collection_writeback;
#[path = "strings/string_eq_relational_2state.rs"]
mod string_eq_relational_2state;
#[path = "strings/string_flag_not_clobbered_by_samed_name_local.rs"]
mod string_flag_not_clobbered_by_samed_name_local;
#[path = "strings/string_index_ref_queue.rs"]
mod string_index_ref_queue;
#[path = "strings/string_is_dynamic.rs"]
mod string_is_dynamic;
#[path = "strings/string_methods_lrm.rs"]
mod string_methods_lrm;
#[path = "strings/string_property_shadowed_by_local.rs"]
mod string_property_shadowed_by_local;
#[path = "strings/system_task_gaps.rs"]
mod system_task_gaps;
#[path = "strings/fixed_string_array_dims.rs"]
mod fixed_string_array_dims;
#[path = "strings/compiled_sformatf_native.rs"]
mod compiled_sformatf_native;
#[path = "strings/native_string_ops.rs"]
mod native_string_ops;
#[path = "strings/string_returning_fn_inline.rs"]
mod string_returning_fn_inline;
#[path = "strings/dpi_unit_scope.rs"]
mod dpi_unit_scope;
