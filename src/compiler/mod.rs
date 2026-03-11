//! SystemVerilog compiler/simulator.
//!
//! Provides elaboration and event-driven simulation for
//! combinatorial logic designs with testbench support.

pub mod value;
pub mod elaborate;
pub mod simulator;

pub use value::Value;
pub use elaborate::{elaborate_module, ElaboratedModule};
pub use simulator::Simulator;
