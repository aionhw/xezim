//! SystemVerilog compiler/simulator.
//!
//! Provides elaboration and event-driven simulation for
//! combinatorial logic designs with testbench support.

pub mod value;
pub mod elaborate;
pub mod simulator;
pub mod bytecode;
pub mod sdf;
pub mod native_codegen;
pub mod vcd_sink;
pub mod stdout_sink;

pub use value::Value;
pub use elaborate::{elaborate_module, ElaboratedModule};
pub use simulator::Simulator;
