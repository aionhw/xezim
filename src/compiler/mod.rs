//! SystemVerilog bytecode interpreter.
//!
//! Shared elaboration/value/sdf/sinks live in `xezim-core` — re-exported
//! here for backwards compatibility so existing `xezim::compiler::...`
//! paths keep resolving.

#[cfg(feature = "jit")]
pub mod aot;
pub mod arena;
pub mod bytecode;
pub mod dispatch;
mod fb_scan;
pub mod fst_sink;
mod interrupt;
pub mod jit;
pub mod prof_sampler;
pub(crate) mod scoped_arrays;
pub mod simulator;
pub mod soa;
pub mod ts_jit;

pub use arena::{Arena, ArenaGuard, ArenaVec};
pub use dispatch::{DispatchTable, NUM_OPCODES, Opcode, get_dispatch_table};
pub use simulator::Simulator;
pub use soa::{InlineSignalArray, PackedWideSignalArray, SoaSignalStore, SoaSignalTable};
pub use xezim_core::Value;
pub use xezim_core::elaborate;
pub use xezim_core::elaborate::{ElaboratedModule, elaborate_module};
pub use xezim_core::packed_value;
pub use xezim_core::sdf;
pub use xezim_core::stdout_sink;
pub use xezim_core::value;
pub use xezim_core::vcd_sink;
