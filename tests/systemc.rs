//! Integration-test group: SystemC simulation (IEEE 1666-2023 designs lowered
//! to SystemVerilog by xezim-core and simulated).
//!
//! The explicit module paths below are required: a crate root resolves a
//! plain `mod x;` beside itself, not into `tests/<group>/`. To add a test,
//! drop the file in this group's directory and add one entry here.

#[path = "systemc/simulation.rs"]
mod simulation;
