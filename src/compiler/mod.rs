//! SystemVerilog bytecode interpreter.
//!
//! Shared elaboration/value/sdf/sinks live in `xezim-core` — re-exported
//! here for backwards compatibility so existing `xezim::compiler::...`
//! paths keep resolving.

pub mod bytecode;
pub mod jit;
pub mod simulator;

pub use simulator::Simulator;
pub use xezim_core::elaborate;
pub use xezim_core::elaborate::{elaborate_module, ElaboratedModule};
pub use xezim_core::sdf;
pub use xezim_core::stdout_sink;
pub use xezim_core::value;
pub use xezim_core::vcd_sink;
pub use xezim_core::Value;
