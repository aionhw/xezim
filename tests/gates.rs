//! Integration-test group: gates.
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

#[path = "gates/assign_pattern_aggregate.rs"]
mod assign_pattern_aggregate;
#[path = "gates/drive_strength_pull.rs"]
mod drive_strength_pull;
#[path = "gates/dump_merged_sv.rs"]
mod dump_merged_sv;
#[path = "gates/fn_return_member_inlines.rs"]
mod fn_return_member_inlines;
#[path = "gates/fst_roundtrip.rs"]
mod fst_roundtrip;
#[path = "gates/opt_pass_equivalence.rs"]
mod opt_pass_equivalence;
#[path = "gates/packed_member_nba_compiles.rs"]
mod packed_member_nba_compiles;
#[path = "gates/packed_member_nesting_compiles.rs"]
mod packed_member_nesting_compiles;
#[path = "gates/streaming_op_compiles.rs"]
mod streaming_op_compiles;
#[path = "gates/specify_flags.rs"]
mod specify_flags;
#[path = "gates/two_d_array_store_compiles.rs"]
mod two_d_array_store_compiles;
#[path = "gates/tran_and_implicit_nets.rs"]
mod tran_and_implicit_nets;
#[path = "gates/udp_primitives.rs"]
mod udp_primitives;
#[path = "gates/wave_flag_gates_dumping.rs"]
mod wave_flag_gates_dumping;
#[path = "gates/vcd_lrm_compliance.rs"]
mod vcd_lrm_compliance;
#[path = "gates/dump_formats_agree.rs"]
mod dump_formats_agree;
#[path = "gates/fst_time_table_breakeven.rs"]
mod fst_time_table_breakeven;
#[path = "gates/interrupt_finalizes_dumps.rs"]
mod interrupt_finalizes_dumps;
#[path = "gates/cont_assign_packed2d_unit_inner.rs"]
mod cont_assign_packed2d_unit_inner;
