//! SystemVerilog bytecode interpreter.
//!
//! Shared elaboration/value/sdf/sinks live in `xezim-core` — re-exported
//! here for backwards compatibility so existing `xezim::compiler::...`
//! paths keep resolving.

pub mod act_stems;
pub mod arena;
pub mod bytecode;
pub mod dispatch;
pub mod fst_sink;
pub mod jit;
pub mod prof_sampler;
pub mod ts_jit;
#[cfg(feature = "jit")]
pub mod aot;
pub mod simulator;
pub mod soa;

pub use arena::{Arena, ArenaGuard, ArenaVec};
pub use dispatch::{DispatchTable, Opcode, NUM_OPCODES, get_dispatch_table};
pub use simulator::Simulator;
pub use xezim_core::elaborate;
pub use xezim_core::packed_value;
pub use xezim_core::elaborate::{elaborate_module, ElaboratedModule};
pub use xezim_core::sdf;
pub use xezim_core::stdout_sink;
pub use xezim_core::value;
pub use xezim_core::vcd_sink;
pub use xezim_core::Value;
