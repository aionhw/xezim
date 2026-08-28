//! Integration-test group: Verilog-AMS (`ams`).
//!
//! Staged support for Verilog-AMS on top of the SystemVerilog core. See
//! `docs/ams-plan.md` for the phasing; each stage adds its files here.
//!
//! As with every other group, the cases live one directory down and are
//! included as modules so the group links ONCE. To add a test, drop the file
//! in `tests/ams/` and add one entry below.

#[path = "ams/util.rs"]
mod util;
#[path = "ams/ams_mode.rs"]
mod ams_mode;
#[path = "ams/rnm_real_foundation.rs"]
mod rnm_real_foundation;
#[path = "ams/wreal_nets.rs"]
mod wreal_nets;
#[path = "ams/discipline_nature.rs"]
mod discipline_nature;
#[path = "ams/lrm_grammar.rs"]
mod lrm_grammar;
#[path = "ams/wreal_scale_and_dump.rs"]
mod wreal_scale_and_dump;
#[path = "ams/ams_mode_isolation.rs"]
mod ams_mode_isolation;
