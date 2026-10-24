//! Integration-test group: VHDL (IEEE 1076-2008) designs.
//!
//! The explicit module paths below are required: a crate root resolves a
//! plain `mod x;` beside itself, not into `tests/<group>/`. To add a test,
//! drop the file in this group's directory and add one entry here.

#[path = "vhdl/front_end.rs"]
mod front_end;
